//! INC-I-237 M12.2 outcome probe (AUDIT-P1-516) — one gossip message carrying
//! 1,000 byte-identical copies of a DelegateBond T already mined in a canonical
//! block filled to the builder budget, where only the mined check refuses T.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(vec![T; 1000]) on the probe node
//!   O1: none   O2: probe node mempool (T resident afterwards)   O3: Verdict
//!   O4: none   O5: stdout `M12.2-PROBE-SETUP block_bytes=<n> block_txs=<n>` and
//!   `M12.2-PROBE handle_ms=<t> body_reads=<n>`; `body_reads` is the probe
//!   node's MinedCheck::body_reads() after the call, read outside the timed region
//! PATHS: P1 (control, separate world, no mined block) one copy of T -> Accept,
//!   resident, so every cheaper check passes T; P2 (measured) T mined at the tip
//!   in a canonical block filled to the builder budget -> 1,000 copies in one
//!   message -> Ignore, T not resident (MPTX014 alone refuses it)
//! INPUT PARTITIONS: delegator D {registered, active, 4 bonds, delegation to P
//!   applied then revoked on the producer set}; T {D -> P, 2 bonds, validly
//!   signed}; block {T last, filler transfers up to the select budget of the
//!   node's params at the block height}; params {testnet, tip 30_000}.
// covers: zero_flow_mined zero_flow_auth zero_flow_policy zero_flow_standing pool gossip_txs validation_checks transaction governance network_events

use std::time::Instant;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{LARGE_BLOCK_SELECT_BUDGET, LEGACY_BLOCK_SELECT_BUDGET};
use doli_core::Transaction;
use network::Verdict;

use super::inc_i_237_m11_common as m11;
use super::inc_i_237_m12_common::{delegate_tx, world, World, BONDS};
use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common as m8;

const COPIES: usize = 1_000;

async fn revoked_delegation(w: &World) -> (KeyPair, Transaction) {
    let d = w.delegator(Some(&w.p)).await;
    w.node
        .producer_set
        .write()
        .await
        .revoke_delegation(d.public_key())
        .expect("fixture revoke");
    let t = delegate_tx(&d, w.p.public_key(), BONDS);
    (d, t)
}

fn filler(kp: &KeyPair, i: u32) -> Transaction {
    let mut prev = [0xF1u8; 32];
    prev[..4].copy_from_slice(&i.to_le_bytes());
    c::transfer(kp, Hash::from_bytes(prev))
}

fn fill_to_budget(t: &Transaction, budget: usize) -> Vec<Transaction> {
    let kp = KeyPair::generate();
    let mut txs = Vec::new();
    let mut bytes = t.size();
    for i in 0u32.. {
        let f = filler(&kp, i);
        if bytes + f.size() > budget {
            break;
        }
        bytes += f.size();
        txs.push(f);
    }
    txs.push(t.clone());
    txs
}

// REQ-GRV-SEC-017 / REQ-GRV-SEC-018 (AC4) — Decision: a failure means one gossip message of copies of a mined DelegateBond still costs the event loop one full canonical-block read per copy (AUDIT-P1-516).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m12_2_audit_p1_516_mined_copies_probe() {
    let control = world().await;
    let (_, ct) = revoked_delegation(&control).await;
    let (cv, c_resident) = control.gossip(&ct).await;
    assert!(
        cv == Verdict::Accept && c_resident,
        "setup: unmined T refused by a cheaper check: verdict={cv:?} resident={c_resident}"
    );

    let w = world().await;
    let (d, t) = revoked_delegation(&w).await;
    let h = w.node.chain_state.read().await.best_height;
    let budget = if h >= w.node.config.network.params().large_block_activation_height {
        LARGE_BLOCK_SELECT_BUDGET
    } else {
        LEGACY_BLOCK_SELECT_BUDGET
    };
    let block = m11::store_block(&w.node, h, fill_to_budget(&t, budget), true);
    let (block_bytes, block_txs) = (block.size(), block.transactions.len());
    println!("M12.2-PROBE-SETUP block_bytes={block_bytes} block_txs={block_txs}");
    assert_eq!(
        w.node.block_store.get_tx_block_height(&t.hash()).unwrap(),
        Some(h),
        "setup: T in the chain tx index"
    );
    assert!(
        block_bytes <= w.node.params.max_block_size(h),
        "setup: block over the validation limit"
    );
    let delegated = w
        .node
        .producer_set
        .read()
        .await
        .get_by_pubkey(d.public_key())
        .and_then(|i| i.delegated_to);
    assert_eq!(delegated, None, "setup: D's delegation revoked");
    assert!(!m8::has(&w.node, &t).await, "setup: T not resident");

    let copies = vec![t.clone(); COPIES];
    let started = Instant::now();
    let verdict = w.node.handle_gossip_txs(copies).await;
    let elapsed = started.elapsed();

    let handle_ms = elapsed.as_secs_f64() * 1_000.0;
    let body_reads = w.node.mempool.read().await.mined_check().body_reads();
    println!("M12.2-PROBE handle_ms={handle_ms:.1} body_reads={body_reads}");
    let resident = m8::has(&w.node, &t).await;
    assert!(
        verdict == Verdict::Ignore && !resident,
        "mined T admitted: verdict={verdict:?} resident={resident}"
    );
    assert_eq!(
        body_reads, 1,
        "AUDIT-P1-516: {COPIES} copies of one mined txid in one message"
    );
}
