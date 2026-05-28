// INC-I-092 RC-C ground-truth: does a correctly-signed channel cooperative
// close PASS the CONSENSUS covenant evaluator (validate_transaction_with_utxos)?
//
// The stress test reported [MPTX007] "covenant condition not satisfied" on
// channel cooperative-close + force-close and bridge claim/refund. The funding
// output is a 2-of-2 multisig (channels/src/conditions.rs::funding_condition).
// Cooperative close therefore REQUIRES both parties' signatures by design.
//
// This test exercises the full, correctly-signed path through the real
// consensus evaluator. If it passes, the covenant machinery is sound and the
// MPTX007 failures are a single-party USAGE artifact (a lone caller cannot
// produce the counterparty signature / the pre-signed commitment / the right
// preimage) — NOT a consensus or witness-encoding bug. If it fails, there is a
// genuine condition/witness mismatch to fix.
//
// OUTPUT CONTRACT: fn validate_transaction_with_utxos(tx, ctx, utxo_provider)
//                  for a Transfer spending a 2-of-2 Multisig funding UTXO.
//   Outputs:
//     O1: returned Result<(), ValidationError>.
//   PATHS:
//     P1: witness carries BOTH valid signatures over signing_message_for_input(0)
//         -> Multisig(2,[alice,bob]) satisfied -> Ok.
//     P2: witness carries only ONE signature (the single-party case the
//         stress-tester actually exercised) -> threshold not met -> Err.
//   INPUT PARTITIONS:
//     IP1: both alice+bob sign the cooperative-close tx, fee >= min_fee.
//     IP2: only alice signs.
//   MATRIX:
//     O1 x P1 x IP1 -> cooperative_close_with_both_sigs_passes_consensus
//     O1 x P2 x IP2 -> cooperative_close_with_one_sig_fails_consensus

use channels::close::{build_cooperative_close, sign_cooperative_close};
use channels::conditions::funding_output;
use channels::types::ChannelBalance;
use crypto::Hash;
use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
use doli_core::network::Network;
use doli_core::validation::{self, UtxoInfo, UtxoProvider, ValidationContext};
use std::collections::HashMap;

const FUNDING_PREV: Hash = Hash::from_bytes([0xF0; 32]);
const CAPACITY: u64 = 100_000_000;
const FEE: u64 = 1_000_000;

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

fn ctx() -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 100,
        10,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0)
}

fn utxos_with_funding(funding: doli_core::transaction::Output) -> MockUtxos {
    let mut m = MockUtxos {
        utxos: HashMap::new(),
    };
    m.utxos.insert(
        (FUNDING_PREV, 0),
        UtxoInfo {
            output: funding,
            pubkey: None,
            spent: false,
        },
    );
    m
}

// O1 x P1 x IP1 — both signatures: the covenant is satisfied at consensus level.
#[test]
fn cooperative_close_with_both_sigs_passes_consensus() {
    let alice = crypto::KeyPair::from_seed([0x11; 32]);
    let bob = crypto::KeyPair::from_seed([0x22; 32]);
    let (a_pkh, b_pkh) = (pkh(&alice), pkh(&bob));

    let funding = funding_output(CAPACITY, a_pkh, b_pkh).expect("funding output");
    let balance = ChannelBalance::new(60_000_000, 40_000_000);

    let mut tx =
        build_cooperative_close(FUNDING_PREV, 0, a_pkh, b_pkh, &balance, CAPACITY, FEE).unwrap();

    // Both parties sign over the per-input signing hash.
    let sh = tx.signing_message_for_input(0);
    let bob_sig = crypto::signature::sign_hash(&sh, bob.private_key());
    sign_cooperative_close(&mut tx, &alice, bob.public_key(), &bob_sig, a_pkh, b_pkh).unwrap();

    let res =
        validation::validate_transaction_with_utxos(&tx, &ctx(), &utxos_with_funding(funding));
    assert!(
        res.is_ok(),
        "a cooperative close signed by BOTH parties must satisfy the 2-of-2 \
         funding covenant at the consensus layer. If this passes, MPTX007 in \
         the stress test was a single-party usage artifact, not a bug. Got: {:?}",
        res
    );
}

// O1 x P2 x IP2 — only one signature: threshold unmet (the stress-tester case).
#[test]
fn cooperative_close_with_one_sig_fails_consensus() {
    let alice = crypto::KeyPair::from_seed([0x11; 32]);
    let bob = crypto::KeyPair::from_seed([0x22; 32]);
    let (a_pkh, b_pkh) = (pkh(&alice), pkh(&bob));

    let funding = funding_output(CAPACITY, a_pkh, b_pkh).expect("funding output");
    let balance = ChannelBalance::new(60_000_000, 40_000_000);

    let mut tx =
        build_cooperative_close(FUNDING_PREV, 0, a_pkh, b_pkh, &balance, CAPACITY, FEE).unwrap();

    // Only alice signs — emulate a single party trying to close a 2-of-2.
    let sh = tx.signing_message_for_input(0);
    let alice_sig = crypto::signature::sign_hash(&sh, alice.private_key());
    let witness = doli_core::Witness {
        signatures: vec![doli_core::ConditionWitnessSignature {
            pubkey: *alice.public_key(),
            signature: alice_sig,
        }],
        preimage: None,
        or_branches: vec![],
    };
    tx.set_covenant_witnesses(&[witness.encode()]);

    let res =
        validation::validate_transaction_with_utxos(&tx, &ctx(), &utxos_with_funding(funding));
    assert!(
        res.is_err(),
        "a single signature must NOT satisfy a 2-of-2 funding covenant — this \
         is the expected MPTX007 a lone stress-tester hits. Got: {:?}",
        res
    );
}
