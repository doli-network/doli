//! INC-I-237 M3 (R2 under R1) — attestation verdicts under a tx flood on a live
//! A -> B -> C line: A floods txs past B's per-peer tx budget (B consumes each
//! tx event slowly, so its bulk lane stays loaded) while publishing member and
//! non-member attestations; B judges attestations decode + Ed25519 + membership.
//!
//! OUTPUT CONTRACT: swarm attestation path under bulk load
//!   O1-O5: none
//!   O6: attestation events at C (which crossed B); B's receive -> verdict
//!       latency per attestation; LATE_VERDICTS[Attestations]; B's tx events
//! PATHS: P1 member attestation under flood -> Accept on time -> at C;
//!   P2 non-member attestation under flood -> Ignore -> never at C
//! INPUT PARTITIONS: attester {member, non-member}; load {A floods txs from the
//!   SAME peer that sends the attestations}; count N = 50 per class
//! MATRIX: O6 x P1/P2 — coverage at C, verdict latency, late counter, flood
//!   non-vacuity (B received >= 100 tx events and the flood exceeded B's budget).

use std::collections::HashSet;
use std::time::Duration;

use crypto::KeyPair;
use network::gossip::staleness::GossipTopic;
use network::service::LATE_VERDICTS;
use network::NetworkCommand;

use super::inc_i_237_m2_common as h;
use super::inc_i_237_m3_fake_node::{attestation, spawn_fake3, wait_for, Fake3, Kind, Mode};

const N: u32 = 50;
const FLOOD: u64 = 3_000;

struct Run {
    members_at_c: usize,
    outsiders_at_c: usize,
    max_verdict_ms: u128,
    late: u64,
    tx_events_at_b: usize,
    stamped_at_b: bool,
}

async fn flood_run() -> Run {
    let member = KeyPair::from_seed([0x41; 32]);
    let outsider = KeyPair::from_seed([0x42; 32]);
    let members: HashSet<_> = [*member.public_key()].into();
    let b_port = h::reserve_port();
    let slow = Duration::from_millis(20);
    let b = spawn_fake3(
        h::start_node(b_port, vec![]).await,
        Mode::Judge,
        members.clone(),
        slow,
    );
    let a = spawn_fake3(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Mode::Judge,
        members.clone(),
        slow,
    );
    let c = spawn_fake3(
        h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        Mode::Judge,
        members,
        slow,
    );
    warm(&a, &c, &member).await;
    let late_before = LATE_VERDICTS.get(GossipTopic::Attestations);
    let flood_cmd = a.cmd.clone();
    tokio::spawn(async move {
        for t in 0..FLOOD {
            let _ = flood_cmd
                .send(NetworkCommand::BroadcastTransaction(h::tx(80_000_000 + t)))
                .await;
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (mut ours, mut theirs) = (Vec::new(), Vec::new());
    for i in 0..N {
        let m = attestation(&member, 0x50, 2_000 + i).to_bytes();
        let o = attestation(&outsider, 0x51, 2_000 + i).to_bytes();
        a.cmd
            .send(NetworkCommand::BroadcastAttestation(m.clone()))
            .await
            .unwrap();
        a.cmd
            .send(NetworkCommand::BroadcastAttestation(o.clone()))
            .await
            .unwrap();
        ours.push(m);
        theirs.push(o);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let at_c = |set: &[Vec<u8>]| {
        set.iter()
            .filter(|d| c.count(Kind::Attestation, d) > 0)
            .count()
    };
    wait_for(Duration::from_secs(6), || at_c(&ours) == N as usize).await;
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    let stamped_at_b = b
        .got
        .lock()
        .unwrap()
        .iter()
        .filter(|g| g.kind == Kind::Attestation && ours.contains(&g.data))
        .all(|g| g.topic.as_deref() == Some("Attestations"));
    let max_verdict_ms = b.attest_verdict_ms.lock().unwrap().iter().copied().max();
    Run {
        members_at_c: at_c(&ours),
        outsiders_at_c: at_c(&theirs),
        max_verdict_ms: max_verdict_ms.unwrap_or(0),
        late: LATE_VERDICTS.get(GossipTopic::Attestations) - late_before,
        tx_events_at_b: b.count_kind(Kind::Tx),
        stamped_at_b,
    }
}

async fn warm(a: &Fake3, c: &Fake3, member: &KeyPair) {
    for i in 0..60u32 {
        if c.count_kind(Kind::Attestation) > 0 {
            for (name, f) in [("A", a), ("C", c)] {
                assert_eq!(
                    f.peer_count().await,
                    1,
                    "topology: {name} must have one peer"
                );
            }
            return;
        }
        let w = attestation(member, 0x4F, 100 + i).to_bytes();
        a.cmd
            .send(NetworkCommand::BroadcastAttestation(w))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("setup: attestation mesh A -> B -> C never formed");
}

// F-7 / REQ-GRV-017 — Decision: a failure means a tx flood (from the same peer) delays or sheds attestation verdicts past the window, or attestations share the tx budget, so attestation relay collapses exactly when the network is under load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f_7_tx_flood_does_not_delay_attestation_verdicts() {
    let r = flood_run().await;
    assert!(
        r.tx_events_at_b >= 100,
        "non-vacuity: B received only {} tx events",
        r.tx_events_at_b
    );
    assert!(
        r.stamped_at_b,
        "attestation events must carry a verdict handle (the node verdict decides the relay)"
    );
    assert!(
        r.max_verdict_ms < 2_000,
        "attestation verdict p100 {} ms >= 2 s under the flood",
        r.max_verdict_ms
    );
    assert_eq!(r.late, 0, "late attestation verdicts under the flood");
    assert_eq!(
        r.outsiders_at_c, 0,
        "non-member attestations crossed B under the flood"
    );
}

// REQ-GRV-010 / F-9 — Decision: a failure means relay-by-verdict lowers attestation coverage under a tx flood below the M0 baseline (0.992-1.000), or coverage is reached by also relaying non-member attestations.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn req_grv_010_attestation_coverage_under_tx_flood_meets_baseline() {
    let r = flood_run().await;
    let coverage = r.members_at_c as f64 / N as f64;
    assert!(
        coverage >= 0.992,
        "coverage {coverage:.3} ({}/{N}) < M0 baseline 0.992",
        r.members_at_c
    );
    assert_eq!(
        r.outsiders_at_c, 0,
        "coverage counted non-member attestations"
    );
}
