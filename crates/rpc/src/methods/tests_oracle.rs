//! Tests for Phase 2.1 Oracle RPC methods (M9-M11).
//!
//! This file is included via `#[path = "tests_oracle.rs"] mod tests;` in
//! `oracle.rs`, so its logical path is `methods::oracle::tests` — the
//! `super::*` glob below resolves to the parent `oracle` module, giving
//! access to `ORACLE_TRUST_MODEL` and the `pub(super) async fn` handlers
//! on `RpcContext`.
//!
//! OUTPUT CONTRACT:
//!   getOraclePrice  -> Result<Value, RpcError> where Value is
//!                       { pair_id, price_cents, last_update_height,
//!                         contributor_count, is_stale, trust_model }
//!                       | null                          (UTXO absent)
//!                       | RpcError::invalid_params      (bad pair_id)
//!   getOracleAttestations -> Result<Value, RpcError> where Value is
//!                       { epoch, pair_id, attestations: [
//!                         { attester_pubkey, attester_pubkey_hash,
//!                           price_cents, bond_weight (u64 | null) } ] }
//!                       | RpcError::invalid_params      (bad epoch / pair_id)
//!
//! INPUT PARTITIONS:
//!   getOraclePrice:
//!     pair_id     = { valid_64hex, malformed_hex, missing }
//!     utxo_state  = { has_oracle_price(pair_id), absent }
//!     freshness   = { age <= blocks_per_reward_epoch,
//!                     age > blocks_per_reward_epoch }
//!   getOracleAttestations:
//!     epoch         = { past_with_attestations, past_empty, current, future,
//!                       missing }
//!     pair_id       = { has_attestations_for_epoch, none_for_epoch, unknown }
//!     bond_source   = { snapshot.epoch == queried.epoch  -> Some(weight),
//!                       snapshot.epoch != queried.epoch  -> null }
//!     signer_dedup  = { unique per signer, duplicate same-signer }

use super::*;
use std::sync::Arc;
use tokio::sync::RwLock;

use crypto::Hash;
use doli_core::network::Network;
use doli_core::transaction::Output;
use mempool::{Mempool, MempoolPolicy};
use storage::{BlockStore, ChainState, Outpoint, UtxoEntry, UtxoSet};
use tempfile::TempDir;

use doli_core::transaction::PriceAttestationData;
use doli_core::{Block, BlockHeader, Transaction};
use storage::StateDb;
use vdf::{VdfOutput, VdfProof};

// ============================================================================
// M9 fixtures (UTXO-only)
// ============================================================================

/// Tempdir held by the test for the BlockStore path; dropped at test
/// end. The `_tempdir` field keeps it alive — RpcContext does not own
/// it because BlockStore is opened under it.
struct TestCtx {
    ctx: RpcContext,
    utxo_set: Arc<RwLock<UtxoSet>>,
    chain_state: Arc<RwLock<ChainState>>,
    _tempdir: TempDir,
}

/// Build a minimal `RpcContext` for M9 testing. Mainnet defaults
/// (blocks_per_reward_epoch = 360); UTXO set is empty; chain_state at
/// height 0 unless mutated by the test.
fn build_ctx() -> TestCtx {
    let tempdir = TempDir::new().expect("tempdir");
    let chain_state = Arc::new(RwLock::new(ChainState::new(Hash::ZERO)));
    let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));
    let block_store = Arc::new(BlockStore::open(tempdir.path()).expect("blockstore"));
    let params = doli_core::consensus::ConsensusParams::default();
    let mempool = Arc::new(RwLock::new(Mempool::new(
        MempoolPolicy::default(),
        params.clone(),
        Network::Mainnet,
    )));

    let ctx = RpcContext::new_for_network(
        chain_state.clone(),
        block_store,
        utxo_set.clone(),
        mempool,
        params,
        Network::Mainnet,
    );
    TestCtx {
        ctx,
        utxo_set,
        chain_state,
        _tempdir: tempdir,
    }
}

