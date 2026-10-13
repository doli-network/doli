//! INC-I-237 M12.2 / AUDIT-P1-516 — a zero-flow tx that is mined on the
//! canonical chain AND refused by a cheaper check costs 0 block-body reads at
//! every admission site; the mined refusal path takes no mempool write lock.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict,
//!   Node::handle_new_transaction(Transaction) -> bool, RPC sendTransaction /
//!   submitMaintainerChange -> Result<Value, RpcError>
//!   O1: none   O2: node.mempool membership; MinedCheck::body_reads() delta
//!   O3: Verdict::{Accept, Ignore}; bool; RPC error_code + message
//!   O4: none (block store and producer / maintainer sets are fixture inputs)
//!   O5: none   O6: none (network = None)
//! PATHS: P1 mined + bad signature -> MPTX010; P2 mined + non-canonical payload
//!   -> INVALID_DELEGATION; P3 (GUARD) mined + resident on gossip -> Ignore;
//!   P4 mined + resident on sendTransaction / submitMaintainerChange ->
//!   TX_ALREADY_EXISTS; P5 mined ChangeIndex variant (delegator / maintainer) ->
//!   TX_ALREADY_EXISTS; P6 mined no-effect change -> MPTX011 / MPTX016;
//!   P7 mined throwaway delegator -> MPTX016; each P1-P7 with 0 body reads;
//!   P8 mined otherwise-admissible T under a held mempool read guard ->
//!   completes, false, 1 body read, MPTX014
//! INPUT PARTITIONS: type {DelegateBond, AddMaintainer}; site {gossip,
//!   sendTransaction, submitMaintainerChange, handle_new_transaction}; residency
//!   {absent, resident}; chain index {canonical block carrying the tx};
//!   params {testnet: auth AH 0, tip 30_000}.
// covers: zero_flow_mined zero_flow_auth zero_flow_policy zero_flow_standing zero_flow_newness pool lib mod gossip_txs validation_checks network_events transaction governance error-codes rpc_reference gossip-relay-verdict-status protocol security_model

use std::time::Duration;

use crypto::KeyPair;
use doli_core::transaction::DelegateBondData;
use doli_core::Transaction;
use network::Verdict;

use super::inc_i_237_m12_common::{delegate_tx, throwaway_delegation, world, BONDS};
use super::inc_i_237_m12_mined_common::{
    self as mc, cheap_refusal_failure, gov, mine, mined_delegation, reads,
};
use super::inc_i_237_m8_common as m8;

// REQ-GRV-SEC-016 — Decision: a failure means a mined DelegateBond with a bad signature still buys a full canonical-block read on gossip or sendTransaction before auth refuses it.
#[tokio::test]
async fn m12_2_sec_016_mined_bad_signature_delegate_bond_reads_no_block() {
    let w = world().await;
    let d = w.delegator(None).await;
    let mut data = DelegateBondData::new(*d.public_key(), *w.q.public_key(), BONDS);
    data.signature =
        crypto::signature::sign_hash(&data.signing_message(), KeyPair::generate().private_key());
    let tx = Transaction::new_delegate_bond(data);
    mine(&w.node, 201, std::slice::from_ref(&tx));
    let bad = cheap_refusal_failure(&w.node, &w.rpc_ctx, "bad signature", &tx, "MPTX010").await;
    assert!(bad.is_none(), "REQ-GRV-SEC-016: {bad:?}");
}

// REQ-GRV-SEC-016 — Decision: a failure means a mined DelegateBond whose payload does not parse still buys a full canonical-block read before structural validation refuses it.
#[tokio::test]
async fn m12_2_sec_016_mined_non_canonical_delegate_bond_reads_no_block() {
    let w = world().await;
    let d = w.delegator(None).await;
    let mut tx = delegate_tx(&d, w.q.public_key(), BONDS);
    tx.extra_data.push(0x00);
    assert!(
        tx.delegate_bond_data().is_none(),
        "fixture: payload no longer parses"
    );
    mine(&w.node, 202, std::slice::from_ref(&tx));
    let bad = cheap_refusal_failure(
        &w.node,
        &w.rpc_ctx,
        "non-canonical payload",
        &tx,
        "INVALID_DELEGATION",
    )
    .await;
    assert!(bad.is_none(), "REQ-GRV-SEC-016: {bad:?}");
}

