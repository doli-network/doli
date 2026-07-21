//! INC-I-143 M4 — F4 snap-anchor integrity (defects D1 + D2 anchor-height).
//! run_id 466.
//!
//! Reproduces the two pre-existing snap-admission defects that spliced seed1 at
//! the wrong height during the 2026-07-21 fleet fork (diagnosis [E7], causal-chain
//! link 7):
//!   D1: `handle_snap_snapshot` computed the independent `quorum_root` reference,
//!       observed `response_root != quorum_root`, logged it at info!, and ACCEPTED
//!       the snapshot anyway — discarding the one cross-peer check that would have
//!       caught a forked anchor.
//!   D2 (anchor-height aspect): the anchor install height was taken from the serving
//!       peer's verbatim claim (serve side serves `cs.best_height` for the current
//!       tip regardless of requested hash). Measured damage: anchor `35574faf…`
//!       installed at 108505 while canonically at 108506 → permanent −1 offset +
//!       45-block hole.
//!
//! F4 hardens ADMISSION only (node-local sync behavior; NO block-content change, NO
//! BlockHeader height field, NO change to apply_block's best_height+1 derivation):
//!   Gate 1 (D1): refuse the snapshot when `response_root != quorum_root`.
//!   Gate 2 (D2): refuse unless the served anchor `(block_hash, block_height)` is
//!                corroborated by a STATUS quorum of current peers — the height the
//!                network associates with that hash is the height to install, never a
//!                single peer's uncorroborated claim.
//!
//! ── OUTPUT CONTRACT ─────────────────────────────────────────────────────────
//! OUTPUT UNDER TEST: the terminal `pipeline_data` of `SyncManager` after one call
//!   to `handle_snap_snapshot` while in the `SnapDownloading` phase.
//!   Domain = {admit → SnapReady} ∪ {refuse → not SnapReady (error/fallback path)}.
//! PATH (single sink): SyncResponse::StateSnapshot → response.rs → handle_snap_snapshot.
//! INPUT PARTITIONS (× served-anchor shape):
//!   P1 root-mismatch     : response_root ≠ quorum_root                → MUST refuse (Gate 1)
//!   P2 uncorroborated-h  : root matches, (hash,height) below STATUS quorum → MUST refuse (Gate 2)
//!   P3 corroborated      : root matches AND (hash,height) ≥ STATUS quorum  → MUST admit (control)
//! ASSERTIONS are on the behavioral observable (SnapReady vs not) that exists in the
//!   current API, so the reproduction tests compile against pre-fix code and FAIL by
//!   assertion (current code admits all three). Fix confidence >0.7: FAIL→PASS shown.
//! ────────────────────────────────────────────────────────────────────────────

use crypto::Hash;
use libp2p::PeerId;
use std::time::Instant;

use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

/// Build a SyncManager parked in `SnapDownloading` with `quorum_root`, and a
/// STATUS peer table: `n_majority` peers reporting `(maj_hash, maj_height)` plus
/// one download peer reporting `(dl_hash, dl_height)`. Returns the download PeerId.
#[allow(clippy::too_many_arguments)]
fn mgr_downloading(
    local_height: u64,
    target_height: u64,
    quorum_root: Hash,
    n_majority: usize,
    maj_hash: Hash,
    maj_height: u64,
    dl_hash: Hash,
    dl_height: u64,
) -> (SyncManager, PeerId) {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = local_height;
    mgr.local_slot = local_height as u32;

    // Network majority: peers whose STATUS tip corroborates (maj_hash, maj_height).
    for _ in 0..n_majority {
        mgr.add_peer(PeerId::random(), maj_height, maj_hash, maj_height as u32);
    }
    // The download peer: its own gossiped STATUS is (dl_hash, dl_height).
    let dl_peer = PeerId::random();
    mgr.add_peer(dl_peer, dl_height, dl_hash, dl_height as u32);

    mgr.pipeline_data = SyncPipelineData::SnapDownloading {
        target_hash: maj_hash,
        target_height,
        quorum_root,
        peer: dl_peer,
        alternate_peers: vec![],
    };
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::SnapDownloading,
        started_at: Instant::now(),
    };
    (mgr, dl_peer)
}

