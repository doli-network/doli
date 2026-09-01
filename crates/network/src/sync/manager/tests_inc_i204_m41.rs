//! INC-I-204 M4.1 / REQ-FORK-012 — the `forceReorgTo` directive: expiry, restart
//! scope, single-shot, and the `plan_reorg` operator variant. TESTS-FIRST (RED).
//!
//! REQ-FORK-012 — Decision: a failure here means the operator escape either never
//! fires in the cell it was built for (`tip == finality`), or it outlives the
//! incident and re-fires on an unrelated branch weeks later — the INC-I-196
//! self-brick shape on an auto-updating fleet.
//! REQ-FORK-003 — Decision: a failure in the `plan_reorg` pass-locks means the
//! refactor that adds the operator variant silently loosened the finality guard for
//! the AUTOMATIC callers too (LB-1/LB-2, trap T1), turning one audited human door
//! into four unaudited ones.
//!
//! OUTPUT CONTRACT — `fn SyncManager::arm_force_reorg(&mut self, Hash)`
//!   O1 mutable params: none.  O2 receiver: `force_reorg` slot REPLACED (single-shot
//!      invariant lives here).  O3 return: `()`.  O4 persistent store: MUST be none
//!      (C9 restart scope).  O5 statics: MUST be none.  O6 events: tracing only.
//!
//! OUTPUT CONTRACT — `fn SyncManager::poll_force_reorg(&mut self, Instant, u64)
//!                       -> ForceReorgPoll`
//!   O1 none.  O2 receiver: the slot is CLEARED on the expiry paths only.
//!   O3 return value — the primary output.  O4/O5/O6 none.
//!   PATHS: P1 nothing armed -> `Idle`.
//!          P2 armed, wall-clock TTL elapsed -> `Expired`, slot cleared.
//!          P3 armed, height span elapsed    -> `Expired`, slot cleared.
//!          P4 armed, live                   -> `Armed(target)`, slot RETAINED.
//!   INPUT PARTITIONS: age {0, TTL-1s, TTL, TTL+1s} x span {0, MAX, MAX+1}.
//!
//! OUTPUT CONTRACT — `fn SyncManager::consume_force_reorg(&mut self) -> Option<Hash>`
//!   O2 receiver: slot cleared unconditionally.  O3 return: the target, once.
//!
//! OUTPUT CONTRACT — `fn ReorgHandler::plan_reorg(..) -> Option<ReorgResult>` and
//!                   `fn ReorgHandler::plan_reorg_operator(..) -> Option<ReorgResult>`
//!   O1/O2/O4/O5/O6 none (`&self`, pure).  O3 return value only.
//!   PATHS asserted: V5 finality refusal (ancestor < finality), the `==` fencepost
//!   (legal, LB-2), and the operator override of V5 ONLY.

use std::time::{Duration, Instant};

use crypto::Hash;

use crate::sync::manager::force_reorg::{
    ForceReorgPoll, FORCE_REORG_MAX_HEIGHT_SPAN, FORCE_REORG_TTL_SECS,
};
use crate::sync::manager::{SyncConfig, SyncManager};
use crate::sync::reorg::ReorgHandler;

const ARMED_AT: u64 = 100;

fn h(tag: &str) -> Hash {
    crypto::hash::hash(format!("inc_i204_m41_{tag}").as_bytes())
}

/// A manager whose local tip sits at `ARMED_AT`.
fn mgr() -> SyncManager {
    let mut m = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    m.update_local_tip(ARMED_AT, h("tip"), ARMED_AT as u32);
    m
}

// ---------------------------------------------------------------------------
// B — expiry. Both bounds, independently.
// ---------------------------------------------------------------------------

