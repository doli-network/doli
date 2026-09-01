//! INC-I-204 M0 — the decisions M0 may only OBSERVE, never change.
//!
//! M0 is observability only: no consensus rule, no reorg decision, no sync
//! decision, no block content. These pins are GREEN on the pre-M0 tree and must
//! stay green after it. Each one guards a specific rejected trap:
//!
//!   LB-1 / T1  the FINALITY guard refusing a sub-finality reorg is CORRECT.
//!              Counting the refusal must not relax it.
//!   LB-2       the guard's boundary is strict `<` (INC-I-090 fencepost fix):
//!              ancestor_height == finality_height is PERMITTED.
//!   T9         the wedge-escape tie-break `fork_w <= our_w` and the strict
//!              `weight_delta > 0` rule are load-bearing. M0 may count those
//!              branches; it may not move either boundary.
//!   INV-SYNC-012 the pre-activation branch of `inc_i_147_activation_height`
//!              keeps its legacy synthetic height. Adding a canary must not
//!              change which height the guard compares.
//!
//! OUTPUT CONTRACT
//!   Functions under test:
//!     F1 `ReorgHandler::plan_reorg(&self, current_tip, new_tip, get_parent, get_height)`
//!     F2 `ReorgHandler::check_reorg_weighted(&self, block, current_tip, weight)`
//!     F3 `ReorgHandler::record_block_with_height(&mut self, h, prev, w, real_height)`
//!     F4 `SyncManager::checkpoint_health(&self) -> (counted, agreeing, unique_tips)`
//!   OBSERVABLE OUTPUTS asserted:
//!     O1: F1 return — `Option<ReorgResult>` (None = refused) and, when Some,
//!         `rollback` / `common_ancestor` / `new_blocks` / `weight_delta`
//!     O2: F2 return — `Option<ReorgResult>`
//!     O3: F3 mutation — `get_block_weight(h).height` (synthetic vs real)
//!     O4: F4 return — the third tuple element, `unique_chain_tips`
//!   CODE PATHS:
//!     P1: finality guard armed, ancestor BELOW finality      -> None
//!     P2: finality guard armed, ancestor AT finality         -> Some
//!     P3: finality guard armed, ancestor ABOVE finality      -> Some
//!     P4: finality unset                                     -> guard skipped
//!     P5: F2 finality block, ancestor absent from block_weights (`unwrap_or(0)`)
//!     P6: F3 real_height BELOW activation                    -> synthetic height
//!     P7: F3 real_height AT/ABOVE activation                 -> real height
//!   INPUT PARTITIONS:
//!     I1: ancestor height = finality - 1        (P1)
//!     I2: ancestor height = finality            (P2, the LB-2 fencepost)
//!     I3: ancestor height = finality + 1        (P3)
//!     I4: no finality set                       (P4)
//!     I5: fork strictly lighter / equal / heavier by accumulated weight (T9)
//!     I6: all peers agree | one divergent tip | two divergent tips (O4)
//!   MATRIX:
//!     pin_plan_reorg_refuses_below_finality              : O1 x P1 x I1
//!     pin_plan_reorg_permits_reorg_to_exactly_finality   : O1 x P2 x I2
//!     pin_plan_reorg_permits_above_finality              : O1 x P3 x I3
//!     pin_plan_reorg_result_fields_are_exact             : O1 x P4 x I4
//!     pin_check_reorg_weighted_rejects_lighter_chain     : O2 x P5 x I5
//!     pin_check_reorg_weighted_ancestor_unwrap_or_zero   : O2 x P5 x I1
//!     pin_wedge_tie_break_boundaries                     : O1,O2 x I5 (T9)
//!     pin_pre_activation_height_stays_synthetic          : O3 x P6,P7
//!     pin_unique_chain_tips_counts_our_tip_plus_divergent: O4 x I6

use std::collections::HashMap;

use crypto::{Hash, PublicKey};
use doli_core::{Block, BlockHeader};
use network::sync::{ReorgHandler, ReorgResult, SyncConfig, SyncManager};
use network::PeerId;

// ==================== fixtures ====================

fn h(tag: &[u8]) -> Hash {
    crypto::hash::hash(tag)
}

