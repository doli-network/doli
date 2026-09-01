//! INC-I-204 M6 / REQ-FORK-004 — the structural property, enumerated.
//! TESTS-FIRST (RED). Design brief §8 (outcome metric).
//!
//! M3's C-6 cell space (`tests_inc_i204_m3.rs`) always carries `StuckFork`, so every
//! one of its 96 cells is CORROBORATED. Rule 2's `rollback_exhausted` and
//! `deep_fork_confirmed` triggers both require `!stuck_fork`, and Rule 4 needs apply
//! failures past 600s — none of which the M3 fixture can produce. This module
//! REUSES M3's cell machinery and adds the two dimensions that make the M6 defect
//! reachable:
//!
//!   × corroboration {Corroborated (StuckFork present), Uncorroborated}
//!   × apply health  {Applying (no failures), Failing (5 failures, 700s)}
//!
//!   4 gap × 3 finality × 2 rollback × 2 sibling × 2 snap × 2 corroboration
//!   × 2 apply health = 384 cells.
//!
//! OUTPUT CONTRACT — `fn RecoveryCoordinator::classify(&self, &RecoveryContext)
//!                       -> RecoveryAction`
//!   O1 mutable params: none.  O2 receiver mutation: none (pure).
//!   O3 return value: `RecoveryAction` — the ONLY observable output.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: tracing only.
//!   PATHS: the full ladder P1..P5, as enumerated in `tests_inc_i204_m3.rs`.
//!   INPUT PARTITIONS: the 384-cell cross product above.
//!   MATRIX: O3 × every cell — each cell's whole 20-tick trajectory is judged
//!           against the lossy-admission predicate below.
//!
//! FAIL EVIDENCE (pre-fix): `m6_no_fork_shaped_cell_reaches_a_lossy_action` and
//!   `m6_lossy_admission_census` are RED with a non-zero printed count;
//!   `m6_census_enumeration_is_not_vacuous` is a PASS-LOCK on both sides.

use libp2p::PeerId;

use crate::sync::manager::recovery::{
    thresholds, RecoveryAction, RecoveryContext, RecoveryCoordinator, RecoveryEvidence,
};
use crate::sync::manager::tests_inc_i204_m3::{
    all_cells, cell_ctx, is_lossy, rung_name, Budget, Cell, LOCAL_H, WEDGED_LAST_APPLIED,
};

/// Ticks driven per cell — same budget M3 uses, so every bounded budget is spent.
const TICKS: usize = 20;

/// Seconds since last apply for a node whose apply path is FAILING (past Rule 4's
/// 600s `truly_stuck` bar).
const DEAD_LAST_APPLIED: u64 = 700;

// ===========================================================================
// The two dimensions M3's fixture cannot express
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Corroboration {
    Corroborated,
    Uncorroborated,
}

impl Corroboration {
    const ALL: [Corroboration; 2] = [Corroboration::Corroborated, Corroboration::Uncorroborated];

    fn name(self) -> &'static str {
        match self {
            Corroboration::Corroborated => "stuck_fork",
            Corroboration::Uncorroborated => "empty_headers_only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyHealth {
    Applying,
    Failing,
}

impl ApplyHealth {
    const ALL: [ApplyHealth; 2] = [ApplyHealth::Applying, ApplyHealth::Failing];

    fn last_applied_secs(self) -> u64 {
        match self {
            ApplyHealth::Applying => WEDGED_LAST_APPLIED,
            ApplyHealth::Failing => DEAD_LAST_APPLIED,
        }
    }

