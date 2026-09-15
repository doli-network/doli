//! UTXO-scalability M4 [F3] — the CLIENT streams verified chunks into a sink instead of
//! reassembling them.
//!
//! `crates/network` has no `storage` dependency, so the session client cannot call
//! `StateDb::stage_utxo_bytes`. The seam is a trait the network crate defines and
//! `bins/node` implements over the `StateDb` (memory.db decision 130). These tests use a
//! recording mock in its place, so they pin the CONTRACT — which calls the session makes,
//! in which order, on which path — without a storage dependency.
//!
//! OUTPUT CONTRACT: the session client
//!   (`handle_state_manifest`, `handle_state_chunk`, `handle_state_session_unavailable`,
//!    `clear_state_session`) driven through `SyncManager::handle_response`, with a
//!    `UtxoChunkSink` installed.
//!   Outputs observable from the stable API:
//!     O1 the sink's `stage` call log — which bodies, in which order
//!     O2 the sink's `clear` call count
//!     O3 `take_snap_snapshot()` — Some/None, and the SHAPE of what it carries
//!     O4 `VerifiedSnapshot.utxo_set` — empty on the staged path
//!     O5 `VerifiedSnapshot.utxo_staged` — the marker (digest + count)
//!   Paths:
//!     P1 sink installed, stream completes, digest matches     — E1 (O1, O3, O4, O5)
//!     P2 sink installed, digest MISMATCHES at session end     — E2 (O2, O3)
//!     P3 sink installed, a `stage` call FAILS mid-stream      — E3 (O1, O2, O3)
//!     P4 sink installed, session cleared / peer switched      — E4 (O2)
//!     P5 sink installed, `ManifestExpired` refusal            — E5 (O2)
//!     P6 NO sink installed (in-memory backend fallback)       — E6 (O3, O4, O5)
//!   MATRIX: P1xO1+O3+O4+O5 -> E1 | P2xO2+O3 -> E2 | P3xO1+O2+O3 -> E3
//!           P4xO2 -> E4 | P5xO2 -> E5 | P6xO3+O4+O5 -> E6
//! INPUT PARTITIONS: (a) a stream of SEVERAL chunks, so "every body reached the sink in
//!   order" is a real observation and not the single-chunk degenerate case; (b) a final
//!   chunk whose reassembly differs from the manifest, so the quarantine arm is exercised;
//!   (c) a sink that fails on a chosen call index, so the write-error arm is INJECTED
//!   rather than argued.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crypto::Hash;
use network::protocols::sync::{StateSessionRefusal, SyncResponse, STATE_CHUNK_MAX_BYTES};
use network::sync::UtxoChunkSink;
use network::{PeerId, SyncConfig, SyncManager};

const SESSION_ID: u64 = 0xD0_11_5E_55_10_00_00_04;
const N_PEERS: usize = 5;
const ANCHOR_HEIGHT: u64 = 200_000;
const N_CHUNKS: usize = 4;
const CHUNK_LEN: usize = 4_096;

// ==================== the recording mock ====================

/// A `UtxoChunkSink` that records every call and can be told to fail on the n-th `stage`.
#[derive(Default)]
struct RecordingSink {
    staged: Mutex<Vec<Vec<u8>>>,
    clears: AtomicUsize,
    /// `stage` call index (0-based) that returns `Err`; `usize::MAX` never fails.
    fail_at: usize,
}

impl RecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            fail_at: usize::MAX,
            ..Default::default()
        })
    }

    fn failing_at(idx: usize) -> Arc<Self> {
        Arc::new(Self {
            fail_at: idx,
            ..Default::default()
        })
    }

    fn staged_bodies(&self) -> Vec<Vec<u8>> {
        self.staged.lock().expect("sink log").clone()
    }

    fn clear_count(&self) -> usize {
        self.clears.load(Ordering::Relaxed)
    }
}

impl UtxoChunkSink for RecordingSink {
    fn stage(&self, body: &[u8]) -> Result<(), String> {
        let mut log = self.staged.lock().expect("sink log");
        if log.len() == self.fail_at {
            return Err("injected staging write error (disk full)".to_string());
        }
        log.push(body.to_vec());
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        self.clears.fetch_add(1, Ordering::Relaxed);
        self.staged.lock().expect("sink log").clear();
        Ok(())
    }

    fn staged_len(&self) -> u64 {
        self.staged.lock().expect("sink log").len() as u64
    }
}

