//! INC-I-139 M6 — RC-1 (snap.threshold sentinel demotion) + RC-2 (emergency-enable
//! taxonomy) tests. Split from tests_inc_i139.rs to honor the 800-line test-file
//! budget. REQ-SNAP-008.
//!
//! INPUT PARTITIONS: N/A — verbatim move. Every OUTPUT CONTRACT + INPUT PARTITION
//! below is unchanged from the original tests_inc_i139.rs M6 section (pure
//! relocation, no logic or assertion edits). See each test's own INPUT PARTITION.

use crypto::Hash;
use libp2p::PeerId;
use std::time::{Duration, Instant};

use super::tests_inc_i139::mgr_with_agreeing_peers;
use crate::sync::manager::{
    RecoveryPhase, RecoveryReason, SyncConfig, SyncManager, SyncPipelineData, SyncState,
};

// ===========================================================================
// === M6 — RC-1 + RC-2 (REQ-SNAP-008) ===
//
// M6 makes snap.threshold a pure enable/disable SENTINEL and re-homes every
// gap-COMPARATOR read onto thresholds::SNAP_SYNC_GAP_MIN. Three consequences:
//   RC-1c: the discv5-grace wait (decision.rs:202) must only park a FRESH node
//          (local_height==0); post-DC-1 an h>0 node never uses snap peers.
//          AMENDED by INC-I-152: "bootstrap-shaped" now means `h==0 ||
//          in_genesis_window`, so the exemption is "h PAST the genesis window".
//          TEST 1 pins the genesis_blocks==0 cell; TEST 1b the production cell.
//   RC-1b: no `gap > self.snap.threshold` comparator read may remain in the
//          admission region of decision.rs (only sentinel `< u64::MAX` reads).
//   RC-2 : the emergency re-enable (production_gate.rs:741 `threshold = 10`) is
//          replaced by an explicit enable sentinel that preserves behavior
//          BIT-FOR-BIT — snap is still admitted under --no-snap-sync for the
//          emergency reason set.
// Spec: specs/sync-snap-admission-architecture.md RC-1 + RC-2 (~154-188).
// ===========================================================================

/// M6 TEST 1 (REQ-SNAP-008, RC-1c): a node with a future discv5-grace deadline,
/// <3 peers and gap>threshold must NOT park in the grace-wait when it is not
/// bootstrap-shaped — it proceeds to header-first.
///
/// CONTRACT AMENDED BY INC-I-152 (was: "an h>0 node never parks for snap peers").
/// That absolute is no longer true in production: the bootstrap exemption is now
/// "h PAST the genesis window", so on mainnet (`genesis_blocks=360`) this test's own
/// h=100 node DOES park in the grace. This test still holds — and still passes —
/// because `mgr_with_agreeing_peers` builds on `SyncConfig::default()`, i.e.
/// `genesis_blocks == 0`, which DISABLES the window. Read it as the
/// window-disabled cell of the contract: with no genesis window plumbed, RC-1c
/// degenerates to the original `local_height == 0` gate and an h>0 node proceeds.
/// The production-shaped cell (`genesis_blocks=360`, h outside the window) is
/// pinned by the companion test immediately below.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O1: self.state        — MUST become Syncing (proceeded to header-first)
///   O2: self.pipeline_data — MUST NOT be SnapCollecting
///   PATH P1: local_height=100, 2 peers (<3), gap=? >threshold(50),
///            discv5_peer_grace_deadline=Some(now+30s), snap enabled, attempts=0,
///            genesis_blocks=0 (SyncConfig::default → window disabled)
///   INPUT PARTITIONS: non-bootstrap-shaped node reaching the discv5-grace guard,
///            window DISABLED (`genesis_blocks == 0`)
///   MATRIX (O1): P1 → Syncing (MUST). FAILED pre-M6: the ungated grace guard parked
///                the h>0 node in Idle (early return at decision.rs:213).
#[test]
fn m6_h_gt_0_skips_discv5_grace_proceeds_header_first() {
    // local=100, 2 agreeing peers at 200 → gap=100 > snap.threshold(50), <3 peers.
    let mut mgr = mgr_with_agreeing_peers(100, 200, 2);
    // Reset to a clean Idle precondition (add_peer auto-drove start_sync during
    // construction — see the class2 note). Then arm the discv5 grace deadline and
    // drive start_sync once with the h>0 node reaching decision.rs:202.
    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;
    mgr.snap.attempts = 0;
    mgr.snap.threshold = 50; // enabled (< u64::MAX); gap=100 > threshold
    mgr.snap.discv5_peer_grace_deadline = Some(Instant::now() + Duration::from_secs(30));

    mgr.start_sync();

    // O2: no snap peer is ever used by an h>0 node (should_snap requires
    // enough_peers anyway, but assert it explicitly against the redesign).
    let snapped = matches!(mgr.pipeline_data, SyncPipelineData::SnapCollecting { .. });
    assert!(
        !snapped,
        "M6 RC-1c (REQ-SNAP-008): h>0 node must never enter SnapCollecting"
    );
    // O1: the h>0 node must have PROCEEDED to header-first (state==Syncing), not
    // parked in Idle behind the discv5 grace. Today the ungated grace guard
    // (decision.rs:202) early-returns for any node with <3 peers + gap>threshold,
    // leaving state==Idle → this assertion FAILS pre-change (documented red).
    assert!(
        matches!(mgr.state, SyncState::Syncing { .. }),
        "M6 RC-1c (REQ-SNAP-008): h>0 node with a future discv5-grace deadline and \
         gap>threshold parked in the grace-wait (state={:?}) instead of proceeding \
         to header-first. Post-RC-1c the grace guard must gate on bootstrap shape; \
         with genesis_blocks==0 that reduces to local_height==0.",
        mgr.state
    );
}

