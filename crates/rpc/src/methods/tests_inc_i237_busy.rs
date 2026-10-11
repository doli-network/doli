//! INC-I-237 M1.5 (R3, F-15) — RPC submit paths reserve the broadcast slot
//! BEFORE mempool admission; a refused reservation is a retryable Busy error.
//!
//! OUTPUT CONTRACT: fn RpcContext::{send_transaction, submit_maintainer_change,
//!   submit_vote}(&self, params) -> Result<Value, RpcError>
//!   O1: none   O2: none
//!   O3: Ok(hash | accepted | submitted) / Err(RpcError {code, data.error_code,
//!       data.stage, data.retryable})
//!   O4: mempool (contains / len)
//!   O5: none
//!   O6: broadcast hook — reserve calls; permit invoked (with which tx) or dropped
//!       uninvoked; vote hook calls (with which bytes)
//! PATHS: P1 reserve None -> Busy; P2 reserve Some + admission ok -> permit(tx)
//!   once; P3 reserve Some + admission error -> permit dropped uninvoked;
//!   P4 vote hook false -> Busy; P5 vote hook true -> submitted
//! INPUT PARTITIONS: tx {valid 0-in/0-out maintainer tx, duplicate of an admitted
//!   tx, transfer spending a missing UTXO}; path {sendTransaction,
//!   submitMaintainerChange, submitVote}; hook {free, busy}
//! MATRIX: O3 x P1-P5, O4 x P1-P3, O6 x P1-P5 — all asserted below.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use super::*;
use crate::error::{codes, RpcError};
use crate::TxBroadcastPermit;
use doli_core::network::Network;
use doli_core::{Input, Output, Transaction};
use mempool::{Mempool, MempoolPolicy};
use serde_json::Value;
use storage::{BlockStore, ChainState, ProducerInfo, ProducerSet, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

#[derive(Default)]
struct Probe {
    reserves: AtomicUsize,
    released: AtomicUsize,
    invoked: Mutex<Vec<Transaction>>,
}

struct ReleaseMark(Arc<Probe>);

impl Drop for ReleaseMark {
    fn drop(&mut self) {
        self.0.released.fetch_add(1, SeqCst);
    }
}

fn reserve_hook(
    probe: Arc<Probe>,
    free: bool,
) -> impl Fn() -> Option<TxBroadcastPermit> + Send + Sync + 'static {
    move || {
        probe.reserves.fetch_add(1, SeqCst);
        if !free {
            return None;
        }
        let mark = ReleaseMark(probe.clone());
        let p = probe.clone();
        Some(Box::new(move |tx: Transaction| {
            let _held = &mark;
            p.invoked.lock().expect("probe").push(tx);
        }) as TxBroadcastPermit)
    }
}

fn base_ctx() -> (RpcContext, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let params = doli_core::consensus::ConsensusParams::default();
    let mempool = Mempool::new(MempoolPolicy::default(), params.clone(), Network::Mainnet);
    let ctx = RpcContext::new_for_network(
        Arc::new(RwLock::new(ChainState::new(crypto::Hash::ZERO))),
        Arc::new(BlockStore::open(dir.path()).expect("blockstore")),
        Arc::new(RwLock::new(UtxoSet::new())),
        Arc::new(RwLock::new(mempool)),
        params,
        Network::Mainnet,
    );
    (ctx, dir)
}

fn ctx_with(probe: &Arc<Probe>, free: bool) -> (RpcContext, TempDir) {
    let (ctx, dir) = base_ctx();
    (ctx.with_broadcast(reserve_hook(probe.clone(), free)), dir)
}

fn sigs(n: u8) -> Vec<doli_core::maintainer::MaintainerSignature> {
    (1..=n)
        .map(|i| {
            let kp = crypto::KeyPair::from_seed([i; 32]);
            doli_core::maintainer::MaintainerSignature {
                pubkey: *kp.public_key(),
                signature: crypto::signature::sign(b"inc-i-237 busy", kp.private_key()),
            }
        })
        .collect()
}

