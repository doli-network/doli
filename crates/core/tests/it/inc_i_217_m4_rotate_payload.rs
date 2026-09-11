//! INC-I-217 M4 — `RotateBlsData` codec and the inner-signature preimage.
//!
//! covers: REQ-ROT-002, REQ-ROT-003, REQ-ROT-SEC-001, REQ-ROT-SEC-009
//!
//! Authority: `specs/bls-key-rotation-architecture.md` D1/D2 + "Preimage byte
//! layouts". The requirements doc is STALE here (248 bytes, `expiry_height`,
//! `allows_empty_io = true`); none of those values appear below.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1: `RotateBlsData::encode(&self) -> [u8; 240]`
//!       O1 return: the 240 fixed bytes — the ONLY output
//!       O2 mutable params / O3 receiver: NONE (`&self`, no interior mutability)
//!       O4 store / O5 statics / O6 channels: NONE
//!       PATHS: one, unconditional (infallible)
//!   F2: `RotateBlsData::decode(&[u8]) -> Option<Self>`
//!       O1 return: `Some(fields)` or `None`; no other output
//!       PATHS: len == 240 -> Some; len != 240 -> None
//!   F3: `rotation_auth_preimage(genesis, new_key, prev_tx, index) -> Vec<u8>`
//!       O1 return: the byte string; PATHS: one, unconditional
//!   F4: `rotation_auth_digest(..) -> [u8; 32]`
//!       O1 return: BLAKE3-256 of F3; PATHS: one, unconditional
//!   F5: `TxType::RotateBlsKey.allows_empty_io() -> bool`
//!       O1 return: the classification; PATHS: one match arm
//!   F6: `validate_transaction(&tx, &ctx) -> Result<(), ValidationError>`
//!       O1 return: `Ok(())` or `Err(ValidationError)` — the ONLY output
//!       O2 mutable params / O3 receiver / O4 store: NONE (both args `&`)
//!       PATHS (this suite): the `RotateBlsKey` dispatch arm -> Err;
//!       the same shape under another tx_type -> Ok
//!
//! INPUT PARTITIONS:
//!   F1  I1 distinct per-field fillers (field-order detector)
//!       I2 non-uniform spread bytes; I3 all-zero; I4 all-0xFF
//!   F2  I5 len 0; I6 len 1; I7 len 239; I8 len 240; I9 len 241; I10 len 1024
//!   F3  I11 the pinned vector; I12 genesis lengths 4 / 32 / 64
//!   F4  I13 output_index perturbed; I14 prev_tx_hash perturbed;
//!       I15 new_bls_pubkey perturbed; I16 genesis_hash perturbed;
//!       I17 index 1 vs 0x01000000 (endianness)
//!   F5  I18 RotateBlsKey; I19 a curated exempt type (non-vacuity)
//!   F6  I20 a canonical 1-in/1-out RotateBlsKey tx carrying 240 valid bytes;
//!       I21 the byte-identical tx retyped Transfer (non-vacuity)
//!
//!   MATRIX 6 functions x 21 partitions: every return value is claimed by a
//!   named test below; the absent output categories are structural and are
//!   asserted once, in the F1 I1 test, by re-reading the receiver after encode.

use crypto::Hash;
use doli_core::consensus::ConsensusParams;
use doli_core::network_params::NetworkParams;
use doli_core::transaction::{
    rotation_auth_digest, rotation_auth_preimage, Input, Output, RotateBlsData, Transaction,
    TxType, ROTATE_BLS_DATA_LEN,
};
use doli_core::validation::{validate_transaction, ValidationContext, ValidationError};
use doli_core::Network;

const GOLDEN_GENESIS: [u8; 32] = [0x11; 32];
const GOLDEN_NEW_KEY: [u8; 48] = [0x22; 48];
const GOLDEN_PREV_TX: [u8; 32] = [0x33; 32];
const GOLDEN_OUTPUT_INDEX: u32 = 7;

