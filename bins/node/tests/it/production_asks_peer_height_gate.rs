//! INC-I-295: the peer-height decision as one pure function, and the test seam
//! on `NetworkService` that the send proof uses.
//!
//! covers: mod gates production_asks service command_api
//
// OUTPUT CONTRACT: fn peer_height_gate(next_height, best_peer_height, gate, latch_holds) -> PeerHeightDecision
//   O3: return — Defer | Proceed. O1, O2, O4, O5, O6: none; the function is pure.
// PATHS: P1 next_height < 10: gap above 3 defers | P2 next_height-1 < gate: gap above 5
//   defers | P3 else latch_holds defers | P4 else Proceed
// INPUT PARTITIONS: grid of next_height {1, 2, 9, 10, 11, 100, gate-1, gate, gate+1, MAX} x
//   gate {0, 50, MAX} x latch_holds {false, true} x best_peer_height {0, MAX, and
//   next_height-1 plus 0, 1, 3, 4, 5, 6, 7}. next_height 0 is left out: the caller
//   passes best_height + 1. P1, P2 hold the value of `v6_46_0_defers`; P3, P4 hold
//   Defer iff latch_holds, with best_peer_height free.
// MATRIX: O3 x every grid cell. Each test also counts both results, so an empty
//   partition or a function with one constant result fails.
//
// OUTPUT CONTRACT: fn NetworkService::for_test() -> (NetworkService, Receiver<NetworkCommand>)
//   O6: a public send through the service reaches the receiver. PATH: one send.

use crypto::KeyPair;
use doli_core::consensus::ConsensusParams;
use doli_node::node::{peer_height_gate, PeerHeightDecision};
use network::{NetworkCommand, NetworkService};

use super::inc_i_289_common::block;

const GATES: [u64; 3] = [0, 50, u64::MAX];

fn next_heights(gate: u64) -> Vec<u64> {
    let mut all = vec![
        1,
        2,
        9,
        10,
        11,
        100,
        gate.saturating_sub(1),
        gate,
        gate.saturating_add(1),
        u64::MAX,
    ];
    all.retain(|h| *h >= 1);
    all
}

fn peer_heights(next_height: u64) -> Vec<u64> {
    let tip = next_height - 1;
    let mut all = vec![0, u64::MAX];
    all.extend([0, 1, 3, 4, 5, 6, 7].map(|gap| tip.saturating_add(gap)));
    all
}

/// The v6.46.0 expression of `try_produce_block`, copied so the rule stays pinned.
fn v6_46_0_defers(h: u64, tip: u64) -> bool {
    tip > h - 1 && tip - (h - 1) > if h < 10 { 3 } else { 5 }
}

fn keeps_old_rule(next_height: u64, gate: u64) -> bool {
    next_height < 10 || next_height - 1 < gate
}

// REQ-PASK-002 — Decision: a failure means an input below the gate or below height 10 now decides differently, a consensus-visible change with no activation height.
#[test]
fn peer_height_gate_matches_old_rule_below_gate() {
    let (mut defers, mut proceeds) = (0u32, 0u32);
    for gate in GATES {
        for h in next_heights(gate)
            .into_iter()
            .filter(|h| keeps_old_rule(*h, gate))
        {
            for tip in peer_heights(h) {
                for latch in [false, true] {
                    let want = if v6_46_0_defers(h, tip) {
                        defers += 1;
                        PeerHeightDecision::Defer
                    } else {
                        proceeds += 1;
                        PeerHeightDecision::Proceed
                    };
                    assert_eq!(
                        peer_height_gate(h, tip, gate, latch),
                        want,
                        "next={h} peer={tip} gate={gate} latch={latch}"
                    );
                }
            }
        }
    }
    assert!(
        defers > 0 && proceeds > 0,
        "the grid must reach both results"
    );
}

// REQ-PASK-001 — Decision: a failure means peer height can still stop a build above the gate, or the latch cannot, so a refused branch still decides what this node builds.
#[test]
fn peer_height_gate_above_gate_only_latch_defers() {
    let (mut defers, mut proceeds) = (0u32, 0u32);
    for gate in GATES {
        for h in next_heights(gate)
            .into_iter()
            .filter(|h| !keeps_old_rule(*h, gate))
        {
            for tip in peer_heights(h) {
                assert_eq!(
                    peer_height_gate(h, tip, gate, true),
                    PeerHeightDecision::Defer,
                    "next={h} peer={tip} gate={gate}: the latch defers"
                );
                defers += 1;
                assert_eq!(
                    peer_height_gate(h, tip, gate, false),
                    PeerHeightDecision::Proceed,
                    "next={h} peer={tip} gate={gate}: no latch, no stop"
                );
                proceeds += 1;
            }
        }
    }
    assert!(
        defers > 0 && proceeds > 0,
        "the grid must reach this partition"
    );
}

// REQ-PASK-004 — Decision: a failure means the seam drops commands, so a test that watches the channel for a sent block shows nothing.
#[tokio::test]
async fn network_service_seam_carries_commands() {
    let (service, mut commands) = NetworkService::for_test();
    let key = KeyPair::generate();
    let built = block(1, 1, crypto::Hash::ZERO, &key, &ConsensusParams::devnet());
    let want = built.hash();

    service
        .broadcast_block(built)
        .await
        .expect("the seam's channel is open");

    match commands.try_recv() {
        Ok(NetworkCommand::BroadcastBlock(sent)) => {
            assert_eq!(sent.hash(), want, "the sent block reaches the receiver")
        }
        other => panic!("expected BroadcastBlock, got {other:?}"),
    }
}
