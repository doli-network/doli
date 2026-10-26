//! INC-I-182 M1 — below the gate (and on a node built before the fix) the signed
//! forms (payload + 64 bytes) and the legacy unsigned forms are admitted,
//! selected by the builder and applied: CLI-signed txs work on old nodes.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(..) -> Result<..>      O1 return at tip 2
// OUTPUT CONTRACT: fn Node::build_block_content(..) -> Result<Option<..>>   O2 resident tx selected at 2
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode::Light) -> Result<()>   O3 return at 2
// OUTPUT CONTRACT: fn Transaction::{withdrawal_request_data, add_bond_data, registration_data}   O4 decoded payload
// PATHS: child process, producer-auth gate env u64::MAX, genesis window env 0
// MATRIX
//   O1-O3 x 3 kinds x signed -> accepted; O4 signed == unsigned
//     (inc_i_182_m1_req_pauth_003_signed_forms_accepted_below_gate)
//   O1-O3 x 3 kinds x unsigned -> accepted (inc_i_182_m1_req_pauth_003_unsigned_forms_accepted_below_gate)
// INPUT PARTITIONS:
//   form: signed by the producer (reference signer) | legacy unsigned

use doli_core::transaction::Transaction;

use super::inc_i_182_m1_node_fx::*;
use super::inc_i_182_m1_node_paths::verdicts;
use super::inc_i_255_m2_common::Keys;

const AT: u64 = 2;
const MOD: &str = "inc_i_182_m1_below_gate_signed::";

async fn refused(form: Form) -> Vec<String> {
    let mut bad = Vec::new();
    for kind in KINDS {
        for (path, v) in verdicts(kind, form, AT, true).await {
            if !v.accepted {
                bad.push(format!("{kind:?} {form:?} on {path}: {}", v.text));
            }
        }
    }
    bad
}

fn decoded(tx: &Transaction) -> String {
    format!(
        "{:?} {:?} {:?}",
        tx.withdrawal_request_data(),
        tx.add_bond_data(),
        tx.registration_data()
    )
}

// REQ-PAUTH-003 — Decision: a failure means a node without the rule (or below the gate) refuses a CLI-signed producer tx, so the CLI cannot ship before every node upgrades.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_003_signed_forms_accepted_below_gate() {
    if !in_child() {
        return run_child(
            &format!("{MOD}inc_i_182_m1_req_pauth_003_signed_forms_accepted_below_gate"),
            &env_at(u64::MAX),
        );
    }
    let p = px(&Keys::new()).await;
    let g = p.genesis();
    for kind in KINDS {
        let signed = producer_tx(&p, kind, Form::Signed, &g);
        let legacy = producer_tx(&p, kind, Form::Unsigned, &g);
        assert_eq!(
            signed.extra_data.len(),
            legacy.extra_data.len() + 64,
            "{kind:?}"
        );
        assert_eq!(
            decoded(&signed),
            decoded(&legacy),
            "{kind:?}: the tail changed a decoded field"
        );
    }
    let bad = refused(Form::Signed).await;
    assert!(
        bad.is_empty(),
        "signed forms below the gate:\n{}",
        bad.join("\n")
    );
}

// REQ-PAUTH-003 — Decision: a failure means the fix changed below-gate verdicts for today's unsigned producer txs on some path, a retroactive rule.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_003_unsigned_forms_accepted_below_gate() {
    if !in_child() {
        return run_child(
            &format!("{MOD}inc_i_182_m1_req_pauth_003_unsigned_forms_accepted_below_gate"),
            &env_at(u64::MAX),
        );
    }
    let bad = refused(Form::Unsigned).await;
    assert!(
        bad.is_empty(),
        "unsigned forms below the gate:\n{}",
        bad.join("\n")
    );
}
