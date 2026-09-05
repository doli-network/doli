//! INC-I-178 M5 — REQ-BLS-007: the verifier is reachable from the LIVE apply
//! path AND observable from outside the process.
//!
//! OUTPUT CONTRACT
//!
//! F1: `metrics::register_metrics()` — the exposition an operator scrapes.
//!   O1 `doli_attestation_verify_total` family PRESENT
//!   O2 `doli_attestation_verify_rejected_total` PRESENT with all FOUR `reason`
//!      series zero-initialised (the INC-I-187 anti-regression: 28 of 57
//!      `doli_*` metrics in this binary are registered but never written, so
//!      "the family exists" is NOT evidence that anything writes it)
//!   O3 `doli_attestation_verify_skipped_light_total` family PRESENT
//!
//! F2: `Node::validate_block_for_apply(..)` — the writer.
//!   O4 an accepted post-AH block moves `verify_total` by exactly 1 and no
//!      reject series
//!   O5 the value read back out of the RENDERED exposition text (not out of the
//!      counter handle) moves too — a handle read passes on a metric that is
//!      registered but never COLLECTED
//!   O6 wall-clock cost of ONE post-AH `validate_block_for_apply` at N=45
//!
//!   PATHS: post-AH accept (P1+P3), post-AH accept at N=45.
//!   INPUT PARTITIONS: I1 honest body / N=12; I2 honest body / N=45.
//!   MATRIX: O1-O3 structural, asserted once; O4/O5 on I1; O6 on I2.
//!
//! COUNTER READS ARE DELTA-BASED and the exclusive-window tests here hold
//! `counter_lock()` across snapshot -> act -> read, because the `it` binary runs
//! these tests concurrently in ONE process against a global registry.

use std::time::Instant;

use crate::inc_i_178_m0_common::{err_text, N_REALISTIC, N_SMALL};
use crate::inc_i_178_m5_common::{counter_lock, post_ah_fixture, Counters, ALL_REASONS};
use crate::inc_i_204_m0_common::{encode_registry, exported_value};

const FAMILY_TOTAL: &str = "doli_attestation_verify_total";
const FAMILY_REJECTED: &str = "doli_attestation_verify_rejected_total";
const FAMILY_SKIPPED: &str = "doli_attestation_verify_skipped_light_total";
/// INC-I-178 M6 / C11: the fallback-rate series that must be registered beside them.
const FAMILY_FILL_RATIO: &str = "doli_attestation_bitfield_fill_ratio";

const N_SIGNERS: usize = 8;

// ===========================================================================
// O1-O3 + O5 — registered AND written.
// ===========================================================================

/// REQ-BLS-007 (Must) — Decision: a failure reproduces INC-I-187 for the one signal
/// that says whether consensus is verifying attestations at all. Two distinct ways to
/// fail, and this test separates them: the family is absent (an alert on it never
/// fires and never alarms — it silently evaluates nothing), or the family is present
/// but nothing ever writes it, which reads on a dashboard exactly like "zero forged
/// blocks" and is indistinguishable from "the verifier is not wired in".
#[tokio::test]
async fn req_bls_007_counters_are_registered_and_written() {
    let _guard = counter_lock().await;

    // Registered: the series must exist BEFORE the first post-AH block, or an
    // operator has nothing to alert on until a forgery already happened.
    let text = encode_registry();
    assert!(
        text.contains(FAMILY_TOTAL),
        "O1: {FAMILY_TOTAL} is not published by register_metrics()"
    );
    assert!(
        text.contains(FAMILY_SKIPPED),
        "O3: {FAMILY_SKIPPED} is not published by register_metrics()"
    );
    for reason in ALL_REASONS {
        assert!(
            exported_value(FAMILY_REJECTED, &[("reason", reason)]).is_some(),
            "O2: {FAMILY_REJECTED}{{reason=\"{reason}\"}} has no series. Zero-initialise \
             every label value in register_metrics() (the FORK_GUARD_REFUSALS pattern), \
             or an alert on an untouched reason has nothing to evaluate"
        );
    }

    // Written: the same series must MOVE for a real validation.
    let before_handle = Counters::now();
    let before_exported =
        exported_value(FAMILY_TOTAL, &[]).expect("O1: the total family must render a series");

    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let verdict = f.validate_full(&f.block).await;
    assert!(
        verdict.is_ok(),
        "the writer path must be the ACCEPT path here; got {:?}",
        err_text(&verdict)
    );

    let after_handle = Counters::now();
    let after_exported =
        exported_value(FAMILY_TOTAL, &[]).expect("O1: the total family must still render a series");

    assert_eq!(
        after_handle.delta(&before_handle).total,
        1,
        "O4: exactly one post-AH block reached the verifier in this window"
    );
    assert!(
        after_exported > before_exported,
        "O5: the value must move in the RENDERED exposition, not only on the handle. \
         Registered-but-never-collected is the INC-I-187 failure ({before_exported} -> \
         {after_exported})"
    );
}

