//! INC-I-237 M12.2 / AUDIT-P1-516 — one gossip message makes one admission
//! attempt per distinct tx, keyed on the full serialized bytes; a message that
//! repeats a tx stays Ignore.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict
//!   O1: none (txs moved in)   O2: node.mempool membership / len / held bytes;
//!   MinedCheck body_reads / index_gets deltas   O3: Verdict::{Accept, Ignore}
//!   O4: none   O5: none   O6: none (network = None)
//! PATHS: P1 (GUARD) [T, T], T new -> Ignore, T admitted once; P2 1,000 copies of
//!   mined otherwise-admissible T -> Ignore, not resident, 1 body read, 0 index
//!   gets; P3 (GUARD) 1,000 copies of resident T -> Ignore, pool unchanged;
//!   P4 (GUARD) [T' , T] sharing a txid, T' with a corrupted input signature ->
//!   Ignore, the valid T admitted
//! INPUT PARTITIONS: T state {new, mined, resident}; copies {2, 1,000};
//!   tx shape {zero-flow DelegateBond, transfer with one input}; params {testnet, tip 30_000}.
// covers: zero_flow_mined zero_flow_auth zero_flow_policy zero_flow_standing zero_flow_newness pool lib mod gossip_txs validation_checks network_events transaction governance error-codes rpc_reference gossip-relay-verdict-status protocol security_model

use crypto::KeyPair;
use network::Verdict;

use super::inc_i_237_m12_common::{delegate_tx, world, BONDS};
use super::inc_i_237_m12_mined_common::{gets, mined_delegation, reads};
use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common as m8;

const COPIES: usize = 1_000;

// REQ-GRV-SEC-018 (GUARD) — Decision: a failure means the per-message dedupe changed the D5 verdict or the admission of a new tx repeated inside one message.
#[tokio::test]
async fn m12_2_sec_018_new_tx_twice_in_one_message_admitted_once_and_ignored() {
    let w = world().await;
    let d = w.delegator(None).await;
    let t = delegate_tx(&d, w.q.public_key(), BONDS);
    let before = m8::pool_len(&w.node).await;
    let v = w.node.handle_gossip_txs(vec![t.clone(), t.clone()]).await;
    let grew = m8::pool_len(&w.node).await - before;
    let resident = m8::has(&w.node, &t).await;
    assert!(
        v == Verdict::Ignore && resident && grew == 1,
        "REQ-GRV-SEC-018 [T, T]: verdict={v:?} resident={resident} pool_grew_by={grew}"
    );
}

// REQ-GRV-SEC-018 — Decision: a failure means one gossip message repeating a mined txid still runs one admission attempt per copy on the event loop.
#[tokio::test]
async fn m12_2_sec_018_one_message_of_1000_mined_copies_makes_one_attempt() {
    let w = world().await;
    let t = mined_delegation(&w, 401).await;
    let (r0, g0) = (reads(&w.node).await, gets(&w.node).await);
    let v = w.node.handle_gossip_txs(vec![t.clone(); COPIES]).await;
    let spent_reads = reads(&w.node).await - r0;
    let spent_gets = gets(&w.node).await - g0;
    let resident = m8::has(&w.node, &t).await;
    assert!(
        v == Verdict::Ignore && !resident && spent_reads == 1 && spent_gets == 0,
        "REQ-GRV-SEC-018 {COPIES} mined copies: verdict={v:?} resident={resident} body_reads={spent_reads} index_gets={spent_gets}"
    );
}

// REQ-GRV-SEC-018 (GUARD) — Decision: a failure means a message repeating a resident tx is no longer Ignore or changes the pool.
#[tokio::test]
async fn m12_2_sec_018_copies_of_a_resident_tx_in_one_message_are_ignored() {
    let w = world().await;
    let d = w.delegator(None).await;
    let t = delegate_tx(&d, w.q.public_key(), BONDS);
    let (v, resident) = w.gossip(&t).await;
    assert!(
        v == Verdict::Accept && resident,
        "fixture: T admitted: {v:?}"
    );
    let before = m8::pool_len(&w.node).await;
    let v = w.node.handle_gossip_txs(vec![t.clone(); COPIES]).await;
    let len = m8::pool_len(&w.node).await;
    let resident = m8::has(&w.node, &t).await;
    assert!(
        v == Verdict::Ignore && resident && len == before,
        "REQ-GRV-SEC-018 {COPIES} resident copies: verdict={v:?} resident={resident} pool_len {before} -> {len}"
    );
}

// REQ-GRV-SEC-018 (GUARD, INV-NETWORK-004) — Decision: a failure means the dedupe keys on the txid, so a malleated copy (same txid, corrupted input signature) placed first suppresses the valid tx behind it.
#[tokio::test]
async fn m12_2_sec_018_same_txid_different_input_signature_both_attempted() {
    let w = world().await;
    let kp = KeyPair::generate();
    c::seed(&w.node, &kp, c::prev(0x41)).await;
    let valid = c::transfer(&kp, c::prev(0x41));
    let malleated = c::malleate(&valid);
    assert_eq!(malleated.hash(), valid.hash(), "fixture: same txid");
    assert_ne!(
        malleated.serialize(),
        valid.serialize(),
        "fixture: different bytes"
    );
    let v = w
        .node
        .handle_gossip_txs(vec![malleated, valid.clone()])
        .await;
    let pool = w.node.mempool.read().await;
    let held = pool
        .iter()
        .find(|(h, _)| **h == valid.hash())
        .map(|(_, e)| e.tx.serialize());
    drop(pool);
    assert!(
        v == Verdict::Ignore && held == Some(valid.serialize()),
        "INV-NETWORK-004: verdict={v:?} valid_tx_held={}",
        held.is_some()
    );
}
