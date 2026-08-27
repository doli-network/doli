//! INC-I-174 — undo a maintainer rotation that a rewind dropped.
//!
//! ## Why this module exists at all
//!
//! `AddMaintainer` / `RemoveMaintainer` mutate the node-local maintainer set —
//! the auto-updater's release-verification trust root — and the mutation is applied
//! IMMEDIATELY in the per-transaction loop (`apply_block/governance.rs`), never
//! epoch-deferred. Until INC-I-174 nothing undid it: `UndoData` carried no maintainer
//! field and neither `rollback_one_block` nor `execute_reorg` mentioned the maintainer
//! set. A reorg that dropped the rotation therefore left this host trusting a key set
//! the canonical chain has no record of — in memory AND on disk, with no self-heal,
//! because the one-shot seed in `periodic.rs` never re-fires once `members` is
//! non-empty.
//!
//! ## Why the logic lives HERE and not in the two callers
//!
//! `rollback_one_block` (`rollback.rs`) and `execute_reorg` (`block_handling.rs`) are
//! two INDEPENDENT rewind loops. INC-I-040 is the precedent, verbatim: a state class
//! was restored in `rollback_one_block` and missed in `execute_reorg`, and the fork
//! persisted. Analysis §7.3 names that drift the highest single regression risk of this
//! fix. Both callers therefore call the SAME two functions here; there is no second
//! implementation to drift from.
//!
//! ## Two phases, and why they are separated
//!
//! [`Node::plan_maintainer_rewind`] is PURE READS and must run EARLY — before either
//! caller purges the height index for the rewound range
//! (`remove_canonical_entry`), because deciding whether a height mutated the
//! maintainer set requires reading that height's block.
//!
//! [`Node::commit_maintainer_rewind`] mutates and must run LATE — after the caller's
//! trailing `atomic_replace` has durably committed the chain rewind. AUDIT-P1-201 (open,
//! P1) records that an aborted undo-rollback already abandons a durable half-applied
//! UTXO undo; placing the trust-root write inside that window would add one more durable
//! side effect to a non-atomic sequence and WIDEN it. After `atomic_replace` the chain
//! rewind is durable, so an abort before that point simply leaves the trust root
//! untouched — which is the correct conservative outcome: the chain did not rewind, so
//! neither should the root.
//!
//! ## Counter semantics
//!
//! The two `Node` counters this module owns are declared in `node/mod.rs` (they are `Node`
//! state, read by tests and by operators) but documented HERE, next to the only code that
//! writes them:
//!
//! * `Node::maintainer_rewind_count` — REQ-174-010. Rewinds (`rollback_one_block` or
//!   `execute_reorg`) that RESTORED the trust root from a `cf_undo` snapshot. Monotonic for
//!   the process lifetime and never reset, because it is a rate DENOMINATOR for the counter
//!   below, not a recovery-progress signal.
//!
//!   **REQ-174-010 metrics surface: PARTIALLY MET, and recorded as such.** The AC says
//!   "exposed on the metrics endpoint"; both counters are plain `Node` fields with NO RPC
//!   method and NO Prometheus gauge. REQ-174-010 is a *Could*, and the sibling
//!   `shallow_rollback_count` has exactly the same status, so this matches existing
//!   practice rather than regressing it — but it is never to be cited as MET. The
//!   operator-facing signal today is the `MAINTAINER_REWIND_RESTORED` /
//!   [`MAINTAINER_REWIND_UNRESTORED_ANCHOR`] log pair; the counters are asserted by test.
//! * `Node::maintainer_rewind_unrestored_count` — REQ-174-005 / REQ-174-010. Rewinds that
//!   crossed a height whose maintainer state could NOT be restored: no snapshot for a block
//!   that carried a rotation, an unreadable block, a snapshot `validate_persisted_set`
//!   refused, a snapshot whose installation would re-arm the one-shot seed, or a persist
//!   that failed. This is the MACHINE-CHECKABLE half of "no silent route exists"
//!   (REQ-174-005 AC-3). Any non-zero value means this host's release-verification trust
//!   root may no longer track the canonical chain; the human half is the
//!   [`MAINTAINER_REWIND_UNRESTORED_ANCHOR`] grep line emitted at the same moment, whose
//!   `reason=` sub-token says WHICH of those routes was taken.

