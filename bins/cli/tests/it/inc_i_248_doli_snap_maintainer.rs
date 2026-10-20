// covers: cmd_snap.rs, cmd_snap_maintainer.rs, cmd_snap_service.rs, lib.rs, main.rs, snapshot.rs (crates/rpc getStateSnapshot)
//! INC-I-248 M1.5 — `doli snap` always writes a maintainer state file.
//! REQ-248-002 — Decision: a red here means `doli snap` leaves the node with no root or a wrong root.
//
// OUTPUT CONTRACT: `doli --network testnet snap --data-dir D --seed S.. --no-restart [--trust]`
//   O1: process exit status — 0 on every path (a maintainer reason never aborts)
//   O2: D/maintainer_state.bin — EXISTS after the snap (replaced, never only deleted)
//   O3: MaintainerState::load(D).set — canonical (members, threshold, last_updated)
//       or EMPTY
//   O4: MaintainerState::load(D).last_derived_height — == snapshot height H
//   O5: D/state_db — installed at H (fail-closed path still completes the snap)
// PATHS:
//   P1: field present, seeds agree on its digest      -> canonical@H
//   P2: snapshot without `maintainerState` (old seed) -> EMPTY@H
//   P3: seeds disagree on the digest                  -> EMPTY@H, snap completes
// INPUT PARTITIONS:
//   P1a: 3 of 3 seeds agree (pre-existing canonical file at another height)
//   P1b: 2 of 3 agree, seed 3 answers another digest
//   P1c: 1 seed with --trust
//   P2a: 3 seeds agree, getStateSnapshot has no `maintainerState` key
//   P3a: 3 seeds, 3 different digests
// MATRIX: P1a/P1b/P1c: O1 O2 O3 O4 ; P2a: O1 O2 O3 O4 ; P3a: O1 O2 O3 O4 O5 — 21 cells
//
// The probe `inc_i_248_m15_probe_doli_snap_root` is a measurement (env-gated, no
// class assertion). Mock seeds: loopback std TcpListener JSON-RPC, unknown
// methods answer a JSON-RPC error.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::maintainer::{maintainer_set_digest, MaintainerSet};
use doli_core::Network;
use serde_json::{json, Value};
use storage::{
    ChainState, MaintainerState, ProducerSet, StateDb, StateSnapshot, UtxoSet,
    MAINTAINER_STATE_VERSION,
};
use tempfile::TempDir;

const SNAP_HEIGHT: u64 = 25_000;
const LAST_CHANGE: u64 = 20_000;
const PRE_EXISTING_DERIVED: u64 = 24_000;
const OTHER_DIGEST_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_DIGEST_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn genesis() -> Hash {
    ConsensusParams::for_network(Network::Testnet).genesis_hash
}

fn canonical_set() -> MaintainerSet {
    let members = (0..5u8)
        .map(|i| *crypto::KeyPair::from_seed([0xA0 + i; 32]).public_key())
        .collect();
    MaintainerSet::with_members(members, LAST_CHANGE)
}

fn canonical_digest() -> String {
    hex::encode(maintainer_set_digest(
        &canonical_set(),
        genesis().as_bytes(),
    ))
}

/// The `getStateSnapshot` result at height H, with or without `maintainerState`.
fn snapshot_result(with_field: bool) -> Value {
    let mut cs = ChainState::new(genesis());
    cs.best_height = SNAP_HEIGHT;
    cs.best_hash = Hash::from_bytes([0x5A; 32]);
    cs.best_slot = SNAP_HEIGHT as u32;
    let ah = Network::Testnet
        .params()
        .inc_i_181_pending_root_activation_height;
    let snap = StateSnapshot::create_gated(&cs, &UtxoSet::new(), &ProducerSet::new(), ah)
        .expect("fixture: snapshot");
    let mut r = json!({
        "height": snap.block_height,
        "blockHash": snap.block_hash.to_hex(),
        "stateRoot": snap.state_root.to_hex(),
        "chainState": hex::encode(&snap.chain_state_bytes),
        "utxoSet": hex::encode(&snap.utxo_set_bytes),
        "producerSet": hex::encode(&snap.producer_set_bytes),
        "epochBondSnapshot": null,
        "epochAccumulators": null,
        "totalBytes": snap.total_bytes(),
    });
    if with_field {
        let set = canonical_set();
        r["maintainerState"] = json!({
            "members": set.members.iter().map(|k| k.to_hex()).collect::<Vec<_>>(),
            "threshold": set.threshold,
            "lastChangeBlock": set.last_updated,
            "lastDerivedHeight": SNAP_HEIGHT,
            "digest": canonical_digest(),
            "genesisHash": genesis().to_hex(),
        });
    }
    r
}

#[derive(Clone)]
struct SeedCfg {
    snapshot: Value,
    maintainer_digest: String,
}