/// REQ-FORK-012 — Decision: a failure means a directive armed during an incident is
/// still live after the incident closed, so the next branch that happens to arrive
/// gets force-adopted with nobody watching.
#[test]
fn b1_wall_clock_ttl_expiry_never_fires_and_is_counted() {
    let target = h("target");
    let mut m = mgr();
    let t0 = Instant::now();
    m.arm_force_reorg(target);

    // Just inside the TTL: still live.
    assert_eq!(
        m.poll_force_reorg(t0 + Duration::from_secs(FORCE_REORG_TTL_SECS - 1), ARMED_AT),
        ForceReorgPoll::Armed(target),
        "P4: a directive one second short of the TTL must still fire"
    );

    // Past the TTL: never fires, and the slot is dropped.
    assert_eq!(
        m.poll_force_reorg(t0 + Duration::from_secs(FORCE_REORG_TTL_SECS + 1), ARMED_AT),
        ForceReorgPoll::Expired,
        "P2: past FORCE_REORG_TTL_SECS the directive must report Expired, never Armed"
    );
    assert_eq!(
        m.force_reorg_target(),
        None,
        "O2: the expiry path must CLEAR the slot, not leave it to fire on the next tick"
    );
    assert_eq!(
        m.poll_force_reorg(t0 + Duration::from_secs(FORCE_REORG_TTL_SECS + 2), ARMED_AT),
        ForceReorgPoll::Idle,
        "P1: an expired directive is counted once, then the manager is idle"
    );
}

/// REQ-FORK-012 — Decision: a failure means a node that already resumed advancing
/// still carries a stale rescue order, so a routine 60-block catch-up ends in a
/// forced reorg nobody asked for.
#[test]
fn b2_height_span_expiry_never_fires_and_is_counted() {
    let target = h("target");
    let t0 = Instant::now();

    // At the span boundary the directive is still live (the wedge band is inclusive).
    let mut live = mgr();
    live.arm_force_reorg(target);
    assert_eq!(
        live.poll_force_reorg(t0, ARMED_AT + FORCE_REORG_MAX_HEIGHT_SPAN),
        ForceReorgPoll::Armed(target),
        "P4: exactly armed_at_height + FORCE_REORG_MAX_HEIGHT_SPAN is still inside the band"
    );

    // One block past it, the node no longer needs rescuing.
    let mut dead = mgr();
    dead.arm_force_reorg(target);
    assert_eq!(
        dead.poll_force_reorg(t0, ARMED_AT + FORCE_REORG_MAX_HEIGHT_SPAN + 1),
        ForceReorgPoll::Expired,
        "P3: past the height span the directive must report Expired, never Armed"
    );
    assert_eq!(
        dead.force_reorg_target(),
        None,
        "O2: the height-expiry path must CLEAR the slot"
    );
}

/// REQ-FORK-012 — Decision: a failure means the two bounds were collapsed into one,
/// so a node that is wedged (height frozen) never times out, or a node that is
/// advancing fast is judged only by the clock.
#[test]
fn b3_the_two_bounds_are_independent() {
    assert_eq!(
        FORCE_REORG_TTL_SECS, 300,
        "anchored to BRANCH_VERDICT_TTL / SyncConfig::stale_timeout, not a fresh magic number"
    );
    assert_eq!(
        FORCE_REORG_MAX_HEIGHT_SPAN,
        crate::sync::manager::recovery::thresholds::MINOR_FORK_GAP_MAX,
        "anchored to the band the wedge lives in (MINOR_FORK_GAP_MAX = 50)"
    );

    // Clock elapsed, height frozen (the wedged node) -> still expires on the clock.
    let mut a = mgr();
    let t0 = Instant::now();
    a.arm_force_reorg(h("target"));
    assert_eq!(
        a.poll_force_reorg(t0 + Duration::from_secs(FORCE_REORG_TTL_SECS + 1), ARMED_AT),
        ForceReorgPoll::Expired,
        "a frozen height must not keep a stale directive alive"
    );

    // Height elapsed, clock fresh (the recovered node) -> still expires on height.
    let mut b = mgr();
    b.arm_force_reorg(h("target"));
    assert_eq!(
        b.poll_force_reorg(t0, ARMED_AT + FORCE_REORG_MAX_HEIGHT_SPAN + 1),
        ForceReorgPoll::Expired,
        "a fresh clock must not keep a directive alive on a node that resumed advancing"
    );
}

// ---------------------------------------------------------------------------
// C — restart scope. Memory only.
// ---------------------------------------------------------------------------

/// REQ-FORK-012 — Decision: a failure means the directive is process-global or
/// persisted, so it survives the restart that is supposed to erase it — the
/// INC-I-196 sticky-operator-mark self-brick on an auto-updating fleet.
#[test]
fn c1_a_fresh_manager_never_inherits_a_directive() {
    let mut armed = mgr();
    armed.arm_force_reorg(h("target"));
    assert_eq!(
        armed.force_reorg_target(),
        Some(h("target")),
        "precondition: the directive is actually armed on the first manager"
    );

    let fresh = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    assert_eq!(
        fresh.force_reorg_target(),
        None,
        "C9: a directive must live in ONE SyncManager instance — never in a static, \
         a lazy_static, or any process-global the next manager can read back"
    );

    // The first manager is unaffected by the second's construction.
    assert_eq!(
        armed.force_reorg_target(),
        Some(h("target")),
        "the two instances must not share storage in either direction"
    );
}

