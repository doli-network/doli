//! INC-I-248 M1 — the maintainer-state sync pair is APPENDED; the wire every deployed
//! binary speaks does not move.
//!
//! REQ-248-002 (Must) — Decision: a failure means the new pair moved an existing sync variant (old and new nodes stop snapping from each other) or an old peer decodes the new request as a different message.
//!
//! OUTPUT CONTRACT: bincode::serialize / bincode::deserialize over SyncRequest and SyncResponse;
//!   SyncResponse::type_name; SyncRequest::is_rate_governed
//!   O3 return: encoded bytes; decode result (Ok variant | Err); type_name; is_rate_governed
//!   O1/O2/O4/O5/O6: none
//! PATHS: P1 every frozen variant: frozen bytes == new bytes, both decoders accept both;
//!   P2 new request under the frozen decoder -> Err; P3 new response under the frozen
//!   decoder -> Err; P4 new variants encode at index 10 (request) and 9 (response) with the
//!   pinned field order; P5 type_name / is_rate_governed of the new variants.
//! INPUT PARTITIONS: unit / tuple / struct variants; empty and populated byte fields;
//!   Option None and Some; empty and non-empty set_bytes.
//! MATRIX: 5 paths x O3 = 5 cells.

use crypto::Hash;
use doli_core::{Block, BlockHeader};
use network::protocols::{SyncRequest, SyncResponse};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[allow(dead_code)]
enum FrozenStateSessionRefusal {
    Busy,
    ManifestExpired,
    Halted(String),
}

#[derive(Serialize, Deserialize)]
#[allow(dead_code)]
enum FrozenSyncRequest {
    GetHeaders {
        start_hash: Hash,
        max_count: u32,
    },
    GetBodies {
        hashes: Vec<Hash>,
    },
    GetBlockByHeight {
        height: u64,
    },
    GetBlockByHash {
        hash: Hash,
    },
    GetStateSnapshot {
        block_hash: Hash,
    },
    GetStateRoot {
        block_hash: Hash,
    },
    DirectAttestation {
        data: Vec<u8>,
    },
    GetHeadersByHeight {
        start_height: u64,
        max_count: u32,
    },
    GetStateManifest {
        block_hash: Hash,
    },
    GetStateChunk {
        session_id: u64,
        start_key: Option<Vec<u8>>,
        max_bytes: u32,
    },
}

#[derive(Serialize, Deserialize)]
#[allow(dead_code, clippy::large_enum_variant)]
enum FrozenSyncResponse {
    Headers(Vec<BlockHeader>),
    Bodies(Vec<Block>),
    Block(Option<Block>),
    StateSnapshot {
        block_hash: Hash,
        block_height: u64,
        chain_state: Vec<u8>,
        utxo_set: Vec<u8>,
        producer_set: Vec<u8>,
        state_root: Hash,
        #[serde(default)]
        block_header_bytes: Option<Vec<u8>>,
        #[serde(default)]
        epoch_bond_snapshot_bytes: Option<Vec<u8>>,
        #[serde(default)]
        epoch_accumulators_bytes: Option<Vec<u8>>,
        #[serde(default)]
        epoch_state_bytes: Option<Vec<u8>>,
    },
    StateRoot {
        block_hash: Hash,
        block_height: u64,
        state_root: Hash,
    },
    Error(String),
    StateManifest {
        session_id: u64,
        block_hash: Hash,
        block_height: u64,
        state_root: Hash,
        utxo_hash: Hash,
        utxo_count: u64,
        chunk_max_bytes: u32,
        chain_state: Vec<u8>,
        producer_set: Vec<u8>,
        #[serde(default)]
        block_header_bytes: Option<Vec<u8>>,
        #[serde(default)]
        epoch_bond_snapshot_bytes: Option<Vec<u8>>,
        #[serde(default)]
        epoch_accumulators_bytes: Option<Vec<u8>>,
        #[serde(default)]
        epoch_state_bytes: Option<Vec<u8>>,
    },
    StateChunk {
        session_id: u64,
        body: Vec<u8>,
        next_key: Option<Vec<u8>>,
    },
    StateSessionUnavailable {
        session_id: u64,
        reason: FrozenStateSessionRefusal,
    },
}

#[derive(Serialize)]
#[allow(dead_code)]
enum ResponseLayoutMirror {
    V0,
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8,
    MaintainerStateAt {
        block_hash: Hash,
        block_height: u64,
        digest: Hash,
        set_bytes: Vec<u8>,
        last_derived_height: u64,
    },
}

fn h(tag: u8) -> Hash {
    Hash::from_bytes([tag; 32])
}

fn enc<T: Serialize>(v: &T) -> Vec<u8> {
    bincode::serialize(v).expect("bincode serialize")
}

fn index_of(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("4-byte variant index"))
}

