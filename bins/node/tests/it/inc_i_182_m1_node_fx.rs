//! INC-I-182 M1 node fixtures: a Run B rig plus a newcomer, the three producer
//! txs in each authorization form, a reference signer built from the documented
//! preimage, and a child-process runner that sets the producer-auth gate.
//!
// OUTPUT CONTRACT: N/A — fixture file. INPUT PARTITIONS: N/A.
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client

#![allow(dead_code)]

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_core::transaction::{Input, OutputType, RegistrationData, Transaction, TxType};
use storage::Outpoint;

use super::inc_i_255_m2_common::{addr, outpoints, rig, Keys, Rig, HONEST_TAG};
use super::inc_i_255_m6_common::seed_coin;

pub const CODE: &str = "ERRTX-PAUTH001";
pub const PAUTH_ENV: &str = "DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT";
pub const GENESIS_ENV: &str = "DOLI_GENESIS_BLOCKS";
pub const CHILD_ENV: &str = "INC_I_182_M1_NODE_CHILD";
pub const BONDER_TAG: u8 = 0x9B;
pub const NEWCOMER_TAG: u8 = 0x9A;
pub const LOCK: u64 = 1_000;
pub const BOND_LOCK: u64 = 1_000_000;
pub const FEE: u64 = 100_000;
pub const KINDS: [TxType; 3] = [
    TxType::RequestWithdrawal,
    TxType::AddBond,
    TxType::Registration,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Signed,
    Unsigned,
    WrongKey,
    Replayed,
}

pub const FORGED: [Form; 3] = [Form::Unsigned, Form::WrongKey, Form::Replayed];

/// Reference signer: payload | Ed25519(BLAKE3(domain | genesis | signing_message | payload)).
pub fn reference_sign(tx: &mut Transaction, genesis: &Hash, kp: &KeyPair) {
    let mut h = crypto::Hasher::new();
    h.update(b"DOLI-PRODUCER-AUTH-V1");
    h.update(genesis.as_bytes());
    h.update(tx.signing_message().as_bytes());
    h.update(&tx.extra_data);
    let sig = crypto::signature::sign_hash(&h.finalize(), kp.private_key());
    tx.extra_data.extend_from_slice(sig.as_bytes());
}

pub fn fixed_keys() -> Keys {
    Keys {
        honest: KeyPair::from_seed([0x11; 32]),
        accused: KeyPair::from_seed([0x22; 32]),
        delegator: KeyPair::from_seed([0x33; 32]),
    }
}

pub struct Px {
    pub r: Rig,
    pub newcomer: KeyPair,
    pub bls: BlsKeyPair,
}

impl Px {
    pub fn genesis(&self) -> Hash {
        self.r.node.params.genesis_hash
    }
}

/// Rig with a funded bonder coin for the honest producer and a funded newcomer.
pub async fn px(keys: &Keys) -> Px {
    let r = rig(keys).await;
    let unit = r.unit();
    let newcomer = KeyPair::from_seed([0x44; 32]);
    seed_coin(&r.node, r.keys.honest.public_key(), BONDER_TAG, unit + FEE).await;
    seed_coin(&r.node, newcomer.public_key(), NEWCOMER_TAG, unit + FEE).await;
    Px {
        r,
        newcomer,
        bls: BlsKeyPair::from_seed(&[0x45; 32]).expect("FIXTURE: BLS seed"),
    }
}

fn signed_inputs(mut tx: Transaction, kp: &KeyPair) -> Transaction {
    for i in 0..tx.inputs.len() {
        tx.inputs[i].public_key = Some(*kp.public_key());
        let msg = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&msg, kp.private_key());
    }
    tx
}

fn one_input(op: Outpoint) -> Vec<Input> {
    vec![Input::new(op.tx_hash, op.index)]
}

fn coin(tag: u8) -> Outpoint {
    Outpoint::new(Hash::from_bytes([tag; 32]), 0)
}

