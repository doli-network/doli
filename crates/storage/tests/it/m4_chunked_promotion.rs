//! UTXO-scalability M4 round 2 — the CHUNKED staged-install promotion.
//!
//! Decision 136: `promote_staged_utxos` gets its own bounded sub-batch write path;
//! `StateDb::atomic_replace` stays UNCHANGED and keeps single-batch all-or-nothing for
//! rollback and reorg replay.
//!
//! REQ-SCALE-014 — Decision: a failure reveals the promotion still commits the whole set in
//! one in-memory `WriteBatch`, so install RAM keeps tracking the UTXO set and an operator's
//! join cost keeps growing with the chain.
//! REQ-SCALE-013 — Decision: a failure reveals a crash between sub-batches can leave a
//! half-replaced live UTXO family that the node cannot tell apart from a finished install.
//! REQ-SCALE-002 — Decision: a failure reveals the new staged-install write path leaked into
//! `atomic_replace`, so rollback and reorg replay lost the all-or-nothing guarantee they are
//! the only remaining users of.
//! INV-SYNC-007 — Decision: a failure reveals a set installed across several sub-batches does
//! not hash to the same canonical digest as the same set installed any other way, which is a
//! chain split on every chunked snap-sync.
//!
//! OUTPUT CONTRACT:
//!   `StateDb::promote_staged_utxos(&self, &ChainState, &ProducerSet) -> Result<(), StorageError>`
//!   `StateDb::atomic_replace(&self, &ChainState, &ProducerSet, impl Iterator) -> Result<(), StorageError>`
//!   Outputs observable from the stable API:
//!     O1 the live UTXO family, read back as `UtxoSet::canonical_digest()`
//!     O2 the live UTXO family row count (`StateDb::iter_utxos().len()`)
//!     O3 `StateDb::staged_utxo_len()`
//!     O4 `StateDb::get_rebuild_in_progress()`
//!     O5 `StateDb::get_chain_state()` — the tip label the install writes
//!     O6 the `Result` of the call under test
//!   Paths:
//!     P1 promotion aborted between sub-batches            — B1 (O1..O6)
//!     P2 promotion runs to completion across sub-batches  — B2 (O1, O2, O3, O5, O6)
//!     P3 `atomic_replace` with the promote fault ARMED    — B3 (O1, O2, O6)
//!     P4 `atomic_replace` whose row stream dies mid-way   — B4 (O1, O2)
//!   MATRIX: P1xO1 + P1xO2 + P1xO3 + P1xO4 + P1xO5 + P1xO6 -> B1
//!           P2xO1 + P2xO2 + P2xO3 + P2xO5 + P2xO6 -> B2
//!           P3xO1 + P3xO2 + P3xO6 -> B3 | P4xO1 + P4xO2 -> B4
//! INPUT PARTITIONS: (a) a staged set sized off `PROMOTE_BATCH_MAX_BYTES` so the promotion
//!   cannot fit in fewer than `MIN_SUB_BATCHES` sub-batches — a single-sub-batch fixture
//!   would make every "between sub-batches" assertion vacuous; (b) an abort after the FIRST
//!   committed sub-batch, the earliest point at which the live family is already damaged;
//!   (c) a live set that differs from the staged set, so "unchanged" and "replaced" are
//!   distinguishable outcomes.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use crypto::Hash;
use storage::chain_state::ChainState;
use storage::producer::ProducerSet;
use storage::state_db::PROMOTE_BATCH_MAX_BYTES;
use storage::utxo::UtxoSet;
use storage::{Outpoint, StateDb, UtxoEntry};
use tempfile::TempDir;

use crate::common;

const LIVE_N: usize = 1_200;
const LIVE_SEED: u64 = 0x004D_3400_4C49_5645;
const STAGED_SEED: u64 = 0x004D_3400_5354_4744;
const SAMPLE_N: usize = 256;

