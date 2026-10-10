//! INC-I-237: header-first sync never lands a batch while single blocks keep
//! arriving through gossip / ORPHAN_CHASE / ATTEST_FETCH.
//!
//! The GetHeaders request is built from `local_hash` at SEND time
//! (dispatch.rs `create_request(self.local_hash)`), but the response is
//! validated against `local_hash` at RECEIVE time
//! (response.rs `process_headers(&headers, self.local_hash)`). When one block
//! is applied between send and receive, the response's first header has
//! `prev_hash == old tip`, so the whole batch is dropped as a "chain break"
//! (live: mainnet n3, h=666271, `valid_so_far=0`).
//!
//! OUTPUT CONTRACT: fn handle_response(&mut self, peer, SyncResponse::Headers(h))
//!   O1: pipeline.pending_headers / headers_needing_bodies (accepted headers)
//!   O2: fork.chain_breaks (diagnostic chain-break record)
//!   O3: fork.consecutive_empty_headers (fork evidence counter)
//! PATHS:
//!   P1: no block applied between send and receive (control)
//!   P2: one block (H+1) applied via block_applied_with_weight between send
//!       and receive (bug path)
//! INPUT PARTITIONS: same canonical chain on both sides; N = 12 headers.
//! MATRIX: 3 outputs x 2 paths
//!   P1: O1 = N headers, O2 = empty, O3 = 0 (passes today)
//!   P2: O1 >= headers H+2..H+N, O2 = empty, O3 = 0 (FAILS today: 0, break, 1)

use libp2p::PeerId;

use crypto::Hash;
use doli_core::BlockHeader;

use super::*;
use crate::protocols::{SyncRequest, SyncResponse};

const LOCAL_H: usize = 5;
const N: usize = 12;

fn header(prev_hash: Hash, slot: u32) -> BlockHeader {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: now,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![0; 32] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

/// Canonical chain of LOCAL_H + N headers on top of genesis (index i = height i+1).
fn canonical_chain(genesis: Hash) -> Vec<BlockHeader> {
    let mut out = Vec::new();
    let mut prev = genesis;
    for i in 0..(LOCAL_H + N) {
        let h = header(prev, (i + 1) as u32);
        prev = h.hash();
        out.push(h);
    }
    out
}

/// Manager at height LOCAL_H (tip T0) with one peer at LOCAL_H + N, which has
/// already sent GetHeaders(start_hash = T0). Returns (manager, peer, chain).
fn manager_with_inflight_headers_request() -> (SyncManager, PeerId, Vec<BlockHeader>) {
    let genesis = Hash::ZERO;
    let chain = canonical_chain(genesis);
    let mut manager = SyncManager::new(SyncConfig::default(), genesis);
    manager.disable_snap_sync();

    let mut prev = genesis;
    for (i, h) in chain.iter().take(LOCAL_H).enumerate() {
        manager.block_applied_with_weight(h.hash(), (i + 1) as u64, h.slot, 1, prev);
        prev = h.hash();
    }
    let t0 = chain[LOCAL_H - 1].hash();
    assert_eq!(manager.local_hash, t0, "setup: local tip is T0");

    let peer = PeerId::random();
    let tip = &chain[LOCAL_H + N - 1];
    manager.add_peer(peer, (LOCAL_H + N) as u64, tip.hash(), tip.slot);
    manager.start_sync();

    let (to, req) = manager
        .next_request()
        .expect("setup: header sync must emit a request");
    assert_eq!(to, peer);
    match req {
        SyncRequest::GetHeaders { start_hash, .. } => {
            assert_eq!(start_hash, t0, "non-vacuity: request starts at T0")
        }
        other => panic!("setup: expected GetHeaders, got {:?}", other),
    }
    (manager, peer, chain)
}

/// Peer's reply: headers H+1..H+N (first header's prev_hash = T0).
fn peer_response(chain: &[BlockHeader]) -> Vec<BlockHeader> {
    chain[LOCAL_H..].to_vec()
}

#[test]
fn test_inc_i237_control_headers_accepted_without_interleaved_apply() {
    let (mut manager, peer, chain) = manager_with_inflight_headers_request();
    let _ = manager.handle_response(peer, SyncResponse::Headers(peer_response(&chain)));

    assert_eq!(manager.pipeline.pending_headers.len(), N);
    assert!(manager.fork.chain_breaks.is_empty());
    assert_eq!(manager.fork.consecutive_empty_headers, 0);
}

#[test]
fn test_inc_i237_headers_accepted_after_gossip_block_applied_mid_request() {
    let (mut manager, peer, chain) = manager_with_inflight_headers_request();

    // Block H+1 (T1, parent T0) lands via the single-block path before the
    // peer's response arrives — same call apply_block/mod.rs:278 makes.
    let t0 = chain[LOCAL_H - 1].hash();
    let b1 = &chain[LOCAL_H];
    manager.block_applied_with_weight(b1.hash(), (LOCAL_H + 1) as u64, b1.slot, 1, t0);
    assert_eq!(manager.local_hash, b1.hash(), "setup: tip advanced to T1");

    let _ = manager.handle_response(peer, SyncResponse::Headers(peer_response(&chain)));

    let beyond_tip: Vec<Hash> = chain[LOCAL_H + 1..].iter().map(|h| h.hash()).collect();
    let accepted = manager
        .pipeline
        .pending_headers
        .iter()
        .filter(|h| beyond_tip.contains(&h.hash()))
        .count();
    let breaks: Vec<(Hash, Hash, u32)> = manager
        .fork
        .chain_breaks
        .iter()
        .map(|b| {
            (
                b.actual_prev_hash,
                b.expected_prev_hash,
                b.valid_so_far_count,
            )
        })
        .collect();
    assert_eq!(
        accepted,
        N - 1,
        "INC-I-237: headers H+2..H+N must be accepted after H+1 was applied \
         mid-request; accepted={} chain_breaks(actual,expected,valid_so_far)={:?}",
        accepted,
        breaks
    );
    assert!(
        manager.fork.chain_breaks.is_empty(),
        "no chain break expected"
    );
    assert_eq!(manager.fork.consecutive_empty_headers, 0);
}
