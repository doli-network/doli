// OUTPUT CONTRACT: fn register_reports_failure_when_tx_is_dropped
// O1: process exit code
// O2: stdout success line presence/absence
// O3: stdout activation-ETA line presence/absence
// PATHS: submit-OK-and-retained / submit-OK-then-dropped / already-pending-rejected
// INPUT PARTITIONS: same-input duplicate (evicted on revalidate); already-pending duplicate (disjoint inputs)
// MATRIX: 3 outputs x 3 paths x 2 partitions
//
// Matrix cells (every cell has an assertion below):
//
//   path                      | O1 exit    | O2 success line | O3 ETA line
//   --------------------------|------------|-----------------|-------------
//   submit-OK-and-retained    | == 0       | PRESENT         | PRESENT
//   submit-OK-then-dropped    | != 0       | ABSENT          | ABSENT
//   already-pending-rejected  | != 0       | ABSENT          | ABSENT
//
//! INC-I-148 reproduction — INV-CLI-002.
//!
//! INV-CLI-002: a CLI submit command MUST NOT report success (exit 0 +
//! "submitted successfully" + an activation ETA) for a transaction that was not
//! retained. Reporting must distinguish accepted-and-retained /
//! rejected-at-submit / accepted-then-dropped.
//!
//! `bins/cli/src/cmd_producer/register.rs:195-218` prints the success line, the
//! TX hash and an activation ETA, and returns `Ok(())` (exit 0), based ONLY on
//! `sendTransaction` returning `Ok`. It never re-queries the node about the tx.
//!
//! TWO SEPARATE DEFECTS ARE COVERED — do not collapse them:
//!
//!   (a) SAME-INPUT duplicate. The node accepts the tx at submit, then
//!       `Mempool::revalidate` evicts it once the first duplicate mines. From
//!       the CLI's vantage point this is exactly `sendTransaction -> Ok(hash)`
//!       followed by `getTransaction(hash) -> tx_not_found`. Pure CLI-reporting
//!       defect: it survives any node-side mempool fix.
//!
//!   (b) ALREADY-PENDING duplicate (disjoint inputs). Here the second
//!       registration IS retained and IS mined, so a naive retention probe
//!       still reports success. The stub therefore answers `getTransaction`
//!       with a fully-confirmed transaction for this partition. It is reachable
//!       because the CLI's own duplicate pre-check at `register.rs:31`
//!       (`"pending" => bail!`) is UNREACHABLE DEAD CODE: the singular
//!       `getProducer` emits only active/unbonding/exited/slashed
//!       (`crates/rpc/src/methods/producer.rs:65-70`) and errors outright with
//!       `producer_not_found` for a first-time registrant whose Register still
//!       sits in `pending_updates`. Only the PLURAL `getProducers` emits
//!       `"pending"` (`crates/rpc/src/methods/producer.rs:253`).
//!
//! HARNESS. `doli-cli` is a bin-only crate (no `src/lib.rs`), so `handle_register`
//! cannot be linked. The tests drive the real binary via
//! `env!("CARGO_BIN_EXE_doli")` against a loopback stub JSON-RPC node built on
//! `std::net::TcpListener`. Zero new dependencies.
//!
//! The stub's responses are modelled on the real node so the tests stay honest:
//!
//! * `getProducer` -> JSON-RPC error `-32006 Producer not found`
//!   (`crates/rpc/src/error.rs:157-159`) — what a real node returns while a
//!   Register is only in `pending_updates`.
//! * `getProducers` -> the pending entry shape of
//!   `crates/rpc/src/methods/producer.rs:244-270`.
//! * `getTransaction` -> the three states of
//!   `crates/rpc/src/methods/transaction.rs:16-76`: in-mempool
//!   (`blockHeight`/`confirmations` OMITTED — they are
//!   `skip_serializing_if = "Option::is_none"` in
//!   `crates/rpc/src/types/block.rs:131-141`), mined, or the JSON-RPC error
//!   `-32001 Transaction not found`.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Literal strings under test — copied verbatim from
// bins/cli/src/cmd_producer/register.rs:197,203,204-209
// ---------------------------------------------------------------------------

/// `register.rs:197` — `println!("Registration submitted successfully!");`
const SUCCESS_LINE: &str = "Registration submitted successfully!";

