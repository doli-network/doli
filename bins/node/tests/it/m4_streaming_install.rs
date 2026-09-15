//! UTXO-scalability M4 [F1] — the INSTALL side of the streaming snap-sync install.
//!
//! M3 proved the staging seam works when called by hand (`m3_staging_install.rs`). M4 wires
//! it: `Node::apply_snap_snapshot` must PROMOTE rows that are already in the staging family
//! instead of decoding a whole image, and every abandon path — including a restart — must
//! clear staging.
//!
//! Pins the install filters from `specs/utxo-scalability-architecture.md`:
//!   F-10 — the post-install root is re-derived from the INSTALLED backend, never from the
//!     in-memory decode of the wire bytes.
//!   F-11 — `rebuild_in_progress` + `rebuild_halt_reason` span the whole promotion window
//!     and are cleared on the Ok arm ONLY.
//!   Res-2 — the staging clear is a SEPARATE post-batch write.
//!
//! OUTPUT CONTRACT: `Node::apply_snap_snapshot` on a snapshot carrying a STAGED marker, and
//!   `Node::reconcile_staged_utxos_on_startup`.
//!   Outputs observable from the stable API:
//!     O1 the live UTXO set, read back from the INSTALLED backend (digest + row count)
//!     O2 `StateDb::staged_utxo_len()`
//!     O3 `StateDb::get_rebuild_in_progress()` / `Node::rebuild_halt_reason()`
//!     O4 the return value of `apply_snap_snapshot` (`Ok` / `Err`)
//!     O5 `Node::chain_state` / `Node::producer_set` — installed in the SAME batch
//!     O6 the cached state root (`Node::cached_state_root`)
//!     O7 `Node::serve_state_snapshot` — does a halted node refuse
//!   Paths:
//!     P1 staged promotion, manifest root MATCHES the installed backend   — E1 (O1..O6)
//!     P2 staged promotion, manifest root MISMATCHES                      — E2 (O1,O2,O3,O4)
//!     P3 a staging write error mid-download                              — E3 (O1, O2)
//!     P4 restart: staging rows present, NO markers                       — E4 (O2)
//!     P5 restart: staging rows present, markers SET                      — E5 (O2, O7)
//!     P6 the same chunks installed on BOTH backends                      — E6 (O1)
//!   MATRIX: P1xO1+O2+O3+O5+O6 -> E1 | P2xO1+O2+O3+O4 -> E2 | P3xO1+O2 -> E3
//!           P4xO2 -> E4 | P5xO2+O7 -> E5 | P6xO1 -> E6
//! INPUT PARTITIONS: (a) a node whose live set is NON-EMPTY before the install, so "live is
//!   unchanged" is a real observation and not the trivial empty case; (b) an incoming set
//!   that DIFFERS from the live one, so the promotion is detectable; (c) a mismatching
//!   manifest root that differs ONLY in the root, so the refusal is attributable.

use crypto::Hash;
use network::{StagedUtxoMarker, VerifiedSnapshot};
use storage::{Outpoint, UtxoEntry};

use crate::inc_i_156_m1_harness as h;
use crate::m3_common;

const LIVE_SET: usize = 1_500;
const INCOMING_SET: usize = 2_500;
const CHUNK_BYTES: usize = 64 * 1024;
const INCOMING_SEED: u64 = m3_common::FIXTURE_SEED ^ 0x4444;

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

