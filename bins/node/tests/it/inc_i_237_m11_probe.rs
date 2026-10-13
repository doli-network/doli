//! INC-I-237 M11 outcome probe (AUDIT-P1-509) — on the live line
//! A(origin) -> B(node) -> C(node), after genuine AddMaintainer T reached C,
//! 20 keyless variants injected at A must not cross B, and B keeps one resident.
//!
//! OUTPUT CONTRACT: Node::handle_network_event(NetworkEvent::NewTransaction) at B and C
//!   O1: none   O2: B's and C's mempools   O3: Result (ignored)
//!   O4: none   O5: stdout probe line `M11-PROBE variant_forwards=<n> middle_residents=<m>`
//!   O6: B's forwards to C (observed as variants present at C)
//! PATHS: P1 genuine T at A -> admitted at B, forwarded, admitted at C;
//!   P2 20 variants at A -> refused at B, none reach C
//! INPUT PARTITIONS: variant {reason, trailing bytes, reordered + reason,
//!   duplicated + reason} x 5 each; params {testnet, same 4-member set at B and C}.
// covers: zero_flow_auth pool lib gossip_txs validation_checks mod transaction governance context verdict relay_decision init startup gossip-relay-verdict-status gossip-relay-verdict-architecture

use std::time::{Duration, Instant};

use doli_core::Transaction;

use super::inc_i_237_m11_common::{self as m11, line, publish, pump_both};
use super::inc_i_237_m8_common as m8;

const PROBE_VARIANTS: u32 = 20;

// REQ-GRV-033 / REQ-GRV-SEC-011 — Decision: a failure means keyless variants of one captured maintainer change still cross an honest node and fill the next hop's mempool (AUDIT-P1-509 on the wire).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m11_audit_p1_509_variant_flood_probe() {
    let mut l = line().await;
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
    for v in &vs {
        publish(&l, v).await;
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut variant_forwards = 0;
    while Instant::now() < deadline && variant_forwards < vs.len() {
        pump_both(&mut l, Duration::from_millis(500)).await;
        variant_forwards = 0;
        for v in &vs {
            variant_forwards += usize::from(m8::has(&l.c, v).await);
        }
    }
    let middle_residents = m11::maintainer_residents(&l.b).await;
    println!("M11-PROBE variant_forwards={variant_forwards} middle_residents={middle_residents}");
    assert_eq!(variant_forwards, 0, "AUDIT-P1-509: variants crossed B");
    assert_eq!(middle_residents, 1, "AUDIT-P1-509: B holds variants");
}