/// Sub-batches the fixture must span before "between sub-batches" means anything.
const MIN_SUB_BATCHES: usize = 4;

/// A budget above this makes a multi-sub-batch fixture unaffordable in a test, and makes the
/// 100k-entry peak probe single-batch — the O(set) term `m4_peak_ratio_red` measures.
const MAX_FIXTURE_ENTRIES: usize = 200_000;

/// Abort after the FIRST committed sub-batch: the live family is already partly replaced and
/// the remaining sub-batches never run.
const ABORT_AFTER_BATCHES: u64 = 1;

const OLD_TIP_HEIGHT: u64 = 4_242;
const NEW_TIP_HEIGHT: u64 = 9_999;

fn genesis() -> Hash {
    Hash::from_bytes([0x11; 32])
}

fn tip(height: u64, tag: u8) -> ChainState {
    let mut cs = ChainState::new(genesis());
    cs.best_height = height;
    cs.best_hash = Hash::from_bytes([tag; 32]);
    cs
}

fn digest_of(db: &Arc<StateDb>) -> Hash {
    UtxoSet::from_state_db(db.clone())
        .canonical_digest()
        .expect("reading the live canonical digest must succeed")
}

fn digest_of_entries(entries: &[(Outpoint, UtxoEntry)]) -> Hash {
    common::build_in_memory(entries)
        .canonical_digest()
        .expect("reference digest must compute")
}

fn canonical_body(entries: &[(Outpoint, UtxoEntry)]) -> Vec<u8> {
    common::build_in_memory(entries).serialize_canonical()[8..].to_vec()
}

/// A staged set that cannot fit in fewer than `MIN_SUB_BATCHES` sub-batches, sized from the
/// developer's budget rather than from a guess about it.
fn multi_sub_batch_entries() -> Vec<(Outpoint, UtxoEntry)> {
    let per_entry =
        canonical_body(&common::randomized_entries(SAMPLE_N, STAGED_SEED)).len() / SAMPLE_N;
    let n = (MIN_SUB_BATCHES * PROMOTE_BATCH_MAX_BYTES).div_ceil(per_entry) + 1;
    assert!(
        n <= MAX_FIXTURE_ENTRIES,
        "PROMOTE_BATCH_MAX_BYTES = {} needs {} entries to span {} sub-batches — a budget that \
         large leaves the 100k-entry install single-batch, which is the O(set) peak M4 exists \
         to remove",
        PROMOTE_BATCH_MAX_BYTES,
        n,
        MIN_SUB_BATCHES
    );
    common::randomized_entries(n, STAGED_SEED)
}

/// Feed `entries` into the staging family exactly as a verified chunk stream would.
fn stage_entries(db: &Arc<StateDb>, entries: &[(Outpoint, UtxoEntry)]) {
    const CHUNK: usize = 64 * 1024;
    for body in canonical_body(entries).chunks(CHUNK) {
        db.stage_utxo_bytes(body)
            .expect("staging a verified chunk body must succeed");
    }
    assert_eq!(
        db.staged_utxo_len(),
        entries.len(),
        "fixture: staging holds {} of {} rows, so the promotion under test would be over a \
         different set than the one asserted against",
        db.staged_utxo_len(),
        entries.len()
    );
}

/// A node mid snap-sync: a live set, an old tip label, an armed rebuild marker and a full
/// staging family.
struct MidPromotion {
    _dir: TempDir,
    db: Arc<StateDb>,
    live: Vec<(Outpoint, UtxoEntry)>,
    staged: Vec<(Outpoint, UtxoEntry)>,
}

fn node_mid_promotion() -> MidPromotion {
    let dir = TempDir::new().expect("fixture: tempdir");
    let live = common::randomized_entries(LIVE_N, LIVE_SEED);
    let db = common::open_state_db_with(dir.path(), &live);
    db.put_chain_state(&tip(OLD_TIP_HEIGHT, 0xAA))
        .expect("fixture: old tip label");

    let staged = multi_sub_batch_entries();
    stage_entries(&db, &staged);
    db.set_rebuild_in_progress(NEW_TIP_HEIGHT)
        .expect("fixture: arm the marker as the install does");

    MidPromotion {
        _dir: dir,
        db,
        live,
        staged,
    }
}