/// The incoming set as the client carries it: canonical BODY runs, the manifest digest, the
/// entry count and the entries themselves for a membership walk.
#[allow(clippy::type_complexity)]
fn incoming_chunks() -> (Vec<Vec<u8>>, Hash, u64, Vec<(Outpoint, UtxoEntry)>) {
    let entries = m3_common::randomized_entries(INCOMING_SET, INCOMING_SEED);
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

/// Stage every body through the production sink, exactly as the wired session does.
fn stage_all(node: &doli_node::node::Node, bodies: &[Vec<u8>]) {
    let sink = doli_node::node::StateDbChunkSink::new(node.state_db.clone());
    for body in bodies.iter() {
        network::sync::UtxoChunkSink::stage(&sink, body).expect("staging a verified chunk");
    }
}

/// A `VerifiedSnapshot` whose UTXO rows are ALREADY in the staging family: no image, a
/// marker instead.
async fn staged_snapshot(
    node: &doli_node::node::Node,
    digest: Hash,
    count: u64,
    force_root: Option<Hash>,
) -> VerifiedSnapshot {
    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;
    let base = storage::StateSnapshot::create(&cs, &utxo, &ps).expect("fixture: base snapshot");

    let incoming_image = {
        let mut set = storage::utxo::UtxoSet::new();
        for (o, e) in m3_common::randomized_entries(INCOMING_SET, INCOMING_SEED) {
            set.insert(o, e).expect("fixture insert");
        }
        set.serialize_canonical()
    };
    let root = storage::compute_state_root_from_bytes(
        &base.chain_state_bytes,
        &incoming_image,
        &base.producer_set_bytes,
    )
    .expect("fixture: root over the incoming set");

    VerifiedSnapshot {
        block_hash: base.block_hash,
        block_height: base.block_height,
        chain_state: base.chain_state_bytes,
        utxo_set: Vec::new(),
        utxo_staged: Some(StagedUtxoMarker {
            utxo_hash: digest,
            utxo_count: count,
        }),
        producer_set: base.producer_set_bytes,
        state_root: force_root.unwrap_or(root),
        block_header_bytes: None,
        epoch_bond_snapshot_bytes: None,
        epoch_accumulators_bytes: None,
        epoch_state_bytes: Some(node.epoch_state.serialize()),
    }
}

// ==================== E1 — the staged promotion installs, atomically ====================

/// REQ-SCALE-014 / F-10 / F-11 — Decision: a failure here means `apply_snap_snapshot` still
/// needs a whole UTXO image to install one, so the streaming download bought nothing: the
/// O(set) allocation simply moved from the session client into the install.
/// INV-SYNC-014 — Decision: an install that does not leave the live handle on the
/// `state_db`-backed variant re-opens INC-I-118 — post-snap `apply_block` writes only to
/// state_db, so a frozen in-memory copy diverges at the first epoch boundary.
#[tokio::test(flavor = "multi_thread")]
async fn m4_staged_snapshot_installs_and_clears_markers_on_the_ok_arm() {
    let (mut node, _t, live_before) = node_with_live_set().await;
    let (bodies, digest, count, entries) = incoming_chunks();
    assert!(
        bodies.len() > 1,
        "the incoming fixture must span several chunks, else staging is never exercised"
    );
    stage_all(&node, &bodies);
    assert!(
        node.state_db.staged_utxo_len() > 0,
        "the fixture must stage something, else the promotion below is vacuous"
    );

    let snapshot = staged_snapshot(&node, digest, count, None).await;
    node.apply_snap_snapshot(snapshot)
        .await
        .expect("a root-verified staged install must succeed");

    let utxo = node.utxo_set.read().await;
    assert!(
        utxo.is_rocksdb(),
        "INV-SYNC-014: the installed set must be the state_db-backed variant"
    );
    assert_ne!(
        utxo.canonical_digest().expect("installed digest"),
        live_before,
        "the live set is unchanged — nothing was promoted, so everything below is vacuous"
    );
    assert_eq!(
        utxo.canonical_digest().expect("installed digest"),
        digest,
        "the digest read back FROM DISK must equal the manifest's utxo_hash"
    );
    assert_eq!(
        node.state_db.utxo_len(),
        INCOMING_SET,
        "the live row count after the install is {} of {} — a truncated promotion",
        node.state_db.utxo_len(),
        INCOMING_SET
    );
    let missing = entries
        .iter()
        .filter(|(outpoint, _)| node.state_db.get_utxo(outpoint).is_none())
        .count();
    assert_eq!(
        missing, 0,
        "{} of {} staged entries are absent from the live set after the install",
        missing, INCOMING_SET
    );

    // Res-2: the clear is a SEPARATE write AFTER the batch commits. Re-reading through a
    // fresh disk-backed handle is what separates "cleared afterwards" from "cleared as part
    // of the commit", which takes the promoted rows with it.
    assert_eq!(
        node.state_db.staged_utxo_len(),
        0,
        "staging still holds {} rows after the install — a whole UTXO set leaked to disk",
        node.state_db.staged_utxo_len()
    );
    let reread = storage::UtxoSet::from_state_db(node.state_db.clone());
    assert_eq!(
        reread.canonical_digest().expect("re-read digest"),
        digest,
        "the live digest changed between the promotion and the staging clear"
    );

    // F-11: the Ok arm disarms.
    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "a completed install must disarm the rebuild marker, or the node refuses to serve \
         state forever"
    );
    assert!(node.rebuild_halt_reason().is_none());

    // The 3-state installs together: chain_state and producer_set come from the SAME batch.
    let cs = node.chain_state.read().await;
    assert!(
        cs.is_snap_synced(),
        "the installed chain state must be stamped snap-synced, as the M1 path stamps it"
    );
    let ps = node.producer_set.read().await;
    let root = storage::compute_state_root(&cs, &utxo, &ps).expect("re-derived root");
    let cached = node
        .cached_state_root
        .read()
        .await
        .expect("the install must cache a root derived from the INSTALLED backend");
    assert_eq!(
        cached.0, root,
        "the cached root must be the one re-derived from the installed backend (F-10), not \
         the one that arrived on the wire"
    );
}

