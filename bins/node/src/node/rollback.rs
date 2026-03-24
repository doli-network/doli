use super::*;

impl Node {
    /// Unconditionally roll back 1 block for fork recovery.
    ///
    /// Unlike `resolve_shallow_fork()`, this has no `empty_headers` or
    /// `shallow_rollback_count` preconditions — it just rolls back. Used by
    /// `maybe_auto_resync()` on mainnet where same-height forks never trigger
    /// empty header responses (peers are at the same height, not ahead).
    ///
    /// Returns `Ok(true)` if rollback succeeded, `Ok(false)` if at height 0.
    pub(super) async fn rollback_one_block(&mut self) -> Result<bool> {
        let local_height = {
            let sync = self.sync_manager.read().await;
            sync.local_tip().0
        };

        if local_height == 0 {
            return Ok(false);
        }

        let target_height = local_height - 1;

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
                            let _ = utxo.spend_transaction(tx);
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

        // Force scheduler rebuild after producer set reconstruction
        self.cached_scheduler = None;

        // Rebuild producer liveness map from canonical block_store.
        // Critical: rollback does NOT undo liveness entries from fork blocks,
        // causing nodes to have divergent live_producers lists and conflicting
        // round-robin assignments. Rebuilding from block_store ensures all nodes
        // converge on the same liveness view.
        self.rebuild_producer_liveness(target_height);

        // Rebuild reactive scheduling flags from canonical block_store (INC-I-006)
        {
            let mut producers = self.producer_set.write().await;
            self.rebuild_scheduled_from_blocks(&mut producers, target_height);
        }

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
        }

        // Atomically persist the rolled-back state via StateDb.
        // Collects all UTXOs from in-memory set and writes everything in one WriteBatch.
        {
            let state = self.chain_state.read().await;
            let producers = self.producer_set.read().await;
            let utxo = self.utxo_set.read().await;
            let utxo_pairs: Vec<(storage::Outpoint, storage::UtxoEntry)> = match &*utxo {
                UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
                // RocksDb variant shouldn't occur (we load InMemory from StateDb), but handle it
                UtxoSet::RocksDb(_) => self.state_db.iter_utxos(),
            };
            self.state_db
                .atomic_replace(&state, &producers, utxo_pairs.into_iter())
                .map_err(|e| anyhow::anyhow!("StateDb atomic_replace failed: {}", e))?;
        }

        info!(
            "Fork recovery rollback complete: now at height {} (hash {:.8})",
            target_height, parent_hash
        );

        Ok(true)
    }

    /// Detect and resolve shallow forks (1-block orphan tips from ungraceful shutdown).
    ///
    /// When a node is killed during block production, it may persist a block that
    /// peers never received. On restart, peers don't recognize our tip hash, causing
    /// consecutive empty header responses. If the gap to peers is small (<100 blocks),
    /// this is a shallow fork — not a deep fork needing genesis resync.
    ///
    /// Fix: roll back one block at a time until we reach a tip peers recognize.
    /// Capped at 10 rollbacks — if the fork is deeper than that, it's not shallow.
    /// Returns `true` if a rollback was performed (caller should skip other periodic tasks).
    pub(super) async fn resolve_shallow_fork(&mut self) -> Result<bool> {
        let (empty_headers, local_height, gap, stuck_signal) = {
            let mut sync = self.sync_manager.write().await;
            let gap = sync.network_tip_height().saturating_sub(sync.local_tip().0);
            let stuck = sync.take_stuck_fork_signal();
            (
                sync.consecutive_empty_headers(),
                sync.local_tip().0,
                gap,
                stuck,
            )
        };

        // Need at least 3 fork evidence signals OR a stuck_fork_signal before activating.
        // The stuck_fork_signal is set by cleanup() and block_apply_failed() as a
        // dedicated signal that doesn't interfere with the counter's natural progression.
        let has_fork_evidence = empty_headers >= 3 || stuck_signal;
        if !has_fork_evidence || local_height == 0 {
            return Ok(false);
        }

        // Death spiral prevention: if we've rolled back more than 10 blocks
        // from our peak height, stop rolling back. The progressive rollback is
        // clearly not finding a common ancestor — force a clean resync instead.
        // Without this cap, the node degrades all the way to genesis, accepting
        // fork blocks from bad peers along the way (INC-I-005).
        const MAX_SAFE_ROLLBACK: u64 = 10;
        let peak_height = {
            let sync = self.sync_manager.read().await;
            sync.peak_height()
        };
        if peak_height > 0 && peak_height.saturating_sub(local_height) > MAX_SAFE_ROLLBACK {
            warn!(
                "Rollback death spiral: rolled back {} blocks from peak h={} to h={}. \
                 Stopping rollback and triggering clean resync.",
                peak_height - local_height,
                peak_height,
                local_height
            );
            let mut sync = self.sync_manager.write().await;
            sync.request_genesis_resync(RecoveryReason::RollbackDeathSpiral {
                peak: peak_height,
                current: local_height,
            });
            return Ok(true);
        }

        // Delegate fork recovery decision to SyncManager's centralized logic.
        // ForkState::recommend_action() evaluates gap, rollback count, and peer
        // availability to decide the best recovery strategy.
        const MAX_ROLLBACK_DEPTH: u32 = 12;
        let action = {
            let sync = self.sync_manager.read().await;
            let best_peer = sync.best_peer_for_recovery();
            sync.recommend_fork_action(
                gap,
                self.shallow_rollback_count,
                MAX_ROLLBACK_DEPTH,
                best_peer,
            )
        };

        match action {
            ForkAction::NeedsGenesisResync => {
                warn!(
                    "Fork recovery: recommend_action returned NeedsGenesisResync \
                     (empty_headers={}, gap={}, h={})",
                    empty_headers, gap, local_height
                );
                let mut sync = self.sync_manager.write().await;
                sync.request_genesis_resync(RecoveryReason::GenesisFallbackEmptyHeaders);
                Ok(true)
            }
            ForkAction::RollbackOne => {
                info!(
                    "Shallow fork (gap={}, empty_headers={}): rolling back 1 block \
                     from h={} to find common ancestor (rollback #{})",
                    gap,
                    empty_headers,
                    local_height,
                    self.shallow_rollback_count + 1
                );
                let rolled_back = self.rollback_one_block().await?;
                if rolled_back {
                    self.shallow_rollback_count += 1;
                    // Reset empty headers so sync retries with new tip hash
                    let mut sync = self.sync_manager.write().await;
                    sync.reset_empty_headers();
                    return Ok(true);
                }
                Ok(false)
            }
            ForkAction::None => Ok(false),
        }
    }
}
