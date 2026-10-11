//! INC-I-237 M1.5 (R1) — two event lanes: classification, size estimate,
//! consensus-first delivery under a full bulk lane (F-7), close (F-11).
//!
//! OUTPUT CONTRACT: fn lane_of(&NetworkEvent) -> Lane / event_bytes(&NetworkEvent) -> usize /
//!   EventSink::{emit, emit_block}(..) -> EmitOutcome / EventLanes::{recv, try_recv}
//!   O1: none   O2: lane queues (observed through recv/try_recv order)
//!   O3: Lane / usize / EmitOutcome / Option<NetworkEvent>   O4: none
//!   O5: LaneStats via stats().snapshot() {bulk_shed, consensus_shed, bulk_bytes};
//!       GossipShedMetrics::blocks_dropped
//!   O6: events delivered to the node (count + order)
//! PATHS: P1 lane_of per variant (19); P2 event_bytes payload vs control; P3 bulk
//!   full by count -> Shed; P4 bulk full by bytes -> Shed; P5 consensus emit while
//!   bulk full; P6 try_recv / recv order; P7 sink dropped -> recv None after drain
//! INPUT PARTITIONS: variants {16 constructible at runtime, 3 ResponseChannel-only
//!   by source scan}; SyncResponse payload {8 B, 1 MiB}; bulk limit {count, bytes};
//!   receive {sync try_recv, async recv}; close {lanes non-empty, then empty}
//! MATRIX: O3 x P1-P7, O5 x P3-P5, O6 x P6-P7 — all asserted below.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use libp2p::PeerId;

use super::inc_i_237_m15_fixtures::{
    att_event, block_event, constructible_events, roomy, sync_resp, tx_event, tx_tag, variant,
};
use crate::service::backpressure::GossipShedMetrics;
use crate::service::event_lanes::EventLanes;
use crate::service::event_sink::{
    event_bytes, event_lanes, lane_of, EmitOutcome, Lane, LaneBudgets,
};
use crate::service::types::NetworkEvent;

const CONSENSUS: [&str; 11] = [
    "NewBlock",
    "NewAttestation",
    "PeerConnected",
    "PeerDisconnected",
    "PeerStatus",
    "NetworkMismatch",
    "GenesisMismatch",
    "VersionMismatch",
    "StatusRequest",
    "SyncRequest",
    "SyncResponse",
];

const BULK: [&str; 8] = [
    "NewTransaction",
    "TxAnnouncement",
    "TxFetchRequest",
    "TxFetchResponse",
    "NewVote",
    "ProducersAnnounced",
    "ProducerAnnouncementsReceived",
    "ProducerDigestReceived",
];

const CHANNEL_ONLY: [&str; 3] = ["StatusRequest", "SyncRequest", "TxFetchRequest"];

const ALL_EMITTED: [EmitOutcome; 3] = [
    EmitOutcome::Emitted,
    EmitOutcome::Emitted,
    EmitOutcome::Emitted,
];

fn contract_lane(name: &str) -> Lane {
    if CONSENSUS.contains(&name) {
        Lane::Consensus
    } else if BULK.contains(&name) {
        Lane::Bulk
    } else {
        panic!("{name} is in neither contract lane")
    }
}

/// `lane_of` body: each `NetworkEvent::<V>` maps to the next `Lane::<L>` token.
fn lane_of_source_map() -> BTreeMap<String, String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/service/event_sink.rs");
    let all = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let code: String = all
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let start = code
        .find("fn lane_of(")
        .expect("event_sink.rs has no fn lane_of");
    let open = start + code[start..].find('{').expect("lane_of body");
    let mut depth = 0usize;
    let mut end = open;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let body = &code[open..=end];
    assert!(
        !body.contains("_ =>"),
        "lane_of has a wildcard arm:\n{body}"
    );
    let ident = |s: &str| -> String {
        s.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect()
    };
    let (mut rest, mut pending, mut out) = (body, Vec::new(), BTreeMap::new());
    loop {
        let ev = rest.find("NetworkEvent::");
        let ln = rest.find("Lane::");
        match (ev, ln) {
            (Some(a), b) if b.is_none_or(|b| a < b) => {
                let name = ident(&rest[a + "NetworkEvent::".len()..]);
                rest = &rest[a + "NetworkEvent::".len() + name.len()..];
                pending.push(name);
            }
            (_, Some(b)) => {
                let lane = ident(&rest[b + "Lane::".len()..]);
                rest = &rest[b + "Lane::".len() + lane.len()..];
                for name in pending.drain(..) {
                    let prev = out.insert(name.clone(), lane.clone());
                    assert!(prev.is_none(), "{name} classified twice");
                }
            }
            (None, None) => break,
            (Some(_), None) => unreachable!(),
        }
    }
    assert!(pending.is_empty(), "variants with no lane: {pending:?}");
    out
}

