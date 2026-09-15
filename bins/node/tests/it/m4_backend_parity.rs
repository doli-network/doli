//! UTXO-scalability M4 — BIT-IDENTITY. The one outcome M4 may not produce a change in.
//!
//! The milestone's three-question answer is Q1 NO / Q2 NO: no user-submittable transaction
//! and no producer action reaches this path, and the behaviour is bit-identical for every
//! reachable input. This file is the evidence for the third clause. If any assertion here
//! fails, M4 is NOT a storage-and-sync change — it is a consensus change, and it needs an
//! activation height and a synchronised deploy.
//!
//! Res-1 — BOTH backends must produce the same root for the same chunks. The in-memory
//! backend has no staging family and keeps the M3 materialised path, so the equality is
//! asserted here rather than assumed.
//!
//! This file is deliberately written against the seam that EXISTS today
//! (`StateDb::stage_utxo_bytes` / `promote_staged_utxos`), so it compiles and PASSES before
//! the implementation. That is the point: it is the `before` witness. A red-first here would
//! prove nothing — these assertions must hold on both sides of the milestone, and the only
//! informative failure is one the M4 implementation introduces.
//!
//! OUTPUT CONTRACT: the canonical digest and the state root over a set delivered as chunks.
//!   Outputs observable from the stable API:
//!     O1 `UtxoSet::canonical_digest()` on the state_db backend after a STAGED install
//!     O2 `UtxoSet::canonical_digest()` on the InMemory backend after a MATERIALISED install
//!     O3 `storage::compute_state_root` over each
//!     O4 the digest of the reference set built by direct insertion (neither path)
//!   Paths: P1 chunks -> staging -> promotion (production backend)
//!          P2 chunks -> materialised image -> decode (in-memory backend)
//!          P3 direct insertion, no transfer at all (the reference)
//!   MATRIX: P1xO1 == P2xO2 == P3xO4, and P1xO3 == P2xO3
//! INPUT PARTITIONS: (a) a set whose canonical image spans SEVERAL chunk bodies, so a
//!   boundary falls inside a record and the residual carry is exercised; (b) a chunk split
//!   that deliberately does NOT align to record boundaries, because an aligned split would
//!   make the residual path dead and the equality trivial.

use crypto::Hash;
use storage::{Outpoint, UtxoEntry};

use crate::inc_i_156_m1_harness as h;
use crate::m3_common;

const N: usize = 2_500;
const SEED: u64 = m3_common::FIXTURE_SEED ^ 0x7777;
/// Deliberately not a multiple of any record length: a boundary must fall INSIDE a record,
/// or `META_UTXO_STAGING_RESIDUAL` is never exercised and the parity is trivial.
const RAGGED_CHUNK: usize = 7_919;

fn reference_entries() -> Vec<(Outpoint, UtxoEntry)> {
    m3_common::randomized_entries(N, SEED)
}

fn reference_set() -> storage::utxo::UtxoSet {
    let mut set = storage::utxo::UtxoSet::new();
    for (o, e) in reference_entries() {
        set.insert(o, e).expect("reference insert");
    }
    set
}

/// Canonical BODY runs at a ragged split, plus the digest and count the manifest announces.
fn ragged_bodies() -> (Vec<Vec<u8>>, Hash, u64) {
    let set = reference_set();
    let image = set.serialize_canonical();
    let digest = set.canonical_digest().expect("reference digest");
    let count = set.utxo_count();
    let bodies = image[8..]
        .chunks(RAGGED_CHUNK)
        .map(|c| c.to_vec())
        .collect::<Vec<_>>();
    (bodies, digest, count)
}

// ==================== B1 — the staged path reproduces the reference digest ====================

