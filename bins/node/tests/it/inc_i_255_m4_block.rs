//! INC-I-255 M4 — a block that holds a SlashProducer for p and a
//! RequestWithdrawal spending p's Bond UTXOs is refused by block validation,
//! before apply, in every mode and in either order.
//!
// covers: slash_state, block, assembly, slash_admission, pool, slash_withdrawal, slash_evidence, entry, mod, lib, protocol, security_model
//
// OUTPUT CONTRACT: fn Node::validate_block_for_apply(&self, &Block, u64, ValidationMode) -> Result<(), ValidationError>
//   O1 return (InvalidSlash message) per mode
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return; O2 chain tip; O3 producer set + pending queue; O4 UTXO set and supply
// PATHS: P-ON devnet slash gate (0) | P-OFF child process with the gate env-raised far above the tip
// MATRIX
//   validate O1 x P-ON x Full x {slash-first, wd-first} -> InvalidSlash(M4)  (probe_1, probe_2)
//   apply O1-O4 x P-ON x Full x {slash-first, wd-first} -> Err(M4), whole state unchanged (probe_1, probe_2)
//   validate O1 x P-ON x {Light, Replay} x both orders -> the Full message (probe_3)
//   apply O1-O4 x P-ON x {Light, Replay} x both orders on fresh nodes -> Err(M4), state unchanged (probe_3)
//   validate, apply O1 x P-OFF x 3 modes x both orders -> pinned pre-fix verdict: validate Ok x3,
//     apply Full/Light Err(slash insufficient fee), apply Replay Ok; never M4 (below_gate)
//   validate O1 x P-ON x 3 modes x other producer q x both orders -> Ok; apply Full -> Ok, slash recorded (other_producer)
// INPUT PARTITIONS:
//   gate: at/above (devnet default) | below (env-raised)
//   order: slash-first | wd-first
//   withdrawing producer: same as the accused | different producer
//   mode: Full | Light | Replay

use crypto::KeyPair;
use doli_core::validation::ValidationMode;

use super::inc_i_255_m1_common::{
    apply_slash_msg, block_slash_msg, in_child, run_in_child, slash_recorded, tip, MODES,
};
use super::inc_i_255_m2_common::{assert_same, live, snap, Snap};
use super::inc_i_255_m4_common::*;

const BELOW: u64 = 1_000_000_000;
const MOD: &str = "inc_i_255_m4_block::";

async fn refused_before_apply(slash_first: bool) {
    let mut r = m4(FEE_TIE).await;
    let wd = r.wd.clone();
    let block = pair_block(&mut r, &wd, slash_first).await;
    let height = tip(&r.w.node).await.0 + 1;
    let verdict =
        r.w.node
            .validate_block_for_apply(&block, height, ValidationMode::Full)
            .await;
    let before = snap(&r.w.node).await;
    let applied = r.w.node.apply_block(block, ValidationMode::Full).await;
    let after = snap(&r.w.node).await;
    let order = order_name(slash_first);
    assert!(
        block_slash_msg(&verdict).is_some_and(|m| m.contains(M4_MSG)),
        "block {order} must be refused by Full block validation with InvalidSlash({M4_MSG:?}); \
         validate got {verdict:?}; apply got {}",
        err_text(&applied)
    );
    assert!(
        apply_slash_msg(&applied).is_some_and(|m| m.contains(M4_MSG)),
        "apply {order} must refuse before the tx loop with the M4 InvalidSlash; got {}",
        err_text(&applied)
    );
    assert_same(&after, &before, &format!("refused {order}"));
}

// REQ-SLASH-003 — Decision: a failure means a block with Slash(p) before RW(p) passes validation and half-applies (phantom queued slash, missed slot).
#[tokio::test]
async fn inc_i_255_m4_probe_1_block_slash_first_rejected() {
    refused_before_apply(true).await;
}

// REQ-SLASH-003 — Decision: a failure means p withdraws its bond in the same block as its slash, before the burn.
#[tokio::test]
async fn inc_i_255_m4_probe_2_block_wd_first_rejected() {
    refused_before_apply(false).await;
}

