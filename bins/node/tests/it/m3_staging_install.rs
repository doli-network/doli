//! UTXO-scalability M3 [F3] — INSTALL side: staging column family, one atomic promotion.
//!
//! Pins the install filters:
//!   Res-2 / Restructurer c2 — `atomic_replace` DELETES every deletable CF before rewriting
//!     (`writes.rs:201`), so streaming chunks into the LIVE `CF_UTXO` is a self-destructive
//!     read. Chunks land in staging; the live set is untouched until promotion.
//!   3g / F-11 / PM-024 — per-chunk batches give up the one-WriteBatch atomicity the install
//!     has today. The only surviving crash-safety is the `rebuild_in_progress` marker, so it
//!     must be armed across the WHOLE promotion window and a half-installed node must refuse
//!     to serve on restart.
//!   F-10 — a stream or frame hash proves TRANSPORT, not STORAGE. The post-install root must
//!     be re-derived from the INSTALLED backend, because `queries.rs` silently drops
//!     undecodable values.
//!
//! OUTPUT CONTRACT: the staged-install seam on `StateDb`
//!   (`stage_utxo_bytes`, `staged_utxo_len`, `promote_staged_utxos`, `clear_staged_utxos`)
//!   and the marker pair (`set_rebuild_in_progress` / `get_rebuild_in_progress`).
//!   Outputs observable from the stable API:
//!     O1 live `CF_UTXO` contents (via `UtxoSet::canonical_digest` over the installed backend)
//!     O2 staging contents / staged length
//!     O3 `CF_META` survival across promotion (the atomic_replace witness)
//!     O4 `get_rebuild_in_progress()` / `rebuild_halt_reason()` before, during, after
//!     O5 the re-derived state root, read back from the installed backend
//!   Paths:
//!     P1 chunks staged, nothing promoted yet      — E1 (O1, O2)
//!     P2 promotion runs to completion             — E2 (O1, O2, O3)
//!     P3 armed, then interrupted before promotion — E3 (O4)
//!     P4 promotion completes, root re-derived     — E4 (O4, O5)
//!     P5 promotion streams OUT of the staging CF  — E5 (O1, O2)
//!   MATRIX: P1xO1 + P1xO2 -> E1 | P2xO1+O2+O3 -> E2 | P3xO4 -> E3 | P4xO4+O5 -> E4
//!           P5xO1+O2 -> E5
//! INPUT PARTITIONS: (a) a node whose live set is NON-EMPTY before the install, so "live is
//!   unchanged" is a real observation and not the trivial empty case; (b) an incoming set
//!   that DIFFERS from the live one, so the promotion is detectable.

use crypto::Hash;
use storage::{Outpoint, UtxoEntry};

use crate::inc_i_156_m1_harness as h;
use crate::m3_common;

const LIVE_SET: usize = 1_500;
const INCOMING_SET: usize = 2_500;
const CHUNK_BYTES: usize = 64 * 1024;

async fn node_with_live_set() -> (doli_node::node::Node, tempfile::TempDir, Hash) {
    let (node, _kp, temp) = h::make_node(1).await;
    h::install_production_utxo_backend(&node).await;
    for (outpoint, entry) in m3_common::randomized_entries(LIVE_SET, m3_common::FIXTURE_SEED) {
        node.state_db
            .insert_utxo(&outpoint, &entry)
            .expect("fixture: insert_utxo");
    }
    let digest = node
        .utxo_set
        .read()
        .await
        .canonical_digest()
        .expect("live digest");
    (node, temp, digest)
}

/// The incoming set, as a client would carry it: its canonical image split into chunk-sized
/// BODY runs plus the digest the manifest would have announced.
#[allow(clippy::type_complexity)]
fn incoming_chunks() -> (Vec<Vec<u8>>, Hash, u64, Vec<(Outpoint, UtxoEntry)>) {
    let entries = m3_common::randomized_entries(INCOMING_SET, m3_common::FIXTURE_SEED ^ 0x1234);
    let mut set = storage::utxo::UtxoSet::new();
    for (o, e) in entries.iter() {
        set.insert(*o, e.clone()).expect("fixture insert");
    }
    let image = set.serialize_canonical();
    let digest = set.canonical_digest().expect("incoming digest");
    let count = set.utxo_count();

    let bodies = image[8..]
        .chunks(CHUNK_BYTES)
        .map(|c| c.to_vec())
        .collect::<Vec<_>>();
    (bodies, digest, count, entries)
}

