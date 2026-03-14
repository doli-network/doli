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

            // Pre-check: snap-synced nodes can't rebuild from genesis — re-snap to recover
            if self.block_store.get_block_by_height(1)?.is_none() {
                warn!("Rollback: snap-synced node with no undo data. Re-snapping to recover.");
                self.reset_state_only().await?;
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
        let (empty_headers, local_height, fork_sync_active, gap) = {
            let sync = self.sync_manager.read().await;
            let gap = sync.network_tip_height().saturating_sub(sync.local_tip().0);
            (
                sync.consecutive_empty_headers(),
                sync.local_tip().0,
                sync.is_fork_sync_active(),
                gap,
            )
        };

        // Don't start a new fork sync if one is already running
        if fork_sync_active {
            return Ok(false);
        }

        // Need at least 3 fork evidence signals before activating
        if empty_headers < 3 || local_height == 0 {
            return Ok(false);
        }

        // Fast path: small gap (<= 10 blocks) — just rollback 1 block.
        // This changes local_hash to the parent, and the next sync attempt
        // will try GetHeaders with the parent hash. After 1-3 rollbacks,
        // we find a hash peers recognize and sync resumes.
        // No binary search needed, no grace check — immediate recovery.
        if gap <= 10 && self.shallow_rollback_count < 10 {
            info!(
                "Shallow fork (gap={}, empty_headers={}): rolling back 1 block \
                 from h={} to find common ancestor (rollback #{})",
                gap, empty_headers, local_height, self.shallow_rollback_count + 1
            );
            let rolled_back = self.rollback_one_block().await?;
            if rolled_back {
                self.shallow_rollback_count += 1;
                // Reset empty headers so sync retries with new tip hash
                let mut sync = self.sync_manager.write().await;
                sync.reset_empty_headers();
                return Ok(true);
            }
        }

        // Deep fork or rollback limit reached: use binary search
        let started = self.sync_manager.write().await.start_fork_sync();
        if started {
            info!(
                "Fork sync: binary search for common ancestor initiated \
                 (empty_headers={}, local_height={}, gap={})",
                empty_headers, local_height, gap
            );
            return Ok(true);
        }
        Ok(false)
    }
}
