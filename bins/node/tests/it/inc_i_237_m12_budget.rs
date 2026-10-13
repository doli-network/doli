//! INC-I-237 M12.3 / REQ-GRV-SEC-020 (REL-574-R1-01) — the mined check's block
//! body reads are drawn from one per-node budget (2 reads/s, burst 2): an index-hit
//! DelegateBond with the budget spent is refused `MPTX017` without a read and
//! without a cache entry; cache hits and honest new DelegateBonds never spend it.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs / RPC sendTransaction on a fresh node
//!   O1: none   O2: node.mempool membership; MinedCheck body_reads delta and
//!   is_cached   O3: Verdict; RPC error_code + message   O4: none (block store and
//!   producer set are fixture inputs)   O5: none   O6: none (network = None)
//! PATHS: P1 1,000 distinct index hits in one message -> Ignore, none resident,
//!   reads within the burst; P2 index miss with the budget spent -> Accept, 0 reads;
//!   P3 cache hit with the budget spent -> MPTX014, 0 reads; P4 index hit with the
//!   budget spent -> Ignore, 0 reads, then after >= 600 ms a new index hit -> 1 read;
//!   P5 same as P4 on sendTransaction -> MPTX017, not MPTX014; P6 a budget refusal
//!   pins nothing: the same tx after >= 600 ms -> 1 read, MPTX014, pinned
//! INPUT PARTITIONS: budget {full, spent by 2 index-hit reads, refilled after
//!   600 ms}; tx {index hit not cached, index hit cached, index miss}; site
//!   {gossip, sendTransaction}; params {testnet, tip 30_000}.
// covers: zero_flow_mined zero_flow_budget zero_flow_auth zero_flow_policy pool lib transaction governance gossip_txs error_codes

use std::time::Duration;

use crypto::KeyPair;
use doli_core::Transaction;
use network::Verdict;

use super::inc_i_237_m12_budget_probe::{distinct_corpus, BURST};
use super::inc_i_237_m12_common::{delegate_tx, world, World};
use super::inc_i_237_m12_mined_common::{self as mc, cached, mine, reads};
use super::inc_i_237_m8_common as m8;

const REFILL: Duration = Duration::from_millis(600);
const BUDGET_CODE: &str = "MPTX017";
const MINED_CODE: &str = "MPTX014";

/// A fresh node, D (4 bonds, undelegated), and 4 distinct D-signed DelegateBonds each mined canonically.
async fn bench() -> (World, KeyPair, Vec<Transaction>) {
    let w = world().await;
    let d = w.producer(4).await;
    let mined = vec![
        delegate_tx(&d, w.p.public_key(), 1),
        delegate_tx(&d, w.p.public_key(), 2),
        delegate_tx(&d, w.q.public_key(), 1),
        delegate_tx(&d, w.q.public_key(), 2),
    ];
    for (i, t) in mined.iter().enumerate() {
        assert!(
            w.boundary_effect(t).await,
            "fixture: every check but the mined one passes mined[{i}]"
        );
        mine(&w.node, 601 + i as u64, std::slice::from_ref(t));
    }
    (w, d, mined)
}

/// Spend the burst: two index-hit DelegateBonds, one gossip message each, one body read each.
async fn drain(w: &World, a: &Transaction, b: &Transaction) {
    let r0 = reads(&w.node).await;
    let (va, ra) = w.gossip(a).await;
    let (vb, rb) = w.gossip(b).await;
    let spent = reads(&w.node).await - r0;
    assert!(
        va == Verdict::Ignore && vb == Verdict::Ignore && !ra && !rb && spent == BURST,
        "fixture: draining copies refused by the full mined check: verdicts={va:?},{vb:?} resident={ra},{rb} body_reads={spent}"
    );
}

// REQ-GRV-SEC-020 (REL-574-R1-01, AC1) — Decision: a failure means one gossip message of 1,000 distinct mined DelegateBonds still costs more block reads than the burst, or a refused copy is penalised or admitted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m12_3_sec_020_distinct_mined_message_reads_at_most_the_burst() {
    let cp = distinct_corpus().await;
    let (verdict, spent, handle_ms) = cp.deliver().await;
    let resident = cp.resident().await;
    assert!(
        verdict == Verdict::Ignore && resident == 0 && (1..=BURST).contains(&spent),
        "REQ-GRV-SEC-020: {} distinct mined DelegateBonds: verdict={verdict:?} resident={resident} body_reads={spent} handle_ms={handle_ms:.1}",
        cp.txs.len()
    );
}

// REQ-GRV-SEC-020 (AC guard) — Decision: a failure means an honest new DelegateBond (not in the tx index) waits on, or spends, the mined-check read budget.
#[tokio::test]
async fn m12_3_sec_020_fresh_delegate_bond_admitted_with_an_empty_bucket() {
    let (w, d, m) = bench().await;
    drain(&w, &m[0], &m[1]).await;
    let honest = delegate_tx(&d, w.p.public_key(), 3);
    assert!(
        w.boundary_effect(&honest).await,
        "fixture: the boundary applies the honest DelegateBond"
    );
    assert_eq!(
        w.node
            .block_store
            .get_tx_block_height(&honest.hash())
            .unwrap(),
        None,
        "fixture: honest DelegateBond not in the tx index"
    );
    let r0 = reads(&w.node).await;
    let (v, resident) = w.gossip(&honest).await;
    w.evict(&honest).await;
    let rpc = w.rpc(&honest).await;
    let rpc_resident = m8::has(&w.node, &honest).await;
    let spent = reads(&w.node).await - r0;
    assert!(
        v == Verdict::Accept && resident && rpc.is_ok() && rpc_resident && spent == 0,
        "REQ-GRV-SEC-020 honest DelegateBond with the budget spent: verdict={v:?} resident={resident} rpc={} rpc_resident={rpc_resident} body_reads={spent}",
        mc::text(&rpc)
    );
}