// REQ-SLASH-002 — Decision: a failure means a syncing (Light) or replaying node reaches another verdict than a live node on the same pair block, a chain split.
#[tokio::test]
async fn inc_i_255_m4_probe_3_mode_parity() {
    let (reporter, accused) = (KeyPair::generate(), KeyPair::generate());
    let mut seen: Vec<(String, Option<String>, String)> = Vec::new();
    let mut states: Vec<(String, Snap, Snap)> = Vec::new();
    for slash_first in [true, false] {
        let order = order_name(slash_first);
        let mut r = m4_with(&reporter, &accused, FEE_TIE).await;
        let wd = r.wd.clone();
        let block = pair_block(&mut r, &wd, slash_first).await;
        let height = tip(&r.w.node).await.0 + 1;
        for mode in MODES {
            let v =
                r.w.node
                    .validate_block_for_apply(&block, height, mode)
                    .await;
            seen.push((
                format!("validate {mode:?} {order}"),
                block_slash_msg(&v),
                format!("{v:?}"),
            ));
        }
        for mode in [ValidationMode::Light, ValidationMode::Replay] {
            let mut fresh = m4_with(&reporter, &accused, FEE_TIE).await;
            let before = snap(&fresh.w.node).await;
            let a = fresh.w.node.apply_block(block.clone(), mode).await;
            let label = format!("apply {mode:?} {order}");
            seen.push((label.clone(), apply_slash_msg(&a), err_text(&a)));
            states.push((label, before, snap(&fresh.w.node).await));
        }
    }
    let missed: Vec<(&String, &String)> = seen
        .iter()
        .filter(|(_, msg, _)| !msg.as_deref().is_some_and(|m| m.contains(M4_MSG)))
        .map(|(path, _, raw)| (path, raw))
        .collect();
    assert!(
        missed.is_empty(),
        "every mode and order must refuse with InvalidSlash({M4_MSG:?}); not refused: {missed:#?}"
    );
    let full = seen[0].1.clone();
    for (path, msg, _) in &seen {
        assert_eq!(msg, &full, "{path}: message differs from Full's");
    }
    for (label, before, after) in &states {
        assert_same(after, before, label);
    }
}

/// Observed verdict of one pair block below the gate: validate per mode, then
/// apply per mode on fresh nodes.
async fn below_gate_verdicts(slash_first: bool) -> (Vec<(String, String)>, String) {
    let (reporter, accused) = (KeyPair::generate(), KeyPair::generate());
    let mut r = m4_with(&reporter, &accused, FEE_TIE).await;
    let wd = r.wd.clone();
    let block = pair_block(&mut r, &wd, slash_first).await;
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
        let mut fresh = m4_with(&reporter, &accused, FEE_TIE).await;
        let a = fresh.w.node.apply_block(block.clone(), mode).await;
        out.push((format!("apply {mode:?}"), err_text(&a)));
    }
    (out, r.slash.hash().to_string())
}

/// The verdict the untouched tree reaches below the gate: validation passes in
/// every mode; Full and Light apply refuse the fee-0 slash; Replay applies.
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

// REQ-SLASH-003 — Decision: a failure means the M4 rule changed a verdict below the slashing gate, a retroactive consensus change.
#[tokio::test]
async fn inc_i_255_m4_below_gate_pair_block_verdict_unchanged() {
    if !in_child() {
        run_in_child(
            &format!("{MOD}inc_i_255_m4_below_gate_pair_block_verdict_unchanged"),
            BELOW,
        );
        return;
    }
    for slash_first in [true, false] {
        let (got, slash_id) = below_gate_verdicts(slash_first).await;
        let order = order_name(slash_first);
        for (path, v) in &got {
            assert!(
                !v.contains(M4_MSG),
                "below the gate {path} {order} must not carry the M4 rule; got {v}"
            );
        }
        assert_eq!(
            got,
            pre_fix_verdict(&slash_id),
            "below the gate {order}: verdicts must equal the pre-fix tree"
        );
    }
}

// REQ-SLASH-003 — Decision: a failure means the M4 rule over-reaches and refuses an unrelated producer's withdrawal mined beside a slash.
#[tokio::test]
async fn inc_i_255_m4_other_producer_withdrawal_with_slash_accepted() {
    for slash_first in [true, false] {
        let label = if slash_first {
            "[Slash(p), RW(q)]"
        } else {
            "[RW(q), Slash(p)]"
        };
        let mut r = m4(FEE_TIE).await;
        let other = r.other_wd.clone();
        let block = pair_block(&mut r, &other, slash_first).await;
        let height = tip(&r.w.node).await.0 + 1;
        for mode in MODES {
            let v =
                r.w.node
                    .validate_block_for_apply(&block, height, mode)
                    .await;
            assert!(
                v.is_ok(),
                "{label}: validate {mode:?} must accept; got {v:?}"
            );
        }
        let q_bond = other.inputs[0].prev_tx_hash;
        let q_op = storage::Outpoint::new(q_bond, other.inputs[0].output_index);
        let a =
            r.w.node
                .apply_block(block.clone(), ValidationMode::Full)
                .await;
        assert!(
            a.is_ok(),
            "{label}: Full apply must accept; got {}",
            err_text(&a)
        );
        assert_eq!(tip(&r.w.node).await, (height, block.hash()), "{label}: tip");
        assert!(
            slash_recorded(&r.w.node, &r.w.accused_pk()).await,
            "{label}: the slash must be recorded"
        );
        assert!(
            live(&r.w.node, &[q_op]).await.is_empty(),
            "{label}: q's withdrawal must spend q's bond"
        );
    }
}
