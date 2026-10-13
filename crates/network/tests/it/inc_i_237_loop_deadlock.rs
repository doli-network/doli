//! INC-I-237 — mutual bounded-channel deadlock between the node event loop
//! and the network swarm loop.
//!
//! Cycle (production code):
//!   - swarm loop `run_swarm` (service/swarm_loop.rs:175-176) awaits
//!     `handle_swarm_event(..).await` inside the `select!` arm; while it is
//!     suspended, the `command_rx.recv()` arm (swarm_loop.rs:180) is not polled.
//!   - tx gossip is forwarded with `event_tx.send(NewTransaction).await`
//!     (service/behaviour_events.rs:290), one await per tx in a 50-tx batch.
//!     When the event channel (4096 at min config) is full, the swarm loop
//!     parks there.
//!   - the node loop is the only event consumer. In periodic.rs:417 it awaits
//!     `network.request_sync(..)` = `command_tx.send(..).await`
//!     (service/mod.rs:569-579). When the command channel (2048) is full, the
//!     node loop parks there and never drains events again.
//!
//! Topology: two real `NetworkService`s over 127.0.0.1 (production transport,
//! behaviour, swarm loop, channel sizing). F floods unique tx batches via its
//! production `broadcast_transaction`. A's "node loop" stops draining events
//! and keeps issuing `request_sync` (the ATTEST_FETCH shape).
//!
//! OUTPUT CONTRACT
//!   O1: each `request_sync` on A completes within SEND_TIMEOUT (liveness)
//!   O2: NewTransaction events A received before the stall (non-vacuity)
//!   O3: A's command channel free capacity at the stall (0 = channel full)
//!   O4 (M1, REQ-GRV-006): A's swarm serves a new peer while A's consumer is
//!       stopped (G sees PeerConnected(A), A's peer table grows) and, after the
//!       consumer resumes, the backlogged PeerConnected(G) control event is
//!       delivered behind the queued gossip
//!   O5 (M1, F-11): dropping A's NetworkService with a full event channel and a
//!       stopped consumer ends A's swarm (F sees PeerDisconnected(A)) <= 5 s
//! PATHS: P1 swarm loop not blocked -> commands drained, O1 holds;
//!        P2 swarm loop parked on event_tx.send -> commands pile up, O1 fails;
//!        P3 control event while the channel is full -> backlog -> try_flush;
//!        P4 node handle dropped while the channel is full -> swarm exits.
//! INPUT PARTITIONS: IP-1 unique tx flood > event_buf, consumer stalled;
//!   IP-2 control event (inbound peer) during IP-1; IP-3 handle drop during IP-1.

use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Transaction;
use network::protocols::SyncRequest;
use network::{NetworkConfig, NetworkEvent, NetworkService, PeerId};

/// max_peers=8 -> event_buf = clamp(128, 4096, 32768) = 4096,
/// command_buf = clamp(32, 2048, 32768) = 2048 (service/mod.rs:87-88).
const MAX_PEERS: usize = 8;
const EVENT_BUF: usize = 4096;
const COMMAND_BUF: usize = 2048;
/// Unique txs flooded by F (> EVENT_BUF; one tx per gossip message since M2 R4).
const FLOOD_TXS: u64 = 12_000;
/// M2: A's swarm admits one tx message per token (500 + 50/s per peer, 1000 +
/// 200/s in total), so a full bulk lane needs extra flooders and a paced window.
const EXTRA_FLOODERS: usize = 4;
const PACED_FLOOD: Duration = Duration::from_secs(12);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("reserve port")
        .port()
}

async fn start_node(port: u16, bootstrap_nodes: Vec<String>) -> NetworkService {
    let listen_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let config = NetworkConfig {
        listen_addr,
        bootstrap_nodes,
        max_peers: MAX_PEERS,
        bootstrap_slots: MAX_PEERS,
        no_dht: true,
        enable_discv5: false,
        node_key_path: None,
        peer_cache_path: None,
        ..NetworkConfig::default()
    };
    NetworkService::new(config)
        .await
        .expect("start NetworkService")
}

