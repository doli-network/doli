//! INC-I-173 M1 — shared test fixtures.
//!
//! OUTPUT CONTRACT: N/A — fixture file. This module asserts nothing. It only
//! builds the 24 canonical 0-in/0-out transactions and the two
//! `ValidationContext` shapes that the INC-I-173 M1 test files drive.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! Spec: `specs/state-only-fee-gate-architecture.md` (F1, F2, F3).
//! Analysis: `docs/redesigns/state-only-fee-gate-redesign-analysis.md`
//! (REQ-173-001 .. REQ-173-007).
//!
//! WHY A SHARED FIXTURE. Every INC-I-173 requirement is a statement about the
//! SAME transaction shape (0 inputs, 0 outputs) evaluated at DIFFERENT heights
//! against the SAME validator. If each test file built its own payloads the
//! bit-identity claim of REQ-173-003 would compare apples to oranges. One
//! builder, one payload per type, every file.

#![allow(dead_code)]

use crypto::{Hash, KeyPair};
use doli_core::maintainer::{MaintainerChangeData, MaintainerSignature, ProtocolActivationData};
use doli_core::transaction::{
    AddBondData, ClaimBondData, ClaimData, DelegateBondData, ExitData, Output, RegistrationData,
    RevokeDelegationData, SlashData, SlashingEvidence, Transaction, TxType, WithdrawalRequestData,
};
use doli_core::validation::{UtxoInfo, UtxoProvider};

// ---------------------------------------------------------------------------
// The synthetic activation height used by every INC-I-173 M1 validator test.
//
// It is deliberately NOT any real network's pinned value: the tests must prove
// the GATE MECHANISM, not a particular literal. The literals themselves are
// pinned separately in `inc_i_173_activation_height.rs` (REQ-173-005).
//
// It is also deliberately far above every network's `genesis_blocks`
// (mainnet 360 / testnet 36 / devnet 40) so that BELOW_GATE is outside the
// genesis window — otherwise the Registration genesis branch would mask the
// fee gate.
// ---------------------------------------------------------------------------

/// The synthetic `inc_i_173_activation_height` under test.
pub const TEST_AH: u64 = 200_000;
/// `AH - 1`. The REQ-173-003 bit-identity height (C8).
pub const BELOW_GATE: u64 = TEST_AH - 1;
/// `AH` exactly. The gate is `>=`, so this is ABOVE the gate.
pub const AT_GATE: u64 = TEST_AH;
/// Far above the gate.
pub const ABOVE_GATE: u64 = 500_000;

/// A mainnet height inside the genesis window (`genesis_blocks = 360`).
/// Used by REQ-173-002 — the genesis `Registration` only takes its 0-in/0-out
/// branch while `Network::is_in_genesis(height)` holds.
pub const MAINNET_GENESIS_HEIGHT: u64 = 300;

/// A devnet height inside the devnet genesis window (`genesis_blocks = 40`).
pub const DEVNET_GENESIS_HEIGHT: u64 = 10;

/// Every live `TxType` variant. `TxType` is not `#[non_exhaustive]` and
/// `from_u32` closes the set, so this array is the value-level twin of the
/// compile-time exhaustive `match` inside `allows_empty_io()`.
///
/// 24 entries = discriminants 0..=22 (9 is the `ClaimWithdrawal` tombstone,
/// which is still a live VARIANT) plus `ZKSettle = 31`. 24..=30 are retired
/// discriminants with no variant.
pub const ALL_TX_TYPES: [TxType; 24] = [
    TxType::Transfer,
    TxType::Registration,
    TxType::Exit,
    TxType::ClaimReward,
    TxType::ClaimBond,
    TxType::SlashProducer,
    TxType::Coinbase,
    TxType::AddBond,
    TxType::RequestWithdrawal,
    TxType::ClaimWithdrawal,
    TxType::EpochReward,
    TxType::RemoveMaintainer,
    TxType::AddMaintainer,
    TxType::DelegateBond,
    TxType::RevokeDelegation,
    TxType::ProtocolActivation,
    TxType::PriceAttestation,
    TxType::MintAsset,
    TxType::BurnAsset,
    TxType::CreatePool,
    TxType::AddLiquidity,
    TxType::RemoveLiquidity,
    TxType::Swap,
    TxType::ZKSettle,
];

