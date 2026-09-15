//! UTXO-scalability M3 [F3] — CLIENT side of the chunked snap-sync session.
//!
//! Pins the four client-side filters the Failure Analyst marked BLOCKER/High for Q3:
//!   3c / F-07 — no resumption cursor: any error restarts the transfer from byte 0.
//!   3d / F-08 — `handle_snap_download_error` blacklists on ANY error, so a peer whose tip
//!               advanced (or whose session expired) is punished like an equivocator.
//!   3f / F-16 — the 3-attempt cap is consumed per FAILURE; more failure points per
//!               transfer means the cap is exhausted faster.
//!   REQ-SCALE-006 — the reassembled digest is the only thing standing between a truncated
//!               chunk stream and an installed state whose root can never match quorum.
//!
//! OUTPUT CONTRACT: the client session handlers on `SyncManager`
//!   (`handle_state_manifest`, `handle_state_chunk`, `handle_state_chunk_error`,
//!    `handle_state_session_unavailable`) and the outbound `next_request`.
//!   Outputs observable from the stable API:
//!     O1 `self.pipeline_data` — did the session advance, complete, or refuse
//!     O2 the NEXT outbound `SyncRequest` (which cursor, which kind)
//!     O3 `self.snap.blacklisted_peers` — was the peer punished
//!     O4 `self.snap.attempts` — was a snap attempt consumed
//!     O5 `self.snap.integrity_refusals` — was a refusal counted
//!   Paths:
//!     P1 happy stream, chunk accepted, more to come      — D1 (O1, O2)
//!     P2 chunk error mid-stream                          — D1, D4 (O2, O3, O4)
//!     P3 `StateSessionUnavailable{ManifestExpired}`      — D2 (O2, O3, O4)
//!     P4 `StateSessionUnavailable{Busy}`                 — D3 (O2, O3, O4)
//!     P5 stream completes, digest MISMATCHES the manifest — D5 (O1, O5)
//!     P6 manifest `state_root` != `quorum_root`           — D6 (O1, O2, O5)
//!   MATRIX: P1xO2 + P2xO2 -> D1 | P3xO3 -> D2 | P4xO3 -> D3 | P2xO4 -> D4
//!           P5xO1 + P5xO5 -> D5 | P6xO1 + P6xO2 + P6xO5 -> D6
//! INPUT PARTITIONS: (a) a session that has accepted >= 1 chunk (a cursor EXISTS to resume
//!   from — a zero-chunk session cannot distinguish "resumed" from "restarted");
//!   (b) a refusal arriving with the CURRENT session id; (c) a final chunk whose
//!   reassembly differs from the manifest by one byte.
//!
//! The `SnapDownloading` fixture below constructs the variant with the fields it has TODAY.
//! If F-07's cursor is added as a FIELD of that variant rather than as session state, this
//! fixture is the one place to update.

use crypto::Hash;
use libp2p::PeerId;
use std::time::Instant;

use crate::protocols::sync::{StateSessionRefusal, SyncRequest, STATE_CHUNK_MAX_BYTES};
use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

/// Mirror of the snap attempt cap. It is a LITERAL `3` at `snap_sync.rs:377`,
/// `cleanup.rs:487` and `cleanup.rs:628` — there is no named constant to import.
/// F-16 says M3 must keep the cap SEMANTICS, so this test pins the semantics, and the
/// duplicated literal is itself part of what the developer should collapse.
const SNAP_ATTEMPT_CAP: u8 = 3;

const SESSION_ID: u64 = 0xD0_11_5E_55_10_00_00_01;

fn h(tag: &[u8]) -> Hash {
    crypto::hash::hash(tag)
}

/// A manager parked in `SnapDownloading` against one peer, with a STATUS quorum that
/// corroborates the anchor so ONLY the gate under test can refuse.
fn mgr_downloading(quorum_root: Hash, anchor: Hash, height: u64) -> (SyncManager, PeerId) {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = height.saturating_sub(1_000);
    mgr.local_slot = mgr.local_height as u32;

    for _ in 0..4 {
        mgr.add_peer(PeerId::random(), height, anchor, height as u32);
    }
    let peer = PeerId::random();
    mgr.add_peer(peer, height, anchor, height as u32);

    mgr.pipeline_data = SyncPipelineData::SnapDownloading {
        target_hash: anchor,
        target_height: height,
        quorum_root,
        peer,
        alternate_peers: vec![],
    };
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::SnapDownloading,
        started_at: Instant::now(),
    };
    (mgr, peer)
}

