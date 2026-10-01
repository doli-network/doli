//! INC-I-235 M1 — a wedged node's by-hash parent walk must be SERVED while the
//! header pipeline is active (C1) and must SURVIVE uncorrelated block responses
//! on the walk peer (C2). Sync-manager twin of the node-level reproduction.
//!
//! covers: dispatch (sync_engine/dispatch.rs `next_request`), fork_recovery
//! (sync/fork_recovery.rs `ForkRecoveryTracker::handle_block` / `next_fetch`)
//!
//! OUTPUT CONTRACT
//!   fn SyncManager::next_request(&mut self) -> Option<(PeerId, SyncRequest)>
//!     O1 return value (peer, request)
//!     O2 receiver: walk pending state (observed via `fork_recovery_current_parent`)
//!     O3 receiver: `peers[p].pending_request` (one in flight per peer)
//!   fn ForkRecoveryTracker::handle_block(&mut self, peer, Option<Block>) -> bool
//!     O1 return (consumed?)   O2 receiver: active / pending / next_parent
//!   fn SyncManager::handle_response(&mut self, peer, SyncResponse::Block) -> Vec<Block>
//!     O1 returned pass-through blocks   O2 receiver: tracker state
//!   PATHS: next_request {Headers in flight, Headers parked, Headers without walk};
//!          handle_block {expected hash, wrong hash, genesis mismatch}
//!   MATRIX: each test names the (output, path) cell it asserts.
//!
//! INPUT PARTITIONS
//!   I1 walk active + header request in flight (seed2 at 16:20, Syncing{DownloadingHeaders})
//!   I2 walk active + Headers parked (>=10 empty headers, gap > 3)
//!   I3 no walk + Headers (local_h = 0 bootstrap, local_h > 0)       — lock
//!   I4 walk pending + wrong-hash block from the walk peer (chase response)
//!   I5 walk pending + wrong-hash block, then REQUEST_TIMEOUT elapses with an alternate
//!   I6 walk pending + expected hash but foreign genesis                — lock
//!   I7 I1 + I4 interleaved over ticks (the incident shape)

use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Block;
use libp2p::PeerId;

use crate::protocols::{SyncRequest, SyncResponse};
use crate::sync::fork_recovery::ForkRecoveryTracker;
use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

/// Local tip height: the losing sibling B sits here.
const FORK_H: u64 = 6;
/// Gap at Wedged entry in the incident (59) — Rule 1/1b closed.
const WEDGE_GAP: u64 = 59;

