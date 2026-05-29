// INC-I-093 — Channel cooperative-close two-party handoff.
//
// The confirmed bug (cmd_channel.rs cooperative-close): the CLI built the close
// tx and set only inputs[0].signature, never calling sign_cooperative_close, so
// the 2-of-2 funding covenant witness was never written -> [MPTX007]. A
// cooperative close spends a 2-of-2 funding output and therefore STRUCTURALLY
// needs both parties' signatures. The fix is a one-shot PSBT-style handoff:
//   - initiator: build_cooperative_close_offer() builds the tx + signs its half,
//     emitting a portable CooperativeCloseOffer (the file the CLI writes).
//   - counterparty: finalize_cooperative_close_offer() verifies the initiator's
//     signature, co-signs via sign_cooperative_close (writing the 2-of-2 covenant
//     witness), and returns a broadcast-ready tx.
//
// This test drives the handoff through the REAL consensus evaluator
// (validate_transaction_with_utxos), the same ground-truth used by
// inc_i_092_close_covenant.rs.
//
// OUTPUT CONTRACT:
//   fn build_cooperative_close_offer(...) -> Result<CooperativeCloseOffer>
//   fn finalize_cooperative_close_offer(offer, finisher_kp) -> Result<Transaction>
//   Outputs:
//     O1: finalize() returned Result<Transaction, ChannelError>.
//     O2: consensus result validate_transaction_with_utxos(finalized_tx) for the
//         Transfer spending the 2-of-2 Multisig funding UTXO.
//   PATHS:
//     P1: offer built by initiator, finalized by the genuine counterparty ->
//         witness carries BOTH funding-party signatures over input(0) ->
//         O1 = Ok AND O2 = Ok.
//     P2: partial_tx tampered after the initiator signed (counterparty rewrites
//         the split) -> initiator signature no longer valid over the new signing
//         hash -> O1 = Err(InvalidSignature). (fail BEFORE broadcast)
//     P3: offer finalized by a THIRD party (not a funding key) -> witness sigs do
//         not match the 2-of-2 funding keys -> O2 = Err.
//   INPUT PARTITIONS:
//     IP1: alice initiates a 60/40 split, bob finalizes, fee paid by alice.
//     IP2: bob tampers the partial_tx to inflate his payout before finalizing.
//     IP3: carol (a non-party key) attempts to finalize the alice/bob offer.
//   MATRIX:
//     O1 x P1 x IP1 -> handoff_both_parties_passes_consensus (Ok + Ok)
//     O1 x P2 x IP2 -> handoff_rejects_tampered_partial_tx (Err before broadcast)
//     O2 x P3 x IP3 -> handoff_wrong_counterparty_fails_consensus (consensus Err)

use channels::close::{
    build_cooperative_close_offer, finalize_cooperative_close_offer, CooperativeCloseOffer,
};
use channels::conditions::funding_output;
use channels::types::{ChannelBalance, ChannelId};
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

fn build_offer(
    initiator: &crypto::KeyPair,
    counterparty_pkh: Hash,
    balance: &ChannelBalance,
) -> CooperativeCloseOffer {
    let channel_id = ChannelId::from_funding_outpoint(&FUNDING_PREV, 0);
    build_cooperative_close_offer(
        &channel_id,
        FUNDING_PREV,
        0,
        pkh(initiator),
        counterparty_pkh,
        balance,
        CAPACITY,
        FEE,
        initiator,
    )
    .expect("build offer")
}

// O1 x P1 x IP1 — genuine two-party handoff satisfies the 2-of-2 covenant.
#[test]
fn handoff_both_parties_passes_consensus() {
    let alice = crypto::KeyPair::from_seed([0x11; 32]);
    let bob = crypto::KeyPair::from_seed([0x22; 32]);
    let (a_pkh, b_pkh) = (pkh(&alice), pkh(&bob));

    let funding = funding_output(CAPACITY, a_pkh, b_pkh).expect("funding output");
    let balance = ChannelBalance::new(60_000_000, 40_000_000);

    // Initiator (alice) builds + signs her half; counterparty (bob) finalizes.
    let offer = build_offer(&alice, b_pkh, &balance);
    let tx = finalize_cooperative_close_offer(&offer, &bob).expect("finalize");

    let res =
        validation::validate_transaction_with_utxos(&tx, &ctx(), &utxos_with_funding(funding));
    assert!(
        res.is_ok(),
        "a handoff co-signed by BOTH funding parties must satisfy the 2-of-2 \
         funding covenant at consensus. Got: {:?}",
        res
    );
}

// O1 x P2 x IP2 — tampering the partial_tx invalidates the initiator signature,
// caught at finalize BEFORE any broadcast.
#[test]
fn handoff_rejects_tampered_partial_tx() {
    let alice = crypto::KeyPair::from_seed([0x11; 32]);
    let bob = crypto::KeyPair::from_seed([0x22; 32]);
    let b_pkh = pkh(&bob);

    let balance = ChannelBalance::new(60_000_000, 40_000_000);
    let mut offer = build_offer(&alice, b_pkh, &balance);

    // Bob rewrites the partial tx to inflate his own payout, then tries to finalize.
    let mut tx = doli_core::Transaction::deserialize(&hex::decode(&offer.partial_tx).expect("hex"))
        .expect("deserialize partial tx");
    // Bump an output amount — changes signing_message_for_input(0).
    tx.outputs[0].amount += 1;
    offer.partial_tx = hex::encode(tx.serialize());

    let res = finalize_cooperative_close_offer(&offer, &bob);
    assert!(
        res.is_err(),
        "finalize must reject an offer whose partial_tx was modified after the \
         initiator signed (initiator signature no longer valid). Got Ok."
    );
}

// O2 x P3 x IP3 — a third party cannot finalize: its signature is not a funding key.
#[test]
fn handoff_wrong_counterparty_fails_consensus() {
    let alice = crypto::KeyPair::from_seed([0x11; 32]);
    let bob = crypto::KeyPair::from_seed([0x22; 32]);
    let carol = crypto::KeyPair::from_seed([0x33; 32]);
    let (a_pkh, b_pkh) = (pkh(&alice), pkh(&bob));

    let funding = funding_output(CAPACITY, a_pkh, b_pkh).expect("funding output");
    let balance = ChannelBalance::new(60_000_000, 40_000_000);

    // Offer is for the alice/bob channel, but carol finalizes.
    let offer = build_offer(&alice, b_pkh, &balance);
    let tx = finalize_cooperative_close_offer(&offer, &carol).expect("finalize builds a tx");

    let res =
        validation::validate_transaction_with_utxos(&tx, &ctx(), &utxos_with_funding(funding));
    assert!(
        res.is_err(),
        "a witness containing a non-funding key (carol) must NOT satisfy the \
         2-of-2 funding covenant. Got: {:?}",
        res
    );
}
