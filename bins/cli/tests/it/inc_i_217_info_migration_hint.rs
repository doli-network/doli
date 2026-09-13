// OUTPUT CONTRACT: `doli [--network N] -w <wallet> info` — the legacy-wallet branch of
//   `cmd_info` (bins/cli/src/cmd_wallet.rs), i.e. the `else` of `Wallet::bls_is_seed_derived()`.
//   INC-I-217. `doli-cli` is a bin-only crate for wallet purposes, so every output below is
//   observed across the process boundary through the real binary (`env!("CARGO_BIN_EXE_doli")`),
//   as in inc_i_217_import_bls_golden.rs.
//
//   O1: proactive-migration hint on stdout — the `RECOMMENDED` block. PRINTED / ABSENT.
//       The only output that tells a pre-v3 operator the exposure can be CLOSED, not just
//       backed up. Absence on a v3 wallet is as load-bearing as presence on a v1/v2 one:
//       a v3 operator who runs a needless rotation pays a fee and a restart for nothing.
//   O2: the activation-height token inside the hint — the literal height for the invoked
//       network, or the words "not yet activated on this network" when the height is
//       u64::MAX. Read from NetworkParams::defaults(network).bls_key_rotation_activation_height
//       (crates/core/src/network_params/defaults.rs), never hardcoded.
//   O3: process exit status — `info` is a read-only report; it must stay success on every
//       wallet version.
//   O4: wallet file bytes — `info` writes nothing. A print change must not become a
//       migration of the file it describes.
//   O5: secret material — the hint must not print the BLS private key it talks about
//       (specs/bls-key-rotation-architecture-security.md).
//   O6: the doc pointer `docs/bls-key-recovery.md` — the hint is 5 lines; the procedure is
//       not. Without the pointer the operator has the verdict and no runbook.
//
// PATHS:
//   P1: legacy wallet (version < 3) that HOLDS a BLS key — the exposure exists, hint printed.
//   P2: version-3 wallet — the phrase already reproduces the key, no hint.
//   P3: legacy wallet with NO BLS key — nothing to rotate (no on-chain key derives from this
//       file); the hint is gated by the same `primary_bls_public_key().is_some()` test as the
//       existing getProducer comparison line, so it must stay absent.
//
// INPUT PARTITIONS (P1 only — P2 and P3 suppress the block, so the height never renders):
//   P1a: --network mainnet — finite height 457855. Distinct relationship: a REAL height
//        the operator can compare against `getChainInfo -> height` today.
//   P1b: --network testnet — finite height 176200. Same branch as P1a, different number:
//        this partition is what makes "read from NetworkParams" non-vacuous. A hardcoded
//        457855 passes P1a and fails here.
//   P1c: --network devnet — u64::MAX. The number must NOT render: printed as a decimal it
//        would read as height 18446744073709551615, and any operator comparison against it
//        is nonsense. Prose replaces it.
//
// MATRIX: 6 outputs × 5 (paths × partitions) = 30 cells.
//   P1a: O1 printed / O2 "457855" / O3 success / O4 unchanged / O5 no secret / O6 present
//   P1b: O1 printed / O2 "176200" / O3 success / O4 unchanged / O5 no secret / O6 present
//   P1c: O1 printed / O2 prose, no MAX digits / O3 success / O4 unchanged / O5 no secret / O6 present
//   P2 : O1 absent  / O2 absent (no height anywhere) / O3 success / O4 unchanged / O5 no secret / O6 absent
//   P3 : O1 absent  / O2 absent / O3 success / O4 unchanged / O5 no secret / O6 absent
//
//! INC-I-217 — `doli info` must not stop at "back up the file". A pre-version-3 wallet holds a
//! BLS attestation key that OsRng produced (crates/crypto/src/bls.rs) and the 24 words cannot
//! reproduce (crates/wallet/src/wallet.rs version history). Since the rotation feature merged,
//! that exposure is CLOSEABLE: restore the phrase into a fresh file, rotate to the phrase-derived
//! key, repoint the node. These tests pin that `info` says so, says it only where it is true, and
//! quotes the activation height of the network the operator actually invoked.

use std::path::Path;
use std::process::{Command, Output};

