//! UTXO-scalability M4 round 2 — the node half of the chunked staged install.
//!
//! Decision 136: the promotion writes in bounded sub-batches, a crash between them halts the
//! node, and the restart contract is NO-RESUME — staging is cleared and recovery is a fresh
//! snap-sync. `m4_chunked_promotion.rs` (storage) owns the partial-live-family assertions;
//! this file owns what the NODE does with a promotion that failed.
//!
//! REQ-SCALE-013 — Decision: a failure reveals a failed promotion does not propagate out of
//! `install_staged_utxos`, so the sync pipeline treats a half-replaced UTXO family as a
//! refusal and falls back to normal sync on a corrupt state — the one Fork 1 branch M4 could
//! not reach without a fault seam.
//! REQ-SCALE-006 — Decision: a failure reveals a node that crashed inside a promotion window
//! either serves state it cannot vouch for, or keeps a whole abandoned UTXO set on disk that
//! the next session's chunks are concatenated onto.
//! REQ-SCALE-002 — Decision: a failure reveals the chunked write path replaced the
//! `atomic_replace` call sites rollback and the legacy decode install still depend on.
//!
//! OUTPUT CONTRACT: `Node::apply_snap_snapshot(VerifiedSnapshot) -> Result<()>` on a STAGED
//!   marker whose promotion aborts, and `Node::reconcile_staged_utxos_on_startup() -> Result<u64>`
//!   over the database that abort left behind.
//!   Outputs observable from the stable API:
//!     O1 the return value of `apply_snap_snapshot` (`Ok` / `Err`)
//!     O2 `StateDb::get_rebuild_in_progress()` / `Node::rebuild_halt_reason()`
//!     O3 `Node::serve_state_snapshot` — does the halted node refuse
//!     O4 `StateDb::staged_utxo_len()`
//!     O5 the live UTXO set digest, read back from the INSTALLED backend
//!     O6 `Node::chain_state` — the in-memory tip
//!     O7 the return value of `reconcile_staged_utxos_on_startup` (rows cleared)
//!   Paths:
//!     P1 staged promotion aborted mid-flight             — C1 (O1, O2, O3, O4, O6)
//!     P2 restart over the database P1 left behind        — C2 (O2, O3, O4, O5, O7)
//!     P3 the `atomic_replace` call sites, as source      — C3
//!   MATRIX: P1xO1 + P1xO2 + P1xO3 + P1xO4 + P1xO6 -> C1
//!           P2xO2 + P2xO3 + P2xO4 + P2xO5 + P2xO7 -> C2 | P3 -> C3
//! INPUT PARTITIONS: (a) a snapshot whose UTXO rows are ALREADY staged and whose root WOULD
//!   match, so the only reason the install fails is the injected abort and not a refusal arm
//!   that already had coverage; (b) a restart driven over the exact database the abort left,
//!   not a synthetic marker written by hand.

use std::path::Path;

use crypto::Hash;
use network::{StagedUtxoMarker, VerifiedSnapshot};

use crate::inc_i_156_m1_harness as h;
use crate::m3_common;

const LIVE_SET: usize = 1_200;
const INCOMING_SET: usize = 2_500;
const CHUNK_BYTES: usize = 64 * 1024;
const INCOMING_SEED: u64 = m3_common::FIXTURE_SEED ^ 0x5555;

/// Abort after the first committed sub-batch — a crash inside the promotion window.
const ABORT_AFTER_BATCHES: u64 = 1;

