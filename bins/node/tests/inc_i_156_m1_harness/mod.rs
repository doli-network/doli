//! INC-I-156 / M1 — shared fixture for the two R1 leak proofs.
//!
//! OUTPUT CONTRACT: N/A — fixture file. It declares no `#[test]`; the enumerations live
//! with the functions under test, in `inc_i_156_m1_rocksdb_clear_leak.rs`
//! (`Node::rollback_one_block`) and `inc_i_156_m1_reorg_clear_leak.rs`
//! (`Node::execute_reorg`). INPUT PARTITIONS: N/A — fixture file.
//!
//! Consumed by both of those files, which need the identical setup — the PRODUCTION
//! `RocksDb` UTXO variant, a real chain applied through `apply_block`, a spend chain so the
//! rolled-back range both CREATES and SPENDS, and a DENSE block store — so it lives here
//! rather than being copy-pasted and drifting.
//!
//! ## The one non-obvious thing in here
//!
//! `install_production_utxo_backend` is load-bearing twice over, not cosmetic:
//!   * SEMANTICALLY — `UtxoSet::clear()` is honest on the `InMemory` variant that
//!     `Node::new_for_test` builds (`init.rs:1129`, `init.rs:1338`), so an InMemory test
//!     PASSES on the broken code and proves nothing. That is precisely how INC-I-152's
//!     first P1-003 test failed to catch its bug.
//!   * MECHANICALLY — since storage Phase 3, `apply_block` writes UTXOs only through
//!     `BlockBatch` into `state_db`, so an `InMemory` set on a test node stays permanently
//!     EMPTY and could evidence nothing at all.
//!
//! Technique copied from `inc_i_152_p1_003_rollback_holed_store.rs:471-474`.

#![allow(dead_code)] // each consumer uses a subset

use std::sync::Once;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{self, ConsensusParams};
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

// ==================== Environment bootstrap ====================

static ENV_INIT: Once = Once::new();

pub fn init_env() {
    ENV_INIT.call_once(|| {
        let _ = Network::Devnet.params();
    });
}

pub async fn make_node(n_producers: usize) -> (Node, Vec<KeyPair>, TempDir) {
    init_env();
    let temp = TempDir::new().unwrap();
    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
        .await
        .expect("Node::new_for_test failed");
    // Keep block_reward positive across the whole test range so every block contributes
    // real state — the state whose corruption these files prove.
    node.params.blocks_per_era = 100_000;
    (node, producers, temp)
}

/// Swap the detached `InMemory` set `new_for_test` leaves behind for the PRODUCTION
/// `state_db`-backed variant (`init.rs:311`; `fork_recovery.rs:363` for a snap-synced node).
/// See this module's doc comment for why this is not optional.
pub async fn install_production_utxo_backend(node: &Node) {
    {
        let mut utxo = node.utxo_set.write().await;
        *utxo = storage::UtxoSet::from_state_db(node.state_db.clone());
    }
    assert!(
        node.utxo_set.read().await.is_rocksdb(),
        "fixture: the node must hold the production RocksDb UTXO variant — an InMemory \
         variant test PASSES on the broken code and proves nothing"
    );
}

// ==================== Block construction ====================

pub fn devnet_genesis_hash() -> Hash {
    doli_core::chainspec::ChainSpec::devnet().genesis_hash()
}

pub fn build_block_with_txs(
    block_slot: u32,
    prev_hash: Hash,
    producer: &KeyPair,
    params: &ConsensusParams,
    txs: Vec<Transaction>,
) -> Block {
    let timestamp = params.genesis_time + (block_slot as u64 * params.slot_duration);
    let merkle_root = doli_core::block::compute_merkle_root(&txs);
    let header = BlockHeader {
        version: 2,
        prev_hash,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: devnet_genesis_hash(),
        timestamp,
        slot: block_slot,
        producer: *producer.public_key(),
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, txs)
}

pub fn coinbase_for(height: u64, slot: u32, params: &ConsensusParams) -> Transaction {
    Transaction::new_coinbase(
        params.block_reward(height),
        consensus::reward_pool_pubkey_hash(),
        height,
        slot,
    )
}

