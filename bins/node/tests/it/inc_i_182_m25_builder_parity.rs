//! INC-I-182 M2.5 — the block builder selects a mempool-resident AddBond or
//! Registration at height = gate only when the producer signed it; the unsigned,
//! wrong-key and replayed forms are skipped with one code.
//!
// covers: producer_auth transaction types pool assembly tx_processing validation_checks register bonds block
//
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 whether the resident tx is selected; O2 the builder's log lines naming the tx
// PATHS: child process, producer-auth gate env 2, genesis window env 0
//   admitted judged below the gate, built at height 2 | control built at height 1
// MATRIX
//   O1, O2 x {AddBond, Registration} x signed x height 2 -> selected, log without the code
//   O1, O2 x {AddBond, Registration} x {unsigned, wrong key, replayed} x height 2 -> skipped, log names the code
//   O1 x the same forged forms x height 1 -> selected (control)
// INPUT PARTITIONS:
//   signer: producer | another registered producer | none; signature source: this tx | a tx with another input

use doli_core::transaction::TxType;

use super::inc_i_182_m1_node_fx::{env_at, in_child, run_child, Form};
use super::inc_i_182_m1_node_paths::builder_path;

const GATE: u64 = 2;
const TAG: &str = "[ERRTX-PAUTH001]";
const MOD: &str = "inc_i_182_m25_builder_parity::";

fn child(test: &str) {
    run_child(&format!("{MOD}{test}"), &env_at(GATE));
}

async fn selected(kind: TxType) {
    let v = builder_path(kind, Form::Signed, GATE).await;
    assert!(
        v.accepted && !v.text.contains(TAG),
        "{kind:?} signed must be selected at height {GATE} with no {TAG}: {}",
        v.text
    );
}

async fn skipped(kind: TxType, form: Form) {
    let v = builder_path(kind, form, GATE).await;
    assert!(
        !v.accepted && v.text.contains(TAG),
        "{kind:?} {form:?} must be skipped with {TAG} at height {GATE}: {}",
        v.text
    );
    let below = builder_path(kind, form, GATE - 1).await;
    assert!(
        below.accepted,
        "CONTROL: {kind:?} {form:?} must be selected at height {} (below the gate): {}",
        GATE - 1,
        below.text
    );
}

// REQ-PAUTH-002 — Decision: a failure means the builder drops a producer's own signed AddBond at the gate, so honest bond top-ups never reach a block.
#[tokio::test]
async fn inc_i_182_m25_builder_add_bond_signed_selected() {
    if !in_child() {
        return child("inc_i_182_m25_builder_add_bond_signed_selected");
    }
    selected(TxType::AddBond).await;
}

// REQ-PAUTH-002 — Decision: a failure means the builder puts an unsigned AddBond into a block at the gate (or skips it under another code), so it builds blocks its peers refuse.
#[tokio::test]
async fn inc_i_182_m25_builder_add_bond_unsigned_skipped() {
    if !in_child() {
        return child("inc_i_182_m25_builder_add_bond_unsigned_skipped");
    }
    skipped(TxType::AddBond, Form::Unsigned).await;
}

// REQ-PAUTH-002 — Decision: a failure means the builder accepts an AddBond signed by another producer's key at the gate, so a third party can raise a producer's allowance through the builder.
#[tokio::test]
async fn inc_i_182_m25_builder_add_bond_wrong_key_skipped() {
    if !in_child() {
        return child("inc_i_182_m25_builder_add_bond_wrong_key_skipped");
    }
    skipped(TxType::AddBond, Form::WrongKey).await;
}

// REQ-PAUTH-002 — Decision: a failure means the builder accepts an AddBond carrying a producer signature lifted from another tx, so one signature authorizes many AddBonds.
#[tokio::test]
async fn inc_i_182_m25_builder_add_bond_replayed_skipped() {
    if !in_child() {
        return child("inc_i_182_m25_builder_add_bond_replayed_skipped");
    }
    skipped(TxType::AddBond, Form::Replayed).await;
}

// REQ-PAUTH-002 — Decision: a failure means the builder drops a newcomer's own signed mempool Registration at the gate, so new producers cannot join.
#[tokio::test]
async fn inc_i_182_m25_builder_registration_signed_selected() {
    if !in_child() {
        return child("inc_i_182_m25_builder_registration_signed_selected");
    }
    selected(TxType::Registration).await;
}

// REQ-PAUTH-002 — Decision: a failure means the builder puts an unsigned mempool Registration into a block at the gate (or skips it under another code), so anyone can register another person's key through the builder.
#[tokio::test]
async fn inc_i_182_m25_builder_registration_unsigned_skipped() {
    if !in_child() {
        return child("inc_i_182_m25_builder_registration_unsigned_skipped");
    }
    skipped(TxType::Registration, Form::Unsigned).await;
}

// REQ-PAUTH-002 — Decision: a failure means the builder accepts a Registration signed by a key other than the registered one at the gate.
#[tokio::test]
async fn inc_i_182_m25_builder_registration_wrong_key_skipped() {
    if !in_child() {
        return child("inc_i_182_m25_builder_registration_wrong_key_skipped");
    }
    skipped(TxType::Registration, Form::WrongKey).await;
}

// REQ-PAUTH-002 — Decision: a failure means the builder accepts a Registration whose producer signature was lifted from another tx, so a seen signature registers the key again.
#[tokio::test]
async fn inc_i_182_m25_builder_registration_replayed_skipped() {
    if !in_child() {
        return child("inc_i_182_m25_builder_registration_replayed_skipped");
    }
    skipped(TxType::Registration, Form::Replayed).await;
}