/// A node with a live set, the incoming set fully staged, and a `VerifiedSnapshot` whose
/// root matches what the promotion WOULD install.
async fn node_ready_to_promote() -> (
    doli_node::node::Node,
    tempfile::TempDir,
    VerifiedSnapshot,
    Hash,
) {
    let (node, _kp, temp) = h::make_node(1).await;
    h::install_production_utxo_backend(&node).await;
    for (outpoint, entry) in m3_common::randomized_entries(LIVE_SET, m3_common::FIXTURE_SEED) {
        node.state_db
            .insert_utxo(&outpoint, &entry)
            .expect("fixture: insert_utxo");
    }

    let incoming = m3_common::randomized_entries(INCOMING_SET, INCOMING_SEED);
    let mut set = storage::utxo::UtxoSet::new();
    for (o, e) in incoming.iter() {
        set.insert(*o, e.clone()).expect("fixture insert");
    }
    let image = set.serialize_canonical();
    let incoming_digest = set.canonical_digest().expect("incoming digest");
    let count = set.utxo_count();

    let sink = doli_node::node::StateDbChunkSink::new(node.state_db.clone());
    for body in image[8..].chunks(CHUNK_BYTES) {
        network::sync::UtxoChunkSink::stage(&sink, body).expect("staging a verified chunk");
    }
    assert_eq!(
        node.state_db.staged_utxo_len() as u64,
        count,
        "fixture: the transfer must be complete, or the install refuses before it promotes"
    );

    let snapshot = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let base = storage::StateSnapshot::create(&cs, &utxo, &ps).expect("fixture: base snapshot");
        let root = storage::compute_state_root_from_bytes(
            &base.chain_state_bytes,
            &image,
            &base.producer_set_bytes,
        )
        .expect("fixture: root over the incoming set");

        VerifiedSnapshot {
            block_hash: base.block_hash,
            block_height: base.block_height,
            chain_state: base.chain_state_bytes,
            utxo_set: Vec::new(),
            utxo_staged: Some(StagedUtxoMarker {
                utxo_hash: incoming_digest,
                utxo_count: count,
            }),
            producer_set: base.producer_set_bytes,
            state_root: root,
            block_header_bytes: None,
            epoch_bond_snapshot_bytes: None,
            epoch_accumulators_bytes: None,
            epoch_state_bytes: Some(node.epoch_state.serialize()),
        }
    };

    (node, temp, snapshot, incoming_digest)
}

fn live_digest(node: &doli_node::node::Node) -> Hash {
    storage::utxo::UtxoSet::from_state_db(node.state_db.clone())
        .canonical_digest()
        .expect("reading the live canonical digest must succeed")
}

// ============ C1 — a failed promotion propagates, with the node halted ============

/// REQ-SCALE-013 — Decision: a failure reveals a promotion that died between sub-batches is
/// reported to the sync pipeline as a refusal instead of an error, so the node falls back to
/// normal sync over a half-replaced UTXO family.
#[tokio::test(flavor = "multi_thread")]
async fn m4_a_failed_promotion_propagates_as_err_with_the_node_halted() {
    let (mut node, _t, snapshot, _incoming) = node_ready_to_promote().await;
    let tip_before = node.chain_state.read().await.best_height;

    node.state_db
        .fault_inject_promote_abort_after(Some(ABORT_AFTER_BATCHES));
    let outcome = node.apply_snap_snapshot(snapshot).await;
    node.state_db.fault_inject_promote_abort_after(None);

    assert!(
        outcome.is_err(),
        "a promotion that aborted mid-flight returned Ok — the pipeline hands the node back \
         to normal sync over a UTXO family it cannot vouch for"
    );
    assert!(
        node.state_db.get_rebuild_in_progress().is_some(),
        "the rebuild marker was cleared by a promotion that did not finish"
    );
    assert!(
        node.rebuild_halt_reason().is_some(),
        "the node reports no halt reason after a failed promotion, so every serve path stays \
         open over a half-replaced set"
    );
    assert_eq!(
        node.serve_state_snapshot(node.chain_state.read().await.best_hash)
            .await
            .type_name(),
        "Error",
        "a node whose promotion failed still served state to a peer"
    );
    assert_eq!(
        node.chain_state.read().await.best_height,
        tip_before,
        "the in-memory tip advanced to the snapshot height on a promotion that failed"
    );
    assert!(
        node.state_db.staged_utxo_len() > 0,
        "staging was cleared after a FAILED staged install — the quarantine clear belongs to \
         the refusal arm, not to the halt arm"
    );
}

// ============ C2 — restart is NO-RESUME ============