// ---------------------------------------------------------------------------
// D — single-shot and replacement.
// ---------------------------------------------------------------------------

/// REQ-FORK-012 — Decision: a failure means one operator command can fire twice, or
/// two directives race, so a single audited action becomes an unbounded number of
/// unaudited retractions.
#[test]
fn d1_a_consumed_directive_is_inert() {
    let target = h("target");
    let mut m = mgr();
    m.arm_force_reorg(target);

    assert_eq!(
        m.consume_force_reorg(),
        Some(target),
        "O3: the first consumption yields the target"
    );
    assert_eq!(
        m.consume_force_reorg(),
        None,
        "single-shot: the second consumption must yield nothing"
    );
    assert_eq!(
        m.poll_force_reorg(Instant::now(), ARMED_AT),
        ForceReorgPoll::Idle,
        "single-shot: a consumed directive leaves the manager idle"
    );
}

/// REQ-FORK-012 — Decision: a failure means arming a corrected target leaves the
/// WRONG one live behind it, so the node adopts a branch the operator explicitly
/// replaced.
#[test]
fn d2_arming_replaces_never_queues() {
    let first = h("first");
    let second = h("second");
    let mut m = mgr();

    m.arm_force_reorg(first);
    m.arm_force_reorg(second);

    assert_eq!(
        m.force_reorg_target(),
        Some(second),
        "the second arm REPLACES the first — there is never more than one directive"
    );
    assert_eq!(
        m.consume_force_reorg(),
        Some(second),
        "consumption yields the replacement"
    );
    assert_eq!(
        m.consume_force_reorg(),
        None,
        "no queue: the replaced directive must not surface after the replacement fires"
    );
}

/// REQ-FORK-012 — Decision: a failure means a poll that merely observed a
/// not-yet-arrived branch burned the operator's single shot, so the escape is
/// useless exactly when it is armed ahead of the branch (its intended use).
#[test]
fn d3_polling_a_live_directive_does_not_consume_it() {
    let target = h("target");
    let mut m = mgr();
    let t0 = Instant::now();
    m.arm_force_reorg(target);

    for _ in 0..5 {
        assert_eq!(
            m.poll_force_reorg(t0, ARMED_AT),
            ForceReorgPoll::Armed(target),
            "P4: polling is non-destructive; only a DECISION consumes"
        );
    }
    assert_eq!(
        m.force_reorg_target(),
        Some(target),
        "O2: five polls must leave the slot untouched"
    );
}

// ---------------------------------------------------------------------------
// G3 / trap T1 — the automatic finality guard is byte-identical after the refactor.
// ---------------------------------------------------------------------------

/// A 3-block canonical chain and a 3-block fork sharing only `c1` (h=1).
/// Returns `(handler, current_tip, fork_tip, heights)`.
fn two_branch_handler() -> (ReorgHandler, Hash, Hash, Vec<(Hash, u64)>) {
    let (c1, c2, c3) = (h("c1"), h("c2"), h("c3"));
    let (f2, f3) = (h("f2"), h("f3"));

    // activation height 0 => plan_reorg reads REAL heights (post-INC-I-147 branch),
    // which is what mainnet runs.
    let mut rh = ReorgHandler::with_activation_height(0);
    rh.record_block_with_height(c1, Hash::ZERO, 1, 1);
    rh.record_block_with_height(c2, c1, 1, 2);
    rh.record_block_with_height(c3, c2, 1, 3);
    rh.record_fork_block(f2, c1, 1);
    rh.record_fork_block(f3, f2, 1);

    let heights = vec![(c1, 1), (c2, 2), (c3, 3), (f2, 2), (f3, 3)];
    (rh, c3, f3, heights)
}

fn parent_of(hash: &Hash) -> Option<Hash> {
    match () {
        _ if *hash == h("c3") => Some(h("c2")),
        _ if *hash == h("c2") => Some(h("c1")),
        _ if *hash == h("c1") => Some(Hash::ZERO),
        _ if *hash == h("f3") => Some(h("f2")),
        _ if *hash == h("f2") => Some(h("c1")),
        _ => None,
    }
}

