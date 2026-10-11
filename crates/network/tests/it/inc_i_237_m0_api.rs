//! INC-I-237 M0 — receive stamp, topic mapping and tx batch stats (C1-C4).
//!
//! OUTPUT CONTRACT: fn GossipHandle::now() / NetworkEvent::gossip_topic(&self) /
//!   NetworkEvent::gossip_handle(&self) / GossipTopic::metric_label(self) /
//!   bucket_index(n) /
//!   TxGossipBatchStats::{new, record(&self, n), snapshot(&self)}
//!   O1: none (no &mut params)
//!   O2: TxGossipBatchStats atomics (observed via snapshot)
//!   O3: recv_at; Option<GossipTopic>; Option<&GossipHandle>; label; bool; index;
//!       TxBatchSnapshot
//!   O4: none   O5: TX_BATCH_BUCKETS const   O6: none
//! PATHS: P1 gossip variant, P2 control variant, P3 each GossipTopic,
//!   P4 record(n>0), P5 record(0) no-op, P6 concurrent record
//! INPUT PARTITIONS: 8 gossip variants (M1: NewHeader/NewHeartbeat deleted);
//!   8 constructible control variants (the 3 ResponseChannel variants are driven
//!   live in inc_i_237_m0_swarm); 5 topics (M1: the subscribed set);
//!   bucket boundaries 1,2,5,6,10,11,25,26,50,51,100,101 + 10_000 + usize::MAX
//! MATRIX: every variant x {topic, handle}; every topic x label;
//!   every boundary x index; record paths x {msgs, txs, buckets}.

use std::time::{Duration, Instant};

use super::inc_i_237_m0_corpus_vectors as v;
use crypto::{Hash, PublicKey};
use doli_core::{Block, BlockHeader, Transaction};
use libp2p::PeerId;
use network::gossip::staleness::GossipTopic;
use network::gossip::tx_batch_stats::{
    bucket_index, TxBatchSnapshot, TxGossipBatchStats, TX_BATCH_BUCKETS, TX_GOSSIP_BATCH_STATS,
};
use network::protocols::{StatusResponse, SyncResponse};
use network::{GossipHandle, NetworkEvent};

const ALL_TOPICS: [GossipTopic; 5] = [
    GossipTopic::Blocks,
    GossipTopic::Producers,
    GossipTopic::Attestations,
    GossipTopic::Votes,
    GossipTopic::Transactions,
];

/// No wildcard: a new GossipTopic variant must be added to this test.
fn expected_label(t: GossipTopic) -> &'static str {
    match t {
        GossipTopic::Blocks => "blocks",
        GossipTopic::Producers => "producers",
        GossipTopic::Attestations => "attestations",
        GossipTopic::Votes => "votes",
        GossipTopic::Transactions => "txs",
    }
}

fn header() -> BlockHeader {
    BlockHeader::deserialize(&v::bytes(v::HEADER)).unwrap()
}

fn tx() -> Transaction {
    Transaction::deserialize(&v::bytes(v::TX_LEGACY_TRANSFER)).unwrap()
}

fn pk() -> PublicKey {
    PublicKey::from_bytes(v::bytes(v::PK9).try_into().unwrap())
}

/// Every gossip variant with the topic C3 maps it to.
fn gossip_events(h: &GossipHandle) -> Vec<(&'static str, NetworkEvent, GossipTopic)> {
    let p = PeerId::random();
    let anns = doli_core::decode_producer_set(&v::bytes(v::PRODUCERS_PROTOBUF_SET)).unwrap();
    let digest = doli_core::decode_digest(&v::bytes(v::PRODUCERS_BLOOM_DIGEST)).unwrap();
    vec![
        (
            "NewBlock",
            NetworkEvent::NewBlock(Block::new(header(), vec![]), p, h.clone()),
            GossipTopic::Blocks,
        ),
        (
            "NewTransaction",
            NetworkEvent::NewTransaction(vec![tx()], h.clone()),
            GossipTopic::Transactions,
        ),
        (
            "TxAnnouncement",
            NetworkEvent::TxAnnouncement {
                peer_id: p,
                hashes: vec![tx().hash()],
                handle: h.clone(),
            },
            GossipTopic::Transactions,
        ),
        (
            "NewVote",
            NetworkEvent::NewVote(v::VOTE_RPC.to_vec(), h.clone()),
            GossipTopic::Votes,
        ),
        (
            "NewAttestation",
            NetworkEvent::NewAttestation(v::bytes(v::ATTESTATION_BLS), p, h.clone()),
            GossipTopic::Attestations,
        ),
        (
            "ProducersAnnounced",
            NetworkEvent::ProducersAnnounced(vec![pk()], h.clone()),
            GossipTopic::Producers,
        ),
        (
            "ProducerAnnouncementsReceived",
            NetworkEvent::ProducerAnnouncementsReceived(anns, h.clone()),
            GossipTopic::Producers,
        ),
        (
            "ProducerDigestReceived",
            NetworkEvent::ProducerDigestReceived {
                peer_id: p,
                digest,
                handle: h.clone(),
            },
            GossipTopic::Producers,
        ),
    ]
}

