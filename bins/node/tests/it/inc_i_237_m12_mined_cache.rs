//! INC-I-237 M12.2 / AUDIT-P1-516 — txids refused as mined live in one bounded
//! per-node negative cache pinned to (height, block_hash): later copies cost a
//! height-index get, not a block read, at every site; a reorged-out carrier
//! re-admits; maintainer changes never pay the mined check.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs / Node::handle_new_transaction /
//!   RPC sendTransaction, and MinedCheck::is_cached
//!   O1: none   O2: node.mempool membership; MinedCheck body_reads / index_gets
//!   deltas and cache membership   O3: Verdict; bool; RPC error_code + message
//!   O4: none (block store, producer and maintainer sets are fixture inputs)
//!   O5: none   O6: none (network = None)
//! PATHS: P1 first copy of mined T -> MPTX014, 1 body read, cached; P2 1,000 later
//!   copies one per message -> Ignore each, 0 body reads, 1,000 index gets;
//!   P3 a copy via RPC or handle_new_transaction after a refusal on another site
//!   -> 0 body reads; P4 canonical index at h re-pointed to X' without T -> next
//!   copy Accept, resident, entry dropped (1 index get + 1 body read);
//!   P5 applied + mined maintainer change -> MPTX011, 0 body reads; P6 reversed
//!   change, its mined carrier -> Accept, resident, 0 body reads
//! INPUT PARTITIONS: copies {1, 1,000 one per message}; first site {gossip,
//!   sendTransaction}; later site {gossip, sendTransaction, handle_new_transaction};
//!   carrier at h {canonical X, re-pointed X'}; maintainer change {AddMaintainer,
//!   RemoveMaintainer} x {applied, applied then reversed}; params {testnet, tip 30_000}.
// covers: zero_flow_mined zero_flow_auth zero_flow_policy zero_flow_standing zero_flow_newness pool lib mod gossip_txs validation_checks network_events transaction governance error-codes rpc_reference gossip-relay-verdict-status protocol security_model

use crypto::KeyPair;
use network::Verdict;

use super::inc_i_237_m11_common as m11;
use super::inc_i_237_m12_common::world;
use super::inc_i_237_m12_mined_common::{
    self as mc, cached, cheap_refusal_failure, gets, gov, mine, mined_delegation, reads,
};
use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common as m8;

const COPIES: u64 = 1_000;

// REQ-GRV-SEC-017 — Decision: a failure means every later copy of a txid already refused as mined, one per message, still costs a full canonical-block read instead of one height-index get.
#[tokio::test]
async fn m12_2_sec_017_copies_after_a_mined_refusal_cost_index_gets_only() {
    let w = world().await;
    let t = mined_delegation(&w, 301).await;
    let r0 = reads(&w.node).await;
    let (v, resident) = w.gossip(&t).await;
    let first = reads(&w.node).await - r0;
    assert!(
        v == Verdict::Ignore && !resident && first == 1,
        "fixture: first copy refused by the full mined check: verdict={v:?} resident={resident} body_reads={first}"
    );
    assert!(
        cached(&w.node, &t).await,
        "REQ-GRV-SEC-017: MPTX014 refusal not cached"
    );
    let (r1, g1) = (reads(&w.node).await, gets(&w.node).await);
    let mut not_ignored = 0;
    for _ in 0..COPIES {
        if w.node.handle_gossip_txs(vec![t.clone()]).await != Verdict::Ignore {
            not_ignored += 1;
        }
    }
    let spent_reads = reads(&w.node).await - r1;
    let spent_gets = gets(&w.node).await - g1;
    let resident = m8::has(&w.node, &t).await;
    assert!(
        not_ignored == 0 && !resident && spent_reads == 0 && spent_gets == COPIES,
        "REQ-GRV-SEC-017: {COPIES} copies -> not_ignored={not_ignored} resident={resident} body_reads={spent_reads} index_gets={spent_gets}"
    );
}

// REQ-GRV-SEC-017 — Decision: a failure means the admission sites do not share one mined-refusal memory, so a copy refused on one site is read again from the block store on gossip, RPC or the 0xAA fetch path.
#[tokio::test]
async fn m12_2_sec_017_one_cache_serves_gossip_rpc_and_fetch() {
    let w = world().await;
    let t1 = mined_delegation(&w, 311).await;
    let t2 = mined_delegation(&w, 312).await;
    let mut bad = Vec::new();

    let r = reads(&w.node).await;
    let (v, resident) = w.gossip(&t1).await;
    let first = reads(&w.node).await - r;
    if v != Verdict::Ignore || resident || first != 1 {
        bad.push(format!(
            "T1 gossip first: verdict={v:?} resident={resident} body_reads={first}"
        ));
    }
    let r = reads(&w.node).await;
    let rpc = mc::text(&w.rpc(&t1).await);
    let fetched = w.node.handle_new_transaction(t1.clone()).await;
    let later = reads(&w.node).await - r;
    if !rpc.contains("MPTX014") || fetched || later != 0 {
        bad.push(format!(
            "T1 then rpc + fetch: rpc={rpc} fetch_admitted={fetched} body_reads={later}"
        ));
    }

    let r = reads(&w.node).await;
    let rpc = mc::text(&w.rpc(&t2).await);
    let first = reads(&w.node).await - r;
    if !rpc.contains("MPTX014") || first != 1 {
        bad.push(format!("T2 rpc first: rpc={rpc} body_reads={first}"));
    }
    let r = reads(&w.node).await;
    let (v, resident) = w.gossip(&t2).await;
    let fetched = w.node.handle_new_transaction(t2.clone()).await;
    let later = reads(&w.node).await - r;
    if v != Verdict::Ignore || resident || fetched || later != 0 {
        bad.push(format!(
            "T2 then gossip + fetch: verdict={v:?} resident={resident} fetch_admitted={fetched} body_reads={later}"
        ));
    }
    assert!(bad.is_empty(), "REQ-GRV-SEC-017 shared cache: {bad:#?}");
}