/// M6 TEST 1b (RC-1c under PRODUCTION config — added by INC-I-152, review F2).
/// The companion to TEST 1. TEST 1 can only ever exercise `genesis_blocks == 0`, so
/// after INC-I-152 it no longer covers the shipped mainnet shape: there the grace
/// gate is `local_height == 0 || in_genesis_window`, and the boundary that matters
/// is the WINDOW edge, not the h==0 fencepost. This test plumbs the real mainnet
/// window (360) and places the node OUTSIDE it (h=400) — the cell TEST 1 intended to
/// pin ("a node that will never use snap peers must not park for them") and no
/// longer can.
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O1: self.state         — MUST become Syncing{DownloadingHeaders} (proceeded)
///   O2: self.pipeline_data — MUST be Headers, never SnapCollecting
///   PATH P1: genesis_blocks=360, local_height=400 (> window), 2 peers (<3),
///            gap=800 > SNAP_SYNC_GAP_MIN(500), discv5_peer_grace_deadline
///            =Some(now+30s), snap enabled (threshold=50), attempts=0
///   INPUT PARTITIONS: past-window node under a PLUMBED genesis window — the
///            production-shaped complement of TEST 1's window-disabled cell.
///   MATRIX (O1,O2): P1 → (Syncing{DownloadingHeaders}, Headers). PASS-lock: green
///            both pre- and post-INC-I-152, since a past-window node is excluded
///            from both bootstrap holds in either version.
#[test]
fn m6_rc1c_past_genesis_window_skips_discv5_grace_proceeds_header_first() {
    // Mainnet NetworkParams::genesis_blocks (crates/core/.../defaults.rs:46).
    const MAINNET_GENESIS_BLOCKS: u64 = 360;
    let local_height = 400; // strictly PAST the window
    let peer_height = 1200; // gap = 800 > SNAP_SYNC_GAP_MIN(500)

    let config = SyncConfig {
        genesis_blocks: MAINNET_GENESIS_BLOCKS,
        ..SyncConfig::default()
    };
    let mut mgr = SyncManager::new(config, Hash::ZERO);
    mgr.local_height = local_height;
    mgr.local_slot = local_height as u32;
    mgr.local_hash = crypto::hash::hash(format!("local_{}", local_height).as_bytes());
    let peer_hash = crypto::hash::hash(b"canonical_agreed_tip");
    for _ in 0..2 {
        mgr.add_peer(PeerId::random(), peer_height, peer_hash, peer_height as u32);
    }
    // Clean Idle precondition (add_peer drove start_sync during construction), then
    // arm the grace deadline the past-window node must NOT honor.
    mgr.fork.needs_genesis_resync = false;
    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;
    mgr.snap.attempts = 0;
    mgr.snap.threshold = 50; // enabled sentinel
    mgr.snap.discv5_peer_grace_deadline = Some(Instant::now() + Duration::from_secs(30));

    mgr.start_sync();

    // O2: a past-window node never reaches snap without needs_genesis_resync.
    assert!(
        !matches!(mgr.pipeline_data, SyncPipelineData::SnapCollecting { .. }),
        "M6 RC-1c/INC-I-152: a node at h={} PAST the genesis window ({}) must not \
         enter SnapCollecting on gap alone",
        local_height,
        MAINNET_GENESIS_BLOCKS
    );
    // O1: and it must not PARK for snap peers it will never use.
    assert!(
        matches!(
            mgr.state,
            SyncState::Syncing {
                phase: crate::sync::manager::SyncPhase::DownloadingHeaders,
                ..
            }
        ) && matches!(mgr.pipeline_data, SyncPipelineData::Headers { .. }),
        "M6 RC-1c/INC-I-152 (production shape): a node at h={} PAST the genesis \
         window ({}) with an armed discv5 grace, 2 peers and gap={} parked instead \
         of proceeding to header-first (state={:?}). The INC-I-152 bootstrap holds \
         must cover ONLY h==0 and the genesis window — never a past-window node.",
        local_height,
        MAINNET_GENESIS_BLOCKS,
        peer_height - local_height,
        mgr.state
    );
}

