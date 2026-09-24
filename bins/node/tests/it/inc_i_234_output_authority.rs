// OUTPUT CONTRACT: fn apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()>
//   (Node::apply_block, bins/node/src/node/apply_block/mod.rs; per-tx rules via
//    validate_transaction / validate_transaction_with_utxos -> check_output_placement /
//    check_value_authority, crates/core/src/validation/output_authority.rs; mempool twin:
//    Mempool::add_transaction, crates/mempool/src/pool.rs)
//   O1 return value      Ok(()) | Err(..) — block accept/reject verdict
//   O2 persistent store  state_db UTXO column after apply:
//                        (a) native-only supply  = Σ amount of is_native_amount() outputs
//                        (b) pool-aware supply   = (a) + Σ Pool.reserve_a
//                        (c) attacker native balance = Σ native outputs to attacker pkh
//   O3 mempool verdict   Mempool::add_transaction Ok | Err for the same txs
// PATHS:
//   P-PHANTOM-CREATE  Transfer whose outputs include a Pool{reserve_a=R} with no DOLI backing
//                     (+ counterfeit FA of the pool's asset_b, + native change)
//   P-PHANTOM-SWAP    Swap B->A spending the phantom Pool + counterfeit FA -> Normal(R-1)
//   P-W2-COUNTERFEIT  Transfer minting FA(asset_b) with no FA input and no input-anchored
//                     asset_id, then Swap B->A against a funded (seeded) pool
// INPUT PARTITIONS: devnet AH = 0 (rule active at h=1,2); R = 1_000_000_000_000 sats;
//   funding N = 10_000_000 sats; coinbase = base reward only.
// MATRIX:
//   O1,O2b   x P-PHANTOM-CREATE -> case1_transfer_creating_phantom_pool_is_rejected
//   O1,O2a,c x P-PHANTOM-SWAP   -> case2_swap_on_phantom_pool_does_not_mint
//   O3       x P-PHANTOM-CREATE,P-PHANTOM-SWAP -> case3_mempool_rejects_phantom_pool_txs
//   O3       x P-W2-COUNTERFEIT -> case3b_mempool_rejects_counterfeit_fa_transfer
//   O1,O2a,c x P-W2-COUNTERFEIT -> case4_counterfeit_asset_cannot_drain_real_pool
//   (W7 coinbase, builder drop, legit flows, below-AH parity: inc_i_234_output_authority_more.rs)

use crypto::{Hash, KeyPair};
use doli_core::conditions::{Condition, Witness, WitnessSignature};
use doli_core::transaction::{Input, Output, OutputType, SighashType, Transaction, TxType};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

pub(crate) const R: u64 = 1_000_000_000_000;
pub(crate) const FUNDING: u64 = 10_000_000;
pub(crate) const FUND_PREV: Hash = Hash::from_bytes([0x71; 32]);
pub(crate) const FEE_BPS: u16 = 30;

pub(crate) async fn make_node() -> (Node, KeyPair, TempDir) {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    (node, kp, temp)
}

