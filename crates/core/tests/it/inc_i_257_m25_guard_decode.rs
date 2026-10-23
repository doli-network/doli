//! INC-I-257 M2.5 — the covenant OutputTypeGuard decoder keeps the base
//! accepted set: a guard on type byte 16 (Unbonding) is an unknown tag.
//!
// covers: encoding parsers pool unbonding revalidate protocol security_model cli
//
// OUTPUT CONTRACT: fn Condition::decode(&[u8]) -> Result<Condition, ConditionError>
//                  fn Condition::decode_prefix(&[u8]) -> Result<(Condition, usize), ConditionError>
//                  fn validate_transaction(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//                  fn OutputType::from_u8(u8) -> Option<OutputType>
//   O1 return value of each; all pure, no receiver or store writes
// PATHS: P-DEC decode | P-PRE decode_prefix | P-FA FungibleAsset arm (ERRTX018)
//        | P-BH BridgeHTLC arm (ERRTX022) | P-CO conditioned arm (ERRTX010) | P-ST storage byte decode
// INPUT PARTITIONS:
//   guard type byte: 0 (Normal, control) | 16 (Unbonding) | 17 (undefined, reference)
//   nesting: bare guard | Or(guard, Signature)
//   height: gate - 1 | gate; network: Devnet | Mainnet (covenants active, guards inactive)
// MATRIX:
//   O1 x P-DEC, P-PRE -> inc_i_257_m25_req_unbond_017_guard16_decode_refused
//   O1 x P-FA -> inc_i_257_m25_req_unbond_017_fungible_asset_guard16_refused
//   O1 x P-BH -> inc_i_257_m25_req_unbond_017_bridge_htlc_guard16_refused
//   O1 x P-CO -> inc_i_257_m25_req_unbond_017_conditioned_output_guard16_refused
//   O1 x P-ST -> inc_i_257_m25_req_unbond_sec_002_storage_from_u8_16_golden

use crypto::Hash;
use doli_core::conditions::{Condition, ConditionError};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_core::validation::{validate_transaction, ValidationContext, ValidationError};
use doli_core::Network;

const GATE: u64 = 1_000_000;
const P: u64 = 60;
const BRIDGE_V2: u8 = 2;

fn owner() -> Hash {
    Hash::from_bytes([0xAA; 32])
}

fn guard(type_byte: u8) -> Vec<u8> {
    let mut b = Condition::OutputTypeGuard {
        expected_type: OutputType::Normal,
        output_index: 0,
    }
    .encode()
    .expect("FIXTURE: encode guard");
    assert_eq!(b.len(), 4, "FIXTURE: [version, tag, type, index]");
    b[2] = type_byte;
    b
}

fn or_guard(type_byte: u8) -> Vec<u8> {
    let mut b = Condition::Or(
        Box::new(Condition::OutputTypeGuard {
            expected_type: OutputType::Normal,
            output_index: 0,
        }),
        Box::new(Condition::Signature(owner())),
    )
    .encode()
    .expect("FIXTURE: encode or-guard");
    assert_eq!(b[3], 0, "FIXTURE: guard type byte sits at offset 3");
    b[3] = type_byte;
    b
}

fn fa_output(cond: Vec<u8>) -> Output {
    let base = Output::fungible_asset(
        1_000,
        owner(),
        Hash::from_bytes([0x11; 32]),
        1_000,
        "TOK",
        &Condition::Signature(owner()),
    )
    .expect("FIXTURE: fungible asset");
    let sig_len = Condition::Signature(owner()).encode().unwrap().len();
    let mut extra = cond;
    extra.extend_from_slice(&base.extra_data[sig_len..]);
    Output {
        extra_data: extra,
        ..base
    }
}

fn bh_output(cond: Vec<u8>) -> Output {
    let mut extra = cond;
    extra.extend_from_slice(&[BRIDGE_V2, 1, 3, 0xB0, 0xB1, 0xB2]);
    extra.extend_from_slice(&[0xC5; 32]);
    Output {
        output_type: OutputType::BridgeHTLC,
        amount: 1_000,
        pubkey_hash: owner(),
        lock_until: 0,
        extra_data: extra,
    }
}

fn co_output(cond: Vec<u8>) -> Output {
    Output {
        output_type: OutputType::Multisig,
        amount: 1_000,
        pubkey_hash: owner(),
        lock_until: 0,
        extra_data: cond,
    }
}

fn transfer(out: Output) -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::Transfer,
        inputs: vec![Input::new(Hash::from_bytes([0x42; 32]), 0)],
        outputs: vec![out],
        extra_data: Vec::new(),
    }
}

