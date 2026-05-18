use super::*;

// =============================================================================
// Block classification — pure decision logic, no side effects
// =============================================================================

/// Classification of a gossip block relative to our current chain state.
/// Separates the decision (what kind of block is this?) from the action
/// (what do we do about it?), making the logic testable and self-documenting.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BlockClass {
    /// Block extends our current tip — apply directly.
    ExtendsTip,
    /// Fork block: parent known, competes with canonical chain.
    ForkBlock(ForkBlockKind),
    /// Orphan: parent not in our block store.
    Orphan {
        /// Height we need to request from the sender.
        need_height: u64,
    },
    /// Rejected: different chain, fork_id mismatch, etc.
    Rejected(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ForkBlockKind {
    /// Height already occupied by a different canonical block — drop.
    HeightOccupied {
        fork_height: u64,
        canonical_slot: u32,
        /// True if the fork block has a better (lower) slot than canonical.
        is_better: bool,
    },
    /// Parent known but no height conflict — cache for potential reorg.
    ReorgCandidate,
}

/// Pure classification function: determines what kind of gossip block this is
/// without performing any side effects (no caching, no network requests, no
/// state mutations). All inputs are read-only snapshots of node state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn classify_gossip_block(
    block: &Block,
    block_hash: Hash,
    best_hash: Hash,
    best_height: u64,
    genesis_hash: Hash,
    our_fork_id: Hash,
    fork_id_activation_height: u64,
    block_store: &BlockStore,
) -> BlockClass {
    // ExtendsTip: block builds directly on our chain tip
    if block.header.prev_hash == best_hash {
        return BlockClass::ExtendsTip;
    }

    // Reject blocks from a different chain (different genesis hash)
    if block.header.genesis_hash != genesis_hash {
        return BlockClass::Rejected(format!(
            "different chain (genesis {:.16} != {:.16})",
            block.header.genesis_hash, genesis_hash
        ));
    }

    // Reject blocks with different fork_id (different hard fork set)
    if best_height >= fork_id_activation_height && block.header.fork_id != our_fork_id {
        return BlockClass::Rejected(format!(
            "fork_id mismatch ({:.16} != {:.16})",
            block.header.fork_id, our_fork_id
        ));
    }

    // Check if parent is in our block store
    if let Ok(Some(parent_height)) = block_store.get_height_by_hash(&block.header.prev_hash) {
        let fork_height = parent_height + 1;
        if let Ok(Some(canonical)) = block_store.get_block_by_height(fork_height) {
            if canonical.hash() != block_hash {
                // Height occupied by a different canonical block
                return BlockClass::ForkBlock(ForkBlockKind::HeightOccupied {
                    fork_height,
                    canonical_slot: canonical.header.slot,
                    is_better: block.header.slot < canonical.header.slot,
                });
            }
        }
        // Parent known, no height conflict → reorg candidate
        return BlockClass::ForkBlock(ForkBlockKind::ReorgCandidate);
    }

    // Parent not in store → orphan
    BlockClass::Orphan {
        need_height: best_height + 1,
    }
}

// =============================================================================
// Block handling — dispatch actions based on classification
// =============================================================================

impl Node {
    /// Insert a block into the fork cache with unified slot-sorted eviction.
    async fn cache_block_with_eviction(&self, hash: Hash, block: Block) {
        let mut cache = self.fork_block_cache.write().await;
        cache.insert(hash, block);
        if cache.len() > 100 {
            let mut blocks_by_slot: Vec<(Hash, u32)> =
                cache.iter().map(|(h, b)| (*h, b.header.slot)).collect();
            blocks_by_slot.sort_by_key(|(_, slot)| *slot);
            for (hash, _) in blocks_by_slot.into_iter().take(50) {
                cache.remove(&hash);
            }
        }
    }

