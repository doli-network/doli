//! INC-I-237 M6 (review F1) — `AttestSeen` is bounded by the wall clock, not
//! only by block apply: with the tip frozen and the node's slot clock moving
//! 112 slots, it holds at most attesters x (ATTEST_STALE_SLOTS +
//! ATTEST_FUTURE_SLOTS + 1) entries.
//!
//! OUTPUT CONTRACT: Node::on_new_attestation(&mut self, data, source_peer) -> Verdict
//!   O2: self.attest_seen, read back through O3: a fresh empty-BLS copy for an
//!   unknown block is Ignored iff its (attester, slot) is retained
//!   O3: Verdict   O4: block_store read only   O5: none   O6: none
//! PATHS: P1 feed: slot == node wall slot, first -> Accept + insert; P2
//!   read-back at wall END, slot in [END-12, END] -> retained (Ignore);
//!   P3 read-back of a stale slot s with the clock rewound to s -> retained
//!   (Ignore) at V, absent (Accept) once pruned; P4 control member never fed
//!   slot s -> Accept (s is judged current, so P3 is observable)
//! INPUT PARTITIONS: slots {START..END-12 (100 stale), END-12..=END (13 in
//!   window)}; attesters {4 fed, 1 control}; tip {frozen at genesis}
//! MATRIX: O2 x P2-P4 (retained count <= bound; window fully retained), O3 x P1.

use doli_core::Attestation;
use network::gossip::staleness::{ATTEST_FUTURE_SLOTS, ATTEST_STALE_SLOTS};
use network::service::Verdict;
use network::PeerId;

use crypto::KeyPair;
use doli_node::node::Node;

use super::inc_i_178_m0_common::{make_node, N_SMALL};
use super::inc_i_237_m3_node_common::attestation;
use super::inc_i_237_m6_common::{is_known, set_wall, WALL};

const ATTESTERS: usize = 4;
const START: u32 = WALL;
const END: u32 = WALL + 112;
const PER_ATTESTER: usize = (ATTEST_STALE_SLOTS + ATTEST_FUTURE_SLOTS + 1) as usize;
const FEED_TAG: u8 = 0x60;
const PROBE_TAG: u8 = 0x7F;

async fn retained(node: &mut Node, kp: &KeyPair, slot: u32) -> bool {
    let probe = attestation(kp, PROBE_TAG, slot).to_bytes();
    node.on_new_attestation(probe, PeerId::random()).await == Verdict::Ignore
}

// REQ-GRV-024 — Decision: a failure means a node whose tip lags the wall clock (stall, stuck fork, long catch-up) keeps one AttestSeen entry per member per slot without bound.
#[tokio::test]
async fn m6_review_f1_attest_seen_bounded_by_wall_clock_with_frozen_tip() {
    let (mut node, producers, _t) = make_node(N_SMALL).await;
    let (attesters, control) = (&producers[..ATTESTERS], &producers[ATTESTERS]);
    let tip = node.chain_state.read().await.best_slot;
    let mut fed = Vec::new();
    for s in START..=END {
        set_wall(&mut node, s);
        for kp in attesters {
            let att: Attestation = attestation(kp, FEED_TAG, s);
            fed.push(
                node.on_new_attestation(att.to_bytes(), PeerId::random())
                    .await,
            );
        }
    }
    let mut in_window = 0;
    for s in END - ATTEST_STALE_SLOTS..=END {
        set_wall(&mut node, END);
        for kp in attesters {
            in_window += usize::from(retained(&mut node, kp, s).await);
        }
    }
    let (mut stale, mut unobservable) = (0, 0);
    for s in START..END - ATTEST_STALE_SLOTS {
        set_wall(&mut node, s);
        if retained(&mut node, control, s).await {
            unobservable += 1;
            continue;
        }
        for kp in attesters {
            stale += usize::from(retained(&mut node, kp, s).await);
        }
    }
    let n = in_window + stale;
    let bound = ATTESTERS * PER_ATTESTER;
    println!("M6_PROBE attest_seen_entries_after_100_wall_slots={n} bound={bound}");

    let unknown = |t: u8| !is_known(&node, &attestation(control, t, START).block_hash);
    assert!(
        unknown(FEED_TAG) && unknown(PROBE_TAG),
        "non-vacuity: unknown-block path"
    );
    assert_eq!(
        node.chain_state.read().await.best_slot,
        tip,
        "non-vacuity: tip frozen (no block applied, so no apply-time prune ran)"
    );
    assert!(
        fed.iter().all(|v| *v == Verdict::Accept),
        "non-vacuity: every fed attestation was first-seen when its slot was current"
    );
    assert_eq!(
        in_window,
        ATTESTERS * (ATTEST_STALE_SLOTS as usize + 1),
        "REQ-GRV-SEC-002: every (attester, slot) inside [wall-12, wall] stays retained"
    );
    assert_eq!(
        unobservable, 0,
        "stale read-back needs the node to judge slot s current once its clock (node.params) is set to s"
    );
    assert!(
        n <= bound,
        "REQ-GRV-024: AttestSeen retains {n} entries for {ATTESTERS} attesters after the wall moved 112 slots with the tip frozen ({stale} stale; bound {bound})"
    );
}
