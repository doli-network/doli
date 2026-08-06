//! INC-I-152 — bootstrap snap admission inside the genesis window. TESTS-FIRST.
//!
//! MEASURED ROOT CAUSE (run 487, mainnet vm-server, wipe→synced 4m43s ×3):
//! a freshly-wiped node parks correctly in the fresh-node snap hold, but within
//! ~10s the first gossip tip block triggers Orphan Chase (v6.16.1 pillar), which
//! fetches and applies genesis blocks 1..14. `local_height` 0→14 destroys the
//! strict `local_height == 0` condition that bootstrap snap requires in
//! `sync_engine/decision.rs::start_sync()`. The node then commits header-first and
//! serially walks all ~129,822 headers at 500/batch × 1 batch/sec ≈ 260s (92% of
//! the wall clock). Snap runs in <10s once the walk ends.
//!
//! THE CONTRACT THESE TESTS PIN: snap admission must key on "bootstrap-shaped",
//! not on the fencepost `local_height == 0`. A node at `0 < h <= genesis_blocks`
//! with a corroborated gap > SNAP_SYNC_GAP_MIN(500) is still bootstrap-shaped.
//!
//! INV-SYNC-011 CONSTRAINT (must survive): the SnapCollecting transition is
//! reachable ONLY via (a) h==0 bootstrap, (b) needs_genesis_resync, and now
//! (c) the genesis window WITH a > SNAP_SYNC_GAP_MIN gap. "No bare-gap term
//! admits snap on any path" — the window is a bootstrap-shape predicate, not a
//! gap relaxation. T3/T4/T6/T7 exist to keep the new term from degenerating.
//!
//! COMPARATOR PARITY (review F5): the window term and BOTH bootstrap holds read
//! the SAME comparator, `gap > SNAP_SYNC_GAP_MIN`. T8/T9 pin the exact boundary
//! so the cluster cannot drift back into a one-value `>=` / `>` asymmetry (at
//! gap==500 that asymmetry admitted snap at >=3 peers while refusing to park for
//! it at <3 peers).
//!
//! OUTPUT CONTRACT (module-wide) — fn SyncManager::start_sync(&mut self)
//!   No parameters (O1 n/a), returns `()` (O3 n/a), no persistent store (O4 n/a),
//!   no global/static state (O5 n/a), no channel/event emission (O6 n/a — the
//!   only side-channel is `tracing` logging, not an observable).
//!   O2 receiver mutations, exhaustive (decision.rs:117-307):
//!     O2.1 self.state           — Idle (parked / no peer) | Syncing{phase}
//!     O2.2 self.pipeline_data   — None | Headers{..} | SnapCollecting{..}
//!     O2.3 self.pipeline.sync_epoch      — +1 iff best_peer() is Some
//!     O2.4 self.pipeline.pending_requests/body_downloader/body_stall_retries
//!                               — cleared iff best_peer() is Some
//!     O2.5 self.pipeline.header_downloader/pending_headers/pending_blocks/
//!          headers_needing_bodies — cleared unless preserve_valid_sync_data()
//!     O2.6 self.peers[*].pending_request — None iff best_peer() is Some
//!     O2.7 self.snap.fresh_node_wait_start — get_or_insert(now) on the fresh-node
//!                               hold; None after the 60s fallback
//!     O2.8 self.snap.discv5_peer_grace_deadline — None after grace expiry
//!     O2.9 self.network.last_sync_activity — reset iff a phase was committed
//!   PATHS through start_sync:
//!     P0 already syncing            → immediate return, no mutation
//!     P1 no eligible peer           → no mutation past the guard
//!     P2 fresh-node hold (<60s)     → early return, O2.7 armed, no commit
//!     P3 discv5 grace (pre-deadline)→ early return, no commit
//!     P4 should_snap && majority    → SnapCollecting
//!     P5 should_snap && no majority → Headers (fragmentation fallback)
//!     P6 !should_snap               → Headers
//!
//! INPUT PARTITIONS (the classes that change the admission math for a FIXED path;
//! every test below is exactly one cell of `local_height × gap × peers × attempts`):
//!   IP-A local_height == 0                    — bootstrap fencepost (today's only
//!        admission); gap/window terms irrelevant.                        → T2
//!   IP-B 0 < local_height < genesis_blocks    — strictly inside the window; identical
//!        code path as IP-C but the window predicate flips.               → T1, T4, T5, T6,
//!                                                                          T8, T9, T10
//!   IP-C local_height > genesis_blocks        — outside the window; a steady-state
//!        node, must stay header-first.                                   → T3
//!   IP-D genesis_blocks == 0 (window disabled)— IP-B height, but the plumbing is
//!        absent → must behave exactly like today.                        → T7
//!   IP-E gap > SNAP_SYNC_GAP_MIN(500)         — bootstrap-scale distance. → T1, T2, T3, T5,
//!                                                                          T6, T7, T9, T10
//!   IP-F 0 < gap < SNAP_SYNC_GAP_MIN          — catch-up distance; header-first is
//!        correct even inside the window (no bare-gap/bare-window admission). → T4
//!   IP-G peers >= 3 (snap quorum reachable)   — admission may commit.     → T1, T2, T3, T4,
//!                                                                          T6, T7, T8, T9, T11
//!   IP-H peers < 3 (sub-quorum)               — must PARK, never commit header-first,
//!        because the holds are the thing that buys snap peers.           → T5, T10
//!   IP-I snap.attempts < 3 vs == 3            — the rate limiter; the new window
//!        term must not buy a bypass.                                     → T6 (==3), rest (0)
//!   IP-J gap == SNAP_SYNC_GAP_MIN exactly     — the comparator boundary; the whole
//!        admission cluster reads `>`, so the floor value itself is REFUSED. → T8 (500),
//!                                                                          T9 (501)
//!   IP-K local_height == genesis_blocks exactly — the inclusive `<=` fencepost of the
//!        window, mirroring Network::is_in_genesis().                     → T11
//!   MATRIX: asserted cells = {O2.1, O2.2, O2.3} × {T1..T4, T6..T9, T11} = 27,
//!   plus O2.7/O2.9 on T5 and O2.7 on T10 (the only tests whose paths reach the
//!   holds) = 27 + 3(T5: O2.1/O2.2/O2.3) + 2 + 3(T10) + 1 = 36 assertions.
//!
//! FAIL EVIDENCE (pre-fix, decision.rs unmodified): T1, T5 and T10 MUST FAIL.
//! T2/T3/T4/T6/T7/T8 are PASS-locks — green before AND after the fix.
//! T9/T11 are post-fix locks (they assert the new admission, so they fail pre-fix
//! for the same reason as T1; they exist to pin the two boundaries, not the defect).

