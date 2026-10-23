// covers: slash_state slash_window slash_evidence slash_admission pool unbonding entry error assembly tx_processing validation_checks mod lib block transaction types set_registration set_lifecycle defaults network_params
//! INC-I-256 M2 / R-4 — the evidence window: an offence at least
//! `unbonding_period` slots before the judged slot (parent slot + 1) is refused
//! with one error on mempool, builder, apply and replay; a younger one is
//! accepted on all four. Devnet child process, `unbonding_period` = 8.
//!
// OUTPUT CONTRACT: fn validate_slash_target(tx, ctx) -> Result<(), ValidationError>
//   O1/O2: none (pure; &Transaction, &ValidationContext)
//   O3: return — Ok | InvalidSlash(EXPIRED), observed through four callers:
//       mempool add_transaction (Ok/Err), builder selection (selected or not),
//       apply_block Light and Replay (Ok + tip moves / Err + tip unchanged)
// PATHS: P1 accepted | P2 refused EXPIRED
// INPUT PARTITIONS (age = parent slot + 1 - offence slot, P = 8, genesis accused, floor 0):
//   P1a dense, age 2 | P1b dense, age = P - 1 | P1c sparse tip at wall - 5, age = P - 1,
//       builder block slot >= parent + 2 (judged by parent, not by block slot)
//   P2a dense, age = P | P2b dense, age 11 | P2c one skipped run of slots, age 9, one block after the offence
// MATRIX: O3 x 4 paths x 6 partitions = 24 cells
//   P1a fresh_valid_accepted_on_every_path | P1b boundary_last_slot_accepted_on_every_path
//   P1c builder_at_later_slot_judges_by_parent_slot | P2a boundary_period_refused_on_every_path
//   P2b expired_refused_on_every_path | P2c expired_by_skipped_slots_refused_on_every_path

use super::inc_i_255_m1_common::wall_slot;
use super::inc_i_255_m2_common::{rig, Keys, Rig};
use super::inc_i_256_m2_common::*;
use super::inc_i_257_m1_fx::period;

const MOD: &str = "inc_i_256_m2_window::";
const P: u64 = 8;

fn run(test: &str) {
    child(&format!("{MOD}{test}"), &[(PERIOD_ENV, P.to_string())]);
}

async fn window_rig() -> Rig {
    let r = rig(&Keys::new()).await;
    assert_eq!(period(&r), P, "FIXTURE: child unbonding period");
    r
}

/// Accused is a genesis producer on devnet (gate 0): tenure = next height.
async fn assert_inside_tenure(r: &Rig, age: u64) {
    let (next, _) = judged(r).await;
    assert!(
        age < next,
        "FIXTURE: age {age} must be inside the tenure {next}"
    );
}

// R-4 / AUDIT-P2-539 — Decision: a failure means fresh, in-tenure evidence on a dense chain is refused on some path, so the window blocks honest punishment.
#[tokio::test]
async fn fresh_valid_accepted_on_every_path() {
    if !in_child() {
        return run("fresh_valid_accepted_on_every_path");
    }
    let mut r = window_rig().await;
    dense_to(&mut r, 12).await;
    let tx = slash_at(&r.keys.accused, 11);
    let age = age_at_next(&r, &tx).await;
    assert_eq!(age, 2, "FIXTURE: fresh age");
    assert_inside_tenure(&r, age).await;
    let p = four_paths(&mut r, &tx, 0).await;
    assert_accepted_everywhere("fresh valid", &p);
}

// R-4 / AUDIT-P2-539 — Decision: a failure means the last slot of the window (age = period - 1) is refused on some path (off-by-one toward over-refusal).
#[tokio::test]
async fn boundary_last_slot_accepted_on_every_path() {
    if !in_child() {
        return run("boundary_last_slot_accepted_on_every_path");
    }
    let mut r = window_rig().await;
    dense_to(&mut r, 12).await;
    let tx = slash_at(&r.keys.accused, 6);
    let age = age_at_next(&r, &tx).await;
    assert_eq!(age, P - 1, "FIXTURE: age = period - 1");
    assert_inside_tenure(&r, age).await;
    let p = four_paths(&mut r, &tx, 0).await;
    assert_accepted_everywhere("age = period - 1", &p);
}

