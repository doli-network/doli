//! INC-I-178 M2 — the frozen BLS attestation preimage (R1), the one dual-signing
//! constructor (D3) and the gossip wire format that both must not move.
//!
//! Every golden constant here was generated from THIS crate at HEAD 5d7f5a63 and
//! hard-coded, so the file fails if the preimage, the DST, the key derivation or
//! the bincode layout ever changes — which is the whole point of freezing them.
//!
//! OUTPUT CONTRACT
//!
//! F1: `attestation::bls_attest_msg(block_hash: &Hash) -> [u8; 32]`
//!   Observable outputs:
//!     O1 return — the 32 preimage bytes (the ONLY output; no params are &mut,
//!        no receiver, no store write)
//!   Paths: single straight-line path; there is no branch to cover.
//!   INPUT PARTITIONS: an arbitrary hash; the all-zero hash; the same hash under
//!     two different slots (the slot must make NO difference — that is R1).
//!
//! F2: `Attestation::new_with_bls(hash, slot, height, weight, sk, pk, bls)
//!        -> Result<Attestation, crypto::BlsError>`
//!   Observable outputs:
//!     O1 return `Ok(Attestation)` — 7 fields; `bls_signature` is the new one
//!     O2 return `Err(BlsError)` — the path that used to be a silent `Vec::new()`
//!     O3 mutable params — NONE (all inputs are by value or `&`)
//!     O4 persistent store writes — NONE
//!   Paths:
//!     P1 BLS signing succeeds -> O1 with 96 bytes over `bls_attest_msg`
//!     P2 BLS signing fails    -> O2 (unreachable from a valid `BlsKeyPair`;
//!        pinned structurally — the silent-empty fallback must be absent)
//!   INPUT PARTITIONS: a seed-derived key (deterministic, the golden vector);
//!     a randomly generated key (the production shape).
//!
//! F3: `Attestation::to_bytes` / `from_bytes` (bincode, positional)
//!   Observable outputs:
//!     O1 return — the serialized byte vector / the decoded struct
//!   Paths:
//!     P1 empty `bls_signature`   -> 180 bytes, the pre-M2 wire
//!     P2 96-byte `bls_signature` -> 276 bytes, the post-M2 wire
//!   INPUT PARTITIONS: a fixture captured BEFORE M2 (old peer -> new binary);
//!     a freshly built dual-signed attestation (new peer -> new binary).
//!
//! F4: the source trees (build inputs, not values) — `crypto::attestation_message`
//!   and the silent-empty fallback must be ABSENT. Rust cannot express the
//!   absence of a symbol in a type, so these are text tripwires.

use std::fs;
use std::path::{Path, PathBuf};

use crypto::{bls_sign, bls_verify, bls_verify_pop, BlsKeyPair, BlsSignature, KeyPair};
use doli_core::attestation::bls_attest_msg;
use doli_core::Attestation;

/// `crypto::hash::hash(b"INC-I-178 M2 golden block")` — pinned so a change to the
/// hash function is not silently absorbed by the vectors below.
const GOLDEN_BLOCK_HASH_HEX: &str =
    "7668f971063ae4c76e5805e410cff1be1bcc8ea6efb3730395936181c1e57e96";

/// Seed for `BlsKeyPair::from_seed` (>= `BLS_KEY_GEN_MIN_IKM`).
const GOLDEN_BLS_SEED: &[u8] = b"INC-I-178-M2-GOLDEN-BLS-SEED-0001";

const GOLDEN_BLS_PUBKEY_HEX: &str =
    "aeb21ea7a6999acee1cba5ef03ace0182cdc23b0455f4b5db06fa9c5b4f0353\
3acc8f4aaf223547beeaeeed53ee389c8";

/// `bls_sign(bls_attest_msg(&GOLDEN_BLOCK_HASH), golden_sk)` — BLS12-381 signing
/// is deterministic, so this is a fixed value, not a sample.
const GOLDEN_BLS_SIG_HEX: &str = "80e96c571b3787140994ba6a8345ffe1f94a821d61aaadec83a485789b8729e6\
0b412775ffd8a6c1e90f5d2d07c676e20d77e6d6aa5403c04d1da8a662c34906e92bfb52b0ef709b0c91ac0cfb7f832d0c9\
fcccf70ce860c1cab9d6d0e6988e3";