/// REQ-SCALE-006 / INV-SYNC-007 — Decision: a failure here means M4 changed `utxo_hash`, so
/// every node that installs through the new path computes a different state root from a
/// node that did not. That is a fleet-wide fork with no activation height behind it, and it
/// is invisible until the next epoch boundary because `BlockHeader` carries no state root.
#[tokio::test(flavor = "multi_thread")]
async fn m4_staged_install_digest_equals_the_reference() {
    let (node, _kp, _t) = h::make_node(1).await;
    h::install_production_utxo_backend(&node).await;

    let (bodies, expected_digest, expected_count) = ragged_bodies();
    assert!(
        bodies.len() > 2,
        "the fixture must span several chunks with a ragged split, else the residual carry \
         is never exercised"
    );

    for body in bodies.iter() {
        node.state_db.stage_utxo_bytes(body).expect("stage");
    }
    assert_eq!(
        node.state_db.staged_utxo_len() as u64,
        expected_count,
        "staging holds {} of {} rows — a ragged chunk boundary dropped the record it split",
        node.state_db.staged_utxo_len(),
        expected_count
    );

    let cs = node.chain_state.read().await.clone();
    let ps = node.producer_set.read().await.clone();
    node.state_db
        .promote_staged_utxos(&cs, &ps)
        .expect("promotion");

    let installed = storage::UtxoSet::from_state_db(node.state_db.clone());
    assert_eq!(
        installed.utxo_count(),
        expected_count,
        "the promoted set holds {} of {} entries",
        installed.utxo_count(),
        expected_count
    );
    assert_eq!(
        installed.canonical_digest().expect("installed digest"),
        expected_digest,
        "the staged install produced a DIFFERENT canonical digest from the same entries — \
         M4 is a consensus change, not a storage change"
    );
}

// ==================== B2 — Res-1: both backends, one root ====================

/// Res-1 / INV-SYNC-014 — Decision: a failure here means the two backends disagree about
/// the same bytes, so a test node and a production node installing the identical chunk
/// stream land on different state roots. Every in-memory-backed test in the suite then
/// certifies behaviour production does not have, which is precisely how INC-I-152's first
/// attempt passed on broken code.
#[tokio::test(flavor = "multi_thread")]
async fn m4_both_backends_produce_one_root_for_the_same_chunks() {
    let (node, _kp, _t) = h::make_node(1).await;
    h::install_production_utxo_backend(&node).await;

    let (bodies, expected_digest, _count) = ragged_bodies();

    // Production path: chunks -> staging -> promotion.
    for body in bodies.iter() {
        node.state_db.stage_utxo_bytes(body).expect("stage");
    }
    let cs = node.chain_state.read().await.clone();
    let ps = node.producer_set.read().await.clone();
    node.state_db
        .promote_staged_utxos(&cs, &ps)
        .expect("promotion");
    let rocks = storage::UtxoSet::from_state_db(node.state_db.clone());

    // Test-only path: the same chunks reassembled into the M3 materialised image.
    let mut image = rocks.utxo_count().to_le_bytes().to_vec();
    for body in bodies.iter() {
        image.extend_from_slice(body);
    }
    let in_memory =
        storage::utxo::UtxoSet::deserialize_canonical(&image).expect("materialised decode");

    assert!(
        rocks.is_rocksdb() && !in_memory.is_rocksdb(),
        "the two handles must be the two DIFFERENT variants, else this compares a set with \
         itself"
    );
    assert_eq!(
        in_memory.canonical_digest().expect("in-memory digest"),
        expected_digest,
        "the materialised (in-memory) path lost the reference digest"
    );
    assert_eq!(
        rocks.canonical_digest().expect("rocksdb digest"),
        in_memory.canonical_digest().expect("in-memory digest"),
        "the staged (RocksDB) and materialised (InMemory) backends disagree on the canonical \
         digest of the SAME chunks"
    );
    assert_eq!(
        storage::compute_state_root(&cs, &rocks, &ps).expect("rocksdb root"),
        storage::compute_state_root(&cs, &in_memory, &ps).expect("in-memory root"),
        "the two backends produce different STATE ROOTS for the same chunks (Res-1)"
    );
}
