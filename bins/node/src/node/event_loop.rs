use super::*;

impl Node {
    /// Main event loop
    pub(super) async fn run_event_loop(&mut self) -> Result<()> {
        info!("Entering main event loop");

        // Check production opportunity - faster for devnet to catch the 700ms heartbeat window
        let production_interval = if self.config.network == Network::Devnet {
            Duration::from_millis(200) // 5 checks per second for devnet
        } else {
            Duration::from_secs(1)
        };
        let mut production_timer = tokio::time::interval(production_interval);

        // Track when production last ran, to guarantee scheduling under event flooding.
        // Without this, biased select! starves production when 100+ peers generate
        // continuous gossip/sync events (the event channel is never empty).
        let mut last_production_check = Instant::now();

        // Gossip our producer identity using adaptive intervals
        // This ensures nodes that aren't directly connected (e.g., Node 2 -> Node 1 -> Node 3)
        // learn about each other through the GossipSub mesh relay
        // Phase 1: Use AdaptiveGossip for dynamic interval adjustment
        let mut current_gossip_interval = self.adaptive_gossip.read().await.interval();
        let mut gossip_timer = tokio::time::interval(current_gossip_interval);

        loop {
            // Check shutdown flag
            if *self.shutdown.read().await {
                break;
            }

            // Use select! to handle network events, production timer, and gossip timer
            // BIASED SELECT: Network events have priority over production
            //
            // This is critical for preventing propagation race forks. Without bias,
            // tokio::select! can choose the production branch even when a NewBlock
            // event is ready in the network queue. This causes nodes to produce
            // blocks on stale chain tips, creating forks.
            //
            // With biased select, we always process pending network events first,
            // ensuring the chain tip is up-to-date before attempting production.
            //
            // PRODUCTION ESCAPE HATCH: After each network event, we check if
            // production is overdue (elapsed > production_interval). If so, we
            // force a production check regardless of pending events. This prevents
            // event flooding from permanently starving block production — the root
            // cause of chain stalls at 100+ peers (see diagnosis-report.md).
            tokio::select! {
                biased;

                // Network event received (HIGHEST PRIORITY)
                // Must be first to ensure blocks are processed before production
                event = async {
                    if let Some(ref mut network) = self.network {
                        network.next_event().await
                    } else {
                        std::future::pending::<Option<NetworkEvent>>().await
                    }
                } => {
                    if let Some(event) = event {
                        if let Err(e) = self.handle_network_event(event).await {
                            warn!("Error handling network event: {}", e);
                        }
                    }

                    // PRODUCTION ESCAPE HATCH: If production hasn't run for a full
                    // interval, force it now. This guarantees production runs at least
                    // once per interval even under infinite event load.
                    if last_production_check.elapsed() >= production_interval {
                        if self.producer_key.is_some() {
                            if let Err(e) = self.try_produce_block().await {
                                warn!("Block production error: {}", e);
                            }
                        }
                        if let Err(e) = self.run_periodic_tasks().await {
                            warn!("Periodic task error: {}", e);
                        }
                        last_production_check = Instant::now();
                        self.sync_requests_this_interval = 0;
                    }
                }

                // Production timer tick
                _ = production_timer.tick() => {
                    // Check for block production opportunity
                    if self.producer_key.is_some() {
                        if let Err(e) = self.try_produce_block().await {
                            warn!("Block production error: {}", e);
                        }
                    }

                    // Run periodic tasks
                    if let Err(e) = self.run_periodic_tasks().await {
                        warn!("Periodic task error: {}", e);
                    }
                    last_production_check = Instant::now();
                    self.sync_requests_this_interval = 0;
                }

                // Gossip timer tick - ANTI-ENTROPY: broadcast producer view
                // Phase 1: Uses adaptive intervals based on network activity
                // Phase 2: Uses delta sync (bloom filter) for large networks
                _ = gossip_timer.tick() => {
                    let genesis_active = {
                        let state = self.chain_state.read().await;
                        self.config.network.is_in_genesis(state.best_height + 1)
                    };
                    if self.config.network == Network::Testnet || self.config.network == Network::Devnet || genesis_active {
                        if let Some(ref network) = self.network {
                            // Update our producer announcement if we're a producer
                            if let Some(ref key) = self.producer_key {
                                let seq = self.announcement_sequence.fetch_add(1, Ordering::SeqCst);
                                let gh = self.chain_state.read().await.genesis_hash;
                                let announcement = ProducerAnnouncement::new(key, self.config.network.id(), seq, gh);

                                {
                                    let mut gset = self.producer_gset.write().await;
                                    let _ = gset.merge_one(announcement.clone());

                                    // Purge ghost producers: entries older than 4 hours
                                    // (2x the active_producers liveness window of 7200s).
                                    // Without this, nodes that announced once and disappeared
                                    // persist forever, inflating the slot % n denominator.
                                    let purged = gset.purge_stale(14400);
                                    if purged > 0 {
                                        info!("GSet: purged {} ghost producer(s)", purged);
                                        if let Err(e) = gset.persist_to_disk() {
                                            warn!("Failed to persist GSet after purge: {}", e);
                                        }
                                    }
                                }

                                *self.our_announcement.write().await = Some(announcement);
                            }

                            // SCALE-T2-002: Always use delta gossip (bloom filter).
                            // Full-state broadcast is O(N * mesh_n * announcements) per round,
                            // which creates ~650K messages at 5000 nodes. Delta gossip sends
                            // a single bloom filter (~64 bytes) and only missing announcements
                            // come back — ~10x reduction in gossip traffic.
                            //
                            // Fallback to full sync only during first 30s after startup when
                            // the GSet is empty (bloom filter would be meaningless).
                            let producer_count = {
                                let gset = self.producer_gset.read().await;
                                gset.len()
                            };

                            if producer_count > 0 {
                                // DELTA SYNC: Send bloom filter, peers respond with missing announcements
                                let bloom = {
                                    let gset = self.producer_gset.read().await;
                                    gset.to_bloom_filter()
                                };
                                debug!(
                                    "Delta sync: broadcasting bloom filter ({} bytes) for {} producers",
                                    bloom.size_bytes(),
                                    producer_count
                                );
                                let _ = network.broadcast_producer_digest(bloom).await;
                            } else {
                                // FULL SYNC FALLBACK: Only when GSet is empty (fresh node startup).
                                // Once any producer is known, delta sync takes over.
                                let announcements = {
                                    let gset = self.producer_gset.read().await;
                                    gset.export()
                                };

                                if !announcements.is_empty() {
                                    debug!(
                                        "Full sync (startup fallback): broadcasting {} producer announcements",
                                        announcements.len()
                                    );
                                    let _ = network.broadcast_producer_announcements(announcements).await;
                                }
                            }

                            // Log producer schedule and detect divergence between GSet and known_producers
                            let gset_list = {
                                let gset = self.producer_gset.read().await;
                                gset.sorted_producers()
                            };
                            let known_list = self.known_producers.read().await.clone();
                            if !gset_list.is_empty() || !known_list.is_empty() {
                                let gset_hashes: Vec<String> = gset_list.iter()
                                    .map(|p| crypto_hash(p.as_bytes()).to_hex()[..8].to_string())
                                    .collect();
                                let known_hashes: Vec<String> = known_list.iter()
                                    .map(|p| crypto_hash(p.as_bytes()).to_hex()[..8].to_string())
                                    .collect();
                                if gset_hashes != known_hashes {
                                    warn!(
                                        "Producer schedule DIVERGENCE: gset={:?} (count={}) vs known={:?} (count={})",
                                        gset_hashes, gset_list.len(), known_hashes, known_list.len()
                                    );
                                } else {
                                    info!(
                                        "Producer schedule view: {:?} (count={}, source=gset)",
                                        gset_hashes, gset_list.len()
                                    );
                                }
                            }

                            // Phase 1: Update gossip interval if adaptive gossip changed it
                            let new_interval = self.adaptive_gossip.read().await.interval();
                            if new_interval != current_gossip_interval {
                                debug!(
                                    "Adaptive gossip: interval changed {:?} -> {:?}",
                                    current_gossip_interval, new_interval
                                );
                                current_gossip_interval = new_interval;
                                gossip_timer = tokio::time::interval(current_gossip_interval);
                            }
                        }
                    }
                }

            }
        }

        Ok(())
    }

