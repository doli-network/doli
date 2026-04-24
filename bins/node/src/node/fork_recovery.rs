use super::*;

impl Node {
    /// Handle a completed fork recovery — evaluate the fork chain and reorg if heavier.
    ///
    /// Called when the parent chain walk connects to a block in our block_store.
    /// Records weights, moves blocks to fork_block_cache, plans reorg, executes if heavier.
    pub async fn handle_completed_fork_recovery(
        &mut self,
        recovery: network::sync::CompletedRecovery,
    ) -> Result<()> {
        let fork_len = recovery.blocks.len();
        info!(
            "Fork recovery complete: {} blocks connected at {}",
            fork_len,
            &recovery.connection_point.to_string()[..16]
        );

        let current_height = self.chain_state.read().await.best_height;
        let current_tip = self.chain_state.read().await.best_hash;

        // 1. Record fork blocks in reorg_handler with weights (forward order for correct accumulation)
        let mut last_block_weight = 1u64;
        {
            let producers = self.producer_set.read().await;
            let mut sync = self.sync_manager.write().await;
            for block in &recovery.blocks {
                let weight = producers
                    .get_by_pubkey(&block.header.producer)
                    .map(|p| p.effective_weight(current_height + 1))
                    .unwrap_or(1);
                sync.record_fork_block_weight(block.hash(), block.header.prev_hash, weight);
                last_block_weight = weight;
            }
        }

        // 2. Move fork blocks to fork_block_cache (execute_reorg reads from here)
        {
            let mut cache = self.fork_block_cache.write().await;
            for block in &recovery.blocks {
                cache.insert(block.hash(), block.clone());
            }
        }

        // 3. Try simple reorg first (works for single-block forks within recent_blocks)
        let fork_tip = recovery.blocks.last().unwrap();
        let simple_reorg = {
            let sync = self.sync_manager.read().await;
            sync.reorg_handler()
                .check_reorg_weighted(fork_tip, current_tip, last_block_weight)
        };

        if let Some(result) = simple_reorg {
            // Deterministic tiebreak: on equal weight, lower block hash wins.
            // All nodes compute the same hash, so all nodes converge to the same chain
            // regardless of gossip arrival order. This eliminates the symmetric-switch bug
            // where both sides of a delta=0 fork switch simultaneously and cross paths.
            let should_switch = if result.weight_delta > 0 {
                true
            } else if result.weight_delta == 0 {
                fork_tip.hash() < current_tip
            } else {
                false
            };
            if should_switch {
                info!(
                    "Fork recovery: switching to network chain (delta={}, fork_hash={}, our_hash={}) — rollback={}, new={}",
                    result.weight_delta,
                    &fork_tip.hash().to_string()[..16],
                    &current_tip.to_string()[..16],
                    result.rollback.len(),
                    result.new_blocks.len()
                );
                let trigger = fork_tip.clone();
                self.execute_reorg(result, trigger).await?;
            } else {
                info!(
                    "Fork not heavier (delta={}, fork_hash={}, our_hash={}) — keeping current chain",
                    result.weight_delta,
                    &fork_tip.hash().to_string()[..16],
                    &current_tip.to_string()[..16],
                );
            }
            return Ok(());
        }

        // 4. Fall back to plan_reorg for deeper forks
        let fork_tip_hash = fork_tip.hash();
        let reorg_result = {
            let sync = self.sync_manager.read().await;
            let store = &self.block_store;
            sync.reorg_handler()
                .plan_reorg(current_tip, fork_tip_hash, |hash| {
                    store.get_header(hash).ok().flatten().map(|h| h.prev_hash)
                })
        };

        // 5. Execute reorg if fork is heavier, or if tied with lower hash
        match reorg_result {
            Some(result)
                if result.weight_delta > 0
                    || (result.weight_delta == 0 && fork_tip_hash < current_tip) =>
            {
                info!(
                    "Fork recovery: switching to network chain (delta={}, fork_hash={}, our_hash={}) — rollback={}, new={}",
                    result.weight_delta,
                    &fork_tip_hash.to_string()[..16],
                    &current_tip.to_string()[..16],
                    result.rollback.len(),
                    result.new_blocks.len()
                );
                let trigger = recovery.blocks.last().unwrap().clone();
                self.execute_reorg(result, trigger).await?;
            }
            Some(result) => {
                info!(
                    "Fork not heavier (delta={}, fork_hash={}, our_hash={}) — keeping current chain",
                    result.weight_delta,
                    &fork_tip_hash.to_string()[..16],
                    &current_tip.to_string()[..16],
                );
            }
            None => {
                warn!("Could not plan reorg from recovered fork — common ancestor not found");
            }
        }

        Ok(())
    }

