//! AUDIT-P1-001 — the snap-attempt limiter's 30s cooldown is VACUOUS for exactly
//! the population it protects. TESTS-FIRST (red before fix).
//!
//! MEASURED DEFECT (security audit, verified in source):
//! `cleanup.rs:484-507` re-arms the snap-sync attempt limiter. When
//! `snap.attempts >= 3 && should_sync() && gap > SNAP_SYNC_GAP_MIN(500) &&
//! peers.len() >= 3` it consults a 30-second cooldown and then zeroes
//! `snap.attempts`. The cooldown reads:
//!
//! ```text
//! let since_last_snap = self.snap.last_snap_completed
//!     .map(|t| t.elapsed().as_secs())
//!     .unwrap_or(60);
//! if since_last_snap >= 30 { self.snap.attempts = 0; ... }
//! ```
//!
//! `last_snap_completed` has EXACTLY ONE write site — `snap_sync.rs:310`, inside
//! `take_snap_snapshot()` — i.e. it is stamped only on snap SUCCESS. A node that
//! has never completed a snap therefore holds `None`, `unwrap_or(60)` yields 60,
//! `60 >= 30` is unconditionally true, and the cooldown never gates anything: the
//! limiter is re-armed on EVERY cleanup tick. The "3 attempts" cap is not a cap at
//! all for never-succeeded nodes — precisely the bootstrap population it exists to
//! protect. Dormant before INC-I-152 (no snap door was open at h > 0); the
//! genesis-window admission makes it reachable.
//!
//! THE CONTRACT THESE TESTS PIN: the cooldown must measure time since the last
//! snap ACTIVITY (attempt or completion), not since the last snap SUCCESS. The
//! stated intent of the 30s spacing is "give discv5 time to discover more peers"
//! — an intent that is about retry pacing, not about success. A never-succeeded
//! node must be paced like any other; a genuinely long-idle node must still be
//! allowed to retry (T2), and the re-arm must not widen past its existing
//! conjuncts (T3a/T3b).
//!
//! OUTPUT CONTRACT (module-wide) — fn SyncManager::cleanup(&mut self)
//!   O1 parameters: none beyond `&mut self` (n/a). O3 returns `()` (n/a).
//!   O4 no persistent store, O5 no global/static state, O6 no channel/event
//!   emission (the only side-channel is `tracing`, not an observable).
//!   O2 receiver mutations — scoped to the UNIT UNDER TEST, the re-arm block at
//!   cleanup.rs:484-507, which writes EXACTLY two fields:
//!     O2.1 self.snap.attempts          — set to 0 iff the re-arm fires
//!     O2.2 self.snap.blacklisted_peers — cleared iff the re-arm fires
//!   Every other branch of `cleanup()` is held INERT by the fixture; see
//!   `mgr_limiter_exhausted` for the per-branch argument. The one branch that is
//!   deliberately allowed to run — the height-offset tracker at cleanup.rs:661,
//!   which is downstream of the re-arm — doubles as the anti-vacuity witness
//!   (`assert_cleanup_ran`), proving `cleanup()` executed past the block under
//!   test rather than returning early.
//!
//!   PATHS through the re-arm block:
//!     P1 attempts < 3                          → no-op (out of scope: the whole
//!        defect is about the exhausted state; every test holds attempts == 3)
//!     P2 attempts >= 3 && !should_sync()       → no-op (not exercised: covered by
//!        the existing sync suite; falsifying it would also void the fixture)
//!     P3 ... && gap <= SNAP_SYNC_GAP_MIN       → no-op                 → T3b
//!     P4 ... && gap > MIN && peers < 3         → no-op                 → T3a
//!     P5 ... && peers >= 3 && cooldown UNSPENT → no-op                 → T1
//!     P6 ... && cooldown SPENT                 → RE-ARM (O2.1, O2.2)   → T2
//!
//! INPUT PARTITIONS (the classes that change the cooldown math for a FIXED path;
//! each test below is exactly one cell of `last_snap_completed × peers × gap`):
//!   IP-A last_snap_completed == None       — never completed a snap. THE DEFECT
//!        POPULATION: fresh/wiped bootstrap nodes.                      → T1
//!   IP-B last_snap_completed == Some(t), elapsed >= 30s — a node whose last snap
//!        activity is genuinely old; retry-after-failure must NOT be starved. → T2, T3a, T3b
//!   IP-C peers >= 3 (snap quorum reachable)                            → T1, T2, T3b
//!   IP-D peers  < 3 (sub-quorum)                                       → T3a
//!   IP-E gap > SNAP_SYNC_GAP_MIN(500)                                  → T1, T2, T3a
//!   IP-F gap == SNAP_SYNC_GAP_MIN exactly — the comparator boundary; the block
//!        reads `>`, so the floor value itself must NOT re-arm.          → T3b
//!   IP-G snap.attempts == 3 (limiter exhausted) — held fixed by every test.
//!   MATRIX: {O2.1, O2.2} × {T1, T2, T3a, T3b} = 8 assertions, plus one
//!   anti-vacuity witness per test = 12.
//!
//! FAIL EVIDENCE (pre-fix, cleanup.rs/snap_sync.rs/types.rs unmodified):
//!   T1 MUST FAIL — the cooldown is vacuous, so attempts is reset to 0.
//!   T2/T3a/T3b are PASS-locks: green BEFORE and AFTER the fix. T2 in particular
//!   is the guard against "fixing" the vacuity by disabling the retry path
//!   altogether; T3a/T3b guard against the fix widening the re-arm.
//!
//! NOTE: T1 needs NO new API. It is a pure behavioral red — it compiles and fails
//! against unmodified source, and it will keep compiling after the fix adds
//! `last_snap_attempt`, because nothing here names that field.
//!
//! ===========================================================================
//! POST-FIX COVERAGE (T5-T8) — the SHIPPED mechanism itself.
//!
//! That NOTE is also the coverage hole. T1-T3b were written red against pre-fix
//! source and name only `last_snap_completed`; the shipped fix paces the re-arm on
//! `SnapSyncState.last_snap_attempt`, and T1 now passes through the
//! `(None, None) => 0` arm. So NO test above ever reads, stamps, or is paced by
//! `last_snap_attempt` — the production mechanism has zero coverage, the same
//! coverage-gap class already caught once on this branch (a shipped amendment with
//! no test). T5-T8 close it.
//!
//! SECOND UNIT UNDER TEST — fn SyncManager::snap_fallback_to_normal(&mut self)
//! [snap_sync.rs:365], the ONLY site in the workspace that increments
//! `snap.attempts` (grep-verified over crates/ + bins/).
//!   O1 parameters: none beyond `&mut self` (n/a). O3 returns `()` (n/a).
//!   O4 no persistent store, O5 no global/static state, O6 no channel emission.
//!   O2 receiver mutations — scoped to the stamp pair at snap_sync.rs:372-373:
//!     O2.3 self.snap.attempts          — incremented by exactly 1
//!     O2.4 self.snap.last_snap_attempt — RE-stamped to now on the SAME call
//!   The call also runs `set_state(Idle)` and may run `start_sync()`; neither can
//!   confound the observation — `decision.rs` only READS `snap.attempts`, and
//!   snap_sync.rs:373 is the sole writer of `last_snap_attempt` anywhere.
//!
//! ADDITIONAL PATHS through the cooldown, which post-fix is a 4-arm match on
//! `(last_snap_attempt, last_snap_completed)`:
//!   P7  (Some recent, None)       → UNSPENT     → no re-arm            → T6
//!   P8  (Some old,    None)       → SPENT       → RE-ARM (O2.1, O2.2)  → T7
//!   P9  (Some recent, Some old)   → min UNSPENT → no re-arm            → T8 arm a
//!   P10 (Some old,    Some recent)→ min UNSPENT → no re-arm            → T8 arm b
//!   P11 (None, None)              → refuse (strict direction)          → T1
//!
//! ADDITIONAL INPUT PARTITIONS:
//!   IP-H last_snap_attempt == Some(t), elapsed < 30s — a node that JUST tried;
//!        the cooldown must bite even with no completion ever.  → T6, T8 both arms
//!   IP-I last_snap_attempt == Some(t), elapsed >= 30s — retry-after-failure must
//!        not be starved.                                       → T7, T8 arm b
//!   IP-J last_snap_attempt == None — never attempted.          → T1-T3b
//!   MATRIX: {O2.1, O2.2} × {T6, T7, T8a, T8b} = 8 assertions + one anti-vacuity
//!   witness each; T5 = {O2.3, O2.4} × 3 successive increments.
//!
//! MUTATION EVIDENCE (cooldown temporarily reverted in place to the old
//! `last_snap_completed.map(|t| t.elapsed().as_secs()).unwrap_or(60)`):
//!   T6 FAILS  — `None` ⇒ 60 ⇒ `60 >= 30` ⇒ the never-completed node is re-armed.
//!   T8 FAILS  — arm a: a 120s-stale SUCCESS alone unlocks a node that attempted
//!               5s ago. (Arm b is a pass-lock under both forms.)
//!   T5/T7 are pass-locks under both forms; T7 exists so the vacuity cannot be
//!   "fixed" by never re-arming at all.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crypto::Hash;
use libp2p::PeerId;

