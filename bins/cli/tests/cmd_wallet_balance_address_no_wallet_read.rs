// OUTPUT CONTRACT: fn cmd_balance(wallet_path, rpc_endpoint, address: Option<String>, show_all: bool)
//   Observed across the process boundary — `doli-cli` is a bin-only crate, so the
//   function is driven through the real binary (`env!("CARGO_BIN_EXE_doli")`).
//   O1: process exit code            — non-zero on every path exercised here
//   O2: stderr wallet-access error   — PRESENT/ABSENT ("cannot read wallet",
//                                      "wallet not found", "Check file permissions",
//                                      "os error 13"; wallet.rs:112-135)
//   O3: stderr node-unreachable line — PRESENT/ABSENT ("Cannot connect to node";
//                                      cmd_wallet.rs:147-149) == proof the RPC step
//                                      was reached, i.e. the wallet was NOT required
// PATHS:
//   P1: address-only      — `balance -A <addr>`         (address.is_some(), show_all=false)
//   P2: wallet-scoped     — `balance`                   (address.is_none(), show_all=false)
//   P3: address-plus-all  — `balance -A <addr> --all`   (address.is_some(), show_all=true)
// INPUT PARTITIONS:
//   ALL PATHS share ONE partition that matters: the wallet file EXISTS but is
//   UNREADABLE by the invoking user (mode 000). Justification for a single
//   partition: the quantity under test is *whether `Wallet::load` is called at
//   all*, and `Wallet::load` (wallet.rs:112) has exactly two observable classes of
//   outcome for a path that does not need key material — succeeds (invisible) or
//   fails (visible). Only the failing class can distinguish "wallet was read" from
//   "wallet was not read", so a readable-wallet partition is provably blind to the
//   defect and adds no cell. Sub-classes of failure (missing file vs unreadable
//   file) collapse to the same observable in O2, and only "exists-but-unreadable"
//   reproduces the operator's real situation (mode 600 producer signing key).
//   P1a / P2a / P3a: wallet file present, mode 000, valid JSON content.
// MATRIX: 3 outputs x 3 paths x 1 partition = 9 cells (every cell asserted below)
//
//   path                       | O1 exit | O2 wallet-access error | O3 "Cannot connect to node"
//   ---------------------------|---------|------------------------|---------------------------
//   P1a address-only           | != 0    | ABSENT                 | PRESENT
//   P2a wallet-scoped          | != 0    | PRESENT                | ABSENT
//   P3a address-plus-all       | != 0    | ABSENT                 | PRESENT
//
//! INC-I-161 reproduction — `doli balance --address <addr>` must not read the wallet.
//!
//! DEFECT. `bins/cli/src/cmd_wallet.rs:143` makes `let wallet = Wallet::load(wallet_path)?;`
//! the unconditional FIRST statement of `cmd_balance`. When `address` is `Some`, the
//! query list is built purely from the CLI argument via `crypto::address::resolve`
//! (`cmd_wallet.rs:197-201`); `wallet.addresses()` is read only in the `else` branch
//! (`:205`) and `wallet.primary_bech32_address()` only when
//! `show_per_address == false` (`:227`, `:276-277`). The wallet handle is therefore
//! provably unused whenever `address.is_some()` — yet an unreadable wallet still
//! aborts the command with `Permission denied (os error 13)`.
//!
//! REAL IMPACT. On a mainnet producer host the wallet file IS the producer signing
//! key (`--producer-key .../wallet.json`), correctly mode 600 and owned by the node
//! service user. An operator running a public, read-only address query is blocked by
//! a file the query never needs.
//!
//! FAIL -> PASS EVIDENCE. On HEAD, P1a and P3a stop at `cmd_wallet.rs:143` and emit
//! the wallet-access error (O2 PRESENT, O3 ABSENT) — both assertions invert. After
//! the fix they must reach the RPC step and report the dead endpoint instead
//! (O2 ABSENT, O3 PRESENT). P2a is the companion guard and must PASS on HEAD *and*
//! after the fix: the wallet requirement must survive on the path that actually
//! needs it.
//!
//! HARNESS NOTES.
//! * P1a/P3a point at a GUARANTEED-DEAD loopback endpoint (bind port 0, read the
//!   port, drop the listener). `RpcClient::ping` (`rpc_client.rs:813-818`) swallows
//!   the transport error and returns `Ok(false)`, so `cmd_balance` bails at
//!   `:147-149` with `Cannot connect to node at` — a stable, wallet-free landing
//!   point. `get_chain_info` uses the plain `call` path (`rpc_client.rs:657-661`
//!   -> `:520` -> `:482`), NOT `call_with_archiver_fallback`, so no request ever
//!   leaves the loopback interface.
//! * P2a deliberately points at a LIVE loopback stub that answers `getChainInfo`,
//!   so `ping()` succeeds. This makes the guard independent of whether the fix
//!   keeps `Wallet::load` above the ping or moves it below: either ordering must
//!   still surface the wallet-access error for a bare `balance`. A dead endpoint
//!   would make P2a fail spuriously against the "ping first, load later" fix shape.
//! * The test SKIPS when running as root, because root bypasses mode bits and the
//!   reproduction would be vacuous. `libc` is not a dependency, so the effective
//!   uid is read dependency-free via `id -u`.
//! * Zero new dependencies: `crypto`, `serde_json` (deps) and `tempfile` (dev-dep).

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Literal strings under test
// ---------------------------------------------------------------------------