// ==================== E2 — a root mismatch REFUSES, loudly ====================

/// F-10 / F-11 — Decision: a failure here means a node that could not re-derive the promised
/// root carries on serving and producing anyway. `BlockHeader` carries no state root, so
/// nothing at block acceptance would ever catch it; the divergence surfaces at the next
/// epoch boundary as a fork, which is INC-I-054's shape.
#[tokio::test(flavor = "multi_thread")]
async fn m4_post_install_root_mismatch_refuses_and_leaves_markers_armed() {
    let (mut node, _t, _live) = node_with_live_set().await;
    let (bodies, digest, count, _entries) = incoming_chunks();
    stage_all(&node, &bodies);

    // Differs from the truth in the ROOT alone, so the refusal is attributable.
    let poisoned = Hash::from_bytes([0x5A; 32]);
    let snapshot = staged_snapshot(&node, digest, count, Some(poisoned)).await;

    let outcome = node.apply_snap_snapshot(snapshot).await;

    assert!(
        outcome.is_err(),
        "an install whose re-derived root does not match the manifest must return Err — a \
         silent Ok leaves the caller believing the node is installed and healthy"
    );
    assert!(
        node.state_db.get_rebuild_in_progress().is_some(),
        "a REFUSED install must leave the rebuild marker ARMED: the live set may have been \
         replaced by the promotion batch, and a disarmed marker says it was not"
    );
    let halted = node
        .rebuild_halt_reason()
        .expect("an armed marker must produce a halt reason");
    assert!(
        halted.contains("[STATE_CORRUPT]"),
        "the halt reason must carry the [STATE_CORRUPT] tag the serve paths refuse on, got \
         {:?}",
        halted
    );
    assert!(
        node.serve_state_snapshot(node.chain_state.read().await.best_hash)
            .await
            .type_name()
            == "Error",
        "a node that refused an install must refuse to serve state"
    );
}

// ============ E3 — a staging write error never touches the live CF ============

/// The value byte a canonical record starts with: `OutputType`. Flipping it to a value no
/// variant claims is the shortest reachable `stage_utxo_bytes` error that is NOT a
/// short-buffer (a short buffer is `Ok(None)` — the normal chunk-boundary case).
const OUTPUT_TYPE_BYTE: usize = 36;
const NO_SUCH_OUTPUT_TYPE: u8 = 0xFF;

