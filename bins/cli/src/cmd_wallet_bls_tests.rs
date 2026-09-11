// OUTPUT CONTRACT: fn Wallet::import_bls_key(&mut self, secret_hex: &str, force: bool)
//                     -> Result<String>
//   INC-I-217 / REQ-ROT-014 (Should). The unit under the `doli import-bls` command. Sibling of
//   Wallet::add_bls_key() (bins/cli/src/wallet.rs:493), which GENERATES a random key and cannot
//   import — the fact refuted in specs/bls-key-rotation-requirements-security.md C7.
//
//   INERT FILE. There is deliberately no `mod` declaration pointing here and no
//   bins/cli/src/cmd_wallet_bls.rs. The developer creates that impl file and wires
//       #[cfg(test)]
//       #[path = "cmd_wallet_bls_tests.rs"]
//       mod tests;
//   at its bottom, matching cmd_wallet.rs:1086 and parsers.rs:442. Until then this file is not
//   part of the crate and `cargo build -p doli-cli` does not see it.
//
//   These tests reach only the crate's PUBLIC wallet API — `Wallet::new`, `addresses()`,
//   `WalletAddress.bls_private_key` / `.bls_public_key` (both pub fields), `has_bls_key()`,
//   `primary_bls_public_key()`, `bls_is_seed_derived()`, `version()`, `save()`, `load()` — so
//   they compile from a `tests` module hosted in ANY module of the crate, not only inside
//   `wallet`. Nothing here depends on where the developer puts the impl.
//
//   O1: return value             — Ok(derived 48-byte public key hex) / Err.
//   O2: self.addresses[0].bls_private_key — INSTALLED(new) / PRESERVED(old) / ABSENT.
//   O3: self.addresses[0].bls_public_key  — must be DERIVED from the secret, never copied from
//                                           the old entry and never caller-supplied.
//   O4: seed-derived marker      — bls_is_seed_derived() (wallet.rs:364). Read in production by
//                                  cmd_wallet.rs:865 ("The phrase is a COMPLETE backup of this
//                                  wallet.") and by the save() overwrite warning at
//                                  wallet.rs:197. After a FOREIGN import it must report false.
//   O5: persistence              — the mutation surviving save() + load().
//
// PATHS, by (does the wallet already hold a BLS key?) x (force) x (is the secret a valid scalar?):
//   U1: has-key + force=true  + valid    -> Ok; key replaced. The real operator path: since
//                                           INC-I-162, Wallet::new() always populates a
//                                           seed-derived BLS key, so has_bls_key() is true for
//                                           every wallet the CLI can create.
//   U2: has-key + force=false + valid    -> Err; the receiver must be left untouched.
//   U3: no-key  + force=false + valid    -> Ok; the legacy INC-I-162 wallet shape.
//   U4: any     + force=true  + INVALID  -> Err; the receiver must be left untouched, proving
//                                           validation runs before mutation.
//
// INPUT PARTITIONS. One per path for U1-U3: the branch predicate reads only has_bls_key(), the
//   flag and scalar validity, never key provenance. The secret used for U1/U2 is FOREIGN to the
//   wallet's seed — the operator's situation, and the only partition under which O4 can be wrong.
//   U4 enumerates the six malformed shapes that reach an operator's paste buffer: non-hex,
//   odd-length, 31 bytes, 33 bytes, a 48-byte PUBLIC key supplied by mistake, and all-zero
//   (rejected as a scalar by BlsSecretKey::from_bytes, crates/crypto/src/bls.rs:294).
//
// NOT ASSERTED, declared rather than guessed. Importing the wallet's OWN seed-derived key is a
//   real input but is deliberately left unpinned: the conservative implementation — mark
//   not-seed-derived unconditionally on any import — is never harmful (it only tells the operator
//   to back up the file), so requiring O4 to stay true in that case would force the impl to
//   re-derive from a seed it does not hold. Only the foreign-key direction of O4 is pinned.
//
// MATRIX: 5 outputs x 4 paths; O5 is observable only where a write happened.
//
//   path | O1 return       | O2 secret     | O3 public     | O4 marker | O5 persistence
//   -----|-----------------|---------------|---------------|-----------|----------------
//   U1   | Ok(new pubkey)  | INSTALLED new | DERIVED new   | false     | survives round-trip
//   U2   | Err             | PRESERVED old | PRESERVED old | unchanged | n/a (no write)
//   U3   | Ok(new pubkey)  | INSTALLED new | DERIVED new   | false     | survives round-trip
//   U4   | Err             | PRESERVED old | PRESERVED old | unchanged | n/a (no write)

