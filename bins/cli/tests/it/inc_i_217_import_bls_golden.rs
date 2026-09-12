// OUTPUT CONTRACT: `doli import-bls <SECRET_HEX> [--force] [--rpc <URL>] [--address <ADDR>]`
//   INC-I-217 / REQ-ROT-014 (Should). New flat clap variant in bins/cli/src/commands.rs,
//   dispatched from bins/cli/src/main.rs into bins/cli/src/cmd_wallet_bls.rs, which calls a new
//   `Wallet::import_bls_key(secret_hex, force)` and then `Wallet::save()`.
//   `doli-cli` is a bin-only crate for wallet purposes (lib.rs exports only cmd_release_verify /
//   cmd_service_helper_units / producer_ledger / upgrade_systemd_plan), so every output below is
//   observed across the process boundary through the real binary (`env!("CARGO_BIN_EXE_doli")`),
//   exactly as in tests/inc_i_167_wallet_overwrite_guard.rs.
//
//   O1: wallet file BLS material   — addresses[0].bls_private_key / .bls_public_key read back
//                                    from the JSON on disk. INSTALLED(new) / PRESERVED(old) /
//                                    ABSENT. The only output that distinguishes "imported" from
//                                    "refused" from "corrupted".
//   O2: process exit status        — success / failure.
//   O3: whole-file byte identity   — the raw bytes of wallet.json before vs after.
//                                    UNCHANGED / CHANGED. This is how "verify BEFORE write" is
//                                    made observable from outside the process: a rejected secret
//                                    that left the file UNCHANGED proves the sign->verify round
//                                    trip and the scalar check ran ahead of the write.
//   O4: stdout guidance            — derived 48-byte PUBLIC key hex; the getProducer/blsPubkey
//                                    self-comparison instruction when no --rpc was given.
//   O5: secret leakage             — the 32-byte secret hex in stdout+stderr combined. MUST BE
//                                    ABSENT on every path. Binding constraint from
//                                    specs/bls-key-rotation-architecture-security.md line 78
//                                    (data classification: "import-bls never prints it").
//   O6: `doli info` Backup verdict — the claim cmd_wallet.rs:865 makes about the 24-word phrase,
//                                    gated on Wallet::bls_is_seed_derived() (wallet.rs:364,
//                                    computed as version >= 3). COMPLETE-BACKUP-CLAIMED /
//                                    FILE-IS-ONLY-COPY. A second production reader of the same
//                                    marker is the save() overwrite warning at wallet.rs:197.
//
// PATHS: the branches import-bls must distinguish, by (does the wallet already hold a BLS key?)
//        x (is --force present?) x (is the supplied secret a valid BLS scalar?):
//   P1: has-key + --force  + valid    -> MUST install. THE REAL OPERATOR PATH (see partitions).
//   P2: has-key + no-force + valid    -> MUST refuse; existing key survives intact.
//   P3: no-key  + no-force + valid    -> MUST install; no consent needed, nothing is at risk.
//   P4: any     + --force  + INVALID  -> MUST refuse; wallet byte-identical.
//   P5: CLI surface (`--help`)        -> the three declared flags must exist, or the promised
//                                        on-chain comparison and address selection are
//                                        unreachable no matter how the body behaves.
//
// INPUT PARTITIONS.
//   The has-key term is not a free choice: Wallet::new() (wallet.rs:91-99) has derived the BLS
//   key from the BIP-39 seed since INC-I-162, and stamps version 3. EVERY wallet the CLI can
//   create therefore already holds a BLS key, so P1 — not P3 — is the path the live lost-key
//   producers will actually take. P3 is reachable only from a legacy v1/v2 wallet whose bls_*
//   fields are absent (both are `#[serde(default, skip_serializing_if)]`), which is precisely
//   the INC-I-162 population this command exists for; the fixture below builds that shape by
//   stripping the fields from a freshly created wallet.
//   For P1/P2/P3 one partition each suffices: the branch predicate reads only has_bls_key(), the
//   flag, and scalar validity, never the key's provenance. The secret chosen for P1/P2 is
//   FOREIGN to the wallet's seed — the operator's situation, and the only partition under which
//   O6 can be wrong.
//   P4 is enumerated over the six malformed shapes that reach a real operator's paste buffer:
//   non-hex, odd-length, 31 bytes, 33 bytes, a 48-byte PUBLIC key supplied by mistake, and
//   all-zero (rejected as a scalar by BlsSecretKey::from_bytes, crates/crypto/src/bls.rs:294).
//   Every P4 case carries --force, so the refusal it measures is the validation branch and not
//   the overwrite guard already covered by P2.
//
// UNTESTED PATHS, declared rather than mocked.
//   (a) `--rpc <URL>` live on-chain comparison against getProducer -> blsPubkey. Not reachable
//       without a running node; no mock is invented for it. Only its existence as a flag is
//       pinned (P5), plus the offline fallback text that must appear when it is omitted (O4).
//   (b) `--address <ADDR>` selecting a non-primary address. Pinned at the surface only (P5):
//       has_bls_key() and primary_bls_public_key() (wallet.rs:477,485) both read addresses[0],
//       so there is no observable multi-address BLS behaviour in the current wallet to assert.
//
// MATRIX: 6 outputs x 5 paths. Cells marked n/a are not observable on that path and are
//   recorded as such rather than asserted vacuously.
//
//   path | O1 BLS material | O2 exit | O3 file bytes | O4 stdout        | O5 leak | O6 info
//   -----|-----------------|---------|---------------|------------------|---------|-----------------
//   P1   | INSTALLED new   | success | CHANGED       | pubkey + hint    | ABSENT  | FILE-IS-ONLY-COPY
//   P2   | PRESERVED old   | failure | UNCHANGED     | names --force    | ABSENT  | n/a (no write)
//   P3   | INSTALLED new   | success | CHANGED       | pubkey           | ABSENT  | n/a (already v2)
//   P4   | PRESERVED old   | failure | UNCHANGED     | n/a              | ABSENT  | n/a (no write)
//   P5   | n/a             | success | n/a           | 3 declared flags | n/a     | n/a
//
//! INC-I-217 M2 — `doli import-bls`, the purely client-side recovery path for a producer
//! operator who still holds the BLS secret registered on chain but whose wallet no longer
//! matches it. No consensus surface: nothing here reaches a block, a transaction or the
//! producer set.
//!
//! RED. On the current tree every test below fails at clap: `import-bls` is not a variant in
//! bins/cli/src/commands.rs, so the binary exits 2 with "unrecognized subcommand".
//!
//! The load-bearing assertion is `inc_i_217_info_must_not_claim_the_phrase_backs_up_an_imported
//! _bls_key`. Wallet::bls_is_seed_derived() is `version >= 3` and nothing else. Import a FOREIGN
//! secret into a version-3 wallet and that marker becomes a lie: the 24 words still restore the
//! address and the funds, but they no longer restore the producer key the chain knows. `doli
//! info` would go on printing "The phrase is a COMPLETE backup of this wallet." to an operator
//! whose producer identity now lives in exactly one file. That is the INC-I-162 / INC-I-167
//! data-loss class, recovery cost ~75% of the bond via exit + re-register.