fn frozen_requests() -> Vec<(FrozenSyncRequest, SyncRequest)> {
    vec![
        (
            FrozenSyncRequest::GetHeaders {
                start_hash: h(1),
                max_count: 7,
            },
            SyncRequest::GetHeaders {
                start_hash: h(1),
                max_count: 7,
            },
        ),
        (
            FrozenSyncRequest::GetBodies {
                hashes: vec![h(2), h(3)],
            },
            SyncRequest::GetBodies {
                hashes: vec![h(2), h(3)],
            },
        ),
        (
            FrozenSyncRequest::GetBlockByHeight { height: 25_000 },
            SyncRequest::GetBlockByHeight { height: 25_000 },
        ),
        (
            FrozenSyncRequest::GetBlockByHash { hash: h(4) },
            SyncRequest::GetBlockByHash { hash: h(4) },
        ),
        (
            FrozenSyncRequest::GetStateSnapshot { block_hash: h(5) },
            SyncRequest::GetStateSnapshot { block_hash: h(5) },
        ),
        (
            FrozenSyncRequest::GetStateRoot { block_hash: h(6) },
            SyncRequest::GetStateRoot { block_hash: h(6) },
        ),
        (
            FrozenSyncRequest::DirectAttestation {
                data: vec![9, 8, 7],
            },
            SyncRequest::DirectAttestation {
                data: vec![9, 8, 7],
            },
        ),
        (
            FrozenSyncRequest::GetHeadersByHeight {
                start_height: 11,
                max_count: 12,
            },
            SyncRequest::GetHeadersByHeight {
                start_height: 11,
                max_count: 12,
            },
        ),
        (
            FrozenSyncRequest::GetStateManifest { block_hash: h(7) },
            SyncRequest::GetStateManifest { block_hash: h(7) },
        ),
        (
            FrozenSyncRequest::GetStateChunk {
                session_id: 3,
                start_key: Some(vec![1, 2]),
                max_bytes: 4096,
            },
            SyncRequest::GetStateChunk {
                session_id: 3,
                start_key: Some(vec![1, 2]),
                max_bytes: 4096,
            },
        ),
    ]
}

fn frozen_responses() -> Vec<(FrozenSyncResponse, SyncResponse)> {
    vec![
        (
            FrozenSyncResponse::Headers(vec![]),
            SyncResponse::Headers(vec![]),
        ),
        (
            FrozenSyncResponse::Bodies(vec![]),
            SyncResponse::Bodies(vec![]),
        ),
        (FrozenSyncResponse::Block(None), SyncResponse::Block(None)),
        (
            FrozenSyncResponse::StateSnapshot {
                block_hash: h(1),
                block_height: 25_000,
                chain_state: vec![1],
                utxo_set: vec![2, 2],
                producer_set: vec![3, 3, 3],
                state_root: h(2),
                block_header_bytes: Some(vec![4]),
                epoch_bond_snapshot_bytes: None,
                epoch_accumulators_bytes: Some(vec![]),
                epoch_state_bytes: Some(vec![5, 5]),
            },
            SyncResponse::StateSnapshot {
                block_hash: h(1),
                block_height: 25_000,
                chain_state: vec![1],
                utxo_set: vec![2, 2],
                producer_set: vec![3, 3, 3],
                state_root: h(2),
                block_header_bytes: Some(vec![4]),
                epoch_bond_snapshot_bytes: None,
                epoch_accumulators_bytes: Some(vec![]),
                epoch_state_bytes: Some(vec![5, 5]),
            },
        ),
        (
            FrozenSyncResponse::StateRoot {
                block_hash: h(3),
                block_height: 9,
                state_root: h(4),
            },
            SyncResponse::StateRoot {
                block_hash: h(3),
                block_height: 9,
                state_root: h(4),
            },
        ),
        (
            FrozenSyncResponse::Error("e".to_string()),
            SyncResponse::Error("e".to_string()),
        ),
        (
            FrozenSyncResponse::StateManifest {
                session_id: 1,
                block_hash: h(5),
                block_height: 10,
                state_root: h(6),
                utxo_hash: h(7),
                utxo_count: 11,
                chunk_max_bytes: 12,
                chain_state: vec![1],
                producer_set: vec![2],
                block_header_bytes: None,
                epoch_bond_snapshot_bytes: Some(vec![3]),
                epoch_accumulators_bytes: None,
                epoch_state_bytes: Some(vec![4]),
            },
            SyncResponse::StateManifest {
                session_id: 1,
                block_hash: h(5),
                block_height: 10,
                state_root: h(6),
                utxo_hash: h(7),
                utxo_count: 11,
                chunk_max_bytes: 12,
                chain_state: vec![1],
                producer_set: vec![2],
                block_header_bytes: None,
                epoch_bond_snapshot_bytes: Some(vec![3]),
                epoch_accumulators_bytes: None,
                epoch_state_bytes: Some(vec![4]),
            },
        ),
        (
            FrozenSyncResponse::StateChunk {
                session_id: 2,
                body: vec![6, 7],
                next_key: None,
            },
            SyncResponse::StateChunk {
                session_id: 2,
                body: vec![6, 7],
                next_key: None,
            },
        ),
        (
            FrozenSyncResponse::StateSessionUnavailable {
                session_id: 3,
                reason: FrozenStateSessionRefusal::Halted("h".to_string()),
            },
            SyncResponse::StateSessionUnavailable {
                session_id: 3,
                reason: network::protocols::StateSessionRefusal::Halted("h".to_string()),
            },
        ),
    ]
}

