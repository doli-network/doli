//! INC-I-182 M1 — RED: a third party that signs only its own coin forces the
//! full exit of a producer that is listed with bonds but owns no Bond UTXO.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings withdrawal_economics genesis register bonds withdrawal exit unbonding_tx core data block slash_withdrawal mod lib producer_tx transaction
//
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return for the forced exit (Err text); O2 residency (builder precondition)
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 transactions selected; O2 the builder's skip log line for the tx
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return for the block holding the forced exit (Err text); O2 P's withdrawal_pending_count (reported)
// PATHS: child process, holdings gate env 0, producer-auth gate env PAUTH_GATE = 2 (devnet default after the fix: 0)
//   P-MEMPOOL admission with the node's tip 2 | P-BUILDER admitted at tip-1 = 0, built at height 2
//   | P-APPLY builder block at height 2 carrying the tx, Full mode
// MATRIX
//   P-MEMPOOL x O1 -> Err with ERRTX-PAUTH001 (inc_i_182_m1_req_pauth_001_mempool_refuses_third_party_forced_exit)
//   P-BUILDER x O1, O2 -> not selected, skip log has ERRTX-PAUTH001 (inc_i_182_m1_req_pauth_001_builder_skips_third_party_forced_exit)
//   P-APPLY x O1 -> Err with ERRTX-PAUTH001 (inc_i_182_m1_req_pauth_001_apply_refuses_third_party_forced_exit)
//   {P-MEMPOOL, P-BUILDER, P-APPLY} x accepted -> count 0 (inc_i_182_m1_outcome_probe, metric PAUTH_FORCED_EXIT_ACCEPTED_PATHS)
// INPUT PARTITIONS:
//   signer: third party T only (P signs nothing); inputs: one Normal UTXO of T, no Bond input
//   P ledger: 2 bonds, 0 Bond UTXOs at its own address; declared count: full allowance (2)
//   payout: one Unbonding output to P's own address, lock far above height + period

use super::inc_i_182_m1_common::*;

const MOD: &str = "inc_i_182_m1_red_forced_exit::";

// REQ-PAUTH-001 — Decision: a failure means any fee payer can get a forced exit of another producer admitted and relayed by the mempool.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_001_mempool_refuses_third_party_forced_exit() {
    if !in_child() {
        return run_in_child(&format!(
            "{MOD}inc_i_182_m1_req_pauth_001_mempool_refuses_third_party_forced_exit"
        ));
    }
    let v = mempool_path().await;
    assert!(
        !v.accepted,
        "third-party forced exit accepted on mempool: {}",
        v.text
    );
    assert!(
        v.coded(),
        "mempool refused the forced exit without {CODE}: {}",
        v.text
    );
}

// REQ-PAUTH-001 — Decision: a failure means a producer packs a resident third-party forced exit into its block (or refuses it for a reason other than the missing producer signature).
#[tokio::test]
async fn inc_i_182_m1_req_pauth_001_builder_skips_third_party_forced_exit() {
    if !in_child() {
        return run_in_child(&format!(
            "{MOD}inc_i_182_m1_req_pauth_001_builder_skips_third_party_forced_exit"
        ));
    }
    let s = builder_path().await;
    assert!(
        s.resident,
        "PRECONDITION: the tx is resident when admitted below the gate set by {PAUTH_GATE_ENV}={PAUTH_GATE}; admission {}",
        s.admission
    );
    assert!(
        !s.v.accepted,
        "third-party forced exit accepted on builder: {}",
        s.v.text
    );
    assert!(
        s.v.coded(),
        "builder skipped the forced exit without {CODE}: {}",
        s.v.text
    );
}

// REQ-PAUTH-001 — Decision: a failure means a block that forces another producer's exit is valid, so the producer leaves the set at the next flush.
#[tokio::test]
async fn inc_i_182_m1_req_pauth_001_apply_refuses_third_party_forced_exit() {
    if !in_child() {
        return run_in_child(&format!(
            "{MOD}inc_i_182_m1_req_pauth_001_apply_refuses_third_party_forced_exit"
        ));
    }
    let v = apply_path().await;
    assert!(
        !v.accepted,
        "third-party forced exit accepted on apply: {}",
        v.text
    );
    assert!(
        v.coded(),
        "apply refused the forced exit without {CODE}: {}",
        v.text
    );
}

// REQ-PAUTH-001 — Decision: the outcome metric; a non-zero count means at least one path still lets a third party force a producer out.
#[tokio::test]
async fn inc_i_182_m1_outcome_probe() {
    if !in_child() {
        return run_in_child(&format!("{MOD}inc_i_182_m1_outcome_probe"));
    }
    let seen = [
        ("mempool", mempool_path().await),
        ("builder", builder_path().await.v),
        ("apply", apply_path().await),
    ];
    let n = seen.iter().filter(|(_, v)| v.accepted).count();
    println!("{METRIC}{n}");
    assert!(
        n == 0,
        "third-party forced exit accepted on {n} of 3 paths: {seen:?}"
    );
}
