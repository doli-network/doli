//! Recovery Coordinator (2026-04-15, synmgrefactor branch)
//!
//! The `#[allow(dead_code)]` below is intentional: the coordinator is fully
//! implemented and tested here, but not yet wired into the production event
//! loop. Integration (detectors reporting evidence + periodic.rs dispatching
//! the action) lands in a follow-up commit so each step can be reviewed and
//! reverted independently.
//!
//! Centralizes the "node detected divergence, what should recovery action be"
//! decision. Before this module, each detector (orphan gossip counter, empty
//! headers counter, apply failures counter, deep fork detector, periodic tick)
//! was directly wired to its own recovery action (start_sync, signal_stuck_fork,
//! request_genesis_resync, shallow_rollback). Adding a new detector required
//! wiring it to one of the actions, and changing behavior for a given evidence
//! pattern meant hunting across multiple files.
//!
//! The Coordinator's responsibility: ingest evidence from detectors, classify
//! the aggregate into a single `RecoveryAction`, and hand that action back to
//! the caller to execute. Detectors now only REPORT — they don't decide.
//!
//! # Structure
//!
//! - `RecoveryEvidence` — enum of signals a detector can report
//! - `RecoveryAction` — enum of recovery responses (monotonic in severity)
//! - `RecoveryContext` — the node-wide snapshot the classifier reads
//! - `RecoveryCoordinator` — holds evidence window + cooldown state
//!
//! # Classifier rule ordering
//!
//! Rules are evaluated from LEAST to MOST invasive. First match wins. The
//! ordering is:
//!
//! 1. Grace period active → None (recovery suppressed by another path)
//! 2. Recent rollback + apply succeeded → None (we're behind, not forked)
//! 3. Cooldown active → None (just took an action, give it time)
//! 4. Few-block minor fork evidence + recently synced → ShallowRollback(1)
//! 5. Medium gap OR stale tip → HeaderFirstSync
//! 6. Deep fork OR rollback exhausted OR large gap → SnapSync
//! 7. Apply failures exhausted AND snap exhausted → GenesisResync
//!
//! # Design constraints
//!
//! - **Deterministic**: given the same evidence + context, always returns the
//!   same action. No hidden state beyond what's declared.
//! - **No direct side effects**: classify() is a pure read. The caller
//!   executes the action.
//! - **Evidence decays**: old evidence (> EVIDENCE_TTL) is pruned so stale
//!   signals don't trigger actions long after the fact.
//! - **Cooldown**: an action is not repeated within COOLDOWN to let it take
//!   effect before re-evaluating.
//!
//! # Non-goals
//!
//! - This is NOT a consensus primitive. state_root / block validation are
//!   untouched. The coordinator only decides WHICH RECOVERY action to take,
//!   never whether a block is valid.
//! - This is NOT a state machine — there is no "current state" the
//!   coordinator owns. Every call to classify() evaluates the whole context
//!   from scratch.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use libp2p::PeerId;

/// A signal that a detector reports to the coordinator.
///
/// Each variant represents a concrete observation. Detectors never decide
/// actions — they only report evidence. The classifier correlates these.
#[derive(Debug, Clone)]
pub enum RecoveryEvidence {
    /// A peer returned 0 headers for our tip hash in a GetHeaders response.
    /// Indicates the peer does not recognize our chain at that hash.
    EmptyHeaders { peer: PeerId, gap: u64 },

    /// A gossiped block arrived whose prev_hash does not match our local tip.
    /// Either we're behind canonical, or we're on a minority fork.
    OrphanGossip { slot: u32, gap: u64 },

    /// apply_block() returned an error (UTXO mismatch, consensus violation,
    /// etc). Repeated ApplyFailure suggests divergent state vs the network.
    ApplyFailure { height: u64 },

    /// Combined signal: many empty headers + significant gap + peers ahead.
    /// Explicit variant so detectors that already synthesize this can report
    /// once instead of forcing the classifier to re-synthesize from finer-
    /// grained variants.
    DeepForkSuspected { empty: u32, gap: u64 },

