//! UTXO-scalability M3 [F3] — wire-protocol locks for the chunked snap-sync session.
//!
//! REQ-SCALE-013 — Decision: a failure here would reveal that the session variants were
//! INSERTED rather than APPENDED, so every already-deployed peer would silently mis-decode
//! `GetStateSnapshot` as `GetStateRoot` (and `StateSnapshot` as `StateRoot`) the moment one
//! upgraded node spoke to it — the Full-Bitfield-Decode index-parity class, on the wire.
//! REQ-SCALE-007 — Decision: a failure of the legacy-decode test would reveal that an
//! un-upgraded binary meeting a session message takes a path other than a clean decode
//! error, i.e. that the rolling deploy is not actually rolling.
//!
//! OUTPUT CONTRACT: `bincode::serialize(&SyncRequest) / (&SyncResponse)` and
//!                  `SyncCodec::read_request` / `read_response`.
//!   Outputs observable from the stable API:
//!     O1 the 4-byte LE variant discriminant that leads every encoded message
//!     O2 total encoded length of one message
//!     O3 decode result on a peer that does NOT know the variant (Ok / Err / panic)
//!     O4 the value of `MAX_SYNC_SIZE`, the bound the codec enforces on O2
//!   Paths:
//!     P1 every PRE-M3 variant, encoded by an M3 binary        — A1 (O1)
//!     P2 an M3-only variant, decoded by a PRE-M3 binary       — A2 (O3)
//!     P3 an M3-only variant, decoded by an M3 binary          — A2 (O3, control)
//!     P4 a maximally sized chunk / a manifest at two scales   — A3, A4 (O2)
//!     P5 the codec bound itself                               — A5 (O4)
//!   MATRIX: P1xO1 -> A1 | P2xO3 + P3xO3 -> A2 | P4xO2 -> A3, A4 | P5xO4 -> A5
//! INPUT PARTITIONS: (a) empty/zero-valued payloads for the discriminant walk — the
//!   discriminant is payload-independent, so the smallest legal value isolates O1;
//!   (b) a body of exactly `STATE_CHUNK_MAX_BYTES` — the worst legal chunk;
//!   (c) `utxo_count` at 10 and at 10_000_000 — the two ends of REQ-SCALE-013's range.

use crypto::Hash;
use network::protocols::sync::{
    StateSessionRefusal, SyncCodec, SyncRequest, SyncResponse, MAX_CONCURRENT_STATE_SESSIONS,
    MAX_SYNC_SIZE, STATE_CHUNK_MAX_BYTES, STATE_SESSION_IDLE_TIMEOUT_SECS, STATE_SESSION_TTL_SECS,
};

use doli_core::{Block, BlockHeader};
use libp2p::request_response::Codec;
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

// ==================== the discriminants as they are TODAY ====================
//
// Read off `crates/network/src/protocols/sync.rs` (SyncRequest at :26, SyncResponse at :90)
// at the start of M3. bincode encodes an enum as a u32 LE variant INDEX, so these numbers
// are the wire contract with every binary already deployed.

const REQ_GET_HEADERS: u32 = 0;
const REQ_GET_BODIES: u32 = 1;
const REQ_GET_BLOCK_BY_HEIGHT: u32 = 2;
const REQ_GET_BLOCK_BY_HASH: u32 = 3;
const REQ_GET_STATE_SNAPSHOT: u32 = 4;
const REQ_GET_STATE_ROOT: u32 = 5;
const REQ_DIRECT_ATTESTATION: u32 = 6;
const REQ_GET_HEADERS_BY_HEIGHT: u32 = 7;
/// First index M3 may consume — anything lower is already spoken for.
const REQ_FIRST_M3_INDEX: u32 = 8;

const RESP_HEADERS: u32 = 0;
const RESP_BODIES: u32 = 1;
const RESP_BLOCK: u32 = 2;
const RESP_STATE_SNAPSHOT: u32 = 3;
const RESP_STATE_ROOT: u32 = 4;
const RESP_ERROR: u32 = 5;
/// First index M3 may consume on the response enum.
const RESP_FIRST_M3_INDEX: u32 = 6;

// ==================== the PRE-M3 mirror enums (the "legacy peer") ====================
//
// Field shapes copied verbatim from sync.rs. These stand in for a binary that is already
// deployed and will never learn the session variants.

#[derive(Serialize, Deserialize)]
#[allow(dead_code)]
enum LegacySyncRequest {
    GetHeaders { start_hash: Hash, max_count: u32 },
    GetBodies { hashes: Vec<Hash> },
    GetBlockByHeight { height: u64 },
    GetBlockByHash { hash: Hash },
    GetStateSnapshot { block_hash: Hash },
    GetStateRoot { block_hash: Hash },
    DirectAttestation { data: Vec<u8> },
    GetHeadersByHeight { start_height: u64, max_count: u32 },
}

