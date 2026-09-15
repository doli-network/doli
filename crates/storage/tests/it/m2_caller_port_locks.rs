//! UTXO-scalability M2 [F2] — caller-port regression locks.
//!
//! REQ-SCALE-001 / INV-SYNC-007 — Decision: a change in the pinned state root would reveal
//! that porting the callers off `serialize_canonical()` altered a consensus value, which is
//! a chain split, not a refactor.
//! REQ-SCALE-006 — Decision: a change in the monitor's reported size would reveal that the
//! snap-sync 12 MB alert threshold now measures a different quantity than the 16 MB wire
//! limit it exists to predict.
//!
//! These tests MUST PASS BEFORE AND AFTER the M2 port. They pin VALUES over fixed inputs;
//! `state_root_golden_identity_test.rs` pins the three-component FORMULA and is not
//! duplicated here.
//!
//! OUTPUT CONTRACT:
//!   fn UtxoSizeMonitor::compute_size(&self) -> u64
//!     O1 return value (bytes)            O2 computation_count() side effect
//!   fn storage::compute_state_root(&ChainState, &UtxoSet, &ProducerSet) -> Result<Hash, _>
//!     O3 return value (the root Hash)
//!   Paths:
//!     P1 populated state_db set  — THIS FILE (O1, O2)
//!     P2 empty state_db set      — THIS FILE (O1)
//!     P3 fixed three-component state — THIS FILE (O3)
//!   MATRIX: P1xO1 -> m2_utxo_size_monitor_reports_the_pinned_canonical_length
//!           P1xO1 -> m2_utxo_size_monitor_equals_serialize_canonical_len (PRE-M2 COMPANION)
//!           P1xO2 -> m2_utxo_size_monitor_counts_every_recomputation
//!           P2xO1 -> m2_utxo_size_monitor_on_an_empty_set_is_the_eight_byte_header
//!           P3xO3 -> m2_state_root_over_the_fixed_state_is_the_pinned_golden
//! INPUT PARTITIONS: populated (n=500 randomized), empty (n=0), fixed golden state (n=64).

use std::sync::Arc;

use crypto::{Hash, PublicKey};
use storage::chain_state::ChainState;
use storage::producer::ProducerSet;
use storage::utxo::UtxoSet;
use storage::{StateDb, UtxoSizeMonitor};
use tempfile::TempDir;

use crate::common;

const BOND_UNIT: u64 = 1_000_000_000;
const MONITOR_N: usize = 500;
const MONITOR_SEED: u64 = 0x4D32_4F4E_4954_4F52;
const GOLDEN_N: usize = 64;
const GOLDEN_SEED: u64 = 0x474F_4C44_454E_3032;

/// Canonical image length of `randomized_entries(MONITOR_N, MONITOR_SEED)`:
/// 8-byte header + 500 x (36-byte key + 61-byte value + extra_data).
const PINNED_MONITOR_BYTES: u64 = 54_525;

/// `compute_state_root` over `golden_state()`.
const PINNED_STATE_ROOT_HEX: &str =
    "cfc2939fc2c7d987c1b91f35e2d1321275d4ae55b6f6cdbfd9251d2aee813609";

/// Canonical UTXO component hash of the golden set.
const PINNED_UTXO_HASH_HEX: &str =
    "977eba887ab7efe9a1d0aa80c749aeb6b2b971bb657ae1989d5e5e957af65bb4";

fn golden_state(dir: &TempDir) -> (ChainState, UtxoSet, ProducerSet) {
    let mut cs = ChainState::new(Hash::from_bytes([7u8; 32]));
    cs.best_hash = Hash::from_bytes([9u8; 32]);
    cs.best_height = 4242;

    let entries = common::randomized_entries(GOLDEN_N, GOLDEN_SEED);
    let utxo = common::build_rocksdb(dir.path(), &entries);

    let mut ps = ProducerSet::new();
    for i in 1u8..=4 {
        ps.register_genesis_producer(PublicKey::from_bytes([i; 32]), 1, BOND_UNIT)
            .expect("fixture: register_genesis_producer must succeed");
    }

    (cs, utxo, ps)
}

fn monitor_fixture(dir: &TempDir, n: usize) -> (Arc<StateDb>, UtxoSizeMonitor) {
    let entries = common::randomized_entries(n, MONITOR_SEED);
    let sdb = common::open_state_db_with(dir.path(), &entries);
    let monitor = UtxoSizeMonitor::new(Arc::clone(&sdb));
    (sdb, monitor)
}