    /// Try to start fork recovery from cached orphan blocks.
    /// Called from production gate when fork is detected (ChainMismatch, AheadOfPeers).
    pub async fn try_trigger_fork_recovery(&mut self) {
        let can_start = self.sync_manager.read().await.can_start_fork_recovery();
        if !can_start {
            return;
        }
        let orphan = {
            let cache = self.fork_block_cache.read().await;
            cache.values().next().cloned()
        };
        if let Some(orphan) = orphan {
            let peer = self.sync_manager.read().await.best_peer_for_recovery();
            if let Some(peer) = peer {
                let started = self
                    .sync_manager
                    .write()
                    .await
                    .start_fork_recovery(orphan, peer);
                if started {
                    info!("[FORK] RECOVERY_START triggered from production gate");
                }
            }
        }
    }

    /// Try to apply a chain of cached blocks when we're behind
    ///
    /// This function attempts to build a chain from cached fork blocks
    /// back to our current tip, then applies them in order.
    pub async fn try_apply_cached_chain(&mut self, latest_block: Block) -> Result<()> {
        let our_tip = self.chain_state.read().await.best_hash;

        // Build chain backwards from latest_block to our tip
        let mut chain: Vec<Block> = Vec::new();
        let mut current = latest_block.clone();

        // Limit how far back we'll look (prevent infinite loops)
        const MAX_CHAIN_LENGTH: usize = 50;

        for _ in 0..MAX_CHAIN_LENGTH {
            let parent_hash = current.header.prev_hash;

            if parent_hash == our_tip {
                // Found connection to our chain!
                chain.reverse(); // Blocks are in reverse order, flip them
                chain.insert(0, current);

                info!(
                    "Found chain of {} cached blocks connecting to our tip, applying",
                    chain.len()
                );

                // Apply all blocks in order
                for block in chain {
                    // Validate producer eligibility
                    if let Err(e) = self.check_producer_eligibility(&block).await {
                        anyhow::bail!(
                            "[FORK_INVALID_PRODUCER] cached block at slot={} has invalid producer {}: {}",
                            block.header.slot,
                            hex::encode(&block.header.producer.as_bytes()[..4]),
                            e
                        );
                    }
                    // Remove from cache before applying
                    {
                        let mut cache = self.fork_block_cache.write().await;
                        cache.remove(&block.hash());
                    }
                    self.apply_block(block, ValidationMode::Full).await?;
                }

                return Ok(());
            }

            // Check if parent is in our block store (not just cache)
            if let Ok(Some(_)) = self.block_store.get_block(&parent_hash) {
                // Parent is in our store but not our tip - this is a fork
                // We can't simply apply these blocks; we'd need to reorg
                debug!(
                    "Parent {} found in store but not at tip - would need reorg",
                    parent_hash
                );
                break;
            }

            // Look for parent in cache
            let cache = self.fork_block_cache.read().await;
            if let Some(parent) = cache.get(&parent_hash) {
                chain.push(current);
                current = parent.clone();
            } else {
                // Parent not in cache - can't build chain
                debug!("Parent {} not in cache, cannot build chain", parent_hash);
                break;
            }
        }

        // Couldn't build complete chain - maybe we need to sync from peers
        // This will be handled by the normal sync process
        anyhow::bail!("[FORK_CHAIN_INCOMPLETE] could not build complete chain from {} cached blocks (missing parent)", chain.len())
    }