/// One `Attestation` serialized by the PRE-M2 binary: Ed25519 key seeded `[7; 32]`,
/// slot `0x01020304`, height `0x1122334455667788`, weight 42, EMPTY `bls_signature`.
const PRE_M2_WIRE_HEX: &str = "20000000000000007668f971063ae4c76e5805e410cff1be1bcc8ea6efb37303959\
36181c1e57e960403020188776655443322112000000000000000ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b\
92421eea691446d22c2a000000000000004000000000000000c174be45710d248cf892c8ff052be6e3691ef3bc14d74b723\
5d30deda964e181e351ffd46e8e275f3f12b09792f5a284f279eb97c9717696bd4980e1e6fc27030000000000000000";

const PRE_M2_WIRE_LEN: usize = 180;
const POST_M2_WIRE_LEN: usize = 276;
const GOLDEN_SLOT: u32 = 0x0102_0304;
const GOLDEN_HEIGHT: u64 = 0x1122_3344_5566_7788;
const GOLDEN_WEIGHT: u64 = 42;

fn golden_hash() -> crypto::Hash {
    crypto::hash::hash(b"INC-I-178 M2 golden block")
}

fn golden_bls() -> BlsKeyPair {
    BlsKeyPair::from_seed(GOLDEN_BLS_SEED).expect("golden BLS seed must derive a key")
}

fn golden_ed() -> KeyPair {
    KeyPair::from_seed([7u8; 32])
}