/// M6 TEST 2 (REQ-SNAP-008, RC-2): the emergency re-enable still admits snap under
/// --no-snap-sync. BIT-FOR-BIT backstop — MUST PASS on current code (threshold→10)
/// AND remain green after RC-2 (threshold→explicit enable sentinel). Asserts the
/// enabled sentinel `< u64::MAX`, NEVER the literal value 10.
///
/// OUTPUT CONTRACT: fn request_genesis_resync(&mut self, reason) -> bool
///   [production_gate.rs:660]
///   O1: return bool                       — MUST be true (all gates pass)
///   O2: self.fork.needs_genesis_resync    — MUST be set (needs_genesis_resync()==true)
///   O3: self.snap.threshold               — MUST be enabled (< u64::MAX) after the call
///   PATH P1: threshold=u64::MAX (disabled), floor=0, attempts=0, phase=Normal,
///            reason=GenesisFallbackEmptyHeaders (emergency set)
///   INPUT PARTITIONS: --no-snap-sync + emergency reason → Gate 4 re-enables snap
///   MATRIX: P1 → (true, set, enabled). PASSES today AND post-RC-2 (bit-for-bit).
#[test]
fn m6_rc2_emergency_reenable_admits_snap_under_no_snap_sync() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.disable_snap_sync(); // snap.threshold = u64::MAX (--no-snap-sync)
    assert_eq!(
        mgr.snap.threshold,
        u64::MAX,
        "precondition: snap must start disabled"
    );
    mgr.confirmed_height_floor = 0;
    mgr.snap.attempts = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;

    // GenesisFallbackEmptyHeaders ∈ the emergency set (production_gate.rs:666-671:
    // GenesisFallbackEmptyHeaders | AllPeersBlacklistedDeepFork | ApplyFailuresSnapThreshold).
    let honored = mgr.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);

    // O1: emergency bypasses Gate 4 (--no-snap-sync) and is honored.
    assert!(
        honored,
        "M6 RC-2 (REQ-SNAP-008): emergency GenesisFallbackEmptyHeaders under \
         --no-snap-sync must be honored"
    );
    // O2: the single X1 guard is armed.
    assert!(
        mgr.needs_genesis_resync(),
        "M6 RC-2 (REQ-SNAP-008): honored emergency must set needs_genesis_resync"
    );
    // O3: snap is now ENABLED. Assert the enabled sentinel, NOT the literal 10 —
    // RC-2 replaces `threshold = 10` with an explicit enable sentinel and this
    // backstop must stay green across that bit-for-bit swap.
    assert!(
        mgr.snap.threshold < u64::MAX,
        "M6 RC-2 (REQ-SNAP-008): emergency re-enable must leave snap ENABLED \
         (threshold < u64::MAX); observed {}",
        mgr.snap.threshold
    );
}

