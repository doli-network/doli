//! INC-I-237 M5 (AUDIT-P2-510) — attestations cannot take the consensus-lane
//! room a block needs: an attestation-only headroom by slots and by bytes.
//!
//! OUTPUT CONTRACT: fn EventSink::emit(NewAttestation | PeerConnected) /
//!   emit_block(NewBlock) -> EmitOutcome with LaneBudgets.{attest_headroom_slots,
//!   attest_headroom_bytes}; fn LaneBudgets::for_max_peers; contract doc §1f
//!   O3: EmitOutcome   O5: LaneSnapshot {consensus_len, consensus_bytes,
//!   shed_by_topic.attestations}   O1, O2, O4, O6: none
//! PATHS: P1 attestation below cap - headroom -> Emitted; P2 at or above ->
//!   Shed + counted as attestations; P3 block into the headroom -> Emitted;
//!   P4 control event into the headroom -> Emitted
//! INPUT PARTITIONS: bound {slots 64 / 8, bytes 16 MiB / 4 MiB with 1 MiB
//!   attestations and a 2 MB block}; production max_peers {1, 8, 50, 125,
//!   1,000, 5,000}
//! MATRIX: O3 x P1-P4 per bound, O5 x P2, production bounds — asserted below.

use libp2p::PeerId;

use super::inc_i_237_m15_fixtures::{att_event, block_event, roomy};
use crate::service::backpressure::GossipShedMetrics;
use crate::service::event_sink::{event_lanes, EmitOutcome, LaneBudgets};
use crate::service::types::NetworkEvent;

fn block_with_payload(bytes: usize) -> NetworkEvent {
    let NetworkEvent::NewBlock(mut b, p, h) = block_event() else {
        unreachable!("block_event builds a NewBlock")
    };
    let mut tx = doli_core::Transaction::new_transfer(vec![], vec![]);
    tx.extra_data = vec![0xB7; bytes];
    b.transactions.push(tx);
    NetworkEvent::NewBlock(b, p, h)
}

// REQ-GRV-017 / REQ-GRV-009 (AUDIT-P2-510) — Decision: a failure means a burst of attestations fills the consensus lane by count and the next candidate block is shed locally (21 peers x 200-token bursts reach the 4,096 cap).
#[test]
fn m5_audit_p2_510_attestations_cannot_fill_the_block_headroom() {
    let b = LaneBudgets {
        consensus_cap: 64,
        attest_headroom_slots: 8,
        ..roomy()
    };
    let (sink, _lanes) = event_lanes(b);
    let m = GossipShedMetrics::new();
    let out: Vec<EmitOutcome> = (0..64).map(|_| sink.emit(att_event(32))).collect();
    let emitted = out
        .iter()
        .take_while(|o| **o == EmitOutcome::Emitted)
        .count();
    assert_eq!(emitted, 56, "attestations stop at cap - headroom");
    assert!(out[56..].iter().all(|o| *o == EmitOutcome::Shed), "{out:?}");
    assert_eq!(
        sink.stats().snapshot().shed_by_topic.attestations,
        8,
        "every refusal is counted"
    );
    let block = sink.emit_block(block_event(), &m);
    assert_eq!(
        block,
        EmitOutcome::Emitted,
        "AUDIT-P2-510: the block was shed"
    );
    let control = sink.emit(NetworkEvent::PeerConnected(PeerId::random()));
    assert_eq!(
        control,
        EmitOutcome::Emitted,
        "control events still use the headroom"
    );
    assert_eq!(sink.stats().snapshot().consensus_len, 58);
}

// REQ-GRV-017 / REQ-GRV-009 (AUDIT-P2-510) — Decision: a failure means large (padded) attestations fill the consensus lane by bytes and a full-size block no longer fits (~31 x 2 MB mutants shed the candidate).
#[test]
fn m5_audit_p2_510_attestation_bytes_cannot_fill_the_block_byte_headroom() {
    let (budget, headroom) = (16usize << 20, 4usize << 20);
    let b = LaneBudgets {
        consensus_cap: 64,
        consensus_bytes: budget,
        attest_headroom_slots: 0,
        attest_headroom_bytes: headroom,
        ..roomy()
    };
    let (sink, _lanes) = event_lanes(b);
    let m = GossipShedMetrics::new();
    let out: Vec<EmitOutcome> = (0..16).map(|_| sink.emit(att_event(1 << 20))).collect();
    assert!(
        out.contains(&EmitOutcome::Shed),
        "byte headroom never engaged: {out:?}"
    );
    let used = sink.stats().snapshot().consensus_bytes;
    assert!(
        used + headroom <= budget,
        "attestations hold {used} B of the reserved {headroom} B"
    );
    let block = sink.emit_block(block_with_payload(2_000_000), &m);
    assert_eq!(
        block,
        EmitOutcome::Emitted,
        "AUDIT-P2-510: a 2 MB block was shed"
    );
}

// REQ-GRV-017 (AUDIT-P2-510) — Decision: a failure means the production reserve is smaller than a block burst (2.5 min at the global block cap, 4 x BASE_BLOCK_SIZE) or swallows the whole lane so no attestation is ever admitted.
#[test]
fn m5_audit_p2_510_production_headroom_fits_block_bursts_and_leaves_attestation_room() {
    let block_bytes = 4 * doli_core::consensus::BASE_BLOCK_SIZE;
    for peers in [1, 8, 50, 125, 1_000, 5_000] {
        let b = LaneBudgets::for_max_peers(peers);
        assert!(b.attest_headroom_slots >= 256, "{peers}: {b:?}");
        assert!(b.attest_headroom_slots < b.consensus_cap, "{peers}: {b:?}");
        assert!(b.attest_headroom_bytes >= block_bytes, "{peers}: {b:?}");
        assert!(
            b.attest_headroom_bytes < b.consensus_bytes,
            "{peers}: {b:?}"
        );
    }
}