    /// Handle a new block from the network
    pub async fn handle_new_block(&mut self, block: Block, source_peer: PeerId) -> Result<()> {
        let block_hash = block.hash();
        let block_slot = block.header.slot;

        // Gossip latency instrumentation (v6.13.22): correlate by slot with
        // [GOSSIP_RECV] and [APPLY_END] lines. See on_new_block_event.
        let apply_start = Instant::now();
        info!("[APPLY_START] slot={} hash={}", block_slot, block_hash);

        // Pre-check: already known?
        if self.block_store.get_block(&block_hash)?.is_some() {
            debug!("Block {} already known", block_hash);
            info!(
                "[APPLY_END] slot={} apply_ms={} status=already_known",
                block_slot,
                apply_start.elapsed().as_millis()
            );
            return Ok(());
        }

        // Check for equivocation (double signing) - even for forks.
        // This is critical for slashing misbehaving producers.
        let equivocation_proof = { self.equivocation_detector.write().await.check_block(&block) };
        if let Some(proof) = equivocation_proof {
            self.handle_equivocation(proof).await;
        }

        // Classify the block against our current chain state
        let (best_hash, best_height, genesis_hash) = {
            let state = self.chain_state.read().await;
            (state.best_hash, state.best_height, state.genesis_hash)
        };
        let our_fork_id = self.current_fork_id();
        let fork_id_activation = self.config.network.params().fork_id_activation_height;

        let class = classify_gossip_block(
            &block,
            block_hash,
            best_hash,
            best_height,
            genesis_hash,
            our_fork_id,
            fork_id_activation,
            &self.block_store,
        );

        match class {
            BlockClass::Rejected(reason) => {
                debug!("[CLASSIFY] Dropping block {}: {}", block_hash, reason);
                return Ok(());
            }

            BlockClass::ForkBlock(ForkBlockKind::HeightOccupied {
                fork_height,
                canonical_slot,
                is_better,
            }) => {
                // Height-occupied fork guard: discard blocks that don't extend our tip
                // if we already have canonical chain at or above their height.
                //
                // Legitimate reorg comes through header-first sync, not gossip.
                // O(1): one get_block_height + one get_block_by_height lookup.
                if is_better {
                    // INC-I-040: If the dropped block has a BETTER (lower) slot,
                    // WE are on the losing fork. Signal stuck_fork for
                    // recovery via the RecoveryCoordinator on the next
                    // periodic tick (~1s).
                    info!(
                        "[FORK_GUARD] Better block dropped (slot {} < {}) at h={} — \
                         signaling fork recovery",
                        block_slot, canonical_slot, fork_height
                    );
                    self.sync_manager.write().await.signal_stuck_fork();
                } else {
                    info!(
                        "[FORK_GUARD] Dropping fork block {} at h={} slot {} — keeping canonical slot {}",
                        &block_hash.to_hex()[..16],
                        fork_height,
                        block_slot,
                        canonical_slot
                    );
                    // INC-I-036: Do NOT call note_orphan_gossip_block() here.
                    // Fork blocks with known parents are NOT orphans. Calling it
                    // inflated the orphan counter, triggering false-positive
                    // rollbacks after 3 fork blocks (190 rollbacks in 19 minutes).
                }
                return Ok(());
            }

            BlockClass::Orphan { need_height } => {
                // Parent not in store — orphan gossip block. The sender has the
                // missing block (they passed through our height to produce this one).
                // Request it directly: causal, deterministic, no heuristics.
                //
                // STABILITY PILLAR: ORPHAN_CHASE — do not modify this request logic.
                if let Some(ref network) = self.network {
                    info!(
                        "[ORPHAN_CHASE] Requesting h={} from {} (orphan block {:.8} at slot {})",
                        need_height, source_peer, block_hash, block_slot
                    );
                    let request = SyncRequest::GetBlockByHeight {
                        height: need_height,
                    };
                    let _ = network.request_sync(source_peer, request).await;
                }
                self.sync_manager
                    .write()
                    .await
                    .note_orphan_gossip_block(need_height, block_slot);
                // Cache the orphan so it's available when the missing parent arrives.
                self.cache_block_with_eviction(block_hash, block).await;
                return Ok(());
            }

            BlockClass::ForkBlock(ForkBlockKind::ReorgCandidate) => {
                // Parent is in our store but block doesn't extend our tip.
                // Cache it for potential reorg evaluation.
                self.cache_block_with_eviction(block_hash, block.clone())
                    .await;

                debug!(
                    "Block {} doesn't build on tip {} (builds on {}), cached for potential reorg",
                    block_hash, best_hash, block.header.prev_hash
                );

                // Check for reorg using weight-based fork choice rule
                let producer_weight = {
                    let producers = self.producer_set.read().await;
                    producers
                        .get_by_pubkey(&block.header.producer)
                        .map(|p| p.effective_weight(best_height + 1))
                        .unwrap_or(1)
                };

                let reorg_result = {
                    self.sync_manager
                        .write()
                        .await
                        .handle_new_block_weighted(block.clone(), producer_weight)
                };

                if let Some(reorg_result) = reorg_result {
                    info!(
                        "Reorg to heavier chain: rolling back {} blocks, applying {} new blocks, weight_delta=+{}",
                        reorg_result.rollback.len(),
                        reorg_result.new_blocks.len(),
                        reorg_result.weight_delta
                    );
                    if let Err(e) = self.execute_reorg(reorg_result, block).await {
                        error!("Failed to execute reorg: {}", e);
                    }
                } else {
                    // Reorg detection failed (parent not in our recent blocks).
                    // Try active fork recovery: walk backward through parent chain.
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
                    let our_slot = self.chain_state.read().await.best_slot;
                    let our_height = self.chain_state.read().await.best_height;
                    let cache_size = {
                        let cache = self.fork_block_cache.read().await;
                        let slot_window = 30u32;
                        let min_slot = our_slot.saturating_sub(slot_window);
                        cache.values().filter(|b| b.header.slot >= min_slot).count()
                    };

                    if cache_size >= 10 && cache_size % 10 == 0 {
                        warn!(
                            "Fork detected: {} orphan blocks don't build on our chain (height {}). Relying on fork recovery + stale chain sync.",
                            cache_size, our_height
                        );
                    }

                    self.try_trigger_fork_recovery().await;

                    if cache_size >= 2 {
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

            BlockClass::ExtendsTip => {
                // Block extends our tip — apply it
            }
        }

        // === ExtendsTip path: apply the block ===

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
            return Ok(());
        }

        // A canonical gossip block was applied on our tip — clear the post-snap gate.
        self.sync_manager
            .write()
            .await
            .clear_awaiting_canonical_block();

        // Post-apply: recursively drain cached orphans that chain on our new tip.
        // Bounded to 50 iterations to prevent unbounded loops from malicious caches.
        {
            let mut drained = 0u32;
            const MAX_DRAIN: u32 = 50;
            while drained < MAX_DRAIN {
                let tip_hash = self.chain_state.read().await.best_hash;
                let next_from_cache = {
                    let cache = self.fork_block_cache.read().await;
                    cache
                        .values()
                        .find(|b| b.header.prev_hash == tip_hash)
                        .cloned()
                };
                match next_from_cache {
                    Some(cached_block) => {
                        let cached_hash = cached_block.hash();
                        info!(
                            "[ORPHAN_APPLY] Applying cached orphan {:.8} (slot {}) from fork cache [{}/{}]",
                            cached_hash, cached_block.header.slot, drained + 1, MAX_DRAIN
                        );
                        self.fork_block_cache.write().await.remove(&cached_hash);
                        if self.apply_block(cached_block, mode).await.is_err() {
                            break;
                        }
                        drained += 1;
                    }
                    None => break,
                }
            }
        }

        // Post-apply catch-up: if any peer has a block above our new tip, pull
        // the next one immediately instead of waiting for gossip.
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

                    // INC-I-071: scan the rollback range and locate the first
                    // undo entry with a non-empty producer_snapshot. Per the
                    // sentinel semantics, empty means "ProducerSet unchanged
                    // from h-1 to h", so the first non-empty entry going
                    // forward from target_height+1 contains the BEFORE state
                    // for some height in the same producer-state era as
                    // target_height — that snapshot IS the state at
                    // target_height.
                    //
                    // Apply UTXO undo in reverse (highest block first) AND
                    // record the producer snapshot to restore from.
                    let mut producer_snapshot_for_restore: Option<Vec<u8>> = None;
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

                        // Track the LOWEST height entry with a non-empty
                        // producer_snapshot (overwrite while iterating in
                        // reverse — the loop ends at h = target_height + 1).
                        // That entry's BEFORE-state equals target_height's state.
                        if !undo.producer_snapshot.is_empty() {
                            producer_snapshot_for_restore = Some(undo.producer_snapshot.clone());
                        }
                    }

                    // Restore ProducerSet from the recorded snapshot (or skip
                    // if every entry in the range was an empty sentinel —
                    // producers were unchanged across the whole rollback range,
                    // so the in-memory ProducerSet is already correct).
                    if let Some(snapshot_bytes) = producer_snapshot_for_restore {
                        if let Ok(restored_producers) =
                            bincode::deserialize::<storage::ProducerSet>(&snapshot_bytes)
                        {
                            let mut producers = self.producer_set.write().await;
                            *producers = restored_producers;
                        } else {
                            warn!("Failed to deserialize producer snapshot from undo data, rebuilding from blocks");
                            let mut producers = self.producer_set.write().await;
                            self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
                        }
                    } else {
                        debug!(
                            "[REORG] All producer_snapshot entries empty in {}..={} — \
                             ProducerSet unchanged across rollback range, skipping restore",
                            target_height + 1,
                            current_height
                        );
                    }
                }

                // INC-I-040: Restore epoch_state from undo snapshot at target_height + 1.
                // The snapshot was taken BEFORE that block was applied = state AT target_height.
                // Without this, execute_reorg leaves stale attestation accumulators from
                // the OLD fork → wrong derive_at_boundary → wrong scheduling → persistent fork.
                // Height-gated: consensus-breaking change (different scheduling after reorg).
                //
                // INC-I-071: epoch_state_snapshot is NOT covered by the empty-sentinel
                // optimization — it is always present on every undo entry. Read once
                // from the same height that originally produced the producer snapshot
                // semantics (target_height + 1).
                if current_height
                    >= self
                        .config
                        .network
                        .params()
                        .epoch_state_reorg_activation_height
                {
                    let first_undo = self.state_db.get_undo(target_height + 1).unwrap();
                    if let Some(ref epoch_bytes) = first_undo.epoch_state_snapshot {
                        match doli_core::EpochState::deserialize(epoch_bytes) {
                            Ok(restored) => {
                                info!(
                                    "[REORG] Restored epoch state from undo: epoch={} producers={} active={}",
                                    restored.epoch, restored.producer_list.len(), restored.active_list.len()
                                );
                                self.epoch_state = restored;
                            }
                            Err(e) => {
                                warn!("[REORG] Failed to deserialize epoch state from undo: {} — rebuilding", e);
                                self.rebuild_epoch_state_from_blocks().await;
                            }
                        }
                    } else {
                        info!("[REORG] No epoch state in undo (pre-upgrade block) — rebuilding");
                        self.rebuild_epoch_state_from_blocks().await;
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
                                    // INC-I-064: Log spend failures in rebuild path
                                    if let Err(e) = utxo.spend_transaction(tx) {
                                        warn!(
                                            "[REBUILD] spend_transaction failed at h={}: {} — continuing rebuild",
                                            height, e
                                        );
                                    }
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

                // Legacy path: rebuild epoch_state from blocks (same activation gate).
                if current_height
                    >= self
                        .config
                        .network
                        .params()
                        .epoch_state_reorg_activation_height
                {
                    self.rebuild_epoch_state_from_blocks().await;
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
                let utxo_pairs = utxo.iter_all();
                self.state_db
                    .atomic_replace(&state, &producers, utxo_pairs.into_iter())
                    .map_err(|e| anyhow::anyhow!("Reorg StateDb atomic_replace failed: {}", e))?;
            }

            // Persist epoch_state to DB after atomic_replace (same gate).
            if current_height
                >= self
                    .config
                    .network
                    .params()
                    .epoch_state_reorg_activation_height
            {
                let epoch_bytes = self.epoch_state.serialize();
                self.state_db.put_epoch_state(&epoch_bytes);
            }
        } // end if rollback_count > 0

        // INC-I-081 Bug 4 / INV-SYNC-004: clear stale finality marker if the
        // post-rollback tip has dropped below the cached finality height.
        // Runs even when rollback_count == 0 (no-op in that case) for safety.
        {
            let mut sync = self.sync_manager.write().await;
            sync.clear_finality_if_below_tip(target_height);
        }

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
