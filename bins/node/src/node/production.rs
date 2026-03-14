use super::*;

impl Node {
    // NOTE: try_broadcast_heartbeat removed in deterministic scheduler model
    // Rewards go 100% to block producer via coinbase, no heartbeats needed

    /// Try to produce a block if we're an eligible producer
    pub(super) async fn try_produce_block(&mut self) -> Result<()> {
        let producer_key = match &self.producer_key {
            Some(k) => k,
            None => return Ok(()),
        };

        // VERSION ENFORCEMENT CHECK
        // If an update has been approved and grace period has passed,
        // outdated nodes cannot produce blocks.
        if let Err(blocked) = node_updater::is_production_allowed(&self.config.data_dir) {
            // Log once per minute to avoid spam
            static LAST_WARNING: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let last = LAST_WARNING.load(std::sync::atomic::Ordering::Relaxed);
            if now_secs - last >= 60 {
                LAST_WARNING.store(now_secs, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!("{}", blocked);
            }
            return Ok(());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let current_slot = self.params.timestamp_to_slot(now);

        // Already produced for this slot — skip before any eligibility/scheduler work.
        // The signed_slots DB is the authoritative slashing guard; this is a fast path
        // that avoids wasted eligibility checks, VDF, and block building when the
        // production timer fires again within the same 10s slot.
        if self.last_produced_slot == Some(current_slot as u64) {
            return Ok(());
        }

        // =========================================================================
        // PRODUCTION GATE CHECK - Single source of truth for production safety
        //
        // This is the FIRST and MOST CRITICAL check. The SyncManager's ProductionGate
        // implements defense-in-depth with multiple layers:
        // 1. Explicit block check (invariant violations)
        // 2. Resync-in-progress check
        // 3. Active sync check (downloading headers/bodies)
        // 4. Bootstrap gate (must have fresh peer status before producing)
        // 5. Post-resync grace period with exponential backoff
        // 6. Peer synchronization check (within N slots/heights)
        //
        // ALL checks must pass. This prevents the infinite resync loop bug where
        // nodes at height 0 would produce orphan blocks for far-ahead slots.
        // =========================================================================
        let auth_result = {
            let mut sync_state = self.sync_manager.write().await;
            let result = sync_state.can_produce(current_slot);
            info!(
                "[NODE_PRODUCE] slot={} can_produce result: {:?}",
                current_slot, result
            );
            result
        }; // sync_state guard dropped here — safe to call &mut self methods below

        match auth_result {
            ProductionAuthorization::Authorized => {
                self.consecutive_fork_blocks = 0;
                self.shallow_rollback_count = 0;
                self.consecutive_forced_recoveries = 0;
                info!(
                    "[NODE_PRODUCE] slot={} AUTHORIZED - proceeding",
                    current_slot
                );
            }
            ProductionAuthorization::BlockedSyncing => {
                info!("[NODE_PRODUCE] slot={} BLOCKED: Syncing", current_slot);
                return Ok(());
            }
            ProductionAuthorization::BlockedResync { .. } => {
                info!("[NODE_PRODUCE] slot={} BLOCKED: Resync", current_slot);
                return Ok(());
            }
            ProductionAuthorization::BlockedBehindPeers {
                local_height,
                peer_height,
                height_diff,
            } => {
                // Being behind peers is NOT fork evidence — it's normal sync lag.
                // Never increment fork counter or trigger rollback from BehindPeers.
                // Sync manager handles catching up; rollbacks only make it worse.
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: BehindPeers local_h={} peer_h={} diff={} (not fork, sync will catch up)",
                    current_slot, local_height, peer_height, height_diff
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedAheadOfPeers {
                local_height,
                peer_height,
                height_ahead,
            } => {
                self.consecutive_fork_blocks += 1;
                warn!(
                    "[NODE_PRODUCE] FORK DETECTED via AheadOfPeers: slot={} local_h={} peer_h={} ahead={} (consecutive={})",
                    current_slot, local_height, peer_height, height_ahead, self.consecutive_fork_blocks
                );
                // Try fork recovery first, then auto-resync if threshold exceeded
                self.try_trigger_fork_recovery().await;
                self.maybe_auto_resync(current_slot).await;
                return Ok(());
            }
            ProductionAuthorization::BlockedSyncFailures { failure_count } => {
                // Low sync failure counts (< 50) are normal network churn.
                // But 50+ consecutive failures means our chain has genuinely diverged
                // from ALL peers — every GetHeaders returns empty because no peer
                // recognizes our tip hash. This IS a fork.
                if failure_count >= 50 {
                    self.consecutive_fork_blocks += 1;
                    error!(
                        "[NODE_PRODUCE] FORK DETECTED via persistent SyncFailures: slot={} failures={} (consecutive={})",
                        current_slot, failure_count, self.consecutive_fork_blocks
                    );
                    self.maybe_auto_resync(current_slot).await;
                } else {
                    warn!(
                        "[NODE_PRODUCE] slot={} BLOCKED: SyncFailures (failures={})",
                        current_slot, failure_count
                    );
                }
                return Ok(());
            }
            ProductionAuthorization::BlockedInsufficientPeers {
                peer_count,
                min_required,
            } => {
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: InsufficientPeers - only {} peers (need {})",
                    current_slot, peer_count, min_required
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedChainMismatch {
                peer_id,
                local_hash,
                peer_hash,
                local_height,
            } => {
                self.consecutive_fork_blocks += 1;
                error!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Chain mismatch with peer {} at height {} (local={}, peer={}) (consecutive={})",
                    current_slot, peer_id, local_height, local_hash, peer_hash, self.consecutive_fork_blocks
                );
                // Try fork recovery first, then auto-resync if threshold exceeded
                self.try_trigger_fork_recovery().await;
                self.maybe_auto_resync(current_slot).await;
                return Ok(());
            }
            ProductionAuthorization::BlockedNoGossipActivity {
                seconds_since_gossip,
                peer_count,
            } => {
                // No gossip activity is NOT a definitive fork signal — it can happen
                // during network startup or temporary connectivity issues.
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: No gossip activity for {}s with {} peers",
                    current_slot, seconds_since_gossip, peer_count
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedExplicit { reason } => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Explicit - {}",
                    current_slot, reason
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedBootstrap { reason } => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Bootstrap - {}",
                    current_slot, reason
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedConflictsFinality {
                local_finalized_height,
            } => {
                warn!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Chain conflicts with finalized block at height {}",
                    current_slot, local_finalized_height
                );
                return Ok(());
            }
            ProductionAuthorization::BlockedAwaitingCanonicalBlock => {
                info!(
                    "[NODE_PRODUCE] slot={} BLOCKED: Awaiting first canonical gossip block after snap sync",
                    current_slot
                );
                return Ok(());
            }
        }

        // Log slot info periodically (every ~60 seconds)
        if current_slot.is_multiple_of(60) {
            info!(
                "Production check: now={}, genesis={}, current_slot={}, last_produced={:?}",
                now, self.params.genesis_time, current_slot, self.last_produced_slot
            );
        }

        // EARLY BLOCK EXISTENCE CHECK (optimization)
        // Check if a block already exists for this slot before spending time on eligibility
        // checks and VDF computation. This is safe because:
        // 1. If we see a block, another producer already succeeded
        // 2. We'll check again after VDF to catch blocks that appeared during computation
        if self.block_store.has_block_for_slot(current_slot as u64) {
            debug!(
                "Block already exists for slot {} - skipping production",
                current_slot
            );
            return Ok(());
        }

        // Get chain state
        let state = self.chain_state.read().await;
        let prev_hash = state.best_hash;
        let prev_slot = state.best_slot;
        let height = state.best_height + 1;
        drop(state);

        // Can't produce if slot hasn't advanced
        if current_slot <= prev_slot {
            debug!(
                "Slot not advanced: current_slot={} <= prev_slot={}",
                current_slot, prev_slot
            );
            return Ok(());
        }

        // =========================================================================
        // DEFENSE-IN-DEPTH: Peer-Aware Behind-Network Check
        //
        // Prevents producing orphan blocks when we're significantly behind the
        // network. A node at height 0 should never produce for slot 92 if peers
        // are at height 90 — the block would be an orphan.
        //
        // Key insight: Compare HEIGHTS, not slot-height gap. In sparse chains
        // (genesis phase, network downtime), slots advance via wall-clock while
        // blocks are produced infrequently. A large slot-height gap (e.g.,
        // slot 100,000 at height 175) is normal and does NOT indicate we're
        // behind — it means the chain has been producing sparsely. What matters
        // is whether peers have blocks we're missing.
        //
        // Previous bug: Using current_slot - height as the gap metric caused a
        // permanent deadlock where joining nodes could never produce because
        // the slot-height gap always exceeded max_gap in sparse chains.
        //
        // Thresholds:
        // - Height < 10: max 3 blocks behind (tight - prevent orphan forks)
        // - Height 10+:  max 5 blocks behind (allow propagation delay)
        // =========================================================================
        let network_tip_height = {
            let sync = self.sync_manager.read().await;
            sync.best_peer_height()
        };

        let network_height_ahead = network_tip_height > height.saturating_sub(1);

        if height > 1 && network_height_ahead {
            let blocks_behind = network_tip_height.saturating_sub(height.saturating_sub(1));
            let max_behind: u64 = if height < 10 {
                3 // Tight during early chain - prevent orphan forks
            } else {
                5 // Normal operation - allow some propagation delay
            };

            if blocks_behind > max_behind {
                debug!(
                    "Behind network by {} blocks (next_height={}, network_tip={}) - deferring production",
                    blocks_behind, height, network_tip_height
                );
                return Ok(());
            }
        }

        // Get active producers with bond counts derived from UTXO set.
        // Per WHITEPAPER Section 7: each bond unit = one ticket in the rotation.
        // Use active_producers_at_height to ensure all nodes have the same view -
        // new producers must wait ACTIVATION_DELAY blocks before entering the scheduler.
        let producers = self.producer_set.read().await;
        let active_producers: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        let total_producers = producers.total_count();
        drop(producers);

        // Derive bond counts from UTXO set (source of truth for bonds)
        let utxo = self.utxo_set.read().await;
        let active_with_weights: Vec<(PublicKey, u64)> = active_producers
            .into_iter()
            .map(|pk| {
                let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pk.as_bytes());
                let count = utxo
                    .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                    .max(1) as u64;
                (pk, count)
            })
            .collect();
        drop(utxo);

        // Check if we're in genesis phase (bond-free production)
        let in_genesis = self.config.network.is_in_genesis(height);

        let our_pk = producer_key.public_key();
        let _we_are_active = active_with_weights.iter().any(|(pk, _)| pk == our_pk);

        // =========================================================================
        // INVARIANT CHECK: Detect inconsistent state after resync
        //
        // If we're at a very low height (< 10) but have many active producers with
        // weights, AND we're not in genesis phase, this indicates an inconsistent
        // state - possibly a failed or incomplete resync.
        //
        // After a proper resync via recover_from_peers(), the producer_set
        // should be cleared. If it's not, we're in a dangerous state where
        // production could create orphan blocks.
        //
        // This check catches edge cases like:
        // - Interrupted resync
        // - State corruption
        // - Race conditions in state updates
        // =========================================================================
        if height < 10 && !in_genesis && active_with_weights.len() > 5 {
            error!(
                "INVARIANT VIOLATION: height {} < 10 but {} active producers (total: {}) \
                 outside genesis phase. This indicates inconsistent state - blocking production.",
                height,
                active_with_weights.len(),
                total_producers
            );
            self.sync_manager.write().await.block_production(&format!(
                "invariant violation: height {} with {} active producers outside genesis",
                height,
                active_with_weights.len()
            ));
            return Ok(());
        }

        // Use bootstrap mode if:
        // 1. Still in genesis phase (no bond required), OR
        // 2. No active producers registered (transition block or testnet/devnet)
        let our_pubkey = *producer_key.public_key();
        let genesis_blocks = self.config.network.genesis_blocks();
        let (eligible, our_bootstrap_rank) = if in_genesis || active_with_weights.is_empty() {
            match self.config.network {
                Network::Mainnet if !in_genesis && height > genesis_blocks + 1 => {
                    // After the transition block, mainnet requires registered producers.
                    // Height genesis_blocks+1 is the transition block — produced via bootstrap,
                    // and its apply_block triggers genesis producer registration.
                    return Ok(());
                }
                Network::Mainnet | Network::Testnet | Network::Devnet => {
                    // PRODUCER LIST STABILITY CHECK: Don't produce until the producer list
                    // has been stable for N seconds. This ensures anti-entropy has converged
                    // and all nodes have discovered all producers before production begins.
                    // Without this, a node might start producing after discovering only 2 of 3
                    // producers, leading to incorrect round-robin and chain forks.
                    //
                    // The stability window must be longer than the gossip interval (10 seconds)
                    // to ensure at least one full anti-entropy round has passed without changes.
                    // Use shorter window for devnet to enable faster testing.
                    let producer_list_stability_secs: u64 =
                        if self.config.network == Network::Devnet {
                            3 // Fast for devnet testing
                        } else {
                            15 // Production-like for testnet
                        };
                    if let Some(last_change) = self.last_producer_list_change {
                        let elapsed = last_change.elapsed();
                        if elapsed.as_secs() < producer_list_stability_secs {
                            debug!(
                                "Producer list changed {:?} ago, waiting for stability ({} secs required)...",
                                elapsed, producer_list_stability_secs
                            );
                            return Ok(());
                        }
                    }

                    // BOOTSTRAP NODE CONNECTION CHECK: Wait for peer status before producing
                    // This is bootstrap-specific: seed nodes have no bootstrap config,
                    // joining nodes have bootstrap config and must wait for connection.
                    // (Sync-before-produce checks are handled globally above)
                    //
                    // Skip during genesis: all nodes are bootstrapping together from scratch,
                    // there's no existing chain to sync from and no seed to wait for.
                    let has_bootstrap_nodes = !self.config.bootstrap_nodes.is_empty();
                    if has_bootstrap_nodes && !in_genesis {
                        let sync_state = self.sync_manager.read().await;
                        let peer_count = sync_state.peer_count();
                        let best_peer_height = sync_state.best_peer_height();
                        drop(sync_state);

                        if peer_count == 0 {
                            // We have bootstrap nodes configured but haven't received their status yet
                            // Wait for connection before producing to avoid chain splits
                            debug!(
                                "Bootstrap sync: waiting for peer status (bootstrap nodes configured but no peers yet)"
                            );
                            return Ok(());
                        }

                        // JOINING NODE BOOTSTRAP GUARD: Ensure chain tip is fresh before producing.
                        //
                        // Problem 1: Joining nodes at height 0 may produce before receiving blocks
                        // from the network, creating an isolated fork with different genesis.
                        //
                        // Problem 2: Even at height > 0, joining nodes may produce before receiving
                        // the latest blocks due to network propagation delay. For example:
                        // - Node A produces block at slot 3, height 2
                        // - Node B (at height 1) doesn't receive it before slot 4
                        // - Node B produces at slot 4, height 2 (different parent!) → fork
                        //
                        // Solution: During early bootstrap (height < 10), joining nodes must ensure
                        // their chain tip is recent (within 1 slot of current) before producing.
                        // This gives time for in-flight blocks to arrive.
                        //
                        // The bootstrap_sync_grace_secs timeout allows production to proceed if:
                        // - The network is genuinely starting fresh (no blocks to receive)
                        // - Or this node happens to be the designated producer for early slots

                        let bootstrap_sync_grace_secs: u64 =
                            if self.config.network == Network::Devnet {
                                15 // ~15 slots at 1s/slot for faster devnet testing
                            } else {
                                90 // ~9 slots at 10s/slot for testnet/mainnet
                            };

                        let within_bootstrap_grace =
                            if let Some(first_peer_time) = self.first_peer_connected {
                                first_peer_time.elapsed().as_secs() < bootstrap_sync_grace_secs
                            } else {
                                true // No peers yet - handled above
                            };

                        // BOOTSTRAP MIN HEIGHT: Joining nodes must wait for seed to establish
                        // a canonical prefix before they can produce. This prevents race conditions
                        // where multiple nodes produce competing blocks at height 1-2.
                        //
                        // The seed node (no bootstrap config) is exempt and can produce at any height.
                        // After the seed establishes the first few blocks, joining nodes sync via
                        // gossip and can then participate in production.
                        //
                        // NOTE: height = best_height + 1, so BOOTSTRAP_MIN_HEIGHT = 3 means
                        // joining nodes must have best_height >= 2 (received 2 blocks from seed).
                        const BOOTSTRAP_MIN_HEIGHT: u64 = 3;

                        // BOOTSTRAP TIMEOUT: After waiting too long, allow production anyway.
                        // This handles the case where the seed node failed or is very slow.
                        // Devnet uses shorter timeout for faster iteration.
                        let bootstrap_timeout_secs: u64 = if self.config.network == Network::Devnet
                        {
                            60 // 1 minute for devnet
                        } else {
                            180 // 3 minutes for testnet/mainnet
                        };

                        let past_bootstrap_timeout = self
                            .first_peer_connected
                            .map(|t| t.elapsed().as_secs() >= bootstrap_timeout_secs)
                            .unwrap_or(false);

                        if height < BOOTSTRAP_MIN_HEIGHT
                            && best_peer_height > 0
                            && !past_bootstrap_timeout
                        {
                            debug!(
                                "Joining node at height {}: waiting for seed to establish chain (min_height={}, peer_count={}, best_peer_height={})",
                                height, BOOTSTRAP_MIN_HEIGHT, peer_count, best_peer_height
                            );
                            return Ok(());
                        }

                        if height < BOOTSTRAP_MIN_HEIGHT && past_bootstrap_timeout {
                            warn!(
                                "Bootstrap timeout reached ({}s) - joining node proceeding to produce at height {} (seed may have failed)",
                                bootstrap_timeout_secs, height
                            );
                        }

                        // Check 2: During bootstrap grace, ensure chain tip is fresh
                        // If current_slot - chain_tip_slot > 1, we might be missing in-flight blocks
                        let chain_tip_slot = {
                            let state = self.chain_state.read().await;
                            state.best_slot
                        };
                        let slot_gap = current_slot.saturating_sub(chain_tip_slot);

                        // During bootstrap grace period, require chain tip to be recent (gap <= 1)
                        // Skip this check if we've passed the bootstrap timeout
                        if height > 0
                            && within_bootstrap_grace
                            && slot_gap > 1
                            && !past_bootstrap_timeout
                        {
                            debug!(
                                "Joining node bootstrap: chain tip not fresh (height={}, chain_tip_slot={}, current_slot={}, gap={}), waiting for in-flight blocks",
                                height, chain_tip_slot, current_slot, slot_gap
                            );
                            return Ok(());
                        }

                        // Check 3: ALWAYS compare against best peer height before producing
                        // This prevents nodes from producing orphan blocks even after grace period expires
                        // If we're more than 2 blocks behind the best peer, defer production
                        if best_peer_height > 0 && height + 2 < best_peer_height {
                            debug!(
                                "Behind peers: our height {} vs best peer height {} (diff={}) - deferring production",
                                height, best_peer_height, best_peer_height - height
                            );
                            return Ok(());
                        }
                    }

                    // DISCOVERY GRACE PERIOD for seed nodes ONLY
                    // If we're the seed node (no bootstrap config) and have peers but only know ourselves,
                    // wait for discovery. This prevents the seed node from monopolizing production
                    // before learning about other producers.
                    //
                    // Nodes WITH bootstrap config should produce immediately after syncing,
                    // so they can be discovered by the seed node.
                    let known = self.known_producers.read().await;
                    let known_count = known.len();
                    drop(known);

                    // Get peer count for discovery checks
                    let peer_count = self.sync_manager.read().await.peer_count();

                    // Grace period only applies to seed nodes (no bootstrap config)
                    let is_seed_node = !has_bootstrap_nodes;
                    // Use shorter grace period for devnet to enable faster testing
                    let discovery_grace_secs: u64 = if self.config.network == Network::Devnet {
                        5 // Fast for devnet testing
                    } else {
                        30 // Production-like for testnet
                    };
                    let grace_period_active =
                        if let Some(first_peer_time) = self.first_peer_connected {
                            first_peer_time.elapsed().as_secs() < discovery_grace_secs
                        } else {
                            false
                        };

                    // Seed nodes wait during grace period if they only know themselves
                    if is_seed_node && peer_count > 0 && known_count <= 1 && grace_period_active {
                        debug!(
                            "Seed node discovery: waiting for producer discovery (peers={}, known={}, grace_remaining={}s)",
                            peer_count,
                            known_count,
                            discovery_grace_secs - self.first_peer_connected.map(|t| t.elapsed().as_secs()).unwrap_or(0)
                        );
                        return Ok(());
                    }

                    // LATE JOINER GUARD: Prevent production while isolated from producer discovery.
                    //
                    // Problem: A joining node connects to peers, passes sync checks (maybe everyone
                    // is at height 0), but hasn't received the producer list via anti-entropy yet.
                    // If it produces with only itself in the list, it uses wrong round-robin order.
                    //
                    // Solution: If we're connected to peers but only know about ourselves, wait.
                    // The anti-entropy gossip runs every 10 seconds, so we should learn about
                    // other producers quickly if they exist.
                    //
                    // Exception: If we've waited past the grace period and still only know ourselves,
                    // we're probably the only producer (or others haven't started yet), so proceed.
                    if !is_seed_node && peer_count > 0 && known_count <= 1 && grace_period_active {
                        debug!(
                            "Late joiner guard: waiting for anti-entropy producer discovery (peers={}, known={}, grace_remaining={}s)",
                            peer_count,
                            known_count,
                            discovery_grace_secs - self.first_peer_connected.map(|t| t.elapsed().as_secs()).unwrap_or(0)
                        );
                        return Ok(());
                    }

                    // EQUITABLE BOOTSTRAP MODE: Round-robin based on known peers
                    //
                    // For devnet/testnet to mimic mainnet's equitable distribution,
                    // we need fair rotation among all bootstrap producers. This works by:
                    // 1. Building a sorted list of all known producer pubkeys (us + peers)
                    // 2. Using slot % num_producers to pick the leader for this slot
                    // 3. Only the designated leader produces
                    //
                    // This ensures truly equitable block production regardless of
                    // network latency or hash luck.

                    // Build sorted list of known producers.
                    // Prefer on-chain ProducerSet (deterministic). Fall back to GSet
                    // when the on-chain set is empty (always the case during genesis
                    // blocks 1-360, since producers aren't registered until height 361).
                    let mut known_producers: Vec<PublicKey> = {
                        let producers = self.producer_set.read().await;
                        let on_chain: Vec<PublicKey> = producers
                            .active_producers_at_height(height)
                            .iter()
                            .map(|p| p.public_key)
                            .collect();
                        if !on_chain.is_empty() {
                            on_chain
                        } else {
                            // On-chain set empty (genesis phase). Use GSet as fallback.
                            drop(producers); // release lock before acquiring gset lock
                            let gset_producers = {
                                let gset = self.producer_gset.read().await;
                                gset.active_producers(7200)
                            };
                            if !gset_producers.is_empty() {
                                gset_producers
                            } else {
                                let known = self.known_producers.read().await;
                                known.clone()
                            }
                        }
                    };

                    // Always include ourselves
                    if !known_producers.iter().any(|p| p == &our_pubkey) {
                        known_producers.push(our_pubkey);
                    }

                    // Filter out producers who are registered but haven't passed
                    // ACTIVATION_DELAY yet. Without this, a registration changes the
                    // round-robin denominator (slot % N) before all nodes agree on N,
                    // causing forks when new producers join mid-chain.
                    //
                    // Exception: genesis producers (registered_at == 0) are always
                    // included — they ARE the initial set and waiting would deadlock.
                    {
                        let producers = self.producer_set.read().await;
                        known_producers.retain(|pk| {
                            match producers.get_by_pubkey(pk) {
                                Some(info) => {
                                    if !info.is_active() {
                                        return false;
                                    }
                                    // Genesis producers: always eligible
                                    if info.registered_at == 0 {
                                        return true;
                                    }
                                    // Late joiners: must wait activation delay
                                    height >= info.registered_at + storage::ACTIVATION_DELAY
                                }
                                None => {
                                    // Not registered (gossip-discovered): include in bootstrap
                                    true
                                }
                            }
                        });
                    }

                    // Sort for deterministic ordering (all nodes compute same order)
                    known_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                    let num_producers = known_producers.len();
                    if num_producers == 0 {
                        warn!("Bootstrap round-robin: no eligible producers after filtering, skipping slot {}", current_slot);
                        return Ok(());
                    }

                    // Liveness filter: split producers into live vs stale.
                    // Live = produced a block within the liveness window.
                    // Stale = haven't produced recently (slots wasted waiting for them).
                    let liveness_window = std::cmp::max(
                        consensus::LIVENESS_WINDOW_MIN,
                        (num_producers as u64).saturating_mul(3),
                    );
                    let chain_height = height.saturating_sub(1);
                    let cutoff = chain_height.saturating_sub(liveness_window);
                    let (live_producers, stale_producers): (Vec<PublicKey>, Vec<PublicKey>) = {
                        let (live, stale): (Vec<_>, Vec<_>) =
                            known_producers.iter().partition(|pk| {
                                match self.producer_liveness.get(pk) {
                                    Some(&last_h) => last_h >= cutoff,
                                    // No chain record: live if chain is young, stale otherwise
                                    None => chain_height < liveness_window,
                                }
                            });
                        (
                            live.into_iter().copied().collect(),
                            stale.into_iter().copied().collect(),
                        )
                    };

                    // Deadlock safety: if all stale, treat all as live
                    let (live_producers, stale_producers) = if live_producers.is_empty() {
                        (known_producers.clone(), Vec::new())
                    } else {
                        (live_producers, stale_producers)
                    };

                    if !stale_producers.is_empty() {
                        debug!(
                            "Liveness filter: {} live, {} stale (window={})",
                            live_producers.len(),
                            stale_producers.len(),
                            liveness_window,
                        );
                    }

                    // Build eligible list using liveness-aware scheduling.
                    // Normal slots: all ranks from live producers only.
                    // Re-entry slots: one stale producer at rank 0, live at ranks 1+.
                    let eligible = doli_core::validation::bootstrap_schedule_with_liveness(
                        current_slot,
                        &live_producers,
                        &stale_producers,
                        consensus::REENTRY_INTERVAL,
                    );

                    if !eligible.contains(&our_pubkey) {
                        return Ok(());
                    }

                    debug!(
                        "Bootstrap fallback: slot={}, {} producers, eligible={}",
                        current_slot,
                        num_producers,
                        eligible.len()
                    );

                    // Pass eligible list to the standard time-window check below.
                    // our_bootstrap_rank = None means it uses is_producer_eligible_ms
                    // (the same 2s exclusive windows as the epoch scheduler).
                    (eligible, None::<u8>)
                }
            }
        } else {
            // EPOCH LOOKAHEAD: Deterministic round-robin selection (anti-grinding)
            //
            // Selection is based ONLY on:
            // 1. Slot number (known in advance)
            // 2. Sorted producer list with bond weights (fixed at epoch start)
            //
            // NO dependency on prev_hash = NO grinding possible.
            // This is the core insight: "Proof of Longevity" over time,
            // not "Proof of Delay" via long VDF.
            //
            // Algorithm: slot % total_bonds -> deterministic ticket assignment
            // Uses cached DeterministicScheduler for O(log n) lookups per slot.
            let current_epoch = self.params.slot_to_epoch(current_slot) as u64;
            let active_count = active_with_weights.len();
            let total_bonds: u64 = active_with_weights.iter().map(|(_, b)| *b).sum();
            let scheduler = match &self.cached_scheduler {
                Some((epoch, count, bonds, sched))
                    if *epoch == current_epoch
                        && *count == active_count
                        && *bonds == total_bonds =>
                {
                    sched
                }
                _ => {
                    // Build new scheduler (epoch changed or active producer set changed)
                    info!(
                        "Rebuilding scheduler: epoch={}, producers={}, total_bonds={}",
                        current_epoch, active_count, total_bonds
                    );
                    let producers: Vec<ScheduledProducer> = active_with_weights
                        .iter()
                        .map(|(pk, bonds)| ScheduledProducer::new(*pk, *bonds as u32))
                        .collect();
                    self.cached_scheduler = Some((
                        current_epoch,
                        active_count,
                        total_bonds,
                        DeterministicScheduler::new(producers),
                    ));
                    &self.cached_scheduler.as_ref().unwrap().3
                }
            };
            // Dedup: a producer may appear at multiple ranks (small producer sets).
            // Without dedup, position() returns the first occurrence, masking later ranks.
            // This matches the dedup in select_producer_for_slot() (consensus.rs).
            let mut eligible: Vec<PublicKey> =
                Vec::with_capacity(self.config.network.params().max_fallback_ranks);
            for rank in 0..self.config.network.params().max_fallback_ranks {
                if let Some(pk) = scheduler.select_producer(current_slot, rank).cloned() {
                    if !eligible.contains(&pk) {
                        eligible.push(pk);
                    }
                }
            }
            (eligible, None)
        };

        if eligible.is_empty() {
            return Ok(());
        }

        // Calculate slot offset in milliseconds for eligibility window
        let slot_start = self.params.slot_to_timestamp(current_slot);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let slot_start_ms = slot_start * 1000;
        let slot_offset_ms = now_ms.saturating_sub(slot_start_ms);

        // Check if we're eligible at this time
        // For bootstrap mode, use continuous time-based scheduling
        // For normal mode, use the standard eligibility check (ms-precision)
        let is_eligible = if let Some(score) = our_bootstrap_rank {
            // Bootstrap mode: continuous time-based scheduling
            // The score (0-255) determines when we should produce within the slot.
            // We can produce when the current time offset exceeds our target offset.
            let slot_duration_ms = self.params.slot_duration * 1000;
            let max_offset_percent = 80; // Leave 20% for propagation
            let target_offset_percent = (score as u64 * max_offset_percent) / 255;
            let target_offset_ms = (slot_duration_ms * target_offset_percent) / 100;

            // We're eligible if current offset >= our target offset
            slot_offset_ms >= target_offset_ms
        } else {
            // Normal mode: ms-precision sequential eligibility check
            consensus::is_producer_eligible_ms(&our_pubkey, &eligible, slot_offset_ms)
        };

        if !is_eligible {
            return Ok(());
        }

        // For devnet, add a minimum delay to allow heartbeat collection
        // Scale delay to 20% of slot duration (capped at 700ms for long slots)
        // This prevents the delay from consuming too much of short slots
        if self.config.network == Network::Devnet {
            let slot_duration_ms = self.params.slot_duration * 1000;
            let heartbeat_collection_ms = std::cmp::min(slot_duration_ms / 5, 700);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let slot_start_ms = slot_start * 1000;
            let offset_ms = now_ms.saturating_sub(slot_start_ms);
            if offset_ms < heartbeat_collection_ms {
                return Ok(()); // Too early, wait for heartbeats
            }
        }

        // We're eligible - produce a block!
        info!(
            "Producing block for slot {} at height {} (offset {}ms)",
            current_slot, height, slot_offset_ms
        );

        // =========================================================================
        // PROPAGATION RACE MITIGATION: Yield before VDF to catch in-flight blocks
        //
        // Problem: During VDF computation (~55ms), the event loop is blocked and
        // cannot process incoming gossip blocks. If another producer's block for
        // slot S is in-flight while we start producing for slot S+1, we won't see
        // it until after we've already broadcast our block, creating a fork.
        //
        // Solution: Before starting VDF computation, yield control briefly to allow
        // any pending network events to be processed. This gives in-flight blocks
        // a chance to arrive before we commit to production.
        //
        // We yield if:
        // 1. Network tip slot suggests there might be a recent block we haven't seen
        // 2. We're not too far into the slot (leave time for our own production)
        //
        // This is a lightweight "micro-sync" that doesn't require protocol changes.
        // =========================================================================
        {
            let network_tip_slot = self.sync_manager.read().await.best_peer_slot();

            // If network tip is at current_slot-1 or current_slot, there might be
            // an in-flight block we should wait for before producing
            if network_tip_slot >= prev_slot && current_slot > prev_slot + 1 {
                // We're potentially missing a block - yield briefly
                debug!(
                    "Propagation race mitigation: yielding before VDF (prev_slot={}, network_tip={}, current={})",
                    prev_slot, network_tip_slot, current_slot
                );

                // Yield for a short time to allow pending network events to be processed
                // This is much shorter than a full slot - just enough for gossip propagation
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

                // Re-check: did a block arrive for a more recent slot?
                let new_state = self.chain_state.read().await;
                let new_prev_slot = new_state.best_slot;
                drop(new_state);

                if new_prev_slot > prev_slot {
                    debug!(
                        "Block arrived during yield (prev_slot: {} -> {}) - restarting production check",
                        prev_slot, new_prev_slot
                    );
                    // A block arrived - abort this production attempt
                    // Next production tick will re-evaluate with updated chain state
                    return Ok(());
                }
            }
        }

        // SIGNED SLOTS PROTECTION: Check if we already signed this slot
        // This prevents double-signing if we restart quickly
        if let Some(ref signed_slots) = self.signed_slots_db {
            if let Err(e) = signed_slots.check_and_mark(current_slot as u64) {
                error!("SLASHING PROTECTION: {}", e);
                return Ok(());
            }
        }

        // Build the block — single transaction list used for both merkle root and block body.
        // All transactions MUST be added to the builder BEFORE build(), which computes the
        // merkle root from exactly the transactions that will be in the final block.
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let mut builder =
            BlockBuilder::new(prev_hash, prev_slot, our_pubkey).with_params(self.params.clone());

        // ALL blocks: coinbase goes to reward pool (built-in mining pool).
        // No producer can spend rewards early — only the consensus engine distributes
        // pool funds to qualified producers at epoch boundaries.
        builder.add_coinbase(height, pool_hash);

        if !self.config.network.is_in_genesis(height) {
            // POST-GENESIS: distribute pool at epoch boundaries.
            let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
            if height > 0 && reward_epoch::is_epoch_start_with(height, blocks_per_epoch) {
                let completed_epoch = (height / blocks_per_epoch) - 1;

                // Skip epoch 0: genesis bonds consumed pool funds, remainder carries to E1.
                // Distributing E0 would create coins from nothing (pool drained by bonds).
                if completed_epoch == 0 {
                    info!("Epoch 0 (genesis): pool remainder carries to E1, no distribution");
                } else {
                    info!(
                        "Epoch {} completed at height {}, distributing pool rewards...",
                        completed_epoch, height
                    );

                    let epoch_outputs = self.calculate_epoch_rewards(completed_epoch).await;

                    if !epoch_outputs.is_empty() {
                        let total_reward: u64 = epoch_outputs.iter().map(|(amt, _)| *amt).sum();
                        info!(
                            "Distributing {} total reward to {} qualified producers for epoch {}",
                            total_reward,
                            epoch_outputs.len(),
                            completed_epoch
                        );

                        let coinbase = Transaction::new_epoch_reward_coinbase(
                            epoch_outputs,
                            height,
                            completed_epoch,
                        );
                        builder.add_transaction(coinbase);
                    } else {
                        debug!(
                            "No qualified producers in epoch {} — pool accumulates to next epoch",
                            completed_epoch
                        );
                    }
                } // else completed_epoch != 0
            }
        }

        // During genesis: include VDF proof Registration TX in EVERY block we produce.
        // Idempotent — derive_genesis_producers_from_chain() deduplicates by public key.
        // Must not use a one-shot flag: orphaned blocks would set it permanently,
        // preventing retry in the canonical chain.
        if let Some(vdf_output_bytes) = self.genesis_vdf_output {
            if self.config.network.is_in_genesis(height) {
                let (bls_pubkey_bytes, bls_pop_bytes) = if let Some(ref bls_kp) = self.bls_key {
                    let pop = bls_kp
                        .proof_of_possession()
                        .expect("PoP signing cannot fail");
                    (
                        bls_kp.public_key().as_bytes().to_vec(),
                        pop.as_bytes().to_vec(),
                    )
                } else {
                    (vec![], vec![])
                };
                let reg_data = RegistrationData {
                    public_key: our_pubkey,
                    epoch: 0,
                    vdf_output: vdf_output_bytes.to_vec(),
                    vdf_proof: vec![],
                    prev_registration_hash: Hash::ZERO,
                    sequence_number: 0,
                    bond_count: 0, // Zero bond — handled at genesis end
                    bls_pubkey: bls_pubkey_bytes,
                    bls_pop: bls_pop_bytes,
                };
                let extra_data = bincode::serialize(&reg_data)
                    .expect("RegistrationData serialization cannot fail");
                let reg_tx = Transaction {
                    version: 1,
                    tx_type: TxType::Registration,
                    inputs: vec![],
                    outputs: vec![],
                    extra_data,
                };
                builder.add_transaction(reg_tx);
                info!(
                    "Included genesis VDF proof Registration TX in block {} for {}",
                    height,
                    hex::encode(&our_pubkey.as_bytes()[..8])
                );
            }
        }

        // Add transactions from mempool (validate covenant conditions before inclusion)
        let mempool_txs: Vec<Transaction> = {
            let mempool = self.mempool.read().await;
            mempool.select_for_block(1_000_000) // Up to ~1MB of transactions per block
        };
        {
            let utxo = self.utxo_set.read().await;
            let utxo_ctx = validation::ValidationContext::new(
                ConsensusParams::for_network(self.config.network),
                self.config.network,
                0,
                height,
            );
            for tx in &mempool_txs {
                if let Err(e) = validation::validate_transaction_with_utxos(tx, &utxo_ctx, &*utxo) {
                    warn!(
                        "Skipping mempool tx {} — UTXO validation failed: {}",
                        tx.hash(),
                        e
                    );
                    continue;
                }
                builder.add_transaction(tx.clone());
            }
        }

        // Recapture timestamp just before building the block.
        // The original `now` (line ~4508) can be 1-2 seconds stale due to async
        // work between capture and here (GSet reads, liveness filtering, VDF, etc).
        // If the stale timestamp maps to a different 2s rank window than the one
        // we passed eligibility for, validation rejects our own block as
        // "invalid producer for slot". Using a fresh timestamp ensures the block's
        // header.timestamp matches the rank window we were actually eligible in.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Sanity: abort if we crossed into a different slot during production
        if self.params.timestamp_to_slot(now) != current_slot {
            warn!(
                "Slot boundary crossed during block production (started slot {}, now slot {}) - aborting",
                current_slot,
                self.params.timestamp_to_slot(now)
            );
            return Ok(());
        }

        // Build attestation bitfield for presence_root.
        // Records which producers attested the current minute (from gossip).
        // 6 blocks per minute from different producers → union mitigates censorship.
        let current_minute = attestation_minute(current_slot);
        let attested_pks = self.minute_tracker.attested_in_minute(current_minute);
        let presence_root = if attested_pks.is_empty() {
            Hash::ZERO
        } else {
            // Build sorted producer list (same ordering as DeterministicScheduler)
            // Use height-aware method to match the decoding path in calculate_epoch_rewards()
            let sorted_producers: Vec<storage::producer::ProducerInfo> = {
                let producers = self.producer_set.read().await;
                let mut ps: Vec<storage::producer::ProducerInfo> = producers
                    .active_producers_at_height(height)
                    .iter()
                    .map(|p| (*p).clone())
                    .collect();
                ps.sort_by(|a, b| a.public_key.as_bytes().cmp(b.public_key.as_bytes()));
                ps
            };
            // Map attesting pubkeys to sorted indices
            let mut attested_indices = Vec::new();
            for pk in &attested_pks {
                if let Some(idx) = sorted_producers.iter().position(|p| &p.public_key == *pk) {
                    attested_indices.push(idx);
                }
            }
            encode_attestation_bitfield(&attested_indices)
        };
        let builder = builder.with_presence_root(presence_root);

        // Build header + finalized transaction list. The merkle root is computed from
        // exactly these transactions, guaranteeing header-body consistency.
        let (header, transactions) = match builder.build(now) {
            Some(result) => result,
            None => {
                warn!("Failed to build block - slot monotonicity violation");
                return Ok(());
            }
        };

        // =========================================================================
        // DRAIN PENDING BLOCKS: Process all queued network events before VDF
        //
        // This is the key fix for the propagation race bug. The problem:
        // - VDF computation takes ~550-700ms
        // - During this time, the event loop is blocked (we await spawn_blocking)
        // - Blocks from other producers may arrive in the network channel queue
        // - Without draining, we don't see them until after we've already produced
        //
        // Solution: Non-blocking drain of all pending NewBlock events from the
        // network channel BEFORE starting VDF. This ensures we have the latest
        // chain state before committing to production.
        //
        // The try_next_event() method uses try_recv() which returns immediately
        // if no events are queued, so this adds negligible latency.
        // =========================================================================
        {
            // First, collect pending events (releases borrow of self.network quickly)
            let pending_events: Vec<NetworkEvent> = {
                if let Some(ref mut network) = self.network {
                    let mut events = Vec::new();
                    // Drain ALL pending events (not just 10)
                    // With 20+ nodes, limiting to 10 can leave blocks unprocessed
                    while let Some(event) = network.try_next_event() {
                        events.push(event);
                    }
                    events
                } else {
                    Vec::new()
                }
            };

            // Now process collected events (self.network borrow is released)
            if !pending_events.is_empty() {
                let mut block_count = 0;
                for event in pending_events {
                    if matches!(event, NetworkEvent::NewBlock(_, _)) {
                        block_count += 1;
                    }
                    if let Err(e) = self.handle_network_event(event).await {
                        warn!("Error handling drained event: {}", e);
                    }
                }

                if block_count > 0 {
                    debug!("Drained {} pending block events before VDF", block_count);

                    // Check if chain advanced during drain
                    let new_state = self.chain_state.read().await;
                    if new_state.best_slot > prev_slot || new_state.best_height >= height {
                        debug!(
                            "Chain advanced during drain (new tip after prev_slot={}) - aborting production",
                            prev_slot
                        );
                        return Ok(());
                    }
                }
            }
        }

        // Compute VDF using hash-chain with dynamically calibrated iterations
        // The VDF serves as anti-grinding protection, not timing enforcement
        // For devnet, VDF is disabled to enable rapid development
        let (vdf_output, vdf_proof) = if self.config.network.vdf_enabled() {
            // Construct VDF input from block context: prev_hash || tx_root || slot || producer_key
            let vdf_input = construct_vdf_input(
                &prev_hash,
                &header.merkle_root,
                header.slot,
                &header.producer,
            );

            // Get network-specific fixed iterations (must match validation)
            let iterations = self.config.network.heartbeat_vdf_iterations();
            info!(
                "Computing hash-chain VDF with {} iterations (network={:?})...",
                iterations, self.config.network
            );

            // Use hash-chain VDF with calibrated iterations
            let vdf_input_clone = vdf_input;
            let vdf_start = std::time::Instant::now();
            let output_bytes =
                tokio::task::spawn_blocking(move || hash_chain_vdf(&vdf_input_clone, iterations))
                    .await
                    .map_err(|e| anyhow::anyhow!("VDF task failed: {}", e))?;
            let vdf_duration = vdf_start.elapsed();

            // Record timing for calibration
            self.vdf_calibrator
                .write()
                .await
                .record_timing(iterations, vdf_duration);

            info!("VDF computed in {:?} (target: ~55ms)", vdf_duration);
            (
                VdfOutput {
                    value: output_bytes.to_vec(),
                },
                VdfProof::empty(), // Hash-chain VDF is self-verifying
            )
        } else {
            // VDF disabled for this network - use placeholder values
            info!(
                "VDF disabled for {:?} network, using placeholder",
                self.config.network
            );
            (
                VdfOutput {
                    value: prev_hash.as_bytes().to_vec(),
                },
                VdfProof::empty(),
            )
        };

        // SAFETY CHECK: Verify chain state hasn't changed during VDF computation
        //
        // The VDF takes ~700ms during which the event loop is blocked. Other
        // producers' blocks may have arrived and been queued. We must check:
        // 1. No block exists for our current slot (same-slot duplicate)
        // 2. Chain tip hasn't advanced (stale parent detection)
        //
        // Without check #2, we'd build on a stale parent and create a fork.
        if self.block_store.has_block_for_slot(current_slot as u64) {
            debug!(
                "Block appeared during VDF computation for slot {} - aborting production",
                current_slot
            );
            return Ok(());
        }

        // Check if chain tip advanced during VDF (stale parent detection)
        {
            let post_vdf_state = self.chain_state.read().await;
            if post_vdf_state.best_height >= height || post_vdf_state.best_hash != prev_hash {
                info!(
                    "Chain advanced during VDF computation (tip moved from height {} to {}) - aborting to avoid fork",
                    height - 1, post_vdf_state.best_height
                );
                return Ok(());
            }
        }

        // Create final block header with VDF
        let final_header = BlockHeader {
            version: header.version,
            prev_hash: header.prev_hash,
            merkle_root: header.merkle_root,
            presence_root: header.presence_root,
            genesis_hash: header.genesis_hash,
            timestamp: header.timestamp,
            slot: header.slot,
            producer: header.producer,
            vdf_output,
            vdf_proof,
        };

        // Use the transactions from the builder — same list used for merkle root computation.
        // No duplicate transaction assembly needed.
        // Aggregate BLS signatures from minute tracker for on-chain proof.
        // Only includes producers that have BLS sigs for this minute.
        let aggregate_bls_signature = {
            let bls_sigs = self.minute_tracker.bls_sigs_for_minute(current_minute);
            info!(
                "BLS aggregate: minute={} sigs_count={} tracker_bls_total={}",
                current_minute,
                bls_sigs.len(),
                self.minute_tracker.bls_sig_count()
            );
            if bls_sigs.is_empty() {
                Vec::new()
            } else {
                let sigs: Vec<crypto::BlsSignature> = bls_sigs
                    .iter()
                    .filter_map(|(_, raw)| crypto::BlsSignature::try_from_slice(raw).ok())
                    .collect();
                if sigs.is_empty() {
                    Vec::new()
                } else {
                    match crypto::bls_aggregate(&sigs) {
                        Ok(agg) => agg.as_bytes().to_vec(),
                        Err(e) => {
                            warn!("BLS aggregation failed: {}", e);
                            Vec::new()
                        }
                    }
                }
            }
        };

        let block = Block {
            header: final_header,
            transactions,
            aggregate_bls_signature,
        };

        let block_hash = block.hash();
        info!(
            "[BLOCK_PRODUCED] hash={} height={} slot={} parent={}",
            block_hash, height, current_slot, block.header.prev_hash
        );

        // Apply the block locally
        self.apply_block(block.clone(), ValidationMode::Full)
            .await?;

        // NOTE: Do NOT call note_block_received_via_gossip() here.
        // Self-produced blocks must not reset the solo production watchdog —
        // otherwise a node that loses gossip connectivity will produce solo
        // indefinitely, deepening a fork without any circuit breaker firing.
        //
        // Similarly, do NOT call refresh_all_peers() — refreshing all peer
        // timestamps after self-production masks actually-stale peers, preventing
        // stale chain detection from triggering.

        // Mark that we produced for this slot
        self.last_produced_slot = Some(current_slot as u64);

        // Broadcast the block to the network
        // This is only done for blocks we produce ourselves - received blocks
        // are already on the network and don't need to be re-broadcast.
        // Header is sent first so peers can pre-validate before the full block arrives.
        if let Some(ref network) = self.network {
            let _ = network.broadcast_header(block.header.clone()).await;
            let _ = network.broadcast_block(block).await;
        }

        // Attest our own block for finality gadget + record in minute tracker.
        self.create_and_broadcast_attestation(block_hash, current_slot, height)
            .await;
        if let Some(ref kp) = self.producer_key {
            let minute = attestation_minute(current_slot);
            if let Some(ref bls_kp) = self.bls_key {
                let bls_msg = crypto::attestation_message(&block_hash, current_slot);
                let bls_sig = crypto::bls_sign(&bls_msg, bls_kp.secret_key())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();
                self.minute_tracker
                    .record_with_bls(*kp.public_key(), minute, bls_sig);
            } else {
                self.minute_tracker.record(*kp.public_key(), minute);
            }
        }

        // Flush any blocks that just reached finality
        self.flush_finalized_to_archive().await;

        Ok(())
    }
}
