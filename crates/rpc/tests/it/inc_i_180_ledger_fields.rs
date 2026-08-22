//! INC-I-180 M3 — RED reproduction for the RPC two-ledger masking defect.
//! covers: producer  (drives crates/rpc/src/methods/producer.rs)
//!
//! KIND: assertion-RED. `producerSetBondCount` is absent today; and getProducer
//! :81-84 / getProducers :186-190 fall back to info.bond_count (434) when
//! utxo_bond_count==0, MASKING the n11 divergence the operator must SEE.
//!
//! ── OUTPUT CONTRACT ─────────────────────────────────────────────────────────
//! Function under test: getProducer / getProducers JSON result.
//! OBSERVABLE OUTPUTS (JSON keys): O1 `bondCount` (UTXO-derived; 0 for n11);
//!   O2 `producerSetBondCount` (NEW additive = info.bond_count; 434 for n11);
//!   O3 `selectionWeight` (present; 434).  no mutable params; no store.
//! CODE PATHS: get_producer (:81-84) and get_producers (:186-190).
//! INPUT PARTITIONS: n11 zero-bond (bond_count>0, zero Bond UTXOs); aligned (equal
//!   bond_count and Bond-UTXO count).
//! MATRIX (O1..O3) × {getProducer, getProducers} × {n11, aligned}.
//! ────────────────────────────────────────────────────────────────────────────

use std::collections::HashMap;
use std::sync::Arc;

use crypto::{Hash, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::Output;
use mempool::{Mempool, MempoolPolicy};
use rpc::types::JsonRpcRequest;
use rpc::RpcContext;
use serde_json::{json, Value};
use storage::{BlockStore, ChainState, Outpoint, ProducerInfo, ProducerSet, UtxoEntry, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

const BOND_UNIT: u64 = 1_000_000_000;

fn key(seed: u8) -> PublicKey {
    *crypto::KeyPair::from_seed([seed; 32]).public_key()
}

// Seed a ProducerSet with ONE Active producer holding `bond_count` ProducerSet
// bonds. `bond_utxos` seeds real Bond UTXOs; 0 = the n11 zero-bond shape.
fn harness(bond_count: u32, bond_utxos: u32) -> (RpcContext, PublicKey, TempDir) {
    let tempdir = TempDir::new().expect("tempdir");
    let params = ConsensusParams::for_network(Network::Mainnet);
    let chain_state = Arc::new(RwLock::new(ChainState::new(Hash::ZERO)));
    let block_store = Arc::new(BlockStore::open(tempdir.path()).expect("blockstore"));
    let mempool = Arc::new(RwLock::new(Mempool::new(
        MempoolPolicy::default(),
        params.clone(),
        Network::Mainnet,
    )));

    let pk = key(11);

    // --- Bond UTXOs (empty for n11) --------------------------------------
    let mut utxo = UtxoSet::new();
    let owner = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
    for i in 0..bond_utxos {
        let mut seed = [0u8; 32];
        seed[0] = 200u8.wrapping_add(i as u8);
        let outpoint = Outpoint::new(crypto::hash::hash(&seed), 0);
        let output = Output::bond(BOND_UNIT, owner, u64::MAX, 0);
        utxo.insert(
            outpoint,
            UtxoEntry {
                output,
                height: 0,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("insert bond utxo");
    }
    let utxo_set = Arc::new(RwLock::new(utxo));

    // --- ProducerSet: one Active producer, bond_count, weight==bond_count -
    let mut producers: HashMap<Hash, ProducerInfo> = HashMap::new();
    let info = ProducerInfo::new_with_bonds(
        pk,
        0,
        bond_count as u64 * BOND_UNIT,
        (Hash::ZERO, 0),
        0,
        bond_count,
    );
    producers.insert(crypto::hash::hash(pk.as_bytes()), info);
    let ps = ProducerSet::from_parts(producers, HashMap::new(), Vec::new());

    let ctx = RpcContext::new_for_network(
        chain_state,
        block_store,
        utxo_set,
        mempool,
        params,
        Network::Mainnet,
    )
    .with_producer_set(Arc::new(RwLock::new(ps)));

    (ctx, pk, tempdir)
}

async fn get_producer(ctx: &RpcContext, pk: &PublicKey) -> Value {
    let resp = ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "getProducer".to_string(),
            params: json!({ "publicKey": hex::encode(pk.as_bytes()) }),
            id: Value::from(1),
        })
        .await;
    resp.result.expect("getProducer result")
}

async fn get_producers_first(ctx: &RpcContext) -> Value {
    let resp = ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "getProducers".to_string(),
            params: json!({ "active_only": false }),
            id: Value::from(1),
        })
        .await;
    let arr = resp.result.expect("getProducers result");
    arr.as_array()
        .expect("bare array")
        .first()
        .cloned()
        .expect("one producer")
}

/// REQ-I180-006 (Must). covers: producer. n11 shape via getProducer: bondCount==0
/// (UTXO-derived), producerSetBondCount==434 (new field), machine-detectable.
#[tokio::test]
async fn req_i180_006_get_producer_n11_shape_exposes_both_ledgers() {
    let (ctx, pk, _t) = harness(434, 0);
    let v = get_producer(&ctx, &pk).await;
    assert_eq!(
        v["bondCount"].as_u64(),
        Some(0),
        "REQ-I180-006: bondCount is UTXO-derived; n11 spent every Bond UTXO. The \
         `else info.bond_count` fallback returning 434 must be removed — it masks the \
         divergence the operator must SEE."
    );
    assert_eq!(
        v["producerSetBondCount"].as_u64(),
        Some(434),
        "REQ-I180-006: the new additive field surfaces info.bond_count so a client can \
         compute P and detect producerSetBondCount>0 while bondCount==0."
    );
    assert_eq!(
        v["selectionWeight"].as_u64(),
        Some(434),
        "REQ-I180-006: selectionWeight must be exposed so the client computes P."
    );
    assert!(
        v["producerSetBondCount"].as_u64().unwrap() > 0 && v["bondCount"].as_u64().unwrap() == 0
    );
}

/// REQ-I180-006 (Must). covers: producer. Same via getProducers (bare array).
#[tokio::test]
async fn req_i180_006_get_producers_n11_shape_exposes_both_ledgers() {
    let (ctx, _pk, _t) = harness(434, 0);
    let v = get_producers_first(&ctx).await;
    assert_eq!(v["bondCount"].as_u64(), Some(0));
    assert_eq!(v["producerSetBondCount"].as_u64(), Some(434));
    assert_eq!(v["selectionWeight"].as_u64(), Some(434));
}

/// REQ-I180-006 (Must). covers: producer. GREEN-lock: aligned producer (5 bonds
/// in both ledgers) reports equal counts — the new field is not a blanket 0.
#[tokio::test]
async fn req_i180_006_aligned_producer_reports_equal_counts() {
    let (ctx, pk, _t) = harness(5, 5);
    let v = get_producer(&ctx, &pk).await;
    assert_eq!(v["bondCount"].as_u64(), Some(5));
    assert_eq!(v["producerSetBondCount"].as_u64(), Some(5));
    assert_eq!(v["selectionWeight"].as_u64(), Some(5));
}