/// `register.rs:204-209` — `"Estimated activation: ~{} minutes (Epoch {}, block {})."`
/// Interpolation-free prefix, safe for presence/absence assertions.
const ETA_LINE_PREFIX: &str = "Estimated activation: ~";

/// `register.rs:203` — `println!("Status: Pending activation (epoch-deferred).",);`
const PENDING_STATUS_LINE: &str = "Status: Pending activation (epoch-deferred).";

/// Hash the stub returns from `sendTransaction` (64 hex chars).
const SUBMITTED_TX_HASH: &str = "b8207ef4b8207ef4b8207ef4b8207ef4b8207ef4b8207ef4b8207ef4b8207ef4";

const FUNDING_TX_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const BEST_BLOCK_HASH: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const GENESIS_HASH: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const MINED_BLOCK_HASH: &str = "4444444444444444444444444444444444444444444444444444444444444444";

/// One bond = 10 DOLI in the stub network.
const BOND_UNIT: u64 = 1_000_000_000;

// ---------------------------------------------------------------------------
// Stub node
// ---------------------------------------------------------------------------

/// Which node-observable reality the stub models.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scenario {
    /// PARTITION (a): submit accepted, then the tx is gone.
    /// `sendTransaction -> Ok`, every `getTransaction -> tx_not_found`.
    SubmitOkThenDropped,
    /// PARTITION (b): a registration for this key is ALREADY pending, and the
    /// second one is retained and mined. A retention-only probe cannot see this.
    AlreadyPending,
    /// POSITIVE CONTROL: the tx is retained (in mempool, then mined).
    SubmitOkAndRetained,
}

struct StubState {
    scenario: Scenario,
    /// Wallet public key (hex) — used to build the pending `getProducers` entry.
    producer_pubkey_hex: String,
    /// Every JSON-RPC method the CLI called, in order. Reported on assertion
    /// failure so a harness problem is never mistaken for the defect.
    calls: Vec<String>,
    get_tx_calls: u32,
}

struct Stub {
    addr: SocketAddr,
    state: Arc<Mutex<StubState>>,
}

impl Stub {
    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
}

fn start_stub(scenario: Scenario, producer_pubkey_hex: &str) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stub: bind loopback");
    let addr = listener.local_addr().expect("stub: local_addr");
    let state = Arc::new(Mutex::new(StubState {
        scenario,
        producer_pubkey_hex: producer_pubkey_hex.to_string(),
        calls: Vec::new(),
        get_tx_calls: 0,
    }));

    let thread_state = Arc::clone(&state);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            serve_one(&mut stream, &thread_state);
            let _ = stream.shutdown(Shutdown::Both);
        }
    });

    Stub { addr, state }
}

fn serve_one(stream: &mut TcpStream, state: &Arc<Mutex<StubState>>) {
    let Some(body) = read_http_body(stream) else {
        return;
    };
    let request: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let payload = {
        let mut s = state.lock().unwrap();
        s.calls.push(method.clone());
        respond(&mut s, &method, &request)
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

/// Read an HTTP request and return its body, honouring `Content-Length`.
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

fn ok(result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "result": result })
}

fn rpc_err(code: i32, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(d) = data {
        error["data"] = d;
    }
    json!({ "jsonrpc": "2.0", "id": 1, "error": error })
}

