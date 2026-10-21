//! INC-I-236 M2 — shared intake gate helper (REQ-AUTH-019) and signature binding (REQ-AUTH-022).
//!
// covers: block validation signature
// covers: crates/core/src/validation/block.rs crates/core/src/validation/mod.rs crates/core/src/block/signature.rs crates/core/src/block.rs
// NEEDS-API: doli_core::validation::verify_block_signature_at(header: &BlockHeader, height: u64, activation_height: u64) -> Result<(), ValidationError>
//
// OUTPUT CONTRACT: fn verify_block_signature_at(&BlockHeader, height: u64, activation_height: u64)
//   O1 return Result<(), ValidationError>; O2 none (pure: no mutable param, receiver or store)
//   PATHS: P-BELOW height < AH, pi never read | P-ARMED height >= AH, verdict == verify_block_signature
//   INPUT PARTITIONS: sig {valid, forged (victim named, attacker pi), unsigned, garbage 5 B};
//     height {AH-1, AH, AH+1}; AH {0, 1_000, u64::MAX}
// OUTPUT CONTRACT: fn verify_block_signature(&BlockHeader)
//   O1 return Result<(), ValidationError>; O2 none
//   PATHS: P-ARMED only
//   INPUT PARTITIONS: one hashed header field changed after signing (12 fields);
//     `doli sign` digest over {preimage hex, lossy UTF-8 preimage, latin-1 preimage}
// MATRIX
//   O1(at) x P-BELOW x sig                  -> req_auth_019_verify_block_signature_at_below_ah_ignores_pi
//   O1(at) x P-ARMED x {forged, unsigned, garbage} x {AH, AH+1}
//                                           -> req_auth_019_verify_block_signature_at_armed_rejects_forged_and_unsigned
//   O1(at) x P-ARMED x valid x {AH, AH+1}   -> req_auth_019_verify_block_signature_at_armed_accepts_valid
//   O1(at) AH-1 vs AH, same header          -> req_auth_019_verify_block_signature_at_boundary_is_inclusive
//   O1(at) AH in {0, u64::MAX}              -> req_auth_019_verify_block_signature_at_extreme_activation_heights
//   O1(verify) x field change               -> req_auth_022_binding_every_hashed_field_breaks_signature
//   O1(verify) x doli sign preimage hex     -> req_auth_022_doli_sign_preimage_hex_rejected
//   O1(verify) x doli sign raw preimage     -> req_auth_022_doli_sign_raw_preimage_unreachable

use super::inc_i_236_forged_block_authn::{forge_block_as, sign_into_pi, signed_block};
use crypto::{Hash, KeyPair, PrivateKey, PublicKey, Signature};
use doli_core::block::{BlockHeader, BLOCK_SIGNATURE_DOMAIN};
use doli_core::validation::{verify_block_signature, ValidationError};

const AH: u64 = 1_000;

fn at(header: &BlockHeader, height: u64, activation_height: u64) -> Result<(), ValidationError> {
    doli_core::validation::verify_block_signature_at(header, height, activation_height)
}

fn valid() -> BlockHeader {
    signed_block().0.header
}

fn forged() -> BlockHeader {
    let (mut block, _ctx) = forge_block_as(PublicKey::from_bytes([0x11; 32]));
    sign_into_pi(&mut block, &KeyPair::from_seed([0x66; 32]));
    block.header
}

fn unsigned() -> BlockHeader {
    forge_block_as(PublicKey::from_bytes([0x11; 32])).0.header
}

fn garbage() -> BlockHeader {
    let mut h = valid();
    h.vdf_proof.pi = vec![0xEE; 5];
    h
}

fn bad_headers() -> Vec<(&'static str, BlockHeader)> {
    let bad = vec![
        ("forged", forged()),
        ("unsigned", unsigned()),
        ("garbage", garbage()),
    ];
    for (name, h) in &bad {
        assert!(
            verify_block_signature(h).is_err(),
            "fixture {name}: must fail verify_block_signature"
        );
    }
    assert!(verify_block_signature(&valid()).is_ok(), "fixture valid");
    bad
}

