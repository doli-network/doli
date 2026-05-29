// INC-I-093 — Bridge HTLC claim/refund witness correctness at consensus.
//
// The stress test reported [MPTX007] on bridge claim AND refund. This test
// drives the REAL consensus evaluator (validate_transaction_with_utxos) against
// a BridgeHTLC UTXO whose covenant is Output::bridge_htlc(...) =
//   htlc_signed_refund(hash, lock, expiry, refund_pkh) =
//     Or( And(Hashlock(hash), Timelock(lock)),               // claim (left)
//         And(Sig(refund_pkh), TimelockExpiry(expiry)) )      // refund (right)
//
// Findings this test locks in:
//   * CLAIM is correct: witness branch(left)+preimage(P) passes iff the preimage
//     hashes to `hash` AND current_height >= lock. The stress-test MPTX007 on
//     claim was a usage artifact (claim before lock, or wrong preimage).
//   * REFUND requires a SIGNATURE in the covenant witness (AUDIT-BRIDGE-001:
//     signed refund). The CLI's old `branch(right)+none()` witness carries NO
//     signature -> the Sig(refund_pkh) sub-condition is unsatisfied -> refund is
//     rejected even after expiry. That is a real witness bug, not a usage
//     artifact. The correct refund witness is branch(right) + the refunder's
//     signature, and it passes iff current_height >= expiry.
//
// OUTPUT CONTRACT: fn validate_transaction_with_utxos(tx, ctx, utxo_provider)
//                  for a Transfer spending a BridgeHTLC (htlc_signed_refund) UTXO.
//   Outputs:
//     O1: returned Result<(), ValidationError>.
//   PATHS:
//     P1: claim branch (left) = And(Hashlock, Timelock).
//     P2: refund branch (right) = And(Sig, TimelockExpiry).
//   INPUT PARTITIONS:
//     IP1: claim, correct preimage, height >= lock        -> Ok.
//     IP2: claim, correct preimage, height <  lock        -> Err (timelock unmet).
//     IP3: claim, wrong preimage, height >= lock          -> Err (hashlock unmet).
//     IP4: refund, valid refunder sig, height >= expiry   -> Ok.
//     IP5: refund, valid refunder sig, height <  expiry   -> Err (expiry unmet).
//     IP6: refund, NO signature (old CLI branch(right)+none()), height >= expiry
//          -> Err (Sig(refund_pkh) unmet) — documents the CLI refund bug.
//   MATRIX:
//     O1 x P1 x IP1 -> claim_with_preimage_after_lock_passes
//     O1 x P1 x IP2 -> claim_before_lock_height_fails
//     O1 x P1 x IP3 -> claim_with_wrong_preimage_fails
//     O1 x P2 x IP4 -> refund_with_signature_after_expiry_passes
//     O1 x P2 x IP5 -> refund_before_expiry_fails
//     O1 x P2 x IP6 -> refund_without_signature_fails

use crypto::Hash;
use doli_core::conditions::{Witness, WitnessSignature};
use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext};
use doli_core::HASHLOCK_DOMAIN;
use std::collections::HashMap;

const HTLC_PREV: Hash = Hash::from_bytes([0xE0; 32]);
const AMOUNT: u64 = 50_000_000;
const FEE: u64 = 1;
const LOCK: u64 = 10;
const EXPIRY: u64 = 20;

struct MockUtxos {
    utxos: HashMap<(Hash, u32), UtxoInfo>,
}
impl UtxoProvider for MockUtxos {
    fn get_utxo(&self, tx_hash: &Hash, index: u32) -> Option<UtxoInfo> {
        self.utxos.get(&(*tx_hash, index)).cloned()
    }
}

fn pkh(kp: &crypto::KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

fn ctx_at(height: u64) -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 100,
        height,
    )
    .with_prev_block(height.saturating_sub(1) as u32, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0)
}

/// Build a BridgeHTLC UTXO whose hashlock is H(preimage) and refund key is `refund_pkh`.
fn htlc_utxo(preimage: &[u8; 32], refund_pkh: Hash) -> (MockUtxos, Hash) {
    let expected_hash = crypto::hash::hash_with_domain(HASHLOCK_DOMAIN, preimage);
    let output = Output::bridge_htlc(
        AMOUNT,
        refund_pkh,
        expected_hash,
        LOCK,
        EXPIRY,
        doli_core::transaction::BRIDGE_CHAIN_BITCOIN,
        b"bc1qexampleaddr",
        Hash::from_bytes([0xCC; 32]),
    )
    .expect("bridge htlc output");
    let mut m = MockUtxos {
        utxos: HashMap::new(),
    };
    m.utxos.insert(
        (HTLC_PREV, 0),
        UtxoInfo {
            output,
            pubkey: None,
            spent: false,
        },
    );
    (m, expected_hash)
}

fn claim_tx(dest: Hash, preimage: [u8; 32]) -> Transaction {
    let input = Input::new(HTLC_PREV, 0);
    let mut tx = Transaction::new_transfer(vec![input], vec![Output::normal(AMOUNT - FEE, dest)]);
    // CLI claim witness: branch(left) + preimage(P).
    let witness = Witness {
        signatures: vec![],
        preimage: Some(preimage),
        or_branches: vec![false],
    };
    tx.set_covenant_witnesses(&[witness.encode()]);
    tx
}