/// The mutating half — `commit_maintainer_rewind` and its announcement path. Split out
/// for the 500-line module budget; see the file header there.
mod commit;

/// The record-binding half — the pure predicate deciding whether a `cf_undo` record still
/// describes the block that NOW occupies its height (AUDIT-P1-001). Staleness/drift
/// detection over public, unkeyed inputs, NOT authentication (AUDIT-P3-401).
mod binding;

use super::*;

use doli_core::TxType;
use storage::MaintainerUndoSnapshot;

/// Fixed grep anchor for a rewind that could NOT restore the trust root.
///
/// REQ-174-005. Same shape and intent as the `periodic.rs` deleted-file re-seed anchor:
/// one token an operator can grep fleet-wide, carrying the digest to compare against and
/// naming the cross-check action.
pub(super) const MAINTAINER_REWIND_UNRESTORED_ANCHOR: &str = "MAINTAINER_REWIND_UNRESTORED";

/// The pseudo-path reported by `validate_persisted_set` when it refuses a snapshot.
/// The gate takes a `&Path` purely to label its error; this is not a real file.
const UNDO_SNAPSHOT_SOURCE: &str = "cf_undo:maintainer_snapshot";

/// Machine-readable `reason=` sub-tokens on [`MAINTAINER_REWIND_UNRESTORED_ANCHOR`].
///
/// QA-174-005. The anchor is graded as a SECURITY signal — the deploy note tells operators
/// to grep the fleet for it — but not every way of reaching it means the trust root
/// diverged. A holed block store is a known prior-incident state (INC-I-152 /
/// AUDIT-P1-003) and gets an anchor line with no rotation involved at all. Without a
/// stable token the two are separable only by parsing English prose, so a fleet-wide grep
/// cannot tell "this host CANNOT PROVE the range was clean" from "this host IS PROVABLY
/// DIVERGED", and the security-grade anchor decays into alarm fatigue.
///
/// The prose still carries the detail; the token is what a `grep | awk` pipeline keys on.
mod reason {
    /// CANNOT PROVE. A block in the rewound range is missing or unreadable, so this node
    /// cannot rule out a rotation at that height. No divergence is established.
    pub(super) const BLOCK_UNREADABLE: &str = "block_unreadable";
    /// PROVABLY DIVERGED. A block in the range demonstrably carries a rotation and no
    /// snapshot below it can undo that.
    pub(super) const ROTATION_WITHOUT_SNAPSHOT: &str = "rotation_without_snapshot";
    /// PROVABLY DIVERGED. A snapshot exists but `validate_persisted_set` refused it, so
    /// the pre-rewind root cannot be reinstated.
    pub(super) const SNAPSHOT_REFUSED: &str = "snapshot_refused";
    /// PROVABLY DIVERGED. The snapshot is the empty, never-seeded state; installing it
    /// would re-arm the one-shot bootstrap seed, so it is refused (REQ-174-002 AC-4).
    pub(super) const SNAPSHOT_WOULD_RE_ARM_SEED: &str = "snapshot_would_re_arm_seed";
    /// PROVABLY DIVERGED, and worse: the restore succeeded in memory and failed to
    /// persist, so it was rolled back to keep memory and disk in agreement.
    pub(super) const PERSIST_FAILED: &str = "persist_failed";
    /// PROVABLY DIVERGED, and the AUDIT-P1-001 case: a snapshot exists at this height and
    /// the block now occupying the height DOES carry a rotation, but the record was
    /// captured for a DIFFERENT block. Its own `block_hash` says so. Reached whenever a
    /// canonical-index bypass writer (`backfillFromPeer`, `doli-node restore`, the
    /// archiver, `rebuild_canonical_index`) put a new block at a height whose record was
    /// never refreshed, because `put_block_canonical` writes only `CF_HEIGHT_INDEX` +
    /// `CF_HASH_TO_HEIGHT` and never touches the 9-byte `cf_undo` family.
    ///
    /// Its own token, separate from [`SNAPSHOT_REFUSED`]: this one says the operator's
    /// LAST RECOVERY ACTION left the record and the chain describing different blocks,
    /// which is a different runbook from "the persisted set is malformed".
    pub(super) const SNAPSHOT_BLOCK_MISMATCH: &str = "snapshot_block_mismatch";
    /// PROVABLY DIVERGED. The record does not carry this format generation's magic or
    /// version, so it was written by another binary generation or is not one of these
    /// records at all. Refused rather than decoded: this value decides which binary the
    /// host installs.
    pub(super) const SNAPSHOT_HEADER_INVALID: &str = "snapshot_header_invalid";
    /// PROVABLY DIVERGED. The record's `set` does not hash to the `set_digest` captured
    /// alongside it, so the member list was altered after it was written.
    pub(super) const SNAPSHOT_DIGEST_MISMATCH: &str = "snapshot_digest_mismatch";
    /// SHOULD BE UNREACHABLE. A `Restore` plan reached the commit phase on a node that
    /// holds no maintainer trust root at all — `plan_maintainer_rewind` returns
    /// `Unchanged` when `maintainer_state` is `None`, so this can only mean the two halves
    /// have drifted. Emitted (and counted) rather than returned silently, because the
    /// "there is no third exit" claim in `commit.rs` must be true LOCALLY and not by
    /// coupling to another file (reviewer F3).
    pub(super) const NO_TRUST_ROOT: &str = "no_trust_root";
}