use std::time::{Duration, Instant};

use crypto::Hash;
use libp2p::PeerId;

use crate::sync::manager::recovery::thresholds;
use crate::sync::manager::{SyncConfig, SyncManager, SyncPhase, SyncPipelineData, SyncState};

/// Mainnet `NetworkParams::genesis_blocks` (crates/core/.../defaults.rs:46).
const MAINNET_GENESIS_BLOCKS: u64 = 360;

/// Live mainnet tip at the measured reproduction (run 487, 2026-08-06).
const MAINNET_TIP: u64 = 129_800;

/// Height orphan chase drove the freshly-wiped node to before block 15 was
/// rejected by genesis-era producer validation (measured: blocks 1..14 applied).
const ORPHAN_CHASE_HEIGHT: u64 = 14;

/// A SyncManager with `n` peers all at `peer_height` sharing one `best_hash`,
/// built on a SyncConfig carrying `genesis_blocks`.
///
/// Local variant of `tests_inc_i139::mgr_with_agreeing_peers`, which hardcodes
/// `SyncConfig::default()` and therefore cannot express "inside the genesis
/// window". Identical (height, hash) across >= 2 peers is required so
/// `consensus_target_hash()` returns Some — otherwise `start_sync` falls back to
/// header-first even when `should_snap` is true, masking the admission probed here.
///
/// Returns the manager on a clean `Idle` + `SyncPipelineData::None` precondition:
/// `add_peer` drives `start_sync` internally during construction, so the state it
/// leaves behind must be reset before the test's own `start_sync()` call.
///
/// INPUT PARTITIONS: N/A — fixture helper.
fn mgr(local_height: u64, peer_height: u64, n: usize, genesis_blocks: u64) -> SyncManager {
    let config = SyncConfig {
        genesis_blocks,
        ..SyncConfig::default()
    };
    let mut mgr = SyncManager::new(config, Hash::ZERO);
    mgr.local_height = local_height;
    mgr.local_slot = local_height as u32;
    mgr.local_hash = crypto::hash::hash(format!("local_{}", local_height).as_bytes());
    let peer_hash = crypto::hash::hash(b"canonical_agreed_tip");
    for _ in 0..n {
        mgr.add_peer(PeerId::random(), peer_height, peer_hash, peer_height as u32);
    }
    mgr.fork.needs_genesis_resync = false;
    mgr.snap.threshold = 50;
    mgr.snap.attempts = 0;
    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;
    mgr
}

