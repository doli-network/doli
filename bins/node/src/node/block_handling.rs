use super::*;

impl Node {
    /// Handle a new block from the network
    pub async fn handle_new_block(
        &mut self,
        block: Block,
        source_peer: PeerId,
    ) -> Result<()> {
        let block_hash = block.hash();

        // Check if we already have this block
        if self.block_store.get_block(&block_hash)?.is_some() {
            debug!("Block {} already known", block_hash);
            return Ok(());
        }

        // Check for equivocation (double signing) - even for forks
        // This is critical for slashing misbehaving producers
        let equivocation_proof = { self.equivocation_detector.write().await.check_block(&block) };
        if let Some(proof) = equivocation_proof {
            // Equivocation detected! Create and submit slash transaction
            self.handle_equivocation(proof).await;
        }

        // Check if block builds on our chain
        let state = self.chain_state.read().await;
        if block.header.prev_hash != state.best_hash {
            // Might be a reorg or we're out of sync
            let current_tip = state.best_hash;
            let current_height = state.best_height;
            drop(state);

            // Cache this block for potential reorg
            {
                let mut cache = self.fork_block_cache.write().await;
                cache.insert(block_hash, block.clone());
                // Keep cache size reasonable (last 100 fork blocks)
                // Evict oldest blocks by slot to keep recent fork candidates
                if cache.len() > 100 {
                    let mut blocks_by_slot: Vec<(Hash, u32)> =
                        cache.iter().map(|(h, b)| (*h, b.header.slot)).collect();
                    blocks_by_slot.sort_by_key(|(_, slot)| *slot);
                    // Remove oldest 50 blocks
                    for (hash, _) in blocks_by_slot.into_iter().take(50) {
                        cache.remove(&hash);
                    }
                }
            }

            debug!(
                "Block {} doesn't build on tip {} (builds on {}), cached for potential reorg",
                block_hash, current_tip, block.header.prev_hash
            );

            // Check for reorg using weight-based fork choice rule
            // Get the producer's effective weight to compare chain weights
            let producer_weight = {
                let producers = self.producer_set.read().await;
                producers
                    .get_by_pubkey(&block.header.producer)
                    .map(|p| p.effective_weight(current_height + 1))
                    .unwrap_or(1) // Default weight 1 for unknown producers (bootstrap)
            };

            let reorg_result = {
                self.sync_manager
                    .write()
                    .await
                    .handle_new_block_weighted(block.clone(), producer_weight)
            };

            if let Some(reorg_result) = reorg_result {
                // Weight-based fork choice: the new chain is heavier
                info!(
                    "Reorg to heavier chain: rolling back {} blocks, applying {} new blocks, weight_delta=+{}",
                    reorg_result.rollback.len(),
                    reorg_result.new_blocks.len(),
                    reorg_result.weight_delta
                );

                // Execute the reorg
                if let Err(e) = self.execute_reorg(reorg_result, block).await {
                    error!("Failed to execute reorg: {}", e);
                }
            } else {
                // Reorg detection failed (parent not in our recent blocks).
                // Try active fork recovery: walk backward through parent chain
                // from this orphan block until we connect to our chain.
                // Use source_peer (who gossiped this block) — they have the fork chain.
                let can_start = self.sync_manager.read().await.can_start_fork_recovery();
                if can_start {
                    let started = self
                        .sync_manager
                        .write()
                        .await
                        .start_fork_recovery(block.clone(), source_peer);
                    if started {
                        info!(
                            "Fork recovery started: walking parents from block {} (asking source peer {})",
                            block_hash, source_peer
                        );
                        return Ok(());
                    }
                }

                // Fallback: Check if we're likely on a fork by looking at RECENT orphan blocks.
                // Only blocks near our current slot count as fork evidence.
                // Old blocks from syncing peers are NOT fork evidence.
                let our_slot = self.chain_state.read().await.best_slot;
                let our_height = self.chain_state.read().await.best_height;
                let cache_size = {
                    let cache = self.fork_block_cache.read().await;
                    let slot_window = 30u32; // only count blocks within last 30 slots (~5 min)
                    let min_slot = our_slot.saturating_sub(slot_window);
                    cache.values().filter(|b| b.header.slot >= min_slot).count()
                };

                // Many orphan blocks indicate we're on a minority fork.
                // Stale chain detector + fork recovery handle this — no genesis resync.
                if cache_size >= 10 && cache_size % 10 == 0 {
                    warn!(
                        "Fork detected: {} orphan blocks don't build on our chain (height {}). Relying on fork recovery + stale chain sync.",
                        cache_size, our_height
                    );
                }

                // Try fork recovery for the orphan block
                self.try_trigger_fork_recovery().await;

                if cache_size >= 2 {
                    // Try to chain the blocks from cache
                    debug!(
                        "Attempting to apply cached chain: {} blocks in cache",
                        cache_size
                    );

                    if let Err(e) = self.try_apply_cached_chain(block).await {
                        debug!("Could not apply cached chain: {}", e);
                    }
                }
            }
            return Ok(());
        }
        drop(state);

        // REMOVED: Pre-apply gossip eligibility check.
        // This check used LOCAL chain state to validate gossip blocks. When the
        // receiving node was on a micro-fork (different tip), it computed different
        // eligibility and rejected valid canonical blocks — causing nodes to fall
        // behind and need expensive sync recovery.
        //
        // Full validation happens in apply_block() below, which correctly validates
        // against the chain state the block actually builds on. Letting apply_block
        // handle validation is both correct and sufficient.

        // Apply the block — absorb errors so an invalid gossip block
        // (e.g. from a forked peer) doesn't crash the process.
        let height = self.chain_state.read().await.best_height + 1;
        let block_slot = block.header.slot;
        let block_producer = block.header.producer;
        if let Err(e) = self.apply_block(block, ValidationMode::Full).await {
            warn!(
                "[BLOCK] REJECT slot={} h={} producer={} error={} — skipping, sync will catch up",
                block_slot,
                height,
                hex::encode(&block_producer.as_bytes()[..4]),
                e,
            );
            return Ok(());
        }

        // A canonical gossip block was applied on our tip — clear the post-snap gate.
        // This proves we're on the canonical chain and our block store has a real parent.
        self.sync_manager
            .write()
            .await
            .clear_awaiting_canonical_block();

        Ok(())
    }

