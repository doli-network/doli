//! INC-I-182 M1 — the producer-auth rule on the stateless core paths:
//! Registration, AddBond and RequestWithdrawal need the producer signature at
//! and above the gate; every forged or rewritten form is refused with one code.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn check_producer_auth(&Transaction, &ValidationContext) -> Result<(), ValidationError>
// OUTPUT CONTRACT: fn validate_transaction(&Transaction, &ValidationContext) -> Result<(), ValidationError>
// OUTPUT CONTRACT: fn validate_transaction_skip_registration_vdf(&Transaction, &ValidationContext) -> Result<(), ValidationError>
//   O1 return: Ok | Err(InvalidTransaction) whose text holds [ERRTX-PAUTH001]; pure, no store writes
// OUTPUT CONTRACT: fn producer_auth_digest(&Hash, &Transaction, &[u8]) -> Hash
//   O2 return: BLAKE3(domain | genesis | signing_message | payload)
// OUTPUT CONTRACT: fn sign_producer_auth(&mut Transaction, &Hash, &KeyPair)
//   O3 receiver: extra_data = payload | 64-byte signature; inputs, outputs, signing_message unchanged
// PATHS: P-AT height = gate | P-ABOVE gate + 1, gate + 10_000 | P-BELOW gate - 1 | P-OFF gate u64::MAX (control)
// MATRIX
//   O1 x {Registration, AddBond, RequestWithdrawal} x signed x {P-AT, P-ABOVE} -> Ok (inc_i_182_m1_req_pauth_002_signed_accepted_at_and_above_gate)
//   O1 x 3 kinds x {unsigned, zero sig, wrong key, replayed, other genesis, tail 63, tail 65,
//      bond_count rewrite, BLS rewrite} x P-AT -> coded on all three functions; P-OFF -> Ok
//      (inc_i_182_m1_req_pauth_002_refused_shapes_carry_one_code)
//   O1 x 3 kinds x every shape x P-BELOW -> Ok (inc_i_182_m1_req_pauth_003_below_gate_no_check)
//   O1 x Registration x Full-mode arm -> inc_i_182_m1_req_pauth_002_registration_full_mode_arm
//   O2, O3 x 3 kinds -> inc_i_182_m1_req_pauth_002_digest_preimage_and_signature_pinned
// INPUT PARTITIONS:
//   signer: producer | stranger | none | zero bytes; genesis: devnet | testnet
//   tail length: 0 | 63 | 64 | 65; payload: as signed | bond_count + 1 | other BLS key and PoP
//   fee payer: a third party (the producer signs only the auth tail)

use crypto::Hash;
use doli_core::transaction::{
    producer_auth_digest, Transaction, TxType, PRODUCER_AUTH_DOMAIN, PRODUCER_AUTH_SIG_LEN,
};
use doli_core::validation::producer_auth::check_producer_auth;
use doli_core::validation::{
    validate_transaction, validate_transaction_skip_registration_vdf, ValidationContext,
    ValidationError,
};

use super::inc_i_182_m1_core_fx::*;

const TAG: u8 = 0x30;

fn shapes_for(kind: TxType) -> Vec<Shape> {
    let mut s = REFUSED.to_vec();
    if kind == TxType::Registration {
        s.push(Shape::BlsRewrite);
    }
    s
}

type Check = fn(&Transaction, &ValidationContext) -> Result<(), ValidationError>;

const CHECKS: [(&str, Check); 3] = [
    ("check_producer_auth", check_producer_auth),
    ("validate_transaction", validate_transaction),
    (
        "validate_transaction_skip_registration_vdf",
        validate_transaction_skip_registration_vdf,
    ),
];

