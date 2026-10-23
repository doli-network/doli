//! INC-I-182 M1 core fixtures: a production-shaped devnet context with the
//! producer-auth gate, the three producer txs in a shape stateless validation
//! accepts, and the relayer and forger variants of each.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: producer_auth transaction types unbonding defaults env_loader network_params core data block

#![allow(dead_code)]

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{
    sign_producer_auth, AddBondData, Input, OutputType, RegistrationData, Transaction, TxType,
    WithdrawalRequestData,
};
use doli_core::validation::{ValidationContext, ValidationError};
use doli_core::Network;

pub const CODE: &str = "[ERRTX-PAUTH001]";
pub const GATE: u64 = 1_000;
pub const BONDS: u32 = 2;
pub const NET: u64 = 1_999_000;
pub const LOCK: u64 = GATE + 100_000;
pub const BOND_LOCK: u64 = GATE + 100_000;
pub const KINDS: [TxType; 3] = [
    TxType::Registration,
    TxType::AddBond,
    TxType::RequestWithdrawal,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Unsigned,
    ZeroSig,
    WrongKey,
    Replayed,
    OtherGenesis,
    Tail63,
    Tail65,
    BondCountRewrite,
    BlsRewrite,
}

pub const REFUSED: [Shape; 8] = [
    Shape::Unsigned,
    Shape::ZeroSig,
    Shape::WrongKey,
    Shape::Replayed,
    Shape::OtherGenesis,
    Shape::Tail63,
    Shape::Tail65,
    Shape::BondCountRewrite,
];

pub fn producer() -> KeyPair {
    KeyPair::from_seed([0x82; 32])
}

pub fn payer() -> KeyPair {
    KeyPair::from_seed([0x83; 32])
}

pub fn stranger() -> KeyPair {
    KeyPair::from_seed([0x18; 32])
}

pub fn pk() -> PublicKey {
    *producer().public_key()
}

pub fn bls(seed: u8) -> BlsKeyPair {
    BlsKeyPair::from_seed(&[seed; 32]).expect("FIXTURE: BLS seed")
}

pub fn genesis() -> Hash {
    ConsensusParams::devnet().genesis_hash
}

pub fn other_genesis() -> Hash {
    let g = ConsensusParams::testnet().genesis_hash;
    assert_ne!(g, genesis(), "FIXTURE: two networks share a genesis hash");
    g
}

pub fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

/// Every devnet gate the node threads, plus the producer-auth gate.
pub fn ctx(height: u64, gate: u64) -> ValidationContext {
    let n = Network::Devnet.params();
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
        .with_sig_verification_height(n.sig_verification_height)
        .with_encrypted_content_activation_height(n.encrypted_content_activation_height)
        .with_encrypted_content_v2_activation_height(n.encrypted_content_v2_activation_height)
        .with_security_audit_activation_height(n.security_audit_activation_height)
        .with_defi_activation_height(n.defi_activation_height)
        .with_amm_activation_height(n.amm_activation_height)
        .with_inc_i_092_activation_height(n.inc_i_092_activation_height)
        .with_inc_i_096_activation_height(n.inc_i_096_activation_height)
        .with_inc_i_173_activation_height(n.inc_i_173_activation_height)
        .with_bls_key_rotation_activation_height(n.bls_key_rotation_activation_height)
        .with_inc_i_234_activation_height(n.inc_i_234_activation_height)
        .with_inc_i_255_slashing_activation_height(n.effective_slashing_activation_height())
        .with_inc_i_257_unbonding(n.inc_i_257_unbonding_activation_height, n.unbonding_period)
        .with_inc_i_182_producer_auth(gate)
}

fn inputs(tag: u8) -> Vec<Input> {
    let mut input = Input::new(Hash::from_bytes([tag; 32]), 0);
    input.public_key = Some(*payer().public_key());
    vec![input]
}

fn sign_inputs(mut tx: Transaction) -> Transaction {
    for i in 0..tx.inputs.len() {
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, payer().private_key());
    }
    tx
}

