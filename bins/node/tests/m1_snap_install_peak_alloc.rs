//! UTXO-scalability M1 [F1] — snap-install peak-allocation probe.
//!
//! REQ-SCALE-014 — Decision: a rise in installed-set copies would reveal that the
//! install re-grew a full-set clone, which is the failure this milestone removes.
//! REQ-SCALE-002 — Decision: a count/hash change here would reveal the probe stopped
//! installing the set it measured, making the peak number meaningless.
//!
//! OUTPUT CONTRACT: fn Node::apply_snap_snapshot(&mut self, VerifiedSnapshot) -> Result<()>
//!   Outputs observable from the stable API:
//!     O1 self.chain_state          O2 self.utxo_set (content + backend)
//!     O3 self.producer_set         O4 self.cached_state_root
//!     O5 state_db durable content  O6 block_store canonical index
//!     O7 sync_manager local tip    O8 return value
//!     O9 process-resident bytes held DURING the call  <-- this file's target
//!   Paths:
//!     P1 recovery_mode set (early return)     — not this file
//!     P2 undecodable bytes (reject)           — not this file
//!     P3 root mismatch (reject)               — m1_snap_install_identity.rs
//!     P4 envelope mismatch (reject)           — m1_snap_install_identity.rs
//!     P5 success (full install)               — THIS FILE (O8, O9, O2 count/hash)
//!     P6 atomic_replace Err                   — unreachable from the stable API
//!   MATRIX (this file): P5 x O8  -> m1_probe_reports_install_peak
//!                       P5 x O9  -> m1_probe_reports_install_peak (measure)
//!                                   m1_install_peak_is_set_by_the_verify_phase_alone (bound)
//!                       P5 x O2  -> both (count + canonical hash printed as witnesses)
//!   O1/O3/O4/O5/O6/O7 on P5 are locked by m1_snap_install_identity.rs.
//! INPUT PARTITIONS: one class — a valid snapshot whose serialized UTXO set is large
//!   enough that one full copy dominates harness noise (N = UTXO_COUNT).

mod inc_i_156_m1_harness;

use inc_i_156_m1_harness as h;
use network::VerifiedSnapshot;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

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

fn live() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

/// Serializes the measured region so the process-wide counters stay attributable
/// even if the binary is run without `--test-threads=1`.
static MEASURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ==================== fixture ====================

/// Recorded N. Reduce only if the run exceeds ~90 s.
const UTXO_COUNT: usize = 50_000;

/// Peak measured by this probe against the pre-M1 install path
/// (2026-09-15, n=50_000, repeatable bit-for-bit across independent nodes).
const PRE_M1_PEAK_BYTES: usize = 15_778_678;
const CHAIN_LEN: u64 = 2;
const N_PRODUCERS: usize = 3;

