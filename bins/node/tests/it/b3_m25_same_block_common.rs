//! B3 M2.5 shared fixtures: producer-signed Registration / AddBond for a world key, an exited
//! key with locked collateral, and builder blocks that hold a slash beside one of them.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: assembly, slash_state, slash_admission

#![allow(dead_code)]

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::block::Block;
use doli_core::transaction::{Input, Output, OutputType, Transaction, TxType};
use doli_node::node::Node;
use storage::{Outpoint, UtxoEntry};

use super::inc_i_255_m2_common::{addr, producer_auth};
use super::inc_i_255_m4_common::{admit_at, block_around};
use super::inc_i_255_m6_common::seed_coin;

pub const CODE: &str = "ERRTX071";
pub const FEE: u64 = 100_000;
pub const LOCK: u64 = 10_000_000;

/// Replaces the two empty trailing `Vec<u8>` prefixes (BLS key, PoP) of the encoded payload.
fn with_bls(tx: &mut Transaction, seed: u8) {
    let bls = BlsKeyPair::from_seed(&[seed; 32]).expect("FIXTURE: BLS seed");
    let pop = bls.proof_of_possession().expect("FIXTURE: PoP");
    let n = tx.extra_data.len();
    assert_eq!(
        &tx.extra_data[n - 16..],
        &[0u8; 16],
        "FIXTURE: RegistrationData layout"
    );
    tx.extra_data.truncate(n - 16);
    for part in [
        bls.public_key().as_bytes().to_vec(),
        pop.as_bytes().to_vec(),
    ] {
        tx.extra_data
            .extend_from_slice(&(part.len() as u64).to_le_bytes());
        tx.extra_data.extend_from_slice(&part);
    }
}

/// Producer-signed `kind` for `kp`, paid from a fresh coin of its own at `[tag; 32]:0`
/// (deterministic: the same call on a fresh world seeds the same coin and yields the same tx).
pub async fn producer_tx(node: &Node, kind: TxType, kp: &KeyPair, tag: u8) -> Transaction {
    let pk = *kp.public_key();
    let unit = node.config.network.bond_unit();
    let op = seed_coin(node, &pk, tag, unit + FEE).await;
    let mut input = Input::new(op.tx_hash, op.index);
    input.public_key = Some(pk);
    let mut tx = if kind == TxType::Registration {
        let mut t = Transaction::new_registration(vec![input], pk, unit, LOCK, 1);
        with_bls(&mut t, tag);
        t
    } else {
        Transaction::new_add_bond(vec![input], pk, 1, unit, LOCK)
    };
    let msg = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    producer_auth(tx, kp)
}

/// `pk` leaves the active set (Exited) but keeps one locked Unbonding UTXO, so it stays slashable.
pub async fn exit_with_collateral(node: &Node, pk: &PublicKey, tag: u8) {
    let h = node.chain_state.read().await.best_height;
    {
        let mut ps = node.producer_set.write().await;
        ps.request_exit(pk, h).expect("FIXTURE: request exit");
        ps.get_by_pubkey_mut(pk)
            .expect("FIXTURE: exited producer")
            .complete_exit();
    }
    let output = Output {
        output_type: OutputType::Unbonding,
        amount: node.config.network.bond_unit(),
        pubkey_hash: addr(pk),
        lock_until: LOCK,
        extra_data: Vec::new(),
    };
    let entry = UtxoEntry {
        output,
        height: 0,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    node.utxo_set
        .write()
        .await
        .insert(Outpoint::new(Hash::from_bytes([tag; 32]), 0), entry)
        .expect("FIXTURE: seed Unbonding UTXO");
}

pub fn coded(r: &anyhow::Result<()>) -> bool {
    matches!(r, Err(e) if format!("{e:#} {e:?}").contains(CODE))
}

/// Builder block for the next height holding the admitted `anchor` and `slash` right before
/// (`slash_first`) or right after it.
pub async fn slash_beside(
    node: &mut Node,
    anchor: &Transaction,
    slash: &Transaction,
    slash_first: bool,
) -> Block {
    let h = node.chain_state.read().await.best_height;
    let r = admit_at(node, anchor, h).await;
    assert!(
        r.is_ok(),
        "PRECONDITION: {:?} admitted with no slash resident; got {r:?}",
        anchor.tx_type
    );
    block_around(node, anchor, Some(slash), slash_first).await
}

pub fn order_name(slash_first: bool, kind: TxType) -> String {
    if slash_first {
        format!("[Slash(K), {kind:?}(K)]")
    } else {
        format!("[{kind:?}(K), Slash(K)]")
    }
}