fn respond(state: &mut StubState, method: &str, request: &Value) -> Value {
    match method {
        // ping() + handle_register:39
        "getChainInfo" => ok(json!({
            "network": "devnet",
            "bestHash": BEST_BLOCK_HASH,
            "bestHeight": 1_000u64,
            "bestSlot": 1_000u64,
            "genesisHash": GENESIS_HASH,
            "rewardPoolBalance": 0u64,
        })),

        // handle_register:40
        "getNetworkParams" => ok(json!({
            "network": "devnet",
            "bondUnit": BOND_UNIT,
            "slotDuration": 1u64,
            "slotsPerEpoch": 100u32,
            "blocksPerRewardEpoch": 100u64,
            "coinbaseMaturity": 10u64,
            "initialReward": 100_000_000u64,
            "genesisTime": 0u64,
        })),

        // handle_register:53 — one fat spendable normal UTXO
        "getUtxos" => ok(json!([{
            "txHash": FUNDING_TX_HASH,
            "outputIndex": 0u32,
            "amount": 50 * BOND_UNIT,
            "outputType": "normal",
            "lockUntil": 0u64,
            "height": 10u64,
            "spendable": true,
            "pending": false,
        }])),

        // handle_register:26. A real node answers `producer_not_found` for a
        // first-time registrant even when a Register is pending, because
        // `get_by_pubkey` reads only the committed `producers` map
        // (crates/storage/src/producer/set_core.rs:292-294) while pending
        // registrations live in the disjoint `pending_updates`
        // (set_core.rs:264-274). This is why register.rs:31 is dead code.
        "getProducer" => rpc_err(-32006, "Producer not found", None),

        // Only the PLURAL method can surface a pending registration
        // (crates/rpc/src/methods/producer.rs:244-270).
        "getProducers" => match state.scenario {
            Scenario::AlreadyPending => ok(json!([{
                "publicKey": state.producer_pubkey_hex,
                "addressHash": "",
                "registrationHeight": 995u64,
                "bondAmount": BOND_UNIT,
                "bondCount": 1u32,
                "status": "pending",
                "era": 1u64,
                "pendingWithdrawals": [],
                "pendingUpdates": [{ "updateType": "register", "bondCount": 1u32 }],
                "blsPubkey": "",
                "delegatedTo": Value::Null,
                "delegatedBonds": 0u32,
                "receivedDelegations": [],
                "selectionWeight": 0u64,
            }])),
            _ => ok(json!([])),
        },

        // handle_register:195. The node ACCEPTS in every partition — that is the
        // whole point: a bare OK is not evidence of retention.
        "sendTransaction" => ok(json!(SUBMITTED_TX_HASH)),

        // handle_register:201
        "getEpochInfo" => ok(json!({
            "currentHeight": 1_000u64,
            "currentEpoch": 10u64,
            "lastCompleteEpoch": 9u64,
            "blocksPerEpoch": 100u64,
            "blocksRemaining": 42u64,
            "epochStartHeight": 1_000u64,
            "epochEndHeight": 1_100u64,
            "blockReward": 100_000_000u64,
        })),

        // NOT called by the CLI today — that is the defect. Scripted so a
        // retention-verifying implementation gets an honest node answer.
        "getTransaction" => {
            state.get_tx_calls += 1;
            let hash = request
                .pointer("/params/hash")
                .and_then(Value::as_str)
                .unwrap_or(SUBMITTED_TX_HASH)
                .to_string();
            match state.scenario {
                // (a) accepted-then-dropped: crates/rpc/src/methods/transaction.rs:39/45/75
                Scenario::SubmitOkThenDropped => rpc_err(
                    -32001,
                    "Transaction not found",
                    Some(json!({ "searched_by": "hash", "hash": hash })),
                ),
                // (b) retained AND mined — a retention probe says "fine".
                Scenario::AlreadyPending => ok(mined_tx(&hash)),
                // positive control: in-mempool first, then mined.
                Scenario::SubmitOkAndRetained => {
                    if state.get_tx_calls == 1 {
                        ok(mempool_tx(&hash))
                    } else {
                        ok(mined_tx(&hash))
                    }
                }
            }
        }

        "getMempoolTransactions" => match state.scenario {
            Scenario::SubmitOkAndRetained => ok(json!([{
                "hash": SUBMITTED_TX_HASH,
                "txType": "registration",
                "size": 500usize,
                "fee": 1u64,
                "feeRate": 1u64,
                "addedTime": 1_700_000_000u64,
            }])),
            _ => ok(json!([])),
        },

        "getMempoolInfo" => ok(json!({
            "txCount": 0u64,
            "totalSize": 0u64,
            "minFeeRate": 1u64,
            "maxSize": 1_000_000u64,
            "maxCount": 5_000u64,
        })),

        _ => rpc_err(-32601, "Method not found", None),
    }
}

/// In-mempool shape: `crates/rpc/src/methods/transaction.rs:24-32` sets ONLY
/// `fee`; `blockHash` / `blockHeight` / `confirmations` are omitted entirely
/// (`skip_serializing_if = "Option::is_none"`, block.rs:131-141).
fn mempool_tx(hash: &str) -> Value {
    json!({
        "hash": hash,
        "version": 1u32,
        "txType": "registration",
        "inputs": [],
        "outputs": [],
        "size": 500usize,
        "fee": 1u64,
    })
}

