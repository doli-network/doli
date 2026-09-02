//! INC-I-204 M3 / REQ-FORK-010 — recovery-ladder termination + `Wedged` terminal.
//! TESTS-FIRST (RED).
//!
//! Measured defect (§4.3): a node with `tip == finality`, `0 < gap < 50` and an
//! exhausted `SiblingFetch` budget matches no terminating rung. Rule 1 needs
//! `recently_synced()` (false for a wedged node); Rule 1b's finality guard
//! correctly refuses the rollback and the sibling budget runs out; Rule 2 needs
//! `gap >= 50` or `gap >= 500` or a spent rollback budget; so Rule 3's
//! `medium_gap` fires and returns `HeaderFirstSync` every 30s forever, against a
//! dispatch arm that is comment-only. The only exit is to fall FURTHER behind
//! until `deep_fork_confirmed` opens `SnapSync` — which destroys block history.
//!
//! M3's contract: every rung terminates or returns `None`, and exactly one
//! absorbing state survives — `RecoveryAction::Wedged { reason }`, named,
//! non-lossy, evidence-acquiring, and never a branch decision (B-F1).
//!
//! OUTPUT CONTRACT — `fn RecoveryCoordinator::classify(&self, &RecoveryContext)
//!                       -> RecoveryAction`
//!   O1 mutable params: none (`&self`, `&ctx`).   O2 receiver mutation: none (pure).
//!   O3 return value: `RecoveryAction` — the ONLY observable output.
//!   O4 persistent store: none.  O5 statics: none.  O6 events: tracing only.
//!
//! OUTPUT CONTRACT — `fn RecoveryCoordinator::record_action(&mut self, RecoveryAction)`
//!   O1 mutable params: none.   O2 receiver mutation: `last_action` (cooldown) and
//!      `sibling_fetch_attempts` (budget) — both observable only through a later
//!      `classify()`, which is how every assertion below reads them.
//!   O3 return value: `()`.     O4/O5/O6: none.
//!
//!   PATHS of `classify` (the ladder, first match wins):
//!     P0 grace period                                  → `None`
//!     P0b applied-since-rollback + orphan/empty        → `HeaderFirstSync`
//!     P0c cooldown active                              → `None`
//!     P1  minor-fork evidence + small gap + recently synced + rollback budget
//!         P1a finality allows  → `ShallowRollback{1}`   P1b refused → `None`
//!     P1b' StuckFork + small gap + rollback budget
//!         P1b'a finality allows → `ShallowRollback{1}`
//!         P1b'b refused + sibling budget → `SiblingFetch{local_height}`
//!         P1b'c refused + sibling spent  → FALL THROUGH  ← the absorbing cell
//!     P2  rollback-exhausted | gap>=500 | deep-fork, snap budget, peers>=3 → `SnapSync`
//!     P3  0<gap<500 | stale tip                        → `HeaderFirstSync`
//!     P4  apply-fails + snap exhausted                 → `GenesisResync`
//!     P5  else                                         → `None`
//!
//!   INPUT PARTITIONS — the C-6 enumeration below is the full cross product
//!     gap band {AtTip, Minor, Medium, Large}
//!       × finality {AtFinality, AboveFinality, Unfinalized}
//!       × rollback budget {Fresh, Spent}
//!       × sibling budget {Fresh, Spent}
//!       × snap budget {Fresh, Spent}   = 96 cells, every one named and asserted.
//!
//!   MATRIX: O3 × {P1..P5} — every path is the fixpoint of at least one cell, and
//!   every cell's fixpoint is checked against the named-terminal predicate.
//!
//! FAIL EVIDENCE (pre-fix, `recovery.rs` classifier unmodified):
//!   RED  — `m3_incident_shape_*`, `c6_every_cell_*`
//!   PASS-LOCK (green before AND after) — `c6_enumeration_is_not_vacuous_*`,
//!          `m3_wedged_is_exitable_by_corroboration`
//!   Per-rung proofs live in `tests_inc_i204_m3_rungs.rs`; traps, the C3 cooldown
//!   contract, INV-SYNC-006 and B-F1 live in `tests_inc_i204_m3_traps.rs`.

use libp2p::PeerId;

use crate::sync::manager::recovery::{
    thresholds, RecoveryAction, RecoveryContext, RecoveryCoordinator, RecoveryEvidence, WedgeReason,
};

/// Our tip for every fixture. Kept constant so `finality` positions are exact.
pub(super) const LOCAL_H: u64 = 1_000;