pub(crate) fn addr(kp: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

pub(crate) async fn seed(node: &Node, prev: Hash, idx: u32, output: Output) {
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let op = Outpoint::new(prev, idx);
    node.state_db
        .insert_utxo(&op, &entry)
        .expect("seed state_db");
    node.utxo_set
        .write()
        .await
        .insert(op, entry)
        .expect("seed utxo_set");
}

pub(crate) struct Supply {
    pub native: u64,
    pub pool_aware: u64,
    pub attacker: u64,
}

pub(crate) fn supply(node: &Node, attacker: Hash) -> Supply {
    let mut s = Supply {
        native: 0,
        pool_aware: 0,
        attacker: 0,
    };
    for (_, e) in node.state_db.iter_utxos().iter() {
        let o = &e.output;
        if o.output_type.is_native_amount() {
            s.native += o.amount;
            s.pool_aware += o.amount;
            if o.pubkey_hash == attacker {
                s.attacker += o.amount;
            }
        }
        if o.output_type == OutputType::Pool {
            s.pool_aware += o.pool_metadata().map(|m| m.reserve_a).unwrap_or(0);
        }
    }
    s
}

pub(crate) fn input(prev: Hash, idx: u32, kp: &KeyPair) -> Input {
    Input {
        prev_tx_hash: prev,
        output_index: idx,
        signature: crypto::Signature::from_bytes([0u8; 64]),
        sighash_type: SighashType::All,
        committed_output_count: 0,
        public_key: Some(*kp.public_key()),
    }
}

pub(crate) fn sign(tx: &mut Transaction, kp: &KeyPair, witnesses: bool) {
    for i in 0..tx.inputs.len() {
        let sh = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
    }
    if witnesses {
        let w: Vec<Vec<u8>> = (0..tx.inputs.len())
            .map(|i| {
                let sh = tx.signing_message_for_input(i);
                Witness {
                    signatures: vec![WitnessSignature {
                        pubkey: *kp.public_key(),
                        signature: crypto::signature::sign_hash(&sh, kp.private_key()),
                    }],
                    preimage: None,
                    or_branches: vec![],
                }
                .encode()
            })
            .collect();
        tx.set_covenant_witnesses(&w);
    }
}

pub(crate) fn honest_coinbase(node: &Node, h: u64, slot: u32) -> Transaction {
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    Transaction::new_coinbase(node.params.block_reward(h), pool_hash, h, slot)
}

pub(crate) fn block_with_coinbase(
    node: &Node,
    producer: &KeyPair,
    prev: Hash,
    slot: u32,
    coinbase: Transaction,
    txs: Vec<Transaction>,
) -> Block {
    let mut all = vec![coinbase];
    all.extend(txs);
    let header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(&all),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (slot as u64 * node.params.slot_duration),
        slot,
        producer: *producer.public_key(),
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

pub(crate) fn block(
    node: &Node,
    producer: &KeyPair,
    prev: Hash,
    h: u64,
    slot: u32,
    txs: Vec<Transaction>,
) -> Block {
    let cb = honest_coinbase(node, h, slot);
    block_with_coinbase(node, producer, prev, slot, cb, txs)
}

pub(crate) fn fa(amount: u64, pkh: Hash, asset: Hash) -> Output {
    fa_supply(amount, pkh, asset, u64::MAX)
}

pub(crate) fn fa_supply(amount: u64, pkh: Hash, asset: Hash, total: u64) -> Output {
    Output::fungible_asset(
        amount,
        pkh,
        asset,
        total,
        "FAKE",
        &Condition::Signature(pkh),
    )
    .expect("fa output")
}

pub(crate) async fn apply(node: &mut Node, b: Block) -> Result<(), String> {
    node.apply_block(b, ValidationMode::Light)
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn mempool_add(node: &Node, tx: Transaction, h: u64) -> Result<(), String> {
    let utxo = node.utxo_set.read().await;
    let mut mp = node.mempool.write().await;
    mp.add_transaction(tx, &utxo, h, h as u32)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Native N -> [Pool{R, reserve_b=1}, FA(asset, R), change].
pub(crate) async fn phantom_transfer(node: &Node, kp: &KeyPair) -> (Transaction, Hash, Hash) {
    let pkh = addr(kp);
    seed(node, FUND_PREV, 0, Output::normal(FUNDING, pkh)).await;
    let asset = Hash::from_bytes([0xDA; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset, FEE_BPS);
    let pool = Output::pool(pool_id, asset, R, 1, 1_000, 0, 1, FEE_BPS, 1);
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input(FUND_PREV, 0, kp)],
        outputs: vec![pool, fa(R, pkh, asset), Output::normal(0, pkh)],
        extra_data: vec![],
    };
    let fee = tx.minimum_fee();
    tx.outputs[2].amount = FUNDING - fee;
    sign(&mut tx, kp, false);
    (tx, pool_id, asset)
}

/// [phantom Pool{R,1}, FA(R)] -> [Pool{1, R+1}, Normal(R-1)].
pub(crate) fn phantom_swap(kp: &KeyPair, t1: Hash, pool_id: Hash, asset: Hash) -> Transaction {
    let pkh = addr(kp);
    let new_pool = Output::pool(pool_id, asset, 1, R + 1, 1_000, 0, 2, FEE_BPS, 1);
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Swap,
        inputs: vec![input(t1, 0, kp), input(t1, 1, kp)],
        outputs: vec![new_pool, Output::normal(R - 1, pkh)],
        extra_data: vec![],
    };
    sign(&mut tx, kp, true);
    tx
}

/// Native funding at `prev:0` -> [FA(asset, amount) with no FA input, native change].
pub(crate) async fn counterfeit_fa_transfer(
    node: &Node,
    kp: &KeyPair,
    prev: Hash,
    asset: Hash,
    amount: u64,
) -> Transaction {
    let pkh = addr(kp);
    seed(node, prev, 0, Output::normal(FUNDING, pkh)).await;
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input(prev, 0, kp)],
        outputs: vec![fa(amount, pkh, asset), Output::normal(0, pkh)],
        extra_data: vec![],
    };
    let fee = tx.minimum_fee();
    tx.outputs[1].amount = FUNDING - fee;
    sign(&mut tx, kp, false);
    tx
}