/// The EXACT exempt set decided by spec F3, curated by AUTHORIZATION and not
/// by wire shape.
///
/// `Exit` and `SlashProducer` are absent BY DESIGN (constraint C1): their apply
/// handlers accept an actor identity without verifying a signature.
/// `ClaimReward` / `ClaimBond` are absent because they carry outputs.
pub const EXPECTED_EXEMPT_SET: [TxType; 5] = [
    TxType::Registration,
    TxType::DelegateBond,
    TxType::RevokeDelegation,
    TxType::AddMaintainer,
    TxType::RemoveMaintainer,
];

/// The FROZEN legacy exempt set — the literal `matches!` at
/// `crates/core/src/validation/utxo.rs:222` that must be retained
/// character-identical on the below-the-gate branch (INV-COMPAT-001).
pub const FROZEN_LEGACY_EXEMPT_SET: [TxType; 3] = [
    TxType::Registration,
    TxType::DelegateBond,
    TxType::RevokeDelegation,
];

/// A `UtxoProvider` that resolves nothing. Correct for INC-I-173: every
/// transaction under test has ZERO inputs, so the provider is never consulted.
/// Returning `None` unconditionally makes that fact load-bearing — if a test
/// accidentally builds a tx WITH inputs, it fails loudly on `UtxoNotFound`
/// instead of silently taking a different path.
pub struct EmptyUtxos;

impl UtxoProvider for EmptyUtxos {
    fn get_utxo(&self, _tx_hash: &Hash, _output_index: u32) -> Option<UtxoInfo> {
        None
    }
}

/// Deterministic primary keypair. Fixed seed — the payloads must be
/// byte-stable across runs so the REQ-173-003 verdict table is reproducible.
pub fn kp_a() -> KeyPair {
    KeyPair::from_seed([7u8; 32])
}

/// Deterministic secondary keypair (delegate / counterparty).
pub fn kp_b() -> KeyPair {
    KeyPair::from_seed([8u8; 32])
}