    fn name(self) -> &'static str {
        match self {
            ApplyHealth::Applying => "apply_ok",
            ApplyHealth::Failing => "apply_failing",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct M6Cell {
    base: Cell,
    corr: Corroboration,
    health: ApplyHealth,
}

impl M6Cell {
    fn name(&self) -> String {
        format!(
            "{} | {} | {}",
            self.base.name(),
            self.corr.name(),
            self.health.name()
        )
    }

    /// Every cell below carries fork evidence by construction (12 empty-header
    /// responses at minimum). This is the census predicate's gap half.
    fn below_snap_floor(&self) -> bool {
        self.base.gap.gap() < thresholds::SNAP_SYNC_GAP_MIN
    }
}

fn all_m6_cells() -> Vec<M6Cell> {
    let mut out = Vec::new();
    for base in all_cells() {
        for corr in Corroboration::ALL {
            for health in ApplyHealth::ALL {
                out.push(M6Cell { base, corr, health });
            }
        }
    }
    out
}

fn m6_ctx(cell: M6Cell) -> RecoveryContext {
    let mut ctx = cell_ctx(cell.base);
    ctx.last_applied_secs = cell.health.last_applied_secs();
    ctx
}

fn m6_coordinator(cell: M6Cell) -> RecoveryCoordinator {
    let mut coord = RecoveryCoordinator::new();
    let gap = cell.base.gap.gap();
    for _ in 0..12 {
        coord.report(RecoveryEvidence::EmptyHeaders {
            peer: PeerId::random(),
            gap,
        });
    }
    if cell.corr == Corroboration::Corroborated {
        coord.report(RecoveryEvidence::StuckFork { gap });
    }
    coord.report(RecoveryEvidence::StaleTip {
        last_applied_secs: cell.health.last_applied_secs(),
        gap,
    });
    if cell.health == ApplyHealth::Failing {
        for i in 0..5 {
            coord.report(RecoveryEvidence::ApplyFailure {
                height: LOCAL_H + i,
            });
        }
    }
    if cell.base.sibling == Budget::Spent {
        for _ in 0..thresholds::SIBLING_FETCH_MAX {
            coord.record_action(RecoveryAction::SiblingFetch { height: LOCAL_H });
        }
    }
    coord.clear_cooldown_for_test();
    coord
}

/// Identical tick model to M3's `drive`: 30s elapses between ticks and an executed
/// `ShallowRollback` burns the Node-side budget; the node's condition does not improve.
fn drive_m6(cell: M6Cell) -> Vec<RecoveryAction> {
    let mut coord = m6_coordinator(cell);
    let mut ctx = m6_ctx(cell);
    let mut seq = Vec::with_capacity(TICKS);
    for _ in 0..TICKS {
        let action = coord.classify(&ctx);
        coord.record_action(action);
        coord.clear_cooldown_for_test();
        if let RecoveryAction::ShallowRollback { depth } = action {
            ctx.shallow_rollback_count = ctx.shallow_rollback_count.saturating_add(depth);
        }
        seq.push(action);
    }
    seq
}

/// The census predicate: a cell counts when it carries fork evidence, sits below
/// `SNAP_SYNC_GAP_MIN`, and admits `SnapSync` or `GenesisResync` at any tick.
fn lossy_admissions() -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for cell in all_m6_cells() {
        if !cell.below_snap_floor() {
            continue;
        }
        if let Some(action) = drive_m6(cell).into_iter().find(|a| is_lossy(*a)) {
            out.push((cell.name(), rung_name(action)));
        }
    }
    out
}

// ===========================================================================
// (E) The structural invariant
// ===========================================================================

/// REQ-FORK-004 — Decision: a failure here names the exact (gap, finality, budget,
/// corroboration, apply-health) cells in which a forked node can still trade its
/// block history for a snapshot or a state wipe below gap 500, which is the one
/// property M6 exists to make structural rather than evidence-dependent.
#[test]
fn m6_no_fork_shaped_cell_reaches_a_lossy_action() {
    let failures = lossy_admissions();
    let shown: Vec<String> = failures
        .iter()
        .take(12)
        .map(|(cell, rung)| format!("{cell} -> {rung}"))
        .collect();

    assert!(
        failures.is_empty(),
        "REQ-FORK-004: {} of {} fork-shaped cells below SNAP_SYNC_GAP_MIN still admit a \
         history-destroying rung. Snap admission must narrow to bootstrap, the genesis \
         window, gap >= {} and the operator flag. First {}:\n  {}",
        failures.len(),
        all_m6_cells()
            .iter()
            .filter(|c| c.below_snap_floor())
            .count(),
        thresholds::SNAP_SYNC_GAP_MIN,
        shown.len(),
        shown.join("\n  ")
    );
}

/// REQ-FORK-004 — Decision: a failure here reveals that the enumeration went
/// vacuous — that it stopped reaching the rungs it is supposed to discriminate
/// between — so a green census above would prove nothing about the ladder.
#[test]
fn m6_census_enumeration_is_not_vacuous() {
    let mut snap_above_floor = 0usize;
    let mut rollbacks = 0usize;
    let mut siblings = 0usize;
    let mut wedges = 0usize;

    for cell in all_m6_cells() {
        for action in drive_m6(cell) {
            match action {
                RecoveryAction::SnapSync if !cell.below_snap_floor() => snap_above_floor += 1,
                RecoveryAction::ShallowRollback { .. } => rollbacks += 1,
                RecoveryAction::SiblingFetch { .. } => siblings += 1,
                RecoveryAction::Wedged { .. } => wedges += 1,
                _ => {}
            }
        }
    }

    assert!(
        snap_above_floor > 0,
        "anti-vacuity: no cell at gap >= {} reaches SnapSync. LB-5 requires snap to stay \
         reachable for genuine behind-ness — a census that never reaches it is a census \
         of nothing.",
        thresholds::SNAP_SYNC_GAP_MIN
    );
    assert!(
        rollbacks > 0,
        "anti-vacuity: no cell reaches ShallowRollback — the enumeration no longer \
         exercises Rule 1/1b"
    );
    assert!(
        siblings > 0,
        "anti-vacuity: no cell reaches SiblingFetch — the enumeration no longer \
         exercises the bounded non-destructive probe"
    );
    assert!(
        wedges > 0,
        "anti-vacuity: no cell reaches the Wedged terminal — the enumeration no longer \
         exercises the ladder's absorbing state"
    );
}

// ===========================================================================
// (F) The outcome probe — brief §8
// ===========================================================================

/// REQ-FORK-004 — Decision: a failure here, with its printed count, is the
/// milestone's before/after metric: any non-zero value means a fork-shaped node
/// below gap 500 can still be admitted to a history-destroying action.
///
/// Probe: `cargo test -p network --lib m6_lossy_admission_census -- --nocapture`.
#[test]
fn m6_lossy_admission_census() {
    let failures = lossy_admissions();
    println!("LOSSY_ADMISSION_CELLS={}", failures.len());

    assert_eq!(
        failures.len(),
        0,
        "REQ-FORK-004 (brief §8): target is 0. Offending cells: {:?}",
        failures.iter().take(8).collect::<Vec<_>>()
    );
}
