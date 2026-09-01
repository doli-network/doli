use super::*;

/// What `rollback_one_block` actually did (INC-I-204 M3).
///
/// The FORK_GUARD_BACKFILL refusal used to return `Ok(true)` — a success that
/// mutated nothing — so the caller burned a rollback-budget rung and logged
/// "rollback succeeded" for a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackOutcome {
    /// One block was undone: chain state, UTXO set and producer set all moved.
    RolledBack,
    /// Refused before any mutation (at genesis, cap reached, or a gapped store).
    RefusedNoMutation,
    /// Refused before any mutation because the caller's authority does not cover
    /// this rewind (INC-I-204 M4.2): a self-apply failure at a height that is not
    /// the local tip, i.e. a block this node never applied and never published.
    RefusedNotAuthorized,
}

impl Node {
    /// Roll back 1 block for fork recovery.
    ///
    /// Called by the RecoveryCoordinator dispatch in periodic.rs on a
    /// `ShallowRollback` action, and by the production BLOCK_POISON arm.
    pub async fn rollback_one_block(
        &mut self,
        authority: RollbackAuthority,
    ) -> Result<RollbackOutcome> {
        let local_height = {
            let sync = self.sync_manager.read().await;
            sync.local_tip().0
        };

        let (authority_label, requested_depth) = match authority {
            RollbackAuthority::CoordinatorApproved { depth } => ("coordinator_approved", depth),
            RollbackAuthority::ProductionSelfApply { .. } => ("production_self_apply", 1),
        };

        // Log all context that led to this rollback being initiated.
        // Captures the numeric state for post-incident root-cause analysis.
        {
            let sync = self.sync_manager.read().await;
            let empty_headers = sync.consecutive_empty_headers();
            let best_peer_h = sync.best_peer_height();
            let gap = best_peer_h.saturating_sub(local_height);
            info!(
                "[ROLLBACK] Initiating: authority={} requested_depth={} depth={} local_h={} target_h={} gap={} empty_headers={} shallow_count={}",
                authority_label,
                requested_depth,
                self.cumulative_rollback_depth + 1,
                local_height,
                local_height.saturating_sub(1),
                gap,
                empty_headers,
                self.shallow_rollback_count
            );
        }

        // INC-I-204 M4.2 / REQ-FORK-002: `apply_block` runs BEFORE broadcast, so a
        // self-apply failure at a height above the tip would rewind the PARENT —
        // this node's already-published, possibly-finalized block. Refuse before
        // any mutation and before the budget moves.
        if let RollbackAuthority::ProductionSelfApply { failed_height } = authority {
            if local_height != failed_height {
                warn!(
                    "[ROLLBACK] Refused: production self-apply failure at h={} is not the \
                     local tip h={} — nothing this node applied, so nothing to undo.",
                    failed_height, local_height
                );
                return Ok(RollbackOutcome::RefusedNotAuthorized);
            }
        }

        if local_height == 0 {
            return Ok(RollbackOutcome::RefusedNoMutation);
        }

        let target_height = local_height - 1;

        // Fix 3: Never rollback to genesis from an established chain.
        // Rolling back to height 0 destroys all chain state and is never the right
        // recovery action for a running node. If we're at height 1, the chain is
        // effectively at genesis — there's nothing useful to rollback to.
        if target_height == 0 && local_height > 1 {
            warn!(
                "Refusing rollback to genesis from height {} — would destroy chain state. \
                 Manual intervention required (recover --yes).",
                local_height
            );
            return Ok(RollbackOutcome::RefusedNoMutation);
        }

        // Fix 4: Cap cumulative rollback depth at 50 blocks.
        // Prevents cascading rollbacks from gradually eroding the chain back to genesis.
        // After 50 rollbacks without a successful block application, the fork is too
        // deep for rollback-based recovery — manual intervention or sync is needed.
        const MAX_CUMULATIVE_ROLLBACK: u32 = 50;
        if self.cumulative_rollback_depth >= MAX_CUMULATIVE_ROLLBACK {
            warn!(
                "Refusing rollback: cumulative depth {} reached limit {} — \
                 too deep for rollback recovery. Waiting for sync or manual intervention.",
                self.cumulative_rollback_depth, MAX_CUMULATIVE_ROLLBACK
            );
            return Ok(RollbackOutcome::RefusedNoMutation);
        }

        // Invalidate genesis producer cache if rollback crosses genesis boundary
        let genesis_blocks = self.config.network.genesis_blocks();
        if genesis_blocks > 0 && target_height <= genesis_blocks {
            info!("[ROLLBACK] Crossing genesis boundary — invalidating genesis producer cache");
            self.cached_genesis_producers = std::sync::OnceLock::new();
        }

        let genesis_hash = self.chain_state.read().await.genesis_hash;

        let (parent_hash, parent_slot) = if target_height == 0 {
            (genesis_hash, 0u32)
        } else {
            match self.block_store.get_block_by_height(target_height)? {
                Some(parent_block) => (parent_block.hash(), parent_block.header.slot),
                None => {
                    error!("Cannot rollback: no block at height {}", target_height);
                    return Ok(RollbackOutcome::RefusedNoMutation);
                }
            }
        };

        info!(
            "Rolling back from height {} to {} for fork recovery",
            local_height, target_height
        );

        // INC-I-071: fetch undo data ONCE and reuse for both the UTXO/Producer
        // restore (this block) and the EpochState restore (later block). The
        // previous code called get_undo(local_height) twice, deserializing the
        // same RocksDB value twice per rollback.
        let cached_undo = self.state_db.get_undo(local_height);

        // INC-I-174: decide the maintainer trust-root rewind NOW, while the block at
        // `local_height` is still reachable through the height index — the fossil purge
        // below (`remove_canonical_entry`) makes it unreadable. Pure reads; the mutation
        // happens in `commit_maintainer_rewind` after `atomic_replace`. Deliberately
        // OUTSIDE the `cached_undo` branch: the maintainer record is keyed independently
        // of `UndoData`, so it is still restorable on the rebuild-from-genesis fallback.
        let maintainer_plan = self.plan_maintainer_rewind(local_height, local_height);

        // INC-I-156 / AUDIT-P1-001: true only if THIS call armed the rebuild
        // marker. The disarm below is the exact inverse of the arm, so an
        // undo-based rollback — which reconstructs nothing — can never silently
        // clear a halt raised by an earlier interrupted rebuild.
        let mut rebuild_marker_armed = false;

        // Try undo-based rollback first (O(1) for single block)
        if let Some(ref undo) = cached_undo {
            info!(
                "Undo-based rollback: reverting block at height {}",
                local_height
            );
            {
                let mut utxo = self.utxo_set.write().await;

                // Remove UTXOs created by this block
                for outpoint in &undo.created_utxos {
                    utxo.remove(outpoint)?;
                }

                // Restore UTXOs spent by this block
                for (outpoint, entry) in &undo.spent_utxos {
                    utxo.insert(*outpoint, entry.clone())?;
                }
            }

            // INC-I-071: empty producer_snapshot is the sentinel meaning
            // "ProducerSet unchanged at this height — skip restore". The
            // in-memory ProducerSet is already correct for h-1. Legacy
            // (pre-fix) undo entries always have a non-empty snapshot and
            // continue to take the deserialize-and-restore path below.
            if undo.producer_snapshot.is_empty() {
                debug!(
                    "[ROLLBACK] Empty producer_snapshot sentinel at h={} — \
                     ProducerSet unchanged at this block, skipping restore",
                    local_height
                );
            } else if let Ok(restored_producers) =
                bincode::deserialize::<storage::ProducerSet>(&undo.producer_snapshot)
            {
                let mut producers = self.producer_set.write().await;
                *producers = restored_producers;
            } else {
                warn!("Failed to deserialize producer snapshot, rebuilding from blocks");
                let mut producers = self.producer_set.write().await;
                self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
            }
        } else {
            // Legacy fallback: rebuild from genesis (no undo data)
            warn!(
                "No undo data for height {} — falling back to rebuild from genesis",
                local_height
            );

            // AUDIT-P1-003 (INC-I-152): prove the rebuild can COMPLETE before any
            // state is mutated. This pre-check used to ask a single-block question
            // — "does block 1 exist?" — while the loop below needs `1..=target_height`
            // to be DENSE. A holed store (genesis prefix + tip present, the middle
            // never fetched — the INC-I-152 shape) passed the block-1-only check,
            // so control entered the mutating section: it took the UTXO write lock
            // and replayed the surviving prefix, where `add_transaction` is a direct
            // `state_db.insert_utxo` on the production RocksDb backend. That
            // RESURRECTS outputs whose compensating spends live inside the hole
            // (money from nothing, the INC-I-041 class), and the loop then aborted
            // at the first missing height, leaving the node half-replayed with no
            // undo of the damage. Same guard, same shape as the sibling reorg path
            // in block_handling.rs: refuse, surface the reason, let sync backfill.
            // Nothing is locked and no UTXO is touched on this path.
            //
            // `.max(1)` preserves the replaced pre-check's unconditional "block 1
            // must exist" requirement for the `target_height == 0` case (rolling
            // block 1 back to genesis), where the rebuild range is empty and
            // `ensure_blocks_present` would otherwise be a no-op. The guard is
            // therefore never weaker than the check it replaces, for any input.
            //
            // Do NOT snap sync — it destroys the block store further.
            if let Err(e) = self
                .block_store
                .ensure_blocks_present(1, target_height.max(1))
            {
                crate::metrics::FORK_GUARD_REFUSALS
                    .with_label_values(&["rollback_rebuild"])
                    .inc();
                warn!(
                    "[FORK_GUARD_BACKFILL_REQUIRED] Rollback rebuild refused: block_store \
                     incomplete over 1..={} — {}. No state mutated. Skipping rollback, \
                     header-first sync will backfill.",
                    target_height.max(1),
                    e
                );
                return Ok(RollbackOutcome::RefusedNoMutation);
            }

            let genesis_producers = if genesis_blocks > 0 && target_height > genesis_blocks {
                self.derive_genesis_producers_from_chain()
            } else {
                Vec::new()
            };
            let bond_unit = self.config.network.bond_unit();

            // INC-I-156 / AUDIT-P1-001: arm the durable rebuild marker BEFORE
            // the wipe below commits. See the twin comment at
            // `block_handling.rs` — same window, same watchdog trigger, same
            // absence of any other detector. A failure to arm aborts the
            // rollback with nothing mutated.
            self.state_db
                .set_rebuild_in_progress(target_height)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Rollback UTXO rebuild: failed to arm the rebuild-in-progress marker \
                         — {}. No wipe attempted; state left unchanged.",
                        e
                    )
                })?;
            rebuild_marker_armed = true;

            {
                let mut utxo = self.utxo_set.write().await;
                // INC-I-156 R1: this now really empties the set on BOTH backends.
                // It used to be a silent no-op on the production RocksDb variant,
                // so the replay below stacked the rebuilt `1..=target_height` set
                // ON TOP of the un-rolled-back one: every output created by the
                // rolled-back range and unspent within it survived, durably
                // (`atomic_replace` below then laundered it to disk). That is the
                // INC-I-041 zombie-UTXO / inflation class and it violates
                // INV-UTXO-001. A FAILED wipe must abort the rollback here with
                // state untouched — replaying onto an un-cleared set is the defect
                // itself, so it must never be reachable via an ignored error.
                utxo.clear().map_err(|e| {
                    anyhow::anyhow!(
                        "Rollback UTXO rebuild: failed to clear the UTXO set — {}. \
                         No replay attempted; state left unchanged.",
                        e
                    )
                })?;
                for height in 1..=target_height {
                    let block = self
                        .block_store
                        .get_block_by_height(height)?
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Rollback UTXO rebuild: missing block at height {}",
                                height
                            )
                        })?;
                    for (tx_index, tx) in block.transactions.iter().enumerate() {
                        let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                        if !is_reward_tx {
                            // INC-I-064: Log spend failures in rollback rebuild path
                            if let Err(e) = utxo.spend_transaction(tx) {
                                warn!(
                                    "[ROLLBACK_REBUILD] spend_transaction failed at h={}: {} — continuing rebuild",
                                    height, e
                                );
                            }
                        }
                        utxo.add_transaction(tx, height, is_reward_tx, block.header.slot)?;
                    }
                    if genesis_blocks > 0 && height == genesis_blocks + 1 {
                        Self::consume_genesis_bond_utxos(
                            &mut utxo,
                            &genesis_producers,
                            bond_unit,
                            height,
                        )?;
                    }
                }
            }

            {
                let mut producers = self.producer_set.write().await;
                self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
            }
        }

        // Rebuild producer liveness map from canonical block_store.
        // Critical: rollback does NOT undo liveness entries from fork blocks,
        // causing nodes to have divergent live_producers lists and conflicting
        // round-robin assignments. Rebuilding from block_store ensures all nodes
        // converge on the same liveness view.
        self.rebuild_producer_liveness(target_height);

        // Update chain state to parent
        {
            let mut state = self.chain_state.write().await;
            state.best_height = target_height;
            state.best_hash = parent_hash;
            state.best_slot = parent_slot;
        }

        // INC-I-144: purge the height-index fossil for the rewound block in the
        // same rollback that rewinds chain_state. Standalone rollback has NO
        // paired re-apply (unlike a reorg, which heals via set_canonical_chain
        // on the winning branch), so without this the rolled-back orphan at
        // `local_height` remains a permanent fossil — get_block_by_height keeps
        // serving it. The deleter is guarded on the stored hash, so a newer
        // branch that already overwrote the entry is not clobbered.
        if let Some(orphan) = self.block_store.get_hash_by_height(local_height)? {
            self.block_store
                .remove_canonical_entry(local_height, orphan)?;
        }

        // Update sync manager: local tip + reset fork signals
        {
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(target_height, parent_hash, parent_slot);
            sync.reset_sync_for_rollback();
            // Fix #2b-bis (2026-04-15, synmgrefactor): record post-rollback
            // height so that note_orphan_gossip_block can detect the
            // "applied since rollback → behind, not forked" case and skip
            // further rollback signals.
            sync.note_rollback_completed(target_height);
            // INC-I-081 Bug 4 / INV-SYNC-004: clear stale finality marker if
            // the post-rollback tip has dropped below the cached finality height.
            // INC-I-204 M4.2 / INV-FINALITY-001 (trap T12): never on the production
            // path. The guard above pins `target == failed_height - 1` while the
            // marker is at most `failed_height - 2`, so this call is already a no-op
            // there — skipping it seals the clause against a future edit.
            if !matches!(authority, RollbackAuthority::ProductionSelfApply { .. }) {
                sync.clear_finality_if_below_tip(target_height);
            }
        }

        // Atomically persist the rolled-back state via StateDb.
        // Collects all UTXOs from in-memory set and writes everything in one WriteBatch.
        {
            let state = self.chain_state.read().await;
            let producers = self.producer_set.read().await;
            let utxo = self.utxo_set.read().await;
            let utxo_pairs = utxo.iter_all();
            self.state_db
                .atomic_replace(&state, &producers, utxo_pairs.into_iter())
                .map_err(|e| anyhow::anyhow!("StateDb atomic_replace failed: {}", e))?;
        }

        // INC-I-174: rewind the maintainer trust root. Placed HERE — after
        // `atomic_replace` has durably committed the chain rewind — on purpose.
        // AUDIT-P1-201 (open, P1) records that this function already abandons a durable
        // half-applied UTXO undo when a step between the UTXO mutation and
        // `atomic_replace` errors; putting the `ms.save()` inside that window would add
        // one more durable side effect to the non-atomic sequence and WIDEN the recorded
        // gap. Above it, an earlier abort simply leaves the trust root untouched, which
        // is correct: the chain did not rewind, so the root must not either.
        //
        // REQ-174-005 AC-3 — it runs BEFORE the rebuild-marker clear below, not after.
        // The clear propagates its error with `?`, so ordering it first opened a route by
        // which the chain rewind was already durable while the trust root still carried
        // the dropped rotation, with NO `MAINTAINER_REWIND_UNRESTORED` line and no counter
        // increment. AC-3 forbids that without qualification. The commit has no dependency
        // on the marker, so moving it up is free. `execute_reorg` carries the identical
        // ordering — them drifting is the INC-I-040 shape this milestone exists to avoid.
        self.commit_maintainer_rewind(maintainer_plan, target_height)
            .await;

        // INC-I-156 / AUDIT-P1-001: disarm ONLY here, and ONLY if this call
        // armed it — the durable set is a complete state again.
        if rebuild_marker_armed {
            self.state_db
                .clear_rebuild_in_progress()
                .map_err(|e| anyhow::anyhow!("Rollback: failed to clear rebuild marker: {}", e))?;
        }

        // Restore epoch scheduler state from undo data (O(1) vs O(chain) rebuild).
        // The undo snapshot was taken BEFORE apply_block, so it reflects the correct
        // scheduler state at the pre-rollback height.
        //
        // INC-I-071: reuse `cached_undo` from the start of this function rather
        // than calling get_undo() again (the previous code deserialized the same
        // RocksDB entry twice). EpochState snapshots are NOT covered by the
        // empty-sentinel optimization — they are always present.
        if let Some(ref undo) = cached_undo {
            if let Some(ref epoch_bytes) = undo.epoch_state_snapshot {
                match doli_core::EpochState::deserialize(epoch_bytes) {
                    Ok(restored) => {
                        info!(
                            "[ROLLBACK] Restored epoch state from undo: epoch={} producers={} active={}",
                            restored.epoch, restored.producer_list.len(), restored.active_list.len()
                        );
                        self.epoch_state = restored;
                        // Persist to DB — atomic_replace above doesn't include epoch_state,
                        // so a crash between here and the next apply_block would lose it.
                        self.state_db.put_epoch_state(epoch_bytes);
                    }
                    Err(e) => {
                        warn!("[ROLLBACK] Failed to deserialize epoch state from undo: {} — rebuilding", e);
                        self.rebuild_epoch_state_from_blocks(target_height).await;
                    }
                }
            } else {
                // Pre-upgrade undo data (no epoch_state_snapshot) — fall back to rebuild
                info!("[ROLLBACK] No epoch state in undo (pre-upgrade block) — rebuilding");
                self.rebuild_epoch_state_from_blocks(target_height).await;
            }
        } else {
            // No undo data at all (legacy path already handled above) — rebuild
            self.rebuild_epoch_state_from_blocks(target_height).await;
        }

        // AUDIT-P2-005: reset oracle_sunset_triggered to reflect the rolled-back
        // chain's sunset state. Pre-fix the atomic retained the sunset state
        // from the rolled-back chain, so up to one epoch of validation
        // decisions would be made against the wrong sunset flag. The
        // persisted OracleSunsetState is local bookkeeping (not in the
        // state root), so we recompute health from the persisted state at
        // the target_height's epoch. Note: OracleSunsetState reflects the
        // most-recent aggregator run; if the rollback crosses an epoch
        // boundary the next aggregator pass will re-derive correctly,
        // but until then the atomic must at minimum match the persisted
        // state at the post-rollback epoch.
        {
            let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
            let target_epoch = target_height.checked_div(blocks_per_epoch).unwrap_or(0);
            let sunset_state = self.state_db.get_oracle_sunset_state().unwrap_or_default();
            let triggered = sunset_state.health(target_epoch).is_sunset_triggered();
            self.oracle_sunset_triggered
                .store(triggered, std::sync::atomic::Ordering::Release);
            info!(
                "[ORACLE] rollback h={}: sunset_triggered={} (target_epoch={})",
                target_height, triggered, target_epoch
            );
        }

        // Chain commitment: invalidate on rollback. Periodic scan in periodic.rs
        // will recompute it correctly on the next tick.
        self.state_db.delete_chain_commitment();

        // Track cumulative rollback depth (Fix 4)
        self.cumulative_rollback_depth += 1;

        info!(
            "[FORK] ROLLBACK_DONE h={} hash={:.8} cumulative_depth={}",
            target_height, parent_hash, self.cumulative_rollback_depth
        );

        Ok(RollbackOutcome::RolledBack)
    }
}
