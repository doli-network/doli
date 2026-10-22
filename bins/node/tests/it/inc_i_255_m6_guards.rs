//! INC-I-255 M6 guards — the bond-spend rule does not over-reach: another
//! producer's bond spend, and p's non-Bond spend, stay valid beside Slash(p);
//! below the gate the verdict is the pre-fix one.
//!
// covers: slash_state, utxo, block, assembly, tx_processing, slash_burn, slash_admission, pool, slash_withdrawal, withdrawal_economics, slash_bond_spend, mod.rs, lib.rs, validation_checks, protocol, security_model
//
// OUTPUT CONTRACT: fn Node::validate_block_for_apply(&self, &Block, u64, ValidationMode) -> Result<(), ValidationError>
//   O1 return per mode
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return; O2 chain tip; O3 slash recorded; O4 spent / burned outpoints
// PATHS: P-ON devnet slash gate (0) | P-OFF child process, gate env-raised far above the tip
// MATRIX
//   validate O1 x P-ON x 3 modes x [Slash(p), T(q bond)] both orders -> Ok; apply Full -> Ok, tip moves,
//     slash recorded, q's bond spent (guard_other_producer_bond_spend)
//   validate O1 x P-ON x 3 modes x [Slash(p), T(p coin)] both orders -> Ok; apply Full -> Ok, tip moves,
//     slash recorded, p's coin spent, p's bond burned (guard_accused_non_bond_spend)
//   validate, apply O1 x P-OFF x 3 modes x [Slash(p), T(p bond)] both orders -> pinned pre-fix verdict (guard_below_gate)
// INPUT PARTITIONS:
//   gate: at/above (devnet default) | below (env-raised)
//   order: slash-first | spend-first
//   spent UTXO: Bond of another producer q | Normal UTXO of p | Bond of p (below gate only)
//   mode: Full | Light | Replay

use crypto::KeyPair;
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use storage::Outpoint;

use super::inc_i_255_m1_common::{in_child, run_in_child, slash_recorded, tip, MODES};
use super::inc_i_255_m4_common::err_text;
use super::inc_i_255_m6_common::*;

const BELOW: u64 = 1_000_000_000;
const COIN: u64 = 3_000_000;

async fn accepted(r: &mut M6, spend: &Transaction, other: &str, slash_first: bool) {
    let order = order_name(slash_first, other);
    let block = block_with(&mut r.w.node, &ordered(&r.slash, spend, slash_first)).await;
    let height = tip(&r.w.node).await.0 + 1;
    for mode in MODES {
        let v =
            r.w.node
                .validate_block_for_apply(&block, height, mode)
                .await;
        assert!(
            v.is_ok(),
            "{order}: validate {mode:?} must accept; got {v:?}"
        );
    }
    let a =
        r.w.node
            .apply_block(block.clone(), ValidationMode::Full)
            .await;
    assert!(
        a.is_ok(),
        "{order}: Full apply must accept; got {}",
        err_text(&a)
    );
    assert_eq!(tip(&r.w.node).await, (height, block.hash()), "{order}: tip");
    assert!(
        slash_recorded(&r.w.node, &r.w.accused_pk()).await,
        "{order}: the slash must be recorded"
    );
    let spent = Outpoint::new(spend.inputs[0].prev_tx_hash, spend.inputs[0].output_index);
    assert!(
        !is_live(&r.w.node, spent).await,
        "{order}: {other} must spend its input"
    );
}

// REQ-SLASH-003 — Decision: a failure means the fix refuses an unrelated producer's bond spend mined beside a slash.
#[tokio::test]
async fn inc_i_255_m6_guard_other_producer_bond_spend() {
    for slash_first in [true, false] {
        let mut r = m6(FEE_TIE).await;
        let tq = r.tq.clone();
        accepted(&mut r, &tq, "T(q bond)", slash_first).await;
    }
}

// REQ-SLASH-003 — Decision: a failure means the fix refuses p's spend of a non-Bond UTXO beside Slash(p), which the burn never touches.
#[tokio::test]
async fn inc_i_255_m6_guard_accused_non_bond_spend() {
    for slash_first in [true, false] {
        let mut r = m6(FEE_TIE).await;
        let coin = seed_coin(&r.w.node, &r.w.accused_pk(), P_COIN_TAG, COIN).await;
        let tc = transfer(&r.w.accused, coin, COIN, FEE_TIE);
        accepted(&mut r, &tc, "T(p coin)", slash_first).await;
        assert!(
            !is_live(&r.w.node, r.p_bond).await,
            "{}: the burn must still spend p's bond",
            order_name(slash_first, "T(p coin)")
        );
    }
}

async fn below_gate_verdicts(slash_first: bool) -> (Vec<(String, String)>, String) {
    let (reporter, accused) = (KeyPair::generate(), KeyPair::generate());
    let mut r = m6_with(&reporter, &accused, FEE_TIE).await;
    let block = block_with(&mut r.w.node, &ordered(&r.slash, &r.t, slash_first)).await;
    let height = tip(&r.w.node).await.0 + 1;
    let mut out = Vec::new();
    for mode in MODES {
        let v =
            r.w.node
                .validate_block_for_apply(&block, height, mode)
                .await;
        out.push((format!("validate {mode:?}"), format!("{v:?}")));
    }
    for mode in MODES {
        let mut fresh = m6_with(&reporter, &accused, FEE_TIE).await;
        let a = fresh.w.node.apply_block(block.clone(), mode).await;
        out.push((format!("apply {mode:?}"), err_text(&a)));
    }
    (out, r.slash.hash().to_string())
}

fn pre_fix_verdict(slash_id: &str) -> Vec<(String, String)> {
    let fee = format!(
        "Err(UTXO validation failed for tx {slash_id}: insufficient fee: got 0, minimum 1 (base 1 + 0 bytes * 1/byte))"
    );
    [
        ("validate Full", "Ok(())".to_string()),
        ("validate Light", "Ok(())".to_string()),
        ("validate Replay", "Ok(())".to_string()),
        ("apply Full", fee.clone()),
        ("apply Light", fee),
        ("apply Replay", "Ok".to_string()),
    ]
    .into_iter()
    .map(|(p, v)| (p.to_string(), v))
    .collect()
}

// REQ-SLASH-003 — Decision: a failure means the bond-spend rule changed a verdict below the slashing gate, a retroactive consensus change.
#[tokio::test]
async fn inc_i_255_m6_guard_below_gate_verdict_unchanged() {
    if !in_child() {
        run_in_child(
            "inc_i_255_m6_guards::inc_i_255_m6_guard_below_gate_verdict_unchanged",
            BELOW,
        );
        return;
    }
    for slash_first in [true, false] {
        let (got, slash_id) = below_gate_verdicts(slash_first).await;
        let order = order_name(slash_first, "T(p)");
        for (path, v) in &got {
            assert!(
                !v.contains(M6_MSG),
                "below the gate {path} {order} must not carry the M6 rule; got {v}"
            );
        }
        assert_eq!(
            got,
            pre_fix_verdict(&slash_id),
            "below the gate {order}: verdicts must equal the pre-fix tree"
        );
    }
}
