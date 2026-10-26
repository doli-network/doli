// covers: slash_state slash_window slash_evidence slash_admission pool unbonding entry error assembly tx_processing validation_checks mod lib block transaction types set_registration set_lifecycle defaults network_params
//! INC-I-256 M2 / R-4 — reuse: the same evidence pair, once mined, is refused
//! with the existing state error on mempool, builder, apply and replay while
//! the slash is queued (same epoch) and after the boundary marks the accused
//! Slashed. Devnet child process, default period.
//!
// OUTPUT CONTRACT: fn validate_slash_target(tx, ctx) -> Result<(), ValidationError>
//   O1/O2: none (pure; &Transaction, &ValidationContext)
//   O3: return — InvalidSlash(PENDING) | InvalidSlash(ALREADY_SLASHED), observed through
//       mempool add_transaction, builder selection, apply_block Light and Replay
// PATHS: P1 accused slash-queued | P2 accused Slashed
// INPUT PARTITIONS: the pair is fresh and inside the tenure in both (age < period, age < tenure),
//   so only the state decides: P1 queued in the pending updates | P2 status Slashed after the flush
// MATRIX: O3 x 4 paths x 2 partitions = 8 cells — reused_evidence_refused_on_every_path

use storage::ProducerStatus;

use super::inc_i_255_m2_common::{queued_slash, rig, status, Keys};
use super::inc_i_256_m2_common::*;
use super::inc_i_257_m2_common::mine;

// R-4 / AUDIT-P2-546 — Decision: a failure means one offence can be mined twice, or a terminally Slashed producer can be slashed again, on some path.
#[tokio::test]
async fn reused_evidence_refused_on_every_path() {
    if !in_child() {
        return child(
            "inc_i_256_m2_reuse::reused_evidence_refused_on_every_path",
            &[],
        );
    }
    let mut r = rig(&Keys::new()).await;
    let accused = r.accused_pk();
    dense_to(&mut r, 9).await;
    let tx = slash_at(&r.keys.accused, 9);
    mine(&mut r, vec![tx.clone()], "the first slash").await;
    assert!(
        queued_slash(&r.node, &accused).await,
        "FIXTURE: the mined slash is queued"
    );
    let queued = four_paths(&mut r, &tx, 0).await;
    assert_refused_everywhere("slash queued", &queued, PENDING);

    dense_to(&mut r, 12).await;
    let st = status(&r.node, &accused).await;
    assert!(
        matches!(st, Some(ProducerStatus::Slashed { .. })),
        "FIXTURE: the boundary flushed the slash; status {st:?}"
    );
    let age = age_at_next(&r, &tx).await;
    assert!(age < 60, "FIXTURE: age {age} inside the period");
    let slashed = four_paths(&mut r, &tx, 0).await;
    assert_refused_everywhere("accused Slashed", &slashed, ALREADY_SLASHED);
}
