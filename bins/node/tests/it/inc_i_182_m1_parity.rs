//! INC-I-182 M1 — one producer-auth verdict on every node path: the producer's
//! signed tx is accepted by mempool admission, the block builder and block
//! apply; an unsigned, wrong-key or replayed form is refused on each with one code.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return (Err text) at tip = gate
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O2 whether the resident tx is selected at height = gate; O3 the builder's log line naming the tx
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O4 return (anyhow chain text) for a builder block at height = gate carrying the tx, Light mode
// PATHS: child process, producer-auth gate env 2, genesis window env 0 (post-genesis Registration)
//   P-MEMPOOL tip 2 | P-BUILDER admitted judged at 0, built at 2 | P-APPLY Light at 2
// MATRIX
//   O1-O4 x RequestWithdrawal x signed -> accepted (inc_i_182_m1_req_pauth_002_signed_withdrawal_accepted_on_three_paths)
//   O1-O4 x RequestWithdrawal x {unsigned, wrong key, replayed} -> refused with ERRTX-PAUTH001
//     (inc_i_182_m1_req_pauth_002_forged_withdrawal_refused_on_three_paths)
//   O1, O4 x AddBond x all forms -> inc_i_182_m1_req_pauth_002_add_bond_mempool_and_apply
//   O1, O4 x Registration x all forms -> inc_i_182_m1_req_pauth_002_registration_mempool_and_apply
// INPUT PARTITIONS:
//   signer: producer | another registered producer | none; signature source: this tx | a tx with another input
//   genesis: the node's own params.genesis_hash

use doli_core::transaction::TxType;

use super::inc_i_182_m1_node_fx::*;
use super::inc_i_182_m1_node_paths::verdicts;

const GATE: u64 = 2;
const MOD: &str = "inc_i_182_m1_parity::";

/// Signed accepted, every forged form refused with the code; returns the mismatches.
async fn mismatches(kind: TxType, forms: &[Form], builder: bool) -> Vec<String> {
    let mut bad = Vec::new();
    for &form in forms {
        for (path, v) in verdicts(kind, form, GATE, builder).await {
            let ok = if form == Form::Signed {
                v.accepted
            } else {
                !v.accepted && v.coded()
            };
            if !ok {
                bad.push(format!("{kind:?} {form:?} on {path}: {}", v.text));
            }
        }
    }
    bad
}

const ALL: [Form; 4] = [Form::Signed, Form::Unsigned, Form::WrongKey, Form::Replayed];

fn child(test: &str) {
    run_child(&format!("{MOD}{test}"), &env_at(GATE));
}

// REQ-PAUTH-002 — Decision: a failure means a producer's own signed withdrawal is refused on mempool, builder or apply, so honest withdrawals stop working once the gate arms.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_002_signed_withdrawal_accepted_on_three_paths() {
    if !in_child() {
        return child("inc_i_182_m1_req_pauth_002_signed_withdrawal_accepted_on_three_paths");
    }
    let bad = mismatches(TxType::RequestWithdrawal, &[Form::Signed], true).await;
    assert!(bad.is_empty(), "signed withdrawal:\n{}", bad.join("\n"));
}

// REQ-PAUTH-002 — Decision: a failure names a path (mempool, builder or apply) that accepts an unsigned, wrong-key or replayed withdrawal, or refuses it under another code, so third parties can force exits or paths split.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_002_forged_withdrawal_refused_on_three_paths() {
    if !in_child() {
        return child("inc_i_182_m1_req_pauth_002_forged_withdrawal_refused_on_three_paths");
    }
    let bad = mismatches(TxType::RequestWithdrawal, &FORGED, true).await;
    assert!(bad.is_empty(), "forged withdrawal:\n{}", bad.join("\n"));
}

// REQ-PAUTH-002 — Decision: a failure means a third party can raise a producer's allowance with an AddBond the producer never signed (front-running its exit), or the producer's own AddBond is refused.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_002_add_bond_mempool_and_apply() {
    if !in_child() {
        return child("inc_i_182_m1_req_pauth_002_add_bond_mempool_and_apply");
    }
    let bad = mismatches(TxType::AddBond, &ALL, false).await;
    assert!(bad.is_empty(), "add-bond:\n{}", bad.join("\n"));
}

// REQ-PAUTH-002 — Decision: a failure means anyone can register another person's key, or the newcomer's own signed registration is refused.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_002_registration_mempool_and_apply() {
    if !in_child() {
        return child("inc_i_182_m1_req_pauth_002_registration_mempool_and_apply");
    }
    let bad = mismatches(TxType::Registration, &ALL, false).await;
    assert!(bad.is_empty(), "registration:\n{}", bad.join("\n"));
}