fn unique_tx(i: u64) -> Transaction {
    Transaction::new_coinbase(1_000 + i, Hash::default(), 1_000_000 + i, i as u32)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inc_i_237_node_loop_command_send_survives_tx_flood() {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let a_port = reserve_port();
    let f_port = reserve_port();
    let mut a = start_node(a_port, Vec::new()).await;
    let f = start_node(f_port, vec![format!("/ip4/127.0.0.1/tcp/{a_port}")]).await;
    let f_id: PeerId = *f.local_peer_id();

    // F's consumer: always drains (F is not the node under test).
    let mut f = f;
    let f_cmd = f.command_sender();
    let f_drain = tokio::spawn(async move { while f.next_event().await.is_some() {} });

    // Phase 1: connect + warm the tx mesh until A sees a NewTransaction.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut warm = 0u64;
    'warm: loop {
        assert!(
            Instant::now() < deadline,
            "A never received warm-up tx gossip"
        );
        for _ in 0..50 {
            f_cmd
                .send(network::NetworkCommand::BroadcastTransaction(unique_tx(
                    10_000_000 + warm,
                )))
                .await
                .unwrap();
            warm += 1;
        }
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            if let Ok(Some(NetworkEvent::NewTransaction(..))) =
                tokio::time::timeout(Duration::from_millis(100), a.next_event()).await
            {
                break 'warm;
            }
        }
    }
    while a.try_next_event().is_some() {}

    // Phase 2: A's node loop stops draining events. F floods unique txs.
    for i in 0..FLOOD_TXS {
        f_cmd
            .send(network::NetworkCommand::BroadcastTransaction(unique_tx(i)))
            .await
            .unwrap();
    }
    // Let the flood land on A and fill its event channel.
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Phase 3: the node loop keeps sending commands (ATTEST_FETCH shape).
    let a_cmd = a.command_sender();
    let mut sent = 0usize;
    let mut stalled_at: Option<usize> = None;
    let mut cap_at_stall = usize::MAX;
    for i in 0..(COMMAND_BUF + 256) {
        let req = SyncRequest::get_block_by_hash(crypto::hash::hash(&(i as u64).to_le_bytes()));
        match tokio::time::timeout(SEND_TIMEOUT, a.request_sync(f_id, req)).await {
            Ok(r) => {
                r.expect("command channel closed");
                sent += 1;
            }
            Err(_) => {
                stalled_at = Some(i);
                cap_at_stall = a_cmd.capacity();
                break;
            }
        }
    }

    // Phase 4 (diagnostic only): resume the consumer and count events.
    let mut new_tx_events = 0usize;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(2), a.next_event()).await {
        if matches!(ev, NetworkEvent::NewTransaction(..)) {
            new_tx_events += 1;
        }
    }
    println!(
        "INC-I-237 deadlock probe: flood_txs={FLOOD_TXS} event_buf={EVENT_BUF} \
         command_buf={COMMAND_BUF} commands_sent_ok={sent} stalled_at={stalled_at:?} \
         cmd_capacity_at_stall={cap_at_stall} new_tx_events_drained_after={new_tx_events}"
    );
    f_drain.abort();

    // Non-vacuity: the flood reached A's node-side event channel.
    assert!(new_tx_events > 0, "flood never reached A (vacuous)");

    // Liveness: the node loop's command send must not hang while it is the
    // stalled event consumer (that is the mutual deadlock).
    assert!(
        stalled_at.is_none(),
        "INC-I-237 DEADLOCK: request_sync #{} blocked > {SEND_TIMEOUT:?}; command \
         channel free capacity={cap_at_stall} (of {COMMAND_BUF}); swarm loop parked on \
         event_tx.send (behaviour_events.rs:290) (event channel buf {EVENT_BUF}; \
         {new_tx_events} NewTransaction events drained after consumer resumed)",
        stalled_at.unwrap()
    );
}

async fn warm_up_tx_mesh(
    a: &mut NetworkService,
    f_cmd: &tokio::sync::mpsc::Sender<network::NetworkCommand>,
    base: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut warm = 0u64;
    loop {
        assert!(
            Instant::now() < deadline,
            "A never received warm-up tx gossip"
        );
        for _ in 0..50 {
            f_cmd
                .send(network::NetworkCommand::BroadcastTransaction(unique_tx(
                    base + warm,
                )))
                .await
                .unwrap();
            warm += 1;
        }
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            if let Ok(Some(NetworkEvent::NewTransaction(..))) =
                tokio::time::timeout(Duration::from_millis(100), a.next_event()).await
            {
                while a.try_next_event().is_some() {}
                return;
            }
        }
    }
}

async fn flood(f_cmd: &tokio::sync::mpsc::Sender<network::NetworkCommand>, base: u64) {
    for i in 0..FLOOD_TXS {
        f_cmd
            .send(network::NetworkCommand::BroadcastTransaction(unique_tx(
                base + i,
            )))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(8)).await;
}