fn refund_tx(refunder: &crypto::KeyPair, with_signature: bool) -> Transaction {
    let input = Input::new(HTLC_PREV, 0);
    let mut tx = Transaction::new_transfer(
        vec![input],
        vec![Output::normal(AMOUNT - FEE, pkh(refunder))],
    );
    let signing_hash = tx.signing_message_for_input(0);
    // Refund branch is And(Sig(refund_pkh), TimelockExpiry): the signature MUST be
    // in the covenant witness (covenant eval ignores the input signature field).
    let signatures = if with_signature {
        vec![WitnessSignature {
            pubkey: *refunder.public_key(),
            signature: crypto::signature::sign_hash(&signing_hash, refunder.private_key()),
        }]
    } else {
        vec![]
    };
    let witness = Witness {
        signatures,
        preimage: None,
        or_branches: vec![true],
    };
    tx.set_covenant_witnesses(&[witness.encode()]);
    tx
}

// ── Claim path (left) ────────────────────────────────────────────────────────

// O1 x P1 x IP1
#[test]
fn claim_with_preimage_after_lock_passes() {
    let receiver = crypto::KeyPair::from_seed([0x44; 32]);
    let refunder = crypto::KeyPair::from_seed([0x55; 32]);
    let preimage = [0x07; 32];
    let (utxos, _) = htlc_utxo(&preimage, pkh(&refunder));

    let tx = claim_tx(pkh(&receiver), preimage);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx_at(LOCK), &utxos);
    assert!(
        res.is_ok(),
        "a correct preimage at height >= lock must satisfy the claim branch. Got: {:?}",
        res
    );
}

// O1 x P1 x IP2
#[test]
fn claim_before_lock_height_fails() {
    let receiver = crypto::KeyPair::from_seed([0x44; 32]);
    let refunder = crypto::KeyPair::from_seed([0x55; 32]);
    let preimage = [0x07; 32];
    let (utxos, _) = htlc_utxo(&preimage, pkh(&refunder));

    let tx = claim_tx(pkh(&receiver), preimage);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx_at(LOCK - 1), &utxos);
    assert!(
        res.is_err(),
        "claim before lock_height must fail (Timelock unmet) — this is the MPTX007 \
         a premature claimer hits. Got Ok."
    );
}

// O1 x P1 x IP3
#[test]
fn claim_with_wrong_preimage_fails() {
    let receiver = crypto::KeyPair::from_seed([0x44; 32]);
    let refunder = crypto::KeyPair::from_seed([0x55; 32]);
    let preimage = [0x07; 32];
    let (utxos, _) = htlc_utxo(&preimage, pkh(&refunder));

    let tx = claim_tx(pkh(&receiver), [0x09; 32]); // wrong preimage
    let res = validation::validate_transaction_with_utxos(&tx, &ctx_at(LOCK), &utxos);
    assert!(
        res.is_err(),
        "a wrong preimage must fail (Hashlock unmet) — also a usage artifact, not a \
         consensus bug. Got Ok."
    );
}

// ── Refund path (right) ──────────────────────────────────────────────────────

// O1 x P2 x IP4
#[test]
fn refund_with_signature_after_expiry_passes() {
    let refunder = crypto::KeyPair::from_seed([0x55; 32]);
    let (utxos, _) = htlc_utxo(&[0x07; 32], pkh(&refunder));

    let tx = refund_tx(&refunder, true);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx_at(EXPIRY), &utxos);
    assert!(
        res.is_ok(),
        "a signed refund at height >= expiry must satisfy And(Sig, TimelockExpiry). \
         Got: {:?}",
        res
    );
}

// O1 x P2 x IP5
#[test]
fn refund_before_expiry_fails() {
    let refunder = crypto::KeyPair::from_seed([0x55; 32]);
    let (utxos, _) = htlc_utxo(&[0x07; 32], pkh(&refunder));

    let tx = refund_tx(&refunder, true);
    let res = validation::validate_transaction_with_utxos(&tx, &ctx_at(EXPIRY - 1), &utxos);
    assert!(
        res.is_err(),
        "refund before expiry must fail (TimelockExpiry unmet). Got Ok."
    );
}

// O1 x P2 x IP6 — documents the CLI refund bug: branch(right)+none() omits the sig.
#[test]
fn refund_without_signature_fails() {
    let refunder = crypto::KeyPair::from_seed([0x55; 32]);
    let (utxos, _) = htlc_utxo(&[0x07; 32], pkh(&refunder));

    let tx = refund_tx(&refunder, false); // no signature in the covenant witness
    let res = validation::validate_transaction_with_utxos(&tx, &ctx_at(EXPIRY), &utxos);
    assert!(
        res.is_err(),
        "a refund witness WITHOUT a signature must fail the signed-refund branch even \
         after expiry. The CLI must place the refunder's signature in the COVENANT \
         witness (not just the input). Got Ok."
    );
}
