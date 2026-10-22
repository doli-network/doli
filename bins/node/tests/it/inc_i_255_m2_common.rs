//! INC-I-255 M2 shared fixtures: a devnet node holding an accused producer with
//! its own, delegated-away and received bonds, chained Light blocks, rollback
//! and whole-state snapshots.
//!
//! OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Output, Transaction};
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::{Node, RollbackAuthority, RollbackOutcome};
use storage::{Outpoint, PendingProducerUpdate, ProducerSet, ProducerStatus, UtxoEntry, UtxoSet};
use tempfile::TempDir;

use super::inc_i_255_m1_common::{canonical, chain_genesis, pair};
use crate::inc_i_156_m1_harness::{
    build_block_with_txs, coinbase_for, install_production_utxo_backend,
};

pub const EPOCH: u64 = 4;
pub const ACCUSED_BONDS: u32 = 3;
pub const DELEGATOR_BONDS: u32 = 2;
pub const HONEST_BONDS: u32 = 2;
pub const ACCUSED_TAG: u8 = 0xA0;
pub const DELEGATOR_TAG: u8 = 0xD0;
pub const HONEST_TAG: u8 = 0xB0;
pub const COIN_TAG: u8 = 0xC0;
pub const COIN: u64 = 777_000;

#[derive(Clone)]
pub struct Keys {
    pub honest: KeyPair,
    pub accused: KeyPair,
    pub delegator: KeyPair,
}

impl Keys {
    pub fn new() -> Self {
        Self {
            honest: KeyPair::generate(),
            accused: KeyPair::generate(),
            delegator: KeyPair::generate(),
        }
    }
}

pub struct Rig {
    pub node: Node,
    pub keys: Keys,
    _tmp: TempDir,
}

pub fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

/// Unsigned RequestWithdrawal in the shape the loaded devnet params accept:
/// one Unbonding payout to the producer while INC-I-257 is armed, else the
/// Normal payout to `legacy_dest` (children run with the lock closed).
pub fn run_b_withdrawal(
    inputs: Vec<doli_core::transaction::Input>,
    pk: PublicKey,
    bond_count: u32,
    legacy_dest: Hash,
    net: u64,
) -> Transaction {
    let armed = doli_core::Network::Devnet
        .params()
        .inc_i_257_unbonding_activation_height
        != u64::MAX;
    let dest = if armed { addr(&pk) } else { legacy_dest };
    let mut tx = Transaction::new_request_withdrawal(inputs, pk, bond_count, dest, net);
    if armed {
        tx.outputs[0].output_type = doli_core::transaction::OutputType::Unbonding;
        tx.outputs[0].lock_until = u64::MAX;
    }
    tx
}

pub fn outpoints(tag: u8, n: u32) -> Vec<Outpoint> {
    let h = Hash::from_bytes([tag; 32]);
    (0..n).map(|i| Outpoint::new(h, i)).collect()
}

