//! INC-I-236 M2 — backfillFromPeer verifies the block signature at h >= AH before storing (REQ-AUTH-019, census P13).
//!
// covers: backfill context validation block
// covers: crates/rpc/src/methods/backfill.rs crates/rpc/src/methods/context.rs crates/core/src/validation/block.rs crates/core/src/validation/mod.rs
// AH source: NetworkParams::load(Devnet).inc_i_236_block_signature_activation_height, pinned per process via
// DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT (honoured on devnet only); no new API needed.
//
// OUTPUT CONTRACT: RPC backfillFromPeer {rpc_url, skip_divergence_check: true}
//   O1 response {started, gaps, total}; O2 block store writes per fetched height (put_block_canonical);
//   O3 backfill_state {running, imported, total, error} read via backfillStatus
//   PATHS: P-BELOW h < AH stored as today | P-ARMED valid stored | P-ARMED forged or unsigned never stored
//     (skip or abort; with the bad block last, imported == 3 either way)
// INPUT PARTITIONS
//   peer serves h1 unsigned (< AH), h2 valid (< AH), h3 valid (== AH), h4 in {forged (producer named, attacker pi), unsigned}
// MATRIX
//   O1,O2,O3 x {forged, unsigned} -> req_auth_019_p13_backfill_forged_not_stored

use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use crypto::{Hash, KeyPair};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::network_params::NetworkParams;
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

const AH: u64 = 3;
const TIP: u64 = 4;

static PIN: Once = Once::new();

fn pin_devnet_ah() {
    PIN.call_once(|| {
        std::env::set_var(
            "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT",
            AH.to_string(),
        )
    });
    let got = NetworkParams::load(Network::Devnet).inc_i_236_block_signature_activation_height;
    assert_eq!(
        got, AH,
        "fixture: devnet AH must be pinned before first load"
    );
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sig {
    Unsigned,
    Valid,
    Forged,
}

fn producer() -> KeyPair {
    KeyPair::from_seed([0x5A; 32])
}

fn block(g: Hash, h: u64, prev: Hash, sig: Sig) -> Block {
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: Hash::ZERO,
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
    let signer = match sig {
        Sig::Unsigned => None,
        Sig::Valid => Some(producer()),
        Sig::Forged => Some(KeyPair::from_seed([0x66; 32])),
    };
    if let Some(kp) = signer {
        let msg = header.signing_message();
        header.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
    }
    let ok = verify_block_signature(&header).is_ok();
    assert_eq!(ok, sig == Sig::Valid, "fixture {sig:?} h={h}");
    Block::new(header, vec![])
}

fn chain(last: Sig) -> Vec<Block> {
    let g = crypto::hash::hash(b"inc-i-236-m2-backfill-genesis");
    let mut out: Vec<Block> = Vec::new();
    for (h, sig) in [
        (1, Sig::Unsigned),
        (2, Sig::Valid),
        (3, Sig::Valid),
        (4, last),
    ] {
        let prev = out.last().map(|b| b.hash()).unwrap_or(g);
        out.push(block(g, h, prev, sig));
    }
    out
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

// REQ-AUTH-019 — Decision: a failure means an admin backfillFromPeer from a hostile or compromised peer writes forged or unsigned post-height blocks into the canonical store.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req_auth_019_p13_backfill_forged_not_stored() {
    pin_devnet_ah();
    for last in [Sig::Forged, Sig::Unsigned] {
        let blocks = chain(last);
        let url = serve_peer(blocks.clone()).await;
        let tmp = TempDir::new().unwrap();
        let ctx = local_node(&tmp);
        let started = call(
            &ctx,
            "backfillFromPeer",
            json!({"rpc_url": url, "skip_divergence_check": true}),
        )
        .await;
        assert_eq!(started["started"], json!(true), "{last:?}: {started}");
        assert_eq!(started["total"], json!(TIP), "{last:?}: {started}");
        let st = wait_done(&ctx).await;
        for h in 1..=3u64 {
            let got = ctx.block_store.get_block_by_height(h).unwrap();
            let want = blocks[h as usize - 1].hash();
            assert_eq!(got.map(|b| b.hash()), Some(want), "{last:?}: h={h}, {st}");
        }
        let h4 = ctx.block_store.get_block_by_height(TIP).unwrap();
        assert!(
            h4.is_none(),
            "{last:?}: h4 stored with pi {:?}, status {st}",
            h4.map(|b| b.header.vdf_proof.pi)
        );
        assert_eq!(st["imported"], json!(3), "{last:?}: {st}");
    }
}
