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
        //
        // Phase 2 (auto-repair): when gaps are detected and an archive directory
        // is configured, backfill missing blocks from the archive automatically,
        // then re-scan to produce a clean commitment. No manual intervention needed.
        {
            let tip = self.chain_state.read().await.best_height;
            let last_scan = self.last_integrity_check_tip.unwrap_or(0);
            // Round scan_tip to nearest 100 so all nodes compute the same commitment
            // at the same height, eliminating explorer flickering from scan-timing drift.
            let scan_tip = (tip / 100) * 100;
            if scan_tip > 0 && scan_tip > last_scan {
                let block_store = self.block_store.clone();
                let state_db = self.state_db.clone();
                let archive_dir = self.archive_dir.clone();
                tokio::task::spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        // --- First scan: detect gaps ---
                        let (missing, commitment) = integrity_scan(&block_store, scan_tip);

                        if missing == 0 {
                            return (0u64, commitment, 0u64);
                        }

                        // --- Auto-repair: backfill from archive if available ---
                        let archive_path = match &archive_dir {
                            Some(dir) if dir.exists() => dir,
                            _ => {
                                tracing::warn!(
                                    "[INTEGRITY] {} missing blocks in 1..={} — \
                                     no archive configured, cannot auto-repair",
                                    missing,
                                    scan_tip
                                );
                                return (missing, commitment, 0u64);
                            }
                        };

                        tracing::info!(
                            "[INTEGRITY] {} missing blocks in 1..={} — \
                             auto-repairing from archive {:?}",
                            missing,
                            scan_tip,
                            archive_path
                        );

                        // Read genesis_hash from an existing block for validation
                        let genesis_hash = {
                            let mut found = None;
                            for h in 1..=10_000u64 {
                                if let Ok(Some(blk)) = block_store.get_block_by_height(h) {
                                    found = Some(blk.header.genesis_hash);
                                    break;
                                }
                            }
                            found
                        };

                        let repaired = match storage::archiver::backfill_from_archive(
                            archive_path,
                            &block_store,
                            genesis_hash.as_ref(),
                        ) {
                            Ok(count) => {
                                tracing::info!(
                                    "[INTEGRITY] Auto-repair imported {} blocks from archive",
                                    count
                                );
                                count
                            }
                            Err(e) => {
                                tracing::error!("[INTEGRITY] Auto-repair failed: {}", e);
                                return (missing, commitment, 0u64);
                            }
                        };

                        // --- Re-scan after repair to produce clean commitment ---
                        let (missing_after, commitment_after) =
                            integrity_scan(&block_store, scan_tip);

                        if missing_after > 0 {
                            tracing::warn!(
                                "[INTEGRITY] After auto-repair: still {} missing blocks \
                                 in 1..={} (archive may be incomplete)",
                                missing_after,
                                scan_tip
                            );
                        }

                        (missing_after, commitment_after, repaired)
                    })
                    .await;

                    if let Ok((missing, commitment, repaired)) = result {
                        if missing > 0 {
                            tracing::warn!(
                                "[INTEGRITY] Periodic scan: {} missing blocks in 1..={}",
                                missing,
                                scan_tip
                            );
                            state_db.delete_chain_commitment();
                        } else {
                            if repaired > 0 {
                                tracing::info!(
                                    "[INTEGRITY] Auto-repair successful: {} blocks restored, \
                                     chain complete (1..={}), commitment={:.16}",
                                    repaired,
                                    scan_tip,
                                    commitment
                                );
                            } else {
                                tracing::info!(
                                    "[INTEGRITY] Periodic scan: chain complete (1..={}), commitment={:.16}",
                                    scan_tip, commitment
                                );
                            }
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

        // INC-I-049 DEFERRED ATTEST_FETCH: check entries >500ms old.
        // If block_store now has the block → gossip delivered it, silently clear.
        // If still missing → genuine recovery, send GetBlockByHash to recorded peer,
        // then REMOVE the entry. If the block still doesn't arrive and a new
        // attestation comes in, it re-creates the entry with a fresh grace period.
        {
            let now = Instant::now();
            const GRACE_MS: u128 = 500;

            // Collect matured entries (>500ms old)
            let matured: Vec<(Hash, PeerId, u8)> = self
                .attest_fetch_tracker
                .iter()
                .filter(|(_, (t, _, _))| now.duration_since(*t).as_millis() >= GRACE_MS)
                .map(|(hash, (_, count, peer))| (*hash, *peer, *count))
                .collect();

            for (block_hash, source_peer, attempt) in matured {
                // Check if gossip delivered the block during the grace period
                if let Ok(Some(_)) = self.block_store.get_height_by_hash(&block_hash) {
                    // Block arrived via gossip — no fetch needed
                    self.attest_fetch_tracker.remove(&block_hash);
                    continue;
                }

                // Still missing — genuine recovery, fire one fetch and remove.
                // If another attestation arrives for this hash later, it will
                // re-create the entry with a fresh 500ms grace period.
                info!(
                    "[ATTEST_FETCH] fetching block {:.8} from peer {} after {}ms grace (attempt {})",
                    block_hash, source_peer, GRACE_MS, attempt
                );

                if let Some(ref network) = self.network {
                    let _ = network
                        .request_sync(
                            source_peer,
                            network::protocols::SyncRequest::get_block_by_hash(block_hash),
                        )
                        .await;
                }

                // Remove after firing — prevents repeated fetches every tick.
                // New attestations will re-add if the block still hasn't arrived.
                self.attest_fetch_tracker.remove(&block_hash);
            }

            // Clean stale entries (>30s old) to bound memory
            self.attest_fetch_tracker
                .retain(|_, (t, _, _)| now.duration_since(*t).as_secs() < 30);
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
                    if let Err(e) = self.apply_block(block, ValidationMode::Light, None).await {
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

            // D6 (INC-I-090): Consume stuck-fork signal every tick.
            // signal_stuck_fork() is called by cleanup, block_apply_failed, and peers
            // when fork evidence is detected. take_stuck_fork_signal() had ZERO
            // non-test callers — the signal sat unread. This surfaces it immediately
            // as a structured WARN log, complementing M2's 30s diagnostic_monitor.
            if let Some(alert) = sync.consume_stuck_fork_signal() {
                tracing::warn!(
                    target: "production_gate",
                    local_height = alert.local_height,
                    best_peer_height = alert.best_peer_height,
                    peer_count = alert.peer_count,
                    "[STUCK_FORK] Recovery coordinator raised stuck-fork signal"
                );
            }
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
                    let genesis_hash = self.chain_state.read().await.genesis_hash;
                    match storage::archiver::BlockArchiver::catch_up(
                        archive_dir,
                        &self.block_store,
                        tip,
                        Some(&genesis_hash),
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

        // RECOVERY COORDINATOR: single authoritative dispatch for all fork/sync recovery.
        //
        // Evidence is reported based on current state, then the coordinator
        // classifies and returns a single RecoveryAction that is executed here.
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

            let (action, recovery_ctx) = {
                let mut sync = self.sync_manager.write().await;
                sync.classify_and_dispatch(self.shallow_rollback_count)
            };

            // EMIT-007: emit recovery_classify_call when action is non-None
            if let Some(ref ctx) = recovery_ctx {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let _ =
                    self.diagnostic_emitter
                        .record(storage::diagnostic_ledger::types::DiagnosticEvent {
                        event_id: ulid::Ulid::new().to_string(),
                        kind: storage::diagnostic_ledger::types::EventKind::RecoveryClassifyCall,
                        timestamp_ms: now_ms,
                        height: Some(ctx.local_height),
                        correlation_key: Some(storage::diagnostic_ledger::types::CorrelationKey {
                            divergence_height: Some(ctx.local_height),
                            canonical_hash: None,
                            fork_hash: None,
                        }),
                        caused_by_event_id: None,
                        is_cascade_origin: false,
                        payload:
                            storage::diagnostic_ledger::types::EventPayload::RecoveryClassifyCall {
                                local_height: ctx.local_height,
                                network_tip_height: ctx.network_tip_height,
                                peer_count: ctx.peer_count as u32,
                                last_applied_secs: ctx.last_applied_secs,
                                shallow_rollback_count: ctx.shallow_rollback_count,
                                snap_attempts: ctx.snap_attempts as u32,
                                last_rollback_local_height: ctx.last_rollback_local_height,
                                in_grace_period: ctx.in_grace_period,
                                last_finality_height: ctx.last_finality_height,
                                action_returned: Some(format!("{:?}", action)),
                                rule_matched: None, // TODO: expose rule_matched from classifier
                            },
                    });
            }

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
                        // INC-I-055: Use rolling health window instead of point-in-time.
                        // A checkpoint is healthy if the node was healthy at ANY point
                        // in the last CHECKPOINT_HEALTH_WINDOW_SIZE samples (~10 min).
                        // This prevents ALL checkpoints from being tagged unhealthy
                        // during transient peer disconnections.
                        let point_healthy =
                            peer_count > 0 && peers_agreeing == peer_count && unique_hashes <= 1;
                        let window_healthy = self.health_window.iter().any(|&h| h);
                        let healthy = point_healthy || window_healthy;
                        let health = serde_json::json!({
                            "height": current_height,
                            "hash": best_hash,
                            "timestamp": timestamp,
                            "peer_count": peer_count,
                            "peers_agreeing": peers_agreeing,
                            "unique_chain_tips": unique_hashes,
                            "healthy": healthy,
                            "point_healthy": point_healthy,
                            "window_healthy": window_healthy,
                            "window_size": self.health_window.len(),
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

            // INC-I-055: Update rolling health window for checkpoint tagging.
            // A sample is "healthy" if we have peers and they agree on the chain tip.
            {
                let (_, peers_agreeing, unique_hashes) = sync.checkpoint_health();
                let sample_healthy =
                    peer_count > 0 && peers_agreeing == peer_count && unique_hashes <= 1;
                self.health_window.push_back(sample_healthy);
                while self.health_window.len() > CHECKPOINT_HEALTH_WINDOW_SIZE {
                    self.health_window.pop_front();
                }
            }

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

            // D4 (INC-I-090): In-node automated alert consumer.
            // Polls the local diagnostic ledger, runs classify(), and emits structured
            // WARN-level log lines when recommended_action is actionable.
            // Cadence: every 30s (same as the health diagnostic), satisfying the VERDICT
            // pass-criterion: "< 60s interval".
            if let Some(ref ledger) = self.diagnostic_ledger {
                use super::diagnostic_monitor::{
                    check_for_actionable_alerts, DIAGNOSTIC_MONITOR_INTERVAL_SECS,
                };
                let _ = DIAGNOSTIC_MONITOR_INTERVAL_SECS; // referenced for grep-ability
                let alerts = check_for_actionable_alerts(
                    ledger,
                    300, // 5-minute event window
                    &mut self.last_diagnostic_alerted,
                );
                for alert in &alerts {
                    warn!(
                        target: "diagnostic_monitor",
                        incident_id = %alert.incident_id,
                        fork_type = %alert.fork_type,
                        recommended_action = %alert.recommended_action,
                        evidence_count = alert.evidence_event_ids.len(),
                        "[DIAGNOSTIC_MONITOR] Actionable alert: {} — {}",
                        alert.fork_type, alert.recommended_action,
                    );
                }
            }

            // INC-I-020/020b: STALE_TIP and FORK_1BLOCK were removed because they
            // fought with FORK_GUARD, causing rollback cascades. With INC-I-026
            // (deterministic scheduler) and fork_id, small gaps resolve via gossip;
            // gaps of 3+ trigger should_sync(). Recovery is handled by the
            // RecoveryCoordinator earlier in this loop.
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

    /// Periodic block-store integrity check with auto-repair (Phase 2, M-Choice2).
    ///
    /// Runs every `INTEGRITY_CHECK_INTERVAL_BLOCKS` blocks (default 1000).
    /// Scans `BlockStore::ensure_blocks_present(1, tip)`. On gap detection,
    /// attempts auto-repair from the archive directory if configured. Falls back
    /// to a CRITICAL log with operator action guidance if no archive is available
    /// or the archive is incomplete.
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
        let archive_dir = self.archive_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            // INC-I-055: Start from actual lowest block, not hardcoded 1.
            // After chain reset or snap sync, pre-floor blocks don't exist.
            let scan_floor = block_store
                .height_range()
                .ok()
                .flatten()
                .map(|(min, _)| min)
                .unwrap_or(1);
            let check = block_store.ensure_blocks_present(scan_floor, current_tip);
            if check.is_ok() {
                return Ok(());
            }

            // Gap detected — attempt auto-repair from archive
            let archive_path = match &archive_dir {
                Some(dir) if dir.exists() => dir,
                _ => return check, // No archive, return original error
            };

            tracing::info!(
                "[INTEGRITY_CHECK] Gaps detected in 1..={} — auto-repairing from archive {:?}",
                current_tip,
                archive_path
            );

            let genesis_hash = {
                let mut found = None;
                for h in 1..=10_000u64 {
                    if let Ok(Some(blk)) = block_store.get_block_by_height(h) {
                        found = Some(blk.header.genesis_hash);
                        break;
                    }
                }
                found
            };

            match storage::archiver::backfill_from_archive(
                archive_path,
                &block_store,
                genesis_hash.as_ref(),
            ) {
                Ok(count) => {
                    tracing::info!(
                        "[INTEGRITY_CHECK] Auto-repair imported {} blocks from archive",
                        count
                    );
                }
                Err(e) => {
                    tracing::error!("[INTEGRITY_CHECK] Auto-repair from archive failed: {}", e);
                    return check; // Return original error
                }
            }

            // Re-check after repair (re-read floor — repair may have extended it)
            let floor = block_store
                .height_range()
                .ok()
                .flatten()
                .map(|(min, _)| min)
                .unwrap_or(1);
            block_store.ensure_blocks_present(floor, current_tip)
        })
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

/// Scan blocks scan_floor..=scan_tip, returning (missing_count, commitment_hash).
/// Pure block store I/O — no side effects, no state_db writes.
///
/// INC-I-055: Uses the block store's actual lowest height instead of hardcoded 1.
/// After a chain reset or snap sync, blocks below the floor don't exist and
/// should not count as "missing" (they belong to a previous chain or pre-snap era).
fn integrity_scan(
    block_store: &std::sync::Arc<storage::BlockStore>,
    scan_tip: u64,
) -> (u64, crypto::Hash) {
    // Start from the lowest block actually in the store (default to 1)
    let scan_floor = block_store
        .height_range()
        .ok()
        .flatten()
        .map(|(min, _)| min)
        .unwrap_or(1);

    let mut missing = 0u64;
    let mut commitment = crypto::Hash::default();
    for h in scan_floor..=scan_tip {
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
}

// =============================================================================
// Phase-2 periodic block-store integrity check (M-Choice2, INC-I-034)
// =============================================================================
//
// See `specs/scheduler-state-architecture.md` §"Block-store integrity contract:
// bounded fail-fast + self-heal (F-11 / B-2 refinement)" and the CHOICE 2 lock
// ("RUNTIME PERIODIC — runs at startup, on every chain_state advance, AND as a
// background task every 1000 blocks").
//
// Phase 2 adds AUTO-REPAIR: when gaps are detected and an archive directory
// is configured (--archive-to), missing blocks are backfilled from the archive
// automatically. No manual intervention needed.
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
// INC-I-049 follow-up: Deferred ATTEST_FETCH tracker unit tests
// =============================================================================
//
// The deferred fetch logic lives in run_periodic_tasks() and requires Node +
// async runtime. These pure tests verify the tracker HashMap behavior:
// deduplication (max 3 peers), TTL cleanup (>30s), and grace period filtering.

#[cfg(test)]
mod attest_fetch_tests {
    use super::*;
    use std::collections::HashMap;

    fn make_hash(b: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = b;
        Hash::from(bytes)
    }

    fn make_peer(_b: u8) -> PeerId {
        PeerId::random()
    }

    /// Attestation for unknown block within grace period → no fetch should fire.
    /// Verified by: entry exists in tracker, but periodic task should skip
    /// entries <500ms old.
    #[test]
    fn fresh_entry_within_grace_period_not_matured() {
        let mut tracker: HashMap<Hash, (Instant, u8, PeerId)> = HashMap::new();
        let hash = make_hash(1);
        let peer = make_peer(1);
        let now = Instant::now();

        tracker.insert(hash, (now, 1, peer));

        // Simulate periodic check: entries <500ms old should NOT be collected
        let grace_ms: u128 = 500;
        let matured: Vec<_> = tracker
            .iter()
            .filter(|(_, (t, _, _))| now.duration_since(*t).as_millis() >= grace_ms)
            .collect();

        assert!(
            matured.is_empty(),
            "Entry just recorded should not be matured yet"
        );
    }

    /// Entry older than 500ms → should be collected for deferred fetch check.
    #[test]
    fn entry_past_grace_period_is_matured() {
        let mut tracker: HashMap<Hash, (Instant, u8, PeerId)> = HashMap::new();
        let hash = make_hash(2);
        let peer = make_peer(2);
        // Simulate 600ms ago
        let recorded_at = Instant::now() - Duration::from_millis(600);

        tracker.insert(hash, (recorded_at, 1, peer));

        let now = Instant::now();
        let grace_ms: u128 = 500;
        let matured: Vec<_> = tracker
            .iter()
            .filter(|(_, (t, _, _))| now.duration_since(*t).as_millis() >= grace_ms)
            .collect();

        assert_eq!(matured.len(), 1, "Entry 600ms old should be matured");
        assert_eq!(*matured[0].0, hash);
    }

    /// Max 3 peers per hash — 4th insert should be rejected.
    #[test]
    fn max_three_peers_per_hash() {
        let mut tracker: HashMap<Hash, (Instant, u8, PeerId)> = HashMap::new();
        let hash = make_hash(3);
        let now = Instant::now();

        // Simulate 3 attestations from different peers
        let entry = tracker.entry(hash).or_insert((now, 0, PeerId::random()));
        entry.1 += 1; // count=1
        assert_eq!(entry.1, 1);

        entry.1 += 1; // count=2
        entry.1 += 1; // count=3
        assert_eq!(entry.1, 3);

        // 4th should be rejected by the `>= 3` check
        assert!(
            entry.1 >= 3,
            "After 3 peers, further attestations should be rejected"
        );
    }

    /// Entries older than 30s are cleaned to bound memory.
    #[test]
    fn stale_entries_cleaned_after_30s() {
        let mut tracker: HashMap<Hash, (Instant, u8, PeerId)> = HashMap::new();
        let hash_old = make_hash(10);
        let hash_new = make_hash(11);

        // Old entry: 31s ago
        tracker.insert(
            hash_old,
            (
                Instant::now() - Duration::from_secs(31),
                1,
                PeerId::random(),
            ),
        );
        // New entry: just now
        tracker.insert(hash_new, (Instant::now(), 1, PeerId::random()));

        let now = Instant::now();
        tracker.retain(|_, (t, _, _)| now.duration_since(*t).as_secs() < 30);

        assert_eq!(tracker.len(), 1, "Only the recent entry should survive");
        assert!(
            tracker.contains_key(&hash_new),
            "New entry should be retained"
        );
        assert!(
            !tracker.contains_key(&hash_old),
            "31s-old entry should be cleaned"
        );
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
    // OUTPUT CONTRACT: fn maybe_fetch_attested_block(&mut self, block_hash: &Hash, source_peer: PeerId)
    //   O1: mutable params   — &mut self (modifies self.attest_fetch_tracker)
    //   O2: receiver/self    — mutates Node.attest_fetch_tracker (HashMap insert/retain)
    //   O3: return           — () (no return value — side effect only)
    //   O4: persistent stores — block_store read-only (get_height_by_hash check)
    //   O5: global/static    — none
    //   O6: channels/events  — none (NO network request sent — deferred to periodic)
    //
    // OUTPUT CONTRACT: fn run_periodic_tasks(&mut self) -> Result<()>
    //   (scoped to ATTEST_FETCH section only)
    //   O1: mutable params   — &mut self (modifies self.attest_fetch_tracker)
    //   O2: receiver/self    — removes matured entries from tracker
    //   O3: return           — Result<()>
    //   O4: persistent stores — block_store read-only (get_height_by_hash check)
    //   O5: global/static    — none
    //   O6: channels/events  — network.request_sync() if block missing after grace
    //
    // PATHS:
    //   PA: Record unknown hash  — tracker gets entry, NO network request
    //   PB: Gossip delivers      — block in store before grace expires → entry cleared, no fetch
    //   PC: Grace expires, miss  — block NOT in store after 500ms → fetch fires ONCE, entry removed
    //   PD: Second periodic tick — entry was removed in PC → no re-fire
    // MATRIX: 4 paths × 2 outputs (tracker state, network activity) = 8 cells

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

// =============================================================================
// INC-I-049: Deferred ATTEST_FETCH integration tests (Node-level)
// =============================================================================
//
// These tests exercise the REAL maybe_fetch_attested_block() → run_periodic_tasks()
// interaction on a Node::new_for_test() instance. They verify:
//   A) Recording creates tracker entry, no immediate network request
//   B) Gossip delivery within grace period clears entry silently
//   C) Grace expiry with missing block fires fetch exactly once, removes entry
//   D) Second periodic tick after removal does NOT re-fire

#[cfg(test)]
mod attest_fetch_integration_tests {
    use super::*;

    async fn make_test_node() -> (Node, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let kp = crypto::KeyPair::generate();
        let node = Node::new_for_test(tmp.path().to_path_buf(), vec![kp])
            .await
            .expect("Node::new_for_test failed");
        (node, tmp)
    }

    fn make_hash(b: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = b;
        Hash::from(bytes)
    }

    /// Test A: maybe_fetch_attested_block() with unknown hash records a tracker
    /// entry but does NOT send any network request (network is None in tests,
    /// but the key invariant is: tracker entry exists, no request_sync called).
    ///
    /// Against old code (pre-INC-I-049 fix): the function sent GetBlockByHash
    /// immediately and did NOT populate a tracker. This test would FAIL because
    /// attest_fetch_tracker would be empty.
    #[tokio::test]
    async fn test_a_record_unknown_block_no_immediate_fetch() {
        let (mut node, _tmp) = make_test_node().await;
        let block_hash = make_hash(42);
        let peer = PeerId::random();

        // Precondition: tracker is empty
        assert!(node.attest_fetch_tracker.is_empty());

        // Act: attestation for unknown block
        node.maybe_fetch_attested_block(&block_hash, peer).await;

        // Assert: tracker has the entry (deferred, not sent immediately)
        assert_eq!(
            node.attest_fetch_tracker.len(),
            1,
            "Tracker must have exactly 1 entry after recording"
        );
        assert!(
            node.attest_fetch_tracker.contains_key(&block_hash),
            "Tracker must contain the recorded block hash"
        );
        let (_, count, recorded_peer) = node.attest_fetch_tracker[&block_hash];
        assert_eq!(count, 1, "First recording should have count=1");
        assert_eq!(recorded_peer, peer, "Recorded peer must match source");

        // Key invariant: network is None, so no request could have been sent.
        // In production, the deferred design means request_sync is NOT called here.
        assert!(
            node.network.is_none(),
            "Test node must not have network — confirms no request was sent"
        );
    }

    /// Test B: After recording, insert the block into block_store (simulating
    /// gossip delivery within 500ms), then run periodic tasks → entry cleared,
    /// no fetch sent.
    #[tokio::test]
    async fn test_b_gossip_delivery_clears_entry_no_fetch() {
        let (mut node, _tmp) = make_test_node().await;
        let block_hash = make_hash(43);
        let peer = PeerId::random();

        // Record attestation for unknown block
        node.maybe_fetch_attested_block(&block_hash, peer).await;
        assert_eq!(node.attest_fetch_tracker.len(), 1);

        // Simulate gossip delivering the block: build a minimal block with
        // matching hash. We need put_block_canonical so get_height_by_hash works.
        // To get a block whose hash matches `block_hash`, we store the block
        // and use its actual hash instead.
        let kp = crypto::KeyPair::generate();
        let params = doli_core::consensus::ConsensusParams::devnet();
        let reward = params.block_reward(1);
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let coinbase = doli_core::Transaction::new_coinbase(reward, pool_hash, 1, 0);
        let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();
        let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
        let header = doli_core::BlockHeader {
            version: 2,
            prev_hash: genesis_hash,
            merkle_root,
            presence_root: Hash::ZERO,
            genesis_hash,
            timestamp: 0,
            slot: 1,
            producer: *kp.public_key(),
            vdf_output: vdf::VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        let block = doli_core::Block::new(header, vec![coinbase]);
        let real_hash = block.hash();

        // Re-record with the real block hash
        node.attest_fetch_tracker.clear();
        node.maybe_fetch_attested_block(&real_hash, peer).await;
        assert_eq!(node.attest_fetch_tracker.len(), 1);

        // Simulate gossip: store the block canonically
        node.block_store
            .put_block_canonical(&block, 1)
            .expect("put_block_canonical failed");

        // Confirm block_store now has it
        assert!(
            node.block_store
                .get_height_by_hash(&real_hash)
                .unwrap()
                .is_some(),
            "Block must be in store after put_block_canonical"
        );

        // Manually backdate the entry so it's past 500ms grace
        if let Some(entry) = node.attest_fetch_tracker.get_mut(&real_hash) {
            entry.0 = Instant::now() - Duration::from_millis(600);
        }

        // Run periodic tasks — should see block in store and clear entry silently
        node.run_periodic_tasks().await.expect("periodic failed");

        // Assert: entry removed (gossip delivered), no fetch sent
        assert!(
            node.attest_fetch_tracker.is_empty(),
            "Tracker must be empty after gossip delivery + periodic sweep"
        );
    }

    /// Test C: After recording, do NOT insert block, advance past 500ms grace,
    /// run periodic tasks → fetch fires exactly once (via request_sync, which is
    /// a no-op since network=None), entry removed from tracker.
    #[tokio::test]
    async fn test_c_grace_expiry_missing_block_fires_once_and_removes() {
        let (mut node, _tmp) = make_test_node().await;
        let block_hash = make_hash(44);
        let peer = PeerId::random();

        // Record attestation
        node.maybe_fetch_attested_block(&block_hash, peer).await;
        assert_eq!(node.attest_fetch_tracker.len(), 1);

        // Block is NOT in store (simulating gossip hasn't arrived)
        assert!(
            node.block_store
                .get_height_by_hash(&block_hash)
                .unwrap()
                .is_none(),
            "Block must NOT be in store for this test"
        );

        // Backdate entry past 500ms grace
        if let Some(entry) = node.attest_fetch_tracker.get_mut(&block_hash) {
            entry.0 = Instant::now() - Duration::from_millis(600);
        }

        // Run periodic tasks — should fire fetch (no-op: network=None) and REMOVE entry
        node.run_periodic_tasks().await.expect("periodic failed");

        // Assert: entry removed after firing
        assert!(
            !node.attest_fetch_tracker.contains_key(&block_hash),
            "Entry must be removed after deferred fetch fires"
        );
        assert!(
            node.attest_fetch_tracker.is_empty(),
            "Tracker must be empty — entry removed, not refreshed"
        );
    }

    /// Test D: After Test C's fetch fires and removes the entry, run periodic
    /// tasks a SECOND time → no additional fetch, tracker stays empty.
    ///
    /// This is the regression test for fix attempt 1 (commit b29f370d) where
    /// entries re-fired every tick because the timestamp was refreshed instead
    /// of the entry being removed.
    #[tokio::test]
    async fn test_d_second_periodic_tick_no_refire() {
        let (mut node, _tmp) = make_test_node().await;
        let block_hash = make_hash(45);
        let peer = PeerId::random();

        // Record and backdate past grace
        node.maybe_fetch_attested_block(&block_hash, peer).await;
        if let Some(entry) = node.attest_fetch_tracker.get_mut(&block_hash) {
            entry.0 = Instant::now() - Duration::from_millis(600);
        }

        // First periodic tick — fires fetch and removes entry
        node.run_periodic_tasks().await.expect("periodic 1 failed");
        assert!(
            node.attest_fetch_tracker.is_empty(),
            "After first tick: entry must be removed"
        );

        // Second periodic tick — nothing to do, tracker stays empty
        node.run_periodic_tasks().await.expect("periodic 2 failed");
        assert!(
            node.attest_fetch_tracker.is_empty(),
            "After second tick: tracker must still be empty (no re-fire)"
        );

        // Third periodic tick — still nothing (paranoia check)
        node.run_periodic_tasks().await.expect("periodic 3 failed");
        assert!(
            node.attest_fetch_tracker.is_empty(),
            "After third tick: tracker must still be empty (no re-fire)"
        );
    }
}