    /// Apply a verified snap sync snapshot, replacing local state.
    ///
    /// Called when the sync manager's snap sync quorum voting + download completes.
    /// The snapshot has already been verified (state root matches quorum) by the
    /// network layer. This method:
    /// 1. Re-verifies state root (defense-in-depth)
    /// 2. Deserializes the 3 state components
    /// 3. Replaces local state atomically
    /// 4. Persists to StateDb
    /// 5. Seeds canonical index for post-snap header sync
    pub async fn apply_snap_snapshot(&mut self, snapshot: network::VerifiedSnapshot) -> Result<()> {
        // Recovery mode: block snap sync consumption (anti-poisoning gate)
        if self.recovery_mode.load(Ordering::Relaxed) {
            warn!("[RECOVERY] Snap sync blocked — node is in recovery mode");
            return Ok(());
        }

        info!(
            "[SNAP_SYNC] Applying snapshot: height={}, hash={:.16}, root={:.16}",
            snapshot.block_height, snapshot.block_hash, snapshot.state_root
        );

        // Step 1: Verify state root (node-side, since network crate has no storage dep)
        let computed_root = match storage::compute_state_root_from_bytes(
            &snapshot.chain_state,
            &snapshot.utxo_set,
            &snapshot.producer_set,
        ) {
            Ok(root) => root,
            Err(e) => {
                error!(
                    "[SNAP_SYNC] Snapshot deserialization failed at height={}: {} — rejecting",
                    snapshot.block_height, e
                );
                self.sync_manager.write().await.snap_fallback_to_normal();
                return Ok(());
            }
        };
        if computed_root != snapshot.state_root {
            error!(
                "[SNAP_SYNC] State root mismatch! computed={}, expected={} — rejecting",
                computed_root, snapshot.state_root
            );
            self.sync_manager.write().await.snap_fallback_to_normal();
            return Ok(());
        }

        // Step 2: Deserialize snapshot components
        let new_chain_state: ChainState = bincode::deserialize(&snapshot.chain_state)
            .map_err(|e| anyhow::anyhow!("[SNAP_SYNC] Failed to deserialize chain_state: {}", e))?;
        let new_utxo_set: storage::UtxoSet =
            storage::UtxoSet::deserialize_canonical(&snapshot.utxo_set).map_err(|e| {
                anyhow::anyhow!("[SNAP_SYNC] Failed to deserialize utxo_set: {}", e)
            })?;
        let new_producer_set: storage::ProducerSet = bincode::deserialize(&snapshot.producer_set)
            .map_err(|e| {
            anyhow::anyhow!("[SNAP_SYNC] Failed to deserialize producer_set: {}", e)
        })?;

        // C3 defense: envelope must match deserialized state
        if new_chain_state.best_hash != snapshot.block_hash
            || new_chain_state.best_height != snapshot.block_height
        {
            error!("[SNAP_SYNC] Envelope/state mismatch — rejecting",);
            self.sync_manager.write().await.snap_fallback_to_normal();
            return Ok(());
        }

        // Step 3: Replace local state
        let genesis_hash = self.chain_state.read().await.genesis_hash;
        {
            let mut cs = self.chain_state.write().await;
            *cs = new_chain_state;
            cs.genesis_hash = genesis_hash;
            cs.mark_snap_synced(snapshot.block_height);

            let mut utxo = self.utxo_set.write().await;
            *utxo = new_utxo_set;

            let mut ps = self.producer_set.write().await;
            *ps = new_producer_set;

            // Cache state root atomically
            if let Ok(root) = storage::compute_state_root(&cs, &utxo, &ps) {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, cs.best_hash, cs.best_height));
            }

            // Persist to StateDb
            let utxo_pairs = utxo.iter_all();
            if let Err(e) = self
                .state_db
                .atomic_replace(&cs, &ps, utxo_pairs.into_iter())
            {
                error!("[SNAP_SYNC] StateDb atomic_replace failed: {}", e);
            }

