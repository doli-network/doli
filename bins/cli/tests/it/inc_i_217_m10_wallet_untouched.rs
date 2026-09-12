//! INC-I-217 M10 — `doli producer rotate-bls` must never write to `wallet.json`.
//!
//! HONEST STATUS: **TRIPWIRE, NOT A RED TEST.** At HEAD the `rotate-bls`
//! subcommand does not exist, so clap exits 2 with "unrecognized subcommand"
//! and the wallet is trivially untouched — this test PASSES today. It is not
//! faked into a red, because the only red available (assert the command exists)
//! is already covered by the pure module. Its value is forward-looking: it
//! fails the first time an implementation writes to `wallet.json` on ANY path
//! of the IO half, which is exactly the regression the pure module is
//! structurally blind to (`rotate_tx.rs` holds no file handle at all).
//!
//! WHY IT MATTERS: rotation changes the CHAIN, not the wallet. The natural
//! wrong implementation — "rotate, then persist the new key locally" — rewrites
//! the operator's only copy of the BLS secret from inside a command whose RPC
//! step can fail at any point. That is the INC-I-162 / INC-I-167 data-loss
//! class: a truncated or half-written `wallet.json` costs ~75% of the bond via
//! exit + re-register.
//!
//! Authority: `specs/bls-key-rotation-requirements.md:310` (REQ-ROT-013
//! acceptance) — "After the command `wallet.json` is byte-identical (the wallet
//! is never modified)." Flagged as an untestable gap by the pure-module pass
//! (SPECS GAPS #2 in `docs/.workflow/inc-i-217-M10-test-red-evidence.txt`);
//! this module closes it at the process boundary.
//!
//! OUTPUT CONTRACT — `doli producer rotate-bls` observed as a process.
//!   The unit under test is the whole binary invocation, so the outputs are
//!   filesystem effects, not return values.
//!   O1 `wallet.json` content   — the raw bytes, before vs after. THE contract.
//!   O2 wallet-directory census — the set of file names in the wallet's
//!      directory; catches "wrote a sibling `wallet.json.bak` / `.tmp`", which
//!      leaves O1 intact while still publishing the secret to a second file.
//!   O3 process exit status     — deliberately NOT asserted: the requirement is
//!      "byte-identical" on EVERY path, so binding this test to an exit code
//!      would make it stop asserting the moment the command starts succeeding.
//!      Captured only for the failure message.
//!   (No other output exists at this boundary: stdout/stderr content and the
//!   built transaction are covered by the pure module's O9..O18.)
//!
//! INPUT PARTITIONS: one, and one is sufficient. The quantity under test is
//!   "did this process write to the wallet path", which no wallet CONTENT can
//!   turn on or off — a command that never opens the file for writing is
//!   byte-safe for every wallet, and one that does is unsafe for every wallet.
//!   The partition chosen is the one an operator actually has: a real
//!   CLI-created version-3 wallet holding a real seed-derived BLS key, against
//!   an RPC endpoint that is guaranteed refused, so the run reaches (and fails
//!   at) the network step with no node and no network dependency.
//!
//! MATRIX: O1 x P1; O2 x P1.
//!
//! HERMETIC: no node, no fixed port, no shared state, no write outside the
//! per-test tempdir. stdin is `/dev/null` so a consent prompt can never hang.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use sha2::{Digest, Sha256};

/// Run the real `doli` binary against an explicit wallet path on a deterministic
/// network, with stdin closed.
fn doli(wallet: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_doli"))
        .arg("--network")
        .arg("devnet")
        .arg("--wallet")
        .arg(wallet)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("failed to run doli binary")
}

/// A loopback port with nothing listening on it: bind an ephemeral port, read
/// it, drop the listener. No fixed port number, so concurrent test binaries
/// cannot collide, and the connection is refused rather than hanging.
fn closed_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback port");
    let port = listener
        .local_addr()
        .expect("ephemeral listener has no local address")
        .port();
    drop(listener);
    port
}

/// O1 as a printable identity. A digest, never the bytes: `wallet.json` holds
/// the plaintext Ed25519 and BLS secrets, and assertion messages get pasted
/// into bug reports.
fn wallet_digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// O2 — the sorted file names beside the wallet.
fn dir_census(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("cannot list the wallet directory")
        .map(|e| {
            e.expect("cannot read a directory entry")
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    names.sort();
    names
}

/// P1 — a real CLI-created producer wallet: version 3, with a seed-derived BLS
/// key, which is what every operator who can run `rotate-bls` at all will have.
fn new_producer_wallet(wallet: &Path) {
    let out = doli(wallet, &["new"]);
    assert!(
        out.status.success(),
        "setup: `doli new` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(wallet).expect("setup: the new wallet is not readable"),
    )
    .expect("setup: the new wallet is not JSON");
    assert!(
        json["addresses"][0]["bls_public_key"].is_string(),
        "setup: the fixture must hold a BLS key, or 'the wallet is unchanged' would be a \
         statement about a file with nothing at stake in it"
    );
}

// REQ-ROT-013 — Decision: whether the IO half of `rotate-bls` ever persists anything into
// wallet.json. A failure here means a command whose entire effect belongs on the chain rewrote
// the operator's only copy of the BLS secret — and it can only fail for that reason, since a
// command that never opens the file for writing cannot change its bytes.
#[test]
fn inc_i_217_rotate_bls_leaves_the_wallet_byte_identical() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let wallet = tmp.path().join("wallet.json");
    new_producer_wallet(&wallet);

    let before_bytes = std::fs::read(&wallet).expect("read the wallet before");
    let before_digest = wallet_digest(&before_bytes);
    let before_census = dir_census(tmp.path());
    assert!(
        !before_bytes.is_empty(),
        "non-vacuity: an empty fixture would make 'unchanged' trivially true"
    );

    // ---- P1: rotate-bls against a guaranteed-refused RPC endpoint ----
    let rpc = format!("http://127.0.0.1:{}", closed_loopback_port());
    let out = doli(&wallet, &["--rpc", &rpc, "producer", "rotate-bls", "--yes"]);
    let exit = out.status.code();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    // O1 x P1 — compared through a digest, never through assert_eq! on the raw
    // bytes: a byte-diff in the panic message would print the secrets.
    let after_bytes = std::fs::read(&wallet).expect(
        "O1: the wallet file must still exist after rotate-bls — it was deleted, renamed, or \
         replaced by a directory",
    );
    let after_digest = wallet_digest(&after_bytes);
    assert!(
        after_bytes == before_bytes,
        "REQ-ROT-013 / O1: `doli producer rotate-bls` must leave wallet.json byte-identical on \
         EVERY path — rotation changes the chain, not the wallet. \
         before sha256={before_digest} ({} bytes), after sha256={after_digest} ({} bytes). \
         exit={exit:?} stderr: {stderr}",
        before_bytes.len(),
        after_bytes.len()
    );

    // O2 x P1 — a backup or temp file beside the wallet is the same disclosure
    // with the byte-identity assertion still green.
    assert_eq!(
        dir_census(tmp.path()),
        before_census,
        "REQ-ROT-013 / O2: rotate-bls must not create or remove any file beside the wallet \
         (a `.bak` or `.tmp` copy publishes the BLS secret to a second file). \
         exit={exit:?} stderr: {stderr}"
    );
}