/// Run the real `doli` binary against an explicit wallet path on an explicit network.
fn doli(network: &str, wallet: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_doli"))
        .arg("--network")
        .arg(network)
        .arg("-w")
        .arg(wallet)
        .args(args)
        .output()
        .expect("failed to run doli binary")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn combined(out: &Output) -> String {
    format!("{}{}", stdout_of(out), stderr_of(out))
}

fn wallet_json(path: &Path) -> serde_json::Value {
    let contents = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read wallet {}: {e}", path.display()));
    serde_json::from_str(&contents).expect("wallet file is not valid JSON")
}

/// O5 — read only to prove it never reaches the terminal. Never printed, never interpolated
/// into a failure message.
fn stored_bls_secret(path: &Path) -> Option<String> {
    wallet_json(path)["addresses"][0]["bls_private_key"]
        .as_str()
        .map(str::to_string)
}

/// The marker of the hint block — first line, so a partial print fails too.
const HINT_MARKER: &str = "RECOMMENDED";
const DOC_POINTER: &str = "docs/bls-key-recovery.md";
const NOT_ACTIVATED: &str = "not yet activated on this network";

/// A version-3 wallet, exactly as `doli new` writes it.
fn new_v3_wallet(network: &str, path: &Path) {
    let out = doli(network, path, &["new"]);
    assert!(
        out.status.success(),
        "setup: `doli new` failed: {}",
        stderr_of(&out)
    );
    assert_eq!(
        wallet_json(path)["version"].as_u64(),
        Some(3),
        "setup: WALLET_VERSION_SEED_DERIVED_BLS must stay 3 (bins/cli/src/wallet.rs). If a \
         fresh wallet is no longer written at version 3, every assertion below about which \
         wallets are 'legacy' is reclassified."
    );
}

/// P1 fixture — a version-2 wallet that still holds its BLS key: the pre-INC-I-162 shape where
/// the file is the only copy of a registered producer identity. Built by relabelling a fresh
/// wallet, so no real key material is ever committed here.
fn legacy_wallet_with_bls(network: &str, path: &Path) {
    new_v3_wallet(network, path);
    let mut v = wallet_json(path);
    v["version"] = serde_json::json!(2);
    std::fs::write(path, serde_json::to_vec_pretty(&v).expect("reserialize")).expect("write");
    assert!(
        stored_bls_secret(path).is_some(),
        "setup: the P1 fixture must still hold a BLS key"
    );
}

/// P3 fixture — a version-2 wallet with the BLS fields stripped. Both are
/// `#[serde(default, skip_serializing_if = "Option::is_none")]`, so their absence is valid.
fn legacy_wallet_without_bls(network: &str, path: &Path) {
    new_v3_wallet(network, path);
    let mut v = wallet_json(path);
    let addr = v["addresses"][0]
        .as_object_mut()
        .expect("addresses[0] is not an object");
    addr.remove("bls_private_key");
    addr.remove("bls_public_key");
    v["version"] = serde_json::json!(2);
    std::fs::write(path, serde_json::to_vec_pretty(&v).expect("reserialize")).expect("write");
    assert!(
        stored_bls_secret(path).is_none(),
        "setup: the P3 fixture must have no BLS key"
    );
}

/// O3 + O4 + O5, asserted identically on every path so no partition can skip them.
fn assert_read_only_and_silent(out: &Output, path: &Path, before: &[u8], label: &str) {
    assert!(
        out.status.success(),
        "{label}/O3: `doli info` must exit 0 on every wallet version: {}",
        stderr_of(out)
    );
    assert_eq!(
        before,
        std::fs::read(path)
            .expect("cannot read wallet bytes")
            .as_slice(),
        "{label}/O4: `doli info` is a read-only report; it must not rewrite the wallet file."
    );
    if let Some(secret) = stored_bls_secret(path) {
        let seen = combined(out);
        assert!(
            !seen.contains(&secret),
            "{label}/O5: the BLS secret reached the terminal. A hint that talks about the key \
             must never print it — printed, it lands in shell scrollback."
        );
        assert!(
            !seen.contains(&secret.to_uppercase()),
            "{label}/O5: the BLS secret leaked in upper-case form."
        );
    }
}

/// P1a — mainnet: the hint renders with the real mainnet activation height.
#[test]
fn p1a_legacy_wallet_on_mainnet_recommends_migration_with_the_mainnet_height() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legacy.json");
    legacy_wallet_with_bls("mainnet", &path);
    let before = std::fs::read(&path).expect("cannot read wallet bytes");

    let out = doli("mainnet", &path, &["info"]);
    let seen = stdout_of(&out);
    assert_read_only_and_silent(&out, &path, &before, "P1a");

    assert!(
        seen.contains(HINT_MARKER),
        "P1a/O1: a version-2 wallet holding a random BLS key must be told the exposure can be \
         CLOSED (restore the phrase into a new file, then `doli producer rotate-bls`), not only \
         that the file needs a backup. Got:\n{seen}"
    );
    assert!(
        seen.contains("rotate-bls"),
        "P1a/O1: the hint must name the command that closes the gap. Got:\n{seen}"
    );
    assert!(
        seen.contains("restore"),
        "P1a/O1: the hint must name the step that produces the phrase-derived key. Got:\n{seen}"
    );
    assert!(
        seen.contains("457855"),
        "P1a/O2: the hint must quote the mainnet bls_key_rotation_activation_height (457_855, \
         crates/core/src/network_params/defaults.rs). Without it the operator cannot tell \
         whether the rotation is submittable today. Got:\n{seen}"
    );
    assert!(
        seen.contains(DOC_POINTER),
        "P1a/O6: the hint must point at {DOC_POINTER}; five lines are a verdict, not a \
         procedure. Got:\n{seen}"
    );
    assert!(
        !seen.contains(NOT_ACTIVATED),
        "P1a/O2: mainnet HAS a pinned activation height; the not-yet-activated prose is false \
         there. Got:\n{seen}"
    );
}

