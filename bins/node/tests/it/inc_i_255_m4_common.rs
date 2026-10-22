//! INC-I-255 M4 shared fixtures: a devnet world where the accused p and the
//! reporter q each hold one live Bond UTXO, signed withdrawals spending them,
//! and builder blocks that place a slash before or after a withdrawal.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: slash_state, block, assembly, slash_admission, pool, slash_withdrawal, slash_evidence, entry, mod, lib, protocol, security_model

#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::{compute_merkle_root, Block, BlockHeader};
use doli_core::transaction::{Input, Output, Transaction};
use doli_node::node::Node;
use mempool::MempoolError;
use storage::{Outpoint, UtxoEntry};

use super::inc_i_178_m0_common::wait_for_fresh_second;
use super::inc_i_255_m1_common::{valid_slash, world_with, World};
use super::inc_i_255_m2_common::{addr, run_b_withdrawal};
use crate::block_sig::sign_header;

pub const M4_MSG: &str = "slash and a bond withdrawal for one producer";
pub const FEE_TIE: u64 = 1;
pub const FEE_OUTBID: u64 = 500_000;
const P_TAG: u8 = 0x4A;
const Q_TAG: u8 = 0x4B;

pub struct M4 {
    pub w: World,
    pub slash: Transaction,
    pub wd: Transaction,
    pub other_wd: Transaction,
}

pub async fn seed_bond(node: &Node, owner: &PublicKey, tag: u8) -> Outpoint {
    let unit = node.config.network.bond_unit();
    let op = Outpoint::new(Hash::from_bytes([tag; 32]), 0);
    let entry = UtxoEntry {
        output: Output::bond(unit, addr(owner), u64::MAX, 0),
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    node.utxo_set
        .write()
        .await
        .insert(op, entry)
        .expect("FIXTURE: seed Bond UTXO");
    op
}

/// One-bond RequestWithdrawal by `kp` paying `fee`.
pub fn withdrawal(node: &Node, kp: &KeyPair, op: Outpoint, fee: u64) -> Transaction {
    let unit = node.config.network.bond_unit();
    let mut input = Input::new(op.tx_hash, op.index);
    input.public_key = Some(*kp.public_key());
    let dest = crypto::hash::hash(b"inc-i-255-m4-withdrawal-destination");
    let mut tx = run_b_withdrawal(vec![input], *kp.public_key(), 1, dest, unit - fee);
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    tx
}

pub async fn m4_with(reporter: &KeyPair, accused: &KeyPair, fee: u64) -> M4 {
    let w = world_with(reporter, accused).await;
    let p_op = seed_bond(&w.node, &w.accused_pk(), P_TAG).await;
    let q_op = seed_bond(&w.node, w.reporter.public_key(), Q_TAG).await;
    let slash = valid_slash(&w);
    let wd = withdrawal(&w.node, &w.accused, p_op, fee);
    let other_wd = withdrawal(&w.node, &w.reporter, q_op, fee);
    M4 {
        w,
        slash,
        wd,
        other_wd,
    }
}

pub async fn m4(fee: u64) -> M4 {
    m4_with(&KeyPair::generate(), &KeyPair::generate(), fee).await
}

/// Normal-lane admission judged as if the tip were `height`.
pub async fn admit_at(node: &Node, tx: &Transaction, height: u64) -> Result<(), MempoolError> {
    let slot = node.chain_state.read().await.best_slot;
    let mut pool = node.mempool.write().await;
    let utxo = node.utxo_set.read().await;
    pool.add_transaction(tx.clone(), &utxo, height, slot)
        .map(|_| ())
}

pub async fn resident(node: &Node, tx: &Transaction) -> bool {
    node.mempool.read().await.contains(&tx.hash())
}

pub async fn revalidate_at(node: &Node, height: u64) {
    let slot = node.chain_state.read().await.best_slot;
    let mut pool = node.mempool.write().await;
    let utxo = node.utxo_set.read().await;
    pool.revalidate(&utxo, height, slot);
}

/// Next-height builder block; `anchor` must be resident and selected, and
/// `extra` (if any) is placed right before or right after it.
pub async fn block_around(
    node: &mut Node,
    anchor: &Transaction,
    extra: Option<&Transaction>,
    before: bool,
) -> Block {
    let kp = node.producer_key.clone().expect("FIXTURE: producer key");
    let pk = *kp.public_key();
    let (prev_hash, prev_slot, height) = {
        let st = node.chain_state.read().await;
        (st.best_hash, st.best_slot, st.best_height + 1)
    };
    for _ in 0..8 {
        let now = wait_for_fresh_second();
        let slot = node.params.timestamp_to_slot(now);
        let built = node
            .build_block_content(prev_hash, prev_slot, height, slot, pk)
            .await
            .expect("FIXTURE: build_block_content returned Err");
        let Some((header, mut txs, bf)) = built else {
            continue;
        };
        let at = txs
            .iter()
            .position(|t| t.hash() == anchor.hash())
            .expect("FIXTURE: the builder must select the resident anchor");
        if let Some(x) = extra {
            txs.insert(if before { at } else { at + 1 }, x.clone());
        }
        let mut header = BlockHeader {
            merkle_root: compute_merkle_root(&txs),
            ..header
        };
        let (vdf_output, vdf_proof) = node
            .compute_block_vdf(&prev_hash, &header)
            .await
            .expect("FIXTURE: compute_block_vdf returned Err");
        header.vdf_output = vdf_output;
        header.vdf_proof = vdf_proof;
        sign_header(&mut header, &kp);
        return Block {
            header,
            transactions: txs,
            aggregate_bls_signature: Vec::new(),
            attestation_bitfield: bf,
        };
    }
    panic!("FIXTURE: build_block_content returned None on every attempt");
}

/// Block [.., Slash(p), RW] (`slash_first`) or [.., RW, Slash(p)] with `rw` selected by the builder.
pub async fn pair_block(r: &mut M4, rw: &Transaction, slash_first: bool) -> Block {
    let h = r.w.node.chain_state.read().await.best_height;
    let ok = admit_at(&r.w.node, rw, h).await;
    assert!(
        ok.is_ok() || resident(&r.w.node, rw).await,
        "FIXTURE: withdrawal admitted; got {ok:?}"
    );
    let slash = r.slash.clone();
    block_around(&mut r.w.node, rw, Some(&slash), slash_first).await
}

pub fn err_text(r: &anyhow::Result<()>) -> String {
    match r {
        Ok(()) => "Ok".to_string(),
        Err(e) => format!("Err({e:#})"),
    }
}

pub fn order_name(slash_first: bool) -> &'static str {
    if slash_first {
        "[Slash(p), RW(p)]"
    } else {
        "[RW(p), Slash(p)]"
    }
}
