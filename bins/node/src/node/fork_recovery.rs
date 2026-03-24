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
        // Only escalate to full snap sync if rollback fails.
        // This prevents micro-forks from triggering expensive state wipes.
        warn!(
            "FORK RECOVERY: {} consecutive fork-blocked slots — rolling back 1 block",
            self.consecutive_fork_blocks
        );
        match self.rollback_one_block().await {
            Ok(true) => {
                info!("Fork recovery: 1-block rollback succeeded");
            }
            Ok(false) => {
                warn!("Fork recovery: rollback not possible, recovering from peers");
                if let Err(e) = self.force_recover_from_peers().await {
                    error!("Fork recovery failed: {}", e);
                }
            }
            Err(e) => {
                error!("Fork recovery rollback failed: {}", e);
                if let Err(e2) = self.force_recover_from_peers().await {
                    error!("Fork recovery fallback also failed: {}", e2);
                }
            }
        }
        self.consecutive_fork_blocks = 0;
        self.last_resync_time = Some(Instant::now());
    }

    /// Apply a snap sync snapshot: verify state root, replace local state, save to disk.
    pub(super) async fn apply_snap_snapshot(
        &mut self,
        snapshot: network::VerifiedSnapshot,
    ) -> Result<()> {
        info!(
            "[SNAP_SYNC] Applying snapshot: height={}, hash={:.16}, root={:.16}",
            snapshot.block_height, snapshot.block_hash, snapshot.state_root
        );

        // Step 1: Verify state root (node-side, since network crate has no storage dep)
        let computed_root = storage::compute_state_root_from_bytes(
            &snapshot.chain_state,
            &snapshot.utxo_set,
            &snapshot.producer_set,
        );
        if computed_root != snapshot.state_root {
            error!(
                "[SNAP_SYNC] State root mismatch! computed={}, expected={} — blacklisting peer",
                computed_root, snapshot.state_root
            );
            // Can't easily get the peer here, but snap_fallback handles it
            self.sync_manager.write().await.snap_fallback_to_normal();
            return Ok(());
        }

        // Step 2: Deserialize via storage::StateSnapshot (avoids bincode dep in node)
        let snap = storage::StateSnapshot {
            block_hash: snapshot.block_hash,
            block_height: snapshot.block_height,
            chain_state_bytes: snapshot.chain_state,
            utxo_set_bytes: snapshot.utxo_set,
            producer_set_bytes: snapshot.producer_set,
            state_root: snapshot.state_root,
        };
        let (new_chain_state, new_utxo_set, new_producer_set) = snap
            .deserialize()
            .map_err(|e| anyhow::anyhow!("[SNAP_SYNC] Failed to deserialize snapshot: {}", e))?;

        info!(
            "[SNAP_SYNC] Deserialized: chain_state(height={}, hash={:.16})",
            new_chain_state.best_height, new_chain_state.best_hash,
        );

        // C3 defense: envelope must match deserialized state
        if new_chain_state.best_hash != snapshot.block_hash
            || new_chain_state.best_height != snapshot.block_height
        {
            error!(
                "[SNAP_SYNC] Envelope/state mismatch: envelope=({}, {:.16}) vs deserialized=({}, {:.16})",
                snapshot.block_height, snapshot.block_hash,
                new_chain_state.best_height, new_chain_state.best_hash,
            );
            self.sync_manager.write().await.snap_fallback_to_normal();
            return Ok(());
        }

        // Step 3: Replace local state (preserve genesis_hash from our chain)
        let genesis_hash = self.chain_state.read().await.genesis_hash;
        {
            let mut cs = self.chain_state.write().await;
            *cs = new_chain_state;
            cs.genesis_hash = genesis_hash; // Preserve our genesis identity
            cs.mark_snap_synced(snapshot.block_height); // Survives restart: block store empty by design

            // Replace in-memory UTXO set (snap sync always deserializes to InMemory)
            let mut utxo = self.utxo_set.write().await;
            *utxo = new_utxo_set;

            let mut ps = self.producer_set.write().await;
            *ps = new_producer_set;

            // Cache state root atomically while all three write locks are held.
            // No TOCTOU race window possible.
            if let Ok(root) = storage::compute_state_root(&cs, &utxo, &ps) {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, cs.best_hash, cs.best_height));
            }

            // Atomically persist to StateDb (single WriteBatch — crash-safe)
            let utxo_pairs: Vec<_> = match &*utxo {
                UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
                UtxoSet::RocksDb(_) => self.state_db.iter_utxos(),
            };
            if let Err(e) = self
                .state_db
                .atomic_replace(&cs, &ps, utxo_pairs.into_iter())
            {
                error!("[SNAP_SYNC] StateDb atomic_replace failed: {}", e);
            }

            // Update sync manager local tip while chain_state is still locked
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(cs.best_height, cs.best_hash, cs.best_slot);
        }

        // Step 5: Rebuild scheduled flags from block store (INC-I-006 defense-in-depth).
        // After snap sync the store is usually empty so this resets all producers to
        // scheduled=true (safe default). If blocks exist, scheduling converges
        // deterministically — prevents stale peer flags from causing slot mismatches.
        {
            let tip = self.chain_state.read().await.best_height;
            let mut ps = self.producer_set.write().await;
            self.rebuild_scheduled_from_blocks(&mut ps, tip);
        }

        // Step 6: Seed the canonical index so the first post-snap-sync block can
        // call set_canonical_chain without walking into an empty block store.
        self.block_store
            .seed_canonical_index(snapshot.block_hash, snapshot.block_height)?;

        // Step 7: Track snap sync height in-memory for validation mode selection
        self.snap_sync_height = Some(snapshot.block_height);

        // Step 7b: Inform sync manager of block store floor so fork sync
        // won't binary-search below available block data.
        {
            let mut sync = self.sync_manager.write().await;
            sync.set_store_floor(snapshot.block_height);
        }

        // Step 8: Seed reorg handler so fork detection works immediately after snap sync
        {
            let mut sync = self.sync_manager.write().await;
            sync.record_block_applied_after_snap(snapshot.block_hash, snapshot.block_height);
        }

        info!(
            "[SNAP_SYNC] Snapshot applied successfully — now at height {} hash={:.16}",
            snapshot.block_height, snapshot.block_hash
        );

        Ok(())
    }

    /// - known_producers: cleared
    /// - fork_block_cache: cleared
    /// - equivocation_detector: cleared
    /// - producer_gset: cleared
    /// - Production timing state: reset
    ///
    /// Automatic recovery: reset state and let snap sync restore from peers.
    ///
    /// Guard: skips recovery if the node is already at >90% of the network tip,
    /// since a near-tip node should wait for reorg rather than wipe state.
    ///
    /// Calls `reset_state_only()` (layers 1-9 + index clear) then lets the sync
    /// manager pick up snap sync on the next tick (h=0 + gap > threshold + peers >= 3).
    ///
    /// Block data (headers/bodies) is preserved — only indexes are cleared.
    /// This is the ONLY automatic recovery path; `force_resync_from_genesis()`
    /// (which also wipes block data) is reserved for manual `recover --yes`.
    ///
    /// Force recovery bypassing the 90% guard — used when fork sync or deep
    /// fork detection has already proven the node is on a different chain.
    ///
    /// Rate-limited: no more than 1 forced recovery per epoch (360 blocks ≈ 1 hour).
    /// Prevents cascade loops where snap sync → fork → re-snap repeats unbounded.
    pub(super) async fn force_recover_from_peers(&mut self) -> Result<()> {
        // Rate limit: exponential backoff between forced recoveries.
        // 60s → 120s → 300s → 600s → 960s (cap at 4 doublings).
        // Old behavior was a flat 3600s cooldown which left nodes stuck for an hour.
        let cooldown_secs = 60u64 * (1u64 << self.consecutive_forced_recoveries.min(4));
        if let Some(last) = self.last_resync_time {
            let elapsed = last.elapsed().as_secs();
            if elapsed < cooldown_secs {
                warn!(
                    "Recovery: cooldown active — last forced recovery {}s ago \
                     (min {}s, attempt #{} between recoveries). Skipping to prevent cascade.",
                    elapsed, cooldown_secs, self.consecutive_forced_recoveries
                );
                return Ok(());
            }
        }
        self.consecutive_forced_recoveries = self.consecutive_forced_recoveries.saturating_add(1);
        self.last_resync_time = Some(std::time::Instant::now());
        self.recover_from_peers_inner(true).await?;
        self.sync_manager.write().await.set_post_recovery_grace();
        Ok(())
    }

    pub(super) async fn recover_from_peers_inner(&mut self, force: bool) -> Result<()> {
        let local_height = self.chain_state.read().await.best_height;
        let best_peer = self.sync_manager.read().await.best_peer_height();

        // Block 1 missing means block store has a gap. Do NOT snap sync for nodes that
        // were previously synced — it makes the gap worse. BUT allow it for fresh nodes
        // at height 0 that have never synced (initial sync requires snap sync when
        // header-first can't bridge a large gap).
        if self.block_store.get_block_by_height(1)?.is_none() && local_height > 0 {
            warn!(
                "Recovery: block 1 missing at h={} — skipping state reset (would deepen gap). \
                 Header-first sync will recover (peer h={})",
                local_height, best_peer
            );
            return Ok(());
        }

        // Guard: don't wipe a node that's close to the tip — let reorg handle it.
        // Bypassed when force=true (fork sync proved we're on a different chain).
        if !force && best_peer > 0 && local_height > best_peer * 90 / 100 {
            warn!(
                "Recovery: at {}% of network tip (h={}/{}), skipping state wipe — waiting for reorg",
                local_height * 100 / best_peer,
                local_height,
                best_peer
            );
            return Ok(());
        }

        warn!(
            "Recovery{}: resetting state (preserving block data), snap sync will restore (local h={}, peer h={})",
            if force { " (forced)" } else { "" },
            local_height, best_peer
        );

        self.reset_state_only().await?;

        info!(
            "State reset complete (height 0). Block data preserved. \
             Production blocked until sync completes + grace period."
        );

        Ok(())
    }

    /// Reset all mutable state to genesis WITHOUT wiping block data.
    ///
    /// Layers 1-9: sync manager, chain state, UTXO, producers, StateDb, caches.
    /// Block store: only indexes cleared (height_index, slot_index, hash_to_height).
    /// Block data (headers, bodies, presence) preserved for future rollbacks.
    ///
    /// After this, snap sync activates on next tick: h=0 + gap > threshold + peers >= 3.
    pub(super) async fn reset_state_only(&mut self) -> Result<()> {
        // Use canonical chainspec genesis hash, not state_db (may be corrupt).
        let genesis_hash = self.canonical_genesis_hash();

        // INC-I-005 Fix C: Check confirmed height floor BEFORE destroying state.
        // If the sync manager refuses the reset (node was previously healthy),
        // we must NOT proceed with layers 2-9 (chain state, UTXO, producer set).
        // Otherwise the node state is destroyed but the sync manager thinks
        // it's still at the floor height — causing a permanent desync.
        {
            let sync = self.sync_manager.read().await;
            let floor = sync.confirmed_height_floor();
            if floor > 0 {
                drop(sync); // Release read lock before write lock
                warn!(
                    "reset_state_only ABORTED: confirmed_height_floor={}. \
                     Node was previously healthy — will not destroy state. \
                     Clearing sync pipeline only (INC-I-005 Fix C).",
                    floor
                );
                let mut sync = self.sync_manager.write().await;
                sync.reset_local_state(genesis_hash);
                return Ok(());
            }
        }

        // LAYER 1: Reset sync manager (blocks production via ProductionGate)
        {
            let mut sync = self.sync_manager.write().await;
            sync.reset_local_state(genesis_hash);
        }

        // LAYER 2: Reset chain state to genesis
        {
            let mut state = self.chain_state.write().await;
            state.best_height = 0;
            state.best_hash = genesis_hash;
            state.best_slot = 0;
        }

        // LAYER 3: Clear UTXO set (in-memory)
        {
            let mut utxo = self.utxo_set.write().await;
            utxo.clear();
        }

        // LAYER 4: Clear producer set (forces true bootstrap mode)
        {
            let mut producers = self.producer_set.write().await;
            producers.clear();
        }

        // LAYER 4.5: Atomic clear of StateDb (UTXOs + producers + chain state)
        {
            let genesis_cs = ChainState::new(genesis_hash);
            self.state_db.clear_and_write_genesis(&genesis_cs);
        }

        // LAYER 5: Clear known producers list
        {
            let mut known = self.known_producers.write().await;
            known.clear();
        }

        // LAYER 6: Clear fork block cache
        {
            let mut cache = self.fork_block_cache.write().await;
            cache.clear();
        }

        // LAYER 7: Clear equivocation detector
        {
            let mut detector = self.equivocation_detector.write().await;
            detector.clear();
        }

        // LAYER 8: Clear producer discovery CRDT
        {
            let mut gset = self.producer_gset.write().await;
            gset.clear();
        }

        // LAYER 9: Reset production timing state
        self.last_produced_slot = None;
        self.first_peer_connected = None;
        *self.last_producer_list_change.write().await = None;

        // LAYER 9.5: Clear block store INDEXES only (not block data).
        // Stale indexes from fork blocks would pollute get_last_rewarded_epoch()
        // and other height-based queries. Block data stays for future rollbacks.
        if let Err(e) = self.block_store.clear_indexes() {
            error!("Failed to clear block store indexes during recovery: {}", e);
        }

        Ok(())
    }

    /// Nuclear reset: wipe ALL state INCLUDING block data.
    ///
    /// This is `reset_state_only()` + full block store clear.
    /// Reserved for manual CLI `recover --yes` — NEVER called automatically.
    #[allow(dead_code)]
    pub(super) async fn force_resync_from_genesis(&mut self) -> Result<()> {
        warn!("Force resync initiated - performing COMPLETE state reset to genesis (including block data)");

        self.reset_state_only().await?;

        // LAYER 10: Clear block store entirely (headers, bodies, indexes)
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