/// INC-I-204 / INV-STORAGE-001 — Decision: a failure here means a RocksDB or decode error
/// mid-download is repaired, retried or partially applied against the LIVE column family.
/// The live write happens only inside the promotion batch, so the guard is that the error
/// arm returns BEFORE promotion — never that it rolls anything back. INC-I-204's lesson is
/// the shape: the error path drops the in-flight transfer and leaves committed state alone.
///
/// The error is INJECTED, not argued: `stage_utxo_bytes` propagates `StorageError` from
/// `canonical::parse_record`, the same `Result` a RocksDB write failure returns.
#[tokio::test(flavor = "multi_thread")]
async fn m4_staging_write_error_leaves_the_live_cf_untouched() {
    // ---- arm A: the failing call writes NOTHING ----
    {
        let (node, _t, live_before) = node_with_live_set().await;
        let live_len_before = node.state_db.utxo_len();
        let (bodies, _d, _c, _e) = incoming_chunks();

        let mut corrupt = bodies[0].clone();
        corrupt[OUTPUT_TYPE_BYTE] = NO_SUCH_OUTPUT_TYPE;

        let sink = doli_node::node::StateDbChunkSink::new(node.state_db.clone());
        let outcome = network::sync::UtxoChunkSink::stage(&sink, &corrupt);

        assert!(
            outcome.is_err(),
            "a body carrying an undecodable record must FAIL the stage call — silently \
             skipping it installs a different-but-valid-looking utxo_hash (AP-7)"
        );
        assert_eq!(
            network::sync::UtxoChunkSink::staged_len(&sink),
            0,
            "the failing call wrote {} rows before returning Err — a staging write must be \
             all-or-nothing, or the residual cursor and the rows disagree",
            network::sync::UtxoChunkSink::staged_len(&sink)
        );
        assert_eq!(
            node.state_db.utxo_len(),
            live_len_before,
            "the LIVE CF_UTXO row count changed on a FAILED staging write (was {}, now {})",
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
            "the LIVE canonical digest changed on a FAILED staging write"
        );
    }

    // ---- arm B: partial progress, then the session abandons ----
    {
        let (node, _t, live_before) = node_with_live_set().await;
        let live_len_before = node.state_db.utxo_len();
        let (bodies, _d, _c, _e) = incoming_chunks();

        let sink = doli_node::node::StateDbChunkSink::new(node.state_db.clone());
        network::sync::UtxoChunkSink::stage(&sink, &bodies[0]).expect("stage");
        network::sync::UtxoChunkSink::stage(&sink, &bodies[1]).expect("stage");
        assert!(
            network::sync::UtxoChunkSink::staged_len(&sink) > 0,
            "arm B fixture: something must be staged, else the clear below is vacuous"
        );

        network::sync::UtxoChunkSink::clear(&sink).expect("the abandon arm must clear staging");

        assert_eq!(
            network::sync::UtxoChunkSink::staged_len(&sink),
            0,
            "staging still holds {} rows after the session abandoned — they get concatenated \
             onto the next peer's chunks and match neither manifest",
            network::sync::UtxoChunkSink::staged_len(&sink)
        );
        assert_eq!(
            node.state_db.utxo_len(),
            live_len_before,
            "the LIVE CF_UTXO row count changed during an ABANDONED download"
        );
        assert_eq!(
            node.utxo_set
                .read()
                .await
                .canonical_digest()
                .expect("live digest"),
            live_before,
            "the LIVE canonical digest changed during an ABANDONED download"
        );
    }
}

// ==================== E4 / E5 — startup reconciliation, both arms ====================