use crypto::bls::BlsKeyPair;

use crate::wallet::{Wallet, WALLET_VERSION_SEED_DERIVED_BLS};

/// A BLS keypair foreign to any wallet: the operator's old, on-chain-registered key.
fn foreign_keypair() -> (String, String) {
    let kp = BlsKeyPair::generate();
    (kp.secret_key().to_hex(), kp.public_key().to_hex())
}

fn stored_secret(w: &Wallet) -> Option<String> {
    w.addresses()[0].bls_private_key.clone()
}

fn stored_public(w: &Wallet) -> Option<String> {
    w.addresses()[0].bls_public_key.clone()
}

/// The legacy INC-I-162 shape: a wallet whose BLS key is absent entirely.
fn strip_bls(w: &mut Wallet) {
    let json = serde_json::to_string(&*w).expect("serialize");
    let mut v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    let addr = v["addresses"][0].as_object_mut().expect("addresses[0]");
    addr.remove("bls_private_key");
    addr.remove("bls_public_key");
    v["version"] = serde_json::json!(2);
    *w = serde_json::from_value(v).expect("reparse");
    assert!(!w.has_bls_key(), "fixture: BLS key must be absent");
}

// REQ-ROT-014 — Decision: whether the constant that decides, for every wallet on every producer
// host, whether the 24-word phrase is a complete backup has silently moved. Nothing may bump it
// as a side effect of adding an import path.
#[test]
fn inc_i_217_wallet_version_seed_derived_bls_stays_three() {
    assert_eq!(
        WALLET_VERSION_SEED_DERIVED_BLS, 3,
        "REQ-ROT-014 / INC-I-217: WALLET_VERSION_SEED_DERIVED_BLS must stay 3. It is the \
         threshold bls_is_seed_derived() compares against (wallet.rs:364); raising it would \
         reclassify every already-written version-3 wallet as not-seed-derived, and lowering it \
         would make version-2 wallets claim a backup they do not have."
    );
}

// REQ-ROT-014 — Decision: whether the recovery actually installs the key the chain knows, or
// installs a secret whose stored public half still points at the old key — which would leave the
// operator believing they recovered while the node keeps failing attestation.
#[test]
fn inc_i_217_import_bls_key_forced_installs_secret_and_derives_public() {
    let (mut w, _phrase) = Wallet::new("producer");
    let old_secret = stored_secret(&w).expect("v3 wallet has a BLS key");
    let old_public = stored_public(&w).expect("v3 wallet has a BLS key");
    let (secret_hex, public_hex) = foreign_keypair();
    assert_ne!(
        old_secret, secret_hex,
        "fixture: the secret must be foreign"
    );

    // ---- U1: has-key + force + valid ----
    let returned = w
        .import_bls_key(&secret_hex, true)
        .expect("U1/O1: a valid foreign secret with force=true must import");

    // O1
    assert_eq!(
        returned, public_hex,
        "U1/O1: the return value must be the public key DERIVED from the supplied secret"
    );
    // O2
    assert_eq!(
        stored_secret(&w).as_deref(),
        Some(secret_hex.as_str()),
        "U1/O2: the supplied secret must be stored"
    );
    // O3 — the half that silently breaks attestation if it is left stale.
    assert_eq!(
        stored_public(&w).as_deref(),
        Some(public_hex.as_str()),
        "U1/O3: the stored public key must be re-derived from the new secret"
    );
    assert_ne!(
        stored_public(&w).as_deref(),
        Some(old_public.as_str()),
        "U1/O3: the old public key must not survive alongside the new secret"
    );
    assert!(w.has_bls_key(), "U1/O2: has_bls_key() must report the key");
    assert_eq!(
        w.primary_bls_public_key(),
        Some(public_hex.as_str()),
        "U1/O3: primary_bls_public_key() must report the imported key"
    );
}

