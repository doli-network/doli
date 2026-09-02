//! INC-I-204 M4.1 / REQ-FORK-012 — every way the escape must say NO, and the one
//! substantive check the poison arm never had. TESTS-FIRST (RED).
//!
//! REQ-FORK-012 — Decision: a failure here means the operator door is a blanket
//! retraction primitive rather than a corroborated one, so it is strictly MORE
//! dangerous than the LB-4 bypass it replaces and must not ship.
//! REQ-FORK-003 — Decision: a failure in the eligibility test means an operator
//! naming a hash can land a branch built by UNSCHEDULED producers, because
//! `execute_reorg` applies forward blocks with `ValidationMode::Light` and skips
//! `check_producer_eligibility` (block_handling.rs:1040-1046) — the escape is the
//! only thing standing there.
//!
//! OUTPUT CONTRACT — `async fn Node::try_consume_force_reorg(&mut self)
//!                       -> Option<ForceReorgOutcome>`  (refusal paths)
//!   O1 none.  O2a `chain_state` MUST be unchanged on every refusal.
//!   O2c the directive slot: CONSUMED on a terminal refusal, RETAINED while the
//!       branch may still be arriving.  O2e `cumulative_rollback_depth` stays 0.
//!   O3 return value — the outcome label.  O4 `block_store` unchanged.
//!   O5 the outcome counter (asserted in `inc_i_204_m41_metrics`).  O6 tracing.
//!   PATHS: P-UNKNOWN target absent · P-SAME target on our canonical branch ·
//!          P-UNCORROB branch producers < 2/3 local weight · P-INELIGIBLE a branch
//!          block by an unscheduled producer, at the target OR at an intermediate.
//!   INPUT PARTITIONS: target ∈ {never-seen hash, current tip, canonical ancestor,
//!   1-of-3-weight branch, exactly-2-of-3-weight branch, unscheduled-producer
//!   branch, eligible target over an unscheduled intermediate}; arrival ∈ {before
//!   branch, after branch}.
//!   MATRIX: {O2a, O2c, O3} x the five paths — all cells asserted.

use std::fs;
use std::path::Path;

use crypto::Hash;
use doli_node::node::ForceReorgOutcome;

use super::inc_i_204_m41_common::{
    build_block, feed_branch, leader, unscheduled, wedged_node, WEDGE_TIP_HEIGHT,
};

/// Assert the chain did not move and no rollback primitive ran.
macro_rules! assert_chain_untouched {
    ($w:expr, $ctx:expr) => {{
        let cs = $w.node.chain_state.read().await;
        assert_eq!(cs.best_hash, $w.block_a.hash(), "{}: tip unchanged", $ctx);
        assert_eq!(
            cs.best_height, WEDGE_TIP_HEIGHT,
            "{}: height unchanged",
            $ctx
        );
        drop(cs);
        assert_eq!(
            $w.node.cumulative_rollback_depth, 0,
            "{}: a refusal must not roll anything back",
            $ctx
        );
    }};
}

// ---------------------------------------------------------------------------
// E(i) — unknown target.
// ---------------------------------------------------------------------------

/// REQ-FORK-012 — Decision: a failure means a typo'd or hostile hash either crashes
/// the node or is silently treated as success, so the operator gets no signal that
/// the rescue never happened.
#[tokio::test]
async fn an_unknown_target_is_refused_and_never_reported_as_success() {
    let mut w = wedged_node().await;
    feed_branch(&mut w, 3).await;
    let ghost = crypto::hash::hash(b"inc_i204_m41_never_seen");

    w.node.sync_manager.write().await.arm_force_reorg(ghost);
    let outcome = w.node.try_consume_force_reorg().await;

    assert_eq!(
        outcome,
        Some(ForceReorgOutcome::UnknownTarget),
        "O3: a hash the node holds no header for gets its OWN outcome"
    );
    assert_chain_untouched!(w, "unknown target");

    // The directive is NOT burned: an operator legitimately arms ahead of the
    // branch's arrival, which is the whole reason this is a directive and not a
    // synchronous call. The TTL and height-span bounds are what end it.
    assert_eq!(
        w.node.sync_manager.read().await.force_reorg_target(),
        Some(ghost),
        "O2c: waiting for a branch must not spend the single shot"
    );
}

/// REQ-FORK-012 — Decision: a failure means the escape only works if the operator
/// wins a race against gossip, which in the recorded incident is not a race an
/// operator can win.
#[tokio::test]
async fn a_directive_armed_before_the_branch_arrives_fires_when_it_does() {
    let mut w = wedged_node().await;
    let winner = w.branch[2].hash();

    // Armed while the node holds NONE of the competing branch.
    w.node.sync_manager.write().await.arm_force_reorg(winner);
    assert_eq!(
        w.node.try_consume_force_reorg().await,
        Some(ForceReorgOutcome::UnknownTarget),
        "the branch has not arrived yet"
    );
    assert_chain_untouched!(w, "before arrival");

    // The branch arrives; the automatic path is still wedged (feed_branch asserts it).
    feed_branch(&mut w, 3).await;

    assert_eq!(
        w.node.try_consume_force_reorg().await,
        Some(ForceReorgOutcome::Executed),
        "the retained directive must fire on the tick after the branch lands"
    );
    assert_eq!(w.node.chain_state.read().await.best_hash, winner);
}

