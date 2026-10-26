//! INC-I-255 M6 shared fixtures: the accused p and the reporter q each hold
//! one lock-expired Bond UTXO; Transfers signed by the owner spend them.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: slash_state, utxo, block, assembly, tx_processing, slash_burn, slash_admission, pool, slash_withdrawal, withdrawal_economics, slash_bond_spend, mod.rs, lib.rs, validation_checks, protocol, security_model

#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::Block;
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::ValidationMode;
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};

use super::inc_i_255_m1_common::{full_block_with, valid_slash, world_with, World};
use super::inc_i_255_m2_common::{addr, live, snap, Snap};
use super::inc_i_255_m4_common::err_text;

pub const M6_MSG: &str = "spend of the accused's bond";
pub const EXPIRED_LOCK: u64 = 1;
pub const FEE_TIE: u64 = 1;
pub const FEE_OUTBID: u64 = 500_000;
pub const P_BOND_TAG: u8 = 0x6A;
pub const Q_BOND_TAG: u8 = 0x6B;
pub const P_COIN_TAG: u8 = 0x6C;
pub const Q_COIN_TAG: u8 = 0x6D;

pub struct M6 {
    pub w: World,
    pub slash: Transaction,
    pub p_bond: Outpoint,
    pub q_bond: Outpoint,
    /// Transfer by p spending p's lock-expired Bond UTXO.
    pub t: Transaction,
    /// Transfer by q spending q's lock-expired Bond UTXO.
    pub tq: Transaction,
}

async fn seed(node: &Node, op: Outpoint, output: Output) -> Outpoint {
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    node.utxo_set
        .write()
        .await
        .insert(op, entry)
        .expect("FIXTURE: seed UTXO");
    op
}

/// Bond UTXO owned by address(owner) with a finite `lock_until`.
pub async fn seed_expired_bond(
    node: &Node,
    owner: &PublicKey,
    tag: u8,
    lock_until: u64,
) -> Outpoint {
    let unit = node.config.network.bond_unit();
    let op = Outpoint::new(Hash::from_bytes([tag; 32]), 0);
    seed(node, op, Output::bond(unit, addr(owner), lock_until, 0)).await
}

/// Normal UTXO owned by address(owner).
pub async fn seed_coin(node: &Node, owner: &PublicKey, tag: u8, amount: u64) -> Outpoint {
    let op = Outpoint::new(Hash::from_bytes([tag; 32]), 0);
    seed(node, op, Output::normal(amount, addr(owner))).await
}

fn signed(kp: &KeyPair, op: Outpoint, outputs: Vec<Output>) -> Transaction {
    let mut input = Input::new(op.tx_hash, op.index);
    input.public_key = Some(*kp.public_key());
    let mut tx = Transaction::new_transfer(vec![input], outputs);
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    tx
}

/// One-input Transfer by `kp` spending `op` (worth `amount_in`) to a fixed
/// Normal destination, paying `fee`.
pub fn transfer(kp: &KeyPair, op: Outpoint, amount_in: u64, fee: u64) -> Transaction {
    let dest = crypto::hash::hash(b"inc-i-255-m6-transfer-destination");
    signed(kp, op, vec![Output::normal(amount_in - fee, dest)])
}

/// Transfer by `kp` spending `op` (worth `amount_in`) that creates one Bond
/// output owned by address(owner) with `lock_until`, at the minimum fee.
pub fn bond_maker(
    kp: &KeyPair,
    op: Outpoint,
    amount_in: u64,
    owner: &PublicKey,
    lock_until: u64,
) -> Transaction {
    let draft = Transaction::new_transfer(
        Vec::new(),
        vec![Output::bond(0, addr(owner), lock_until, 0)],
    );
    let fee = draft.minimum_fee();
    signed(
        kp,
        op,
        vec![Output::bond(amount_in - fee, addr(owner), lock_until, 0)],
    )
}

pub async fn m6_with(reporter: &KeyPair, accused: &KeyPair, fee: u64) -> M6 {
    let w = world_with(reporter, accused).await;
    let unit = w.node.config.network.bond_unit();
    let p_bond = seed_expired_bond(&w.node, &w.accused_pk(), P_BOND_TAG, EXPIRED_LOCK).await;
    let q_bond =
        seed_expired_bond(&w.node, w.reporter.public_key(), Q_BOND_TAG, EXPIRED_LOCK).await;
    let slash = valid_slash(&w);
    let t = transfer(&w.accused, p_bond, unit, fee);
    let tq = transfer(&w.reporter, q_bond, unit, fee);
    M6 {
        w,
        slash,
        p_bond,
        q_bond,
        t,
        tq,
    }
}

pub async fn m6(fee: u64) -> M6 {
    m6_with(&KeyPair::generate(), &KeyPair::generate(), fee).await
}

/// `slash` and `other` in block order.
pub fn ordered(slash: &Transaction, other: &Transaction, slash_first: bool) -> Vec<Transaction> {
    if slash_first {
        vec![slash.clone(), other.clone()]
    } else {
        vec![other.clone(), slash.clone()]
    }
}

pub fn order_name(slash_first: bool, other: &str) -> String {
    if slash_first {
        format!("[Slash(p), {other}]")
    } else {
        format!("[{other}, Slash(p)]")
    }
}

/// Next-height builder block (empty mempool) carrying `txs` after the coinbase.
pub async fn block_with(node: &mut Node, txs: &[Transaction]) -> Block {
    full_block_with(node, txs).await.0
}

/// Applied result plus the whole state before and after.
pub async fn apply_observed(
    node: &mut Node,
    block: Block,
    mode: ValidationMode,
) -> (String, Snap, Snap) {
    let before = snap(node).await;
    let res = node.apply_block(block, mode).await;
    let after = snap(node).await;
    (err_text(&res), before, after)
}

pub async fn is_live(node: &Node, op: Outpoint) -> bool {
    !live(node, &[op]).await.is_empty()
}
