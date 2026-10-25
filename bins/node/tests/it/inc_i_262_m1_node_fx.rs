//! INC-I-262 M1 node fixtures: the desktop wallet's producer txs as today's code
//! builds them, rebuilt with core types, and path drivers over a tx builder.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: producer builder producer_tx unbonding_tx

#![allow(dead_code)]

use crypto::{bls_sign_pop, BlsSecretKey, Hash, KeyPair, PublicKey};
use doli_cli::producer_tx::{
    build_add_bond_tx, build_registration_tx, chain_genesis_hash, finalize_producer_tx,
    AddBondSpec, RegistrationSpec,
};
use doli_cli::unbonding_tx::{build_withdrawal_tx, builder_params, WithdrawalSpec};
use doli_core::transaction::{Input, Output, OutputType, RegistrationData, Transaction, TxType};
use doli_core::validation::{bond_owner, ValidationMode};
use doli_core::Network;

use super::inc_i_182_m1_common::Verdict;
use super::inc_i_182_m1_node_fx::{fixed_keys, px, Px, BONDER_TAG, NEWCOMER_TAG};
use super::inc_i_255_m1_common::{admit, full_block_with};
use super::inc_i_255_m2_common::{addr, outpoints, HONEST_TAG};
use super::inc_i_255_m4_common::{err_text, resident};
use super::inc_i_255_m6_common::{block_with, seed_coin};

pub const WALLET_COIN_TAG: u8 = 0x62;
pub const WALLET_BOND_UNIT: u64 = 1_000_000_000;
pub const GUI_FEE: u64 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Tip {
    pub height: u64,
    pub slot: u32,
}

pub type Build = fn(&Px, &Tip) -> Transaction;

/// Rig plus a coin the wallet's AddBond can fund with its compiled bond unit.
pub async fn rig() -> Px {
    let p = px(&fixed_keys()).await;
    seed_coin(
        &p.r.node,
        p.r.keys.honest.public_key(),
        WALLET_COIN_TAG,
        WALLET_BOND_UNIT + GUI_FEE,
    )
    .await;
    p
}

pub async fn tip(p: &Px) -> Tip {
    let (height, slot, _) = p.r.best().await;
    Tip { height, slot }
}

/// The era the GUI and CLI add to the tip for a Bond lock (the rig widens the devnet era).
pub fn era(p: &Px) -> u64 {
    p.r.node.params.blocks_per_era
}

pub fn hex_of(h: &Hash) -> String {
    hex::encode(h.as_bytes())
}

/// The fields the GUI derives before it builds a Registration.
pub struct GuiReg {
    pub inputs: Vec<(Hash, u32)>,
    pub reg_data: RegistrationData,
    pub bond_unit: u64,
    pub lock_until: u64,
    pub change: u64,
    pub change_to: Hash,
}

pub fn gui_registration_parts(p: &Px, tip: &Tip) -> GuiReg {
    let pk: PublicKey = *p.newcomer.public_key();
    let bond_count = 1u32;
    let bond_unit = p.r.unit();
    let required = bond_unit * u64::from(bond_count);
    let fee = doli_core::consensus::BASE_FEE
        + u64::from(bond_count) * 4 * doli_core::consensus::FEE_PER_BYTE
            / doli_core::consensus::FEE_DIVISOR;
    let coin_amount = bond_unit + super::inc_i_182_m1_node_fx::FEE;
    let epoch = (u64::from(tip.slot) / p.r.node.config.network.blocks_per_reward_epoch()) as u32;
    let vdf_input = vdf::registration_input(&pk, epoch);
    let vdf_output = doli_core::tpop::heartbeat::hash_chain_vdf(&vdf_input, vdf::T_REGISTER_BASE);
    let bls_sk = BlsSecretKey::from_hex(&p.bls.secret_key().to_hex()).expect("FIXTURE: BLS hex");
    let bls_pk = bls_sk.public_key();
    let bls_pop = bls_sign_pop(&bls_sk, &bls_pk).expect("FIXTURE: PoP");
    GuiReg {
        inputs: vec![(Hash::from_bytes([NEWCOMER_TAG; 32]), 0)],
        reg_data: RegistrationData {
            public_key: pk,
            epoch,
            vdf_output: vdf_output.to_vec(),
            vdf_proof: vec![],
            prev_registration_hash: Hash::ZERO,
            sequence_number: 0,
            bond_count,
            bls_pubkey: bls_pk.as_bytes().to_vec(),
            bls_pop: bls_pop.as_bytes().to_vec(),
        },
        bond_unit,
        lock_until: tip.height + era(p) + 1000,
        change: coin_amount - required - fee,
        change_to: addr(&pk),
    }
}