// REQ-PAUTH-002 — Decision: a failure means the producer's own signed Registration, AddBond or RequestWithdrawal is refused at or above the gate, so producers cannot register, bond or withdraw at all.
#[test]
fn inc_i_182_m1_req_pauth_002_signed_accepted_at_and_above_gate() {
    let mut bad = Vec::new();
    for kind in KINDS {
        let tx = signed(kind, TAG);
        for h in [GATE, GATE + 1, GATE + 10_000] {
            for (name, f) in CHECKS {
                let r = f(&tx, &ctx(h, GATE));
                if r.is_err() {
                    bad.push(format!("{kind:?} at {h}, {name}: {r:?}"));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "signed producer txs refused:\n{}",
        bad.join("\n")
    );
}

// REQ-PAUTH-002 — Decision: a failure names a forged or rewritten producer tx that one stateless path accepts or refuses under another code, so a third party can still force an exit or bond change, or paths disagree.
#[test]
fn inc_i_182_m1_req_pauth_002_refused_shapes_carry_one_code() {
    let mut bad = Vec::new();
    for kind in KINDS {
        for shape in shapes_for(kind) {
            let tx = shaped(kind, TAG, shape);
            let off = validate_transaction(&tx, &ctx(GATE, u64::MAX));
            assert!(
                off.is_ok(),
                "NON-VACUITY: {kind:?} {shape:?} must be valid in every other respect (gate off); got {off:?}"
            );
            for (name, f) in CHECKS {
                let r = f(&tx, &ctx(GATE, GATE));
                if !coded(&r) {
                    bad.push(format!("{kind:?} {shape:?}, {name}: {r:?}"));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "at the gate each shape must be refused with {CODE}:\n{}",
        bad.join("\n")
    );
}

// REQ-PAUTH-003 — Decision: a failure means the rule reaches below its activation height, so blocks that are valid today become invalid (a retroactive consensus change).
#[test]
fn inc_i_182_m1_req_pauth_003_below_gate_no_check() {
    let mut bad = Vec::new();
    for kind in KINDS {
        for shape in shapes_for(kind) {
            let tx = shaped(kind, TAG, shape);
            for (name, f) in CHECKS {
                let r = f(&tx, &ctx(GATE - 1, GATE));
                if r.is_err() {
                    bad.push(format!("{kind:?} {shape:?}, {name}: {r:?}"));
                }
            }
        }
        let signed_below = validate_transaction(&signed(kind, TAG), &ctx(GATE - 1, GATE));
        if signed_below.is_err() {
            bad.push(format!("{kind:?} signed: {signed_below:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "below the gate no producer-auth check may run:\n{}",
        bad.join("\n")
    );
}

// REQ-PAUTH-002 — Decision: a failure means the block validator's Registration arm (used when registration VDFs are pre-verified, and by the builder and apply through UTXO validation) skips the rule, so a relayer can swap the BLS key in a mined registration.
#[test]
fn inc_i_182_m1_req_pauth_002_registration_full_mode_arm() {
    let kind = TxType::Registration;
    let ok = validate_transaction_skip_registration_vdf(&signed(kind, TAG), &ctx(GATE, GATE));
    assert!(
        ok.is_ok(),
        "signed registration on the Full-mode arm: {ok:?}"
    );
    for shape in [Shape::Unsigned, Shape::BlsRewrite, Shape::BondCountRewrite] {
        let tx = shaped(kind, TAG, shape);
        let below = validate_transaction_skip_registration_vdf(&tx, &ctx(GATE - 1, GATE));
        assert!(
            below.is_ok(),
            "NON-VACUITY: {shape:?} below the gate: {below:?}"
        );
        let r = validate_transaction_skip_registration_vdf(&tx, &ctx(GATE, GATE));
        assert!(
            coded(&r),
            "{shape:?} on the Full-mode arm at the gate: {r:?}"
        );
    }
}

fn reference_digest(g: &Hash, tx: &Transaction, payload: &[u8]) -> Hash {
    let mut h = crypto::Hasher::new();
    h.update(b"DOLI-PRODUCER-AUTH-V1");
    h.update(g.as_bytes());
    h.update(tx.signing_message().as_bytes());
    h.update(payload);
    h.finalize()
}

// REQ-PAUTH-002 — Decision: a failure means the node's digest or signature placement differs from the documented preimage, so a signer built from the spec (CLI, external wallet) makes signatures consensus refuses, or signing breaks the fee payer's input signatures.
#[test]
fn inc_i_182_m1_req_pauth_002_digest_preimage_and_signature_pinned() {
    assert_eq!(PRODUCER_AUTH_DOMAIN, b"DOLI-PRODUCER-AUTH-V1");
    assert_eq!(PRODUCER_AUTH_SIG_LEN, 64);
    for kind in KINDS {
        let before = unsigned(kind, TAG);
        let payload = before.extra_data.clone();
        let want = reference_digest(&genesis(), &before, &payload);
        assert_eq!(
            producer_auth_digest(&genesis(), &before, &payload),
            want,
            "{kind:?}: digest preimage"
        );
        let after = signed(kind, TAG);
        assert_eq!(
            (&after.inputs, &after.outputs, after.signing_message()),
            (&before.inputs, &before.outputs, before.signing_message()),
            "{kind:?}: signing may only append to extra_data"
        );
        assert_eq!(
            &after.extra_data[..payload.len()],
            &payload[..],
            "{kind:?}: payload moved"
        );
        let sig = crypto::signature::sign_hash(&want, producer().private_key());
        assert_eq!(
            &after.extra_data[payload.len()..],
            &sig.as_bytes()[..],
            "{kind:?}: tail must be the Ed25519 signature of the digest"
        );
        for i in 0..after.inputs.len() {
            let ok = crypto::signature::verify_hash(
                &after.signing_message_for_input(i),
                &after.inputs[i].signature,
                payer().public_key(),
            );
            assert!(
                ok.is_ok(),
                "{kind:?}: input {i} signature broken by signing"
            );
        }
    }
}