// R-4 / AUDIT-P2-539 — Decision: a failure means evidence exactly one period old is still mineable on some path (off-by-one toward a late burn after the payout unlocks).
#[tokio::test]
async fn boundary_period_refused_on_every_path() {
    if !in_child() {
        return run("boundary_period_refused_on_every_path");
    }
    let mut r = window_rig().await;
    dense_to(&mut r, 12).await;
    let tx = slash_at(&r.keys.accused, 5);
    let age = age_at_next(&r, &tx).await;
    assert_eq!(age, P, "FIXTURE: age = period");
    assert_inside_tenure(&r, age).await;
    let p = four_paths(&mut r, &tx, 0).await;
    assert!(p.resident, "FIXTURE: the builder judged a resident slash");
    assert_refused_everywhere("age = period", &p, EXPIRED);
}

// R-4 / AUDIT-P2-539 — Decision: a failure means expired evidence is admitted, selected or mined on some path, so a slash can burn collateral after the window or the paths split.
#[tokio::test]
async fn expired_refused_on_every_path() {
    if !in_child() {
        return run("expired_refused_on_every_path");
    }
    let mut r = window_rig().await;
    dense_to(&mut r, 12).await;
    let tx = slash_at(&r.keys.accused, 2);
    let age = age_at_next(&r, &tx).await;
    assert_eq!(age, 11, "FIXTURE: expired age");
    assert_inside_tenure(&r, age).await;
    let p = four_paths(&mut r, &tx, 0).await;
    print_invalid_accepted("expired", &p);
    assert!(p.resident, "FIXTURE: the builder judged a resident slash");
    assert_refused_everywhere("expired", &p, EXPIRED);
}

// R-4 / AUDIT-P2-539 — Decision: a failure means the window counts blocks instead of slots, so evidence from a long slot gap with one block since stays mineable.
#[tokio::test]
async fn expired_by_skipped_slots_refused_on_every_path() {
    if !in_child() {
        return run("expired_by_skipped_slots_refused_on_every_path");
    }
    let mut r = window_rig().await;
    dense_to(&mut r, 9).await;
    block_at_slot(&mut r, 30).await;
    let tx = slash_at(&r.keys.accused, 22);
    let age = age_at_next(&r, &tx).await;
    assert_eq!(age, 9, "FIXTURE: slot age past the period");
    assert_inside_tenure(&r, age).await;
    let p = four_paths(&mut r, &tx, 0).await;
    assert!(p.resident, "FIXTURE: the builder judged a resident slash");
    assert_refused_everywhere("skipped slots", &p, EXPIRED);
}

// R-4 / AUDIT-P2-539 — Decision: a failure means the builder judges by its own block slot instead of the parent slot, so it drops a slash that apply accepts on the same parent.
#[tokio::test]
async fn builder_at_later_slot_judges_by_parent_slot() {
    if !in_child() {
        return run("builder_at_later_slot_judges_by_parent_slot");
    }
    let mut r = window_rig().await;
    dense_to(&mut r, 11).await;
    let parent_target = wall_slot(&r.node) - 5;
    block_at_slot(&mut r, parent_target).await;
    let (_, parent_slot, _) = r.best().await;
    let offence = parent_slot + 1 - (P as u32 - 1);
    let tx = slash_at(&r.keys.accused, offence);
    let age = age_at_next(&r, &tx).await;
    assert_eq!(age, P - 1, "FIXTURE: age = period - 1 at the parent");
    assert_inside_tenure(&r, age).await;
    let p = four_paths(&mut r, &tx, 4).await;
    println!(
        "builder block slot = parent slot + {}",
        p.builder_slot - parent_slot
    );
    assert!(
        p.builder_slot >= parent_slot + 2 && u64::from(p.builder_slot - offence) >= P,
        "FIXTURE: the builder block slot must be past the window for the offence; {p:?}"
    );
    assert_eq!(
        p.v[1].accepted(),
        p.v[2].accepted(),
        "builder at slot {} and apply on parent slot {parent_slot} disagree; {p:?}",
        p.builder_slot
    );
    assert_accepted_everywhere("builder at a later slot", &p);
}