/// Insert an OraclePrice UTXO at the deterministic per-pair address
/// (mirrors M6's aggregator) so M9 can find it via
/// `oracle_price_outpoint(pair_id)`.
async fn insert_oracle_price(
    utxo_set: &Arc<RwLock<UtxoSet>>,
    pair_id: Hash,
    price_cents: u64,
    last_update_height: u64,
    contributor_count: u16,
    creation_height: u64,
) {
    let output = Output::oracle_price(pair_id, price_cents, last_update_height, contributor_count);
    let (tx_hash, index) = doli_core::oracle::oracle_price_outpoint(&pair_id);
    let outpoint = Outpoint::new(tx_hash, index);
    let entry = UtxoEntry {
        output,
        height: creation_height,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo_set
        .write()
        .await
        .insert(outpoint, entry)
        .expect("insert oracle price utxo");
}

/// Set the chain state's best height to a known value so staleness
/// computation is deterministic.
async fn set_best_height(chain_state: &Arc<RwLock<ChainState>>, height: u64) {
    chain_state.write().await.best_height = height;
}

fn pair_id_fixture() -> Hash {
    // BLAKE3("ORACLE_PAIR" || "DOLI/USD") — same shape as production
    // but value is irrelevant; tests use bit-identical pair_id throughout.
    crypto::hash::hash_with_domain(b"ORACLE_PAIR", b"DOLI/USD")
}

// ============================================================================
// M9: getOraclePrice tests
// ============================================================================

// ---------- partition: utxo_state = has_oracle_price + freshness fresh ----------
#[tokio::test]
async fn m9_happy_path_returns_parsed_extra_data() {
    let t = build_ctx();
    let pair_id = pair_id_fixture();

    insert_oracle_price(&t.utxo_set, pair_id, 12_345, 1_000, 8, 1_000).await;
    set_best_height(&t.chain_state, 1_100).await;

    let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
    let resp = t
        .ctx
        .get_oracle_price(params)
        .await
        .expect("M9 happy-path Ok");

    assert_eq!(resp["pair_id"].as_str().unwrap(), pair_id.to_hex());
    assert_eq!(resp["price_cents"].as_u64().unwrap(), 12_345);
    assert_eq!(resp["last_update_height"].as_u64().unwrap(), 1_000);
    assert_eq!(resp["contributor_count"].as_u64().unwrap(), 8);
    assert!(!resp["is_stale"].as_bool().unwrap());
    assert_eq!(resp["trust_model"].as_str().unwrap(), "structural-anchored");
}

// ---------- partition: freshness = age > blocks_per_reward_epoch ----------
#[tokio::test]
async fn m9_is_stale_true_when_age_exceeds_epoch_width() {
    let t = build_ctx();
    let pair_id = pair_id_fixture();

    // Mainnet blocks_per_reward_epoch = 360. age = 1000 - 100 = 900 > 360.
    insert_oracle_price(&t.utxo_set, pair_id, 100, 100, 1, 100).await;
    set_best_height(&t.chain_state, 1_000).await;

    let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
    let resp = t.ctx.get_oracle_price(params).await.unwrap();
    assert!(
        resp["is_stale"].as_bool().unwrap(),
        "age={} should be > epoch_width={}",
        900,
        360
    );
}

// ---------- partition: freshness = age <= blocks_per_reward_epoch ----------
#[tokio::test]
async fn m9_is_stale_false_when_age_within_window() {
    let t = build_ctx();
    let pair_id = pair_id_fixture();

    // age = 1000 - 900 = 100, well within 360-block window
    insert_oracle_price(&t.utxo_set, pair_id, 100, 900, 1, 900).await;
    set_best_height(&t.chain_state, 1_000).await;

    let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
    let resp = t.ctx.get_oracle_price(params).await.unwrap();
    assert!(!resp["is_stale"].as_bool().unwrap());
}

// ---------- partition: utxo_state = absent (pre-aggregation OR pre-activation) ----------
#[tokio::test]
async fn m9_returns_null_when_utxo_absent() {
    let t = build_ctx();
    let pair_id = pair_id_fixture();

    let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
    let resp = t.ctx.get_oracle_price(params).await.unwrap();
    assert!(
        resp.is_null(),
        "Expected null when OraclePrice UTXO is absent, got {:?}",
        resp
    );
}

// ---------- partition: trust_model byte-equality ----------
#[tokio::test]
async fn m9_trust_model_byte_equal_to_constant() {
    let t = build_ctx();
    let pair_id = pair_id_fixture();
    insert_oracle_price(&t.utxo_set, pair_id, 1, 1, 1, 1).await;
    set_best_height(&t.chain_state, 1).await;

    let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
    let resp = t.ctx.get_oracle_price(params).await.unwrap();
    assert_eq!(
        resp["trust_model"].as_str().unwrap().as_bytes(),
        b"structural-anchored",
        "trust_model must be the literal string 'structural-anchored'"
    );
    // Locks the production const against accidental edit.
    assert_eq!(ORACLE_TRUST_MODEL, "structural-anchored");
}

// ---------- partition: pair_id = malformed hex ----------
#[tokio::test]
async fn m9_malformed_pair_id_returns_invalid_params() {
    let t = build_ctx();

    let params = serde_json::json!({ "pair_id": "not-hex" });
    let err = t
        .ctx
        .get_oracle_price(params)
        .await
        .expect_err("expected invalid_params");
    assert_eq!(err.code, -32602, "invalid_params code expected");
}

// ---------- partition: pair_id = missing ----------
#[tokio::test]
async fn m9_missing_pair_id_returns_invalid_params() {
    let t = build_ctx();

    let params = serde_json::json!({});
    let err = t
        .ctx
        .get_oracle_price(params)
        .await
        .expect_err("expected invalid_params");
    assert_eq!(err.code, -32602);
}

// ============================================================================
// M10 fixtures (BlockStore + StateDb)
// ============================================================================

/// Extended test context with an attached StateDb (mainnet defaults).
/// `block_store` is exposed so tests can `put_block_canonical` blocks.
struct M10Ctx {
    ctx: RpcContext,
    block_store: Arc<storage::BlockStore>,
    state_db: Arc<StateDb>,
    _tempdirs: Vec<TempDir>,
}

fn build_m10_ctx() -> M10Ctx {
    let block_dir = TempDir::new().expect("blockstore tempdir");
    let state_dir = TempDir::new().expect("statedb tempdir");

    let chain_state = Arc::new(RwLock::new(ChainState::new(Hash::ZERO)));
    let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));
    let block_store = Arc::new(BlockStore::open(block_dir.path()).expect("blockstore"));
    let state_db = Arc::new(StateDb::open(state_dir.path()).expect("statedb"));
    let params = doli_core::consensus::ConsensusParams::default();
    let mempool = Arc::new(RwLock::new(Mempool::new(
        MempoolPolicy::default(),
        params.clone(),
        Network::Mainnet,
    )));

    let ctx = RpcContext::new_for_network(
        chain_state,
        block_store.clone(),
        utxo_set,
        mempool,
        params,
        Network::Mainnet,
    )
    .with_state_db(state_db.clone());

    M10Ctx {
        ctx,
        block_store,
        state_db,
        _tempdirs: vec![block_dir, state_dir],
    }
}

