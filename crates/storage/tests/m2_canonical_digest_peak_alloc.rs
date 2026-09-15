//! UTXO-scalability M2 [F2] — state-root allocation probe (outcome-metric harness).
//!
//! REQ-SCALE-014 — Decision: a rise in full-set copies held during a root compute would
//! reveal the fold re-materialised the canonical image, which is the RAM ceiling M2 removes.
//! REQ-SCALE-018 — Decision: a peak that does not fall after the fold ships would reveal
//! the per-root cost is still O(N) resident bytes, not O(1).
//! REQ-SCALE-001 / INV-SYNC-007 — Decision: a change in `M2_UTXO_HASH` between the before
//! and after runs would reveal the RAM win was bought with a consensus-state change.
//!
//! OUTPUT CONTRACT: fn storage::compute_state_root(&ChainState, &UtxoSet, &ProducerSet)
//!                    -> Result<Hash, StorageError>
//!   Outputs observable from the stable API:
//!     O1 return value (the state root Hash)
//!     O2 the utxo component hash (re-derived witness, printed)
//!     O3 process-resident Rust bytes allocated DURING the call  <-- this file's target
//!     O4 the UTXO set is unchanged by the call (count preserved)
//!   Paths:
//!     P1 InMemory backend, populated set   — THIS FILE (O1, O2, O3, O4)
//!     P2 RocksDb backend, populated set    — THIS FILE (O1, O2, O3, O4)
//!     P3 empty set / single entry          — m2_canonical_fold_fail_loud.rs
//!     P4 undecodable entry (Err)           — m2_canonical_fold_fail_loud.rs
//!   MATRIX (this file): P1xO3, P2xO3 -> m2_probe_reports_state_root_peak (measure)
//!                       P1xO2, P2xO2 -> m2_probe_backends_agree_on_utxo_hash (bit-identity)
//!                       P1xO1, P2xO1 -> both (root must compute Ok)
//!                       P1xO4, P2xO4 -> m2_probe_reports_state_root_peak (count witness)
//! INPUT PARTITIONS: one class — a randomized set large enough that one full canonical
//!   image dominates harness noise (N = UTXO_COUNT), built per backend OUTSIDE the
//!   measured window.
//!
//! The counting allocator sees Rust allocations only; RocksDB's C++ arenas are invisible
//! to it. That is the intended scope: the number under test is the materialised canonical
//! image, which is Rust-side.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crypto::{Hash, PublicKey};
use storage::chain_state::ChainState;
use storage::producer::ProducerSet;
use storage::utxo::UtxoSet;
use tempfile::TempDir;

// ==================== counting allocator ====================

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct CountingAlloc;