/// Mined shape: `crates/rpc/src/methods/transaction.rs:53-56`.
fn mined_tx(hash: &str) -> Value {
    json!({
        "hash": hash,
        "version": 1u32,
        "txType": "registration",
        "inputs": [],
        "outputs": [],
        "size": 500usize,
        "fee": 1u64,
        "blockHash": MINED_BLOCK_HASH,
        "blockHeight": 1_001u64,
        "confirmations": 1u64,
    })
}

// ---------------------------------------------------------------------------
// Wallet fixture + CLI invocation
// ---------------------------------------------------------------------------

struct WalletFixture {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    public_key_hex: String,
}

/// Write a `wallet.json` matching `bins/cli/src/wallet.rs` (`Wallet` +
/// `WalletAddress`). A BLS key is mandatory — `register.rs:131-136` bails
/// without one.
fn make_wallet() -> WalletFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("wallet.json");

    let kp = crypto::KeyPair::generate();
    let bls = crypto::BlsKeyPair::generate();
    let public_key_hex = kp.public_key().to_hex();

    let wallet = json!({
        "name": "inc-i-148-test",
        "version": 2u32,
        "addresses": [{
            "address": kp.address().to_hex(),
            "public_key": public_key_hex,
            "private_key": kp.private_key().to_hex(),
            "label": "primary",
            "bls_private_key": bls.secret_key().to_hex(),
            "bls_public_key": bls.public_key().to_hex(),
        }],
    });

    std::fs::write(&path, serde_json::to_string_pretty(&wallet).unwrap())
        .expect("write wallet.json");

    WalletFixture {
        _dir: dir,
        path,
        public_key_hex,
    }
}

struct CliRun {
    exit_code: Option<i32>,
    success: bool,
    stdout: String,
    stderr: String,
}