    /// We have not applied a block in `last_applied_secs` despite the network
    /// producing them (gap > threshold). Distinct from EmptyHeaders /
    /// OrphanGossip: it's a passive observation (nothing's arriving at all).
    StaleTip { last_applied_secs: u64, gap: u64 },
}

/// What the coordinator tells the caller to do.
///
/// Ordered by severity (None < ShallowRollback < HeaderFirstSync < SnapSync <
/// GenesisResync). The classifier always returns the LEAST severe action that
/// fits the evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// No action — evidence insufficient, cooldown active, or node is healthy.
    None,

    /// Shallow rollback: revert `depth` blocks and retry header-first sync.
    /// Typically depth=1 (minority fork resolution).
    ShallowRollback { depth: u32 },

    /// Normal header-first sync from current tip. For catchup when we've
    /// fallen behind but are still on canonical.
    HeaderFirstSync,

    /// Snap sync: download a state snapshot from peers with quorum.
    /// For gaps too large to close via header-first, or when header-first
    /// is blocked (empty headers → peers don't recognize our chain).
    SnapSync,

    /// Last resort: reset local state to genesis and re-sync from scratch.
    /// Only when snap sync has also failed.
    GenesisResync,
}

/// Everything the classifier needs to read from the node to decide an action.
///
/// Snapshot form so the classifier never holds locks. The caller gathers this
/// from SyncManager + Node once per tick and passes it in.
#[derive(Debug, Clone)]
pub struct RecoveryContext {
    pub local_height: u64,
    pub network_tip_height: u64,
    pub peer_count: usize,
    pub last_applied_secs: u64,
    pub shallow_rollback_count: u32,
    pub snap_attempts: u8,
    /// Post-rollback local_height. Some(h) if the most recent rollback brought
    /// us to height h. local_height > h means we've applied at least one block
    /// since the rollback (Fix #2b-bis: "applied since rollback" = behind not
    /// forked).
    pub last_rollback_local_height: Option<u64>,
    /// True if we're inside a post-recovery grace window where another path
    /// owns recovery decisions (e.g. active snap sync, first canonical block
    /// wait).
    pub in_grace_period: bool,
}

impl RecoveryContext {
    /// gap from us to the network tip.
    pub fn gap(&self) -> u64 {
        self.network_tip_height.saturating_sub(self.local_height)
    }

    /// True if the most recent rollback was followed by at least one apply.
    /// Fix #2b-bis discriminator.
    pub fn applied_since_rollback(&self) -> bool {
        match self.last_rollback_local_height {
            Some(h) => self.local_height > h,
            None => false,
        }
    }

    /// "Recently synced" heuristic — we had a successful apply < 60s ago.
    pub fn recently_synced(&self) -> bool {
        self.last_applied_secs < 60
    }
}

/// Classifier thresholds. Named constants so tuning is visible and testable.
pub mod thresholds {
    /// How many EmptyHeaders/OrphanGossip/ApplyFailure events required before
    /// escalation to an action. Matches the 3-consecutive threshold used by
    /// the pre-coordinator detectors.
    pub const MIN_MINOR_FORK_EVIDENCE: u32 = 3;

    /// Gap threshold separating "minor fork" (shallow rollback can resolve)
    /// from "behind canonical" (header-first sync).
    pub const MINOR_FORK_GAP_MAX: u64 = 50;

    /// Gap threshold above which header-first is not viable (peers don't have
    /// our old hashes) — escalate to snap sync.
    pub const SNAP_SYNC_GAP_MIN: u64 = 500;

    /// Shallow rollback call attempts before escalating to snap.
    pub const SHALLOW_ROLLBACK_MAX: u32 = 10;

    /// Snap sync attempts before genesis resync.
    pub const SNAP_ATTEMPTS_MAX: u8 = 3;

    /// Peer count required for snap sync (quorum).
    pub const SNAP_MIN_PEERS: usize = 3;

    /// Stale tip threshold — last apply older than this triggers HeaderFirst
    /// at minimum, or SnapSync escalation.
    pub const STALE_TIP_SECS: u64 = 300;
}

