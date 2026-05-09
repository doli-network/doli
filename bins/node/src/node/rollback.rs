use super::*;

impl Node {
    /// Unconditionally roll back 1 block for fork recovery.
    ///
    /// No preconditions — it just rolls back. Called by the RecoveryCoordinator
    /// dispatch in periodic.rs when ShallowRollback action is classified.
    ///
    /// Returns `Ok(true)` if rollback succeeded, `Ok(false)` if at height 0.
    pub async fn rollback_one_block(&mut self) -> Result<bool> {
        let local_height = {
            let sync = self.sync_manager.read().await;
            sync.local_tip().0
        };

        // Log all context that led to this rollback being initiated.
        // Captures the numeric state for post-incident root-cause analysis.
        {
            let sync = self.sync_manager.read().await;
            let empty_headers = sync.consecutive_empty_headers();
            let best_peer_h = sync.best_peer_height();
            let gap = best_peer_h.saturating_sub(local_height);
            info!(
                "[ROLLBACK] Initiating: depth={} local_h={} target_h={} gap={} empty_headers={} shallow_count={}",
                self.cumulative_rollback_depth + 1,
                local_height,
                local_height.saturating_sub(1),
                gap,
                empty_headers,
                self.shallow_rollback_count
            );
        }

        if local_height == 0 {
            return Ok(false);
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
            return Ok(false);
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
            return Ok(false);
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
                    return Ok(false);
                }
            }
        };

        info!(
            "Rolling back from height {} to {} for fork recovery",
            local_height, target_height
        );

        // Try undo-based rollback first (O(1) for single block)
        if let Some(undo) = self.state_db.get_undo(local_height) {
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

            // Restore ProducerSet from undo snapshot
            if let Ok(restored_producers) =
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

            // Pre-check: can't rebuild from genesis without block 1.
            // Do NOT snap sync — it destroys the block store further.
            // Skip the rollback and let header-first sync recover.
            if self.block_store.get_block_by_height(1)?.is_none() {
                warn!("Rollback: block 1 missing — cannot rebuild. Skipping rollback, header-first sync will recover.");
                return Ok(true);
            }

            let genesis_producers = if genesis_blocks > 0 && target_height > genesis_blocks {
                self.derive_genesis_producers_from_chain()
            } else {
                Vec::new()
            };
            let bond_unit = self.config.network.bond_unit();
            {
                let mut utxo = self.utxo_set.write().await;
                utxo.clear();
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

        // Restore epoch scheduler state from undo data (O(1) vs O(chain) rebuild).
        // The undo snapshot was taken BEFORE apply_block, so it reflects the correct
        // scheduler state at the pre-rollback height.
        if let Some(undo) = self.state_db.get_undo(local_height) {
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
                        self.rebuild_epoch_state_from_blocks().await;
                    }
                }
            } else {
                // Pre-upgrade undo data (no epoch_state_snapshot) — fall back to rebuild
                info!("[ROLLBACK] No epoch state in undo (pre-upgrade block) — rebuilding");
                self.rebuild_epoch_state_from_blocks().await;
            }
        } else {
            // No undo data at all (legacy path already handled above) — rebuild
            self.rebuild_epoch_state_from_blocks().await;
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

        Ok(true)
    }
}