/// Signed 1-in / 1-out Transfer. Pattern copied from
/// `inc_i_152_p1_003_rollback_holed_store.rs:226-250`.
pub fn signed_transfer(spend: Outpoint, amount: u64, owner: &KeyPair, to: Hash) -> Transaction {
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input {
            prev_tx_hash: spend.tx_hash,
            output_index: spend.index,
            signature: crypto::Signature::default(),
            sighash_type: doli_core::transaction::SighashType::All,
            committed_output_count: 0,
            public_key: Some(*owner.public_key()),
        }],
        outputs: vec![Output {
            amount,
            pubkey_hash: to,
            output_type: OutputType::Normal,
            lock_until: 0,
            extra_data: vec![],
        }],
        extra_data: vec![],
    };
    let sighash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sighash, owner.private_key());
    tx
}

pub async fn apply_plain_up_to(
    node: &mut Node,
    producers: &[KeyPair],
    up_to_height: u64,
    params: &ConsensusParams,
) {
    let mut prev = node.chain_state.read().await.best_hash;
    let start_h = node.chain_state.read().await.best_height;
    for h in (start_h + 1)..=up_to_height {
        let block = build_block_with_txs(
            h as u32,
            prev,
            &producers[(h as usize) % producers.len()],
            params,
            vec![coinbase_for(h, h as u32, params)],
        );
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("setup: apply_block failed at h={h}: {e}"));
    }
}

/// Apply one block carrying `[coinbase, user_tx]` and return it, so the caller can name the
/// outpoints it CREATED — the residual set the defect leaks.
pub async fn apply_block_with_transfer(
    node: &mut Node,
    producers: &[KeyPair],
    height: u64,
    params: &ConsensusParams,
    user_tx: Transaction,
) -> Block {
    let prev = node.chain_state.read().await.best_hash;
    let block = build_block_with_txs(
        height as u32,
        prev,
        &producers[(height as usize) % producers.len()],
        params,
        vec![coinbase_for(height, height as u32, params), user_tx],
    );
    node.apply_block(block.clone(), ValidationMode::Light)
        .await
        .unwrap_or_else(|e| panic!("setup: apply_block with transfer failed at h={height}: {e}"));
    block
}

/// Every outpoint a block creates, with its amount.
pub fn created_outpoints(block: &Block) -> Vec<(Outpoint, u64)> {
    block
        .transactions
        .iter()
        .flat_map(|tx| {
            let h = tx.hash();
            tx.outputs
                .iter()
                .enumerate()
                .map(move |(i, o)| (Outpoint::new(h, i as u32), o.amount))
        })
        .collect()
}

/// Write a synthetic, spendable UTXO straight into the set and return its outpoint.
///
/// On the RocksDb backend `UtxoSet::insert` writes through to `cf_utxo`, so this one call
/// is the whole funding step.
///
/// CAUTION for callers: this entry is not produced by any block, so a rebuild-from-genesis
/// can never recreate it. It MUST be fully spent at a height at or below the rollback
/// target, otherwise it appears in the canonical snapshot but never in a rebuild and every
/// set-equality assertion becomes a false failure.
pub async fn fund(node: &Node, owner_pkh: Hash, amount: u64, height: u64, seed: &[u8]) -> Outpoint {
    let outpoint = Outpoint::new(crypto::hash::hash(seed), 0);
    let mut utxo = node.utxo_set.write().await;
    utxo.insert(
        outpoint,
        UtxoEntry {
            output: Output {
                amount,
                pubkey_hash: owner_pkh,
                output_type: OutputType::Normal,
                lock_until: 0,
                extra_data: vec![],
            },
            height,
            is_coinbase: false,
            is_epoch_reward: false,
        },
    )
    .expect("fixture: funding insert failed");
    outpoint
}

pub fn address_of(owner: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, owner.public_key().as_bytes())
}

// ==================== Observation surface ====================

/// Content snapshot of the UTXO set: sorted `(outpoint, amount)` plus the byte-exact
/// canonical encoding the state root is computed over.
///
/// REQ-I156-006 requires CONTENT, not counts. `pairs` gives a readable diff; `canonical` is
/// the strongest form and is the same encoding consensus hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtxoContent {
    pub pairs: Vec<(Outpoint, u64)>,
    pub canonical: Vec<u8>,
    pub total_value: u64,
    pub len: usize,
}