// REQ-234-SEC-004 — Decision: a failure means a plain Transfer still mints an unbacked Pool at apply.
#[tokio::test]
async fn case1_transfer_creating_phantom_pool_is_rejected() {
    let (mut node, kp, _t) = make_node().await;
    let pkh = addr(&kp);
    let (t1, _, _) = phantom_transfer(&node, &kp).await;
    let before = supply(&node, pkh);
    let v = {
        let blk = block(&node, &kp, Hash::ZERO, 1, 1, vec![t1]);
        apply(&mut node, blk).await
    };
    let after = supply(&node, pkh);
    assert!(
        v.is_err(),
        "case1: Transfer creating Pool{{reserve_a={R}}} funded by 0 DOLI was ACCEPTED; \
         pool-aware supply delta {}",
        after.pool_aware as i128 - before.pool_aware as i128
    );
    assert!(
        after.pool_aware <= before.pool_aware + node.params.block_reward(1),
        "case1: pool-aware supply rose beyond the block reward"
    );
}

// REQ-234-SEC-009 — Decision: a failure means native DOLI can still be minted via a phantom-pool swap.
#[tokio::test]
async fn case2_swap_on_phantom_pool_does_not_mint() {
    let (mut node, kp, _t) = make_node().await;
    let pkh = addr(&kp);
    let (t1, pool_id, asset) = phantom_transfer(&node, &kp).await;
    let t1h = t1.hash();
    let s0 = supply(&node, pkh);
    let b1 = block(&node, &kp, Hash::ZERO, 1, 1, vec![t1]);
    let b1h = b1.hash();
    let v1 = apply(&mut node, b1).await;
    let t2 = phantom_swap(&kp, t1h, pool_id, asset);
    let v2 = {
        let blk = block(&node, &kp, b1h, 2, 2, vec![t2]);
        apply(&mut node, blk).await
    };
    let s2 = supply(&node, pkh);
    let rewards = node.params.block_reward(1) + node.params.block_reward(2);
    assert!(
        s2.native as i128 - s0.native as i128 <= rewards as i128,
        "case2: native supply rose by {} (> rewards {rewards}); b1={v1:?} b2={v2:?}",
        s2.native as i128 - s0.native as i128
    );
    assert!(
        s2.attacker <= FUNDING,
        "case2: attacker turned {FUNDING} sats into {} native sats",
        s2.attacker
    );
}

