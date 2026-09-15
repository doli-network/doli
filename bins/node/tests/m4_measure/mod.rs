//! UTXO-scalability M4 — the counting allocator and the one measurement routine
//! shared by `m4_install_peak_probe.rs` (green, yields the `before` reading) and
//! `m4_peak_ratio_red.rs` (red, asserts the shape).
//!
//! OUTPUT CONTRACT: N/A — fixture file. It declares no `#[test]`; the enumerations
//! live with the two binaries that include it.
//! INPUT PARTITIONS: N/A — fixture file.
//!
//! The `#[global_allocator]` below is per-BINARY. Both consumers `#[path]`-include
//! THIS file, so there is one copy of the allocator source and two instances of it,
//! each wrapping only its own binary.
//!
//! `measure_download_install` drives the entry points a real receiving node drives —
//! `SyncManager::handle_response` for the manifest and every chunk, then
//! `Node::apply_snap_snapshot` on what `take_snap_snapshot` yields — so the same
//! workload is measured through the pre-M4 and the post-M4 implementation.
//!
//! `rocksdb::WriteBatch` is an FFI handle over C++ `malloc` and never reaches this
//! `#[global_allocator]`, so batch memory is outside the accounting entirely.
//!
//! Known measurement artifact: `staged_len` is read once inside the measured window,
//! between the last chunk and the install. The read is a RocksDB iterator count whose
//! live footprint is one key/value pair at a time, so it is O(1) in the set size and
//! cannot make a staged run look flatter than it is.

#![allow(dead_code)] // each consumer uses a subset

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use crypto::Hash;
use network::protocols::sync::{SyncResponse, STATE_CHUNK_MAX_BYTES};
use network::{PeerId, SyncConfig, SyncManager};

#[path = "../it/m3_common.rs"]
pub mod m3_common;

#[path = "../inc_i_156_m1_harness/mod.rs"]
pub mod h;

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

/// Serialises the two measurements against each other so each peak is attributable
/// to the run that produced it.
pub static MEASURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ==================== fixture constants ====================

pub const PROBE_SEED: u64 = 0x00D0_1100_2026_0915;
const CHAIN_LEN: u64 = 2;
const N_PRODUCERS: usize = 3;
const SESSION_ID: u64 = 0xD0_11_5E_55_10_00_00_04;
/// `snap_quorum()` is `max(3, min(peers/2 + 1, 5))`; five corroborating peers clears it.
const N_PEERS: usize = 5;

// ==================== the measurement ====================

pub struct Measured {
    pub n: usize,
    pub peak_bytes: usize,
    pub set_bytes: usize,
    pub installed_count: u64,
    pub installed_hash: String,
    pub staged_len_at_session_end: u64,
    pub transfer_path: &'static str,
    pub chunks: usize,
}