fn maintainer_tx(seed: u8) -> Transaction {
    let target = *crypto::KeyPair::from_seed([seed; 32]).public_key();
    Transaction::new_add_maintainer(target, sigs(3))
}

fn send_params(tx: &Transaction) -> Value {
    serde_json::json!({ "tx": hex::encode(tx.serialize()) })
}

fn change_params(seed: u8) -> Value {
    let sig_json: Vec<Value> = sigs(3)
        .iter()
        .map(|s| serde_json::json!({ "pubkey": s.pubkey.to_hex(), "signature": s.signature.to_hex() }))
        .collect();
    serde_json::json!({
        "action": "add",
        "target_pubkey": crypto::KeyPair::from_seed([seed; 32]).public_key().to_hex(),
        "signatures": sig_json,
    })
}

fn assert_busy(err: &RpcError) {
    assert_eq!(err.code, codes::MEMPOOL_FULL, "Busy reuses -32004: {err:?}");
    let data = err.data.as_ref().expect("Busy carries data");
    assert_eq!(data["error_code"], "BROADCAST_BUSY", "{err:?}");
    assert_eq!(data["stage"], "broadcast", "{err:?}");
    assert_eq!(data["retryable"], true, "{err:?}");
}

fn assert_not_busy(err: &RpcError) {
    let code = err.data.as_ref().map(|d| d["error_code"].clone());
    assert_ne!(
        code,
        Some(Value::from("BROADCAST_BUSY")),
        "admission error expected: {err:?}"
    );
}

// REQ-GRV-007 / F-15 — Decision: a failure means a Busy broadcast slot still admits the tx to the mempool (admitted but never broadcast, stranded until it expires) or answers without the retryable Busy shape.
#[tokio::test]
async fn req_grv_007_f15_send_transaction_busy_is_retryable_and_leaves_mempool_untouched() {
    let probe = Arc::new(Probe::default());
    let (ctx, _dir) = ctx_with(&probe, false);
    let tx = maintainer_tx(0xa1);
    let err = ctx
        .send_transaction(send_params(&tx))
        .await
        .expect_err("Busy");
    assert_busy(&err);
    assert_eq!(probe.reserves.load(SeqCst), 1);
    let pool = ctx.mempool.read().await;
    assert!(
        !pool.contains(&tx.hash()) && pool.is_empty(),
        "Busy must not admit"
    );
}

// REQ-GRV-007 / F-15 — Decision: a failure means an admitted tx is broadcast zero or two times, or a different tx is broadcast than the one admitted.
#[tokio::test]
async fn req_grv_007_f15_send_transaction_reserved_slot_admits_then_broadcasts_once() {
    let probe = Arc::new(Probe::default());
    let (ctx, _dir) = ctx_with(&probe, true);
    let tx = maintainer_tx(0xa2);
    let out = ctx
        .send_transaction(send_params(&tx))
        .await
        .expect("accepted");
    assert_eq!(out, Value::String(tx.hash().to_hex()));
    assert!(ctx.mempool.read().await.contains(&tx.hash()));
    let sent = probe.invoked.lock().expect("probe").clone();
    assert_eq!(sent.len(), 1, "permit invoked exactly once");
    assert_eq!(sent[0].hash(), tx.hash());
    assert_eq!(
        (probe.reserves.load(SeqCst), probe.released.load(SeqCst)),
        (1, 1)
    );
}

// REQ-GRV-007 / F-15 — Decision: a failure means a rejected tx is broadcast anyway (relay of invalid content) or its reserved slot leaks, so the command channel fills with phantom reservations.
#[tokio::test]
async fn req_grv_007_f15_send_transaction_admission_error_drops_permit_uninvoked() {
    let probe = Arc::new(Probe::default());
    let (ctx, _dir) = ctx_with(&probe, true);
    let tx = maintainer_tx(0xa3);
    ctx.send_transaction(send_params(&tx))
        .await
        .expect("first admit");
    let dup = ctx
        .send_transaction(send_params(&tx))
        .await
        .expect_err("duplicate");
    assert_not_busy(&dup);
    let missing = Transaction::new_transfer(
        vec![Input::new(crypto::hash::hash(b"inc-i-237 missing utxo"), 0)],
        vec![Output::normal(1, crypto::Hash::ZERO)],
    );
    let bad = ctx
        .send_transaction(send_params(&missing))
        .await
        .expect_err("missing input");
    assert_not_busy(&bad);
    assert!(!ctx.mempool.read().await.contains(&missing.hash()));
    assert_eq!(
        probe.invoked.lock().expect("probe").len(),
        1,
        "only the admitted tx"
    );
    assert_eq!(
        (probe.reserves.load(SeqCst), probe.released.load(SeqCst)),
        (3, 3)
    );
}