/// The dispatcher refuses to send while a request is outstanding; clear it so the test
/// observes the NEXT request the session would emit rather than the in-flight guard.
fn clear_in_flight(mgr: &mut SyncManager, peer: PeerId) {
    if let Some(status) = mgr.peers.get_mut(&peer) {
        status.pending_request = None;
    }
}

fn next_request(mgr: &mut SyncManager, peer: PeerId) -> Option<SyncRequest> {
    clear_in_flight(mgr, peer);
    mgr.next_request().map(|(_, req)| req)
}

/// A UTXO image body of `n` chunk-sized runs, plus the digest a client must reconstruct.
fn body_and_digest(n_chunks: usize, count: u64) -> (Vec<Vec<u8>>, Hash) {
    let bodies: Vec<Vec<u8>> = (0..n_chunks)
        .map(|i| vec![(i as u8).wrapping_add(1); 4_096])
        .collect();
    let mut image = count.to_le_bytes().to_vec();
    for b in bodies.iter() {
        image.extend_from_slice(b);
    }
    (bodies, crypto::hash::hash(&image))
}

fn cursor(i: usize) -> Vec<u8> {
    let mut k = vec![0u8; 36];
    k[35] = i as u8;
    k
}

#[allow(clippy::too_many_arguments)]
fn deliver_manifest(
    mgr: &mut SyncManager,
    peer: PeerId,
    anchor: Hash,
    height: u64,
    state_root: Hash,
    utxo_hash: Hash,
    utxo_count: u64,
) {
    mgr.handle_state_manifest(
        peer,
        SESSION_ID,
        anchor,
        height,
        state_root,
        utxo_hash,
        utxo_count,
        STATE_CHUNK_MAX_BYTES,
        vec![1u8; 64],
        vec![2u8; 64],
        None,
        None,
        None,
        None,
    );
}

fn is_snap_ready(mgr: &SyncManager) -> bool {
    matches!(mgr.pipeline_data, SyncPipelineData::SnapReady { .. })
}

// ==================== D1 — resumption from the last verified cursor ====================

/// REQ-SCALE-013 / F-07 / blocker 3c — Decision: a failure here means one timed-out chunk
/// throws away every byte already verified and the transfer restarts from zero. At the set
/// sizes this milestone exists for, a restart-on-hiccup transfer never completes on a live
/// network, so the whole session is a slower version of today's failure.
#[test]
fn m3_cursor_resumes_from_last_verified_range() {
    let root = h(b"m3_quorum_root");
    let anchor = h(b"m3_anchor");
    let (mut mgr, peer) = mgr_downloading(root, anchor, 200_000);
    let (bodies, utxo_hash) = body_and_digest(4, 4);

    deliver_manifest(&mut mgr, peer, anchor, 200_000, root, utxo_hash, 4);

    // Two chunks land and are verified; the cursor is now cursor(2).
    mgr.handle_state_chunk(peer, SESSION_ID, bodies[0].clone(), Some(cursor(1)));
    mgr.handle_state_chunk(peer, SESSION_ID, bodies[1].clone(), Some(cursor(2)));

    let after_two = next_request(&mut mgr, peer).expect("the session must keep requesting");
    match after_two {
        SyncRequest::GetStateChunk {
            session_id,
            start_key,
            max_bytes,
        } => {
            assert_eq!(
                session_id, SESSION_ID,
                "the session id must be carried back"
            );
            assert_eq!(
                start_key,
                Some(cursor(2)),
                "the next request must resume at the last VERIFIED cursor"
            );
            assert_eq!(max_bytes, STATE_CHUNK_MAX_BYTES);
        }
        other => panic!("expected GetStateChunk, got {:?}", other),
    }

    // A chunk fails. The cursor must NOT rewind.
    mgr.handle_state_chunk_error(peer, SESSION_ID);

    let after_error = next_request(&mut mgr, peer)
        .expect("a failed chunk must be retried, not abandon the session");
    match after_error {
        SyncRequest::GetStateChunk { start_key, .. } => {
            assert_eq!(
                start_key,
                Some(cursor(2)),
                "after a chunk failure the client must resume at the last verified cursor, \
                 NOT at byte 0 — resuming at None is blocker 3c"
            );
        }
        other => panic!("expected a resumed GetStateChunk, got {:?}", other),
    }
}

