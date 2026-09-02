//! INC-I-147 defect D6 — `BlockWeight.height` is a PER-PROCESS counter, not a chain height.
//!
//! `record_block_internal` computes `height = parent_height + 1`, where `parent_height`
//! comes from `block_weights.get(&prev_hash).unwrap_or((0, 0))`. `ReorgHandler::new()`
//! leaves `block_weights` EMPTY (only `recent_blocks` / `lru_order` are seeded with
//! `Hash::ZERO`), so the FIRST block a process records always receives height 1 —
//! regardless of its real chain height. Every subsequent block counts up from there:
//!
//!     H_syn = H_real - I        where I = (height of the first recorded block) - 1
//!
//! `plan_reorg` (`mod.rs:439-466`) then compares that synthetic counter against
//! `last_finality_height`, which IS a real chain height (set from `check_finality()`).
//! On any node whose init offset exceeds its finality lag, the guard can never pass and
//! NO reorg is ever approved — the node is permanently wedged on a losing fork.
//!
//! MEASURED on the live testnet specimen (2026-07-31, RUN_ID=475): the SAME block
//! `7eb7d785` at real height 57067 weighed **267 on the seed** (init 56800) and
//! **25897 on n7** (init 31170), logged 5.6 ms apart. That divergence is the defect.
//!
//! Why the existing suite cannot catch this: every test in `tests.rs` roots its chain at
//! `Hash::ZERO`, which yields I = 0 — the single value at which the synthetic counter
//! coincides with the real height.
//!
//! Commit `e25a9a97` guarded only the `None` (ancestor-absent) branch of the height
//! lookup. This incident's ancestor was PRESENT, so the `Some(h)` branch shadows the
//! correct `get_height` lookup and returns the wrong-unit value.

// OUTPUT CONTRACT: fn ReorgHandler::plan_reorg(&self, current_tip, new_tip, get_parent, get_height)
//   Outputs:
//     O1: return — Option<ReorgResult> (None = rejected, Some = reorg plan)
//   Paths:
//     P2: rejected_below_finality — ancestor height < finality_height -> None (mod.rs:459-465)
//   INPUT PARTITIONS:
//     (new cells — absent from tests.rs, which only ever exercises I = 0)
//     P2c: ancestor IS in block_weights, recorded by a process that started ABOVE genesis,
//          so the stored height is synthetic (H_real - I) and compares below finality even
//          though the ancestor's REAL height is at/above finality.
//     P2d: two handlers with DIFFERENT init offsets record the SAME real block and must
//          agree on its height (the measured seed-vs-n7 divergence).
//   MATRIX: 1 output x 2 partitions = 2 cells
//     P2c: O1(Some)   — plan_reorg_uses_real_ancestor_height_not_per_process_counter  [FAIL test]
//     P2d: O1(equal)  — block_height_is_chain_derived_not_process_derived             [FAIL test]

use std::collections::HashMap;

use super::*;

/// Measured: the seed's chain height when its process started.
const INIT_HEIGHT: u64 = 56_800;
/// Measured: the real height of the fork point in the reproduced cascade.
const ANCESTOR_REAL_H: u64 = 57_067;
/// Measured: the synthetic height the seed actually stored for that block.
const SEED_SYNTHETIC_H: u64 = ANCESTOR_REAL_H - INIT_HEIGHT; // 267
/// Measured: n7's chain height when ITS process started.
const N7_INIT_HEIGHT: u64 = 31_170;
/// Measured: the synthetic height n7 stored for the very same block.
const N7_SYNTHETIC_H: u64 = ANCESTOR_REAL_H - N7_INIT_HEIGHT; // 25_897

/// Chain fixture mirroring a node that restarted (or snap-synced) mid-chain.
struct MidChainFixture {
    handler: ReorgHandler,
    ancestor: Hash,
    current_tip: Hash,
    fork_tip: Hash,
    heights: HashMap<Hash, u64>,
    parents: HashMap<Hash, Hash>,
}

/// Build a handler whose first recorded block sits at `init_height + 1`, exactly as a
/// freshly-started process does: the parent of that first block is absent from
/// `block_weights`, so it silently defaults to `(0, 0)`.
fn build_mid_chain(init_height: u64, tag: &str) -> MidChainFixture {
    let mut handler = ReorgHandler::new();
    let mut heights: HashMap<Hash, u64> = HashMap::new();
    let mut parents: HashMap<Hash, Hash> = HashMap::new();

    // The block the node was already sitting on at process start. Never recorded —
    // this is precisely why the counter's base is wrong.
    let startup_parent =
        crypto::hash::hash(format!("inc_i_147_{tag}_start_{init_height}").as_bytes());
    heights.insert(startup_parent, init_height);

    // Record exactly as production does (`block_lifecycle.rs:34`), which knows the real
    // height. Before the D6 fix this height was discarded.
    let mut prev = startup_parent;
    let mut ancestor = startup_parent;
    for real_h in (init_height + 1)..=ANCESTOR_REAL_H {
        let h = crypto::hash::hash(format!("inc_i_147_main_{real_h}").as_bytes());
        handler.record_block_with_height(h, prev, 1, real_h);
        heights.insert(h, real_h);
        parents.insert(h, prev);
        prev = h;
        ancestor = h;
    }

    // Our tip: the block this node built on the fork point.
    let current_tip = crypto::hash::hash(b"inc_i_147_poisoned_tip_57068");
    handler.record_block_with_height(current_tip, ancestor, 1, ANCESTOR_REAL_H + 1);
    heights.insert(current_tip, ANCESTOR_REAL_H + 1);
    parents.insert(current_tip, ancestor);

    // The competing sibling at the same real height, strictly heavier so that fork
    // choice has an unambiguous winner and only the finality guard can refuse it.
    let fork_tip = crypto::hash::hash(b"inc_i_147_canonical_sibling_57068");
    handler.record_fork_block(fork_tip, ancestor, 100);
    heights.insert(fork_tip, ANCESTOR_REAL_H + 1);
    parents.insert(fork_tip, ancestor);

    MidChainFixture {
        handler,
        ancestor,
        current_tip,
        fork_tip,
        heights,
        parents,
    }
}