fn synthetic_utxo_set(n: usize) -> storage::UtxoSet {
    let mut set = storage::UtxoSet::new();
    for i in 0..n {
        let i64v = i as u64;
        let mut tx_hash = [0u8; 32];
        tx_hash[0..8].copy_from_slice(&i64v.to_le_bytes());
        tx_hash[8..16].copy_from_slice(&(i64v.wrapping_mul(0x9E37_79B9_7F4A_7C15)).to_le_bytes());
        let mut pkh = [0u8; 32];
        pkh[0..8].copy_from_slice(&(i64v ^ 0x5555_5555_5555_5555).to_le_bytes());
        set.insert(
            storage::Outpoint::new(crypto::Hash::from_bytes(tx_hash), (i % 4) as u32),
            storage::UtxoEntry {
                output: doli_core::transaction::Output::normal(
                    1_000 + i64v,
                    crypto::Hash::from_bytes(pkh),
                ),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: synthetic UTXO insert must succeed");
    }
    set
}

struct Measurement {
    peak_delta: usize,
    set_bytes: usize,
    decoded_bytes: usize,
    installed_count: u64,
    installed_hash: String,
    copies: f64,
}

/// Build a node on the production backend, hand it a large valid snapshot, and
/// measure the bytes the install holds live at its worst instant.
async fn measure_install(n: usize) -> Measurement {
    // Held for the whole function: a concurrent test freeing memory under the
    // process-wide allocator counters drives `live()` backwards and zeroes the
    // floor witness.
    let guard = MEASURE_LOCK.lock().await;
    let (mut node, producers, _temp) = h::make_node(N_PRODUCERS).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    assert!(
        !node.recovery_mode.load(Ordering::Relaxed),
        "fixture: recovery_mode must be false or apply_snap_snapshot returns before installing"
    );

    let snapshot = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let base = storage::StateSnapshot::create(&cs, &utxo, &ps)
            .expect("fixture: StateSnapshot::create must succeed");

        let big = synthetic_utxo_set(n);
        let utxo_bytes = big.serialize_canonical();
        drop(big);

        let root = storage::compute_state_root_from_bytes(
            &base.chain_state_bytes,
            &utxo_bytes,
            &base.producer_set_bytes,
        )
        .expect("fixture: root over the synthetic snapshot bytes must compute");

        VerifiedSnapshot {
            block_hash: base.block_hash,
            block_height: base.block_height,
            chain_state: base.chain_state_bytes,
            utxo_set: utxo_bytes,
            // Legacy single-frame path: the image arrived whole.
            utxo_staged: None,
            producer_set: base.producer_set_bytes,
            state_root: root,
            block_header_bytes: None,
            epoch_bond_snapshot_bytes: None,
            epoch_accumulators_bytes: None,
            epoch_state_bytes: Some(node.epoch_state.serialize()),
        }
    };

    let set_bytes = snapshot.utxo_set.len();

    // Floor witness: the live bytes ONE decoded in-memory copy of this set occupies.
    // No install can hold less than this while it owns a decoded set.
    let decoded_bytes = {
        let before = live();
        let decoded = storage::UtxoSet::deserialize_canonical(&snapshot.utxo_set)
            .expect("fixture: the snapshot bytes must decode");
        let held = live().saturating_sub(before);
        drop(decoded);
        held
    };

    let baseline = reset_peak();
    node.apply_snap_snapshot(snapshot)
        .await
        .expect("apply_snap_snapshot must succeed on a valid snapshot");
    let observed_peak = peak();
    drop(guard);

    let peak_delta = observed_peak.saturating_sub(baseline);

    let (installed_count, installed_hash) = {
        let utxo = node.utxo_set.read().await;
        (
            utxo.utxo_count(),
            hex::encode(crypto::hash::hash(&utxo.serialize_canonical()).as_bytes()),
        )
    };

    let copies = peak_delta as f64 / set_bytes as f64;

    // Leading newline: libtest leaves the `test <name> ...` prefix unterminated, which
    // would otherwise make the first witness line un-greppable at line start.
    println!();
    println!("M1_PEAK_BYTES={}", peak_delta);
    println!("M1_SET_BYTES={}", set_bytes);
    println!("M1_FULL_SET_COPIES={:.1}", copies);
    println!("M1_UTXO_COUNT={}", installed_count);
    println!("M1_UTXO_HASH={}", installed_hash);
    println!("M1_BASELINE_BYTES={}", baseline);
    println!("M1_DECODED_SET_BYTES={}", decoded_bytes);
    println!(
        "M1_DECODED_SET_COPIES={:.1}",
        decoded_bytes as f64 / set_bytes as f64
    );

    Measurement {
        peak_delta,
        set_bytes,
        decoded_bytes,
        installed_count,
        installed_hash,
        copies,
    }
}

// ==================== tests ====================

/// REQ-SCALE-014 — Decision: a failure here means the probe itself stopped
/// measuring a real install, so every later peak number is uninterpretable.
#[tokio::test]
async fn m1_probe_reports_install_peak() {
    let m = measure_install(UTXO_COUNT).await;

    assert_eq!(
        m.installed_count, UTXO_COUNT as u64,
        "the measured install must have installed all {} UTXOs (got {})",
        UTXO_COUNT, m.installed_count
    );
    assert_eq!(
        m.installed_hash.len(),
        64,
        "the installed canonical hash must be a 32-byte hex witness"
    );
    assert!(
        m.set_bytes > 1_000_000,
        "M1_SET_BYTES={} is too small to dominate harness noise",
        m.set_bytes
    );
    assert!(
        m.peak_delta > 0,
        "the install must allocate; M1_PEAK_BYTES=0 means the probe measured nothing"
    );
    assert!(
        m.decoded_bytes > 0,
        "the floor witness must be measurable; M1_DECODED_SET_BYTES=0 means the decode \
         was optimised away and the peak bound cannot be audited"
    );
}

/// REQ-SCALE-014 — Decision: failing this means the M1 install still materialises the
/// whole UTXO set an extra time, which is the RAM ceiling that breaks snap-sync at
/// large set sizes.
/// REQ-SCALE-001 / REQ-SCALE-006 — Decision: the bound is only meaningful while the
/// same install still yields the count and hash asserted in the sibling lock file.
#[tokio::test]
async fn m1_install_peak_is_set_by_the_verify_phase_alone() {
    let m = measure_install(UTXO_COUNT).await;

    assert!(
        m.peak_delta < PRE_M1_PEAK_BYTES,
        "snap install peak M1_PEAK_BYTES={} did not fall below the measured pre-M1 peak {} \
         (M1_SET_BYTES={}, copies={:.1}, n={})",
        m.peak_delta,
        PRE_M1_PEAK_BYTES,
        m.set_bytes,
        m.copies,
        m.installed_count
    );
    let verify_phase_ceiling = m.decoded_bytes + m.set_bytes * 13 / 10;
    assert!(
        m.peak_delta <= verify_phase_ceiling,
        "snap install peak M1_PEAK_BYTES={} exceeds the verify-phase ceiling {} \
         (one decoded set {} + 1.3x one canonical image {}); the install, the \
         atomic_replace or the post-install derivation is still materialising the set",
        m.peak_delta,
        verify_phase_ceiling,
        m.decoded_bytes,
        m.set_bytes
    );
    assert!(
        m.peak_delta >= m.decoded_bytes,
        "the measured peak ({}) fell below one decoded copy ({}); the probe is not \
         measuring the install",
        m.peak_delta,
        m.decoded_bytes
    );
}
