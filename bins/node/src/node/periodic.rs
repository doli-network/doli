use super::*;

impl Node {
    /// Flush pending archive blocks up to the last finalized height.
    /// Only blocks that the protocol has declared irreversible get archived.
    pub(super) async fn flush_finalized_to_archive(&mut self) {
        let finalized_height = {
            let sync = self.sync_manager.read().await;
            match sync.last_finalized_height() {
                Some(h) => h,
                None => return, // No finality yet
            }
        };

        if let Some(ref tx) = self.archive_tx {
            while let Some(front) = self.pending_archive.front() {
                if front.height > finalized_height {
                    break;
                }
                let block = self.pending_archive.pop_front().unwrap();
                let _ = tx.try_send(block);
            }
        }
    }

    /// Bootstrap the maintainer set from the first 5 registered producers.
    /// Called once at the epoch boundary where the 5th producer is first available.
    /// After bootstrap, maintainer membership only changes via MaintainerAdd/Remove txs.
    pub(super) async fn maybe_bootstrap_maintainer_set(&self, height: u64) {
        use doli_core::maintainer::INITIAL_MAINTAINER_COUNT;

        let maintainer_state = match &self.maintainer_state {
            Some(ms) => ms,
            None => return,
        };

        // Already bootstrapped?
        {
            let state = maintainer_state.read().await;
            if state.set.is_fully_bootstrapped() {
                return;
            }
        }

        // Need at least INITIAL_MAINTAINER_COUNT producers to bootstrap
        let producers = self.producer_set.read().await;
        let mut sorted: Vec<_> = producers.all_producers().into_iter().cloned().collect();
        if sorted.len() < INITIAL_MAINTAINER_COUNT {
            return;
        }

        // Take the first 5 by registration height (deterministic)
        sorted.sort_by_key(|p| p.registered_at);
        let bootstrap_keys: Vec<_> = sorted
            .into_iter()
            .take(INITIAL_MAINTAINER_COUNT)
            .map(|p| p.public_key)
            .collect();

        let mut state = maintainer_state.write().await;
        // Double-check under write lock
        if state.set.is_fully_bootstrapped() {
            return;
        }

        let set =
            doli_core::maintainer::MaintainerSet::with_members(bootstrap_keys.clone(), height);
        state.set = set;
        state.last_derived_height = height;

        // Persist to disk
        if let Err(e) = state.save(&self.config.data_dir) {
            warn!("Failed to persist maintainer state: {}", e);
        }

        info!(
            "Bootstrapped maintainer set from first {} producers at height {} (keys: {})",
            INITIAL_MAINTAINER_COUNT,
            height,
            bootstrap_keys
                .iter()
                .map(|k| format!("{}...", &k.to_hex()[..16]))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    /// Run periodic tasks
    pub(super) async fn run_periodic_tasks(&mut self) -> Result<()> {
        // Apply pending sync blocks in correct order BEFORE cleanup.
        //
        // The body downloader fetches blocks in parallel, so they arrive out of order.
        // handle_response() returns them to handle_new_block() which requires strict
        // chain order (prev_hash == tip). Out-of-order blocks get orphaned.
        //
        // get_blocks_to_apply() walks pending_headers in order and extracts matching
        // bodies from pending_blocks, returning them in the correct chain order.
        // This MUST run before cleanup() so blocks get applied before the stuck
        // timeout fires and clears pending state.
        {
            let blocks = self.sync_manager.write().await.get_blocks_to_apply();
            if !blocks.is_empty() {
                info!("Applying {} pending sync blocks in order", blocks.len());
                for block in blocks {
                    if let Err(e) = self.apply_block(block, ValidationMode::Light).await {
                        warn!("Failed to apply pending sync block: {}", e);
                        self.sync_manager.write().await.block_apply_failed();
                        break;
                    }
                }
            }
        }

        // Check for ready checkpoint state (downloaded from peer, waiting to be applied)
        {
            let checkpoint_data = self.sync_manager.write().await.take_checkpoint_state();
            if let Some((block_hash, block_height, cs_bytes, utxo_bytes, ps_bytes, state_root)) =
                checkpoint_data
            {
                info!(
                    "[CHECKPOINT] Consuming checkpoint state at height={}",
                    block_height
                );
                match self
                    .apply_checkpoint_state(
                        block_hash,
                        block_height,
                        cs_bytes,
                        utxo_bytes,
                        ps_bytes,
                        state_root,
                    )
                    .await
                {
                    Ok(()) => {
                        info!(
                            "[CHECKPOINT] Successfully applied checkpoint state at height={}",
                            block_height
                        );
                    }
                    Err(e) => {
                        error!(
                            "[CHECKPOINT] Failed to apply checkpoint state: {} — falling back to header-first sync",
                            e
                        );
                    }
                }
            }
        }

        // Clean up sync manager and prune stale finality entries
        {
            let current_slot = {
                let state = self.chain_state.read().await;
                state.best_slot
            };
            let mut sync = self.sync_manager.write().await;
            sync.cleanup();
            sync.prune_finality(current_slot);
        }

        // Archive catch-up: after sync completes, backfill archive from block_store.
        // Runs once — fills any missing .block files between 1 and tip.
        if !self.archive_caught_up {
            if let Some(ref archive_dir) = self.archive_dir {
                let tip = self.chain_state.read().await.best_height;
                if tip > 0 {
                    info!(
                        "[ARCHIVER] Catch-up: scanning for gaps up to height {}",
                        tip
                    );
                    match storage::archiver::BlockArchiver::catch_up(
                        archive_dir,
                        &self.block_store,
                        tip,
                    ) {
                        Ok(n) if n > 0 => info!("[ARCHIVER] Catch-up: filled {} missing blocks", n),
                        Ok(_) => info!("[ARCHIVER] Catch-up: archive complete, no gaps"),
                        Err(e) => warn!("[ARCHIVER] Catch-up error: {}", e),
                    }
                    self.archive_caught_up = true;
                }
            } else {
                self.archive_caught_up = true;
            }
        }

        // Expire old mempool transactions
        self.mempool.write().await.expire_old();

        // Poll fork recovery: check if parent chain reached our block_store
        {
            let parent_hash = self
                .sync_manager
                .read()
                .await
                .fork_recovery_current_parent();
            if let Some(parent_hash) = parent_hash {
                let parent_known = self.block_store.has_block(&parent_hash).unwrap_or(false);
                if parent_known {
                    let completed = self
                        .sync_manager
                        .write()
                        .await
                        .check_fork_recovery_connection(true);
                    if let Some(recovery) = completed {
                        if let Err(e) = self.handle_completed_fork_recovery(recovery).await {
                            warn!("Fork recovery reorg failed: {}", e);
                        }
                    }
                }
            }
        }

        // SAFETY NET: If fork recovery exceeded max depth, log warning.
        // The fork is too deep for reorg — sync will recover via header-first download.
        {
            let exceeded = self
                .sync_manager
                .write()
                .await
                .take_fork_exceeded_max_depth();
            if exceeded {
                warn!(
                    "Fork recovery exceeded max depth — waiting for header-first sync to recover"
                );
            }
        }

        // PEER MAINTENANCE: Periodically redial bootstrap nodes when isolated.
        // REQ-NET-001: Exponential backoff per bootstrap address to avoid
        // saturating the event loop with failed TCP handshakes to dead peers.
        {
            let peer_count = self.sync_manager.read().await.peer_count();
            if peer_count > 0 {
                // Connected — reset all backoff counters
                self.bootstrap_backoff.clear();
            } else if !self.config.bootstrap_nodes.is_empty() {
                let now = std::time::Instant::now();
                if let Some(ref network) = self.network {
                    for addr in &self.config.bootstrap_nodes {
                        let (count, last_attempt) = self
                            .bootstrap_backoff
                            .entry(addr.clone())
                            .or_insert((0, now - Duration::from_secs(300)));

                        // Backoff: 1s, 2s, 4s, 8s, ... capped at 60s for bootstrap nodes
                        let backoff_secs = std::cmp::min(60, 1u64 << (*count).min(6));
                        let backoff = Duration::from_secs(backoff_secs);

                        if last_attempt.elapsed() >= backoff {
                            *last_attempt = now;
                            *count = count.saturating_add(1);
                            let _ = network.connect(addr).await;
                        }
                    }
                }
            }
        }

        // STALE CHAIN DETECTION (Ethereum-style):
        // If we haven't received any block (gossip or sync) for 3 slots, something is wrong.
        // Diagnose: no peers → re-bootstrap Kademlia; peers exist → aggressive status requests.
        // Status responses trigger update_peer() → should_sync() → start_sync() automatically.
        {
            let stale_threshold = Duration::from_secs(self.params.slot_duration * 3);
            let (is_stale, is_syncing, peer_count) = {
                let sync = self.sync_manager.read().await;
                (
                    sync.is_chain_stale(stale_threshold),
                    sync.state().is_syncing(),
                    sync.peer_count(),
                )
            };

            if is_stale && !is_syncing {
                if peer_count == 0 {
                    // FIX #5: Infected node auto-recovery.
                    // If we're stuck near genesis (height < 10) with 0 peers, we were
                    // likely wiped by a bad snap sync. The DHT cache is full of dead/infected
                    // peers. Reset bootstrap backoff so we reconnect immediately to
                    // hardcoded seeds instead of waiting 256s between retries.
                    let local_height = self.chain_state.read().await.best_height;
                    if local_height < 10 {
                        warn!(
                            "INFECTED NODE RECOVERY: height={} with 0 peers — resetting bootstrap backoff for immediate reconnection",
                            local_height
                        );
                        self.bootstrap_backoff.clear();
                    }

                    // No peers — redial bootstrap nodes and re-bootstrap DHT
                    info!("Stale chain detected (no blocks for 3 slots) with 0 peers — redialing bootstrap nodes");
                    if let Some(ref network) = self.network {
                        for addr in &self.config.bootstrap_nodes {
                            if let Err(e) = network.connect(addr).await {
                                warn!("Failed to redial bootstrap {}: {}", addr, e);
                            }
                        }
                        let _ = network.bootstrap().await;
                    }
                } else {
                    // Peers exist but no blocks — request status from ALL peers
                    // This forces update_peer() which triggers should_sync()/start_sync()
                    debug!(
                        "Stale chain detected with {} peers — requesting status from all",
                        peer_count
                    );
                    if let Some(ref network) = self.network {
                        let genesis_hash = self.chain_state.read().await.genesis_hash;
                        let status_request = if let Some(ref key) = self.producer_key {
                            network::protocols::StatusRequest::with_producer(
                                self.config.network.id(),
                                genesis_hash,
                                *key.public_key(),
                            )
                        } else {
                            network::protocols::StatusRequest::new(
                                self.config.network.id(),
                                genesis_hash,
                            )
                        };
                        let peer_ids: Vec<_> = {
                            let sync = self.sync_manager.read().await;
                            sync.peer_ids().collect()
                        };
                        for peer_id in peer_ids.iter().take(10) {
                            let _ = network
                                .request_status(*peer_id, status_request.clone())
                                .await;
                        }
                    }
                }
            }
        }

        // FORK SYNC: Binary search for common ancestor when on a dead fork.
        // Triggers after 3+ consecutive empty header responses. O(log N) recovery.
        if self.resolve_shallow_fork().await? {
            // Fork sync was just initiated, don't do anything else this tick
            return Ok(());
        }

        // Drive active fork sync: compare probes with block_store, handle transitions
        {
            // Phase 1: Binary search — compare peer's block hash with ours
            let probe = self.sync_manager.read().await.fork_sync_pending_probe();
            if let Some((height, peer_hash)) = probe {
                let our_hash = self.block_store.get_hash_by_height(height).ok().flatten();
                let result = match our_hash {
                    Some(h) if h == peer_hash => network::sync::ProbeResult::Match,
                    Some(_) => network::sync::ProbeResult::Mismatch,
                    None => network::sync::ProbeResult::NotInStore,
                };
                self.sync_manager
                    .write()
                    .await
                    .fork_sync_handle_probe(result);
            }

            // Transition: search complete — provide ancestor hash from our block_store.
            //
            // Store-limited: search stopped because block store doesn't cover the range
            // (snap sync gap). This is NOT a deep fork — the node just doesn't have
            // the historical blocks. Re-snap to get back to tip and resume producing.
            if self.sync_manager.read().await.fork_sync_store_limited() {
                let floor = self.sync_manager.read().await.store_floor();
                warn!(
                    "Fork sync: search limited by block store floor (height {}). \
                     Skipping — NOT a deep fork. Header-first sync will recover.",
                    floor
                );
                self.sync_manager.write().await.fork_sync_clear();
                // Do NOT snap sync. Clear fork state and let header-first sync handle it.
                self.sync_manager.write().await.set_post_recovery_grace();
                return Ok(());
            }

            // Bottomed out: genuine deep fork — no common ancestor within MAX_FORK_SYNC_DEPTH
            // and the block store covers the full range. Log warning, clear fork sync,
            // and let header-first sync attempt recovery.
            if self.sync_manager.read().await.fork_sync_bottomed_out() {
                warn!("Fork sync: binary search hit floor without finding common ancestor — clearing fork sync, header-first sync will recover");
                self.sync_manager.write().await.fork_sync_clear();
                self.sync_manager.write().await.set_post_recovery_grace();
                return Ok(());
            }
            let ancestor_height = self.sync_manager.read().await.fork_sync_ancestor_height();
            if let Some(height) = ancestor_height {
                let ancestor_hash = self
                    .block_store
                    .get_hash_by_height(height)
                    .ok()
                    .flatten()
                    .unwrap_or(self.chain_state.read().await.genesis_hash);
                self.sync_manager
                    .write()
                    .await
                    .fork_sync_set_ancestor(height, ancestor_hash);
            }

            // Phase 2/3 complete: take result and execute reorg
            let result = self.sync_manager.write().await.fork_sync_take_result();
            if let Some(result) = result {
                if let Err(e) = self.execute_fork_sync_reorg(result).await {
                    warn!("Fork sync reorg failed: {}", e);
                }
                return Ok(());
            }
        }

        // DEEP FORK DETECTION: If peers consistently reject our chain tip (10+ empty
        // header responses), log warning. Fork sync handles recovery via binary search.
        {
            let is_deep_fork = self.sync_manager.read().await.is_deep_fork_detected();
            if is_deep_fork {
                warn!("Deep fork detected: peers consistently reject our chain tip. Fork sync will attempt recovery.");
            }
        }

        // Check if we need to request sync
        {
            let mut sm = self.sync_manager.write().await;
            if let Some((peer_id, request)) = sm.next_request() {
                if let Some(ref network) = self.network {
                    let _ = network.request_sync(peer_id, request).await;
                }
            }
        }

        // PERIODIC STATUS REFRESH: Request status from peers to keep network tip updated
        // This is critical for production gating - without fresh peer status, we can't
        // know if other nodes have produced blocks we haven't received via gossip yet.
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // During bootstrap (height 0), be VERY aggressive about requesting status
        // from ALL peers - we need to find at least one peer with height > 0
        let local_height = self.chain_state.read().await.best_height;
        let is_bootstrap = local_height == 0;

        // Request status every ~2 seconds during bootstrap, ~5 seconds during normal ops
        let status_interval = if is_bootstrap {
            2 // Aggressive during bootstrap
        } else {
            5 // All networks: 5s keeps peer status fresh for fork detection
        };

        if now_secs.is_multiple_of(status_interval) {
            if let Some(ref network) = self.network {
                let peer_ids: Vec<_> = {
                    let sync = self.sync_manager.read().await;
                    sync.peer_ids().collect()
                };

                if !peer_ids.is_empty() {
                    let genesis_hash = self.chain_state.read().await.genesis_hash;
                    let status_request = if let Some(ref key) = self.producer_key {
                        network::protocols::StatusRequest::with_producer(
                            self.config.network.id(),
                            genesis_hash,
                            *key.public_key(),
                        )
                    } else {
                        network::protocols::StatusRequest::new(
                            self.config.network.id(),
                            genesis_hash,
                        )
                    };

                    if is_bootstrap {
                        // During bootstrap, request from ALL peers to find any with height > 0
                        for peer_id in peer_ids.iter().take(5) {
                            // Limit to 5 to avoid flooding
                            debug!("Bootstrap status request to peer {}", peer_id);
                            let _ = network
                                .request_status(*peer_id, status_request.clone())
                                .await;
                        }
                    } else {
                        // Normal operation - request from one peer at a time
                        let peer_idx = (now_secs as usize) % peer_ids.len();
                        let peer_id = peer_ids[peer_idx];
                        debug!("Periodic status request to peer {}", peer_id);
                        let _ = network.request_status(peer_id, status_request).await;
                    }
                }
            }
        }

        // PORT REACHABILITY WARNING (one-shot, mainnet producers only)
        // After 60s of running, if we have zero peers it likely means the P2P
        // port is not reachable from the internet (firewall/NAT misconfiguration).
        if !self.port_check_done
            && self.config.network == Network::Mainnet
            && self.producer_key.is_some()
        {
            let uptime = self
                .first_peer_connected
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            // Wait at least 60s after first peer, or 120s total if no peer ever connected
            let threshold = if self.first_peer_connected.is_some() {
                60
            } else {
                120
            };
            if uptime >= threshold
                || (self.first_peer_connected.is_none()
                    && now_secs.is_multiple_of(120)
                    && now_secs > 0)
            {
                let peer_count = self.sync_manager.read().await.peer_count();
                if peer_count == 0 {
                    let p2p_port = self
                        .config
                        .listen_addr
                        .split(':')
                        .next_back()
                        .unwrap_or("30300");
                    warn!("════════════════════════════════════════════════════════════════");
                    warn!(
                        "  WARNING: 0 peers after {}s — P2P port {} may be unreachable",
                        threshold, p2p_port
                    );
                    warn!("  Blocks you produce will NOT propagate to the network.");
                    warn!(
                        "  Fix: ensure TCP port {} is open (inbound) on your firewall.",
                        p2p_port
                    );
                    warn!("════════════════════════════════════════════════════════════════");
                } else {
                    info!(
                        "Port check: {} peers connected after {}s — OK",
                        peer_count, threshold
                    );
                }
                self.port_check_done = true;
            }
        }

        // Periodic health diagnostic — one-line summary every 30s for fork debugging
        if now_secs.is_multiple_of(30) {
            let cs = self.chain_state.read().await;
            let sync = self.sync_manager.read().await;
            let peer_count = sync.peer_count();
            let best_peer_h = sync.best_peer_height();
            let best_peer_s = sync.best_peer_slot();
            let net_tip_h = sync.network_tip_height();
            let net_tip_s = sync.network_tip_slot();
            let sync_fails = sync.consecutive_sync_failure_count();
            warn!(
                "[HEALTH] h={} s={} hash={:.8} | peers={} best_peer_h={} best_peer_s={} net_tip_h={} net_tip_s={} | sync_fails={} fork_counter={} state={:?}",
                cs.best_height, cs.best_slot, cs.best_hash,
                peer_count, best_peer_h, best_peer_s, net_tip_h, net_tip_s,
                sync_fails, self.consecutive_fork_blocks, sync.sync_state_name()
            );
        }

        Ok(())
    }
}
