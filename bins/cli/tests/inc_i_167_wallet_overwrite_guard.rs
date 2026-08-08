// OUTPUT CONTRACT: fn Wallet::save(&self, path: &Path) -> Result<()>
//   Defined in bins/cli/src/wallet.rs; reached from bins/cli/src/cmd_wallet.rs
//   (cmd_import, cmd_export, cmd_address, cmd_new, cmd_restore) and from
//   bins/cli/src/cmd_init.rs (cmd_init, including --force).
//   `doli-cli` is a bin-only crate, so save() is observed across the process
//   boundary through the real binary (`env!("CARGO_BIN_EXE_doli")`).
//
//   O1: destination file identity — WHOSE wallet is at `path` afterwards, read
//       back as addresses[0].public_key from the JSON on disk. The only output
//       that distinguishes "refused" from "destroyed"; PRESERVED / REPLACED / CREATED.
//   O2: process exit status       — success / failure.
//   O3: stderr refusal text       — PRESENT/ABSENT ("Refusing to overwrite" +
//       remediation naming --force; wallet.rs save() guard).
//
// PATHS: the three call shapes save() must distinguish, by (does `path` exist?)
//        x (is `path` the file this wallet was loaded from?):
//   P1: save-back      — path EXISTS and path == origin   (cmd_address: load(w) -> save(w))
//                        MUST overwrite; this is how every legitimate mutation persists.
//   P2: create         — path ABSENT                      (cmd_import into a fresh path)
//                        MUST create.
//   P3: cross-path     — path EXISTS and path != origin   (cmd_import over a live wallet)
//                        MUST refuse. THE DEFECT.
//
// INPUT PARTITIONS: one partition per path, and one is sufficient — the quantity
//   under test is a two-term predicate (exists? / same-origin?) whose terms are
//   both fully determined by the path arguments, not by wallet contents. Wallet
//   contents cannot change which branch is taken, so a second contents-partition
//   is provably blind to the defect and adds no cell. The one partition chosen for
//   P3 is the one that reproduces the operator's real situation: the destination
//   holds a DIFFERENT, valid, key-bearing wallet (a live producer's wallet.json,
//   which per INC-I-162 is the sole durable carrier of its BLS secret).
//
// MATRIX: 3 outputs x 3 paths x 1 partition = 9 cells, every cell asserted below.
//
//   path                  | O1 destination identity | O2 exit  | O3 refusal text
//   ----------------------|-------------------------|----------|----------------
//   P1 save-back          | PRESERVED (+1 address)  | success  | ABSENT
//   P2 create             | CREATED (= source)      | success  | ABSENT
//   P3 cross-path         | PRESERVED (= victim)    | failure  | PRESENT
//
//! INC-I-167 reproduction — `Wallet::save()` must not clobber a different wallet.
//!
//! DEFECT. `bins/cli/src/wallet.rs:142` `save()` writes unconditionally, and safety
//! is opt-in at each callsite. Two callers guard (`cmd_new` at cmd_wallet.rs:11-16,
//! `cmd_restore` at :69-74); `cmd_import` at :803-811 does not — it calls
//! `Wallet::import(input)` then `wallet.save(wallet_path)` with no existence check
//! and no prompt. On a producer host `wallet.json` may be the ONLY copy of the
//! registered BLS secret (INC-I-162), which the 24-word seed phrase does not
//! restore, so a single `doli import` silently and irreversibly destroys the
//! producer identity. Recovery then costs ~75% of the bond via exit + re-register.
//!
//! P3 below FAILS before the fix: `doli import` exits 0 and the victim wallet has
//! been replaced by the source wallet.

use std::path::Path;
use std::process::{Command, Output};

/// Run the real `doli` binary with an explicit wallet path and deterministic network.
fn doli(wallet: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_doli"));
    cmd.arg("--network")
        .arg("devnet")
        .arg("-w")
        .arg(wallet)
        .args(args);
    cmd.output().expect("failed to run doli binary")
}

