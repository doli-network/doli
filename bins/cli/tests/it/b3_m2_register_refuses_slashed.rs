//! B3 M2 — `doli producer register` and `doli producer add-bond` refuse a key
//! the node reports as slashed or slash-queued, before they submit anything;
//! an exited key may still re-register and a clean producer may still add bonds.
//!
// covers: register bonds producer_ledger mod
//
// OUTPUT CONTRACT: process `doli producer register --bonds 1 --yes` / `doli producer add-bond --count 1 --yes`
//   O3 exit status: failure on refusal; success on submit
//   O6 RPC calls to the stub node: no `sendTransaction` on refusal; `sendTransaction` on submit
//   O3 output text: the refusal names the slash ("slash" in stdout/stderr)
// PATHS: P-REFUSE (slashed | slash-queued) | P-SUBMIT (control)
// INPUT PARTITIONS:
//   getProducers entry for the wallet key: status slashed | status exited + pending "slash" update
//      | status active + pending "slash" update | status exited (no slash) | status active (no slash)
//   command: register | add-bond
// MATRIX:
//   O3,O6 x P-REFUSE x {slashed, exited+queued} x register -> b3_m2_req_b3_003_cli_register_refuses_slashed_or_queued_key
//   O3,O6 x P-REFUSE x {slashed, active+queued} x add-bond -> b3_m2_req_b3_003_cli_add_bond_refuses_slashed_or_queued_producer
//   O3,O6 x P-SUBMIT x {exited register, active add-bond} -> b3_m2_req_b3_003_cli_exited_reregistration_and_clean_add_bond_still_submit

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

const BOND_UNIT: u64 = 1_000_000_000;
const TX_HASH: &str = "b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3";
const FUNDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const BEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const GENESIS: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const MINED: &str = "4444444444444444444444444444444444444444444444444444444444444444";

#[derive(Clone, Copy, Debug)]
struct Seen {
    status: &'static str,
    slash_queued: bool,
}

struct Stub {
    addr: SocketAddr,
    calls: Arc<Mutex<Vec<String>>>,
}

fn start_stub(seen: Seen, pk_hex: String) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stub: bind loopback");
    let addr = listener.local_addr().expect("stub: local_addr");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let thread_calls = Arc::clone(&calls);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Some(body) = read_body(&mut stream) else {
                continue;
            };
            let req: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let method = req
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            thread_calls.lock().unwrap().push(method.clone());
            let payload = serde_json::to_string(&respond(&method, seen, &pk_hex)).unwrap();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
    Stub { addr, calls }
}

fn read_body(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
    let len = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while buf.len() < end + len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Some(String::from_utf8_lossy(&buf[end..(end + len).min(buf.len())]).to_string())
}

fn ok(result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "result": result })
}

fn producer_entry(seen: Seen, pk_hex: &str) -> Value {
    let pending = if seen.slash_queued {
        json!([{ "updateType": "slash" }])
    } else {
        json!([])
    };
    json!([{
        "publicKey": pk_hex, "addressHash": "", "registrationHeight": 10u64,
        "bondAmount": BOND_UNIT, "bondCount": 1u32, "producerSetBondCount": 1u32,
        "status": seen.status, "era": 0u64, "pendingWithdrawals": [], "pendingUpdates": pending,
        "blsPubkey": "", "delegatedTo": Value::Null, "delegatedBonds": 0u32,
        "receivedDelegations": [], "selectionWeight": 0u64,
    }])
}

fn respond(method: &str, seen: Seen, pk_hex: &str) -> Value {
    match method {
        "getChainInfo" => ok(json!({
            "network": "devnet", "bestHash": BEST, "bestHeight": 1_000u64, "bestSlot": 1_000u64,
            "genesisHash": GENESIS, "rewardPoolBalance": 0u64,
        })),
        "getNetworkParams" => ok(json!({
            "network": "devnet", "bondUnit": BOND_UNIT, "slotDuration": 1u64, "slotsPerEpoch": 100u32,
            "blocksPerRewardEpoch": 100u64, "coinbaseMaturity": 10u64, "initialReward": 100_000_000u64,
            "genesisTime": 0u64,
        })),
        "getUtxos" => ok(json!([{
            "txHash": FUNDING, "outputIndex": 0u32, "amount": 50 * BOND_UNIT, "outputType": "normal",
            "lockUntil": 0u64, "height": 10u64, "spendable": true, "pending": false,
        }])),
        "getProducers" => ok(producer_entry(seen, pk_hex)),
        "getProducer" => ok(producer_entry(seen, pk_hex)[0].clone()),
        "sendTransaction" => ok(json!(TX_HASH)),
        "getEpochInfo" => ok(json!({
            "currentHeight": 1_000u64, "currentEpoch": 10u64, "lastCompleteEpoch": 9u64,
            "blocksPerEpoch": 100u64, "blocksRemaining": 42u64, "epochStartHeight": 1_000u64,
            "epochEndHeight": 1_100u64, "blockReward": 100_000_000u64,
        })),
        "getTransaction" => ok(json!({
            "hash": TX_HASH, "version": 1u32, "txType": "registration", "inputs": [], "outputs": [],
            "size": 500usize, "fee": 1u64, "blockHash": MINED, "blockHeight": 1_001u64, "confirmations": 1u64,
        })),
        "getMempoolTransactions" => ok(json!([])),
        "getMempoolInfo" => ok(json!({
            "txCount": 0u64, "totalSize": 0u64, "minFeeRate": 1u64, "maxSize": 1_000_000u64, "maxCount": 5_000u64,
        })),
        _ => {
            json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32601, "message": "Method not found" } })
        }
    }
}