fn is_snap(mgr: &SyncManager) -> bool {
    matches!(mgr.pipeline_data, SyncPipelineData::SnapCollecting { .. })
}

fn is_headers(mgr: &SyncManager) -> bool {
    matches!(mgr.pipeline_data, SyncPipelineData::Headers { .. })
}

/// O2.3 witness: `start_sync` bumps `sync_epoch` immediately after `best_peer()`
/// resolves, BEFORE any hold or admission branch. Asserting it rules out a vacuous
/// pass via the P0 (already-syncing) / P1 (no eligible peer) early returns — without
/// it, "state is still Idle" could mean "the function never ran".
fn assert_ran(mgr: &SyncManager, epoch_before: u64, test: &str) {
    assert_eq!(
        mgr.pipeline.sync_epoch,
        epoch_before + 1,
        "{}: O2.3 — start_sync must have run past its P0/P1 guards (sync_epoch \
         {} -> {}, expected +1). Any admission assertion below would be vacuous.",
        test,
        epoch_before,
        mgr.pipeline.sync_epoch
    );
}

// ===========================================================================
// T1 — CORE REPRODUCTION. MUST FAIL pre-fix.
// ===========================================================================

/// T1 (IP-B × IP-E × IP-G): the measured INC-I-152 node. Orphan chase applied
/// genesis blocks 1..14, so `local_height == 14` — inside the mainnet genesis
/// window (360) but no longer the fencepost 0. With 3 agreeing peers at the live
/// tip (gap ≈ 129,786 > SNAP_SYNC_GAP_MIN) it is still a bootstrap-shaped node and
/// MUST take snap, not a 260-second serial header walk.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state         — Syncing{phase: SnapCollecting}
///   O2.2 self.pipeline_data — SnapCollecting{..}
///   O2.3 self.pipeline.sync_epoch — +1 (ran past P0/P1)
/// PATH P4: should_snap && consensus_target_hash() == Some
/// INPUT PARTITIONS: IP-B (0 < h < genesis_blocks), IP-E (gap > 500),
///   IP-G (3 peers), attempts=0, needs_genesis_resync=false — the ONLY difference
///   from T2 is the height partition, which is exactly the defect.
/// MATRIX: P4 → O2.1 ✓ O2.2 ✓ O2.3 ✓.
/// VERDICT: **FAILS pre-fix** — `should_snap` requires `local_height == 0 ||
/// needs_genesis_resync`; at h=14 both are false, so start_sync commits header-first.
#[test]
fn inc_i_152_orphan_chased_node_in_genesis_window_admits_snap() {
    // Precondition: the measured gap must be bootstrap-scale (compile-time).
    const _: () = assert!(MAINNET_TIP - ORPHAN_CHASE_HEIGHT > thresholds::SNAP_SYNC_GAP_MIN);

    let mut mgr = mgr(ORPHAN_CHASE_HEIGHT, MAINNET_TIP, 3, MAINNET_GENESIS_BLOCKS);
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T1");
    // O2.2 — the admission decision itself.
    assert!(
        is_snap(&mgr),
        "INC-I-152 T1: a node at h={} (inside the genesis window of {}) with {} \
         agreeing peers at h={} (gap={}) committed to {:?} instead of SnapCollecting. \
         Orphan chase applying genesis blocks 1..14 must not foreclose bootstrap snap \
         — this is the measured 260s header-walk defect.",
        ORPHAN_CHASE_HEIGHT,
        MAINNET_GENESIS_BLOCKS,
        mgr.peers.len(),
        MAINNET_TIP,
        MAINNET_TIP - ORPHAN_CHASE_HEIGHT,
        std::mem::discriminant(&mgr.pipeline_data)
    );
    // O2.1 — the phase label must agree with the pipeline data.
    assert!(
        matches!(
            mgr.state,
            SyncState::Syncing {
                phase: SyncPhase::SnapCollecting,
                ..
            }
        ),
        "INC-I-152 T1: O2.1 — state must be Syncing{{SnapCollecting}}, observed {:?}",
        mgr.state
    );
}

// ===========================================================================
// T2-T4, T6, T7 — PASS-LOCKS. Green pre-fix AND post-fix.
// ===========================================================================

