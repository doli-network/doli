//! INC-I-308 M2 — handler effects behind the admin gate (P1-RPC-004, -005, -007, -008).
//! Driven through `RpcContext::handle_request`, which bypasses the HTTP admin gate.
//!
// covers: crates/rpc/src/methods/pruning.rs crates/rpc/src/methods/guardian.rs crates/rpc/src/methods/backfill.rs
// covers: crates/rpc/src/methods/balance.rs crates/rpc/src/methods/history.rs crates/rpc/src/methods/context.rs
// covers: crates/crypto/src/address.rs crates/storage/src/block_store/maintenance.rs crates/storage/src/block_store/writes.rs
//
// OUTPUT CONTRACT
//   O1 JSON-RPC reply (result | error.code); O2 block_store height index; O3 archive dir *.block files
//   PATHS: getBalance/getUtxos/getHistory bad address -> -32602 | pruneBlocks no covering archive -> refuse or keep,
//     force -> prune | repairArchiveFromPeer -> only decodable blocks <= local tip | backfillFromPeer -> anchored to h+1.prev_hash
// INPUT PARTITIONS
//   address: 19 ASCII + 'é' at bytes 19..21 + 'a'; prune: [2000] vs [2000, true], archive w/o manifest;
//   repair peer: tip 5 > local 2, garbage bytes, empty blake3; backfill peer: self-signed forged h3, prev = honest h2
// MATRIX
//   O1 x bad address -> audit_p1_rpc_008_* | O1,O2 x {no force, force} -> audit_p1_rpc_004_*
//   O3 x garbage/above-tip -> audit_p1_rpc_007_* | O2 x forged neighbor -> audit_p1_rpc_005_*

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::legacy::deserialize_block_compat;
use doli_core::validation::verify_block_signature;
use doli_core::{Block, BlockHeader};
use mempool::{Mempool, MempoolPolicy};
use rpc::types::JsonRpcRequest;
use rpc::RpcContext;
use serde_json::{json, Value};
use storage::{BlockStore, ChainState, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

const BAD_ADDR: &str = "aaaaaaaaaaaaaaaaaaa\u{e9}a";

fn block(g: Hash, h: u64, prev: Hash, kp: &KeyPair) -> Block {
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(&[]),
        presence_root: Hash::ZERO,
        genesis_hash: g,
        timestamp: 40_000 + h,
        slot: h as u32,
        producer: *kp.public_key(),
        vdf_output: VdfOutput {
            value: vec![h as u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let msg = header.signing_message();
    header.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
    assert!(verify_block_signature(&header).is_ok(), "fixture h={h}");
    Block::new(header, vec![])
}

fn local_node(store_dir: &Path, best_height: u64) -> RpcContext {
    let network = Network::Devnet;
    let params = ConsensusParams::for_network(network);
    let mut cs = ChainState::new(Hash::ZERO);
    cs.best_height = best_height;
    cs.best_hash = Hash::from_bytes([0x77; 32]);
    let block_store = Arc::new(BlockStore::open(store_dir).unwrap());
    let mempool = Mempool::new(MempoolPolicy::default(), params.clone(), network);
    RpcContext::new_for_network(
        Arc::new(RwLock::new(cs)),
        block_store,
        Arc::new(RwLock::new(UtxoSet::new())),
        Arc::new(RwLock::new(mempool)),
        params,
        network,
    )
}

async fn raw(ctx: &RpcContext, method: &str, params: Value) -> Value {
    let resp = ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Value::from(1),
        })
        .await;
    serde_json::to_value(&resp).unwrap()
}

async fn call(ctx: &RpcContext, method: &str, params: Value) -> Value {
    let v = raw(ctx, method, params).await;
    assert!(v.get("error").is_none_or(Value::is_null), "{method}: {v}");
    v["result"].clone()
}

async fn wait_done(ctx: &RpcContext) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let st = call(ctx, "backfillStatus", json!({})).await;
        if st["running"] == json!(false) {
            return st;
        }
        assert!(Instant::now() < deadline, "backfill did not finish: {st}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[derive(Clone)]
struct Peer {
    tip: u64,
    blocks: Vec<Block>,
    garbage: bool,
}

async fn peer_rpc(State(p): State<Arc<Peer>>, Json(req): Json<Value>) -> Json<Value> {
    let id = req["id"].clone();
    let height = req["params"]["height"].as_u64().unwrap_or(0);
    let ok = |r: Value| json!({"jsonrpc": "2.0", "id": id, "result": r});
    let none = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "stub"}});
    let pos = req["params"][0].as_u64().unwrap_or(0);
    Json(match req["method"].as_str() {
        Some("getChainInfo") => ok(json!({"bestHeight": p.tip})),
        Some("verifyChainIntegrity") => ok(json!({"chainCommitment": "ff"})),
        Some("getBlockByHeight") => match p.blocks.iter().find(|b| b.header.slot as u64 == pos) {
            Some(b) => ok(json!({"hash": b.hash().to_hex()})),
            None => none,
        },
        Some("getBlockRaw") if p.garbage && height >= 1 => ok(json!({
            "block": base64::engine::general_purpose::STANDARD.encode(b"not a block"),
            "blake3": "", "height": height})),
        Some("getBlockRaw") => match p.blocks.iter().find(|b| b.header.slot as u64 == height) {
            Some(b) => {
                let data = bincode::serialize(b).unwrap();
                ok(
                    json!({"block": base64::engine::general_purpose::STANDARD.encode(&data),
                          "blake3": crypto::hash::hash(&data).to_string(), "height": height}),
                )
            }
            None => none,
        },
        _ => none,
    })
}

