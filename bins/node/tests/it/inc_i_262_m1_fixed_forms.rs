//! INC-I-262 M1 — the fixed desktop-wallet entry points return the shared
//! builders' signed txs byte for byte; the node admits, selects and applies
//! them at the producer-auth gate (the withdrawal also at the unbonding gate),
//! and a malformed chain genesis or network aborts the build.
//!
// covers: producer builder producer_tx unbonding_tx mod lib
//
// OUTPUT CONTRACT: fn wallet::TxBuilder::build_add_bond(&[Utxo], bond_count: u32, bond_unit: u64, lock_until: u64, fee: u64, &ProducerSigner) -> anyhow::Result<String>
// OUTPUT CONTRACT: fn wallet::TxBuilder::build_request_withdrawal(&WithdrawalRequest, &ProducerSigner) -> anyhow::Result<String>
//   O1 return Ok: hex of the tx = build_add_bond_tx / build_withdrawal_tx + finalize_producer_tx over the same fields;
//      every input verifies against signing_message_for_input(i); withdrawal at/above the unbonding gate: one Unbonding output to the bond owner
//   O2 return Err: genesis_hash not 64 hex chars, or unknown network; nothing built
// OUTPUT CONTRACT: fn doli_cli::producer_tx::build_signed_registration(&RegistrationSpec, &KeyPair, &Hash) -> Result<Transaction, String>
//   O3 return: equals the shared-builder reference Registration
// OUTPUT CONTRACT: Mempool::add_transaction / Node::build_block_content / Node::apply_block(Light)   O4 accepted at height 2
// PATHS: producer-auth gate 0 | unbonding gate == 2 | producer-auth gate u64::MAX | malformed genesis / network
// MATRIX
//   O1, O3 x 3 kinds x gate 0 -> inc_i_262_m1_req_def_001_fixed_forms_equal_shared_builder
//   O4 x 3 kinds x gate 0 -> inc_i_262_m1_req_def_001_fixed_forms_admitted_at_gate (INC_I_262_FIXED_ADMITTED)
//   O1, O4 x RequestWithdrawal x unbonding gate 2 -> inc_i_262_m1_req_def_001_fixed_withdrawal_at_unbonding_gate
//   O4 x Registration x gate u64::MAX -> inc_i_262_m1_req_def_001_fixed_registration_admitted_below_gate
//   O2 x {"", "zz", 62 hex, 64 non-hex, network "moonnet"} -> inc_i_262_m1_req_def_001_wallet_malformed_genesis_refused
// INPUT PARTITIONS: honest producer, one bond, net = bond unit, fee 1; AddBond coin = unit + 100_000, no change

use crypto::{Hash, KeyPair};
use doli_cli::producer_tx::build_signed_registration;
use doli_core::transaction::Transaction;
use doli_core::Network;
use wallet::{ProducerSigner, TxBuilder, Utxo, WithdrawalRequest};

use super::inc_i_182_m1_node_fx::{env_at, fixed_keys, in_child, run_child, Px, FEE};
use super::inc_i_262_m1_node_fx::*;
use super::inc_i_262_m1_wallet_sign::bad_inputs;

const AT: u64 = 2;
const MOD: &str = "inc_i_262_m1_fixed_forms::";
const UNBONDING_ENV: &str = "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT";

fn utxo(op: (Hash, u32), amount: u64, kind: &str) -> Utxo {
    Utxo {
        tx_hash: hex_of(&op.0),
        output_index: op.1,
        amount,
        output_type: kind.to_string(),
        lock_until: 0,
        height: 0,
        spendable: kind == "normal",
    }
}

fn decode(hex: &str) -> Transaction {
    Transaction::deserialize(&hex::decode(hex).expect("hex")).expect("decodes into core")
}

fn add_bond_hex(p: &Px, t: &Tip, genesis: &str) -> anyhow::Result<String> {
    let signer = ProducerSigner {
        keypair: &p.r.keys.honest,
        genesis_hash: genesis,
    };
    let coins = [utxo(bonder_coin(), p.r.unit() + FEE, "normal")];
    TxBuilder::build_add_bond(&coins, 1, p.r.unit(), add_bond_lock(p, t), FEE, &signer)
}

fn withdrawal_hex(p: &Px, t: &Tip, genesis: &str, network: &str) -> anyhow::Result<String> {
    let signer = ProducerSigner {
        keypair: &p.r.keys.honest,
        genesis_hash: genesis,
    };
    let bonds = [utxo(honest_bond(), p.r.unit(), "bond")];
    let req = WithdrawalRequest {
        network,
        best_height: t.height,
        bond_count: 1,
        bond_utxos: &bonds,
        total_net: p.r.unit(),
        fee_utxos: &[],
        fee: 1,
        destination: None,
    };
    TxBuilder::build_request_withdrawal(&req, &signer)
}

fn fixed_registration(p: &Px, t: &Tip) -> Transaction {
    build_signed_registration(&registration_spec(p, t), &p.newcomer, &chain_genesis(p))
        .expect("registration")
}

fn fixed_add_bond(p: &Px, t: &Tip) -> Transaction {
    decode(&add_bond_hex(p, t, &hex_of(&p.genesis())).expect("wallet AddBond"))
}

fn fixed_withdrawal(p: &Px, t: &Tip) -> Transaction {
    decode(
        &withdrawal_hex(p, t, &hex_of(&p.genesis()), "devnet").expect("wallet RequestWithdrawal"),
    )
}

