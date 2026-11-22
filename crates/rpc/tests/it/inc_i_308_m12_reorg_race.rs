//! INC-I-308 M12 — a backfill write re-derives its anchor from the CURRENT store; a reorg during the fetch stops the pass.
//!
// covers: crates/rpc/src/methods/backfill_anchor.rs
//
// OUTPUT CONTRACT:
//   O1 block_store height index at the target height; O2 backfillStatus {error}
//   PATHS: store_at re-check fails (chain moved) -> no write, no remove, error naming the height
// INPUT PARTITIONS: reorg rewrites index + tip at the target height while the peer serves it
// MATRIX: O1,O2 x chain moved -> m12_backfill_reorg_during_fetch_keeps_new_entry

use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use crypto::{Hash, KeyPair};
use doli_core::Block;
use rpc::RpcContext;
use serde_json::{json, Value};
use storage::{BlockStore, ChainState};
use tempfile::TempDir;
use tokio::sync::RwLock;

use super::audit_rpc_effects::{block, call, local_node, wait_done};

struct ReorgPeer {
    tip_hash: Hash,
    serve: Block,
    store: Arc<BlockStore>,
    chain: Arc<RwLock<ChainState>>,
    reorg: Vec<(u64, Block)>,
}

async fn reorg_peer(State(p): State<Arc<ReorgPeer>>, Json(req): Json<Value>) -> Json<Value> {
    let id = req["id"].clone();
    let ok = |r: Value| json!({"jsonrpc": "2.0", "id": id, "result": r});
    let none = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "stub"}});
    let height = req["params"]["height"].as_u64().unwrap_or(0);
    Json(match req["method"].as_str() {
        Some("getBlockByHeight") if req["params"].is_object() => {
            ok(json!({"hash": p.tip_hash.to_hex()}))
        }
        Some("getBlockRaw") if height == p.serve.header.slot as u64 => {
            for (h, b) in &p.reorg {
                p.store.put_block_canonical(b, *h).unwrap();
            }
            let (top_h, top) = p.reorg.last().unwrap();
            {
                let mut cs = p.chain.write().await;
                cs.best_height = *top_h;
                cs.best_hash = top.hash();
            }
            let data = bincode::serialize(&p.serve).unwrap();
            ok(
                json!({"block": base64::engine::general_purpose::STANDARD.encode(&data),
                      "blake3": crypto::hash::hash(&data).to_string(), "height": height}),
            )
        }
        _ => none,
    })
}

async fn serve(p: ReorgPeer) -> String {
    let app = Router::new()
        .route("/", post(reorg_peer))
        .with_state(Arc::new(p));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn idx(ctx: &RpcContext, h: u64) -> Option<Hash> {
    ctx.block_store.get_hash_by_height(h).unwrap()
}

// INC-I-308-M12 — Decision: a failure means a stale anchor overwrites the index entry a concurrent reorg just wrote.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m12_backfill_reorg_during_fetch_keeps_new_entry() {
    let g = crypto::hash::hash(b"inc-i-308-m12");
    let (a, b) = (
        KeyPair::from_seed([0x5A; 32]),
        KeyPair::from_seed([0x6B; 32]),
    );
    let h1 = block(g, 1, g, &a);
    let h2 = block(g, 2, h1.hash(), &a);
    let h3 = block(g, 3, h2.hash(), &a);
    let r2 = block(g, 2, h1.hash(), &b);
    let r3 = block(g, 3, r2.hash(), &b);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 3, h3.hash());
    ctx.block_store.put_block_canonical(&h1, 1).unwrap();
    ctx.block_store.put_block_canonical(&h3, 3).unwrap();
    let url = serve(ReorgPeer {
        tip_hash: h3.hash(),
        serve: h2.clone(),
        store: ctx.block_store.clone(),
        chain: ctx.chain_state.clone(),
        reorg: vec![(2, r2.clone()), (3, r3.clone())],
    })
    .await;
    let reply = call(&ctx, "backfillFromPeer", json!({"rpc_url": url})).await;
    assert_eq!(reply["started"], json!(true), "{reply}");
    let st = wait_done(&ctx).await;
    assert_eq!(
        idx(&ctx, 2),
        Some(r2.hash()),
        "the reorg's entry at h2 was replaced by the stale anchor's block; status {st}"
    );
    let err = st["error"].as_str().unwrap_or("");
    assert!(
        err.contains("height 2") && err.contains("chain moved"),
        "status must report the chain moved at height 2: {st}"
    );
}
