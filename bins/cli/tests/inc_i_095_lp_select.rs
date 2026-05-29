//! INC-I-095 — `pool remove` must select LP UTXOs belonging to the TARGET pool only.
//!
//! Bug: `cmd_pool_remove` selected the first `lpShare` UTXO with sufficient amount
//! without checking its embedded pool_id. With LP shares from multiple pools, input 1
//! became a foreign-pool LP UTXO and the node rejected the tx with [MPTX007].
//!
//! This test exercises the pure selection helper directly (no RPC/wallet).

#[path = "../src/lp_select.rs"]
mod lp_select;

use lp_select::{select_lp_share_utxos, LpCandidate};

const POOL_A: &str = "1b50ca0f152c072fc240bfa1030e623e96997d769c0d9cbbd881fb0b635ba479";
const POOL_B: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn lp(pool: &'static str, amount: u64, tx: &'static str, idx: u32) -> LpCandidate<'static> {
    LpCandidate {
        output_type: "lpShare",
        pool_id: Some(pool),
        amount,
        tx_hash: tx,
        output_index: idx,
    }
}

// OUTPUT CONTRACT: fn select_lp_share_utxos(candidates, target_pool_id, shares_to_burn) -> Result<Vec<&LpCandidate>>
// O1: Ok(selected) — every selected candidate has pool_id == Some(target) AND sum(amount) >= shares_to_burn
// O2: Err — insufficient LP shares FOR THE TARGET POOL (distinct message when only foreign-pool shares exist)
// PATHS: P1 only-target-pool sufficient, P2 mixed foreign+target (skip foreign), P3 only-foreign (Err),
//        P4 accumulate across multiple target utxos, P5 non-lpShare ignored
// INPUT PARTITIONS: candidate set composition × shares_to_burn
// MATRIX: 2 outputs x 5 paths

// P2 — THE REGRESSION: foreign-pool LP UTXO listed FIRST, target-pool second.
// Buggy selection picks the foreign one (input 1 mismatch -> MPTX007).
#[test]
fn inc_i_095_skips_foreign_pool_lp_utxo() {
    let candidates = vec![
        lp(POOL_B, 1000, "bbbb", 0), // foreign pool, listed first
        lp(POOL_A, 1000, "aaaa", 0), // target pool
    ];
    let selected = select_lp_share_utxos(&candidates, POOL_A, 100)
        .expect("sufficient target-pool LP shares available");
    assert!(!selected.is_empty(), "must select at least one LP UTXO");
    for c in &selected {
        assert_eq!(
            c.pool_id,
            Some(POOL_A),
            "selected LP UTXO must belong to the target pool, not {:?}",
            c.pool_id
        );
        assert_eq!(c.tx_hash, "aaaa", "must pick the target-pool UTXO");
    }
}

// P1 — single target pool, sufficient.
#[test]
fn inc_i_095_selects_target_pool_when_alone() {
    let candidates = vec![lp(POOL_A, 500, "aaaa", 0)];
    let selected = select_lp_share_utxos(&candidates, POOL_A, 300).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].pool_id, Some(POOL_A));
    assert_eq!(selected[0].output_index, 0);
}

// P3 — wallet holds ONLY foreign-pool LP shares -> Err (and message guides the user).
#[test]
fn inc_i_095_errors_when_only_foreign_pool_shares() {
    let candidates = vec![lp(POOL_B, 1000, "bbbb", 0)];
    let err = select_lp_share_utxos(&candidates, POOL_A, 100)
        .expect_err("must error: no LP shares for the target pool");
    let msg = err.to_string();
    assert!(
        msg.contains("other pool") || msg.contains("other pools"),
        "error should explain foreign-pool LP shares are blocking selection, got: {msg}"
    );
}

// P4 — accumulate across multiple target-pool UTXOs; ignore foreign ones in between.
#[test]
fn inc_i_095_accumulates_across_target_utxos() {
    let candidates = vec![
        lp(POOL_A, 40, "a1", 0),
        lp(POOL_B, 9999, "bbbb", 0), // foreign — must be skipped even though large
        lp(POOL_A, 40, "a2", 0),
        lp(POOL_A, 40, "a3", 0),
    ];
    let selected = select_lp_share_utxos(&candidates, POOL_A, 100).unwrap();
    let total: u64 = selected.iter().map(|c| c.amount).sum();
    assert!(total >= 100, "selected total {total} must cover 100");
    for c in &selected {
        assert_eq!(
            c.pool_id,
            Some(POOL_A),
            "no foreign-pool UTXO may be selected"
        );
    }
}

// P5 — non-lpShare UTXOs are ignored even if they carry a matching pool_id field.
#[test]
fn inc_i_095_ignores_non_lpshare() {
    let candidates = vec![
        LpCandidate {
            output_type: "normal",
            pool_id: Some(POOL_A),
            amount: 9999,
            tx_hash: "n1",
            output_index: 0,
        },
        lp(POOL_A, 100, "aaaa", 0),
    ];
    let selected = select_lp_share_utxos(&candidates, POOL_A, 100).unwrap();
    for c in &selected {
        assert_eq!(
            c.output_type, "lpShare",
            "only lpShare UTXOs are selectable"
        );
    }
}
