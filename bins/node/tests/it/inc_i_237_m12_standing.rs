//! INC-I-237 M12.1 / AUDIT-P1-515 — a signed DelegateBond / RevokeDelegation
//! that the epoch boundary (`delegate_bonds_capped` / `revoke_delegation`) would
//! turn into a no-op is refused at admission on both delegation sites: gossip
//! (`Node::handle_gossip_txs`) and RPC `sendTransaction`, with code MPTX016.
//! The two INC-I-246 stop-gap arms keep MPTX013.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict and
//!   RpcContext::handle_request(sendTransaction) -> JsonRpcResponse
//!   O1: none   O2: node.mempool membership   O3: Verdict::{Accept, Ignore};
//!   RPC Ok(hash) | Err(code, message carrying [MPTXnnn])
//!   O4: none (producer set is a fixture input; the boundary runs on copies)
//!   O5: none   O6: none (network = None)
//! PATHS: P1 DelegateBond failing one boundary precondition -> Ignore, not
//!   resident; RPC Err MPTX016, not resident; P2 RevokeDelegation with no
//!   effective delegation -> same; P3 (GUARD) revoke naming another delegate /
//!   over the received cap -> RPC Err MPTX013, not MPTX016
//! INPUT PARTITIONS: delegator {unregistered, registered inactive, registered
//!   active}; delegate {unregistered, inactive, active, = delegator};
//!   bond_count {<= available, available + 1 with 1 pending withdrawal};
//!   effective delegation {none, to another producer, to the named one};
//!   params {testnet: auth AH 0, received cap active, tip 30_000}.
//! MATRIX: every row asserts O2 x O3 on both sites; every row first asserts the
//!   boundary outcome (no-op), so a refusal is never stricter than apply.
// covers: zero_flow_policy zero_flow_auth zero_flow_newness zero_flow_standing pool lib transaction

use crypto::{Hash, KeyPair};

use super::inc_i_237_m12_common::{
    self as m12, delegate_tx, revoke_tx, world, BONDS, PARITY_CODE, STOP_GAP_CODE,
};

