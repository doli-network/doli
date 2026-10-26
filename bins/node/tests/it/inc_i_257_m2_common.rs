//! INC-I-257 M2 node fixtures: withdrawals by the accused of the Run B rig,
//! the collateral it owns, and the observed effect of one slash block.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use crypto::PublicKey;
use doli_core::transaction::{OutputType, Transaction};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};

use super::inc_i_255_m1_common::full_block_with;
use super::inc_i_255_m2_common::{
    addr, coinbase_outputs, outpoints, snap, Rig, ACCUSED_BONDS, ACCUSED_TAG, EPOCH,
};
use super::inc_i_255_m4_common::{admit_at, err_text};
use super::inc_i_257_m1_common::{current_withdrawal, FEE};
use super::inc_i_257_m1_fx::period;

pub const REFUSAL: &str = "not an active or unbonding producer";

/// One withdrawal by the accused, mined at `height`.
pub struct AccusedWd {
    pub tx: Transaction,
    pub payout: Outpoint,
    pub spent: Vec<Outpoint>,
    pub height: u64,
    pub lock: u64,
    pub net: u64,
}

/// Withdrawal of the accused's Bond UTXOs `idx` at the next height; lock = n + period.
pub async fn accused_withdrawal(r: &Rig, idx: &[u32]) -> AccusedWd {
    let n = r.best().await.0 + 1;
    let lock = n + period(r);
    let all = outpoints(ACCUSED_TAG, ACCUSED_BONDS);
    let spent: Vec<Outpoint> = idx.iter().map(|i| all[*i as usize]).collect();
    let net = r.unit() * spent.len() as u64 - FEE;
    let tx = current_withdrawal(&r.keys.accused, &spent, net, lock);
    AccusedWd {
        payout: Outpoint::new(tx.hash(), 0),
        tx,
        spent,
        height: n,
        lock,
        net,
    }
}

pub async fn mine(r: &mut Rig, txs: Vec<Transaction>, what: &str) {
    let block = r.next_block(txs, 0).await;
    let h = r.best().await.0 + 1;
    let res = r.apply(block).await;
    assert!(
        res.is_ok(),
        "PRECONDITION: {what} is mined at {h}; got {}",
        err_text(&res)
    );
}

/// Mine a withdrawal of `idx` at the next height and return it.
pub async fn mine_withdrawal(r: &mut Rig, idx: &[u32], what: &str) -> AccusedWd {
    let wd = accused_withdrawal(r, idx).await;
    mine(r, vec![wd.tx.clone()], what).await;
    wd
}

/// Full withdrawal at EPOCH + 1, then the next boundary, so the accused is Exited.
pub async fn exit_by_full_withdrawal(r: &mut Rig) -> AccusedWd {
    r.advance_to(EPOCH).await;
    let all: Vec<u32> = (0..ACCUSED_BONDS).collect();
    let wd = mine_withdrawal(r, &all, "the accused's full withdrawal").await;
    r.advance_to(2 * EPOCH).await;
    wd
}

fn burnable_type(e: &UtxoEntry) -> bool {
    e.output.output_type == OutputType::Bond || e.output.output_type == OutputType::Unbonding
}

/// Bond + Unbonding value owned by address(pk).
pub async fn collateral(node: &Node, pk: &PublicKey) -> u64 {
    let a = addr(pk);
    let utxo = node.utxo_set.read().await;
    utxo.iter_all()
        .into_iter()
        .filter(|(_, e)| e.output.pubkey_hash == a && burnable_type(e))
        .map(|(_, e)| e.output.amount)
        .sum()
}

/// Normal value owned by address(pk), minted entries excluded.
pub async fn normal_owned(node: &Node, pk: &PublicKey) -> u64 {
    let a = addr(pk);
    let utxo = node.utxo_set.read().await;
    utxo.iter_all()
        .into_iter()
        .filter(|(_, e)| {
            e.output.pubkey_hash == a
                && e.output.output_type == OutputType::Normal
                && !e.is_coinbase
                && !e.is_epoch_reward
        })
        .map(|(_, e)| e.output.amount)
        .sum()
}

pub async fn entry_of(node: &Node, op: &Outpoint) -> Option<UtxoEntry> {
    node.utxo_set.read().await.get(op)
}

/// One block at the next height holding `txs`: result, supply drop net of
/// coinbase minting, and whether the tip moved.
#[derive(Debug)]
pub struct Effect {
    pub applied: String,
    pub supply_drop: i128,
    pub tip_moved: bool,
}

pub async fn block_effect(r: &mut Rig, txs: Vec<Transaction>) -> Effect {
    let before = snap(&r.node).await;
    let block = r.next_block(txs, 0).await;
    let minted = coinbase_outputs(&block).0;
    let res = r.apply(block).await;
    let after = snap(&r.node).await;
    let minted = if res.is_ok() { minted } else { 0 };
    Effect {
        applied: err_text(&res),
        supply_drop: i128::from(before.supply) + i128::from(minted) - i128::from(after.supply),
        tip_moved: after.height != before.height,
    }
}

/// Verdicts of the three paths on one slash at the next height.
#[derive(Debug)]
pub struct Paths {
    pub mempool: String,
    pub resident: bool,
    pub selected: bool,
    pub block: Effect,
}

/// Mempool (judged at tip + 1), builder (the slash made resident through the
/// system lane when the normal lane refuses it), then a block holding it.
pub async fn three_paths(r: &mut Rig, tx: &Transaction) -> Paths {
    let tip = r.best().await.0;
    let admitted = admit_at(&r.node, tx, tip).await;
    let resident = admitted.is_ok()
        || r.node
            .mempool
            .write()
            .await
            .add_system_transaction(tx.clone(), tip, None)
            .is_ok();
    let (_, selected) = full_block_with(&mut r.node, &[]).await;
    r.node.mempool.write().await.remove_transaction(&tx.hash());
    let block = block_effect(r, vec![tx.clone()]).await;
    Paths {
        mempool: format!("{admitted:?}"),
        resident,
        selected: selected.contains(&tx.hash()),
        block,
    }
}