/// Every control variant that can be built without a live ResponseChannel.
fn control_events() -> Vec<(&'static str, NetworkEvent)> {
    let p = PeerId::random();
    let status = StatusResponse {
        version: 1,
        network_id: 1,
        genesis_hash: Hash::ZERO,
        best_height: 0,
        best_hash: Hash::ZERO,
        best_slot: 0,
        producer_pubkey: None,
        fork_id: Hash::ZERO,
    };
    vec![
        ("PeerConnected", NetworkEvent::PeerConnected(p)),
        ("PeerDisconnected", NetworkEvent::PeerDisconnected(p)),
        (
            "SyncResponse",
            NetworkEvent::SyncResponse {
                peer_id: p,
                response: SyncResponse::Headers(vec![]),
            },
        ),
        (
            "PeerStatus",
            NetworkEvent::PeerStatus { peer_id: p, status },
        ),
        (
            "NetworkMismatch",
            NetworkEvent::NetworkMismatch {
                peer_id: p,
                our_network_id: 1,
                their_network_id: 2,
            },
        ),
        (
            "GenesisMismatch",
            NetworkEvent::GenesisMismatch { peer_id: p },
        ),
        (
            "VersionMismatch",
            NetworkEvent::VersionMismatch {
                peer_id: p,
                our_version: 2,
                their_version: 1,
            },
        ),
        (
            "TxFetchResponse",
            NetworkEvent::TxFetchResponse {
                peer_id: p,
                transactions: vec![tx()],
            },
        ),
    ]
}

// REQ-GRV-008 — Decision: a failure means the receive stamp is not taken at call time, so every verdict-latency sample (the M0 SLO gate) measures the wrong interval.
#[test]
fn req_grv_008_gossip_handle_now_stamps_the_call_instant() {
    let before = Instant::now();
    let h = GossipHandle::now();
    let after = Instant::now();
    assert!(
        before <= h.recv_at && h.recv_at <= after,
        "recv_at outside the call window"
    );
    std::thread::sleep(Duration::from_millis(5));
    let c = h.clone();
    assert_eq!(
        c.recv_at, h.recv_at,
        "a clone must keep the original receive instant"
    );
    assert!(
        GossipHandle::now().recv_at > h.recv_at,
        "a later stamp must be later"
    );
    let via_service: network::service::GossipHandle = GossipHandle::now();
    assert!(
        via_service.recv_at >= after,
        "network::service re-export is the same type"
    );
}

// REQ-GRV-008 — Decision: a failure means a gossip event is counted under the wrong topic or loses its stamp, so the per-topic histogram the M0 gate reads is wrong for that topic.
#[test]
fn req_grv_008_every_gossip_variant_maps_to_its_topic_and_keeps_its_handle() {
    let h = GossipHandle::now();
    let events = gossip_events(&h);
    assert_eq!(events.len(), 8, "C2 lists eight gossip variants (M1)");
    for (name, ev, topic) in &events {
        assert_eq!(ev.gossip_topic(), Some(*topic), "{name}: wrong topic");
        let got = ev
            .gossip_handle()
            .unwrap_or_else(|| panic!("{name}: no handle"));
        assert_eq!(
            got.recv_at, h.recv_at,
            "{name}: handle is not the one it was built with"
        );
    }
}

