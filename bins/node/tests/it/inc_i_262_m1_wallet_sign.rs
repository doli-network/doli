//! INC-I-262 M1 — every input signature of a wallet-built transfer verifies
//! against the per-input signing message the node checks, and the node admits it.
//!
// covers: builder
//
// OUTPUT CONTRACT: fn wallet::TxBuilder::sign_and_build(&mut self, &KeyPair) -> Result<String>
//   O1 return: hex that decodes to a doli_core::Transaction whose input i carries
//      Ed25519(pk, tx.signing_message_for_input(i)) and public_key = pk
// OUTPUT CONTRACT: fn Mempool::add_transaction(..) -> Result<..>   O2 return for that transfer
// PATHS: one input | two inputs (distinct per-input messages)
// MATRIX
//   O1 x {one input, two inputs} -> inc_i_262_m1_req_def_001_wallet_transfer_inputs_verify_per_input
//   O2 x {one input} -> inc_i_262_m1_req_def_001_wallet_transfer_admitted_by_node
// INPUT PARTITIONS: coins 5e8 + 7e8, amount 3e8 (one input) | 9e8 (two inputs), fee 100_000

use crypto::{Hash, KeyPair};
use doli_core::transaction::Transaction;

use super::inc_i_255_m1_common::admit;
use super::inc_i_255_m2_common::addr;
use super::inc_i_255_m6_common::seed_coin;
use super::inc_i_262_m1_node_fx::{hex_of, rig};

const FEE: u64 = 100_000;

fn utxo(tag: u8, amount: u64) -> wallet::Utxo {
    wallet::Utxo {
        tx_hash: hex_of(&Hash::from_bytes([tag; 32])),
        output_index: 0,
        amount,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 1,
        spendable: true,
    }
}

fn transfer(kp: &KeyPair, coins: &[wallet::Utxo], amount: u64) -> Transaction {
    let sender = *addr(kp.public_key()).as_bytes();
    let mut b = wallet::TxBuilder::build_transfer(coins, [0x5E; 32], amount, FEE, sender)
        .expect("FIXTURE: transfer builds");
    let hex = b.sign_and_build(kp).expect("FIXTURE: transfer signs");
    Transaction::deserialize(&hex::decode(hex).expect("hex")).expect("decodes into core")
}

pub fn bad_inputs(tx: &Transaction, kp: &KeyPair) -> Vec<usize> {
    (0..tx.inputs.len())
        .filter(|&i| {
            let ok = tx.inputs[i].public_key == Some(*kp.public_key())
                && crypto::signature::verify_hash(
                    &tx.signing_message_for_input(i),
                    &tx.inputs[i].signature,
                    kp.public_key(),
                )
                .is_ok();
            !ok
        })
        .collect()
}

// REQ-DEF-001 — Decision: a failure means the wallet signs inputs over a message the node does not check, so every wallet-built tx is refused for its input signatures.
#[test]
fn inc_i_262_m1_req_def_001_wallet_transfer_inputs_verify_per_input() {
    let kp = KeyPair::from_seed([0x63; 32]);
    let coins = [utxo(0x71, 500_000_000), utxo(0x72, 700_000_000)];
    let mut bad = Vec::new();
    for (what, amount, n) in [
        ("one input", 300_000_000, 1),
        ("two inputs", 900_000_000, 2),
    ] {
        let tx = transfer(&kp, &coins, amount);
        assert_eq!(tx.inputs.len(), n, "FIXTURE: {what}");
        let mut forged = tx.clone();
        forged.inputs[0].signature = crypto::signature::sign_hash(&Hash::ZERO, kp.private_key());
        assert!(
            bad_inputs(&forged, &kp).contains(&0),
            "NON-VACUITY: a wrong signature is caught"
        );
        let b = bad_inputs(&tx, &kp);
        println!("GOLDEN_OBSERVED wallet transfer {what}: inputs failing per-input verify {b:?}");
        if !b.is_empty() {
            bad.push(format!("{what}: inputs {b:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "wallet input signatures do not verify per input: {bad:?}"
    );
}

// REQ-DEF-001 — Decision: a failure means a node refuses a wallet-built transfer, so the wallet's signing step is not node-valid.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_wallet_transfer_admitted_by_node() {
    let p = rig().await;
    let kp = p.r.keys.honest.clone();
    seed_coin(&p.r.node, kp.public_key(), 0x73, 500_000_000).await;
    let tx = transfer(&kp, &[utxo(0x73, 500_000_000)], 300_000_000);
    let r = admit(&p.r.node, &tx).await;
    println!("GOLDEN_OBSERVED wallet transfer admission: {r:?}");
    assert!(r.is_ok(), "node refuses a wallet-built transfer: {r:?}");
}