/// `cmd_wallet.rs:148` — the bail that proves the RPC step was reached.
const NODE_UNREACHABLE: &str = "Cannot connect to node";

/// `wallet.rs:116` — `Wallet::load` failed on an existing-but-unreadable file.
const WALLET_UNREADABLE: &str = "cannot read wallet";

/// `wallet.rs:116` — the hint appended to the same context.
const WALLET_PERM_HINT: &str = "Check file permissions.";

/// `wallet.rs:123/130` — `Wallet::load` failed because the file was absent.
/// Asserted ABSENT on P1a/P3a so a harness slip (wrong path) cannot be mistaken
/// for the defect being fixed.
const WALLET_MISSING: &str = "wallet not found";

/// The `Caused by:` leaf of the anyhow chain for EACCES.
const RAW_EACCES: &str = "os error 13";

/// Deterministic 32-byte "public key" used to derive the query address. Its only
/// requirement is that `crypto::address::resolve` accepts the encoded form.
const ADDRESS_SEED: [u8; 32] = [0x11; 32];

const BEST_BLOCK_HASH: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const GENESIS_HASH: &str = "3333333333333333333333333333333333333333333333333333333333333333";

// ---------------------------------------------------------------------------
// Root guard — root ignores mode bits, which would make the test vacuous.
// ---------------------------------------------------------------------------

/// Effective uid without adding a `libc` dependency.
fn effective_uid() -> Option<u32> {
    let out = Command::new("id").arg("-u").output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Returns `true` (and prints a notice) when the test must be skipped.
fn skip_if_root(test_name: &str) -> bool {
    match effective_uid() {
        Some(0) => {
            println!(
                "SKIP {test_name}: running as root — root bypasses file mode bits, \
                 so an unreadable-wallet reproduction cannot be observed."
            );
            true
        }
        Some(_) => false,
        None => {
            println!("SKIP {test_name}: could not determine the effective uid via `id -u`.");
            true
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A wallet file that exists, parses as a wallet, and CANNOT be read by the
/// invoking user. The content must never be observed by a correct
/// `balance -A` run — it is present only so that a hypothetical fix cannot pass
/// by making the file parse-fail in some other way.
struct UnreadableWallet {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
}

fn make_unreadable_wallet() -> UnreadableWallet {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("wallet.json");

    let kp = crypto::KeyPair::generate();
    let bls = crypto::BlsKeyPair::generate();
    let wallet = json!({
        "name": "inc-i-161-test",
        "version": 2u32,
        "addresses": [{
            "address": kp.address().to_hex(),
            "public_key": kp.public_key().to_hex(),
            "private_key": kp.private_key().to_hex(),
            "label": "primary",
            "bls_private_key": bls.secret_key().to_hex(),
            "bls_public_key": bls.public_key().to_hex(),
        }],
    });
    std::fs::write(&path, serde_json::to_string_pretty(&wallet).unwrap())
        .expect("write wallet.json");

    // Model the operator's real situation: the file is present and the process
    // has no read access to it.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
        .expect("restrict wallet mode");

    // Positive control for the fixture itself: if this read SUCCEEDS the whole
    // test is vacuous, so fail loudly rather than silently pass.
    assert!(
        std::fs::read_to_string(&path).is_err(),
        "HARNESS FAILURE (not the defect): the wallet at {} is still readable by \
         this process, so the reproduction cannot be observed.",
        path.display()
    );

    UnreadableWallet { _dir: dir, path }
}

/// A bech32m `doli1...` address, verified in-test to parse through the exact
/// resolver `cmd_balance` uses (`cmd_wallet.rs:199`). Without this guard a bad
/// literal would abort at address parsing and the test would measure nothing.
fn query_address() -> String {
    let addr = crypto::address::from_pubkey(&ADDRESS_SEED, "doli").expect("encode doli1 address");
    crypto::address::resolve(&addr, None).unwrap_or_else(|e| {
        panic!("HARNESS FAILURE (not the defect): test address {addr} does not resolve: {e}")
    });
    addr
}

/// A loopback endpoint guaranteed to refuse connections: bind port 0, take the
/// port, drop the listener.
fn dead_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);
    format!("http://{addr}")
}

// ---------------------------------------------------------------------------
// Live stub node — used ONLY by P2a, so that `ping()` succeeds and the guard
// stays valid regardless of where the fix places `Wallet::load`.
// ---------------------------------------------------------------------------

struct Stub {
    addr: SocketAddr,
}

impl Stub {
    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

fn start_stub() -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stub: bind loopback");
    let addr = listener.local_addr().expect("stub: local_addr");

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            serve_one(&mut stream);
            let _ = stream.shutdown(Shutdown::Both);
        }
    });

    Stub { addr }
}

