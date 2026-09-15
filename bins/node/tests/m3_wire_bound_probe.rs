//! UTXO-scalability M3 [F3] — the OUTCOME PROBE (docs/.workflow/m3-outcome-metric.txt).
//!
//! REQ-SCALE-013 — Decision: the primary metric. `M3_MAX_WIRE_BYTES` is the largest single
//! bincode-encoded `SyncResponse` a receiving node must accept to obtain the full UTXO state
//! of a peer. When it exceeds `MAX_SYNC_SIZE` the state is not slow to transfer, it is
//! UNTRANSFERABLE (`M3_SYNC_OK=0`), which is the failure Stage 3 exists to remove.
//! REQ-SCALE-006 / INV-SYNC-007 — Decision: `M3_UTXO_HASH` must be IDENTICAL in the before
//! and after runs. A changed digest would mean the wire win was bought with a consensus
//! change, which is a different (and unacceptable) outcome.
//! REQ-SCALE-018 — Decision: `M3_INSTALL_PEAK_BYTES` is the secondary metric; a peak that
//! does not fall would mean the receiving side still materialises the whole set.
//!
//! OUTPUT CONTRACT: this file is a MEASUREMENT harness. Its observable outputs are the
//!   `M3_*` lines on stdout; it asserts only the non-vacuity of what it measured.
//!     O1 M3_MAX_WIRE_BYTES          O2 M3_SYNC_OK
//!     O3 M3_INSTALL_PEAK_BYTES      O4 M3_UTXO_HASH
//!     O5 M3_CANONICAL_IMAGE_BYTES   O6 M3_UTXO_COUNT
//!   Paths: P1 a set whose canonical image EXCEEDS MAX_SYNC_SIZE — O1, O2, O4, O5, O6
//!          P2 n = 100_000, the M1/M2 install-peak comparison point — O3
//! INPUT PARTITIONS: one class per path; both are seeded, so the numbers are comparable
//!   across runs and across machines.
//!
//! Its OWN test binary on purpose: the counting `#[global_allocator]` below is per-BINARY,
//! so folding it into `tests/it/` would wrap every unrelated node test and make the measured
//! peak unattributable. Declared in `bins/node/Cargo.toml`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use network::VerifiedSnapshot;

#[path = "it/m3_common.rs"]
mod m3_common;

#[path = "inc_i_156_m1_harness/mod.rs"]
mod h;

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

fn reset_peak() -> usize {
    let base = CURRENT.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    base
}

fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

static MEASURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ==================== fixture constants ====================

/// Mirror of the private `MAX_SYNC_SIZE` in `crates/network/src/protocols/sync.rs:22`.
/// The real constant is pinned by `m3_max_sync_size_unchanged` in
/// `crates/network/tests/it/m3_protocol_session.rs`; this copy exists only because the
/// constant is not exported today and this probe must compile against UNCHANGED code.
const MAX_SYNC_SIZE: usize = 16 * 1024 * 1024;

const PROBE_SEED: u64 = 0x00D0_1100_2026_0915;
/// The M1/M2 install-peak comparison point.
const INSTALL_N: usize = 100_000;
const CHAIN_LEN: u64 = 2;
const N_PRODUCERS: usize = 3;

// ==================== THE MEASUREMENT SEAM ====================
//
// Returns (largest single encoded SyncResponse the transfer needs, path label).
//
// TODAY there is no session API, so the ONLY way to move a UTXO set is the single-frame
// `SyncResponse::StateSnapshot` — one message carrying the whole canonical image. That is
// the `before` reading, and it is a legitimate PASS for this one test.
//
// AFTER M3 the developer replaces this body with a manifest + chunk walk and returns the
// LARGEST of `{StateManifest, StateChunk...}`. The probe command line does not change.

fn largest_message_for_transfer(
    set: &storage::utxo::UtxoSet,
    chain_state: Vec<u8>,
    producer_set: Vec<u8>,
    block_hash: crypto::Hash,
    block_height: u64,
    state_root: crypto::Hash,
) -> (usize, &'static str) {
    use network::protocols::sync::{SyncResponse, STATE_CHUNK_MAX_BYTES};

    let (utxo_hash, _) = set
        .canonical_digest()
        .map(|h| (h, ()))
        .expect("canonical_digest");

    let manifest = SyncResponse::StateManifest {
        session_id: 1,
        block_hash,
        block_height,
        state_root,
        utxo_hash,
        utxo_count: set.utxo_count(),
        chunk_max_bytes: STATE_CHUNK_MAX_BYTES,
        chain_state,
        producer_set,
        block_header_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
        epoch_state_bytes: None,
    };
    let mut max_wire = bincode::serialize(&manifest)
        .expect("the manifest must encode")
        .len();

    let mut cursor: Option<Vec<u8>> = None;
    let mut chunks = 0usize;
    loop {
        let (body, next_key) = set
            .canonical_range(cursor.as_deref(), STATE_CHUNK_MAX_BYTES as usize)
            .expect("canonical_range");
        let frame = SyncResponse::StateChunk {
            session_id: 1,
            body,
            next_key: next_key.clone(),
        };
        let encoded = bincode::serialize(&frame).expect("the chunk must encode");
        max_wire = max_wire.max(encoded.len());
        chunks += 1;
        match next_key {
            Some(k) => cursor = Some(k),
            None => break,
        }
    }
    assert!(
        chunks > 1,
        "the session walked the whole set in {} message(s) — the chunked path is not being \
         measured",
        chunks
    );

    (max_wire, "session_manifest_chunks")
}