fn entry(output: Output) -> UtxoEntry {
    UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

pub async fn rig(keys: &Keys) -> Rig {
    rig_with(keys, ACCUSED_BONDS).await
}

/// `accused_bond_utxos` live Bond UTXOs for the accused; its ledger always holds
/// `ACCUSED_BONDS`, one delegated to the honest producer, one received from the delegator.
pub async fn rig_with(keys: &Keys, accused_bond_utxos: u32) -> Rig {
    let tmp = TempDir::new().unwrap();
    let mut node = Node::new_for_test(
        tmp.path().to_path_buf(),
        vec![
            keys.honest.clone(),
            keys.accused.clone(),
            keys.delegator.clone(),
        ],
    )
    .await
    .expect("Node::new_for_test failed");
    node.params.blocks_per_era = 100_000;
    assert_eq!(
        node.config.network.blocks_per_reward_epoch(),
        EPOCH,
        "FIXTURE: devnet epoch length"
    );
    install_production_utxo_backend(&node).await;
    let unit = node.config.network.bond_unit();
    let (h, a, d) = (
        *keys.honest.public_key(),
        *keys.accused.public_key(),
        *keys.delegator.public_key(),
    );
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(h, HONEST_BONDS, unit).unwrap();
    ps.register_genesis_producer(a, ACCUSED_BONDS, unit)
        .unwrap();
    ps.register_genesis_producer(d, DELEGATOR_BONDS, unit)
        .unwrap();
    ps.delegate_bonds(&a, &h, 1)
        .expect("FIXTURE: accused delegates away");
    ps.delegate_bonds(&d, &a, 1)
        .expect("FIXTURE: delegator delegates to accused");
    node.state_db.write_producer_set(&ps).expect("persist ps");
    *node.producer_set.write().await = ps;
    {
        let mut utxo = node.utxo_set.write().await;
        for (owner, tag, n) in [
            (a, ACCUSED_TAG, accused_bond_utxos),
            (d, DELEGATOR_TAG, DELEGATOR_BONDS),
            (h, HONEST_TAG, HONEST_BONDS),
        ] {
            for op in outpoints(tag, n) {
                utxo.insert(op, entry(Output::bond(unit, addr(&owner), u64::MAX, 0)))
                    .expect("seed bond");
            }
        }
        utxo.insert(
            outpoints(COIN_TAG, 1)[0],
            entry(Output::normal(COIN, addr(&a))),
        )
        .expect("seed coin");
    }
    Rig {
        node,
        keys: keys.clone(),
        _tmp: tmp,
    }
}

impl Rig {
    pub fn accused_pk(&self) -> PublicKey {
        *self.keys.accused.public_key()
    }

    pub fn unit(&self) -> u64 {
        self.node.config.network.bond_unit()
    }

    pub fn slash(&self) -> Transaction {
        let (a, b) = pair(&self.keys.accused, chain_genesis());
        canonical(self.accused_pk(), a, b)
    }

    pub async fn best(&self) -> (u64, u32, Hash) {
        let st = self.node.chain_state.read().await;
        (st.best_height, st.best_slot, st.best_hash)
    }

    /// Next-height block by the honest producer: coinbase, then `txs`.
    pub async fn next_block(&self, txs: Vec<Transaction>, slot_bump: u32) -> Block {
        let (h, s, prev) = self.best().await;
        let (height, slot) = (h + 1, s + 1 + slot_bump);
        let mut all = vec![coinbase_for(height, slot, &self.node.params)];
        all.extend(txs);
        build_block_with_txs(slot, prev, &self.keys.honest, &self.node.params, all)
    }

    pub async fn apply(&mut self, block: Block) -> anyhow::Result<()> {
        self.node.apply_block(block, ValidationMode::Light).await
    }

    pub async fn step(&mut self, txs: Vec<Transaction>) -> Block {
        let block = self.next_block(txs, 0).await;
        let h = self.best().await.0 + 1;
        self.apply(block.clone())
            .await
            .unwrap_or_else(|e| panic!("FIXTURE: apply failed at h={h}: {e:#}"));
        block
    }

    pub async fn advance_to(&mut self, height: u64) {
        while self.best().await.0 < height {
            self.step(Vec::new()).await;
        }
    }

    pub async fn rollback(&mut self) {
        let h = self.best().await.0;
        let r = self
            .node
            .rollback_one_block(RollbackAuthority::CoordinatorApproved { depth: 1 })
            .await
            .unwrap_or_else(|e| panic!("FIXTURE: rollback at h={h} failed: {e:#}"));
        assert!(
            matches!(r, RollbackOutcome::RolledBack),
            "FIXTURE: rollback at h={h} refused"
        );
    }
}

pub fn coinbase_outputs(block: &Block) -> (u64, Vec<Outpoint>) {
    let cb = &block.transactions[0];
    let total = cb.outputs.iter().map(|o| o.amount).sum();
    let ops = (0..cb.outputs.len() as u32)
        .map(|i| Outpoint::new(cb.hash(), i))
        .collect();
    (total, ops)
}

pub async fn live(node: &Node, ops: &[Outpoint]) -> Vec<Outpoint> {
    let utxo = node.utxo_set.read().await;
    ops.iter()
        .copied()
        .filter(|o| utxo.get(o).is_some())
        .collect()
}

pub async fn bond_outpoints_of(node: &Node, pk: &PublicKey) -> Vec<Outpoint> {
    let utxo = node.utxo_set.read().await;
    utxo.get_bond_entries(&addr(pk))
        .into_iter()
        .map(|(o, _, _)| o)
        .collect()
}

pub async fn status(node: &Node, pk: &PublicKey) -> Option<ProducerStatus> {
    let ps = node.producer_set.read().await;
    ps.get_by_pubkey(pk).map(|p| p.status)
}

pub async fn queued_slash(node: &Node, pk: &PublicKey) -> bool {
    let ps = node.producer_set.read().await;
    ps.pending_updates_for(pk)
        .iter()
        .any(|u| matches!(u, PendingProducerUpdate::Slash { .. }))
}

pub async fn schedulable(node: &Node, pk: &PublicKey, height: u64) -> bool {
    let p = node.config.network.params();
    let ps = node.producer_set.read().await;
    ps.active_producers_for_scheduling_at_height(
        height,
        p.inc_i_068_weight_filter_activation_height,
        p.security_audit_activation_height,
    )
    .iter()
    .any(|i| i.public_key == *pk)
}

pub async fn weight(node: &Node, pk: &PublicKey, height: u64) -> u64 {
    let audit = node
        .config
        .network
        .params()
        .security_audit_activation_height;
    let ps = node.producer_set.read().await;
    ps.get_by_pubkey(pk)
        .map(|p| p.selection_weight_at(height, audit))
        .unwrap_or(0)
}

pub fn sorted_ops(set: &UtxoSet) -> Vec<Outpoint> {
    let mut ops: Vec<Outpoint> = set.iter_all().into_iter().map(|(o, _)| o).collect();
    ops.sort_by_key(|o| o.to_bytes());
    ops
}

pub fn root_of(node: &Node, cs: &storage::ChainState, utxo: &UtxoSet, ps: &ProducerSet) -> Hash {
    let gate = node
        .config
        .network
        .params()
        .inc_i_181_pending_root_activation_height;
    storage::compute_state_root_gated(cs, utxo, ps, gate).expect("state root")
}

/// Whole consensus state of one node, field by field.
#[derive(Debug, Clone, PartialEq)]
pub struct Snap {
    pub height: u64,
    pub tip: Hash,
    pub chain: Vec<u8>,
    pub producers: Vec<u8>,
    pub queue: Vec<u8>,
    pub exits: Vec<(Hash, u64)>,
    pub ops: Vec<Outpoint>,
    pub utxo_digest: Hash,
    pub supply: u64,
    pub root: Hash,
}

pub async fn snap(node: &Node) -> Snap {
    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;
    let (_, exit_history, pending) = ps.as_parts();
    let mut exits: Vec<(Hash, u64)> = exit_history.iter().map(|(k, v)| (*k, *v)).collect();
    exits.sort_by_key(|(k, _)| *k.as_bytes());
    Snap {
        height: cs.best_height,
        tip: cs.best_hash,
        chain: cs.serialize_canonical().to_vec(),
        producers: ps.serialize_canonical(),
        queue: bincode::serialize(pending).expect("queue bytes"),
        exits,
        ops: sorted_ops(&utxo),
        utxo_digest: utxo.canonical_digest().expect("utxo digest"),
        supply: utxo.total_value(),
        root: root_of(node, &cs, &utxo, &ps),
    }
}

/// Every component except `total_work` and the root: a bare rollback leaves
/// `total_work` at the undone height until the next apply rewrites it.
pub fn assert_same(got: &Snap, want: &Snap, what: &str) {
    assert_eq!(
        (got.height, got.tip),
        (want.height, want.tip),
        "{what}: chain tip differs"
    );
    assert_eq!(
        got.queue, want.queue,
        "{what}: pending producer-update queue differs"
    );
    assert_eq!(
        got.producers, want.producers,
        "{what}: producer set differs"
    );
    assert_eq!(got.exits, want.exits, "{what}: exit history differs");
    let missing: Vec<_> = want.ops.iter().filter(|o| !got.ops.contains(o)).collect();
    let extra: Vec<_> = got.ops.iter().filter(|o| !want.ops.contains(o)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}: UTXO set differs; missing {missing:?}, extra {extra:?}"
    );
    assert_eq!(got.supply, want.supply, "{what}: supply differs");
    assert_eq!(
        got.utxo_digest, want.utxo_digest,
        "{what}: UTXO entries differ"
    );
    let strip = |c: &[u8]| [&c[..44], &c[52..]].concat();
    assert_eq!(
        strip(&got.chain),
        strip(&want.chain),
        "{what}: chain state differs"
    );
}

/// Full equality including the state root.
pub fn assert_same_root(got: &Snap, want: &Snap, what: &str) {
    assert_same(got, want, what);
    assert_eq!(got.chain, want.chain, "{what}: chain state differs");
    assert_eq!(got.root, want.root, "{what}: state root differs");
}

/// Both nodes apply the same next block; then the whole state, root included, must agree.
pub async fn assert_converged(a: &mut Rig, b: &mut Rig, what: &str) {
    let block = b.next_block(Vec::new(), 0).await;
    a.apply(block.clone())
        .await
        .unwrap_or_else(|e| panic!("{what}: next block on the undone node: {e:#}"));
    b.apply(block)
        .await
        .unwrap_or_else(|e| panic!("{what}: next block on the reference node: {e:#}"));
    assert_same_root(&snap(&a.node).await, &snap(&b.node).await, what);
}
