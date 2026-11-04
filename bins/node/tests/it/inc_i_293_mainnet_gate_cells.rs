//! INC-I-293 ZB-2: mainnet edge cells for `inc_i_288_fork_choice_activation_height`.
//! The gate is read from `NetworkParams::defaults(Network::Mainnet)` (mainnet
//! refuses an env override) and is fed to a `SyncManager` through the same three
//! `SyncConfig` gate fields `Node::new` sets. Each door runs at a synthetic tip of
//! 881_799 (gate - 1: OFF) and 881_800 (gate: ON); no chain is built.
//
// OUTPUT CONTRACT (outputs x paths x heights):
//   door                                     output                            881_799        881_800
//   O0 NetworkParams::defaults(Mainnet)      .inc_i_288_..._activation_height  == 881_800 (every cell)
//   O1 reorg_handler().weighs_from_ancestor  bool (doors, branch_rank, wedge)  false          true
//   O2 finality sink (K-2 persist)           checkpoints handed to the sink    []             [(tip-2, F)]
//   O3 last_finalized_height (in memory)     Option<u64>                       Some(tip-2)    Some(tip-2)
//   O4 classify_and_dispatch, Rule 1         RecoveryAction (2 orphan reports) HeaderFirst*   not ShallowRb
//   O5 classify_and_dispatch, Rule 1b        RecoveryAction (1 stuck fork)     HeaderFirst*   not ShallowRb
//   * INC-I-294 M-2: no header proof for the tip, so no depth-1 rollback below the pin;
//     with a fresh proof for the tip, O4/O5 at 881_799 = ShallowRb{1} (the old rule).
//   O6 LRU dedup (block's own height)        knows_block(A) after LRU_CAP      false          true
//                                            re-records of child B
//   PATHS: gate - 1 | gate. Not here: the init.rs finality restore (needs a stored
//   mainnet chain at 881_800; its operator is pinned by j4_finality_restart_*).
// INPUT PARTITIONS: compared height = 881_799 (one below the pin) | = 881_800 (the
//   pin). Rollback evidence: two orphan reports (Rule 1) | one stuck-fork report
//   (Rule 1b), both with gap 1 and a fresh apply. Finality: F at tip-2 with 1/1
//   weight. LRU: A recorded once, child B recorded LRU_CAP times.

use crypto::{Hash, KeyPair};
use doli_core::{FinalityCheckpoint, Network, NetworkParams};
use network::{PeerId, RecoveryAction, SyncConfig, SyncManager};
use std::sync::{Arc, Mutex};

const PIN: u64 = 881_800;
const OFF: u64 = PIN - 1;
/// `ReorgHandler::new()` tracks this many LRU slots.
const LRU_CAP: usize = 10_000;
const HEADER_FIRST: RecoveryAction = RecoveryAction::HeaderFirstSync;

fn mainnet_gate() -> u64 {
    let gate = NetworkParams::defaults(Network::Mainnet).inc_i_288_fork_choice_activation_height;
    assert_eq!(gate, PIN, "mainnet inc_i_288 pin moved");
    gate
}

/// The three fork-choice gate fields `Node::new` copies into `SyncConfig`.
fn mainnet_sync() -> SyncManager {
    let p = NetworkParams::defaults(Network::Mainnet);
    let config = SyncConfig {
        inc_i_147_activation_height: p.inc_i_147_activation_height,
        inc_i_204_fork_choice_activation_height: p.inc_i_204_fork_choice_activation_height,
        inc_i_288_fork_choice_activation_height: mainnet_gate(),
        ..SyncConfig::default()
    };
    SyncManager::new(config, Hash::ZERO)
}

fn h(tag: &str) -> Hash {
    crypto::hash::hash(tag.as_bytes())
}

/// O2 + O3: F tracked at `tip - 2` with 1/1 weight, attested at applied tip `tip`.
fn finality_at(tip: u64) -> (Vec<(u64, Hash)>, Option<u64>) {
    let mut sm = mainnet_sync();
    let sunk = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&sunk);
    sm.set_finality_sink(Box::new(move |cp: &FinalityCheckpoint| {
        sink.lock().unwrap().push((cp.height, cp.block_hash))
    }));
    let f = h("F");
    sm.track_block_for_finality(f, tip - 2, 1, 1);
    sm.update_local_tip(tip, h("tip"), 3);
    sm.add_attestation_weight(&f, *KeyPair::generate().public_key(), 1);
    let got = sunk.lock().unwrap().clone();
    (got, sm.last_finalized_height())
}

enum Evidence {
    Orphans,
    StuckFork,
}

/// O4 / O5: gap 1 (one peer at `tip + 1`), fresh apply, no finality, no rollback yet.
fn classify_at(tip: u64, ev: Evidence) -> RecoveryAction {
    classify_at_with(tip, ev, false)
}

/// `classify_at`; with `proof`, a fresh header proof for `tip` is reported first.
fn classify_at_with(tip: u64, ev: Evidence, proof: bool) -> RecoveryAction {
    let mut sm = mainnet_sync();
    sm.update_local_tip(tip, h("tip"), 10);
    sm.add_peer(PeerId::random(), tip + 1, h("peer tip"), 11);
    match ev {
        Evidence::Orphans => {
            sm.report_orphan_gossip(11, 1);
            sm.report_orphan_gossip(11, 1);
        }
        Evidence::StuckFork => sm.report_stuck_fork(1),
    }
    if proof {
        sm.report_header_divergence(PeerId::random(), tip);
    }
    sm.classify_and_dispatch(0).0
}