fn is_snap_ready(mgr: &SyncManager) -> bool {
    matches!(mgr.pipeline_data, SyncPipelineData::SnapReady { .. })
}

/// P1 (Gate 1 / D1): a snapshot whose `response_root` disagrees with the
/// quorum-agreed `quorum_root` MUST be refused — even when its (hash, height)
/// is fully corroborated. Current code logs the mismatch and admits → FAILS now.
#[test]
fn f4_response_root_mismatch_refuses_snapshot() {
    let quorum_root = crypto::hash::hash(b"quorum_agreed_root");
    let response_root = crypto::hash::hash(b"forked_peer_advanced_root"); // != quorum_root
    let anchor = crypto::hash::hash(b"anchor_35574faf");

    // Fully corroborated (hash, height) so ONLY the root mismatch can refuse.
    let (mut mgr, dl) = mgr_downloading(
        108_400,
        108_506,
        quorum_root,
        4,
        anchor,
        108_506,
        anchor,
        108_506,
    );

    mgr.handle_snap_snapshot(
        dl,
        anchor,
        108_506,
        vec![],
        vec![],
        vec![],
        response_root,
        None,
        None,
        None,
        None,
    );

    assert!(
        !is_snap_ready(&mgr),
        "F4 Gate 1: snapshot with response_root != quorum_root MUST be refused, not stored as SnapReady"
    );
}

/// P2 (Gate 2 / D2): the incident shape. Root matches the quorum, but the serving
/// peer claims the anchor at height N-1 (108505) while the network STATUS majority
/// has it at N (108506). The uncorroborated -1 height MUST be refused. Current code
/// installs the peer's verbatim height → FAILS now (this is the splice reproduction).
#[test]
fn f4_uncorroborated_minus_one_anchor_height_refused() {
    let quorum_root = crypto::hash::hash(b"quorum_agreed_root");
    let anchor = crypto::hash::hash(b"anchor_35574faf");
    let canonical_height = 108_506;
    let bad_height = 108_505; // the measured -1 offset

    // 4 majority peers corroborate (anchor, 108506); the download peer alone
    // reports (anchor, 108505) — a single uncorroborated claim.
    let (mut mgr, dl) = mgr_downloading(
        108_400,
        canonical_height,
        quorum_root,
        4,
        anchor,
        canonical_height,
        anchor,
        bad_height,
    );

    // Gate 1 passes (response_root == quorum_root); only Gate 2 can refuse.
    mgr.handle_snap_snapshot(
        dl,
        anchor,
        bad_height,
        vec![],
        vec![],
        vec![],
        quorum_root,
        None,
        None,
        None,
        None,
    );

    assert!(
        !is_snap_ready(&mgr),
        "F4 Gate 2: anchor height uncorroborated by STATUS quorum (1 peer @ N-1 vs majority @ N) MUST be refused"
    );
}

/// P3 (control): a snapshot whose root matches the quorum AND whose (hash, height)
/// is corroborated by a STATUS quorum MUST be admitted at the corroborated height.
/// Proves F4 does not falsely refuse a good anchor (behavior identical to pre-fix
/// for corroborated inputs — INC-I-075 Q3 evidence).
#[test]
fn f4_corroborated_anchor_admitted_at_quorum_height() {
    let quorum_root = crypto::hash::hash(b"quorum_agreed_root");
    let anchor = crypto::hash::hash(b"anchor_35574faf");
    let height = 108_506;

    let (mut mgr, dl) = mgr_downloading(
        108_400,
        height,
        quorum_root,
        4,
        anchor,
        height,
        anchor,
        height,
    );

    mgr.handle_snap_snapshot(
        dl,
        anchor,
        height,
        vec![],
        vec![],
        vec![],
        quorum_root,
        None,
        None,
        None,
        None,
    );

    assert!(
        is_snap_ready(&mgr),
        "F4: a root-matching, STATUS-corroborated anchor MUST be admitted as SnapReady"
    );
    if let SyncPipelineData::SnapReady { snapshot } = &mgr.pipeline_data {
        assert_eq!(
            snapshot.block_height, height,
            "F4: admitted anchor must install at the quorum-corroborated height"
        );
    }
}