// REQ-GRV-007 / F-15 — Decision: a failure means submitMaintainerChange admits a governance tx it cannot broadcast (the INC-I-195 strand, now under load) or leaks the slot of a rejected one.
#[tokio::test]
async fn req_grv_007_f15_maintainer_change_reserves_before_admission() {
    let busy = Arc::new(Probe::default());
    let (ctx, _dir) = ctx_with(&busy, false);
    let err = ctx
        .submit_maintainer_change(change_params(0xb1))
        .await
        .expect_err("Busy");
    assert_busy(&err);
    assert_eq!(ctx.mempool.read().await.len(), 0, "Busy must not admit");

    let free = Arc::new(Probe::default());
    let (ctx, _dir2) = ctx_with(&free, true);
    let out = ctx
        .submit_maintainer_change(change_params(0xb2))
        .await
        .expect("accepted");
    assert_eq!(out["status"], "accepted");
    let dup = ctx
        .submit_maintainer_change(change_params(0xb2))
        .await
        .expect_err("dup");
    assert_not_busy(&dup);
    let sent = free.invoked.lock().expect("probe").clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(Value::String(sent[0].hash().to_hex()), out["tx_hash"]);
    assert_eq!(
        (free.reserves.load(SeqCst), free.released.load(SeqCst)),
        (2, 2)
    );
}

fn vote_ctx(accept: bool, seen: Arc<Mutex<Vec<Vec<u8>>>>) -> (RpcContext, TempDir, Value) {
    let kp = crypto::KeyPair::from_seed([0xc1; 32]);
    let pk = *kp.public_key();
    let key = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
    let info = ProducerInfo::new(pk, 10, 1_000_000_000, (crypto::Hash::ZERO, 0), 0, 0);
    let set = ProducerSet::from_parts(HashMap::from([(key, info)]), HashMap::new(), Vec::new());
    let (ctx, dir) = base_ctx();
    let ctx = ctx
        .with_producer_set(Arc::new(RwLock::new(set)))
        .with_broadcast_vote(move |bytes| {
            seen.lock().expect("votes").push(bytes);
            accept
        });
    let msg = "6.44.0:approve:1700000000";
    let sig = crypto::signature::sign(msg.as_bytes(), kp.private_key());
    let params = serde_json::json!({ "vote": {
        "version": "6.44.0", "vote": "approve", "producerId": pk.to_hex(),
        "timestamp": 1_700_000_000u64, "signature": sig.to_hex(),
    }});
    (ctx, dir, params)
}

// REQ-GRV-007 / F-15 — Decision: a failure means a vote the node could not hand to the swarm is reported as broadcast, so the producer believes its veto is on the network when it was dropped.
#[tokio::test]
async fn req_grv_007_f15_submit_vote_busy_when_the_vote_slot_is_refused() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (ctx, _dir, params) = vote_ctx(false, seen.clone());
    let err = ctx.submit_vote(params).await.expect_err("Busy");
    assert_busy(&err);
    assert_eq!(
        seen.lock().expect("votes").len(),
        1,
        "the hook was asked once"
    );

    let seen = Arc::new(Mutex::new(Vec::new()));
    let (ctx, _dir2, params) = vote_ctx(true, seen.clone());
    let out = ctx.submit_vote(params).await.expect("submitted");
    assert_eq!(out["status"], "submitted");
    let votes = seen.lock().expect("votes").clone();
    assert_eq!(votes.len(), 1);
    let v: Value = serde_json::from_slice(&votes[0]).expect("vote json");
    assert_eq!(v["version"], "6.44.0");
}