/// M6 TEST 3 (REQ-SNAP-008, RC-1b): the admission source of decision.rs must
/// retain the enable/disable SENTINEL read of snap.threshold but contain NO
/// gap-COMPARATOR read. FAILS pre-change (the `> self.snap.threshold` substring
/// exists at decision.rs:177 and :202); PASSES once RC-1b re-homes those reads
/// onto thresholds::SNAP_SYNC_GAP_MIN.
///
/// OUTPUT CONTRACT: structural invariant over include_str!("sync_engine/decision.rs")
///   O1: presence of sentinel read `self.snap.threshold < u64::MAX`  — MUST be true
///   O2: presence of comparator read `> self.snap.threshold`         — MUST be false
///   INPUT PARTITIONS: single structural partition — the decision.rs source text
///   MATRIX: post-RC-1b → (present, absent). FAILS today: O2 present at :177/:202.
#[test]
fn m6_rc1b_no_gap_comparator_read_of_threshold_in_decision() {
    let src = include_str!("sync_engine/decision.rs");

    // O1: the sentinel enable/disable read is PRESERVED (snap.threshold remains a
    // pure enable flag). Whitespace-robust: the canonical form has single spaces.
    assert!(
        src.contains("self.snap.threshold < u64::MAX"),
        "M6 RC-1b (REQ-SNAP-008): the enable/disable sentinel read \
         `self.snap.threshold < u64::MAX` must be preserved in decision.rs"
    );

    // O2: NO gap-comparator read remains. `> self.snap.threshold` is the
    // load-bearing substring of both `gap > self.snap.threshold` sites
    // (decision.rs:177 fresh-node wait, :202 discv5 grace). Post-RC-1b these read
    // thresholds::SNAP_SYNC_GAP_MIN instead. Robust to interior whitespace by
    // matching only the comparator+field fragment.
    assert!(
        !src.contains("> self.snap.threshold"),
        "M6 RC-1b (REQ-SNAP-008): a gap-comparator read `> self.snap.threshold` \
         still exists in decision.rs admission (decision.rs:177/:202). Post-RC-1b \
         gap comparisons must use thresholds::SNAP_SYNC_GAP_MIN; snap.threshold is \
         a pure enable/disable sentinel."
    );
}

// ===========================================================================
// M6 — RC-2 three-capability taxonomy locks + RC-1 preservation.
//
// The spec (RC-2, ~173-177) requires RC-2 to "document three orthogonal
// capabilities per RecoveryReason". Tests 1-3 above cover the discv5 h==0 gate,
// the structural threshold demotion, and one emergency-bypass backstop. The tests
// below LOCK the remaining contract so a future edit cannot silently re-couple the
// three capabilities or regress bootstrap:
//   (i)   bypass-floor        (Gate 1): emergency ∪ forward-large-gap  → class4 (above)
//   (ii)  bypass-op-disable   (Gate 4): emergency ONLY                 → m6_rc2_forward_...
//   (iii) rate/attempt limits (Gates 2/3/5): ALL reasons, no exception → m6_rc2_rate_...
// Plus the exact enabled sentinel (== enable_snap_sync()==50, never the magic 10)
// and RC-1 fresh-node (h==0) bootstrap preservation.
// All are PASS-locks against the shipped M6 code (regression protection).
// ===========================================================================

/// M6 TEST 4 (RC-2 capability ii): bypass-operator-disable is emergency-ONLY.
/// A forward-large-gap reason (CoordinatorSnapEscalation) is floor-exempt (Gate 1)
/// but is NOT emergency, so --no-snap-sync (Gate 4) still REFUSES it and snap stays
/// disabled. This keeps bypass-floor and bypass-operator-disable orthogonal.
///
/// OUTPUT CONTRACT: fn request_genesis_resync(&mut self, reason) -> bool
///   O1: return bool        — false (Gate 4 refuses a non-emergency under --no-snap-sync)
///   O2: self.snap.threshold — stays u64::MAX (NOT re-enabled)
///   O3: self.fork.needs_genesis_resync — stays false
///   PATH P1: threshold==u64::MAX, floor>0, reason=CoordinatorSnapEscalation
///   INPUT PARTITIONS: floor-exempt non-emergency reason under --no-snap-sync
///   MATRIX: P1 → (false, u64::MAX, false). PASS-lock (today AND future).
#[test]
fn m6_rc2_forward_large_gap_not_operator_disable_exempt() {
    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = 100;
    mgr.network.network_tip_height = 700; // gap=600 (forward large-gap)
    mgr.disable_snap_sync(); // threshold = u64::MAX (--no-snap-sync)
    mgr.snap.attempts = 0;
    mgr.confirmed_height_floor = 100; // floor>0: Gate 1 exempts forward-large-gap...
    mgr.consecutive_resync_count = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;

    let honored = mgr.request_genesis_resync(RecoveryReason::CoordinatorSnapEscalation);

    // O1: ...but Gate 4 (--no-snap-sync) still refuses it — it is NOT emergency.
    assert!(
        !honored,
        "M6 RC-2 (capability ii): CoordinatorSnapEscalation is floor-exempt (Gate 1) but \
         must NOT bypass --no-snap-sync (Gate 4) — only emergency reasons do. RC-2 must \
         keep bypass-floor and bypass-operator-disable orthogonal."
    );
    // O2: a non-emergency reason must NOT re-enable snap.
    assert_eq!(
        mgr.snap.threshold,
        u64::MAX,
        "M6 RC-2 (capability ii): a non-emergency reason must not re-enable snap"
    );
    // O3: refused request must not arm the X1 guard.
    assert!(
        !mgr.needs_genesis_resync(),
        "M6 RC-2 (capability ii): refused request must not set needs_genesis_resync"
    );
}