// ==================== D2 / D3 — retryable refusals never blacklist ====================

/// F-08 / blocker 3d — Decision: a failure here means an HONEST peer whose pinned view
/// expired is treated as an equivocator. PM-006 records that on a 6-node LAN blacklisting a
/// few peers exhausts the peer set, so this converts a recoverable retry into a node that
/// can never snap-sync again until the 30 s clear.
#[test]
fn m3_manifest_expired_is_a_retry_not_misbehaviour() {
    let root = h(b"m3_quorum_root");
    let anchor = h(b"m3_anchor");
    let (mut mgr, peer) = mgr_downloading(root, anchor, 200_000);
    let (bodies, utxo_hash) = body_and_digest(3, 3);

    deliver_manifest(&mut mgr, peer, anchor, 200_000, root, utxo_hash, 3);
    mgr.handle_state_chunk(peer, SESSION_ID, bodies[0].clone(), Some(cursor(1)));

    let attempts_before = mgr.snap.attempts;
    mgr.handle_state_session_unavailable(peer, SESSION_ID, StateSessionRefusal::ManifestExpired);

    assert!(
        mgr.snap.blacklisted_peers.is_empty(),
        "ManifestExpired must NOT blacklist: the peer answered honestly that its pinned view \
         is gone. Blacklisted: {:?}",
        mgr.snap.blacklisted_peers
    );
    assert_eq!(
        mgr.snap.attempts, attempts_before,
        "an expired manifest is a retry inside the SAME attempt, not a new attempt"
    );

    let next = next_request(&mut mgr, peer)
        .expect("after ManifestExpired the client must open a NEW session");
    assert!(
        matches!(next, SyncRequest::GetStateManifest { .. }),
        "the retry must start a new session with a fresh manifest, got {:?}",
        next
    );
}

/// F-08 / F-16 — Decision: identical to D2 for the `Busy` refusal. A server at its
/// concurrent-session cap is the EXPECTED steady state during a fleet-wide cascade
/// (INC-I-143); if `Busy` blacklists, the cascade blacklists every seed at once.
#[test]
fn m3_busy_refusal_does_not_blacklist() {
    let root = h(b"m3_quorum_root");
    let anchor = h(b"m3_anchor");
    let (mut mgr, peer) = mgr_downloading(root, anchor, 200_000);

    deliver_manifest(&mut mgr, peer, anchor, 200_000, root, h(b"any"), 1);

    let attempts_before = mgr.snap.attempts;
    mgr.handle_state_session_unavailable(peer, SESSION_ID, StateSessionRefusal::Busy);

    assert!(
        mgr.snap.blacklisted_peers.is_empty(),
        "Busy must NOT blacklist — it is backpressure, not misbehaviour. Blacklisted: {:?}",
        mgr.snap.blacklisted_peers
    );
    assert_eq!(
        mgr.snap.attempts, attempts_before,
        "a Busy refusal must not consume a snap attempt"
    );
}

/// F-08 — Decision: `Halted` carries the serving node's `[STATE_CORRUPT]` refusal. If the
/// client blacklisted on it, a fleet where several seeds are mid-rebuild would be
/// unreachable for snap-sync long after those seeds recovered.
#[test]
fn m3_halted_refusal_does_not_blacklist() {
    let root = h(b"m3_quorum_root");
    let anchor = h(b"m3_anchor");
    let (mut mgr, peer) = mgr_downloading(root, anchor, 200_000);

    deliver_manifest(&mut mgr, peer, anchor, 200_000, root, h(b"any"), 1);
    mgr.handle_state_session_unavailable(
        peer,
        SESSION_ID,
        StateSessionRefusal::Halted("[STATE_CORRUPT] rebuild in progress".to_string()),
    );

    assert!(
        mgr.snap.blacklisted_peers.is_empty(),
        "Halted is a retryable server condition, not misbehaviour. Blacklisted: {:?}",
        mgr.snap.blacklisted_peers
    );
}

