// OUTPUT CONTRACT: fn validate_block_economics(&self, block: &Block, height: u64, mode: ValidationMode) -> Result<()>
//   (reached through `Node::apply_block(&mut self, Block, ValidationMode)` at apply_block/mod.rs:121,
//    and mirrored by the builder `Node::build_block_content(..)` coinbase at production/assembly.rs:367-374)
//   O1 return value      Ok(()) | Err("[ECON_COINBASE_AMOUNT] ...") — the coinbase-amount verdict
//   O2 persistent store  state_db UTXO column after apply_block: native supply
//                        (Σ native-output amounts + Σ Pool reserve_a) — the SUPPLY output
//   O3 builder output    coinbase amount chosen by build_block_content for a block holding the tx
// PATHS:
//   P-EXEMPT-BURN   BurnAsset (fee-exempt, utxo.rs:282-291), zero native in/out, padded FA output
//   P-EXEMPT-SWAP   Swap (fee-exempt, AMM E1 surplus = 0), padded FA token_b output
//   P-FEE-PAYING    Transfer (fee-checked, utxo.rs:292-294), same padding — control
//   P-BUILDER       honest builder over a mempool holding the P-EXEMPT-BURN tx
// INPUT PARTITIONS: padding = 524,288 B (per-output era-0 cap) | Swap without padding (pool+FA
//   metadata only); coinbase = reward + Σ floor(len/100) (the builder's formula).
// MATRIX:
//   O1,O2 x P-EXEMPT-BURN  -> case_a_burn_asset_padded_supply_delta_le_reward     [EXPECTED RED]
//   O1    x P-EXEMPT-BURN  -> case_a2_validate_block_economics_rejects_unbacked   [EXPECTED RED]
//   O1,O2 x P-EXEMPT-SWAP  -> case_b_swap_padded_supply_delta_le_reward           [EXPECTED RED]
//   O1,O2 x P-EXEMPT-SWAP  -> case_b0_swap_unpadded_supply_delta_le_reward        [measured]
//   O1,O2 x P-FEE-PAYING   -> case_c_fee_paying_control_supply_delta_le_reward    [EXPECTED GREEN]
//   O3    x P-BUILDER      -> case_d_builder_coinbase_includes_unbacked_credit     [EXPECTED RED]
//
//! INC-I-233 reproduction — coinbase per-byte credit over fee-exempt TxTypes.
//!
//! Supply invariant under test: for every block, native supply after apply minus
//! native supply before apply <= block_reward(h). Every native sat the coinbase
//! mints above the reward must be backed by a native sat a transaction destroyed.

use std::time::{SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair};
use doli_core::conditions::{Condition, Witness, WitnessSignature};
use doli_core::transaction::{Input, Output, OutputType, SighashType, Transaction, TxType};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const PAD_TO: usize = 524_288; // BASE_EXTRA_DATA_SIZE, era 0 per-output cap
const TOKENS: u64 = 10_000;
const H: u64 = 1;
const SLOT: u32 = 1;

async fn make_node() -> (Node, KeyPair, TempDir) {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    (node, kp, temp)
}

