//! Channel close transactions: cooperative, unilateral, and penalty.

use crypto::{Hash, KeyPair};
use doli_core::conditions::{Witness, WitnessSignature};
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::Amount;

use crate::commitment::{build_delayed_claim_witness, build_penalty_witness, CommitmentPair};
use crate::error::{ChannelError, Result};
use crate::types::ChannelBalance;

/// Build a cooperative close transaction.
///
/// Spends the 2-of-2 funding output directly to two Normal outputs.
/// Both parties must sign. No timelocks, no revocation.
pub fn build_cooperative_close(
    funding_tx_hash: Hash,
    funding_output_index: u32,
    local_pubkey_hash: Hash,
    remote_pubkey_hash: Hash,
    balance: &ChannelBalance,
) -> Result<Transaction> {
    let input = Input::new(funding_tx_hash, funding_output_index);
    let mut outputs = Vec::new();

    if balance.local > 0 {
        outputs.push(Output::normal(balance.local, local_pubkey_hash));
    }
    if balance.remote > 0 {
        outputs.push(Output::normal(balance.remote, remote_pubkey_hash));
    }

    if outputs.is_empty() {
        return Err(ChannelError::InsufficientBalance { need: 1, have: 0 });
    }

    Ok(Transaction::new_transfer(vec![input], outputs))
}

/// Sign a cooperative close transaction for the 2-of-2 multisig.
///
/// The witness needs signatures from both parties.
pub fn sign_cooperative_close(
    tx: &mut Transaction,
    local_keypair: &KeyPair,
    remote_pubkey: &crypto::PublicKey,
    remote_signature: &crypto::Signature,
    local_pubkey_hash: Hash,
    remote_pubkey_hash: Hash,
) -> Result<()> {
    let signing_hash = tx.signing_message_for_input(0);
    let local_sig = crypto::signature::sign_hash(&signing_hash, local_keypair.private_key());

    // Build multisig witness with both signatures, sorted by pubkey hash
    let mut sigs = vec![
        (local_pubkey_hash, *local_keypair.public_key(), local_sig),
        (remote_pubkey_hash, *remote_pubkey, *remote_signature),
    ];
    sigs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let witness = Witness {
        signatures: sigs
            .into_iter()
            .map(|(_, pubkey, signature)| WitnessSignature { pubkey, signature })
            .collect(),
        preimage: None,
        or_branches: Vec::new(),
    };

    tx.set_covenant_witnesses(&[witness.encode()]);
    Ok(())
}

/// Build a unilateral (force) close transaction.
///
/// Broadcasts the latest commitment transaction. The to_local output
/// will be timelocked (dispute window), and the to_remote output
/// is immediately spendable by the counterparty.
pub fn build_force_close(
    commitment: &CommitmentPair,
    funding_tx_hash: Hash,
    funding_output_index: u32,
    local_pubkey_hash: Hash,
    remote_pubkey_hash: Hash,
    dispute_height: u64,
) -> Result<Transaction> {
    commitment.build_local_commitment(
        funding_tx_hash,
        funding_output_index,
        local_pubkey_hash,
        remote_pubkey_hash,
        dispute_height,
    )
}

/// Build a penalty transaction that sweeps a revoked commitment's to_local output.
///
/// When the counterparty broadcasts a revoked commitment, we can claim their
/// to_local output using the revocation preimage + our signature.
pub fn build_penalty_tx(
    revoked_tx_hash: Hash,
    to_local_output_index: u32,
    to_local_amount: Amount,
    claim_pubkey_hash: Hash,
    keypair: &KeyPair,
    revocation_preimage: &[u8; 32],
) -> Result<Transaction> {
    let input = Input::new(revoked_tx_hash, to_local_output_index);
    let claim_output = Output::normal(to_local_amount, claim_pubkey_hash);

    let mut tx = Transaction::new_transfer(vec![input], vec![claim_output]);

    let signing_hash = tx.signing_message_for_input(0);
    let witness = build_penalty_witness(&signing_hash, keypair, revocation_preimage);
    tx.set_covenant_witnesses(&[witness.encode()]);

    // Sign the input
    let sig = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    tx.inputs[0].signature = sig;

    Ok(tx)
}

/// Build a delayed claim transaction for our to_local output after the dispute window.
pub fn build_delayed_claim(
    commitment_tx_hash: Hash,
    to_local_output_index: u32,
    to_local_amount: Amount,
    claim_pubkey_hash: Hash,
    keypair: &KeyPair,
) -> Result<Transaction> {
    let input = Input::new(commitment_tx_hash, to_local_output_index);
    let claim_output = Output::normal(to_local_amount, claim_pubkey_hash);

    let mut tx = Transaction::new_transfer(vec![input], vec![claim_output]);

    let signing_hash = tx.signing_message_for_input(0);
    let witness = build_delayed_claim_witness(&signing_hash, keypair);
    tx.set_covenant_witnesses(&[witness.encode()]);

    let sig = crypto::signature::sign_hash(&signing_hash, keypair.private_key());
    tx.inputs[0].signature = sig;

    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChannelBalance;

    #[test]
    fn cooperative_close_creates_correct_outputs() {
        let local = crypto::hash::hash(b"local");
        let remote = crypto::hash::hash(b"remote");
        let funding_hash = crypto::hash::hash(b"funding");

        let tx = build_cooperative_close(
            funding_hash,
            0,
            local,
            remote,
            &ChannelBalance::new(600_000, 400_000),
        )
        .unwrap();

        assert_eq!(tx.inputs.len(), 1);
        assert_eq!(tx.outputs.len(), 2);
        assert_eq!(tx.outputs[0].amount, 600_000);
        assert_eq!(tx.outputs[1].amount, 400_000);
    }

    #[test]
    fn cooperative_close_skips_zero_balance() {
        let local = crypto::hash::hash(b"local");
        let remote = crypto::hash::hash(b"remote");
        let funding_hash = crypto::hash::hash(b"funding");

        let tx = build_cooperative_close(
            funding_hash,
            0,
            local,
            remote,
            &ChannelBalance::new(1_000_000, 0),
        )
        .unwrap();

        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, 1_000_000);
    }

    #[test]
    fn penalty_tx_builds_successfully() {
        let keypair = crypto::KeyPair::generate();
        let revocation_preimage = [42u8; 32];
        let revoked_hash = crypto::hash::hash(b"revoked_tx");
        let claim_pubkey = crypto::hash::hash(b"claim");

        let tx = build_penalty_tx(
            revoked_hash,
            0,
            1_000_000,
            claim_pubkey,
            &keypair,
            &revocation_preimage,
        )
        .unwrap();

        assert_eq!(tx.inputs.len(), 1);
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, 1_000_000);
        assert!(!tx.extra_data.is_empty()); // covenant witness set
    }
}