// ============ B1 — a crash between sub-batches halts, it does not half-succeed ============

/// REQ-SCALE-013 / REQ-SCALE-014 — Decision: a failure reveals a crash between sub-batches
/// can leave a partly replaced live UTXO family that no marker records, so the node comes
/// back up serving a set that is neither the old one nor the new one.
#[test]
fn m4_promote_aborted_between_sub_batches_halts_with_a_partial_live_family() {
    let MidPromotion {
        _dir,
        db,
        live,
        staged,
    } = node_mid_promotion();
    let old_digest = digest_of(&db);

    db.fault_inject_promote_abort_after(Some(ABORT_AFTER_BATCHES));
    let outcome = db.promote_staged_utxos(&tip(NEW_TIP_HEIGHT, 0xBB), &ProducerSet::new());
    db.fault_inject_promote_abort_after(None);

    assert!(
        outcome.is_err(),
        "an abort between sub-batches returned Ok — the caller would clear the marker and \
         start serving a half-replaced UTXO set"
    );
    assert!(
        db.get_rebuild_in_progress().is_some(),
        "the rebuild marker was cleared by a promotion that did not finish — the marker is \
         the only evidence the live set may be half-replaced"
    );

    let live_rows = db.iter_utxos().len();
    assert!(
        live_rows < staged.len(),
        "the live family holds all {} staged rows after an abort at sub-batch {} — the \
         promotion committed the whole set in one write, so there is no sub-batch boundary \
         to crash between",
        staged.len(),
        ABORT_AFTER_BATCHES
    );
    assert_ne!(
        live_rows,
        live.len(),
        "the live family still holds exactly the {} pre-install rows — Phase 1 never ran, so \
         this fixture is not exercising the promotion it claims to",
        live.len()
    );

    let after = digest_of(&db);
    assert_ne!(
        after, old_digest,
        "the live digest is unchanged, so nothing was written and the partial-state contract \
         is untested"
    );
    assert_ne!(
        after,
        digest_of_entries(&staged),
        "the live digest equals the staged set's — the abort did not land between sub-batches"
    );

    assert_eq!(
        db.get_chain_state()
            .expect("the tip label must survive a failed promotion")
            .best_height,
        OLD_TIP_HEIGHT,
        "a failed promotion labelled the database with the INCOMING tip — a partial UTXO \
         family under a finished-looking tip is state corruption nothing downstream rejects"
    );
    assert!(
        db.staged_utxo_len() > 0,
        "staging was cleared by a promotion that failed — the rows are the only record of \
         what the half-written live family was supposed to become"
    );
}

// ============ B2 — the completed chunked promotion is byte-identical ============

/// INV-SYNC-007 / REQ-SCALE-002 — Decision: a failure reveals a set installed across several
/// sub-batches hashes differently from the same set installed by direct insertion, which is
/// a chain split on every chunked snap-sync.
#[test]
fn m4_completed_chunked_promotion_digest_equals_the_reference() {
    let MidPromotion {
        _dir,
        db,
        live: _live,
        staged,
    } = node_mid_promotion();

    assert!(
        canonical_body(&staged).len() > MIN_SUB_BATCHES * PROMOTE_BATCH_MAX_BYTES / 2,
        "fixture: the staged image is too small to span sub-batches, so this asserts nothing \
         about the chunked path"
    );

    db.promote_staged_utxos(&tip(NEW_TIP_HEIGHT, 0xBB), &ProducerSet::new())
        .expect("a complete chunked promotion must succeed");

    assert_eq!(
        digest_of(&db),
        digest_of_entries(&staged),
        "the chunked promotion installed a set whose canonical digest differs from the same \
         set built by direct insertion — the sub-batch split changed the consensus image"
    );
    assert_eq!(
        db.iter_utxos().len(),
        staged.len(),
        "the chunked promotion installed {} of {} rows",
        db.iter_utxos().len(),
        staged.len()
    );
    assert_eq!(
        db.get_chain_state()
            .expect("the install must write the new tip")
            .best_height,
        NEW_TIP_HEIGHT,
        "the UTXO rows landed but the tip label did not — the install is not complete"
    );
    assert_eq!(
        db.staged_utxo_len(),
        0,
        "staging survived a completed install and would be concatenated onto the next \
         session's chunks"
    );
}