use std::path::Path;
use std::process::{Command, Output};

use crypto::bls::BlsKeyPair;

/// Run the real `doli` binary against an explicit wallet path on a deterministic network.
fn doli(wallet: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_doli"));
    cmd.arg("--network")
        .arg("devnet")
        .arg("-w")
        .arg(wallet)
        .args(args);
    cmd.output().expect("failed to run doli binary")
}

/// Run the real `doli` binary with no wallet context — for surface checks.
fn doli_bare(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_doli"))
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

/// stdout and stderr together — the operator's terminal, and their shell history file.
fn combined(out: &Output) -> String {
    format!("{}{}", stdout_of(out), stderr_of(out))
}

fn wallet_json(path: &Path) -> serde_json::Value {
    let contents = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read wallet {}: {e}", path.display()));
    serde_json::from_str(&contents).expect("wallet file is not valid JSON")
}

/// O1 — the BLS secret recorded on disk. Read only to compare with a key this test generated
/// itself; never printed, never interpolated into a failure message.
fn stored_bls_secret(path: &Path) -> Option<String> {
    wallet_json(path)["addresses"][0]["bls_private_key"]
        .as_str()
        .map(str::to_string)
}

/// O1 — the BLS public key recorded on disk.
fn stored_bls_public(path: &Path) -> Option<String> {
    wallet_json(path)["addresses"][0]["bls_public_key"]
        .as_str()
        .map(str::to_string)
}

fn wallet_version(path: &Path) -> u64 {
    wallet_json(path)["version"]
        .as_u64()
        .expect("wallet version missing")
}

/// O3 — whole-file byte identity.
fn wallet_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("cannot read wallet bytes")
}