// ---------------------------------------------------------------------------
// E(ii) — the target is already on our own canonical branch.
// ---------------------------------------------------------------------------

/// REQ-FORK-012 — Decision: a failure means "nothing to do" is indistinguishable
/// from "rescued", so an operator who names the wrong side of the fork is told the
/// wedge is cleared when it is not.
#[tokio::test]
async fn a_target_already_on_our_canonical_branch_is_a_counted_no_op() {
    for (label, pick_tip) in [("current tip", true), ("canonical ancestor", false)] {
        let mut w = wedged_node().await;
        feed_branch(&mut w, 3).await;

        let target = if pick_tip {
            w.block_a.hash()
        } else {
            w.base_tip
        };

        w.node.sync_manager.write().await.arm_force_reorg(target);
        let outcome = w.node.try_consume_force_reorg().await;

        assert_eq!(
            outcome,
            Some(ForceReorgOutcome::SameBranch),
            "{label}: a target on our OWN canonical branch is a distinct, counted \
             refusal — never a reorg and never `Executed`"
        );
        assert_chain_untouched!(w, label);
        assert_eq!(
            w.node.sync_manager.read().await.force_reorg_target(),
            None,
            "{label}: this is a terminal decision — the single shot is spent"
        );
    }
}

// ---------------------------------------------------------------------------
// F — corroboration is load-bearing. >= 2/3 of LOCAL ProducerSet weight.
// ---------------------------------------------------------------------------

/// REQ-FORK-012 — Decision: a failure means the escape carries no substantive check
/// at all — it would land any branch an operator names, including a branch a single
/// producer (or a compromised operator account) manufactured, which makes it worse
/// than the LB-4 bypass it replaces.
#[tokio::test]
async fn a_branch_below_two_thirds_of_local_producer_weight_is_refused() {
    let mut w = wedged_node().await;
    // Only the fork's first block arrives: ONE distinct producer out of three,
    // i.e. 1/3 of local ProducerSet weight — below the 2/3 bar.
    feed_branch(&mut w, 1).await;
    let thin_target = w.branch[0].hash();

    w.node
        .sync_manager
        .write()
        .await
        .arm_force_reorg(thin_target);
    let outcome = w.node.try_consume_force_reorg().await;

    assert_eq!(
        outcome,
        Some(ForceReorgOutcome::Uncorroborated),
        "F7-safe corroboration: a branch whose DISTINCT block producers hold less \
         than 2/3 of the LOCAL ProducerSet weight is refused even though the \
         operator asked for it"
    );
    assert_chain_untouched!(w, "uncorroborated");
    assert_eq!(
        w.node.sync_manager.read().await.force_reorg_target(),
        None,
        "a corroboration refusal is terminal — it must not retry every tick"
    );
}

/// REQ-FORK-012 — Decision: a failure at exactly 2/3 means the bar was implemented
/// as strict `>`, which makes the escape unusable on a 3-producer fork (the
/// recorded shape) while still claiming to implement the 2/3 rule.
#[tokio::test]
async fn a_branch_at_exactly_two_thirds_is_accepted() {
    let mut w = wedged_node().await;
    // Two blocks by two DISTINCT scheduled leaders: 2 of 3 producers, weight 1
    // each => 2*3 >= 3*2 exactly at the bar.
    feed_branch(&mut w, 2).await;
    let target = w.branch[1].hash();

    w.node.sync_manager.write().await.arm_force_reorg(target);
    let outcome = w.node.try_consume_force_reorg().await;

    assert_eq!(
        outcome,
        Some(ForceReorgOutcome::Executed),
        "the bar is `sum * 3 >= total * 2` — inclusive at exactly two thirds"
    );
    let cs = w.node.chain_state.read().await;
    assert_eq!(cs.best_hash, target);
    assert_eq!(cs.best_height, 7);
}

// ---------------------------------------------------------------------------
// G — the escape never bypasses producer eligibility.
// ---------------------------------------------------------------------------