/// T2 (IP-A × IP-E × IP-G): INV-SYNC-011 path (a) is untouched. A true fresh node
/// (h == 0) with 3 agreeing peers and a bootstrap-scale gap still takes snap.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state — Syncing{SnapCollecting} · O2.2 pipeline_data — SnapCollecting
///   O2.3 sync_epoch — +1
/// PATH P4. INPUT PARTITIONS: IP-A (h==0) — the fencepost the fix must PRESERVE,
///   not replace. MATRIX: P4 → O2.1 ✓ O2.2 ✓ O2.3 ✓.
/// VERDICT: PASS-lock (pre-fix and post-fix).
#[test]
fn inc_i_152_fresh_node_h0_still_admits_snap() {
    let mut mgr = mgr(0, MAINNET_TIP, 3, MAINNET_GENESIS_BLOCKS);
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T2");
    assert!(
        is_snap(&mgr),
        "INC-I-152 T2 (INV-SYNC-011 path (a)): h==0 bootstrap admission regressed — \
         observed {:?}",
        std::mem::discriminant(&mgr.pipeline_data)
    );
    assert!(
        matches!(
            mgr.state,
            SyncState::Syncing {
                phase: SyncPhase::SnapCollecting,
                ..
            }
        ),
        "INC-I-152 T2: O2.1 — state must be Syncing{{SnapCollecting}}, observed {:?}",
        mgr.state
    );
}

/// T3 (IP-C × IP-E × IP-G): one block PAST the window (h = genesis_blocks + 1) with
/// a huge gap and 3 peers must stay header-first. This is the guard that keeps the
/// new window term from degenerating into a bare-gap admission (INV-SYNC-011: "no
/// bare-gap term admits snap on any path") — a steady-state node that fell far
/// behind still needs `needs_genesis_resync` evidence.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state — Syncing{DownloadingHeaders} · O2.2 pipeline_data — Headers
///   O2.3 sync_epoch — +1
/// PATH P6 (!should_snap). INPUT PARTITIONS: IP-C (h > genesis_blocks) — the exact
///   off-by-one neighbour of T1's IP-B. MATRIX: P6 → O2.1 ✓ O2.2 ✓ O2.3 ✓.
/// VERDICT: PASS-lock (pre-fix and post-fix).
#[test]
fn inc_i_152_height_outside_genesis_window_does_not_admit_snap() {
    let mut mgr = mgr(
        MAINNET_GENESIS_BLOCKS + 1,
        MAINNET_TIP,
        3,
        MAINNET_GENESIS_BLOCKS,
    );
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T3");
    assert!(
        !is_snap(&mgr),
        "INC-I-152 T3 (INV-SYNC-011 no-bare-gap): h={} is OUTSIDE the genesis window \
         ({}) — a far-behind steady-state node must not be admitted to snap on gap \
         alone; it needs needs_genesis_resync evidence.",
        MAINNET_GENESIS_BLOCKS + 1,
        MAINNET_GENESIS_BLOCKS
    );
    assert!(
        is_headers(&mgr)
            && matches!(
                mgr.state,
                SyncState::Syncing {
                    phase: SyncPhase::DownloadingHeaders,
                    ..
                }
            ),
        "INC-I-152 T3: O2.1/O2.2 — an outside-window node must COMMIT header-first \
         (not park); observed state={:?}",
        mgr.state
    );
}

/// T4 (IP-B × IP-F × IP-G): inside the window but the gap is a catch-up gap
/// (< SNAP_SYNC_GAP_MIN). The window alone must not admit snap — the same
/// no-bare-admission constraint as T3, approached from the gap axis.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state — Syncing{DownloadingHeaders} · O2.2 pipeline_data — Headers
///   O2.3 sync_epoch — +1
/// PATH P6 (!should_snap). INPUT PARTITIONS: IP-F (0 < gap < 500) with IP-B held
///   fixed against T1 — isolates the gap conjunct. MATRIX: P6 → O2.1 ✓ O2.2 ✓ O2.3 ✓.
/// VERDICT: PASS-lock (pre-fix and post-fix).
#[test]
fn inc_i_152_in_window_small_gap_does_not_admit_snap() {
    let peer_height = 300;
    assert!(
        peer_height - ORPHAN_CHASE_HEIGHT < thresholds::SNAP_SYNC_GAP_MIN,
        "precondition: this test's gap must be BELOW the snap floor"
    );
    let mut mgr = mgr(ORPHAN_CHASE_HEIGHT, peer_height, 3, MAINNET_GENESIS_BLOCKS);
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T4");
    assert!(
        !is_snap(&mgr),
        "INC-I-152 T4 (INV-SYNC-011 no-bare-gap, gap axis): h={} inside the window \
         with gap={} (< SNAP_SYNC_GAP_MIN {}) must stay header-first — the genesis \
         window is a bootstrap-SHAPE predicate, never a gap relaxation.",
        ORPHAN_CHASE_HEIGHT,
        peer_height - ORPHAN_CHASE_HEIGHT,
        thresholds::SNAP_SYNC_GAP_MIN
    );
    assert!(
        is_headers(&mgr)
            && matches!(
                mgr.state,
                SyncState::Syncing {
                    phase: SyncPhase::DownloadingHeaders,
                    ..
                }
            ),
        "INC-I-152 T4: O2.1/O2.2 — a small-gap in-window node must COMMIT header-first \
         (not park); observed state={:?}",
        mgr.state
    );
}

