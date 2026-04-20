use super::*;

impl Node {
    /// Handle a new block from the network
    pub async fn handle_new_block(&mut self, block: Block, source_peer: PeerId) -> Result<()> {
        let block_hash = block.hash();
        let block_slot = block.header.slot;

        // Gossip latency instrumentation (v6.13.22): correlate by slot with
        // [GOSSIP_RECV] and [APPLY_END] lines. See on_new_block_event.
        let apply_start = Instant::now();
        info!("[APPLY_START] slot={} hash={}", block_slot, block_hash);

        // Check if we already have this block
        if self.block_store.get_block(&block_hash)?.is_some() {
            debug!("Block {} already known", block_hash);
            info!(
                "[APPLY_END] slot={} apply_ms={} status=already_known",
                block_slot,
                apply_start.elapsed().as_millis()
            );
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

            // Reject blocks from a different chain (different genesis hash).
            // Without this, zombie nodes on old chains contaminate our block
            // store via fork recovery, causing "common ancestor not found".
            let our_genesis = self.chain_state.read().await.genesis_hash;
            if block.header.genesis_hash != our_genesis {
                debug!(
                    "Dropping block {} from different chain (genesis {} != {})",
                    block_hash,
                    block.header.genesis_hash.to_hex()[..16].to_string(),
                    our_genesis.to_hex()[..16].to_string(),
                );
                return Ok(());
            }

            // Fork identity — reject blocks from nodes with different active hard forks.
            // Same level of filtering as genesis_hash: O(1), pre-validation drop.
            let fork_id_activation = self.config.network.params().fork_id_activation_height;
            if current_height >= fork_id_activation {
                let our_fork_id = self.current_fork_id();
                if block.header.fork_id != our_fork_id {
                    debug!(
                        "[FORK_ID] Dropping block {} at h={} — fork_id {} != {}",
                        block_hash,
                        current_height,
                        &block.header.fork_id.to_hex()[..16],
                        &our_fork_id.to_hex()[..16],
                    );
                    return Ok(());
                }
            }

            // Height-occupied guard: discard blocks that don't extend our tip
            // if we already have canonical chain at or above their height.
            //
            // A fork block's parent exists in our store (shared history) but
            // we already have a different canonical block at parent_height+1.
            // Letting it into the fork cache triggers fork recovery → rollback
            // cascade → block store gaps → stuck nodes.
            //
            // Legitimate reorg comes through header-first sync, not gossip.
            // O(1): one get_block_height + one get_block_by_height lookup.
            if let Ok(Some(parent_height)) =
                self.block_store.get_height_by_hash(&block.header.prev_hash)
            {
                let fork_block_height = parent_height + 1;
                if let Ok(Some(canonical)) = self.block_store.get_block_by_height(fork_block_height)
                {
                    if canonical.hash() != block_hash {
                        // Fork choice: lower slot wins (deterministic tiebreak).
                        // If the new block was produced in a strictly lower slot,
                        // it has priority — rollback the fork block and apply canonical.
                        // Direct execution: the weight-based reorg path can't handle
                        // this case because the canonical block's prev_hash doesn't
                        // connect to the fork block's hash (different block at same height).
                        if block.header.slot < canonical.header.slot {
                            info!(
                                "[FORK_CHOICE] Reorg at h={}: new slot {} < fork slot {} — rollback + apply",
                                fork_block_height,
                                block.header.slot,
                                canonical.header.slot
                            );
                            // Save the existing block before rollback (for restore on failure)
                            let existing_block = canonical.clone();
                            self.rollback_one_block().await?;
                            let mode = if self.snap_sync_height.is_some() {
                                ValidationMode::Light
                            } else {
                                ValidationMode::Full
                            };
                            if let Err(e) = self.apply_block(block, mode).await {
                                // Transactional: restore original block — all or nothing
                                warn!(
                                    "[FORK_CHOICE] Apply failed: {} — restoring original block",
                                    e
                                );
                                let _ = self
                                    .apply_block(existing_block, ValidationMode::Light)
                                    .await;
                            }
                            return Ok(());
                        } else {
                            info!(
                                "[FORK_GUARD] Dropping fork block {} at h={} slot {} — canonical slot {} wins",
                                &block_hash.to_hex()[..16],
                                fork_block_height,
                                block.header.slot,
                                canonical.header.slot
                            );
                            self.sync_manager
                                .write()
                                .await
                                .note_orphan_gossip_block(fork_block_height, block.header.slot);
                            return Ok(());
                        }
                    }
                }
            } else {
                // Parent not in store — orphan gossip block.
                //
                // Two cases:
                // A) Normal orphan: we're missing h+1 (gossip out of order).
                //    Request h+1 from peer → apply when it arrives.
                // B) Fork orphan: we have a DIFFERENT block at h (fork block).
                //    The orphan's prev_hash points to the canonical h, not ours.
                //    Request h from peer → canonical arrives → FORK_CHOICE
                //    replaces our fork block (lower slot wins) → orphan from
                //    cache chains on top. One request, one reorg, resolved.
                let need_height = if let Ok(Some(our_block)) =
                    self.block_store.get_block_by_height(current_height)
                {
                    if our_block.hash() != block.header.prev_hash {
                        // Case B: fork — request the canonical block at OUR height
                        info!(
                            "[ORPHAN_FORK_CHASE] Fork at h={}: our={:.8} != orphan prev={:.8}. Requesting canonical h={} from {}",
                            current_height, our_block.hash(), block.header.prev_hash,
                            current_height, source_peer
                        );
                        current_height
                    } else {
                        // Case A: normal orphan — request next height
                        current_height + 1
                    }
                } else {
                    current_height + 1
                };

                if let Some(ref network) = self.network {
                    if need_height == current_height {
                        info!(
                            "[ORPHAN_FORK_CHASE] Requesting h={} from {} (fork resolution for orphan {:.8} at slot {})",
                            need_height, source_peer, block_hash, block.header.slot
                        );
                    } else {
                        info!(
                            "[ORPHAN_CHASE] Requesting h={} from {} (orphan block {:.8} at slot {})",
                            need_height, source_peer, block_hash, block.header.slot
                        );
                    }
                    let request = SyncRequest::GetBlockByHeight {
                        height: need_height,
                    };
                    let _ = network.request_sync(source_peer, request).await;
                }
                self.sync_manager
                    .write()
                    .await
                    .note_orphan_gossip_block(current_height + 1, block.header.slot);
                // Cache the orphan so it's available when the missing parent arrives.
                // Without this, the orphan is lost and we wait for gossip re-delivery.
                {
                    let mut cache = self.fork_block_cache.write().await;
                    cache.insert(block_hash, block.clone());
                    if cache.len() > 100 {
                        let oldest = cache.keys().next().copied();
                        if let Some(k) = oldest {
                            cache.remove(&k);
                        }
                    }
                }
                return Ok(());
            }

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
        // INC-I-010 layer 3: After snap sync, epoch_producer_list contains ALL active
        // producers instead of the attestation-filtered subset. Full validation would
        // reject valid gossip blocks (slot%N divergence). Use Light mode until the next
        // epoch boundary rebuilds the list correctly and clears snap_sync_height.
        let mode = if self.snap_sync_height.is_some() {
            ValidationMode::Light
        } else {
            ValidationMode::Full
        };
        if let Err(e) = self.apply_block(block, mode).await {
            let err_str = e.to_string();
            warn!(
                "[BLOCK] REJECT slot={} h={} producer={} error={} — skipping, sync will catch up",
                block_slot,
                height,
                hex::encode(&block_producer.as_bytes()[..4]),
                err_str,
            );
            // NOTE: auto-heal removed. EpochState::derive_at_boundary() is now the
            // single canonical derivation — called at every epoch boundary in post_commit.
            // If "invalid producer" is hit, it self-corrects at the next boundary.
            return Ok(());
        }

        // A canonical gossip block was applied on our tip — clear the post-snap gate.
        // This proves we're on the canonical chain and our block store has a real parent.
        self.sync_manager
            .write()
            .await
            .clear_awaiting_canonical_block();

        // Post-apply: check fork cache for the next block (orphan that arrived
        // before its parent). If found, apply it immediately — no gossip wait.
        {
            let new_tip_hash = self.chain_state.read().await.best_hash;
            let next_from_cache = {
                let cache = self.fork_block_cache.read().await;
                cache
                    .values()
                    .find(|b| b.header.prev_hash == new_tip_hash)
                    .cloned()
            };
            if let Some(cached_block) = next_from_cache {
                let cached_hash = cached_block.hash();
                info!(
                    "[ORPHAN_APPLY] Applying cached orphan {:.8} (slot {}) from fork cache",
                    cached_hash, cached_block.header.slot
                );
                let _ = self.apply_block(cached_block, mode).await;
                self.fork_block_cache.write().await.remove(&cached_hash);
            }
        }

        // Post-apply catch-up: if any peer has a block above our new tip, pull
        // the next one immediately instead of waiting for gossip. Without this,
        // nodes on resource-contended hosts stabilize at a persistent lag of 1:
        // gossip delivers h=N while the network produces h=N+1, apply takes long
        // enough that by the time we finish, h=N+1 already exists but our gap
        // (=1) stays below the sync-manager threshold, so no catch-up fires.
        //
        // catch_up_request() is read-only; it returns None when we are actively
        // syncing, already at tip, or have a pending request for the next block.
        // Self-limiting: zero requests when caught up.
        let catch_up = self.sync_manager.read().await.catch_up_request();
        if let Some((peer_id, request)) = catch_up {
            if let Some(ref network) = self.network {
                let _ = network.request_sync(peer_id, request).await;
            }
        }

        // Gossip latency instrumentation (v6.13.22).
        info!(
            "[APPLY_END] slot={} apply_ms={} status=applied",
            block_slot,
            apply_start.elapsed().as_millis()
        );

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

            // REQ-REDESIGN-011 (FORK_GUARD backfill invariant): chain_state
            // must NEVER advance past block_store completeness. Before we
            // mutate chain_state, verify the entire 1..=target_height range
            // is dense in block_store. Any gap means a prior partial reorg,
            // archiver prune, or snap-sync race left the store inconsistent
            // — completing the reorg would corrupt chain_state by writing a
            // best_hash whose block is missing from local storage. The
            // 2026-04-16 santiago/ivan/seed3 cascade (INC-I-034) traces back
            // to exactly this silent advance. If the range is incomplete,
            // refuse the switch and surface the error so sync can backfill.
            //
            // target_height == 0 is the legitimate full-rollback-to-genesis
            // case; ensure_blocks_present treats low=0 as a no-op so that
            // path is unaffected.
            self.block_store
                .ensure_blocks_present(1, target_height)
                .map_err(|e| {
                    error!(
                        "[FORK_GUARD_BACKFILL_REQUIRED] Reorg refused: \
                         block_store missing canonical blocks in 1..={} — {}. \
                         chain_state.best_hash NOT advanced. Backfill required \
                         before this reorg can proceed.",
                        target_height, e
                    );
                    anyhow::anyhow!(
                        "[FORK_GUARD_BACKFILL_REQUIRED] block_store incomplete \
                         in range 1..={}: {}",
                        target_height,
                        e
                    )
                })?;

            // Find the common ancestor block. With ensure_blocks_present above
            // we are guaranteed get_block_by_height(target_height) returns
            // Some(_) when target_height > 0; the only acceptable None branch
            // is the genesis case (target_height == 0).
            let common_ancestor_block = if target_height == 0 {
                None
            } else {
                match self.block_store.get_block_by_height(target_height)? {
                    Some(b) => Some(b),
                    None => {
                        // Defense-in-depth: ensure_blocks_present already
                        // rejected this case. If we somehow reach here, refuse
                        // rather than silently substituting genesis_hash (the
                        // pre-fix bug at this site).
                        error!(
                            "[FORK_GUARD_BACKFILL_REQUIRED] Reorg refused: \
                             common ancestor at h={} missing from block_store \
                             after completeness check. chain_state.best_hash \
                             NOT advanced.",
                            target_height
                        );
                        anyhow::bail!(
                            "[FORK_GUARD_BACKFILL_REQUIRED] common ancestor at \
                             h={} missing from block_store",
                            target_height
                        );
                    }
                }
            };

            // Genesis (target_height == 0) anchors at genesis_hash; otherwise
            // the common ancestor's own hash. No silent substitution.
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
}