/// REQ-SCALE-006 — Decision: a failure reveals the restart contract drifted from no-resume:
/// either the node resumes a half-promotion nothing else exercises, or it keeps a whole
/// abandoned UTXO set on disk for the next session's chunks to be concatenated onto.
#[tokio::test(flavor = "multi_thread")]
async fn m4_restart_after_a_mid_promotion_abort_clears_staging_and_stays_halted() {
    let (mut node, _t, snapshot, incoming_digest) = node_ready_to_promote().await;

    node.state_db
        .fault_inject_promote_abort_after(Some(ABORT_AFTER_BATCHES));
    let outcome = node.apply_snap_snapshot(snapshot).await;
    node.state_db.fault_inject_promote_abort_after(None);
    assert!(
        outcome.is_err(),
        "fixture: the promotion must have failed, or the restart contract is untested"
    );

    let staged_before = node.state_db.staged_utxo_len();
    let digest_after_abort = live_digest(&node);
    assert!(
        staged_before > 0,
        "fixture: staging must still hold rows for the clear to be observable"
    );

    let cleared = node
        .reconcile_staged_utxos_on_startup()
        .expect("startup reconciliation must not fail");

    assert_eq!(
        cleared, staged_before as u64,
        "startup cleared {} of the {} rows an aborted promotion left — anything staging keeps \
         is prepended to the next peer's stream",
        cleared, staged_before
    );
    assert_eq!(
        node.state_db.staged_utxo_len(),
        0,
        "staging survived the restart: recovery is a FRESH snap-sync, so these rows can never \
         be completed and only cost disk"
    );
    assert!(
        node.state_db.get_rebuild_in_progress().is_some(),
        "startup cleared the rebuild marker — the live UTXO family is still half-replaced and \
         the marker is the only thing keeping the node out of service"
    );
    assert!(
        node.rebuild_halt_reason().is_some(),
        "a node that came back up inside a promotion window reports no halt reason"
    );
    assert_eq!(
        node.serve_state_snapshot(node.chain_state.read().await.best_hash)
            .await
            .type_name(),
        "Error",
        "a node that came back up inside a promotion window served state to a peer"
    );
    assert_eq!(
        live_digest(&node),
        digest_after_abort,
        "the live UTXO family changed during startup — something re-promoted the staged rows, \
         and a resume path is exactly what the no-resume contract forbids"
    );
    assert_ne!(
        live_digest(&node),
        incoming_digest,
        "startup completed the abandoned promotion — the live set now equals the incoming set \
         without ever passing the post-install root check"
    );
}

// ============ C3 — the atomic_replace call sites ============

fn source(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {} must succeed: {}", path.display(), e))
}

/// REQ-SCALE-002 — Decision: a failure reveals the chunked write path was introduced by
/// changing `atomic_replace` itself rather than beside it, which silently converts rollback
/// and the legacy decode install — neither of which arms a rebuild marker — from
/// all-or-nothing to resumable-from-nothing.
#[test]
fn m4_the_chunked_write_path_is_staged_install_only() {
    let staging = source("../../crates/storage/src/state_db/staging.rs");
    let code: String = staging
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("atomic_replace"),
        "`promote_staged_utxos` still delegates to `atomic_replace` — the single in-memory \
         WriteBatch is the O(set) install peak, so nothing about M4's ratio can have changed"
    );

    let writes = source("../../crates/storage/src/state_db/writes.rs");
    let body = writes
        .split_once("pub fn atomic_replace")
        .expect("`atomic_replace` must still exist in writes.rs")
        .1;
    let body = body
        .split_once("\n    pub fn ")
        .map(|(b, _)| b)
        .unwrap_or(body);
    assert_eq!(
        body.matches("WriteBatch::default()").count(),
        1,
        "`atomic_replace` builds {} WriteBatches — its whole contract is that a crash leaves \
         either the old state or the new one, never a prefix of the new one",
        body.matches("WriteBatch::default()").count()
    );

    for (file, what) in [
        ("src/node/rollback.rs", "rebuild-from-genesis rollback"),
        ("src/node/snapshot_install.rs", "the legacy decode install"),
    ] {
        assert!(
            source(file).contains(".atomic_replace("),
            "{} no longer calls `atomic_replace` — it was moved onto the staged-install \
             chunked path, which has no all-or-nothing guarantee",
            what
        );
    }
}
