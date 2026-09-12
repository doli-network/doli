//! INC-I-217 M4 — the rotation proof-of-possession under its own DST.
//!
//! covers: REQ-ROT-SEC-002, REQ-ROT-SEC-009
//!
//! Authority: `specs/bls-key-rotation-architecture.md` "Preimage byte layouts".
//! The message is `genesis_hash ‖ producer_ed25519[32] ‖ new_bls_pubkey[48]`
//! signed under `ROTATE_POP_DST`. The requirements doc's ordering
//! (`producer ‖ new_key ‖ genesis`) is STALE and is not asserted here.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1: `sign_rotation_pop(sk, pk, genesis, producer) -> Result<BlsSignature, BlsError>`
//!       O1 return: `Ok(sig)` — the only output
//!       O2 mutable params / O3 receiver: NONE (all shared references)
//!       O4 store / O5 statics / O6 channels: NONE
//!       PATHS: Ok for any keypair `BlsKeyPair::from_seed` can build; the Err
//!         arm is unreachable from a validated keypair and is not asserted
//!   F2: `verify_rotation_pop(pk, genesis, producer, pop) -> Result<(), BlsError>`
//!       O1 return: `Ok(())` or `Err(BlsError)`; no other output
//!       PATHS: Ok on a matching triple; Err on any mismatch or wrong DST
//!
//! INPUT PARTITIONS:
//!   I1  the matching (pk, genesis, producer, pop) quadruple — accept
//!   I2  a registration PoP (`bls_sign_pop`, POP_DST) offered as a rotation PoP
//!   I3  a rotation PoP offered to `bls_verify_pop` (the reverse direction)
//!   I4  genesis flipped after signing
//!   I5  producer_ed25519 flipped after signing
//!   I6  a different BLS public key
//!   I7  an all-zero / garbage signature
//!   I8  the (genesis, producer) pair swapped — concatenation order sensitivity
//!   I9  genesis of a different LENGTH — no length prefix means no collision
//!   I10 the same call twice — determinism
//!
//!   MATRIX 2 functions x 10 partitions: every return value is claimed by a
//!   named test below; the absent output categories are structural and are
//!   asserted once, in the I1 test, by re-reading the inputs after the call.

use crypto::bls::BlsPublicKeyWrapped;
use crypto::{
    bls_sign_pop, bls_verify_pop, sign_rotation_pop, verify_rotation_pop, BlsKeyPair, BlsSignature,
};

const GENESIS_A: [u8; 32] = [0x11; 32];
const GENESIS_B: [u8; 32] = [0x99; 32];
const PRODUCER_A: [u8; 32] = [0x44; 32];
const PRODUCER_B: [u8; 32] = [0x55; 32];

fn keypair(seed: u8) -> BlsKeyPair {
    BlsKeyPair::from_seed(&[seed; 32]).expect("seeded BLS keypair")
}

fn rotation_pop(kp: &BlsKeyPair, genesis: &[u8], producer: &[u8; 32]) -> BlsSignature {
    sign_rotation_pop(kp.secret_key(), kp.public_key(), genesis, producer)
        .expect("rotation PoP signs")
}

fn pk_of(kp: &BlsKeyPair) -> &BlsPublicKeyWrapped {
    kp.public_key()
}

// REQ-ROT-SEC-002 — Decision: whether the signer and the verifier still agree on the message and DST, without which no producer could ever rotate.
#[test]
fn req_rot_sec_002_a_rotation_pop_verifies_under_its_own_dst() {
    let kp = keypair(1);
    let pop = rotation_pop(&kp, &GENESIS_A, &PRODUCER_A);

    assert!(
        verify_rotation_pop(pk_of(&kp), &GENESIS_A, &PRODUCER_A, &pop).is_ok(),
        "the matching quadruple must verify"
    );

    // I10 determinism, and O2/O3: nothing observable was mutated by the call.
    let again = rotation_pop(&kp, &GENESIS_A, &PRODUCER_A);
    assert_eq!(pop.as_bytes(), again.as_bytes());
    assert_eq!(GENESIS_A, [0x11; 32]);
    assert_eq!(PRODUCER_A, [0x44; 32]);
}

// REQ-ROT-SEC-002 — Decision: whether a registration PoP already published on chain can be lifted into a rotation, which is exactly the C1 cross-producer replay.
#[test]
fn req_rot_sec_002_a_registration_pop_is_not_a_valid_rotation_pop() {
    let kp = keypair(2);
    let registration = bls_sign_pop(kp.secret_key(), kp.public_key()).expect("registration PoP");

    assert!(
        verify_rotation_pop(pk_of(&kp), &GENESIS_A, &PRODUCER_A, &registration).is_err(),
        "POP_DST and ROTATE_POP_DST must not accept each other's signatures"
    );

    // Non-vacuity: the same registration PoP IS valid in its own domain.
    assert!(bls_verify_pop(pk_of(&kp), &registration).is_ok());
}