// ==================== fixture ====================

fn hh(tag: &[u8]) -> Hash {
    crypto::hash::hash(tag)
}

/// A manager parked in `SnapDownloading` against a corroborating quorum, reached ONLY
/// through public API: `min_peers_for_sync = N_PEERS` holds `start_sync()` back until every
/// peer agrees on the anchor, so the snap branch (not header-first) is the one taken.
fn mgr_downloading(state_root: Hash, anchor: Hash) -> (SyncManager, Vec<PeerId>) {
    let cfg = SyncConfig {
        min_peers_for_sync: N_PEERS,
        ..SyncConfig::default()
    };
    let mut mgr = SyncManager::new(cfg, hh(b"m4_genesis"));
    let mut peers = Vec::with_capacity(N_PEERS);
    for _ in 0..N_PEERS {
        let p = PeerId::random();
        mgr.add_peer(p, ANCHOR_HEIGHT, anchor, ANCHOR_HEIGHT as u32);
        peers.push(p);
    }
    for p in peers.iter() {
        mgr.handle_response(
            *p,
            SyncResponse::StateRoot {
                block_hash: anchor,
                block_height: ANCHOR_HEIGHT,
                state_root,
            },
        );
    }
    assert!(
        mgr.is_snap_syncing(),
        "fixture: the manager must reach the snap pipeline, else every assertion is vacuous"
    );
    (mgr, peers)
}

/// `N_CHUNKS` distinguishable bodies plus the digest the manifest announces over
/// `count_le || bodies`.
fn bodies_and_digest(count: u64) -> (Vec<Vec<u8>>, Hash) {
    let bodies: Vec<Vec<u8>> = (0..N_CHUNKS)
        .map(|i| vec![(i as u8).wrapping_add(1); CHUNK_LEN])
        .collect();
    let mut image = count.to_le_bytes().to_vec();
    for b in bodies.iter() {
        image.extend_from_slice(b);
    }
    (bodies, crypto::hash::hash(&image))
}

fn cursor(i: usize) -> Vec<u8> {
    let mut k = vec![0u8; 36];
    k[35] = i as u8;
    k
}

#[allow(clippy::too_many_arguments)]
fn deliver_manifest(mgr: &mut SyncManager, peer: PeerId, anchor: Hash, root: Hash, digest: Hash) {
    mgr.handle_response(
        peer,
        SyncResponse::StateManifest {
            session_id: SESSION_ID,
            block_hash: anchor,
            block_height: ANCHOR_HEIGHT,
            state_root: root,
            utxo_hash: digest,
            utxo_count: N_CHUNKS as u64,
            chunk_max_bytes: STATE_CHUNK_MAX_BYTES,
            chain_state: vec![1u8; 64],
            producer_set: vec![2u8; 64],
            block_header_bytes: None,
            epoch_bond_snapshot_bytes: None,
            epoch_accumulators_bytes: None,
            epoch_state_bytes: None,
        },
    );
}

fn deliver_chunk(mgr: &mut SyncManager, peer: PeerId, body: Vec<u8>, next_key: Option<Vec<u8>>) {
    mgr.handle_response(
        peer,
        SyncResponse::StateChunk {
            session_id: SESSION_ID,
            body,
            next_key,
        },
    );
}

/// Drive a complete, digest-matching session with `sink` installed.
fn run_session(sink: Arc<RecordingSink>) -> (SyncManager, Hash) {
    let root = hh(b"m4_quorum_root");
    let anchor = hh(b"m4_anchor");
    let (mut mgr, peers) = mgr_downloading(root, anchor);
    mgr.set_utxo_chunk_sink(sink);

    let (bodies, digest) = bodies_and_digest(N_CHUNKS as u64);
    deliver_manifest(&mut mgr, peers[0], anchor, root, digest);
    for (i, body) in bodies.iter().enumerate() {
        let last = i + 1 == bodies.len();
        deliver_chunk(
            &mut mgr,
            peers[0],
            body.clone(),
            if last { None } else { Some(cursor(i + 1)) },
        );
    }
    (mgr, digest)
}

// ==================== E1 — every verified body reaches the sink ====================

