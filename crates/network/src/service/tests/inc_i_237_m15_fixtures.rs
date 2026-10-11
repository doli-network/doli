//! INC-I-237 M1.5 — shared event-lane fixtures for `inc_i_237_m15_*`.
//!
//! OUTPUT CONTRACT: N/A — fixture file (builders and name helpers, no tests).
//! INPUT PARTITIONS: N/A — fixture file.

use crypto::{Hash, KeyPair};
use doli_core::{Input, Output, ProducerBloomFilter, Transaction};
use libp2p::PeerId;

use crate::protocols::{StatusResponse, SyncResponse};
use crate::service::event_sink::LaneBudgets;
use crate::service::types::NetworkEvent;
use crate::service::GossipHandle;

/// Every bound large enough that no test-irrelevant lane limit engages.
pub(super) fn roomy() -> LaneBudgets {
    LaneBudgets {
        consensus_cap: 64,
        bulk_cap: 64,
        consensus_bytes: 1 << 20,
        bulk_bytes: 1 << 20,
        backlog_bytes: 1 << 20,
        backlog_cap: 64,
    }
}

pub(super) fn tx(tag: u64) -> Transaction {
    Transaction::new_transfer(
        vec![Input::new(Hash::ZERO, 0)],
        vec![Output::normal(tag, Hash::ZERO)],
    )
}

pub(super) fn tx_event(tag: u64) -> NetworkEvent {
    NetworkEvent::NewTransaction(vec![tx(tag)], GossipHandle::now())
}

pub(super) fn tx_tag(ev: &NetworkEvent) -> u64 {
    match ev {
        NetworkEvent::NewTransaction(t, _) => t[0].outputs[0].amount,
        other => panic!("expected NewTransaction, got {other:?}"),
    }
}

pub(super) fn att_event(payload: usize) -> NetworkEvent {
    NetworkEvent::NewAttestation(vec![0xa7; payload], PeerId::random(), GossipHandle::now())
}

pub(super) fn sync_resp(session_id: u64, payload: usize) -> NetworkEvent {
    NetworkEvent::SyncResponse {
        peer_id: PeerId::random(),
        response: SyncResponse::StateChunk {
            session_id,
            body: vec![0x5c; payload],
            next_key: None,
        },
    }
}

pub(super) fn block_event() -> NetworkEvent {
    let header = doli_core::BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot: 1,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    NetworkEvent::NewBlock(
        doli_core::Block::new(header, vec![]),
        PeerId::random(),
        GossipHandle::now(),
    )
}

pub(super) fn status() -> StatusResponse {
    StatusResponse {
        version: 1,
        network_id: 1,
        genesis_hash: Hash::ZERO,
        best_height: 0,
        best_hash: Hash::ZERO,
        best_slot: 0,
        producer_pubkey: None,
        fork_id: Hash::ZERO,
    }
}

/// Every variant that can be built without a live libp2p `ResponseChannel`.
pub(super) fn constructible_events() -> Vec<NetworkEvent> {
    let p = PeerId::random();
    let h = GossipHandle::now;
    vec![
        NetworkEvent::PeerConnected(p),
        NetworkEvent::PeerDisconnected(p),
        block_event(),
        tx_event(1),
        sync_resp(1, 8),
        NetworkEvent::PeerStatus {
            peer_id: p,
            status: status(),
        },
        NetworkEvent::NetworkMismatch {
            peer_id: p,
            our_network_id: 1,
            their_network_id: 2,
        },
        NetworkEvent::GenesisMismatch { peer_id: p },
        NetworkEvent::VersionMismatch {
            peer_id: p,
            our_version: 2,
            their_version: 1,
        },
        NetworkEvent::ProducersAnnounced(vec![*KeyPair::generate().public_key()], h()),
        NetworkEvent::ProducerAnnouncementsReceived(Vec::new(), h()),
        NetworkEvent::ProducerDigestReceived {
            peer_id: p,
            digest: ProducerBloomFilter::new(100),
            handle: h(),
        },
        NetworkEvent::NewVote(b"{}".to_vec(), h()),
        att_event(32),
        NetworkEvent::TxAnnouncement {
            peer_id: p,
            hashes: vec![tx(1).hash()],
            handle: h(),
        },
        NetworkEvent::TxFetchResponse {
            peer_id: p,
            transactions: vec![tx(1)],
        },
    ]
}

/// Exhaustive on purpose: a new variant fails to compile here until classified.
pub(super) fn variant(ev: &NetworkEvent) -> &'static str {
    match ev {
        NetworkEvent::PeerConnected(_) => "PeerConnected",
        NetworkEvent::PeerDisconnected(_) => "PeerDisconnected",
        NetworkEvent::NewBlock(..) => "NewBlock",
        NetworkEvent::NewTransaction(..) => "NewTransaction",
        NetworkEvent::StatusRequest { .. } => "StatusRequest",
        NetworkEvent::SyncRequest { .. } => "SyncRequest",
        NetworkEvent::SyncResponse { .. } => "SyncResponse",
        NetworkEvent::PeerStatus { .. } => "PeerStatus",
        NetworkEvent::NetworkMismatch { .. } => "NetworkMismatch",
        NetworkEvent::GenesisMismatch { .. } => "GenesisMismatch",
        NetworkEvent::VersionMismatch { .. } => "VersionMismatch",
        NetworkEvent::ProducersAnnounced(..) => "ProducersAnnounced",
        NetworkEvent::ProducerAnnouncementsReceived(..) => "ProducerAnnouncementsReceived",
        NetworkEvent::ProducerDigestReceived { .. } => "ProducerDigestReceived",
        NetworkEvent::NewVote(..) => "NewVote",
        NetworkEvent::NewAttestation(..) => "NewAttestation",
        NetworkEvent::TxAnnouncement { .. } => "TxAnnouncement",
        NetworkEvent::TxFetchRequest { .. } => "TxFetchRequest",
        NetworkEvent::TxFetchResponse { .. } => "TxFetchResponse",
    }
}

/// Delivery label: `PC:<peer>` for PeerConnected, `SR:<session>` for a state chunk.
pub(super) fn label(ev: &NetworkEvent) -> String {
    match ev {
        NetworkEvent::PeerConnected(p) => format!("PC:{p}"),
        NetworkEvent::SyncResponse {
            response: SyncResponse::StateChunk { session_id, .. },
            ..
        } => format!("SR:{session_id}"),
        other => variant(other).to_string(),
    }
}
