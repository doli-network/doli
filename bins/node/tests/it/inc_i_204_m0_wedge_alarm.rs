//! INC-I-204 M0 / D7 — MILESTONE GATE. The incident signature must alert in
//! MINUTES, not days.
//!
//! Recorded shape: 9 nodes held a STALE tip while the winning chain advanced.
//! FINALITY_GUARD / plan_reorg refusals accumulated steadily (n6=43, n17=53,
//! seed=32 over DAYS) and `unique_chain_tips > 1` stayed sustained. Nothing
//! alerted; a human noticed, 7 days in.
//!
//! Time is INJECTED (`WedgeSample::at_secs`). Nothing here sleeps — a detector
//! whose test needs wall-clock time cannot be evaluated over a 5-minute window in
//! a test suite, and the temptation is then to shorten the window until the test
//! is fast, which is how the window stops matching production.
//!
//! OUTPUT CONTRACT
//!   Function under test: `WedgeAlarm::observe(&mut self, WedgeSample) -> WedgeVerdict`
//!   OBSERVABLE OUTPUTS asserted:
//!     O1: return value — `Clear` vs `Wedged { .. }`
//!     O2: `Wedged.stalled_secs`       — how long the tip has not moved
//!     O3: `Wedged.refusals_in_window` — refusals inside the bounded window only
//!     O4: `Wedged.unique_chain_tips`  — the fleet-divergence witness
//!     O5: receiver mutation — the alarm's own rolling window is bounded, so a
//!         long-lived alarm cannot fire on refusals older than `window_secs`
//!   CODE PATHS:
//!     P1: tip stalled AND refusals >= threshold in-window AND tips > 1  -> Wedged
//!     P2: tip advancing                                                -> Clear
//!     P3: refusals below threshold inside the window                   -> Clear
//!     P4: fleet unanimous (tips == 1)                                  -> Clear
//!     P5: previously Wedged, tip advances again                        -> Clear
//!   INPUT PARTITIONS:
//!     I1: the INC-I-204 replay — stalled tip, climbing refusals, tips=2,
//!         peers ahead
//!     I2: healthy control — tip advances each sample, no refusals, tips=1
//!     I3: refusals climbing WHILE the tip advances (a node making progress)
//!     I4: tip stalled, refusals climbing, tips=1 (whole-fleet stall, a
//!         different incident — this alarm must not claim it)
//!     I5: refusals spread thinner than the threshold per window
//!     I6: recovery — I1 followed by an advancing tip
//!   MATRIX:
//!     d7_incident_signature_alerts_within_one_window     : O1,O2,O3,O4 x P1 x I1
//!     d7_healthy_control_never_alerts                    : O1 x P2 x I2
//!     d7_refusals_with_an_advancing_tip_do_not_alert     : O1 x P2 x I3
//!     d7_unanimous_fleet_stall_does_not_alert            : O1 x P4 x I4
//!     d7_refusals_thinner_than_the_window_do_not_alert   : O1,O5 x P3 x I5
//!     d7_alarm_clears_when_the_tip_advances_again        : O1 x P5 x I6
//!
//! COMPILE-RED: `doli_node::node::wedge_alarm` does not exist yet. The types below
//! ARE the contract; signatures are in docs/.workflow/inc-i-204-M0-test-plan.md.

use doli_node::node::wedge_alarm::{WedgeAlarm, WedgeAlarmConfig, WedgeSample, WedgeVerdict};

/// Five minutes. "Within minutes" is a property of the CONFIG, not of the test.
const WINDOW_SECS: u64 = 300;
/// Three refusals inside one window. INC-I-204's slowest node still cleared this
/// bar many times over inside any 5-minute slice of its multi-day wedge.
const MIN_REFUSALS: u64 = 3;
/// The node polls its own health once a slot (10 s) in `run_periodic_tasks`.
const SAMPLE_INTERVAL_SECS: u64 = 10;

fn cfg() -> WedgeAlarmConfig {
    WedgeAlarmConfig {
        window_secs: WINDOW_SECS,
        min_refusals_in_window: MIN_REFUSALS,
    }
}

/// One sample. `refusals_total` is CUMULATIVE, as a Prometheus counter is; the
/// alarm is responsible for differencing it inside the window.
fn sample(
    at_secs: u64,
    tip_height: u64,
    refusals_total: u64,
    unique_chain_tips: usize,
    best_peer_height: u64,
) -> WedgeSample {
    WedgeSample {
        at_secs,
        tip_height,
        refusals_total,
        unique_chain_tips,
        best_peer_height,
    }
}

