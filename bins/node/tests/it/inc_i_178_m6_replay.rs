//! INC-I-178 M6 — REQ-BLS-009 (Must) / C17 / P5: the measured qualifier delta
//! between the two bit semantics, over a healthy AND a degraded epoch.
//!
//! This is the acceptance evidence for the whole redesign. Assumption 1 of the analysis
//! ("redefining a bit is acceptable") was recorded as conditional on a MEASURED delta,
//! and disclosure 1 ("whether the minute-boundary edge effects are benign") was recorded
//! as unresolved. These tests resolve both, or they fail.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1: `replay_epoch(&ReplayEpoch) -> ReplayReport` (harness; drives the shipped
//!       builder rule, `EpochState::accumulate_block`, `Node::calculate_epoch_rewards`
//!       and `EpochState::derive_at_boundary`)
//!       O1 `reward_qualifiers` per semantics — who is paid
//!       O2 `demotion_survivors` per semantics — who keeps a production slot
//!       O3 `reward_minutes` per producer — the minute count behind O1
//!       O4 `accum_minutes` per producer — the minute count behind O2
//!       O5 `blocks_with_differing_bitfield` / `blocks_with_differing_presence_root` —
//!          the anti-vacuity instrument: proof the two runs were really two rules
//!       O6 mutable params / statics / channels — the harness owns its nodes and temp
//!          dirs; process-global metric writes are serialised by `counter_lock`
//!       PATHS: P-healthy (full attendance), P-degraded (four failure patterns).
//!       INPUT PARTITIONS per producer: I-full, I-half, I-late, I-brink, I-silent.
//!
//!   F2: `load_epoch_fixture(&Path) -> ReplayEpoch`
//!       O7 return value — the parsed capture
//!       O8 panic on a malformed capture (asserted structurally by the in-range checks
//!          the loader performs on the shipped fixture)
//!
//!   MATRIX: O1/O2/O5 on both paths; O3/O4 on P-degraded across all five partitions;
//!     O7 on the shipped fixture.
//!
//! COUNTER HAZARD (M5 lesson). `replay_epoch` drives the real builder thousands of times
//! and writes process-global metrics, so every test here holds `counter_lock()` for its
//! whole body.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use doli_core::attestation::{
    attestation_minute, attestation_minutes_per_epoch, attestation_qualification_threshold,
};
use doli_core::consensus::ACTIVE_PRODUCERS_CAP;
use doli_core::Network;

use crate::inc_i_178_m5_common::counter_lock;
use crate::inc_i_178_m6_replay_fixture::load_epoch_fixture;
use crate::inc_i_178_m6_replay_harness::{
    degraded_epoch, healthy_epoch, replay_epoch, replay_producer_count, BitSemantics,
    QualifierThreshold, ReplayReport, DEGRADED_ROLES, FIXTURE_FORMAT,
};

/// The epoch geometry the harness replays: mainnet-shaped, so 60 attestation minutes and
/// a 54-minute threshold actually exist. Read from the shipped params, never spelled.
fn blocks_per_epoch() -> u64 {
    Network::Mainnet.blocks_per_reward_epoch()
}

/// An epoch strictly after epoch 1 — `derive_at_boundary` short-circuits at `epoch <= 1`
/// and `calculate_epoch_rewards` auto-qualifies everyone at epoch 0.
const REPLAY_EPOCH: u64 = 2;

fn expect_no_delta(report: &ReplayReport, threshold: QualifierThreshold) {
    let delta = report.symmetric_difference(threshold);
    assert!(
        delta.is_empty(),
        "{:?}: the qualifier set must not depend on the bit semantics; {} producer(s) \
         differ: {:?}\n{}",
        threshold,
        delta.len(),
        delta,
        report.summary()
    );
}

// ===========================================================================
// P-healthy — the baseline claim of REQ-BLS-009 AC-1.
// ===========================================================================