/// Read `addresses[0].public_key` from a wallet file on disk — the identity marker.
/// Public key only: no secret material is ever read, printed, or asserted on.
fn primary_public_key(path: &Path) -> String {
    let contents = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read wallet {}: {e}", path.display()));
    let v: serde_json::Value =
        serde_json::from_str(&contents).expect("wallet file is not valid JSON");
    v["addresses"][0]["public_key"]
        .as_str()
        .expect("addresses[0].public_key missing")
        .to_string()
}

fn address_count(path: &Path) -> usize {
    let contents = std::fs::read_to_string(path).expect("cannot read wallet");
    let v: serde_json::Value = serde_json::from_str(&contents).expect("invalid JSON");
    v["addresses"]
        .as_array()
        .expect("addresses not an array")
        .len()
}

#[test]
fn inc_i_167_save_must_refuse_to_clobber_a_different_wallet() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let victim = tmp.path().join("victim.json");
    let source = tmp.path().join("source.json");
    let fresh = tmp.path().join("fresh.json");

    // Two independent wallets. `victim` stands in for a live producer's wallet.json.
    let out = doli(&victim, &["new"]);
    assert!(out.status.success(), "setup: creating victim wallet failed");
    let out = doli(&source, &["new"]);
    assert!(out.status.success(), "setup: creating source wallet failed");

    let victim_pk = primary_public_key(&victim);
    let source_pk = primary_public_key(&source);
    assert_ne!(
        victim_pk, source_pk,
        "setup invalid: the two wallets must have different identities"
    );

    // ---- P3: cross-path overwrite (path EXISTS, path != origin) — THE DEFECT ----
    let out = doli(&victim, &["import", source.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    // O1 x P3 — the destination identity must be PRESERVED.
    assert_eq!(
        primary_public_key(&victim),
        victim_pk,
        "P3/O1: `doli import` REPLACED the wallet at the destination path. \
         On a producer host this destroys the only copy of the registered BLS key."
    );
    // O2 x P3 — must fail, not silently succeed.
    assert!(
        !out.status.success(),
        "P3/O2: `doli import` over an existing wallet must fail, but it exited 0. stderr: {stderr}"
    );
    // O3 x P3 — must explain itself and name the escape hatch.
    assert!(
        stderr.contains("Refusing to overwrite"),
        "P3/O3: refusal must say 'Refusing to overwrite'. stderr: {stderr}"
    );
    assert!(
        stderr.contains("--force"),
        "P3/O3: refusal must name the --force remediation. stderr: {stderr}"
    );

    // ---- P2: create (path ABSENT) — must still work ----
    let out = doli(&fresh, &["import", source.to_str().unwrap()]);
    let stderr_p2 = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "P2/O2: import into a fresh path must succeed. stderr: {stderr_p2}"
    );
    assert_eq!(
        primary_public_key(&fresh),
        source_pk,
        "P2/O1: import into a fresh path must create the source wallet there"
    );
    assert!(
        !stderr_p2.contains("Refusing to overwrite"),
        "P2/O3: creating at an absent path must not be refused. stderr: {stderr_p2}"
    );

    // ---- P1: save-back (path EXISTS, path == origin) — must still overwrite ----
    let before = address_count(&victim);
    let out = doli(&victim, &["address", "--label", "second"]);
    let stderr_p1 = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        out.status.success(),
        "P1/O2: save-back to the wallet's own path must succeed — this is how every \
         legitimate mutation persists. stderr: {stderr_p1}"
    );
    assert_eq!(
        primary_public_key(&victim),
        victim_pk,
        "P1/O1: save-back must preserve the primary identity"
    );
    assert_eq!(
        address_count(&victim),
        before + 1,
        "P1/O1: save-back must have persisted the new address"
    );
    assert!(
        !stderr_p1.contains("Refusing to overwrite"),
        "P1/O3: save-back must not be refused. stderr: {stderr_p1}"
    );
}