fn ctxs() -> Vec<(String, ValidationContext)> {
    let mut v = Vec::new();
    for (net, params) in [
        (Network::Devnet, ConsensusParams::devnet()),
        (Network::Mainnet, ConsensusParams::mainnet()),
    ] {
        for h in [GATE - 1, GATE] {
            let c =
                ValidationContext::new(params.clone(), net, 0, h).with_inc_i_257_unbonding(GATE, P);
            v.push((format!("{net:?} H={h}"), c));
        }
    }
    v
}

fn carries(r: &Result<(), ValidationError>, code: &str) -> bool {
    matches!(r, Err(e) if e.to_string().contains(code))
}

/// Control first (byte 0 never refused with `code`), then byte 16 must be refused with `code`.
fn assert_arm(build: fn(Vec<u8>) -> Output, code: &str) {
    for (label, c) in ctxs() {
        for (shape, enc) in [("bare", guard as fn(u8) -> Vec<u8>), ("or", or_guard)] {
            let ok = validate_transaction(&transfer(build(enc(0))), &c);
            assert!(
                !carries(&ok, code),
                "NON-VACUITY {label} {shape}: guard byte 0 must not hit {code}: {ok:?}"
            );
        }
    }
    for (label, c) in ctxs() {
        for (shape, enc) in [("bare", guard as fn(u8) -> Vec<u8>), ("or", or_guard)] {
            let r = validate_transaction(&transfer(build(enc(16))), &c);
            assert!(
                carries(&r, code) && carries(&r, "unknown condition tag: 0x10"),
                "{label} {shape}: guard byte 16 must be refused with {code} (unknown tag 0x10), got {r:?}"
            );
        }
    }
}

// REQ-UNBOND-017 — Decision: a failure means the guard decoder accepts type byte 16, so a condition the base binary refuses now decodes and the mixed fleet splits.
#[test]
fn inc_i_257_m25_req_unbond_017_guard16_decode_refused() {
    assert_eq!(
        Condition::decode(&guard(17)),
        Err(ConditionError::UnknownTag { tag: 17 }),
        "NON-VACUITY: undefined byte 17 is the reference unknown tag"
    );
    assert!(
        Condition::decode(&guard(0)).is_ok(),
        "NON-VACUITY: guard byte 0 (Normal) decodes"
    );
    assert_eq!(
        Condition::decode(&guard(16)),
        Err(ConditionError::UnknownTag { tag: 16 }),
        "decode: guard byte 16 must be UnknownTag {{ tag: 16 }}"
    );
    assert_eq!(
        Condition::decode_prefix(&guard(16)).map(|(_, n)| n),
        Err(ConditionError::UnknownTag { tag: 16 }),
        "decode_prefix: guard byte 16 must be UnknownTag {{ tag: 16 }}"
    );
    assert_eq!(
        Condition::decode(&or_guard(16)),
        Err(ConditionError::UnknownTag { tag: 16 }),
        "nested Or(guard16, Signature) must be UnknownTag {{ tag: 16 }}"
    );
}

// REQ-UNBOND-017 — Decision: a failure means a Transfer creating a FungibleAsset with a guard-16 owner condition is accepted where the base refuses it (ERRTX018).
#[test]
fn inc_i_257_m25_req_unbond_017_fungible_asset_guard16_refused() {
    assert_arm(fa_output, "[ERRTX018]");
}

// REQ-UNBOND-017 — Decision: a failure means a Transfer creating a BridgeHTLC with a guard-16 condition is accepted where the base refuses it (ERRTX022).
#[test]
fn inc_i_257_m25_req_unbond_017_bridge_htlc_guard16_refused() {
    assert_arm(bh_output, "[ERRTX022]");
}

// REQ-UNBOND-017 — Decision: a failure means a conditioned output with a guard-16 condition is accepted where the base refuses it (ERRTX010).
#[test]
fn inc_i_257_m25_req_unbond_017_conditioned_output_guard16_refused() {
    let c = ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, GATE)
        .with_inc_i_257_unbonding(GATE, P);
    let ok = validate_transaction(&transfer(co_output(guard(0))), &c);
    assert!(
        !carries(&ok, "[ERRTX010]"),
        "NON-VACUITY: guard byte 0 must not hit ERRTX010: {ok:?}"
    );
    let r = validate_transaction(&transfer(co_output(guard(16))), &c);
    assert!(
        carries(&r, "[ERRTX010]") && carries(&r, "unknown condition tag: 0x10"),
        "devnet guards active: guard byte 16 must be refused with ERRTX010, got {r:?}"
    );
}

// REQ-UNBOND-SEC-002 — Decision: a failure means the guard fix also broke storage decode of the Unbonding type byte, so stored Unbonding rows stop loading.
#[test]
fn inc_i_257_m25_req_unbond_sec_002_storage_from_u8_16_golden() {
    assert_eq!(OutputType::from_u8(16), Some(OutputType::Unbonding));
    assert_eq!(OutputType::from_u8(17), None);
}
