//! INC-I-174 — the MUTATING half of the maintainer rewind.
//!
//! Split from `mod.rs` for the 500-line module budget (global rule 19), along the phase
//! boundary the module header already draws: `mod.rs` holds the PURE-READ planning half
//! (`capture_maintainer_undo`, `plan_maintainer_rewind`) and this file holds the half that
//! writes the trust root and the observability counters. Both callers still reach exactly
//! two functions, so there is still no second implementation to drift from (INC-I-040).

use super::*;

impl Node {
    /// Apply a [`MaintainerRewindPlan`] to the live trust root and persist it.
    ///
    /// Call AFTER the caller's `atomic_replace` — see the module header for the
    /// AUDIT-P1-201 placement argument. `target_height` is the post-rewind tip and is
    /// used only for the log anchors.
    ///
    /// ## Every exit is either counted or a no-op
    ///
    /// A restore increments `maintainer_rewind_count`; any way of NOT restoring
    /// something that needed restoring increments `maintainer_rewind_unrestored_count`
    /// and emits [`MAINTAINER_REWIND_UNRESTORED_ANCHOR`]. There is no third exit, which
    /// is what makes "no silent route exists" (REQ-174-005) checkable by a test rather
    /// than by reading the code.
    ///
    /// That claim is now true LOCALLY, which it was not before (reviewer F3). Two exits —
    /// the `maintainer_state.is_none()` guard here and the same guard inside
    /// [`Self::announce_unrestored_rewind`] — used to `return` without counting, and were
    /// unreachable only because `plan_maintainer_rewind` returns `Unchanged` in that case.
    /// That is a coupling between two files, not an invariant this function can state. Both
    /// now `debug_assert!` in test builds and take the counted, announced exit in release.
    // `pub(in crate::node)`, not `pub(super)`: after the file split `super` is
    // `maintainer_rewind`, and the two callers (`rollback`, `block_handling`) are siblings
    // of it under `node`. This keeps the exact visibility the single-file version had.
    pub(in crate::node) async fn commit_maintainer_rewind(
        &mut self,
        plan: MaintainerRewindPlan,
        target_height: u64,
    ) {
        let genesis_hash = self.params.genesis_hash;
        let (restore_height, snapshot) = match plan {
            MaintainerRewindPlan::Unchanged => return,
            MaintainerRewindPlan::Unrestorable {
                height,
                token,
                reason,
            } => {
                self.announce_unrestored_rewind(target_height, height, token, &reason)
                    .await;
                return;
            }
            MaintainerRewindPlan::Restore { height, snapshot } => (height, *snapshot),
        };

        // REQ-174-SEC-001. `cf_undo` is node-local, unsigned and attacker-writable given
        // data-dir access, exactly like `maintainer_state.bin` — so a snapshot that
        // DECODES is still not authority. This is the SAME function `MaintainerState::load`
        // runs (`storage::validate_persisted_set`), not a copy, so the two gates cannot
        // drift. Refuse, never repair: a deduplicated or threshold-corrected set is still
        // an attacker-chosen member list.
        if let Err(e) = storage::validate_persisted_set(
            std::path::Path::new(UNDO_SNAPSHOT_SOURCE),
            &snapshot.set,
        ) {
            let reason = format!(
                "the undo snapshot at h={restore_height} was REFUSED by the well-formedness \
                 gate ({e}). The live trust root is kept unchanged — it is NEVER degraded to \
                 an empty/default set, which would re-arm the compiled bootstrap keys \
                 (INC-I-172 F5)"
            );
            self.announce_unrestored_rewind(
                target_height,
                restore_height,
                reason::SNAPSHOT_REFUSED,
                &reason,
            )
            .await;
            return;
        }

        // REQ-174-002 AC-4 / REQ-174-SEC-001 AC-3 — the REWIND-SPECIFIC guard, deliberately
        // layered AFTER the shared gate above rather than folded INTO it.
        //
        // `validate_persisted_set` carves the empty set out ON PURPOSE
        // (`crates/storage/src/maintainer_wellformed.rs`): the LOAD path must keep an
        // unseeded node (`MaintainerState::default()`) loadable, and must keep an emptied
        // on-chain root failing closed instead of unbootable. Tightening the shared gate to
        // catch this would break `MaintainerState::load` for every fresh node — and
        // REQ-174-SEC-001 AC-4 requires the carve-out be honoured IDENTICALLY on both paths
        // so the two gates cannot drift. Hence: shared gate unchanged, extra guard here.
        //
        // What the REWIND may not do that a LOAD may: install a set the seed predicate
        // reads as "not yet seeded". The one-shot bootstrap is driven on EVERY applied
        // block (`apply_block/state_update.rs`), so installing such a set makes the next
        // block re-derive the root from LIVE producer state and RE-ARM any key governance
        // removed (INC-I-172 R1). A rewind that produces that state is strictly worse than
        // a rewind that does nothing.
        //
        // The predicate is CALLED, never mirrored (reviewer F2). Its shape depends on which
        // side of `maintainer_derivation_activation_height` the post-rewind tip is on:
        // above the gate it is "has this root EVER been seeded?" (`!members.is_empty() ||
        // last_derived_height != 0`), below it, it is the frozen historical
        // `is_fully_bootstrapped()` (`len >= 5`). An inline copy of the post-activation
        // form was correct only for Devnet, whose gate is 0 — mainnet's gate is 172_000 and
        // mainnet is BELOW it, so a restored set of 1..4 members passed the copy, re-armed
        // the seed, and was still counted a success.
        //
        // Reachable with no attacker: `capture_maintainer_undo` keys on the transaction
        // TYPE before verification, so a node whose root is still the default records
        // `{[], 0}` for the block carrying a rotation, and the seed fires later in that same
        // block. Fail CLOSED (keep the live root) and LOUD (anchor + counter) — a refusal
        // counted as a success would leave the operator no signal from the rewind at all.
        let one_shot = target_height
            >= self
                .config
                .network
                .params()
                .maintainer_derivation_activation_height;
        let candidate = storage::MaintainerState {
            set: snapshot.set.clone(),
            last_derived_height: snapshot.last_derived_height,
            ..Default::default()
        };
        if !Self::maintainer_seed_is_done(&candidate, one_shot) {
            let reason = format!(
                "the undo snapshot at h={} (members={}, last_derived_height={}) does NOT \
                 satisfy `maintainer_seed_is_done` at target height {} (one_shot={}). \
                 Installing it would let the one-shot bootstrap re-derive this host's trust \
                 root from LIVE producer state on the next applied block, re-arming any key \
                 governance removed (INC-I-172 R1). The live trust root is kept unchanged",
                restore_height,
                candidate.set.member_count(),
                candidate.last_derived_height,
                target_height,
                one_shot
            );
            self.announce_unrestored_rewind(
                target_height,
                restore_height,
                reason::SNAPSHOT_WOULD_RE_ARM_SEED,
                &reason,
            )
            .await;
            return;
        }

        // `plan_maintainer_rewind` returns `Unchanged` when `maintainer_state` is `None`,
        // so a `Restore` plan can only exist when the root is attached. That is a coupling
        // between two files, not a local invariant, so it is asserted rather than assumed —
        // and on the release path it takes the counted exit, never a silent one, because
        // "there is no third exit" (above) is what makes REQ-174-005 checkable by a test.
        let Some(maintainer_state) = self.maintainer_state.clone() else {
            debug_assert!(
                false,
                "plan_maintainer_rewind guarantees Some(maintainer_state) for a Restore plan"
            );
            let reason = format!(
                "the undo snapshot at h={restore_height} could not be installed: this node \
                 holds no maintainer trust root at all, which `plan_maintainer_rewind` is \
                 supposed to make unreachable for a Restore plan"
            );
            self.announce_unrestored_rewind(
                target_height,
                restore_height,
                reason::NO_TRUST_ROOT,
                &reason,
            )
            .await;
            return;
        };

        let mut ms = maintainer_state.write().await;
        let previous_set = ms.set.clone();
        let previous_derived = ms.last_derived_height;

        // REQ-174-002 AC-4: `last_derived_height` is restored with the set, never zeroed.
        // `maintainer_seed_is_done` reads (members.is_empty() && last_derived_height == 0)
        // as "never seeded", and the seed is driven on EVERY applied block
        // (`apply_block/state_update.rs`), so a restore that zeroed both would re-derive
        // the root from LIVE producer state on the next block and re-arm any key
        // governance removed — the INC-I-172 R1 hazard.
        ms.set = snapshot.set.clone();
        ms.last_derived_height = snapshot.last_derived_height;

        // REQ-174-006: an in-memory-only restore is undone by the next restart, because
        // `Node::new` re-reads this file and the updater reads it to decide who may
        // authorize a root install.
        if let Err(e) = ms.save(&self.config.data_dir) {
            // Surfaced, not swallowed. And the in-memory value is put BACK, so memory and
            // disk still agree: leaving them divergent would replace one silent
            // inconsistency with a worse one that no restart can resolve. The rollback
            // itself is already durable at this point, so returning an error here would
            // make the caller re-attempt a rewind that succeeded.
            ms.set = previous_set;
            ms.last_derived_height = previous_derived;
            let digest =
                doli_core::maintainer::maintainer_set_digest(&ms.set, genesis_hash.as_bytes());
            error!(
                "[MAINTAINER] {} h={} unrestored_height={} reason={} MAINTAINER_SET_DIGEST={} \
                 members={} threshold={} last_updated={} height={} — the rewind restored the \
                 trust root in memory but FAILED TO PERSIST it ({}). The in-memory value has \
                 been put back so memory and disk agree, which means this host's maintainer \
                 root may no longer match the canonical chain. Compare `getMaintainerSet` \
                 against a known-good node before trusting auto-update on this host.",
                MAINTAINER_REWIND_UNRESTORED_ANCHOR,
                target_height,
                restore_height,
                reason::PERSIST_FAILED,
                hex::encode(digest),
                ms.set.member_count(),
                ms.set.threshold,
                ms.set.last_updated,
                target_height,
                e
            );
            drop(ms);
            self.maintainer_rewind_unrestored_count += 1;
            return;
        }

        // REQ-174-008: the SAME anchor and field set the apply path publishes
        // (`apply_block/governance.rs`), so one grep covers both directions of travel.
        let digest = doli_core::maintainer::maintainer_set_digest(&ms.set, genesis_hash.as_bytes());
        info!(
            "[MAINTAINER] MAINTAINER_REWIND_RESTORED from_snapshot_h={} \
             MAINTAINER_SET_DIGEST={} members={} threshold={} last_updated={} height={}",
            restore_height,
            hex::encode(digest),
            ms.set.member_count(),
            ms.set.threshold,
            ms.set.last_updated,
            target_height
        );
        drop(ms);
        self.maintainer_rewind_count += 1;
    }