fn run_register(wallet: &WalletFixture, stub: &Stub) -> CliRun {
    let output = Command::new(env!("CARGO_BIN_EXE_doli"))
        .arg("--wallet")
        .arg(&wallet.path)
        .arg("--network")
        .arg("devnet")
        .arg("--rpc")
        .arg(stub.url())
        .args(["producer", "register", "--bonds", "1"])
        .output()
        .expect("failed to run the doli binary");

    CliRun {
        exit_code: output.status.code(),
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn report(label: &str, run: &CliRun, stub: &Stub) -> String {
    format!(
        "\n--- {label} ---\nexit_code: {:?}\nrpc calls: {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}\n",
        run.exit_code,
        stub.calls(),
        run.stdout,
        run.stderr
    )
}

/// Guard against a harness failure masquerading as the defect. The register
/// flow must have reached the submit step for the assertions to mean anything.
fn assert_reached_submit(run: &CliRun, stub: &Stub, label: &str) {
    let calls = stub.calls();
    assert!(
        calls.iter().any(|m| m == "sendTransaction"),
        "HARNESS FAILURE (not the defect): the CLI never reached sendTransaction.{}",
        report(label, run, stub)
    );
}

/// Evaluate ALL THREE output-contract cells for a path that must NOT report
/// success, and return every violation. Collecting instead of short-circuiting
/// keeps the failure output complete: O1, O2 and O3 are all reported at once.
fn violations_for_must_fail_path(run: &CliRun) -> Vec<String> {
    let mut v = Vec::new();

    // O1 — process exit code.
    if run.success {
        v.push(format!(
            "O1 exit code: expected != 0, got {:?}",
            run.exit_code
        ));
    }

    // O2 — stdout success line.
    if run.stdout.contains(SUCCESS_LINE) {
        v.push(format!("O2 success line: stdout contains {SUCCESS_LINE:?}"));
    }

    // O3 — stdout activation-ETA line (and the heading that introduces it).
    if run.stdout.contains(ETA_LINE_PREFIX) {
        v.push(format!(
            "O3 activation-ETA line: stdout contains {ETA_LINE_PREFIX:?}"
        ));
    }
    if run.stdout.contains(PENDING_STATUS_LINE) {
        v.push(format!(
            "O3 activation-ETA heading: stdout contains {PENDING_STATUS_LINE:?}"
        ));
    }

    v
}

// ---------------------------------------------------------------------------
// PARTITION (a) — same-input duplicate: accepted at submit, then evicted.
// ---------------------------------------------------------------------------

/// INC-I-148 / INV-CLI-002.
///
/// The node accepts the registration (`sendTransaction -> Ok`) and then drops
/// it (`getTransaction -> tx_not_found`), exactly as `Mempool::revalidate` does
/// to a same-input duplicate once the first copy mines.
///
/// FAILS TODAY: `register.rs:195-218` never re-queries the node, so it prints
/// the success line + the activation ETA and returns `Ok(())` -> exit 0.
#[test]
fn register_reports_failure_when_tx_is_dropped() {
    let wallet = make_wallet();
    let stub = start_stub(Scenario::SubmitOkThenDropped, &wallet.public_key_hex);
    let run = run_register(&wallet, &stub);
    let label = "PARTITION (a) submit-OK-then-dropped";

    assert_reached_submit(&run, &stub, label);

    let violations = violations_for_must_fail_path(&run);
    assert!(
        violations.is_empty(),
        "INV-CLI-002 VIOLATED — the CLI reported success for a transaction the node did not retain.\nviolations:\n  - {}{}",
        violations.join("\n  - "),
        report(label, &run, &stub)
    );
}

// ---------------------------------------------------------------------------
// PARTITION (b) — already-pending duplicate with disjoint inputs.
// ---------------------------------------------------------------------------

/// INC-I-148 / INV-CLI-002, second partition.
///
/// A Register for this key is already pending. The stub reports it the way a
/// real node does: `getProducer` errors with `producer_not_found` (the pending
/// entry is not in the committed producer map) while `getProducers` lists the
/// key with `status: "pending"`.
///
/// The second registration IS retained and IS mined here — `getTransaction`
/// returns `confirmations: 1`. A retention-only fix therefore does NOT rescue
/// this case; the CLI must refuse on UNIQUENESS grounds.
///
/// FAILS TODAY: the `"pending" => bail!` arm at `register.rs:31` is unreachable
/// (the singular `getProducer` cannot emit `"pending"` — producer.rs:65-70), so
/// the CLI builds and submits a duplicate and reports plain success.
#[test]
fn register_reports_failure_when_registration_already_pending() {
    let wallet = make_wallet();
    let stub = start_stub(Scenario::AlreadyPending, &wallet.public_key_hex);
    let run = run_register(&wallet, &stub);
    let label = "PARTITION (b) already-pending-rejected";

    let violations = violations_for_must_fail_path(&run);
    assert!(
        violations.is_empty(),
        "INV-CLI-002 VIOLATED — the CLI reported plain success while a registration for this key was already pending.\nviolations:\n  - {}{}",
        violations.join("\n  - "),
        report(label, &run, &stub)
    );
}

// ---------------------------------------------------------------------------
// POSITIVE CONTROL — must PASS before AND after the fix.
// ---------------------------------------------------------------------------

/// The happy path must keep reporting success. This pins the fix so it cannot
/// be "always fail": submit is accepted, no duplicate is pending, and
/// `getTransaction` shows the tx retained (in mempool, then mined).
///
/// PASSES TODAY and must keep passing.
#[test]
fn register_reports_success_when_tx_is_retained() {
    let wallet = make_wallet();
    let stub = start_stub(Scenario::SubmitOkAndRetained, &wallet.public_key_hex);
    let run = run_register(&wallet, &stub);
    let label = "POSITIVE CONTROL submit-OK-and-retained";

    assert_reached_submit(&run, &stub, label);

    // O1 — exit 0.
    assert_eq!(
        run.exit_code,
        Some(0),
        "positive control (O1): CLI must exit 0 for a retained registration.{}",
        report(label, &run, &stub)
    );

    // O2 — the success line must be present.
    assert!(
        run.stdout.contains(SUCCESS_LINE),
        "positive control (O2): stdout is missing {SUCCESS_LINE:?}.{}",
        report(label, &run, &stub)
    );

    // O3 — the activation-ETA line must be present.
    assert!(
        run.stdout.contains(ETA_LINE_PREFIX),
        "positive control (O3): stdout is missing {ETA_LINE_PREFIX:?}.{}",
        report(label, &run, &stub)
    );

    assert!(
        run.stdout.contains(PENDING_STATUS_LINE),
        "positive control (O3): stdout is missing {PENDING_STATUS_LINE:?}.{}",
        report(label, &run, &stub)
    );
}