/// INC-I-156 / INC-I-190 — Decision: a failure here means a node restarts onto staging rows
/// whose peer, cursor and manifest died with the process. Resuming that transfer produces a
/// digest matching NEITHER manifest, and promoting it installs a set nothing rejects. The
/// opposite failure is worse: dropping the rebuild marker along with the rows throws away the
/// only evidence that the live set is half-replaced.
#[tokio::test(flavor = "multi_thread")]
async fn m4_startup_clears_staging_on_both_arms_and_a_marked_node_stays_halted() {
    // ---- arm A: rows present, NO marker -> abandoned session, cleared ----
    {
        let (node, _t, _live) = node_with_live_set().await;
        let (bodies, _d, _c, _e) = incoming_chunks();
        stage_all(&node, &bodies);
        assert!(
            node.state_db.staged_utxo_len() > 0
                && node.state_db.get_rebuild_in_progress().is_none(),
            "arm A fixture: staged rows and a DISARMED marker, else the clear is vacuous"
        );

        let cleared = node
            .reconcile_staged_utxos_on_startup()
            .expect("startup reconciliation must not fail");

        assert!(
            cleared > 0,
            "startup reported {} rows cleared while staging was non-empty",
            cleared
        );
        assert_eq!(
            node.state_db.staged_utxo_len(),
            0,
            "staging survived startup: a session that can never be completed keeps a whole \
             UTXO set on disk and gets concatenated onto the next peer's chunks"
        );

        // The residual tail is invisible to `staged_utxo_len`, so clearing rows is not
        // evidence that `META_UTXO_STAGING_RESIDUAL` went with them. A surviving residual
        // would be prepended to the next session's FIRST body and shift every record by its
        // length. Staging one body into the reconciled family and comparing against a
        // pristine node is what distinguishes the two.
        node.state_db
            .stage_utxo_bytes(&bodies[0])
            .expect("a reconciled staging family must accept a fresh first body");
        let (pristine, _t2, _l2) = node_with_live_set().await;
        pristine
            .state_db
            .stage_utxo_bytes(&bodies[0])
            .expect("pristine stage");
        assert_eq!(
            node.state_db.staged_utxo_len(),
            pristine.state_db.staged_utxo_len(),
            "the reconciled family parsed {} rows from the same body a pristine family parsed \
             {} from — a residual tail survived startup",
            node.state_db.staged_utxo_len(),
            pristine.state_db.staged_utxo_len()
        );
    }

    // ---- arm B: rows present, marker SET -> the crash landed in the promotion window ----
    // Decision 136 — NO-RESUME: recovery is a fresh snap-sync, so the rows are cleared and
    // the marker alone keeps the node out of service.
    {
        let (node, _t, _live) = node_with_live_set().await;
        let (bodies, _d, _c, _e) = incoming_chunks();
        stage_all(&node, &bodies);
        let staged_before = node.state_db.staged_utxo_len();
        node.state_db
            .set_rebuild_in_progress(4_242)
            .expect("arm the marker as the promotion window does");

        let cleared = node
            .reconcile_staged_utxos_on_startup()
            .expect("startup reconciliation must not fail");

        assert_eq!(
            cleared, staged_before as u64,
            "startup cleared {} of the {} staged rows a MARKED restart found — no path can \
             ever complete them, so keeping them only costs disk and poisons the next stream",
            cleared, staged_before
        );
        assert_eq!(
            node.state_db.staged_utxo_len(),
            0,
            "staging survived a MARKED restart"
        );
        assert!(
            node.state_db.get_rebuild_in_progress().is_some(),
            "clearing staging also cleared the rebuild marker — the live set may be \
             half-replaced and the marker is the only record of it"
        );
        assert!(
            node.serve_state_snapshot(node.chain_state.read().await.best_hash)
                .await
                .type_name()
                == "Error",
            "a node that came back up inside a promotion window must refuse to serve state"
        );
    }
}

// ============ E7 — the four refusal arms of the STAGED install ============

/// Shorter than the 32-byte hash `ChainState` opens with, so the decode cannot
/// succeed: the shape a truncated or mis-framed transfer delivers.
const TRUNCATED_STATE_BYTES: usize = 5;

/// A node holding a non-empty live set AND a full staging family, with the marker
/// fields the install checks the staging against.
#[allow(clippy::type_complexity)]
async fn node_with_staged_transfer() -> (
    doli_node::node::Node,
    tempfile::TempDir,
    Hash,
    usize,
    Hash,
    u64,
) {
    let (node, temp, live_digest) = node_with_live_set().await;
    let (bodies, digest, count, _entries) = incoming_chunks();
    stage_all(&node, &bodies);
    let live_rows = node.state_db.utxo_len();
    assert!(
        node.state_db.staged_utxo_len() > 0 && live_rows > 0,
        "fixture: a FULL staging family and a NON-EMPTY live set, else every refusal \
         below is vacuous — an empty live set cannot be observed to survive"
    );
    (node, temp, live_digest, live_rows, digest, count)
}