/// Centralized recovery decision maker.
///
/// Stores a rolling window of evidence reports and the last action taken.
/// `classify()` reads both plus a `RecoveryContext` to decide the current
/// action. `record_action()` updates the cooldown state after the caller
/// executes the action.
pub struct RecoveryCoordinator {
    evidence: VecDeque<(Instant, RecoveryEvidence)>,
    last_action: Option<(Instant, RecoveryAction)>,
}

/// Max evidence entries retained (prevents unbounded growth under event floods).
const MAX_ENTRIES: usize = 256;
/// How long an evidence entry lives before pruning.
const EVIDENCE_TTL: Duration = Duration::from_secs(120);
/// Minimum time between same-action repeats.
const ACTION_COOLDOWN: Duration = Duration::from_secs(5);

impl RecoveryCoordinator {
    /// Create an empty coordinator with no evidence and no prior action.
    pub fn new() -> Self {
        Self {
            evidence: VecDeque::new(),
            last_action: None,
        }
    }

    /// A detector reports one piece of evidence.
    pub fn report(&mut self, evidence: RecoveryEvidence) {
        let now = Instant::now();
        self.evidence.push_back((now, evidence));
        self.prune(now);
    }

    /// Apply the decision rules to the current evidence + context.
    ///
    /// Pure function of (self.evidence, self.last_action, ctx, now). No
    /// side effects. Safe to call repeatedly.
    pub fn classify(&self, ctx: &RecoveryContext) -> RecoveryAction {
        // Gate 0: grace period — another recovery path owns the decision.
        if ctx.in_grace_period {
            return RecoveryAction::None;
        }

        // Gate 1: applied since rollback → Fix #2b-bis semantics.
        // After a rollback, if we applied any block, we reconnected to
        // canonical. Further orphan accumulation means we're behind, not
        // forked — don't rollback again.
        if ctx.applied_since_rollback() && !self.evidence.is_empty() {
            let has_orphan_or_empty = self.evidence.iter().any(|(_, e)| {
                matches!(
                    e,
                    RecoveryEvidence::OrphanGossip { .. } | RecoveryEvidence::EmptyHeaders { .. }
                )
            });
            if has_orphan_or_empty {
                // Use HeaderFirstSync instead of ShallowRollback.
                return RecoveryAction::HeaderFirstSync;
            }
        }

        // Gate 2: cooldown.
        if let Some((when, _)) = self.last_action {
            if when.elapsed() < ACTION_COOLDOWN {
                return RecoveryAction::None;
            }
        }

        let empty_count = self.count(|e| matches!(e, RecoveryEvidence::EmptyHeaders { .. }));
        let orphan_count = self.count(|e| matches!(e, RecoveryEvidence::OrphanGossip { .. }));
        let apply_fails = self.count(|e| matches!(e, RecoveryEvidence::ApplyFailure { .. }));
        let deep_fork = self.count(|e| matches!(e, RecoveryEvidence::DeepForkSuspected { .. }));
        let stale_tip = self
            .evidence
            .iter()
            .any(|(_, e)| matches!(e, RecoveryEvidence::StaleTip { .. }));

        let gap = ctx.gap();

        // --- Rule 1: shallow rollback for minor fork with recent apply ---
        //
        // Evidence: >= 3 empty headers OR orphan gossip blocks.
        // Preconditions: small gap AND recently synced AND rollback budget
        // not exhausted.
        let minor_fork_evidence = empty_count >= thresholds::MIN_MINOR_FORK_EVIDENCE
            || orphan_count >= thresholds::MIN_MINOR_FORK_EVIDENCE;

        if minor_fork_evidence
            && gap > 0
            && gap < thresholds::MINOR_FORK_GAP_MAX
            && ctx.recently_synced()
            && ctx.shallow_rollback_count < thresholds::SHALLOW_ROLLBACK_MAX
        {
            return RecoveryAction::ShallowRollback { depth: 1 };
        }

        // --- Rule 2: snap sync for deep fork, rollback exhausted, or large gap ---
        //
        // Evaluated BEFORE Rule 3 (header-first) because a large gap / deep
        // fork signal must not be intercepted by the medium-gap header-first
        // path. Severity ordering: SnapSync is more invasive than
        // HeaderFirstSync, but when the evidence clearly points to "cannot
        // catch up via headers" we must escalate.
        let rollback_exhausted =
            minor_fork_evidence && ctx.shallow_rollback_count >= thresholds::SHALLOW_ROLLBACK_MAX;
        let large_gap = gap >= thresholds::SNAP_SYNC_GAP_MIN;
        let deep_fork_confirmed = deep_fork > 0
            || (empty_count >= 10 && ctx.last_applied_secs >= thresholds::STALE_TIP_SECS);

        if (rollback_exhausted || large_gap || deep_fork_confirmed)
            && ctx.snap_attempts < thresholds::SNAP_ATTEMPTS_MAX
            && ctx.peer_count >= thresholds::SNAP_MIN_PEERS
        {
            return RecoveryAction::SnapSync;
        }

        // --- Rule 3: header-first sync for medium gap or stale tip ---
        //
        // Covers "we're behind canonical" without strong fork evidence.
        let medium_gap = gap > 0 && gap < thresholds::SNAP_SYNC_GAP_MIN;
        let stale_and_behind = stale_tip && gap > 0;
        if medium_gap || stale_and_behind {
            return RecoveryAction::HeaderFirstSync;
        }

        // --- Rule 4: last-resort genesis resync ---
        //
        // Only when apply keeps failing AND snap has also exhausted. This
        // should be very rare — it wipes local state.
        let truly_stuck = apply_fails >= 5 && ctx.last_applied_secs >= 600;
        let snap_exhausted = ctx.snap_attempts >= thresholds::SNAP_ATTEMPTS_MAX;
        if truly_stuck && snap_exhausted {
            return RecoveryAction::GenesisResync;
        }

        RecoveryAction::None
    }