// REQ-234-SEC-012 — Decision: a failure means the mempool relays phantom-pool txs the chain must reject.
#[tokio::test]
async fn case3_mempool_rejects_phantom_pool_txs() {
    let (mut node, kp, _t) = make_node().await;
    let (t1, pool_id, asset) = phantom_transfer(&node, &kp).await;
    let t1h = t1.hash();
    let a1 = mempool_add(&node, t1.clone(), 1).await;
    let v1 = {
        let blk = block(&node, &kp, Hash::ZERO, 1, 1, vec![t1]);
        apply(&mut node, blk).await
    };
    for (op, e) in node.state_db.iter_utxos().iter() {
        if op.tx_hash == t1h {
            let _ = node.utxo_set.write().await.insert(*op, e.clone());
        }
    }
    let a2 = mempool_add(&node, phantom_swap(&kp, t1h, pool_id, asset), 2).await;
    assert!(
        a1.is_err(),
        "case3: mempool admitted the phantom-Pool Transfer"
    );
    assert!(
        v1.is_err() || a2.is_err(),
        "case3: mempool admitted the Swap draining the phantom pool"
    );
}

// REQ-234-SEC-012 — Decision: a failure means the mempool admits an FA mint with no authority (M4 unwired).
#[tokio::test]
async fn case3b_mempool_rejects_counterfeit_fa_transfer() {
    let (node, kp, _t) = make_node().await;
    let t1 = counterfeit_fa_transfer(&node, &kp, FUND_PREV, Hash::from_bytes([0xB2; 32]), R).await;
    let a = mempool_add(&node, t1, 1).await;
    assert!(
        a.is_err(),
        "case3b: mempool admitted a Transfer minting counterfeit FA"
    );
}

// REQ-234-SEC-007 — Decision: a failure means counterfeit asset_b can still drain a real pool's reserve_a.
#[tokio::test]
async fn case4_counterfeit_asset_cannot_drain_real_pool() {
    let (mut node, kp, _t) = make_node().await;
    let pkh = addr(&kp);
    let asset_b = Hash::from_bytes([0xB2; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, FEE_BPS);
    let (da, db): (u64, u64) = (1_000_000, 1_000_000);
    let pool_prev = Hash::from_bytes([0xA7; 32]);
    seed(
        &node,
        pool_prev,
        0,
        Output::pool(pool_id, asset_b, da, db, 1_000_000, 0, 0, FEE_BPS, 0),
    )
    .await;
    let x = da * db;
    let t1 = counterfeit_fa_transfer(&node, &kp, FUND_PREV, asset_b, x).await;
    let t1h = t1.hash();
    let s0 = supply(&node, pkh);
    let b1 = block(&node, &kp, Hash::ZERO, 1, 1, vec![t1]);
    let b1h = b1.hash();
    let v1 = apply(&mut node, b1).await;
    let mut t2 = Transaction {
        version: 1,
        tx_type: TxType::Swap,
        inputs: vec![input(pool_prev, 0, &kp), input(t1h, 0, &kp)],
        outputs: vec![
            Output::pool(pool_id, asset_b, 1, db + x, 1_000_000, 0, 2, FEE_BPS, 0),
            Output::normal(da - 1, pkh),
        ],
        extra_data: vec![],
    };
    sign(&mut t2, &kp, true);
    let v2 = {
        let blk = block(&node, &kp, b1h, 2, 2, vec![t2]);
        apply(&mut node, blk).await
    };
    let s2 = supply(&node, pkh);
    assert!(
        v1.is_err() || v2.is_err(),
        "case4: counterfeit asset_b swapped for {} DOLI of real reserve; attacker {} -> {}",
        da - 1,
        s0.attacker,
        s2.attacker
    );
    assert!(
        s2.attacker <= FUNDING,
        "case4: attacker native balance {} exceeds funding {FUNDING}",
        s2.attacker
    );
}
