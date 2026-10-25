//! INC-I-262 M1 — the desktop wallet's three producer txs, built from the GUI's
//! inputs through the shared builders and finalize, are admitted, selected and
//! applied at the producer-auth gate (and the withdrawal at the unbonding gate);
//! below the gate the signed GUI Registration is today's tx plus the 64-byte tail.
//!
// covers: producer builder producer_tx unbonding_tx
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(..) -> Result<..>   O1 return at tip 2
// OUTPUT CONTRACT: fn Node::build_block_content(..)   O2 the admitted tx is selected for height 2
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode::Light) -> Result<()>   O3 return at 2
// OUTPUT CONTRACT: the tx itself   O4 outputs (RequestWithdrawal: one Unbonding to the bond owner); extra_data = today | tail
// PATHS: producer-auth gate 0 | unbonding gate == block height | producer-auth gate u64::MAX
// MATRIX
//   O1-O4 x {Registration, AddBond, RequestWithdrawal} x gate 0 -> inc_i_262_m1_req_def_001_shared_builder_forms_admitted_at_gate
//   O2-O4 x RequestWithdrawal x unbonding gate 2 -> inc_i_262_m1_req_def_001_withdrawal_admitted_at_unbonding_gate
//   O1-O4 x Registration x gate u64::MAX -> inc_i_262_m1_req_def_001_d2_signed_registration_below_gate
// INPUT PARTITIONS: tx at height 2; stripped tail (armed-gate falsifier)

use doli_core::transaction::{sign_producer_auth, Transaction};
use doli_core::Network;

use super::inc_i_182_m1_node_fx::{env_at, fixed_keys, in_child, run_child, Px};
use super::inc_i_262_m1_node_fx::*;

const AT: u64 = 2;
const MOD: &str = "inc_i_262_m1_reference_forms::";
const UNBONDING_ENV: &str = "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT";

async fn refused_paths(builds: &[(&str, Build)], builder: bool) -> Vec<String> {
    let mut bad = Vec::new();
    for (kind, build) in builds {
        for (path, v, tx) in all_paths(*build, AT, builder).await {
            if !v.accepted {
                bad.push(format!("{kind} on {path}: {}", v.text));
            }
            if *kind == "RequestWithdrawal"
                && !one_unbonding_to(&tx, fixed_keys().honest.public_key())
            {
                bad.push(format!(
                    "{kind} on {path}: not one Unbonding output to the bond owner: {:?}",
                    tx.outputs
                ));
            }
        }
    }
    bad
}

fn stripped_add_bond(p: &Px, t: &Tip) -> Transaction {
    let mut tx = reference_add_bond(p, t);
    tx.extra_data.truncate(tx.extra_data.len() - 64);
    tx
}

// REQ-DEF-001 — Decision: a failure means the shared builders do not produce node-valid desktop-wallet producer txs at the gate, so delegating the GUI and wallet to them cannot fix REQ-DEF-001.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_shared_builder_forms_admitted_at_gate() {
    if !in_child() {
        return run_child(
            &format!("{MOD}inc_i_262_m1_req_def_001_shared_builder_forms_admitted_at_gate"),
            &env_at(0),
        );
    }
    let (armed, _) = on_mempool(stripped_add_bond, AT).await;
    assert!(
        !armed.accepted && armed.text.contains("[ERRTX-PAUTH001]"),
        "NON-VACUITY: the gate is armed; a tail-less AddBond is refused: {}",
        armed.text
    );
    let bad = refused_paths(&REFERENCE, true).await;
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

// REQ-DEF-001 — Decision: a failure means a withdrawal built by the shared builder for the block at exactly the unbonding gate is not the one-Unbonding-output form the node accepts there.
#[tokio::test]
async fn inc_i_262_m1_req_def_001_withdrawal_admitted_at_unbonding_gate() {
    if !in_child() {
        let mut env = env_at(0);
        env.push((UNBONDING_ENV, AT.to_string()));
        return run_child(
            &format!("{MOD}inc_i_262_m1_req_def_001_withdrawal_admitted_at_unbonding_gate"),
            &env,
        );
    }
    assert_eq!(
        doli_cli::unbonding_tx::builder_params(Network::Devnet)
            .inc_i_257_unbonding_activation_height,
        AT,
        "PRECONDITION: the builder reads the unbonding gate the node runs with"
    );
    let bad = refused_paths(&REFERENCE[2..], true).await;
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

// REQ-DEF-001 — Decision: a failure means signing the GUI Registration changes its payload, inputs or outputs, or a node below the gate refuses the signed form (D2).
#[tokio::test]
async fn inc_i_262_m1_req_def_001_d2_signed_registration_below_gate() {
    if !in_child() {
        return run_child(
            &format!("{MOD}inc_i_262_m1_req_def_001_d2_signed_registration_below_gate"),
            &env_at(u64::MAX),
        );
    }
    let p = rig().await;
    let t = tip(&p).await;
    let today = gui_registration_today(&p, &t);
    let signed = reference_registration(&p, &t);
    assert_eq!(
        (
            signed.version,
            signed.tx_type,
            &signed.inputs,
            &signed.outputs
        ),
        (today.version, today.tx_type, &today.inputs, &today.outputs),
        "D2: inputs (signatures included) and outputs equal the pre-fix GUI construction"
    );
    assert_eq!(
        signed.extra_data.len(),
        today.extra_data.len() + 64,
        "only 64 bytes are appended"
    );
    assert_eq!(
        &signed.extra_data[..today.extra_data.len()],
        &today.extra_data[..],
        "payload unchanged"
    );
    let mut want = today.clone();
    sign_producer_auth(&mut want, &chain_genesis(&p), &p.newcomer);
    assert_eq!(
        signed, want,
        "the tail is sign_producer_auth over the pre-fix tx"
    );
    let bad = refused_paths(&REFERENCE[..1], true).await;
    assert!(
        bad.is_empty(),
        "signed GUI Registration below the gate: {}",
        bad.join("\n")
    );
}
