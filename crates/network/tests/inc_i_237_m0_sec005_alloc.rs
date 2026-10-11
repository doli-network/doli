//! INC-I-237 M0 / REQ-GRV-SEC-005 (C7 b) — adversarial 0xBA headers never
//! pre-allocate more than 2x the wire bytes. Own binary: counting global allocator.
//!
//! OUTPUT CONTRACT: fn decode_tx_gossip(data) -> Option<TxGossipMessage>
//!   O1: none   O2: none   O3: None for every adversarial input (decode result
//!   is unchanged by a fix)   O4: none
//!   O5: heap — peak growth during the call (counting allocator)   O6: none
//! PATHS: P1 count header larger than the body can hold (early None after the
//!   pre-allocation), P2 0xAA announce with a short body, P3 legacy single-tx
//!   whose inner Vec length is huge (bincode/serde prealloc, recorded only)
//! INPUT PARTITIONS: P1a count 10_000 + 4 KB zero body, P1b count 10_000 + 64 B
//!   body, P1c count 10_000 + one valid tx then a truncated length, P1d count
//!   10_000 + 1 MiB zero body; P2a count 1_000 + 64 B; P3a inputs len 2^40
//! MATRIX: O3 x O5 for P1a-d, P2a; O3 + recorded O5 for P3a.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use network::gossip::decode_tx_gossip;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// The two tests share one allocator: run their bodies one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

struct CountingAlloc;

fn grow(bytes: usize) {
    let now = CURRENT.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            grow(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            grow(layout.size());
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
                grow(new_size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// Peak heap growth of one decode call, and whether it decoded.
fn measure(wire: &[u8]) -> (usize, bool) {
    let base = CURRENT.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let decoded = decode_tx_gossip(wire).is_some();
    (PEAK.load(Ordering::Relaxed).saturating_sub(base), decoded)
}

fn header(prefix: u8, count: u32, body: &[u8]) -> Vec<u8> {
    let mut b = vec![prefix];
    b.extend_from_slice(&count.to_le_bytes());
    b.extend_from_slice(body);
    b
}

fn min_tx() -> Vec<u8> {
    vec![
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ]
}

/// C7 (b) RECORDED, not bounded (outside `decode_tx_batch`; M2 input): serde's
/// cautious prealloc (1 MiB cap / 140-byte `Input` = 7_489 slots) for a legacy tx
/// whose `inputs` length claims 2^40 elements. Measured peak on v6.43.3: 1_058_183.
const LEGACY_INNER_VEC_PREALLOC_RECORDED: usize = 1_048_460;

// REQ-GRV-SEC-005 — Decision: a failure means a short 0xBA message with a large count header makes `decode_tx_batch` reserve more than 2x its wire size before it fails, so a peer amplifies our memory per message.
#[test]
fn req_grv_sec_005_adversarial_batch_headers_never_preallocate_over_two_x_wire() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut valid_then_truncated = Vec::new();
    valid_then_truncated.extend_from_slice(&32u32.to_le_bytes());
    valid_then_truncated.extend_from_slice(&min_tx());
    valid_then_truncated.extend_from_slice(&u32::MAX.to_le_bytes());
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "0xBA count 10_000 + 4 KB zeros",
            header(0xBA, 10_000, &[0u8; 4_000]),
        ),
        (
            "0xBA count 10_000 + 64 B zeros",
            header(0xBA, 10_000, &[0u8; 64]),
        ),
        (
            "0xBA count 10_000 + 1 tx + truncated",
            header(0xBA, 10_000, &valid_then_truncated),
        ),
        (
            "0xBA count 10_000 + 1 MiB zeros",
            header(0xBA, 10_000, &vec![0u8; 1 << 20]),
        ),
        ("0xAA count 1_000 + 64 B", header(0xAA, 1_000, &[0u8; 64])),
    ];
    let mut over = Vec::new();
    for (name, wire) in &cases {
        let (peak, decoded) = measure(wire);
        eprintln!(
            "C7(b) {name}: wire={} peak={peak} ratio={:.2}",
            wire.len(),
            peak as f64 / wire.len() as f64
        );
        assert!(!decoded, "{name}: adversarial input must still not decode");
        if peak > 2 * wire.len() {
            over.push(format!("{name}: peak {peak} > 2 x {}", wire.len()));
        }
    }
    assert!(
        over.is_empty(),
        "SEC-005 pre-allocation over 2x wire:\n{}",
        over.join("\n")
    );
}

// REQ-GRV-SEC-005 — Decision: a failure means serde/bincode's inner-Vec preallocation for a legacy tx changed, so the recorded per-message amplification M2 must budget for is stale.
#[test]
fn req_grv_sec_005_legacy_inner_vec_prealloc_is_recorded() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut legacy = vec![1, 0, 0, 0, 0, 0, 0, 0];
    legacy.extend_from_slice(&(1u64 << 40).to_le_bytes());
    legacy.extend_from_slice(&[0u8; 24]);
    let (peak, decoded) = measure(&legacy);
    eprintln!("C7(b) legacy inner vec: wire={} peak={peak}", legacy.len());
    assert!(!decoded);
    let band =
        LEGACY_INNER_VEC_PREALLOC_RECORDED - 65_536..LEGACY_INNER_VEC_PREALLOC_RECORDED + 65_536;
    assert!(
        band.contains(&peak),
        "recorded M2 input moved: peak {peak} outside {band:?}"
    );
}