/// Build a PriceAttestation tx for testing. Signature is a dummy
/// (all-zero) — the RPC handler does NOT verify signatures; that is
/// M4's job at consensus-validation time. The handler only decodes
/// the payload and reads its fields.
fn make_attestation_tx(
    signer_pubkey_bytes: [u8; 32],
    pair_id: crypto::Hash,
    epoch_number: u64,
    price_cents: u64,
) -> Transaction {
    let data = PriceAttestationData {
        signer_pubkey: crypto::PublicKey::from_bytes(signer_pubkey_bytes),
        price_cents,
        pair_id,
        epoch_number,
        signature: crypto::Signature::from_bytes([0u8; 64]),
    };
    Transaction::new_price_attestation(data)
}

/// Minimal header for test blocks — all hash fields ZERO, slot taken
/// from height. Signature, VDF, etc. are not exercised by M10.
fn make_test_header(slot: u32) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 1000 + slot as u64 * 10,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: VdfOutput { value: Vec::new() },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

fn insert_attestation_block(
    block_store: &Arc<storage::BlockStore>,
    height: u64,
    txs: Vec<Transaction>,
) {
    let header = make_test_header(height as u32);
    let block = Block::new(header, txs);
    block_store
        .put_block_canonical(&block, height)
        .expect("put_block_canonical");
}

fn persist_bond_snapshot(
    state_db: &Arc<StateDb>,
    snapshot: std::collections::HashMap<crypto::Hash, u64>,
    epoch: u64,
) {
    let mut batch = state_db.begin_batch();
    batch.put_epoch_bond_snapshot(&snapshot, epoch);
    batch.commit().expect("commit bond_snapshot");
}

// ============================================================================
// M10: getOracleAttestations tests
// ============================================================================