    /// Handle network events
    pub(super) async fn handle_network_event(&mut self, event: NetworkEvent) -> Result<()> {
        match event {
            NetworkEvent::PeerConnected(peer_id) => {
                info!("Peer connected: {}", peer_id);

                // Track when we first connected to a peer (for bootstrap discovery grace period)
                if self.first_peer_connected.is_none() {
                    self.first_peer_connected = Some(Instant::now());
                    info!("First peer connected - starting discovery grace period");
                }

                // Enable bootstrap gate in SyncManager - production will be blocked
                // until we receive at least one peer status response
                self.sync_manager.write().await.set_peer_connected();

                // Request status from the new peer to learn their chain state
                // Include our producer pubkey so peers can discover us before blocks are exchanged
                let genesis_hash = self.chain_state.read().await.genesis_hash;
                let status_request = if let Some(ref key) = self.producer_key {
                    network::protocols::StatusRequest::with_producer(
                        self.config.network.id(),
                        genesis_hash,
                        *key.public_key(),
                    )
                } else {
                    network::protocols::StatusRequest::new(self.config.network.id(), genesis_hash)
                };

                if let Some(ref network) = self.network {
                    if let Err(e) = network.request_status(peer_id, status_request).await {
                        warn!("Failed to request status from peer {}: {}", peer_id, e);
                    } else {
                        debug!("Requested status from peer {}", peer_id);
                    }
                }
            }

            NetworkEvent::PeerDisconnected(peer_id) => {
                info!("Peer disconnected: {}", peer_id);
                self.sync_manager.write().await.remove_peer(&peer_id);

                // Rate-limited reconnect: delegate to the periodic redial in
                // run_periodic_tasks() (every slot_duration ≈ 10s).  Only do an
                // immediate dial if we haven't tried recently — this prevents a
                // spin loop where rapid connect/disconnect floods the event queue
                // and starves the production timer via the biased select!.
                let peer_count = self.sync_manager.read().await.peer_count();
                if peer_count == 0 && !self.config.bootstrap_nodes.is_empty() {
                    let recently_dialed = self
                        .last_peer_redial
                        .map(|t| t.elapsed().as_secs() < self.params.slot_duration)
                        .unwrap_or(false);
                    if !recently_dialed {
                        info!("Lost all peers — reconnecting to bootstrap nodes");
                        self.last_peer_redial = Some(std::time::Instant::now());
                        if let Some(ref network) = self.network {
                            for addr in &self.config.bootstrap_nodes {
                                if let Err(e) = network.connect(addr).await {
                                    warn!("Failed to reconnect to bootstrap {}: {}", addr, e);
                                }
                            }
                        }
                    }
                }
            }

            NetworkEvent::NewBlock(block, source_peer) => {
                // Skip gossip blocks during snap sync — they cause spurious fork
                // recovery that corrupts state mid-download.
                if self.sync_manager.read().await.state().is_snap_syncing() {
                    debug!("Ignoring gossip block {} during snap sync", block.hash());
                    return Ok(());
                }

                debug!("Received new block: {} from {}", block.hash(), source_peer);

                // DEFENSE: Slot sanity — reject gossip blocks with wildly wrong slots.
                // Prevents genesis-time-hijack attacks where a node compiled with a
                // different GENESIS_TIME produces blocks with desfasados slots.
                // Only applies to GOSSIP blocks — sync blocks (header-first download)
                // bypass this via apply_block() directly, so initial sync is unaffected.
                {
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let current_slot = self.params.timestamp_to_slot(now_secs) as u64;
                    let block_slot = block.header.slot as u64;

                    if block_slot > current_slot + consensus::MAX_FUTURE_SLOTS {
                        warn!(
                            "SLOT_SANITY: Rejecting gossip block {} — slot {} too far in future (current={}, limit=+{})",
                            block.hash(), block_slot, current_slot, consensus::MAX_FUTURE_SLOTS
                        );
                        return Ok(());
                    }
                    if current_slot > block_slot + consensus::MAX_PAST_SLOTS {
                        warn!(
                            "SLOT_SANITY: Rejecting gossip block {} — slot {} too far in past (current={}, limit=-{})",
                            block.hash(), block_slot, current_slot, consensus::MAX_PAST_SLOTS
                        );
                        return Ok(());
                    }
                }

                // Update network tip slot from gossip - this tells us what slot the network has reached
                // even if we don't know which specific peer sent the block.
                // This is critical for the "behind peers" production safety check.
                //
                // Note: Height is updated when blocks are successfully applied (in apply_block).
                {
                    let mut sync = self.sync_manager.write().await;
                    // Only refresh the specific source peer — refreshing ALL peers
                    // masks actually-stale peers and defeats stale chain detection.
                    sync.note_block_received_from_peer(source_peer);
                    sync.update_network_tip_slot(block.header.slot);
                    sync.note_block_received_via_gossip();
                }
                self.handle_new_block(block, source_peer).await?;
            }

            NetworkEvent::NewHeader(header) => {
                debug!(
                    "Header pre-announcement: slot={} producer={:.16} hash={:.16}",
                    header.slot,
                    hex::encode(header.producer.as_bytes()),
                    hex::encode(header.hash().as_bytes()),
                );
            }

            NetworkEvent::NewTransaction(tx) => {
                debug!("Received new transaction: {}", tx.hash());
                self.handle_new_transaction(tx).await?;
            }

            NetworkEvent::PeerStatus { peer_id, status } => {
                debug!(
                    "Peer {} status: height={}, slot={}",
                    peer_id, status.best_height, status.best_slot
                );
                {
                    let mut sync = self.sync_manager.write().await;
                    sync.add_peer(
                        peer_id,
                        status.best_height,
                        status.best_hash,
                        status.best_slot,
                    );
                    // CRITICAL: Notify SyncManager that we received a valid peer status.
                    // This satisfies the bootstrap gate and allows production to proceed.
                    sync.note_peer_status_received();
                }

                // BOOTSTRAP PRODUCER DISCOVERY: If the peer is a producer, add them to
                // known_producers. This allows nodes to discover each other
                // before any blocks are exchanged, solving the chicken-and-egg problem
                // where blocks can't be applied (they look like forks) because producers
                // don't know about each other.
                if let Some(ref producer_pubkey) = status.producer_pubkey {
                    let genesis_active = {
                        let state = self.chain_state.read().await;
                        self.config.network.is_in_genesis(state.best_height + 1)
                    };
                    if self.config.network == Network::Testnet
                        || self.config.network == Network::Devnet
                        || genesis_active
                    {
                        let mut known = self.known_producers.write().await;
                        if !known.contains(producer_pubkey) {
                            known.push(*producer_pubkey);
                            // Keep sorted for deterministic ordering
                            known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                            let pubkey_hash = crypto_hash(producer_pubkey.as_bytes());
                            info!(
                                "Bootstrap producer discovered via status: {} (now {} known)",
                                &pubkey_hash.to_hex()[..16],
                                known.len()
                            );
                            drop(known);
                            // Reset stability timer - new producer discovered
                            *self.last_producer_list_change.write().await = Some(Instant::now());
                        }
                    }
                }
            }

            NetworkEvent::StatusRequest {
                peer_id,
                channel,
                request,
            } => {
                debug!("Status request from {}", peer_id);

                // BOOTSTRAP PRODUCER DISCOVERY: If the requesting peer is a producer,
                // add them to known_producers (same as we do for PeerStatus)
                if let Some(ref producer_pubkey) = request.producer_pubkey {
                    let genesis_active = {
                        let state = self.chain_state.read().await;
                        self.config.network.is_in_genesis(state.best_height + 1)
                    };
                    if self.config.network == Network::Testnet
                        || self.config.network == Network::Devnet
                        || genesis_active
                    {
                        let mut known = self.known_producers.write().await;
                        if !known.contains(producer_pubkey) {
                            known.push(*producer_pubkey);
                            known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                            let pubkey_hash = crypto_hash(producer_pubkey.as_bytes());
                            info!("Bootstrap producer discovered via status request: {} (now {} known)",
                                  &pubkey_hash.to_hex()[..16],
                                  known.len());
                            drop(known);
                            // Reset stability timer - new producer discovered
                            *self.last_producer_list_change.write().await = Some(Instant::now());
                        }
                    }
                }

                let state = self.chain_state.read().await;
                let response = if let Some(ref key) = self.producer_key {
                    network::protocols::StatusResponse {
                        version: 1,
                        network_id: self.config.network.id(),
                        genesis_hash: state.genesis_hash,
                        best_height: state.best_height,
                        best_hash: state.best_hash,
                        best_slot: state.best_slot,
                        producer_pubkey: Some(*key.public_key()),
                    }
                } else {
                    network::protocols::StatusResponse {
                        version: 1,
                        network_id: self.config.network.id(),
                        genesis_hash: state.genesis_hash,
                        best_height: state.best_height,
                        best_hash: state.best_hash,
                        best_slot: state.best_slot,
                        producer_pubkey: None,
                    }
                };

                if let Some(ref network) = self.network {
                    let _ = network.send_status_response(channel, response).await;
                }
            }

            NetworkEvent::SyncRequest {
                peer_id,
                request,
                channel,
            } => {
                // GLOBAL SYNC SERVING RATE LIMIT: Cap aggregate sync responses per
                // production interval. Without this, 100+ syncing peers each sending
                // 20 req/sec (per-peer limit) create 2000+ req/sec aggregate, saturating
                // the event loop with block_store I/O and starving production.
                const MAX_SYNC_REQUESTS_PER_INTERVAL: u32 = 8;
                if self.sync_requests_this_interval >= MAX_SYNC_REQUESTS_PER_INTERVAL {
                    debug!(
                        "Sync request from {} deferred — serving limit reached ({}/{})",
                        peer_id, self.sync_requests_this_interval, MAX_SYNC_REQUESTS_PER_INTERVAL
                    );
                } else {
                    debug!("Sync request from {}: {:?}", peer_id, request);
                    self.sync_requests_this_interval += 1;
                    self.handle_sync_request(request, channel).await?;
                }
            }

            NetworkEvent::SyncResponse { peer_id, response } => {
                debug!("Sync response from {}", peer_id);

                // P1 #5: Note that this peer is sending data (active), not just reachable
                self.sync_manager
                    .write()
                    .await
                    .note_block_received_from_peer(peer_id);

                let blocks = self
                    .sync_manager
                    .write()
                    .await
                    .handle_response(peer_id, response);
                for block in blocks {
                    // Route through handle_new_block for orphan/fork detection.
                    // For normal sync blocks that build on tip, this falls through
                    // to apply_block unchanged. For orphan blocks (e.g., peer's tip
                    // when we're on a fork), they get cached and trigger fork recovery.
                    self.handle_new_block(block, peer_id).await?;
                }

                // Check if snap sync produced a ready snapshot
                let snap = self.sync_manager.write().await.take_snap_snapshot();
                if let Some(snapshot) = snap {
                    self.apply_snap_snapshot(snapshot).await?;
                }
            }

            NetworkEvent::NetworkMismatch {
                peer_id,
                our_network_id,
                their_network_id,
            } => {
                warn!(
                    "Disconnected peer {} due to network mismatch: we are on network {}, they are on {}",
                    peer_id, our_network_id, their_network_id
                );
                self.sync_manager.write().await.remove_peer(&peer_id);
            }

            NetworkEvent::GenesisMismatch { peer_id } => {
                warn!(
                    "Disconnected peer {} due to genesis hash mismatch (different chain fork)",
                    peer_id
                );
                self.sync_manager.write().await.remove_peer(&peer_id);
            }

            // ── GOSSIP PIPELINE (spawned to dedicated tasks) ────────────
            // These events involve CPU-intensive signature verification and
            // CRDT merges. Processing them inline blocks the event loop at
            // 100+ peers. Spawning to their own tasks lets block processing
            // and production continue uninterrupted.
            //
            // All shared state accessed via Arc<RwLock<>> clones.
            NetworkEvent::ProducersAnnounced(remote_list) => {
                let chain_state = self.chain_state.clone();
                let network_type = self.config.network;
                let known_producers = self.known_producers.clone();
                let last_change = self.last_producer_list_change.clone();

                tokio::spawn(async move {
                    let genesis_active = {
                        let state = chain_state.read().await;
                        network_type.is_in_genesis(state.best_height + 1)
                    };
                    if network_type == Network::Testnet
                        || network_type == Network::Devnet
                        || genesis_active
                    {
                        let changed = {
                            let mut known = known_producers.write().await;
                            let mut changed = false;

                            for producer in &remote_list {
                                if !known.contains(producer) {
                                    known.push(*producer);
                                    changed = true;
                                    let pubkey_hash = crypto_hash(producer.as_bytes());
                                    info!(
                                        "Bootstrap producer discovered via ANTI-ENTROPY: {} (now {} known)",
                                        &pubkey_hash.to_hex()[..16],
                                        known.len()
                                    );
                                }
                            }

                            if changed {
                                known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                                info!(
                                    "Producer set updated via anti-entropy: {} total known producers",
                                    known.len()
                                );
                            }
                            changed
                        };

                        if changed {
                            *last_change.write().await = Some(Instant::now());
                        }
                    }
                });
            }

            NetworkEvent::ProducerAnnouncementsReceived(announcements) => {
                // GSet CRDT merge: CPU-intensive (ed25519 sig verify per announcement).
                // Spawned to prevent blocking block processing at 5000+ nodes.
                let gset = self.producer_gset.clone();
                let adaptive = self.adaptive_gossip.clone();
                let sync_mgr = self.sync_manager.clone();
                let known_producers = self.known_producers.clone();
                let last_change = self.last_producer_list_change.clone();

                tokio::spawn(async move {
                    let merge_result = {
                        let mut g = gset.write().await;
                        g.merge(announcements)
                    };

                    let peer_count = sync_mgr.read().await.peer_count();
                    {
                        let mut gossip = adaptive.write().await;
                        gossip.on_gossip_result(&merge_result, peer_count);
                    }

                    if merge_result.added > 0 {
                        info!(
                            "Producer announcements: added={}, new_producers={}, rejected={}, duplicates={}",
                            merge_result.added, merge_result.new_producers, merge_result.rejected, merge_result.duplicates
                        );
                        if merge_result.new_producers > 0 {
                            *last_change.write().await = Some(Instant::now());
                        }

                        let g = gset.read().await;
                        let producers = g.sorted_producers();
                        let mut known = known_producers.write().await;
                        for pubkey in producers {
                            if !known.contains(&pubkey) {
                                known.push(pubkey);
                            }
                        }
                        known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                    } else if merge_result.rejected > 0 {
                        debug!(
                            "Producer announcements rejected: {} (invalid signature, stale, or wrong network)",
                            merge_result.rejected
                        );
                    }
                });
            }

            NetworkEvent::ProducerDigestReceived { peer_id, digest } => {
                // Bloom filter delta: spawned to avoid blocking event loop
                let gset = self.producer_gset.clone();
                let cmd_tx = self.network.as_ref().map(|n| n.command_sender());

                tokio::spawn(async move {
                    debug!(
                        "Received producer digest from {} ({} elements)",
                        peer_id,
                        digest.element_count()
                    );

                    let delta = {
                        let g = gset.read().await;
                        g.delta_for_peer(&digest)
                    };

                    if !delta.is_empty() {
                        debug!("Sending {} producers as delta to {}", delta.len(), peer_id);
                        if let Some(tx) = cmd_tx {
                            let _ = tx
                                .send(NetworkCommand::SendProducerDelta {
                                    peer_id,
                                    announcements: delta,
                                })
                                .await;
                        }
                    }
                });
            }

            NetworkEvent::NewVote(vote_data) => {
                debug!("Received vote message ({} bytes)", vote_data.len());
                if let Some(ref vote_tx) = self.vote_tx {
                    match serde_json::from_slice::<node_updater::VoteMessage>(&vote_data) {
                        Ok(vote_msg) => {
                            info!(
                                "Vote received via gossip: {} vote for v{} from {}",
                                if vote_msg.vote == node_updater::Vote::Veto {
                                    "VETO"
                                } else {
                                    "APPROVE"
                                },
                                vote_msg.version,
                                &vote_msg.producer_id[..16.min(vote_msg.producer_id.len())]
                            );
                            let _ = vote_tx.try_send(vote_msg);
                        }
                        Err(e) => {
                            debug!("Failed to decode vote message: {}", e);
                        }
                    }
                }
            }

            // NOTE: Heartbeats removed in deterministic scheduler model
            // Rewards go 100% to block producer via coinbase
            NetworkEvent::NewHeartbeat(_) => {
                // Ignored - deterministic scheduler model doesn't use heartbeats
            }
            NetworkEvent::NewAttestation(data) => {
                // Decode and apply attestation for finality + liveness tracking
                if let Some(attestation) = doli_core::Attestation::from_bytes(&data) {
                    if attestation.verify().is_ok() {
                        // Finality gadget: accumulate weight per block
                        let mut sync = self.sync_manager.write().await;
                        sync.add_attestation_weight(
                            &attestation.block_hash,
                            attestation.attester_weight,
                        );
                        drop(sync);

                        // Minute tracker: record for on-chain bitfield + BLS aggregation
                        let minute = attestation_minute(attestation.slot);
                        if attestation.bls_signature.is_empty() {
                            self.minute_tracker.record(attestation.attester, minute);
                        } else {
                            self.minute_tracker.record_with_bls(
                                attestation.attester,
                                minute,
                                attestation.bls_signature.clone(),
                            );
                        }

                        // Flush any blocks that just reached finality
                        self.flush_finalized_to_archive().await;
                    } else {
                        debug!("Received invalid attestation signature");
                    }
                }
            }

            // ── TX ANNOUNCE-REQUEST PROTOCOL ────────────────────────────
            // EIP-4938 style: peers announce tx hashes, we fetch missing ones.
            NetworkEvent::TxAnnouncement { peer_id, hashes } => {
                let mempool = self.mempool.read().await;
                let mut new_count = 0;
                for hash in &hashes {
                    if !mempool.contains(hash)
                        && self.pending_tx_announcements.record(*hash, peer_id)
                    {
                        new_count += 1;
                    }
                }
                drop(mempool);

                if new_count > 0 {
                    // Fetch missing txs from announcing peers
                    let batches = self.pending_tx_announcements.take_batch();
                    if let Some(ref network) = self.network {
                        for (peer, fetch_hashes) in batches {
                            debug!(
                                "Requesting {} txs from peer {} (announce-request)",
                                fetch_hashes.len(),
                                peer
                            );
                            let _ = network.request_tx_fetch(peer, fetch_hashes).await;
                        }
                    }
                }
            }

            NetworkEvent::TxFetchRequest {
                peer_id,
                hashes,
                channel,
            } => {
                debug!(
                    "TxFetch request from {} for {} hashes",
                    peer_id,
                    hashes.len()
                );
                let mempool = self.mempool.read().await;
                let mut txs = Vec::new();
                for hash in &hashes {
                    if let Some(entry) = mempool.get(hash) {
                        txs.push(entry.tx.clone());
                    }
                }
                drop(mempool);

                if let Some(ref network) = self.network {
                    let response = network::protocols::TxFetchResponse { transactions: txs };
                    let _ = network.send_tx_fetch_response(channel, response).await;
                }
            }

            NetworkEvent::TxFetchResponse {
                peer_id,
                transactions,
            } => {
                debug!(
                    "TxFetch response from {} with {} txs",
                    peer_id,
                    transactions.len()
                );
                for tx in transactions {
                    let tx_hash = tx.hash();
                    self.pending_tx_announcements.complete(&tx_hash);
                    self.handle_new_transaction(tx).await?;
                }
            }
        }

        Ok(())
    }
}