/// A BLS keypair foreign to any wallet: the operator's old, on-chain-registered key.
/// Generated in-test; no real operator key is ever hardcoded here.
fn foreign_keypair() -> (String, String) {
    let kp = BlsKeyPair::generate();
    (kp.secret_key().to_hex(), kp.public_key().to_hex())
}

/// A version-3 wallet as `doli new` writes it: BLS key present and seed-derived.
fn new_v3_wallet(path: &Path) {
    let out = doli(path, &["new"]);
    assert!(
        out.status.success(),
        "setup: `doli new` failed: {}",
        stderr_of(&out)
    );
    assert_eq!(
        wallet_version(path),
        3,
        "setup: REQ-ROT-014 / INC-I-217 — WALLET_VERSION_SEED_DERIVED_BLS must stay 3 \
         (wallet.rs:27); a fresh wallet must still be written at version 3. Bumping it would \
         silently reclassify every existing wallet's backup story."
    );
    assert!(
        stored_bls_secret(path).is_some(),
        "setup: a version-3 wallet must already hold a seed-derived BLS key (wallet.rs:91-99)"
    );
}

/// A legacy wallet with NO BLS key at all — the INC-I-162 shape this command exists to serve.
/// Built by stripping the bls_* fields from a fresh wallet; both are
/// `#[serde(default, skip_serializing_if = "Option::is_none")]`, so their absence is valid.
fn legacy_wallet_without_bls(path: &Path) {
    let out = doli(path, &["new"]);
    assert!(
        out.status.success(),
        "setup: `doli new` failed: {}",
        stderr_of(&out)
    );
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
        "setup: the legacy fixture must have no BLS key"
    );
}

/// O5 — the binding security constraint, applied uniformly on every path.
fn assert_no_secret_leak(out: &Output, secret_hex: &str, label: &str) {
    let seen = combined(out);
    assert!(
        !seen.contains(secret_hex),
        "{label}/O5: the 32-byte BLS secret appeared in the command output. \
         specs/bls-key-rotation-architecture-security.md classifies it as secret and requires \
         that import-bls never prints it — printed, it lands in shell scrollback and in any log \
         the operator pastes into a support channel."
    );
    assert!(
        !seen.contains(&secret_hex.to_uppercase()),
        "{label}/O5: the BLS secret leaked in upper-case form."
    );
}