fn blk(prev_hash: Hash, slot: u32, genesis_hash: Hash) -> Block {
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash,
        timestamp: slot as u64 * 10,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

/// Stored fork point (h=5), canonical sibling A (h=6, lower slot), its child A1
/// (h=7, slot below B's — as 02f75ac6 < 605e167d), and A2 (h=8).
struct Fork {
    base: Hash,
    a: Block,
    a1: Block,
    a2: Block,
}

fn fork() -> Fork {
    let base = crypto::hash::hash(b"inc_i_235_stored_fork_point_h5");
    let a = blk(base, 100, Hash::ZERO);
    let a1 = blk(a.hash(), 103, Hash::ZERO);
    let a2 = blk(a1.hash(), 112, Hash::ZERO);
    Fork { base, a, a1, a2 }
}

/// Node on losing sibling B, three peers on the canonical tip `gap` ahead, header
/// pipeline owned by `header_peer`. Returns (mgr, header_peer, walk_peer).
fn wedged_mgr(gap: u64) -> (SyncManager, PeerId, PeerId) {
    let mut m = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    m.local_height = FORK_H;
    m.local_slot = 108;
    m.local_hash = crypto::hash::hash(b"inc_i_235_losing_sibling_B");
    let header_peer = PeerId::random();
    let walk_peer = PeerId::random();
    let tip = crypto::hash::hash(b"inc_i_235_canonical_tip");
    for p in [header_peer, walk_peer, PeerId::random()] {
        m.add_peer(p, FORK_H + gap, tip, 108 + gap as u32);
    }
    m.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    m.pipeline_data = SyncPipelineData::Headers {
        target_slot: 108 + gap as u32,
        peer: header_peer,
        headers_count: 0,
    };
    for s in m.peers.values_mut() {
        s.pending_request = None;
    }
    (m, header_peer, walk_peer)
}

/// Put one header request in flight on `header_peer` (Syncing{DownloadingHeaders}).
fn header_in_flight(m: &mut SyncManager, header_peer: PeerId) {
    let first = m.next_request();
    assert!(
        matches!(&first, Some((p, r)) if *p == header_peer
            && !matches!(r, SyncRequest::GetBlockByHash { .. })),
        "fixture: the header pipeline must own an in-flight request, got {:?}",
        first.map(|(p, r)| (p, format!("{:?}", r)))
    );
    assert!(m.peers[&header_peer].pending_request.is_some());
}

fn by_hash(req: &Option<(PeerId, SyncRequest)>) -> Option<(PeerId, Hash)> {
    match req {
        Some((p, SyncRequest::GetBlockByHash { hash })) => Some((*p, *hash)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// C1 — dispatch while the header pipeline is active (REQ-I235-003)
// ---------------------------------------------------------------------------

// REQ-I235-003 — Decision: a FAIL means a started walk is starved for as long as
// header sync owns the pipeline, so the Wedged fix can never reach the fork point.
// Cell: O1 × I1.
#[test]
fn c1_walk_fetch_dispatched_while_header_request_in_flight() {
    let f = fork();
    let (mut m, header_peer, walk_peer) = wedged_mgr(WEDGE_GAP);
    header_in_flight(&mut m, header_peer);
    assert!(m.start_fork_recovery(f.a1.clone(), walk_peer));

    let req = m.next_request();
    assert_eq!(
        by_hash(&req),
        Some((walk_peer, f.a.hash())),
        "C1: with Headers in flight, the active walk's GetBlockByHash(prev of A1 = A) \
         must be sent on this tick; dispatch.rs serves the walk only in the None arm"
    );
}

// REQ-I235-003 — Decision: a FAIL means a parked header pipeline (>=10 empty
// headers, gap>3) swallows the walk fetch on the tick it is due.
// Cell: O1 × I2.
#[test]
fn c1_walk_fetch_dispatched_while_headers_parked() {
    let f = fork();
    let (mut m, _header_peer, walk_peer) = wedged_mgr(WEDGE_GAP);
    m.fork.consecutive_empty_headers = 10;
    assert!(m.start_fork_recovery(f.a1.clone(), walk_peer));

    let req = m.next_request();
    assert_eq!(
        by_hash(&req),
        Some((walk_peer, f.a.hash())),
        "C1: a parked Headers arm must not consume the tick an active walk needs"
    );
}

// REQ-I235-SEC-001 (INV-SYNC-009) — Decision: a FAIL means the hoisted walk
// dispatch emits more than one by-hash request in flight, an ungoverned burst.
// Cell: O1/O3 × I2 over repeated ticks.
#[test]
fn c1_walk_keeps_one_request_in_flight_under_headers() {
    let f = fork();
    let (mut m, _header_peer, walk_peer) = wedged_mgr(WEDGE_GAP);
    m.fork.consecutive_empty_headers = 10;
    assert!(m.start_fork_recovery(f.a1.clone(), walk_peer));

    let walk_fetches = (0..8)
        .map(|_| m.next_request())
        .filter(|r| by_hash(r).is_some())
        .count();
    assert_eq!(walk_fetches, 1, "one walk fetch in flight, never a burst");
}

// REQ-I235-003 / INV-SYNC-015 lock — Decision: a FAIL means the dispatch change
// altered header sync for a node with NO walk (bootstrap / behind-ness path).
// Cell: O1 × I3.
#[test]
fn c1_lock_headers_arm_unchanged_without_walk() {
    for local_h in [0u64, FORK_H] {
        let (mut m, header_peer, _walk_peer) = wedged_mgr(WEDGE_GAP);
        m.local_height = local_h;
        assert!(!m.is_fork_recovery_active());
        let req = m.next_request();
        assert!(
            matches!(&req, Some((p, r)) if *p == header_peer
                && !matches!(r, SyncRequest::GetBlockByHash { .. })),
            "no walk → the Headers arm must still issue the header request (local_h={})",
            local_h
        );
    }
}

// ---------------------------------------------------------------------------
// C2 — content-keyed intercept (REQ-I235-SEC-001 "wrong-hash → alternate peer")
// ---------------------------------------------------------------------------

// REQ-I235-SEC-001 — Decision: a FAIL means one uncorrelated block from the walk
// peer (the live ORPHAN_CHASE response) still kills the walk and arms a 30 s cooldown.
// Cell: O1/O2 × I4.
#[test]
fn c2_wrong_hash_block_passes_through_tracker_and_walk_stays_pending() {
    let f = fork();
    let peer = PeerId::random();
    let mut t = ForkRecoveryTracker::new();
    assert!(t.start(f.a1.clone(), peer));
    assert_eq!(t.next_fetch(), Some((peer, f.a.hash())));

    let consumed = t.handle_block(peer, Some(f.a2.clone()));
    assert!(
        !consumed,
        "C2: a wrong-hash block must pass through, not be consumed"
    );
    assert!(
        t.is_active(),
        "C2: the walk must survive a wrong-hash block"
    );
    assert_eq!(
        t.current_parent(),
        None,
        "C2: the walk must stay pending on A"
    );

    assert!(
        t.handle_block(peer, Some(f.a.clone())),
        "the expected block is consumed"
    );
    assert_eq!(
        t.current_parent(),
        Some(f.base),
        "walk advanced to the stored fork point"
    );
}

// REQ-I235-SEC-001 — Decision: a FAIL means the intercept still swallows (and the
// node never sees) a chase block arriving on the walk peer via SyncManager.
// Cell: handle_response O1/O2 × I4.
#[test]
fn c2_chase_response_on_walk_peer_passes_through_sync_manager() {
    let f = fork();
    let (mut m, _header_peer, walk_peer) = wedged_mgr(WEDGE_GAP);
    m.fork.consecutive_empty_headers = 10;
    assert!(m.start_fork_recovery(f.a1.clone(), walk_peer));
    let _ = m.next_request();
    let _ = m.next_request();

    let out = m.handle_response(walk_peer, SyncResponse::Block(Some(f.a1.clone())));
    assert_eq!(
        out.iter().map(|b| b.hash()).collect::<Vec<_>>(),
        vec![f.a1.hash()],
        "C2: the uncorrelated by-height chase block must reach block handling"
    );
    assert!(m.is_fork_recovery_active(), "C2: the walk must survive it");

    let out = m.handle_response(walk_peer, SyncResponse::Block(Some(f.a.clone())));
    assert!(
        out.is_empty(),
        "the requested block is consumed by the walk"
    );
    assert_eq!(m.fork_recovery_current_parent(), Some(f.base));
}

// REQ-I235-SEC-001 — Decision: a FAIL means a peer that answers with the wrong
// block is never failed over (walk dead or stuck on one peer). 10 s real wait:
// REQUEST_TIMEOUT is a const with no injection seam.
// Cell: next_fetch O1 × I5.
#[test]
fn c2_wrong_hash_then_request_timeout_fails_over_to_alternate() {
    let f = fork();
    let (peer, alt) = (PeerId::random(), PeerId::random());
    let mut t = ForkRecoveryTracker::new();
    assert!(t.start(f.a1.clone(), peer));
    t.set_alternate_peers(vec![alt]);
    assert_eq!(t.next_fetch(), Some((peer, f.a.hash())));
    let _ = t.handle_block(peer, Some(f.a2.clone()));

    std::thread::sleep(Duration::from_millis(10_300));
    assert_eq!(
        t.next_fetch(),
        Some((alt, f.a.hash())),
        "C2: after REQUEST_TIMEOUT the same parent must be re-requested from the alternate"
    );
}

// REQ-I235-SEC-001 lock — Decision: a FAIL means the C2 relaxation also let a
// block from a different chain into the walk.
// Cell: O1/O2 × I6.
#[test]
fn c2_lock_genesis_mismatch_still_cancels() {
    let ours = crypto::hash::hash(b"inc_i_235_our_genesis");
    let theirs = crypto::hash::hash(b"inc_i_235_other_genesis");
    let foreign_parent = blk(crypto::hash::hash(b"x"), 50, theirs);
    let child = blk(foreign_parent.hash(), 51, ours);
    let peer = PeerId::random();
    let mut t = ForkRecoveryTracker::new();
    t.set_genesis_hash(ours);
    assert!(t.start(child, peer));
    let _ = t.next_fetch();

    assert!(t.handle_block(peer, Some(foreign_parent)));
    assert!(
        !t.is_active(),
        "genesis mismatch must still cancel the walk"
    );
}

// ---------------------------------------------------------------------------
// REQ-I235-001 — sync-manager twin of the seed2 reproduction
// ---------------------------------------------------------------------------

// REQ-I235-001 — Decision: a FAIL means a wedged node with Headers in flight and
// a live chase loop on the walk peer never fetches the fork-point sibling A
// (0 of 135,508 requests in the incident).
// Cell: next_request O1 + handle_response O1/O2 × I7; terminal = connected.
#[test]
fn repro_sm_walk_reaches_fork_point_under_headers_and_chase() {
    let f = fork();
    let (mut m, header_peer, walk_peer) = wedged_mgr(WEDGE_GAP);
    header_in_flight(&mut m, header_peer);
    assert!(m.start_fork_recovery(f.a1.clone(), walk_peer));

    let serve = |h: Hash| {
        [&f.a, &f.a1, &f.a2]
            .into_iter()
            .find(|b| b.hash() == h)
            .cloned()
    };
    let mut fetched = Vec::new();
    for _ in 0..5 {
        let req = m.next_request();
        if let Some((q, hash)) = by_hash(&req) {
            fetched.push(hash);
            let chase = m.handle_response(q, SyncResponse::Block(Some(f.a1.clone())));
            assert!(chase.iter().all(|b| b.hash() == f.a1.hash()));
            let _ = m.handle_response(q, SyncResponse::Block(serve(hash)));
        }
        if m.fork_recovery_current_parent() == Some(f.base) {
            break;
        }
    }

    assert!(
        fetched.contains(&f.a.hash()),
        "REQ-I235-001: the fork-point sibling A was never requested by hash (fetched={})",
        fetched.len()
    );
    let done = m
        .check_fork_recovery_connection(true)
        .expect("REQ-I235-001: walk must reach the stored fork point");
    assert_eq!(done.connection_point, f.base);
    assert_eq!(
        done.blocks.iter().map(|b| b.hash()).collect::<Vec<_>>(),
        vec![f.a.hash(), f.a1.hash()]
    );
    assert!(!m.is_snap_syncing(), "REQ-I235-005: never snap");
}
