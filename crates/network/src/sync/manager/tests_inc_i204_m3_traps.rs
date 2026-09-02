//! INC-I-204 M3 / REQ-FORK-010 — regression traps, the C3 cooldown contract, the
//! B-F1 shape guard, and INV-SYNC-006's first regression tests. TESTS-FIRST.
//!
//! Each `t*` test is a live falsifier for one catalogued "plausible fix" that
//! would break a named invariant. They are written so that MAKING THE TRAP fails
//! the test, not so that they merely restate a constant:
//!
//!   T1  relax the finality guard `<` → `≤`      breaks INV-SYNC-001/004/008, LB-1/LB-2
//!   T5  lower `MINOR_FORK_GAP_MAX`              speeds up the lossy exit, fixes nothing
//!   T6  raise or reset `SNAP_ATTEMPTS_MAX`      fixes a non-cause (attempts measured 0)
//!   T8  let the rollback budget exhaust to snap makes the lossy terminal reachable
//!   T11 delete `wedge_escape` / `SiblingFetch`  recreates the INC-I-143 454-refusal livelock
//!
//! OUTPUT CONTRACT — `fn RecoveryCoordinator::classify(&self, &RecoveryContext)
//!                       -> RecoveryAction`
//!   O1 mutable params: none.  O2 receiver mutation: none (`&self`, pure).
//!   O3 return value: `RecoveryAction` — the ONLY observable output; every
//!      assertion in this file reads it.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: tracing only.
//!   PATHS: the ladder P0..P5, enumerated in `tests_inc_i204_m3.rs`.
//!
//!   INPUT PARTITIONS (one test each):
//!     IP-1 target height == finality (fencepost, legal)       → `t1_..._is_legal`
//!     IP-2 target height <  finality (refusal, load-bearing)  → `t1_..._unwind`
//!     IP-3 gap == MINOR_FORK_GAP_MAX - 1 / == MAX (band edge) → `t5_...`
//!     IP-4 snap budget one-left / spent / spent-then-recorded → `t6_...`
//!     IP-5 rollback budget spent at a fork-sized gap          → `t8_...`
//!     IP-6 sibling budget 1..MAX, then MAX+1                  → `t11_...`
//!     IP-7 peers reject our tip hash, sibling budget spent    → `c3_header_first_...`
//!     IP-8 a `None` verdict recorded between two classifies   → `c3_none_...`
//!     IP-9 whole wedge band (gap < 500 ∧ tip == finality)     → `inv_sync_006_wedge_band_...`
//!     IP-10 repeated classify on an unchanged context         → `m3_classify_is_deterministic`
//!
//! OUTPUT CONTRACT — `fn RecoveryCoordinator::record_action(&mut self, RecoveryAction)`
//!   O1 mutable params: none.  O2 receiver mutation: `last_action` (cooldown) and
//!      `sibling_fetch_attempts` (probe budget) — read back only through a later
//!      `classify`, which is how IP-4, IP-6 and IP-8 observe them.
//!   O3 return value: `()`.  O4/O5/O6: none.
//!   PATHS: R1 `None` → no cooldown, no budget change. R2 `SiblingFetch` → cooldown
//!          armed, budget +1. R3 any other action → cooldown armed, budget reset.
//!   INPUT PARTITIONS: R1 → IP-8; R2 → IP-6; R3 → IP-4.
//!
//! OUTPUT CONTRACT — `fn SyncManager::sibling_fetch_requests(&self, height: u64)
//!                       -> Vec<(PeerId, SyncRequest)>`
//!   O1 mutable params: none.  O2 receiver mutation: none (`&self`).
//!   O3 return value: the request list — the ONLY observable output.
//!   O4/O5/O6: none.
//!   PATHS: Q1 a request for `height` is already in flight → empty vec.
//!          Q2 no peer at/above `height`                   → empty vec.
//!          Q3 otherwise → up to fanout `GetBlockByHeight{height}`, never any
//!             other height and never any other request kind (INV-SYNC-006).
//!   INPUT PARTITIONS: IP-11 nine peers above our tip, nothing in flight → Q3,
//!     covered by `inv_sync_006_recovery_fetch_...`. Q1 and Q2 are already covered
//!     by the INC-I-143 suite and are not re-tested here.
//!
//! OUTPUT CONTRACT — the `RecoveryAction` / `WedgeReason` types themselves
//!   O3 is the type's SHAPE: its size and its label set. PATHS: S1 the action fits
//!   in a word pair (no branch identity can be smuggled in), S2 every reason maps to
//!   a distinct non-empty metric label.
//!   INPUT PARTITIONS: IP-12 each `WedgeReason` variant → `b_f1_...`.
//!
//! FAIL EVIDENCE (pre-fix, `recovery.rs` classifier unmodified):
//!   RED       — `t8_*`, `c3_header_first_*`, `inv_sync_006_wedge_band_*`
//!   PASS-LOCK — `t1_*`, `t5_*`, `t6_*`, `t11_*`, `c3_none_*`,
//!               `inv_sync_006_recovery_fetch_*`, `b_f1_*`, `m3_classify_*`,
//!               `m3_cell_fixtures_*`