/// What every refusal arm owes the operator. Asserting the outcome alone would pass
/// against a guard that refuses only AFTER the promotion has replaced the live set.
async fn assert_refusal_is_inert(
    node: &doli_node::node::Node,
    outcome: anyhow::Result<()>,
    live_digest_before: Hash,
    live_rows_before: usize,
    guard: &str,
) {
    assert!(
        outcome.is_ok(),
        "the {} guard must SOFT-refuse: an Err here is how a node that installed \
         nothing gets treated as half-installed, and the caller stops falling back to \
         header-first sync",
        guard
    );
    assert_eq!(
        node.utxo_set
            .read()
            .await
            .canonical_digest()
            .expect("live digest"),
        live_digest_before,
        "the {} guard refused yet the LIVE UTXO digest moved — the promotion ran before \
         the refusal, so the node now serves a set no manifest describes",
        guard
    );
    assert_eq!(
        node.state_db.utxo_len(),
        live_rows_before,
        "the {} guard refused yet the live row count moved from {} to {}",
        guard,
        live_rows_before,
        node.state_db.utxo_len()
    );
    assert!(
        node.state_db.get_rebuild_in_progress().is_none(),
        "the {} guard fires BEFORE the promotion window opens, so an armed marker halts \
         a node whose durable state was never touched — it stays out of service for a \
         transfer it correctly rejected",
        guard
    );
    assert!(
        node.rebuild_halt_reason().is_none(),
        "the {} guard left a halt reason on a node that installed nothing",
        guard
    );
    assert_eq!(
        node.state_db.staged_utxo_len(),
        0,
        "the {} guard refused but left {} rows in staging — they are concatenated onto \
         the next peer's chunks and match neither manifest",
        guard,
        node.state_db.staged_utxo_len()
    );
}

/// REQ-SCALE-013 — Decision: a failure here means the staged install trusts the
/// envelope's ChainState bytes, so a peer that streams well-formed rows behind a
/// corrupt header either panics the node or promotes under a 3-state whose chain
/// half never decoded.
#[tokio::test(flavor = "multi_thread")]
async fn m4_refuses_a_staged_install_whose_chain_state_will_not_decode() {
    let (mut node, _t, live_digest, live_rows, digest, count) = node_with_staged_transfer().await;

    let mut snapshot = staged_snapshot(&node, digest, count, None).await;
    assert!(
        snapshot.chain_state.len() > TRUNCATED_STATE_BYTES,
        "fixture: the ChainState bytes must be longer than the truncation, else the \
         'corruption' is the well-formed encoding"
    );
    snapshot.chain_state.truncate(TRUNCATED_STATE_BYTES);

    let outcome = node.apply_snap_snapshot(snapshot).await;
    assert_refusal_is_inert(&node, outcome, live_digest, live_rows, "ChainState decode").await;
}

/// REQ-SCALE-013 — Decision: a failure here means the ProducerSet half of the
/// snapshot can be garbage and still reach `promote_staged_utxos`, which writes it
/// into the same batch as the UTXO rows — the producer registry and the ledger would
/// be replaced from a message that never parsed.
#[tokio::test(flavor = "multi_thread")]
async fn m4_refuses_a_staged_install_whose_producer_set_will_not_decode() {
    let (mut node, _t, live_digest, live_rows, digest, count) = node_with_staged_transfer().await;

    let mut snapshot = staged_snapshot(&node, digest, count, None).await;
    assert!(
        snapshot.producer_set.len() > TRUNCATED_STATE_BYTES,
        "fixture: the ProducerSet bytes must be longer than the truncation"
    );
    snapshot.producer_set.truncate(TRUNCATED_STATE_BYTES);

    let outcome = node.apply_snap_snapshot(snapshot).await;
    assert_refusal_is_inert(&node, outcome, live_digest, live_rows, "ProducerSet decode").await;
}

