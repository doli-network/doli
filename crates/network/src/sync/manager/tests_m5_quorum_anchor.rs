//! UTXO-scalability M5 / INC-I-231 — manifest admission is anchored on the QUORUM ANCHOR,
//! not on per-peer `PeerSyncStatus`.
//!
//! The defect: `handle_state_manifest` corroborates the manifest anchor by counting peers
//! whose STORED `(best_hash, best_height)` equals the manifest's. That status is refreshed
//! only by periodic status responses, so on a 10 s-slot chain the stored tuples lag the
//! moving tip and a manifest served AT the quorum height matches 0 peers even when every
//! peer served exactly that manifest. Runtime evidence (N6, h=3505): quorum reached on
//! root=4e246f28…, then four IDENTICAL manifests refused with `corroborated by 0/4`, three
//! snap attempts burned, fallback to header-first sync.
//!
//! OUTPUT CONTRACT: `SyncManager::handle_state_manifest` plus the outbound `next_request`.
//!   Outputs observable from the stable API:
//!     O1 `self.state_session`            — was a session opened
//!     O2 `self.pipeline_data`            — SnapDownloading kept, or torn down / SnapReady
//!     O3 the NEXT outbound `SyncRequest` — is a chunk actually requested
//!     O4 `self.snap.integrity_refusals`  — was a refusal COUNTED
//!     O5 `self.snap.blacklisted_peers`   — was the serving peer punished
//!     O6 `self.snap.attempts`            — was a snap attempt consumed
//!   Paths (all with STALE stored statuses — the production condition):
//!     P1 manifest == (target_hash, target_height, quorum_root)           — ADMIT
//!     P2 root == quorum_root, height == target-1, hash == anchor         — REFUSE (anchor)
//!     P3 root == quorum_root, height == target, hash != target_hash      — REFUSE (anchor)
//!     P4 root != quorum_root, height == target                           — REFUSE (root)
//!     P5 root != quorum_root, height == target+1 (peer advanced)         — RETRY, not blame
//!   MATRIX: P1x{O1,O2,O3,O4,O5} | P2x{O1,O3,O4} | P3x{O1,O3,O4} | P4x{O1,O4}
//!           P5x{O1,O4,O5,O6}
//! INPUT PARTITIONS: the stored `PeerSyncStatus` of EVERY peer is either one block BEHIND
//!   the anchor (with a foreign hash) or one block AHEAD of it — never equal to the anchor.
//!   That is the whole point: the M3 fixture makes every status corroborate the anchor,
//!   which is exactly why the M3 tests pass while production refuses. A fixture whose
//!   statuses match the anchor cannot falsify this defect.
//!
//! The faithful corroboration source is the quorum anchor already stored in
//! `SyncPipelineData::SnapDownloading { target_hash, target_height, quorum_root }`
//! at "Quorum reached" (`snap_sync.rs`). No wire format changes.

use crypto::Hash;
use libp2p::PeerId;
use std::time::Instant;

use crate::protocols::sync::{SyncRequest, STATE_CHUNK_MAX_BYTES};
use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

const SESSION_ID: u64 = 0xD0_11_5E_55_10_00_00_05;

/// The height N6 reached quorum on in `docs/.workflow/runtime-evidence.md`.
const ANCHOR_HEIGHT: u64 = 3_505;

fn h(tag: &[u8]) -> Hash {
    crypto::hash::hash(tag)
}