// ---------- partition: 3 attestations + matching bond_snapshot ----------
#[tokio::test]
async fn m10_happy_path_three_attestations_with_bond_weights() {
    let t = build_m10_ctx();
    let pair_id = pair_id_fixture();
    let epoch: u64 = 1;
    // Mainnet blocks_per_reward_epoch = 360. Epoch 1 spans [360, 720).
    let start = 360_u64;

    // Three distinct attesters with deterministic pubkeys
    let pk_a = [0xAAu8; 32];
    let pk_b = [0xBBu8; 32];
    let pk_c = [0xCCu8; 32];

    insert_attestation_block(
        &t.block_store,
        start,
        vec![make_attestation_tx(pk_a, pair_id, epoch, 100)],
    );
    insert_attestation_block(
        &t.block_store,
        start + 1,
        vec![make_attestation_tx(pk_b, pair_id, epoch, 200)],
    );
    insert_attestation_block(
        &t.block_store,
        start + 2,
        vec![make_attestation_tx(pk_c, pair_id, epoch, 300)],
    );

    // Persist a bond_snapshot for epoch 1 with known weights.
    let h = |pk: [u8; 32]| crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &pk);
    let mut snap = std::collections::HashMap::new();
    snap.insert(h(pk_a), 1_000);
    snap.insert(h(pk_b), 2_000);
    snap.insert(h(pk_c), 3_000);
    persist_bond_snapshot(&t.state_db, snap, epoch);

    let params = serde_json::json!({
        "epoch": epoch,
        "pair_id": pair_id.to_hex()
    });
    let resp = t
        .ctx
        .get_oracle_attestations(params)
        .await
        .expect("M10 happy-path Ok");

    assert_eq!(resp["epoch"].as_u64().unwrap(), epoch);
    assert_eq!(resp["pair_id"].as_str().unwrap(), pair_id.to_hex());
    let atts = resp["attestations"].as_array().expect("attestations array");
    assert_eq!(atts.len(), 3, "expected 3 attestations, got {}", atts.len());

    // Build a lookup by attester_pubkey to verify all three are present
    // with correct prices and bond_weights.
    let mut by_pubkey: std::collections::HashMap<String, (u64, u64)> =
        std::collections::HashMap::new();
    for a in atts {
        let pk = a["attester_pubkey"].as_str().unwrap().to_string();
        let price = a["price_cents"].as_u64().unwrap();
        let bw = a["bond_weight"].as_u64().unwrap();
        // attester_pubkey_hash must be exactly hash_with_domain(ADDRESS_DOMAIN, pubkey)
        let pk_bytes = hex::decode(&pk).unwrap();
        let expected_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &pk_bytes);
        assert_eq!(
            a["attester_pubkey_hash"].as_str().unwrap(),
            expected_hash.to_hex(),
            "attester_pubkey_hash must match hash_with_domain(ADDRESS_DOMAIN, pubkey)"
        );
        by_pubkey.insert(pk, (price, bw));
    }
    assert_eq!(by_pubkey[&hex::encode(pk_a)], (100, 1_000));
    assert_eq!(by_pubkey[&hex::encode(pk_b)], (200, 2_000));
    assert_eq!(by_pubkey[&hex::encode(pk_c)], (300, 3_000));
}

// ---------- partition: unknown epoch (no blocks in range) ----------
#[tokio::test]
async fn m10_unknown_epoch_returns_empty_list() {
    let t = build_m10_ctx();
    let pair_id = pair_id_fixture();
    let params = serde_json::json!({
        "epoch": 9_999_u64,
        "pair_id": pair_id.to_hex()
    });
    let resp = t.ctx.get_oracle_attestations(params).await.unwrap();
    let atts = resp["attestations"].as_array().unwrap();
    assert!(
        atts.is_empty(),
        "unknown epoch must return empty list, got {:?}",
        atts
    );
}

// ---------- partition: future epoch (> current_epoch) ----------
#[tokio::test]
async fn m10_future_epoch_returns_empty_list() {
    let t = build_m10_ctx();
    let pair_id = pair_id_fixture();
    // Chain state height = 0 (default), so any epoch > 0 is "future".
    let params = serde_json::json!({
        "epoch": 42_u64,
        "pair_id": pair_id.to_hex()
    });
    let resp = t.ctx.get_oracle_attestations(params).await.unwrap();
    assert!(resp["attestations"].as_array().unwrap().is_empty());
}

// ---------- partition: blocks exist but no attestations for this pair ----------
#[tokio::test]
async fn m10_pair_id_without_attestations_returns_empty_list() {
    let t = build_m10_ctx();
    let queried_pair = pair_id_fixture();
    let other_pair = crypto::hash::hash_with_domain(b"ORACLE_PAIR", b"OTHER/USD");
    let epoch: u64 = 1;
    let start = 360_u64;

    // Insert an attestation for a DIFFERENT pair in the queried epoch's range.
    insert_attestation_block(
        &t.block_store,
        start,
        vec![make_attestation_tx([0xDDu8; 32], other_pair, epoch, 100)],
    );

    let params = serde_json::json!({
        "epoch": epoch,
        "pair_id": queried_pair.to_hex()
    });
    let resp = t.ctx.get_oracle_attestations(params).await.unwrap();
    assert!(resp["attestations"].as_array().unwrap().is_empty());
}