/// P1b — testnet: same branch, a different height. This is the partition a hardcoded
/// 457855 fails.
#[test]
fn p1b_legacy_wallet_on_testnet_quotes_the_testnet_height_not_the_mainnet_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legacy.json");
    legacy_wallet_with_bls("testnet", &path);
    let before = std::fs::read(&path).expect("cannot read wallet bytes");

    let out = doli("testnet", &path, &["info"]);
    let seen = stdout_of(&out);
    assert_read_only_and_silent(&out, &path, &before, "P1b");

    assert!(
        seen.contains(HINT_MARKER),
        "P1b/O1: the hint must render on testnet too. Got:\n{seen}"
    );
    assert!(
        seen.contains("176200"),
        "P1b/O2: on testnet the hint must quote 176_200. Got:\n{seen}"
    );
    assert!(
        !seen.contains("457855"),
        "P1b/O2: the mainnet height must not be printed to a testnet operator — the height has \
         to come from NetworkParams::defaults(network), not from a literal. Got:\n{seen}"
    );
    assert!(
        seen.contains(DOC_POINTER),
        "P1b/O6: the doc pointer must render on every network. Got:\n{seen}"
    );
}

/// P1c — devnet: the height is u64::MAX, so no number may render.
#[test]
fn p1c_legacy_wallet_on_devnet_says_not_activated_instead_of_printing_u64_max() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legacy.json");
    legacy_wallet_with_bls("devnet", &path);
    let before = std::fs::read(&path).expect("cannot read wallet bytes");

    let out = doli("devnet", &path, &["info"]);
    let seen = stdout_of(&out);
    assert_read_only_and_silent(&out, &path, &before, "P1c");

    assert!(
        seen.contains(HINT_MARKER),
        "P1c/O1: the hint must render on devnet too — the migration is still the right plan, \
         only the timing differs. Got:\n{seen}"
    );
    assert!(
        seen.contains(NOT_ACTIVATED),
        "P1c/O2: devnet pins bls_key_rotation_activation_height to u64::MAX. The hint must say \
         so in words. Got:\n{seen}"
    );
    assert!(
        !seen.contains("18446744073709551615"),
        "P1c/O2: u64::MAX must never be rendered as a decimal height — an operator cannot wait \
         for it and cannot compare it with getChainInfo. Got:\n{seen}"
    );
    assert!(
        !seen.contains("457855") && !seen.contains("176200"),
        "P1c/O2: another network's height must not leak into a devnet report. Got:\n{seen}"
    );
}

/// P2 — a version-3 wallet must NOT be told to migrate.
#[test]
fn p2_seed_derived_wallet_is_not_told_to_migrate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("v3.json");
    new_v3_wallet("mainnet", &path);
    let before = std::fs::read(&path).expect("cannot read wallet bytes");

    let out = doli("mainnet", &path, &["info"]);
    let seen = stdout_of(&out);
    assert_read_only_and_silent(&out, &path, &before, "P2");

    assert!(
        seen.contains("COMPLETE backup"),
        "P2/setup: a version-3 wallet must still get the complete-backup verdict. Got:\n{seen}"
    );
    assert!(
        !seen.contains(HINT_MARKER),
        "P2/O1: a version-3 wallet already derives its BLS key from the phrase. Recommending a \
         rotation there costs a transaction fee and a node restart for no change. Got:\n{seen}"
    );
    assert!(
        !seen.contains("rotate-bls") && !seen.contains("457855"),
        "P2/O1+O2: neither the command nor the activation height belongs in a version-3 \
         report. Got:\n{seen}"
    );
    assert!(
        !seen.contains(DOC_POINTER),
        "P2/O6: the recovery runbook is for wallets that have the problem. Got:\n{seen}"
    );
}

/// P3 — a legacy wallet with no BLS key at all: nothing on chain derives from this file, so
/// the same `primary_bls_public_key().is_some()` gate that hides the getProducer comparison
/// must hide the hint.
#[test]
fn p3_legacy_wallet_without_a_bls_key_gets_no_rotation_hint() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("nobls.json");
    legacy_wallet_without_bls("mainnet", &path);
    let before = std::fs::read(&path).expect("cannot read wallet bytes");

    let out = doli("mainnet", &path, &["info"]);
    let seen = stdout_of(&out);
    assert_read_only_and_silent(&out, &path, &before, "P3");

    assert!(
        seen.contains("BLS Key:    none"),
        "P3/setup: the fixture must report no BLS key. Got:\n{seen}"
    );
    assert!(
        !seen.contains(HINT_MARKER) && !seen.contains("rotate-bls"),
        "P3/O1: there is no key to rotate. `doli producer rotate-bls` would fail at the wallet \
         read, so recommending it here sends the operator down a dead end. Got:\n{seen}"
    );
    assert!(
        !seen.contains("457855"),
        "P3/O2: no height without a hint to carry it. Got:\n{seen}"
    );
}