struct Seed {
    url: String,
    calls: Arc<Mutex<Vec<String>>>,
}

fn start_seed(cfg: SeedCfg) -> Seed {
    let listener = TcpListener::bind("127.0.0.1:0").expect("seed: bind");
    let url = format!("http://{}", listener.local_addr().expect("seed: addr"));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let thread_calls = Arc::clone(&calls);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            serve_one(&mut stream, &cfg, &thread_calls);
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
    Seed { url, calls }
}

fn serve_one(stream: &mut TcpStream, cfg: &SeedCfg, calls: &Arc<Mutex<Vec<String>>>) {
    let Some(body) = read_http_body(stream) else {
        return;
    };
    let request: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let method = request["method"].as_str().unwrap_or("").to_string();
    calls.lock().unwrap().push(method.clone());
    let payload = respond(cfg, &method);
    let body = serde_json::to_string(&payload).expect("seed: serialize");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn read_http_body(stream: &mut TcpStream) -> Option<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let content_length = headers
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let end = (header_end + content_length).min(buf.len());
    Some(String::from_utf8_lossy(&buf[header_end..end]).to_string())
}

fn respond(cfg: &SeedCfg, method: &str) -> Value {
    let ok = |result: Value| json!({ "jsonrpc": "2.0", "id": 1, "result": result });
    match method {
        "getStateRootDebug" => ok(json!({
            "height": cfg.snapshot["height"],
            "stateRoot": cfg.snapshot["stateRoot"],
        })),
        "getStateSnapshot" => ok(cfg.snapshot.clone()),
        "getMaintainerSet" => {
            let set = canonical_set();
            ok(json!({
                "maintainers": set.members.iter().map(|k| json!({ "pubkey": k.to_hex() })).collect::<Vec<_>>(),
                "threshold": set.threshold,
                "member_count": set.members.len(),
                "max_maintainers": 5,
                "min_maintainers": 3,
                "initial_maintainer_count": 5,
                "last_change_block": set.last_updated,
                "source": "on-chain",
                "enforced": true,
                "maintainer_derivation_activation_height": 15_087,
                "maintainer_set_digest": cfg.maintainer_digest,
                "genesis_hash": genesis().to_hex(),
            }))
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "Method not found" }
        }),
    }
}

fn seeds(with_field: bool, digests: &[&str]) -> Vec<Seed> {
    let snapshot = snapshot_result(with_field);
    digests
        .iter()
        .map(|d| {
            start_seed(SeedCfg {
                snapshot: snapshot.clone(),
                maintainer_digest: d.to_string(),
            })
        })
        .collect()
}

fn write_pre_existing_canonical(dir: &Path) {
    MaintainerState {
        version: MAINTAINER_STATE_VERSION,
        set: canonical_set(),
        last_derived_height: PRE_EXISTING_DERIVED,
    }
    .save(dir)
    .expect("fixture: pre-existing maintainer_state.bin");
}

fn run_snap(dir: &Path, seeds: &[Seed], trust: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_doli"));
    cmd.args(["--network", "testnet", "snap", "--data-dir"])
        .arg(dir)
        .arg("--no-restart");
    for s in seeds {
        cmd.args(["--seed", &s.url]);
    }
    if trust {
        cmd.arg("--trust");
    }
    for v in [
        "DOLI_NETWORK",
        "DOLI_RPC_URL",
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        cmd.env_remove(v);
    }
    cmd.env("NO_PROXY", "127.0.0.1");
    cmd.output().expect("spawn doli")
}

fn diag(out: &Output, seeds: &[Seed]) -> String {
    let calls: Vec<Vec<String>> = seeds
        .iter()
        .map(|s| s.calls.lock().unwrap().clone())
        .collect();
    format!(
        "exit={:?}\nstdout:\n{}\nstderr:\n{}\nseed calls: {:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        calls
    )
}

fn assert_exit_ok(out: &Output, seeds: &[Seed]) {
    assert!(
        out.status.success(),
        "O1: doli snap must exit 0\n{}",
        diag(out, seeds)
    );
}

fn load_written(dir: &Path, out: &Output, seeds: &[Seed]) -> MaintainerState {
    assert!(
        dir.join("maintainer_state.bin").exists(),
        "O2: maintainer_state.bin must exist after doli snap (replaced, never only deleted)\n{}",
        diag(out, seeds)
    );
    MaintainerState::load(dir).expect("O2: written file must load")
}

fn assert_canonical_at_h(dir: &Path, out: &Output, seeds: &[Seed]) {
    assert_exit_ok(out, seeds);
    let st = load_written(dir, out, seeds);
    let want = canonical_set();
    assert_eq!(st.set.members, want.members, "O3: canonical members");
    assert_eq!(st.set.threshold, want.threshold, "O3: canonical threshold");
    assert_eq!(st.set.last_updated, LAST_CHANGE, "O3: last_updated");
    assert_eq!(
        st.last_derived_height, SNAP_HEIGHT,
        "O4: last_derived_height"
    );
    assert_eq!(
        hex::encode(maintainer_set_digest(&st.set, genesis().as_bytes())),
        canonical_digest(),
        "O3: digest"
    );
}