/// REQ-SCALE-014 — Decision: a failure here means the client still accumulates the whole
/// canonical image in `StateSessionClient.image` and hands it on as a `Vec<u8>`. The peak
/// RAM a node needs to join the network from a snapshot then keeps growing with the UTXO
/// set — the ceiling this milestone exists to remove, one layer below M3's wire bound.
/// INV-SYNC-007 — Decision: bodies staged OUT OF ORDER or with one dropped would install a
/// set whose digest cannot match quorum, and nothing at block acceptance would catch it.
#[test]
fn m4_every_verified_chunk_is_staged_in_order_and_no_image_is_carried() {
    let sink = RecordingSink::new();
    let (mut mgr, digest) = run_session(sink.clone());

    let (expected_bodies, _) = bodies_and_digest(N_CHUNKS as u64);
    assert_eq!(
        sink.staged_bodies(),
        expected_bodies,
        "the sink must receive every verified body, in wire order — a missing or reordered \
         body installs a set the manifest digest can never describe"
    );

    let snapshot = mgr
        .take_snap_snapshot()
        .expect("a digest-matching session must complete into SnapReady");
    assert!(
        snapshot.utxo_set.is_empty(),
        "VerifiedSnapshot still carries {} B of UTXO image — the O(set) term moved from the \
         wire to the heap, it did not go away",
        snapshot.utxo_set.len()
    );
    let marker = snapshot
        .utxo_staged
        .as_ref()
        .expect("a staged session must carry the staged marker instead of an image");
    assert_eq!(
        marker.utxo_hash, digest,
        "the marker must carry the manifest digest, so the install can re-check what it \
         promotes"
    );
    assert_eq!(
        marker.utxo_count, N_CHUNKS as u64,
        "the marker must carry the manifest entry count, so a truncated promotion is \
         detectable"
    );
}

// ==================== E2 — a bad digest quarantines, it never promotes ====================

/// REQ-SCALE-006 / INV-SYNC-007 — Decision: a failure here means bytes that never passed the
/// manifest digest are left in the staging family where the next session concatenates them
/// onto a second peer's chunks. The resulting image matches NEITHER manifest, and a hostile
/// peer gets to choose what a bootstrapping node installs.
#[test]
fn m4_digest_mismatch_clears_staging_and_promotes_nothing() {
    let sink = RecordingSink::new();
    let root = hh(b"m4_quorum_root");
    let anchor = hh(b"m4_anchor");
    let (mut mgr, peers) = mgr_downloading(root, anchor);
    mgr.set_utxo_chunk_sink(sink.clone());

    let (bodies, digest) = bodies_and_digest(N_CHUNKS as u64);
    deliver_manifest(&mut mgr, peers[0], anchor, root, digest);

    for (i, body) in bodies.iter().enumerate() {
        let last = i + 1 == bodies.len();
        // The last body arrives corrupted by one byte: the transport succeeded, the
        // content did not.
        let mut delivered = body.clone();
        if last {
            delivered[0] ^= 0xFF;
        }
        deliver_chunk(
            &mut mgr,
            peers[0],
            delivered,
            if last { None } else { Some(cursor(i + 1)) },
        );
    }

    assert!(
        mgr.take_snap_snapshot().is_none(),
        "a session whose digest does not match the manifest must NOT become SnapReady"
    );
    assert!(
        sink.clear_count() >= 1,
        "the staging family must be cleared when the digest refuses — quarantined bytes left \
         behind are spliced onto the next peer's stream"
    );
    assert_eq!(
        sink.staged_len(),
        0,
        "staging still holds {} rows after a refused digest",
        sink.staged_len()
    );
}

// ==================== E3 — a staging write error abandons the session ====================

/// REQ-SCALE-002 / INC-I-204 — Decision: a failure here means a disk-full or RocksDB error
/// mid-download is swallowed and the session continues, so the promotion installs whatever
/// subset of the set happened to be writable. INC-I-204's lesson is the shape of the fix:
/// the error arm DROPS the in-flight transfer and leaves committed state alone; it must not
/// attempt repair and must not carry on.
#[test]
fn m4_staging_write_error_abandons_the_session_and_clears_staging() {
    let sink = RecordingSink::failing_at(2);
    let root = hh(b"m4_quorum_root");
    let anchor = hh(b"m4_anchor");
    let (mut mgr, peers) = mgr_downloading(root, anchor);
    mgr.set_utxo_chunk_sink(sink.clone());

    let (bodies, digest) = bodies_and_digest(N_CHUNKS as u64);
    deliver_manifest(&mut mgr, peers[0], anchor, root, digest);
    for (i, body) in bodies.iter().enumerate() {
        let last = i + 1 == bodies.len();
        deliver_chunk(
            &mut mgr,
            peers[0],
            body.clone(),
            if last { None } else { Some(cursor(i + 1)) },
        );
    }

    assert!(
        mgr.take_snap_snapshot().is_none(),
        "a session that could not stage a chunk must NOT complete into SnapReady — the \
         promotion would install the subset that happened to be writable"
    );
    assert!(
        sink.clear_count() >= 1,
        "a staging write error must clear staging: the partial rows belong to a transfer \
         that can never be completed"
    );
    assert_eq!(
        sink.staged_len(),
        0,
        "staging still holds {} rows after a failed write",
        sink.staged_len()
    );
}

