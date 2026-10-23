//! INC-I-257 M1 shared fixtures: bond withdrawals in the instant shape (the
//! production constructor) and in the gated shape (type byte 16, own address,
//! locked), and the native value an address owns.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.

#![allow(dead_code)]

use crypto::{KeyPair, PublicKey};
use doli_core::transaction::{Input, Output, OutputType, Transaction};
use doli_node::node::Node;
use storage::Outpoint;

use super::inc_i_255_m2_common::addr;

pub const UNBONDING_TYPE_BYTE: u8 = 16;
pub const DEVNET_UNBONDING_PERIOD: u64 = 60;
pub const FEE: u64 = 1;

pub fn gated_payout_type() -> OutputType {
    OutputType::from_u8(UNBONDING_TYPE_BYTE).unwrap_or(OutputType::Normal)
}

pub fn gated_type_exists() -> bool {
    OutputType::from_u8(UNBONDING_TYPE_BYTE).is_some()
}

fn bond_inputs(kp: &KeyPair, ops: &[Outpoint]) -> Vec<Input> {
    ops.iter()
        .map(|o| {
            let mut input = Input::new(o.tx_hash, o.index);
            input.public_key = Some(*kp.public_key());
            input
        })
        .collect()
}

fn signed(mut tx: Transaction, kp: &KeyPair) -> Transaction {
    for i in 0..tx.inputs.len() {
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    }
    super::inc_i_255_m2_common::producer_auth(tx, kp)
}

fn unsigned_withdrawal(kp: &KeyPair, ops: &[Outpoint], net: u64) -> Transaction {
    let pk = *kp.public_key();
    Transaction::new_request_withdrawal(bond_inputs(kp, ops), pk, ops.len() as u32, addr(&pk), net)
}

/// Production constructor, payout to the producer's own address.
pub fn instant_withdrawal(kp: &KeyPair, ops: &[Outpoint], net: u64) -> Transaction {
    signed(unsigned_withdrawal(kp, ops, net), kp)
}

/// One payout output of type byte 16 to the producer's own address.
pub fn gated_withdrawal(kp: &KeyPair, ops: &[Outpoint], net: u64, lock_until: u64) -> Transaction {
    let mut tx = unsigned_withdrawal(kp, ops, net);
    tx.outputs = vec![Output {
        output_type: gated_payout_type(),
        amount: net,
        pubkey_hash: addr(kp.public_key()),
        lock_until,
        extra_data: Vec::new(),
    }];
    signed(tx, kp)
}

/// Gated shape once type byte 16 decodes, else the production constructor.
pub fn current_withdrawal(
    kp: &KeyPair,
    ops: &[Outpoint],
    net: u64,
    lock_until: u64,
) -> Transaction {
    if gated_type_exists() {
        gated_withdrawal(kp, ops, net, lock_until)
    } else {
        instant_withdrawal(kp, ops, net)
    }
}

/// Native value owned by address(owner), spendable or locked, minted entries excluded.
pub async fn owned_native_value(node: &Node, owner: &PublicKey) -> u64 {
    let a = addr(owner);
    let utxo = node.utxo_set.read().await;
    utxo.iter_all()
        .into_iter()
        .filter(|(_, e)| e.output.pubkey_hash == a && !e.is_coinbase && !e.is_epoch_reward)
        .filter(|(_, e)| {
            e.output.output_type.is_native_amount()
                || e.output.output_type as u8 == UNBONDING_TYPE_BYTE
        })
        .map(|(_, e)| e.output.amount)
        .sum()
}