/// True when the block carries a transaction that mutates the maintainer set.
///
/// The predicate is PURELY "does this block carry `AddMaintainer` / `RemoveMaintainer`".
/// It deliberately does NOT include the `at_epoch_boundary` term that
/// `needs_producer_snapshot` ORs in (`apply_block/mod.rs`): producer mutations are
/// epoch-DEFERRED so a boundary block can change the producer set without carrying a
/// producer transaction, while maintainer changes are applied immediately at the exact
/// block that carries them (`apply_block/governance.rs`). Reusing the producer predicate
/// would capture snapshots at heights that changed nothing, and would only avoid missing
/// a real change by accident.
///
/// It keys on the transaction TYPE, before verification. A rotation whose signatures do
/// not verify therefore still captures a snapshot — one that is byte-identical to the
/// live value, so the restore is a no-op. Keying on the OUTCOME instead would mean
/// re-deriving the verification verdict on the rollback path, where the set has already
/// moved.
pub(super) fn block_mutates_maintainer_set(block: &Block) -> bool {
    block
        .transactions
        .iter()
        .any(|tx| matches!(tx.tx_type, TxType::AddMaintainer | TxType::RemoveMaintainer))
}

/// What a rewind over a height range must do to the maintainer trust root.
#[derive(Debug)]
pub(super) enum MaintainerRewindPlan {
    /// No block in the range mutated the maintainer set. Do nothing — and in particular
    /// do NOT rewrite `maintainer_state.bin`, or a 100-block reorg becomes 100 durable
    /// writes of the install authority.
    Unchanged,
    /// Restore this snapshot. It was captured before the OLDEST rotation in the range,
    /// so it is the state as of the rewind target.
    Restore {
        /// The height whose snapshot this is (for logging).
        height: u64,
        snapshot: Box<MaintainerUndoSnapshot>,
    },
    /// A height in the range mutated the maintainer set and no snapshot below it can
    /// undo that. Restoring anything available would install an intermediate state, so
    /// the live root is kept and the divergence is ANNOUNCED instead.
    Unrestorable {
        /// The offending height.
        height: u64,
        /// The machine-readable `reason=` sub-token — one of [`reason`].
        token: &'static str,
        /// Why, in one clause, for the operator-facing log line.
        reason: String,
    },
}