use crate::sync::manager::recovery::thresholds;
use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

/// Live mainnet tip at the INC-I-152 reproduction (run 487, 2026-08-06).
const MAINNET_TIP: u64 = 129_800;

/// Height orphan chase drove the freshly-wiped node to (INC-I-152, measured:
/// blocks 1..14 applied before genesis-era producer validation rejected 15).
/// A node at this height has *never* completed a snap.
const BOOTSTRAP_HEIGHT: u64 = 14;

/// A SyncManager parked in header-first with the snap limiter EXHAUSTED
/// (`attempts == 3`) and one peer already snap-blacklisted, driving `n` peers
/// agreeing at `peer_height`.
///
/// INERTNESS ARGUMENT — why the only `cleanup()` branch that can write
/// `snap.attempts` / `snap.blacklisted_peers` in this fixture is the re-arm block
/// under test (cleanup.rs:484-507):
///   * tip decay (:31) — `network_tip_height` is set by `add_peer` from the peer
///     claims, so it is never `> peer_max + 5`; the else-branch runs.
///   * body-deadlock (:94) / snap timeouts (:173) — require phase
///     `DownloadingBodies` / `SnapCollecting` / `SnapDownloading`; we are in
///     `DownloadingHeaders`.
///   * stale-peer removal (:157) — `add_peer` stamps `last_status_response = now`.
///   * stall recovery (:239) — requires `Synchronized`.
///   * stuck-sync escape (:274) — requires `last_sync_activity.elapsed() > 30s`;
///     `SyncManager::new` stamps it `now`.
///   * header-blacklist escape (:429) — `header_blacklisted_peers` is empty.
///   * periodic sync retry (:512) — requires `Idle`, and it calls `start_sync()`,
///     which can re-enter snap admission and mutate `snap.attempts` AFTER the
///     block under test. This is exactly why the fixture parks in
///     `Syncing{DownloadingHeaders}` rather than `Idle`: it is also the realistic
///     state for a node whose snap attempts are exhausted (INC-I-152 T6 pins that
///     `attempts == 3` falls back to header-first), and it makes the post-call
///     observation of `snap.attempts` unambiguous.
///   * stuck-sync escalation (:600) — requires `last_block_applied.elapsed() > 300s`.
///   * height-offset tracker (:661) — DOES run, writes only `fork.stable_gap_since`.
///     Downstream of the re-arm, so it is the anti-vacuity witness.
///
/// INPUT PARTITIONS: N/A — fixture helper.
fn mgr_limiter_exhausted(local_height: u64, peer_height: u64, n: usize) -> SyncManager {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = local_height;
    mgr.local_slot = local_height as u32;
    mgr.local_hash = crypto::hash::hash(format!("local_{}", local_height).as_bytes());

    let peer_hash = crypto::hash::hash(b"canonical_agreed_tip");
    let mut last_peer = PeerId::random();
    for _ in 0..n {
        last_peer = PeerId::random();
        mgr.add_peer(last_peer, peer_height, peer_hash, peer_height as u32);
    }

    mgr.fork.needs_genesis_resync = false;
    mgr.fork.stable_gap_since = None; // armed as the anti-vacuity witness

    // The limiter is exhausted and the node has fallen back to header-first.
    mgr.snap.attempts = 3;
    mgr.snap.blacklisted_peers = HashSet::from([BLACKLISTED_PEER.with(|p| *p)]);
    mgr.state = SyncState::Syncing {
        phase: SyncPhase::DownloadingHeaders,
        started_at: Instant::now(),
    };
    mgr.pipeline_data = SyncPipelineData::Headers {
        target_slot: peer_height as u32,
        peer: last_peer,
        headers_count: 0,
    };
    mgr
}