fn dummy_header(producer: crypto::PublicKey, slot: u32, nonce: u8) -> doli_core::BlockHeader {
    doli_core::BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: crypto::hash::hash(&[nonce]),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 1,
        slot,
        producer,
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

/// The EXACT genesis `RegistrationData` shape the block builder emits at
/// `bins/node/src/node/production/assembly.rs:124-143`.
///
/// Field-for-field mirror — `epoch: 0`, empty `vdf_proof`,
/// `prev_registration_hash: Hash::ZERO`, `sequence_number: 0`,
/// `bond_count: 0`, and a REAL BLS pubkey + proof-of-possession (the genesis
/// branch of `validate_registration_data` calls `validate_bls_pop`, so a
/// placeholder would make REQ-173-002 fail for the wrong reason).
pub fn genesis_registration_data() -> RegistrationData {
    let bls = crypto::BlsKeyPair::from_seed(&[9u8; 32]).expect("BLS seed is valid");
    let pop = bls.proof_of_possession().expect("PoP signing cannot fail");
    RegistrationData {
        public_key: *kp_a().public_key(),
        epoch: 0,
        vdf_output: vec![1, 2, 3],
        vdf_proof: vec![],
        prev_registration_hash: Hash::ZERO,
        sequence_number: 0,
        bond_count: 0,
        bls_pubkey: bls.public_key().as_bytes().to_vec(),
        bls_pop: pop.as_bytes().to_vec(),
    }
}

/// The EXACT genesis `Registration` TRANSACTION built by
/// `assembly.rs:137-143` — `version: 1`, `inputs: vec![]`, `outputs: vec![]`,
/// `extra_data = bincode(RegistrationData)`.
///
/// REQ-173-002 / constraint C3: the new exempt predicate MUST be a strict
/// superset of the frozen three, or genesis and every fresh sync break.
pub fn genesis_registration_tx() -> Transaction {
    Transaction {
        version: 1,
        tx_type: TxType::Registration,
        inputs: vec![],
        outputs: vec![],
        extra_data: bincode::serialize(&genesis_registration_data())
            .expect("RegistrationData serialization cannot fail"),
    }
}

/// A structurally well-formed `extra_data` payload for `t`.
///
/// The goal is that as many types as possible SURVIVE structural validation and
/// actually REACH the fee gate at `utxo.rs:222`. A type that dies structurally
/// tells us nothing about the gate. With these payloads the following types
/// reach the gate: `Exit`, `SlashProducer`, `AddMaintainer`, `RemoveMaintainer`,
/// `ProtocolActivation`, `DelegateBond`, `RevokeDelegation`, and (inside the
/// genesis window) `Registration`.
pub fn payload_for(t: TxType) -> Vec<u8> {
    let pk = *kp_a().public_key();
    let pk2 = *kp_b().public_key();
    match t {
        TxType::Registration => bincode::serialize(&genesis_registration_data()).unwrap(),
        TxType::Exit => bincode::serialize(&ExitData { public_key: pk }).unwrap(),
        TxType::ClaimReward => bincode::serialize(&ClaimData { public_key: pk }).unwrap(),
        TxType::ClaimBond => bincode::serialize(&ClaimBondData { public_key: pk }).unwrap(),
        TxType::SlashProducer => bincode::serialize(&SlashData {
            producer_pubkey: pk,
            evidence: SlashingEvidence::DoubleProduction {
                block_header_1: dummy_header(pk, 5, 1),
                block_header_2: dummy_header(pk, 5, 2),
            },
            // FM-1: this field has ZERO verification readers anywhere in
            // `crates/` or `bins/`. A default (all-zero) signature is therefore
            // exactly as acceptable to the validator as a real one — which is
            // precisely why spec F3 classifies SlashProducer `false`.
            reporter_signature: crypto::Signature::default(),
        })
        .unwrap(),
        TxType::AddBond => bincode::serialize(&AddBondData {
            producer_pubkey: pk,
            bond_count: 1,
        })
        .unwrap(),
        TxType::RequestWithdrawal => bincode::serialize(&WithdrawalRequestData {
            producer_pubkey: pk,
            bond_count: 1,
            destination: crypto::hash::hash(b"inc-i-173-destination"),
        })
        .unwrap(),
        TxType::AddMaintainer | TxType::RemoveMaintainer => maintainer_change_payload(),
        TxType::DelegateBond => DelegateBondData {
            delegator: pk,
            delegate: pk2,
            bond_count: 1,
            signature: crypto::Signature::default(),
        }
        .to_bytes(),
        TxType::RevokeDelegation => RevokeDelegationData {
            delegator: pk,
            delegate: pk2,
            signature: crypto::Signature::default(),
        }
        .to_bytes(),
        TxType::ProtocolActivation => bincode::serialize(&ProtocolActivationData {
            protocol_version: 99,
            activation_epoch: 10_000,
            description: "inc-i-173".to_string(),
            signatures: vec![MaintainerSignature::new(pk, crypto::Signature::default())],
        })
        .unwrap(),
        _ => Vec::new(),
    }
}

/// A structurally valid `MaintainerChangeData` payload — the exact shape the
/// RPC `submitMaintainerChange` path produces.
pub fn maintainer_change_payload() -> Vec<u8> {
    let pk = *kp_a().public_key();
    MaintainerChangeData::new(
        pk,
        vec![MaintainerSignature::new(pk, crypto::Signature::default())],
    )
    .to_bytes()
}

/// A 0-input / 0-output transaction of type `t` with its well-formed payload.
/// This is THE shape INC-I-173 is about.
pub fn zero_flow_tx(t: TxType) -> Transaction {
    Transaction {
        version: 1,
        tx_type: t,
        inputs: vec![],
        outputs: vec![],
        extra_data: payload_for(t),
    }
}

/// A 0-input transaction of type `t` carrying `n` non-zero outputs.
/// REQ-173-004 / constraint C2: the mint guard. Nothing with outputs may ever
/// be fee/balance exempt.
pub fn tx_with_outputs(t: TxType, amounts: &[u64]) -> Transaction {
    let recipient = crypto::hash::hash(b"inc-i-173-recipient");
    Transaction {
        version: 1,
        tx_type: t,
        inputs: vec![],
        outputs: amounts
            .iter()
            .map(|a| Output::normal(*a, recipient))
            .collect(),
        extra_data: payload_for(t),
    }
}
