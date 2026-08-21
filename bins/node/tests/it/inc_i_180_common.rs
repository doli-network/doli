//! INC-I-180 M1 — shared fixture for the withdrawal-holdings gate suites.
//!
//! OUTPUT CONTRACT: N/A — fixture module (a harness, not a test). It asserts
//! nothing; every assertion lives in the sibling suites that import it.
//! INPUT PARTITIONS: N/A — fixture module.
//!
//! covers: validation_checks.rs, tx_processing.rs (fixture only)
//!
//! The gate reads TWO ledgers: the `ProducerSet` (holdings) and the pre-block
//! `UtxoSet` (which inputs are Bond-typed). A fixture that seeds only the first
//! makes every withdrawal look like it destroys ZERO Bond UTXOs, so it cannot
//! observe the count-binding rule at all. `seed_bond_utxos` is therefore part
//! of the contract of building a case, not an optional extra.

#![allow(dead_code)]

use std::collections::HashSet;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{Outpoint, PendingProducerUpdate, ProducerSet, ProducerStatus, UtxoEntry, UtxoSet};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// n11's ProducerSet bond count at the moment of the mainnet incident.
pub const N11_BONDS: u32 = 433;

/// One block inside the devnet pre-activation band (devnet gate is pinned to 20).
pub const PRE_AH: u64 = 5;

/// Far above any plausible devnet gate.
pub const POST_AH: u64 = 1_000_007;

/// A post-activation height that is ALSO a devnet epoch boundary: devnet runs
/// `blocks_per_reward_epoch = 4` and `genesis_blocks = 40`, so `44` is the first
/// multiple of 4 past genesis. The node's block store is empty in these tests,
/// so `calculate_epoch_rewards(10)` returns `IncompleteEpochStoreError` — the
/// freshly-snap-synced shape that makes the epoch-reward section return early.
pub const EPOCH_BOUNDARY_POST_AH: u64 = 44;

pub const SLOT: u32 = 1_234;

/// Ceiling on the number of `Input`s a fixture transaction carries. `u32::MAX`
/// bonds must stay expressible as a DECLARED count without allocating 4·10⁹
/// inputs, so declared count and input count are decoupled above this bound.
pub const INPUT_CAP: u32 = 1_024;

/// A devnet node with one genesis producer. `Node::new_for_test` is hardwired to
/// `Network::Devnet`, so the devnet activation band is the one that applies.
pub async fn make_node() -> (Node, KeyPair, TempDir) {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    (node, kp, temp)
}

pub fn bond_unit(node: &Node) -> u64 {
    node.config.network.bond_unit()
}

/// Build the ProducerSet half of the `U > P` precondition: `bond_count` bonds
/// already flushed, plus `pending_addbond` bonds queued-but-NOT-yet-flushed.
pub fn build_ledger(
    node: &Node,
    pk: &PublicKey,
    bond_count: u32,
    pending_addbond: u32,
) -> ProducerSet {
    let unit = bond_unit(node);
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*pk, bond_count, unit)
        .expect("register_genesis_producer");
    if pending_addbond > 0 {
        let h = Hash::from_bytes([0xAB; 32]);
        ps.queue_update(PendingProducerUpdate::AddBond {
            pubkey: *pk,
            outpoints: (0..pending_addbond).map(|i| (h, i)).collect(),
            bond_unit: unit,
            creation_slot: 0,
        });
    }
    ps
}

/// A `RequestWithdrawal` declaring `n` bonds and spending `min(n, INPUT_CAP)`
/// Bond UTXOs — the honest on-chain shape.
pub fn withdrawal_tx(pk: &PublicKey, n: u32, tag: u8) -> Transaction {
    withdrawal_tx_with_inputs(pk, n, n.min(INPUT_CAP), tag)
}