fn is_wedged(v: &WedgeVerdict) -> bool {
    matches!(v, WedgeVerdict::Wedged { .. })
}

// ==================== I1 — the replay ====================

/// REQ-FORK-016 — Decision: this IS the milestone gate. A failure means the exact
/// INC-I-204 signature can recur and still take a human 7 days to notice.
#[test]
fn d7_incident_signature_alerts_within_one_window() {
    let mut alarm = WedgeAlarm::new(cfg());

    // The measured shape: our tip pinned at 43_100 while peers climb, refusals
    // accumulating, two tips in the fleet.
    const STALE_TIP: u64 = 43_100;
    let mut refusals = 0u64;
    let mut fired_at: Option<u64> = None;
    let mut verdict = WedgeVerdict::Clear;

    for i in 0..=(WINDOW_SECS / SAMPLE_INTERVAL_SECS) {
        let at = i * SAMPLE_INTERVAL_SECS;
        // One refusal every other sample — far slower than n17's real rate.
        if i % 2 == 1 {
            refusals += 1;
        }
        verdict = alarm.observe(sample(at, STALE_TIP, refusals, 2, STALE_TIP + i));
        if is_wedged(&verdict) && fired_at.is_none() {
            fired_at = Some(at);
        }
    }

    let fired_at = fired_at.expect(
        "O1/P1: the INC-I-204 signature (stalled tip + accumulating refusals + \
         sustained unique_chain_tips > 1) must raise the alarm",
    );
    assert!(
        fired_at <= WINDOW_SECS,
        "O1: the alarm must fire within one evaluation window ({WINDOW_SECS}s), not \
         after days. Fired at {fired_at}s."
    );

    match verdict {
        WedgeVerdict::Wedged {
            stalled_secs,
            refusals_in_window,
            unique_chain_tips,
        } => {
            assert!(
                stalled_secs >= WINDOW_SECS,
                "O2: the verdict must report how long the tip has been stalled"
            );
            assert!(
                refusals_in_window >= MIN_REFUSALS,
                "O3: the verdict must report the IN-WINDOW refusal count, so the \
                 operator sees a rate and not a since-boot total"
            );
            assert_eq!(unique_chain_tips, 2, "O4: the fleet-divergence witness");
        }
        WedgeVerdict::Clear => unreachable!("checked above"),
    }
}

// ==================== I2-I5 — the controls ====================

/// REQ-FORK-016 — Decision: a detector that always fires is not a detector; a
/// failure here means the alarm would page on every healthy node and be muted
/// within a week.
#[test]
fn d7_healthy_control_never_alerts() {
    let mut alarm = WedgeAlarm::new(cfg());
    for i in 0..=(2 * WINDOW_SECS / SAMPLE_INTERVAL_SECS) {
        let at = i * SAMPLE_INTERVAL_SECS;
        let height = 43_100 + i;
        assert!(
            !is_wedged(&alarm.observe(sample(at, height, 0, 1, height))),
            "O1/P2/I2: a node advancing in step with a unanimous fleet must never \
             be called wedged (sample at {at}s)"
        );
    }
}

/// REQ-FORK-016 — Decision: refusals alone are normal traffic (LB-1: the guard
/// refusing is CORRECT); a failure means every node that legitimately refuses a
/// sub-finality reorg gets paged as wedged.
#[test]
fn d7_refusals_with_an_advancing_tip_do_not_alert() {
    let mut alarm = WedgeAlarm::new(cfg());
    let mut refusals = 0u64;
    for i in 0..=(2 * WINDOW_SECS / SAMPLE_INTERVAL_SECS) {
        let at = i * SAMPLE_INTERVAL_SECS;
        let height = 43_100 + i;
        refusals += 1;
        assert!(
            !is_wedged(&alarm.observe(sample(at, height, refusals, 2, height + 1))),
            "O1/P2/I3: a node that refuses reorgs but keeps ADVANCING is healthy — \
             the refusal is the guard doing its job (sample at {at}s)"
        );
    }
}