// ===========================================================================
// T5 — SUB-QUORUM HOLD. MUST FAIL pre-fix.
// ===========================================================================

/// T5 (IP-B × IP-E × IP-H): inside the window with only 2 peers. The node must PARK
/// in the fresh-node hold exactly like an h==0 node does today
/// (`m6_rc1_fresh_node_h0_still_waits`) — committing header-first here is precisely
/// what buys the 260-second walk, because the hold is the mechanism that waits for
/// the snap quorum to appear.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state         — Idle (parked, NO commit)
///   O2.2 self.pipeline_data — None
///   O2.3 self.pipeline.sync_epoch — +1 (the bump precedes the hold)
///   O2.7 self.snap.fresh_node_wait_start — Some (hold armed via get_or_insert)
///   O2.9 self.network.last_sync_activity — UNCHANGED (only a commit resets it)
/// PATH P2 (fresh-node hold, waited < 60s).
/// INPUT PARTITIONS: IP-H (peers < 3) with IP-B/IP-E held fixed against T1 —
///   isolates the quorum conjunct. MATRIX: P2 → O2.1 ✓ O2.2 ✓ O2.3 ✓ O2.7 ✓ O2.9 ✓.
/// VERDICT: **FAILS pre-fix** — the hold at decision.rs:173 is keyed on
/// `local_height == 0`, so an h=14 node skips it and commits header-first.
#[test]
fn inc_i_152_in_window_sub_quorum_parks_instead_of_header_first() {
    let mut mgr = mgr(ORPHAN_CHASE_HEIGHT, MAINNET_TIP, 2, MAINNET_GENESIS_BLOCKS);
    mgr.snap.fresh_node_wait_start = None; // first get_or_insert → now, waited < 60s
    let epoch_before = mgr.pipeline.sync_epoch;
    let activity_before = mgr.network.last_sync_activity;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T5");
    // O2.2 — no pipeline commitment at all.
    assert!(
        matches!(mgr.pipeline_data, SyncPipelineData::None),
        "INC-I-152 T5: an in-window node ({} peers < 3, gap={}) must PARK awaiting \
         snap peers, exactly like an h==0 node. Observed pipeline {:?} — committing \
         header-first here is the 260s walk.",
        mgr.peers.len(),
        MAINNET_TIP - ORPHAN_CHASE_HEIGHT,
        std::mem::discriminant(&mgr.pipeline_data)
    );
    // O2.1 — parked means Idle, not Syncing.
    assert!(
        matches!(mgr.state, SyncState::Idle),
        "INC-I-152 T5: O2.1 — parked node must remain Idle, observed {:?}",
        mgr.state
    );
    // O2.7 — the hold must be armed so the 60s fallback timer starts.
    assert!(
        mgr.snap.fresh_node_wait_start.is_some(),
        "INC-I-152 T5: O2.7 — the fresh-node hold must arm fresh_node_wait_start so \
         the 60s deadlock fallback can expire"
    );
    // O2.9 — only a committed phase resets the stuck-sync timer.
    assert_eq!(
        mgr.network.last_sync_activity, activity_before,
        "INC-I-152 T5: O2.9 — last_sync_activity must NOT be reset on a park; a reset \
         proves a phase was committed"
    );
}

/// T6 (IP-B × IP-E × IP-G × IP-I): inside the window, quorum present, huge gap —
/// but `snap.attempts == 3`. The new admission term must grant NO bypass of the
/// attempt limiter; the node falls back to header-first.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state — Syncing{DownloadingHeaders} · O2.2 pipeline_data — Headers
///   O2.3 sync_epoch — +1
/// PATH P6 (!should_snap: attempts limiter). INPUT PARTITIONS: IP-I (attempts == 3)
///   with every T1 partition otherwise identical — isolates the limiter conjunct.
///   MATRIX: P6 → O2.1 ✓ O2.2 ✓ O2.3 ✓.
/// VERDICT: PASS-lock (pre-fix and post-fix).
#[test]
fn inc_i_152_in_window_exhausted_attempts_does_not_admit_snap() {
    let mut mgr = mgr(ORPHAN_CHASE_HEIGHT, MAINNET_TIP, 3, MAINNET_GENESIS_BLOCKS);
    mgr.snap.attempts = 3;
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T6");
    assert!(
        !is_snap(&mgr),
        "INC-I-152 T6: snap.attempts==3 must still veto admission inside the genesis \
         window — the window term must not bypass the attempt limiter"
    );
    assert!(
        is_headers(&mgr)
            && matches!(
                mgr.state,
                SyncState::Syncing {
                    phase: SyncPhase::DownloadingHeaders,
                    ..
                }
            ),
        "INC-I-152 T6: O2.1/O2.2 — with attempts exhausted the node must COMMIT \
         header-first (not park forever); observed state={:?}",
        mgr.state
    );
}