thread_local! {
    /// A stable PeerId used as the pre-seeded snap blacklist entry, so O2.2 can be
    /// asserted by identity rather than by set length alone.
    static BLACKLISTED_PEER: PeerId = PeerId::random();
}

/// Anti-vacuity witness: `cleanup()` sets `fork.stable_gap_since` at cleanup.rs:690,
/// strictly DOWNSTREAM of the re-arm block. Asserting it rules out a false pass via
/// an early return — without it, "attempts is still 3" could mean "cleanup never
/// reached the block at all".
fn assert_cleanup_ran(mgr: &SyncManager, test: &str) {
    assert!(
        mgr.fork.stable_gap_since.is_some(),
        "{}: cleanup() must have run PAST the snap re-arm block (cleanup.rs:484-507) \
         to the height-offset tracker at :661 that stamps fork.stable_gap_since. A \
         None here means the function returned early and every assertion below is \
         vacuous.",
        test
    );
}

// ===========================================================================
// T1 — CORE REPRODUCTION. MUST FAIL pre-fix.
// ===========================================================================

/// T1 (IP-A × IP-C × IP-E): a node that has NEVER completed a snap
/// (`last_snap_completed == None`) with the limiter exhausted, 3 peers and a
/// bootstrap-scale gap. The 30s cooldown must actually gate the re-arm — the
/// limiter must NOT be silently reset on this tick.
///
/// Pre-fix, `unwrap_or(60)` turns the missing timestamp into "60 seconds since the
/// last snap", so `60 >= 30` holds on EVERY cleanup tick and the 3-attempt cap is
/// re-armed forever. This is the AUDIT-P1-001 defect verbatim: the only population
/// with `last_snap_completed == None` is the bootstrap population the cap protects.
///
/// OUTPUT CONTRACT: fn cleanup(&mut self) [cleanup.rs:13], block at :484-507
///   O2.1 self.snap.attempts          — UNCHANGED at 3 (no re-arm)
///   O2.2 self.snap.blacklisted_peers — UNCHANGED (only the re-arm clears it)
/// PATH P5 (all conjuncts true, cooldown UNSPENT → the branch must not be taken).
/// INPUT PARTITIONS: IP-A (None) — the ONLY difference from T2, which is exactly
///   the defect; IP-C (3 peers), IP-E (gap ≫ 500), IP-G (attempts == 3).
/// MATRIX: P5 → O2.1 ✓ O2.2 ✓ + witness ✓.
/// VERDICT: **FAILS pre-fix** — the cooldown is vacuous for `None`, so
/// `snap.attempts` is observed as 0.
#[test]
fn inc_i_152_p1_001_never_completed_snap_does_not_rearm_limiter() {
    let mut mgr = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, MAINNET_TIP, 3);
    // The defect population: no snap has EVER completed on this node.
    mgr.snap.last_snap_completed = None;

    // Preconditions — every other conjunct of the re-arm is satisfied, so the
    // cooldown is the sole gate this test probes.
    assert_eq!(mgr.snap.attempts, 3, "precondition: limiter exhausted");
    assert!(mgr.peers.len() >= 3, "precondition: snap quorum reachable");
    const {
        assert!(
            MAINNET_TIP - BOOTSTRAP_HEIGHT > thresholds::SNAP_SYNC_GAP_MIN,
            "precondition: gap must be bootstrap-scale (> SNAP_SYNC_GAP_MIN)"
        )
    };

    mgr.cleanup();

    assert_cleanup_ran(&mgr, "AUDIT-P1-001 T1");
    // O2.1 — the re-arm must NOT have fired.
    assert_eq!(
        mgr.snap.attempts, 3,
        "AUDIT-P1-001 T1: a node that has NEVER completed a snap \
         (last_snap_completed == None) had its 3-attempt limiter re-armed to {} on a \
         single cleanup tick. The 30s cooldown reads \
         `last_snap_completed.map(elapsed).unwrap_or(60)`, and last_snap_completed is \
         written ONLY on snap SUCCESS (snap_sync.rs:310) — so for never-succeeded \
         nodes `60 >= 30` is unconditionally true and the cooldown gates NOTHING. \
         The cap is therefore no cap at all for exactly the bootstrap population it \
         was written to protect. Pace the retry on the last snap ATTEMPT, not on the \
         last snap SUCCESS.",
        mgr.snap.attempts
    );
    // O2.2 — the blacklist is cleared only by the re-arm; an empty set is a second,
    // independent witness that the branch fired.
    assert!(
        mgr.snap
            .blacklisted_peers
            .contains(&BLACKLISTED_PEER.with(|p| *p)),
        "AUDIT-P1-001 T1: O2.2 — snap.blacklisted_peers was cleared, which only the \
         re-arm branch does. Peers that served bad snapshots are forgiven on every \
         tick along with the attempt counter."
    );
}

