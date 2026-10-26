//! INC-I-236 M2 — `doli-node restore --from-rpc` verifies the block signature at h >= AH before storing (REQ-AUTH-019, census P15).
//!
// covers: restore operations main validation block
// covers: bins/node/src/operations/restore.rs bins/node/src/operations/mod.rs bins/node/src/main.rs crates/core/src/validation/block.rs crates/core/src/validation/mod.rs
// AH source: NetworkParams::load(network) in the binary; devnet honours DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT,
// set here on the child process and in its data-dir .env; no new API needed.
//
// OUTPUT CONTRACT: CLI `doli-node --network devnet --data-dir D restore --from-rpc URL --yes [--backfill --skip-genesis-check]`
//   O1 process exit status (Ok when it skips, Err when it aborts on the bad block: either accepted);
//   O2 block store writes in D/blocks per fetched height (put_block_canonical)
//   PATHS: P-BELOW h < AH stored as today | P-ARMED valid stored | P-ARMED forged or unsigned never stored
// INPUT PARTITIONS
//   mode {backfill (--skip-genesis-check), restore (local genesis seeded at h=9000)}
//   archiver serves h1 unsigned (< AH), h2 valid (< AH), h3 valid (== AH), h4 in {forged (producer named, attacker pi), unsigned}
// MATRIX
//   O1,O2 x mode x {forged, unsigned} -> req_auth_019_p15_restore_from_rpc_forged_not_stored

use std::path::Path;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use crypto::{Hash, KeyPair};
use doli_core::validation::verify_block_signature;
use doli_core::{Block, BlockHeader};
use serde_json::{json, Value};
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const AH: u64 = 3;
const TIP: u64 = 4;
const AH_VAR: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sig {
    Unsigned,
    Valid,
    Forged,
}

fn producer() -> KeyPair {
    KeyPair::from_seed([0x5A; 32])
}

fn genesis() -> Hash {
    crypto::hash::hash(b"inc-i-236-m2-restore-genesis")
}

fn block(h: u64, prev: Hash, sig: Sig) -> Block {
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: genesis(),
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
    let mut out: Vec<Block> = Vec::new();
    for (h, sig) in [
        (1, Sig::Unsigned),
        (2, Sig::Valid),
        (3, Sig::Valid),
        (4, last),
    ] {
        let prev = out.last().map(|b| b.hash()).unwrap_or(genesis());
        out.push(block(h, prev, sig));
    }
    out
}

async fn archiver_rpc(
    State(blocks): State<Arc<Vec<Block>>>,
    Json(req): Json<Value>,
) -> Json<Value> {
    let id = req["id"].clone();
    let height = req["params"]["height"].as_u64().unwrap_or(0) as usize;
    let result = match req["method"].as_str() {
        Some("getChainInfo") => Some(json!({"bestHeight": blocks.len()})),
        Some("getBlockRaw") if height >= 1 => blocks.get(height - 1).map(|b| {
            let data = bincode::serialize(b).unwrap();
            json!({"block": base64::engine::general_purpose::STANDARD.encode(&data),
                   "blake3": crypto::hash::hash(&data).to_string(), "height": height})
        }),
        _ => None,
    };
    Json(match result {
        Some(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        None => json!({"jsonrpc": "2.0", "id": id,
                       "error": {"code": -32601, "message": "stub: unsupported"}}),
    })
}

async fn serve_archiver(blocks: Vec<Block>) -> String {
    let app = Router::new()
        .route("/", post(archiver_rpc))
        .with_state(Arc::new(blocks));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn run_restore(data_dir: &Path, url: &str, backfill: bool) -> Output {
    std::fs::write(data_dir.join(".env"), format!("{AH_VAR}={AH}\n")).unwrap();
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_doli-node"));
    cmd.arg("--network")
        .arg("devnet")
        .arg("--data-dir")
        .arg(data_dir)
        .args(["restore", "--from-rpc", url, "--yes"])
        .env(AH_VAR, AH.to_string())
        .kill_on_drop(true);
    if backfill {
        cmd.args(["--backfill", "--skip-genesis-check"]);
    }
    tokio::time::timeout(Duration::from_secs(120), cmd.output())
        .await
        .expect("restore did not exit in 120 s")
        .expect("spawn doli-node")
}

fn tail(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let start = s.len().saturating_sub(600);
    s.get(start..).unwrap_or(&s).to_string()
}

// REQ-AUTH-019 — Decision: a failure means an operator `restore --from-rpc` from a hostile or compromised archiver writes forged or unsigned post-height blocks into the canonical store.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn req_auth_019_p15_restore_from_rpc_forged_not_stored() {
    for last in [Sig::Forged, Sig::Unsigned] {
        let blocks = chain(last);
        for backfill in [true, false] {
            let what = format!(
                "{} h4 {last:?}",
                if backfill { "backfill" } else { "restore" }
            );
            let url = serve_archiver(blocks.clone()).await;
            let dir = TempDir::new().unwrap();
            let blocks_dir = dir.path().join("blocks");
            if !backfill {
                let s = BlockStore::open(&blocks_dir).unwrap();
                s.put_block_canonical(&block(9_000, Hash::ZERO, Sig::Unsigned), 9_000)
                    .unwrap();
            }
            let out = run_restore(dir.path(), &url, backfill).await;
            let ctx = format!(
                "{what}: status {:?}\nstdout: {}\nstderr: {}",
                out.status,
                tail(&out.stdout),
                tail(&out.stderr)
            );
            let s = BlockStore::open(&blocks_dir).unwrap();
            for h in 1..=3u64 {
                let got = s.get_block_by_height(h).unwrap().map(|b| b.hash());
                assert_eq!(got, Some(blocks[h as usize - 1].hash()), "h={h} {ctx}");
            }
            let h4 = s.get_block_by_height(TIP).unwrap();
            assert!(
                h4.is_none(),
                "h4 stored with pi {:?}; {ctx}",
                h4.map(|b| b.header.vdf_proof.pi)
            );
        }
    }
}
