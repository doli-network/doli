//! B3 M2.5 — REQ-B3-003 / REV-B3-001: apply, replay and the builder agree on a block with Slash(K) and Registration / AddBond for K.
//!
// covers: assembly, slash_state, slash_admission
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return: Err whose chain carries "[ERRTX071]" for a block holding Slash(K) and AddBond(K) / Registration(K),
//      either order, in Full (live), Light and Replay (fresh nodes); Ok for Slash(K) beside a tx for another key
//   O2 receiver chain tip: unchanged on Err, advanced on Ok
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 selected txs: with Slash(K) and X(K) both resident, the slash is selected and X(K) is not; the producer's own
//      apply of that block is Ok and records the slash
// PATHS: P-APPLY Full | P-REPLAY Light + Replay on fresh nodes | P-BUILD (child, slash gate 2: X(K) admitted as judged
//        below the gate, the only way it sits beside a resident Slash(K) once admission refuses the pair) | P-OTHER control
// INPUT PARTITIONS:
//   X: AddBond(K), K Active | Registration(K), K Exited with a locked Unbonding UTXO (both reach apply at HEAD)
//   order: slash first | X first; control: AddBond(reporter) | Registration(fresh key)
// MATRIX:
//   O1,O2 x P-APPLY x AddBond x both orders (+ P-OTHER) -> b3_m25_apply_refuses_slash_with_add_bond_same_key
//   O1,O2 x P-APPLY x Registration x both orders (+ P-OTHER) -> b3_m25_apply_refuses_slash_with_registration_same_key
//   O1,O2 x P-REPLAY x AddBond x both orders -> b3_m25_replay_refuses_slash_with_add_bond_same_key
//   O1,O2 x P-REPLAY x Registration x both orders -> b3_m25_replay_refuses_slash_with_registration_same_key
//   O1 x P-BUILD x AddBond -> b3_m25_builder_keeps_slash_drops_add_bond_same_key
//   O1 x P-BUILD x Registration -> b3_m25_builder_keeps_slash_drops_registration_same_key

use crypto::KeyPair;
use doli_core::block::Block;
use doli_core::transaction::{Transaction, TxType};
use doli_core::validation::ValidationMode;

use super::b3_m25_same_block_common::*;
use super::inc_i_255_m1_common::{
    full_block_with, in_child, run_in_child, slash_recorded, tip, valid_slash, world_with, World,
};
use super::inc_i_255_m4_common::{admit_at, err_text, resident};
use crate::inc_i_156_m1_harness::apply_plain_up_to;

const GATE: u64 = 2;
const X_TAG: u8 = 0x9B;
const CONTROL_TAG: u8 = 0x9C;
const COLLATERAL_TAG: u8 = 0x9A;

/// World whose accused K can carry `kind`: Exited with locked collateral for a Registration.
async fn world_for(kind: TxType, reporter: &KeyPair, accused: &KeyPair) -> (World, Transaction) {
    let w = world_with(reporter, accused).await;
    if kind == TxType::Registration {
        exit_with_collateral(&w.node, &w.accused_pk(), COLLATERAL_TAG).await;
    }
    let x = producer_tx(&w.node, kind, &w.accused, X_TAG).await;
    (w, x)
}

async fn pair_block(kind: TxType, keys: &(KeyPair, KeyPair), slash_first: bool) -> Block {
    let (mut w, x) = world_for(kind, &keys.0, &keys.1).await;
    let s = valid_slash(&w);
    slash_beside(&mut w.node, &x, &s, slash_first).await
}

async fn assert_refused(w: &mut World, block: Block, mode: ValidationMode, what: &str) {
    let before = tip(&w.node).await;
    let r = w.node.apply_block(block, mode).await;
    assert!(
        coded(&r),
        "{what}: apply {mode:?} must reject with {CODE}; got {}",
        err_text(&r)
    );
    assert_eq!(
        tip(&w.node).await,
        before,
        "{what}: a rejected block must not move the tip"
    );
}

async fn other_key_control(kind: TxType) {
    let (mut w, _) = world_for(kind, &KeyPair::generate(), &KeyPair::generate()).await;
    let other = if kind == TxType::AddBond {
        w.reporter.clone()
    } else {
        KeyPair::generate()
    };
    let x = producer_tx(&w.node, kind, &other, CONTROL_TAG).await;
    let s = valid_slash(&w);
    let block = slash_beside(&mut w.node, &x, &s, true).await;
    let height = tip(&w.node).await.0 + 1;
    let r = w.node.apply_block(block, ValidationMode::Full).await;
    assert!(
        r.is_ok() && tip(&w.node).await.0 == height,
        "NON-VACUITY: Slash(K) beside {kind:?} for another key applies; got {}",
        err_text(&r)
    );
    assert!(
        slash_recorded(&w.node, &w.accused_pk()).await,
        "NON-VACUITY: the control block records the slash"
    );
}