// ===========================================================================
// T2 — PASS-LOCK: legitimate retry-after-failure must NOT be starved.
// ===========================================================================

/// T2 (IP-B × IP-C × IP-E): a node whose last snap activity is genuinely long past
/// (120s ago, well beyond the 30s cooldown), otherwise identical to T1. The re-arm
/// MUST fire.
///
/// This is the guard against "fixing" the vacuity by neutering the retry path.
/// The cooldown exists to space retry cycles so discv5 can discover more peers —
/// not to forbid retries. A fix that makes T1 pass by never re-arming would strand
/// every node that legitimately exhausted 3 attempts.
///
/// OUTPUT CONTRACT: fn cleanup(&mut self) [cleanup.rs:13], block at :484-507
///   O2.1 self.snap.attempts          — reset to 0
///   O2.2 self.snap.blacklisted_peers — cleared
/// PATH P6 (cooldown SPENT → re-arm).
/// INPUT PARTITIONS: IP-B (Some, elapsed 120s ≥ 30s) with every T1 partition held
///   fixed — isolates the cooldown term itself. Post-fix this cell is unchanged:
///   with no attempt ever stamped, the most recent snap activity is still the
///   120s-old completion.
/// MATRIX: P6 → O2.1 ✓ O2.2 ✓ + witness ✓.
/// VERDICT: PASS-lock (pre-fix and post-fix).
#[test]
fn inc_i_152_p1_001_long_idle_after_completed_snap_rearms_limiter() {
    let mut mgr = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, MAINNET_TIP, 3);
    // Genuinely long-idle: a real snap completed, 120s ago — 4× the cooldown.
    mgr.snap.last_snap_completed = Some(Instant::now() - Duration::from_secs(120));

    mgr.cleanup();

    assert_cleanup_ran(&mgr, "AUDIT-P1-001 T2");
    // O2.1 — retry-after-failure must still be reachable.
    assert_eq!(
        mgr.snap.attempts,
        0,
        "AUDIT-P1-001 T2: a node whose last snap activity was 120s ago (≥ the 30s \
         cooldown) with {} peers and a gap of {} MUST have its attempt limiter \
         re-armed. Observed attempts={}. Fixing the vacuous cooldown must not \
         disable the retry path — the cooldown SPACES retries so discv5 can find \
         more peers, it does not forbid them.",
        mgr.peers.len(),
        MAINNET_TIP - BOOTSTRAP_HEIGHT,
        mgr.snap.attempts
    );
    // O2.2 — the paired mutation: a fresh retry cycle also forgives the blacklist.
    assert!(
        mgr.snap.blacklisted_peers.is_empty(),
        "AUDIT-P1-001 T2: O2.2 — the re-arm must ALSO clear snap.blacklisted_peers; \
         a retry cycle that keeps the old blacklist cannot rebuild a quorum. \
         Observed {} entries.",
        mgr.snap.blacklisted_peers.len()
    );
}

