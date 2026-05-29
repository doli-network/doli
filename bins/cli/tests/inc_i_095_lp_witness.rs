//! INC-I-095 follow-up — `pool remove` must attach a covenant witness to the
//! conditioned LPShare input, or the node rejects the tx with [MPTX007].
//!
//! cmd_pool_remove signed inputs (input.signature) but never called
//! set_covenant_witnesses. LPShare is a conditioned output: the node evaluates its
//! Condition::Signature covenant from `get_covenant_witness(i)` via `Witness::decode`,
//! NOT from `input.signature`. With no witness, the signature set is empty -> covenant
//! fails -> [MPTX007], even when the correct LP UTXO (by pool_id) was selected.
//!
//! This test builds a remove-style transaction the way cmd_pool_remove does and
//! verifies the LP input's covenant actually evaluates true once witnesses are
//! assembled by the shared helper — and fails without them (reproducing MPTX007).

#[path = "../src/pool_tx.rs"]
mod pool_tx;

use crypto::{hash::hash_with_domain, Hash, KeyPair, ADDRESS_DOMAIN};
use doli_core::conditions::{evaluate, EvalContext};
use doli_core::transaction::TxType;
use doli_core::{Input, Output, Transaction, Witness};

/// Input layout mirrors cmd_pool_remove: [pool(0)] [LP share(1)] [DOLI fee(2)].
fn build_remove_tx(owner: Hash, pool_id: Hash) -> (Transaction, Output) {
    let lp_output = Output::lp_share(100, pool_id, owner);
    let tx = Transaction {
        version: 1,
        tx_type: TxType::RemoveLiquidity,
        inputs: vec![
            Input::new(Hash::from_bytes([1u8; 32]), 0), // pool UTXO (signature-exempt, RC-A)
            Input::new(Hash::from_bytes([2u8; 32]), 0), // LP share UTXO (conditioned)
            Input::new(Hash::from_bytes([3u8; 32]), 0), // DOLI fee UTXO (Normal)
        ],
        outputs: vec![Output::normal(50, owner)],
        extra_data: Vec::new(),
    };
    (tx, lp_output)
}

/// The address hash the LPShare Signature covenant expects: domain-tagged hash of the pubkey.
fn owner_hash(kp: &KeyPair) -> Hash {
    hash_with_domain(ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

// OUTPUT CONTRACT: fn sign_with_covenant_witnesses(tx, keypair, conditioned: &[bool])
// O1: every conditioned[i]==true input gets a non-empty covenant witness whose signature
//     satisfies the spent output's Signature covenant under evaluate()
// O2: conditioned[i]==false inputs get an empty covenant witness
// PATHS: P1 LP input (conditioned) -> covenant passes; P2 pool/fee inputs -> empty witness;
//        P3 control: no witnesses assembled -> LP covenant fails (reproduces MPTX007)
// INPUT PARTITIONS: input index × conditioned flag
// MATRIX: 2 outputs x 3 paths

#[test]
fn inc_i_095_lp_input_covenant_satisfied_after_witness_assembly() {
    let kp = KeyPair::generate();
    let owner = owner_hash(&kp);
    let pool_id = Hash::from_bytes([9u8; 32]);
    let (mut tx, lp_output) = build_remove_tx(owner, pool_id);

    // [pool(0)=not conditioned, LP(1)=conditioned, fee(2)=not conditioned]
    let conditioned = [false, true, false];
    pool_tx::sign_with_covenant_witnesses(&mut tx, &kp, &conditioned);

    // P1 — LP input witness present and satisfies the covenant the node evaluates.
    let lp_witness_bytes = tx
        .get_covenant_witness(1)
        .expect("LP input must have a witness slot");
    assert!(
        !lp_witness_bytes.is_empty(),
        "LP input covenant witness must not be empty (INC-I-095)"
    );

    let cond = lp_output
        .condition()
        .expect("LPShare is conditioned")
        .expect("condition decodes");
    let witness = Witness::decode(lp_witness_bytes).expect("witness decodes");
    let signing_hash = tx.signing_message_for_input(1);
    let ctx = EvalContext {
        current_height: 0,
        signing_hash: &signing_hash,
        transaction: Some(&tx),
    };
    let mut or_idx = 0usize;
    assert!(
        evaluate(&cond, &witness, &ctx, &mut or_idx),
        "LP input covenant must be satisfied by the assembled witness — otherwise the node returns [MPTX007]"
    );

    // P2 — non-conditioned inputs (pool, fee) get empty witnesses.
    assert!(
        tx.get_covenant_witness(0).unwrap_or(&[]).is_empty(),
        "pool input (index 0) witness must be empty"
    );
    assert!(
        tx.get_covenant_witness(2).unwrap_or(&[]).is_empty(),
        "DOLI fee input (index 2) witness must be empty"
    );
}

// P3 — control: WITHOUT witness assembly the LP covenant fails. This is the bug.
#[test]
fn inc_i_095_lp_covenant_fails_without_witnesses() {
    let kp = KeyPair::generate();
    let owner = owner_hash(&kp);
    let pool_id = Hash::from_bytes([9u8; 32]);
    let (tx, lp_output) = build_remove_tx(owner, pool_id);
    // No sign_with_covenant_witnesses call -> empty/absent witness for the LP input.

    let cond = lp_output.condition().unwrap().unwrap();
    let witness = Witness::decode(tx.get_covenant_witness(1).unwrap_or(&[])).unwrap();
    let signing_hash = tx.signing_message_for_input(1);
    let ctx = EvalContext {
        current_height: 0,
        signing_hash: &signing_hash,
        transaction: Some(&tx),
    };
    let mut or_idx = 0usize;
    assert!(
        !evaluate(&cond, &witness, &ctx, &mut or_idx),
        "without witnesses the LP covenant must FAIL — this is the [MPTX007] the bug produced"
    );
}