// REQ-GRV-SEC-012 — Decision: a failure means a DelegateBond signed by a key no node has registered is still admitted and relayed fee-free at every hop (AUDIT-P1-515).
#[tokio::test]
async fn m12_sec_012_unregistered_delegator_is_refused() {
    let w = world().await;
    let tx = delegate_tx(&KeyPair::generate(), w.p.public_key(), 1);
    let bad = w
        .refusal_failures(vec![("unregistered delegator", tx)], PARITY_CODE)
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-012: {bad:#?}");
}

// REQ-GRV-SEC-012 — Decision: a failure means an exited delegator's DelegateBond, a boundary no-op, is still admitted and relayed.
#[tokio::test]
async fn m12_sec_012_inactive_delegator_is_refused() {
    let w = world().await;
    let d = w.delegator(None).await;
    w.deactivate(&d).await;
    let tx = delegate_tx(&d, w.p.public_key(), 1);
    let bad = w
        .refusal_failures(vec![("inactive delegator", tx)], PARITY_CODE)
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-012: {bad:#?}");
}

// REQ-GRV-SEC-012 — Decision: a failure means one registered key can still mint unlimited admissible DelegateBonds to random unregistered delegates.
#[tokio::test]
async fn m12_sec_012_unregistered_delegate_is_refused() {
    let w = world().await;
    let d = w.delegator(None).await;
    let tx = delegate_tx(&d, KeyPair::generate().public_key(), 1);
    let bad = w
        .refusal_failures(vec![("unregistered delegate", tx)], PARITY_CODE)
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-012: {bad:#?}");
}

// REQ-GRV-SEC-012 — Decision: a failure means a DelegateBond to an exited delegate, a boundary no-op, is still admitted and relayed.
#[tokio::test]
async fn m12_sec_012_inactive_delegate_is_refused() {
    let w = world().await;
    let d = w.delegator(None).await;
    let gone = w.producer(1).await;
    w.deactivate(&gone).await;
    let tx = delegate_tx(&d, gone.public_key(), 1);
    let bad = w
        .refusal_failures(vec![("inactive delegate", tx)], PARITY_CODE)
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-012: {bad:#?}");
}

// REQ-GRV-SEC-012 — Decision: a failure means a self-delegation, refused at the boundary, is still admitted and relayed.
#[tokio::test]
async fn m12_sec_012_self_delegation_is_refused() {
    let w = world().await;
    let d = w.delegator(None).await;
    let tx = delegate_tx(&d, d.public_key(), 1);
    let bad = w
        .refusal_failures(vec![("self-delegation", tx)], PARITY_CODE)
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-012: {bad:#?}");
}

// REQ-GRV-SEC-012 — Decision: a failure means admission ignores the delegator's available bonds (bond_count minus pending withdrawals), so an over-count delegation the boundary refuses still relays.
#[tokio::test]
async fn m12_sec_012_bond_count_above_available_is_refused() {
    let w = world().await;
    let d = w.delegator(None).await;
    w.set_withdrawal_pending(&d, 1).await;
    let available = BONDS * 2 - 1;
    let tx = delegate_tx(&d, w.p.public_key(), available + 1);
    let fits = delegate_tx(&d, w.p.public_key(), available);
    assert!(
        w.boundary_effect(&fits).await,
        "fixture: available = bond_count - withdrawal_pending_count"
    );
    let bad = w
        .refusal_failures(vec![("bond_count = available + 1", tx)], PARITY_CODE)
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-012: {bad:#?}");
}

// REQ-GRV-SEC-012 — Decision: a failure means a delegator already delegated to Q (no revoke queued) can still relay DelegateBonds to other producers that the boundary refuses.
#[tokio::test]
async fn m12_sec_012_effective_delegation_to_another_producer_is_refused() {
    let w = world().await;
    let d = w.delegator(Some(&w.q)).await;
    let tx = delegate_tx(&d, w.p.public_key(), BONDS);
    let bad = w
        .refusal_failures(
            vec![("effective delegation to Q, DelegateBond to P", tx)],
            PARITY_CODE,
        )
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-012: {bad:#?}");
}

// REQ-GRV-SEC-013 — Decision: a failure means a RevokeDelegation signed by a key no node has registered is still admitted and relayed fee-free.
#[tokio::test]
async fn m12_sec_013_throwaway_revoke_is_refused() {
    let w = world().await;
    let tx = revoke_tx(&KeyPair::generate(), w.p.public_key());
    let bad = w
        .refusal_failures(vec![("throwaway-key revoke", tx)], PARITY_CODE)
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-013: {bad:#?}");
}

// REQ-GRV-SEC-013 — Decision: a failure means a registered producer with no applied or queued delegation can still relay revokes the boundary turns into no-ops.
#[tokio::test]
async fn m12_sec_013_revoke_without_any_delegation_is_refused() {
    let w = world().await;
    let d = w.delegator(None).await;
    let tx = revoke_tx(&d, w.p.public_key());
    let bad = w
        .refusal_failures(vec![("registered, no delegation", tx)], PARITY_CODE)
        .await;
    assert!(bad.is_empty(), "REQ-GRV-SEC-013: {bad:#?}");
}

// REQ-GRV-SEC-013 (GUARD) — Decision: a failure means the parity code swallowed the INC-I-246 stop-gap arms, so an operator can no longer tell a stale revoke or a full delegate from a key with no standing.
#[tokio::test]
async fn m12_sec_013_stop_gap_arms_keep_mptx013_on_rpc() {
    let w = world().await;
    let moved = w.delegator(Some(&w.q)).await;
    let cap = w.node.config.network.params().received_delegation_cap;
    {
        let mut ps = w.node.producer_set.write().await;
        ps.get_by_pubkey_mut(w.p.public_key())
            .expect("P registered")
            .received_delegations
            .push((Hash::ZERO, cap as u32));
    }
    let fresh = w.delegator(None).await;
    let rows = vec![
        (
            "revoke names P, effective delegate is Q",
            revoke_tx(&moved, w.p.public_key()),
        ),
        (
            "DelegateBond over P's received cap",
            delegate_tx(&fresh, w.p.public_key(), 1),
        ),
    ];
    let mut bad = Vec::new();
    for (row, tx) in rows {
        let text = match w.rpc(&tx).await {
            Ok(v) => format!("Ok({v})"),
            Err(e) => m12::error_text(&e),
        };
        if !text.contains(STOP_GAP_CODE) || text.contains(PARITY_CODE) {
            bad.push(format!("{row}: rpc={text}"));
        }
    }
    assert!(bad.is_empty(), "INC-I-246 stop-gap codes: {bad:#?}");
}