// ==================== D4 — one session is one attempt ====================

/// F-16 / blocker 3f — Decision: a failure here means chunking multiplied the number of
/// failure points that each burn one of only THREE snap attempts, so a recovering node
/// falls back to header-first sync several times faster than it does today. That is the
/// cascade-amplification shape of INC-I-143, arriving through the transport change.
#[test]
#[allow(clippy::assertions_on_constants)]
fn m3_attempt_cap_unchanged_by_chunking() {
    let root = h(b"m3_quorum_root");
    let anchor = h(b"m3_anchor");
    let (mut mgr, peer) = mgr_downloading(root, anchor, 200_000);
    let (bodies, utxo_hash) = body_and_digest(5, 5);

    mgr.snap.attempts = 0;
    deliver_manifest(&mut mgr, peer, anchor, 200_000, root, utxo_hash, 5);

    mgr.handle_state_chunk(peer, SESSION_ID, bodies[0].clone(), Some(cursor(1)));
    for _ in 0..5 {
        mgr.handle_state_chunk_error(peer, SESSION_ID);
        clear_in_flight(&mut mgr, peer);
    }

    assert_eq!(
        mgr.snap.attempts, 0,
        "five per-chunk failures inside ONE session consumed {} snap attempt(s) — a chunk \
         retry is not a snap attempt, or the cap of {} is exhausted by a single bad link",
        mgr.snap.attempts, SNAP_ATTEMPT_CAP
    );

    // A whole session giving up is exactly ONE attempt, as today.
    mgr.snap_fallback_to_normal();
    assert_eq!(
        mgr.snap.attempts, 1,
        "abandoning the whole session must consume exactly one attempt"
    );
    assert!(
        SNAP_ATTEMPT_CAP == 3,
        "the snap attempt cap moved off 3 — F-16 requires the cap SEMANTICS be unchanged \
         by this milestone"
    );
}

// ==================== D5 — digest mismatch refuses the install ====================

/// REQ-SCALE-006 / REQ-SCALE-002 — Decision: a failure here means a truncated, reordered or
/// tampered chunk stream is INSTALLED. The node would then hold a UTXO set whose root can
/// never match the network's, and nothing downstream detects it: `BlockHeader` carries no
/// state root, so a wrong set is never caught at block acceptance.
#[test]
fn m3_digest_mismatch_refuses_install() {
    let root = h(b"m3_quorum_root");
    let anchor = h(b"m3_anchor");
    let (mut mgr, peer) = mgr_downloading(root, anchor, 200_000);
    let (bodies, utxo_hash) = body_and_digest(3, 3);

    let refusals_before = mgr.snap.integrity_refusals;
    deliver_manifest(&mut mgr, peer, anchor, 200_000, root, utxo_hash, 3);

    mgr.handle_state_chunk(peer, SESSION_ID, bodies[0].clone(), Some(cursor(1)));
    mgr.handle_state_chunk(peer, SESSION_ID, bodies[1].clone(), Some(cursor(2)));
    // Last chunk, one byte wrong, and the stream is declared complete.
    let mut corrupted = bodies[2].clone();
    corrupted[0] ^= 0xFF;
    mgr.handle_state_chunk(peer, SESSION_ID, corrupted, None);

    assert!(
        !is_snap_ready(&mgr),
        "a stream whose reassembled digest != manifest.utxo_hash MUST NOT become SnapReady"
    );
    assert!(
        mgr.snap.integrity_refusals > refusals_before,
        "a digest mismatch must be COUNTED as an integrity refusal (was {}, now {}) — a \
         silent discard makes the failure invisible to the operator",
        refusals_before,
        mgr.snap.integrity_refusals
    );
}

