//! INC-I-289 M-1: the snap descent proof rides the existing `GetHeaders` request
//! before the manifest. OUTPUT: the next request + `snap_descent_proven`.
//! PATHS: P1 two batches link f to the anchor, then the manifest is sent | P2 a
//! broken batch drops the proof and leaves the peer | P3 span over the bound falls
//! back before any download.

use crypto::Hash;
use doli_core::{BlockHeader, FinalityCheckpoint};
use libp2p::PeerId;
use std::time::Instant;

use super::snap_descent::SNAP_DESCENT_PROOF_MAX_HEADERS;
use crate::protocols::{SyncRequest, SyncResponse};
use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

const FH: u64 = 10;

fn header(prev_hash: Hash, slot: u32) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: slot as u64,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![0; 32] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

/// Headers FH+1..=FH+n on `f`.
fn chain(f: Hash, n: u64) -> Vec<BlockHeader> {
    let mut prev = f;
    (1..=n)
        .map(|i| {
            let hd = header(prev, (FH + i) as u32);
            prev = hd.hash();
            hd
        })
        .collect()
}

fn mgr(f: Hash, anchor: (u64, Hash)) -> (SyncManager, PeerId) {
    let config = SyncConfig {
        inc_i_288_fork_choice_activation_height: 0,
        ..Default::default()
    };
    let mut mgr = SyncManager::new(config, Hash::ZERO);
    mgr.local_height = FH + 2;
    mgr.restore_finality(FinalityCheckpoint {
        block_hash: f,
        height: FH,
        slot: FH as u32,
        attestation_weight: 1,
        total_weight: 1,
    });
    let peer = PeerId::random();
    mgr.add_peer(peer, anchor.0, anchor.1, anchor.0 as u32);
    mgr.pipeline_data = SyncPipelineData::SnapDownloading {
        target_hash: anchor.1,
        target_height: anchor.0,
        quorum_root: Hash::ZERO,
        peer,
        alternate_peers: vec![],
    };
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::SnapDownloading,
        started_at: Instant::now(),
    };
    (mgr, peer)
}

fn next(mgr: &mut SyncManager, peer: PeerId) -> Option<SyncRequest> {
    if let Some(s) = mgr.peers.get_mut(&peer) {
        s.pending_request = None;
    }
    mgr.next_request().map(|(_, r)| r)
}

#[test]
fn m1_inc_i_289_proof_batches_then_manifest() {
    let f = crypto::hash::hash(b"f");
    let headers = chain(f, 700);
    let anchor = (FH + 700, headers.last().unwrap().hash());
    let (mut m, peer) = mgr(f, anchor);

    match next(&mut m, peer) {
        Some(SyncRequest::GetHeaders {
            start_hash,
            max_count,
        }) => {
            assert_eq!((start_hash, max_count), (f, 500), "first batch from f")
        }
        other => panic!("expected the first proof batch, got {other:?}"),
    }
    m.handle_response(peer, SyncResponse::Headers(headers[..500].to_vec()));
    match next(&mut m, peer) {
        Some(SyncRequest::GetHeaders {
            start_hash,
            max_count,
        }) => {
            assert_eq!(
                (start_hash, max_count),
                (headers[499].hash(), 200),
                "second batch"
            )
        }
        other => panic!("expected the second proof batch, got {other:?}"),
    }
    m.handle_response(peer, SyncResponse::Headers(headers[500..].to_vec()));
    assert!(
        m.snap_descent_proven(anchor.0, anchor.1, f),
        "proof complete"
    );
    assert!(
        matches!(
            next(&mut m, peer),
            Some(SyncRequest::GetStateManifest { .. })
        ),
        "the manifest follows a complete proof"
    );
}

#[test]
fn m1_inc_i_289_broken_batch_drops_the_proof() {
    let f = crypto::hash::hash(b"f");
    let mut headers = chain(f, 600);
    let anchor = (FH + 600, headers.last().unwrap().hash());
    let (mut m, peer) = mgr(f, anchor);
    assert!(matches!(
        next(&mut m, peer),
        Some(SyncRequest::GetHeaders { .. })
    ));
    headers[3].prev_hash = Hash::ZERO;
    m.handle_response(peer, SyncResponse::Headers(headers[..500].to_vec()));
    assert!(!m.snap_descent_proven(anchor.0, anchor.1, f), "no proof");
    assert!(
        !matches!(m.pipeline_data, SyncPipelineData::SnapDownloading { peer: p, .. } if p == peer),
        "the peer that served a broken chain is left"
    );
}

#[test]
fn m1_inc_i_289_span_over_bound_falls_back_before_download() {
    let f = crypto::hash::hash(b"f");
    let anchor = (
        FH + SNAP_DESCENT_PROOF_MAX_HEADERS + 1,
        crypto::hash::hash(b"a"),
    );
    let (mut m, peer) = mgr(f, anchor);
    assert!(next(&mut m, peer).is_none(), "no request");
    assert!(
        !m.pipeline_data.is_snap_syncing(),
        "over the bound the node falls back to block sync before any download"
    );
}

