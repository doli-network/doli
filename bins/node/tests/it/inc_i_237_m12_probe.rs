//! INC-I-237 M12.1 outcome probe (AUDIT-P1-515) — on the live line
//! A(origin) -> B(node) -> C(node), 100 validly signed DelegateBonds from 100
//! fresh keys published at A must not be forwarded by B, and B keeps none.
//!
//! OUTPUT CONTRACT: Node::handle_network_event(NetworkEvent::NewTransaction) at B and C
//!   O1: none   O2: B's mempool (residents among the 100)   O3: Result (ignored)
//!   O4: none   O5: stdout probe line `M12.1-PROBE middle_forwards=<n> middle_residents=<m>`
//!   O6: B's forwards, observed as NewTransaction arrivals at C (C's only peer is B)
//! PATHS: P1 100 throwaway DelegateBonds at A -> refused at B, none arrive at C;
//!   P2 (non-vacuity marker) a registered genuine DelegateBond published after
//!   them -> forwarded by B, arrives at C, so B processed all 100 before the count
//! INPUT PARTITIONS: delegator / delegate {fresh KeyPair::generate() each};
//!   params {testnet, same 4-member maintainer set at B and C}.
// covers: zero_flow_policy zero_flow_auth zero_flow_newness zero_flow_standing pool lib transaction

use std::collections::HashSet;

use crypto::Hash;
use doli_core::Transaction;

use super::inc_i_237_m11_common::{line, publish};
use super::inc_i_237_m12_common::{self as m12, throwaway_delegation};
use super::inc_i_237_m8_common as m8;

const PROBE_TXS: usize = 100;

// REQ-GRV-SEC-012 / REQ-GRV-038 (AC10) — Decision: a failure means one keygen plus one signature still buys a fee-free forward at every honest hop (AUDIT-P1-515 on the wire).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m12_1_audit_p1_515_throwaway_delegation_probe() {
    let mut l = line().await;
    let b_id = m12::peer_id(&l.b);
    let c_peers = m12::peers_of(&l.c).await;
    assert_eq!(c_peers, vec![b_id], "setup: C's only peer is B");
    let txs: Vec<Transaction> = (0..PROBE_TXS).map(|_| throwaway_delegation()).collect();
    let ids: HashSet<Hash> = txs.iter().map(|t| t.hash()).collect();
    let marker = m8::genuine_delegate_bond(&[&l.b, &l.c]).await;
    let mut watch = ids.clone();
    watch.insert(marker.hash());
    for tx in &txs {
        publish(&l, tx).await;
    }
    publish(&l, &marker).await;
    let mut arrived = HashSet::new();
    let marker_reached =
        m12::pump_until_marker(&mut l, marker.hash(), &watch, &mut arrived, 30).await;
    let middle_forwards = arrived.intersection(&ids).count();
    let pool = l.b.mempool.read().await;
    let middle_residents = ids.iter().filter(|h| pool.contains(h)).count();
    drop(pool);
    println!("M12.1-PROBE middle_forwards={middle_forwards} middle_residents={middle_residents}");
    assert!(
        marker_reached,
        "setup: the registered marker DelegateBond never reached C"
    );
    assert_eq!(
        middle_forwards, 0,
        "AUDIT-P1-515: B forwarded throwaway DelegateBonds to C"
    );
    assert_eq!(
        middle_residents, 0,
        "AUDIT-P1-515: B holds throwaway DelegateBonds"
    );
}