    /// Notify the coordinator that the caller executed `action`. Starts the
    /// cooldown timer. No-op for `None`.
    pub fn record_action(&mut self, action: RecoveryAction) {
        if action != RecoveryAction::None {
            self.last_action = Some((Instant::now(), action));
        }
    }

    /// Drop evidence older than EVIDENCE_TTL and cap entries at MAX_ENTRIES.
    fn prune(&mut self, now: Instant) {
        while self
            .evidence
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > EVIDENCE_TTL)
        {
            self.evidence.pop_front();
        }
        while self.evidence.len() > MAX_ENTRIES {
            self.evidence.pop_front();
        }
    }

    fn count(&self, pred: impl Fn(&RecoveryEvidence) -> bool) -> u32 {
        self.evidence.iter().filter(|(_, e)| pred(e)).count() as u32
    }

    /// Test-only: count current evidence entries.
    #[cfg(test)]
    pub(crate) fn evidence_len(&self) -> usize {
        self.evidence.len()
    }
}

impl Default for RecoveryCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn base_ctx() -> RecoveryContext {
        RecoveryContext {
            local_height: 1000,
            network_tip_height: 1000,
            peer_count: 10,
            last_applied_secs: 5,
            shallow_rollback_count: 0,
            snap_attempts: 0,
            last_rollback_local_height: None,
            in_grace_period: false,
        }
    }

    fn fake_peer() -> PeerId {
        PeerId::random()
    }

    // --- Gate tests -----------------------------------------------------------

    #[test]
    fn grace_period_suppresses_all_actions() {
        let mut c = RecoveryCoordinator::new();
        for _ in 0..5 {
            c.report(RecoveryEvidence::OrphanGossip { slot: 1, gap: 5 });
        }
        let mut ctx = base_ctx();
        ctx.in_grace_period = true;
        ctx.network_tip_height = 1005;
        assert_eq!(c.classify(&ctx), RecoveryAction::None);
    }

    #[test]
    fn empty_evidence_healthy_chain_returns_none() {
        let c = RecoveryCoordinator::new();
        let ctx = base_ctx();
        assert_eq!(c.classify(&ctx), RecoveryAction::None);
    }

    #[test]
    fn cooldown_blocks_immediate_re_action() {
        let mut c = RecoveryCoordinator::new();
        c.record_action(RecoveryAction::ShallowRollback { depth: 1 });
        for _ in 0..3 {
            c.report(RecoveryEvidence::OrphanGossip { slot: 1, gap: 5 });
        }
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1005;
        assert_eq!(c.classify(&ctx), RecoveryAction::None);
    }

    // --- Applied-since-rollback (Fix #2b-bis) -------------------------------

    #[test]
    fn applied_since_rollback_skips_rollback_returns_headerfirst() {
        let mut c = RecoveryCoordinator::new();
        for _ in 0..3 {
            c.report(RecoveryEvidence::OrphanGossip { slot: 1, gap: 3 });
        }
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1003;
        // Most recent rollback was to 999; we're now at 1000 → applied since.
        ctx.last_rollback_local_height = Some(999);
        assert_eq!(c.classify(&ctx), RecoveryAction::HeaderFirstSync);
    }

    #[test]
    fn no_applied_since_rollback_allows_shallow_rollback() {
        let mut c = RecoveryCoordinator::new();
        for _ in 0..3 {
            c.report(RecoveryEvidence::OrphanGossip { slot: 1, gap: 3 });
        }
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1003;
        // Most recent rollback was to 1000 and we are still at 1000.
        ctx.last_rollback_local_height = Some(1000);
        assert_eq!(
            c.classify(&ctx),
            RecoveryAction::ShallowRollback { depth: 1 }
        );
    }

    // --- Rule 1: shallow rollback --------------------------------------------

    #[test]
    fn three_orphans_small_gap_recent_apply_triggers_shallow_rollback() {
        let mut c = RecoveryCoordinator::new();
        for _ in 0..3 {
            c.report(RecoveryEvidence::OrphanGossip { slot: 1, gap: 3 });
        }
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1003;
        assert_eq!(
            c.classify(&ctx),
            RecoveryAction::ShallowRollback { depth: 1 }
        );
    }

    #[test]
    fn three_empty_headers_small_gap_triggers_shallow_rollback() {
        let mut c = RecoveryCoordinator::new();
        for _ in 0..3 {
            c.report(RecoveryEvidence::EmptyHeaders {
                peer: fake_peer(),
                gap: 2,
            });
        }
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1002;
        assert_eq!(
            c.classify(&ctx),
            RecoveryAction::ShallowRollback { depth: 1 }
        );
    }

    #[test]
    fn shallow_rollback_exhausted_escalates_to_snap() {
        let mut c = RecoveryCoordinator::new();
        for _ in 0..3 {
            c.report(RecoveryEvidence::OrphanGossip { slot: 1, gap: 3 });
        }
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1003;
        ctx.shallow_rollback_count = thresholds::SHALLOW_ROLLBACK_MAX;
        assert_eq!(c.classify(&ctx), RecoveryAction::SnapSync);
    }

    // --- Rule 2: header-first sync --------------------------------------------

    #[test]
    fn medium_gap_no_fork_evidence_triggers_headerfirst() {
        let c = RecoveryCoordinator::new();
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1100;
        assert_eq!(c.classify(&ctx), RecoveryAction::HeaderFirstSync);
    }

    #[test]
    fn stale_tip_triggers_headerfirst() {
        let mut c = RecoveryCoordinator::new();
        c.report(RecoveryEvidence::StaleTip {
            last_applied_secs: 120,
            gap: 3,
        });
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1003;
        ctx.last_applied_secs = 120;
        assert_eq!(c.classify(&ctx), RecoveryAction::HeaderFirstSync);
    }

    // --- Rule 3: snap sync ---------------------------------------------------

    #[test]
    fn large_gap_triggers_snap() {
        let c = RecoveryCoordinator::new();
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1600;
        assert_eq!(c.classify(&ctx), RecoveryAction::SnapSync);
    }

    #[test]
    fn deep_fork_suspected_triggers_snap() {
        let mut c = RecoveryCoordinator::new();
        c.report(RecoveryEvidence::DeepForkSuspected {
            empty: 15,
            gap: 100,
        });
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1100;
        assert_eq!(c.classify(&ctx), RecoveryAction::SnapSync);
    }

    #[test]
    fn snap_attempts_exhausted_blocks_snap() {
        let c = RecoveryCoordinator::new();
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1600;
        ctx.snap_attempts = thresholds::SNAP_ATTEMPTS_MAX;
        // Gap is huge but snap exhausted AND not truly stuck on apply → None.
        assert_eq!(c.classify(&ctx), RecoveryAction::None);
    }

    #[test]
    fn insufficient_peers_blocks_snap() {
        let c = RecoveryCoordinator::new();
        let mut ctx = base_ctx();
        ctx.network_tip_height = 1600;
        ctx.peer_count = 1;
        assert_eq!(c.classify(&ctx), RecoveryAction::None);
    }

    // --- Rule 4: genesis resync ----------------------------------------------

    #[test]
    fn apply_failures_plus_snap_exhausted_triggers_genesis_resync() {
        let mut c = RecoveryCoordinator::new();
        for i in 0..5 {
            c.report(RecoveryEvidence::ApplyFailure { height: 1000 + i });
        }
        let mut ctx = base_ctx();
        ctx.last_applied_secs = 700;
        ctx.snap_attempts = thresholds::SNAP_ATTEMPTS_MAX;
        assert_eq!(c.classify(&ctx), RecoveryAction::GenesisResync);
    }

    // --- Severity ordering / monotonicity ------------------------------------

    #[test]
    fn actions_are_ordered_by_severity() {
        // Sanity: make sure the variants order makes sense (derived equality
        // plus documented ordering).
        assert_ne!(RecoveryAction::None, RecoveryAction::HeaderFirstSync);
        assert_ne!(RecoveryAction::HeaderFirstSync, RecoveryAction::SnapSync);
        assert_ne!(RecoveryAction::SnapSync, RecoveryAction::GenesisResync);
    }

    // --- Replay: 2026-04-15 folsi cascade ------------------------------------

    /// Reproduces the folsi cascade situation on 2026-04-15 14:52-14:56.
    /// After the first rollback, folsi applied h=34558 canonical successfully.
    /// Then orphans accumulated because peers advanced. Pre-fix, Fix #2b
    /// signaled stuck_fork and Fix #2c rolled back AGAIN — 18+ more times.
    /// Post-fix, the coordinator should see `applied_since_rollback` and
    /// return HeaderFirstSync instead of ShallowRollback.
    #[test]
    fn replay_folsi_cascade_returns_headerfirst_not_rollback() {
        let mut c = RecoveryCoordinator::new();
        // After initial rollback and apply, orphans arrive from ahead peers.
        for slot in [35760, 35761, 35762, 35763] {
            c.report(RecoveryEvidence::OrphanGossip { slot, gap: 5 });
        }
        let ctx = RecoveryContext {
            local_height: 34558,
            network_tip_height: 34563,
            peer_count: 20,
            last_applied_secs: 6,
            shallow_rollback_count: 1,
            snap_attempts: 0,
            // Post-first-rollback: rolled to 34557, applied 34558 → applied_since true.
            last_rollback_local_height: Some(34557),
            in_grace_period: false,
        };
        assert_eq!(
            c.classify(&ctx),
            RecoveryAction::HeaderFirstSync,
            "Coordinator must not rollback again when we've applied since last rollback"
        );
    }

    // --- Evidence window pruning ---------------------------------------------

    #[test]
    fn evidence_capacity_bounded() {
        let mut c = RecoveryCoordinator::new();
        // Push way more than MAX_ENTRIES.
        for _ in 0..(MAX_ENTRIES + 50) {
            c.report(RecoveryEvidence::OrphanGossip { slot: 1, gap: 1 });
        }
        assert!(c.evidence_len() <= MAX_ENTRIES);
    }
}
