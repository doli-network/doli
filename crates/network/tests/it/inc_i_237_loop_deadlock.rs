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
//! PATHS: P1 swarm loop not blocked -> commands drained, O1 holds;
//!        P2 swarm loop parked on event_tx.send -> commands pile up, O1 fails.
//! INPUT PARTITIONS: IP-1 unique tx flood > event_buf, consumer stalled.

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
/// Unique txs flooded by F (> EVENT_BUF, sent in 50-tx gossip batches).
const FLOOD_TXS: u64 = 12_000;
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