use crypto::Hash;
use libp2p::PeerId;

use crate::protocols::SyncRequest;
use crate::sync::manager::recovery::{
    thresholds, RecoveryAction, RecoveryContext, RecoveryCoordinator, WedgeReason,
};
use crate::sync::manager::tests_inc_i204_m3::{
    all_cells, cell_coordinator, cell_ctx, is_lossy, report_wedge_evidence, rung_name, Budget,
    Cell, FinalityPos, GapBand, COHORT_PEERS, LOCAL_H, WEDGED_LAST_APPLIED,
};
use crate::sync::manager::{SyncConfig, SyncManager};

/// A wedge context at an arbitrary tip/finality/gap, with everything else held at
/// the measured wedge values so only the dimension under test varies.
fn ctx_at(local_height: u64, gap: u64, finality: Option<u64>) -> RecoveryContext {
    RecoveryContext {
        local_height,
        network_tip_height: local_height + gap,
        peer_count: COHORT_PEERS,
        last_applied_secs: WEDGED_LAST_APPLIED,
        shallow_rollback_count: 0,
        snap_attempts: 0,
        last_rollback_local_height: None,
        last_rollback_time: None,
        in_grace_period: false,
        last_finality_height: finality,
    }
}

fn coord_with_wedge_evidence(gap: u64) -> RecoveryCoordinator {
    let mut coord = RecoveryCoordinator::new();
    report_wedge_evidence(&mut coord, gap);
    coord
}

// ===========================================================================
// T1 — the finality guard's strict `<` (LB-1, LB-2). IP-1, IP-2.
// ===========================================================================

/// REQ-FORK-010 / trap T1 — Decision: a failure here reveals that the guard was
/// widened from `<` to `≤`, which re-breaks the INC-I-090 fencepost and blocks
/// every legal one-block fork resolution — the fleet then wedges on ties instead
/// of resolving them.
#[test]
fn t1_finality_guard_stays_strict_a_rollback_landing_exactly_on_finality_is_legal() {
    let coord = coord_with_wedge_evidence(10);
    let ctx = ctx_at(LOCAL_H + 1, 10, Some(LOCAL_H));

    assert_eq!(
        coord.classify(&ctx),
        RecoveryAction::ShallowRollback { depth: 1 },
        "LB-2 / INC-I-090: rolling back TO the finalized height (target {LOCAL_H} == \
         finality {LOCAL_H}) PRESERVES the finalized block and is legal. A `≤` guard would \
         refuse it and strand every one-block fork."
    );
}