// ===========================================================================
// T3a/T3b — PASS-LOCKS: the other conjuncts still gate the re-arm.
// ===========================================================================

/// T3a (IP-B × IP-D × IP-E): the cooldown is satisfied and the gap is
/// bootstrap-scale, but only 2 peers are connected — below the snap quorum of 3.
/// The re-arm must NOT fire.
///
/// The cooldown is deliberately SPENT here so the peer-count conjunct is the sole
/// gate, pre-fix AND post-fix. `min_peers_for_sync` defaults to 1, so
/// `should_sync()` is still true at 2 peers — the block is genuinely reached and
/// genuinely refuses.
///
/// OUTPUT CONTRACT: fn cleanup(&mut self) [cleanup.rs:13], block at :484-507
///   O2.1 self.snap.attempts          — UNCHANGED at 3
///   O2.2 self.snap.blacklisted_peers — UNCHANGED
/// PATH P4 (peers < 3).
/// INPUT PARTITIONS: IP-D (2 peers) with T2's partitions otherwise identical —
///   isolates the quorum conjunct.
/// MATRIX: P4 → O2.1 ✓ O2.2 ✓ + witness ✓.
/// VERDICT: PASS-lock (pre-fix and post-fix). Pins that the fix does not WIDEN
/// the re-arm while making the cooldown real.
#[test]
fn inc_i_152_p1_001_sub_quorum_peers_does_not_rearm_limiter() {
    let mut mgr = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, MAINNET_TIP, 2);
    mgr.snap.last_snap_completed = Some(Instant::now() - Duration::from_secs(120));

    assert_eq!(mgr.peers.len(), 2, "precondition: sub-quorum peer count");

    mgr.cleanup();

    assert_cleanup_ran(&mgr, "AUDIT-P1-001 T3a");
    assert_eq!(
        mgr.snap.attempts,
        3,
        "AUDIT-P1-001 T3a: with only {} peers (snap quorum is 3) the limiter must \
         stay exhausted even though the cooldown is spent — re-arming without a \
         reachable quorum burns attempts on a snap that cannot succeed. Observed \
         attempts={}.",
        mgr.peers.len(),
        mgr.snap.attempts
    );
    assert!(
        mgr.snap
            .blacklisted_peers
            .contains(&BLACKLISTED_PEER.with(|p| *p)),
        "AUDIT-P1-001 T3a: O2.2 — snap.blacklisted_peers must not be cleared when \
         the re-arm is refused"
    );
}

/// T3b (IP-B × IP-C × IP-F): the cooldown is satisfied and the quorum is present,
/// but the gap is EXACTLY `SNAP_SYNC_GAP_MIN` — the block reads `gap > MIN`, so the
/// floor value itself must NOT re-arm.
///
/// This pins the comparator, matching the parity contract INC-I-152 T8/T9 already
/// pin on the admission side: one comparator across the whole snap cluster. A drift
/// to `>=` here would re-arm the limiter for catch-up-scale gaps that header-first
/// resolves faster than snap.
///
/// OUTPUT CONTRACT: fn cleanup(&mut self) [cleanup.rs:13], block at :484-507
///   O2.1 self.snap.attempts          — UNCHANGED at 3
///   O2.2 self.snap.blacklisted_peers — UNCHANGED
/// PATH P3 (gap == floor, so `gap > MIN` is false).
/// INPUT PARTITIONS: IP-F (gap == 500) with T2's partitions otherwise identical —
///   isolates the gap conjunct at its exact boundary.
/// MATRIX: P3 → O2.1 ✓ O2.2 ✓ + witness ✓.
/// VERDICT: PASS-lock (pre-fix and post-fix).
#[test]
fn inc_i_152_p1_001_gap_at_floor_does_not_rearm_limiter() {
    let peer_height = BOOTSTRAP_HEIGHT + thresholds::SNAP_SYNC_GAP_MIN;
    let mut mgr = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, peer_height, 3);
    mgr.snap.last_snap_completed = Some(Instant::now() - Duration::from_secs(120));

    assert_eq!(
        peer_height - BOOTSTRAP_HEIGHT,
        thresholds::SNAP_SYNC_GAP_MIN,
        "precondition: the gap must sit EXACTLY on the floor"
    );

    mgr.cleanup();

    assert_cleanup_ran(&mgr, "AUDIT-P1-001 T3b");
    assert_eq!(
        mgr.snap.attempts,
        3,
        "AUDIT-P1-001 T3b (comparator parity): gap == SNAP_SYNC_GAP_MIN ({}) exactly \
         must NOT re-arm the limiter — the block reads `gap > SNAP_SYNC_GAP_MIN`, the \
         same comparator the INC-I-152 admission cluster reads. Observed attempts={}.",
        thresholds::SNAP_SYNC_GAP_MIN,
        mgr.snap.attempts
    );
    assert!(
        mgr.snap
            .blacklisted_peers
            .contains(&BLACKLISTED_PEER.with(|p| *p)),
        "AUDIT-P1-001 T3b: O2.2 — snap.blacklisted_peers must not be cleared when \
         the re-arm is refused"
    );
}