// ==================== E1 — chunks land in staging, never in the live CF ====================

/// Res-2 — Decision: a failure here means the download writes into the very column family
/// `atomic_replace` deletes before rewriting. A node interrupted mid-download would then
/// hold a MIXTURE of its old set and a partial new one, with no marker distinguishing it
/// from a healthy node, and it would serve that mixture to the next bootstrapping peer.
#[tokio::test(flavor = "multi_thread")]
async fn m3_chunks_land_in_staging_not_live_cf() {
    let (node, _t, live_before) = node_with_live_set().await;
    let live_len_before = node.state_db.utxo_len();
    let (bodies, _digest, _count, _entries) = incoming_chunks();

    assert!(
        bodies.len() > 1,
        "the incoming fixture must span several chunks, else staging is never exercised"
    );

    for body in bodies.iter() {
        node.state_db
            .stage_utxo_bytes(body)
            .expect("staging a verified chunk must succeed");
    }

    assert!(
        node.state_db.staged_utxo_len() > 0,
        "the staged chunks must be somewhere — a staged length of 0 means the bytes were \
         dropped or written to the live CF"
    );
    assert_eq!(
        node.state_db.utxo_len(),
        live_len_before,
        "the LIVE CF_UTXO row count changed during download (was {}, now {})",
        live_len_before,
        node.state_db.utxo_len()
    );
    assert_eq!(
        node.utxo_set
            .read()
            .await
            .canonical_digest()
            .expect("live digest"),
        live_before,
        "the LIVE canonical digest changed during download — chunks are landing in CF_UTXO"
    );
}

// ==================== E2 — promotion is one atomic replace ====================

/// M1 seam / 3g — Decision: a failure here means the install stopped going through the one
/// `atomic_replace` WriteBatch. That batch is the ONLY reason a crash mid-install leaves a
/// coherent database; per-chunk commits to the live CF trade the milestone's RAM win for a
/// corruption window that nothing in the node detects.
#[tokio::test(flavor = "multi_thread")]
async fn m3_promotion_is_one_atomic_replace() {
    let (node, _t, live_before) = node_with_live_set().await;
    let (bodies, incoming_digest, _count, _entries) = incoming_chunks();

    // A CF_META key that atomic_replace deliberately preserves (writes.rs:216-221). Its
    // survival is the witness that promotion went through atomic_replace and not a
    // hand-rolled delete-all.
    node.state_db
        .set_rebuild_in_progress(4_242)
        .expect("write a CF_META key");

    for body in bodies.iter() {
        node.state_db.stage_utxo_bytes(body).expect("stage");
    }

    let cs = node.chain_state.read().await.clone();
    let ps = node.producer_set.read().await.clone();
    node.state_db
        .promote_staged_utxos(&cs, &ps)
        .expect("promotion must succeed");

    let installed = storage::UtxoSet::from_state_db(node.state_db.clone());
    let after = installed.canonical_digest().expect("installed digest");

    assert_ne!(
        after, live_before,
        "promotion did not change the live set — the staged bytes were never installed"
    );
    assert_eq!(
        after, incoming_digest,
        "the installed canonical digest must equal the digest the manifest announced"
    );
    assert_eq!(
        node.state_db.staged_utxo_len(),
        0,
        "the staging CF must be empty after promotion — leftover staged rows are a disk leak \
         the size of a whole UTXO set"
    );
    assert!(
        node.state_db.get_rebuild_in_progress().is_some(),
        "CF_META must survive the promotion: atomic_replace excludes it from deletable_cfs, \
         so its loss means promotion took a different, non-atomic path"
    );
}

// ==================== E3 — the marker spans the promotion window ====================