// REQ-GRV-SEC-016 (GUARD) — Decision: a failure means the reordering moved the node site's residency short-circuit behind the mined check, so a resident duplicate on gossip now pays a block read.
#[tokio::test]
async fn m12_2_sec_016_mined_resident_duplicate_on_gossip_reads_no_block() {
    let w = world().await;
    let d = w.delegator(None).await;
    let t = delegate_tx(&d, w.q.public_key(), BONDS);
    let (v, resident) = w.gossip(&t).await;
    assert!(
        v == Verdict::Accept && resident,
        "fixture: T admitted: {v:?}"
    );
    mine(&w.node, 203, std::slice::from_ref(&t));
    let len = m8::pool_len(&w.node).await;
    let before = reads(&w.node).await;
    let (v, resident) = w.gossip(&t).await;
    let spent = reads(&w.node).await - before;
    let grew = m8::pool_len(&w.node).await != len;
    assert!(
        v == Verdict::Ignore && resident && !grew && spent == 0,
        "REQ-GRV-SEC-016 gossip resident duplicate: verdict={v:?} resident={resident} pool_grew={grew} body_reads={spent}"
    );
}

// REQ-GRV-SEC-016 — Decision: a failure means sendTransaction still reads a canonical block for a tx it already holds before residency refuses it (P4).
#[tokio::test]
async fn m12_2_sec_016_mined_resident_duplicate_on_send_transaction_reads_no_block() {
    let w = world().await;
    let d = w.delegator(None).await;
    let t = delegate_tx(&d, w.q.public_key(), BONDS);
    let (v, resident) = w.gossip(&t).await;
    assert!(
        v == Verdict::Accept && resident,
        "fixture: T admitted: {v:?}"
    );
    mine(&w.node, 204, std::slice::from_ref(&t));
    let before = reads(&w.node).await;
    let rpc = mc::text(&w.rpc(&t).await);
    let spent = reads(&w.node).await - before;
    let resident = m8::has(&w.node, &t).await;
    assert!(
        rpc.contains("TX_ALREADY_EXISTS") && resident && spent == 0,
        "REQ-GRV-SEC-016 sendTransaction resident duplicate: rpc={rpc} resident={resident} body_reads={spent}"
    );
}

// REQ-GRV-SEC-016 — Decision: a failure means submitMaintainerChange still reads a canonical block for a change it already holds before residency refuses it (P4).
#[tokio::test]
async fn m12_2_sec_016_mined_resident_duplicate_on_submit_maintainer_change_reads_no_block() {
    let g = gov().await;
    let t = m8::genuine_add(&g.node, &g.m).await;
    let first = mc::submit_change(&g.rpc_ctx, &t).await;
    let txid = first
        .as_ref()
        .ok()
        .and_then(|v| v["tx_hash"].as_str().map(str::to_string));
    assert_eq!(
        txid,
        Some(t.hash().to_hex()),
        "fixture: the RPC built T byte-for-byte and admitted it: {}",
        mc::text(&first)
    );
    mine(&g.node, 205, std::slice::from_ref(&t));
    let before = reads(&g.node).await;
    let rpc = mc::text(&mc::submit_change(&g.rpc_ctx, &t).await);
    let spent = reads(&g.node).await - before;
    let resident = m8::has(&g.node, &t).await;
    assert!(
        rpc.contains("TX_ALREADY_EXISTS") && resident && spent == 0,
        "REQ-GRV-SEC-016 submitMaintainerChange resident duplicate: rpc={rpc} resident={resident} body_reads={spent}"
    );
}

// REQ-GRV-SEC-016 — Decision: a failure means a mined second DelegateBond from a delegator that already has a resident one still buys a block read before newness refuses it.
#[tokio::test]
async fn m12_2_sec_016_mined_delegator_change_index_variant_reads_no_block() {
    let w = world().await;
    let d = w.delegator(None).await;
    let held = delegate_tx(&d, w.p.public_key(), BONDS);
    let (v, resident) = w.gossip(&held).await;
    assert!(
        v == Verdict::Accept && resident,
        "fixture: first DelegateBond admitted: {v:?}"
    );
    let second = delegate_tx(&d, w.q.public_key(), BONDS);
    assert!(
        w.boundary_effect(&second).await,
        "fixture: the second one alone is boundary-applied"
    );
    mine(&w.node, 206, std::slice::from_ref(&second));
    let bad = cheap_refusal_failure(
        &w.node,
        &w.rpc_ctx,
        "second DelegateBond of a delegator",
        &second,
        "TX_ALREADY_EXISTS",
    )
    .await;
    assert!(bad.is_none(), "REQ-GRV-SEC-016: {bad:?}");
}