/// REQ-FORK-016 — Decision: a stalled tip with a unanimous fleet is a
/// whole-network halt, not a wedge; a failure means the alarm mislabels the
/// incident class and sends the operator down the wrong runbook.
#[test]
fn d7_unanimous_fleet_stall_does_not_alert() {
    let mut alarm = WedgeAlarm::new(cfg());
    let mut refusals = 0u64;
    for i in 0..=(2 * WINDOW_SECS / SAMPLE_INTERVAL_SECS) {
        let at = i * SAMPLE_INTERVAL_SECS;
        refusals += 1;
        assert!(
            !is_wedged(&alarm.observe(sample(at, 43_100, refusals, 1, 43_100))),
            "O1/P4/I4: unique_chain_tips == 1 means nobody is ahead of us — this \
             alarm must not claim a fleet-wide halt (sample at {at}s)"
        );
    }
}

/// REQ-FORK-016 — Decision: this is what makes the window BOUNDED rather than
/// cumulative; a failure means the alarm degenerates into "have we ever refused",
/// which fires once and then never says anything again.
#[test]
fn d7_refusals_thinner_than_the_window_do_not_alert() {
    let mut alarm = WedgeAlarm::new(cfg());
    let mut refusals = 0u64;

    // One refusal per window - 1 below the MIN_REFUSALS=3 threshold, sustained
    // over many windows so the CUMULATIVE total climbs far past the threshold.
    for i in 0..30u64 {
        let at = i * WINDOW_SECS;
        refusals += 1;
        let v = alarm.observe(sample(at, 43_100, refusals, 2, 43_100 + i));
        assert!(
            !is_wedged(&v),
            "O1/O5/P3/I5: cumulative refusals are {refusals} but only 1 falls inside \
             the {WINDOW_SECS}s window. A rolling window must not accumulate \
             forever (sample at {at}s)."
        );
    }
}

/// REQ-FORK-016 — Decision: `checkpoint_health` returns `(0, 0, 0)` when the node
/// has no peers, so an isolated node's gauge reads 0, not 1. Zero is NO EVIDENCE:
/// a peerless node cannot witness a fork, and paging on it would page every node
/// during a network outage — a failure here means the detector fires on isolation.
///
/// Developer-added cell (test plan section 5.2 left `tips == 0` uncovered).
#[test]
fn d7_isolated_node_with_zero_tips_never_alerts() {
    let mut alarm = WedgeAlarm::new(cfg());
    let mut refusals = 0u64;
    for i in 0..=(2 * WINDOW_SECS / SAMPLE_INTERVAL_SECS) {
        let at = i * SAMPLE_INTERVAL_SECS;
        refusals += 1;
        assert!(
            !is_wedged(&alarm.observe(sample(at, 43_100, refusals, 0, 0))),
            "O1/I7: unique_chain_tips == 0 is an ISOLATED node (peer_count == 0), \
             which is no evidence of a fork — not `fewer than one tip` (sample at {at}s)"
        );
    }
}

/// REQ-FORK-016 — Decision: an alarm that latches cannot report recovery, so an
/// operator cannot tell a fixed node from a still-broken one.
#[test]
fn d7_alarm_clears_when_the_tip_advances_again() {
    let mut alarm = WedgeAlarm::new(cfg());
    const STALE_TIP: u64 = 43_100;
    let mut refusals = 0u64;
    let mut at = 0u64;

    for i in 0..=(WINDOW_SECS / SAMPLE_INTERVAL_SECS) {
        at = i * SAMPLE_INTERVAL_SECS;
        refusals += 1;
        alarm.observe(sample(at, STALE_TIP, refusals, 2, STALE_TIP + i));
    }
    assert!(
        is_wedged(&alarm.observe(sample(at, STALE_TIP, refusals, 2, STALE_TIP + 99))),
        "fixture: the alarm must be firing before recovery can be observed"
    );

    // The escape lands: the tip moves and the fleet converges.
    let mut height = STALE_TIP;
    for i in 1..=(WINDOW_SECS / SAMPLE_INTERVAL_SECS) {
        at += SAMPLE_INTERVAL_SECS;
        height += 1;
        let v = alarm.observe(sample(at, height, refusals, 1, height));
        if i == WINDOW_SECS / SAMPLE_INTERVAL_SECS {
            assert!(
                !is_wedged(&v),
                "O1/P5/I6: once the tip advances and the fleet converges the alarm \
                 must clear — a latched alarm reports nothing about recovery"
            );
        }
    }
}