#[inline]
fn record_growth(bytes: usize) {
    let now = CURRENT.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            record_growth(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            record_growth(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            if new_size >= layout.size() {
                record_growth(new_size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// Arm the peak at the current live total; returns that baseline.
fn reset_peak() -> usize {
    let base = CURRENT.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    base
}

fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

/// Serializes the measured region so the process-wide counters stay attributable
/// even if the binary is run without `--test-threads=1`.
static MEASURE_LOCK: Mutex<()> = Mutex::new(());

// ==================== fixture ====================

/// Recorded N — the architecture gate's threshold. Reduce only if a run exceeds ~120 s.
const UTXO_COUNT: usize = 100_000;
const FIXTURE_SEED: u64 = 0x00D0_1100_2026_0915;
const BOND_UNIT: u64 = 1_000_000_000;

fn sample_chain_state() -> ChainState {
    let mut cs = ChainState::new(Hash::from_bytes([7u8; 32]));
    cs.best_hash = Hash::from_bytes([9u8; 32]);
    cs.best_height = 4242;
    cs
}

fn sample_producer_set() -> ProducerSet {
    let mut ps = ProducerSet::new();
    for i in 1u8..=4 {
        ps.register_genesis_producer(PublicKey::from_bytes([i; 32]), 1, BOND_UNIT)
            .expect("fixture: register_genesis_producer must succeed");
    }
    ps
}

struct Measurement {
    peak_delta: usize,
    utxo_count: u64,
    utxo_hash: String,
    copies: f64,
}

/// Compute one state root over `set` and report the Rust bytes the call held live
/// at its worst instant. Everything the measurement must not count is built and
/// dropped before the counter is armed.
fn measure_root(set: &UtxoSet, set_bytes: usize) -> Measurement {
    let cs = sample_chain_state();
    let ps = sample_producer_set();

    let utxo_count = set.utxo_count();
    let utxo_hash = crypto::hash::hash(&set.serialize_canonical()).to_hex();

    let guard = MEASURE_LOCK.lock().expect("measure lock");
    let baseline = reset_peak();
    let root = storage::compute_state_root(&cs, set, &ps).expect("compute_state_root must succeed");
    let observed = peak();
    drop(guard);

    assert_ne!(root, Hash::ZERO, "a computed state root must not be zero");

    let peak_delta = observed.saturating_sub(baseline);
    Measurement {
        peak_delta,
        utxo_count,
        utxo_hash,
        copies: peak_delta as f64 / set_bytes as f64,
    }
}

// ==================== tests ====================

/// REQ-SCALE-014 / REQ-SCALE-018 — Decision: a failure here means the probe stopped
/// measuring a real root compute over a full set, so every before/after peak number
/// published as the M2 outcome metric is uninterpretable.
/// REQ-SCALE-001 — Decision: a divergence between the two backends' utxo hashes would
/// reveal the measured paths are not computing the same consensus value.
#[test]
fn m2_probe_reports_state_root_peak() {
    let entries = common::randomized_entries(UTXO_COUNT, FIXTURE_SEED);

    let mem_set = common::build_in_memory(&entries);
    let set_bytes = mem_set.serialize_canonical().len();

    let dir = TempDir::new().expect("tempdir");
    let rocks_set = common::build_rocksdb(dir.path(), &entries);
    drop(entries);

    let mem = measure_root(&mem_set, set_bytes);
    let rocks = measure_root(&rocks_set, set_bytes);

    // Leading newline: libtest leaves the `test <name> ...` prefix unterminated, which
    // would otherwise make the first witness line un-greppable at line start.
    println!();
    println!("M2_UTXO_COUNT={}", mem.utxo_count);
    println!("M2_SET_BYTES={}", set_bytes);
    println!("M2_PEAK_BYTES_INMEM={}", mem.peak_delta);
    println!("M2_FULL_SET_COPIES_INMEM={:.2}", mem.copies);
    println!("M2_PEAK_BYTES_ROCKS={}", rocks.peak_delta);
    println!("M2_FULL_SET_COPIES_ROCKS={:.2}", rocks.copies);
    println!("M2_UTXO_HASH={}", mem.utxo_hash);

    assert_eq!(
        mem.utxo_count, UTXO_COUNT as u64,
        "the in-memory set under measurement must hold all {} UTXOs",
        UTXO_COUNT
    );
    assert_eq!(
        rocks.utxo_count, UTXO_COUNT as u64,
        "the state_db set under measurement must hold all {} UTXOs",
        UTXO_COUNT
    );
    assert!(
        set_bytes > 1_000_000,
        "M2_SET_BYTES={} is too small to dominate harness noise",
        set_bytes
    );
    assert!(
        mem.peak_delta > 0,
        "M2_PEAK_BYTES_INMEM=0 means the probe measured nothing"
    );
    assert!(
        rocks.peak_delta > 0,
        "M2_PEAK_BYTES_ROCKS=0 means the probe measured nothing"
    );
}

/// REQ-SCALE-001 / INV-SYNC-007 — Decision: a failure here would reveal that the two
/// backends disagree on the consensus UTXO component, making the probe's before/after
/// bit-identity witness worthless.
#[test]
fn m2_probe_backends_agree_on_utxo_hash() {
    let entries = common::randomized_entries(10_000, FIXTURE_SEED ^ 0xABCD);

    let mem_set = common::build_in_memory(&entries);
    let dir = TempDir::new().expect("tempdir");
    let rocks_set = common::build_rocksdb(dir.path(), &entries);

    let mem_hash = crypto::hash::hash(&mem_set.serialize_canonical()).to_hex();
    let rocks_hash = crypto::hash::hash(&rocks_set.serialize_canonical()).to_hex();

    assert_eq!(mem_hash.len(), 64, "the witness must be a 32-byte hex hash");
    assert_eq!(
        mem_hash, rocks_hash,
        "InMemory and state_db backends must agree on the canonical UTXO hash"
    );

    let cs = sample_chain_state();
    let ps = sample_producer_set();
    let mem_root = storage::compute_state_root(&cs, &mem_set, &ps).expect("in-memory root");
    let rocks_root = storage::compute_state_root(&cs, &rocks_set, &ps).expect("state_db root");
    assert_eq!(
        mem_root, rocks_root,
        "the state root must not depend on which backend holds the UTXO set"
    );
}