impl Node {
    /// The pre-block maintainer trust root to record for `block`, or `None` when the
    /// block does not touch it (REQ-174-001).
    ///
    /// Called from `apply_block` BEFORE the transaction loop, so a block carrying TWO
    /// rotations records the state before the FIRST — a per-transaction capture would
    /// leave an intermediate state in `cf_undo` and restore a set that existed at no
    /// block boundary on any chain.
    ///
    /// `None` is the "unchanged at this height" sentinel: no record is written and no
    /// set bytes are serialized (INC-I-071 `cf_undo` bloat discipline). The conditional
    /// exists to keep a per-block `RwLock` read off the hot path — the snapshot itself is
    /// only ~200 B, so this is not a size optimization.
    ///
    /// The record is stamped with the hash of the block it is captured FOR and with the
    /// digest of the set it carries (AUDIT-P1-001 / SYS-001). Both are written HERE, at
    /// the one moment the record's subject is unambiguous — inside `apply_block`, holding
    /// the block that is about to mutate the root. Every later reader has only a HEIGHT to
    /// go on, and a height is exactly what a canonical-index bypass writer can silently
    /// re-point at another block.
    pub(super) async fn capture_maintainer_undo(
        &self,
        block: &Block,
    ) -> Option<MaintainerUndoSnapshot> {
        if !block_mutates_maintainer_set(block) {
            return None;
        }
        let state = self.maintainer_state.as_ref()?;
        let ms = state.read().await;
        let digest = doli_core::maintainer::maintainer_set_digest(
            &ms.set,
            self.params.genesis_hash.as_bytes(),
        );
        Some(MaintainerUndoSnapshot::new(
            block.hash(),
            digest,
            ms.set.clone(),
            ms.last_derived_height,
        ))
    }