/// ~100 unique tx messages/s from every flooder for PACED_FLOOD.
async fn paced_flood(cmds: &[tokio::sync::mpsc::Sender<network::NetworkCommand>], base: u64) {
    let until = Instant::now() + PACED_FLOOD;
    let mut i = 0u64;
    while Instant::now() < until {
        for cmd in cmds {
            cmd.send(network::NetworkCommand::BroadcastTransaction(unique_tx(
                base + i,
            )))
            .await
            .unwrap();
            i += 1;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
}

// REQ-GRV-006 / F-6 — Decision: a failure means the sync-only sink loses control events or stops the swarm while the node is behind: a peer joining during a node stall is never reported, or the swarm stops serving connections and commands.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_006_swarm_serves_while_node_stopped_and_backlog_delivered_after_resume() {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let a_port = reserve_port();
    let mut a = start_node(a_port, Vec::new()).await;
    let a_id: PeerId = *a.local_peer_id();
    let mut f = start_node(reserve_port(), vec![format!("/ip4/127.0.0.1/tcp/{a_port}")]).await;
    let f_id: PeerId = *f.local_peer_id();
    let f_cmd = f.command_sender();
    let f_drain = tokio::spawn(async move { while f.next_event().await.is_some() {} });
    let mut cmds = vec![f_cmd.clone()];
    let mut drains = Vec::new();
    for _ in 0..EXTRA_FLOODERS {
        let mut x = start_node(reserve_port(), vec![format!("/ip4/127.0.0.1/tcp/{a_port}")]).await;
        cmds.push(x.command_sender());
        drains.push(tokio::spawn(async move {
            while x.next_event().await.is_some() {}
        }));
    }

    warm_up_tx_mesh(&mut a, &f_cmd, 20_000_000).await;
    paced_flood(&cmds, 21_000_000).await;

    let mut g = start_node(reserve_port(), vec![format!("/ip4/127.0.0.1/tcp/{a_port}")]).await;
    let g_id: PeerId = *g.local_peer_id();
    let (g_tx, g_rx) = tokio::sync::oneshot::channel::<()>();
    let g_drain = tokio::spawn(async move {
        let mut g_tx = Some(g_tx);
        while let Some(ev) = g.next_event().await {
            if matches!(ev, NetworkEvent::PeerConnected(p) if p == a_id) {
                if let Some(tx) = g_tx.take() {
                    let _ = tx.send(());
                }
            }
        }
    });
    assert!(
        tokio::time::timeout(Duration::from_secs(20), g_rx)
            .await
            .is_ok(),
        "A's swarm did not serve a new peer while A's node consumer was stopped"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while a.peer_count().await < 2 + EXTRA_FLOODERS {
        assert!(
            Instant::now() < deadline,
            "A's swarm never recorded G in its peer table"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for i in 0..64u64 {
        let req = SyncRequest::get_block_by_hash(crypto::hash::hash(&(i + 1_000).to_le_bytes()));
        let r = tokio::time::timeout(SEND_TIMEOUT, a.request_sync(f_id, req)).await;
        assert!(
            r.is_ok(),
            "command #{i} not served while A's node consumer was stopped"
        );
    }

    let mut gossip_before = 0usize;
    let mut delivered = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), a.next_event()).await {
            Ok(Some(NetworkEvent::PeerConnected(p))) if p == g_id => {
                delivered = true;
                break;
            }
            Ok(Some(NetworkEvent::NewTransaction(..))) => gossip_before += 1,
            Ok(Some(_)) => {}
            Ok(None) => panic!("A's event channel closed"),
            Err(_) => break,
        }
    }
    let queued_behind = std::iter::from_fn(|| a.try_next_event())
        .filter(|ev| matches!(ev, NetworkEvent::NewTransaction(..)))
        .count();
    f_drain.abort();
    g_drain.abort();
    drains.iter().for_each(|d| d.abort());
    assert!(
        delivered,
        "the PeerConnected(G) control event backlogged during the stall was never delivered after resume"
    );
    let gossip_queued = gossip_before + queued_behind;
    assert!(
        gossip_queued >= EVENT_BUF / 2,
        "non-vacuity: the stalled node must have held a full bulk lane ({gossip_queued} gossip events queued; R1 delivers PeerConnected(G) ahead of them)"
    );
}

// F-11 / REQ-GRV-006 — Decision: a failure means shutting the node down while its event channel is full and its consumer stopped leaves the swarm running (it waits on the swarm/node channel or never notices), so shutdown hangs or leaks the swarm and its connections.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_11_dropping_the_node_handle_under_full_channel_ends_the_swarm_within_5s() {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let a_port = reserve_port();
    let mut a = start_node(a_port, Vec::new()).await;
    let a_id: PeerId = *a.local_peer_id();
    let mut f = start_node(reserve_port(), vec![format!("/ip4/127.0.0.1/tcp/{a_port}")]).await;
    let f_cmd = f.command_sender();
    let (gone_tx, mut gone_rx) = tokio::sync::mpsc::unbounded_channel::<Instant>();
    let f_watch = tokio::spawn(async move {
        while let Some(ev) = f.next_event().await {
            if matches!(ev, NetworkEvent::PeerDisconnected(p) if p == a_id) {
                let _ = gone_tx.send(Instant::now());
            }
        }
    });

    warm_up_tx_mesh(&mut a, &f_cmd, 30_000_000).await;
    flood(&f_cmd, 31_000_000).await;
    assert!(
        a.peer_count().await >= 1,
        "non-vacuity: F must be connected at drop time"
    );

    let dropped_at = Instant::now();
    drop(a);
    let deadline = dropped_at + Duration::from_secs(5);
    let mut ended = false;
    while let Ok(Some(at)) = tokio::time::timeout_at(deadline.into(), gone_rx.recv()).await {
        if at >= dropped_at {
            ended = true;
            break;
        }
    }
    f_watch.abort();
    assert!(
        ended,
        "F-11: A's swarm still running 5 s after its node handle was dropped (full event channel, stopped consumer)"
    );
}