async fn apply_refuses(kind: TxType) {
    other_key_control(kind).await;
    for slash_first in [true, false] {
        let (mut w, x) = world_for(kind, &KeyPair::generate(), &KeyPair::generate()).await;
        let s = valid_slash(&w);
        let block = slash_beside(&mut w.node, &x, &s, slash_first).await;
        let what = order_name(slash_first, kind);
        assert_refused(&mut w, block, ValidationMode::Full, &what).await;
        assert!(
            !slash_recorded(&w.node, &w.accused_pk()).await,
            "{what}: a rejected block must not queue the slash"
        );
    }
}

async fn replay_refuses(kind: TxType) {
    let keys = (KeyPair::generate(), KeyPair::generate());
    for slash_first in [true, false] {
        let block = pair_block(kind, &keys, slash_first).await;
        for mode in [ValidationMode::Light, ValidationMode::Replay] {
            let (mut fresh, _) = world_for(kind, &keys.0, &keys.1).await;
            let what = format!("fresh node {}", order_name(slash_first, kind));
            assert_refused(&mut fresh, block.clone(), mode, &what).await;
        }
    }
}

async fn builder_keeps_slash(kind: TxType) {
    let mut w = world_with(&KeyPair::generate(), &KeyPair::generate()).await;
    let keys = [w.reporter.clone(), w.accused.clone()];
    let params = w.node.params.clone();
    apply_plain_up_to(&mut w.node, &keys, GATE, &params).await;
    let h = tip(&w.node).await.0;
    assert_eq!(h, GATE, "FIXTURE: the next block is one above the gate");
    if kind == TxType::Registration {
        exit_with_collateral(&w.node, &w.accused_pk(), COLLATERAL_TAG).await;
    }
    let x = producer_tx(&w.node, kind, &w.accused, X_TAG).await;
    let s = valid_slash(&w);
    for (tx, at) in [(&s, h), (&x, h - 2)] {
        let r = admit_at(&w.node, tx, at).await;
        assert!(r.is_ok(), "FIXTURE: {:?} admitted; got {r:?}", tx.tx_type);
    }
    assert!(
        resident(&w.node, &s).await && resident(&w.node, &x).await,
        "PRECONDITION: Slash(K) and {kind:?}(K) both resident"
    );
    let (block, selected) = full_block_with(&mut w.node, &[]).await;
    assert!(
        selected.contains(&s.hash()) && !selected.contains(&x.hash()),
        "the builder must pack Slash(K) and not {kind:?}(K); slash selected={}, {kind:?} selected={}",
        selected.contains(&s.hash()),
        selected.contains(&x.hash())
    );
    let r = w.node.apply_block(block, ValidationMode::Full).await;
    assert!(
        r.is_ok() && slash_recorded(&w.node, &w.accused_pk()).await,
        "the producer's own apply must accept its block and record the slash; got {}",
        err_text(&r)
    );
}

// REQ-B3-003 — Decision: a failure means live apply accepts Slash(K) with AddBond(K) in one block, so the AddBond's Bond UTXOs escape the burn and are orphaned.
#[tokio::test]
async fn b3_m25_apply_refuses_slash_with_add_bond_same_key() {
    apply_refuses(TxType::AddBond).await;
}

// REQ-B3-003 — Decision: a failure means live apply accepts Slash(K) with Registration(K) for an exited K, so the new bond is orphaned.
#[tokio::test]
async fn b3_m25_apply_refuses_slash_with_registration_same_key() {
    apply_refuses(TxType::Registration).await;
}

// REQ-B3-003 — Decision: a failure means a syncing or replaying node accepts the Slash(K) plus AddBond(K) block that a live node refuses, a chain split.
#[tokio::test]
async fn b3_m25_replay_refuses_slash_with_add_bond_same_key() {
    replay_refuses(TxType::AddBond).await;
}

// REQ-B3-003 — Decision: a failure means a syncing or replaying node accepts the Slash(K) plus Registration(K) block that a live node refuses.
#[tokio::test]
async fn b3_m25_replay_refuses_slash_with_registration_same_key() {
    replay_refuses(TxType::Registration).await;
}

// REQ-B3-003 — Decision: a failure means the builder packs Slash(K) with AddBond(K), so this node produces a block every upgraded peer rejects.
#[tokio::test]
async fn b3_m25_builder_keeps_slash_drops_add_bond_same_key() {
    if !in_child() {
        return run_in_child(
            "b3_m25_same_block_parity::b3_m25_builder_keeps_slash_drops_add_bond_same_key",
            GATE,
        );
    }
    builder_keeps_slash(TxType::AddBond).await;
}

// REQ-B3-003 — Decision: a failure means the builder packs Slash(K) with Registration(K), so this node produces a block every upgraded peer rejects.
#[tokio::test]
async fn b3_m25_builder_keeps_slash_drops_registration_same_key() {
    if !in_child() {
        return run_in_child(
            "b3_m25_same_block_parity::b3_m25_builder_keeps_slash_drops_registration_same_key",
            GATE,
        );
    }
    builder_keeps_slash(TxType::Registration).await;
}
