use super::*;

impl Node {
    /// Handle a completed fork recovery — evaluate the fork chain and reorg if heavier.
    ///
    /// Called when the parent chain walk connects to a block in our block_store.
    /// Records weights, moves blocks to fork_block_cache, plans reorg, executes if heavier.
    pub(super) async fn handle_completed_fork_recovery(
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

        // Detect if our local fork is solo-produced (all blocks by same producer).
        // If the same dominant producer is on both chains, weights will tie.
        // A solo fork should never win a tie against the network-majority chain.
        let our_fork_is_solo = {
            let mut producers_seen = std::collections::HashSet::new();
            let store = &self.block_store;
            let mut hash = current_tip;
            for _ in 0..fork_len.max(1) {
                if hash == recovery.connection_point {
                    break;
                }
                if let Ok(Some(header)) = store.get_header(&hash) {
                    producers_seen.insert(header.producer);
                    hash = header.prev_hash;
                } else {
                    break;
                }
            }
            producers_seen.len() <= 1
        };

        // 3. Try simple reorg first (works for single-block forks within recent_blocks)
        let fork_tip = recovery.blocks.last().unwrap();
        let simple_reorg = {
            let sync = self.sync_manager.read().await;
            sync.reorg_handler()
                .check_reorg_weighted(fork_tip, current_tip, last_block_weight)
        };

        if let Some(result) = simple_reorg {
            // In fork recovery: switch if heavier, OR if equal weight and we're solo-producing.
            // A solo fork should yield to the network-majority chain on ties.
            let should_switch =
                result.weight_delta > 0 || (result.weight_delta == 0 && our_fork_is_solo);
            if should_switch {
                info!(
                    "Fork recovery: switching to network chain (delta={}, solo={}) — rollback={}, new={}",
                    result.weight_delta,
                    our_fork_is_solo,
                    result.rollback.len(),
                    result.new_blocks.len()
                );
                let trigger = fork_tip.clone();
                self.execute_reorg(result, trigger).await?;
            } else {
                info!(
                    "Fork is lighter ({}) and not solo — keeping current chain",
                    result.weight_delta
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

        // 5. Execute reorg if fork is heavier, or if tied and we're solo
        match reorg_result {
            Some(result)
                if result.weight_delta > 0 || (result.weight_delta == 0 && our_fork_is_solo) =>
            {
                info!(
                    "Fork recovery: switching to network chain (delta={}, solo={}) — rollback={}, new={}",
                    result.weight_delta,
                    our_fork_is_solo,
                    result.rollback.len(),
                    result.new_blocks.len()
                );
                let trigger = recovery.blocks.last().unwrap().clone();
                self.execute_reorg(result, trigger).await?;
            }
            Some(result) => {
                info!(
                    "Fork is lighter ({}) and not solo — keeping current chain",
                    result.weight_delta
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
    pub(super) async fn try_trigger_fork_recovery(&mut self) {
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
                    info!("Fork recovery triggered from production gate");
                }
            }
        }
    }

    /// Try to apply a chain of cached blocks when we're behind
    ///
    /// This function attempts to build a chain from cached fork blocks
    /// back to our current tip, then applies them in order.
    pub(super) async fn try_apply_cached_chain(&mut self, latest_block: Block) -> Result<()> {
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
                        warn!(
                            "Cached chain rejected block at slot {}: {}",
                            block.header.slot, e
                        );
                        anyhow::bail!("Cached chain contains invalid producer: {}", e);
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
        anyhow::bail!("Could not build complete chain from cached blocks")
    }

    /// Force resync from genesis (devnet/testnet recovery mechanism)
    ///
    /// This is an aggressive recovery mechanism for when a node gets stuck on a fork.
    /// It performs a COMPLETE state reset to genesis and re-syncs from peers.
    ///
    /// ## Defense-in-Depth: Complete State Reset
    ///
    /// This function resets ALL chain-dependent state to ensure semantic completeness:
    /// - chain_state: height, hash, slot → genesis
    /// - utxo_set: cleared completely
    /// - producer_set: cleared (keeping exit_history for anti-Sybil)
    ///
    /// Auto-resync when fork detection has blocked production for too long.
    ///
    /// Triggers `recover_from_peers()` after N consecutive fork-blocked
    /// slots, respecting cooldown to prevent resync loops.
    pub(super) async fn maybe_auto_resync(&mut self, _current_slot: u32) {
        // Threshold: trigger resync after 10 consecutive fork-blocked slots
        // This gives the node enough time for normal recovery (reorg, sync)
        // before escalating to a full resync.
        let fork_resync_threshold: u32 = match self.config.network {
            Network::Devnet => 5,
            _ => 10,
        };

        if self.consecutive_fork_blocks < fork_resync_threshold {
            return;
        }

        // Don't resync at height 0 — there's nothing to resync from.
        // At genesis, fork signals (NoGossipActivity, etc.) are normal startup noise.
        let local_height = {
            let cs = self.chain_state.read().await;
            cs.best_height
        };
        if local_height == 0 {
            debug!("Fork recovery: skipping resync at height 0 (genesis)");
            self.consecutive_fork_blocks = 0;
            return;
        }

        // Respect cooldown (reuse existing exponential backoff logic)
        let (resync_in_progress, consecutive_resyncs) = {
            let sync = self.sync_manager.read().await;
            (
                sync.is_resync_in_progress(),
                sync.consecutive_resync_count(),
            )
        };

        if resync_in_progress {
            debug!("Fork recovery: resync already in progress, waiting");
            return;
        }

        // Fix 2: Hard cap on consecutive resyncs — stop the loop after N failures.
        // Exponential backoff alone is insufficient; after 5 failed resyncs the node
        // needs manual intervention (recover --yes or wipe + resync).
        if consecutive_resyncs >= network::MAX_CONSECUTIVE_RESYNCS {
            warn!(
                "Fork recovery: reached max consecutive resyncs ({}) — manual intervention required. \
                 Run `doli-node recover --yes` or wipe and resync.",
                consecutive_resyncs
            );
            return;
        }

        // Fix 3: Don't interrupt active sync progress. If blocks are being applied,
        // the node is making forward progress and should not be reset to genesis.
        // This prevents the cycle: force_resync → sync to h=720 → fork_blocked → resync → h=0.
        let blocks_applied = {
            let sync = self.sync_manager.read().await;
            sync.blocks_applied()
        };
        if blocks_applied > 0 {
            debug!(
                "Fork recovery: sync in progress ({} blocks applied), skipping resync",
                blocks_applied
            );
            self.consecutive_fork_blocks = 0;
            return;
        }

        // Exponential backoff: 60s * 2^(resyncs), max ~16 min
        let cooldown_secs = 60u64 * (1u64 << consecutive_resyncs.min(4));
        let can_resync = match self.last_resync_time {
            Some(last) => last.elapsed() > Duration::from_secs(cooldown_secs),
            None => true,
        };

        if !can_resync {
            debug!(
                "Fork recovery: cooldown active ({}s remaining)",
                cooldown_secs.saturating_sub(
                    self.last_resync_time
                        .map(|t| t.elapsed().as_secs())
                        .unwrap_or(0)
                )
            );
            return;
        }

        // All networks: try 1-block rollback first (lightweight).
        // If rollback fails, log error — do not wipe state.
        warn!(
            "FORK RECOVERY: {} consecutive fork-blocked slots — rolling back 1 block",
            self.consecutive_fork_blocks
        );
        match self.rollback_one_block().await {
            Ok(true) => {
                info!("Fork recovery: 1-block rollback succeeded");
            }
            Ok(false) => {
                warn!("Fork recovery: rollback not possible — waiting for sync to recover");
            }
            Err(e) => {
                error!(
                    "Fork recovery rollback failed: {} — waiting for sync to recover",
                    e
                );
            }
        }
        self.consecutive_fork_blocks = 0;
        self.last_resync_time = Some(Instant::now());
    }

    /// Apply checkpoint state downloaded from a peer.
    ///
    /// This replaces local state (chain_state, utxo_set, producer_set) with the
    /// received checkpoint data after verifying the state root matches the hardcoded
    /// CHECKPOINT_STATE_ROOT constant compiled into the binary.
    ///
    /// Only called for new nodes (height=0) during initial sync.
    pub(super) async fn apply_checkpoint_state(
        &mut self,
        block_hash: Hash,
        block_height: u64,
        chain_state_bytes: Vec<u8>,
        utxo_set_bytes: Vec<u8>,
        producer_set_bytes: Vec<u8>,
        received_state_root: Hash,
    ) -> Result<()> {
        // 1. Recompute state root from received bytes
        let computed_root = storage::compute_state_root_from_bytes(
            &chain_state_bytes,
            &utxo_set_bytes,
            &producer_set_bytes,
        );
        if computed_root == Hash::ZERO {
            anyhow::bail!("Checkpoint state deserialization failed (computed root = ZERO)");
        }
        if computed_root != received_state_root {
            anyhow::bail!(
                "Checkpoint state root mismatch: computed={} received={}",
                &computed_root.to_string()[..16],
                &received_state_root.to_string()[..16]
            );
        }

        // 2. Verify against hardcoded checkpoint state root
        let expected_root_str = doli_core::consensus::CHECKPOINT_STATE_ROOT;
        let expected_root = Hash::from_hex(expected_root_str).unwrap_or(Hash::ZERO);
        if expected_root == Hash::ZERO {
            // Checkpoint state root not configured (all zeros) — skip verification
            // This is normal for the initial release before any checkpoint is set
            info!(
                "[CHECKPOINT] State root verification skipped (CHECKPOINT_STATE_ROOT not configured)"
            );
        } else if computed_root != expected_root {
            anyhow::bail!(
                "Checkpoint state root doesn't match hardcoded constant: computed={} expected={}",
                &computed_root.to_string()[..16],
                &expected_root.to_string()[..16]
            );
        } else {
            info!(
                "[CHECKPOINT] State root verified: {}",
                &computed_root.to_string()[..16]
            );
        }

        // 3. Deserialize components
        let new_cs: ChainState = bincode::deserialize(&chain_state_bytes)
            .map_err(|e| anyhow::anyhow!("Checkpoint ChainState deserialize failed: {}", e))?;
        let new_ps: storage::ProducerSet = bincode::deserialize(&producer_set_bytes)
            .map_err(|e| anyhow::anyhow!("Checkpoint ProducerSet deserialize failed: {}", e))?;
        // UtxoSet uses canonical format
        let new_utxo = storage::UtxoSet::deserialize_canonical(&utxo_set_bytes)
            .map_err(|e| anyhow::anyhow!("Checkpoint UtxoSet deserialize failed: {}", e))?;

        info!(
            "[CHECKPOINT] Applying state: height={}, hash={}, utxos={}",
            block_height,
            &block_hash.to_string()[..16],
            new_utxo.len()
        );

        // 4. Replace local state
        {
            let mut cs = self.chain_state.write().await;
            *cs = new_cs;
        }
        {
            let mut utxo = self.utxo_set.write().await;
            *utxo = new_utxo;
        }
        {
            let mut ps = self.producer_set.write().await;
            *ps = new_ps;
        }

        // 5. Update sync manager
        {
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(block_height, block_hash, 0);
        }

        info!(
            "[CHECKPOINT] State applied successfully at height={} — resuming normal sync",
            block_height
        );

        Ok(())
    }

    /// Nuclear reset: wipe ALL state INCLUDING block data.
    ///
    /// This is a complete state reset + full block store clear.
    /// Reserved for manual CLI `recover --yes` — NEVER called automatically.
    #[allow(dead_code)]
    pub(super) async fn force_resync_from_genesis(&mut self) -> Result<()> {
        warn!("Force resync initiated - performing COMPLETE state reset to genesis (including block data)");

        // Use canonical chainspec genesis hash, not state_db (may be corrupt).
        let genesis_hash = self.canonical_genesis_hash();

        // Reset sync manager (blocks production via ProductionGate)
        {
            let mut sync = self.sync_manager.write().await;
            sync.reset_local_state(genesis_hash);
        }

        // Reset chain state to genesis
        {
            let mut state = self.chain_state.write().await;
            state.best_height = 0;
            state.best_hash = genesis_hash;
            state.best_slot = 0;
        }

        // Clear UTXO set
        {
            let mut utxo = self.utxo_set.write().await;
            utxo.clear();
        }

        // Clear producer set
        {
            let mut producers = self.producer_set.write().await;
            producers.clear();
        }

        // Atomic clear of StateDb
        {
            let genesis_cs = ChainState::new(genesis_hash);
            self.state_db.clear_and_write_genesis(&genesis_cs);
        }

        // Clear caches
        {
            let mut known = self.known_producers.write().await;
            known.clear();
        }
        {
            let mut cache = self.fork_block_cache.write().await;
            cache.clear();
        }
        {
            let mut detector = self.equivocation_detector.write().await;
            detector.clear();
        }
        {
            let mut gset = self.producer_gset.write().await;
            gset.clear();
        }

        // Reset production timing state
        self.last_produced_slot = None;
        self.first_peer_connected = None;
        self.last_producer_list_change = None;

        // Clear block store entirely (headers, bodies, indexes)
        if let Err(e) = self.block_store.clear() {
            error!("Failed to clear block store during resync: {}", e);
        }

        info!(
            "Complete state reset to genesis (height 0, block store wiped). \
             Production blocked until sync completes + grace period."
        );

        Ok(())
    }
}
