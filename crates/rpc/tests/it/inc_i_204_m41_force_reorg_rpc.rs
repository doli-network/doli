//! INC-I-204 M4.1 / REQ-FORK-012 — the `forceReorgTo` RPC boundary. TESTS-FIRST (RED).
//!
//! REQ-FORK-012 — Decision: a failure means the only operator-facing entry point to
//! the wedge escape either does not exist, accepts garbage as a chain hash, or arms
//! nothing — so the replacement for LB-4 is unreachable in an incident.
//!
//! SCOPE LIMIT, stated so nobody reads a false guarantee into this file:
//! `RpcContext::handle_request` bypasses `check_admin_auth` entirely (the auth
//! decision lives in the axum layer, `server.rs:189-241`). No test in this file can
//! cover the admin gate. That assertion lives in `crates/rpc/src/server.rs`.
//!
//! OUTPUT CONTRACT — `async fn RpcContext::force_reorg_to(&self, Value)
//!                       -> Result<Value, RpcError>`, driven through `handle_request`
//!   O1 mutable params: none.
//!   O2 receiver mutation: none on `RpcContext` itself; the ONLY state change is
//!      through the shared `Arc<RwLock<SyncManager>>` — the armed directive.
//!   O3 return value: `JsonRpcResponse.result` on success, `.error` otherwise.
//!   O4 persistent store: NONE — the directive is memory-only (C9).
//!   O5 statics: none.  O6 events: tracing only.
//!   PATHS: P1 well-formed hash -> armed, `result` present ·
//!          P2 malformed hex    -> `invalid_params` (-32602), nothing armed ·
//!          P3 params absent    -> `invalid_params`, nothing armed ·
//!          P4 no SyncManager attached -> an error, never a silent success.
//!   INPUT PARTITIONS: hash ∈ {64 valid hex, "not-a-hash", "" , 63 hex chars,
//!   66 hex chars, non-hex unicode}; params ∈ {object, null}.
//!   MATRIX: {O2, O3} x {P1, P2, P3, P4} — all cells asserted.

use std::sync::Arc;

use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use mempool::{Mempool, MempoolPolicy};
use network::{SyncConfig, SyncManager};
use rpc::types::JsonRpcRequest;
use rpc::RpcContext;
use serde_json::{json, Value};
use storage::{BlockStore, ChainState, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;

struct Harness {
    ctx: RpcContext,
    sync: Option<Arc<RwLock<SyncManager>>>,
    _tempdir: TempDir,
}

fn harness(with_sync_manager: bool) -> Harness {
    let tempdir = TempDir::new().expect("tempdir");
    let network = Network::Devnet;
    let params = ConsensusParams::for_network(network);
    let chain_state = Arc::new(RwLock::new(ChainState::new(Hash::ZERO)));
    let utxo_set = Arc::new(RwLock::new(UtxoSet::new()));
    let block_store = Arc::new(BlockStore::open(tempdir.path()).expect("blockstore"));
    let mempool = Arc::new(RwLock::new(Mempool::new(
        MempoolPolicy::default(),
        params.clone(),
        network,
    )));

    let mut ctx =
        RpcContext::new_for_network(chain_state, block_store, utxo_set, mempool, params, network);

    let sync = if with_sync_manager {
        let mgr = Arc::new(RwLock::new(SyncManager::new(
            SyncConfig::default(),
            Hash::ZERO,
        )));
        ctx = ctx.with_sync_manager(mgr.clone());
        Some(mgr)
    } else {
        None
    };

    Harness {
        ctx,
        sync,
        _tempdir: tempdir,
    }
}

async fn call(h: &Harness, params: Value) -> rpc::types::JsonRpcResponse {
    h.ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "forceReorgTo".to_string(),
            params,
            id: Value::from(1),
        })
        .await
}

/// REQ-FORK-012 — Decision: a failure means the escape has no operator entry point
/// at all, or arms something other than the hash the operator typed.
#[tokio::test]
async fn a_well_formed_hash_arms_exactly_that_target() {
    let h = harness(true);
    let target = crypto::hash::hash(b"inc_i204_m41_rpc_target");

    let response = call(&h, json!({ "hash": target.to_hex() })).await;
    assert!(
        response.error.is_none(),
        "P1: a 64-hex chain hash must be accepted; got {:?}",
        response.error
    );
    assert!(
        response.result.is_some(),
        "P1: the operator must get a result confirming what was armed"
    );

    let sync = h.sync.as_ref().expect("harness attached a SyncManager");
    assert_eq!(
        sync.read().await.force_reorg_target(),
        Some(target),
        "O2: the RPC must arm the EXACT hash it was given, on the shared SyncManager \
         the Node polls — that shared handle is the whole plumbing story"
    );
}

/// REQ-FORK-012 — Decision: a failure means a truncated paste or a shell-mangled
/// argument is silently coerced into SOME hash, and the node force-reorgs toward a
/// branch the operator never named.
#[tokio::test]
async fn a_malformed_hash_is_invalid_params_and_arms_nothing() {
    for bad in [
        json!({ "hash": "not-a-hash" }),
        json!({ "hash": "" }),
        json!({ "hash": "0".repeat(63) }),
        json!({ "hash": "0".repeat(66) }),
        json!({ "hash": "ZZ".repeat(32) }),
        json!({ "hash": "café".repeat(16) }),
        json!({ "hash": 12345 }),
        json!({}),
        Value::Null,
    ] {
        let h = harness(true);
        let response = call(&h, bad.clone()).await;

        let err = response
            .error
            .unwrap_or_else(|| panic!("P2/P3: `{bad}` must be REJECTED, not accepted"));
        assert_eq!(
            err.code, -32602,
            "P2/P3: `{bad}` must fail as invalid_params (-32602), not as an internal \
             error that hides which side was wrong"
        );
        assert_eq!(
            h.sync
                .as_ref()
                .expect("harness attached a SyncManager")
                .read()
                .await
                .force_reorg_target(),
            None,
            "P2/P3: a rejected request must leave NOTHING armed"
        );
    }
}

/// REQ-FORK-012 — Decision: a failure means a node whose RPC context has no
/// SyncManager reports the rescue as armed and then never acts on it, which in an
/// incident is worse than an error.
#[tokio::test]
async fn a_context_without_a_sync_manager_errors_rather_than_lying() {
    let h = harness(false);
    let target = crypto::hash::hash(b"inc_i204_m41_rpc_target");

    let response = call(&h, json!({ "hash": target.to_hex() })).await;
    assert!(
        response.error.is_some(),
        "P4: without the shared SyncManager there is nothing to arm — say so"
    );
}