            // Update sync manager local tip
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(cs.best_height, cs.best_hash, cs.best_slot);
        }

        // Step 4: Seed canonical index for post-snap header sync
        self.block_store
            .seed_canonical_index(snapshot.block_hash, snapshot.block_height)?;

        // Option C: persist anchor header if included in snapshot (post-activation)
        if let Some(header_bytes) = &snapshot.block_header_bytes {
            if let Ok(header) = bincode::deserialize::<doli_core::BlockHeader>(header_bytes) {
                // Create a minimal block with just the header (no transactions)
                // so put_block persists the header to CF_HEADERS
                let anchor_block = doli_core::Block {
                    header,
                    transactions: vec![],
                    aggregate_bls_signature: vec![],
                    attestation_bitfield: vec![],
                };
                if let Err(e) = self
                    .block_store
                    .put_block(&anchor_block, snapshot.block_height)
                {
                    warn!(
                        "[SNAP_SYNC] Failed to persist anchor header at h={}: {}",
                        snapshot.block_height, e
                    );
                } else {
                    info!(
                        "[SNAP_SYNC] Persisted anchor header at h={} (Option C)",
                        snapshot.block_height
                    );
                }
            }
        }

        // M7: Complete EpochState transfer — use directly when available.
        // This is the canonical state from the sender, computed by derive_at_boundary().
        // No reconstruction, no filtering, no fallback — the sender's state IS the state.
        if let Some(ref epoch_state_bytes) = snapshot.epoch_state_bytes {
            match doli_core::EpochState::deserialize(epoch_state_bytes) {
                Ok(epoch_state) => {
                    info!(
                        "[SNAP_SYNC] Complete EpochState from peer: epoch={} producers={} active={}",
                        epoch_state.epoch,
                        epoch_state.producer_list.len(),
                        epoch_state.active_list.len()
                    );
                    self.epoch_state = epoch_state;
                    // Persist atomically
                    let mut batch = self.state_db.begin_batch();
                    batch.put_epoch_producer_list(&self.epoch_state.producer_list);
                    batch.put_active_production_list(&self.epoch_state.active_list);
                    batch.put_attestation_accumulators(
                        &self.epoch_state.attested_sets,
                        &self.epoch_state.attestation_accum,
                        &self.epoch_state.blocks_produced,
                    );
                    batch.put_epoch_bond_snapshot(
                        &self.epoch_state.bond_snapshot,
                        self.epoch_state.epoch,
                    );
                    batch.put_epoch_state(&self.epoch_state.serialize());
                    batch.put_epoch_state_version(CURRENT_PROTOCOL_VERSION);
                    let _ = batch.commit();
                }
                Err(e) => {
                    warn!(
                        "[SNAP_SYNC] Failed to deserialize epoch_state_bytes: {} — falling back to reconstruction",
                        e
                    );
                    // Fall through to legacy reconstruction below
                }
            }
        }

        // Legacy reconstruction (for older peers that don't send epoch_state_bytes).
        // Only runs if epoch_state was not loaded from the fast path above.
        if snapshot.epoch_state_bytes.is_none() || self.epoch_state.producer_list.is_empty() {
            // Step 5: Load epoch_bond_snapshot from persisted state_db (downloaded from peer).
            // The peer persisted the correct bond snapshot at the epoch boundary.
            // Do NOT recalculate from UTXO — the downloaded UTXO set reflects snap_height,
            // not epoch_boundary, so count_bonds() includes mid-epoch add-bonds that diverge
            // from the canonical snapshot computed at epoch boundary.
            {
                let ps = self.producer_set.read().await;
                let h = snapshot.block_height;
                let bpe = self.config.network.blocks_per_reward_epoch();
                let active = ps.active_producers_at_height(h);

                // Try 3 sources in order:
                // 1. Bond snapshot from sync protocol payload (v6.13.17+ peers)
                // 2. Persisted in state_db (from previous epoch boundary)
                // 3. Fallback: recalculate from UTXO (may diverge)
                let from_payload: Option<(std::collections::HashMap<crypto::Hash, u64>, u64)> =
                    snapshot
                        .epoch_bond_snapshot_bytes
                        .as_ref()
                        .and_then(|bytes| bincode::deserialize(bytes).ok());

                {
                    let has_bond_snap = from_payload.is_some();
                    let (bond_producers, bond_total, bond_epoch) = from_payload
                        .as_ref()
                        .map(|(s, e)| (s.len(), s.values().sum::<u64>(), *e))
                        .unwrap_or((0, 0, 0));
                    info!(
                    "[SNAP_SYNC] Bond snapshot from peer: included={} epoch={} producers={} total_bonds={}",
                    has_bond_snap, bond_epoch, bond_producers, bond_total
                );
                }

                if let Some((snap, epoch)) = from_payload {
                    let total: u64 = snap.values().sum();
                    self.epoch_state.bond_snapshot = snap;
                    self.epoch_state.epoch = epoch;
                    // Persist so restarts don't lose it
                    let mut batch = self.state_db.begin_batch();
                    batch.put_epoch_bond_snapshot(&self.epoch_state.bond_snapshot, epoch);
                    let _ = batch.commit();
                    info!(
                    "[SNAP_SYNC] Bond snapshot from peer payload: {} producers, total_bonds={}, epoch={}",
                    self.epoch_state.bond_snapshot.len(), total, epoch
                );
                } else if let Some((snap, epoch)) = self.state_db.get_epoch_bond_snapshot() {
                    let total: u64 = snap.values().sum();
                    self.epoch_state.bond_snapshot = snap;
                    self.epoch_state.epoch = epoch;
                    info!(
                    "[SNAP_SYNC] Loaded persisted bond snapshot: {} producers, total_bonds={}, epoch={}",
                    self.epoch_state.bond_snapshot.len(), total, epoch
                );
                } else {
                    // Fallback for peers on pre-v6.13.14 (no persisted bond snapshot).
                    let us = self.utxo_set.read().await;
                    let bond_unit = self.config.network.bond_unit();
                    let mut snap = std::collections::HashMap::new();
                    for p in &active {
                        let pkh = crypto::hash::hash_with_domain(
                            crypto::ADDRESS_DOMAIN,
                            p.public_key.as_bytes(),
                        );
                        let count = us.count_bonds(&pkh, bond_unit).max(1) as u64;
                        snap.insert(pkh, count);
                    }
                    let total: u64 = snap.values().sum();
                    let epoch = h.checked_div(bpe).unwrap_or(0);
                    self.epoch_state.bond_snapshot = snap;
                    self.epoch_state.epoch = epoch;
                    warn!(
                    "[SNAP_SYNC] No persisted bond snapshot — rebuilt from UTXO (may diverge): {} producers, total_bonds={}, epoch={}",
                    self.epoch_state.bond_snapshot.len(), total, epoch
                );
                }

                // Temporary unfiltered epoch_producer_list. Will be replaced by
                // derive_at_boundary() below which applies the attestation filter
                // using the in-memory epoch_attested_set loaded from the peer's
                // accumulator payload.
                let mut pks: Vec<_> = active.iter().map(|p| p.public_key).collect();
                pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                self.epoch_state.producer_list = pks;
                self.epoch_state.active_list = self.epoch_state.producer_list.clone();

                let total: u64 = self.epoch_state.bond_snapshot.values().sum();
                info!(
                "[SNAP_SYNC] Rebuilt epoch state (pre-filter): {} producers, total_bonds={}, epoch={}",
                self.epoch_state.bond_snapshot.len(),
                total,
                self.epoch_state.epoch
            );
            }

            // Step 5b: Load attestation accumulators from peer payload.
            // Eliminates the 3-epoch convergence window where attestation data
            // diverges after snap sync.
            //
            // Fix #6 (2026-04-15, synmgrefactor): MUST run BEFORE the attestation-
            // filter rebuild below, because that rebuild reads epoch_attested_set
            // to decide which producers belong in epoch_producer_list.
            if let Some(ref accum_bytes) = snapshot.epoch_accumulators_bytes {
                type AccumType = (
                    [std::collections::HashSet<crypto::PublicKey>; 3],
                    [std::collections::HashMap<crypto::PublicKey, std::collections::HashSet<u32>>;
                        3],
                    std::collections::HashMap<crypto::PublicKey, u32>,
                );
                if let Ok((attested, accum, produced)) =
                    bincode::deserialize::<AccumType>(accum_bytes)
                {
                    info!(
                    "[SNAP_SYNC] Attestation accumulators from peer: attested=[{},{},{}] accum=[{},{},{}] produced={}",
                    attested[0].len(), attested[1].len(), attested[2].len(),
                    accum[0].len(), accum[1].len(), accum[2].len(),
                    produced.len()
                );
                    self.epoch_state.attested_sets = attested;
                    self.epoch_state.attestation_accum = accum;
                    self.epoch_state.blocks_produced = produced;
                    // Persist so restarts don't lose them
                    let mut batch = self.state_db.begin_batch();
                    batch.put_attestation_accumulators(
                        &self.epoch_state.attested_sets,
                        &self.epoch_state.attestation_accum,
                        &self.epoch_state.blocks_produced,
                    );
                    let _ = batch.commit();
                }
            }

            // Apply attestation filter via the canonical derive_at_boundary().
            // The accumulators were loaded above (from peer payload or persisted).
            // derive_at_boundary uses them for the 3-epoch lookback filter.
            {
                let snap_h = snapshot.block_height;
                let bpe = self.config.network.blocks_per_reward_epoch();
                let epoch = snap_h.checked_div(bpe).unwrap_or(0);

                let producers = self.producer_set.read().await;
                let active_producers: Vec<PublicKey> = producers
                    .active_producers_at_height(snap_h)
                    .iter()
                    .map(|p| p.public_key)
                    .collect();
                let registered_at: std::collections::HashMap<PublicKey, u64> = producers
                    .active_producers_at_height(snap_h)
                    .iter()
                    .map(|p| (p.public_key, p.registered_at))
                    .collect();
                drop(producers);

                let input = doli_core::EpochDerivationInput {
                    active_producers,
                    bond_counts: self.epoch_state.bond_snapshot.clone(),
                    blocks_per_epoch: bpe,
                    snap_attestation_skip_height: self
                        .config
                        .network
                        .params()
                        .snap_attestation_skip_height,
                    height: snap_h,
                    epoch,
                    registered_at,
                    ghost_exclusion_activation_height: self
                        .config
                        .network
                        .params()
                        .ghost_exclusion_activation_height,
                };
                let derived = doli_core::EpochState::derive_at_boundary(&self.epoch_state, &input);
                info!(
                    "[SNAP_SYNC] Derived epoch state: epoch={} producers={} active={}",
                    derived.epoch,
                    derived.producer_list.len(),
                    derived.active_list.len()
                );
                self.epoch_state = derived;

                // Persist the derived state
                let mut batch = self.state_db.begin_batch();
                batch.put_epoch_producer_list(&self.epoch_state.producer_list);
                batch.put_active_production_list(&self.epoch_state.active_list);
                batch.put_epoch_state(&self.epoch_state.serialize());
                batch.put_epoch_state_version(CURRENT_PROTOCOL_VERSION);
                let _ = batch.commit();
            }
        } // end legacy reconstruction guard

        // Step 6: Track snap sync height for validation mode selection
        self.snap_sync_height = Some(snapshot.block_height);

        // Step 6b: Clear fork_block_cache — pre-snap cached blocks are stale
        // and would waste memory, pollute eviction, and cause pointless validation.
        {
            let mut cache = self.fork_block_cache.write().await;
            let cleared = cache.len();
            cache.clear();
            if cleared > 0 {
                info!(
                    "[SNAP_SYNC] Cleared {} stale blocks from fork_block_cache",
                    cleared
                );
            }
        }

        // Step 7: Inform sync manager of block store floor
        {
            let mut sync = self.sync_manager.write().await;
            sync.set_store_floor(snapshot.block_height);
            sync.record_block_applied_after_snap(snapshot.block_hash, snapshot.block_height);
        }

        info!(
            "[SNAP_SYNC] Snapshot applied successfully — now at height {} hash={:.16}",
            snapshot.block_height, snapshot.block_hash
        );

        Ok(())
    }
}
