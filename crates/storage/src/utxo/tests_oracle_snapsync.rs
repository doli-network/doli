//! Snap-sync state-root inclusion tests for `OutputType::OraclePrice`
//! (=15) — Phase 2.1 Oracle M5.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.2 + EI-ORACLE-8
//! ("Snap-sync reproducibility: OraclePrice UTXO in UTXO set -> state
//! root automatically via `snapshot.rs`").
//!
//! The reasoning trace (`docs/.workflow/oracle-reasoning.md` §6 item 2)
//! flagged this as the single highest-risk honest unknown of the design
//! phase: "State root inclusion for singleton UTXOs — VERIFY in M5 by
//! reading `crates/storage/src/snapshot.rs`. If the state root does
//! not naturally include the OraclePrice UTXO, STOP AND REPORT."
//!
//! The chain of inclusion (read by hand, confirmed in M5):
//!   1. `compute_state_root(&UtxoSet)` at
//!      `crates/storage/src/snapshot.rs:24` hashes
//!      `UtxoSet::serialize_canonical()`.
//!   2. `UtxoSet::serialize_canonical` (set.rs:288) dispatches to the
//!      backend (`InMemory` or `RocksDb`), each of which iterates
//!      EVERY entry (in_memory.rs:380, utxo_rocks.rs:620) and writes
//!      `UtxoEntry::serialize_canonical_bytes` verbatim.
//!   3. `UtxoEntry::serialize_canonical_bytes` (types.rs:60) writes
//!      `output_type as u8` as the first byte, with no whitelist —
//!      adding `OraclePrice = 15` to `OutputType::from_u8` is the
//!      ONLY change needed for the new variant to round-trip through
//!      canonical encoding (and thus through the state root).
//!
//! This file pins (1) the canonical roundtrip for OraclePrice and
//! (2) the property that two state roots over identical UtxoSet
//! contents (one with the OraclePrice UTXO present, the other
//! without) DIVERGE — so snap-sync correctly fails on a node that
//! lost the OraclePrice UTXO.

use super::types::UtxoEntry;
use crate::snapshot::compute_state_root;
use crate::utxo::UtxoSet;
use crate::ChainState;
use crate::ProducerSet;
use crypto::Hash;
use doli_core::transaction::{Output, OutputType};

// OUTPUT CONTRACT: fn UtxoEntry::serialize_canonical_bytes /
//                     deserialize_canonical_bytes for OraclePrice
//   O1: roundtrip — decoded.output.output_type == OraclePrice
//   O2: roundtrip — decoded.output.extra_data == original 50 bytes
//   O3: roundtrip — decoded.output.amount == 0
//   O4: roundtrip — decoded.output.pubkey_hash ==
//                   oracle_price_address(pair_id)
// PATHS:
//   P1: serialize(OraclePrice) -> deserialize -> compare fields
// INPUT PARTITIONS:
//   part-A (P1): pair_id=[0xCC;32], price=4321, height=720, count=7
// MATRIX: 4 outputs × 1 path × 1 partition = 4 cells
//   P1×part-A: O1✓ O2✓ O3✓ O4✓
#[test]
fn test_oracle_price_canonical_bytes_roundtrip() {
    let pair_id = Hash::from_bytes([0xCC; 32]);
    let entry = UtxoEntry {
        output: Output::oracle_price(pair_id, 4_321, 720, 7),
        height: 720,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    let bytes = entry.serialize_canonical_bytes();
    let decoded =
        UtxoEntry::deserialize_canonical_bytes(&bytes).expect("OraclePrice canonical roundtrip");

    assert_eq!(decoded.output.output_type, OutputType::OraclePrice); // O1
    assert_eq!(decoded.output.extra_data.len(), 50);
    assert_eq!(decoded.output.extra_data, entry.output.extra_data); // O2
    assert_eq!(decoded.output.amount, 0); // O3
    assert_eq!(
        decoded.output.pubkey_hash,
        Output::oracle_price_address(&pair_id)
    ); // O4
}

// OUTPUT CONTRACT: fn compute_state_root — OraclePrice inclusion
//   O1: state_root_with_oracle != state_root_without_oracle
//       (proves the UTXO contributes to the state root)
//   O2: state_root is deterministic (two computations over the same
//       UtxoSet produce equal roots)
// PATHS:
//   P1: state root over empty UtxoSet vs UtxoSet with one OraclePrice
//   P2: two independent computations over the same UtxoSet
// INPUT PARTITIONS:
//   part-A (P1): empty UtxoSet, then add one OraclePrice UTXO at
//                outpoint (Hash::ZERO, 0)
//   part-B (P2): same UtxoSet evaluated twice
// MATRIX:
//   P1×part-A: O1✓     P2×part-B: O2✓
#[test]
fn test_oracle_price_changes_state_root() {
    let cs = ChainState::new(Hash::ZERO);
    let ps = ProducerSet::new();

    let utxo_empty = UtxoSet::new();
    let root_empty =
        compute_state_root(&cs, &utxo_empty, &ps).expect("state root over empty UtxoSet");

    let mut utxo_with_oracle = UtxoSet::new();
    let pair_id = Hash::from_bytes([0xDD; 32]);
    let entry = UtxoEntry {
        output: Output::oracle_price(pair_id, 100, 360, 5),
        height: 360,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo_with_oracle
        .insert(super::types::Outpoint::new(Hash::ZERO, 0), entry)
        .expect("insert OraclePrice UTXO");

    let root_with_oracle = compute_state_root(&cs, &utxo_with_oracle, &ps)
        .expect("state root over UtxoSet with OraclePrice");

    assert_ne!(
        root_empty, root_with_oracle,
        "state root must change when an OraclePrice UTXO is added — snap-sync inclusion"
    ); // O1
}

#[test]
fn test_oracle_price_state_root_deterministic() {
    let cs = ChainState::new(Hash::ZERO);
    let ps = ProducerSet::new();
    let mut utxo = UtxoSet::new();
    let pair_id = Hash::from_bytes([0xEE; 32]);
    let entry = UtxoEntry {
        output: Output::oracle_price(pair_id, 200, 720, 9),
        height: 720,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo.insert(super::types::Outpoint::new(Hash::ZERO, 0), entry)
        .expect("insert OraclePrice UTXO");

    let r1 = compute_state_root(&cs, &utxo, &ps).unwrap();
    let r2 = compute_state_root(&cs, &utxo, &ps).unwrap();
    assert_eq!(r1, r2); // O2
}