// ===========================================================================
// T5 — THE DESYNCHRONISATION LOCK: the stamp actually happens.
// ===========================================================================

/// T5: drive the ONLY production site that increments `snap.attempts` —
/// `snap_fallback_to_normal()` (snap_sync.rs:365) — and pin that the counter and
/// its timestamp move as ONE, on EVERY increment.
///
/// The whole fix rests on an unwritten invariant: `attempts >= 3` implies a
/// `last_snap_attempt` no older than the 3rd failure. If a future edit splits the
/// pair — an early return between the two lines, a second increment site, a
/// `get_or_insert` instead of an assignment — the exhausted limiter drops into the
/// `(None, None)` arm and the pacing silently reverts to the AUDIT-P1-001 defect
/// shape with every cooldown test still green. Nothing else in the suite would
/// notice. This test is the only thing standing between that edit and production.
///
/// The stamp is deliberately AGED to 600s before calls 2 and 3, so "fresh
/// afterwards" can only mean the call RE-stamped it — a first-write-only stamp
/// (the exact shape that would strand an exhausted node) is caught, not just a
/// never-write.
///
/// OUTPUT CONTRACT: fn snap_fallback_to_normal(&mut self) [snap_sync.rs:365]
///   O2.3 self.snap.attempts          — +1 per call (0→1→2→3)
///   O2.4 self.snap.last_snap_attempt — None → Some(now), re-stamped every call
/// PATH: the single unconditional path through the stamp pair at :372-373.
/// INPUT PARTITIONS: IP-J → IP-H (the transition itself); attempts crosses the
///   limiter threshold so the post-state is exactly the state T6 asserts on.
/// MATRIX: {O2.3, O2.4} × 3 increments = 6 assertions.
/// VERDICT: PASS-lock. Fails against any edit that desynchronises the pair.
#[test]
fn inc_i_152_p1_001_attempt_increment_stamps_last_snap_attempt() {
    let mut mgr = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, MAINNET_TIP, 3);
    // Rewind the limiter to virgin — this test OBSERVES the increments through the
    // production path, it must not inherit them from the fixture.
    mgr.snap.attempts = 0;
    mgr.snap.last_snap_attempt = None;
    mgr.snap.last_snap_completed = None;

    assert!(
        mgr.snap.last_snap_attempt.is_none(),
        "precondition: no snap attempt has ever been recorded"
    );

    for expected in 1..=3u8 {
        if expected > 1 {
            // Age the stamp so a stale value cannot masquerade as a fresh one.
            mgr.snap.last_snap_attempt = Some(Instant::now() - Duration::from_secs(600));
        }

        mgr.snap_fallback_to_normal();

        // O2.3 — the counter advanced by exactly one, through the real path.
        assert_eq!(
            mgr.snap.attempts, expected,
            "AUDIT-P1-001 T5: snap_fallback_to_normal() must increment snap.attempts \
             by exactly 1; expected {} after {} call(s), observed {}.",
            expected, expected, mgr.snap.attempts
        );

        // O2.4 — and stamped the attempt time on the SAME call.
        let stamp = match mgr.snap.last_snap_attempt {
            Some(t) => t,
            None => panic!(
                "AUDIT-P1-001 T5: snap.attempts reached {} but last_snap_attempt is \
                 still None. cleanup() paces the limiter re-arm on this field \
                 (cleanup.rs:502-513); a counter that can advance without it being \
                 stamped drops the exhausted node into the (None, None) arm, and the \
                 whole AUDIT-P1-001 fix becomes dead code.",
                mgr.snap.attempts
            ),
        };
        assert!(
            stamp.elapsed() < Duration::from_secs(30),
            "AUDIT-P1-001 T5: the stamp must be taken AT the increment — after call \
             {} it is already {}s old, i.e. it was not refreshed. A stamp older than \
             the 30s cooldown lets a node that just burned an attempt re-arm on the \
             very next cleanup tick, which is the defect this fix removed.",
            expected,
            stamp.elapsed().as_secs()
        );
    }
}

// ===========================================================================
// T6/T7 — the attempt stamp PACES the re-arm for a never-succeeded node.
// ===========================================================================