/// A `RequestWithdrawal` whose DECLARED count and INPUT count are set
/// independently — the ISSUE-002 partition.
pub fn withdrawal_tx_with_inputs(
    pk: &PublicKey,
    declared: u32,
    input_count: u32,
    tag: u8,
) -> Transaction {
    let h = Hash::from_bytes([tag; 32]);
    let inputs: Vec<Input> = (0..input_count.min(INPUT_CAP))
        .map(|i| Input::new(h, i))
        .collect();
    let dest = crypto::hash::hash(b"inc-i-180-withdrawal-destination");
    Transaction::new_request_withdrawal(inputs, *pk, declared, dest, 1)
}

pub fn exit_tx(pk: &PublicKey) -> Transaction {
    Transaction::new_exit(*pk)
}

/// An `AddBond` creating `n` Bond outputs for `pk` — the in-block AddBond term
/// of the allowance. Its own inputs are never resolved by the gate, which reads
/// inputs of `RequestWithdrawal` transactions only.
pub fn add_bond_tx(node: &Node, pk: &PublicKey, n: u32, tag: u8) -> Transaction {
    let h = Hash::from_bytes([tag; 32]);
    let unit = bond_unit(node);
    Transaction::new_add_bond(vec![Input::new(h, 0)], *pk, n, unit * n as u64, u64::MAX)
}

/// Insert a Bond UTXO for every input of `tx` into the node's pre-block UTXO
/// view, so the gate resolves them as `OutputType::Bond`.
pub async fn seed_bond_utxos(node: &Node, tx: &Transaction, owner: &PublicKey) {
    let pkh = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.as_bytes());
    let unit = bond_unit(node);
    let mut utxo = node.utxo_set.write().await;
    for input in &tx.inputs {
        let entry = UtxoEntry {
            output: Output::bond(unit, pkh, u64::MAX, 0),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(Outpoint::new(input.prev_tx_hash, input.output_index), entry)
            .expect("seed Bond UTXO");
    }
}

/// Seed the first `split_at` inputs as Bond UTXOs owned by `first`, the rest as
/// Bond UTXOs owned by `second` — the mixed-ownership partition.
pub async fn seed_bond_utxos_split(
    node: &Node,
    tx: &Transaction,
    first: &PublicKey,
    split_at: usize,
    second: &PublicKey,
) {
    let unit = bond_unit(node);
    let mut utxo = node.utxo_set.write().await;
    for (i, input) in tx.inputs.iter().enumerate() {
        let owner = if i < split_at { first } else { second };
        let pkh = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.as_bytes());
        let entry = UtxoEntry {
            output: Output::bond(unit, pkh, u64::MAX, 0),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(Outpoint::new(input.prev_tx_hash, input.output_index), entry)
            .expect("seed split Bond UTXO");
    }
}

/// Seed `count` Bond UTXOs at `owner`'s derived address at outpoints
/// `(Hash([tag; 32]), 0..count)`, WITHOUT reference to any transaction.
///
/// This is what makes `owned_live_bonds > bond_inputs` expressible: pass a
/// `tag` no fixture transaction uses and the bonds are live, owned, and unspent
/// by the block under test.
pub async fn seed_owned_bond_utxos(node: &Node, owner: &PublicKey, tag: u8, count: u32) {
    let pkh = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.as_bytes());
    let unit = bond_unit(node);
    let h = Hash::from_bytes([tag; 32]);
    let mut utxo = node.utxo_set.write().await;
    for i in 0..count {
        let entry = UtxoEntry {
            output: Output::bond(unit, pkh, u64::MAX, 0),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(Outpoint::new(h, i), entry)
            .expect("seed unspent Bond UTXO");
    }
}

/// A `RequestWithdrawal` whose inputs are `pre_block` synthetic outpoints under
/// `tag` FOLLOWED BY the first `prior_outputs` real outpoints of `prior`.
///
/// `prior.hash()` is the genuine transaction hash, so when `prior` sits at a
/// LOWER index in the same block the second group is same-block-created — the
/// AUDIT-P1-006 chain, invisible to the pre-block UTXO view.
pub fn withdrawal_tx_chained(
    pk: &PublicKey,
    declared: u32,
    pre_block: u32,
    tag: u8,
    prior: &Transaction,
    prior_outputs: u32,
) -> Transaction {
    let h = Hash::from_bytes([tag; 32]);
    let prior_hash = prior.hash();
    let inputs: Vec<Input> = (0..pre_block)
        .map(|i| Input::new(h, i))
        .chain((0..prior_outputs).map(|j| Input::new(prior_hash, j)))
        .collect();
    let dest = crypto::hash::hash(b"inc-i-180-withdrawal-destination");
    Transaction::new_request_withdrawal(inputs, *pk, declared, dest, 1)
}