/// REQ-BLS-010 (Must) — INC-I-178 M6 sibling. Decision: a failure means the C11 fallback
/// series joins the 28 `doli_*` metrics INC-I-187 found registered and never written. The
/// registry check has to live beside the verify counters because they are the SAME
/// exposition: an operator correlating "verifier ran" against "coverage collapsed" needs
/// both families present from process start, not from first write.
#[tokio::test]
async fn req_bls_010_m6_the_fill_ratio_family_is_registered_beside_the_verify_counters() {
    let _guard = counter_lock().await;
    let text = encode_registry();
    for family in [FAMILY_TOTAL, FAMILY_SKIPPED, FAMILY_FILL_RATIO] {
        assert!(
            text.contains(family),
            "{family} is not published by register_metrics()"
        );
    }
    assert!(
        exported_value(FAMILY_FILL_RATIO, &[]).is_some(),
        "{FAMILY_FILL_RATIO} must render a series from process start (the zero-initialised \
         FORK_GUARD_REFUSALS pattern), or an alert on attestation coverage evaluates \
         nothing until coverage has already collapsed"
    );
}

// ===========================================================================
// O4 — the honest post-AH block, built from REAL pooled signatures.
// ===========================================================================

/// REQ-BLS-007 (Must) — Decision: a failure means the verifier is unreachable from
/// the live apply path — either it was never called, or it was called somewhere that
/// gossip does not traverse. Every reject assertion in this milestone would then be
/// testing a function that production never runs, which is the most expensive way a
/// security control can pass its own tests.
#[tokio::test]
async fn req_bls_007_honest_post_ah_block_accepts_and_counts() {
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    assert_eq!(
        f.block.aggregate_bls_signature.len(),
        96,
        "anti-vacuity: the aggregate is real, built from the pooled signatures"
    );
    assert!(
        f.block.attestation_bitfield.iter().any(|b| *b != 0),
        "anti-vacuity: bits ARE set, so the pairing runs"
    );

    let before = Counters::now();
    let verdict = f.validate_full(&f.block).await;
    let after = Counters::now();
    let d = after.delta(&before);

    assert!(
        verdict.is_ok(),
        "an honest post-AH block must ACCEPT; got {:?}",
        err_text(&verdict)
    );
    assert_eq!(d.total, 1, "O4: verify_total +1");
    assert_eq!(d.rejected_total(), 0, "O4: no reject series moved");
    assert_eq!(
        d.skipped_light, 0,
        "O4: Full mode does not take the Light-skip branch"
    );
    assert_eq!(
        d.total,
        d.rejected_total() + d.skipped_light + 1,
        "the counter invariant: total == accepted + rejected + skipped_light"
    );
}

// ===========================================================================
// O6 — the measured cost that the commit's cost line reports.
// ===========================================================================

/// REQ-BLS-007 (Should) — Decision: a failure means one post-AH verification costs
/// more than a slot's error budget at a realistic producer count, which turns a
/// security control into a liveness hazard — every node pays it on every block, and
/// the ones that miss their slot get excluded for absence they did not choose. The
/// assertion is deliberately loose (a shared CI box is not a benchmark harness); the
/// PRINTED number, not the bound, is the observed basis for the commit's cost line.
#[tokio::test]
async fn req_bls_002_measured_verify_time_at_n45() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_REALISTIC, N_REALISTIC).await;

    assert!(
        f.block.attestation_bitfield.iter().any(|b| *b != 0),
        "anti-vacuity: N=45 attesters must actually be credited, or the timing is \
         of the empty path"
    );
    assert_eq!(
        f.block.aggregate_bls_signature.len(),
        96,
        "anti-vacuity: 45 signatures aggregate to ONE 96-byte signature"
    );

    // Warm the path once so the number is steady-state, not first-touch.
    let warm = f.validate_full(&f.block).await;
    assert!(
        warm.is_ok(),
        "the N=45 fixture must validate before it is timed; got {:?}",
        err_text(&warm)
    );

    let start = Instant::now();
    let verdict = f.validate_full(&f.block).await;
    let elapsed = start.elapsed();

    assert!(
        verdict.is_ok(),
        "the timed call must be the ACCEPT path; got {:?}",
        err_text(&verdict)
    );
    println!(
        "[M5_VERIFY_COST] n_attesters={} full_validate_block_for_apply_us={}",
        N_REALISTIC,
        elapsed.as_micros()
    );
    assert!(
        elapsed.as_millis() < 50,
        "one post-AH validation at N={} took {} ms; the design measured 869 us, so \
         anything near the 50 ms bound means the verifier is not O(1) in the \
         attester count",
        N_REALISTIC,
        elapsed.as_millis()
    );
}