/// P2c — the wedge itself.
///
/// A restarted node holds a fork point whose REAL height equals the finalized height, so
/// reorging to it is legal (it does not go *past* finality). The guard must therefore
/// approve. It refuses, because it compares a per-process counter (267) against a real
/// chain height (57067).
///
/// MUST FAIL before the D6 fix.
#[test]
fn plan_reorg_uses_real_ancestor_height_not_per_process_counter() {
    let mut f = build_mid_chain(INIT_HEIGHT, "seed");

    // The stored height must be the REAL chain height. Pre-fix this was
    // SEED_SYNTHETIC_H (= 267), because the height the caller knew was discarded and
    // re-derived from an empty `block_weights` map.
    let stored = f
        .handler
        .block_weights
        .get(&f.ancestor)
        .expect("ancestor must be tracked")
        .height;
    assert_eq!(
        stored, ANCESTOR_REAL_H,
        "INC-I-147 D6: BlockWeight.height must be the REAL chain height. \
         Pre-fix it was {SEED_SYNTHETIC_H} (= {ANCESTOR_REAL_H} - {INIT_HEIGHT}), a \
         per-process counter that is not comparable with any chain-global height."
    );

    // Finality is a REAL chain height — it comes from check_finality(), which counts
    // attestation weight against real block heights.
    f.handler.set_last_finality_height(ANCESTOR_REAL_H);

    let result = f.handler.plan_reorg(
        f.current_tip,
        f.fork_tip,
        |h| f.parents.get(h).copied(),
        |h| f.heights.get(h).copied(),
        crate::sync::ForkChoiceFinality::default(),
    );

    assert!(
        result.is_some(),
        "INC-I-147 D6: plan_reorg must compare the ancestor's REAL height ({ANCESTOR_REAL_H}) \
         against the finalized height ({ANCESTOR_REAL_H}) — reorging TO the finalized height \
         is legal. It instead compared the per-process counter ({SEED_SYNTHETIC_H}) and refused, \
         which is why the seed and n7 could never leave the losing fork."
    );
}

/// P2d — the measured cross-node divergence, stated as an invariant.
///
/// Two nodes that started at different heights must agree on the height of the same
/// block. A quantity that depends on process start time cannot be compared against a
/// chain-global value. This is the property that makes D6 a unit mismatch rather than a
/// tuning problem.
///
/// MUST FAIL before the D6 fix.
#[test]
fn block_height_is_chain_derived_not_process_derived() {
    let seed = build_mid_chain(INIT_HEIGHT, "seed");
    let n7 = build_mid_chain(N7_INIT_HEIGHT, "n7");

    let seed_h = seed
        .handler
        .block_weights
        .get(&seed.ancestor)
        .expect("seed must track the fork point")
        .height;
    let n7_h = n7
        .handler
        .block_weights
        .get(&n7.ancestor)
        .expect("n7 must track the fork point")
        .height;

    // Both nodes must now store the REAL height. The pre-fix values are recorded in
    // the failure message below: they are the measurement that established D6
    // (seed.log:746641 / n7.log:288861, same block 7eb7d785, 5.6 ms apart).
    assert_eq!(
        seed_h, ANCESTOR_REAL_H,
        "seed must store the real height (pre-fix it stored {SEED_SYNTHETIC_H})"
    );
    assert_eq!(
        n7_h, ANCESTOR_REAL_H,
        "n7 must store the real height (pre-fix it stored {N7_SYNTHETIC_H})"
    );

    assert_eq!(
        seed_h, n7_h,
        "INC-I-147 D6: two nodes must agree on the height of the same block. \
         Measured on the live specimen: the identical block at real height {ANCESTOR_REAL_H} \
         was recorded as {SEED_SYNTHETIC_H} by the seed (started at {INIT_HEIGHT}) and \
         {N7_SYNTHETIC_H} by n7 (started at {N7_INIT_HEIGHT}). A per-process quantity must \
         never be compared against a chain-global finalized height."
    );
}