    /// Decide what a rewind of `[lo, hi]` (inclusive, `lo = target_height + 1`) must do
    /// to the maintainer trust root. **Pure reads — mutates nothing.**
    ///
    /// ## Oldest wins
    ///
    /// The set can change more than once inside one rewind range, so the restore target
    /// is the snapshot captured before the LOWEST rotation in the range — the same
    /// "oldest wins" rule `producer_snapshot` already uses in `execute_reorg`. Taking
    /// the newest would install a state that existed only between two rolled-back
    /// blocks, i.e. at no block boundary on any chain.
    ///
    /// ## The BLOCK decides, never the record alone (reviewer F1)
    ///
    /// A `cf_undo` maintainer record is authority for height `h` ONLY while the block now
    /// occupying `h` still carries the rotation the record was captured for. Nothing keeps
    /// that true on its own: `batch.put_undo` (`apply_block/mod.rs`) is UNCONDITIONAL and
    /// so always overwrites the stale `UndoData` at a re-applied height, but
    /// `batch.put_maintainer_undo` is written only for a block that carries a rotation, no
    /// rewind path deletes maintainer records, and the only reaper —
    /// `prune_undo_before(height - UNDO_KEEP_DEPTH)` — walks the TAIL. A record from a
    /// branch that was abandoned therefore survives at the tip for up to `UNDO_KEEP_DEPTH`
    /// blocks, over a height the replacement branch may re-use with NO rotation.
    ///
    /// Trusting "a snapshot exists at `h`, therefore restore it" installs that fossil as
    /// the release-verification trust root: a member list that exists on NO chain, through
    /// the SUCCESS exit, so `maintainer_rewind_count` increments and
    /// `MAINTAINER_REWIND_RESTORED` is logged at `info!`. The operator gets a success
    /// signal for a divergence in the auto-updater's install authority.
    ///
    /// So the scan below is a single upward pass in which the BLOCK is always consulted
    /// first, and it answers all three questions at each height:
    ///
    /// * block carries a rotation, snapshot present → hand the record to
    ///   [`binding::check_snapshot_binding`], which promotes it to
    ///   [`MaintainerRewindPlan::Restore`] only if the record says it belongs to THIS
    ///   block. Oldest wins, because the walk stops at the first such height.
    /// * block carries a rotation, snapshot absent → `Unrestorable{rotation_without_snapshot}`
    ///   (applied by a binary predating INC-I-174, or the record was pruned).
    /// * block carries NO rotation → any snapshot at this height is a FOSSIL. Ignore it and
    ///   continue; the height changed nothing, so there is nothing to undo.
    /// * block unreadable → `Unrestorable{block_unreadable}`. A missing block cannot rule
    ///   out a rotation, so "no rotation here" may not be assumed.
    ///
    /// ## And the BLOCK's IDENTITY decides, never its height (AUDIT-P1-001)
    ///
    /// The four bullets above ask the height index which block sits at `h`, and that index
    /// is rewritable out of band: `put_block_canonical` (`backfillFromPeer`, `doli-node
    /// restore`, the archiver, `rebuild_canonical_index`) re-points it with no
    /// `apply_block` and no refresh of the maintainer record. A replacement block that
    /// happens to carry a rotation therefore passes the tx-type cross-check while the
    /// record below it still describes the block that was there before — and the fossil
    /// installs through the SUCCESS exit. So the record is additionally required to name
    /// the block it was captured for; see [`binding`] for the full reachability argument
    /// and for why the check cannot live in the shared `validate_persisted_set` gate.
    ///
    /// The alternative — deleting the maintainer record for every rewound height — was
    /// rejected: it adds `depth` durable writes to the exact non-atomic sequence
    /// AUDIT-P1-201 already records as half-applied-on-abort, and it FAILS OPEN, because a
    /// missed delete silently restores the wrong root again. This cross-check adds no
    /// durable side effect and is self-healing: a fossil that is never trusted needs no
    /// deleting. (`StateDb::delete_maintainer_undo` is the primitive that variant would
    /// have needed; see the note on its definition for why it stays uncalled here.)
    ///
    /// ## What the scan costs (QA-174-006)
    ///
    /// One `get_block_by_height` per height from `lo` up to and including the restore point
    /// — and the whole range when there is no rotation in it, which is the steady state. So
    /// the common case is the worst case, bounded by the reorg depth. Folding the lookup in
    /// costs at most ONE extra block decode versus the previous two-pass shape (the case
    /// where the restore point is `lo` itself). That is acceptable because both callers
    /// already pay a full `atomic_replace` over the entire UTXO set on this same path,
    /// which dominates.
    pub(super) fn plan_maintainer_rewind(&self, lo: u64, hi: u64) -> MaintainerRewindPlan {
        if self.maintainer_state.is_none() || lo > hi {
            return MaintainerRewindPlan::Unchanged;
        }

        for h in lo..=hi {
            let block = match self.block_store.get_block_by_height(h) {
                Ok(Some(block)) => block,
                Ok(None) => {
                    return MaintainerRewindPlan::Unrestorable {
                        height: h,
                        token: reason::BLOCK_UNREADABLE,
                        reason: format!(
                            "the block at h={h} is missing from the block store, so this node \
                             CANNOT PROVE that height carried no maintainer rotation. No \
                             divergence is established — a holed block store (INC-I-152 / \
                             AUDIT-P1-003) reaches this line with no rotation anywhere in the \
                             range"
                        ),
                    }
                }
                Err(e) => {
                    return MaintainerRewindPlan::Unrestorable {
                        height: h,
                        token: reason::BLOCK_UNREADABLE,
                        reason: format!(
                            "the block at h={h} could not be read ({e}), so this node CANNOT \
                             PROVE that height carried no maintainer rotation. No divergence is \
                             established"
                        ),
                    }
                }
            };

            if !block_mutates_maintainer_set(&block) {
                // This height changed nothing, so there is nothing to undo here — and any
                // `cf_undo` record sitting at it belongs to a block that is no longer
                // canonical. Ignoring it is the whole of the F1 fix.
                continue;
            }

            return match self.state_db.get_maintainer_undo(h) {
                // AUDIT-P1-001: "a rotation is here and a record is here" is a statement
                // about POSITIONS, not about the record. The record itself must say it
                // belongs to THIS block, on THIS chain, in THIS format — see `binding`.
                Some(snapshot) => binding::check_snapshot_binding(
                    h,
                    block.hash(),
                    self.params.genesis_hash.as_bytes(),
                    snapshot,
                ),
                None => MaintainerRewindPlan::Unrestorable {
                    height: h,
                    token: reason::ROTATION_WITHOUT_SNAPSHOT,
                    reason: format!(
                        "the block at h={h} carries a maintainer rotation but no undo \
                         snapshot — it was applied by a binary that predates INC-I-174, or \
                         the record was pruned. This host's root is PROVABLY out of step \
                         with the post-rewind chain"
                    ),
                },
            };
        }

        MaintainerRewindPlan::Unchanged
    }
}