/// REQ-FORK-010 / trap T1 — Decision: a failure here reveals that the guard was
/// loosened and the coordinator will now unwind a finalized block, which is
/// INV-SYNC-001's exact violation and deadlocks `plan_reorg`.
#[test]
fn t1_finality_guard_refuses_a_rollback_that_would_unwind_a_finalized_block() {
    let coord = coord_with_wedge_evidence(10);
    let ctx = ctx_at(LOCAL_H, 10, Some(LOCAL_H));

    let action = coord.classify(&ctx);
    assert!(
        !matches!(action, RecoveryAction::ShallowRollback { .. }),
        "INV-SYNC-001 / LB-1: target height {} is BELOW finality {LOCAL_H}. The refusal is \
         the hero of this incident, not the bug. Got `{}`.",
        LOCAL_H - 1,
        rung_name(action)
    );
}

// ===========================================================================
// T5 — MINOR_FORK_GAP_MAX must not be lowered. IP-3.
// ===========================================================================

/// REQ-FORK-010 / trap T5 — Decision: a failure here reveals that the minor-fork
/// band was narrowed, which does not fix the absorbing state — it only makes
/// nodes reach the LOSSY snap exit sooner.
#[test]
fn t5_minor_fork_band_still_reaches_the_last_gap_below_fifty() {
    let last_in_band = thresholds::MINOR_FORK_GAP_MAX - 1;
    let coord = coord_with_wedge_evidence(last_in_band);
    let ctx = ctx_at(LOCAL_H, last_in_band, Some(LOCAL_H - 10));

    assert_eq!(
        coord.classify(&ctx),
        RecoveryAction::ShallowRollback { depth: 1 },
        "trap T5: gap={last_in_band} must still be treated as a MINOR fork resolvable by a \
         bounded rollback. Narrowing the band pushes this node straight at the snap exit."
    );

    assert_eq!(
        thresholds::MINOR_FORK_GAP_MAX,
        50,
        "trap T5: MINOR_FORK_GAP_MAX is a measured cliff (INC-I-190 crossed it at 52 \
         slots). Lowering it is a threshold move dressed as a fix."
    );

    let just_out = coord_with_wedge_evidence(thresholds::MINOR_FORK_GAP_MAX);
    let out_ctx = ctx_at(LOCAL_H, thresholds::MINOR_FORK_GAP_MAX, Some(LOCAL_H - 10));
    assert!(
        !matches!(
            just_out.classify(&out_ctx),
            RecoveryAction::ShallowRollback { .. }
        ),
        "the band must stay half-open: gap == MINOR_FORK_GAP_MAX is OUTSIDE it"
    );
}

// ===========================================================================
// T6 — SNAP_ATTEMPTS_MAX must not be raised or reset. IP-4.
// ===========================================================================

/// REQ-FORK-010 / trap T6 — Decision: a failure here reveals that the snap budget
/// was raised or silently refilled, buying more attempts at a rung measured at zero
/// attempts during the incident — a fix aimed at a non-cause, and one that re-opens
/// the lossy exit LB-6 deliberately closes.
#[test]
fn t6_snap_attempt_budget_is_three_and_the_coordinator_never_refills_it() {
    assert_eq!(
        thresholds::SNAP_ATTEMPTS_MAX,
        3,
        "trap T6: SNAP_ATTEMPTS_MAX is deliberate (LB-6 — never reset, exonerated by \
         measurement)"
    );

    let mut ctx = ctx_at(LOCAL_H, 900, None);
    ctx.snap_attempts = thresholds::SNAP_ATTEMPTS_MAX - 1;
    assert_eq!(
        coord_with_wedge_evidence(900).classify(&ctx),
        RecoveryAction::SnapSync,
        "fixture: with one attempt left and a genuine 900-block gap, snap must be offered"
    );

    ctx.snap_attempts = thresholds::SNAP_ATTEMPTS_MAX;
    let mut coord = coord_with_wedge_evidence(900);
    assert_ne!(
        coord.classify(&ctx),
        RecoveryAction::SnapSync,
        "trap T6: the budget is spent — snap must be closed"
    );

    for action in [
        RecoveryAction::SiblingFetch { height: LOCAL_H },
        RecoveryAction::HeaderFirstSync,
        RecoveryAction::SnapSync,
        RecoveryAction::None,
    ] {
        coord.record_action(action);
        coord.clear_cooldown_for_test();
        assert_ne!(
            coord.classify(&ctx),
            RecoveryAction::SnapSync,
            "trap T6 / LB-6: recording `{}` must not refill the snap budget",
            rung_name(action)
        );
    }
}

