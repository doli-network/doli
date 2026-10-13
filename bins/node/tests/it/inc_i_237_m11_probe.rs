//! INC-I-237 M11 outcome probe (AUDIT-P1-509) — on the live line
//! A(origin) -> B(node) -> C(node), after genuine AddMaintainer T reached C,
//! 20 keyless variants injected at A must not be forwarded by B, and B keeps one resident.
//!
//! OUTPUT CONTRACT: Node::handle_network_event(NetworkEvent::NewTransaction) at B and C
//!   O1: none   O2: B's mempool (maintainer-change residents)   O3: Result (ignored)
//!   O4: none   O5: stdout probe line `M11-PROBE variant_forwards=<n> middle_residents=<m>`
//!   O6: B's forwards, observed as NewTransaction arrivals at C that carry a
//!   variant (C's only peer is B)
//! PATHS: P1 genuine T at A -> admitted at B, forwarded, admitted at C;
//!   P2 20 variants at A -> refused at B, none arrive at C; P3 (non-vacuity
//!   marker) a registered genuine DelegateBond published after them -> forwarded
//!   by B, arrives at C, so B processed all 20 before the count
//! INPUT PARTITIONS: variant {reason, trailing bytes, reordered + reason,
//!   duplicated + reason} x 5 each; params {testnet, same 4-member set at B and C}.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::Transaction;

use super::inc_i_237_m11_common::{self as m11, line, publish, pump_both};
use super::inc_i_237_m12_common as m12;
use super::inc_i_237_m8_common as m8;

const PROBE_VARIANTS: u32 = 20;

// REQ-GRV-033 / REQ-GRV-SEC-011 — Decision: a failure means keyless variants of one captured maintainer change still cross an honest node (B forwards them) or pile up in its mempool (AUDIT-P1-509 on the wire).
// REQ-GRV-040 — Decision: a failure means the probe no longer measures B's forwards, so a relay regression at the middle hop would pass unseen behind C's residency.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m11_audit_p1_509_variant_flood_probe() {
    let mut l = line().await;
    let b_id = m12::peer_id(&l.b);
    let c_peers = m12::peers_of(&l.c).await;
    assert_eq!(c_peers, vec![b_id], "setup: C's only peer is B");
    let t = m8::genuine_add(&l.b, &l.m).await;
    publish(&l, &t).await;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !m8::has(&l.c, &t).await {
        assert!(
            Instant::now() < deadline,
            "setup: genuine T never reached C"
        );
        pump_both(&mut l, Duration::from_millis(500)).await;
    }
    let vs: Vec<Transaction> = (0..PROBE_VARIANTS)
        .map(|i| m11::nth_variant(&t, i))
        .collect();
    let ids: HashSet<Hash> = vs.iter().map(|v| v.hash()).collect();
    let marker = m8::genuine_delegate_bond(&[&l.b, &l.c]).await;
    let mut watch = ids.clone();
    watch.insert(marker.hash());
    for v in &vs {
        publish(&l, v).await;
    }
    publish(&l, &marker).await;
    let mut arrived = HashSet::new();
    let marker_reached =
        m12::pump_until_marker(&mut l, marker.hash(), &watch, &mut arrived, 30).await;
    let variant_forwards = arrived.intersection(&ids).count();
    let middle_residents = m11::maintainer_residents(&l.b).await;
    println!("M11-PROBE variant_forwards={variant_forwards} middle_residents={middle_residents}");
    assert!(
        marker_reached,
        "setup: the registered marker DelegateBond never reached C"
    );
    assert_eq!(
        variant_forwards, 0,
        "AUDIT-P1-509: B forwarded variants to C"
    );
    assert_eq!(middle_residents, 1, "AUDIT-P1-509: B holds variants");
}