fn is_sig_err(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(ValidationError::InvalidBlockSignature { .. }))
}

// REQ-AUTH-019 — Decision: a failure means pre-height archive/backfill/restore imports of unsigned history start failing, or the helper reads pi below the height.
#[test]
fn req_auth_019_verify_block_signature_at_below_ah_ignores_pi() {
    let mut all = bad_headers();
    all.push(("valid", valid()));
    for (name, h) in &all {
        for height in [0, 1, AH - 1] {
            let r = at(h, height, AH);
            assert!(r.is_ok(), "{name} at h={height} < AH={AH}: got {r:?}");
        }
    }
}

// REQ-AUTH-019 — Decision: a failure means an intake writer that calls the helper still stores forged or unsigned blocks at or above the height.
#[test]
fn req_auth_019_verify_block_signature_at_armed_rejects_forged_and_unsigned() {
    for (name, h) in &bad_headers() {
        for height in [AH, AH + 1, AH + 1_000_000] {
            let r = at(h, height, AH);
            assert!(
                is_sig_err(&r),
                "{name} at h={height} >= AH={AH}: want InvalidBlockSignature, got {r:?}"
            );
        }
    }
}

// REQ-AUTH-019 — Decision: a failure means the helper rejects honest signed blocks, so every import path would refuse the real chain after the height.
#[test]
fn req_auth_019_verify_block_signature_at_armed_accepts_valid() {
    let h = valid();
    assert!(verify_block_signature(&h).is_ok(), "fixture valid");
    for height in [AH, AH + 1, AH + 1_000_000] {
        let r = at(&h, height, AH);
        assert!(r.is_ok(), "valid at h={height} >= AH={AH}: got {r:?}");
    }
}

// REQ-AUTH-019 — Decision: a failure means an off-by-one lets the activation block itself be imported unsigned, or arms one block early.
#[test]
fn req_auth_019_verify_block_signature_at_boundary_is_inclusive() {
    for (name, h) in &bad_headers() {
        let below = at(h, AH - 1, AH);
        let on = at(h, AH, AH);
        assert!(below.is_ok(), "{name} at AH-1: got {below:?}");
        assert!(is_sig_err(&on), "{name} at AH: got {on:?}");
    }
}

// REQ-AUTH-019 — Decision: a failure means devnet (AH 0) imports unsigned blocks, or mainnet/testnet (AH u64::MAX) refuse their unsigned history.
#[test]
fn req_auth_019_verify_block_signature_at_extreme_activation_heights() {
    for (name, h) in &bad_headers() {
        let devnet = at(h, 1, 0);
        assert!(is_sig_err(&devnet), "{name} at h=1, AH=0: got {devnet:?}");
        for height in [1, 10_000_000, u64::MAX - 1] {
            let r = at(h, height, u64::MAX);
            assert!(r.is_ok(), "{name} at h={height}, AH=MAX: got {r:?}");
        }
    }
    let r = at(&valid(), 1, 0);
    assert!(r.is_ok(), "valid at h=1, AH=0: got {r:?}");
}