// REQ-ROT-014 — Decision: whether an operator is told the 24 words back up a key the 24 words
// cannot produce. bls_is_seed_derived() is `version >= 3` and nothing else; after a foreign
// import that answer is false, and two production call sites act on it.
#[test]
fn inc_i_217_import_bls_key_clears_the_seed_derived_marker() {
    let (mut w, _phrase) = Wallet::new("producer");
    assert!(
        w.bls_is_seed_derived(),
        "fixture: a fresh version-3 wallet IS seed-derived — the assertion below is not vacuous"
    );
    assert_eq!(w.version(), 3, "fixture: fresh wallets are written at v3");

    let (secret_hex, _public_hex) = foreign_keypair();
    w.import_bls_key(&secret_hex, true).expect("import");

    // O4
    assert!(
        !w.bls_is_seed_derived(),
        "U1/O4: after importing a FOREIGN BLS secret, bls_is_seed_derived() still reports true. \
         Its two production readers then lie: cmd_wallet.rs:865 prints \"The phrase is a \
         COMPLETE backup of this wallet.\" and the save() warning at wallet.rs:197 tells the \
         operator a version-3 wallet can be restored from its phrase. Neither is true of an \
         imported key — wallet.json is now the only copy, and losing it costs the producer \
         identity permanently (INC-I-162)."
    );
}

// REQ-ROT-014 — Decision: whether the recovery survives the process exiting. An in-memory-only
// mutation would look correct in every other assertion here and still leave the node loading the
// wrong key on restart.
#[test]
fn inc_i_217_import_bls_key_survives_save_and_load() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("producer.json");

    let (mut w, _phrase) = Wallet::new("producer");
    let (secret_hex, public_hex) = foreign_keypair();
    w.import_bls_key(&secret_hex, true).expect("import");
    w.save(&path)
        .expect("U1/O5: save to a fresh path must succeed");

    let reloaded = Wallet::load(&path).expect("U1/O5: reload");
    // O5 x O2/O3
    assert_eq!(
        stored_secret(&reloaded).as_deref(),
        Some(secret_hex.as_str()),
        "U1/O5: the imported secret must survive save() + load()"
    );
    assert_eq!(
        reloaded.primary_bls_public_key(),
        Some(public_hex.as_str()),
        "U1/O5: the derived public key must survive save() + load()"
    );
    // O5 x O4 — the marker is what the operator reads AFTER a restart, so it must persist too.
    assert!(
        !reloaded.bls_is_seed_derived(),
        "U1/O5/O4: the not-seed-derived verdict must be recorded in the FILE, not only in \
         memory — `doli info` reads a freshly loaded wallet"
    );
}

// REQ-ROT-014 — Decision: whether a single mistyped secret can overwrite the only copy of a
// working producer key. add_bls_key() already refuses (wallet.rs:493-495); an import that
// replaced silently would be strictly more dangerous than the command beside it.
#[test]
fn inc_i_217_import_bls_key_refuses_without_force_and_leaves_the_receiver_untouched() {
    let (mut w, _phrase) = Wallet::new("producer");
    let old_secret = stored_secret(&w).expect("v3 wallet has a BLS key");
    let old_public = stored_public(&w).expect("v3 wallet has a BLS key");
    let old_marker = w.bls_is_seed_derived();
    let (secret_hex, public_hex) = foreign_keypair();

    // ---- U2: has-key + no force + valid ----
    let err = w
        .import_bls_key(&secret_hex, false)
        .expect_err("U2/O1: importing over an existing BLS key without force must fail");
    let msg = err.to_string();

    // O1 — the error must name the escape hatch, as INC-I-167 established for save().
    assert!(
        msg.contains("--force") || msg.contains("force"),
        "U2/O1: the refusal must name the force remediation. error: {msg}"
    );
    // O1/O5 — the error must not carry secret material into a log line.
    assert!(
        !msg.contains(&secret_hex) && !msg.contains(&old_secret),
        "U2/O1: the error message must not contain any BLS secret"
    );
    // O2 / O3 — the receiver is untouched.
    assert_eq!(
        stored_secret(&w).as_deref(),
        Some(old_secret.as_str()),
        "U2/O2: the existing secret must survive the refusal"
    );
    assert_eq!(
        stored_public(&w).as_deref(),
        Some(old_public.as_str()),
        "U2/O3: the existing public key must survive the refusal"
    );
    assert_ne!(
        stored_public(&w).as_deref(),
        Some(public_hex.as_str()),
        "U2/O3: the refused key must not have been installed"
    );
    // O4
    assert_eq!(
        w.bls_is_seed_derived(),
        old_marker,
        "U2/O4: a refused import must not move the seed-derived marker"
    );
}

