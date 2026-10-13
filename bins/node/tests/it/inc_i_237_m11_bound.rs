//! INC-I-237 M11 / REQ-GRV-SEC-011 — the maintainer-change lane is bounded:
//! one captured change yields at most one resident and one forward however many
//! keyless variants arrive, and residents never exceed 2 * MAX_MAINTAINERS.
//!
//! OUTPUT CONTRACT: Node::handle_gossip_txs(Vec<Transaction>) -> network::Verdict
//!   O1: none   O2: self.mempool (maintainer-change resident count)
//!   O3: Verdict::{Accept, Ignore} (Accept count after the first)
//!   O4: none   O5: none   O6: none (Accept is the forward authorization)
//! PATHS: P1 genuine T -> Accept; P2 10,000 distinct variants of T -> Ignore,
//!   residents stay 1; P3 genuine changes for many distinct targets -> residents
//!   capped at 2 * MAX_MAINTAINERS, first Add and first Remove still admitted
//! INPUT PARTITIONS: variant family {reason, trailing bytes, reordered + reason,
//!   duplicated + reason}; distinct targets {20 Adds, 4 Removes (every member)};
//!   params {testnet, 4-member set, tip past every AH}.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use crypto::KeyPair;
use doli_core::maintainer::MAX_MAINTAINERS;
use network::Verdict;

use super::inc_i_237_m11_common as m11;
use super::inc_i_237_m8_common as m8;

const VARIANTS: u32 = 10_000;

// REQ-GRV-SEC-011 — Decision: a failure means one captured maintainer change still buys an unbounded number of fee-0 residents and forwards per node.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m11_ten_thousand_variants_cost_one_resident_and_no_forward() {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let t = m8::genuine_add(&node, &m).await;
    assert_eq!(
        node.handle_gossip_txs(vec![t.clone()]).await,
        Verdict::Accept,
        "non-vacuity: genuine T relays"
    );
    let mut accepts = 0u32;
    for i in 0..VARIANTS {
        if node.handle_gossip_txs(vec![m11::nth_variant(&t, i)]).await == Verdict::Accept {
            accepts += 1;
        }
    }
    let residents = m11::maintainer_residents(&node).await;
    let t_in = m8::has(&node, &t).await;
    assert!(
        accepts == 0 && residents == 1 && t_in,
        "REQ-GRV-SEC-011: {VARIANTS} variants -> accepts={accepts} residents={residents} T_resident={t_in}"
    );
}

// REQ-GRV-SEC-011 — Decision: a failure means the fee-0 maintainer lane grows with every signed target, so historic carriers can fill it.
#[tokio::test]
async fn m11_maintainer_change_residents_never_exceed_twice_max_maintainers() {
    let (node, _kp, m, _t) = m8::auth_node().await;
    let bound = 2 * MAX_MAINTAINERS;
    let mut txs = Vec::new();
    for _ in 0..4 * MAX_MAINTAINERS {
        let target = *KeyPair::generate().public_key();
        txs.push(m8::signed_change(&node, true, target, &[&m[0], &m[1], &m[2]]).await);
    }
    for i in 0..m.len() {
        let others: Vec<&KeyPair> = m
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, k)| k)
            .collect();
        txs.push(m8::signed_change(&node, false, *m[i].public_key(), &others).await);
    }
    let first_remove = txs[4 * MAX_MAINTAINERS].clone();
    let first_add = txs[0].clone();
    let mut peak = 0;
    for tx in txs {
        node.handle_gossip_txs(vec![tx]).await;
        peak = peak.max(m11::maintainer_residents(&node).await);
    }
    let (add_in, rm_in) = (
        m8::has(&node, &first_add).await,
        m8::has(&node, &first_remove).await,
    );
    assert!(
        add_in && rm_in,
        "non-vacuity: first Add {add_in} / first Remove {rm_in} admitted"
    );
    assert!(
        peak <= bound,
        "REQ-GRV-SEC-011: maintainer-change residents peaked at {peak} > {bound}"
    );
}