/// `"DOLI-ROTATE-BLS-V1" ‖ 0x11*32 ‖ 0x22*48 ‖ 0x33*32 ‖ 07000000`, written out
/// from the spec layout by hand — NOT captured from an implementation run.
const GOLDEN_PREIMAGE_HEX: &str = concat!(
    "444f4c492d524f544154452d424c532d5631",
    "1111111111111111111111111111111111111111111111111111111111111111",
    "222222222222222222222222222222222222222222222222",
    "222222222222222222222222222222222222222222222222",
    "3333333333333333333333333333333333333333333333333333333333333333",
    "07000000",
);

/// BLAKE3-256 of `GOLDEN_PREIMAGE_HEX`.
const GOLDEN_DIGEST_HEX: &str = "18ab518033c6720442af298538f831e3d58536f6d87df9aac3dc1872a477196b";

const DOMAIN_LEN: usize = 18;
const FIXED_TAIL_LEN: usize = 48 + 32 + 4;

fn golden_preimage() -> Vec<u8> {
    hex::decode(GOLDEN_PREIMAGE_HEX).expect("golden preimage is valid hex")
}

fn golden_digest() -> [u8; 32] {
    let v = hex::decode(GOLDEN_DIGEST_HEX).expect("golden digest is valid hex");
    v.try_into().expect("golden digest is 32 bytes")
}

fn sample() -> RotateBlsData {
    RotateBlsData {
        producer: [0xA1; 32],
        new_bls_pubkey: [0xB2; 48],
        bls_pop: [0xC3; 96],
        signature: [0xD4; 64],
    }
}

/// Deterministic non-uniform bytes so a field-order swap cannot pass unnoticed.
fn spread(seed: u8, n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| {
            seed.wrapping_mul(31)
                .wrapping_add((i as u8).wrapping_mul(7))
        })
        .collect()
}

fn spread_payload() -> RotateBlsData {
    RotateBlsData {
        producer: spread(1, 32).try_into().unwrap(),
        new_bls_pubkey: spread(2, 48).try_into().unwrap(),
        bls_pop: spread(3, 96).try_into().unwrap(),
        signature: spread(4, 64).try_into().unwrap(),
    }
}

fn golden_pre(genesis: &[u8]) -> Vec<u8> {
    rotation_auth_preimage(
        genesis,
        &GOLDEN_NEW_KEY,
        &GOLDEN_PREV_TX,
        GOLDEN_OUTPUT_INDEX,
    )
}

fn golden_dig(genesis: &[u8]) -> [u8; 32] {
    rotation_auth_digest(
        genesis,
        &GOLDEN_NEW_KEY,
        &GOLDEN_PREV_TX,
        GOLDEN_OUTPUT_INDEX,
    )
}

// ===========================================================================
// F1 / F2 — the 240-byte codec (REQ-ROT-002)
// ===========================================================================

// REQ-ROT-002 — Decision: whether the on-wire payload length every future decoder and length check assumes has silently moved off 240.
#[test]
fn req_rot_002_encode_produces_exactly_240_bytes() {
    assert_eq!(ROTATE_BLS_DATA_LEN, 240);
    assert_eq!(ROTATE_BLS_DATA_LEN, 32 + 48 + 96 + 64);
    assert_eq!(sample().encode().len(), 240);
    assert_eq!(spread_payload().encode().len(), 240);
}

// REQ-ROT-002 — Decision: whether two fields were transposed, which would make every rotation signature verify against the wrong key while the length stays right.
#[test]
fn req_rot_002_every_field_lands_at_its_documented_offset() {
    let d = sample();
    let bytes = d.encode();

    assert_eq!(&bytes[0..32], &d.producer[..], "producer offset 0..32");
    assert_eq!(
        &bytes[32..80],
        &d.new_bls_pubkey[..],
        "new_bls_pubkey offset 32..80"
    );
    assert_eq!(&bytes[80..176], &d.bls_pop[..], "bls_pop offset 80..176");
    assert_eq!(
        &bytes[176..240],
        &d.signature[..],
        "signature offset 176..240"
    );

    // O2/O3: encode takes `&self` and mutates nothing observable.
    assert_eq!(d, sample());
}