fn assert_empty_at_h(dir: &Path, out: &Output, seeds: &[Seed]) {
    assert_exit_ok(out, seeds);
    let st = load_written(dir, out, seeds);
    assert!(
        st.set.members.is_empty(),
        "O3: fail-closed root must be EMPTY, got {} members",
        st.set.members.len()
    );
    assert_eq!(
        st.last_derived_height, SNAP_HEIGHT,
        "O4: last_derived_height"
    );
}

// INC-I-248 / REQ-248-002 — Decision: red means a quorum-agreed snap still loses the canonical root.
#[test]
fn inc_i_248_doli_snap_keeps_canonical_root() {
    let tmp = TempDir::new().unwrap();
    write_pre_existing_canonical(tmp.path());
    let d = canonical_digest();
    let s = seeds(true, &[&d, &d, &d]);
    let out = run_snap(tmp.path(), &s, false);
    assert_canonical_at_h(tmp.path(), &out, &s);
}

// INC-I-248 / REQ-248-002 — Decision: red means one dissenting seed can deny the canonical root.
#[test]
fn inc_i_248_doli_snap_one_seed_disagrees_still_canonical() {
    let tmp = TempDir::new().unwrap();
    write_pre_existing_canonical(tmp.path());
    let d = canonical_digest();
    let s = seeds(true, &[&d, &d, OTHER_DIGEST_1]);
    let out = run_snap(tmp.path(), &s, false);
    assert_canonical_at_h(tmp.path(), &out, &s);
}

// INC-I-248 / REQ-248-002 — Decision: red means a snap from an old seed keeps or deletes the root instead of EMPTY@H.
#[test]
fn inc_i_248_doli_snap_old_seed_without_field_fails_closed() {
    let tmp = TempDir::new().unwrap();
    write_pre_existing_canonical(tmp.path());
    let d = canonical_digest();
    let s = seeds(false, &[&d, &d, &d]);
    let out = run_snap(tmp.path(), &s, false);
    assert_empty_at_h(tmp.path(), &out, &s);
}

// INC-I-248 / REQ-248-002 — Decision: red means seed disagreement either aborts the snap or installs an unagreed root.
#[test]
fn inc_i_248_doli_snap_seeds_disagree_fails_closed_snap_completes() {
    let tmp = TempDir::new().unwrap();
    write_pre_existing_canonical(tmp.path());
    let d = canonical_digest();
    let s = seeds(true, &[&d, OTHER_DIGEST_1, OTHER_DIGEST_2]);
    let out = run_snap(tmp.path(), &s, false);
    assert_empty_at_h(tmp.path(), &out, &s);
    let db = StateDb::open(&tmp.path().join("state_db")).expect("O5: state_db opens");
    let cs = db.get_chain_state().expect("O5: chain state installed");
    assert_eq!(cs.best_height, SNAP_HEIGHT, "O5: state_db at H");
}

// INC-I-248 / REQ-248-002 — Decision: red means --trust with one seed cannot install the canonical root.
#[test]
fn inc_i_248_doli_snap_trust_single_seed_canonical() {
    let tmp = TempDir::new().unwrap();
    write_pre_existing_canonical(tmp.path());
    let d = canonical_digest();
    let s = seeds(true, &[&d]);
    let out = run_snap(tmp.path(), &s, true);
    assert_canonical_at_h(tmp.path(), &out, &s);
}

// INC-I-248 M1.5 outcome probe (measurement only). Run: INC_I_248_M15_PROBE=1.
#[test]
fn inc_i_248_m15_probe_doli_snap_root() {
    if std::env::var("INC_I_248_M15_PROBE").as_deref() != Ok("1") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    write_pre_existing_canonical(tmp.path());
    let d = canonical_digest();
    let s = seeds(true, &[&d, &d, &d]);
    let out = run_snap(tmp.path(), &s, false);
    eprintln!("{}", diag(&out, &s));
    let class = if !tmp.path().join("maintainer_state.bin").exists() {
        "absent".to_string()
    } else {
        match MaintainerState::load(tmp.path()) {
            Err(_) => "other@unreadable".to_string(),
            Ok(st) if st.set.members.is_empty() => format!("empty@{}", st.last_derived_height),
            Ok(st) => {
                let want = canonical_set();
                if st.set.members == want.members
                    && st.set.threshold == want.threshold
                    && st.set.last_updated == want.last_updated
                {
                    format!("canonical@{}", st.last_derived_height)
                } else {
                    format!("other@{}", st.last_derived_height)
                }
            }
        }
    };
    println!("\nINC-I-248-M1.5 doli-snap maintainer_state={class}");
}