/// The measured cohort size (INC-I-204: nine wedged nodes).
pub(super) const COHORT_PEERS: usize = 9;

/// Seconds since last apply in a wedged node — past `STALE_TIP_SECS`.
pub(super) const WEDGED_LAST_APPLIED: u64 = 420;

/// Ticks driven per cell. > SHALLOW_ROLLBACK_MAX + SIBLING_FETCH_MAX so every
/// bounded budget is spent inside the run and any true fixpoint is reached.
const TICKS: usize = 20;

/// Trailing ticks that must be identical for the sequence to count as settled.
const FIXPOINT_WINDOW: usize = 4;

// ===========================================================================
// The C-6 cell space
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GapBand {
    AtTip,
    Minor,
    Medium,
    Large,
}

impl GapBand {
    pub(super) const ALL: [GapBand; 4] = [
        GapBand::AtTip,
        GapBand::Minor,
        GapBand::Medium,
        GapBand::Large,
    ];

    pub(super) fn gap(self) -> u64 {
        match self {
            GapBand::AtTip => 0,
            GapBand::Minor => 10,
            GapBand::Medium => 120,
            GapBand::Large => 900,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            GapBand::AtTip => "gap=0",
            GapBand::Minor => "0<gap<50",
            GapBand::Medium => "50<=gap<500",
            GapBand::Large => "gap>=500",
        }
    }