const FIXED: [(&str, Build); 3] = [
    ("Registration", fixed_registration),
    ("AddBond", fixed_add_bond),
    ("RequestWithdrawal", fixed_withdrawal),
];

fn signer_of(p: &Px, kind: &str) -> KeyPair {
    if kind == "Registration" {
        p.newcomer.clone()
    } else {
        p.r.keys.honest.clone()
    }
}

async fn refused(builds: &[(&str, Build)]) -> (usize, Vec<String>) {
    let mut admitted = 0;
    let mut bad = Vec::new();
    for (kind, build) in builds {
        let mut all = true;
        for (path, v, tx) in all_paths(*build, AT, true).await {
            all &= v.accepted;
            if !v.accepted {
                bad.push(format!("{kind} on {path}: {}", v.text));
            }
            if *kind == "RequestWithdrawal"
                && !one_unbonding_to(&tx, fixed_keys().honest.public_key())
            {
                bad.push(format!(
                    "{kind} on {path}: not one Unbonding output to the bond owner"
                ));
            }
        }
        admitted += usize::from(all);
    }
    (admitted, bad)
}

// REQ-DEF-001 — Decision: a failure means a desktop-wallet entry point keeps its own payload or signing code, so its tx differs from the one tested implementation the CLI uses.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_fixed_forms_equal_shared_builder() {
    if !in_child() {
        return run_child(
            &format!("{MOD}inc_i_262_m1_req_def_001_fixed_forms_equal_shared_builder"),
            &env_at(0),
        );
    }
    let p = rig().await;
    let t = tip(&p).await;
    let mut bad = Vec::new();
    for ((kind, fixed), (_, reference)) in FIXED.iter().zip(REFERENCE.iter()) {
        let got = fixed(&p, &t);
        if got != reference(&p, &t) {
            bad.push(format!(
                "{kind}: differs from the shared builder + finalize"
            ));
        }
        let unsigned = bad_inputs(&got, &signer_of(&p, kind));
        if !unsigned.is_empty() {
            bad.push(format!(
                "{kind}: inputs {unsigned:?} fail the per-input signature check"
            ));
        }
    }
    assert!(bad.is_empty(), "{bad:?}");
}

// REQ-DEF-001 — Decision: the outcome metric; below 3 means a desktop-wallet producer tx is still refused at the producer-auth gate.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_fixed_forms_admitted_at_gate() {
    if !in_child() {
        return run_child(
            &format!("{MOD}inc_i_262_m1_req_def_001_fixed_forms_admitted_at_gate"),
            &env_at(0),
        );
    }
    let (admitted, bad) = refused(&FIXED).await;
    println!("GOLDEN_OBSERVED INC_I_262_FIXED_ADMITTED={admitted}");
    assert!(
        bad.is_empty() && admitted == 3,
        "admitted {admitted}/3:\n{}",
        bad.join("\n")
    );
}

// REQ-DEF-001 — Decision: a failure means the wallet withdrawal built for the block at exactly the unbonding gate lacks the single Unbonding output the node requires there.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_fixed_withdrawal_at_unbonding_gate() {
    if !in_child() {
        let mut env = env_at(0);
        env.push((UNBONDING_ENV, AT.to_string()));
        return run_child(
            &format!("{MOD}inc_i_262_m1_req_def_001_fixed_withdrawal_at_unbonding_gate"),
            &env,
        );
    }
    assert_eq!(
        doli_cli::unbonding_tx::builder_params(Network::Devnet)
            .inc_i_257_unbonding_activation_height,
        AT,
        "PRECONDITION: unbonding gate at the block height"
    );
    let (_, bad) = refused(&FIXED[2..]).await;
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

// REQ-DEF-001 — Decision: a failure means a node below the producer-auth gate refuses the GUI's signed Registration, so the always-sign rule (D1) breaks registration before activation.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_fixed_registration_admitted_below_gate() {
    if !in_child() {
        return run_child(
            &format!("{MOD}inc_i_262_m1_req_def_001_fixed_registration_admitted_below_gate"),
            &env_at(u64::MAX),
        );
    }
    let (_, bad) = refused(&FIXED[..1]).await;
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

// REQ-DEF-001 — Decision: a failure means the wallet signs over a genesis it did not parse strictly, or for a network it does not know, instead of aborting the build.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_wallet_malformed_genesis_refused() {
    let p = rig().await;
    let t = tip(&p).await;
    let good = hex_of(&p.genesis());
    assert!(
        add_bond_hex(&p, &t, &good).is_ok(),
        "NON-VACUITY: the well-formed genesis builds"
    );
    assert!(
        withdrawal_hex(&p, &t, &good, "devnet").is_ok(),
        "NON-VACUITY: devnet builds"
    );
    let mut bad = Vec::new();
    for g in [
        String::new(),
        "zz".to_string(),
        good[..62].to_string(),
        "g".repeat(64),
    ] {
        if let Ok(h) = add_bond_hex(&p, &t, &g) {
            bad.push(format!(
                "AddBond built with genesis {g:?}: {} hex chars",
                h.len()
            ));
        }
        if let Ok(h) = withdrawal_hex(&p, &t, &g, "devnet") {
            bad.push(format!(
                "RequestWithdrawal built with genesis {g:?}: {} hex chars",
                h.len()
            ));
        }
    }
    if withdrawal_hex(&p, &t, &good, "moonnet").is_ok() {
        bad.push("RequestWithdrawal built for an unknown network".to_string());
    }
    assert!(bad.is_empty(), "{bad:?}");
}
