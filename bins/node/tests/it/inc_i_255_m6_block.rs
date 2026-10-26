//! INC-I-255 M6 — a block that holds a SlashProducer for p and any tx that
//! spends a Bond UTXO owned by p is refused before apply, in every mode and
//! in either order.
//!
// covers: slash_state, utxo, block, assembly, tx_processing, slash_burn, slash_admission, pool, slash_withdrawal, withdrawal_economics, slash_bond_spend, mod.rs, lib.rs, validation_checks, protocol, security_model
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return (error text); O2 chain tip; O3 producer set + pending queue (slash recorded?);
//   O4 UTXO set (p's Bond outpoint live?) and supply
// PATHS: P-ON devnet slash gate (0); block height 1; p's Bond lock_until 1 (expired at the block)
// MATRIX
//   O1-O4 x Full x [Slash(p), T(p)] -> Err(M6_MSG), state unchanged (probe_1)
//   O1-O4 x Full x [T(p), Slash(p)] -> Err(M6_MSG), state unchanged (probe_2)
//   O1-O4 x {Light, Replay} x both orders on fresh nodes -> Err text == Full's, state unchanged (probe_3)
//   O1 x Full x [A(q -> Bond of p), T(p spends A:0)] -> Ok (control, non-vacuity) (probe_8)
//   O1-O4 x Full x [A, Slash(p), T(p spends A:0)] -> Err(M6_MSG), state unchanged (probe_8)
// INPUT PARTITIONS:
//   order: slash-first | spend-first
//   spent Bond source: pre-block UTXO set | created earlier in the same block
//   mode: Full | Light | Replay
//   spender tx type: Transfer (not RequestWithdrawal)

use crypto::KeyPair;
use doli_core::validation::ValidationMode;
use storage::Outpoint;

use super::inc_i_255_m1_common::{in_child, run_in_child, slash_recorded, tip};
use super::inc_i_255_m2_common::{assert_same, Snap};
use super::inc_i_255_m6_common::*;

async fn refused_in_full(slash_first: bool) {
    let mut r = m6(FEE_TIE).await;
    let txs = ordered(&r.slash, &r.t, slash_first);
    let block = block_with(&mut r.w.node, &txs).await;
    let (h0, _) = tip(&r.w.node).await;
    let (got, before, after) = apply_observed(&mut r.w.node, block, ValidationMode::Full).await;
    let order = order_name(slash_first, "T(p)");
    assert!(
        got.contains(M6_MSG),
        "block {order} must be refused before apply with {M6_MSG:?}; apply got {got}"
    );
    assert_eq!(tip(&r.w.node).await.0, h0, "{order}: chain height moved");
    assert!(
        is_live(&r.w.node, r.p_bond).await,
        "{order}: p's Bond outpoint must stay unspent"
    );
    assert!(
        !slash_recorded(&r.w.node, &r.w.accused_pk()).await,
        "{order}: p must not be slashed or slash-queued"
    );
    assert_same(&after, &before, &format!("refused {order}"));
}

// REQ-SLASH-003 — Decision: a failure means [Slash(p), T(p)] passes validation and half-applies (phantom queued slash, missed slot).
#[tokio::test]
async fn inc_i_255_m6_probe_1_block_slash_first_rejected() {
    refused_in_full(true).await;
}

// REQ-SLASH-003 — Decision: a failure means p moves its expired bond in the slash block before the burn, escaping it.
#[tokio::test]
async fn inc_i_255_m6_probe_2_block_spend_first_rejected() {
    refused_in_full(false).await;
}