// ==================== E4 / E5 — every abandon path clears staging ====================

/// PM-006 / INC-I-179 — Decision: a failure here means staging left over from peer A is
/// silently concatenated with peer B's chunks on the next attempt. The digest then matches
/// neither manifest, every retry fails the same way, and the node burns its 3-attempt snap
/// budget without one line of evidence naming the cause.
#[test]
fn m4_peer_switch_and_manifest_expiry_both_clear_staging() {
    for (label, abandon) in [
        (
            "peer switch / session cleared",
            Box::new(|mgr: &mut SyncManager, _p: PeerId| mgr.snap_fallback_to_normal())
                as Box<dyn Fn(&mut SyncManager, PeerId)>,
        ),
        (
            "manifest expired",
            Box::new(|mgr: &mut SyncManager, p: PeerId| {
                mgr.handle_response(
                    p,
                    SyncResponse::StateSessionUnavailable {
                        session_id: SESSION_ID,
                        reason: StateSessionRefusal::ManifestExpired,
                    },
                );
            }),
        ),
    ] {
        let sink = RecordingSink::new();
        let root = hh(b"m4_quorum_root");
        let anchor = hh(b"m4_anchor");
        let (mut mgr, peers) = mgr_downloading(root, anchor);
        mgr.set_utxo_chunk_sink(sink.clone());

        let (bodies, digest) = bodies_and_digest(N_CHUNKS as u64);
        deliver_manifest(&mut mgr, peers[0], anchor, root, digest);
        deliver_chunk(&mut mgr, peers[0], bodies[0].clone(), Some(cursor(1)));
        assert_eq!(
            sink.staged_len(),
            1,
            "[{label}] the fixture must stage something before abandoning, else the clear \
             below is vacuous"
        );

        abandon(&mut mgr, peers[0]);

        assert!(
            sink.clear_count() >= 1,
            "[{label}] the abandon path did not clear staging"
        );
        assert_eq!(
            sink.staged_len(),
            0,
            "[{label}] staging still holds {} rows after abandoning",
            sink.staged_len()
        );
    }
}

// ==================== E6 — no sink installed keeps the M3 materialised path ====================

/// INV-SYNC-014 / Res-1 — Decision: a failure here means a node with no staging family (the
/// in-memory test backend) silently loses the transfer instead of falling back. The fallback
/// is what keeps the two backends comparable, and Res-1 requires BOTH to produce the same
/// root for the same chunks.
#[test]
fn m4_without_a_sink_the_session_still_materialises_the_image() {
    let root = hh(b"m4_quorum_root");
    let anchor = hh(b"m4_anchor");
    let (mut mgr, peers) = mgr_downloading(root, anchor);
    // deliberately NO set_utxo_chunk_sink

    let (bodies, digest) = bodies_and_digest(N_CHUNKS as u64);
    deliver_manifest(&mut mgr, peers[0], anchor, root, digest);
    for (i, body) in bodies.iter().enumerate() {
        let last = i + 1 == bodies.len();
        deliver_chunk(
            &mut mgr,
            peers[0],
            body.clone(),
            if last { None } else { Some(cursor(i + 1)) },
        );
    }

    let snapshot = mgr
        .take_snap_snapshot()
        .expect("with no sink the session must still complete on the M3 materialised path");
    assert!(
        snapshot.utxo_staged.is_none(),
        "a sink-less session must NOT claim rows are staged — nothing staged them"
    );
    assert_eq!(
        crypto::hash::hash(&snapshot.utxo_set),
        digest,
        "the materialised image must still hash to the manifest digest"
    );
}