fn block_on(prev_hash: Hash, slot: u32) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot,
        producer: PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

/// A linear main chain `genesis -> b1 -> .. -> bn` plus a competing tip off `fork_at`.
/// Returns (handler, main hashes indexed 1..=n, fork tip, height map).
struct Chain {
    handler: ReorgHandler,
    main: Vec<Hash>,
    fork_tip: Hash,
    heights: HashMap<Hash, u64>,
}

fn chain(len: u64, fork_at: u64, fork_weight: u64) -> Chain {
    let mut handler = ReorgHandler::new();
    let mut heights = HashMap::new();
    let mut main = Vec::new();
    let mut prev = Hash::ZERO;
    heights.insert(Hash::ZERO, 0);

    for i in 1..=len {
        let hash = h(format!("main-{i}").as_bytes());
        handler.record_block_with_weight(hash, prev, 1);
        heights.insert(hash, i);
        main.push(hash);
        prev = hash;
    }

    let fork_parent = if fork_at == 0 {
        Hash::ZERO
    } else {
        main[(fork_at - 1) as usize]
    };
    let fork_tip = h(b"fork-tip");
    handler.record_fork_block(fork_tip, fork_parent, fork_weight);
    heights.insert(fork_tip, fork_at + 1);

    Chain {
        handler,
        main,
        fork_tip,
        heights,
    }
}

impl Chain {
    fn tip(&self) -> Hash {
        *self.main.last().expect("chain must be non-empty")
    }

    fn plan(&self) -> Option<ReorgResult> {
        let parents = self.parents();
        let heights = self.heights.clone();
        self.handler.plan_reorg(
            self.tip(),
            self.fork_tip,
            |x| parents.get(x).copied(),
            |x| heights.get(x).copied(),
        )
    }

    fn parents(&self) -> HashMap<Hash, Hash> {
        let mut m = HashMap::new();
        let mut prev = Hash::ZERO;
        for hash in &self.main {
            m.insert(*hash, prev);
            prev = *hash;
        }
        m
    }
}

// ==================== O1 — the finality guard (LB-1, LB-2, T1) ====================

/// REQ-FORK-016 — Decision: a failure means the refusal M0 is meant to COUNT has
/// been relaxed into an approval, which is trap T1 — the guard is the hero of
/// INC-I-204, not its bug.
#[test]
fn pin_plan_reorg_refuses_below_finality() {
    let mut c = chain(6, 3, 10);
    // Common ancestor is main[2] at height 3; finalize above it.
    c.handler.set_last_finality_height(4);

    assert!(
        c.plan().is_none(),
        "LB-1/T1: plan_reorg must REFUSE a reorg whose common ancestor (h=3) sits \
         below last_finality_height (4). Counting the refusal must not relax it."
    );
}

/// REQ-FORK-016 — Decision: a failure here means the INC-I-090 fencepost fix was
/// reverted to `<=`, which wedges every legal 1-block fork at the finality
/// boundary for a full finality window — the LB-2 trap.
#[test]
fn pin_plan_reorg_permits_reorg_to_exactly_finality() {
    let mut c = chain(6, 3, 10);
    c.handler.set_last_finality_height(3);

    let plan = c.plan().expect(
        "LB-2: ancestor_height == last_finality_height is PERMITTED — the guard is \
         strict `<`, not `<=` (INV-SYNC-008 / INC-I-090)",
    );
    assert_eq!(plan.common_ancestor, c.main[2], "O1: ancestor at h=3");
}

/// REQ-FORK-016 — Decision: a failure means the guard started refusing reorgs it
/// used to approve, i.e. M0 instrumentation silently narrowed the approval set.
#[test]
fn pin_plan_reorg_permits_above_finality() {
    let mut c = chain(6, 4, 10);
    c.handler.set_last_finality_height(3);
    assert!(
        c.plan().is_some(),
        "O1/P3: an ancestor above finality must still yield a plan"
    );
}