/// Unsigned tx of `kind` spending `op`, inputs signed by the owner.
fn unsigned_on(p: &Px, kind: TxType, op: Outpoint) -> Transaction {
    let unit = p.r.unit();
    match kind {
        TxType::RequestWithdrawal => {
            let kp = &p.r.keys.honest;
            let pk = *kp.public_key();
            let mut tx =
                Transaction::new_request_withdrawal(one_input(op), pk, 1, addr(&pk), unit - 1);
            tx.outputs[0].output_type = OutputType::Unbonding;
            tx.outputs[0].lock_until = LOCK;
            signed_inputs(tx, kp)
        }
        TxType::AddBond => {
            let kp = &p.r.keys.honest;
            let tx = Transaction::new_add_bond(one_input(op), *kp.public_key(), 1, unit, BOND_LOCK);
            signed_inputs(tx, kp)
        }
        TxType::Registration => {
            let pk = *p.newcomer.public_key();
            let mut tx = Transaction::new_registration(one_input(op), pk, unit, BOND_LOCK, 1);
            let data = RegistrationData {
                public_key: pk,
                epoch: 0,
                vdf_output: vec![0x11; 32],
                vdf_proof: Vec::new(),
                prev_registration_hash: Hash::ZERO,
                sequence_number: 0,
                bond_count: 1,
                bls_pubkey: p.bls.public_key().as_bytes().to_vec(),
                bls_pop: p
                    .bls
                    .proof_of_possession()
                    .expect("FIXTURE: PoP")
                    .as_bytes()
                    .to_vec(),
            };
            tx.extra_data = bincode::serialize(&data).expect("FIXTURE: encode registration");
            signed_inputs(tx, &p.newcomer)
        }
        other => panic!("FIXTURE: {other:?} is not a producer-auth type"),
    }
}

/// The live outpoint `kind` spends, and a second one used only as a replay source.
fn spends(kind: TxType) -> (Outpoint, Outpoint) {
    match kind {
        TxType::RequestWithdrawal => {
            let ops = outpoints(HONEST_TAG, 2);
            (ops[0], ops[1])
        }
        TxType::AddBond => (coin(BONDER_TAG), coin(BONDER_TAG.wrapping_add(1))),
        _ => (coin(NEWCOMER_TAG), coin(NEWCOMER_TAG.wrapping_add(1))),
    }
}

pub fn producer_key(p: &Px, kind: TxType) -> &KeyPair {
    if kind == TxType::Registration {
        &p.newcomer
    } else {
        &p.r.keys.honest
    }
}

/// `kind` in `form`, signed (when it is) against `genesis`.
pub fn producer_tx(p: &Px, kind: TxType, form: Form, genesis: &Hash) -> Transaction {
    let (live, other) = spends(kind);
    let mut tx = unsigned_on(p, kind, live);
    match form {
        Form::Unsigned => {}
        Form::Signed => reference_sign(&mut tx, genesis, producer_key(p, kind)),
        Form::WrongKey => reference_sign(&mut tx, genesis, &p.r.keys.accused),
        Form::Replayed => {
            let mut src = unsigned_on(p, kind, other);
            reference_sign(&mut src, genesis, producer_key(p, kind));
            let tail = src.extra_data[src.extra_data.len() - 64..].to_vec();
            tx.extra_data.extend_from_slice(&tail);
        }
    }
    tx
}

pub fn in_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Re-exec `test` alone with `env` set; re-raise the first line of its panic.
pub fn run_child(test: &str, env: &[(&str, String)]) {
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env_remove(PAUTH_ENV)
        .env_remove(GENESIS_ENV);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in stdout
        .lines()
        .filter_map(|l| l.find("GOLDEN_OBSERVED").map(|i| &l[i..]))
    {
        println!("{line}");
    }
    if out.status.success() && stdout.contains("1 passed") {
        return;
    }
    let why: Vec<&str> = stderr
        .lines()
        .skip_while(|l| !l.contains("panicked at"))
        .skip(1)
        .take_while(|l| !l.starts_with("note: run with"))
        .collect();
    if why.is_empty() {
        panic!("child {test} failed or did not run:\n{stdout}\n{stderr}");
    }
    panic!("{}", why.join("\n"));
}

/// Child env for a world where the rule arms at `gate` and Registration takes its post-genesis path.
pub fn env_at(gate: u64) -> Vec<(&'static str, String)> {
    vec![
        (PAUTH_ENV, gate.to_string()),
        (GENESIS_ENV, "0".to_string()),
    ]
}
