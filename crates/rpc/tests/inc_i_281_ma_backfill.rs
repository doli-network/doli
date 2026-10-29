// INC-I-281 run 619 M-A body sweep row 34: backfillFromPeer stores a peer body with no apply.
//
// covers: backfill body_commitments commitment block
//
// OUTPUT CONTRACT: RPC backfillFromPeer {rpc_url, skip_divergence_check: true} into an empty
//   store (local tip h=4).
//   O1 response {started}; O2 block store canonical body per height (put_block_canonical);
//   O3 backfillStatus {running, imported, error}
// PATHS: honest body stored | variant body (same hash as the honest h4) not stored (skip or abort)
// INPUT PARTITIONS: peer serves honest h1..h3 and h4' = honest h4 plus one attestation bit
//   (merkle root holds, presence_root fails pre-AH); every block signed.
// MATRIX: O1 O2 O3 x {honest h1..h3, variant h4'} -> backfill_presence_junk

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
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

const TIP: u64 = 4;

fn producer() -> KeyPair {
    KeyPair::from_seed([0x5A; 32])
}

fn block(g: Hash, h: u64, prev: Hash) -> Block {
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(&[]),
        presence_root: Hash::ZERO,
        genesis_hash: g,
        timestamp: 40_000 + h,
        slot: h as u32,
        producer: *producer().public_key(),
        vdf_output: VdfOutput {
            value: vec![h as u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let msg = header.signing_message();
    header.vdf_proof.pi =
        crypto::signature::sign(msg.as_bytes(), producer().private_key()).to_vec();
    assert!(verify_block_signature(&header).is_ok(), "fixture: h={h}");
    Block::new(header, vec![])
}

fn chain() -> Vec<Block> {
    let g = crypto::hash::hash(b"inc-i-281-ma-backfill-genesis");
    let mut out: Vec<Block> = Vec::new();
    for h in 1..=TIP {
        let prev = out.last().map(|b| b.hash()).unwrap_or(g);
        out.push(block(g, h, prev));
    }
    out
}

/// `b` with one attestation bit flipped: same header hash, merkle root holds, presence_root fails.
fn presence_variant(b: &Block) -> Block {
    let mut v = b.clone();
    match v.attestation_bitfield.first_mut() {
        Some(byte) => *byte ^= 1,
        None => v.attestation_bitfield.push(1),
    }
    assert_eq!(v.hash(), b.hash(), "fixture: same header hash");
    assert!(v.verify_merkle_root(), "fixture: merkle root holds");
    assert_ne!(v, *b, "fixture: a different body");
    v
}

async fn peer_rpc(State(blocks): State<Arc<Vec<Block>>>, Json(req): Json<Value>) -> Json<Value> {
    let id = req["id"].clone();
    let height = req["params"]["height"].as_u64().unwrap_or(0) as usize;
    let found = match req["method"].as_str() {
        Some("getBlockRaw") if height >= 1 => blocks.get(height - 1),
        _ => None,
    };
    Json(match found {
        Some(b) => {
            let data = bincode::serialize(b).unwrap();
            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
            let sum = crypto::hash::hash(&data).to_string();
            json!({"jsonrpc": "2.0", "id": id,
                   "result": {"block": b64, "blake3": sum, "height": height}})
        }
        None => json!({"jsonrpc": "2.0", "id": id,
                       "error": {"code": -32601, "message": "stub: unsupported"}}),
    })
}

async fn serve_peer(blocks: Vec<Block>) -> String {
    let app = Router::new()
        .route("/", post(peer_rpc))
        .with_state(Arc::new(blocks));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn local_node(tmp: &TempDir) -> RpcContext {
    let network = Network::Devnet;
    let params = ConsensusParams::for_network(network);
    let mut cs = ChainState::new(Hash::ZERO);
    cs.best_height = TIP;
    cs.best_hash = Hash::from_bytes([0x77; 32]);
    let block_store = Arc::new(BlockStore::open(tmp.path()).unwrap());
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

async fn call(ctx: &RpcContext, method: &str, params: Value) -> Value {
    let resp = ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Value::from(1),
        })
        .await;
    let v = serde_json::to_value(&resp).unwrap();
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

// F-3 row 34 (Should) — Decision: a failure means backfillFromPeer writes a presence-root variant of a block into the canonical store with no apply, and the node then serves that body onward.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backfill_presence_junk() {
    let honest = chain();
    let mut served = honest.clone();
    served[TIP as usize - 1] = presence_variant(&honest[TIP as usize - 1]);
    let url = serve_peer(served.clone()).await;
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(&tmp);
    let started = call(
        &ctx,
        "backfillFromPeer",
        json!({"rpc_url": url, "skip_divergence_check": true}),
    )
    .await;
    assert_eq!(started["started"], json!(true), "precondition: {started}");
    let st = wait_done(&ctx).await;
    let mut wrong = Vec::new();
    for h in 1..TIP {
        let got = ctx.block_store.get_block_by_height(h).unwrap();
        if got.as_ref() != Some(&honest[h as usize - 1]) {
            wrong.push(format!("h={h}: the honest body is not stored"));
        }
    }
    let at_tip = ctx.block_store.get_block_by_height(TIP).unwrap();
    if at_tip.as_ref() == Some(&served[TIP as usize - 1]) {
        wrong.push(format!("h={TIP}: the variant body is stored"));
    }
    if st["imported"] != json!(3) {
        wrong.push(format!("imported {}", st["imported"]));
    }
    println!(
        "INC281_PROBE bodyvariant=backfill_presence_junk wrong={}",
        u8::from(!wrong.is_empty())
    );
    assert!(
        wrong.is_empty(),
        "backfillFromPeer with a variant at h={TIP}: {wrong:?} (status {st})"
    );
}