// REQ-GRV-SEC-020 (AC guard) — Decision: a failure means a txid already pinned as mined is no longer refused MPTX014 from the cache once the read budget is spent.
#[tokio::test]
async fn m12_3_sec_020_cache_hit_refused_mptx014_with_an_empty_bucket() {
    let (w, _d, m) = bench().await;
    drain(&w, &m[0], &m[1]).await;
    assert!(
        cached(&w.node, &m[0]).await,
        "fixture: the first draining copy is pinned as mined"
    );
    let r0 = reads(&w.node).await;
    let rpc = mc::text(&w.rpc(&m[0]).await);
    let rpc_resident = m8::has(&w.node, &m[0]).await;
    let (v, resident) = w.gossip(&m[0]).await;
    let spent = reads(&w.node).await - r0;
    assert!(
        rpc.contains(MINED_CODE)
            && !rpc.contains(BUDGET_CODE)
            && !rpc_resident
            && v == Verdict::Ignore
            && !resident
            && spent == 0,
        "REQ-GRV-SEC-020 cache hit with the budget spent: rpc={rpc} rpc_resident={rpc_resident} verdict={v:?} resident={resident} body_reads={spent}"
    );
}

// REQ-GRV-SEC-020 (AC2) — Decision: a failure means an index-hit DelegateBond still reads a block body with the budget spent, or the budget never refills.
#[tokio::test]
async fn m12_3_sec_020_index_hit_with_an_empty_bucket_reads_no_block_then_refills() {
    let (w, _d, m) = bench().await;
    drain(&w, &m[0], &m[1]).await;
    let mut bad = Vec::new();
    let r0 = reads(&w.node).await;
    let (v, resident) = w.gossip(&m[2]).await;
    let spent = reads(&w.node).await - r0;
    if v != Verdict::Ignore || resident || spent != 0 {
        bad.push(format!(
            "budget spent: verdict={v:?} resident={resident} body_reads={spent}"
        ));
    }
    tokio::time::sleep(REFILL).await;
    let r0 = reads(&w.node).await;
    let (v, resident) = w.gossip(&m[3]).await;
    let spent = reads(&w.node).await - r0;
    if v != Verdict::Ignore || resident || spent != 1 {
        bad.push(format!(
            "after {REFILL:?}: verdict={v:?} resident={resident} body_reads={spent}"
        ));
    }
    assert!(bad.is_empty(), "REQ-GRV-SEC-020 budget: {bad:#?}");
}

// REQ-GRV-SEC-020 (AC3) — Decision: a failure means sendTransaction reports a budget refusal as MPTX014 (a permanent "already mined") instead of the retryable MPTX017, or admits the tx.
#[tokio::test]
async fn m12_3_sec_020_send_transaction_with_an_empty_bucket_returns_mptx017() {
    let (w, _d, m) = bench().await;
    drain(&w, &m[0], &m[1]).await;
    let r0 = reads(&w.node).await;
    let res = w.rpc(&m[2]).await;
    let resident = m8::has(&w.node, &m[2]).await;
    let spent = reads(&w.node).await - r0;
    let rpc = mc::text(&res);
    assert!(
        res.is_err()
            && rpc.contains(BUDGET_CODE)
            && !rpc.contains(MINED_CODE)
            && !resident
            && spent == 0,
        "REQ-GRV-SEC-020 sendTransaction with the budget spent: rpc={rpc} resident={resident} body_reads={spent}"
    );
}

// REQ-GRV-SEC-020 (AC4) — Decision: a failure means a budget refusal is remembered as mined (pinned without a read), so a later copy is refused MPTX014 without the full check.
#[tokio::test]
async fn m12_3_sec_020_budget_refusal_leaves_no_cache_entry() {
    let (w, _d, m) = bench().await;
    drain(&w, &m[0], &m[1]).await;
    let t = &m[2];
    let mut bad = Vec::new();
    let r0 = reads(&w.node).await;
    let (v, resident) = w.gossip(t).await;
    let spent = reads(&w.node).await - r0;
    let pinned = cached(&w.node, t).await;
    if v != Verdict::Ignore || resident || spent != 0 || pinned {
        bad.push(format!(
            "budget refusal: verdict={v:?} resident={resident} body_reads={spent} pinned={pinned}"
        ));
    }
    tokio::time::sleep(REFILL).await;
    let r0 = reads(&w.node).await;
    let rpc = mc::text(&w.rpc(t).await);
    let spent = reads(&w.node).await - r0;
    let pinned = cached(&w.node, t).await;
    if !rpc.contains(MINED_CODE) || spent != 1 || !pinned {
        bad.push(format!(
            "after {REFILL:?}: rpc={rpc} body_reads={spent} pinned={pinned}"
        ));
    }
    assert!(
        bad.is_empty(),
        "REQ-GRV-SEC-020 no pin on refusal: {bad:#?}"
    );
}