/// REQ-SCALE-013 / C3 — Decision: a failure here means a peer can label a snapshot
/// with any (hash, height) it likes. The envelope is what seeds the canonical index
/// and the post-snap header sync, so an install under a lying envelope leaves the
/// node asking for headers from a tip its own state never had.
#[tokio::test(flavor = "multi_thread")]
async fn m4_refuses_a_staged_install_whose_envelope_contradicts_its_chain_state() {
    // ---- arm A: the envelope's block HASH is not the decoded tip ----
    {
        let (mut node, _t, live_digest, live_rows, digest, count) =
            node_with_staged_transfer().await;
        let mut snapshot = staged_snapshot(&node, digest, count, None).await;
        let honest = snapshot.block_hash;
        snapshot.block_hash = Hash::from_bytes([0xC3; 32]);
        assert_ne!(
            snapshot.block_hash, honest,
            "fixture: the envelope hash must actually differ from the decoded tip"
        );

        let outcome = node.apply_snap_snapshot(snapshot).await;
        assert_refusal_is_inert(&node, outcome, live_digest, live_rows, "C3 envelope hash").await;
    }

    // ---- arm B: the envelope's HEIGHT is not the decoded tip ----
    {
        let (mut node, _t, live_digest, live_rows, digest, count) =
            node_with_staged_transfer().await;
        let mut snapshot = staged_snapshot(&node, digest, count, None).await;
        snapshot.block_height = snapshot.block_height.wrapping_add(1);

        let outcome = node.apply_snap_snapshot(snapshot).await;
        assert_refusal_is_inert(&node, outcome, live_digest, live_rows, "C3 envelope height").await;
    }
}

/// REQ-SCALE-002 / REQ-SCALE-014 — Decision: a failure here means staging left by
/// some OTHER transfer is promoted under this manifest's root. `BlockHeader` carries
/// no state root, so nothing downstream rejects the result — the node runs on a
/// ledger that is a blend of two peers' sets until the next epoch boundary forks it.
#[tokio::test(flavor = "multi_thread")]
async fn m4_refuses_a_staged_install_when_staging_is_not_the_manifests_row_count() {
    let (mut node, _t, live_digest, live_rows, digest, count) = node_with_staged_transfer().await;
    assert_eq!(
        node.state_db.staged_utxo_len() as u64,
        count,
        "fixture: staging must hold exactly the honest count, so the +1 below is the \
         ONLY thing the guard can be reacting to"
    );

    let snapshot = staged_snapshot(&node, digest, count + 1, None).await;

    let outcome = node.apply_snap_snapshot(snapshot).await;
    assert_refusal_is_inert(&node, outcome, live_digest, live_rows, "staged row count").await;
}

/// REQ-SCALE-006 / INV-SYNC-007 — Decision: a failure here means the call site treats
/// a REFUSED install as fatal or as success. Either way the snap pipeline is never
/// handed back: a node that correctly rejected one peer's snapshot stops retrying with
/// another and never reaches header-first sync.
#[tokio::test(flavor = "multi_thread")]
async fn m4_a_refused_staged_install_hands_the_pipeline_back_to_normal_sync() {
    let (mut node, _t, _live_digest, _live_rows, digest, count) = node_with_staged_transfer().await;

    const PEER_TIP: u64 = 100;
    let peer_hash = Hash::from_bytes([0x11; 32]);
    {
        let mut sync = node.sync_manager.write().await;
        sync.add_peer(network::PeerId::random(), PEER_TIP, peer_hash, 0);
        sync.block_applied(peer_hash, PEER_TIP, 0);
        assert_eq!(
            sync.sync_state_name(),
            "Synchronized",
            "fixture: the manager must start OUT of Idle, else 'Idle afterwards' is the \
             state it was already in and proves nothing"
        );
    }

    let mut snapshot = staged_snapshot(&node, digest, count, None).await;
    snapshot.chain_state.truncate(TRUNCATED_STATE_BYTES);

    node.apply_snap_snapshot(snapshot).await.expect(
        "a refused install must reach the caller as Ok — the legacy arms all \
                 soft-refuse, and an Err here halts a node that installed nothing",
    );

    assert_eq!(
        node.sync_manager.read().await.sync_state_name(),
        "Idle",
        "the refusal never reached snap_fallback_to_normal: the manager still reports \
         Synchronized, so the snap attempt counter never advanced and the node keeps \
         waiting on a pipeline that was abandoned"
    );
}