impl UtxoContent {
    fn of(pairs_src: Vec<(Outpoint, UtxoEntry)>, canonical: Vec<u8>, total_value: u64) -> Self {
        let mut pairs: Vec<(Outpoint, u64)> = pairs_src
            .into_iter()
            .map(|(op, e)| (op, e.output.amount))
            .collect();
        pairs.sort_by_key(|(op, _)| op.to_bytes());
        let len = pairs.len();
        Self {
            pairs,
            canonical,
            total_value,
            len,
        }
    }

    pub fn contains(&self, op: &Outpoint) -> bool {
        self.pairs.iter().any(|(p, _)| p == op)
    }
}

/// Read the UTXO content through the node's façade (`utxo_set`).
pub async fn utxo_content(node: &Node) -> UtxoContent {
    let utxo = node.utxo_set.read().await;
    UtxoContent::of(
        utxo.iter_all(),
        utxo.serialize_canonical(),
        utxo.total_value(),
    )
}

/// Read the UTXO content INDEPENDENTLY, straight from the persistent store, bypassing the
/// façade (Rule AQ-5: after a function writes to a store, read the store back rather than
/// trusting the writer's own view). `atomic_replace` is what makes the leak durable, so
/// this is the observation that matters operationally.
pub fn persisted_utxo_content(node: &Node) -> UtxoContent {
    let pairs = node.state_db.iter_utxos();
    let total = node.state_db.utxo_total_value();
    UtxoContent::of(pairs, node.state_db.serialize_canonical_utxo(), total)
}

pub fn describe(pairs: &[(Outpoint, u64)], limit: usize) -> String {
    pairs
        .iter()
        .take(limit)
        .map(|(op, amt)| format!("{:.8}#{} ({} doli)", op.tx_hash, op.index, amt))
        .collect::<Vec<_>>()
        .join(", ")
}

// ==================== Shared post-conditions ====================

/// INV-GUARD-001 (INC-I-136) + INV-SYNC-014 (INC-I-118) / REQ-I156-010.
///
/// Asserted after EVERY completed rollback or reorg in both files: the counter must agree
/// with the store, and the live variant must still be the `state_db`-backed one. The second
/// is not redundant with any UTXO-content assertion — a fix that rebuilt into a scratch
/// `InMemory` set and published it would satisfy every content assertion while silently
/// detaching the node from state_db, which is the exact regression INC-I-118 was opened for
/// and the reason the INC-I-136 fence at `init.rs:111` exists.
pub async fn assert_utxo_invariants(node: &Node, scenario: &str) {
    let (count, live) = {
        let utxo = node.utxo_set.read().await;
        (utxo.utxo_count() as usize, utxo.iter_all().len())
    };
    assert_eq!(
        count, live,
        "[{scenario}] / INV-GUARD-001: utxo_count() ({count}) must equal the number of live \
         cf_utxo entries ({live}). `StateDb::clear_utxos` stores utxo_count = 0 \
         (writes.rs:100) and `insert_utxo` is counter-idempotent (writes.rs:44-46), so the \
         replay must land the counter exactly on the key count."
    );
    assert!(
        node.utxo_set.read().await.is_rocksdb(),
        "[{scenario}] / INV-SYNC-014 (REQ-I156-010): the live UtxoSet must STILL be the \
         state_db-backed variant. A fix that rebuilt into a scratch InMemory set and \
         published it would satisfy every content assertion while silently detaching the \
         node from state_db (INC-I-118), and would bypass the INC-I-136 fence at init.rs:111."
    );
    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "[{scenario}] / AUDIT-P1-001: a COMPLETED rollback or reorg must leave \
         CF_META[rebuild_in_progress] disarmed. The marker is armed before the destructive \
         `clear()` and cleared only after the trailing `atomic_replace` succeeds; a marker \
         surviving a successful rebuild would halt production and snapshot service on a node \
         whose ledger is in fact complete — a false positive that costs an operator resync."
    );
}