// REQ-GRV-SEC-016 — Decision: a failure means a mined keyless variant of a resident maintainer change still buys a block read before newness refuses it.
#[tokio::test]
async fn m12_2_sec_016_mined_maintainer_change_index_variant_reads_no_block() {
    let g = gov().await;
    let t = m8::genuine_add(&g.node, &g.m).await;
    assert_eq!(
        g.node.handle_gossip_txs(vec![t.clone()]).await,
        Verdict::Accept,
        "fixture: genuine T admitted"
    );
    let variant = super::inc_i_237_m11_common::with_reason(&t, "mined variant");
    assert_ne!(variant.hash(), t.hash(), "fixture: a new txid");
    mine(&g.node, 207, std::slice::from_ref(&variant));
    let bad = cheap_refusal_failure(
        &g.node,
        &g.rpc_ctx,
        "maintainer variant",
        &variant,
        "TX_ALREADY_EXISTS",
    )
    .await;
    assert!(bad.is_none(), "REQ-GRV-SEC-016: {bad:?}");
}

// REQ-GRV-SEC-016 — Decision: a failure means a mined AddMaintainer of a current member still buys a block read before the no-effect precondition refuses it.
#[tokio::test]
async fn m12_2_sec_016_mined_no_effect_maintainer_change_reads_no_block() {
    let g = gov().await;
    let signers: Vec<&KeyPair> = g.m.iter().take(3).collect();
    let tx = m8::signed_change(&g.node, true, *g.m[0].public_key(), &signers).await;
    mine(&g.node, 208, std::slice::from_ref(&tx));
    let bad = cheap_refusal_failure(&g.node, &g.rpc_ctx, "add a member", &tx, "MPTX011").await;
    assert!(bad.is_none(), "REQ-GRV-SEC-016: {bad:?}");
}

// REQ-GRV-SEC-016 — Decision: a failure means a mined DelegateBond that repeats the effective delegation still buys a block read before standing refuses it.
#[tokio::test]
async fn m12_2_sec_016_mined_no_effect_delegate_bond_reads_no_block() {
    let w = world().await;
    let d = w.delegator(Some(&w.p)).await;
    let tx = delegate_tx(&d, w.p.public_key(), BONDS);
    assert!(
        !w.boundary_effect(&tx).await,
        "fixture: the boundary turns it into a no-op"
    );
    mine(&w.node, 209, std::slice::from_ref(&tx));
    let bad = cheap_refusal_failure(&w.node, &w.rpc_ctx, "repeat delegation", &tx, "MPTX016").await;
    assert!(bad.is_none(), "REQ-GRV-SEC-016: {bad:?}");
}

// REQ-GRV-SEC-016 — Decision: a failure means a mined DelegateBond from an unregistered key still buys a block read before M12.1 standing refuses it.
#[tokio::test]
async fn m12_2_sec_016_mined_throwaway_delegator_reads_no_block() {
    let w = world().await;
    let tx = throwaway_delegation();
    mine(&w.node, 210, std::slice::from_ref(&tx));
    let bad =
        cheap_refusal_failure(&w.node, &w.rpc_ctx, "throwaway delegator", &tx, "MPTX016").await;
    assert!(bad.is_none(), "REQ-GRV-SEC-016: {bad:?}");
}

// REQ-GRV-SEC-016 (FM5) — Decision: a failure means the mined refusal still takes the mempool write lock, so block production and RPC readers wait behind a canonical-block read.
#[tokio::test]
async fn m12_2_sec_016_mined_refusal_completes_under_a_held_mempool_read_guard() {
    let w = world().await;
    let t = mined_delegation(&w, 211).await;
    let guard = w.node.mempool.read().await;
    let before = guard.mined_check().body_reads();
    let done = tokio::time::timeout(
        Duration::from_secs(5),
        w.node.handle_new_transaction(t.clone()),
    )
    .await;
    let spent = guard.mined_check().body_reads().saturating_sub(before);
    drop(guard);
    let resident = m8::has(&w.node, &t).await;
    let rpc = mc::text(&w.rpc(&t).await);
    assert!(
        matches!(done, Ok(false)) && spent == 1 && !resident && rpc.contains("MPTX014"),
        "REQ-GRV-SEC-016 FM5: completed={done:?} body_reads={spent} resident={resident} rpc={rpc}"
    );
}