/// T6 (IP-H × IP-C × IP-E): the limiter is exhausted, the node has NEVER completed
/// a snap (`last_snap_completed == None`) and it attempted 5 seconds ago. The
/// re-arm must NOT fire — the cooldown is finally real for the bootstrap
/// population.
///
/// This is the test that catches the ORIGINAL defect through the PRODUCTION
/// mechanism rather than through T1's `(None, None)` fallback arm. It is the exact
/// live state after T5's third increment: a freshly-wiped node that snap-failed
/// three times in a row within a few seconds. Pre-fix, `unwrap_or(60)` re-armed it
/// on every tick, so the "3 attempts" cap never existed for it at all.
///
/// OUTPUT CONTRACT: fn cleanup(&mut self) [cleanup.rs:13], block at :487-523
///   O2.1 self.snap.attempts          — UNCHANGED at 3 (no re-arm)
///   O2.2 self.snap.blacklisted_peers — UNCHANGED
/// PATH P7 (attempt recent, no completion → cooldown UNSPENT).
/// INPUT PARTITIONS: IP-H (attempt 5s ago) — the ONLY difference from T7; IP-A
///   (never completed), IP-C (3 peers), IP-E (gap ≫ 500), IP-G (attempts == 3).
/// MATRIX: P7 → O2.1 ✓ O2.2 ✓ + witness ✓.
/// VERDICT: **FAILS against the pre-fix cooldown** (`unwrap_or(60)` ⇒ re-arm).
#[test]
fn inc_i_152_p1_001_recent_attempt_paces_never_completed_node() {
    let mut mgr = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, MAINNET_TIP, 3);
    // Never succeeded, but tried 5s ago — well inside the 30s cooldown.
    mgr.snap.last_snap_completed = None;
    mgr.snap.last_snap_attempt = Some(Instant::now() - Duration::from_secs(5));

    assert_eq!(mgr.snap.attempts, 3, "precondition: limiter exhausted");
    assert!(mgr.peers.len() >= 3, "precondition: snap quorum reachable");

    mgr.cleanup();

    assert_cleanup_ran(&mgr, "AUDIT-P1-001 T6");
    // O2.1 — the cooldown must gate on the ATTEMPT, so the re-arm is refused.
    assert_eq!(
        mgr.snap.attempts, 3,
        "AUDIT-P1-001 T6: a node that has never completed a snap and attempted one \
         5s ago had its 3-attempt limiter re-armed to {} on this tick. The 30s \
         cooldown must measure time since the last snap ACTIVITY \
         (cleanup.rs:502-513 reads last_snap_attempt); if it reads only \
         last_snap_completed it degenerates to `unwrap_or(60) >= 30` — always true \
         — and the cap does not exist for the bootstrap population it protects.",
        mgr.snap.attempts
    );
    // O2.2 — the paired mutation is likewise withheld.
    assert!(
        mgr.snap
            .blacklisted_peers
            .contains(&BLACKLISTED_PEER.with(|p| *p)),
        "AUDIT-P1-001 T6: O2.2 — snap.blacklisted_peers was cleared, which only the \
         re-arm branch does. Peers that served bad snapshots are forgiven 5s after \
         the attempt that proved them bad."
    );
}

/// T7 (IP-I × IP-C × IP-E): identical to T6 except the attempt was 120s ago — 4×
/// the cooldown. The re-arm MUST fire.
///
/// The mirror pass-lock of T6: making the cooldown real must not starve legitimate
/// retry-after-failure. A node whose only snap activity is a 2-minute-old failed
/// attempt is exactly who the retry path exists for; if T6's strictness leaked into
/// this cell, every node that exhausted 3 attempts would be permanently stranded on
/// header-first sync — a worse outage than the defect.
///
/// OUTPUT CONTRACT: fn cleanup(&mut self) [cleanup.rs:13], block at :487-523
///   O2.1 self.snap.attempts          — reset to 0
///   O2.2 self.snap.blacklisted_peers — cleared
/// PATH P8 (attempt old, no completion → cooldown SPENT → re-arm).
/// INPUT PARTITIONS: IP-I (attempt 120s ago), every other partition held equal to
///   T6 — isolates the elapsed term of the attempt stamp alone.
/// MATRIX: P8 → O2.1 ✓ O2.2 ✓ + witness ✓.
/// VERDICT: PASS-lock under both the pre-fix and post-fix cooldown.
#[test]
fn inc_i_152_p1_001_old_attempt_permits_rearm() {
    let mut mgr = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, MAINNET_TIP, 3);
    mgr.snap.last_snap_completed = None;
    mgr.snap.last_snap_attempt = Some(Instant::now() - Duration::from_secs(120));

    mgr.cleanup();

    assert_cleanup_ran(&mgr, "AUDIT-P1-001 T7");
    assert_eq!(
        mgr.snap.attempts,
        0,
        "AUDIT-P1-001 T7: a node whose last snap ATTEMPT was 120s ago (≥ the 30s \
         cooldown), with {} peers and a gap of {}, MUST have its limiter re-armed. \
         Observed attempts={}. Pacing the cooldown on the attempt must not forbid \
         retries — a never-succeeded node that is genuinely long-idle would be \
         stranded on header-first sync forever.",
        mgr.peers.len(),
        MAINNET_TIP - BOOTSTRAP_HEIGHT,
        mgr.snap.attempts
    );
    assert!(
        mgr.snap.blacklisted_peers.is_empty(),
        "AUDIT-P1-001 T7: O2.2 — the re-arm must ALSO clear snap.blacklisted_peers; \
         a retry cycle that keeps the old blacklist cannot rebuild a quorum. \
         Observed {} entries.",
        mgr.snap.blacklisted_peers.len()
    );
}