    /// REQ-174-005: the one place a rewind admits it left the trust root un-restored.
    ///
    /// Carries the current digest and member count so the operator can compare, and names
    /// the cross-check action — the same shape as the `periodic.rs` re-seed warning.
    ///
    /// `token` is the machine-readable [`reason`] sub-token. It is a separate argument from
    /// `reason` so that "cannot prove" ([`reason::BLOCK_UNREADABLE`]) and "provably
    /// diverged" stay separable by a fleet-wide grep without parsing the prose (QA-174-005).
    ///
    /// The counter increment is UNCONDITIONAL (reviewer F3). It used to sit after an early
    /// `return` taken when no trust root was attached, which discarded an `Unrestorable`
    /// plan without counting it — a silent route, and one whose unreachability rested on a
    /// coupling to `plan_maintainer_rewind` in another file rather than on anything local.
    async fn announce_unrestored_rewind(
        &mut self,
        target_height: u64,
        height: u64,
        token: &'static str,
        reason: &str,
    ) {
        let (digest, members, threshold, last_updated) = match &self.maintainer_state {
            Some(ms) => {
                let guard = ms.read().await;
                (
                    hex::encode(doli_core::maintainer::maintainer_set_digest(
                        &guard.set,
                        self.params.genesis_hash.as_bytes(),
                    )),
                    guard.set.member_count(),
                    guard.set.threshold,
                    guard.set.last_updated,
                )
            }
            None => {
                debug_assert!(
                    false,
                    "plan_maintainer_rewind returns Unchanged when maintainer_state is None, \
                     so an Unrestorable plan cannot reach this arm"
                );
                // No root means no digest to publish — but the rewind still may not exit
                // silently, so the anchor and the counter both still fire.
                ("<no-trust-root>".to_string(), 0, 0, 0)
            }
        };

        error!(
            "[MAINTAINER] {} h={} unrestored_height={} reason={} MAINTAINER_SET_DIGEST={} \
             members={} threshold={} last_updated={} height={} — {}. This host's \
             release-verification trust root may no longer match the canonical chain, and \
             nothing re-derives it above the one-shot seed. Compare `getMaintainerSet` against \
             a known-good node before trusting auto-update on this host.",
            MAINTAINER_REWIND_UNRESTORED_ANCHOR,
            target_height,
            height,
            token,
            digest,
            members,
            threshold,
            last_updated,
            target_height,
            reason
        );
        self.maintainer_rewind_unrestored_count += 1;
    }
}
