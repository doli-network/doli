//! INC-I-237 M12.3 outcome probe (REL-574-R1-01) — one gossip message carrying
//! 1,000 distinct DelegateBonds signed by one registered key, all mined in one
//! canonical block filled to the builder budget, where only the mined check refuses them.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(corpus) on the probe node
//!   O1: none   O2: probe node mempool (no corpus tx resident)   O3: Verdict
//!   O4: none   O5: stdout `M12.3-PROBE-SETUP block_bytes=<n> block_txs=<n>` and
//!   `M12.3-PROBE handle_ms=<t> body_reads=<n>`   O6: MinedCheck::body_reads() delta over the call
//! PATHS: P1 control (unmined D -> P) Accept + resident, then evicted; P2 measured:
//!   the 1,000-tx message -> Ignore, none resident, body reads within the burst
//! INPUT PARTITIONS: delegator D {registered, active, 50 bonds, never delegated};
//!   delegates {20 registered active}; corpus {20 delegates x bond_count 1..=50,
//!   deterministic signer, distinct txids}; block {corpus last, filler transfers up
//!   to the select budget at the tip}; params {testnet, tip 30_000}.
// covers: zero_flow_mined zero_flow_budget zero_flow_auth zero_flow_policy pool lib transaction governance gossip_txs error_codes

use std::collections::HashSet;
use std::time::Instant;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{LARGE_BLOCK_SELECT_BUDGET, LEGACY_BLOCK_SELECT_BUDGET};
use doli_core::Transaction;
use network::Verdict;

use super::inc_i_237_m11_common as m11;
use super::inc_i_237_m12_common::{delegate_tx, world, World};
use super::inc_i_237_m12_mined_common::reads;
use super::inc_i_237_m2_node_common as c;
use super::inc_i_237_m8_common as m8;

pub const DELEGATES: usize = 20;
pub const BOND_COUNTS: u32 = 50;
pub const BURST: u64 = 2;

/// The probe world: D, its 1,000 mined DelegateBonds and the carrying block's shape.
pub struct Corpus {
    pub w: World,
    pub txs: Vec<Transaction>,
    pub block_bytes: usize,
    pub block_txs: usize,
}

impl Corpus {
    pub async fn resident(&self) -> usize {
        let mut n = 0;
        for t in &self.txs {
            if m8::has(&self.w.node, t).await {
                n += 1;
            }
        }
        n
    }

    /// One gossip message carrying the whole corpus: (verdict, body reads spent, elapsed ms).
    pub async fn deliver(&self) -> (Verdict, u64, f64) {
        let msg = self.txs.clone();
        let r0 = reads(&self.w.node).await;
        let started = Instant::now();
        let verdict = self.w.node.handle_gossip_txs(msg).await;
        let elapsed = started.elapsed();
        let spent = reads(&self.w.node).await.saturating_sub(r0);
        (verdict, spent, elapsed.as_secs_f64() * 1_000.0)
    }
}

fn filler(kp: &KeyPair, i: u32) -> Transaction {
    let mut prev = [0xF3u8; 32];
    prev[..4].copy_from_slice(&i.to_le_bytes());
    c::transfer(kp, Hash::from_bytes(prev))
}

/// D (50 bonds, undelegated), 20 active delegates, and 20 x 50 distinct D-signed DelegateBonds mined at the tip.
pub async fn distinct_corpus() -> Corpus {
    let w = world().await;
    let d = w.producer(BOND_COUNTS).await;
    let control = delegate_tx(&d, w.p.public_key(), BOND_COUNTS);
    let (cv, c_resident) = w.gossip(&control).await;
    assert!(
        cv == Verdict::Accept && c_resident,
        "setup: an unmined D-signed DelegateBond is refused by a cheaper check: verdict={cv:?} resident={c_resident}"
    );
    w.evict(&control).await;

    let mut txs = Vec::with_capacity(DELEGATES * BOND_COUNTS as usize);
    for _ in 0..DELEGATES {
        let k = w.producer(1).await;
        for n in 1..=BOND_COUNTS {
            txs.push(delegate_tx(&d, k.public_key(), n));
        }
    }
    let distinct: HashSet<Hash> = txs.iter().map(|t| t.hash()).collect();
    assert_eq!(distinct.len(), txs.len(), "setup: corpus txids distinct");

    let h = w.node.chain_state.read().await.best_height;
    let budget = if h >= w.node.config.network.params().large_block_activation_height {
        LARGE_BLOCK_SELECT_BUDGET
    } else {
        LEGACY_BLOCK_SELECT_BUDGET
    };
    let kp = KeyPair::generate();
    let mut bytes: usize = txs.iter().map(|t| t.size()).sum();
    let mut body = Vec::new();
    for i in 0u32.. {
        let f = filler(&kp, i);
        if bytes + f.size() > budget {
            break;
        }
        bytes += f.size();
        body.push(f);
    }
    body.extend(txs.iter().cloned());
    let block = m11::store_block(&w.node, h, body, true);
    assert!(
        block.size() <= w.node.params.max_block_size(h),
        "setup: block over the validation limit"
    );
    let unindexed = txs
        .iter()
        .filter(|t| w.node.block_store.get_tx_block_height(&t.hash()).unwrap() != Some(h))
        .count();
    assert_eq!(unindexed, 0, "setup: every corpus tx in the chain tx index");
    let delegated = w
        .node
        .producer_set
        .read()
        .await
        .get_by_pubkey(d.public_key())
        .and_then(|i| i.delegated_to);
    assert_eq!(delegated, None, "setup: D holds no delegation");
    let corpus = Corpus {
        block_bytes: block.size(),
        block_txs: block.transactions.len(),
        w,
        txs,
    };
    assert_eq!(corpus.resident().await, 0, "setup: no corpus tx resident");
    corpus
}

// REQ-GRV-SEC-020 (REL-574-R1-01) — Decision: a failure means one gossip message of distinct mined DelegateBonds from one key still buys one full canonical-block read per txid on the event loop.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m12_3_rel_574_r1_01_distinct_mined_probe() {
    let cp = distinct_corpus().await;
    println!(
        "M12.3-PROBE-SETUP block_bytes={} block_txs={}",
        cp.block_bytes, cp.block_txs
    );
    let (verdict, body_reads, handle_ms) = cp.deliver().await;
    println!("M12.3-PROBE handle_ms={handle_ms:.1} body_reads={body_reads}");
    let resident = cp.resident().await;
    assert!(
        verdict == Verdict::Ignore && resident == 0,
        "corpus admitted or penalised: verdict={verdict:?} resident={resident}"
    );
    assert!(
        (1..=BURST).contains(&body_reads),
        "REL-574-R1-01: {} distinct mined DelegateBonds in one message cost {body_reads} block reads (budget burst {BURST})",
        cp.txs.len()
    );
}
