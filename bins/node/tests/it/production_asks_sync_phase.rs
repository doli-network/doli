//! INC-I-295: above the gate no sync phase stops a build; below the gate a sync
//! phase stops it. Every test drives the real `try_produce_block`.
//!
//! covers: first_reply production_gate production_asks force_sync_phase_for_test
//
// OUTPUT CONTRACT: fn Node::try_produce_block(&mut self) -> Result<()>
//   O2: self.last_produced_slot — Some(block slot) iff minted, else unchanged (None)
//   O2: self.chain_state best_height / best_hash — tip+1 and the block hash iff minted
//   O3: return — Ok(()) on every path; asserted inside `drive` on every poll
//   O4: block_store[tip+1] — Some(block with parent = old tip, by this key) iff minted
//   O1, O5, O6, O7: none here; `production_asks_chosen_branch` covers the send.
// PATHS: P1 proceed (mint) | P2 stopped by can_produce with BlockedSyncing
// INPUT PARTITIONS (next height >= 10, latch spent):
//   A..E: each of the 5 phases, above the gate, peers 20 taller -> P1
//   F: DownloadingHeaders, below the gate, peer gap 5           -> P2
// MATRIX: {last_produced_slot, tip, block_store} x {A..E} by `assert_minted`, x {F} by
//   `assert_not_minted`; O3 in `drive`. A..E first show the phase and
//   can_produce == Authorized. F first shows can_produce == Authorized with no phase, so
//   the phase alone turns it to BlockedSyncing; `below_gate_keeps_old_peer_rule`
//   partition H shows the same gap mints with no phase.
// RIG: `production_asks_chosen_branch`.
// REQ-PASK-SEC-003 — Decision: a grep hit in the names and messages of this file would reveal a public-detail leak; checked by grep, no runtime test.

use network::{ProductionAuthorization, SyncPhase, SyncState};

use super::inc_i_289_common::BELOW_AH;
use super::production_asks_chosen_branch::{
    assert_gate_authorizes, assert_minted, assert_not_minted, assert_peer_gap, current_slot, drive,
    in_child_of, level_peer_with_tip_at, reply, rig_at, taller_peers, Rig, ABOVE_GATE, MINT_BUDGET,
    NO_MINT_BUDGET, TIP_ABOVE_TEN,
};

const MODULE: &str = "production_asks_sync_phase";
const TALL_GAP: u64 = 20;
const SMALL_GAP: u64 = 5;

/// Sets the phase and shows that the node holds it.
async fn force_phase(r: &Rig, phase: SyncPhase, partition: &str) {
    let mut sm = r.node.sync_manager.write().await;
    sm.force_sync_phase_for_test(phase.clone());
    match sm.state() {
        SyncState::Syncing { phase: held, .. } => {
            assert_eq!(held, &phase, "{partition}: precondition: phase");
        }
        other => panic!("{partition}: precondition: not syncing: {other:?}"),
    }
}

async fn assert_builds_in_phase(test: &str, phase: SyncPhase) {
    if !in_child_of(MODULE, test, ABOVE_GATE) {
        return;
    }
    let mut r = rig_at(TIP_ABOVE_TEN).await;
    let peer = taller_peers(&r, TIP_ABOVE_TEN + TALL_GAP).await;
    reply(&r, peer).await;
    force_phase(&r, phase, test).await;
    assert_peer_gap(&r, TALL_GAP, test).await;
    assert_gate_authorizes(&r, test).await;

    let minted = drive(&mut r, MINT_BUDGET).await;

    assert!(minted, "{test}: no mint above the gate in this sync phase");
    assert_minted(&r, test).await;
}

// REQ-PASK-006 — Decision: a failure means a node in the header phase cannot build on its branch above the gate, so a peer height claim stops it through the phase.
#[tokio::test]
async fn producer_keeps_building_in_downloading_headers() {
    assert_builds_in_phase(
        "producer_keeps_building_in_downloading_headers",
        SyncPhase::DownloadingHeaders,
    )
    .await;
}

// REQ-PASK-006 — Decision: a failure means a node in the body phase cannot build on its branch above the gate, so a peer height claim stops it through the phase.
#[tokio::test]
async fn producer_keeps_building_in_downloading_bodies() {
    assert_builds_in_phase(
        "producer_keeps_building_in_downloading_bodies",
        SyncPhase::DownloadingBodies,
    )
    .await;
}

// REQ-PASK-006 — Decision: a failure means a node in the block phase cannot build on its branch above the gate, so a peer height claim stops it through the phase.
#[tokio::test]
async fn producer_keeps_building_in_processing_blocks() {
    assert_builds_in_phase(
        "producer_keeps_building_in_processing_blocks",
        SyncPhase::ProcessingBlocks,
    )
    .await;
}

// REQ-PASK-006 — Decision: a failure means a node that collects snapshot votes cannot build on its branch above the gate, so a peer height claim stops it through the phase.
#[tokio::test]
async fn producer_keeps_building_in_snap_collecting() {
    assert_builds_in_phase(
        "producer_keeps_building_in_snap_collecting",
        SyncPhase::SnapCollecting,
    )
    .await;
}

// REQ-PASK-006 — Decision: a failure means a node that downloads a snapshot cannot build on its branch above the gate, so a peer height claim stops it through the phase.
#[tokio::test]
async fn producer_keeps_building_in_snap_downloading() {
    assert_builds_in_phase(
        "producer_keeps_building_in_snap_downloading",
        SyncPhase::SnapDownloading,
    )
    .await;
}

// REQ-PASK-006 — Decision: a failure means the change leaked below its activation height, so a syncing node on history before the gate builds without a gate.
#[tokio::test]
async fn syncing_below_gate_stops_the_build() {
    if !in_child_of(MODULE, "syncing_below_gate_stops_the_build", BELOW_AH) {
        return;
    }
    let partition = "F";
    let mut r = rig_at(TIP_ABOVE_TEN).await;
    let peer = level_peer_with_tip_at(&r, TIP_ABOVE_TEN + SMALL_GAP).await;
    reply(&r, peer).await;
    assert_peer_gap(&r, SMALL_GAP, partition).await;
    assert_gate_authorizes(&r, partition).await;

    force_phase(&r, SyncPhase::DownloadingHeaders, partition).await;
    let slot = current_slot(&r.node);
    let auth = r.node.sync_manager.write().await.can_produce(slot);
    assert_eq!(auth, ProductionAuthorization::BlockedSyncing, "F: the gate");

    let held = !drive(&mut r, NO_MINT_BUDGET).await;

    assert!(held, "F: minted below the gate in a sync phase");
    assert_not_minted(&r, partition).await;
}
