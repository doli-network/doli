use super::*;

impl Node {
    /// Flush pending archive blocks up to the last finalized height.
    /// Only blocks that the protocol has declared irreversible get archived.
    pub async fn flush_finalized_to_archive(&mut self) {
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
    pub async fn maybe_bootstrap_maintainer_set(&self, height: u64) {
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
    pub async fn run_periodic_tasks(&mut self) -> Result<()> {
        // Periodic chain integrity scan: full BLAKE3(h1||h2||...||hn) every 100 blocks.
        // Always correct by construction — no incremental state to corrupt.
        // Replaces the incremental commitment which broke on every code path that
        // modified the chain without updating it (fork replacement, sync, rsync, snap sync).
        // With 40K+ blocks, full scan takes <1 second via BLAKE3.
        {
            let tip = self.chain_state.read().await.best_height;
            let last_scan = self.last_integrity_check_tip.unwrap_or(0);
            // Round scan_tip to nearest 100 so all nodes compute the same commitment
            // at the same height, eliminating explorer flickering from scan-timing drift.
            let scan_tip = (tip / 100) * 100;
            if scan_tip > 0 && scan_tip > last_scan {
                let block_store = self.block_store.clone();
                let state_db = self.state_db.clone();
                tokio::task::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        let mut missing = 0u64;
                        let mut commitment = crypto::Hash::default();
                        for h in 1..=scan_tip {
                            match block_store.get_block_by_height(h) {
                                Ok(Some(blk)) => {
                                    let hash = blk.hash();
                                    let mut hasher = crypto::Hasher::new();
                                    hasher.update(commitment.as_bytes());
                                    hasher.update(hash.as_bytes());
                                    commitment = hasher.finalize();
                                }
                                _ => missing += 1,
                            }
                        }
                        (missing, commitment)
                    })
                    .await;
                    if let Ok((missing, commitment)) = result {
                        if missing > 0 {
                            tracing::warn!(
                                "[INTEGRITY] Periodic scan: {} missing blocks in 1..={}",
                                missing,
                                scan_tip
                            );
                            state_db.delete_chain_commitment();
                        } else {
                            tracing::info!(
                                "[INTEGRITY] Periodic scan: chain complete (1..={}), commitment={:.16}",
                                scan_tip,
                                commitment
                            );
                            state_db.put_chain_commitment_with_tip(&commitment, scan_tip);
                        }
                    }
                });
                self.last_integrity_check_tip = Some(scan_tip);
            }
        }

        // Clean stale entries from seen_blocks_for_slot (keep last 10 slots)
        {
            let current_slot = self.chain_state.read().await.best_slot;
            self.seen_blocks_for_slot
                .retain(|&s| (s as u64) + 10 > current_slot as u64);
        }

        // TTL sweep: evict cached blocks older than 30 slots (~5 min at 10s/slot).
        // Blocks that sit in fork_block_cache for this long were never resolved
        // (parent never arrived, fork never connected). Drop them to free memory
        // and prevent stale blocks from interfering with future fork recovery.
        {
            let current_slot = self.chain_state.read().await.best_slot;
            const CACHE_TTL_SLOTS: u32 = 30; // ~5 minutes at 10s/slot
            let min_slot = current_slot.saturating_sub(CACHE_TTL_SLOTS);
            let mut cache = self.fork_block_cache.write().await;
            let before = cache.len();
            cache.retain(|_, b| b.header.slot >= min_slot);
            let evicted = before - cache.len();
            if evicted > 0 {
                info!(
                    "[CACHE_TTL] Evicted {} stale blocks from fork_block_cache (slot < {})",
                    evicted, min_slot
                );
            }
        }

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

        // Check for ready snap sync snapshot (downloaded from peer, waiting to be applied)
        {
            let snapshot = self.sync_manager.write().await.take_snap_snapshot();
            if let Some(snap) = snapshot {
                info!(
                    "[SNAP_SYNC] Consuming snapshot state at height={}",
                    snap.block_height
                );
                match self.apply_snap_snapshot(snap).await {
                    Ok(()) => {
                        info!("[SNAP_SYNC] Snapshot applied successfully");
                    }
                    Err(e) => {
                        error!(
                            "[SNAP_SYNC] Failed to apply snapshot: {} — falling back to header-first sync",
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

        // DISCV5 SEED FALLBACK: If discv5 is active but after 60s we still have
        // 0 peers, reconnect to TCP seeds as safety net. This handles the case where
        // no discv5 bootnodes are reachable (misconfigured ENR, UDP blocked, etc.).
        // The seed is a last resort, not the primary discovery mechanism.
        if !self.config.no_discv5 && self.seeds_released {
            let peer_count = self.sync_manager.read().await.peer_count();
            if peer_count == 0 {
                if let Some(first) = self.first_peer_connected {
                    // 60s since we last had peers — reconnect to seeds
                    if first.elapsed() > Duration::from_secs(60)
                        && self
                            .last_peer_redial
                            .map(|t| t.elapsed() > Duration::from_secs(60))
                            .unwrap_or(true)
                    {
                        warn!(
                            "[DISCV5_FALLBACK] 0 peers for >60s with discv5 active — reconnecting to {} TCP seed(s)",
                            self.seed_peer_ids.len()
                        );
                        if let Some(ref net) = self.network {
                            for addr in &self.config.bootstrap_nodes {
                                let _ = net.connect(addr).await;
                            }
                        }
                        self.last_peer_redial = Some(Instant::now());
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
                        let fork_id = self.current_fork_id();
                        let status_request = if let Some(ref key) = self.producer_key {
                            network::protocols::StatusRequest::with_producer(
                                self.config.network.id(),
                                genesis_hash,
                                fork_id,
                                *key.public_key(),
                            )
                        } else {
                            network::protocols::StatusRequest::new(
                                self.config.network.id(),
                                genesis_hash,
                                fork_id,
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

        // SILENCE PULL: if gossip hasn't delivered a block in 30s, request
        // the next block from a random peer. Complements orphan chase —
        // chase handles "got something I can't use", this handles "got nothing".
        {
            let last_applied = self.sync_manager.read().await.last_block_applied_secs();
            if last_applied >= 30 {
                let catch_up = self.sync_manager.read().await.catch_up_request();
                if let Some((peer_id, request)) = catch_up {
                    if let Some(ref network) = self.network {
                        info!(
                            "[SILENCE_PULL] No block for {}s, requesting from {}",
                            last_applied, peer_id
                        );
                        let _ = network.request_sync(peer_id, request).await;
                    }
                }
            }
        }

        // RECOVERY COORDINATOR: single dispatch point for all fork/sync recovery.
        //
        // Replaces 3 independent detector→action paths (ACTIVE_FORK_DETECT,
        // resolve_shallow_fork, DEEP_FORK_DETECT) with the RecoveryCoordinator's
        // classify→execute dispatch. Evidence is reported based on current state,
        // then the coordinator classifies and returns a single action.
        //
        // Phase 2 ran this in shadow mode (log only). Phase 3 (M2) makes it
        // authoritative — the coordinator's action is executed.
        {
            // Report evidence based on current state before classifying
            {
                let sync = self.sync_manager.read().await;
                let local_h = sync.local_tip().0;
                let gap = sync.network_tip_height().saturating_sub(local_h);
                let last_applied = sync.last_block_applied_secs();
                let empty_headers = sync.consecutive_empty_headers();
                drop(sync);

                let mut sync_w = self.sync_manager.write().await;
                if empty_headers >= 3 {
                    sync_w.report_empty_headers(PeerId::random(), gap);
                }
                if last_applied >= 30 && gap > 0 {
                    sync_w.report_stale_tip(last_applied, gap);
                }
            }

            let action = {
                let mut sync = self.sync_manager.write().await;
                sync.classify_and_dispatch(self.shallow_rollback_count)
            };
            match action {
                network::RecoveryAction::None => {}
                network::RecoveryAction::ShallowRollback { depth } => {
                    for _ in 0..depth {
                        if !self.rollback_one_block().await? {
                            break;
                        }
                        self.shallow_rollback_count += 1;
                    }
                    return Ok(());
                }
                network::RecoveryAction::HeaderFirstSync => {
                    // Trigger header-first sync by resetting empty headers,
                    // which allows should_sync() → start_sync() on next tick.
                    let mut sync = self.sync_manager.write().await;
                    sync.reset_empty_headers();
                }
                network::RecoveryAction::SnapSync => {
                    let mut sync = self.sync_manager.write().await;
                    sync.request_genesis_resync(network::RecoveryReason::CoordinatorSnapEscalation);
                }
                network::RecoveryAction::GenesisResync => {
                    let mut sync = self.sync_manager.write().await;
                    sync.request_genesis_resync(
                        network::RecoveryReason::CoordinatorGenesisEscalation,
                    );
                }
            }
        }

        // Check if we need to request sync
        {
            let mut sm = self.sync_manager.write().await;

            // Snap sync uses batch requests (all eligible peers at once) to
            // collect state root votes within the quorum window. Without this,
            // next_request() returns None for SnapCollecting and no GetStateRoot
            // requests are ever sent — snap sync silently times out. (INC-I-017)
            let snap_batch = sm.next_snap_requests();
            if !snap_batch.is_empty() {
                if let Some(ref network) = self.network {
                    for (peer_id, request) in snap_batch {
                        let _ = network.request_sync(peer_id, request).await;
                    }
                }
            }

            if let Some((peer_id, request)) = sm.next_request() {
                if let Some(ref network) = self.network {
                    let _ = network.request_sync(peer_id, request).await;
                }
            }
        }

        // PERIODIC STATUS REFRESH: Request status from ALL peers to keep sync manager fresh.
        // Critical for:
        // 1. checkpoint_health() — needs accurate per-peer heights to distinguish
        //    stale connections (h=0) from real forks
        // 2. Production gating — knowing if peers are ahead of us
        //
        // Previous approach used one-peer-at-a-time round-robin: (now_secs % peer_count).
        // Bug: when peer_count divides evenly into the interval (e.g., 5 peers, 5s interval),
        // now_secs is always a multiple of 5, so now_secs % 5 = 0 — always peer[0].
        // Fix: request ALL peers every 30s. Same total bandwidth, guaranteed freshness.
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let local_height = self.chain_state.read().await.best_height;
        let is_bootstrap = local_height == 0;

        // Bootstrap: all peers every 2s (need to find peers with height > 0)
        // Normal: all peers every 30s (sufficient for checkpoint health)
        let status_interval = if is_bootstrap { 2 } else { 30 };

        let force_refresh = {
            let mut sync = self.sync_manager.write().await;
            sync.take_needs_mass_status_refresh()
        };

        if force_refresh || now_secs.is_multiple_of(status_interval) {
            if let Some(ref network) = self.network {
                let peer_ids: Vec<_> = {
                    let sync = self.sync_manager.read().await;
                    sync.peer_ids().collect()
                };

                if !peer_ids.is_empty() {
                    let genesis_hash = self.chain_state.read().await.genesis_hash;
                    let fork_id = self.current_fork_id();
                    let status_request = if let Some(ref key) = self.producer_key {
                        network::protocols::StatusRequest::with_producer(
                            self.config.network.id(),
                            genesis_hash,
                            fork_id,
                            *key.public_key(),
                        )
                    } else {
                        network::protocols::StatusRequest::new(
                            self.config.network.id(),
                            genesis_hash,
                            fork_id,
                        )
                    };

                    // Request from ALL peers (capped to prevent flooding large networks)
                    let cap = if is_bootstrap { 5 } else { 20 };
                    for peer_id in peer_ids.iter().take(cap) {
                        debug!("Periodic status request to peer {}", peer_id);
                        let _ = network
                            .request_status(*peer_id, status_request.clone())
                            .await;
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

        // AUTO-CHECKPOINT: Create RocksDB snapshot every N blocks.
        // Keeps last 5 checkpoints for fast recovery from poison/fork corruption.
        if let Some(interval) = self.config.auto_checkpoint_interval {
            let current_height = self.chain_state.read().await.best_height;
            if current_height > 0 && current_height >= self.last_checkpoint_height + interval {
                let checkpoint_base = self.config.data_dir.join("checkpoints");
                let timestamp = now_secs;
                let checkpoint_name = format!("h{}-{}", current_height, timestamp);
                let checkpoint_dir = checkpoint_base.join(&checkpoint_name);

                if let Err(e) = std::fs::create_dir_all(&checkpoint_dir) {
                    warn!("[AUTO_CHECKPOINT] Failed to create dir: {}", e);
                } else {
                    let state_ok = self
                        .state_db
                        .create_checkpoint(&checkpoint_dir.join("state_db"))
                        .is_ok();
                    let blocks_ok = self
                        .block_store
                        .create_checkpoint(&checkpoint_dir.join("blocks"))
                        .is_ok();

                    if state_ok && blocks_ok {
                        self.last_checkpoint_height = current_height;

                        // Write health.json — tags checkpoint with peer consensus data
                        // so recovery can find the last HEALTHY checkpoint.
                        let (peer_count, peers_agreeing, unique_hashes) = {
                            let sync = self.sync_manager.read().await;
                            sync.checkpoint_health()
                        };
                        let best_hash = {
                            let cs = self.chain_state.read().await;
                            cs.best_hash.to_hex()
                        };
                        let healthy =
                            peer_count > 0 && peers_agreeing == peer_count && unique_hashes <= 1;
                        let health = serde_json::json!({
                            "height": current_height,
                            "hash": best_hash,
                            "timestamp": timestamp,
                            "peer_count": peer_count,
                            "peers_agreeing": peers_agreeing,
                            "unique_chain_tips": unique_hashes,
                            "healthy": healthy,
                        });
                        let _ = std::fs::write(
                            checkpoint_dir.join("health.json"),
                            serde_json::to_string_pretty(&health).unwrap_or_default(),
                        );

                        if healthy {
                            info!(
                                "[AUTO_CHECKPOINT] HEALTHY at height={} ({}/{} peers agree) path={}",
                                current_height, peers_agreeing, peer_count,
                                checkpoint_dir.display()
                            );
                        } else {
                            warn!(
                                "[AUTO_CHECKPOINT] UNHEALTHY at height={} ({}/{} peers agree, {} tips) path={}",
                                current_height, peers_agreeing, peer_count, unique_hashes,
                                checkpoint_dir.display()
                            );
                        }

                        // Rotate: keep only the last 5 checkpoints
                        if let Ok(entries) = std::fs::read_dir(&checkpoint_base) {
                            let mut dirs: Vec<_> = entries
                                .filter_map(|e| e.ok())
                                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                                .collect();
                            dirs.sort_by_key(|e| {
                                parse_checkpoint_height(&e.file_name().to_string_lossy())
                            });
                            if dirs.len() > 5 {
                                for old in &dirs[..dirs.len() - 5] {
                                    let _ = std::fs::remove_dir_all(old.path());
                                    info!(
                                        "[AUTO_CHECKPOINT] Rotated old: {}",
                                        old.file_name().to_string_lossy()
                                    );
                                }
                            }
                        }
                    } else {
                        warn!(
                            "[AUTO_CHECKPOINT] Failed at height={} (state={} blocks={})",
                            current_height, state_ok, blocks_ok
                        );
                        // Clean up partial checkpoint
                        let _ = std::fs::remove_dir_all(&checkpoint_dir);
                    }
                }
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
            let snap_bonds: u64 = self.epoch_state.bond_snapshot.values().sum();
            let snap_producers = self.epoch_state.bond_snapshot.len();
            warn!(
                "[HEALTH] h={} s={} hash={:.8} | peers={} best_peer_h={} best_peer_s={} net_tip_h={} net_tip_s={} | sync_fails={} state={:?} | snap_epoch={} snap_bonds={} snap_producers={}",
                cs.best_height, cs.best_slot, cs.best_hash,
                peer_count, best_peer_h, best_peer_s, net_tip_h, net_tip_s,
                sync_fails, sync.sync_state_name(),
                self.epoch_state.epoch, snap_bonds, snap_producers
            );

            // Sync state summary — captures key sync variables for post-incident analysis.
            {
                let gap = net_tip_h.saturating_sub(cs.best_height);
                let secs_since_last_apply = sync.last_block_applied_secs();
                let epoch_list_len = self.epoch_state.producer_list.len();
                info!(
                    "[SYNC_STATE] gap={} phase={:?} last_applied_ago={}s epoch_list={} rollback_depth={}",
                    gap,
                    sync.sync_state_name(),
                    secs_since_last_apply,
                    epoch_list_len,
                    self.cumulative_rollback_depth
                );
            }

            // Recovery Coordinator: shadow dispatch removed (M2 promotion).
            // The coordinator is now authoritative — classify_and_dispatch()
            // runs earlier in the periodic loop and executes the action directly.

            // INC-I-020/020b: DISABLED.
            //
            // STALE_TIP and FORK_1BLOCK removed. They fought with FORK_GUARD:
            // STALE_TIP requested a block → peer sent a different block at same height →
            // FORK_GUARD dropped it → STALE_TIP triggered rollback → gap grew → cascade.
            //
            // With INC-I-026 (deterministic scheduler) and fork_id, gaps of 1-2 blocks
            // resolve via gossip within seconds. Gaps of 3+ trigger should_sync().
            // No active intervention needed for small gaps.
        }

        // SEED RELEASE: Disconnect from seed/bootstrap nodes after DHT bootstrap + gossip verified.
        // Frees seed peer slots so the network scales without the seed as a bottleneck.
        // Conditions (all must be true):
        //   1. Not already released
        //   2. Have seed peer IDs to release
        //   3. Have 5+ peers from DHT (enough to maintain gossip mesh)
        //   4. Receiving blocks via gossip (network_tip_height > local_height - 2)
        //   5. Not a seed/relay node ourselves (they need to stay connected)
        if !self.seeds_released && !self.seed_peer_ids.is_empty() {
            let sync = self.sync_manager.read().await;
            let peer_count = sync.peer_count();
            let net_tip = sync.network_tip_height();
            let local_h = self.chain_state.read().await.best_height;
            drop(sync);

            let has_enough_peers = peer_count >= 5;
            let receiving_blocks =
                net_tip > 0 && local_h > 0 && net_tip >= local_h.saturating_sub(2);
            let is_relay = self.config.relay_server;

            if has_enough_peers && receiving_blocks && !is_relay {
                if let Some(ref net) = self.network {
                    for seed_id in &self.seed_peer_ids {
                        let _ = net.disconnect(*seed_id).await;
                    }
                    info!(
                        "[SEED_RELEASE] Disconnected from {} seed(s) — DHT has {} peers, receiving blocks at h={}",
                        self.seed_peer_ids.len(), peer_count, local_h
                    );
                }
                self.seeds_released = true;
            }
        }

        // INC-I-034 / M-Choice2: Phase-1 observability-only periodic block-store
        // integrity check. Runs every INTEGRITY_CHECK_INTERVAL_BLOCKS blocks,
        // emits CRITICAL on gap detection. See specs/scheduler-state-architecture.md
        // Block-store integrity contract + locked Choice 2 (RUNTIME PERIODIC).
        self.maybe_run_integrity_check().await;

        Ok(())
    }

    /// Phase-1 observability-only periodic block-store integrity check (M-Choice2).
    ///
    /// Runs every `INTEGRITY_CHECK_INTERVAL_BLOCKS` blocks (default 1000).
    /// Scans `BlockStore::ensure_blocks_present(1, tip)`. On gap detection,
    /// emits a CRITICAL log line with a clear operator-action message pointing
    /// to `doli chain-repair`. Does NOT halt production (Phase 2 HF concern).
    ///
    /// Runs the scan in a blocking task to avoid starving the async runtime;
    /// O(range) hot CF point lookups.
    pub(crate) async fn maybe_run_integrity_check(&mut self) {
        let current_tip = self.chain_state.read().await.best_height;
        if !should_run_integrity_check(
            current_tip,
            self.last_integrity_check_tip,
            INTEGRITY_CHECK_INTERVAL_BLOCKS,
        ) {
            return;
        }

        let block_store = self.block_store.clone();
        let result =
            tokio::task::spawn_blocking(move || block_store.ensure_blocks_present(1, current_tip))
                .await;

        match result {
            Ok(Ok(())) => {
                info!(
                    "[INTEGRITY_CHECK] block_store complete 1..={} (next scan in {} blocks)",
                    current_tip, INTEGRITY_CHECK_INTERVAL_BLOCKS
                );
            }
            Ok(Err(e)) => {
                error!(
                    "[INTEGRITY_CHECK] CRITICAL: {}. This node's block_store has a gap. \
                     Run `doli chain-repair --peer <RPC_URL>` against a known-good peer to heal. \
                     Production will continue for now; at the M-Choice1 HardForkSchedule \
                     activation height, gapped nodes will enter HALT_PRODUCTION.",
                    e
                );
            }
            Err(join_err) => {
                warn!(
                    "[INTEGRITY_CHECK] scan task join error at tip={}: {}",
                    current_tip, join_err
                );
            }
        }

        // Update the last-checked marker regardless of scan result — we tried.
        // On success, this prevents re-scanning for another 1000 blocks.
        // On failure, this prevents log spam every tick; operator will see the
        // CRITICAL once per interval until they run chain-repair.
        self.last_integrity_check_tip = Some(current_tip);
    }
}

/// Parse the numeric height from a checkpoint directory name like "h4535-1774889941".
/// Returns 0 if the name doesn't match the expected format.
pub(crate) fn parse_checkpoint_height(name: &str) -> u64 {
    name.strip_prefix('h')
        .and_then(|s| s.split('-').next())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

// =============================================================================
// Phase-1 periodic block-store integrity check (M-Choice2, INC-I-034)
// =============================================================================
//
// See `specs/scheduler-state-architecture.md` §"Block-store integrity contract:
// bounded fail-fast + self-heal (F-11 / B-2 refinement)" and the CHOICE 2 lock
// ("RUNTIME PERIODIC — runs at startup, on every chain_state advance, AND as a
// background task every 1000 blocks").
//
// Phase 1 is OBSERVABILITY-ONLY: detect gaps, emit CRITICAL log, record the
// scan tip. No HALT_PRODUCTION, no automatic backfill dispatch.
//
// The pure helper `should_run_integrity_check` below is the scheduling
// predicate that decides — given a tip and the last-checked tip — whether a
// new scan is due. Everything stateful and async (the actual scan, log
// emission, and tip bookkeeping) lives in the `Node` method that calls this
// helper from `run_periodic_tasks`.
//
// The helper is pure so it can be unit-tested without a tokio runtime, a
// BlockStore fixture, or any timing dependency.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_checkpoint_height() {
        assert_eq!(parse_checkpoint_height("h526-1774849792"), 526);
        assert_eq!(parse_checkpoint_height("h4535-1774889941"), 4535);
        assert_eq!(parse_checkpoint_height("h12345-9999999999"), 12345);
        assert_eq!(parse_checkpoint_height("h0-0"), 0);
        assert_eq!(parse_checkpoint_height("garbage"), 0);
        assert_eq!(parse_checkpoint_height(""), 0);
    }

    #[test]
    fn test_checkpoint_sort_order_numeric_vs_lexicographic() {
        // These are the actual directory names from the production bug.
        // Lexicographic sort puts h526-h926 AFTER h3635-h4535 (wrong).
        // Numeric sort must put h526-h926 BEFORE h3635-h4535 (correct).
        let mut names = vec![
            "h526-1774849792",
            "h626-1774850792",
            "h726-1774851792",
            "h826-1774852792",
            "h926-1774853792",
            "h3635-1774880882",
            "h3735-1774881882",
            "h4335-1774887902",
            "h4435-1774888902",
            "h4535-1774889941",
        ];

        // Sort numerically by height (the fix)
        names.sort_by_key(|n| parse_checkpoint_height(n));

        // After numeric sort, lowest heights first, highest last
        assert_eq!(parse_checkpoint_height(names[0]), 526);
        assert_eq!(parse_checkpoint_height(names[1]), 626);
        assert_eq!(parse_checkpoint_height(names.last().unwrap()), 4535);

        // Rotation keeps last 5 → should keep h3635, h3735, h4335, h4435, h4535
        let keep = &names[names.len() - 5..];
        let keep_heights: Vec<u64> = keep.iter().map(|n| parse_checkpoint_height(n)).collect();
        assert_eq!(keep_heights, vec![3635, 3735, 4335, 4435, 4535]);

        // The old checkpoints (h526-h926) are in the "delete" range
        let delete = &names[..names.len() - 5];
        let delete_heights: Vec<u64> = delete.iter().map(|n| parse_checkpoint_height(n)).collect();
        assert_eq!(delete_heights, vec![526, 626, 726, 826, 926]);
    }
}

// =============================================================================
// M-Choice2 / INC-I-034 — Phase 1 periodic integrity-check scheduling predicate
// =============================================================================

/// Minimum number of blocks between periodic block-store integrity scans.
///
/// Phase 1 observability-only: scan runs every ~1000 blocks (~3h at 10s slots).
/// Spec: `specs/scheduler-state-architecture.md` — "Block-store integrity contract"
/// + locked USER DECISION Choice 2 (RUNTIME PERIODIC).
pub(crate) const INTEGRITY_CHECK_INTERVAL_BLOCKS: u64 = 1000;

/// Phase-1 scheduling predicate for the periodic integrity check.
///
/// Returns `true` iff (a) we've never scanned, OR (b) tip has advanced
/// `min_interval_blocks` or more since the last scan. Genesis (tip=0)
/// always returns `false` (nothing to scan yet). Defensive against tip
/// going backwards (rollback): treats it as "no advance" and returns `false`.
///
/// Pure: no I/O, no locks, no time source.
pub(crate) fn should_run_integrity_check(
    current_tip: u64,
    last_checked_tip: Option<u64>,
    min_interval_blocks: u64,
) -> bool {
    if current_tip == 0 {
        return false;
    }
    match last_checked_tip {
        None => true,
        Some(last) => {
            if current_tip <= last {
                return false; // no advance or rollback — defensive
            }
            current_tip.saturating_sub(last) >= min_interval_blocks
        }
    }
}
//
// OUTPUT CONTRACT: fn should_run_integrity_check(current_tip: u64, last_checked_tip: Option<u64>, min_interval_blocks: u64) -> bool
//   O1: mutable params        — none (all parameters are by-value `u64` / `Option<u64>`, no `&mut`)
//   O2: receiver/self         — none (free function, no self)
//   O3: return                — bool: `true` => caller SHOULD run the scan, `false` => skip
//   O4: persistent stores     — none (pure function, no I/O)
//   O5: global/static state   — none (no statics, no env)
//   O6: channels/events       — none (no send/emit/callback)
// PATHS (enumerated per milestone brief):
//   P1: Genesis            — tip=0, last=None, interval=1000           => false
//   P2: First-run crossed  — tip=1500, last=None, interval=1000        => true
//   P3: Too soon           — tip=1500, last=Some(1499), interval=1000  => false
//   P4: Boundary inclusive — tip=2000, last=Some(1000), interval=1000  => true
//   P5: Just past boundary — tip=2001, last=Some(1000), interval=1000  => true
//   P6: Zero interval      — tip=5,    last=Some(5),    interval=0     => false
//   P7: Tip backward       — tip=100,  last=Some(500),  interval=1000  => false
//   P8: u64::MAX boundary  — tip=MAX,  last=Some(MAX-1000), interval=1000 => true   (adversarial)
//   P9: u64::MAX no-advance— tip=MAX,  last=Some(MAX),  interval=1000   => false  (adversarial)
// MATRIX: 1 output (O3 bool return) × 9 paths = 9 cells, each asserted by a dedicated #[test].
//
// Notes
//  - The helper is pure by contract: the developer MUST NOT add I/O, locks, or a
//    time source to its signature. All stateful glue (calling BlockStore,
//    updating `last_integrity_check_tip`, emitting the CRITICAL log) belongs in
//    the `Node` method that INVOKES this helper from `run_periodic_tasks`.
//  - P1 (genesis) intentionally returns `false` — a fresh node at tip=0 has
//    nothing to scan, and we do not want CRITICAL-log spam on cold starts.
//  - P6 (interval=0) guards against an operator misconfiguration or uninitialised
//    constant turning the periodic scan into a busy loop. Contract: require
//    actual tip advancement (strictly greater than `last_checked_tip`) before
//    running; when `interval == 0` AND `last == Some(tip)`, no advance
//    happened, so return `false`.
//  - P7 (tip backward) is defensive — in a healthy chain `current_tip` is
//    monotonically non-decreasing, but a rollback path could make
//    `last_checked_tip > current_tip` transiently. The helper must NOT
//    underflow and must NOT re-scan on a backward move.

#[cfg(test)]
mod integrity_check_tests {
    use super::*;

    // ---- P1: Genesis — no scan on a fresh node ----
    // Requirement: INC-I-034 / M-Choice2 / CHOICE 2 ("RUNTIME PERIODIC")
    // Acceptance: tip=0 with no prior scan returns false (no log spam on cold start).
    #[test]
    fn p1_genesis_tip_zero_no_prior_scan_returns_false() {
        let ran = should_run_integrity_check(0, None, INTEGRITY_CHECK_INTERVAL_BLOCKS);
        assert!(
            !ran,
            "tip=0 with no prior scan must return false (nothing to check at genesis)"
        );
    }

    // ---- P2: First-ever run, tip has crossed the threshold ----
    // Requirement: INC-I-034 / M-Choice2
    // Acceptance: first run after tip advances >= interval returns true.
    #[test]
    fn p2_first_run_tip_past_interval_returns_true() {
        let ran = should_run_integrity_check(1500, None, 1000);
        assert!(
            ran,
            "first-ever run with tip >= interval must return true (initial scan is due)"
        );
    }

    // ---- P3: Last scan too recent — must skip ----
    // Requirement: INC-I-034 / M-Choice2
    // Acceptance: fewer than `interval` blocks since last scan returns false.
    #[test]
    fn p3_last_scan_recent_returns_false() {
        let ran = should_run_integrity_check(1500, Some(1499), 1000);
        assert!(
            !ran,
            "delta=1 < interval=1000 must return false (too soon to re-scan)"
        );
    }

    // ---- P4: Exact boundary — inclusive >= ----
    // Requirement: INC-I-034 / M-Choice2
    // Acceptance: tip - last == interval returns true (inclusive boundary).
    #[test]
    fn p4_exact_boundary_returns_true() {
        let ran = should_run_integrity_check(2000, Some(1000), 1000);
        assert!(
            ran,
            "delta == interval must return true (inclusive >=, not strict >)"
        );
    }

    // ---- P5: Just past the boundary ----
    // Requirement: INC-I-034 / M-Choice2
    // Acceptance: tip - last == interval + 1 returns true.
    #[test]
    fn p5_just_past_boundary_returns_true() {
        let ran = should_run_integrity_check(2001, Some(1000), 1000);
        assert!(ran, "delta > interval must return true");
    }

    // ---- P6: Pathological zero interval, no tip advance ----
    // Requirement: INC-I-034 / M-Choice2 — guard against busy loop on misconfig.
    // Acceptance: interval=0 AND tip did not advance returns false.
    #[test]
    fn p6_zero_interval_no_advance_returns_false() {
        let ran = should_run_integrity_check(5, Some(5), 0);
        assert!(
            !ran,
            "interval=0 with no tip advance must return false (no busy-loop)"
        );
    }

    // ---- P7: Tip went backward — defensive ----
    // Requirement: INC-I-034 / M-Choice2
    // Acceptance: current_tip < last_checked_tip returns false; no underflow.
    #[test]
    fn p7_tip_backward_returns_false_no_underflow() {
        let ran = should_run_integrity_check(100, Some(500), 1000);
        assert!(
            !ran,
            "current_tip < last_checked_tip must return false (and must not underflow)"
        );
    }

    // ---- P8: u64::MAX boundary (adversarial) ----
    // Requirement: INC-I-034 / M-Choice2 — arithmetic must not overflow at extremes.
    // Acceptance: tip=MAX, last=MAX-1000, interval=1000 returns true.
    #[test]
    fn p8_u64_max_boundary_returns_true() {
        let ran = should_run_integrity_check(u64::MAX, Some(u64::MAX - 1000), 1000);
        assert!(
            ran,
            "delta == interval at u64::MAX must return true (no overflow)"
        );
    }

    // ---- P9: u64::MAX no advance (adversarial) ----
    // Requirement: INC-I-034 / M-Choice2
    // Acceptance: tip=MAX, last=MAX, interval=1000 returns false.
    #[test]
    fn p9_u64_max_no_advance_returns_false() {
        let ran = should_run_integrity_check(u64::MAX, Some(u64::MAX), 1000);
        assert!(
            !ran,
            "tip did not advance at u64::MAX must return false (no spurious re-scan)"
        );
    }

    // ---- Sanity guard on the documented default constant ----
    // Requirement: INC-I-034 / M-Choice2 — default interval is 1000 blocks.
    #[test]
    fn default_interval_constant_is_1000() {
        assert_eq!(
            INTEGRITY_CHECK_INTERVAL_BLOCKS, 1000,
            "INTEGRITY_CHECK_INTERVAL_BLOCKS default is locked to 1000 blocks (~3h of slot time)"
        );
    }
}