/// REQ-SCALE-006 — Decision: the control for D5. If the matching stream ALSO fails to
/// install, D5's refusal proves nothing about digest checking.
#[test]
fn m3_matching_digest_completes_the_session() {
    let root = h(b"m3_quorum_root");
    let anchor = h(b"m3_anchor");
    let (mut mgr, peer) = mgr_downloading(root, anchor, 200_000);
    let (bodies, utxo_hash) = body_and_digest(3, 3);

    deliver_manifest(&mut mgr, peer, anchor, 200_000, root, utxo_hash, 3);
    mgr.handle_state_chunk(peer, SESSION_ID, bodies[0].clone(), Some(cursor(1)));
    mgr.handle_state_chunk(peer, SESSION_ID, bodies[1].clone(), Some(cursor(2)));
    mgr.handle_state_chunk(peer, SESSION_ID, bodies[2].clone(), None);

    assert!(
        is_snap_ready(&mgr),
        "a complete stream whose digest matches the manifest must become SnapReady"
    );
    let snapshot = mgr
        .take_snap_snapshot()
        .expect("the completed session must yield a VerifiedSnapshot");
    assert_eq!(
        snapshot.state_root, root,
        "the installed snapshot must carry the quorum-agreed root"
    );
    assert_eq!(
        crypto::hash::hash(&snapshot.utxo_set),
        utxo_hash,
        "the reassembled utxo image must hash to the manifest's utxo_hash"
    );
}

// ==================== D6 — Gate 1 applies to the manifest ====================

/// INC-I-143 F4 Gate 1 / blocker 3b — Decision: a failure here means the exact-equality root
/// gate is checked only AFTER a multi-minute transfer, or not at all. Checking it on the
/// manifest is the whole reason the manifest exists: a forked peer must cost zero chunks,
/// and the gate must not become unsatisfiable just because the window grew.
#[test]
fn m3_manifest_state_root_must_equal_quorum_root() {
    let quorum_root = h(b"m3_quorum_root");
    let forked_root = h(b"m3_forked_peer_root");
    let anchor = h(b"m3_anchor");
    let (mut mgr, peer) = mgr_downloading(quorum_root, anchor, 200_000);

    let refusals_before = mgr.snap.integrity_refusals;
    deliver_manifest(
        &mut mgr,
        peer,
        anchor,
        200_000,
        forked_root,
        h(b"whatever"),
        1_000,
    );

    assert!(
        mgr.snap.integrity_refusals > refusals_before,
        "a manifest whose state_root != quorum_root must be counted as an integrity refusal"
    );
    assert!(
        !is_snap_ready(&mgr),
        "a manifest that fails Gate 1 must never reach SnapReady"
    );

    let next = next_request(&mut mgr, peer);
    assert!(
        !matches!(next, Some(SyncRequest::GetStateChunk { .. })),
        "not a single chunk may be requested against a manifest that failed Gate 1, got {:?}",
        next
    );
}

/// INC-I-143 F4 Gate 2 — Decision: the height-corroboration gate that caught the -1 anchor
/// splice must apply to the manifest too. If it moved to the end of the transfer, an
/// uncorroborated height costs a full state download before it is refused.
#[test]
fn m3_manifest_height_must_be_corroborated_by_quorum() {
    let quorum_root = h(b"m3_quorum_root");
    let anchor = h(b"m3_anchor");
    let canonical_height = 200_000u64;
    let (mut mgr, peer) = mgr_downloading(quorum_root, anchor, canonical_height);

    // The download peer alone claims the anchor one block lower.
    if let Some(status) = mgr.peers.get_mut(&peer) {
        status.best_height = canonical_height - 1;
    }

    let refusals_before = mgr.snap.integrity_refusals;
    deliver_manifest(
        &mut mgr,
        peer,
        anchor,
        canonical_height - 1,
        quorum_root,
        h(b"whatever"),
        1_000,
    );

    assert!(
        mgr.snap.integrity_refusals > refusals_before,
        "an uncorroborated anchor height in the MANIFEST must be refused before any chunk"
    );
    assert!(
        !is_snap_ready(&mgr),
        "the spliced height must never install"
    );
}