// REQ-ROT-014 — Decision: whether the INC-I-162 population this command exists for — legacy
// wallets with no BLS key — is forced to pass a destructive-consent flag to recover, when there
// is nothing to destroy.
#[test]
fn inc_i_217_import_bls_key_into_a_wallet_without_a_key_needs_no_force() {
    let (mut w, _phrase) = Wallet::new("legacy");
    strip_bls(&mut w);
    let (secret_hex, public_hex) = foreign_keypair();

    // ---- U3: no-key + no force + valid ----
    let returned = w
        .import_bls_key(&secret_hex, false)
        .expect("U3/O1: a wallet with no BLS key must import without force");

    assert_eq!(
        returned, public_hex,
        "U3/O1: returns the derived public key"
    );
    assert_eq!(
        stored_secret(&w).as_deref(),
        Some(secret_hex.as_str()),
        "U3/O2: the secret must be installed"
    );
    assert_eq!(
        stored_public(&w).as_deref(),
        Some(public_hex.as_str()),
        "U3/O3: the derived public key must be installed"
    );
    assert!(
        !w.bls_is_seed_derived(),
        "U3/O4: an imported key is not seed-derived"
    );
}

// REQ-ROT-014 — Decision: whether a bad paste leaves wallet.json half-written, with a secret and
// a public key that do not match each other — a state in which the producer can neither attest
// nor tell what went wrong.
#[test]
fn inc_i_217_import_bls_key_rejects_malformed_secrets_before_mutating() {
    let (_, foreign_public) = foreign_keypair();
    let cases: [(&str, String); 6] = [
        ("non-hex", "z".repeat(64)),
        ("odd-length", "ab".repeat(31) + "c"),
        ("31 bytes / too short", "ab".repeat(31)),
        ("33 bytes / too long", "ab".repeat(33)),
        ("48-byte PUBLIC key pasted by mistake", foreign_public),
        ("all-zero / invalid BLS scalar", "00".repeat(32)),
    ];

    for (label, payload) in &cases {
        let (mut w, _phrase) = Wallet::new("producer");
        let old_secret = stored_secret(&w).expect("v3 wallet has a BLS key");
        let old_public = stored_public(&w).expect("v3 wallet has a BLS key");
        let old_marker = w.bls_is_seed_derived();

        // ---- U4: force + INVALID ----
        // O1
        assert!(
            w.import_bls_key(payload, true).is_err(),
            "U4/O1 [{label}]: a malformed BLS secret must be rejected"
        );

        // O2 / O3 — validation ran before any mutation.
        assert_eq!(
            stored_secret(&w).as_deref(),
            Some(old_secret.as_str()),
            "U4/O2 [{label}]: the existing secret must be untouched"
        );
        assert_eq!(
            stored_public(&w).as_deref(),
            Some(old_public.as_str()),
            "U4/O3 [{label}]: the existing public key must be untouched"
        );
        // O4
        assert_eq!(
            w.bls_is_seed_derived(),
            old_marker,
            "U4/O4 [{label}]: a rejected import must not move the seed-derived marker"
        );
    }
}