// REQ-ROT-SEC-002 — Decision: whether a rotation PoP could be replayed as a registration PoP, letting a rotation payload double as a fresh producer registration.
#[test]
fn req_rot_sec_002_a_rotation_pop_is_not_a_valid_registration_pop() {
    let kp = keypair(3);
    let rot = rotation_pop(&kp, &GENESIS_A, &PRODUCER_A);

    assert!(
        bls_verify_pop(pk_of(&kp), &rot).is_err(),
        "a rotation PoP must not satisfy the registration PoP check"
    );

    // Non-vacuity: the same key CAN produce an accepted registration PoP.
    let registration = bls_sign_pop(kp.secret_key(), kp.public_key()).expect("registration PoP");
    assert!(bls_verify_pop(pk_of(&kp), &registration).is_ok());
}

// REQ-ROT-SEC-002 — Decision: whether the PoP binds the producer identity, without which one producer's PoP authorises another producer's rotation.
#[test]
fn req_rot_sec_002_the_pop_binds_the_producer_ed25519_key() {
    let kp = keypair(4);
    let pop = rotation_pop(&kp, &GENESIS_A, &PRODUCER_A);

    assert!(
        verify_rotation_pop(pk_of(&kp), &GENESIS_A, &PRODUCER_B, &pop).is_err(),
        "a PoP made for producer A must not verify for producer B"
    );
    assert!(verify_rotation_pop(pk_of(&kp), &GENESIS_A, &PRODUCER_A, &pop).is_ok());
}

// REQ-ROT-SEC-002 — Decision: whether the PoP binds the key it claims possession of, without which it proves possession of nothing.
#[test]
fn req_rot_sec_002_the_pop_binds_the_bls_public_key() {
    let kp = keypair(5);
    let other = keypair(6);
    let pop = rotation_pop(&kp, &GENESIS_A, &PRODUCER_A);

    assert_ne!(pk_of(&kp).as_bytes(), pk_of(&other).as_bytes());
    assert!(
        verify_rotation_pop(pk_of(&other), &GENESIS_A, &PRODUCER_A, &pop).is_err(),
        "a PoP must not verify against a public key it does not belong to"
    );

    // A PoP signed for one key but claiming another must also fail.
    let mismatched = sign_rotation_pop(kp.secret_key(), pk_of(&other), &GENESIS_A, &PRODUCER_A)
        .expect("signing with a mismatched claimed key still produces bytes");
    assert!(verify_rotation_pop(pk_of(&other), &GENESIS_A, &PRODUCER_A, &mismatched).is_err());
}

// REQ-ROT-SEC-002 — Decision: whether a structurally invalid or all-zero signature is accepted, which would make the PoP check a formality.
#[test]
fn req_rot_sec_002_garbage_signatures_are_rejected() {
    let kp = keypair(7);

    let zero = BlsSignature::from_bytes_unchecked([0u8; 96]);
    assert!(verify_rotation_pop(pk_of(&kp), &GENESIS_A, &PRODUCER_A, &zero).is_err());

    let mut tampered = *rotation_pop(&kp, &GENESIS_A, &PRODUCER_A).as_bytes();
    tampered[95] ^= 0x01;
    let tampered = BlsSignature::from_bytes_unchecked(tampered);
    assert!(verify_rotation_pop(pk_of(&kp), &GENESIS_A, &PRODUCER_A, &tampered).is_err());
}

// REQ-ROT-SEC-002 — Decision: whether the three message fields are concatenated in a fixed order, without which a (genesis, producer) pair aliases its own transpose.
#[test]
fn req_rot_sec_002_the_message_concatenation_is_order_sensitive() {
    let kp = keypair(8);

    let straight = rotation_pop(&kp, &PRODUCER_A, &GENESIS_A);
    let swapped = rotation_pop(&kp, &GENESIS_A, &PRODUCER_A);
    assert_ne!(
        straight.as_bytes(),
        swapped.as_bytes(),
        "swapping genesis and producer must change the signed message"
    );
    assert!(verify_rotation_pop(pk_of(&kp), &GENESIS_A, &PRODUCER_A, &straight).is_err());

    // A genesis of a different LENGTH must not alias: there is no length prefix,
    // but the 32-byte producer and 48-byte key tail keep the parse unambiguous.
    let short = rotation_pop(&kp, &GENESIS_A[..4], &PRODUCER_A);
    assert_ne!(short.as_bytes(), swapped.as_bytes());
    assert!(verify_rotation_pop(pk_of(&kp), &GENESIS_A[..4], &PRODUCER_A, &short).is_ok());
}

// REQ-ROT-SEC-009 — Decision: whether a PoP produced on testnet can be replayed into a mainnet rotation payload unchanged.
#[test]
fn req_rot_sec_009_the_pop_is_bound_to_the_genesis_hash() {
    let kp = keypair(9);
    let pop = rotation_pop(&kp, &GENESIS_A, &PRODUCER_A);

    assert!(
        verify_rotation_pop(pk_of(&kp), &GENESIS_B, &PRODUCER_A, &pop).is_err(),
        "a PoP made under genesis A must not verify under genesis B"
    );
    assert!(verify_rotation_pop(pk_of(&kp), &GENESIS_A, &PRODUCER_A, &pop).is_ok());

    let pop_b = rotation_pop(&kp, &GENESIS_B, &PRODUCER_A);
    assert_ne!(pop.as_bytes(), pop_b.as_bytes());
}