/// REQ-SCALE-006 — Decision: a failure here would reveal the snap-sync size alarm changed
/// the quantity it reports while its 12 MB threshold stayed put, silently moving the alert
/// away from the 16 MB wire limit it is calibrated against.
#[test]
fn m2_utxo_size_monitor_reports_the_pinned_canonical_length() {
    let dir = TempDir::new().expect("tempdir");
    let (_sdb, monitor) = monitor_fixture(&dir, MONITOR_N);

    let size = monitor.compute_size();
    println!("M2_MONITOR_BYTES={}", size);
    assert_eq!(
        size, PINNED_MONITOR_BYTES,
        "the monitor must report the same byte count before and after the streaming port"
    );
}

/// PRE-M2 COMPANION — delete this whole fn together with `serialize_canonical()`.
/// REQ-SCALE-006 — Decision: a failure here would reveal the pinned number above stopped
/// describing the canonical image, making the pin a number with no referent.
#[test]
fn m2_utxo_size_monitor_equals_serialize_canonical_len() {
    let dir = TempDir::new().expect("tempdir");
    let (sdb, monitor) = monitor_fixture(&dir, MONITOR_N);
    let set = UtxoSet::from_state_db(Arc::clone(&sdb));

    assert_eq!(
        monitor.compute_size(),
        set.serialize_canonical().len() as u64,
        "the monitor's size must be the length of the canonical image it claims to measure"
    );
}

/// REQ-SCALE-006 — Decision: a failure here would reveal the port lost the cache/recompute
/// accounting the F1 monitor's 60 s TTL depends on.
#[test]
fn m2_utxo_size_monitor_counts_every_recomputation() {
    let dir = TempDir::new().expect("tempdir");
    let (_sdb, monitor) = monitor_fixture(&dir, MONITOR_N);

    assert_eq!(monitor.computation_count(), 0);
    let first = monitor.compute_size();
    let second = monitor.compute_size();
    assert_eq!(monitor.computation_count(), 2);
    assert_eq!(
        first, second,
        "two recomputations over an unchanged set must agree"
    );

    let cached = monitor.get_cached_size();
    assert_eq!(cached, first);
    assert_eq!(
        monitor.computation_count(),
        3,
        "the first cached read must still compute once"
    );
    assert_eq!(monitor.get_cached_size(), first);
    assert_eq!(
        monitor.computation_count(),
        3,
        "a read inside the TTL must not recompute"
    );
}

/// REQ-SCALE-006 — Decision: a failure here would reveal the streaming count dropped the
/// 8-byte header, which is the exact off-by-8 that would desync a snap deserializer.
#[test]
fn m2_utxo_size_monitor_on_an_empty_set_is_the_eight_byte_header() {
    let dir = TempDir::new().expect("tempdir");
    let (_sdb, monitor) = monitor_fixture(&dir, 0);

    assert_eq!(
        monitor.compute_size(),
        8,
        "an empty canonical image is exactly the 8-byte LE count header"
    );
}

/// REQ-SCALE-001 / INV-SYNC-007 — Decision: a failure here would reveal the M2 port changed
/// the consensus state root for an unchanged state, which forks the chain at the next
/// header comparison.
#[test]
fn m2_state_root_over_the_fixed_state_is_the_pinned_golden() {
    let dir = TempDir::new().expect("tempdir");
    let (cs, utxo, ps) = golden_state(&dir);

    let utxo_hash = crypto::hash::hash(&utxo.serialize_canonical()).to_hex();
    let root = storage::compute_state_root(&cs, &utxo, &ps).expect("compute_state_root");
    println!("M2_GOLDEN_UTXO_HASH={}", utxo_hash);
    println!("M2_GOLDEN_STATE_ROOT={}", root.to_hex());

    assert_eq!(
        utxo_hash, PINNED_UTXO_HASH_HEX,
        "the canonical UTXO component of the fixed state must not move"
    );
    assert_eq!(
        root.to_hex(),
        PINNED_STATE_ROOT_HEX,
        "the state root of the fixed state must not move"
    );
}

/// REQ-SCALE-001 — Decision: a failure here would reveal the pinned root is backend-specific,
/// which would make a snap-synced node's root differ from a full node's at the same height.
#[test]
fn m2_pinned_state_root_is_backend_independent() {
    let dir = TempDir::new().expect("tempdir");
    let (cs, rocks_utxo, ps) = golden_state(&dir);

    let entries = common::randomized_entries(GOLDEN_N, GOLDEN_SEED);
    let mem_utxo = common::build_in_memory(&entries);

    let rocks_root = storage::compute_state_root(&cs, &rocks_utxo, &ps).expect("state_db root");
    let mem_root = storage::compute_state_root(&cs, &mem_utxo, &ps).expect("in-memory root");

    assert_eq!(
        rocks_root, mem_root,
        "the state root must not depend on which backend holds the UTXO set"
    );
    assert_eq!(rocks_root.to_hex(), PINNED_STATE_ROOT_HEX);
}
