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
        // Clean stale entries from seen_blocks_for_slot (keep last 10 slots)
        {
            let current_slot = self.chain_state.read().await.best_slot;
            self.seen_blocks_for_slot
                .retain(|&s| (s as u64) + 10 > current_slot as u64);
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

        // Fork sync binary search removed — fork recovery is now handled by
        // the ReorgHandler in the network crate's fork_recovery module.

        // DEEP FORK DETECTION: If peers consistently reject our chain tip (10+ empty
        // header responses), log warning. Fork sync handles recovery via binary search.
        {
            let is_deep_fork = self.sync_manager.read().await.is_deep_fork_detected();
            if is_deep_fork {
                warn!("[FORK] DEEP_FORK peers consistently reject our chain tip — fork sync will attempt recovery");
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
            let snap_bonds: u64 = self.epoch_bond_snapshot.values().sum();
            let snap_producers = self.epoch_bond_snapshot.len();
            warn!(
                "[HEALTH] h={} s={} hash={:.8} | peers={} best_peer_h={} best_peer_s={} net_tip_h={} net_tip_s={} | sync_fails={} fork_counter={} state={:?} | snap_epoch={} snap_bonds={} snap_producers={}",
                cs.best_height, cs.best_slot, cs.best_hash,
                peer_count, best_peer_h, best_peer_s, net_tip_h, net_tip_s,
                sync_fails, self.consecutive_fork_blocks, sync.sync_state_name(),
                self.epoch_bond_snapshot_epoch, snap_bonds, snap_producers
            );

            // INC-I-020: Stale tip recovery.
            // The sync engine ignores gaps of 1-2 blocks (assumes gossip will deliver).
            // But if gossip missed the block (reconnection, TTL expiry), the gap becomes
            // permanent — the node is "Idle" but stuck behind. This safety net runs every
            // 30s and requests the missing block directly from the best peer.
            let local_h = cs.best_height;
            let gap = best_peer_h.saturating_sub(local_h);
            if (1..=2).contains(&gap) && sync.sync_state_name() == "Idle" && peer_count > 0 {
                if let Some((peer_id, _peer_h, peer_hash)) = sync.best_peer_with_hash() {
                    warn!(
                        "[STALE_TIP] Behind by {} block(s) (local={}, peer={}). Requesting hash={:.8} from {}",
                        gap, local_h, best_peer_h, peer_hash, peer_id
                    );
                    drop(sync); // release read lock before write
                    drop(cs);
                    let request = SyncRequest::GetBlockByHash { hash: peer_hash };
                    if let Some(ref net) = self.network {
                        let _ = net.request_sync(peer_id, request).await;
                    }
                }
            }
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

        Ok(())
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
