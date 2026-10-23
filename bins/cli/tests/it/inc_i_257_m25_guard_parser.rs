//! INC-I-257 M2.5 — the CLI covenant parser does not build an output-type
//! guard on "unbonding" (the decoder refuses type byte 16).
//!
// covers: encoding parsers pool unbonding revalidate protocol security_model cli
//
// OUTPUT CONTRACT: fn parse_condition(&str) -> Result<Condition> (binary-only; reached through `doli send --condition`)
//   O1 process exit status; O2 stderr text; O3 RPC methods the binary called after the connection ping
//   no file writes (the wallet file is read-only to `send`)
// PATHS: P-REFUSE guard type name refused before UTXO selection | P-PASS name accepted, send continues
// INPUT PARTITIONS:
//   type name: "unbonding" | "Unbonding" (case-insensitive parser) | "normal" (control)
// MATRIX:
//   O1,O2,O3 x P-REFUSE, P-PASS -> inc_i_257_m25_req_unbond_017_cli_guard_parser_refuses_unbonding

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};

const HASH64: &str = "1111111111111111111111111111111111111111111111111111111111111111";

struct Stub {
    url: String,
    calls: Arc<Mutex<Vec<String>>>,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn read_body(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(p) = find(&buf, b"\r\n\r\n") {
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
    Some(String::from_utf8_lossy(&buf[end..]).to_string())
}

fn reply(method: &str) -> Value {
    if method == "getChainInfo" {
        return json!({"jsonrpc": "2.0", "id": 1, "result": {
            "network": "devnet", "bestHash": HASH64, "bestHeight": 100u64,
            "bestSlot": 100u64, "genesisHash": HASH64, "rewardPoolBalance": 0u64}});
    }
    json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32000, "message": "stub: not served"}})
}

fn start_stub() -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stub: bind loopback");
    let url = format!("http://{}", listener.local_addr().expect("stub: addr"));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&calls);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let Some(body) = read_body(&mut s) else {
                continue;
            };
            let req: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let method = req["method"].as_str().unwrap_or("").to_string();
            seen.lock().unwrap().push(method.clone());
            let out = reply(&method).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                out.len(),
                out
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
            let _ = s.shutdown(Shutdown::Both);
        }
    });
    Stub { url, calls }
}

fn write_wallet(dir: &std::path::Path) -> (std::path::PathBuf, String) {
    let kp = crypto::KeyPair::from_seed([0x57; 32]);
    let path = dir.join("wallet.json");
    let wallet = json!({
        "name": "m25", "version": 2u32,
        "addresses": [{
            "address": kp.address().to_hex(),
            "public_key": kp.public_key().to_hex(),
            "private_key": kp.private_key().to_hex(),
            "label": "primary"}],
    });
    std::fs::write(&path, wallet.to_string()).expect("write wallet.json");
    let to =
        crypto::address::from_pubkey(kp.public_key().as_bytes(), "ddoli").expect("bech32 address");
    (path, to)
}

struct Run {
    ok: bool,
    stderr: String,
    after_ping: Vec<String>,
}

fn send_with_guard(type_name: &str) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let (wallet, to) = write_wallet(dir.path());
    let stub = start_stub();
    let cond = format!("output_type_guard({type_name}, 0)");
    let out = Command::new(env!("CARGO_BIN_EXE_doli"))
        .arg("--wallet")
        .arg(&wallet)
        .args(["--network", "devnet", "--rpc", &stub.url])
        .args(["send", &to, "1", "--condition", &cond, "--yes"])
        .output()
        .expect("run doli");
    let calls = stub.calls.lock().unwrap().clone();
    Run {
        ok: out.status.success(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        after_ping: calls.into_iter().filter(|m| m != "getChainInfo").collect(),
    }
}

// REQ-UNBOND-017 — Decision: a failure means the shipped CLI still builds a guard on type byte 16, a condition the base binary refuses and the decoder must refuse.
#[test]
fn inc_i_257_m25_req_unbond_017_cli_guard_parser_refuses_unbonding() {
    let control = send_with_guard("normal");
    assert!(
        !control.stderr.contains("Unknown output type") && !control.after_ping.is_empty(),
        "NON-VACUITY: guard type 'normal' passes the parser and send continues to RPC: {:?} / {}",
        control.after_ping,
        control.stderr
    );
    for name in ["unbonding", "Unbonding"] {
        let r = send_with_guard(name);
        assert!(
            !r.ok && r.stderr.contains("Unknown output type"),
            "guard type '{name}' must be refused by the parser: ok={} stderr={}",
            r.ok,
            r.stderr
        );
        assert!(
            r.after_ping.is_empty(),
            "guard type '{name}': no RPC call may follow a refused condition: {:?}",
            r.after_ping
        );
    }
}
