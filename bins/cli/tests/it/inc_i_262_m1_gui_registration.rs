//! INC-I-262 M1 — the GUI Registration is built by one pure function: the
//! pre-fix GUI construction with only the 64-byte producer-auth tail appended,
//! valid with the rule armed and without it.
//!
// covers: producer_tx producer
//
// OUTPUT CONTRACT: fn doli_cli::producer_tx::build_signed_registration(&RegistrationSpec, &KeyPair, &Hash) -> Result<Transaction, String>
//   O1 return Ok: version, tx_type, inputs (signed per input, public_key set), outputs = the pre-fix GUI construction;
//      extra_data = bincode(reg_data) | sign_producer_auth tail (64 bytes)
//   O2 return Err: only when the payload cannot be encoded (unreachable for a well-formed RegistrationData)
// PATHS: P-CHANGE change > 0 | P-EXACT change == 0
// MATRIX
//   O1 x {P-CHANGE, P-EXACT} x {gate 0, gate off} -> inc_i_262_m1_req_def_001_d2_gui_registration_is_prefix_plus_tail
// INPUT PARTITIONS: 2 inputs, 2 bonds, change 5_000 | 1 input, 1 bond, change 0; devnet genesis from chain-info hex

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_cli::producer_tx::{build_signed_registration, chain_genesis_hash, RegistrationSpec};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::{
    sign_producer_auth, Input, Output, RegistrationData, Transaction, TxType,
};
use doli_core::validation::{validate_transaction, ValidationContext, ValidationError};
use doli_core::Network;

const BEST: u64 = 1_000;

fn wallet() -> KeyPair {
    KeyPair::from_seed([0x64; 32])
}

fn ctx(gate: u64) -> ValidationContext {
    let p = doli_core::NetworkParams::defaults(Network::Devnet);
    ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, BEST + 1)
        .with_inc_i_234_activation_height(0)
        .with_inc_i_257_unbonding(0, p.unbonding_period)
        .with_inc_i_182_producer_auth(gate)
}

fn spec(inputs: Vec<(Hash, u32)>, bonds: u32, change: u64) -> RegistrationSpec {
    let pk = *wallet().public_key();
    let bls = BlsKeyPair::from_seed(&[0x65; 32]).expect("FIXTURE: BLS");
    RegistrationSpec {
        inputs,
        reg_data: RegistrationData {
            public_key: pk,
            epoch: 0,
            vdf_output: vec![0x11; 32],
            vdf_proof: vec![],
            prev_registration_hash: Hash::ZERO,
            sequence_number: 0,
            bond_count: bonds,
            bls_pubkey: bls.public_key().as_bytes().to_vec(),
            bls_pop: bls.proof_of_possession().expect("PoP").as_bytes().to_vec(),
        },
        bond_unit: Network::Devnet.bond_unit(),
        lock_until: BEST + 576 + 1000,
        change,
        change_to: crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes()),
    }
}

/// `register_producer` before the fix, from the same derived fields.
fn gui_before(s: &RegistrationSpec, kp: &KeyPair) -> Transaction {
    let pk = s.reg_data.public_key;
    let extra_data = bincode::serialize(&s.reg_data).expect("encode");
    let owner = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
    let outputs = (0..s.reg_data.bond_count)
        .map(|_| Output::bond(s.bond_unit, owner, s.lock_until, 0))
        .collect();
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Registration,
        inputs: s.inputs.iter().map(|(h, i)| Input::new(*h, *i)).collect(),
        outputs,
        extra_data,
    };
    if s.change > 0 {
        tx.outputs.push(Output::normal(s.change, s.change_to));
    }
    for i in 0..tx.inputs.len() {
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, kp.private_key());
        tx.inputs[i].public_key = Some(*kp.public_key());
    }
    tx
}

fn coded(r: &Result<(), ValidationError>) -> bool {
    matches!(r, Err(ValidationError::InvalidTransaction(m)) if m.contains("[ERRTX-PAUTH001]"))
}

// REQ-DEF-001 — Decision: a failure means signing the GUI Registration changed its payload, inputs or outputs (D2), or the signed form is refused with or without the rule.
#[test]
fn inc_i_262_m1_req_def_001_d2_gui_registration_is_prefix_plus_tail() {
    let genesis =
        chain_genesis_hash(&ConsensusParams::devnet().genesis_hash.to_hex()).expect("genesis");
    let cases = [
        (
            "change",
            spec(
                vec![
                    (Hash::from_bytes([0xA1; 32]), 0),
                    (Hash::from_bytes([0xA2; 32]), 3),
                ],
                2,
                5_000,
            ),
        ),
        ("exact", spec(vec![(Hash::from_bytes([0xA3; 32]), 1)], 1, 0)),
    ];
    for (what, s) in cases {
        let before = gui_before(&s, &wallet());
        let fixed = build_signed_registration(&s, &wallet(), &genesis).expect("builds");
        assert_eq!(
            (fixed.version, fixed.tx_type, &fixed.inputs, &fixed.outputs),
            (
                before.version,
                before.tx_type,
                &before.inputs,
                &before.outputs
            ),
            "{what}: inputs and outputs equal the pre-fix GUI tx"
        );
        assert_eq!(
            fixed.extra_data.len(),
            before.extra_data.len() + 64,
            "{what}: only the tail is added"
        );
        let mut want = before.clone();
        sign_producer_auth(&mut want, &genesis, &wallet());
        assert_eq!(fixed, want, "{what}: pre-fix tx + sign_producer_auth");
        for gate in [0, u64::MAX] {
            let r = validate_transaction(&fixed, &ctx(gate));
            assert!(r.is_ok(), "{what}, gate {gate}: {r:?}");
        }
        assert!(
            coded(&validate_transaction(&before, &ctx(0))),
            "{what}: NON-VACUITY: the pre-fix tx is refused at the gate"
        );
    }
}