/// The PRE-R1 preimage that M2 deletes: `block_hash || slot` (big-endian).
fn legacy_preimage(block_hash: &crypto::Hash, slot: u32) -> Vec<u8> {
    let mut msg = Vec::with_capacity(36);
    msg.extend_from_slice(block_hash.as_bytes());
    msg.extend_from_slice(&slot.to_be_bytes());
    msg
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Source with every whole-line `//` comment removed, so a tombstone comment
/// naming a deleted symbol cannot fail a deletion tripwire.
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ============================================================
// R1 — the frozen preimage (REQ-BLS-001)
// ============================================================

// REQ-BLS-001 — Decision: if the preimage is anything but the 32 block-hash bytes,
// every aggregate M4 builds is verified against a message no verifier can
// reconstruct from the block alone, and the aggregate is unusable forever.
#[test]
fn m2_r1_the_frozen_preimage_is_the_block_hash_alone() {
    let hash = golden_hash();
    assert_eq!(
        hex::encode(hash.as_bytes()),
        GOLDEN_BLOCK_HASH_HEX,
        "the golden hash moved; the vectors below are no longer about this block"
    );

    let msg = bls_attest_msg(&hash);
    assert_eq!(msg.len(), 32, "the preimage must be exactly 32 bytes");
    assert_eq!(
        msg,
        *hash.as_bytes(),
        "the preimage must BE the block hash, with nothing appended"
    );
}

// REQ-BLS-001 — Decision: the slot dropping out of the preimage is the whole R1
// change; if any slot still reaches the message, two honest attesters on the same
// block sign different messages and the aggregate never verifies.
#[test]
fn m2_r1_the_preimage_ignores_the_slot_entirely() {
    let hash = golden_hash();
    let zero = crypto::Hash::ZERO;
    assert_eq!(bls_attest_msg(&hash), bls_attest_msg(&hash));
    assert_eq!(
        bls_attest_msg(&zero),
        [0u8; 32],
        "zero hash -> zero preimage"
    );
    assert_ne!(
        bls_attest_msg(&hash),
        bls_attest_msg(&zero),
        "different blocks must not collide onto one preimage"
    );
}

// REQ-BLS-001 — Decision: a drift in DST, key derivation or signing would still
// verify against itself; only a hard-coded vector catches a preimage that changed
// on both the signing and the verifying side at once.
#[test]
fn m2_r1_golden_signature_over_the_frozen_preimage_is_byte_stable() {
    let hash = golden_hash();
    let kp = golden_bls();
    assert_eq!(
        hex::encode(kp.public_key().as_bytes()),
        GOLDEN_BLS_PUBKEY_HEX,
        "BLS key derivation from the seed drifted"
    );

    let sig = bls_sign(&bls_attest_msg(&hash), kp.secret_key()).expect("golden signing must work");
    assert_eq!(
        hex::encode(sig.as_bytes()),
        GOLDEN_BLS_SIG_HEX,
        "the signature over the frozen preimage is no longer the golden vector"
    );
    assert!(
        bls_verify(&bls_attest_msg(&hash), &sig, kp.public_key()).is_ok(),
        "the golden signature must verify against the frozen preimage"
    );
}

// REQ-BLS-001 — Decision: this is the migration hazard. If a signature over the
// frozen preimage ALSO verified against `block_hash || slot`, the R1 change would
// be silently reversible and a stale signer would look valid.
#[test]
fn m2_r1_the_frozen_signature_is_rejected_under_the_pre_r1_preimage() {
    let hash = golden_hash();
    let kp = golden_bls();
    let frozen = BlsSignature::try_from_slice(&hex::decode(GOLDEN_BLS_SIG_HEX).unwrap())
        .expect("golden signature must be a valid G2 point");

    assert!(
        bls_verify(
            &legacy_preimage(&hash, GOLDEN_SLOT),
            &frozen,
            kp.public_key()
        )
        .is_err(),
        "a frozen-preimage signature must NOT verify as the old block_hash||slot form"
    );

    let legacy_sig = bls_sign(&legacy_preimage(&hash, GOLDEN_SLOT), kp.secret_key()).unwrap();
    assert!(
        bls_verify(&bls_attest_msg(&hash), &legacy_sig, kp.public_key()).is_err(),
        "an old block_hash||slot signature must NOT verify under the frozen preimage"
    );
}

// REQ-BLS-001 — Decision: a shared DST would let a registration proof-of-possession
// be replayed as an attestation for an attacker-chosen block (rogue-key reuse).
#[test]
fn m2_r1_attestation_and_proof_of_possession_domains_do_not_cross_verify() {
    let hash = golden_hash();
    let kp = golden_bls();
    let attest = bls_sign(&bls_attest_msg(&hash), kp.secret_key()).unwrap();
    let pop = kp
        .proof_of_possession()
        .expect("PoP generation must succeed");

    assert!(
        bls_verify_pop(kp.public_key(), &attest).is_err(),
        "an attestation signature must not pass as a proof-of-possession"
    );
    assert!(
        bls_verify(kp.public_key().as_bytes(), &pop, kp.public_key()).is_err(),
        "a proof-of-possession must not pass as an attestation over its own key bytes"
    );
    assert!(bls_verify_pop(kp.public_key(), &pop).is_ok());
}

// REQ-BLS-001 — Decision: leaving the old builder alive lets any future caller
// re-introduce the slot suffix and split the signer set without a compile error.
#[test]
fn m2_r1_the_pre_r1_message_builder_and_its_re_export_are_deleted() {
    let bls = code_only(&read("crates/crypto/src/bls.rs"));
    assert!(
        !bls.contains("pub fn attestation_message("),
        "R1: crypto::attestation_message must be deleted, not merely unused"
    );
    let lib = code_only(&read("crates/crypto/src/lib.rs"));
    assert!(
        !lib.contains("attestation_message,"),
        "R1: the crypto::lib re-export of attestation_message must be deleted"
    );
}

// ============================================================
// D3 — one dual-signing constructor (REQ-BLS-006)
// ============================================================

// REQ-BLS-006 — Decision: if the emitted signature is not over the frozen preimage,
// every honest peer scores the sender as invalid and the mesh degrades peer by peer.
#[test]
fn m2_d3_new_with_bls_signs_the_frozen_preimage_with_96_bytes() {
    let hash = golden_hash();
    let ed = golden_ed();
    let bls = golden_bls();

    let att = Attestation::new_with_bls(
        hash,
        GOLDEN_SLOT,
        GOLDEN_HEIGHT,
        GOLDEN_WEIGHT,
        ed.private_key(),
        *ed.public_key(),
        &bls,
    )
    .expect("dual signing must succeed for a valid BLS key pair");

    assert_eq!(att.bls_signature.len(), 96, "the wire carries a G2 point");
    assert_eq!(
        hex::encode(&att.bls_signature),
        GOLDEN_BLS_SIG_HEX,
        "the emitted BLS half must equal the frozen golden vector"
    );
    let sig = BlsSignature::try_from_slice(&att.bls_signature).expect("must be a G2 point");
    assert!(bls_verify(&bls_attest_msg(&att.block_hash), &sig, bls.public_key()).is_ok());
}

// REQ-BLS-006 — Decision: Release N is gossip-only with no activation height, so an
// Ed25519 half that differs by one byte from `Attestation::new` partitions the fleet
// on the FIRST block — every un-upgraded peer rejects every upgraded attestation.
#[test]
fn m2_d3_the_ed25519_half_is_byte_identical_to_the_single_signing_constructor() {
    let hash = golden_hash();
    let ed = golden_ed();
    let bls = golden_bls();

    let plain = Attestation::new(
        hash,
        GOLDEN_SLOT,
        GOLDEN_HEIGHT,
        GOLDEN_WEIGHT,
        ed.private_key(),
        *ed.public_key(),
    );
    let dual = Attestation::new_with_bls(
        hash,
        GOLDEN_SLOT,
        GOLDEN_HEIGHT,
        GOLDEN_WEIGHT,
        ed.private_key(),
        *ed.public_key(),
        &bls,
    )
    .unwrap();

    assert_eq!(
        dual.signature, plain.signature,
        "the Ed25519 signature must be untouched by dual signing"
    );
    assert_eq!(dual.block_hash, plain.block_hash);
    assert_eq!(dual.slot, plain.slot);
    assert_eq!(dual.height, plain.height);
    assert_eq!(dual.attester, plain.attester);
    assert_eq!(dual.attester_weight, plain.attester_weight);
    assert!(plain.bls_signature.is_empty());
    assert!(
        dual.verify().is_ok(),
        "the old verifier must still accept it"
    );
}

// REQ-BLS-006 — Decision: the Ed25519 preimage keeps the slot while the BLS one
// drops it; if the Ed25519 side ever loses the slot too, an attestation becomes
// replayable across slots for the same block hash.
#[test]
fn m2_d3_the_ed25519_preimage_still_binds_the_slot() {
    let hash = golden_hash();
    let ed = golden_ed();
    let bls = golden_bls();

    let a =
        Attestation::new_with_bls(hash, 7, 1, 1, ed.private_key(), *ed.public_key(), &bls).unwrap();
    let b =
        Attestation::new_with_bls(hash, 8, 1, 1, ed.private_key(), *ed.public_key(), &bls).unwrap();

    assert_ne!(
        a.signature, b.signature,
        "the Ed25519 signature must change with the slot"
    );
    assert_eq!(
        a.bls_signature, b.bls_signature,
        "R1: the BLS half must NOT change with the slot"
    );

    let mut replayed = a.clone();
    replayed.slot = 8;
    assert!(
        replayed.verify().is_err(),
        "moving an attestation to another slot must break Ed25519"
    );
}

// REQ-BLS-010 — Decision: `verify()` is the ONLY gate both ingresses run before
// admitting attendance, and it does not cover `bls_signature`. That is exactly why
// a relayed attestation can carry a mutated BLS blob with a valid Ed25519 half — the
// premise of the whole INVALID branch. If it ever changed, an old peer's attestation
// would start failing at the new binary and cost it attendance.
#[test]
fn m2_d3_ed25519_verification_ignores_the_bls_half_entirely() {
    let hash = golden_hash();
    let ed = golden_ed();
    let bls = golden_bls();

    let mut att = Attestation::new_with_bls(
        hash,
        GOLDEN_SLOT,
        GOLDEN_HEIGHT,
        GOLDEN_WEIGHT,
        ed.private_key(),
        *ed.public_key(),
        &bls,
    )
    .unwrap();
    assert!(att.verify().is_ok());

    att.bls_signature[0] ^= 0xFF;
    assert!(
        att.verify().is_ok(),
        "Ed25519 does not cover the BLS field (halt-mutation premise, C1)"
    );

    att.bls_signature.clear();
    assert!(
        att.verify().is_ok(),
        "an empty BLS half must keep Ed25519 valid (mixed-fleet bridge)"
    );
}

// REQ-BLS-010 — Decision: the deleted fallback returned an EMPTY signature on a BLS
// error, so a broken signer would silently drop out of every aggregate while looking
// healthy; the error must reach the egress that decides to ship anyway (F3).
#[test]
fn m2_d3_the_silent_empty_signature_fallback_is_gone_and_the_error_is_returned() {
    let src = code_only(&read("crates/core/src/attestation/message.rs"));
    assert!(
        !src.contains("Err(_) => Vec::new()"),
        "D3/F3: the silent empty-signature fallback must be removed"
    );
    assert!(
        src.contains("Result<Self, crypto::BlsError>") || src.contains("Result<Self, BlsError>"),
        "D3: new_with_bls must return the BLS error to its caller"
    );

    let hash = golden_hash();
    let ed = golden_ed();
    let bls = golden_bls();
    let out: Result<Attestation, crypto::BlsError> = Attestation::new_with_bls(
        hash,
        GOLDEN_SLOT,
        GOLDEN_HEIGHT,
        GOLDEN_WEIGHT,
        ed.private_key(),
        *ed.public_key(),
        &bls,
    );
    assert!(out.is_ok(), "a valid key pair must take the Ok path");
}

// ============================================================
// F3 — the gossip wire (REQ-BLS-006, Release-N mixed fleet)
// ============================================================

// REQ-BLS-006 — Decision: `Attestation` is bincode-POSITIONAL. If M2 adds, moves or
// retypes a field, every attestation an un-upgraded peer gossips decodes into
// garbage or fails outright — a fleet-wide attendance stop with no error message.
#[test]
fn m2_wire_a_pre_m2_attestation_still_decodes_field_for_field() {
    let bytes = hex::decode(PRE_M2_WIRE_HEX).expect("fixture must be valid hex");
    assert_eq!(bytes.len(), PRE_M2_WIRE_LEN);

    let att = Attestation::from_bytes(&bytes).expect("a pre-M2 attestation must still decode");
    assert_eq!(att.block_hash, golden_hash());
    assert_eq!(att.slot, GOLDEN_SLOT);
    assert_eq!(att.height, GOLDEN_HEIGHT);
    assert_eq!(att.attester, *golden_ed().public_key());
    assert_eq!(att.attester_weight, GOLDEN_WEIGHT);
    assert!(
        att.bls_signature.is_empty(),
        "a pre-M2 peer sends no BLS bytes"
    );
    assert!(
        att.verify().is_ok(),
        "and its Ed25519 half must still verify"
    );
    assert_eq!(
        att.to_bytes(),
        bytes,
        "re-serialization must reproduce the pre-M2 bytes exactly"
    );
}

// REQ-BLS-006 — Decision: pins the field ORDER, not just decodability. A reorder
// that happens to keep the total length would pass the round-trip test above and
// still silently swap `height` with `attester_weight` on the wire.
#[test]
fn m2_wire_the_bincode_field_layout_is_unchanged() {
    let bytes = hex::decode(PRE_M2_WIRE_HEX).unwrap();
    // bincode 1.x fixint/LE: every byte container carries an 8-byte length prefix.
    assert_eq!(
        &bytes[0..8],
        &32u64.to_le_bytes(),
        "block_hash length prefix"
    );
    assert_eq!(&bytes[8..40], golden_hash().as_bytes(), "block_hash");
    assert_eq!(&bytes[40..44], &GOLDEN_SLOT.to_le_bytes(), "slot");
    assert_eq!(&bytes[44..52], &GOLDEN_HEIGHT.to_le_bytes(), "height");
    assert_eq!(
        &bytes[52..60],
        &32u64.to_le_bytes(),
        "attester length prefix"
    );
    assert_eq!(&bytes[92..100], &GOLDEN_WEIGHT.to_le_bytes(), "weight");
    assert_eq!(&bytes[100..108], &64u64.to_le_bytes(), "signature length");
    assert_eq!(
        &bytes[172..180],
        &0u64.to_le_bytes(),
        "bls_signature is the LAST field and is empty here"
    );
}

// REQ-BLS-006 — Decision: the whole per-attestation bandwidth cost of D3. If the
// growth is not exactly 96 bytes, the encoder is emitting something other than one
// raw G2 point (hex, base64, a nested struct) and the aggregate size model is wrong.
#[test]
fn m2_wire_a_dual_signed_attestation_round_trips_and_costs_exactly_96_more_bytes() {
    let hash = golden_hash();
    let ed = golden_ed();
    let bls = golden_bls();

    let dual = Attestation::new_with_bls(
        hash,
        GOLDEN_SLOT,
        GOLDEN_HEIGHT,
        GOLDEN_WEIGHT,
        ed.private_key(),
        *ed.public_key(),
        &bls,
    )
    .unwrap();

    let bytes = dual.to_bytes();
    assert_eq!(bytes.len(), POST_M2_WIRE_LEN);
    assert_eq!(
        bytes.len() - PRE_M2_WIRE_LEN,
        96,
        "the length prefix is fixed at 8 bytes, so the delta is the G2 point alone"
    );
    assert_eq!(
        &bytes[..PRE_M2_WIRE_LEN - 8],
        &hex::decode(PRE_M2_WIRE_HEX).unwrap()[..PRE_M2_WIRE_LEN - 8],
        "every field BEFORE bls_signature must be byte-identical to the pre-M2 wire"
    );
    assert_eq!(&bytes[172..180], &96u64.to_le_bytes(), "bls length prefix");
    assert_eq!(&bytes[180..], &dual.bls_signature[..]);

    let decoded = Attestation::from_bytes(&bytes).expect("must round-trip");
    assert_eq!(decoded, dual);
}