// REQ-ROT-014 — Decision: whether an operator can complete the only available recovery (install
// the key the chain already knows) without simultaneously publishing that key to their shell
// history, and whether the key actually reached the file rather than a log line.
#[test]
fn inc_i_217_import_bls_force_installs_the_key_and_never_prints_the_secret() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let wallet = tmp.path().join("producer.json");
    new_v3_wallet(&wallet);

    let (secret_hex, public_hex) = foreign_keypair();
    let before_secret = stored_bls_secret(&wallet).expect("v3 wallet has a BLS key");
    assert_ne!(
        before_secret, secret_hex,
        "setup: the imported secret must be foreign to the wallet's seed"
    );

    // ---- P1: has-key + --force + valid ----
    let out = doli(&wallet, &["import-bls", &secret_hex, "--force"]);
    let stdout = stdout_of(&out);
    let stderr = stderr_of(&out);

    // O2 x P1
    assert!(
        out.status.success(),
        "P1/O2: `doli import-bls <secret> --force` must succeed. stderr: {stderr}"
    );
    // O1 x P1 — the key must actually be installed, both halves.
    assert_eq!(
        stored_bls_secret(&wallet).as_deref(),
        Some(secret_hex.as_str()),
        "P1/O1: the supplied secret must be written to addresses[0].bls_private_key"
    );
    assert_eq!(
        stored_bls_public(&wallet).as_deref(),
        Some(public_hex.as_str()),
        "P1/O1: addresses[0].bls_public_key must be the key DERIVED from the supplied secret, \
         not the old public key and not a caller-supplied value"
    );
    // O4 x P1 — the derived 48-byte public key is what the operator compares with the chain.
    assert_eq!(
        public_hex.len(),
        96,
        "sanity: a BLS public key is 48 bytes / 96 hex chars"
    );
    assert!(
        stdout.contains(&public_hex),
        "P1/O4: the derived BLS public key must be printed so the operator can check it against \
         the chain. stdout: {stdout}"
    );
    // O4 x P1 — with no --rpc, the command must hand the comparison back to the operator.
    assert!(
        stdout.contains("getProducer"),
        "P1/O4: without --rpc the command must name getProducer as the comparison to run. \
         stdout: {stdout}"
    );
    assert!(
        stdout.contains("blsPubkey"),
        "P1/O4: without --rpc the command must name the blsPubkey field to compare against. \
         stdout: {stdout}"
    );
    // O5 x P1
    assert_no_secret_leak(&out, &secret_hex, "P1");

    // Round-trip through save() + load(): the change must survive re-reading the file.
    let info = doli(&wallet, &["info"]);
    assert!(
        info.status.success(),
        "P1: `doli info` after import failed: {}",
        stderr_of(&info)
    );
    assert!(
        stdout_of(&info).contains(&public_hex),
        "P1/O1: the imported key must survive the save()/load() round-trip and show up in \
         `doli info`. stdout: {}",
        stdout_of(&info)
    );
    assert_no_secret_leak(&info, &secret_hex, "P1-info");
}

// REQ-ROT-014 — Decision: whether one mistyped or misremembered secret can destroy a working
// producer's only BLS key. Wallet::add_bls_key() already refuses to clobber (wallet.rs:493); an
// import that silently replaced would be strictly more dangerous than the command it sits beside,
// and every CLI-created wallet is in this branch.
#[test]
fn inc_i_217_import_bls_refuses_without_force_and_leaves_the_key_intact() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let wallet = tmp.path().join("producer.json");
    new_v3_wallet(&wallet);

    let (secret_hex, public_hex) = foreign_keypair();
    let before_bytes = wallet_bytes(&wallet);
    let before_secret = stored_bls_secret(&wallet).expect("v3 wallet has a BLS key");
    let before_public = stored_bls_public(&wallet).expect("v3 wallet has a BLS key");

    // ---- P2: has-key + no-force + valid ----
    let out = doli(&wallet, &["import-bls", &secret_hex]);
    let stderr = stderr_of(&out);

    // O2 x P2
    assert!(
        !out.status.success(),
        "P2/O2: importing over an existing BLS key without --force must exit non-zero. \
         output: {}",
        combined(&out)
    );
    // O1 x P2 — the existing key survives, both halves.
    assert_eq!(
        stored_bls_secret(&wallet).as_deref(),
        Some(before_secret.as_str()),
        "P2/O1: the refusal must leave the existing BLS secret in place"
    );
    assert_eq!(
        stored_bls_public(&wallet).as_deref(),
        Some(before_public.as_str()),
        "P2/O1: the refusal must leave the existing BLS public key in place"
    );
    assert_ne!(
        stored_bls_public(&wallet).as_deref(),
        Some(public_hex.as_str()),
        "P2/O1: the refused key must not have been installed"
    );
    // O3 x P2 — nothing was written at all.
    assert_eq!(
        wallet_bytes(&wallet),
        before_bytes,
        "P2/O3: a refused import must leave wallet.json byte-identical"
    );
    // O4 x P2 — the refusal must name its own escape hatch, as INC-I-167 established.
    assert!(
        stderr.contains("--force"),
        "P2/O4: the refusal must name the --force remediation, or the operator has no documented \
         way forward. stderr: {stderr}"
    );
    // O5 x P2
    assert_no_secret_leak(&out, &secret_hex, "P2");
}