/// M6 TEST 5 (RC-2 capability iii): rate/attempt limits (Gates 2/3/5) apply to ALL
/// reasons — NO emergency exception. An emergency reason must still be refused once
/// snap attempts are exhausted (Gate 5) or the consecutive-resync cap is hit (Gate 3).
///
/// OUTPUT CONTRACT: fn request_genesis_resync(&mut self, reason) -> bool
///   O1: return bool — false on BOTH partitions
///   PATH P1a: emergency reason, snap.attempts>=3 (Gate 5)
///   PATH P1b: emergency reason, consecutive_resync_count>=MAX (Gate 3)
///   INPUT PARTITIONS: P1a attempts-exhausted ; P1b resync-cap-hit
///   MATRIX: P1a → false ; P1b → false. PASS-lock (today AND future).
#[test]
fn m6_rc2_rate_and_attempt_limits_apply_to_emergencies() {
    // Partition A — Gate 5 (snap attempts exhausted) refuses even an emergency.
    let mut mgr_a = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr_a.local_height = 100;
    mgr_a.snap.threshold = 50; // snap enabled
    mgr_a.snap.attempts = 3; // exhausted
    mgr_a.confirmed_height_floor = 0;
    mgr_a.consecutive_resync_count = 0;
    mgr_a.recovery_phase = RecoveryPhase::Normal;

    assert!(
        !mgr_a.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders),
        "M6 RC-2 (capability iii): an emergency with snap.attempts>=3 must be REFUSED by \
         Gate 5 — rate/attempt limits have NO emergency exception."
    );

    // Partition B — Gate 3 (consecutive-resync cap) refuses even an emergency.
    let mut mgr_b = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr_b.local_height = 100;
    mgr_b.snap.threshold = 50; // snap enabled
    mgr_b.snap.attempts = 0;
    mgr_b.confirmed_height_floor = 0;
    mgr_b.consecutive_resync_count = crate::sync::manager::MAX_CONSECUTIVE_RESYNCS; // cap hit
    mgr_b.recovery_phase = RecoveryPhase::Normal;

    assert!(
        !mgr_b.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders),
        "M6 RC-2 (capability iii): an emergency at the consecutive-resync cap \
         (MAX_CONSECUTIVE_RESYNCS) must be REFUSED by Gate 3 — no emergency exception."
    );
}

/// M6 TEST 6 (RC-2 exact sentinel): the emergency re-enable restores the CANONICAL
/// enabled sentinel (`enable_snap_sync()` == 50), never the ad-hoc magic value 10
/// (the old OQ-2 "snap at gap>10" minor-fork-hole number). Post-RC-1 the value is
/// dead gap-floor semantics, but the literal 10 must be gone.
///
/// OUTPUT CONTRACT: fn request_genesis_resync(&mut self, reason) -> bool
///   O1: self.snap.threshold — == fresh-manager enabled sentinel (50); != 10; < u64::MAX
///   PATH P1: threshold==u64::MAX, emergency reason, gates pass → Gate 4 re-enable
///   INPUT PARTITIONS: disabled snap + emergency reason → Gate 4 re-enable
///   MATRIX: P1 → threshold == enabled_sentinel(50). PASS-lock (today AND future).
#[test]
fn m6_rc2_emergency_reenable_restores_enabled_sentinel_not_magic_10() {
    // The canonical "snap enabled" sentinel = a fresh manager's default threshold.
    let enabled_sentinel = SyncManager::new(SyncConfig::default(), Hash::ZERO)
        .snap
        .threshold;

    let mut mgr = SyncManager::new(SyncConfig::default(), Hash::ZERO);
    mgr.local_height = 100;
    mgr.disable_snap_sync(); // threshold = u64::MAX (--no-snap-sync)
    mgr.snap.attempts = 0;
    mgr.confirmed_height_floor = 0;
    mgr.consecutive_resync_count = 0;
    mgr.recovery_phase = RecoveryPhase::Normal;

    let _ = mgr.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);

    // O1: the magic literal 10 is gone; the enable is the canonical enabled sentinel.
    assert_ne!(
        mgr.snap.threshold, 10,
        "M6 RC-2 (sentinel): emergency re-enable must NOT set the magic literal 10 \
         (the old OQ-2 minor-fork-hole value); RC-2 replaced it with an explicit sentinel."
    );
    assert_eq!(
        mgr.snap.threshold, enabled_sentinel,
        "M6 RC-2 (sentinel): emergency re-enable must restore the canonical enabled \
         sentinel ({}), matching enable_snap_sync(). Observed {}",
        enabled_sentinel, mgr.snap.threshold
    );
}