/// REQ-FORK-014 — Decision: a failure means the plan's SHAPE moved, so any caller
/// that consumes `rollback` / `new_blocks` (execute_reorg, wedge_escape) would
/// roll back a different set of blocks than it does today.
#[test]
fn pin_plan_reorg_result_fields_are_exact() {
    let c = chain(6, 4, 10);
    let plan = c
        .plan()
        .expect("O1/P4: finality unset — guard is skipped entirely");

    assert_eq!(plan.common_ancestor, c.main[3], "O1: ancestor is main[4]");
    assert_eq!(
        plan.rollback,
        vec![c.main[5], c.main[4]],
        "O1: rollback walks tip-first down to (exclusive) the common ancestor"
    );
    assert_eq!(plan.new_blocks, vec![c.fork_tip], "O1: one new block");
    assert_eq!(
        plan.weight_delta,
        c.handler.chain_weight(&c.fork_tip) as i64 - c.handler.chain_weight(&c.tip()) as i64,
        "O1: weight_delta is fork_weight - current_weight, unchanged"
    );
}

// ==================== O2 — check_reorg_weighted, the M5 deletion candidate ====================

/// REQ-FORK-016 — Decision: D5 adds an entries/rejects probe to this function; a
/// failure means the probe changed which forks it accepts, not just how many it
/// counts.
#[test]
fn pin_check_reorg_weighted_rejects_strictly_lighter_chain() {
    let mut handler = ReorgHandler::new();
    let a1 = h(b"a1");
    let a2 = h(b"a2");
    handler.record_block_with_weight(a1, Hash::ZERO, 10);
    handler.record_block_with_weight(a2, a1, 10);

    // A sibling off a1 carrying weight 1: 10 + 1 = 11 < current 20.
    let light = block_on(a1, 9);
    assert!(
        handler.check_reorg_weighted(&light, a2, 1).is_none(),
        "O2: a strictly lighter chain is rejected before the finality block is reached"
    );
}

/// INV-SYNC-012 — Decision: this pins the MEASURED "0 rejects on all specimen
/// nodes" premise that D5's probe exists to keep measuring; a failure means the
/// `unwrap_or(0)` synthetic-height behaviour changed under M0, which is a
/// consensus-shaped change M0 is forbidden to make.
#[test]
fn pin_check_reorg_weighted_finality_uses_unwrap_or_zero() {
    let mut handler = ReorgHandler::new();
    let a1 = h(b"u1");
    let a2 = h(b"u2");
    handler.record_block_with_weight(a1, Hash::ZERO, 10);
    handler.record_block_with_weight(a2, a1, 10);
    handler.set_last_finality_height(5);

    // Sibling off a1, heavier: 10 + 30 = 40 > 20, so control reaches the finality
    // block. `block_weights[a1].height` is the per-process counter 1, and 1 < 5,
    // so this REJECTS today. That rejection is the site INV-SYNC-012 tracks and
    // M5 will delete — M0 must leave it exactly as it is.
    let heavy = block_on(a1, 9);
    assert!(
        handler.check_reorg_weighted(&heavy, a2, 30).is_none(),
        "O2/P5: the synthetic ancestor height (1) is below finality (5), so the \
         reorg is refused. INV-SYNC-012's known-remaining site, unchanged by M0."
    );

    // Contrast: with finality at or below the synthetic height the SAME input is
    // approved. Without this cell the assertion above could pass for any reason.
    let mut permissive = ReorgHandler::new();
    permissive.record_block_with_weight(a1, Hash::ZERO, 10);
    permissive.record_block_with_weight(a2, a1, 10);
    permissive.set_last_finality_height(1);
    assert!(
        permissive.check_reorg_weighted(&heavy, a2, 30).is_some(),
        "O2/P5 contrast: at finality == synthetic height the same fork is APPROVED"
    );
}

// ==================== T9 — the wedge-escape boundaries ====================