fn addr(kp: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

/// Seed one UTXO into BOTH the RocksDB state (read by apply_block's batch overlay)
/// and the in-memory UtxoSet (read by the mempool and the builder).
async fn seed(node: &Node, prev: Hash, idx: u32, output: Output) {
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

/// Native DOLI supply held in UTXOs: native-typed output amounts plus the DOLI
/// reserve (reserve_a) held inside Pool outputs (Pool.amount is 0 by design).
fn supply(node: &Node) -> u64 {
    node.state_db
        .iter_utxos()
        .iter()
        .map(|(_, e)| {
            let native = if e.output.output_type.is_native_amount() {
                e.output.amount
            } else {
                0
            };
            let reserve = if e.output.output_type == OutputType::Pool {
                e.output.pool_metadata().map(|m| m.reserve_a).unwrap_or(0)
            } else {
                0
            };
            native + reserve
        })
        .sum()
}

/// The block Σ exactly as validation_checks/mod.rs:571-589 and assembly.rs:367-374 compute it.
fn byte_credit(txs: &[Transaction]) -> u64 {
    txs.iter()
        .flat_map(|t| t.outputs.iter())
        .map(|o| {
            o.extra_data.len() as u64 * doli_core::consensus::FEE_PER_BYTE
                / doli_core::consensus::FEE_DIVISOR
        })
        .sum()
}

fn pad(out: &mut Output) {
    assert!(out.extra_data.len() <= PAD_TO);
    out.extra_data.resize(PAD_TO, 0xAB);
}

fn input(prev: Hash, idx: u32, kp: &KeyPair) -> Input {
    Input {
        prev_tx_hash: prev,
        output_index: idx,
        signature: crypto::Signature::from_bytes([0u8; 64]),
        sighash_type: SighashType::All,
        committed_output_count: 0,
        public_key: Some(*kp.public_key()),
    }
}

fn sign_plain(tx: &mut Transaction, kp: &KeyPair) {
    for i in 0..tx.inputs.len() {
        let sh = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&sh, kp.private_key());
    }
}

/// Sign every input AND attach a Signature-condition witness for every input
/// (all inputs of the BurnAsset are conditioned FungibleAsset UTXOs).
fn sign_with_witnesses(tx: &mut Transaction, kp: &KeyPair) {
    sign_plain(tx, kp);
    let witnesses: Vec<Vec<u8>> = (0..tx.inputs.len())
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
    tx.set_covenant_witnesses(&witnesses);
}

fn block_at(node: &Node, producer: &KeyPair, coinbase_amount: u64, txs: Vec<Transaction>) -> Block {
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let mut all = vec![Transaction::new_coinbase(
        coinbase_amount,
        pool_hash,
        H,
        SLOT,
    )];
    all.extend(txs);
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: doli_core::block::compute_merkle_root(&all),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (SLOT as u64 * node.params.slot_duration),
        slot: SLOT,
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

// ---------------------------------------------------------------- fixtures

const FA_PREV: Hash = Hash::from_bytes([0xF1; 32]);

/// Padded zero-burn BurnAsset: 1 FA input (TOKENS) -> 1 FA output (TOKENS, same asset_id),
/// output extra_data padded to 524,288 B. Native in = 0, native out = 0.
async fn padded_burn_asset(node: &Node, kp: &KeyPair) -> Transaction {
    let pkh = addr(kp);
    let asset_id = Output::compute_asset_id(&Hash::from_bytes([0xEE; 32]), 0);
    let fa = |amt| {
        Output::fungible_asset(
            amt,
            pkh,
            asset_id,
            TOKENS,
            "TKN",
            &Condition::Signature(pkh),
        )
        .expect("fa output")
    };
    seed(node, FA_PREV, 0, fa(TOKENS)).await;
    let mut out = fa(TOKENS);
    pad(&mut out);
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::BurnAsset,
        inputs: vec![input(FA_PREV, 0, kp)],
        outputs: vec![out],
        extra_data: vec![],
    };
    sign_with_witnesses(&mut tx, kp);
    tx
}

struct Measured {
    reward: u64,
    credit: u64,
    before: u64,
    after: u64,
    apply: Result<(), String>,
}

async fn apply_with_inflated_coinbase(
    node: &mut Node,
    kp: &KeyPair,
    tx: Transaction,
    label: &str,
) -> Measured {
    let reward = node.params.block_reward(H);
    let credit = byte_credit(std::slice::from_ref(&tx));
    let block = block_at(node, kp, reward + credit, vec![tx]);
    let before = supply(node);
    let apply = node
        .apply_block(block, ValidationMode::Light)
        .await
        .map_err(|e| e.to_string());
    let after = supply(node);
    let delta = after as i128 - before as i128;
    println!(
        "[INC-I-233][{label}] reward={reward} credit={credit} coinbase={} supply_before={before} \
         supply_after={after} delta={delta} excess_over_reward={} apply={:?} best_height={}",
        reward + credit,
        delta - reward as i128,
        apply,
        node.chain_state.read().await.best_height
    );
    Measured {
        reward,
        credit,
        before,
        after,
        apply,
    }
}

fn assert_supply_bounded(m: &Measured, label: &str) {
    let delta = m.after as i128 - m.before as i128;
    assert!(
        m.apply.is_err() || delta <= m.reward as i128,
        "{label}: native supply rose by {delta} sats in one block, above block_reward {} \
         (excess {} = unbacked coinbase credit {}). apply_block accepted it: {:?}",
        m.reward,
        delta - m.reward as i128,
        m.credit,
        m.apply
    );
}

// ---------------------------------------------------------------- cases

/// (a) BurnAsset padded, zero native in/out, coinbase = reward + Σ bytes/100.
#[tokio::test]
async fn case_a_burn_asset_padded_supply_delta_le_reward() {
    let (mut node, kp, _t) = make_node().await;
    let tx = padded_burn_asset(&node, &kp).await;
    let m = apply_with_inflated_coinbase(&mut node, &kp, tx, "a-BurnAsset").await;
    assert_supply_bounded(&m, "case (a) BurnAsset");
}

/// (a2) The block-level verdict alone (O1) — no per-tx validation involved.
#[tokio::test]
async fn case_a2_validate_block_economics_rejects_unbacked() {
    let (node, kp, _t) = make_node().await;
    let tx = padded_burn_asset(&node, &kp).await;
    let reward = node.params.block_reward(H);
    let credit = byte_credit(std::slice::from_ref(&tx));
    let block = block_at(&node, &kp, reward + credit, vec![tx]);
    let v = node
        .validate_block_economics(&block, H, ValidationMode::Full)
        .await
        .map_err(|e| e.to_string());
    println!("[INC-I-233][a2] reward={reward} credit={credit} verdict={v:?}");
    assert!(
        v.is_err(),
        "case (a2): validate_block_economics accepted coinbase = reward {reward} + {credit} \
         while the only user tx (BurnAsset) destroyed 0 native sats"
    );
}

/// (b) Swap A->B with E1 surplus = 0 (100 DOLI in, reserve_a +100), FA token_b output padded.
#[tokio::test]
async fn case_b_swap_padded_supply_delta_le_reward() {
    let (mut node, kp, _t) = make_node().await;
    let tx = swap_tx(&node, &kp, true).await;
    let m = apply_with_inflated_coinbase(&mut node, &kp, tx, "b-Swap-padded").await;
    assert_supply_bounded(&m, "case (b) Swap padded");
}

/// (b0) Same Swap, NO padding: measures the credit from Pool + FA metadata alone.
#[tokio::test]
async fn case_b0_swap_unpadded_supply_delta_le_reward() {
    let (mut node, kp, _t) = make_node().await;
    let tx = swap_tx(&node, &kp, false).await;
    let m = apply_with_inflated_coinbase(&mut node, &kp, tx, "b0-Swap-unpadded").await;
    assert_supply_bounded(&m, "case (b0) Swap unpadded");
}

const POOL_PREV: Hash = Hash::from_bytes([0xA1; 32]);
const FUND_PREV: Hash = Hash::from_bytes([0xA3; 32]);

async fn swap_tx(node: &Node, kp: &KeyPair, padded: bool) -> Transaction {
    let pkh = addr(kp);
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 30);
    // Same numbers as crates/core/tests/inc_i_096_amm_conservation.rs (swap A->B).
    let old_pool = Output::pool(pool_id, asset_b, 1000, 1000, 707, 0, 100, 30, 100);
    let new_pool = Output::pool(pool_id, asset_b, 1100, 910, 707, 0, 101, 30, 100);
    let mut tokens_out = Output::fungible_asset(
        90,
        pkh,
        asset_b,
        1_000_000,
        "TKN",
        &Condition::Signature(pkh),
    )
    .expect("fa");
    if padded {
        pad(&mut tokens_out);
    }
    seed(node, POOL_PREV, 0, old_pool).await;
    seed(node, FUND_PREV, 0, Output::normal(100, pkh)).await;
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Swap,
        inputs: vec![input(POOL_PREV, 0, kp), input(FUND_PREV, 0, kp)],
        outputs: vec![new_pool, tokens_out],
        extra_data: vec![],
    };
    sign_plain(&mut tx, kp);
    tx
}

