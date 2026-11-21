//! INC-I-308 M5 — backfillFromPeer Phase 2 sends `peer_token` to the peer and reports a refused divergence check.
//!
// covers: crates/rpc/src/methods/backfill.rs crates/rpc/src/types/chain.rs crates/rpc/src/server.rs
// covers: crates/rpc/src/methods/backfill_peer_commitment.rs
//
// OUTPUT CONTRACT: RPC backfillFromPeer {rpc_url, peer_token?} with blocks 1..=TIP stored, peer tip == local tip
//   O1 reply {started, message, divergence_check?}; O2 requests the peer receives (method + Authorization)
//   PATHS: P-ACCEPT peer answers chainCommitment | P-REFUSE peer answers -32008 (no or wrong token)
// INPUT PARTITIONS:
//   I1 peer_token == token the peer expects (peer answers the matching commitment)
//   I2 peer_token absent (peer refuses -32008)
//   I3 call through the real server with local admin token L and peer_token P, P != L
// MATRIX
//   O2 x P-ACCEPT x I1 -> inc_i_308_m5_peer_token_sent_on_verify_chain_integrity
//   O1 x P-REFUSE x I2 -> inc_i_308_m5_refused_divergence_check_is_reported
//   O2 x P-ACCEPT x I3 -> inc_i_308_m5_local_token_never_sent_to_peer

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use crypto::{Hash, Hasher};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::{Block, BlockHeader};
use mempool::{Mempool, MempoolPolicy};
use rpc::types::JsonRpcRequest;
use rpc::{RpcContext, RpcServer, RpcServerConfig};
use serde_json::{json, Value};
use storage::{BlockStore, ChainState, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

const TIP: u64 = 4;
const PEER_TOKEN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const LOCAL_TOKEN: &str = "0f9e8d7c6b5a49382716051f2e3d4c5b0f9e8d7c6b5a49382716051f2e3d4c5b";

fn chain() -> Vec<Block> {
    let g = crypto::hash::hash(b"inc-i-308-m5-backfill-genesis");
    let mut out: Vec<Block> = Vec::new();
    for h in 1..=TIP {
        let prev = out.last().map(|b| b.hash()).unwrap_or(g);
        let header = BlockHeader {
            version: 2,
            prev_hash: prev,
            merkle_root: doli_core::block::compute_merkle_root(&[]),
            presence_root: Hash::ZERO,
            genesis_hash: g,
            timestamp: 50_000 + h,
            slot: h as u32,
            producer: *crypto::KeyPair::from_seed([0x3C; 32]).public_key(),
            vdf_output: VdfOutput {
                value: vec![h as u8; 32],
            },
            vdf_proof: VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        out.push(Block::new(header, vec![]));
    }
    out
}

/// Same fold as backfill.rs Phase 1: commitment = H(commitment || block_hash) for h = 1..=tip.
fn commitment_hex(blocks: &[Block]) -> String {
    let mut c = Hash::default();
    for b in blocks {
        let mut hasher = Hasher::new();
        hasher.update(c.as_bytes());
        hasher.update(b.hash().as_bytes());
        c = hasher.finalize();
    }
    c.to_hex()
}

struct Stub {
    tip_hash: String,
    commitment: String,
    seen: Mutex<Vec<(String, Option<String>)>>,
}

impl Stub {
    fn seen(&self) -> Vec<(String, Option<String>)> {
        self.seen.lock().unwrap().clone()
    }
}

async fn peer_rpc(
    State(stub): State<Arc<Stub>>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> Json<Value> {
    let id = req["id"].clone();
    let method = req["method"].as_str().unwrap_or("").to_string();
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    stub.seen
        .lock()
        .unwrap()
        .push((method.clone(), auth.clone()));
    let refused = json!({"jsonrpc": "2.0", "id": id,
        "error": {"code": -32008, "message": "Unauthorized: invalid or missing bearer token"}});
    Json(match method.as_str() {
        "getBlockByHeight" => {
            json!({"jsonrpc": "2.0", "id": id, "result": {"hash": stub.tip_hash, "height": TIP}})
        }
        "verifyChainIntegrity" if auth.as_deref() == Some(&format!("Bearer {PEER_TOKEN}")) => {
            json!({"jsonrpc": "2.0", "id": id,
                   "result": {"chainCommitment": stub.commitment, "complete": true}})
        }
        "verifyChainIntegrity" => refused,
        _ => json!({"jsonrpc": "2.0", "id": id,
                     "error": {"code": -32601, "message": "stub: unsupported"}}),
    })
}

async fn serve_peer(blocks: &[Block]) -> (String, Arc<Stub>) {
    let stub = Arc::new(Stub {
        tip_hash: blocks.last().unwrap().hash().to_string(),
        commitment: commitment_hex(blocks),
        seen: Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .route("/", post(peer_rpc))
        .with_state(stub.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), stub)
}

/// Local node on Devnet (loopback URL allowed) with blocks 1..=TIP stored and tip == peer tip.
fn local_node(tmp: &TempDir, blocks: &[Block]) -> RpcContext {
    let network = Network::Devnet;
    let params = ConsensusParams::for_network(network);
    let block_store = Arc::new(BlockStore::open(tmp.path()).unwrap());
    for (i, b) in blocks.iter().enumerate() {
        block_store.put_block_canonical(b, i as u64 + 1).unwrap();
    }
    let mut cs = ChainState::new(Hash::ZERO);
    cs.best_height = TIP;
    cs.best_hash = blocks.last().unwrap().hash();
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

async fn backfill(ctx: &RpcContext, params: Value) -> Value {
    let resp = ctx
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "backfillFromPeer".to_string(),
            params,
            id: Value::from(1),
        })
        .await;
    serde_json::to_value(&resp).unwrap()
}

fn verify_calls(seen: &[(String, Option<String>)]) -> Vec<Option<String>> {
    seen.iter()
        .filter(|(m, _)| m == "verifyChainIntegrity")
        .map(|(_, a)| a.clone())
        .collect()
}

/// Real `RpcServer` with `admin_token = LOCAL_TOKEN` on a free loopback port (probe + retry, as the audit fixture).
async fn start_server(ctx_of: impl Fn() -> (RpcContext, TempDir)) -> (String, TempDir) {
    for _ in 0..5 {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .unwrap()
            .port();
        if (8500..=8517).contains(&port) || (28500..=28550).contains(&port) {
            continue;
        }
        let (ctx, tmp) = ctx_of();
        let config = RpcServerConfig {
            listen_addr: SocketAddr::from(([127, 0, 0, 1], port)),
            enable_cors: false,
            allowed_origins: vec![],
            admin_token: Some(LOCAL_TOKEN.to_string()),
            trusted_proxies: vec![],
        };
        let (ws_tx, _ws_rx) = rpc::ws::broadcast_channel();
        let server = RpcServer::new(config, ctx, ws_tx).spawn();
        let mut up = false;
        for _ in 0..300 {
            if server.is_finished() {
                break;
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                up = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        if up && !server.is_finished() {
            return (format!("http://127.0.0.1:{port}"), tmp);
        }
        server.abort();
    }
    panic!("no RPC server came up on a free 127.0.0.1 port in 5 attempts");
}

// INC-I-308-M5 — Decision: a failure means the divergence scan can never authenticate to a post-M1 peer, so backfill never repairs a forked block.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inc_i_308_m5_peer_token_sent_on_verify_chain_integrity() {
    let blocks = chain();
    let (url, stub) = serve_peer(&blocks).await;
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(&tmp, &blocks);
    let resp = backfill(&ctx, json!({"rpc_url": url, "peer_token": PEER_TOKEN})).await;
    let seen = stub.seen();
    let verify = verify_calls(&seen);
    assert!(
        !verify.is_empty(),
        "Phase 2 must call verifyChainIntegrity on the peer; seen={seen:?} resp={resp}"
    );
    let want = format!("Bearer {PEER_TOKEN}");
    assert!(
        verify.iter().all(|a| a.as_deref() == Some(want.as_str())),
        "verifyChainIntegrity must carry Authorization: Bearer <peer_token>; got {verify:?}, resp={resp}"
    );
    assert_eq!(
        resp["result"]["started"],
        json!(false),
        "matching commitment -> no repair: {resp}"
    );
}

// INC-I-308-M5 — Decision: a failure means a refused divergence check is still reported as "chain is complete" with no signal to the operator.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inc_i_308_m5_refused_divergence_check_is_reported() {
    let blocks = chain();
    let (url, stub) = serve_peer(&blocks).await;
    let tmp = TempDir::new().unwrap();
    let ctx = local_node(&tmp, &blocks);
    let resp = backfill(&ctx, json!({"rpc_url": url})).await;
    let seen = stub.seen();
    assert_eq!(
        verify_calls(&seen),
        vec![None],
        "fixture: Phase 2 reached the peer once with no token; seen={seen:?}"
    );
    let result = &resp["result"];
    assert_eq!(result["started"], json!(false), "{resp}");
    assert_eq!(
        result["message"],
        json!("No gaps detected — chain is complete."),
        "existing message unchanged: {resp}"
    );
    let check = result["divergence_check"].as_str().unwrap_or_else(|| {
        panic!("refused peer must be reported in `divergence_check`, got silent reply: {resp}")
    });
    assert!(
        check.starts_with("skipped:"),
        "divergence_check must start with \"skipped:\": {check:?}"
    );
    assert!(
        check.contains("peer_token"),
        "divergence_check must name peer_token: {check:?}"
    );
}

// INC-I-308-M5 — Decision: a failure means backfillFromPeer leaks the node's own admin token to a remote peer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inc_i_308_m5_local_token_never_sent_to_peer() {
    let blocks = chain();
    let (peer_url, stub) = serve_peer(&blocks).await;
    let (node_url, _tmp) = start_server(|| {
        let tmp = TempDir::new().unwrap();
        (local_node(&tmp, &blocks), tmp)
    })
    .await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "backfillFromPeer",
                      "params": {"rpc_url": peer_url, "peer_token": PEER_TOKEN}});
    let resp: Value = client
        .post(&node_url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {LOCAL_TOKEN}"))
        .body(body.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        resp.get("error").is_none_or(Value::is_null),
        "fixture: admin call with the local token must be accepted: {resp}"
    );
    let seen = stub.seen();
    assert!(!seen.is_empty(), "fixture: the peer was contacted: {resp}");
    let leaked: Vec<_> = seen
        .iter()
        .filter(|(_, a)| a.as_deref().is_some_and(|a| a.contains(LOCAL_TOKEN)))
        .collect();
    assert!(
        leaked.is_empty(),
        "local admin token sent to the peer: {leaked:?}"
    );
    let want = format!("Bearer {PEER_TOKEN}");
    assert_eq!(
        verify_calls(&seen),
        vec![Some(want)],
        "verifyChainIntegrity must carry the peer token: {seen:?}, resp={resp}"
    );
}