/// T7 (IP-D × IP-E × IP-G): the plumbing contract. With `SyncConfig::default()`
/// (`genesis_blocks == 0`, window DISABLED) an h=14 node behaves exactly as it does
/// today — header-first. This is what keeps every pre-existing sync test
/// bit-identical after the field is added, and it mirrors
/// `Network::is_in_genesis()`'s own `genesis_blocks > 0` guard.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state — Syncing{DownloadingHeaders} · O2.2 pipeline_data — Headers
///   O2.3 sync_epoch — +1
/// PATH P6 (!should_snap). INPUT PARTITIONS: IP-D (genesis_blocks == 0) — same
///   height/gap/peers as T1, only the plumbed window value differs, so this is the
///   direct A/B of the config term. MATRIX: P6 → O2.1 ✓ O2.2 ✓ O2.3 ✓.
/// VERDICT: PASS-lock (pre-fix and post-fix).
#[test]
fn inc_i_152_window_disabled_by_default_does_not_admit_snap() {
    assert_eq!(
        SyncConfig::default().genesis_blocks,
        0,
        "INC-I-152 T7: SyncConfig::default() must leave the genesis window DISABLED \
         so unplumbed call sites keep today's behavior"
    );
    let mut mgr = mgr(ORPHAN_CHASE_HEIGHT, MAINNET_TIP, 3, 0);
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T7");
    assert!(
        !is_snap(&mgr),
        "INC-I-152 T7: with genesis_blocks=0 the window is disabled — admission must \
         be identical to pre-INC-I-152 behavior (header-first)"
    );
    assert!(
        is_headers(&mgr)
            && matches!(
                mgr.state,
                SyncState::Syncing {
                    phase: SyncPhase::DownloadingHeaders,
                    ..
                }
            ),
        "INC-I-152 T7: O2.1/O2.2 — window-disabled node must COMMIT header-first; \
         observed state={:?}",
        mgr.state
    );
}

// ===========================================================================
// T8/T9 — COMPARATOR BOUNDARY (review F5). One comparator across the cluster.
// ===========================================================================

/// T8 (IP-B × IP-J × IP-G): gap == SNAP_SYNC_GAP_MIN EXACTLY must NOT admit snap.
/// The whole admission cluster — the window term in `should_snap` and both bootstrap
/// holds — reads `gap > SNAP_SYNC_GAP_MIN`. Before review F5 the window term alone
/// read `>=`, so at this exact value a >=3-peer node snapped while a <3-peer node
/// refused to park for the very quorum that would have let it snap. T8 pins the
/// strictly-conservative side of that boundary.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state — Syncing{DownloadingHeaders} · O2.2 pipeline_data — Headers
///   O2.3 sync_epoch — +1
/// PATH P6 (!should_snap: gap term false at the floor).
/// INPUT PARTITIONS: IP-J lower cell (gap == 500) with every T1 partition otherwise
///   identical — isolates the comparator itself against T9. MATRIX: P6 → O2.1 ✓
///   O2.2 ✓ O2.3 ✓.
/// VERDICT: PASS-lock post-fix; it also FAILED against the pre-F5 `>=` form, which
/// is exactly the drift it exists to catch.
#[test]
fn inc_i_152_in_window_gap_exactly_at_floor_does_not_admit_snap() {
    let peer_height = ORPHAN_CHASE_HEIGHT + thresholds::SNAP_SYNC_GAP_MIN;
    let mut mgr = mgr(ORPHAN_CHASE_HEIGHT, peer_height, 3, MAINNET_GENESIS_BLOCKS);
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T8");
    assert!(
        !is_snap(&mgr),
        "INC-I-152 T8 (comparator parity): gap == SNAP_SYNC_GAP_MIN ({}) exactly must \
         NOT admit snap — `should_snap` must read `gap > SNAP_SYNC_GAP_MIN`, the same \
         comparator as both bootstrap holds. A `>=` here means a 3-peer node snaps at \
         this value while a 2-peer node refuses to park for the quorum.",
        thresholds::SNAP_SYNC_GAP_MIN
    );
    assert!(
        is_headers(&mgr)
            && matches!(
                mgr.state,
                SyncState::Syncing {
                    phase: SyncPhase::DownloadingHeaders,
                    ..
                }
            ),
        "INC-I-152 T8: O2.1/O2.2 — a floor-value node must COMMIT header-first (not \
         park); observed state={:?}",
        mgr.state
    );
}