// REQ-SLASH-002 — Decision: a failure means a syncing (Light) or replaying node reaches another verdict than a live node on the same block, a chain split.
#[tokio::test]
async fn inc_i_255_m6_probe_3_mode_parity() {
    let (reporter, accused) = (KeyPair::generate(), KeyPair::generate());
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut states: Vec<(String, Snap, Snap)> = Vec::new();
    let mut mismatched: Vec<String> = Vec::new();
    for slash_first in [true, false] {
        let order = order_name(slash_first, "T(p)");
        let mut r = m6_with(&reporter, &accused, FEE_TIE).await;
        let txs = ordered(&r.slash, &r.t, slash_first);
        let block = block_with(&mut r.w.node, &txs).await;
        let mut texts = Vec::new();
        for mode in [
            ValidationMode::Full,
            ValidationMode::Light,
            ValidationMode::Replay,
        ] {
            let mut fresh = m6_with(&reporter, &accused, FEE_TIE).await;
            let (got, before, after) = apply_observed(&mut fresh.w.node, block.clone(), mode).await;
            let label = format!("apply {mode:?} {order}");
            seen.push((label.clone(), got.clone()));
            texts.push((label.clone(), got));
            states.push((label, before, after));
        }
        let full = texts[0].1.clone();
        mismatched.extend(
            texts
                .iter()
                .filter(|(_, t)| *t != full)
                .map(|(l, t)| format!("{l}: {t} (Full: {full})")),
        );
    }
    let missed: Vec<&(String, String)> = seen.iter().filter(|(_, t)| !t.contains(M6_MSG)).collect();
    assert!(
        missed.is_empty(),
        "every mode and order must refuse with {M6_MSG:?}; not refused: {missed:#?}"
    );
    assert!(
        mismatched.is_empty(),
        "error text differs from Full's: {mismatched:#?}"
    );
    for (label, before, after) in &states {
        assert_same(after, before, label);
    }
}

// REQ-SLASH-003 — Decision: a failure means a Bond for p created earlier in the slash block can be spent after the burn (half-apply) or escapes the in-block check.
#[tokio::test]
async fn inc_i_255_m6_probe_8_in_block_bond_spend_rejected() {
    if !in_child() {
        let test = "inc_i_255_m6_block::inc_i_255_m6_probe_8_in_block_bond_spend_rejected";
        return run_in_child(test, 0);
    }
    let unit;
    {
        let mut c = m6(FEE_TIE).await;
        unit = c.w.node.config.network.bond_unit();
        let coin = seed_coin(&c.w.node, c.w.reporter.public_key(), Q_COIN_TAG, unit).await;
        let a = bond_maker(&c.w.reporter, coin, unit, &c.w.accused_pk(), EXPIRED_LOCK);
        let t2 = transfer(
            &c.w.accused,
            Outpoint::new(a.hash(), 0),
            a.outputs[0].amount,
            FEE_TIE,
        );
        let block = block_with(&mut c.w.node, &[a, t2]).await;
        let (got, _, _) = apply_observed(&mut c.w.node, block, ValidationMode::Full).await;
        assert_eq!(
            got, "Ok",
            "NON-VACUITY: [A(Bond of p), T(p spends A:0)] without a slash applies"
        );
    }
    let mut r = m6(FEE_TIE).await;
    let coin = seed_coin(&r.w.node, r.w.reporter.public_key(), Q_COIN_TAG, unit).await;
    let a = bond_maker(&r.w.reporter, coin, unit, &r.w.accused_pk(), EXPIRED_LOCK);
    let a_bond = Outpoint::new(a.hash(), 0);
    let t2 = transfer(&r.w.accused, a_bond, a.outputs[0].amount, FEE_TIE);
    let txs = vec![a, r.slash.clone(), t2];
    let block = block_with(&mut r.w.node, &txs).await;
    let (h0, _) = tip(&r.w.node).await;
    let (got, before, after) = apply_observed(&mut r.w.node, block, ValidationMode::Full).await;
    assert!(
        got.contains(M6_MSG),
        "[A(Bond of p), Slash(p), T(p spends A:0)] must be refused before apply with {M6_MSG:?}; apply got {got}"
    );
    assert_eq!(tip(&r.w.node).await.0, h0, "chain height moved");
    assert!(
        !slash_recorded(&r.w.node, &r.w.accused_pk()).await,
        "p must not be slashed or slash-queued"
    );
    assert!(is_live(&r.w.node, coin).await, "q's coin must stay unspent");
    assert_same(&after, &before, "refused in-block chain");
}
