//! INC-I-255 M5 (AUDIT-P1-517) — the per-header evidence predicate and the
//! SlashData fit predicate the detector uses agree with consensus validation.
//!
// OUTPUT CONTRACT: fn evidence_header_admissible(&BlockHeader, &Hash, Network) -> Result<(), ValidationError>
//   O1 return (pure; no params mutated, no store writes)
// OUTPUT CONTRACT: fn double_production_evidence_fits(&PublicKey, &BlockHeader, &BlockHeader) -> bool
//   O1 return (pure)
// PATHS: genesis check | signature check | VDF check | all pass ; fits | does not fit
// MATRIX
//   admissible O1 x {valid, foreign genesis, other-key signature, tampered after signing, garbage VDF}
//     -> Ok only for valid; each verdict equals validate_transaction on (h, valid sibling)
//     (inc_i_255_m5_admissible_*)
//   fits O1 x {two small headers, oversize + small (both orders)} -> true | false,
//     and false matches the consensus size refusal (inc_i_255_m5_fits_*)
// INPUT PARTITIONS:
//   header: valid | foreign genesis | wrong signer | tampered | garbage VDF | 2,100 missed producers
//   ctx: devnet, slash and signature gates active (HEIGHT)

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::block::BlockHeader;
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{SlashData, Transaction};
use doli_core::validation::{
    double_production_evidence_fits, evidence_header_admissible, validate_transaction,
    ValidationError,
};

use super::inc_i_255_slash_evidence::{accused, ctx_at, header, sign, HEIGHT};

const SLOT: u32 = 5_000;
const OVERSIZE_MISSED: usize = 2_100;

fn chain() -> Hash {
    ConsensusParams::devnet().genesis_hash
}

fn signed_with(prev: u8, edit: impl FnOnce(&mut BlockHeader), kp: &KeyPair) -> BlockHeader {
    let mut h = header(prev, SLOT, *accused().public_key());
    edit(&mut h);
    sign(&mut h, kp);
    h
}

fn good(prev: u8) -> BlockHeader {
    signed_with(prev, |_| {}, &accused())
}

fn admissible(h: &BlockHeader) -> Result<(), ValidationError> {
    evidence_header_admissible(h, &chain(), Network::Devnet)
}

fn consensus(h1: BlockHeader, h2: BlockHeader) -> Result<(), ValidationError> {
    let pk = *accused().public_key();
    let tx = Transaction::new_slash_producer(SlashData::double_production(pk, h1, h2));
    validate_transaction(&tx, &ctx_at(HEIGHT, HEIGHT, HEIGHT))
}

fn partitions() -> Vec<(&'static str, BlockHeader)> {
    let other = KeyPair::generate();
    let mut tampered = good(0x34);
    tampered.timestamp += 1;
    vec![
        (
            "foreign genesis",
            signed_with(
                0x31,
                |h| h.genesis_hash = Hash::from_bytes([0x66; 32]),
                &accused(),
            ),
        ),
        ("other-key signature", signed_with(0x32, |_| {}, &other)),
        (
            "garbage VDF",
            signed_with(0x33, |h| h.vdf_output.value = vec![0xEE; 32], &accused()),
        ),
        ("tampered after signing", tampered),
    ]
}

// REQ-SLASH-004 — Decision: a failure means the detector refuses a header consensus accepts, so a real equivocation is never paired (AUDIT-P1-517).
#[test]
fn inc_i_255_m5_admissible_valid_header_ok() {
    let h = good(0x30);
    let r = admissible(&h);
    assert!(
        r.is_ok(),
        "a signed, chain-genesis, VDF-valid header is admissible; got {r:?}"
    );
    let c = consensus(h, good(0x2F));
    assert!(
        c.is_ok(),
        "NON-VACUITY: consensus accepts the pair; got {c:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means the detector stores a decoy consensus refuses (foreign genesis, bad signature, bad VDF) (AUDIT-P1-517).
#[test]
fn inc_i_255_m5_admissible_rejects_each_invalid_header_like_consensus() {
    for (case, h) in partitions() {
        let r = admissible(&h);
        assert!(r.is_err(), "{case}: must not be admissible; got Ok");
        let c = consensus(h, good(0x2F));
        assert!(
            matches!(c, Err(ValidationError::InvalidSlash(_))),
            "{case}: PARITY: consensus refuses the pair; got {c:?}"
        );
    }
}

// REQ-SLASH-004 — Decision: a failure means the fit predicate refuses a small pair, so the detector never proves a real equivocation.
#[test]
fn inc_i_255_m5_fits_two_small_headers() {
    let pk: PublicKey = *accused().public_key();
    let (a, b) = (good(0x40), good(0x41));
    assert!(double_production_evidence_fits(&pk, &a, &b));
    assert!(
        double_production_evidence_fits(&pk, &b, &a),
        "order must not matter"
    );
}

// REQ-SLASH-004 — Decision: a failure means the detector proves an oversize pair consensus refuses, wasting the offence (AUDIT-P1-517).
#[test]
fn inc_i_255_m5_fits_oversize_pair_refused_like_consensus() {
    let pk: PublicKey = *accused().public_key();
    let o = signed_with(
        0x42,
        |h| h.missed_producers = vec![pk; OVERSIZE_MISSED],
        &accused(),
    );
    let a = good(0x43);
    let r = admissible(&o);
    assert!(
        r.is_ok(),
        "FIXTURE: the oversize header is otherwise valid; got {r:?}"
    );
    assert!(
        !double_production_evidence_fits(&pk, &o, &a),
        "oversize + small must not fit"
    );
    assert!(
        !double_production_evidence_fits(&pk, &a, &o),
        "order must not matter"
    );
    let c = consensus(o, a);
    assert!(
        c.is_err(),
        "PARITY: consensus refuses the oversize pair; got Ok"
    );
}