/// REQ-FORK-003 — Decision: a failure means an operator (or anyone who reaches the
/// admin token) can land a branch produced by validators that were never scheduled
/// for those slots, because `execute_reorg`'s forward apply runs in
/// `ValidationMode::Light` and skips the eligibility check entirely.
#[tokio::test]
async fn a_branch_with_an_unscheduled_producer_is_refused() {
    let mut w = wedged_node().await;
    feed_branch(&mut w, 3).await;

    // A competing branch off the SAME common ancestor whose blocks are signed by
    // genuine, bonded ProducerSet members who are NOT the scheduled slot leaders.
    // Two distinct producers => corroboration PASSES, so eligibility is the only
    // thing that can refuse this branch.
    let bad_b = build_block(
        6,
        300,
        w.base_tip,
        unscheduled(&w.producers, 300),
        &w.params,
    );
    let bad_d1 = build_block(
        7,
        301,
        bad_b.hash(),
        unscheduled(&w.producers, 301),
        &w.params,
    );
    w.node.block_store.put_block(&bad_b, 6).expect("put bad_b");
    w.node
        .block_store
        .put_block(&bad_d1, 7)
        .expect("put bad_d1");

    w.node
        .sync_manager
        .write()
        .await
        .arm_force_reorg(bad_d1.hash());
    let outcome = w.node.try_consume_force_reorg().await;

    assert_eq!(
        outcome,
        Some(ForceReorgOutcome::Ineligible),
        "every block on an operator-named branch must pass check_producer_eligibility \
         BEFORE the plan reaches execute_reorg"
    );
    assert_chain_untouched!(w, "ineligible branch");
}

/// REQ-FORK-012 — Decision: a failure means the eligibility gate covers only the
/// endpoints of the operator's branch rather than the whole set `execute_reorg`
/// would apply, so an operator can land an unscheduled INTERMEDIATE block by naming
/// a legitimate descendant of it — the gate reads as present and is not.
#[tokio::test]
async fn an_unscheduled_producer_on_an_intermediate_block_is_refused() {
    let mut w = wedged_node().await;
    feed_branch(&mut w, 3).await;

    // A branch off the SAME common ancestor whose FIRST and LAST blocks are by their
    // scheduled slot leaders and whose MIDDLE block is not. Two distinct producers,
    // so corroboration passes and eligibility is the only thing that can refuse it.
    let ok_first = build_block(6, 303, w.base_tip, leader(&w.producers, 303), &w.params);
    let bad_mid = build_block(
        7,
        304,
        ok_first.hash(),
        unscheduled(&w.producers, 304),
        &w.params,
    );
    let ok_tip = build_block(8, 306, bad_mid.hash(), leader(&w.producers, 306), &w.params);

    // Non-vacuity: the target and the first block must PASS the same gate, or this
    // test degenerates into the endpoint case the sibling test already covers.
    assert!(
        w.node.check_producer_eligibility(&ok_first).await.is_ok(),
        "precondition: the first branch block is by its scheduled leader"
    );
    assert!(
        w.node.check_producer_eligibility(&ok_tip).await.is_ok(),
        "precondition: the NAMED TARGET is by its scheduled leader — only the \
         intermediate block is ineligible"
    );

    w.node
        .block_store
        .put_block(&ok_first, 6)
        .expect("put ok_first");
    w.node
        .block_store
        .put_block(&bad_mid, 7)
        .expect("put bad_mid");
    w.node
        .block_store
        .put_block(&ok_tip, 8)
        .expect("put ok_tip");

    w.node
        .sync_manager
        .write()
        .await
        .arm_force_reorg(ok_tip.hash());
    let outcome = w.node.try_consume_force_reorg().await;

    assert_eq!(
        outcome,
        Some(ForceReorgOutcome::Ineligible),
        "the eligibility gate must cover EVERY block execute_reorg would apply, not \
         just the operator-named target"
    );
    assert_chain_untouched!(w, "ineligible intermediate block");
    assert_eq!(
        w.node.sync_manager.read().await.force_reorg_target(),
        None,
        "O2c: a security refusal is terminal — the single shot is spent"
    );
}

// ---------------------------------------------------------------------------
// C — restart scope, node side. The directive never reaches disk.
// ---------------------------------------------------------------------------

fn contains_bytes(dir: &Path, needle: &[u8]) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if contains_bytes(&path, needle) {
                return true;
            }
        } else if let Ok(bytes) = fs::read(&path) {
            if bytes.windows(needle.len()).any(|w| w == needle) {
                return true;
            }
        }
    }
    false
}

/// REQ-FORK-012 — Decision: a failure means an operator mark survives the restart
/// that is supposed to erase it, which on an auto-updating fleet is the INC-I-196
/// self-brick: a node that force-reorgs itself onto a stale branch every time it
/// comes back up.
#[tokio::test]
async fn the_directive_is_never_written_to_the_node_data_directory() {
    let mut w = wedged_node().await;
    // A hash the node has never seen, so its bytes cannot legitimately be on disk.
    let ghost: Hash = crypto::hash::hash(b"inc_i204_m41_disk_probe");
    assert!(
        !contains_bytes(w.tmp.path(), ghost.as_bytes()),
        "precondition: the probe hash must be absent from the data dir BEFORE arming"
    );

    w.node.sync_manager.write().await.arm_force_reorg(ghost);
    let _ = w.node.try_consume_force_reorg().await;

    assert!(
        !contains_bytes(w.tmp.path(), ghost.as_bytes()),
        "C9 restart scope: arming and polling a directive must write NOTHING to the \
         state DB, the config, or any file under the data directory"
    );
}
