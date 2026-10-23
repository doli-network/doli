// covers: slash_state slash_window slash_evidence slash_admission pool unbonding entry error assembly tx_processing validation_checks mod lib block transaction types set_registration set_lifecycle defaults network_params
//! INC-I-256 M2 / R-4 tripwire — every path judges the evidence age with the
//! tip slot: one expired slash on one parent whose slot is far above the
//! offence is refused identically by mempool, builder, apply and replay. A path
//! that keeps `prev_slot` at 0 judges slot 1, sees age 0 and admits it.
//!
// OUTPUT CONTRACT: fn validate_slash_target(tx, ctx) -> Result<(), ValidationError>
//   O1/O2: none (pure; &Transaction, &ValidationContext)
//   O3: return — InvalidSlash(EXPIRED) on all four callers
// PATHS: P1 refused EXPIRED
// INPUT PARTITIONS: P1a tip slot 100_000, offence slot 4, default devnet period 60 — one partition:
//   the defect class is a ctx site with prev_slot = 0, and any offence slot >= 1 under a
//   large tip slot separates a wired site (expired) from an unwired one (age 0)
// MATRIX: O3 x 4 paths x 1 partition = 4 cells — tripwire_every_path_judges_with_the_tip_slot

use super::inc_i_255_m2_common::{rig, Keys};
use super::inc_i_256_m2_common::*;
use super::inc_i_257_m1_fx::period;

// R-4 / AUDIT-P2-539 — Decision: a failure names a path whose context still carries prev_slot 0 above the gate, so it admits or builds an expired slash that apply refuses (a poisoned block or a lost slot).
#[tokio::test]
async fn tripwire_every_path_judges_with_the_tip_slot() {
    if !in_child() {
        return child(
            "inc_i_256_m2_tripwire::tripwire_every_path_judges_with_the_tip_slot",
            &[],
        );
    }
    let mut r = rig(&Keys::new()).await;
    dense_to(&mut r, 5).await;
    block_at_slot(&mut r, 100_000).await;
    let tx = slash_at(&r.keys.accused, 4);
    let age = age_at_next(&r, &tx).await;
    assert!(
        age >= period(&r),
        "FIXTURE: age {age} past the period {}",
        period(&r)
    );
    let p = four_paths(&mut r, &tx, 0).await;
    let n = splits(&p);
    println!("B2M2_METRIC PATH_VERDICT_SPLITS={n}");
    assert!(p.resident, "FIXTURE: the builder judged a resident slash");
    assert_refused_everywhere("tip slot 100000", &p, EXPIRED);
    assert_eq!(n, 0, "paths split on one tx and one parent: {:?}", p.v);
}
