//! INC-I-237 M0 — the live swarm stamps gossip at receive (C2) and counts tx
//! batch sizes (C4). Two real `NetworkService`s on 127.0.0.1: A -> B.
//!
//! OUTPUT CONTRACT: swarm `gossipsub::Event::Message` arm (behaviour_events.rs)
//!   O1: none   O2: none   O3: none
//!   O4: none
//!   O5: TX_GOSSIP_BATCH_STATS (process-global: asserted as lower-bound deltas,
//!       other modules in this binary run tx floods in parallel)
//!   O6: NetworkEvent stream at B — gossip events carry Some(handle) with
//!       publish_at <= recv_at <= received_at; one handle per message (a 0xBA
//!       batch of 3 -> 3 events with the same recv_at); control events None
//! PATHS: P1 legacy-mode 0xBA batch of 1 (warm-up), P2 0xBA batch of 3,
//!   P3 inbound StatusRequest / SyncRequest / TxFetchRequest (ResponseChannel
//!   variants T1 cannot build), P4 PeerConnected
//! INPUT PARTITIONS: batch size 1 (bucket "1"), batch size 3 (bucket "2_5")
//! MATRIX: O5 x {P2}, O6 x {P1, P2, P3, P4} — all asserted.

use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Transaction;
use network::gossip::tx_batch_stats::TX_GOSSIP_BATCH_STATS;
use network::protocols::{StatusRequest, SyncRequest};
use network::{NetworkCommand, NetworkConfig, NetworkEvent, NetworkService};

const WARM_BASE: u64 = 37_000_000;
const TRIO_BASE: u64 = 37_500_000;

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("reserve port")
        .port()
}

fn config(port: u16, bootstrap_nodes: Vec<String>) -> NetworkConfig {
    let listen_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    NetworkConfig {
        listen_addr,
        bootstrap_nodes,
        max_peers: 8,
        bootstrap_slots: 8,
        no_dht: true,
        enable_discv5: false,
        node_key_path: None,
        peer_cache_path: None,
        ..NetworkConfig::default()
    }
}

fn tx(tag: u64) -> Transaction {
    Transaction::new_coinbase(tag, Hash::default(), tag, tag as u32)
}

fn tag(t: &Transaction) -> u64 {
    t.outputs.first().map(|o| o.amount).unwrap_or(0)
}

/// Exhaustive (no wildcard): every variant shape of C2 is named here.
fn kind(ev: &NetworkEvent) -> &'static str {
    match ev {
        NetworkEvent::PeerConnected(_) => "PeerConnected",
        NetworkEvent::PeerDisconnected(_) => "PeerDisconnected",
        NetworkEvent::NewBlock(_, _, _) => "NewBlock",
        NetworkEvent::NewHeader(_, _) => "NewHeader",
        NetworkEvent::NewTransaction(_, _) => "NewTransaction",
        NetworkEvent::StatusRequest { .. } => "StatusRequest",
        NetworkEvent::SyncRequest { .. } => "SyncRequest",
        NetworkEvent::SyncResponse { .. } => "SyncResponse",
        NetworkEvent::PeerStatus { .. } => "PeerStatus",
        NetworkEvent::NetworkMismatch { .. } => "NetworkMismatch",
        NetworkEvent::GenesisMismatch { .. } => "GenesisMismatch",
        NetworkEvent::VersionMismatch { .. } => "VersionMismatch",
        NetworkEvent::ProducersAnnounced(_, _) => "ProducersAnnounced",
        NetworkEvent::ProducerAnnouncementsReceived(_, _) => "ProducerAnnouncementsReceived",
        NetworkEvent::ProducerDigestReceived { .. } => "ProducerDigestReceived",
        NetworkEvent::NewVote(_, _) => "NewVote",
        NetworkEvent::NewHeartbeat(_, _) => "NewHeartbeat",
        NetworkEvent::NewAttestation(_, _, _) => "NewAttestation",
        NetworkEvent::TxAnnouncement { .. } => "TxAnnouncement",
        NetworkEvent::TxFetchRequest { .. } => "TxFetchRequest",
        NetworkEvent::TxFetchResponse { .. } => "TxFetchResponse",
    }
}

/// Every event B saw: kind, whether it had a topic, whether it had a handle.
struct Seen {
    log: Vec<(&'static str, bool, bool)>,
}

impl Seen {
    fn note(&mut self, ev: &NetworkEvent, received_at: Instant, start: Instant) {
        if let Some(h) = ev.gossip_handle() {
            assert!(
                start <= h.recv_at && h.recv_at <= received_at,
                "{}: recv_at outside [test start, event delivery]",
                kind(ev)
            );
        }
        self.log.push((
            kind(ev),
            ev.gossip_topic().is_some(),
            ev.gossip_handle().is_some(),
        ));
    }
}

// REQ-GRV-008 — Decision: a failure means the swarm stamps per tx instead of per message, stamps after decode/queueing, or skips the batch counter, so verdict latency and the batch-size baseline M0 gates on are wrong.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_008_swarm_stamps_one_handle_per_message_and_counts_the_batch() {
    std::env::set_var("DOLI_IP_COLOCATION_THRESHOLD", "500");
    let start = Instant::now();
    let b_port = reserve_port();
    let b_cfg = config(b_port, Vec::new());
    let (net_id, genesis, fork) = (b_cfg.network_id, b_cfg.genesis_hash, b_cfg.fork_id);
    let mut b = NetworkService::new(b_cfg).await.expect("start B");
    let b_id = *b.local_peer_id();
    let mut a = NetworkService::new(config(
        reserve_port(),
        vec![format!("/ip4/127.0.0.1/tcp/{b_port}")],
    ))
    .await
    .expect("start A");
    let a_cmd = a.command_sender();
    tokio::spawn(async move { while a.next_event().await.is_some() {} });
    let mut seen = Seen { log: Vec::new() };