// REQ-GRV-017 / C3 — Decision: a failure means a constructible event lands in the wrong lane, so a tx flood can again starve blocks, attestations or control (or bulk gossip jumps the consensus queue).
#[test]
fn req_grv_017_lane_of_classifies_every_constructible_variant_per_contract() {
    let mut seen = BTreeSet::new();
    for ev in constructible_events() {
        let name = variant(&ev);
        assert_eq!(lane_of(&ev), contract_lane(name), "lane_of({name})");
        assert!(seen.insert(name), "{name} listed twice");
    }
    let all: BTreeSet<&str> = CONSENSUS.iter().chain(BULK.iter()).copied().collect();
    let channel_only: BTreeSet<&str> = CHANNEL_ONLY.iter().copied().collect();
    assert_eq!(all.len(), 19, "contract lanes overlap");
    assert_eq!(
        seen,
        all.difference(&channel_only).copied().collect(),
        "runtime cases must cover every variant except the 3 ResponseChannel-only ones"
    );
}

// REQ-GRV-017 / C3 — Decision: a failure means lane_of misroutes a ResponseChannel variant (StatusRequest, SyncRequest, TxFetchRequest) or hides a variant behind a wildcard arm, so a new event silently picks a lane.
#[test]
fn req_grv_017_lane_of_source_routes_all_19_variants_without_a_wildcard() {
    let map = lane_of_source_map();
    for name in CONSENSUS.iter().chain(BULK.iter()) {
        let want = match contract_lane(name) {
            Lane::Consensus => "Consensus",
            Lane::Bulk => "Bulk",
        };
        assert_eq!(
            map.get(*name).map(String::as_str),
            Some(want),
            "lane_of source arm for {name}"
        );
    }
    assert_eq!(map.len(), 19, "lane_of names an unknown variant: {map:?}");
}

// REQ-GRV-007 — Decision: a failure means a SyncResponse payload or an attestation payload is not charged to its lane, so the byte bound cannot hold a 16 MiB response (or an attestation flood), or a control event is charged so much that one evicts real work.
#[test]
fn req_grv_007_event_bytes_charges_payloads_and_keeps_control_small() {
    const N: usize = 1 << 20;
    assert!(event_bytes(&sync_resp(1, N)) >= N, "SyncResponse payload");
    assert!(
        event_bytes(&sync_resp(1, N)) > event_bytes(&sync_resp(1, 8)),
        "size must track the payload"
    );
    assert!(event_bytes(&att_event(4096)) >= 4096, "attestation payload");
    assert!(event_bytes(&tx_event(1)) > 0, "a tx must cost bulk bytes");
    assert!(
        event_bytes(&block_event()) > 0,
        "a block must cost consensus bytes"
    );
    assert!(event_bytes(&NetworkEvent::PeerConnected(PeerId::random())) < 1024);
}

fn emit_consensus_trio(
    sink: &crate::service::event_sink::EventSink,
    m: &GossipShedMetrics,
) -> [EmitOutcome; 3] {
    [
        sink.emit_block(block_event(), m),
        sink.emit(att_event(64)),
        sink.emit(NetworkEvent::PeerConnected(PeerId::random())),
    ]
}

fn assert_consensus_first(got: &[&str], bulk: usize) {
    assert_eq!(
        &got[..3],
        ["NewBlock", "NewAttestation", "PeerConnected"],
        "consensus events first and in emit order: {got:?}"
    );
    assert_eq!(got.len(), 3 + bulk, "{got:?}");
    assert!(got[3..].iter().all(|v| *v == "NewTransaction"), "{got:?}");
}

