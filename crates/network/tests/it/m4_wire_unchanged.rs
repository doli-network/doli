//! UTXO-scalability M4 — the session WIRE FORMAT does not move.
//!
//! M4 changes only what the client does with a chunk AFTER it is verified. A 6.40.1 server
//! must keep serving an M4 client and an M4 server must keep serving a 6.40.1 client, so
//! the manifest and chunk frames must encode to the SAME bytes as they do today.
//!
//! `m3_protocol_session.rs` already locks the variant DISCRIMINANTS, the graceful-decode
//! failure of an unknown variant on a legacy peer, and the size bounds. What it does not
//! lock is the byte image of a populated frame, which is what a field added to, removed
//! from, or retyped inside these variants would change. This file adds only that.
//!
//! OUTPUT CONTRACT: `bincode::serialize` over `SyncResponse::{StateManifest, StateChunk}`
//!   and `SyncRequest::{GetStateManifest, GetStateChunk}`.
//!   Outputs observable from the stable API:
//!     O1 the encoded byte image of each fully-populated frame
//!     O2 the round-trip decode of that image back into the variant
//!   Paths: P1 a manifest with every optional field ABSENT
//!          P2 a manifest with every optional field PRESENT
//!          P3 a chunk that is NOT the last (next_key = Some)
//!          P4 a chunk that IS the last (next_key = None)
//!   MATRIX: P1xO1+O2 | P2xO1+O2 | P3xO1+O2 | P4xO1+O2
//! INPUT PARTITIONS: the `Option` fields are exercised in BOTH states, because bincode
//!   encodes `None` as one byte and a field added after an always-`None` tail would be
//!   invisible to a `None`-only vector.

use crypto::Hash;
use network::protocols::sync::{SyncRequest, SyncResponse, STATE_CHUNK_MAX_BYTES};

fn h(tag: u8) -> Hash {
    Hash::from_bytes([tag; 32])
}

/// Local hex, so this file needs no dependency the `network` crate does not already have.
fn enc<T: serde::Serialize>(v: &T) -> String {
    bincode::serialize(v)
        .expect("the frame must encode")
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn manifest(populated: bool) -> SyncResponse {
    SyncResponse::StateManifest {
        session_id: 0x0102_0304_0506_0708,
        block_hash: h(0x11),
        block_height: 123_456,
        state_root: h(0x22),
        utxo_hash: h(0x33),
        utxo_count: 987_654,
        chunk_max_bytes: STATE_CHUNK_MAX_BYTES,
        chain_state: vec![7u8; 8],
        producer_set: vec![8u8; 8],
        block_header_bytes: populated.then(|| vec![0xA1u8; 4]),
        epoch_bond_snapshot_bytes: populated.then(|| vec![0xA2u8; 4]),
        epoch_accumulators_bytes: populated.then(|| vec![0xA3u8; 4]),
        epoch_state_bytes: populated.then(|| vec![0xA4u8; 4]),
    }
}

// ==================== W1 — the frames are byte-frozen ====================

/// REQ-SCALE-013 / REQ-SCALE-007 — Decision: a failure here means M4 changed a message a
/// deployed 6.40.1 peer must parse. Both directions break at once — an M4 client cannot
/// snap-sync from the fleet, and the fleet cannot snap-sync from an M4 node — and the
/// symptom on the wire is a decode error attributed to the peer, not to the release.
/// The `after` bytes are the `before` bytes: this file is written against UNCHANGED code
/// and its vectors are captured from it.
#[test]
fn m4_session_frames_are_byte_identical_to_m3() {
    const MANIFEST_EMPTY_TAIL: &str =
        "0600000008070605040302012000000000000000111111111111111111111111111111111111111111111111111111111111111140e2010000000000200000000000000022222222222222222222222222222222222222222222222222222222222222222000000000000000333333333333333333333333333333333333333333333333333333333333333306120f000000000000001000080000000000000007070707070707070800000000000000080808080808080800000000";
    const MANIFEST_FULL_TAIL: &str =
        "0600000008070605040302012000000000000000111111111111111111111111111111111111111111111111111111111111111140e2010000000000200000000000000022222222222222222222222222222222222222222222222222222222222222222000000000000000333333333333333333333333333333333333333333333333333333333333333306120f0000000000000010000800000000000000070707070707070708000000000000000808080808080808010400000000000000a1a1a1a1010400000000000000a2a2a2a2010400000000000000a3a3a3a3010400000000000000a4a4a4a4";
    const CHUNK_MORE: &str =
        "070000002a000000000000000800000000000000cdcdcdcdcdcdcdcd010400000000000000efefefef";
    const CHUNK_LAST: &str = "070000002a000000000000000800000000000000cdcdcdcdcdcdcdcd00";
    const REQ_MANIFEST: &str =
        "0800000020000000000000000909090909090909090909090909090909090909090909090909090909090909";
    const REQ_CHUNK: &str = "090000002a00000000000000010400000000000000abababab00001000";

    let cases: [(&str, String, &str); 6] = [
        (
            "StateManifest{tail:None}",
            enc(&manifest(false)),
            MANIFEST_EMPTY_TAIL,
        ),
        (
            "StateManifest{tail:Some}",
            enc(&manifest(true)),
            MANIFEST_FULL_TAIL,
        ),
        (
            "StateChunk{next_key:Some}",
            enc(&SyncResponse::StateChunk {
                session_id: 42,
                body: vec![0xCDu8; 8],
                next_key: Some(vec![0xEFu8; 4]),
            }),
            CHUNK_MORE,
        ),
        (
            "StateChunk{next_key:None}",
            enc(&SyncResponse::StateChunk {
                session_id: 42,
                body: vec![0xCDu8; 8],
                next_key: None,
            }),
            CHUNK_LAST,
        ),
        (
            "GetStateManifest",
            enc(&SyncRequest::GetStateManifest { block_hash: h(9) }),
            REQ_MANIFEST,
        ),
        (
            "GetStateChunk",
            enc(&SyncRequest::GetStateChunk {
                session_id: 42,
                start_key: Some(vec![0xABu8; 4]),
                max_bytes: STATE_CHUNK_MAX_BYTES,
            }),
            REQ_CHUNK,
        ),
    ];

    for (name, actual, expected) in cases.iter() {
        assert_eq!(
            actual, expected,
            "{} no longer encodes to the bytes a 6.40.1 peer parses",
            name
        );
    }
}