/// (c) Control: fee-paying Transfer (token issuance shape) with the SAME padding.
/// Pays exactly minimum_fee() = BASE_FEE + floor(524,288/100) = 5,243.
#[tokio::test]
async fn case_c_fee_paying_control_supply_delta_le_reward() {
    let (mut node, kp, _t) = make_node().await;
    let pkh = addr(&kp);
    let prev = Hash::from_bytes([0xC0; 32]);
    let funding: u64 = 10_000_000;
    seed(&node, prev, 0, Output::normal(funding, pkh)).await;
    let mut fa = Output::fungible_asset(
        TOKENS,
        pkh,
        // INC-I-234: an issuance without asset inputs must anchor on input 0.
        Output::compute_asset_id(&prev, 0),
        TOKENS,
        "CTL",
        &Condition::Signature(pkh),
    )
    .expect("fa");
    pad(&mut fa);
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![input(prev, 0, &kp)],
        outputs: vec![fa, Output::normal(0, pkh)],
        extra_data: vec![],
    };
    let fee = tx.minimum_fee();
    tx.outputs[1].amount = funding - fee;
    sign_plain(&mut tx, &kp);
    println!("[INC-I-233][c] minimum_fee={fee}");
    let m = apply_with_inflated_coinbase(&mut node, &kp, tx, "c-Transfer-control").await;
    assert!(m.apply.is_ok(), "control fixture must apply: {:?}", m.apply);
    assert_supply_bounded(&m, "case (c) fee-paying control");
}