/// Insert a NON-Bond (Normal) UTXO for every input of `tx` — models a
/// withdrawal that spends ordinary coins instead of bonds.
pub async fn seed_normal_utxos(node: &Node, tx: &Transaction, owner: &PublicKey) {
    let pkh = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.as_bytes());
    let unit = bond_unit(node);
    let mut utxo = node.utxo_set.write().await;
    for input in &tx.inputs {
        let entry = UtxoEntry {
            output: Output::normal(unit, pkh),
            height: 1,
            is_coinbase: false,
            is_epoch_reward: false,
        };
        utxo.insert(Outpoint::new(input.prev_tx_hash, input.output_index), entry)
            .expect("seed Normal UTXO");
    }
}

/// A minimal valid-economics block: coinbase + the given transactions.
pub fn block_with(node: &Node, height: u64, producer: PublicKey, txs: Vec<Transaction>) -> Block {
    let reward = node.params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, SLOT);

    let mut all = vec![coinbase];
    all.extend(txs);

    let merkle_root = doli_core::block::compute_merkle_root(&all);
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (SLOT as u64 * node.params.slot_duration),
        slot: SLOT,
        producer,
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, all)
}

/// What one (ledger, block, height) triple produces at BOTH layers.
#[derive(Debug)]
pub struct Outcome {
    /// O1 — `validate_block_economics` admitted the block.
    pub validation_ok: bool,
    /// O5 — queued `PendingProducerUpdate::RequestWithdrawal` entries.
    pub queued_withdrawals: usize,
    /// O4 — `withdrawal_pending_count` after apply, before the flush.
    pub withdrawal_pending: u32,
    /// O2 — post-flush producer state.
    pub bond_count: u32,
    pub status: ProducerStatus,
    pub weight: u64,
}

impl Outcome {
    /// O3 — the parity half. `Ok` at validation means the Bond UTXOs WILL be
    /// spent, so the producer-set effect must have happened too.
    pub fn parity_holds(&self, requested_withdrawals: usize) -> bool {
        if self.validation_ok {
            self.queued_withdrawals == requested_withdrawals
        } else {
            true
        }
    }
}

/// Run an arbitrary transaction list through validation AND apply AND the
/// epoch-boundary flush, against independently built ledgers so an apply-side
/// mutation cannot leak backwards into the validation verdict.
pub async fn run_block_case(
    node: &Node,
    kp: &KeyPair,
    bond_count: u32,
    pending_addbond: u32,
    txs: Vec<Transaction>,
    height: u64,
) -> Outcome {
    let pk = *kp.public_key();
    for tx in &txs {
        if tx.tx_type == doli_core::transaction::TxType::RequestWithdrawal {
            seed_bond_utxos(node, tx, &pk).await;
        }
    }
    run_block_case_unseeded(node, kp, bond_count, pending_addbond, txs, height).await
}