// REQ-ROT-002 — Decision: whether a same-length field permutation exists that the offset test with uniform fillers would miss.
#[test]
fn req_rot_002_non_uniform_fields_survive_a_round_trip_in_order() {
    let d = spread_payload();
    let bytes = d.encode();

    assert_eq!(&bytes[0..32], &d.producer[..]);
    assert_eq!(&bytes[32..80], &d.new_bls_pubkey[..]);
    assert_eq!(&bytes[80..176], &d.bls_pop[..]);
    assert_eq!(&bytes[176..240], &d.signature[..]);

    let back = RotateBlsData::decode(&bytes).expect("240 bytes decode");
    assert_eq!(back, d);
}

// REQ-ROT-002 — Decision: whether decode drops or reorders a field, so a producer's rotation would install a key it never signed for.
#[test]
fn req_rot_002_decode_round_trips_every_field() {
    for d in [
        sample(),
        spread_payload(),
        RotateBlsData {
            producer: [0x00; 32],
            new_bls_pubkey: [0x00; 48],
            bls_pop: [0x00; 96],
            signature: [0x00; 64],
        },
        RotateBlsData {
            producer: [0xFF; 32],
            new_bls_pubkey: [0xFF; 48],
            bls_pop: [0xFF; 96],
            signature: [0xFF; 64],
        },
    ] {
        let back = RotateBlsData::decode(&d.encode()).expect("round-trip decodes");
        assert_eq!(back.producer, d.producer);
        assert_eq!(back.new_bls_pubkey, d.new_bls_pubkey);
        assert_eq!(back.bls_pop, d.bls_pop);
        assert_eq!(back.signature, d.signature);
        assert_eq!(back, d);
    }
}

// REQ-ROT-002 — Decision: whether decode became length-tolerant, which would let an attacker append or truncate bytes around a payload whose signature only covers the canonical 240.
#[test]
fn req_rot_002_decode_rejects_every_length_but_240() {
    for len in [0usize, 1, 239, 241, 1024] {
        assert!(
            RotateBlsData::decode(&vec![0x5A; len]).is_none(),
            "decode must reject a {len}-byte payload"
        );
    }

    // Non-vacuity: the same filler at exactly 240 DOES decode.
    assert!(RotateBlsData::decode(&vec![0x5A; 240]).is_some());

    // A canonical payload with ONE trailing byte is not canonical.
    let mut trailing = sample().encode().to_vec();
    trailing.push(0x00);
    assert!(
        RotateBlsData::decode(&trailing).is_none(),
        "a trailing byte must not be ignored"
    );

    let truncated = &sample().encode()[..239];
    assert!(RotateBlsData::decode(truncated).is_none());
}

// ===========================================================================
// F5 — fee-paying shape (REQ-ROT-003, architecture D2)
// ===========================================================================

// REQ-ROT-003 — Decision: whether RotateBlsKey was classified fee-exempt, which would turn D2's single-use outpoint into a free unlimited-retry rotation channel.
#[test]
fn req_rot_003_rotate_bls_key_is_not_empty_io_exempt() {
    assert!(
        !TxType::RotateBlsKey.allows_empty_io(),
        "D2 makes rotation a fee-paying 1-in/1-out tx"
    );

    // Non-vacuity: the predicate still returns true for a curated exempt type.
    assert!(TxType::Registration.allows_empty_io());

    let exempt = (0..=255u32)
        .filter_map(TxType::from_u32)
        .filter(|t| t.allows_empty_io())
        .count();
    assert_eq!(exempt, 5, "the empty-io exempt set must stay at five types");
}

// ===========================================================================
// F3 / F4 — the inner-signature preimage (REQ-ROT-SEC-001)
// ===========================================================================

// REQ-ROT-SEC-001 — Decision: whether the bytes a producer's wallet signs still equal the bytes the node will verify, byte for byte.
#[test]
fn req_rot_sec_001_preimage_matches_the_pinned_spec_bytes() {
    assert_eq!(
        golden_pre(&GOLDEN_GENESIS),
        golden_preimage(),
        "the preimage drifted from the pinned spec layout"
    );
}