/// `register_producer` today: payload, Bond outputs, change, inputs signed; no producer-auth tail.
pub fn gui_registration_today(p: &Px, tip: &Tip) -> Transaction {
    let g = gui_registration_parts(p, tip);
    let pk = g.reg_data.public_key;
    let extra_data = bincode::serialize(&g.reg_data).expect("FIXTURE: encode");
    let owner = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes());
    let outputs = (0..g.reg_data.bond_count)
        .map(|_| Output::bond(g.bond_unit, owner, g.lock_until, 0))
        .collect();
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::Registration,
        inputs: g.inputs.iter().map(|(h, i)| Input::new(*h, *i)).collect(),
        outputs,
        extra_data,
    };
    if g.change > 0 {
        tx.outputs.push(Output::normal(g.change, g.change_to));
    }
    for i in 0..tx.inputs.len() {
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, p.newcomer.private_key());
        tx.inputs[i].public_key = Some(pk);
    }
    tx
}

/// `TxBuilder::sign_and_build` today: one signature over the whole-tx message on every input.
pub fn wallet_sign(mut tx: Transaction, kp: &KeyPair) -> Transaction {
    let sig = crypto::signature::sign_hash(&tx.signing_message(), kp.private_key());
    for input in &mut tx.inputs {
        input.signature = sig;
        input.public_key = Some(*kp.public_key());
    }
    tx
}

fn raw_output(output_type: OutputType, amount: u64, to: Hash, lock_until: u64) -> Output {
    Output {
        output_type,
        amount,
        pubkey_hash: to,
        lock_until,
        extra_data: Vec::new(),
    }
}

/// `TxBuilder::build_add_bond(utxos, count, sender, fee)` + `sign_and_build` today.
pub fn wallet_add_bond_today(
    kp: &KeyPair,
    coins: &[(Hash, u32, u64)],
    count: u32,
    fee: u64,
) -> Transaction {
    let sender = addr(kp.public_key());
    let needed = u64::from(count) * WALLET_BOND_UNIT + fee;
    let mut inputs = Vec::new();
    let mut have = 0u64;
    for (h, i, amount) in coins {
        inputs.push(Input::new(*h, *i));
        have += amount;
        if have >= needed {
            break;
        }
    }
    assert!(
        have >= needed,
        "FIXTURE: the wallet refuses an underfunded AddBond"
    );
    let mut outputs: Vec<Output> = (0..count)
        .map(|_| raw_output(OutputType::Bond, WALLET_BOND_UNIT, sender, u64::MAX))
        .collect();
    if have > needed {
        outputs.push(raw_output(OutputType::Normal, have - needed, sender, 0));
    }
    let tx = Transaction {
        version: 1,
        tx_type: TxType::AddBond,
        inputs,
        outputs,
        extra_data: Vec::new(),
    };
    wallet_sign(tx, kp)
}

/// `TxBuilder::build_request_withdrawal(count, sender, dest)` + `sign_and_build` today.
pub fn wallet_request_withdrawal_today(
    kp: &KeyPair,
    count: u32,
    dest: Option<Hash>,
) -> Transaction {
    let sender = addr(kp.public_key());
    let mut extra = count.to_le_bytes().to_vec();
    if let Some(d) = dest {
        extra.extend_from_slice(d.as_bytes());
    }
    let tx = Transaction {
        version: 1,
        tx_type: TxType::RequestWithdrawal,
        inputs: vec![Input::new(sender, 0)],
        outputs: vec![raw_output(OutputType::Normal, 0, dest.unwrap_or(sender), 0)],
        extra_data: extra,
    };
    wallet_sign(tx, kp)
}

pub fn current_registration(p: &Px, t: &Tip) -> Transaction {
    gui_registration_today(p, t)
}

pub fn current_add_bond(p: &Px, _t: &Tip) -> Transaction {
    let coin = (
        Hash::from_bytes([WALLET_COIN_TAG; 32]),
        0,
        WALLET_BOND_UNIT + GUI_FEE,
    );
    wallet_add_bond_today(&p.r.keys.honest, &[coin], 1, GUI_FEE)
}

pub fn current_withdrawal(p: &Px, _t: &Tip) -> Transaction {
    wallet_request_withdrawal_today(&p.r.keys.honest, 1, None)
}

pub const CURRENT: [(&str, Build); 3] = [
    ("Registration", current_registration),
    ("AddBond", current_add_bond),
    ("RequestWithdrawal", current_withdrawal),
];

pub fn bonder_coin() -> (Hash, u32) {
    (Hash::from_bytes([BONDER_TAG; 32]), 0)
}

pub fn honest_bond() -> (Hash, u32) {
    let op = outpoints(HONEST_TAG, 1)[0];
    (op.tx_hash, op.index)
}

