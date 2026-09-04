//! INC-I-178 M6 — the JSON capture format REQ-BLS-009 replays, and its loader.
//!
//! OUTPUT CONTRACT: N/A — fixture file. The enumeration for `load_epoch_fixture` lives
//! in `inc_i_178_m6_replay.rs`. INPUT PARTITIONS: N/A.
//!
//! FORMAT — `inc-i-178-m6-epoch-replay/1`. This is the wire M7 drops a real
//! testnet capture into. All producer references are INDICES into the epoch producer list
//! sorted by pubkey bytes, which is the universe order every consensus site uses.
//!
//! ```json
//! {
//!   "format": "inc-i-178-m6-epoch-replay/1",
//!   "label": "testnet-epoch-287",
//!   "epoch": 287,
//!   "producer_count": 7,
//!   "blocks": [
//!     {
//!       "height": 10332,
//!       "slot": 10332,
//!       "producer": 3,
//!       "parent_hash": "8f14e45fceea167a5a36dedd4bea2543f14e45fceea167a5a36dedd4bea25431",
//!       "attendance": [ {"attester": 0, "minute": 1722}, {"attester": 1, "minute": 1722} ],
//!       "parent_attesters": [0, 1, 4]
//!     }
//!   ]
//! }
//! ```
//!
//! - `attendance` — the `(attester, minute)` pairs the MINUTE TRACKER held when this block
//!   was built, as observed at ingest. Only the entries whose `minute` equals
//!   `attestation_minute(slot)` can set a bit under `PreAhMinuteUnion`; the rest are the
//!   tracker's other-minute rows and are carried so a capture can be replayed verbatim.
//! - `parent_attesters` — the indices holding a VALID pooled BLS signature over
//!   `parent_hash` when this block was built. This is the ONLY input `PostAhParentAttestation`
//!   reads.
//! - `parent_hash` — 64 lowercase hex characters. It is the pool key, so it only has to be
//!   consistent within one epoch; it is never re-hashed.
//!

use std::path::Path;

use crypto::Hash;

use crate::inc_i_178_m6_replay_harness::{Attendance, ReplayBlock, ReplayEpoch, FIXTURE_FORMAT};

/// Parse a `inc-i-178-m6-epoch-replay/1` capture. Every failure panics with the field
/// that was wrong: a fixture that silently loses half its blocks would be replayed as a
/// short epoch and reported as "no delta".
pub fn load_epoch_fixture(path: &Path) -> ReplayEpoch {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("M6 replay fixture missing at {}: {e}", path.display()));
    let doc: serde_json::Value =
        serde_json::from_str(&raw).expect("the replay fixture must be valid JSON");

    let format = doc["format"].as_str().expect("`format` must be a string");
    assert_eq!(
        format, FIXTURE_FORMAT,
        "unknown replay fixture format — M7 must emit {FIXTURE_FORMAT}"
    );
    let producer_count = usize_at(&doc, "producer_count");
    let epoch = doc["epoch"].as_u64().expect("`epoch` must be a number");
    let label = doc["label"].as_str().unwrap_or("fixture").to_string();

    let blocks = doc["blocks"]
        .as_array()
        .expect("`blocks` must be an array")
        .iter()
        .map(|b| {
            let attendance = b["attendance"]
                .as_array()
                .expect("`attendance` must be an array")
                .iter()
                .map(|a| Attendance {
                    attester: index_in_range(usize_at(a, "attester"), producer_count, "attester"),
                    minute: usize_at(a, "minute") as u32,
                })
                .collect();
            let parent_attesters = b["parent_attesters"]
                .as_array()
                .expect("`parent_attesters` must be an array")
                .iter()
                .map(|v| {
                    index_in_range(
                        v.as_u64().expect("a parent attester must be a number") as usize,
                        producer_count,
                        "parent_attesters",
                    )
                })
                .collect();
            ReplayBlock {
                height: b["height"].as_u64().expect("`height` must be a number"),
                slot: usize_at(b, "slot") as u32,
                producer: index_in_range(usize_at(b, "producer"), producer_count, "producer"),
                parent_hash: parse_hash(
                    b["parent_hash"]
                        .as_str()
                        .expect("`parent_hash` must be a hex string"),
                ),
                attendance,
                parent_attesters,
            }
        })
        .collect::<Vec<ReplayBlock>>();

    assert!(
        !blocks.is_empty(),
        "a replay fixture with no blocks would report an empty delta for free"
    );
    ReplayEpoch {
        label,
        epoch,
        producer_count,
        blocks,
    }
}

fn usize_at(v: &serde_json::Value, key: &str) -> usize {
    v[key]
        .as_u64()
        .unwrap_or_else(|| panic!("`{key}` must be a non-negative number")) as usize
}

fn index_in_range(i: usize, producer_count: usize, field: &str) -> usize {
    assert!(
        i < producer_count,
        "{field}={i} is outside the {producer_count}-producer universe"
    );
    i
}

fn parse_hash(hex_str: &str) -> Hash {
    let bytes = hex::decode(hex_str).expect("`parent_hash` must be valid hex");
    let arr: [u8; 32] = bytes
        .try_into()
        .expect("`parent_hash` must be exactly 32 bytes");
    Hash::from_bytes(arr)
}