// REQ-ROT-014 — Decision: whether the INC-I-162 population this command was written for —
// legacy wallets whose BLS key is absent — can use it at all, or whether the consent guard
// demands --force from an operator with nothing to lose.
#[test]
fn inc_i_217_import_bls_into_a_wallet_without_a_bls_key_needs_no_force() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let wallet = tmp.path().join("legacy.json");
    legacy_wallet_without_bls(&wallet);

    let (secret_hex, public_hex) = foreign_keypair();

    // ---- P3: no-key + no-force + valid ----
    let out = doli(&wallet, &["import-bls", &secret_hex]);
    let stdout = stdout_of(&out);

    // O2 x P3
    assert!(
        out.status.success(),
        "P3/O2: importing into a wallet with no BLS key must succeed without --force. \
         output: {}",
        combined(&out)
    );
    // O1 x P3
    assert_eq!(
        stored_bls_secret(&wallet).as_deref(),
        Some(secret_hex.as_str()),
        "P3/O1: the secret must be installed"
    );
    assert_eq!(
        stored_bls_public(&wallet).as_deref(),
        Some(public_hex.as_str()),
        "P3/O1: the derived public key must be installed"
    );
    // O4 x P3
    assert!(
        stdout.contains(&public_hex),
        "P3/O4: the derived public key must be shown. stdout: {stdout}"
    );
    // O5 x P3
    assert_no_secret_leak(&out, &secret_hex, "P3");
}

// REQ-ROT-014 — Decision: whether a bad paste (truncated key, the PUBLIC key by mistake, a stray
// character turned into non-hex) corrupts wallet.json into a state where the producer can neither
// attest nor recover. Every case carries --force, so what is measured is the validate-before-write
// ordering, not the consent guard.
#[test]
fn inc_i_217_import_bls_rejects_malformed_secrets_and_leaves_the_wallet_byte_identical() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let wallet = tmp.path().join("producer.json");
    new_v3_wallet(&wallet);

    let before_bytes = wallet_bytes(&wallet);
    let before_secret = stored_bls_secret(&wallet).expect("v3 wallet has a BLS key");
    let (_, public_hex) = foreign_keypair();

    let cases: [(&str, String); 6] = [
        ("non-hex", "z".repeat(64)),
        ("odd-length", "ab".repeat(31) + "c"),
        ("31 bytes / too short", "ab".repeat(31)),
        ("33 bytes / too long", "ab".repeat(33)),
        ("48-byte PUBLIC key pasted by mistake", public_hex.clone()),
        ("all-zero / invalid BLS scalar", "00".repeat(32)),
    ];

    for (label, payload) in &cases {
        // ---- P4: --force + INVALID ----
        let out = doli(&wallet, &["import-bls", payload, "--force"]);

        // O2 x P4 — the rejection must come from validating the secret, not from the command
        // not existing. Without this the whole case passes vacuously on a tree where
        // import-bls has never been implemented.
        assert!(
            !combined(&out).contains("unrecognized subcommand"),
            "P4/O2 [{label}]: `doli import-bls` must exist for this rejection to mean anything. \
             output: {}",
            combined(&out)
        );
        assert!(
            !out.status.success(),
            "P4/O2 [{label}]: a malformed BLS secret must exit non-zero. output: {}",
            combined(&out)
        );
        // O3 x P4 — this is the observable form of "verify BEFORE write".
        assert_eq!(
            wallet_bytes(&wallet),
            before_bytes,
            "P4/O3 [{label}]: a rejected secret must leave wallet.json byte-identical. \
             Validation must run before any write, not after a partial one."
        );
        // O1 x P4
        assert_eq!(
            stored_bls_secret(&wallet).as_deref(),
            Some(before_secret.as_str()),
            "P4/O1 [{label}]: the existing BLS key must be untouched"
        );
        // O5 x P4 — an error path must not echo the wallet's own secret back.
        assert_no_secret_leak(&out, &before_secret, "P4");
    }
}