// ===========================================================================
// T8 — with BOTH timestamps present, the MORE RECENT one wins.
// ===========================================================================

/// T8 (IP-H × IP-I, both arms): when both timestamps are set, the cooldown must
/// read the most recent snap ACTIVITY — i.e. the SMALLEST elapsed of the two.
/// Neither a stale success nor a stale attempt may unlock a node that just moved.
///
/// Arm a (P9) — completion 120s old, attempt 5s ago: a node that snapped
/// successfully two minutes ago and has been failing ever since. The stale success
/// must NOT unlock it. This is the cell where "read completion only" and "read the
/// most recent" disagree, so it is the mutation-kill arm.
/// Arm b (P10) — completion 5s ago, attempt 120s old: a node that just finished a
/// snap. Also no re-arm; a `max`-instead-of-`min` slip would fire here.
/// Together the two arms bracket the combinator: only `min(elapsed)` passes both.
///
/// OUTPUT CONTRACT: fn cleanup(&mut self) [cleanup.rs:13], block at :487-523
///   O2.1 self.snap.attempts          — UNCHANGED at 3 in BOTH arms
///   O2.2 self.snap.blacklisted_peers — UNCHANGED in BOTH arms
/// PATHS P9, P10 (the `(Some, Some)` arm of the cooldown match).
/// INPUT PARTITIONS: IP-H × IP-I in both orderings, with T6's peer/gap/attempts
///   partitions held fixed — isolates the two-timestamp combinator itself.
/// MATRIX: {P9, P10} × {O2.1, O2.2} = 4 assertions + one witness per arm.
/// VERDICT: **arm a FAILS against the pre-fix cooldown** (it reads the 120s-old
///   completion, 120 >= 30, and re-arms); arm b is a pass-lock under both.
#[test]
fn inc_i_152_p1_001_most_recent_snap_activity_wins() {
    // --- Arm a (P9): stale SUCCESS must not unlock a node that just attempted.
    let mut mgr_a = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, MAINNET_TIP, 3);
    mgr_a.snap.last_snap_completed = Some(Instant::now() - Duration::from_secs(120));
    mgr_a.snap.last_snap_attempt = Some(Instant::now() - Duration::from_secs(5));

    mgr_a.cleanup();

    assert_cleanup_ran(&mgr_a, "AUDIT-P1-001 T8a");
    assert_eq!(
        mgr_a.snap.attempts, 3,
        "AUDIT-P1-001 T8a: last_snap_completed 120s ago but last_snap_attempt 5s ago \
         — the node is actively retrying RIGHT NOW and the limiter was re-armed to {} \
         anyway. The cooldown must take the MOST RECENT activity (the smallest \
         elapsed of the two), never just the completion: a success from two minutes \
         ago is not evidence that discv5 has had time to find new peers since the \
         attempt that failed 5 seconds ago.",
        mgr_a.snap.attempts
    );
    assert!(
        mgr_a
            .snap
            .blacklisted_peers
            .contains(&BLACKLISTED_PEER.with(|p| *p)),
        "AUDIT-P1-001 T8a: O2.2 — snap.blacklisted_peers must not be cleared when the \
         re-arm is refused"
    );

    // --- Arm b (P10): the mirror — a stale ATTEMPT must not unlock a node that
    // just completed a snap. Catches a `max`-for-`min` slip in the combinator.
    let mut mgr_b = mgr_limiter_exhausted(BOOTSTRAP_HEIGHT, MAINNET_TIP, 3);
    mgr_b.snap.last_snap_completed = Some(Instant::now() - Duration::from_secs(5));
    mgr_b.snap.last_snap_attempt = Some(Instant::now() - Duration::from_secs(120));

    mgr_b.cleanup();

    assert_cleanup_ran(&mgr_b, "AUDIT-P1-001 T8b");
    assert_eq!(
        mgr_b.snap.attempts, 3,
        "AUDIT-P1-001 T8b: last_snap_attempt 120s ago but last_snap_completed 5s ago \
         — snap activity 5 seconds old, and the limiter was re-armed to {}. The \
         combinator must be min(elapsed) over both stamps; taking the max (or the \
         attempt alone) re-arms a node that has just this second finished snapping.",
        mgr_b.snap.attempts
    );
    assert!(
        mgr_b
            .snap
            .blacklisted_peers
            .contains(&BLACKLISTED_PEER.with(|p| *p)),
        "AUDIT-P1-001 T8b: O2.2 — snap.blacklisted_peers must not be cleared when the \
         re-arm is refused"
    );
}