/// F-11 / PM-024 — Decision: a failure here means a crash between "delete the live set" and
/// "write the new one" leaves a truncated ledger with NO marker. `BlockHeader` carries no
/// state root, so a wrong set is never caught at block acceptance: the node would serve and
/// produce on a silently truncated ledger indefinitely.
#[tokio::test(flavor = "multi_thread")]
async fn m3_rebuild_markers_armed_across_promotion_window() {
    let (node, _t, _live) = node_with_live_set().await;
    let (bodies, _digest, _count, _entries) = incoming_chunks();

    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "a healthy node must start with the marker disarmed, else the assertions below are \
         vacuous"
    );
    assert!(node.rebuild_halt_reason().is_none());

    for body in bodies.iter() {
        node.state_db.stage_utxo_bytes(body).expect("stage");
    }

    // Arm, as the install must, immediately before the destructive step.
    let target = node.chain_state.read().await.best_height + 1;
    node.state_db
        .set_rebuild_in_progress(target)
        .expect("arm before promotion");

    assert!(
        node.state_db.get_rebuild_in_progress().is_some(),
        "the marker must be Some for the whole promotion window"
    );
    let halted = node
        .rebuild_halt_reason()
        .expect("an armed marker must produce a halt reason");
    assert!(
        halted.contains("[STATE_CORRUPT]"),
        "the halt reason must carry the [STATE_CORRUPT] tag the serve paths refuse on, got {:?}",
        halted
    );

    // The interruption: the process dies here. Nothing clears the marker, so a node that
    // comes back up still refuses to serve.
    assert!(
        node.serve_state_snapshot(node.chain_state.read().await.best_hash)
            .await
            .type_name()
            == "Error",
        "a node interrupted between arm and promote must refuse to serve state"
    );

    // The completed path clears it.
    let cs = node.chain_state.read().await.clone();
    let ps = node.producer_set.read().await.clone();
    node.state_db
        .promote_staged_utxos(&cs, &ps)
        .expect("promotion");
    node.state_db
        .clear_rebuild_in_progress()
        .expect("disarm after promotion");

    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "a completed promotion must disarm the marker, or the node refuses to serve forever"
    );
    assert!(node.rebuild_halt_reason().is_none());
}

// ==================== E4 — the root comes from the INSTALLED backend ====================

/// F-10 / M1 seam — Decision: a failure here means the post-install root is re-derived from
/// the in-memory decode of the wire bytes rather than from RocksDB. That check proves the
/// transport was faithful and proves NOTHING about what landed on disk — and `queries.rs`
/// silently drops undecodable values, so a set that failed to persist correctly would pass.
#[tokio::test(flavor = "multi_thread")]
async fn m3_post_install_root_rederived_from_installed_backend() {
    let (node, _t, _live) = node_with_live_set().await;
    let (bodies, incoming_digest, _count, _entries) = incoming_chunks();

    for body in bodies.iter() {
        node.state_db.stage_utxo_bytes(body).expect("stage");
    }

    let cs = node.chain_state.read().await.clone();
    let ps = node.producer_set.read().await.clone();
    node.state_db
        .promote_staged_utxos(&cs, &ps)
        .expect("promotion");

    // Swap the live handle to the installed backend, as fork_recovery.rs:401 does.
    {
        let mut utxo = node.utxo_set.write().await;
        *utxo = storage::UtxoSet::from_state_db(node.state_db.clone());
    }
    let utxo = node.utxo_set.read().await;

    assert!(
        utxo.is_rocksdb(),
        "the installed set must be the state_db-backed variant (INV-SYNC-014); an InMemory \
         handle would make the re-derivation read the wire decode, not the disk"
    );
    assert_eq!(
        utxo.canonical_digest().expect("installed digest"),
        incoming_digest,
        "the digest read back FROM DISK must equal the manifest's utxo_hash"
    );

    let rederived = storage::compute_state_root(&cs, &utxo, &ps).expect("re-derived root");
    let expected = {
        let mut reference = storage::utxo::UtxoSet::new();
        for (o, e) in m3_common::randomized_entries(INCOMING_SET, m3_common::FIXTURE_SEED ^ 0x1234)
        {
            reference.insert(o, e).expect("reference insert");
        }
        storage::compute_state_root(&cs, &reference, &ps).expect("reference root")
    };
    assert_eq!(
        rederived, expected,
        "the root re-derived from the INSTALLED backend must equal the root of the set that \
         was transferred"
    );
}

// ============ E5 — the staging CF is not a delete target of its own promotion ============

