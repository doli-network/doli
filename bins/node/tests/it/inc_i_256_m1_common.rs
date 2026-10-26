//! INC-I-256 M1 fixtures: twin rigs (production RocksDB UTXO backend), the
//! failing block F = [coinbase, RequestWithdrawal of every bond of p, Transfer
//! spending an outpoint that does not exist], and the producer-set observables
//! of a node compared field by field with a control that never saw F.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: apply_block producer_undo helpers mod block_handling rollback periodic fork_cache set_core set_lifecycle set_persistence chain snapshot_install

#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::Transaction;
use doli_core::Block;
use doli_node::node::Node;
use storage::{Outpoint, ProducerSet};

use super::inc_i_255_m2_common::{outpoints, Rig, HONEST_BONDS, HONEST_TAG};
use super::inc_i_255_m6_common::transfer;
use super::inc_i_257_m1_common::{gated_withdrawal, FEE};
use super::inc_i_257_m1_fx::period;
use crate::inc_i_156_m1_harness::{build_block_with_txs, coinbase_for};

pub use super::inc_i_257_m1_fx::twins;

pub const P_BONDS: u32 = HONEST_BONDS;
pub const GHOST_TAG: u8 = 0xEE;

/// p = the rig's honest producer: listed with `P_BONDS` bonds, owns `P_BONDS` Bond UTXOs.
pub fn p_key(r: &Rig) -> &KeyPair {
    &r.keys.honest
}

pub fn p_of(r: &Rig) -> PublicKey {
    *r.keys.honest.public_key()
}

/// Gated withdrawal by `kp` of `bonds` (first `bonds` outpoints of `tag`) mined at `height`.
pub fn withdrawal(r: &Rig, kp: &KeyPair, tag: u8, bonds: u32, height: u64) -> Transaction {
    let ops = outpoints(tag, bonds);
    let net = r.unit() * u64::from(bonds) - FEE;
    gated_withdrawal(kp, &ops, net, height + period(r))
}

/// RequestWithdrawal(p, every bond) mined at `height`.
pub fn withdraw_all(r: &Rig, height: u64) -> Transaction {
    withdrawal(r, p_key(r), HONEST_TAG, P_BONDS, height)
}

pub fn ghost_outpoint() -> Outpoint {
    Outpoint::new(Hash::from_bytes([GHOST_TAG; 32]), 0)
}

/// Transfer by p spending an outpoint no node holds.
pub fn ghost_spend(r: &Rig) -> Transaction {
    transfer(p_key(r), ghost_outpoint(), 1_000, FEE)
}

/// Block by the honest producer at an explicit (height, slot, parent): coinbase, then `txs`.
pub fn block_at(r: &Rig, height: u64, slot: u32, prev: Hash, txs: Vec<Transaction>) -> Block {
    let mut all = vec![coinbase_for(height, slot, &r.node.params)];
    all.extend(txs);
    build_block_with_txs(slot, prev, &r.keys.honest, &r.node.params, all)
}

/// F at `height` on `prev`.
pub fn failing_block_at(r: &Rig, height: u64, slot: u32, prev: Hash) -> Block {
    block_at(
        r,
        height,
        slot,
        prev,
        vec![withdraw_all(r, height), ghost_spend(r)],
    )
}

/// F at the next height on the tip.
pub async fn failing_block(r: &Rig) -> Block {
    let (h, s, prev) = r.best().await;
    failing_block_at(r, h + 1, s + 1, prev)
}

/// Producer-set observables of one node.
#[derive(Debug, Clone, PartialEq)]
pub struct Obs {
    pub canonical: Vec<u8>,
    pub pending_count: usize,
    pub pending_for_p: usize,
    pub wpc: Option<u32>,
    pub slashed_or_queued: bool,
    pub addbond_in_flight: u32,
    pub root_bound: Hash,
    pub root_unbound: Hash,
}

pub fn obs_of(ps: &ProducerSet, p: &PublicKey) -> Obs {
    Obs {
        canonical: ps.serialize_canonical(),
        pending_count: ps.pending_update_count(),
        pending_for_p: ps.pending_updates_for(p).len(),
        wpc: ps.get_by_pubkey(p).map(|i| i.withdrawal_pending_count),
        slashed_or_queued: ps.is_slashed_or_slash_queued(p),
        addbond_in_flight: ps.pending_addbond_count(p),
        root_bound: ps.state_root_component(true).0,
        root_unbound: ps.state_root_component(false).0,
    }
}

pub async fn obs(node: &Node, p: &PublicKey) -> Obs {
    obs_of(&*node.producer_set.read().await, p)
}

/// How many of the six residue observables differ from `want`.
pub fn residue_fields(got: &Obs, want: &Obs) -> usize {
    [
        got.canonical != want.canonical,
        got.pending_count != want.pending_count,
        got.pending_for_p != want.pending_for_p,
        got.wpc != want.wpc,
        got.root_bound != want.root_bound,
        got.root_unbound != want.root_unbound,
    ]
    .iter()
    .filter(|d| **d)
    .count()
}

/// Prints the metric line for one failed attempt.
pub fn report(got: &Obs, want: &Obs) -> usize {
    let n = residue_fields(got, want);
    println!("B2M1_METRIC RESIDUE_FIELDS={n}");
    n
}

/// Every observable equals the control's.
pub fn assert_obs(got: &Obs, want: &Obs, what: &str) {
    assert_eq!(
        got.pending_count, want.pending_count,
        "{what}: pending_update_count differs from the control (residue queued)"
    );
    assert_eq!(
        got.pending_for_p, want.pending_for_p,
        "{what}: pending_updates_for(p) differs from the control"
    );
    assert_eq!(
        got.wpc, want.wpc,
        "{what}: withdrawal_pending_count of p differs from the control"
    );
    assert_eq!(
        got.slashed_or_queued, want.slashed_or_queued,
        "{what}: is_slashed_or_slash_queued(p) differs from the control"
    );
    assert_eq!(
        got.addbond_in_flight, want.addbond_in_flight,
        "{what}: pending_addbond_count(p) differs from the control"
    );
    assert!(
        got.canonical == want.canonical,
        "{what}: serialize_canonical() differs from the control ({} vs {} bytes)",
        got.canonical.len(),
        want.canonical.len()
    );
    assert_eq!(
        got.root_bound, want.root_bound,
        "{what}: state_root_component(true) differs from the control"
    );
    assert_eq!(
        got.root_unbound, want.root_unbound,
        "{what}: state_root_component(false) differs from the control"
    );
}

pub async fn tip(r: &Rig) -> (u64, Hash) {
    let (h, _, hash) = r.best().await;
    (h, hash)
}

/// Parks the tip block at `height` (keeping hash and slot) and indexes it there,
/// so the canonical-chain walk of the next block stops at the parked tip.
pub async fn park_tip(r: &Rig, height: u64) {
    let (_, hash) = tip(r).await;
    let block = r
        .node
        .block_store
        .get_block(&hash)
        .expect("FIXTURE: block store read")
        .expect("FIXTURE: tip block stored");
    r.node
        .block_store
        .put_block_canonical(&block, height)
        .expect("FIXTURE: index the parked tip");
    r.node.chain_state.write().await.best_height = height;
}