// REQ-GRV-008 — Decision: a failure means a control event (status, sync, peer churn) is counted as gossip, so sync traffic pollutes the verdict-latency series.
#[test]
fn req_grv_008_control_variants_have_no_topic_and_no_handle() {
    let events = control_events();
    assert_eq!(events.len(), 8);
    for (name, ev) in &events {
        assert_eq!(
            ev.gossip_topic(),
            None,
            "{name}: control event mapped to a topic"
        );
        assert!(
            ev.gossip_handle().is_none(),
            "{name}: control event carries a handle"
        );
    }
}

// REQ-GRV-008 / REQ-GRV-015 — Decision: a failure means a Prometheus label changes, two topics share one, or a headers/heartbeats/tier1 label survives D3, so dashboards and the probe script read the wrong series.
#[test]
fn req_grv_008_metric_label_for_the_five_subscribed_topics() {
    let mut labels = Vec::new();
    for t in ALL_TOPICS {
        assert_eq!(t.metric_label(), expected_label(t), "{t:?}: label");
        labels.push(t.metric_label());
    }
    labels.sort();
    labels.dedup();
    assert_eq!(labels.len(), 5, "labels must be distinct");
    for gone in ["headers", "heartbeats", "tier1_blocks"] {
        assert!(!labels.contains(&gone), "{gone} label must be gone (D3)");
    }
}

// REQ-GRV-008 — Decision: a failure means a batch lands in the wrong size bucket, so the batch-size distribution (R4 re-batching condition) is misread.
#[test]
fn req_grv_008_bucket_index_at_every_boundary() {
    let cases: [(usize, usize); 14] = [
        (1, 0),
        (2, 1),
        (5, 1),
        (6, 2),
        (10, 2),
        (11, 3),
        (25, 3),
        (26, 4),
        (50, 4),
        (51, 5),
        (100, 5),
        (101, 6),
        (10_000, 6),
        (usize::MAX, 6),
    ];
    for (n, want) in cases {
        assert_eq!(bucket_index(n), want, "bucket_index({n})");
    }
    assert_eq!(
        TX_BATCH_BUCKETS,
        ["1", "2_5", "6_10", "11_25", "26_50", "51_100", "101_plus"]
    );
}

// REQ-GRV-008 — Decision: a failure means msgs, txs or a bucket miscounts, so the origin-rate and batch-size baseline M0 must record is wrong.
#[test]
fn req_grv_008_record_and_snapshot_on_a_local_stats() {
    let s = TxGossipBatchStats::new();
    assert_eq!(
        s.snapshot(),
        TxBatchSnapshot::default(),
        "a new stats is all zero"
    );
    for n in [1usize, 3, 3, 50, 101] {
        s.record(n);
    }
    let snap = s.snapshot();
    assert_eq!(snap.msgs, 5);
    assert_eq!(snap.txs, 158);
    assert_eq!(snap.buckets, [1, 2, 0, 0, 1, 0, 1]);
    assert_eq!(
        snap.buckets.iter().sum::<u64>(),
        snap.msgs,
        "every msg lands in one bucket"
    );
    assert_eq!(s.snapshot(), snap, "snapshot does not reset the counters");
}

// REQ-GRV-008 — Decision: a failure means an empty decode counts as a message, so msgs and the "1" bucket are inflated by undecodable batches.
#[test]
fn req_grv_008_record_zero_is_a_no_op() {
    let s = TxGossipBatchStats::new();
    s.record(0);
    assert_eq!(s.snapshot(), TxBatchSnapshot::default());
    s.record(2);
    s.record(0);
    let snap = s.snapshot();
    assert_eq!((snap.msgs, snap.txs, snap.buckets[1]), (1, 2, 1));
}

// REQ-GRV-008 — Decision: a failure means concurrent records lose updates, so the global static under-counts on a busy swarm.
#[test]
fn req_grv_008_concurrent_records_are_not_lost() {
    static S: TxGossipBatchStats = TxGossipBatchStats::new();
    let threads: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(|| (0..1_000).for_each(|_| S.record(2))))
        .collect();
    threads.into_iter().for_each(|t| t.join().unwrap());
    let snap = S.snapshot();
    assert_eq!(
        (snap.msgs, snap.txs, snap.buckets[1]),
        (8_000, 16_000, 8_000)
    );
    let _global: TxBatchSnapshot = TX_GOSSIP_BATCH_STATS.snapshot();
}