fn maintainer_state_at(set_bytes: Vec<u8>) -> SyncResponse {
    SyncResponse::MaintainerStateAt {
        block_hash: h(0xA1),
        block_height: 25_000,
        digest: h(0xA2),
        set_bytes,
        last_derived_height: 24_999,
    }
}

#[test]
fn inc_i_248_sync_wire_append_only() {
    for (i, (frozen, new)) in frozen_requests().iter().enumerate() {
        let old_bytes = enc(frozen);
        let new_bytes = enc(new);
        assert_eq!(old_bytes, new_bytes, "request index {i}: bytes moved");
        assert_eq!(
            index_of(&new_bytes),
            i as u32,
            "request index {i}: discriminant moved"
        );
        let decoded: SyncRequest = bincode::deserialize(&old_bytes)
            .unwrap_or_else(|e| panic!("request index {i}: new decoder rejects old bytes: {e}"));
        assert_eq!(
            enc(&decoded),
            old_bytes,
            "request index {i}: new decode differs"
        );
        let back: FrozenSyncRequest = bincode::deserialize(&new_bytes)
            .unwrap_or_else(|e| panic!("request index {i}: old decoder rejects new bytes: {e}"));
        assert_eq!(
            enc(&back),
            new_bytes,
            "request index {i}: old decode differs"
        );
    }
    for (i, (frozen, new)) in frozen_responses().iter().enumerate() {
        let old_bytes = enc(frozen);
        let new_bytes = enc(new);
        assert_eq!(old_bytes, new_bytes, "response index {i}: bytes moved");
        assert_eq!(
            index_of(&new_bytes),
            i as u32,
            "response index {i}: discriminant moved"
        );
        let decoded: SyncResponse = bincode::deserialize(&old_bytes)
            .unwrap_or_else(|e| panic!("response index {i}: new decoder rejects old bytes: {e}"));
        assert_eq!(
            enc(&decoded),
            old_bytes,
            "response index {i}: new decode differs"
        );
        let back: FrozenSyncResponse = bincode::deserialize(&new_bytes)
            .unwrap_or_else(|e| panic!("response index {i}: old decoder rejects new bytes: {e}"));
        assert_eq!(
            enc(&back),
            new_bytes,
            "response index {i}: old decode differs"
        );
    }

    let req_bytes = enc(&SyncRequest::GetMaintainerState);
    assert_eq!(
        req_bytes,
        10u32.to_le_bytes().to_vec(),
        "GetMaintainerState must be unit index 10"
    );
    assert!(
        bincode::deserialize::<FrozenSyncRequest>(&req_bytes).is_err(),
        "an old peer must reject GetMaintainerState, never decode another request"
    );
    let req_back: SyncRequest =
        bincode::deserialize(&req_bytes).expect("new decoder accepts index 10");
    assert!(matches!(req_back, SyncRequest::GetMaintainerState));
    assert!(!SyncRequest::GetMaintainerState.is_rate_governed());

    for set_bytes in [vec![], vec![0x11; 173]] {
        let resp = maintainer_state_at(set_bytes.clone());
        let resp_bytes = enc(&resp);
        assert_eq!(
            index_of(&resp_bytes),
            9,
            "MaintainerStateAt must be index 9"
        );
        assert_eq!(
            resp_bytes,
            enc(&ResponseLayoutMirror::MaintainerStateAt {
                block_hash: h(0xA1),
                block_height: 25_000,
                digest: h(0xA2),
                set_bytes: set_bytes.clone(),
                last_derived_height: 24_999,
            }),
            "MaintainerStateAt field order or types moved"
        );
        assert!(
            bincode::deserialize::<FrozenSyncResponse>(&resp_bytes).is_err(),
            "an old peer must reject MaintainerStateAt, never decode another response"
        );
        match bincode::deserialize::<SyncResponse>(&resp_bytes)
            .expect("new decoder accepts index 9")
        {
            SyncResponse::MaintainerStateAt {
                block_hash,
                block_height,
                digest,
                set_bytes: got,
                last_derived_height,
            } => {
                assert_eq!(block_hash, h(0xA1));
                assert_eq!(block_height, 25_000);
                assert_eq!(digest, h(0xA2));
                assert_eq!(got, set_bytes);
                assert_eq!(last_derived_height, 24_999);
            }
            other => panic!("decoded as {}", other.type_name()),
        }
        assert_eq!(resp.type_name(), "MaintainerStateAt");
    }
}