// ============ B3 + B4 — atomic_replace is untouched ============

/// REQ-SCALE-002 — Decision: a failure reveals the staged-install fault seam or the chunked
/// write path reached `atomic_replace`, so rollback and reorg replay — which have no rebuild
/// marker to fall back on — lost the all-or-nothing guarantee that makes them safe.
#[test]
fn m4_atomic_replace_stays_single_batch_all_or_nothing_for_reorg_replay() {
    let replacement = common::randomized_entries(LIVE_N * 2, STAGED_SEED);

    // ---- B3: the staged-install fault seam must not reach the reorg path ----
    {
        let dir = TempDir::new().expect("fixture: tempdir");
        let live = common::randomized_entries(LIVE_N, LIVE_SEED);
        let db = common::open_state_db_with(dir.path(), &live);

        db.fault_inject_promote_abort_after(Some(ABORT_AFTER_BATCHES));
        let outcome = db.atomic_replace(
            &tip(NEW_TIP_HEIGHT, 0xCC),
            &ProducerSet::new(),
            replacement.clone().into_iter(),
        );
        db.fault_inject_promote_abort_after(None);

        assert!(
            outcome.is_ok(),
            "an armed staged-install fault aborted a reorg replay — the seam is not \
             staged-install-only and every rollback now inherits a snap-sync test hook"
        );
        assert_eq!(
            digest_of(&db),
            digest_of_entries(&replacement),
            "the reorg replay installed a different set than it was given"
        );
        assert_eq!(
            db.iter_utxos().len(),
            replacement.len(),
            "the reorg replay landed {} of {} rows",
            db.iter_utxos().len(),
            replacement.len()
        );
    }

    // ---- B4: a death inside the row stream leaves the OLD state fully intact ----
    {
        let dir = TempDir::new().expect("fixture: tempdir");
        let live = common::randomized_entries(LIVE_N, LIVE_SEED);
        let db = common::open_state_db_with(dir.path(), &live);
        db.put_chain_state(&tip(OLD_TIP_HEIGHT, 0xAA))
            .expect("fixture: old tip label");
        let old_digest = digest_of(&db);
        let halfway = replacement.len() / 2;

        let died = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let stream = replacement
                .clone()
                .into_iter()
                .enumerate()
                .map(|(i, pair)| {
                    assert!(i < halfway, "fixture: deliberate death mid-stream");
                    pair
                });
            db.atomic_replace(&tip(NEW_TIP_HEIGHT, 0xCC), &ProducerSet::new(), stream)
        }));

        assert!(
            died.is_err(),
            "fixture: the row stream did not die, so all-or-nothing is untested"
        );
        assert_eq!(
            digest_of(&db),
            old_digest,
            "a reorg replay that died mid-stream left a changed live UTXO family — \
             `atomic_replace` no longer holds its writes until the stream is complete"
        );
        assert_eq!(
            db.iter_utxos().len(),
            live.len(),
            "a reorg replay that died mid-stream left {} rows instead of the {} it started \
             with",
            db.iter_utxos().len(),
            live.len()
        );
        assert_eq!(
            db.get_chain_state()
                .expect("the tip label must survive")
                .best_height,
            OLD_TIP_HEIGHT,
            "a reorg replay that died mid-stream still relabelled the tip"
        );
    }
}