/// REQ-BLS-009 (Must) — Decision: a failure means the semantic switch changes who gets
/// paid and who keeps a production slot even when EVERY producer attests EVERY parent.
/// There would be no operating point at which the switch is safe, and the activation
/// height could never be pinned — the milestone would have to be abandoned rather than
/// tuned. A delta here is also the only signal that separates "the new rule is different"
/// from "the new rule is broken".
#[tokio::test]
async fn req_bls_009_m6_a_healthy_epoch_has_no_qualifier_delta_at_either_threshold() {
    let _guard = counter_lock().await;
    let bpe = blocks_per_epoch();
    let n = replay_producer_count();
    let report = replay_epoch(&healthy_epoch(n, REPLAY_EPOCH, bpe)).await;
    println!("{}", report.summary());

    // Anti-vacuity: the two runs must really have been two different rules. The bits
    // agree (that is what healthy means), but the COMMITMENT does not — pre-AH the root
    // is BLAKE3(bitfield), post-AH it binds the aggregate too.
    assert_eq!(
        report.blocks_with_differing_bitfield, 0,
        "healthy: full attendance under both rules must set the same bits"
    );
    assert_eq!(
        report.blocks_with_differing_presence_root, report.block_count,
        "anti-vacuity: every block's presence_root must differ across the two runs, or \
         the harness ran the same rule twice and the empty delta is meaningless"
    );

    for threshold in QualifierThreshold::BOTH {
        expect_no_delta(&report, threshold);
    }

    // Anti-vacuity on the sets themselves: an empty delta between two EMPTY sets would
    // also pass.
    for semantics in BitSemantics::BOTH {
        assert_eq!(
            report
                .qualifiers(semantics, QualifierThreshold::Reward54)
                .len(),
            n,
            "{semantics:?}: every producer attended every minute and must be paid"
        );
        assert_eq!(
            report
                .qualifiers(semantics, QualifierThreshold::Demotion30)
                .len(),
            ACTIVE_PRODUCERS_CAP,
            "{semantics:?}: with one producer more than the cap, the tier branch must run \
             and hand back exactly the cap — if it hands back everyone, the branch that \
             consults MIN_ATTESTATION_MINUTES was never entered and Demotion30 is untested"
        );
    }
}

// ===========================================================================
// P-degraded — C17, and the answer to disclosure 1.
// ===========================================================================

/// REQ-BLS-009 (Must) / C17 — Decision: a failure means the delta on a degraded epoch is
/// not the one bounded case this milestone claims it is, and the activation height cannot
/// be pinned on the strength of the healthy-epoch replay alone.
///
/// WHAT THIS MEASURES, AND THE BOUND. The reward qualifier unions MINUTES, so shifting a
/// bit from block S to block S+1 is invisible unless the shift crosses a minute boundary
/// AND that minute has no other attestation. Four patterns pin the whole space:
///
/// | role   | pattern                                   | pre-AH minutes | post-AH minutes | delta |
/// |--------|-------------------------------------------|----------------|-----------------|-------|
/// | half   | attests every other parent                | all            | all             | none  |
/// | late   | attests only the minute's LAST slot        | all            | all             | none  |
/// | brink  | as `late`, from minute `M - T` onward      | exactly T      | T - 1           | DROPS |
/// | silent | never attests                             | 0              | 0               | none  |
///
/// `M` is `attestation_minutes_per_epoch`, `T` the 90 % threshold. The BOUND: the new
/// semantics costs a producer AT MOST ONE attestation minute per epoch, and only the
/// LAST one — the credit for the epoch's final minute lands in the first block of the
/// NEXT epoch. So the reward qualifier can only change for a producer sitting EXACTLY on
/// the threshold. `late` is the direct answer to analysis disclosure 1 ("one bit instead
/// of six"): it loses five sixths of its bits and none of its minutes.
#[tokio::test]
async fn req_bls_009_m6_a_degraded_epoch_delta_is_exactly_the_threshold_edge_producer() {
    let _guard = counter_lock().await;
    let bpe = blocks_per_epoch();
    let n = replay_producer_count();
    let report = replay_epoch(&degraded_epoch(n, REPLAY_EPOCH, bpe)).await;
    println!("{}", report.summary());

    let minutes = attestation_minutes_per_epoch(bpe) as usize;
    let threshold = attestation_qualification_threshold(bpe) as usize;
    let r = DEGRADED_ROLES;

    // Anti-vacuity: the two rules must disagree about the BITS somewhere, or "degraded"
    // degraded nothing.
    assert!(
        report.blocks_with_differing_bitfield > 0,
        "the degraded patterns must produce different bitfields under the two rules\n{}",
        report.summary()
    );

    let pre = report.run(BitSemantics::PreAhMinuteUnion);
    let post = report.run(BitSemantics::PostAhParentAttestation);

    // O3 — the per-role minute counts, which is where the bound is visible.
    assert_eq!(
        (pre.reward_minutes[r.half], post.reward_minutes[r.half]),
        (minutes, minutes),
        "half: attesting every OTHER parent still touches every minute under both rules"
    );
    assert_eq!(
        (pre.reward_minutes[r.late], post.reward_minutes[r.late]),
        (minutes, minutes),
        "late: attesting ONLY the minute's last slot earns one bit instead of six, and \
         loses NO minute — the shifted credit lands in the next minute, which was empty. \
         This is the measured answer to analysis disclosure 1"
    );
    assert!(
        post.set_bits_total < pre.set_bits_total,
        "anti-vacuity for the line above: the post-AH run must carry strictly fewer set \
         bits ({} vs {}), or 'one bit instead of six' did not happen",
        post.set_bits_total,
        pre.set_bits_total
    );
    assert_eq!(
        (pre.reward_minutes[r.brink], post.reward_minutes[r.brink]),
        (threshold, threshold - 1),
        "brink: a producer sitting EXACTLY on the threshold loses its final minute, \
         because that credit lands in the first block of the NEXT epoch"
    );
    assert_eq!(
        (pre.reward_minutes[r.silent], post.reward_minutes[r.silent]),
        (0, 0),
        "silent: no attestation under either rule"
    );

    // O1 — the delta the milestone must own.
    assert_eq!(
        report.symmetric_difference(QualifierThreshold::Reward54),
        BTreeSet::from([r.brink]),
        "the reward qualifier delta must be EXACTLY the threshold-edge producer\n{}",
        report.summary()
    );
    assert!(
        pre.reward_qualifiers.contains(&r.brink),
        "brink must be paid under today's rule, or the delta is not a REGRESSION"
    );
    assert!(
        !post.reward_qualifiers.contains(&r.brink),
        "brink must lose its reward under the new rule — that is the whole finding"
    );
    assert!(
        !pre.reward_qualifiers.contains(&r.silent) && !post.reward_qualifiers.contains(&r.silent),
        "silent is unpaid under both rules; if it were paid the threshold never ran"
    );

    // O2 — the demotion decision is untouched: the 30-minute floor has 20+ minutes of
    // slack over a one-minute shift.
    expect_no_delta(&report, QualifierThreshold::Demotion30);
    for (label, run) in [("pre", pre), ("post", post)] {
        assert!(
            !run.demotion_survivors.contains(&r.silent),
            "{label}: the silent producer must be DEMOTED, or the MIN_ATTESTATION_MINUTES \
             retain never ran and the Demotion30 delta is vacuous"
        );
        for role in [r.half, r.late, r.brink] {
            assert!(
                run.demotion_survivors.contains(&role),
                "{label}: producer {role} keeps well over the 30-minute floor under both \
                 rules ({} accumulated minutes)",
                run.accum_minutes[role]
            );
        }
    }
}

