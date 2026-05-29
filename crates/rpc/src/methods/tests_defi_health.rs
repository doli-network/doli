//! Tests for D4 `getDefiHealthMetric` (AC-6 monitoring metric).
//!
//! Included via `#[path = "tests_defi_health.rs"] mod tests;` in
//! `defi_health.rs`, so the logical path is
//! `methods::defi_health::tests` — `super::*` resolves to the parent
//! `defi_health` module.
//!
//! OUTPUT CONTRACT (mirrors defi_health.rs header):
//!   getDefiHealthMetric -> Result<Value, RpcError> where Value is
//!     {
//!       totalActiveBonds: u64,
//!       maxPoolTvl: u64,
//!       maxPoolId: hex32 | null,
//!       bondToTvlRatio: f64 | null,
//!       status: "ok" | "degraded" | "no_pools",
//!       disclosure: string,
//!       note: string
//!     }
//!
//! INPUT PARTITIONS:
//!   utxo_state = { empty, bonds_only, pools_only, ratio_ok,
//!                  ratio_degraded, multi_pool_max, dup_pool_dedup,
//!                  overflow_safe }

use super::*;
use std::sync::Arc;
use tokio::sync::RwLock;

use crypto::Hash;
use doli_core::network::Network;
use doli_core::transaction::Output;
use mempool::{Mempool, MempoolPolicy};
use storage::{BlockStore, ChainState, Outpoint, UtxoEntry, UtxoSet};
use tempfile::TempDir;

struct TestCtx {
    ctx: RpcContext,
    utxo_set: Arc<RwLock<UtxoSet>>,
    _tempdir: TempDir,
}

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
        chain_state,
        block_store,
        utxo_set.clone(),
        mempool,
        params,
        Network::Mainnet,
    );
    TestCtx {
        ctx,
        utxo_set,
        _tempdir: tempdir,
    }
}

fn fake_outpoint(seed: u8) -> Outpoint {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    Outpoint::new(Hash::from_bytes(bytes), 0)
}

async fn insert_bond(utxo_set: &Arc<RwLock<UtxoSet>>, seed: u8, amount: u64) {
    let mut pkh = [0u8; 32];
    pkh[31] = seed; // owner key differs per bond
    let output = Output::bond(amount, Hash::from_bytes(pkh), u64::MAX, 0);
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo_set
        .write()
        .await
        .insert(fake_outpoint(seed), entry)
        .expect("insert bond");
}

async fn insert_pool(utxo_set: &Arc<RwLock<UtxoSet>>, seed: u8, reserve_a: u64) {
    let mut asset_b = [0u8; 32];
    asset_b[0] = seed;
    let asset_b_hash = Hash::from_bytes(asset_b);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b_hash, 30);
    let output = Output::pool(
        pool_id,
        asset_b_hash,
        reserve_a,
        reserve_a, // reserve_b irrelevant; metric uses 2*reserve_a
        1_000,
        0,
        0,
        30,
        0,
    );
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let mut op = fake_outpoint(seed);
    op.index = 1; // distinguish from bond outpoints
    utxo_set
        .write()
        .await
        .insert(op, entry)
        .expect("insert pool");
}

async fn insert_pool_dup(utxo_set: &Arc<RwLock<UtxoSet>>, seed: u8, reserve_a: u64, slot_seed: u8) {
    // Same pool_id (same seed → same asset_b), different outpoint → simulates
    // a duplicate appearing during a rollback artifact.
    let mut asset_b = [0u8; 32];
    asset_b[0] = seed;
    let asset_b_hash = Hash::from_bytes(asset_b);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b_hash, 30);
    let output = Output::pool(
        pool_id,
        asset_b_hash,
        reserve_a,
        reserve_a,
        1_000,
        0,
        0,
        30,
        0,
    );
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    let mut outpoint_bytes = [0u8; 32];
    outpoint_bytes[0] = slot_seed;
    outpoint_bytes[1] = 0xAA;
    let op = Outpoint::new(Hash::from_bytes(outpoint_bytes), 2);
    utxo_set
        .write()
        .await
        .insert(op, entry)
        .expect("insert dup pool");
}

// ---------------------------------------------------------------------------
// Partition 1: empty UTXO set → no_pools
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_pools_returns_no_pools_status() {
    let t = build_ctx();
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    assert_eq!(v["status"], "no_pools");
    assert_eq!(v["totalActiveBonds"], 0u64);
    assert_eq!(v["maxPoolTvl"], 0u64);
    assert!(v["maxPoolId"].is_null());
    assert!(v["bondToTvlRatio"].is_null());
    assert!(v["disclosure"].as_str().unwrap().contains("No AMM pools"));
    assert!(v["note"].as_str().unwrap().contains("self-referential"));
}

