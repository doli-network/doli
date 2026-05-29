//! Signing + covenant-witness assembly for AMM pool transactions (INC-I-095).

use crypto::{signature, KeyPair};
use doli_core::{ConditionWitnessSignature, Transaction, Witness};

/// Sign every input and attach covenant witnesses.
///
/// Conditioned outputs (LPShare, FungibleAsset, ...) are spent by satisfying their
/// covenant, which the node evaluates from `get_covenant_witness(i)` via
/// `Witness::decode` — NOT from `input.signature`. `conditioned[i] == true` means
/// input `i` spends such an output and gets a real signature witness; `false` (the
/// signature-exempt Pool input under RC-A, and Normal DOLI fee inputs) gets an empty
/// witness. Omitting a witness for a conditioned input yields [MPTX007] at mempool
/// admission — which is exactly the bug `pool remove` had before this helper existed.
pub fn sign_with_covenant_witnesses(tx: &mut Transaction, keypair: &KeyPair, conditioned: &[bool]) {
    let mut witnesses: Vec<Vec<u8>> = Vec::with_capacity(tx.inputs.len());
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        let sig = signature::sign_hash(&signing_hash, keypair.private_key());
        tx.inputs[i].signature = sig;
        tx.inputs[i].public_key = Some(*keypair.public_key());
        if conditioned.get(i).copied().unwrap_or(false) {
            let witness = Witness {
                signatures: vec![ConditionWitnessSignature {
                    pubkey: *keypair.public_key(),
                    signature: sig,
                }],
                ..Default::default()
            };
            witnesses.push(witness.encode());
        } else {
            witnesses.push(Vec::new());
        }
    }
    tx.set_covenant_witnesses(&witnesses);
}
