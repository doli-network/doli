//! Funding transaction construction.
//!
//! Creates the on-chain funding transaction that locks coins into the 2-of-2
//! multisig. Supports dual-funded channels via AnyoneCanPay sighash.

use crypto::{Hash, KeyPair};
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::Amount;

use crate::conditions::funding_output;
use crate::error::{ChannelError, Result};

/// Build a funding transaction.
///
/// The funding tx spends one or more inputs into a single 2-of-2 multisig output.
/// For single-funded channels, only one party provides inputs.
/// For dual-funded channels, each party signs their inputs with AnyoneCanPay
/// so the other party can add their inputs later.
pub fn build_funding_tx(
    inputs: Vec<(Hash, u32)>,
    alice_pubkey_hash: Hash,
    bob_pubkey_hash: Hash,
    capacity: Amount,
    change_output: Option<Output>,
    dual_funded: bool,
) -> Result<Transaction> {
    if capacity == 0 {
        return Err(ChannelError::InsufficientBalance { need: 1, have: 0 });
    }

    let tx_inputs: Vec<Input> = inputs
        .into_iter()
        .map(|(tx_hash, idx)| {
            if dual_funded {
                Input::new_anyone_can_pay(tx_hash, idx)
            } else {
                Input::new(tx_hash, idx)
            }
        })
        .collect();

    let funding = funding_output(capacity, alice_pubkey_hash, bob_pubkey_hash)?;
    let mut outputs = vec![funding];

    if let Some(change) = change_output {
        outputs.push(change);
    }

    Ok(Transaction::new_transfer(tx_inputs, outputs))
}

/// Sign a specific input of the funding transaction.
pub fn sign_funding_input(
    tx: &mut Transaction,
    input_index: usize,
    keypair: &KeyPair,
) -> Result<()> {
    if input_index >= tx.inputs.len() {
        return Err(ChannelError::Protocol(format!(
            "input index {} out of range ({})",
            input_index,
            tx.inputs.len()
        )));
    }

    let signing_hash = tx.signing_message_for_input(input_index);
    let sig = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    tx.inputs[input_index].signature = sig;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::hash::hash;
    use doli_core::{OutputType, SighashType};

    #[test]
    fn build_single_funded_tx() {
        let alice = hash(b"alice");
        let bob = hash(b"bob");
        let input_hash = hash(b"prev_tx");

        let tx =
            build_funding_tx(vec![(input_hash, 0)], alice, bob, 1_000_000, None, false).unwrap();

        assert_eq!(tx.inputs.len(), 1);
        assert_eq!(tx.inputs[0].sighash_type, SighashType::All);
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].output_type, OutputType::Multisig);
        assert_eq!(tx.outputs[0].amount, 1_000_000);
    }

    #[test]
    fn build_dual_funded_tx() {
        let alice = hash(b"alice");
        let bob = hash(b"bob");

        let tx = build_funding_tx(
            vec![(hash(b"alice_utxo"), 0), (hash(b"bob_utxo"), 0)],
            alice,
            bob,
            2_000_000,
            None,
            true,
        )
        .unwrap();

        assert_eq!(tx.inputs.len(), 2);
        assert_eq!(tx.inputs[0].sighash_type, SighashType::AnyoneCanPay);
        assert_eq!(tx.inputs[1].sighash_type, SighashType::AnyoneCanPay);
    }

    #[test]
    fn build_with_change() {
        let alice = hash(b"alice");
        let bob = hash(b"bob");
        let change = Output::normal(500_000, alice);

        let tx = build_funding_tx(
            vec![(hash(b"prev"), 0)],
            alice,
            bob,
            1_000_000,
            Some(change),
            false,
        )
        .unwrap();

        assert_eq!(tx.outputs.len(), 2);
        assert_eq!(tx.outputs[0].amount, 1_000_000); // funding
        assert_eq!(tx.outputs[1].amount, 500_000); // change
    }

    #[test]
    fn zero_capacity_rejected() {
        let alice = hash(b"alice");
        let bob = hash(b"bob");
        let result = build_funding_tx(vec![(hash(b"prev"), 0)], alice, bob, 0, None, false);
        assert!(result.is_err());
    }
}
