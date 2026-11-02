//! INC-I-289 starvation: a peer claims a height 3 above ours that it never
//! delivers. `add_peer` -> `should_sync` (gap >= 3) -> `start_sync` puts the
//! SyncManager in `SyncState::Syncing`, and `can_produce` Check 1 returns
//! `BlockedSyncing` for every slot: producers on the live branch log
//! `BLOCKED: Syncing` and the branch starves. Contract at the gate: an
//! undeliverable claim does not take the producer's authorization; a forced
//! resync (`ResyncInProgress`) still blocks. Below the gate: old rule.
//!
// OUTPUT CONTRACT: fn SyncManager::can_produce(slot) -> ProductionAuthorization
//   (after SyncManager::add_peer -> should_sync -> start_sync)
//   O1: return value ProductionAuthorization (not BlockedSyncing | BlockedSyncing)
//   O2: SyncManager state (is_syncing) — the path precondition
// PATHS: P1 Syncing from one peer claim, nothing delivered; P2 ResyncInProgress
// INPUT PARTITIONS: D1 at/above AH, Syncing on an undeliverable claim -> authorized;
//   D2 at/above AH, start_resync (not Syncing) -> BlockedSyncing (kept);
//   D0 below AH (child re-exec), D1 shape -> BlockedSyncing (old rule)
// MATRIX: D1 O1,O2 · D2 O1,O2 · D0 O1,O2
//
//! covers: production_gate.rs (Check 1) peers.rs (add_peer) decision.rs

use network::ProductionAuthorization;

use super::inc_i_289_common::*;

/// Tip D at F+1 (slot 210); one peer claims F+4 and delivers nothing.
/// Returns (rig, is_syncing after the claim, authorization for slot 211).
async fn undeliverable_claim() -> (Rig, bool, ProductionAuthorization) {
    let r = rig(&[210]).await;
    peer_at(&r, F + 4, crypto::hash::hash(b"undeliverable H+3"), 214).await;
    let mut sm = r.node.sync_manager.write().await;
    let syncing = sm.state().is_syncing();
    let auth = sm.can_produce(211);
    drop(sm);
    (r, syncing, auth)
}

// D1 INC-I-289 — Decision: a failure means one peer's undeliverable height claim still blocks every producer with BlockedSyncing, starving the live branch (the h=835,607 lock).
#[tokio::test]
async fn d1_inc_i_289_producer_keeps_authorization_while_peer_claims_undeliverable_height() {
    let (_r, syncing, auth) = undeliverable_claim().await;
    eprintln!("[D1] syncing={syncing} auth={auth:?}");

    assert!(
        syncing,
        "precondition: the claim put the SyncManager in Syncing"
    );
    assert_ne!(
        auth,
        ProductionAuthorization::BlockedSyncing,
        "D1: production blocked by a claim no peer delivers"
    );
}

// D2 INC-I-289 — Decision: a failure means the fix also removed the forced-resync production block, so a node mid-resync produces on a state it is replacing.
#[tokio::test]
async fn d2_inc_i_289_control_resync_in_progress_still_blocks() {
    let r = rig(&[210]).await;
    let mut sm = r.node.sync_manager.write().await;
    sm.start_resync();
    let syncing = sm.state().is_syncing();
    let auth = sm.can_produce(211);
    drop(sm);

    assert!(!syncing, "precondition: only the resync phase blocks here");
    assert_eq!(
        auth,
        ProductionAuthorization::BlockedSyncing,
        "D2: ResyncInProgress must block production"
    );
}

// D0 INC-I-289 — Decision: a failure means the production-gate change leaked below its activation height, so the pre-gate Check 1 changed without a gate.
#[tokio::test]
async fn d0_inc_i_289_below_ah_syncing_still_blocks() {
    if !in_below_ah_child(
        "inc_i_289_d_producer_not_starved::d0_inc_i_289_below_ah_syncing_still_blocks",
    ) {
        return;
    }
    let (_r, syncing, auth) = undeliverable_claim().await;

    assert!(
        syncing,
        "precondition: the claim put the SyncManager in Syncing"
    );
    assert_eq!(
        auth,
        ProductionAuthorization::BlockedSyncing,
        "D0 below AH: Syncing blocks production (old rule)"
    );
}