// REQ-ROT-SEC-001 — Decision: whether a field was reordered or the domain tag changed, which the whole-string golden reports only as "different".
#[test]
fn req_rot_sec_001_each_preimage_field_sits_at_its_spec_offset() {
    let pre = golden_pre(&GOLDEN_GENESIS);
    let g = GOLDEN_GENESIS.len();

    assert_eq!(&pre[0..DOMAIN_LEN], b"DOLI-ROTATE-BLS-V1");
    assert_eq!(&pre[DOMAIN_LEN..DOMAIN_LEN + g], &GOLDEN_GENESIS[..]);
    assert_eq!(
        &pre[DOMAIN_LEN + g..DOMAIN_LEN + g + 48],
        &GOLDEN_NEW_KEY[..]
    );
    assert_eq!(
        &pre[DOMAIN_LEN + g + 48..DOMAIN_LEN + g + 80],
        &GOLDEN_PREV_TX[..]
    );
    assert_eq!(
        &pre[DOMAIN_LEN + g + 80..],
        &GOLDEN_OUTPUT_INDEX.to_le_bytes()[..],
        "output_index is a u32 little-endian tail"
    );
}

// REQ-ROT-SEC-001 — Decision: whether a length prefix or a key-kind byte crept in, which would make every wallet-built signature unverifiable by the node.
#[test]
fn req_rot_sec_001_preimage_carries_no_length_prefix_and_no_key_kind_byte() {
    for glen in [4usize, 32, 64] {
        let genesis = vec![0x11u8; glen];
        let pre = golden_pre(&genesis);
        assert_eq!(
            pre.len(),
            DOMAIN_LEN + glen + FIXED_TAIL_LEN,
            "length must be exactly 18 + genesis.len() + 84 for genesis len {glen}"
        );
        assert_eq!(&pre[DOMAIN_LEN..DOMAIN_LEN + glen], &genesis[..]);
    }
}

// REQ-ROT-SEC-001 — Decision: whether the digest the CLI computes still equals BLAKE3-256 of the published preimage, so M10's signer cannot drift from the verifier.
#[test]
fn req_rot_sec_001_digest_is_the_pinned_blake3_of_the_pinned_preimage() {
    assert_eq!(golden_dig(&GOLDEN_GENESIS), golden_digest());
    assert_eq!(
        golden_dig(&GOLDEN_GENESIS),
        *crypto::hash::hash(&golden_preimage()).as_bytes(),
        "digest must be BLAKE3-256 of the preimage, with no second domain tag"
    );
}

// REQ-ROT-SEC-001 — Decision: whether the signature still binds the spending outpoint, without which a third party can re-wrap an old authorisation around a fresh input.
#[test]
fn req_rot_sec_001_digest_commits_to_the_spending_outpoint() {
    let base = golden_dig(&GOLDEN_GENESIS);

    let other_index = rotation_auth_digest(
        &GOLDEN_GENESIS,
        &GOLDEN_NEW_KEY,
        &GOLDEN_PREV_TX,
        GOLDEN_OUTPUT_INDEX + 1,
    );
    assert_ne!(base, other_index, "output_index must change the digest");

    let other_tx = rotation_auth_digest(
        &GOLDEN_GENESIS,
        &GOLDEN_NEW_KEY,
        &[0x34; 32],
        GOLDEN_OUTPUT_INDEX,
    );
    assert_ne!(base, other_tx, "prev_tx_hash must change the digest");

    // Endianness is load-bearing: index 1 and index 0x01000000 must differ.
    let le_probe_one = rotation_auth_digest(&GOLDEN_GENESIS, &GOLDEN_NEW_KEY, &GOLDEN_PREV_TX, 1);
    let le_probe_big = rotation_auth_digest(
        &GOLDEN_GENESIS,
        &GOLDEN_NEW_KEY,
        &GOLDEN_PREV_TX,
        0x0100_0000,
    );
    assert_ne!(le_probe_one, le_probe_big);
}