pub async fn on_mempool(build: Build, at: u64) -> (Verdict, Transaction) {
    let mut p = rig().await;
    p.r.advance_to(at).await;
    let tx = build(&p, &tip(&p).await);
    let r = admit(&p.r.node, &tx).await;
    let v = Verdict {
        accepted: r.is_ok(),
        text: format!("mempool at tip {at}: {r:?}"),
    };
    (v, tx)
}

pub async fn on_builder(build: Build, at: u64) -> (Verdict, Transaction) {
    let mut p = rig().await;
    p.r.advance_to(at - 1).await;
    let tx = build(&p, &tip(&p).await);
    let admission = admit(&p.r.node, &tx).await;
    let pooled = resident(&p.r.node, &tx).await;
    let selected = pooled
        && full_block_with(&mut p.r.node, &[])
            .await
            .1
            .contains(&tx.hash());
    let v = Verdict {
        accepted: selected,
        text: format!("builder at {at}: admission {admission:?}, selected {selected}"),
    };
    (v, tx)
}

pub async fn on_apply(build: Build, at: u64) -> (Verdict, Transaction) {
    let mut p = rig().await;
    p.r.advance_to(at - 1).await;
    let tx = build(&p, &tip(&p).await);
    let block = block_with(&mut p.r.node, std::slice::from_ref(&tx)).await;
    let carried = block.transactions.iter().any(|t| t.hash() == tx.hash());
    let r = p.r.node.apply_block(block, ValidationMode::Light).await;
    let v = Verdict {
        accepted: carried && r.is_ok(),
        text: format!("apply at {at}: carried {carried}, {}", err_text(&r)),
    };
    (v, tx)
}

pub async fn all_paths(
    build: Build,
    at: u64,
    builder: bool,
) -> Vec<(&'static str, Verdict, Transaction)> {
    let mut out = Vec::new();
    let (v, tx) = on_mempool(build, at).await;
    out.push(("mempool", v, tx));
    if builder {
        let (v, tx) = on_builder(build, at).await;
        out.push(("builder", v, tx));
    }
    let (v, tx) = on_apply(build, at).await;
    out.push(("apply", v, tx));
    out
}

pub fn registration_spec(p: &Px, t: &Tip) -> RegistrationSpec {
    let g = gui_registration_parts(p, t);
    RegistrationSpec {
        inputs: g.inputs,
        reg_data: g.reg_data,
        bond_unit: g.bond_unit,
        lock_until: g.lock_until,
        change: g.change,
        change_to: g.change_to,
    }
}

pub fn chain_genesis(p: &Px) -> Hash {
    chain_genesis_hash(&hex_of(&p.genesis())).expect("FIXTURE: chain info genesis")
}

/// The GUI Registration through the shared builder and finalize.
pub fn reference_registration(p: &Px, t: &Tip) -> Transaction {
    let mut tx = build_registration_tx(&registration_spec(p, t)).expect("FIXTURE: registration");
    finalize_producer_tx(&mut tx, &p.newcomer, &chain_genesis(p));
    tx
}

pub fn add_bond_lock(p: &Px, t: &Tip) -> u64 {
    t.height + era(p) + 1000
}

pub fn reference_add_bond(p: &Px, t: &Tip) -> Transaction {
    let kp = &p.r.keys.honest;
    let pk = *kp.public_key();
    let mut tx = build_add_bond_tx(&AddBondSpec {
        inputs: vec![bonder_coin()],
        producer: pk,
        count: 1,
        bond_unit: p.r.unit(),
        lock_until: add_bond_lock(p, t),
        change: 0,
        change_to: addr(&pk),
    });
    finalize_producer_tx(&mut tx, kp, &chain_genesis(p));
    tx
}

pub fn reference_withdrawal(p: &Px, t: &Tip) -> Transaction {
    let kp = &p.r.keys.honest;
    let pk = *kp.public_key();
    let spec = WithdrawalSpec {
        producer: pk,
        bond_count: 1,
        bond_inputs: vec![honest_bond()],
        total_net: p.r.unit(),
        destination: bond_owner(&pk),
        fee_utxos: Vec::new(),
        fee: 1,
    };
    let mut tx = build_withdrawal_tx(builder_params(Network::Devnet), t.height, &spec)
        .expect("FIXTURE: withdrawal");
    finalize_producer_tx(&mut tx, kp, &chain_genesis(p));
    tx
}

pub const REFERENCE: [(&str, Build); 3] = [
    ("Registration", reference_registration),
    ("AddBond", reference_add_bond),
    ("RequestWithdrawal", reference_withdrawal),
];

/// One output, Unbonding, owned by the bond owner of `pk`.
pub fn one_unbonding_to(tx: &Transaction, pk: &PublicKey) -> bool {
    tx.outputs.len() == 1
        && tx.outputs[0].output_type == OutputType::Unbonding
        && tx.outputs[0].pubkey_hash == bond_owner(pk)
}