/// One full client-side snap-sync of an `n`-entry synthetic set: manifest admitted
/// through `handle_response`, every pre-served chunk frame fed through
/// `handle_response`, then `Node::apply_snap_snapshot` on `take_snap_snapshot()`.
///
/// `peak_bytes` is the allocator high-water delta over exactly those steps. The
/// baseline is taken after the served set AND all its chunk frames are built, so both
/// are resident and neither is part of the delta. Nothing the sending peer does runs
/// inside the window.
pub async fn measure_download_install(n: usize, seed: u64) -> Measured {
    let _guard = MEASURE_LOCK.lock().await;

    let (mut node, producers, _temp) = h::make_node(N_PRODUCERS).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    // ---- server side: the set the peer would serve, and the manifest over it ----
    let served = m3_common::build_in_memory(n, seed);
    let utxo_hash = served.canonical_digest().expect("served digest");
    let utxo_count = served.utxo_count();

    let (anchor_hash, anchor_height, chain_state_bytes, producer_set_bytes, state_root, set_bytes) = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let base = storage::StateSnapshot::create(&cs, &utxo, &ps)
            .expect("fixture: StateSnapshot::create must succeed");

        let image = served.serialize_canonical();
        let len = image.len();
        let root = storage::compute_state_root_from_bytes(
            &base.chain_state_bytes,
            &image,
            &base.producer_set_bytes,
        )
        .expect("fixture: root over the served set must compute");
        drop(image);

        (
            base.block_hash,
            base.block_height,
            base.chain_state_bytes,
            base.producer_set_bytes,
            root,
            len,
        )
    };
    let epoch_state_bytes = node.epoch_state.serialize();

    // ---- client side: a manager parked in SnapDownloading against a quorum ----
    // `min_peers_for_sync = 3` holds `start_sync()` back until three peers agree on the
    // anchor. At the default of 1 the FIRST peer starts a header-first epoch, and every
    // later peer is a no-op because `start_sync` refuses to re-enter while syncing.
    let cfg = SyncConfig {
        min_peers_for_sync: N_PEERS,
        ..SyncConfig::default()
    };
    let mut mgr = SyncManager::new(cfg, h::devnet_genesis_hash());
    let mut peers = Vec::with_capacity(N_PEERS);
    for _ in 0..N_PEERS {
        let p = PeerId::random();
        mgr.add_peer(p, anchor_height, anchor_hash, anchor_height as u32);
        peers.push(p);
    }
    for p in peers.iter() {
        mgr.handle_response(
            *p,
            SyncResponse::StateRoot {
                block_hash: anchor_hash,
                block_height: anchor_height,
                state_root,
            },
        );
    }
    assert!(
        mgr.is_snap_syncing(),
        "fixture: the manager must be in the snap pipeline before the manifest arrives"
    );
    // The sink `Node::new` installs in production, over the SAME StateDb the
    // install below reads back from.
    mgr.set_utxo_chunk_sink(std::sync::Arc::new(doli_node::node::StateDbChunkSink::new(
        node.state_db.clone(),
    )));
    let peer = peers[0];

    // ---- serve every chunk body to exhaustion, OUTSIDE the window ----
    let mut frames: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    let mut cursor: Option<Vec<u8>> = None;
    loop {
        let (body, next_key) = served
            .canonical_range(cursor.as_deref(), STATE_CHUNK_MAX_BYTES as usize)
            .expect("canonical_range");
        cursor = next_key.clone();
        frames.push((body, next_key));
        if cursor.is_none() {
            break;
        }
    }
    let chunks = frames.len();
    let frame_bytes_before: usize = frames.iter().map(|(b, _)| b.len()).sum();

    // ==================== MEASURED WINDOW ====================
    let baseline = reset_peak();

    mgr.handle_response(
        peer,
        SyncResponse::StateManifest {
            session_id: SESSION_ID,
            block_hash: anchor_hash,
            block_height: anchor_height,
            state_root,
            utxo_hash,
            utxo_count,
            chunk_max_bytes: STATE_CHUNK_MAX_BYTES,
            chain_state: chain_state_bytes,
            producer_set: producer_set_bytes,
            block_header_bytes: None,
            epoch_bond_snapshot_bytes: None,
            epoch_accumulators_bytes: None,
            epoch_state_bytes: Some(epoch_state_bytes),
        },
    );

    for (body, next_key) in frames.iter() {
        mgr.handle_response(
            peer,
            SyncResponse::StateChunk {
                session_id: SESSION_ID,
                body: body.clone(),
                next_key: next_key.clone(),
            },
        );
    }

    let staged_len_at_session_end = node.state_db.staged_utxo_len() as u64;

    let snapshot = mgr
        .take_snap_snapshot()
        .expect("the session must complete into SnapReady");
    let transfer_path = if snapshot.utxo_set.is_empty() {
        "staged_chunks"
    } else {
        "materialised_image"
    };
    node.apply_snap_snapshot(snapshot)
        .await
        .expect("the install must succeed on a digest-verified session");

    let observed = peak();
    // ==================== /MEASURED WINDOW ====================

    // The frames were cloned in, never moved out: still whole, so still baseline.
    let frame_bytes_after: usize = frames.iter().map(|(b, _)| b.len()).sum();
    assert_eq!(
        frame_bytes_before, frame_bytes_after,
        "pre-served frames must stay resident across the window"
    );
    assert_eq!(chunks, frames.len(), "frame count must not move");

    let installed = {
        let utxo = node.utxo_set.read().await;
        (
            utxo.utxo_count(),
            utxo.canonical_digest().expect("installed digest").to_hex(),
        )
    };

    Measured {
        n,
        peak_bytes: observed.saturating_sub(baseline),
        set_bytes,
        installed_count: installed.0,
        installed_hash: installed.1,
        staged_len_at_session_end,
        transfer_path,
        chunks,
    }
}

/// The manifest digest the served set announces, without running a session.
pub fn served_digest(n: usize, seed: u64) -> Hash {
    m3_common::build_in_memory(n, seed)
        .canonical_digest()
        .expect("served digest")
}