/// Res-2 / F-11 — Decision: a failure here means the promotion's WriteBatch lists the staging
/// column family among the families it wipes (`writes.rs:201`, `deletable_cfs`), i.e. the
/// batch's delete set overlaps the very rows the promotion streams out of. `CF_META` is the
/// precedent for this exclusion (`fork_recovery.rs:364` depends on the rebuild marker
/// surviving). The observable cost of getting it wrong is the one thing nothing downstream
/// detects: a live UTXO set that is a PREFIX of the transferred one. `BlockHeader` carries no
/// state root, so a short ledger is never rejected at block acceptance — the node produces
/// and serves on it until an epoch boundary forks the chain.
///
/// (a) needs `StateDb::deletable_cf_names()` — the accessor `atomic_replace` itself must
/// iterate, so that this assertion reads the real list and not a second copy that can drift —
/// and the public name of the staging CF. Neither exists yet.
#[tokio::test(flavor = "multi_thread")]
async fn m3_staging_cf_survives_the_promotion_batch() {
    let deletable = storage::StateDb::deletable_cf_names();

    // Non-vacuity: an empty or stub list would make the exclusion below trivially true.
    assert!(
        deletable.contains(&storage::state_db::CF_UTXO),
        "deletable_cf_names() must be the list atomic_replace really deletes — CF_UTXO is \
         missing from it, so this accessor is not that list"
    );
    assert!(
        !deletable.contains(&storage::state_db::CF_UTXO_STAGING),
        "the staging CF is in deletable_cfs {:?} — the promotion batch deletes the rows it is \
         promoting FROM",
        deletable
    );

    let (node, _t, live_before) = node_with_live_set().await;
    let (bodies, incoming_digest, incoming_count, entries) = incoming_chunks();
    assert!(
        bodies.len() > 1 && incoming_count as usize == INCOMING_SET,
        "the fixture must span several chunks and carry every entry, else a truncated prefix \
         would be indistinguishable from the whole set"
    );

    for body in bodies.iter() {
        node.state_db.stage_utxo_bytes(body).expect("stage");
    }
    let cs = node.chain_state.read().await.clone();
    let ps = node.producer_set.read().await.clone();
    node.state_db
        .promote_staged_utxos(&cs, &ps)
        .expect("promotion must succeed");

    // (b) every one of the N entries, not a prefix. Row count alone would pass on a set that
    // kept N rows of the OLD ledger, so the membership walk and the digest both run.
    let installed = storage::UtxoSet::from_state_db(node.state_db.clone());
    assert_ne!(
        installed.canonical_digest().expect("installed digest"),
        live_before,
        "the live set is unchanged — nothing was promoted, so completeness below is vacuous"
    );
    assert_eq!(
        node.state_db.utxo_len(),
        INCOMING_SET,
        "the live row count after promotion is {} of {} — a truncated promotion",
        node.state_db.utxo_len(),
        INCOMING_SET
    );
    let missing = entries
        .iter()
        .filter(|(outpoint, _)| node.state_db.get_utxo(outpoint).is_none())
        .count();
    assert_eq!(
        missing, 0,
        "{} of {} staged entries are absent from the live set after promotion",
        missing, INCOMING_SET
    );
    assert_eq!(
        installed.canonical_digest().expect("installed digest"),
        incoming_digest,
        "the live canonical digest after promotion must equal the staged digest"
    );

    // (c) staging is cleared by a SEPARATE write after the batch commits. Re-reading the live
    // set from a fresh disk-backed handle is what separates "cleared afterwards" from
    // "cleared as part of, or before, the commit" — the latter takes the promoted rows with it.
    assert_eq!(
        node.state_db.staged_utxo_len(),
        0,
        "staging still holds {} rows after promotion — a whole UTXO set leaked to disk",
        node.state_db.staged_utxo_len()
    );
    let reread = storage::UtxoSet::from_state_db(node.state_db.clone());
    assert_eq!(
        reread.utxo_count(),
        incoming_count,
        "the live set read back AFTER the staging clear holds {} of {} entries — the clear \
         removed rows the promotion had just written",
        reread.utxo_count(),
        incoming_count
    );
    assert_eq!(
        reread.canonical_digest().expect("re-read digest"),
        incoming_digest,
        "the live digest changed between promotion and the staging clear"
    );
}