fn serve_one(stream: &mut TcpStream) {
    let Some(body) = read_http_body(stream) else {
        return;
    };
    let request: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");

    let payload = match method {
        // The only method `ping()` needs (rpc_client.rs:657-661).
        "getChainInfo" => json!({
            "jsonrpc": "2.0", "id": 1,
            "result": {
                "network": "mainnet",
                "bestHash": BEST_BLOCK_HASH,
                "bestHeight": 1_000u64,
                "bestSlot": 1_000u64,
                "genesisHash": GENESIS_HASH,
                "rewardPoolBalance": 0u64,
            }
        }),
        // `cmd_balance` tolerates an Err from getProducers (cmd_wallet.rs:187-190).
        _ => json!({
            "jsonrpc": "2.0", "id": 1,
            "error": { "code": -32601, "message": "Method not found" }
        }),
    };

    let body = serde_json::to_string(&payload).expect("stub: serialize response");
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
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
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

    Some(String::from_utf8_lossy(&buf[header_end..header_end + content_length]).to_string())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// ---------------------------------------------------------------------------
// CLI invocation
// ---------------------------------------------------------------------------

struct CliRun {
    exit_code: Option<i32>,
    success: bool,
    stdout: String,
    stderr: String,
}

/// Run `doli -w <wallet> -r <endpoint> --network mainnet balance [args...]`.
///
/// `DOLI_RPC_URL` / `DOLI_NETWORK` are cleared so an ambient environment cannot
/// redirect the run at a real node.
fn run_balance(wallet: &UnreadableWallet, endpoint: &str, extra: &[&str]) -> CliRun {
    let output = Command::new(env!("CARGO_BIN_EXE_doli"))
        .env_remove("DOLI_RPC_URL")
        .env_remove("DOLI_NETWORK")
        .arg("--wallet")
        .arg(&wallet.path)
        .arg("--network")
        .arg("mainnet")
        .arg("--rpc")
        .arg(endpoint)
        .arg("balance")
        .args(extra)
        .output()
        .expect("failed to run the doli binary");

    CliRun {
        exit_code: output.status.code(),
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn report(label: &str, run: &CliRun) -> String {
    format!(
        "\n--- {label} ---\nexit_code: {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}\n",
        run.exit_code, run.stdout, run.stderr
    )
}

/// Every marker that means "the wallet file was opened and the open failed".
fn wallet_access_error_markers(run: &CliRun) -> Vec<&'static str> {
    [
        WALLET_UNREADABLE,
        WALLET_PERM_HINT,
        WALLET_MISSING,
        RAW_EACCES,
    ]
    .into_iter()
    .filter(|m| run.stderr.contains(m))
    .collect()
}

/// Assert O1+O2+O3 for a path where the wallet is provably unused. All three
/// cells are evaluated before failing, so one run reports the whole contract.
fn assert_wallet_not_required(run: &CliRun, label: &str) {
    let mut violations: Vec<String> = Vec::new();

    // O1 — the run must still fail (the node is dead), but for the RIGHT reason.
    if run.success {
        violations.push(format!(
            "O1 exit code: expected != 0 (dead RPC endpoint), got {:?}",
            run.exit_code
        ));
    }

    // O2 — no wallet-access error may appear: the wallet is never consulted on
    // this path (cmd_wallet.rs:197-202).
    for marker in wallet_access_error_markers(run) {
        violations.push(format!(
            "O2 wallet-access error: stderr contains {marker:?} — the wallet was \
             read even though `--address` needs no key material"
        ));
    }

    // O3 — the run must have reached the RPC step.
    if !run.stderr.contains(NODE_UNREACHABLE) {
        violations.push(format!(
            "O3 node-unreachable line: stderr is missing {NODE_UNREACHABLE:?} — the \
             command never reached the RPC step"
        ));
    }

    assert!(
        violations.is_empty(),
        "INC-I-161 — `balance --address` required the wallet file.\nviolations:\n  - {}{}",
        violations.join("\n  - "),
        report(label, run)
    );
}

// ---------------------------------------------------------------------------
// P1a — address-only. THE REPRODUCTION. Must FAIL on HEAD.
// ---------------------------------------------------------------------------

/// INC-I-161. `doli balance -A <addr>` must not open the wallet file.
///
/// The address is supplied entirely on the command line and resolved by
/// `crypto::address::resolve` (`cmd_wallet.rs:199`); no key material is needed.
/// With an unreadable wallet the command must sail past the wallet entirely and
/// fail on the dead RPC endpoint instead.
///
/// FAILS TODAY: `cmd_wallet.rs:143` loads the wallet unconditionally, so the run
/// aborts with `cannot read wallet ... Permission denied (os error 13)` (O2
/// PRESENT) and never reaches `ping()` (O3 ABSENT).
#[test]
fn cmd_wallet_balance_with_address_does_not_read_wallet() {
    if skip_if_root("cmd_wallet_balance_with_address_does_not_read_wallet") {
        return;
    }

    let wallet = make_unreadable_wallet();
    let endpoint = dead_endpoint();
    let addr = query_address();

    let run = run_balance(&wallet, &endpoint, &["--address", &addr]);
    assert_wallet_not_required(&run, "P1a address-only (`balance -A <addr>`)");
}

// ---------------------------------------------------------------------------
// P3a — address-plus-all. `address.is_some()` must win over `--all`.
// Must FAIL on HEAD.
// ---------------------------------------------------------------------------

/// INC-I-161. `--all` must not resurrect the wallet requirement when
/// `--address` is given.
///
/// `show_per_address = address.is_some() || show_all` (`cmd_wallet.rs:227`), and
/// the query list is still built from the CLI argument alone (`:197-202`) —
/// `--all` only widens the *display* mode, never the *data source*. So this path
/// has exactly the same contract as P1a.
///
/// FAILS TODAY for the same reason as P1a: the unconditional load at `:143`
/// happens before either flag is inspected.
#[test]
fn cmd_wallet_balance_with_address_and_all_does_not_read_wallet() {
    if skip_if_root("cmd_wallet_balance_with_address_and_all_does_not_read_wallet") {
        return;
    }

    let wallet = make_unreadable_wallet();
    let endpoint = dead_endpoint();
    let addr = query_address();

    let run = run_balance(&wallet, &endpoint, &["--address", &addr, "--all"]);
    assert_wallet_not_required(&run, "P3a address-plus-all (`balance -A <addr> --all`)");
}

// ---------------------------------------------------------------------------
// P2a — wallet-scoped. COMPANION GUARD. Must PASS on HEAD *and* after the fix.
// ---------------------------------------------------------------------------

/// INC-I-161 guard: the fix must narrow the wallet requirement, not delete it.
///
/// A bare `balance` builds its query list from `wallet.addresses()`
/// (`cmd_wallet.rs:204-221`), so it genuinely needs the wallet. With the wallet
/// unreadable this run MUST fail with a wallet-access error.
///
/// The node here is a LIVE stub that answers `getChainInfo`, so `ping()`
/// succeeds. That keeps the guard valid whether the fix leaves `Wallet::load`
/// above the connectivity check or moves it below: in both orderings the bare
/// path must still reach — and fail at — the wallet.
///
/// PASSES TODAY and must keep passing.
#[test]
fn cmd_wallet_balance_without_address_still_requires_wallet() {
    if skip_if_root("cmd_wallet_balance_without_address_still_requires_wallet") {
        return;
    }

    let wallet = make_unreadable_wallet();
    let stub = start_stub();

    let run = run_balance(&wallet, &stub.url(), &[]);
    let label = "P2a wallet-scoped (`balance`)";

    let mut violations: Vec<String> = Vec::new();

    // O1 — the run must fail.
    if run.success {
        violations.push(format!(
            "O1 exit code: expected != 0, got {:?}",
            run.exit_code
        ));
    }

    // O2 — the wallet-access error MUST be present on this path.
    let markers = wallet_access_error_markers(&run);
    if !markers.contains(&WALLET_UNREADABLE) && !markers.contains(&RAW_EACCES) {
        violations.push(format!(
            "O2 wallet-access error: stderr is missing both {WALLET_UNREADABLE:?} and \
             {RAW_EACCES:?} — a bare `balance` must still require a readable wallet"
        ));
    }

    // O3 — the node is reachable, so the unreachable line must NOT appear.
    // This also proves the failure came from the wallet and not from the harness.
    if run.stderr.contains(NODE_UNREACHABLE) {
        violations.push(format!(
            "O3 node-unreachable line: stderr contains {NODE_UNREACHABLE:?} — HARNESS \
             FAILURE, the stub node did not answer getChainInfo"
        ));
    }

    assert!(
        violations.is_empty(),
        "INC-I-161 guard violated — the wallet requirement was removed from the path \
         that needs it.\nviolations:\n  - {}{}",
        violations.join("\n  - "),
        report(label, &run)
    );
}