// ==================== F1 — the wire bound ====================

/// REQ-SCALE-013 — Decision: this is the milestone's externally observable number. A run
/// that prints `M3_SYNC_OK=0` says a real peer cannot put this state on a real socket in the
/// messages the protocol offers; a run that prints `M3_SYNC_OK=1` with a bounded
/// `M3_MAX_WIRE_BYTES` says it can. Nothing here is a test count or an agent verdict.
#[test]
fn m3_wire_bound_probe() {
    let (n, image_bytes) = m3_common::entries_exceeding(MAX_SYNC_SIZE, PROBE_SEED);
    let set = m3_common::build_in_memory(n, PROBE_SEED);

    let utxo_count = set.utxo_count();
    let utxo_hash = set.canonical_digest().expect("canonical_digest").to_hex();
    let image = set.serialize_canonical();
    assert_eq!(
        image.len(),
        image_bytes,
        "the measured image size must match canonical_len, else the probe is reporting two \
         different sets"
    );
    assert!(
        image_bytes > MAX_SYNC_SIZE,
        "M3_CANONICAL_IMAGE_BYTES={} does not exceed MAX_SYNC_SIZE={} — the bound is argued, \
         not exercised",
        image_bytes,
        MAX_SYNC_SIZE
    );
    drop(image);

    let chain_state = vec![0u8; 256];
    let producer_set = vec![0u8; 4_096];
    let (max_wire, path) = largest_message_for_transfer(
        &set,
        chain_state,
        producer_set,
        crypto::Hash::from_bytes([0x11; 32]),
        1_000_000,
        crypto::Hash::from_bytes([0x22; 32]),
    );
    let sync_ok = u8::from(max_wire <= MAX_SYNC_SIZE);

    // Leading newline: libtest leaves the `test <name> ...` prefix unterminated, which would
    // otherwise make the first witness line un-greppable at line start.
    println!();
    println!("M3_MAX_WIRE_BYTES={}", max_wire);
    println!("M3_SYNC_OK={}", sync_ok);
    println!("M3_UTXO_HASH={}", utxo_hash);
    println!("M3_CANONICAL_IMAGE_BYTES={}", image_bytes);
    println!("M3_UTXO_COUNT={}", utxo_count);
    println!("M3_TRANSFER_PATH={}", path);

    assert_eq!(
        utxo_count, n as u64,
        "the measured set must hold all {} entries",
        n
    );
    assert_eq!(
        utxo_hash.len(),
        64,
        "the witness must be a 32-byte hex hash"
    );
    assert!(
        max_wire > 0,
        "M3_MAX_WIRE_BYTES=0 means the probe measured nothing"
    );
}

// ==================== F2 — the install peak ====================

/// REQ-SCALE-018 — Decision: the secondary metric. A client that still holds the whole set
/// in RAM to install it has moved the O(N) cost from the wire to the heap, which is the same
/// ceiling one layer down.
#[tokio::test(flavor = "multi_thread")]
async fn m3_install_peak_at_100k() {
    let guard = MEASURE_LOCK.lock().await;

    let (mut node, producers, _temp) = h::make_node(N_PRODUCERS).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    let snapshot = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let base = storage::StateSnapshot::create(&cs, &utxo, &ps)
            .expect("fixture: StateSnapshot::create must succeed");

        let big = m3_common::build_in_memory(INSTALL_N, PROBE_SEED ^ 0xBEEF);
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

    let baseline = reset_peak();
    node.apply_snap_snapshot(snapshot)
        .await
        .expect("the install must succeed on a valid snapshot");
    let observed = peak();
    drop(guard);

    let peak_delta = observed.saturating_sub(baseline);
    let installed = {
        let utxo = node.utxo_set.read().await;
        (
            utxo.utxo_count(),
            utxo.canonical_digest().expect("installed digest").to_hex(),
        )
    };

    println!();
    println!("M3_INSTALL_PEAK_BYTES={}", peak_delta);
    println!("M3_INSTALL_SET_BYTES={}", set_bytes);
    println!("M3_INSTALL_UTXO_COUNT={}", installed.0);
    println!("M3_INSTALL_UTXO_HASH={}", installed.1);
    println!(
        "M3_INSTALL_FULL_SET_COPIES={:.2}",
        peak_delta as f64 / set_bytes as f64
    );

    assert_eq!(
        installed.0, INSTALL_N as u64,
        "the install must land all {} entries, else the peak describes a different workload",
        INSTALL_N
    );
    assert!(
        peak_delta > 0,
        "M3_INSTALL_PEAK_BYTES=0 means the probe measured nothing"
    );
    assert!(
        set_bytes > 1_000_000,
        "M3_INSTALL_SET_BYTES={} is too small to dominate harness noise",
        set_bytes
    );
}
