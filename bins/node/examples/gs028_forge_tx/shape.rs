//! Pure builders for the GS-028 transaction forge: producer transactions paid and
//! signed by a third party. No I/O.

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{sign_producer_auth, Input, Output, OutputType, Transaction};
use doli_core::validation::bond_owner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ForcedExit,
    ForcedExitWrongTail,
    AddBondUntailed,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "forced-exit" => Some(Kind::ForcedExit),
            "forced-exit-wrong-tail" => Some(Kind::ForcedExitWrongTail),
            "addbond-untailed" => Some(Kind::AddBondUntailed),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payout {
    Normal,
    Unbonding,
}

impl Payout {
    pub fn parse(s: &str) -> Option<Payout> {
        match s {
            "normal" => Some(Payout::Normal),
            "unbonding" => Some(Payout::Unbonding),
            _ => None,
        }
    }
}

pub struct Coin {
    pub tx_hash: Hash,
    pub index: u32,
    pub amount: u64,
}

pub struct Spec {
    pub kind: Kind,
    pub victim: PublicKey,
    pub bonds: u32,
    pub payout: Payout,
    pub lock_until: u64,
    pub fee: u64,
    pub bond_unit: u64,
    pub genesis: Hash,
}

fn forced_exit(spec: &Spec, input: Input, coin: &Coin) -> Result<Transaction, String> {
    if coin.amount <= spec.fee {
        return Err(format!(
            "coin {} does not exceed the fee {}",
            coin.amount, spec.fee
        ));
    }
    let mut tx = Transaction::new_request_withdrawal(
        vec![input],
        spec.victim,
        spec.bonds,
        bond_owner(&spec.victim),
        coin.amount - spec.fee,
    );
    if spec.payout == Payout::Unbonding {
        tx.outputs[0].output_type = OutputType::Unbonding;
        tx.outputs[0].lock_until = spec.lock_until;
    }
    Ok(tx)
}

fn add_bond(
    spec: &Spec,
    payer: &KeyPair,
    input: Input,
    coin: &Coin,
) -> Result<Transaction, String> {
    let bonded = spec
        .bond_unit
        .checked_mul(u64::from(spec.bonds))
        .ok_or("bond amount overflows")?;
    let need = bonded
        .checked_add(spec.fee)
        .ok_or("bond amount overflows")?;
    if coin.amount < need {
        return Err(format!("coin {} is below {need}", coin.amount));
    }
    let mut tx = Transaction::new_add_bond(
        vec![input],
        spec.victim,
        spec.bonds,
        bonded,
        spec.lock_until,
    );
    let change = coin.amount - need;
    if change > 0 {
        tx.outputs
            .push(Output::normal(change, bond_owner(payer.public_key())));
    }
    Ok(tx)
}

pub fn build(spec: &Spec, payer: &KeyPair, coin: &Coin) -> Result<Transaction, String> {
    let mut input = Input::new(coin.tx_hash, coin.index);
    input.public_key = Some(*payer.public_key());
    let mut tx = match spec.kind {
        Kind::ForcedExit | Kind::ForcedExitWrongTail => forced_exit(spec, input, coin)?,
        Kind::AddBondUntailed => add_bond(spec, payer, input, coin)?,
    };
    for i in 0..tx.inputs.len() {
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, payer.private_key());
    }
    if spec.kind == Kind::ForcedExitWrongTail {
        sign_producer_auth(&mut tx, &spec.genesis, payer);
    }
    Ok(tx)
}