/// T9 (IP-B × IP-J × IP-G): gap == SNAP_SYNC_GAP_MIN + 1 — the first value ABOVE the
/// floor — MUST admit snap. Paired with T8 this brackets the comparator to a single
/// value, so neither loosening it to `>=` nor tightening it further can pass both.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state — Syncing{SnapCollecting} · O2.2 pipeline_data — SnapCollecting
///   O2.3 sync_epoch — +1
/// PATH P4 (should_snap && majority hash).
/// INPUT PARTITIONS: IP-J upper cell (gap == 501); identical to T8 in every other
///   partition. MATRIX: P4 → O2.1 ✓ O2.2 ✓ O2.3 ✓.
/// VERDICT: post-fix lock (fails pre-fix for the same reason as T1 — the window term
/// does not exist there).
#[test]
fn inc_i_152_in_window_gap_one_above_floor_admits_snap() {
    let peer_height = ORPHAN_CHASE_HEIGHT + thresholds::SNAP_SYNC_GAP_MIN + 1;
    let mut mgr = mgr(ORPHAN_CHASE_HEIGHT, peer_height, 3, MAINNET_GENESIS_BLOCKS);
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T9");
    assert!(
        is_snap(&mgr),
        "INC-I-152 T9 (comparator parity): gap == SNAP_SYNC_GAP_MIN + 1 ({}) is the \
         first bootstrap-scale distance and MUST admit snap inside the window; \
         observed {:?}",
        thresholds::SNAP_SYNC_GAP_MIN + 1,
        std::mem::discriminant(&mgr.pipeline_data)
    );
    assert!(
        matches!(
            mgr.state,
            SyncState::Syncing {
                phase: SyncPhase::SnapCollecting,
                ..
            }
        ),
        "INC-I-152 T9: O2.1 — state must be Syncing{{SnapCollecting}}, observed {:?}",
        mgr.state
    );
}

// ===========================================================================
// T10 — DISCV5 GRACE HOLD (review F8 / QA ISSUE 3). MUST FAIL pre-fix.
// ===========================================================================