struct Run {
    success: bool,
    text: String,
    calls: Vec<String>,
}

/// Run `args` against a stub that reports the wallet key as `seen`.
fn run(seen: Seen, args: &[&str]) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("wallet.json");
    let kp = crypto::KeyPair::generate();
    let bls = crypto::BlsKeyPair::generate();
    let pk_hex = kp.public_key().to_hex();
    let wallet = json!({
        "name": "b3-m2-test", "version": 2u32,
        "addresses": [{
            "address": kp.address().to_hex(), "public_key": pk_hex,
            "private_key": kp.private_key().to_hex(), "label": "primary",
            "bls_private_key": bls.secret_key().to_hex(), "bls_public_key": bls.public_key().to_hex(),
        }],
    });
    std::fs::write(&path, serde_json::to_string_pretty(&wallet).unwrap()).expect("write wallet");
    let stub = start_stub(seen, pk_hex);
    let out = Command::new(env!("CARGO_BIN_EXE_doli"))
        .arg("--wallet")
        .arg(&path)
        .args([
            "--network",
            "devnet",
            "--rpc",
            &format!("http://{}", stub.addr),
        ])
        .args(args)
        .output()
        .expect("run the doli binary");
    let calls = stub.calls.lock().unwrap().clone();
    Run {
        success: out.status.success(),
        text: format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        calls,
    }
}

const REGISTER: &[&str] = &["producer", "register", "--bonds", "1", "--yes"];
const ADD_BOND: &[&str] = &["producer", "add-bond", "--count", "1", "--yes"];

fn submitted(r: &Run) -> bool {
    r.calls.iter().any(|c| c == "sendTransaction")
}

fn assert_refused(seen: Seen, cmd: &str, args: &[&str]) {
    let r = run(seen, args);
    assert!(
        !submitted(&r) && !r.success && r.text.to_ascii_lowercase().contains("slash"),
        "{cmd} for a key reported {seen:?} must refuse before sendTransaction and name the slash.\nrpc calls: {:?}\n{}",
        r.calls,
        r.text
    );
}

// REQ-B3-003 — Decision: a failure means the CLI builds and submits a Registration for a slashed or slash-queued key, which the chain mines and storage orphans (or the node now refuses with a cryptic error after the fee is spent).
#[test]
fn b3_m2_req_b3_003_cli_register_refuses_slashed_or_queued_key() {
    for seen in [
        Seen {
            status: "slashed",
            slash_queued: false,
        },
        Seen {
            status: "exited",
            slash_queued: true,
        },
    ] {
        assert_refused(seen, "register", REGISTER);
    }
}

// REQ-B3-003 — Decision: a failure means the CLI submits an AddBond for a slashed or slash-queued producer, locking coins in Bond UTXOs no producer can use.
#[test]
fn b3_m2_req_b3_003_cli_add_bond_refuses_slashed_or_queued_producer() {
    for seen in [
        Seen {
            status: "slashed",
            slash_queued: false,
        },
        Seen {
            status: "active",
            slash_queued: true,
        },
    ] {
        assert_refused(seen, "add-bond", ADD_BOND);
    }
}

// REQ-B3-003 — Decision: a failure means the slash refusal also blocks an exited key's re-registration or a clean producer's add-bond, or the stub harness cannot reach the submit step at all (vacuous refusals).
#[test]
fn b3_m2_req_b3_003_cli_exited_reregistration_and_clean_add_bond_still_submit() {
    for (cmd, seen, args) in [
        (
            "register",
            Seen {
                status: "exited",
                slash_queued: false,
            },
            REGISTER,
        ),
        (
            "add-bond",
            Seen {
                status: "active",
                slash_queued: false,
            },
            ADD_BOND,
        ),
    ] {
        let r = run(seen, args);
        assert!(
            submitted(&r),
            "{cmd} for a key reported {seen:?} must still reach sendTransaction.\nrpc calls: {:?}\n{}",
            r.calls,
            r.text
        );
    }
}