// REQ-GRV-017 / F-7 — Decision: a failure means a bulk lane full by COUNT sheds or delays a block, an attestation or a control event, or the node reads txs before consensus events (the M0 starvation shape).
#[test]
fn req_grv_017_full_bulk_lane_by_count_never_sheds_or_delays_consensus() {
    let b = LaneBudgets {
        bulk_cap: 4,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    let m = GossipShedMetrics::new();
    for tag in 1..=4 {
        assert_eq!(sink.emit(tx_event(tag)), EmitOutcome::Emitted, "bulk {tag}");
    }
    assert_eq!(
        sink.emit(tx_event(90)),
        EmitOutcome::Shed,
        "bulk at count cap"
    );
    assert_eq!(emit_consensus_trio(&sink, &m), ALL_EMITTED);
    assert_eq!(
        sink.emit(tx_event(91)),
        EmitOutcome::Shed,
        "bulk still full"
    );
    let s = sink.stats().snapshot();
    assert_eq!((s.bulk_shed, s.consensus_shed), (2, 0));
    assert_eq!(m.blocks_dropped(), 0);
    let evs: Vec<NetworkEvent> = std::iter::from_fn(|| lanes.try_recv()).collect();
    let got: Vec<&str> = evs.iter().map(variant).collect();
    assert_consensus_first(&got, 4);
    let tags: Vec<u64> = evs[3..].iter().map(tx_tag).collect();
    assert_eq!(
        tags,
        [1, 2, 3, 4],
        "admitted txs in emit order, shed ones absent"
    );
}

// REQ-GRV-017 / F-7 — Decision: a failure means a bulk lane full by BYTES sheds or delays consensus events, or the async receive the node loop uses is not biased consensus-first.
#[tokio::test]
async fn req_grv_017_full_bulk_lane_by_bytes_never_sheds_or_delays_consensus() {
    let sz = event_bytes(&tx_event(5));
    assert!(sz > 0);
    let b = LaneBudgets {
        bulk_bytes: 3 * sz,
        ..roomy()
    };
    let (sink, mut lanes) = event_lanes(b);
    let m = GossipShedMetrics::new();
    for i in 0..3 {
        assert_eq!(sink.emit(tx_event(5)), EmitOutcome::Emitted, "bulk {i}");
    }
    assert_eq!(
        sink.emit(tx_event(5)),
        EmitOutcome::Shed,
        "bulk at byte cap"
    );
    assert!(sink.stats().snapshot().bulk_bytes <= b.bulk_bytes);
    assert_eq!(emit_consensus_trio(&sink, &m), ALL_EMITTED);
    let s = sink.stats().snapshot();
    assert_eq!((s.bulk_shed, s.consensus_shed), (1, 0));
    let mut got = Vec::new();
    for _ in 0..6 {
        let ev = tokio::time::timeout(Duration::from_secs(5), lanes.recv())
            .await
            .expect("recv must not hang with queued events")
            .expect("lanes open");
        got.push(variant(&ev));
    }
    assert_consensus_first(&got, 3);
    assert!(
        lanes.try_recv().is_none(),
        "nothing beyond the 6 admitted events"
    );
}

// REQ-GRV-006 / F-11 — Decision: a failure means the node loop hangs forever (or loses queued events) after the swarm side is gone, so shutdown waits on a dead channel.
#[tokio::test]
async fn req_grv_006_f11_recv_drains_both_lanes_then_returns_none_after_sink_drop() {
    let (sink, mut lanes): (_, EventLanes) = event_lanes(roomy());
    assert_eq!(sink.emit(tx_event(1)), EmitOutcome::Emitted);
    assert_eq!(
        sink.emit(NetworkEvent::PeerConnected(PeerId::random())),
        EmitOutcome::Emitted
    );
    drop(sink);
    let t = Duration::from_secs(5);
    let mut got = Vec::new();
    for _ in 0..2 {
        let ev = tokio::time::timeout(t, lanes.recv())
            .await
            .expect("no hang");
        got.push(variant(&ev.expect("queued event survives sink drop")));
    }
    assert_eq!(got, ["PeerConnected", "NewTransaction"]);
    let end = tokio::time::timeout(t, lanes.recv())
        .await
        .expect("recv must return once the sink is dropped and the lanes are empty");
    assert!(end.is_none());
    assert!(lanes.try_recv().is_none());
}