/// REQ-FORK-003 — Decision: a failure means extracting the operator variant changed
/// what the AUTOMATIC callers see, so the strict `<` finality refusal (LB-1/LB-2,
/// INV-SYNC-001/004/008) was loosened for gossip and recovery too — trap T1.
#[test]
fn t1_plan_reorg_still_refuses_below_finality_for_automatic_callers() {
    let (mut rh, tip, fork_tip, heights) = two_branch_handler();
    let height_of = move |x: &Hash| heights.iter().find(|(k, _)| k == x).map(|(_, v)| *v);

    // Common ancestor c1 is at h=1; finality at 2 puts it BELOW.
    rh.set_last_finality_height(2);
    assert!(
        rh.plan_reorg(tip, fork_tip, parent_of, &height_of)
            .is_none(),
        "V5 UNCHANGED: an automatic caller must still be refused when the common \
         ancestor (h=1) is below the finalized height (2)"
    );
}

/// REQ-FORK-003 — Decision: a failure means the guard drifted to `<=`, which blocks
/// the legal 1-block fork at the boundary and wedges nodes for a full finality
/// window (INV-SYNC-008's recorded fencepost).
#[test]
fn t1_plan_reorg_still_permits_an_ancestor_exactly_at_finality() {
    let (mut rh, tip, fork_tip, heights) = two_branch_handler();
    let height_of = move |x: &Hash| heights.iter().find(|(k, _)| k == x).map(|(_, v)| *v);

    rh.set_last_finality_height(1);
    let plan = rh
        .plan_reorg(tip, fork_tip, parent_of, &height_of)
        .expect("LB-2 fencepost: ancestor height == finality height is LEGAL (strict `<`)");
    assert_eq!(
        plan.common_ancestor,
        h("c1"),
        "the fencepost plan resolves the shared ancestor, not a synthetic one"
    );
}

/// REQ-FORK-012 — Decision: a failure means the escape is inert in the exact cell it
/// was built for — `tip == finality`, where EVERY common ancestor is below the
/// marker — so LB-4 would be removed with no working replacement.
#[test]
fn the_operator_variant_is_the_only_caller_that_crosses_the_marker() {
    let (mut rh, tip, fork_tip, heights) = two_branch_handler();
    let height_of = move |x: &Hash| heights.iter().find(|(k, _)| k == x).map(|(_, v)| *v);

    rh.set_last_finality_height(2);

    // Same handler, same inputs, same instant: the automatic door is shut...
    assert!(
        rh.plan_reorg(tip, fork_tip, parent_of, &height_of)
            .is_none(),
        "precondition: the automatic caller is refused, so the operator plan below \
         cannot be passing for a trivial reason"
    );

    // ...and the audited operator door is the one that opens.
    let plan = rh
        .plan_reorg_operator(tip, fork_tip, parent_of, &height_of)
        .expect("REQ-FORK-012: the operator variant must cross the finality MARKER");
    assert_eq!(plan.common_ancestor, h("c1"));
    assert_eq!(
        plan.rollback,
        vec![h("c3"), h("c2")],
        "the plan retracts exactly our two post-ancestor blocks"
    );
    assert_eq!(
        plan.new_blocks.len(),
        2,
        "and applies exactly the fork's two blocks"
    );

    assert_eq!(
        rh.last_finality_height(),
        Some(2),
        "the operator variant is PURE: planning must not erase the finality marker"
    );
}

/// REQ-FORK-012 — Decision: a failure means the operator variant became a blanket
/// bypass rather than a finality-marker-only one, so `MAX_REORG_DEPTH` (V1/V2) and
/// the no-common-ancestor refusal (V3) stopped binding an operator-named branch.
#[test]
fn the_operator_variant_bypasses_only_the_finality_marker() {
    let (mut rh, tip, _fork_tip, heights) = two_branch_handler();
    let height_of = move |x: &Hash| heights.iter().find(|(k, _)| k == x).map(|(_, v)| *v);
    rh.set_last_finality_height(2);

    // V3: a hash with no ancestry into our chain has no common ancestor.
    assert!(
        rh.plan_reorg_operator(tip, h("unrelated"), parent_of, &height_of)
            .is_none(),
        "V3 still binds: no common ancestor means no plan, whoever asked"
    );
}