// ---------------------------------------------------------------------------
// Partition 2: bonds only, no pools → no_pools, but bond sum reported
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bonds_only_no_pools() {
    let t = build_ctx();
    insert_bond(&t.utxo_set, 1, 10_000_000_000).await;
    insert_bond(&t.utxo_set, 2, 20_000_000_000).await;
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    assert_eq!(v["status"], "no_pools");
    assert_eq!(v["totalActiveBonds"], 30_000_000_000u64);
    assert_eq!(v["maxPoolTvl"], 0u64);
    assert!(v["bondToTvlRatio"].is_null());
}

// ---------------------------------------------------------------------------
// Partition 3: pools only, no bonds → degraded, ratio == 0.0
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pools_only_no_bonds() {
    let t = build_ctx();
    insert_pool(&t.utxo_set, 1, 5_000_000_000).await;
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    assert_eq!(v["status"], "degraded");
    assert_eq!(v["totalActiveBonds"], 0u64);
    assert_eq!(v["maxPoolTvl"], 10_000_000_000u64); // 2 * reserve_a
    assert_eq!(v["bondToTvlRatio"].as_f64().unwrap(), 0.0);
}

// ---------------------------------------------------------------------------
// Partition 4: bonds >= max pool TVL → ok
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ratio_ok_when_bonds_exceed_max_pool() {
    let t = build_ctx();
    // bonds: 20M, max pool TVL: 10M (2 * 5M)
    insert_bond(&t.utxo_set, 1, 20_000_000).await;
    insert_pool(&t.utxo_set, 2, 5_000_000).await;
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    assert_eq!(v["status"], "ok");
    assert_eq!(v["totalActiveBonds"], 20_000_000u64);
    assert_eq!(v["maxPoolTvl"], 10_000_000u64);
    let ratio = v["bondToTvlRatio"].as_f64().unwrap();
    assert!((ratio - 2.0).abs() < 1e-9, "ratio was {}", ratio);
    assert!(v["disclosure"].as_str().unwrap().contains("intact"));
}

// ---------------------------------------------------------------------------
// Partition 5: bonds < max pool TVL → degraded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ratio_degraded_when_pool_exceeds_bonds() {
    let t = build_ctx();
    // bonds: 5M, max pool TVL: 10M (2 * 5M)
    insert_bond(&t.utxo_set, 1, 5_000_000).await;
    insert_pool(&t.utxo_set, 2, 5_000_000).await;
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    assert_eq!(v["status"], "degraded");
    let ratio = v["bondToTvlRatio"].as_f64().unwrap();
    assert!((ratio - 0.5).abs() < 1e-9, "ratio was {}", ratio);
    assert!(v["disclosure"].as_str().unwrap().contains("degraded"));
}

// ---------------------------------------------------------------------------
// Partition 6: multi-pool → metric picks the largest TVL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multiple_pools_picks_max() {
    let t = build_ctx();
    insert_pool(&t.utxo_set, 1, 1_000_000).await; // TVL = 2M
    insert_pool(&t.utxo_set, 2, 5_000_000).await; // TVL = 10M ← max
    insert_pool(&t.utxo_set, 3, 3_000_000).await; // TVL = 6M
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    assert_eq!(v["maxPoolTvl"], 10_000_000u64);
}

// ---------------------------------------------------------------------------
// Partition 7: TVL formula and `note` presence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_pool_tvl_uses_2x_reserve_a() {
    let t = build_ctx();
    insert_pool(&t.utxo_set, 1, 7_777_777).await;
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    assert_eq!(v["maxPoolTvl"], 15_555_554u64);
    let note = v["note"].as_str().unwrap();
    assert!(note.contains("self-referential"));
    assert!(note.contains("pool"));
}

// ---------------------------------------------------------------------------
// Partition 8: dedupe by pool_id — same pool_id appearing twice → max reserve_a wins
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dedupe_pool_by_pool_id() {
    let t = build_ctx();
    insert_pool(&t.utxo_set, 5, 1_000_000).await; // pool_id(5), reserve_a=1M
    insert_pool_dup(&t.utxo_set, 5, 4_000_000, 99).await; // same pool_id, reserve_a=4M
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    // After dedupe by pool_id picking max reserve_a (4M), TVL = 8M.
    assert_eq!(v["maxPoolTvl"], 8_000_000u64);
}

// ---------------------------------------------------------------------------
// Partition 9: bond sum overflow safety
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bond_sum_overflow_safe() {
    let t = build_ctx();
    // Two bonds whose sum exceeds u64::MAX → saturate at u64::MAX, no panic.
    insert_bond(&t.utxo_set, 1, u64::MAX).await;
    insert_bond(&t.utxo_set, 2, 1).await;
    let v = t
        .ctx
        .get_defi_health_metric(serde_json::Value::Null)
        .await
        .expect("call");
    assert_eq!(v["totalActiveBonds"], u64::MAX);
}