    /// True when snap sync is a legitimate remedy for this band (LB-5): the node
    /// is genuinely far behind, not resolving a fork.
    pub(super) fn snap_is_legitimate(self) -> bool {
        self.gap() >= thresholds::SNAP_SYNC_GAP_MIN
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinalityPos {
    AtFinality,
    AboveFinality,
    Unfinalized,
}

impl FinalityPos {
    pub(super) const ALL: [FinalityPos; 3] = [
        FinalityPos::AtFinality,
        FinalityPos::AboveFinality,
        FinalityPos::Unfinalized,
    ];

    pub(super) fn height(self) -> Option<u64> {
        match self {
            FinalityPos::AtFinality => Some(LOCAL_H),
            FinalityPos::AboveFinality => Some(LOCAL_H - 10),
            FinalityPos::Unfinalized => None,
        }
    }

    /// The INV-SYNC-001 guard verdict for a depth-1 rollback (strict `<`, LB-2).
    pub(super) fn rollback_refused(self) -> bool {
        match self.height() {
            Some(f) => LOCAL_H.saturating_sub(1) < f,
            None => false,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            FinalityPos::AtFinality => "tip==finality",
            FinalityPos::AboveFinality => "tip>finality",
            FinalityPos::Unfinalized => "no-finality",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Budget {
    Fresh,
    Spent,
}

impl Budget {
    pub(super) const ALL: [Budget; 2] = [Budget::Fresh, Budget::Spent];

    pub(super) fn name(self) -> &'static str {
        match self {
            Budget::Fresh => "fresh",
            Budget::Spent => "spent",
        }
    }
}

/// One cell of the C-6 enumeration. Every dimension is an exhaustively-matched
/// enum, so a new dimension variant cannot be introduced without naming it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Cell {
    pub(super) gap: GapBand,
    pub(super) fin: FinalityPos,
    pub(super) rollback: Budget,
    pub(super) sibling: Budget,
    pub(super) snap: Budget,
}

impl Cell {
    pub(super) fn name(&self) -> String {
        format!(
            "[{} | {} | rollback={} | sibling={} | snap={}]",
            self.gap.name(),
            self.fin.name(),
            self.rollback.name(),
            self.sibling.name(),
            self.snap.name()
        )
    }
}

/// Every reachable cell of the fork-evidence space.
pub(super) fn all_cells() -> Vec<Cell> {
    let mut out = Vec::new();
    for gap in GapBand::ALL {
        for fin in FinalityPos::ALL {
            for rollback in Budget::ALL {
                for sibling in Budget::ALL {
                    for snap in Budget::ALL {
                        out.push(Cell {
                            gap,
                            fin,
                            rollback,
                            sibling,
                            snap,
                        });
                    }
                }
            }
        }
    }
    out
}

// ===========================================================================
// Fixtures
// ===========================================================================

/// The evidence a wedged node actually accumulates: peers repeatedly return zero
/// headers for our tip hash (so our tip is not on their chain), the G3 detector
/// raises `StuckFork`, and nothing applies.
pub(super) fn report_wedge_evidence(coord: &mut RecoveryCoordinator, gap: u64) {
    for _ in 0..12 {
        coord.report(RecoveryEvidence::EmptyHeaders {
            peer: PeerId::random(),
            gap,
        });
    }
    coord.report(RecoveryEvidence::StuckFork { gap });
    coord.report(RecoveryEvidence::StaleTip {
        last_applied_secs: WEDGED_LAST_APPLIED,
        gap,
    });
}

pub(super) fn cell_ctx(cell: Cell) -> RecoveryContext {
    RecoveryContext {
        local_height: LOCAL_H,
        network_tip_height: LOCAL_H + cell.gap.gap(),
        peer_count: COHORT_PEERS,
        last_applied_secs: WEDGED_LAST_APPLIED,
        shallow_rollback_count: match cell.rollback {
            Budget::Fresh => 0,
            Budget::Spent => thresholds::SHALLOW_ROLLBACK_MAX,
        },
        snap_attempts: match cell.snap {
            Budget::Fresh => 0,
            Budget::Spent => thresholds::SNAP_ATTEMPTS_MAX,
        },
        last_rollback_local_height: None,
        last_rollback_time: None,
        in_grace_period: false,
        last_finality_height: cell.fin.height(),
    }
}

/// A coordinator carrying wedge evidence, with the sibling budget pre-spent when
/// the cell asks for it. The sibling budget is coordinator-internal, so it is
/// spent the only way production spends it — through `record_action`.
pub(super) fn cell_coordinator(cell: Cell) -> RecoveryCoordinator {
    let mut coord = RecoveryCoordinator::new();
    report_wedge_evidence(&mut coord, cell.gap.gap());
    if cell.sibling == Budget::Spent {
        for _ in 0..thresholds::SIBLING_FETCH_MAX {
            coord.record_action(RecoveryAction::SiblingFetch { height: LOCAL_H });
        }
    }
    coord.clear_cooldown_for_test();
    coord
}

// ===========================================================================
// Action taxonomy — exhaustive, so a new variant cannot dodge classification
// ===========================================================================

/// The metric label of a rung. The match is exhaustive: adding a `RecoveryAction`
/// variant without naming it here is a compile error (C-6's "every cell named").
pub(super) fn rung_name(action: RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::None => "none",
        RecoveryAction::SiblingFetch { .. } => "sibling_fetch",
        RecoveryAction::ShallowRollback { .. } => "shallow_rollback",
        RecoveryAction::HeaderFirstSync => "header_first_sync",
        RecoveryAction::SnapSync => "snap_sync",
        RecoveryAction::GenesisResync => "genesis_resync",
        RecoveryAction::Wedged { reason } => reason.label(),
    }
}

/// True when the rung replaces local state instead of extending it from the
/// current tip — i.e. it cannot satisfy INV-SYNC-006's parent-hash walk and
/// discards block history on the way.
pub(super) fn is_lossy(action: RecoveryAction) -> bool {
    match action {
        RecoveryAction::SnapSync | RecoveryAction::GenesisResync => true,
        RecoveryAction::None
        | RecoveryAction::SiblingFetch { .. }
        | RecoveryAction::ShallowRollback { .. }
        | RecoveryAction::HeaderFirstSync
        | RecoveryAction::Wedged { .. } => false,
    }
}

/// REQ-FORK-010's acceptance predicate: is `action`, repeated forever at this
/// cell, a NAMED terminal — or is it a rung that neither acts nor names itself?
pub(super) fn fixpoint_verdict(cell: Cell, action: RecoveryAction) -> Result<(), String> {
    match action {
        RecoveryAction::None => Ok(()),
        RecoveryAction::Wedged { .. } => Ok(()),
        RecoveryAction::SnapSync => {
            if cell.gap.snap_is_legitimate() {
                Ok(())
            } else {
                Err(format!(
                    "terminates via SnapSync at {} — the LOSSY exit reached by getting worse \
                     (§4.3 / trap T8): block history is destroyed to resolve a fork",
                    cell.gap.name()
                ))
            }
        }
        RecoveryAction::HeaderFirstSync => Err(
            "cycles on HeaderFirstSync: peers returned empty headers for our tip hash, so \
             header-first cannot extend from it — a rung that cannot act must return None \
             and name a terminal (C3)"
                .to_string(),
        ),
        RecoveryAction::SiblingFetch { .. } => Err(
            "cycles on SiblingFetch: the INC-I-143 bound (SIBLING_FETCH_MAX) no longer \
             terminates the probe (trap T11's inverse)"
                .to_string(),
        ),
        RecoveryAction::ShallowRollback { .. } => Err(
            "cycles on ShallowRollback: SHALLOW_ROLLBACK_MAX never closes the rung, so the \
             node erodes its own chain without ever naming a terminal"
                .to_string(),
        ),
        RecoveryAction::GenesisResync => Err(
            "terminates via GenesisResync: the most lossy rung on the ladder is not a \
             fork remedy — full state is wiped"
                .to_string(),
        ),
    }
}

/// Drive `classify -> record_action` for `TICKS`, modelling exactly what
/// `periodic.rs` does between ticks: 30s elapses (cooldown clears) and an
/// executed `ShallowRollback` burns the Node-side budget. The node's condition
/// itself does NOT improve — that is the wedge being modelled.
pub(super) fn drive(cell: Cell) -> Vec<RecoveryAction> {
    let mut coord = cell_coordinator(cell);
    let mut ctx = cell_ctx(cell);
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

/// `Some(action)` when the tail of `seq` has settled on one rung, `None` when it
/// is still oscillating (a multi-step cycle).
pub(super) fn settled(seq: &[RecoveryAction]) -> Option<RecoveryAction> {
    let tail = &seq[seq.len() - FIXPOINT_WINDOW..];
    if tail.iter().all(|a| *a == tail[0]) {
        Some(tail[0])
    } else {
        None
    }
}

// ===========================================================================
// (1) INCIDENT SHAPE — MUST FAIL pre-fix
// ===========================================================================

/// REQ-FORK-010 — Decision: a failure here reveals that a wedged cohort still has
/// no named exit, so recovery continues to require the node's condition to
/// DEGRADE past gap=50 and then escapes only by destroying its block history —
/// the exact seven-day INC-I-204 shape, reproduced from the classifier alone.
///
/// The trajectory is the measured one: nine peers, `tip == finality` (so every
/// rollback is correctly refused by INV-SYNC-001), gap starting at 0 and growing
/// as the cohort produces without us. Covers O3 on P1b'c → P2/P3.
#[test]
fn m3_incident_shape_wedged_cohort_must_reach_a_named_non_lossy_terminal() {
    let mut coord = RecoveryCoordinator::new();
    let mut seen = Vec::new();

    for tick in 0..24u64 {
        let gap = tick * 3;
        report_wedge_evidence(&mut coord, gap);
        let ctx = RecoveryContext {
            local_height: LOCAL_H,
            network_tip_height: LOCAL_H + gap,
            peer_count: COHORT_PEERS,
            last_applied_secs: WEDGED_LAST_APPLIED + tick * 30,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            last_rollback_time: None,
            in_grace_period: false,
            last_finality_height: Some(LOCAL_H),
        };
        let action = coord.classify(&ctx);
        coord.record_action(action);
        coord.clear_cooldown_for_test();
        seen.push((gap, action));
    }

    let lossy: Vec<String> = seen
        .iter()
        .filter(|(_, a)| is_lossy(*a))
        .map(|(g, a)| format!("gap={g} -> {}", rung_name(*a)))
        .collect();
    assert!(
        lossy.is_empty(),
        "REQ-FORK-010: a wedged node must never resolve a FORK by discarding its block \
         history. The classifier escalated to a lossy rung at: {lossy:?}. This is §4.3's \
         'recovery requires getting worse' — the node had to fall past MINOR_FORK_GAP_MAX \
         to earn an exit, and the exit it earned wipes the chain it was serving."
    );

    let sibling_probes = seen
        .iter()
        .filter(|(_, a)| matches!(a, RecoveryAction::SiblingFetch { .. }))
        .count();
    assert_eq!(
        sibling_probes,
        thresholds::SIBLING_FETCH_MAX as usize,
        "LB-9 / trap T11: the non-destructive sibling probe must run exactly \
         SIBLING_FETCH_MAX times before the ladder moves on — not fewer (the INC-I-143 \
         454-refusal livelock returns) and not more (an unbounded probe)."
    );

    let wedged = seen
        .iter()
        .filter(|(_, a)| matches!(a, RecoveryAction::Wedged { .. }))
        .count();
    assert!(
        wedged > 0,
        "REQ-FORK-010: after the sibling budget is spent the node has no rung that can \
         act, yet it never named that condition. Observed trajectory: {:?}. It must reach \
         RecoveryAction::Wedged — named, alarmed, non-lossy, evidence-acquiring.",
        seen.iter()
            .map(|(g, a)| format!("{g}:{}", rung_name(*a)))
            .collect::<Vec<_>>()
    );

    let (_, last) = seen.last().copied().expect("trajectory is non-empty");
    assert_eq!(
        last,
        RecoveryAction::Wedged {
            reason: WedgeReason::FinalityConflict
        },
        "REQ-FORK-010: the terminal for `tip == finality` must be named \
         FinalityConflict so the wedge is alarmable and distinguishable from mere \
         behind-ness. Got {}.",
        rung_name(last)
    );
}

// ===========================================================================
// (2) C-6 EXHAUSTIVE CELL ENUMERATION — the acceptance gate
// ===========================================================================

/// REQ-FORK-010 — Decision: a failure here names the exact (gap, finality, budget)
/// cells for which the ladder still has no terminating rung, which is the only
/// evidence that would let M6 delete snap's fork reachability without stranding a
/// reachable state.
///
/// Every one of the 96 cells is driven to a fixpoint and judged by
/// `fixpoint_verdict`. Covers O3 × every path P1..P5.
#[test]
fn c6_every_cell_of_the_recovery_state_space_has_a_named_terminating_rung() {
    let cells = all_cells();
    assert_eq!(
        cells.len(),
        4 * 3 * 2 * 2 * 2,
        "C-6 fixture: the enumeration must cover the full cross product"
    );

    let mut failures: Vec<String> = Vec::new();
    for cell in cells {
        let seq = drive(cell);
        match settled(&seq) {
            None => failures.push(format!(
                "{} CYCLES: last {FIXPOINT_WINDOW} ticks were {:?}",
                cell.name(),
                seq[seq.len() - FIXPOINT_WINDOW..]
                    .iter()
                    .map(|a| rung_name(*a))
                    .collect::<Vec<_>>()
            )),
            Some(fixpoint) => {
                if let Err(why) = fixpoint_verdict(cell, fixpoint) {
                    failures.push(format!("{} {}", cell.name(), why));
                }
            }
        }
    }

    let shown: Vec<&String> = failures.iter().take(12).collect();
    assert!(
        failures.is_empty(),
        "REQ-FORK-010 (C-6): {} of 96 cells have no named, non-lossy terminating rung. \
         First {}:\n  {}",
        failures.len(),
        shown.len(),
        shown
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// REQ-FORK-010 — Decision: a failure here reveals that the enumeration above went
/// vacuous — that the cells stopped exercising distinct rungs — so a later green
/// C-6 run would prove nothing about the ladder.
#[test]
fn c6_enumeration_is_not_vacuous_every_ladder_rung_is_exercised() {
    let mut hit: Vec<&'static str> = Vec::new();
    for cell in all_cells() {
        for action in drive(cell) {
            let name = rung_name(action);
            if !hit.contains(&name) {
                hit.push(name);
            }
        }
    }
    for expected in ["shallow_rollback", "sibling_fetch", "snap_sync"] {
        assert!(
            hit.contains(&expected),
            "C-6 fixture went vacuous: rung `{expected}` is never reached by any cell, so \
             the enumeration no longer covers the ladder. Reached: {hit:?}"
        );
    }
}

// ===========================================================================
// (4) THE TERMINAL IS A STOP, NOT A LATCH
// ===========================================================================

/// REQ-FORK-010 — Decision: a failure here reveals that `Wedged` became a sticky
/// latch that outlives the condition that caused it, which would strand a node
/// that the network already re-absorbed.
#[test]
fn m3_wedged_is_exitable_by_corroboration() {
    let cell = Cell {
        gap: GapBand::Minor,
        fin: FinalityPos::AtFinality,
        rollback: Budget::Fresh,
        sibling: Budget::Spent,
        snap: Budget::Fresh,
    };
    let mut coord = cell_coordinator(cell);
    let wedge = coord.classify(&cell_ctx(cell));
    coord.record_action(wedge);
    coord.clear_cooldown_for_test();

    let resolved = RecoveryContext {
        local_height: LOCAL_H + 40,
        network_tip_height: LOCAL_H + 40,
        peer_count: COHORT_PEERS,
        last_applied_secs: 3,
        shallow_rollback_count: 0,
        snap_attempts: 0,
        last_rollback_local_height: None,
        last_rollback_time: None,
        in_grace_period: false,
        last_finality_height: Some(LOCAL_H),
    };
    assert!(
        !matches!(coord.classify(&resolved), RecoveryAction::Wedged { .. }),
        "REQ-FORK-010: the competing branch arrived and the node advanced 40 blocks above \
         finality, yet the classifier still reports Wedged. The terminal must be a \
         verdict on the CURRENT context, never a latch."
    );
}