async fn serve_peer(p: Peer) -> String {
    let app = Router::new()
        .route("/", post(peer_rpc))
        .with_state(Arc::new(p));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn prune_ctx(tmp: &TempDir) -> RpcContext {
    let g = crypto::hash::hash(b"inc-i-308-m2-prune");
    let kp = KeyPair::from_seed([0x5A; 32]);
    let ctx = local_node(&tmp.path().join("blocks"), 2_100)
        .with_data_dir(tmp.path().to_path_buf())
        .with_archive_dir(tmp.path().join("arch"));
    std::fs::create_dir_all(tmp.path().join("arch")).unwrap();
    let mut prev = g;
    for h in 1..=5u64 {
        let b = block(g, h, prev, &kp);
        prev = b.hash();
        ctx.block_store.put_block_canonical(&b, h).unwrap();
    }
    assert!(ctx.block_store.get_block_by_height(1).unwrap().is_some());
    ctx
}

// P1-RPC-008 (INC-I-308) — Decision: a failure means one unauthenticated read RPC with a multibyte address aborts the node.
#[tokio::test]
async fn audit_p1_rpc_008_multibyte_address_returns_invalid_params() {
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 1);
    for method in ["getBalance", "getUtxos", "getHistory"] {
        let v = raw(&ctx, method, json!({"address": BAD_ADDR})).await;
        assert_eq!(v["error"]["code"], json!(-32602), "{method}: {v}");
    }
}

// P1-RPC-004 (INC-I-308) — Decision: a failure means pruneBlocks deletes history no archive holds, on any node a token holder reaches.
#[tokio::test]
async fn audit_p1_rpc_004_prune_without_covering_archive_keeps_blocks() {
    let tmp = TempDir::new().unwrap();
    let ctx = prune_ctx(&tmp);
    let v = raw(&ctx, "pruneBlocks", json!([2000])).await;
    let code = v["error"]["code"].as_i64();
    assert!(code != Some(-32602) && code != Some(-32601), "fixture: {v}");
    let kept = ctx.block_store.get_block_by_height(1).unwrap().is_some();
    assert!(
        code.is_some() || kept,
        "h1 pruned without a covering archive: {v}"
    );
}

// P1-RPC-004 (INC-I-308) — Decision: a failure means the refusal blocks the explicit operator override, so no node can ever prune.
#[tokio::test]
async fn audit_p1_rpc_004_prune_with_force_runs() {
    let tmp = TempDir::new().unwrap();
    let ctx = prune_ctx(&tmp);
    // Param shape: positional array [keep_last_n, force] (pruning.rs already reads arr[0]).
    let r = call(&ctx, "pruneBlocks", json!([2000, true])).await;
    assert!(
        ctx.block_store.get_block_by_height(1).unwrap().is_none(),
        "force did not prune h1: {r}"
    );
}

// P1-RPC-007 (INC-I-308) — Decision: a failure means repairArchiveFromPeer writes undecodable or above-tip files into the archive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_p1_rpc_007_repair_archive_rejects_garbage_and_clamps_to_local_tip() {
    let tmp = TempDir::new().unwrap();
    let arch = tmp.path().join("arch");
    let ctx = local_node(&tmp.path().join("blocks"), 2).with_archive_dir(arch.clone());
    let url = serve_peer(Peer {
        tip: 5,
        blocks: vec![],
        garbage: true,
    })
    .await;
    let v = raw(&ctx, "repairArchiveFromPeer", json!({"rpc_url": url})).await;
    let code = v["error"]["code"].as_i64();
    assert!(code != Some(-32602) && code != Some(-32601), "fixture: {v}");
    for h in 3..=5u64 {
        let p = arch.join(format!("{h:010}.block"));
        assert!(!p.exists(), "above-tip archive file {p:?} written: {v}");
    }
    for e in std::fs::read_dir(&arch).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "block") {
            let data = std::fs::read(&p).unwrap();
            assert!(
                deserialize_block_compat(&data).is_some(),
                "garbage in {p:?}: {v}"
            );
        }
    }
}