// ===========================================================================
// F2 — the wire M7 fills with a real testnet capture.
// ===========================================================================

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
        .join("crates")
        .join("core")
        .join("tests")
        .join("fixtures")
        .join("inc_i_178_m6_epoch_replay.json")
}

/// REQ-BLS-009 (Must) — Decision: a failure means M7 cannot hand a live testnet capture to
/// this harness without editing it. The replay is the ONLY acceptance evidence for pinning
/// the activation height, and evidence that can only be produced by hand-editing the tool
/// that consumes it is not evidence. The loader is also the last line of defence against a
/// truncated capture being replayed as a short epoch and reported as "no delta".
#[test]
fn req_bls_009_m6_the_capture_format_round_trips_through_the_loader() {
    let spec = load_epoch_fixture(&fixture_path());

    assert_eq!(spec.label, "synthetic-two-minutes");
    assert!(
        spec.producer_count >= 2,
        "a capture needs at least two producers for a bit index to mean anything"
    );
    assert!(
        !spec.blocks.is_empty(),
        "the loader must reject an empty capture"
    );

    // Heights are contiguous and slots track them: the reward scan walks the window by
    // HEIGHT, so a capture with a hole would be replayed as an incomplete epoch.
    for (k, b) in spec.blocks.iter().enumerate() {
        assert_eq!(
            b.height,
            spec.blocks[0].height + k as u64,
            "block {k}: the capture must be contiguous in height"
        );
        assert!(
            b.producer < spec.producer_count,
            "block {k}: the producer index must be inside the universe"
        );
        for a in &b.attendance {
            assert!(
                a.minute <= attestation_minute(b.slot),
                "block {k}: the minute tracker cannot hold a FUTURE minute ({} > {})",
                a.minute,
                attestation_minute(b.slot)
            );
        }
    }

    // The two inputs must be independently expressible, or the capture cannot represent
    // a fleet where attendance and parent signatures disagree — which is the only shape
    // that produces a delta.
    let diverges = spec.blocks.iter().any(|b| {
        let minute = attestation_minute(b.slot);
        let attending: BTreeSet<usize> = b
            .attendance
            .iter()
            .filter(|a| a.minute == minute)
            .map(|a| a.attester)
            .collect();
        let pooled: BTreeSet<usize> = b.parent_attesters.iter().copied().collect();
        attending != pooled
    });
    assert!(
        diverges,
        "the shipped fixture must contain at least one block where minute attendance and \
         the parent-signature pool DISAGREE, or it cannot exercise the format that {FIXTURE_FORMAT} \
         exists to carry"
    );
}