/// O6: A at `child_h - 1` once, its child B at `child_h` LRU_CAP times.
/// Returns (A still tracked, B still tracked).
fn lru_after_rerecords(child_h: u64) -> (bool, bool) {
    let mut sm = mainnet_sync();
    let (a, b) = (h("A"), h("B"));
    sm.block_applied_with_weight(a, child_h - 1, 1, 1, Hash::ZERO);
    for _ in 0..LRU_CAP {
        sm.block_applied_with_weight(b, child_h, 2, 1, a);
    }
    let rh = sm.reorg_handler();
    (rh.knows_block(&a), rh.knows_block(&b))
}

// INC-I-293 ZB-2 PIN — Decision: a failure means someone moved the mainnet pin, so every cell below now tests a height mainnet does not run.
#[test]
fn zb2_mainnet_pin_matches_cells() {
    assert_eq!(mainnet_gate(), 881_800);
    assert_eq!(OFF, 881_799);
}

// INC-I-293 ZB-2 DOORS-OFF — Decision: a failure means the bond-weighted doors open one block before the mainnet pin.
#[test]
fn zb2_doors_weigh_by_count_below_gate() {
    let on = mainnet_sync().reorg_handler().weighs_from_ancestor(OFF);
    assert!(!on, "DOORS-OFF: tip 881_799 weighs from the ancestor");
}

// INC-I-293 ZB-2 DOORS-ON — Decision: a failure means the mainnet doors still weigh by block count at tip 881_800 (the `>=` became `>` or the pin did not reach SyncConfig).
#[test]
fn zb2_doors_weigh_from_ancestor_at_gate() {
    let on = mainnet_sync().reorg_handler().weighs_from_ancestor(PIN);
    assert!(on, "DOORS-ON: tip 881_800 still weighs by block count");
}

// INC-I-293 ZB-2 FIN-OFF — Decision: a failure means a mainnet node persists a finality checkpoint one block before the pin.
#[test]
fn zb2_finality_not_persisted_below_gate() {
    let (sunk, fin) = finality_at(OFF);
    assert_eq!(sunk, vec![], "FIN-OFF: checkpoint persisted at tip 881_799");
    assert_eq!(fin, Some(OFF - 2), "FIN-OFF: in-memory finality changed");
}

// INC-I-293 ZB-2 FIN-ON — Decision: a failure means a mainnet node at tip 881_800 does not persist finality, so a restart at the pin forgets it.
#[test]
fn zb2_finality_persisted_at_gate() {
    let (sunk, fin) = finality_at(PIN);
    let want = vec![(PIN - 2, h("F"))];
    assert_eq!(
        sunk, want,
        "FIN-ON: checkpoint not persisted at tip 881_800"
    );
    assert_eq!(fin, Some(PIN - 2), "FIN-ON: in-memory finality changed");
}

// INC-I-293 ZB-2 BLIND-OFF + INC-I-294 M-2 — Decision: a failure means a mainnet node below the pin still rolls back its tip on counters alone, with no header proof that a peer's chain leaves it.
#[test]
fn zb2_blind_rollback_needs_header_proof_below_gate() {
    assert_eq!(
        classify_at(OFF, Evidence::Orphans),
        HEADER_FIRST,
        "BLIND-OFF Rule 1 without header proof"
    );
    assert_eq!(
        classify_at(OFF, Evidence::StuckFork),
        HEADER_FIRST,
        "BLIND-OFF Rule 1b without header proof"
    );
    for (ev, rule) in [(Evidence::Orphans, "1"), (Evidence::StuckFork, "1b")] {
        assert_eq!(
            classify_at_with(OFF, ev, true),
            RecoveryAction::ShallowRollback { depth: 1 },
            "BLIND-OFF Rule {rule} with a fresh header proof for the tip"
        );
    }
}

// INC-I-293 ZB-2 BLIND-ON — Decision: a failure means a mainnet node at tip 881_800 still rolls back without weighing (the INC-I-289 blind door).
#[test]
fn zb2_blind_rollback_closed_at_gate() {
    for (ev, rule) in [(Evidence::Orphans, "1"), (Evidence::StuckFork, "1b")] {
        let got = classify_at(PIN, ev);
        assert!(
            !matches!(got, RecoveryAction::ShallowRollback { .. }),
            "BLIND-ON Rule {rule}: tip 881_800 still rolls back blind: {got:?}"
        );
    }
}

// INC-I-293 ZB-2 LRU-OFF — Decision: a failure means re-records of one block stop pushing LRU slots before the pin (old eviction behaviour changed below it).
#[test]
fn zb2_lru_rerecord_evicts_below_gate() {
    let got = lru_after_rerecords(OFF);
    assert_eq!(got, (false, true), "LRU-OFF: (A tracked, B tracked)");
}

// INC-I-293 ZB-2 LRU-ON — Decision: a failure means re-records of one block at height 881_800 still evict other tracked blocks.
#[test]
fn zb2_lru_rerecord_keeps_slot_at_gate() {
    let got = lru_after_rerecords(PIN);
    assert_eq!(got, (true, true), "LRU-ON: (A tracked, B tracked)");
}