// P1-RPC-005 (INC-I-308) — Decision: a failure means backfillFromPeer stores a self-signed block that contradicts the stored h+1 neighbor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_p1_rpc_005_backfill_refuses_block_contradicting_stored_neighbor() {
    let g = crypto::hash::hash(b"inc-i-308-m2-backfill");
    let honest = KeyPair::from_seed([0x5A; 32]);
    let attacker = KeyPair::from_seed([0x66; 32]);
    let h1 = block(g, 1, g, &honest);
    let h2 = block(g, 2, h1.hash(), &honest);
    let h3 = block(g, 3, h2.hash(), &honest);
    let h4 = block(g, 4, h3.hash(), &honest);
    let forged = block(g, 3, h2.hash(), &attacker);
    assert_ne!(forged.hash(), h3.hash(), "fixture");
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 4);
    for (b, h) in [(&h1, 1), (&h2, 2), (&h4, 4)] {
        ctx.block_store.put_block_canonical(b, h).unwrap();
    }
    let url = serve_peer(Peer {
        tip: 4,
        blocks: vec![h1.clone(), h2.clone(), forged.clone(), h4.clone()],
        garbage: false,
    })
    .await;
    let started = call(
        &ctx,
        "backfillFromPeer",
        json!({"rpc_url": url, "skip_divergence_check": true}),
    )
    .await;
    assert_eq!(started["started"], json!(true), "fixture: {started}");
    let st = wait_done(&ctx).await;
    let anchor = ctx
        .block_store
        .get_block_by_height(4)
        .unwrap()
        .unwrap()
        .header
        .prev_hash;
    let got = ctx
        .block_store
        .get_block_by_height(3)
        .unwrap()
        .map(|b| b.hash());
    assert!(
        got.is_none() || got == Some(anchor),
        "h3 {got:?} contradicts stored h4.prev_hash {anchor:?}; forged={}, status {st}",
        forged.hash()
    );
}

fn chain3(g: Hash, kp: &KeyPair) -> (Block, Block, Block) {
    let h1 = block(g, 1, g, kp);
    let h2 = block(g, 2, h1.hash(), kp);
    let h3 = block(g, 3, h2.hash(), kp);
    (h1, h2, h3)
}

async fn repair_with_h2(served: &Block, h1: &Block, h3: &Block) -> (TempDir, Value) {
    let tmp = TempDir::new().unwrap();
    let arch = tmp.path().join("arch");
    std::fs::create_dir_all(&arch).unwrap();
    let ctx = local_node(&tmp.path().join("blocks"), 3).with_archive_dir(arch);
    for (b, h) in [(h1, 1), (h3, 3)] {
        ctx.block_store.put_block_canonical(b, h).unwrap();
    }
    let url = serve_peer(Peer {
        tip: 3,
        blocks: vec![h1.clone(), served.clone(), h3.clone()],
        garbage: false,
    })
    .await;
    let v = call(&ctx, "repairArchiveFromPeer", json!({"rpc_url": url})).await;
    (tmp, v)
}

// P1-RPC-007 (INC-I-308) — Decision: a failure means repairArchiveFromPeer archives a self-signed block that does not link to the stored chain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_p1_rpc_007_repair_archive_refuses_block_not_linked_to_stored_chain() {
    let attacker = KeyPair::from_seed([0x66; 32]);
    let (h1, _h2, h3) = chain3(Hash::ZERO, &KeyPair::from_seed([0x5A; 32]));
    let forged = block(Hash::ZERO, 2, h1.hash(), &attacker);
    assert_ne!(forged.hash(), h3.header.prev_hash, "fixture");
    let (tmp, v) = repair_with_h2(&forged, &h1, &h3).await;
    let arch = tmp.path().join("arch");
    assert!(
        !arch.join("0000000002.block").exists(),
        "forged h2 archived: {v}"
    );
    assert!(
        v["fetched"].as_u64().unwrap() <= 2,
        "fetched counts h2: {v}"
    );
}