    /// Execute a chain reorganization
    ///
    /// This function is atomic: either the full reorg succeeds, or the chain
    /// remains unchanged. We build new state in temporary structures and only
    /// swap them in on success.
    pub async fn execute_reorg(
        &mut self,
        reorg_result: ReorgResult,
        triggering_block: Block,
    ) -> Result<()> {
        let rollback_count = reorg_result.rollback.len();
        let new_block_count = reorg_result.new_blocks.len();

        info!(
            "Executing reorg: rolling back {} blocks, applying {} new blocks",
            rollback_count, new_block_count
        );

        // Collect all new blocks we need to apply
        let mut new_blocks: Vec<Block> = Vec::new();

        {
            let cache = self.fork_block_cache.read().await;
            for block_hash in &reorg_result.new_blocks {
                if *block_hash == triggering_block.hash() {
                    new_blocks.push(triggering_block.clone());
                } else if let Some(cached_block) = cache.get(block_hash) {
                    new_blocks.push(cached_block.clone());
                } else if let Ok(Some(stored_block)) = self.block_store.get_block(block_hash) {
                    debug!(
                        "Reorg block {} found in block_store (not in fork cache)",
                        block_hash
                    );
                    new_blocks.push(stored_block);
                } else {
                    warn!(
                        "Cannot execute reorg: missing block {} (need to sync from peers)",
                        block_hash
                    );
                    return Ok(());
                }
            }
        }

        // Sort new blocks by slot number (provides a total order)
        new_blocks.sort_by_key(|b| b.header.slot);

        // Validate the chain: first block must build on common ancestor,
        // and each subsequent block must build on the previous.
        if let Some(first) = new_blocks.first() {
            if first.header.prev_hash != reorg_result.common_ancestor {
                error!(
                    "Reorg chain is broken: first block {} prev_hash={} doesn't match \
                     common ancestor {}. Aborting reorg to prevent height offset.",
                    first.hash(),
                    first.header.prev_hash,
                    reorg_result.common_ancestor
                );
                return Ok(());
            }
        }
        for i in 1..new_blocks.len() {
            if new_blocks[i].header.prev_hash != new_blocks[i - 1].hash() {
                error!(
                    "Reorg chain is broken: block {} doesn't build on {}",
                    new_blocks[i].hash(),
                    new_blocks[i - 1].hash()
                );
                return Ok(());
            }
        }

        // Get current state
        let current_height = self.chain_state.read().await.best_height;
        let target_height = current_height - rollback_count as u64;

        // No-op reorg: rollback_count=0 means we're already at the common ancestor.
        // Skip the rollback path entirely — there's nothing to undo, and calling
        // get_undo(target_height + 1) would panic because that undo doesn't exist.
        if rollback_count > 0 {
            // Invalidate genesis producer cache if reorg crosses genesis boundary
            let genesis_blocks = self.config.network.genesis_blocks();
            if genesis_blocks > 0 && target_height <= genesis_blocks {
                info!("[REORG] Crossing genesis boundary — invalidating genesis producer cache");
                self.cached_genesis_producers = std::sync::OnceLock::new();
            }

            info!(
                "Rolling back from height {} to {} (common ancestor)",
                current_height, target_height
            );

            // Find the common ancestor block
            let common_ancestor_block = if target_height == 0 {
                None
            } else {
                self.block_store.get_block_by_height(target_height)?
            };

            let genesis_hash = self.chain_state.read().await.genesis_hash;
            let common_ancestor_hash = common_ancestor_block
                .as_ref()
                .map(|b| b.hash())
                .unwrap_or(genesis_hash);
            let common_ancestor_slot = common_ancestor_block
                .as_ref()
                .map(|b| b.header.slot)
                .unwrap_or(0);

            // Undo-based rollback: apply undo data in reverse from current_height to target_height+1.
            // This is O(rollback_depth) instead of O(chain_height).
            // Fallback: if undo data is missing (pre-undo blocks), use legacy rebuild.
            let has_undo =
                (target_height + 1..=current_height).all(|h| self.state_db.get_undo(h).is_some());

            if has_undo {
                info!(
                    "Undo-based rollback: reverting {} blocks ({} → {})",
                    rollback_count, current_height, target_height
                );

                {
                    let mut utxo = self.utxo_set.write().await;

                    // Apply undo data in reverse order (highest block first)
                    for h in (target_height + 1..=current_height).rev() {
                        let undo = self.state_db.get_undo(h).unwrap();

                        // Remove UTXOs created by this block
                        for outpoint in &undo.created_utxos {
                            utxo.remove(outpoint)?;
                        }

                        // Restore UTXOs spent by this block
                        for (outpoint, entry) in &undo.spent_utxos {
                            utxo.insert(*outpoint, entry.clone())?;
                        }
                    }

                    // Restore ProducerSet from the undo snapshot at target_height + 1
                    // (which captured the state BEFORE that block was applied = state AT target_height)
                    let first_undo = self.state_db.get_undo(target_height + 1).unwrap();
                    if let Ok(restored_producers) =
                        bincode::deserialize::<storage::ProducerSet>(&first_undo.producer_snapshot)
                    {
                        let mut producers = self.producer_set.write().await;
                        *producers = restored_producers;
                    } else {
                        warn!("Failed to deserialize producer snapshot from undo data, rebuilding from blocks");
                        let mut producers = self.producer_set.write().await;
                        self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
                    }
                }

                // Update chain state
                {
                    let mut state = self.chain_state.write().await;
                    state.best_height = target_height;
                    state.best_hash = common_ancestor_hash;
                    state.best_slot = common_ancestor_slot;
                }
            } else {
                // Legacy fallback: rebuild from genesis (no undo data available)
                warn!(
                "Undo data missing for rollback range {}..={} — falling back to rebuild from genesis",
                target_height + 1,
                current_height
            );

                let genesis_blocks = self.config.network.genesis_blocks();
                let genesis_producers = if genesis_blocks > 0 && target_height > genesis_blocks {
                    self.derive_genesis_producers_from_chain()
                } else {
                    Vec::new()
                };
                let bond_unit = self.config.network.bond_unit();

                {
                    let mut state = self.chain_state.write().await;
                    let mut utxo = self.utxo_set.write().await;
                    state.best_height = target_height;
                    state.best_hash = common_ancestor_hash;
                    state.best_slot = common_ancestor_slot;
                    utxo.clear();
                    for height in 1..=target_height {
                        if let Some(block) =
                            self.block_store.get_block_by_height(height).ok().flatten()
                        {
                            for (tx_index, tx) in block.transactions.iter().enumerate() {
                                let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                                if !is_reward_tx {
                                    let _ = utxo.spend_transaction(tx);
                                }
                                utxo.add_transaction(tx, height, is_reward_tx, block.header.slot)?;
                            }
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

            // Atomically persist common ancestor state to StateDb
            {
                let state = self.chain_state.read().await;
                let utxo = self.utxo_set.read().await;
                let producers = self.producer_set.read().await;
                let utxo_pairs: Vec<_> = match &*utxo {
                    UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
                    UtxoSet::RocksDb(_) => self.state_db.iter_utxos(),
                };
                self.state_db
                    .atomic_replace(&state, &producers, utxo_pairs.into_iter())
                    .map_err(|e| anyhow::anyhow!("Reorg StateDb atomic_replace failed: {}", e))?;
            }
        } // end if rollback_count > 0

        // Now apply the new blocks through normal path
        // Note: we skip check_producer_eligibility here because the fork blocks were
        // validated when originally produced, and re-validating against rolled-back
        // state uses the wrong producer set (common ancestor, not fork chain).
        info!("Applying {} new blocks from fork", new_blocks.len());
        let pre_reorg_height = current_height;
        for (i, block) in new_blocks.into_iter().enumerate() {
            if let Err(e) = self.apply_block(block, ValidationMode::Light).await {
                let post_height = self.chain_state.read().await.best_height;
                error!(
                    "Reorg apply_block failed at block {}: {} — rolled back from {} to {}, \
                     only applied {}/{} blocks. State is at height {}.",
                    i + 1,
                    e,
                    pre_reorg_height,
                    target_height,
                    i,
                    new_block_count,
                    post_height
                );
                // CRITICAL: If we rolled back significantly but applied very few blocks,
                // this was a bad reorg (peer had a different/invalid chain). Log the
                // damage so the operator knows what happened. Header-first sync will
                // recover from post_height, but the height loss is real.
                if pre_reorg_height > post_height + 10 {
                    error!(
                        "CATASTROPHIC REORG: lost {} blocks ({} → {}). \
                         The fork sync peer had an incompatible chain. \
                         Header-first sync will recover but this should not happen.",
                        pre_reorg_height - post_height,
                        pre_reorg_height,
                        post_height
                    );
                }
                // State is consistent (common ancestor + whatever blocks succeeded).
                // Don't propagate error — let normal sync fill the gap.
                return Ok(());
            }
        }

        // Clear applied blocks from fork cache
        {
            let mut cache = self.fork_block_cache.write().await;
            for hash in &reorg_result.new_blocks {
                cache.remove(hash);
            }
        }

        // Invalidate mempool - transactions may now be invalid
        {
            let mut mempool = self.mempool.write().await;
            let utxo = self.utxo_set.read().await;
            let height = self.chain_state.read().await.best_height;
            mempool.revalidate(&utxo, height);
        }

        info!(
            "Reorg complete: now at height {}",
            self.chain_state.read().await.best_height
        );

        Ok(())
    }

    /// Execute a fork sync reorg: the binary search found the common ancestor,
    /// canonical blocks have been downloaded. Build a ReorgResult and call execute_reorg.
    pub async fn execute_fork_sync_reorg(
        &mut self,
        result: network::sync::ForkSyncResult,
    ) -> Result<()> {
        let current_height = self.chain_state.read().await.best_height;
        let rollback_depth = current_height.saturating_sub(result.ancestor_height);

        // Safety limit: reject forks deeper than MAX_SAFE_REORG_DEPTH blocks.
        // Undo data is kept for UNDO_KEEP_DEPTH (2000) blocks, so reorgs up to 500
        // are safe with a 4x margin. Previous limit of 10 caused infinite loops when
        // fork_sync found valid ancestors after snap sync gaps.
        const MAX_SAFE_REORG_DEPTH: u64 = 500;
        if rollback_depth > MAX_SAFE_REORG_DEPTH {
            warn!(
                "Fork sync: reorg depth {} exceeds safety limit ({} blocks). \
                 Refusing automatic reorg (ancestor h={}, current h={}). \
                 Investigate manually — this is likely a snap sync gap or isolation issue, \
                 not a legitimate fork.",
                rollback_depth, MAX_SAFE_REORG_DEPTH, result.ancestor_height, current_height
            );
            {
                let mut sync = self.sync_manager.write().await;
                sync.reset_sync_for_rollback();
            }
            // Do NOT auto-recover. The operator must investigate.
            return Ok(());
        }

        info!(
            "Fork sync reorg: rolling back {} blocks (height {} → {}), \
             applying {} canonical blocks",
            rollback_depth,
            current_height,
            result.ancestor_height,
            result.canonical_blocks.len()
        );

        if result.canonical_blocks.is_empty() {
            warn!("Fork sync: no canonical blocks to apply — aborting reorg");
            let mut sync = self.sync_manager.write().await;
            sync.reset_sync_for_rollback();
            return Ok(());
        }

        // Guard: don't reorg to a shorter chain — this causes cascading height loss
        let new_chain_height = result.ancestor_height + result.canonical_blocks.len() as u64;
        if new_chain_height < current_height {
            warn!(
                "Fork sync: new chain ({}) shorter than current ({}) — aborting reorg",
                new_chain_height, current_height
            );
            let mut sync = self.sync_manager.write().await;
            sync.mark_fork_sync_rejected();
            sync.reset_sync_for_rollback();
            return Ok(());
        }

        // Guard: reject reorgs where the new chain is marginally longer but the
        // rollback depth is catastrophically deep. A peer offering 440 blocks when
        // we need to rollback 437 is NOT a legitimate fork — it's a different chain.
        //
        // Exception 1: if the new chain is significantly longer (>20% more blocks),
        // the deep rollback IS legitimate — minority fork switching to canonical.
        // Exception 2: during initial sync (height < 200), deep rollbacks are
        // normal — a node may have produced its own Block 1 during bootstrap and
        // needs to switch to the canonical chain. Blocking this causes nodes to
        // get stuck at low heights forever.
        if rollback_depth > 0 && current_height > 200 {
            let rollback_ratio = rollback_depth * 100 / current_height;
            let length_gain = new_chain_height.saturating_sub(current_height);
            let gain_ratio = if current_height > 0 {
                length_gain * 100 / current_height
            } else {
                100
            };
            if rollback_ratio > 50 && gain_ratio < 20 {
                warn!(
                    "Fork sync: rollback depth {} is {}% of chain height {} — too deep for a \
                     legitimate fork (ancestor h={}, gain={}%). Rejecting.",
                    rollback_depth,
                    rollback_ratio,
                    current_height,
                    result.ancestor_height,
                    gain_ratio
                );
                let mut sync = self.sync_manager.write().await;
                sync.mark_fork_sync_rejected();
                if let Some(peer) = sync.best_peer_for_recovery() {
                    sync.blacklist_peer_for_fork_sync(peer);
                }
                sync.reset_sync_for_rollback();
                return Ok(());
            }
            if rollback_ratio > 50 {
                info!(
                    "Fork sync: allowing deep rollback ({}% of h={}) because new chain is {}% longer ({} vs {})",
                    rollback_ratio, current_height, gain_ratio, new_chain_height, current_height
                );
            }
        }

        // Pre-check: verify rollback stays within blocks that have undo data.
        // Snap-synced nodes don't have blocks before the snap anchor, but DO have
        // undo data for blocks applied after snap sync. Allow reorg if the rollback
        // only touches blocks with undo data (post-snap). Block only if rollback
        // would need to rebuild from genesis (below store floor).
        let store_floor = self.sync_manager.read().await.store_floor();
        let rollback_depth = current_height.saturating_sub(result.ancestor_height);
        if result.ancestor_height < store_floor && store_floor > 0 {
            warn!(
                "Fork sync reorg: ancestor h={} below store floor h={} (rollback depth={}). \
                 Skipping — undo-based rollback can't reach that far.",
                result.ancestor_height, store_floor, rollback_depth
            );
            let mut sync = self.sync_manager.write().await;
            sync.mark_fork_sync_rejected();
            sync.reset_sync_for_rollback();
            return Ok(());
        }

        // Build rollback list: our blocks from current_height down to ancestor+1
        let mut rollback = Vec::new();
        for height in (result.ancestor_height + 1..=current_height).rev() {
            if let Some(hash) = self.block_store.get_hash_by_height(height)? {
                rollback.push(hash);
            }
        }

        // Compute weight of new canonical blocks vs rolled-back blocks
        let new_chain_weight: i64 = {
            let producers = self.producer_set.read().await;
            result
                .canonical_blocks
                .iter()
                .map(|b| {
                    producers
                        .get_by_pubkey(&b.header.producer)
                        .map(|p| p.effective_weight(current_height) as i64)
                        .unwrap_or(1)
                })
                .sum()
        };
        let old_chain_weight: i64 = {
            let producers = self.producer_set.read().await;
            let mut weight = 0i64;
            for height in (result.ancestor_height + 1)..=current_height {
                if let Some(block) = self.block_store.get_block_by_height(height)? {
                    weight += producers
                        .get_by_pubkey(&block.header.producer)
                        .map(|p| p.effective_weight(current_height) as i64)
                        .unwrap_or(1);
                }
            }
            weight
        };
        let weight_delta = new_chain_weight - old_chain_weight;

        if weight_delta <= 0 {
            info!(
                "Fork sync: new chain not heavier (delta={}, new={}, old={}) — keeping current",
                weight_delta, new_chain_weight, old_chain_weight
            );
            let mut sync = self.sync_manager.write().await;
            // Record rejection time to activate reorg cooldown.
            // This prevents the infinite loop: reject → post_rollback → fork_sync → reject.
            sync.mark_fork_sync_rejected();
            sync.reset_sync_for_rollback();
            return Ok(());
        }

        // Insert canonical blocks into fork_block_cache so execute_reorg can find them
        let new_block_hashes: Vec<crypto::Hash> =
            result.canonical_blocks.iter().map(|b| b.hash()).collect();
        {
            let mut cache = self.fork_block_cache.write().await;
            for block in &result.canonical_blocks {
                cache.insert(block.hash(), block.clone());
            }
        }

        // Use the last canonical block as the triggering block
        let trigger_block = result.canonical_blocks.last().unwrap().clone();

        let reorg_result = network::sync::ReorgResult {
            rollback,
            common_ancestor: result.ancestor_hash,
            new_blocks: new_block_hashes,
            weight_delta,
        };

        // Execute the reorg using the existing atomic reorg path.
        // IMPORTANT: Always reset sync state afterward, even on failure, to prevent
        // infinite fork sync retry loops.
        match self.execute_reorg(reorg_result, trigger_block).await {
            Ok(()) => {
                // Reset sync state so normal sync can resume from the new tip.
                {
                    let mut sync = self.sync_manager.write().await;
                    sync.reset_sync_for_rollback();
                    // Recovery was successful — tip is now canonical. Clear post_rollback
                    // to prevent unnecessary fork_sync loop (N5 incident 2026-03-23).
                    sync.set_post_rollback(false);
                    let (height, hash, slot) = {
                        let state = self.chain_state.read().await;
                        (state.best_height, state.best_hash, state.best_slot)
                    };
                    sync.update_local_tip(height, hash, slot);
                }

                self.shallow_rollback_count = 0;
                self.consecutive_fork_blocks = 0;
                self.cumulative_rollback_depth = 0;

                info!(
                    "Fork sync reorg complete: now at height {}",
                    self.chain_state.read().await.best_height
                );

                Ok(())
            }
            Err(e) => {
                // Always reset sync state to prevent infinite retry loop
                {
                    let mut sync = self.sync_manager.write().await;
                    sync.reset_sync_for_rollback();
                }

                let err_msg = e.to_string();
                if err_msg.contains("missing block") || err_msg.contains("UTXO rebuild") {
                    if self.block_store.get_block_by_height(1)?.is_none() {
                        warn!(
                            "Fork sync reorg failed due to block store gap: {}. \
                             Skipping — header-first sync will recover.",
                            e
                        );
                        // Do NOT snap sync. Let header-first sync catch up.
                    } else {
                        error!(
                            "Reorg failed with block store intact — possible corruption: {}. \
                             Sync will attempt recovery via header-first download.",
                            e
                        );
                    }
                    return Ok(());
                }

                Err(e)
            }
        }
    }
}