type Mutation = (&'static str, fn(&mut BlockHeader));

fn mutations() -> Vec<Mutation> {
    vec![
        ("version", |h| h.version ^= 1),
        ("prev_hash", |h| h.prev_hash = Hash::from_bytes([0xC1; 32])),
        ("merkle_root", |h| {
            h.merkle_root = Hash::from_bytes([0xC2; 32])
        }),
        ("presence_root", |h| {
            h.presence_root = Hash::from_bytes([0xC3; 32])
        }),
        ("genesis_hash", |h| {
            h.genesis_hash = Hash::from_bytes([0xC4; 32])
        }),
        ("timestamp", |h| h.timestamp += 1),
        ("slot", |h| h.slot += 1),
        ("producer", |h| {
            h.producer = *KeyPair::from_seed([0x66; 32]).public_key()
        }),
        ("vdf_output", |h| h.vdf_output.value[0] ^= 1),
        ("missed_producers", |h| {
            h.missed_producers.push(PublicKey::from_bytes([0x11; 32]))
        }),
        ("data_root", |h| h.data_root = Hash::from_bytes([0xC5; 32])),
        ("fork_id", |h| h.fork_id = Hash::from_bytes([0xC6; 32])),
    ]
}

// REQ-AUTH-022 — Decision: a failure means a signature lifted from one block validates a header that differs in that field, so a forger reuses real signatures.
#[test]
fn req_auth_022_binding_every_hashed_field_breaks_signature() {
    let signed = valid();
    assert!(verify_block_signature(&signed).is_ok(), "fixture valid");
    let mut moved = Vec::new();
    for (field, mutate) in mutations() {
        let mut h = signed.clone();
        mutate(&mut h);
        assert_eq!(h.vdf_proof.pi, signed.vdf_proof.pi, "{field}: pi kept");
        assert_ne!(
            h.hash(),
            signed.hash(),
            "{field}: mutation must change hash()"
        );
        if verify_block_signature(&h).is_ok() {
            moved.push(field);
        }
    }
    assert!(
        moved.is_empty(),
        "signature still verifies after changing: {moved:?}"
    );
}

fn doli_sign(message: &str, kp: &KeyPair) -> Vec<u8> {
    let private_key = PrivateKey::from_hex(&kp.private_key().to_hex()).expect("key hex");
    let message_hash = crypto::hash::hash(message.as_bytes());
    let sig_hex = crypto::signature::sign(message_hash.as_bytes(), &private_key).to_hex();
    Signature::from_hex(&sig_hex).expect("sig hex").to_vec()
}

fn preimage(h: &BlockHeader) -> Vec<u8> {
    let mut p = BLOCK_SIGNATURE_DOMAIN.to_vec();
    p.extend_from_slice(h.hash().as_bytes());
    assert_eq!(p.len(), 50, "domain 18 B + hash 32 B");
    p
}

// REQ-AUTH-022 — Decision: a failure means a producer phished into `doli sign` of the preimage hex hands out a usable block signature.
#[test]
fn req_auth_022_doli_sign_preimage_hex_rejected() {
    let (block, _ctx, kp) = signed_block();
    let hex: String = preimage(&block.header)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hex.len(), 100);
    for text in [hex.clone(), hex.to_uppercase()] {
        let mut h = block.header.clone();
        h.vdf_proof.pi = doli_sign(&text, &kp);
        assert_eq!(h.vdf_proof.pi.len(), 64);
        let r = verify_block_signature(&h);
        assert!(is_sig_err(&r), "doli sign {text:?}: got {r:?}");
    }
}

// REQ-AUTH-022 — Decision: a failure means some text a user can type into `doli sign` reproduces the raw preimage and yields a block signature.
#[test]
fn req_auth_022_doli_sign_raw_preimage_unreachable() {
    let (block, _ctx, kp) = signed_block();
    let raw = preimage(&block.header);
    assert!(
        std::str::from_utf8(&raw).is_err(),
        "raw preimage must not be a &str"
    );
    let lossy = String::from_utf8_lossy(&raw).into_owned();
    let latin1: String = raw.iter().map(|&b| b as char).collect();
    for (name, text) in [("lossy", lossy), ("latin1", latin1)] {
        assert_ne!(text.as_bytes(), &raw[..], "{name}: transcoding must differ");
        let mut h = block.header.clone();
        h.vdf_proof.pi = doli_sign(&text, &kp);
        let r = verify_block_signature(&h);
        assert!(is_sig_err(&r), "doli sign {name} preimage: got {r:?}");
    }
}
