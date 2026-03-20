use super::*;

impl Node {
    /// Determine eligible producers using bootstrap mode (genesis/no registered producers).
    /// Returns `Some((eligible, optional_rank))` or `None` to skip this slot.
    pub(super) async fn resolve_bootstrap_eligibility(
        &mut self,
        current_slot: u32,
        height: u64,
        our_pubkey: PublicKey,
        in_genesis: bool,
    ) -> Option<(Vec<PublicKey>, Option<u8>)> {
        let genesis_blocks = self.config.network.genesis_blocks();

        match self.config.network {
            Network::Mainnet if !in_genesis && height > genesis_blocks + 1 => {
                // After the transition block, mainnet requires registered producers.
                // Height genesis_blocks+1 is the transition block — produced via bootstrap,
                // and its apply_block triggers genesis producer registration.
                None
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
                let producer_list_stability_secs: u64 = if self.config.network == Network::Devnet {
                    3 // Fast for devnet testing
                } else {
                    15 // Production-like for testnet
                };
                if let Some(last_change) = *self.last_producer_list_change.read().await {
                    let elapsed = last_change.elapsed();
                    if elapsed.as_secs() < producer_list_stability_secs {
                        debug!(
                            "Producer list changed {:?} ago, waiting for stability ({} secs required)...",
                            elapsed, producer_list_stability_secs
                        );
                        return None;
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
                        return None;
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
                    // - Node B produces at slot 4, height 2 (different parent!) -> fork
                    //
                    // Solution: During early bootstrap (height < 10), joining nodes must ensure
                    // their chain tip is recent (within 1 slot of current) before producing.
                    // This gives time for in-flight blocks to arrive.
                    //
                    // The bootstrap_sync_grace_secs timeout allows production to proceed if:
                    // - The network is genuinely starting fresh (no blocks to receive)
                    // - Or this node happens to be the designated producer for early slots

                    let bootstrap_sync_grace_secs: u64 = if self.config.network == Network::Devnet {
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
                    let bootstrap_timeout_secs: u64 = if self.config.network == Network::Devnet {
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
                        return None;
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
                        return None;
                    }

                    // Check 3: ALWAYS compare against best peer height before producing
                    // This prevents nodes from producing orphan blocks even after grace period expires
                    // If we're more than 2 blocks behind the best peer, defer production
                    if best_peer_height > 0 && height + 2 < best_peer_height {
                        debug!(
                            "Behind peers: our height {} vs best peer height {} (diff={}) - deferring production",
                            height, best_peer_height, best_peer_height - height
                        );
                        return None;
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
                let grace_period_active = if let Some(first_peer_time) = self.first_peer_connected {
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
                    return None;
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
                    return None;
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
                // INC-001 RC-5A: During genesis, use HARDCODED genesis producers.
                // The on-chain ProducerSet may be empty and the GSet has DIFFERENT
                // contents on different nodes (anti-entropy hasn't converged).
                // Both cause divergent schedulers → competing blocks → forks.
                // Hardcoded genesis producers are IDENTICAL on all nodes.
                let mut known_producers: Vec<PublicKey> = if in_genesis {
                    let genesis_producers = match self.config.network {
                        Network::Testnet => doli_core::genesis::testnet_genesis_producers()
                            .into_iter()
                            .map(|(pk, _)| pk)
                            .collect::<Vec<_>>(),
                        Network::Mainnet => doli_core::genesis::mainnet_genesis_producers()
                            .into_iter()
                            .map(|(pk, _)| pk)
                            .collect::<Vec<_>>(),
                        Network::Devnet => Vec::new(),
                    };
                    if !genesis_producers.is_empty() {
                        info!(
                            "[SCHED] Genesis mode: using {} hardcoded producers (deterministic)",
                            genesis_producers.len()
                        );
                        genesis_producers
                    } else {
                        // Devnet fallback: use on-chain or GSet
                        let producers = self.producer_set.read().await;
                        let on_chain: Vec<PublicKey> = producers
                            .active_producers_at_height(height)
                            .iter()
                            .map(|p| p.public_key)
                            .collect();
                        if !on_chain.is_empty() {
                            on_chain
                        } else {
                            drop(producers);
                            let gset = self.producer_gset.read().await;
                            let gp = gset.active_producers(7200);
                            if !gp.is_empty() { gp } else {
                                let known = self.known_producers.read().await;
                                known.clone()
                            }
                        }
                    }
                } else {
                    let producers = self.producer_set.read().await;
                    let on_chain: Vec<PublicKey> = producers
                        .active_producers_at_height(height)
                        .iter()
                        .map(|p| p.public_key)
                        .collect();
                    if !on_chain.is_empty() {
                        on_chain
                    } else {
                        drop(producers);
                        let gset = self.producer_gset.read().await;
                        let gp = gset.active_producers(7200);
                        if !gp.is_empty() { gp } else {
                            let known = self.known_producers.read().await;
                            known.clone()
                        }
                    }
                };

                // Include ourselves if not already in the list.
                // The GSet may contain stale entries from remote bootstrap nodes that
                // don't match our local producer keys. In that case, replace the GSet
                // list entirely with just the locally-known producers.
                if !known_producers.iter().any(|p| p == &our_pubkey) {
                    // Our pubkey is not in the GSet — the GSet likely has stale entries
                    // from a previous genesis or remote nodes. Fall back to the locally
                    // known producers list which is populated from peer discovery.
                    let local_known = self.known_producers.read().await;
                    if !local_known.is_empty() {
                        known_producers = local_known.clone();
                    }
                    // Always include ourselves
                    if !known_producers.iter().any(|p| p == &our_pubkey) {
                        known_producers.push(our_pubkey);
                    }
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
                    return None;
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
                    let (live, stale): (Vec<_>, Vec<_>) = known_producers.iter().partition(|pk| {
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
                    return None;
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
                Some((eligible, None::<u8>))
            }
        }
    }

    /// Determine eligible producers using epoch-based deterministic scheduler.
    ///
    /// EPOCH LOOKAHEAD: Deterministic round-robin selection (anti-grinding)
    ///
    /// Selection is based ONLY on:
    /// 1. Slot number (known in advance)
    /// 2. Sorted producer list with bond weights (fixed at epoch start)
    ///
    /// NO dependency on prev_hash = NO grinding possible.
    /// This is the core insight: "Proof of Longevity" over time,
    /// not "Proof of Delay" via long VDF.
    ///
    /// Algorithm: slot % total_bonds -> deterministic ticket assignment
    /// Uses cached DeterministicScheduler for O(log n) lookups per slot.
    pub(super) fn resolve_epoch_eligibility(
        &mut self,
        current_slot: u32,
        _height: u64,
        active_with_weights: &[(PublicKey, u64)],
    ) -> Vec<PublicKey> {
        // REMOVED: Liveness filter from epoch scheduler.
        // producer_liveness is local state — different on each node at different heights.
        // Filtering out "stale" producers changes the scheduler input non-deterministically.
        // The bond-weighted scheduler uses raw on-chain weights only.
        let effective_weights = active_with_weights;

        let current_epoch = self.params.slot_to_epoch(current_slot) as u64;
        let active_count = effective_weights.len();
        let total_bonds: u64 = effective_weights.iter().map(|(_, b)| *b).sum();
        let scheduler = match &self.cached_scheduler {
            Some((epoch, count, bonds, sched))
                if *epoch == current_epoch && *count == active_count && *bonds == total_bonds =>
            {
                sched
            }
            _ => {
                // Build new scheduler (epoch changed or active producer set changed)
                info!(
                    "Rebuilding scheduler: epoch={}, producers={}, total_bonds={}",
                    current_epoch, active_count, total_bonds
                );
                let producers: Vec<ScheduledProducer> = effective_weights
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
        eligible
    }
}