// ---------- partition: determinism — same query twice → byte-equal ----------
#[tokio::test]
async fn m10_determinism_repeated_query_byte_equal() {
    let t = build_m10_ctx();
    let pair_id = pair_id_fixture();
    let epoch: u64 = 1;
    let start = 360_u64;

    // Two attestations in non-sorted order by pubkey; M10 must sort
    // deterministically by attester_pubkey_hash.
    insert_attestation_block(
        &t.block_store,
        start,
        vec![
            make_attestation_tx([0xFFu8; 32], pair_id, epoch, 100),
            make_attestation_tx([0x11u8; 32], pair_id, epoch, 200),
        ],
    );

    let params = serde_json::json!({
        "epoch": epoch,
        "pair_id": pair_id.to_hex()
    });
    let resp1 = t.ctx.get_oracle_attestations(params.clone()).await.unwrap();
    let resp2 = t.ctx.get_oracle_attestations(params).await.unwrap();

    // Byte-equal serialization confirms deterministic ordering.
    let s1 = serde_json::to_string(&resp1).unwrap();
    let s2 = serde_json::to_string(&resp2).unwrap();
    assert_eq!(
        s1, s2,
        "two identical M10 queries must return byte-equal JSON"
    );
}

// ---------- partition: bond_weight null when snapshot epoch mismatch ----------
#[tokio::test]
async fn m10_bond_weight_null_when_snapshot_epoch_mismatch() {
    let t = build_m10_ctx();
    let pair_id = pair_id_fixture();
    let queried_epoch: u64 = 1;
    let other_epoch: u64 = 5;
    let start = 360_u64; // epoch 1

    insert_attestation_block(
        &t.block_store,
        start,
        vec![make_attestation_tx(
            [0xAAu8; 32],
            pair_id,
            queried_epoch,
            100,
        )],
    );

    // Persist a bond_snapshot for a DIFFERENT epoch.
    let h = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &[0xAAu8; 32]);
    let mut snap = std::collections::HashMap::new();
    snap.insert(h, 999);
    persist_bond_snapshot(&t.state_db, snap, other_epoch);

    let params = serde_json::json!({
        "epoch": queried_epoch,
        "pair_id": pair_id.to_hex()
    });
    let resp = t.ctx.get_oracle_attestations(params).await.unwrap();
    let atts = resp["attestations"].as_array().unwrap();
    assert_eq!(atts.len(), 1);
    assert!(
        atts[0]["bond_weight"].is_null(),
        "bond_weight must be null when persisted snapshot epoch ({}) != queried epoch ({}), got {:?}",
        other_epoch,
        queried_epoch,
        atts[0]["bond_weight"]
    );
}

// ---------- partition: duplicate attestations from same signer → keep latest ----------
#[tokio::test]
async fn m10_duplicate_signer_keeps_latest_price() {
    let t = build_m10_ctx();
    let pair_id = pair_id_fixture();
    let epoch: u64 = 1;
    let start = 360_u64;
    let pk = [0xAAu8; 32];

    // Two attestations from the SAME signer at different heights —
    // defense-in-depth dedup must keep the LAST one (price=999).
    insert_attestation_block(
        &t.block_store,
        start,
        vec![make_attestation_tx(pk, pair_id, epoch, 100)],
    );
    insert_attestation_block(
        &t.block_store,
        start + 5,
        vec![make_attestation_tx(pk, pair_id, epoch, 999)],
    );

    let params = serde_json::json!({
        "epoch": epoch,
        "pair_id": pair_id.to_hex()
    });
    let resp = t.ctx.get_oracle_attestations(params).await.unwrap();
    let atts = resp["attestations"].as_array().unwrap();
    assert_eq!(
        atts.len(),
        1,
        "dedup must collapse same-signer to single entry"
    );
    assert_eq!(
        atts[0]["price_cents"].as_u64().unwrap(),
        999,
        "latest-wins: expected price from the higher-height block"
    );
}

// ---------- partition: missing epoch param ----------
#[tokio::test]
async fn m10_missing_epoch_returns_invalid_params() {
    let t = build_m10_ctx();
    let pair_id = pair_id_fixture();
    let params = serde_json::json!({ "pair_id": pair_id.to_hex() });
    let err = t
        .ctx
        .get_oracle_attestations(params)
        .await
        .expect_err("expected invalid_params");
    assert_eq!(err.code, -32602);
}