/// (d) Honest builder: offer the padded BurnAsset to the node's own mempool and
/// build. The builder coinbase must not exceed reward + native fee actually paid (0).
#[tokio::test]
async fn case_d_builder_coinbase_includes_unbacked_credit() {
    let (mut node, kp, _t) = make_node().await;
    let tx = padded_burn_asset(&node, &kp).await;
    let tx_hash = tx.hash();
    let admit = {
        let utxo = node.utxo_set.read().await;
        let mut mp = node.mempool.write().await;
        mp.add_transaction(tx, &utxo, H, 1)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    println!("[INC-I-233][d] mempool admission = {admit:?}");
    assert!(
        admit.is_ok(),
        "fixture: mempool must admit the tx: {admit:?}"
    );

    let pk = *kp.public_key();
    let mut built = None;
    for _ in 0..12 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let slot = node.params.timestamp_to_slot(now);
        if let Some((header, txs, _)) = node
            .build_block_content(Hash::ZERO, slot.saturating_sub(1), H, slot, pk)
            .await
            .expect("build_block_content Err")
        {
            built = Some(Block::new(header, txs));
            break;
        }
    }
    let block = built.expect("fixture: builder aborted 12 times");
    let included = block.transactions.iter().any(|t| t.hash() == tx_hash);
    let reward = node.params.block_reward(H);
    let coinbase = block.transactions[0].outputs[0].amount;
    let credit = byte_credit(&block.transactions[1..]);
    let verdict = node
        .validate_block_economics(&block, H, ValidationMode::Full)
        .await
        .map_err(|e| e.to_string());
    println!(
        "[INC-I-233][d] included={included} reward={reward} coinbase={coinbase} \
         builder_credit={credit} excess={} own_verdict={verdict:?}",
        coinbase as i128 - reward as i128
    );
    assert!(included, "fixture: builder must include the admitted tx");
    assert!(
        coinbase <= reward,
        "case (d): honest builder minted coinbase {coinbase} = reward {reward} + {} for a block \
         whose only user tx destroyed 0 native sats",
        coinbase - reward
    );
}