// REQ-ROT-SEC-001 — Decision: whether the authorisation still names the key being installed, without which any new key could be swapped into a valid signature.
#[test]
fn req_rot_sec_001_digest_commits_to_the_new_bls_pubkey() {
    let mut other_key = GOLDEN_NEW_KEY;
    other_key[47] ^= 0x01;

    assert_ne!(
        golden_dig(&GOLDEN_GENESIS),
        rotation_auth_digest(
            &GOLDEN_GENESIS,
            &other_key,
            &GOLDEN_PREV_TX,
            GOLDEN_OUTPUT_INDEX
        ),
        "a one-bit change in new_bls_pubkey must change the digest"
    );
}

// ===========================================================================
// Genesis binding
// ===========================================================================

// REQ-ROT-SEC-009 — Decision: whether a rotation signed on testnet could be replayed onto mainnet or devnet unchanged.
#[test]
fn req_rot_sec_009_digest_is_bound_to_the_genesis_hash() {
    let g1 = GOLDEN_GENESIS;
    let mut g2 = GOLDEN_GENESIS;
    g2[0] ^= 0xFF;

    assert_ne!(
        golden_dig(&g1),
        golden_dig(&g2),
        "the same rotation under a different genesis must not share a digest"
    );
    assert_ne!(golden_pre(&g1), golden_pre(&g2));

    // The genesis bytes are present verbatim, so the binding is not a hash of a hash.
    assert!(golden_pre(&g1).windows(32).any(|w| w == &g1[..]));
}

// ===========================================================================
// F6 — the fail-closed dispatch arm (REQ-ROT-001, architecture D6)
// ===========================================================================

// REQ-ROT-001 — Decision: whether a decodable RotateBlsKey transaction can reach a consensus path
// below its activation height — the INC-I-075 shape.
//
// M5 HAND-OFF (flipped 2026-09-11). M4 wrote this as a tripwire on its own blanket
// `InvalidTransaction` reject arm, due to fire the moment M5 replaced that arm. M5 landed the gate,
// so the assertion now names the typed refusal instead. The INVARIANT IS UNCHANGED AND STRICTER: a
// well-formed RotateBlsKey still does not validate on shipped mainnet params, and the refusal now
// has to prove WHY — `[ERRTX-ROT002]`, below `bls_key_rotation_activation_height`, whose shipped
// value on every network is `u64::MAX`.
#[test]
fn req_rot_001_rotate_bls_key_is_rejected_below_the_m5_gate() {
    let ctx = ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, 1_000_000);
    let recipient = crypto::hash::hash(b"inc-i-217-m4-fail-closed");

    // I20 — 1 input / 1 output carrying a canonical 240-byte payload, so every
    // structural check upstream of the dispatch match is satisfied.
    let tx = Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output::normal(1_000_000, recipient)],
        extra_data: sample().encode().to_vec(),
    };
    assert!(
        RotateBlsData::decode(&tx.extra_data).is_some(),
        "the probe must carry a payload M5 would accept as well-formed"
    );

    let verdict = validate_transaction(&tx, &ctx);
    assert!(
        verdict.is_err(),
        "a well-formed RotateBlsKey tx must not validate before the M5 gate exists"
    );
    let err = verdict.expect_err("checked immediately above");
    assert!(
        matches!(
            err,
            ValidationError::RotateBlsNotActivated {
                activation_height: u64::MAX,
                ..
            }
        ),
        "expected the typed below-gate refusal at the shipped u64::MAX height, got {err:?}"
    );
    assert!(
        err.to_string().contains("[ERRTX-ROT002]"),
        "the refusal must carry its stable code, got: {err}"
    );
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).bls_key_rotation_activation_height,
        u64::MAX,
        "the gate this test leans on must still be frozen on mainnet"
    );

    // I21 non-vacuity — the byte-identical transaction retyped Transfer DOES
    // validate, so the refusal above is the RotateBlsKey arm and not the shape.
    let control = Transaction {
        tx_type: TxType::Transfer,
        ..tx.clone()
    };
    assert!(
        validate_transaction(&control, &ctx).is_ok(),
        "the control shape must pass every structural check"
    );
}