// REQ-ROT-014 — Decision: whether an operator who has just imported a foreign BLS key is told
// their 24 words back it up. Wallet::bls_is_seed_derived() is `version >= 3` and nothing more
// (wallet.rs:364); after this import that claim is false, and the operator acts on it by not
// backing up the file. This is the highest-value assertion in M2 — the INC-I-162 / INC-I-167
// data-loss class, ~75% of the bond to recover from.
#[test]
fn inc_i_217_info_must_not_claim_the_phrase_backs_up_an_imported_bls_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let wallet = tmp.path().join("producer.json");
    new_v3_wallet(&wallet);

    // Pin the pre-state, so the post-import assertion cannot pass vacuously: a version-3 wallet
    // DOES make the complete-backup claim today, and must keep making it.
    let before = doli(&wallet, &["info"]);
    assert!(before.status.success(), "setup: `doli info` failed");
    assert!(
        stdout_of(&before).contains("COMPLETE backup"),
        "setup/O6: a seed-derived version-3 wallet must still claim the phrase is a complete \
         backup — that claim is TRUE before the import and must not be removed wholesale. \
         stdout: {}",
        stdout_of(&before)
    );

    let (secret_hex, public_hex) = foreign_keypair();
    let out = doli(&wallet, &["import-bls", &secret_hex, "--force"]);
    assert!(
        out.status.success(),
        "import must succeed for this test to mean anything. output: {}",
        combined(&out)
    );

    let after = doli(&wallet, &["info"]);
    let stdout = stdout_of(&after);
    assert!(after.status.success(), "`doli info` after import failed");

    // O6 — the marker must stop lying.
    assert!(
        !stdout.contains("COMPLETE backup"),
        "O6: after importing a FOREIGN BLS secret, `doli info` still claims \"The phrase is a \
         COMPLETE backup of this wallet.\" It is not. The 24 words restore the address and the \
         funds; they re-derive the ORIGINAL seed-derived BLS key, not the imported one the chain \
         knows. An operator who believes this will not back up wallet.json, and losing it costs \
         the producer identity permanently (INC-I-162). stdout: {stdout}"
    );
    // O6 — and must say the true thing in its place.
    assert!(
        stdout.contains("only copy"),
        "O6: after an import, `doli info` must tell the operator that this FILE is the only copy \
         of the BLS key and must be backed up itself. stdout: {stdout}"
    );
    // O1 — the same run must still report the imported key, so O6 is about this key.
    assert!(
        stdout.contains(&public_hex),
        "O6/O1: `doli info` must report the imported BLS public key. stdout: {stdout}"
    );
    // O5
    assert_no_secret_leak(&after, &secret_hex, "O6-info");
}

// REQ-ROT-014 — Decision: whether the on-chain re-comparison and the address selection the
// requirement promises are reachable at all. A body that implements them behind a flag clap does
// not accept is indistinguishable, from the operator's side, from not having shipped.
#[test]
fn inc_i_217_import_bls_surface_exposes_force_rpc_and_address() {
    // ---- P5: CLI surface ----
    let out = doli_bare(&["import-bls", "--help"]);
    let help = combined(&out);

    assert!(
        !help.contains("unrecognized subcommand"),
        "P5: `doli import-bls` must exist as a top-level command. The CLI has no \
         `doli wallet ...` namespace — add-bls, import, export, info, new and restore are all \
         flat clap variants — so the spec string `doli wallet import-bls` is drift. output: {help}"
    );
    assert!(
        out.status.success(),
        "P5/O2: `doli import-bls --help` must exit 0. output: {help}"
    );
    for flag in ["--force", "--rpc", "--address"] {
        assert!(
            help.contains(flag),
            "P5/O4: `{flag}` must be part of the declared surface of import-bls. output: {help}"
        );
    }
}