/// Quorum: `anchors[0]` is the primary, the rest are its alternates.
fn quorum(f: Hash, anchors: &[(u64, Hash)]) -> (SyncManager, Vec<PeerId>) {
    let (mut m, primary) = mgr(f, anchors[0]);
    let mut peers = vec![primary];
    let mut alts = Vec::new();
    for a in &anchors[1..] {
        let p = PeerId::random();
        m.add_peer(p, a.0, a.1, a.0 as u32);
        alts.push((p, a.1, a.0));
        peers.push(p);
    }
    if let SyncPipelineData::SnapDownloading {
        alternate_peers, ..
    } = &mut m.pipeline_data
    {
        *alternate_peers = alts;
    }
    (m, peers)
}

/// AUDIT-P2-644-003: one over-span vote is a per-peer fault, not the end of the attempt.
#[test]
fn o1_inc_i_289_over_span_vote_moves_the_proof_to_an_honest_peer() {
    let f = crypto::hash::hash(b"f");
    let headers = chain(f, 600);
    let honest = (FH + 600, headers.last().unwrap().hash());
    let over = (
        FH + SNAP_DESCENT_PROOF_MAX_HEADERS + 1,
        crypto::hash::hash(b"a"),
    );
    let (mut m, peers) = quorum(f, &[over, honest, honest]);

    assert!(
        next(&mut m, peers[0]).is_none(),
        "no request to the over-span peer"
    );
    assert_eq!(
        m.snap.attempts, 0,
        "one over-span vote must not end the attempt"
    );
    let (to, req) = m
        .next_request()
        .expect("the proof continues with an honest peer");
    assert!(
        peers[1..].contains(&to),
        "the next request goes to an honest quorum peer"
    );
    assert!(
        matches!(req, SyncRequest::GetHeaders { start_hash, max_count: 500 } if start_hash == f),
        "the proof starts from f at the honest anchor, got {req:?}"
    );
}

/// Control: when every quorum peer is over the span the attempt falls back once.
#[test]
fn o1_inc_i_289_all_over_span_falls_back() {
    let f = crypto::hash::hash(b"f");
    let over = (
        FH + SNAP_DESCENT_PROOF_MAX_HEADERS + 1,
        crypto::hash::hash(b"a"),
    );
    let (mut m, peers) = quorum(f, &[over, over, over]);
    assert!(next(&mut m, peers[0]).is_none(), "no request");
    assert_eq!(m.snap.attempts, 1, "the attempt ends once");
    assert!(
        !m.pipeline_data.is_snap_syncing(),
        "no quorum peer is in bound: block sync"
    );
}

/// AUDIT-P2-644-004: one proof batch per 1 s tick and the manifest is served at
/// the peer's current tip, so a proof of exactly the bound must reach the
/// manifest within half a slot of ticks.
#[test]
fn o1_inc_i_289_proof_of_the_bound_reaches_the_manifest_within_half_a_slot() {
    let f = crypto::hash::hash(b"f");
    let headers = chain(f, SNAP_DESCENT_PROOF_MAX_HEADERS);
    let anchor = (
        FH + SNAP_DESCENT_PROOF_MAX_HEADERS,
        headers.last().unwrap().hash(),
    );
    let (mut m, peer) = mgr(f, anchor);
    let budget = doli_core::consensus::SLOT_DURATION / 2;
    let mut served = 0usize;
    for tick in 1..=budget {
        match next(&mut m, peer) {
            Some(SyncRequest::GetHeaders { max_count, .. }) => {
                let end = served + max_count as usize;
                m.handle_response(peer, SyncResponse::Headers(headers[served..end].to_vec()));
                served = end;
            }
            Some(SyncRequest::GetStateManifest { .. }) => return,
            other => panic!("tick {tick}: expected a proof batch or the manifest, got {other:?}"),
        }
    }
    panic!(
        "a proof of the bound ({SNAP_DESCENT_PROOF_MAX_HEADERS} headers) did not reach the manifest within {budget} ticks; {served} headers served"
    );
}

/// The documented window is the constant.
#[test]
fn o1_inc_i_289_documented_bound_is_the_constant() {
    let n = SNAP_DESCENT_PROOF_MAX_HEADERS;
    let shown = format!("{},{:03}", n / 1000, n % 1000);
    let spec = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../specs/protocol.md"
    ));
    let doc = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/troubleshooting.md"
    ));
    assert!(
        spec.contains(&format!("`SNAP_DESCENT_PROOF_MAX_HEADERS` ({shown}")),
        "specs/protocol.md states the bound {shown}"
    );
    assert!(
        doc.contains(&format!("anchor is above {shown} headers")),
        "docs/troubleshooting.md states the bound {shown}"
    );
}