// REQ-GRV-SEC-017 (FM1) — Decision: a failure means a cached mined refusal outlives the reorg that removed its carrying block, so a tx no longer on the canonical chain is refused forever.
#[tokio::test]
async fn m12_2_sec_017_reorged_out_carrier_readmits_the_next_copy() {
    let w = world().await;
    let h = 321;
    let t = mined_delegation(&w, h).await;
    let (v, resident) = w.gossip(&t).await;
    assert!(
        v == Verdict::Ignore && !resident,
        "fixture: mined T refused: {v:?}"
    );
    assert!(
        cached(&w.node, &t).await,
        "REQ-GRV-SEC-017: MPTX014 refusal not cached"
    );
    let kp = KeyPair::generate();
    let x2 = m11::store_block(&w.node, h, vec![c::transfer(&kp, c::prev(0x33))], true);
    assert!(
        !x2.transactions.iter().any(|x| x.hash() == t.hash()),
        "fixture: X' does not carry T"
    );
    assert_eq!(
        w.node.block_store.get_tx_block_height(&t.hash()).unwrap(),
        Some(h),
        "fixture: the raw tx index still places T at h"
    );
    let (r0, g0) = (reads(&w.node).await, gets(&w.node).await);
    let (v, resident) = w.gossip(&t).await;
    let spent_reads = reads(&w.node).await - r0;
    let spent_gets = gets(&w.node).await - g0;
    let still = cached(&w.node, &t).await;
    assert!(
        v == Verdict::Accept && resident && !still && spent_gets == 1 && spent_reads == 1,
        "REQ-GRV-SEC-017 reorg: verdict={v:?} resident={resident} still_cached={still} index_gets={spent_gets} body_reads={spent_reads}"
    );
}

/// After a genuine change T applied and was mined canonically: its copy on gossip and sendTransaction.
async fn applied_copy_failure(is_add: bool) -> Option<String> {
    let g = gov().await;
    let t = if is_add {
        m8::genuine_add(&g.node, &g.m).await
    } else {
        m8::genuine_remove(&g.node, &g.m).await
    };
    assert!(
        g.node.handle_new_transaction(t.clone()).await,
        "fixture: T admitted"
    );
    m11::apply_and_mine(&g.node, &t).await;
    let tip = g.node.chain_state.read().await.best_height;
    mine(&g.node, tip, std::slice::from_ref(&t));
    let target = m11::data(&t).target;
    assert_eq!(
        m11::is_member(&g.node, &target).await,
        is_add,
        "fixture: T took effect"
    );
    let row = if is_add {
        "applied AddMaintainer"
    } else {
        "applied RemoveMaintainer"
    };
    cheap_refusal_failure(&g.node, &g.rpc_ctx, row, &t, "MPTX011").await
}

// REQ-GRV-SEC-019 — Decision: a failure means a copy of an applied, mined maintainer change still pays a block read (or is refused by MPTX014 instead of MPTX011), so the maintainer arm of the mined check was not dropped.
#[tokio::test]
async fn m12_2_sec_019_applied_maintainer_change_copy_refused_mptx011_without_block_read() {
    let mut bad = Vec::new();
    for is_add in [true, false] {
        bad.extend(applied_copy_failure(is_add).await);
    }
    assert!(bad.is_empty(), "REQ-GRV-SEC-019: {bad:#?}");
}

// REQ-GRV-SEC-019 (residual) — Decision: a failure means maintainer admission still reads the canonical block, so the documented residual (a reversed change's mined carrier is admissible again) is not what ships.
#[tokio::test]
async fn m12_2_sec_019_reversed_change_carrier_admitted_without_block_read() {
    let g = gov().await;
    let t = m8::genuine_add(&g.node, &g.m).await;
    let x = m11::data(&t).target;
    assert!(
        g.node.handle_new_transaction(t.clone()).await,
        "fixture: T admitted"
    );
    m11::apply_and_mine(&g.node, &t).await;
    let tip = g.node.chain_state.read().await.best_height;
    mine(&g.node, tip, std::slice::from_ref(&t));
    let signers: Vec<&KeyPair> = g.m.iter().take(3).collect();
    let reverse = m8::signed_change(&g.node, false, x, &signers).await;
    m11::apply_and_mine(&g.node, &reverse).await;
    assert!(
        !m11::is_member(&g.node, &x).await,
        "fixture: the change was reversed"
    );
    let h = m8::next_height(&g.node).await;
    assert!(
        m8::apply_effect(&g.node, &t, h).await,
        "fixture: T re-adds its target at the next height"
    );
    let before = reads(&g.node).await;
    let v = g.node.handle_gossip_txs(vec![t.clone()]).await;
    let spent = reads(&g.node).await - before;
    let resident = m8::has(&g.node, &t).await;
    assert!(
        v == Verdict::Accept && resident && spent == 0,
        "REQ-GRV-SEC-019 residual: verdict={v:?} resident={resident} body_reads={spent}"
    );
}