    // P1: warm the A -> B tx mesh.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut n = 0;
    'warm: loop {
        assert!(Instant::now() < deadline, "B never received A's tx");
        a_cmd
            .send(NetworkCommand::BroadcastTransaction(tx(WARM_BASE + n)))
            .await
            .unwrap();
        n += 1;
        let until = Instant::now() + Duration::from_millis(400);
        while Instant::now() < until {
            if let Ok(Some(ev)) =
                tokio::time::timeout(Duration::from_millis(100), b.next_event()).await
            {
                seen.note(&ev, Instant::now(), start);
                if matches!(ev, NetworkEvent::NewTransaction(..)) {
                    break 'warm;
                }
            }
        }
    }

    // P2: three txs in one 100 ms flush -> one 0xBA batch of 3 at B.
    let mut proof = None;
    let mut attempts = Vec::new();
    for attempt in 0..5u64 {
        let base = TRIO_BASE + attempt * 10;
        let before = TX_GOSSIP_BATCH_STATS.snapshot();
        let publish_at = Instant::now();
        for k in 0..3 {
            a_cmd
                .send(NetworkCommand::BroadcastTransaction(tx(base + k)))
                .await
                .unwrap();
        }
        let mut trio = Vec::new();
        let until = Instant::now() + Duration::from_secs(5);
        while trio.len() < 3 && Instant::now() < until {
            if let Ok(Some(ev)) =
                tokio::time::timeout(Duration::from_millis(100), b.next_event()).await
            {
                let received_at = Instant::now();
                seen.note(&ev, received_at, start);
                if let NetworkEvent::NewTransaction(t, h) = &ev {
                    if (base..base + 3).contains(&tag(t)) {
                        trio.push((h.recv_at, received_at));
                    }
                }
            }
        }
        let after = TX_GOSSIP_BATCH_STATS.snapshot();
        let one_message = trio.len() == 3 && trio.iter().all(|(r, _)| *r == trio[0].0);
        attempts.push((trio.len(), one_message));
        if one_message {
            proof = Some((publish_at, trio, before, after));
            break;
        }
    }
    let (publish_at, trio, before, after) = proof.unwrap_or_else(|| {
        panic!("no 3-tx batch arrived as 3 events sharing one recv_at: {attempts:?}")
    });
    let recv_at = trio[0].0;
    assert!(publish_at <= recv_at, "recv_at precedes the publish");
    assert!(
        trio.iter().all(|(_, got)| recv_at <= *got),
        "recv_at after delivery"
    );
    assert!(
        after.msgs - before.msgs >= 1,
        "C4: the batch message was not counted"
    );
    assert!(
        after.buckets[1] - before.buckets[1] >= 1,
        "C4: a 3-tx batch must land in bucket 2_5"
    );
    assert!(
        after.txs - before.txs >= 3,
        "C4: the 3 txs were not counted"
    );

    // P3: the three ResponseChannel variants T1 cannot build.
    let request = StatusRequest::new(net_id, genesis, fork);
    a_cmd
        .send(NetworkCommand::RequestStatus {
            peer_id: b_id,
            request,
        })
        .await
        .unwrap();
    let request = SyncRequest::get_block_by_height(1);
    a_cmd
        .send(NetworkCommand::RequestSync {
            peer_id: b_id,
            request,
        })
        .await
        .unwrap();
    let hashes = vec![Hash::default()];
    a_cmd
        .send(NetworkCommand::RequestTxFetch {
            peer_id: b_id,
            hashes,
        })
        .await
        .unwrap();
    let want = [
        "StatusRequest",
        "SyncRequest",
        "TxFetchRequest",
        "PeerConnected",
    ];
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until && !want.iter().all(|w| seen.log.iter().any(|(k, _, _)| k == w)) {
        if let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(100), b.next_event()).await
        {
            seen.note(&ev, Instant::now(), start);
        }
    }
    for w in want {
        let rows: Vec<_> = seen.log.iter().filter(|(k, _, _)| *k == w).collect();
        assert!(!rows.is_empty(), "{w} never reached B (non-vacuity)");
        assert!(
            rows.iter().all(|(_, t, h)| !t && !h),
            "{w} must carry no topic and no handle"
        );
    }
    for (k, has_topic, has_handle) in &seen.log {
        assert_eq!(
            has_topic, has_handle,
            "{k}: gossip_topic and gossip_handle disagree"
        );
    }
}