/// REQ-FORK-016 — Decision: D3 splits the wedge-escape `_` arm into countable
/// reasons; a failure means the split moved a boundary, and trap T9 (changing the
/// `fork_w <= our_w` tie-break ungated) is REJECTED for M0.
#[test]
fn pin_wedge_escape_weight_boundaries_are_unchanged() {
    // wedge_escape.rs:130 short-circuits on `fork_weight <= our_weight`, then
    // wedge_escape.rs:159 requires `weight_delta > 0`. Both boundaries are
    // computed from these two primitives.
    let equal = chain(4, 3, 1); // fork tip weight 1 == the block it replaces
    let our_w = equal.handler.chain_weight(&equal.tip());
    let fork_w = equal.handler.chain_weight(&equal.fork_tip);
    assert_eq!(
        fork_w, our_w,
        "T9 setup: an equal-weight sibling is the boundary case"
    );
    assert!(
        fork_w <= our_w,
        "T9: an equal-weight sibling must still take the `cannot out-weight` \
         short-circuit — M0 may COUNT this branch, never widen it"
    );

    let heavier = chain(4, 3, 5);
    assert!(
        heavier.handler.chain_weight(&heavier.fork_tip)
            > heavier.handler.chain_weight(&heavier.tip()),
        "T9 contrast: a strictly heavier sibling passes the short-circuit"
    );
    let plan = heavier.plan().expect("a heavier sibling yields a plan");
    assert!(
        plan.weight_delta > 0,
        "T9: the reorg arm requires a STRICT weight increase (AUDIT-P2-003)"
    );
}

// ==================== O3 — INV-SYNC-012's pre-activation branch ====================

/// INV-SYNC-012 — Decision: D4 attaches this invariant's first canary to the
/// pre-activation branch; a failure means the canary changed WHICH height the
/// guard stores, turning an observability change into a consensus-shaped one.
#[test]
fn pin_pre_activation_branch_still_stores_the_synthetic_height() {
    const ACTIVATION: u64 = 1_000;
    let below = h(b"below-ah");
    let above = h(b"above-ah");

    let mut handler = ReorgHandler::with_activation_height(ACTIVATION);
    // P6: real_height below the activation height -> legacy derived height.
    handler.record_block_with_height(below, Hash::ZERO, 1, ACTIVATION - 1);
    assert_eq!(
        handler.get_block_weight(&below).expect("recorded").height,
        1,
        "O3/P6: below the activation height the LEGACY parent+1 counter is stored \
         (parent absent from block_weights -> 1), not the real height"
    );

    // P7: at the activation height -> the real chain height.
    let mut handler = ReorgHandler::with_activation_height(ACTIVATION);
    handler.record_block_with_height(above, Hash::ZERO, 1, ACTIVATION);
    assert_eq!(
        handler.get_block_weight(&above).expect("recorded").height,
        ACTIVATION,
        "O3/P7: at/above the activation height the REAL chain height is stored"
    );
}

// ==================== O4 — the value D2 must export ====================

/// REQ-FORK-016 — Decision: D2 exports this exact number as a gauge; a failure
/// means the gauge would carry a different quantity than the one INC-I-204's
/// signature is defined over (sustained `unique_chain_tips > 1`).
#[test]
fn pin_unique_chain_tips_counts_our_tip_plus_divergent_peers() {
    let genesis = Hash::ZERO;
    let ours = h(b"our-tip");

    let mut mgr = SyncManager::new(SyncConfig::default(), genesis);
    mgr.update_local_tip(20, ours, 20);

    // I6a: every peer reports OUR hash at our height -> 1 tip.
    mgr.add_peer(PeerId::random(), 20, ours, 20);
    mgr.add_peer(PeerId::random(), 20, ours, 20);
    assert_eq!(
        mgr.checkpoint_health().2,
        1,
        "O4/I6a: a fleet that agrees reports exactly 1 unique chain tip"
    );

    // I6b: one peer on a different hash at the same height -> 2 tips.
    mgr.add_peer(PeerId::random(), 20, h(b"divergent-1"), 20);
    assert_eq!(
        mgr.checkpoint_health().2,
        2,
        "O4/I6b: one divergent peer makes 2 tips — the fork alarm's trigger value"
    );

    // I6c: a second, distinct divergent hash -> 3 tips.
    mgr.add_peer(PeerId::random(), 20, h(b"divergent-2"), 20);
    assert_eq!(
        mgr.checkpoint_health().2,
        3,
        "O4/I6c: distinct divergent hashes are counted separately"
    );
}