// P1-RPC-007 (INC-I-308) — Decision: a failure means the link check also refuses the honest block, so repair never fills a gap.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_p1_rpc_007_repair_archive_writes_linked_block() {
    let (h1, h2, h3) = chain3(Hash::ZERO, &KeyPair::from_seed([0x5A; 32]));
    assert_eq!(h2.hash(), h3.header.prev_hash, "fixture");
    let (tmp, v) = repair_with_h2(&h2, &h1, &h3).await;
    let data = std::fs::read(tmp.path().join("arch").join("0000000002.block"))
        .unwrap_or_else(|e| panic!("honest h2 not archived ({e}): {v}"));
    assert_eq!(deserialize_block_compat(&data).unwrap().hash(), h2.hash());
}

async fn backfill(ctx: &RpcContext, blocks: Vec<Block>) -> Value {
    let url = serve_peer(Peer {
        tip: 3,
        blocks,
        garbage: false,
    })
    .await;
    let p = json!({"rpc_url": url, "skip_divergence_check": true});
    let started = call(ctx, "backfillFromPeer", p).await;
    assert_eq!(started["started"], json!(true), "fixture: {started}");
    wait_done(ctx).await
}

// P1-RPC-005 (INC-I-308) — Decision: a failure means backfill overwrites a height-index entry outside the repair set.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_p1_rpc_005_backfill_refuses_block_when_height_index_holds_different_hash() {
    let g = crypto::hash::hash(b"inc-i-308-m6b");
    let (h1, h2, h3) = chain3(g, &KeyPair::from_seed([0x5A; 32]));
    let other = Hash::from_bytes([0x42; 32]);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 3);
    for (b, h) in [(&h1, 1), (&h3, 3)] {
        ctx.block_store.put_block_canonical(b, h).unwrap();
    }
    ctx.block_store.seed_canonical_index(other, 2).unwrap();
    assert!(
        ctx.block_store.get_block_by_height(2).unwrap().is_none(),
        "fixture"
    );
    let st = backfill(&ctx, vec![h1, h2, h3]).await;
    let got = ctx.block_store.get_hash_by_height(2).unwrap();
    assert_eq!(got, Some(other), "index at h2 overwritten; status {st}");
}

// P1-RPC-005 (INC-I-308) — Decision: a failure means backfill stores a block whose prev_hash contradicts the stored h-1.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_p1_rpc_005_backfill_refuses_block_contradicting_stored_prev() {
    let g = crypto::hash::hash(b"inc-i-308-m6b");
    let kp = KeyPair::from_seed([0x5A; 32]);
    let (h1, h2, _h3) = chain3(g, &kp);
    let forged = block(g, 3, Hash::from_bytes([0x09; 32]), &kp);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 3);
    for (b, h) in [(&h1, 1), (&h2, 2)] {
        ctx.block_store.put_block_canonical(b, h).unwrap();
    }
    let st = backfill(&ctx, vec![h1, h2, forged]).await;
    let got = ctx.block_store.get_block_by_height(3).unwrap();
    assert!(got.is_none(), "forged h3 stored; status {st}");
}

// P1-RPC-005 (INC-I-308) — Decision: a failure means the neighbor check blocks fork repair, so a forked node can never re-adopt the peer chain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_p1_rpc_005_backfill_repair_set_overwrites_stale_successor() {
    let g = crypto::hash::hash(b"inc-i-308-m6b");
    let (h1, h2, h3) = chain3(g, &KeyPair::from_seed([0x5A; 32]));
    let fork = KeyPair::from_seed([0x66; 32]);
    let f2 = block(g, 2, h1.hash(), &fork);
    let f3 = block(g, 3, f2.hash(), &fork);
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(tmp.path(), 3);
    for (b, h) in [(&h1, 1), (&f2, 2), (&f3, 3)] {
        ctx.block_store.put_block_canonical(b, h).unwrap();
    }
    let st = backfill(&ctx, vec![h1, h2.clone(), h3.clone()]).await;
    for (h, want) in [(2, h2.hash()), (3, h3.hash())] {
        let got = ctx.block_store.get_hash_by_height(h).unwrap();
        assert_eq!(got, Some(want), "h{h} still on the fork; status {st}");
    }
}

// P1-RPC-004 (INC-I-308) — Decision: a failure means a covering archive no longer admits a plain prune.
#[tokio::test]
async fn audit_p1_rpc_004_prune_without_force_admitted_when_archive_covers_cutoff() {
    let tmp = TempDir::new().unwrap();
    let ctx = prune_ctx(&tmp);
    let manifest = json!({"latest_height": 100}).to_string();
    std::fs::write(tmp.path().join("arch").join("manifest.json"), manifest).unwrap();
    let r = call(&ctx, "pruneBlocks", json!([2000])).await;
    assert!(
        ctx.block_store.get_block_by_height(1).unwrap().is_none(),
        "covered prune kept h1: {r}"
    );
}