/// T10 (IP-B × IP-E × IP-H): the SECOND changed hold. The INC-I-152 amendment widened
/// BOTH bootstrap holds; T5 covers only the fresh-node hold (P2). QA mutation-tested
/// the discv5-grace hold (decision.rs:221) by reverting it to `local_height == 0` and
/// found ALL 511 lib tests still green — the half of the fix that parks a node while
/// discv5's UDP random walk finds TCP peers shipped with zero coverage.
///
/// To reach P3 the fresh-node hold at P2 must be made to FALL THROUGH, not return:
/// `fresh_node_wait_start` is pre-armed 61s in the past so its 60s budget is already
/// spent. The test therefore asserts `fresh_node_wait_start == None` (the P2 fallback
/// clears it) as positive proof the park observed below came from the discv5 grace and
/// not from the fresh-node hold — without that witness this test would pass for T5's
/// reason and the mutant would survive again.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state         — Idle (parked, NO commit)
///   O2.2 self.pipeline_data — None
///   O2.3 self.pipeline.sync_epoch — +1 (the bump precedes both holds)
///   O2.7 self.snap.fresh_node_wait_start — None (P2's 60s fallback fired and fell
///        through; this is the path witness that distinguishes P3 from P2)
///   O2.8 self.snap.discv5_peer_grace_deadline — still Some (grace NOT expired, so
///        the expiry branch that clears it was not taken)
/// PATH P3 (discv5 grace, now < deadline).
/// INPUT PARTITIONS: IP-H (peers < 3) with IP-B/IP-E held fixed against T1, plus an
///   EXHAUSTED fresh-node budget — the only cell that reaches the grace guard.
///   MATRIX: P3 → O2.1 ✓ O2.2 ✓ O2.3 ✓ O2.7 ✓ O2.8 ✓.
/// VERDICT: **FAILS pre-fix** — with the grace keyed on `local_height == 0` an h=14
/// node skips it entirely and commits header-first (Headers, state Syncing).
#[test]
fn inc_i_152_in_window_sub_quorum_parks_in_discv5_grace() {
    let mut mgr = mgr(ORPHAN_CHASE_HEIGHT, MAINNET_TIP, 2, MAINNET_GENESIS_BLOCKS);
    // Spend the fresh-node hold's 60s budget so P2 warns and falls through to P3.
    mgr.snap.fresh_node_wait_start = Some(Instant::now() - Duration::from_secs(61));
    mgr.snap.discv5_peer_grace_deadline = Some(Instant::now() + Duration::from_secs(30));
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T10");
    // O2.7 — path witness: P2 expired and fell through, so the park below is P3's.
    assert!(
        mgr.snap.fresh_node_wait_start.is_none(),
        "INC-I-152 T10: O2.7 — the fresh-node hold must have EXPIRED and fallen \
         through (its 60s budget was pre-spent); a Some() here means the park came \
         from P2 and this test does not exercise the discv5 grace at all"
    );
    // O2.2 — no pipeline commitment: the grace parked the node.
    assert!(
        matches!(mgr.pipeline_data, SyncPipelineData::None),
        "INC-I-152 T10: an in-window node ({} peers < 3, gap={}) with an unexpired \
         discv5 grace must PARK while discv5 discovers TCP peers, exactly like an \
         h==0 node. Observed pipeline {:?} — committing header-first here forecloses \
         snap for the whole ~260s walk.",
        mgr.peers.len(),
        MAINNET_TIP - ORPHAN_CHASE_HEIGHT,
        std::mem::discriminant(&mgr.pipeline_data)
    );
    // O2.1 — parked means Idle, not Syncing.
    assert!(
        matches!(mgr.state, SyncState::Idle),
        "INC-I-152 T10: O2.1 — parked node must remain Idle, observed {:?}",
        mgr.state
    );
    // O2.8 — an unexpired grace is never cleared; only the expiry branch clears it.
    assert!(
        mgr.snap.discv5_peer_grace_deadline.is_some(),
        "INC-I-152 T10: O2.8 — an UNEXPIRED discv5 grace must not be cleared; a None \
         here means the expiry branch ran and the park was not the grace"
    );
}

// ===========================================================================
// T11 — WINDOW FENCEPOST (review F9). Inclusive upper bound.
// ===========================================================================

/// T11 (IP-K × IP-E × IP-G): `local_height == genesis_blocks` EXACTLY is still inside
/// the window. T3 probes the boundary only from OUTSIDE (`genesis_blocks + 1`), which
/// leaves the inclusive `<=` unpinned — a drift to `<` would keep T3 green while
/// silently excluding the last block of the genesis window. The bound is inclusive to
/// mirror `Network::is_in_genesis()` (core/src/network/economics.rs:56, `height <=
/// genesis_blocks`); T3 and T11 together bracket it to a single value.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O2.1 self.state — Syncing{SnapCollecting} · O2.2 pipeline_data — SnapCollecting
///   O2.3 sync_epoch — +1
/// PATH P4 (should_snap && majority hash).
/// INPUT PARTITIONS: IP-K (h == genesis_blocks) — the immediate off-by-one neighbour
///   of T3's IP-C, every other partition identical. MATRIX: P4 → O2.1 ✓ O2.2 ✓ O2.3 ✓.
/// VERDICT: post-fix lock (fails pre-fix for the same reason as T1).
#[test]
fn inc_i_152_height_at_window_fencepost_admits_snap() {
    let mut mgr = mgr(
        MAINNET_GENESIS_BLOCKS,
        MAINNET_TIP,
        3,
        MAINNET_GENESIS_BLOCKS,
    );
    let epoch_before = mgr.pipeline.sync_epoch;

    mgr.start_sync();

    assert_ran(&mgr, epoch_before, "INC-I-152 T11");
    assert!(
        is_snap(&mgr),
        "INC-I-152 T11 (inclusive fencepost): h == genesis_blocks ({}) is the LAST \
         height inside the window — the bound is `<=`, mirroring \
         Network::is_in_genesis(). Observed {:?}; a `<` drift would exclude it while \
         leaving T3 (h = genesis_blocks + 1) green.",
        MAINNET_GENESIS_BLOCKS,
        std::mem::discriminant(&mgr.pipeline_data)
    );
    assert!(
        matches!(
            mgr.state,
            SyncState::Syncing {
                phase: SyncPhase::SnapCollecting,
                ..
            }
        ),
        "INC-I-152 T11: O2.1 — state must be Syncing{{SnapCollecting}}, observed {:?}",
        mgr.state
    );
}