/// M6 TEST 7 (RC-1 preservation): a fresh node (h==0) still WAITS for snap peers via
/// the fresh-node wait (decision.rs:173) — RC-1 re-homed its gap comparator onto
/// SNAP_SYNC_GAP_MIN(500) but must not regress bootstrap (REQ-SNAP-003). With a gap
/// above 500 and <3 peers, the fresh node parks (no header-first commit).
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O1: self.pipeline_data — None (fresh node still waits, no header-first commit)
///   PATH P1: h==0, 2 peers (<3), gap=600 (> SNAP_SYNC_GAP_MIN 500), snap enabled
///   INPUT PARTITIONS: fresh node (h==0) below quorum with a large gap
///   MATRIX (O1): P1 → None (waiting). PASS-lock: bootstrap preserved.
#[test]
fn m6_rc1_fresh_node_h0_still_waits() {
    // h==0, 2 peers (<3), gap=600 (> SNAP_SYNC_GAP_MIN 500) → fresh-node wait engages.
    let mut mgr = mgr_with_agreeing_peers(0, 600, 2);
    mgr.snap.fresh_node_wait_start = None; // first-call get_or_insert → now, waited<60
    mgr.snap.threshold = 50; // enabled
    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;

    mgr.start_sync();

    // O1: the fresh-node wait returns early (waited<60s) → no header-first commit.
    // RC-1 must not regress bootstrap: h==0 still parks to await snap peers.
    assert!(
        matches!(mgr.pipeline_data, SyncPipelineData::None),
        "M6 RC-1 preservation (REQ-SNAP-003): fresh node (h==0, gap>500) must still WAIT \
         for snap peers rather than commit to header-first. Observed pipeline discriminant \
         {:?}",
        std::mem::discriminant(&mgr.pipeline_data)
    );
}

/// M6 TEST 8 (RC-1, REQ-SNAP-008 exact-ceiling): a node parked at exactly the
/// minor-fork ceiling (gap == MINOR_FORK_GAP_MAX == 50) does NOT auto-promote to
/// snap, and `snap.threshold` set BELOW the gap (25) does not float it — proving the
/// zero-margin coupling is structurally dissolved (threshold is a sentinel, not a floor).
///
/// OUTPUT CONTRACT: fn start_sync(&mut self) [decision.rs:117]
///   O1: self.pipeline_data — NOT SnapCollecting
///   PATH P1: h>0, 3 agreeing peers, gap==50, no evidence, snap.threshold=25 (<gap)
///   INPUT PARTITIONS: h>0 node at exactly the minor-fork ceiling with threshold<gap
///   MATRIX (O1): P1 → NOT SnapCollecting. PASS-lock (post-DC-1/RC-1).
#[test]
fn m6_rc1_exact_ceiling_gap_does_not_float_snap() {
    // h=100, 3 agreeing peers at 150 → gap == MINOR_FORK_GAP_MAX (50) exactly.
    let mut mgr = mgr_with_agreeing_peers(100, 150, 3);
    mgr.fork.needs_genesis_resync = false; // no deep-fork signal
    mgr.snap.threshold = 25; // BELOW the gap — must NOT act as an admission floor
    mgr.state = SyncState::Idle;
    mgr.pipeline_data = SyncPipelineData::None;

    mgr.start_sync();

    // O1: should_snap requires `local_height==0 || needs_genesis_resync` (both false);
    // gap(50) > threshold(25) is irrelevant — threshold is an on/off sentinel.
    let snapped = matches!(mgr.pipeline_data, SyncPipelineData::SnapCollecting { .. });
    assert!(
        !snapped,
        "M6 RC-1 (REQ-SNAP-008): node at gap==MINOR_FORK_GAP_MAX(50) with threshold=25 \
         auto-promoted to snap — snap.threshold must be an on/off sentinel, not a gap \
         floor; the zero-margin coupling must be structurally dissolved."
    );
}