pub fn registration_data(bond_count: u32, key: &BlsKeyPair) -> RegistrationData {
    RegistrationData {
        public_key: pk(),
        epoch: 0,
        vdf_output: vec![0x11; 32],
        vdf_proof: Vec::new(),
        prev_registration_hash: Hash::ZERO,
        sequence_number: 0,
        bond_count,
        bls_pubkey: key.public_key().as_bytes().to_vec(),
        bls_pop: key
            .proof_of_possession()
            .expect("FIXTURE: PoP")
            .as_bytes()
            .to_vec(),
    }
}

/// Payload bytes of `kind` with `bond_count`; Registration carries BLS key `bls_seed`.
pub fn payload(kind: TxType, bond_count: u32, bls_seed: u8) -> Vec<u8> {
    match kind {
        TxType::Registration => bincode::serialize(&registration_data(bond_count, &bls(bls_seed)))
            .expect("FIXTURE: encode registration"),
        TxType::AddBond => AddBondData::new(pk(), bond_count).to_bytes(),
        TxType::RequestWithdrawal => {
            WithdrawalRequestData::new(pk(), bond_count, addr(&pk())).to_bytes()
        }
        other => panic!("FIXTURE: {other:?} is not a producer-auth type"),
    }
}

/// Unsigned tx of `kind` paid by `payer()` from outpoint `[tag; 32]:0`.
pub fn unsigned(kind: TxType, tag: u8) -> Transaction {
    let unit = Network::Devnet.bond_unit();
    let p = pk();
    let mut tx = match kind {
        TxType::Registration => {
            Transaction::new_registration(inputs(tag), p, unit * BONDS as u64, BOND_LOCK, BONDS)
        }
        TxType::AddBond => {
            Transaction::new_add_bond(inputs(tag), p, BONDS, unit * BONDS as u64, BOND_LOCK)
        }
        TxType::RequestWithdrawal => {
            let mut t = Transaction::new_request_withdrawal(inputs(tag), p, BONDS, addr(&p), NET);
            t.outputs[0].output_type = OutputType::Unbonding;
            t.outputs[0].lock_until = LOCK;
            t
        }
        other => panic!("FIXTURE: {other:?} is not a producer-auth type"),
    };
    tx.extra_data = payload(kind, BONDS, 0x44);
    sign_inputs(tx)
}

pub fn signed_by(kind: TxType, tag: u8, signer: &KeyPair, g: &Hash) -> Transaction {
    let mut tx = unsigned(kind, tag);
    sign_producer_auth(&mut tx, g, signer);
    tx
}

pub fn signed(kind: TxType, tag: u8) -> Transaction {
    signed_by(kind, tag, &producer(), &genesis())
}

fn tail(tx: &Transaction) -> Vec<u8> {
    tx.extra_data[tx.extra_data.len() - 64..].to_vec()
}

fn with_payload(tx: &Transaction, payload: Vec<u8>) -> Transaction {
    let mut t = tx.clone();
    t.extra_data = payload;
    t.extra_data.extend_from_slice(&tail(tx));
    t
}

/// `kind` paid from `[tag; 32]:0` in the given shape.
pub fn shaped(kind: TxType, tag: u8, shape: Shape) -> Transaction {
    let good = signed(kind, tag);
    match shape {
        Shape::Unsigned => unsigned(kind, tag),
        Shape::ZeroSig => {
            let mut t = unsigned(kind, tag);
            t.extra_data.extend_from_slice(&[0u8; 64]);
            t
        }
        Shape::WrongKey => signed_by(kind, tag, &stranger(), &genesis()),
        Shape::Replayed => {
            let mut t = unsigned(kind, tag);
            t.extra_data
                .extend_from_slice(&tail(&signed(kind, tag.wrapping_add(1))));
            t
        }
        Shape::OtherGenesis => signed_by(kind, tag, &producer(), &other_genesis()),
        Shape::Tail63 => {
            let mut t = good;
            t.extra_data.pop();
            t
        }
        Shape::Tail65 => {
            let mut t = good;
            t.extra_data.push(0);
            t
        }
        Shape::BondCountRewrite => with_payload(&good, payload(kind, BONDS + 1, 0x44)),
        Shape::BlsRewrite => with_payload(&good, payload(kind, BONDS, 0x55)),
    }
}

pub fn coded(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(ValidationError::InvalidTransaction(m)) if m.contains(CODE))
}