// ===========================================================================
// T8 — the rollback budget must not exhaust into snap. IP-5.
// ===========================================================================

/// REQ-FORK-010 / trap T8 — Decision: a failure here reveals that spending the
/// rollback budget is still an admission ticket to snap sync at a fork-sized gap,
/// which is the measured "recovery requires getting worse" shape and destroys the
/// block history the wedged node was still serving.
#[test]
fn t8_an_exhausted_rollback_budget_must_not_open_the_lossy_snap_exit() {
    let mut offenders: Vec<String> = Vec::new();
    for cell in all_cells() {
        if cell.rollback != Budget::Spent || cell.gap.snap_is_legitimate() {
            continue;
        }
        let action = cell_coordinator(cell).classify(&cell_ctx(cell));
        if is_lossy(action) {
            offenders.push(format!("{} -> {}", cell.name(), rung_name(action)));
        }
    }
    assert!(
        offenders.is_empty(),
        "trap T8: {} fork-band cells escalate to a LOSSY rung purely because the rollback \
         budget is spent. `rollback_exhausted` is a statement about a BUDGET, never \
         evidence that the node is behind. Offenders:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

// ===========================================================================
// T11 — SiblingFetch / wedge_escape must survive until C-6 is green. IP-6.
// ===========================================================================

/// REQ-FORK-010 / trap T11 — Decision: a failure here reveals that the bounded
/// non-destructive probe was deleted or unbounded, either of which recreates the
/// INC-I-143 seed1 livelock (454 hot refusals with no probe between them).
#[test]
fn t11_sibling_fetch_remains_the_bounded_non_destructive_probe_at_our_own_tip() {
    let mut coord = coord_with_wedge_evidence(10);
    let ctx = ctx_at(LOCAL_H, 10, Some(LOCAL_H));

    for attempt in 1..=thresholds::SIBLING_FETCH_MAX {
        let action = coord.classify(&ctx);
        assert_eq!(
            action,
            RecoveryAction::SiblingFetch { height: LOCAL_H },
            "trap T11 / LB-9: attempt {attempt} of {} must be a non-destructive probe at \
             OUR tip height, not `{}`",
            thresholds::SIBLING_FETCH_MAX,
            rung_name(action)
        );
        coord.record_action(action);
        coord.clear_cooldown_for_test();
    }

    let after = coord.classify(&ctx);
    assert!(
        !matches!(after, RecoveryAction::SiblingFetch { .. }),
        "trap T11: the probe must be BOUNDED — an unbounded fetch is the livelock with \
         extra network traffic"
    );
    assert!(
        !is_lossy(after),
        "REQ-FORK-010: the probe's successor must be non-lossy; got `{}`",
        rung_name(after)
    );
}

// ===========================================================================
// C3 — no cooldown for a rung that cannot act. IP-7, IP-8.
// ===========================================================================

/// REQ-FORK-010 (C3) — Decision: a failure here reveals that the classifier still
/// emits `HeaderFirstSync` for a node whose peers reject its tip hash, arming a
/// 30s cooldown against a dispatch arm that performs no action — the node is
/// silenced by a rung that did nothing, and re-silences itself every tick.
#[test]
fn c3_header_first_is_not_emitted_when_peers_reject_our_tip_hash() {
    let mut offenders: Vec<String> = Vec::new();
    for cell in all_cells() {
        if cell.sibling != Budget::Spent || cell.fin != FinalityPos::AtFinality {
            continue;
        }
        let action = cell_coordinator(cell).classify(&cell_ctx(cell));
        if action == RecoveryAction::HeaderFirstSync {
            offenders.push(cell.name());
        }
    }
    assert!(
        offenders.is_empty(),
        "REQ-FORK-010 (C3): {} cells return HeaderFirstSync after twelve peers answered \
         ZERO headers for our tip hash. An action that cannot act must return `None` and \
         name a terminal — never consume the cooldown. Offenders:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// REQ-FORK-010 (C3) — Decision: a failure here reveals that a `None` verdict began
/// arming the action cooldown, which would suppress the next 30s of evaluation on a
/// node that took no action at all.
#[test]
fn c3_none_never_consumes_the_action_cooldown() {
    let mut coord = coord_with_wedge_evidence(10);
    let ctx = ctx_at(LOCAL_H, 10, Some(LOCAL_H));
    let first = coord.classify(&ctx);

    coord.record_action(RecoveryAction::None);
    assert_eq!(
        coord.classify(&ctx),
        first,
        "C3: recording `None` must leave the classifier free to re-evaluate immediately. \
         A cooldown armed by a non-action is a self-inflicted silence."
    );
}

// ===========================================================================
// INV-SYNC-006 — chain continuity on any path that advances the tip. IP-9, IP-11.
// ===========================================================================

/// REQ-FORK-010 / INV-SYNC-006 — Decision: a failure here reveals that a wedged
/// node still selects a rung which installs a new tip without a parent-hash walk
/// from the current one, producing the gapped block-store this invariant was
/// written against (four contiguous gap ranges measured 2026-05-18).
#[test]
fn inv_sync_006_wedge_band_never_selects_a_continuity_breaking_rung() {
    let mut offenders: Vec<String> = Vec::new();
    for cell in all_cells() {
        if cell.fin != FinalityPos::AtFinality || cell.gap.snap_is_legitimate() {
            continue;
        }
        let action = cell_coordinator(cell).classify(&cell_ctx(cell));
        if is_lossy(action) {
            offenders.push(format!("{} -> {}", cell.name(), rung_name(action)));
        }
    }
    assert!(
        offenders.is_empty(),
        "INV-SYNC-006: {} wedge-band cells select a rung that replaces local state instead \
         of extending it from the current tip. Snap and genesis resync install a floor with \
         no parent linkage to the blocks below it — the node becomes a future cascade \
         trigger via incomplete_block_store. Offenders:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

/// REQ-FORK-010 / INV-SYNC-006 — Decision: a failure here reveals that the recovery
/// fetch path started requesting a height other than the one it is resolving, which
/// is precisely how a contiguous hole enters the block store.
#[test]
fn inv_sync_006_recovery_fetch_requests_exactly_one_height_and_never_skips() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = LOCAL_H;
    mgr.local_slot = LOCAL_H as u32;
    mgr.local_hash = crypto::hash::hash(b"inc_i204_m3_local_tip");
    let peer_hash = crypto::hash::hash(b"inc_i204_m3_competing_tip");
    for _ in 0..COHORT_PEERS {
        mgr.add_peer(
            PeerId::random(),
            LOCAL_H + 10,
            peer_hash,
            (LOCAL_H + 10) as u32,
        );
    }

    let requests = mgr.sibling_fetch_requests(LOCAL_H);
    assert!(
        !requests.is_empty(),
        "fixture: nine peers at/above our tip must yield at least one probe"
    );
    for (_, request) in &requests {
        match request {
            SyncRequest::GetBlockByHeight { height } => assert_eq!(
                *height, LOCAL_H,
                "INV-SYNC-006: the probe must request OUR tip height ({LOCAL_H}) — the \
                 competing sibling. Any other height leaves a hole between the tip and \
                 whatever arrives."
            ),
            other => panic!(
                "INV-SYNC-006: the evidence-acquiring path must issue read-only block \
                 fetches, not `{other:?}`"
            ),
        }
    }
}

// ===========================================================================
// B-F1 — the terminal acquires evidence, it never decides a branch. IP-12.
// ===========================================================================

/// REQ-FORK-010 (B-F1) — Decision: a failure here reveals that the recovery terminal
/// grew a payload wide enough to carry a branch identity (a 32-byte hash), i.e. the
/// recovery layer started DECIDING which chain wins — a consensus change wearing a
/// recovery costume, outside every activation-height gate.
#[test]
fn b_f1_the_wedged_terminal_carries_a_reason_never_a_branch_identity() {
    let action_size = std::mem::size_of::<RecoveryAction>();
    assert!(
        action_size <= 16,
        "B-F1: `RecoveryAction` grew to {action_size} bytes. A 32-byte block hash now \
         fits inside a recovery action, which means the ladder can name the winning \
         branch. Fork choice decides branches; recovery only fetches evidence for it."
    );

    let reason_size = std::mem::size_of::<WedgeReason>();
    assert!(
        reason_size <= 8,
        "B-F1: `WedgeReason` grew to {reason_size} bytes — a reason is a metric label, \
         not a payload"
    );

    let labels = [
        WedgeReason::FinalityConflict.label(),
        WedgeReason::RollbackBudgetExhausted.label(),
        WedgeReason::NoActionableRung.label(),
    ];
    for (i, label) in labels.iter().enumerate() {
        assert!(
            !label.is_empty(),
            "every wedge reason must carry a metric label (index {i})"
        );
        assert_eq!(
            labels.iter().filter(|l| *l == label).count(),
            1,
            "wedge reason labels must be distinct so the alarm can tell the cases apart: \
             `{label}` is duplicated"
        );
    }
}

// ===========================================================================
// Fixture integrity — keeps the enumeration from passing vacuously. IP-10.
// ===========================================================================

/// REQ-FORK-010 — Decision: a failure here reveals that the classifier stopped being
/// a pure read of its context, so two nodes with identical evidence could take
/// different recovery actions and the C-6 enumeration would stop being a proof.
#[test]
fn m3_classify_is_deterministic_for_every_cell() {
    for cell in all_cells() {
        let coord = cell_coordinator(cell);
        let ctx = cell_ctx(cell);
        let first = coord.classify(&ctx);
        for _ in 0..3 {
            assert_eq!(
                coord.classify(&ctx),
                first,
                "classify must be a pure function of (evidence, last_action, ctx) at {}",
                cell.name()
            );
        }
    }
}

/// REQ-FORK-010 — Decision: a failure here reveals a cell whose fixture no longer
/// builds the state it names, which would let the whole C-6 enumeration pass
/// vacuously while testing a state space nobody intended.
#[test]
fn m3_cell_fixtures_build_the_state_they_name() {
    for cell in all_cells() {
        let ctx = cell_ctx(cell);
        assert_eq!(ctx.gap(), cell.gap.gap(), "{} gap mismatch", cell.name());
        assert_eq!(
            cell.fin.rollback_refused(),
            match ctx.last_finality_height {
                Some(f) => ctx.local_height.saturating_sub(1) < f,
                None => false,
            },
            "{} finality verdict mismatch",
            cell.name()
        );
        assert_eq!(
            ctx.snap_attempts >= thresholds::SNAP_ATTEMPTS_MAX,
            cell.snap == Budget::Spent,
            "{} snap budget mismatch",
            cell.name()
        );
        assert_eq!(
            ctx.shallow_rollback_count >= thresholds::SHALLOW_ROLLBACK_MAX,
            cell.rollback == Budget::Spent,
            "{} rollback budget mismatch",
            cell.name()
        );
    }

    let sample = Cell {
        gap: GapBand::Minor,
        fin: FinalityPos::AtFinality,
        rollback: Budget::Fresh,
        sibling: Budget::Spent,
        snap: Budget::Fresh,
    };
    assert!(
        !matches!(
            cell_coordinator(sample).classify(&cell_ctx(sample)),
            RecoveryAction::SiblingFetch { .. }
        ),
        "fixture: `sibling: Spent` must actually exhaust the coordinator-internal probe \
         budget, otherwise every wedge cell is tested one rung too early"
    );
}