/// A manager parked in `SnapDownloading` at a quorum anchor, with every peer's STORED
/// status lagging or leading that anchor — the 10 s-slot status lag of INC-I-231.
fn mgr_anchor_stale_statuses(
    quorum_root: Hash,
    anchor: Hash,
    height: u64,
) -> (SyncManager, PeerId) {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = height.saturating_sub(1_000);
    mgr.local_slot = mgr.local_height as u32;

    let behind = h(b"m5_tip_one_block_behind");
    let ahead = h(b"m5_tip_one_block_ahead");
    for _ in 0..2 {
        mgr.add_peer(PeerId::random(), height - 1, behind, (height - 1) as u32);
    }
    mgr.add_peer(PeerId::random(), height + 1, ahead, (height + 1) as u32);
    let peer = PeerId::random();
    mgr.add_peer(peer, height + 1, ahead, (height + 1) as u32);

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

/// The fixture is only evidence if NO stored status equals the anchor.
fn assert_statuses_are_stale(mgr: &SyncManager, anchor: Hash, height: u64) {
    assert!(
        mgr.peers
            .values()
            .all(|s| !(s.best_hash == anchor && s.best_height == height)),
        "fixture invalid: a stored PeerSyncStatus already equals the anchor, so the 10 s-slot \
         status lag this milestone exists for is not modelled"
    );
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

fn is_snap_ready(mgr: &SyncManager) -> bool {
    matches!(mgr.pipeline_data, SyncPipelineData::SnapReady { .. })
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

// ==================== P1 — the reproduction ====================

/// REQ-SCALE-002 — Decision: a failure here means a correct snapshot offer is refused purely
/// because gossip status refresh is slower than the 10 s slot, so no node can ever snap-sync
/// a live chain and every join degrades to header-first replay.
/// REQ-SCALE-013 — Decision: a failure here means the bounded chunked transfer is unreachable
/// at any UTXO-set size, because the session is refused before the first chunk is requested.
#[test]
fn inc_i_231_manifest_at_quorum_anchor_is_admitted_despite_stale_peer_status() {
    let quorum_root = h(b"m5_quorum_root");
    let anchor = h(b"m5_anchor");
    let (mut mgr, peer) = mgr_anchor_stale_statuses(quorum_root, anchor, ANCHOR_HEIGHT);
    assert_statuses_are_stale(&mgr, anchor, ANCHOR_HEIGHT);

    let refusals_before = mgr.snap.integrity_refusals;
    deliver_manifest(
        &mut mgr,
        peer,
        anchor,
        ANCHOR_HEIGHT,
        quorum_root,
        h(b"m5_utxo_hash"),
        1_000,
    );

    assert!(
        mgr.state_session.is_some(),
        "a manifest EQUAL to the quorum anchor (hash, height, root) must open a session; \
         refusing it because per-peer status lags the tip is INC-I-231"
    );
    assert_eq!(
        mgr.snap.integrity_refusals, refusals_before,
        "an anchor-equal manifest is not an integrity event — counting it as one hides real \
         refusals behind status-refresh latency"
    );
    assert!(
        !mgr.snap.blacklisted_peers.contains(&peer),
        "the peer served exactly the quorum anchor; blacklisting it removes honest peers \
         from a small fleet. Blacklisted: {:?}",
        mgr.snap.blacklisted_peers
    );

    let next = next_request(&mut mgr, peer).expect("an admitted session must request a chunk");
    match next {
        SyncRequest::GetStateChunk {
            session_id,
            max_bytes,
            ..
        } => {
            assert_eq!(
                session_id, SESSION_ID,
                "the session id must be carried back"
            );
            assert_eq!(max_bytes, STATE_CHUNK_MAX_BYTES);
        }
        other => panic!("expected GetStateChunk after admission, got {:?}", other),
    }
}

// ==================== P2 / P3 — the anchor gate survives ====================

/// REQ-SCALE-006 — Decision: a failure here means the INC-I-143 anchor defence was traded
/// away to fix the false refusal, and a spliced height (the -1 anchor) installs a state the
/// quorum never agreed to.
#[test]
fn inc_i_231_manifest_with_quorum_root_but_wrong_anchor_is_refused() {
    let quorum_root = h(b"m5_quorum_root");
    let anchor = h(b"m5_anchor");

    // Variant 1: right hash, height spliced one block lower.
    let (mut mgr, peer) = mgr_anchor_stale_statuses(quorum_root, anchor, ANCHOR_HEIGHT);
    let refusals_before = mgr.snap.integrity_refusals;
    deliver_manifest(
        &mut mgr,
        peer,
        anchor,
        ANCHOR_HEIGHT - 1,
        quorum_root,
        h(b"m5_whatever"),
        1_000,
    );
    assert!(
        mgr.snap.integrity_refusals > refusals_before,
        "a manifest whose height != the quorum anchor height must be COUNTED as a refusal"
    );
    assert!(
        mgr.state_session.is_none(),
        "a spliced anchor height must never open a session"
    );
    assert!(
        !is_snap_ready(&mgr),
        "the spliced height must never install"
    );
    let next = next_request(&mut mgr, peer);
    assert!(
        !matches!(next, Some(SyncRequest::GetStateChunk { .. })),
        "not a single chunk may be requested against a spliced anchor, got {:?}",
        next
    );

    // Variant 2: right height, foreign block hash.
    let (mut mgr, peer) = mgr_anchor_stale_statuses(quorum_root, anchor, ANCHOR_HEIGHT);
    let refusals_before = mgr.snap.integrity_refusals;
    deliver_manifest(
        &mut mgr,
        peer,
        h(b"m5_foreign_block_hash"),
        ANCHOR_HEIGHT,
        quorum_root,
        h(b"m5_whatever"),
        1_000,
    );
    assert!(
        mgr.snap.integrity_refusals > refusals_before,
        "a manifest whose block hash != the quorum anchor hash must be COUNTED as a refusal — \
         the state root commits height and hash, so an honest peer cannot produce this pair"
    );
    assert!(
        mgr.state_session.is_none(),
        "a foreign anchor hash must never open a session"
    );
    let next = next_request(&mut mgr, peer);
    assert!(
        !matches!(next, Some(SyncRequest::GetStateChunk { .. })),
        "not a single chunk may be requested against a foreign anchor hash, got {:?}",
        next
    );
}

// ==================== P4 — the root gate survives ====================

/// REQ-SCALE-006 — Decision: a failure here means INC-I-143 F4 Gate 1 was lost while
/// replacing the status count, so a forked peer's snapshot is downloaded (and possibly
/// installed) at the anchor height with a root the quorum never voted for.
#[test]
fn inc_i_231_manifest_with_foreign_root_at_anchor_height_is_refused() {
    let quorum_root = h(b"m5_quorum_root");
    let forked_root = h(b"m5_forked_peer_root");
    let anchor = h(b"m5_anchor");
    let (mut mgr, peer) = mgr_anchor_stale_statuses(quorum_root, anchor, ANCHOR_HEIGHT);

    let refusals_before = mgr.snap.integrity_refusals;
    deliver_manifest(
        &mut mgr,
        peer,
        anchor,
        ANCHOR_HEIGHT,
        forked_root,
        h(b"m5_whatever"),
        1_000,
    );

    assert!(
        mgr.snap.integrity_refusals > refusals_before,
        "a manifest whose state_root != quorum_root at the anchor height must stay a counted \
         integrity refusal"
    );
    assert!(
        mgr.state_session.is_none(),
        "a foreign root must never open a session"
    );
    assert!(!is_snap_ready(&mgr), "a foreign root must never install");
}

// ==================== P5 — an advanced tip is a retry, not misbehaviour ====================

/// REQ-SCALE-006 — Decision: a failure here means a peer that simply produced the next block
/// between quorum and manifest is punished like an equivocator (F-08), and on the live 6-node
/// fleet a few such advances blacklist the whole peer set out of snap-sync (F-09: peer scoring
/// is NOT the mitigation).
#[test]
fn inc_i_231_tip_advanced_manifest_is_a_retry_not_misbehaviour() {
    let quorum_root = h(b"m5_quorum_root");
    let anchor = h(b"m5_anchor");
    let (mut mgr, peer) = mgr_anchor_stale_statuses(quorum_root, anchor, ANCHOR_HEIGHT);

    let refusals_before = mgr.snap.integrity_refusals;
    let attempts_before = mgr.snap.attempts;
    deliver_manifest(
        &mut mgr,
        peer,
        h(b"m5_tip_plus_one_hash"),
        ANCHOR_HEIGHT + 1,
        h(b"m5_tip_plus_one_root"),
        h(b"m5_whatever"),
        1_000,
    );

    assert!(
        !mgr.snap.blacklisted_peers.contains(&peer),
        "a peer whose tip ADVANCED past the quorum anchor must not be blacklisted. \
         Blacklisted: {:?}",
        mgr.snap.blacklisted_peers
    );
    assert_eq!(
        mgr.snap.integrity_refusals, refusals_before,
        "an advanced tip is not an integrity event — counting it makes the refusal metric \
         useless as a fork signal"
    );
    assert_eq!(
        mgr.snap.attempts,
        attempts_before + 1,
        "the advance must consume exactly ONE snap attempt through the pre-existing cap, so \
         the retry stays bounded without a new counter"
    );
    assert!(
        mgr.state_session.is_none(),
        "the advanced manifest must not open a session against the stale anchor"
    );
}

// ==================== The outcome-metric probe ====================

/// REQ-SCALE-002 — Decision: a failure here reproduces the N6 outcome exactly — four correct,
/// identical manifests at the quorum anchor, zero admitted — which is the externally
/// observable metric this milestone moves from 0 to 4.
#[test]
fn inc_i_231_probe_manifests_admitted() {
    let quorum_root = h(b"m5_quorum_root");
    let anchor = h(b"m5_anchor");
    let utxo_hash = h(b"m5_utxo_hash");

    // Each of the four peers is an INDEPENDENT admission test: admitting replaces the
    // session, so the manager is rebuilt between deliveries.
    let mut admitted = 0usize;
    for _ in 0..4 {
        let (mut mgr, peer) = mgr_anchor_stale_statuses(quorum_root, anchor, ANCHOR_HEIGHT);
        assert_statuses_are_stale(&mgr, anchor, ANCHOR_HEIGHT);
        deliver_manifest(
            &mut mgr,
            peer,
            anchor,
            ANCHOR_HEIGHT,
            quorum_root,
            utxo_hash,
            1_000,
        );
        if mgr.state_session.is_some() {
            admitted += 1;
        }
    }

    println!("M5_MANIFESTS_ADMITTED={}", admitted);
    assert_eq!(
        admitted, 4,
        "four identical manifests at the quorum anchor must ALL be admitted; {} were",
        admitted
    );
}