/// `run_block_case` without the automatic Bond seeding. The drain partitions
/// need a producer whose ledger disagrees with its UTXOs, which the auto-seed
/// makes unexpressible: seeding is the caller's job here.
pub async fn run_block_case_unseeded(
    node: &Node,
    kp: &KeyPair,
    bond_count: u32,
    pending_addbond: u32,
    txs: Vec<Transaction>,
    height: u64,
) -> Outcome {
    let pk = *kp.public_key();

    {
        let mut guard = node.producer_set.write().await;
        *guard = build_ledger(node, &pk, bond_count, pending_addbond);
    }
    let block = block_with(node, height, pk, txs);
    let validation_ok = node
        .validate_block_economics(&block, height, ValidationMode::Light)
        .await
        .is_ok();

    let mut applied = build_ledger(node, &pk, bond_count, pending_addbond);
    let utxo = UtxoSet::new();
    let mut dirty: HashSet<Hash> = HashSet::new();
    let mut regs: Vec<PublicKey> = Vec::new();
    for tx in block.transactions.iter().skip(1) {
        node.process_transaction_producer_effects(
            tx,
            height,
            SLOT,
            &utxo,
            &mut applied,
            &mut dirty,
            &mut regs,
        );
    }

    let queued_withdrawals = applied
        .pending_updates_for(&pk)
        .iter()
        .filter(|u| matches!(u, PendingProducerUpdate::RequestWithdrawal { .. }))
        .count();
    let withdrawal_pending = applied
        .get_by_pubkey(&pk)
        .map(|i| i.withdrawal_pending_count)
        .unwrap_or(0);

    applied.apply_pending_updates_with_cap(0);
    let (bond_count_after, status, weight) = applied
        .get_by_pubkey(&pk)
        .map(|i| (i.bond_count, i.status, i.selection_weight()))
        .unwrap_or((0, ProducerStatus::Exited, 0));

    Outcome {
        validation_ok,
        queued_withdrawals,
        withdrawal_pending,
        bond_count: bond_count_after,
        status,
        weight,
    }
}

/// The withdrawal-only shorthand the original suite is written against.
pub async fn run_case(
    node: &Node,
    kp: &KeyPair,
    bond_count: u32,
    pending_addbond: u32,
    withdrawals: &[u32],
    height: u64,
) -> Outcome {
    let pk = *kp.public_key();
    let txs: Vec<Transaction> = withdrawals
        .iter()
        .enumerate()
        .map(|(i, n)| withdrawal_tx(&pk, *n, 0x10 + i as u8))
        .collect();
    run_block_case(node, kp, bond_count, pending_addbond, txs, height).await
}

/// Install a freshly built ledger for `pk`, then return the verdict of
/// `validate_block_economics` in `mode` as a STRING, so a caller can assert on
/// the bracketed error code and not merely on `is_err` (QA OBS-R2-003).
/// UTXO seeding is the caller's job: ownership is the variable under test.
pub async fn verdict_in_mode(
    node: &Node,
    pk: &PublicKey,
    bond_count: u32,
    pending_addbond: u32,
    txs: Vec<Transaction>,
    height: u64,
    mode: ValidationMode,
) -> Result<(), String> {
    {
        let mut guard = node.producer_set.write().await;
        *guard = build_ledger(node, pk, bond_count, pending_addbond);
    }
    let block = block_with(node, height, *pk, txs);
    node.validate_block_economics(&block, height, mode)
        .await
        .map_err(|e| e.to_string())
}

/// A Bond output the fixture can hand to a Registration/AddBond transaction.
pub fn bond_output(node: &Node, owner: &PublicKey) -> Output {
    let pkh = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.as_bytes());
    Output::bond(bond_unit(node), pkh, u64::MAX, 0)
}

/// Assert-friendly view of one producer.
pub fn snapshot(ps: &ProducerSet, pk: &PublicKey) -> (u32, ProducerStatus, u64, u32) {
    ps.get_by_pubkey(pk)
        .map(|i| {
            (
                i.bond_count,
                i.status,
                i.selection_weight(),
                i.withdrawal_pending_count,
            )
        })
        .unwrap_or((0, ProducerStatus::Exited, 0, 0))
}

/// Count queued `RequestWithdrawal` updates for one producer.
pub fn queued_withdrawal_count(ps: &ProducerSet, pk: &PublicKey) -> usize {
    ps.pending_updates_for(pk)
        .iter()
        .filter(|u| matches!(u, PendingProducerUpdate::RequestWithdrawal { .. }))
        .count()
}

/// Guard against a fixture that silently stops exercising the Bond path.
pub fn is_bond(output: &Output) -> bool {
    output.output_type == OutputType::Bond
}