#[derive(Serialize, Deserialize)]
#[allow(dead_code, clippy::large_enum_variant)]
enum LegacySyncResponse {
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
}

// ==================== helpers ====================

fn h(tag: u8) -> Hash {
    Hash::from_bytes([tag; 32])
}

fn discriminant_of(bytes: &[u8]) -> u32 {
    assert!(
        bytes.len() >= 4,
        "a bincode enum encoding is at least its 4-byte LE variant index"
    );
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn enc_req(r: &SyncRequest) -> Vec<u8> {
    bincode::serialize(r).expect("SyncRequest must encode")
}

fn enc_resp(r: &SyncResponse) -> Vec<u8> {
    bincode::serialize(r).expect("SyncResponse must encode")
}

fn manifest_with_count(utxo_count: u64) -> SyncResponse {
    SyncResponse::StateManifest {
        session_id: 0x0102_0304_0506_0708,
        block_hash: h(0x11),
        block_height: 123_456,
        state_root: h(0x22),
        utxo_hash: h(0x33),
        utxo_count,
        chunk_max_bytes: STATE_CHUNK_MAX_BYTES,
        chain_state: vec![7u8; 128],
        producer_set: vec![8u8; 512],
        block_header_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
        epoch_state_bytes: None,
    }
}

fn read_request_frame(payload: &[u8]) -> std::io::Result<SyncRequest> {
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    let mut io = futures::io::Cursor::new(frame);
    let proto = StreamProtocol::new("/doli/sync/1.0.0");
    futures::executor::block_on(SyncCodec.read_request(&proto, &mut io))
}

// ==================== A1 — discriminant parity ====================

/// REQ-SCALE-013 — Decision: a failure here means an M3 binary and a deployed binary
/// disagree on what variant index 4 means, so a legacy peer would decode a session
/// request as `GetStateSnapshot` and serve a full-set frame, or decode `StateChunk` as
/// `StateSnapshot` and install a truncated UTXO set. This is the only test that catches
/// a variant inserted in the MIDDLE of the enum, which compiles cleanly.
#[test]
fn m3_appended_variants_preserve_legacy_discriminants() {
    let cases: [(u32, SyncRequest); 8] = [
        (
            REQ_GET_HEADERS,
            SyncRequest::GetHeaders {
                start_hash: h(1),
                max_count: 0,
            },
        ),
        (REQ_GET_BODIES, SyncRequest::GetBodies { hashes: vec![] }),
        (
            REQ_GET_BLOCK_BY_HEIGHT,
            SyncRequest::GetBlockByHeight { height: 0 },
        ),
        (
            REQ_GET_BLOCK_BY_HASH,
            SyncRequest::GetBlockByHash { hash: h(2) },
        ),
        (
            REQ_GET_STATE_SNAPSHOT,
            SyncRequest::GetStateSnapshot { block_hash: h(3) },
        ),
        (
            REQ_GET_STATE_ROOT,
            SyncRequest::GetStateRoot { block_hash: h(4) },
        ),
        (
            REQ_DIRECT_ATTESTATION,
            SyncRequest::DirectAttestation { data: vec![] },
        ),
        (
            REQ_GET_HEADERS_BY_HEIGHT,
            SyncRequest::GetHeadersByHeight {
                start_height: 0,
                max_count: 0,
            },
        ),
    ];

    for (expected, req) in cases.iter() {
        let got = discriminant_of(&enc_req(req));
        assert_eq!(
            got, *expected,
            "SyncRequest variant index moved: expected {}, got {} — every deployed peer \
             decodes by INDEX, so a moved index is a silent cross-version mis-decode",
            expected, got
        );
    }

    let resp_cases: [(u32, SyncResponse); 6] = [
        (RESP_HEADERS, SyncResponse::Headers(vec![])),
        (RESP_BODIES, SyncResponse::Bodies(vec![])),
        (RESP_BLOCK, SyncResponse::Block(None)),
        (
            RESP_STATE_SNAPSHOT,
            SyncResponse::StateSnapshot {
                block_hash: h(5),
                block_height: 1,
                chain_state: vec![],
                utxo_set: vec![],
                producer_set: vec![],
                state_root: h(6),
                block_header_bytes: None,
                epoch_bond_snapshot_bytes: None,
                epoch_accumulators_bytes: None,
                epoch_state_bytes: None,
            },
        ),
        (
            RESP_STATE_ROOT,
            SyncResponse::StateRoot {
                block_hash: h(7),
                block_height: 1,
                state_root: h(8),
            },
        ),
        (RESP_ERROR, SyncResponse::Error(String::new())),
    ];

    for (expected, resp) in resp_cases.iter() {
        let got = discriminant_of(&enc_resp(resp));
        assert_eq!(
            got, *expected,
            "SyncResponse variant index moved: expected {}, got {}",
            expected, got
        );
    }

    // The new variants must live strictly ABOVE every legacy index.
    let manifest_req = discriminant_of(&enc_req(&SyncRequest::GetStateManifest {
        block_hash: h(9),
    }));
    let chunk_req = discriminant_of(&enc_req(&SyncRequest::GetStateChunk {
        session_id: 1,
        start_key: None,
        max_bytes: STATE_CHUNK_MAX_BYTES,
    }));
    assert!(
        manifest_req >= REQ_FIRST_M3_INDEX && chunk_req >= REQ_FIRST_M3_INDEX,
        "M3 request variants must be APPENDED (index >= {}), got manifest={} chunk={}",
        REQ_FIRST_M3_INDEX,
        manifest_req,
        chunk_req
    );

    let manifest_resp = discriminant_of(&enc_resp(&manifest_with_count(10)));
    let chunk_resp = discriminant_of(&enc_resp(&SyncResponse::StateChunk {
        session_id: 1,
        body: vec![],
        next_key: None,
    }));
    let refusal_resp = discriminant_of(&enc_resp(&SyncResponse::StateSessionUnavailable {
        session_id: 1,
        reason: StateSessionRefusal::Busy,
    }));
    assert!(
        manifest_resp >= RESP_FIRST_M3_INDEX
            && chunk_resp >= RESP_FIRST_M3_INDEX
            && refusal_resp >= RESP_FIRST_M3_INDEX,
        "M3 response variants must be APPENDED (index >= {}), got manifest={} chunk={} refusal={}",
        RESP_FIRST_M3_INDEX,
        manifest_resp,
        chunk_resp,
        refusal_resp
    );
}

// ==================== A2 — graceful degradation on a legacy peer ====================

/// REQ-SCALE-007 / REQ-SCALE-013 — Decision: a failure here would reveal that an
/// un-upgraded peer receiving a session message either ACCEPTS it as some other variant
/// (silent corruption) or panics the codec task (a crash the rolling deploy would spread
/// fleet-wide). The failure-mode matrix's `none-recorded` row states this claim is "under
/// test, not asserted" — this is that test.
#[test]
fn m3_legacy_peer_fails_gracefully_on_unknown_variant() {
    let manifest_req = enc_req(&SyncRequest::GetStateManifest { block_hash: h(9) });
    let chunk_req = enc_req(&SyncRequest::GetStateChunk {
        session_id: 42,
        start_key: Some(vec![0xABu8; 36]),
        max_bytes: STATE_CHUNK_MAX_BYTES,
    });
    let chunk_resp = enc_resp(&SyncResponse::StateChunk {
        session_id: 42,
        body: vec![0xCDu8; 1024],
        next_key: Some(vec![0xEFu8; 36]),
    });
    let refusal_resp = enc_resp(&SyncResponse::StateSessionUnavailable {
        session_id: 42,
        reason: StateSessionRefusal::ManifestExpired,
    });

    // The decode must be an Err, not a panic — wrap so "does not panic" is a real assertion.
    let legacy_req_decode = std::panic::catch_unwind(|| {
        (
            bincode::deserialize::<LegacySyncRequest>(&manifest_req).is_err(),
            bincode::deserialize::<LegacySyncRequest>(&chunk_req).is_err(),
        )
    })
    .expect("a legacy binary must not PANIC on an unknown sync-request variant");
    assert!(
        legacy_req_decode.0,
        "a pre-M3 peer must REFUSE GetStateManifest, not decode it as a legacy variant"
    );
    assert!(
        legacy_req_decode.1,
        "a pre-M3 peer must REFUSE GetStateChunk, not decode it as a legacy variant"
    );

    let legacy_resp_decode = std::panic::catch_unwind(|| {
        (
            bincode::deserialize::<LegacySyncResponse>(&chunk_resp).is_err(),
            bincode::deserialize::<LegacySyncResponse>(&refusal_resp).is_err(),
        )
    })
    .expect("a legacy binary must not PANIC on an unknown sync-response variant");
    assert!(
        legacy_resp_decode.0,
        "a pre-M3 peer must REFUSE StateChunk, not decode it as StateSnapshot"
    );
    assert!(
        legacy_resp_decode.1,
        "a pre-M3 peer must REFUSE StateSessionUnavailable"
    );

    // Same shape through the REAL codec: an index no binary knows must surface as a
    // decode error on the stream, never a panic.
    let mut unknown = 9_999u32.to_le_bytes().to_vec();
    unknown.extend_from_slice(&[0u8; 8]);
    let codec_result = std::panic::catch_unwind(|| read_request_frame(&unknown).is_err())
        .expect("SyncCodec::read_request must not panic on an unknown variant index");
    assert!(
        codec_result,
        "SyncCodec::read_request must return Err(InvalidData) for an unknown variant index"
    );

    // Control (P3): an M3 binary decodes its own new variant through the same codec.
    let ok = read_request_frame(&manifest_req)
        .expect("an M3 binary must decode GetStateManifest through the live codec");
    assert!(
        matches!(ok, SyncRequest::GetStateManifest { .. }),
        "the codec must round-trip GetStateManifest, otherwise the Err above proves nothing"
    );
}

// ==================== A3 / A4 / A5 — the size bounds ====================

/// REQ-SCALE-013 — Decision: a failure here means the chunk bound and the codec bound are
/// too close together, so a chunk at the maximum body size could still be refused by
/// `read_response` — the exact "state is untransferable" failure Stage 3 exists to remove.
#[test]
fn m3_chunk_response_fits_far_under_max_sync_size() {
    let body = vec![0xA5u8; STATE_CHUNK_MAX_BYTES as usize];
    let encoded = enc_resp(&SyncResponse::StateChunk {
        session_id: 7,
        body,
        next_key: Some(vec![0x5Au8; 36]),
    });

    assert!(
        encoded.len() < MAX_SYNC_SIZE / 4,
        "a maximal StateChunk encodes to {} bytes; it must stay under MAX_SYNC_SIZE/4 = {} \
         so the wire bound is never the binding constraint on a chunk",
        encoded.len(),
        MAX_SYNC_SIZE / 4
    );
    assert!(
        encoded.len() >= STATE_CHUNK_MAX_BYTES as usize,
        "the encoding must actually carry the {}-byte body — a smaller encoding means the \
         measurement is not of a full chunk",
        STATE_CHUNK_MAX_BYTES
    );
}

/// REQ-SCALE-013 — Decision: a manifest that grows with `utxo_count` would re-introduce the
/// O(N) frame this milestone removes, just one message later. The whole point of the
/// manifest is that it is a fixed-size description of an unbounded set.
#[test]
fn m3_manifest_size_is_independent_of_utxo_count() {
    let small = enc_resp(&manifest_with_count(10)).len();
    let huge = enc_resp(&manifest_with_count(10_000_000)).len();

    let delta = small.abs_diff(huge);
    assert!(
        delta < 64,
        "StateManifest grew by {} bytes between utxo_count=10 ({} B) and utxo_count=10_000_000 \
         ({} B) — the manifest must be O(1) in the set size",
        delta,
        small,
        huge
    );
}

/// REQ-SCALE-013 / Res-7 — Decision: a failure here means someone "fixed" the transfer by
/// raising the codec bound instead of moving the bound into the session. Res-7 records that
/// `MAX_SYNC_SIZE` bounds FIVE payload classes; raising it re-opens the INC-I-012 F13
/// allocation DoS on all five. This is the tripwire, not a style check.
#[test]
fn m3_max_sync_size_unchanged() {
    assert_eq!(
        MAX_SYNC_SIZE,
        16 * 1024 * 1024,
        "MAX_SYNC_SIZE must stay at 16 MiB — M3 moves the bound to the session, never the constant"
    );
}

/// REQ-SCALE-013 — Decision: a failure here means the session's operating limits were
/// written as literals at their use sites, so the serve cap, the TTL and the chunk budget
/// can drift apart across the three files that read them.
#[test]
#[allow(clippy::assertions_on_constants)]
fn m3_session_limits_are_named_constants() {
    assert_eq!(
        STATE_CHUNK_MAX_BYTES,
        1024 * 1024,
        "the chunk budget must be 1 MiB"
    );
    assert_eq!(
        MAX_CONCURRENT_STATE_SESSIONS, 4,
        "the concurrent pinned-view cap must be 4"
    );
    assert_eq!(STATE_SESSION_TTL_SECS, 120, "the session TTL must be 120 s");
    assert_eq!(
        STATE_SESSION_IDLE_TIMEOUT_SECS, 30,
        "the session idle timeout must be 30 s"
    );
    assert!(
        STATE_SESSION_IDLE_TIMEOUT_SECS < STATE_SESSION_TTL_SECS,
        "an idle timeout at or above the TTL can never fire — the slot would only ever be \
         freed by expiry, which is the unbounded-pin failure the cap exists to prevent"
    );
}
