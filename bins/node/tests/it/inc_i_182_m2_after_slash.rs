//! INC-I-182 M2 — after a slash the producer is already out of the set
//! (Slashed), and its own signed zero-Bond exit is refused on every path by
//! the existing slash rule, never by the producer-auth rule.
//!
// covers: exit unbonding_tx producer_tx withdrawal_economics withdrawal_holdings slash_withdrawal slash_admission tx_processing pool assembly validation_checks producer_auth unbonding transaction
//
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return (Err text); O2 P's (bond_count, withdrawal_pending_count, status); O3 scheduling
// OUTPUT CONTRACT: fn Mempool::add_transaction(..) -> Result<.., MempoolError>   O4 return (Err text)
// OUTPUT CONTRACT: fn Node::build_block_content(..)   O5 transactions selected; O6 log lines naming the tx
// PATHS: child process, producer-auth gate 2, holdings gate 0, genesis blocks 0, Light apply
//   P-SLASH a block at 1 holds a valid slash of P (epoch 0: applied at once)
//   P-MEMPOOL admission at tip 2 | P-BUILDER admitted at tip 0 before the slash, built at 2 | P-APPLY block at 2
// MATRIX
//   O2, O3 x P-SLASH -> ..._after_slash_producer_is_out_of_the_set
//   O4 x P-MEMPOOL -> slash refusal text, no ERRTX-PAUTH001 (..._after_slash_mempool_refuses_with_run_b_error)
//   O5, O6 x P-BUILDER -> not selected, no ERRTX-PAUTH001 (..._after_slash_builder_never_selects_self_exit)
//   O1 x P-APPLY -> [ECON_WITHDRAWAL_SLASHED], no ERRTX-PAUTH001 (..._after_slash_apply_refuses_with_run_b_error)
// INPUT PARTITIONS:
//   P ledger 3 bonds, 0 Bond UTXOs, one Normal coin; exit: P-signed, zero Bond inputs, full allowance

use storage::ProducerStatus;

use super::inc_i_182_m2_node_fx::*;
use super::inc_i_255_m4_common::resident;

const MOD: &str = "inc_i_182_m2_after_slash::";
const RUN_B_MEMPOOL: &str = "slashed or slash-queued producer";
const RUN_B_BLOCK: &str = "ECON_WITHDRAWAL_SLASHED";

async fn slash_block(s: &mut Sx) {
    let slash = s.r.slash();
    s.r.step(vec![slash]).await;
}

async fn slashed() -> Sx {
    let mut s = sx(0).await;
    slash_block(&mut s).await;
    s
}

// REQ-PAUTH-004 — Decision: a failure means a slashed producer is still listed as able to produce or exit, so it would need its own exit path under the new rule.
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_after_slash_producer_is_out_of_the_set() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_after_slash_producer_is_out_of_the_set"
        ));
    }
    let s = slashed().await;
    let got = s.ledger().await;
    assert!(
        matches!(got, Some((_, _, ProducerStatus::Slashed { slashed_at: 1 }))),
        "after the slash block P is Slashed at 1; got {got:?}, queued {}",
        s.queued().await
    );
    assert!(!s.schedulable_next().await, "a Slashed P is not scheduled");
}

// REQ-PAUTH-004 — Decision: a failure means the mempool admits a slashed producer's exit, or refuses it with the producer-auth code (the new rule would mask the slash rule).
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_after_slash_mempool_refuses_with_run_b_error() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_after_slash_mempool_refuses_with_run_b_error"
        ));
    }
    let mut s = slashed().await;
    s.r.advance_to(GATE).await;
    let m = s.admit(&s.self_exit(Form::Signed)).await;
    assert!(
        !m.accepted && m.text.contains(RUN_B_MEMPOOL) && !m.coded(),
        "want the slash refusal ({RUN_B_MEMPOOL}) without {CODE}; {}",
        m.text
    );
}

// REQ-PAUTH-004 — Decision: a failure means a self-exit resident before the slash is packed into a block after it (an invalid block), or skipped with the producer-auth code.
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_after_slash_builder_never_selects_self_exit() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_after_slash_builder_never_selects_self_exit"
        ));
    }
    let mut s = sx(0).await;
    let tx = s.self_exit(Form::Signed);
    let before = s.admit(&tx).await;
    assert!(
        before.accepted && resident(&s.r.node, &tx).await,
        "PRECONDITION: P's signed self-exit is resident before the slash; {}",
        before.text
    );
    slash_block(&mut s).await;
    let b = s.build_next(&tx).await;
    assert!(
        !b.accepted && !b.coded(),
        "want the self-exit not selected after the slash, never for {CODE}; {}",
        b.text
    );
}

// REQ-PAUTH-004 — Decision: a failure means a block holding a slashed producer's exit is valid, or is refused by the producer-auth rule instead of the slash rule.
#[tokio::test]
async fn inc_i_182_m2_req_pauth_004_after_slash_apply_refuses_with_run_b_error() {
    if !in_child() {
        return run_child(&format!(
            "{MOD}inc_i_182_m2_req_pauth_004_after_slash_apply_refuses_with_run_b_error"
        ));
    }
    let mut s = slashed().await;
    let a = s.apply_with(&s.self_exit(Form::Signed)).await;
    assert!(
        !a.accepted && a.text.contains(RUN_B_BLOCK) && !a.coded(),
        "want {RUN_B_BLOCK} without {CODE}; {}",
        a.text
    );
}
