use super::*;

impl Node {
    /// Main event loop
    pub async fn run_event_loop(&mut self) -> Result<()> {
        info!("Entering main event loop");

        // Check production opportunity - faster for devnet to catch the 700ms heartbeat window
        let production_interval = if self.config.network == Network::Devnet {
            Duration::from_millis(200) // 5 checks per second for devnet
        } else {
            Duration::from_secs(1)
        };
        let mut production_timer = tokio::time::interval(production_interval);

        // Gossip our producer identity using adaptive intervals
        // This ensures nodes that aren't directly connected (e.g., Node 2 -> Node 1 -> Node 3)
        // learn about each other through the GossipSub mesh relay
        // Phase 1: Use AdaptiveGossip for dynamic interval adjustment
        let mut current_gossip_interval = self.adaptive_gossip.read().await.interval();
        let mut gossip_timer = tokio::time::interval(current_gossip_interval);

        // Reset GSet broadcast tracker at event loop start.
        self.last_broadcast_gset_len = 0;

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
                }

                // Production timer tick
                _ = production_timer.tick() => {
                    // Check for block production opportunity
                    if self.producer_key.is_some() {
                        if let Err(e) = self.try_produce_block().await {
                            warn!("Block production error: {}", e);
                            // Purge toxic TXs from mempool to prevent infinite retry
                            // loops that freeze the chain. Covers: duplicate NFT token_id,
                            // duplicate pool_id, duplicate registration, or any TX that
                            // causes "already exists" errors in apply_block.
                            // See: testnet incident 2026-03-25.
                            let err_msg = e.to_string();
                            if err_msg.contains("already exists") || err_msg.contains("already registered") {
                                let mut mempool = self.mempool.write().await;
                                let before = mempool.len();
                                mempool.remove_by_error_pattern(&err_msg);
                                let after = mempool.len();
                                if before != after {
                                    warn!(
                                        "Purged {} toxic TXs from mempool after production error: {}",
                                        before - after, err_msg
                                    );
                                }
                            }
                        }
                    }

                    // Run periodic tasks
                    if let Err(e) = self.run_periodic_tasks().await {
                        warn!("Periodic task error: {}", e);
                    }
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
                            // Ensure our announcement is in the GSet (idempotent).
                            // Sequence is stable — producers prove liveness by producing
                            // blocks, not by bumping sequence numbers. The GSet's only
                            // job is discovery.
                            if let Some(ref key) = self.producer_key {
                                let seq = self.announcement_sequence.load(Ordering::SeqCst);
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

                            // Only broadcast when GSet has new entries to share.
                            // GSet is a grow-only CRDT: once all producers are discovered,
                            // the set is converged and broadcasts stop. This prevents
                            // flooding the network with duplicate state.
                            let current_gset_len = {
                                let gset = self.producer_gset.read().await;
                                gset.len()
                            };

                            if current_gset_len > self.last_broadcast_gset_len {
                                self.last_broadcast_gset_len = current_gset_len;

                                // Get adaptive gossip settings
                                let use_delta = {
                                    let adaptive = self.adaptive_gossip.read().await;
                                    adaptive.use_delta_sync()
                                };

                                // Choose sync strategy based on network size
                                if use_delta && current_gset_len > 50 {
                                    let bloom = {
                                        let gset = self.producer_gset.read().await;
                                        gset.to_bloom_filter()
                                    };
                                    debug!(
                                        "Delta sync: broadcasting bloom filter ({} bytes) for {} producers",
                                        bloom.size_bytes(),
                                        current_gset_len
                                    );
                                    let _ = network.broadcast_producer_digest(bloom).await;
                                } else {
                                    let announcements = {
                                        let gset = self.producer_gset.read().await;
                                        gset.export()
                                    };

                                    if !announcements.is_empty() {
                                        debug!(
                                            "Full sync: broadcasting {} producer announcements",
                                            announcements.len()
                                        );
                                        let _ = network.broadcast_producer_announcements(announcements).await;
                                    }
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
    pub async fn handle_network_event(&mut self, event: NetworkEvent) -> Result<()> {
        match event {
            NetworkEvent::PeerConnected(peer_id) => {
                info!("Peer connected: {}", peer_id);

                // Track when we first connected to a peer (for bootstrap discovery grace period)
                if self.first_peer_connected.is_none() {
                    self.first_peer_connected = Some(Instant::now());
                    info!("First peer connected - starting discovery grace period");
                }

                // Re-broadcast GSet to new peer on next gossip tick.
                // Without this, late-connecting peers never receive the GSet
                // because broadcasts stop once the set converges.
                self.last_broadcast_gset_len = 0;

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
                // Track that we've seen a block for this slot via gossip.
                // Rank 1 checks this before producing — if rank 0's block arrived
                // via gossip but hasn't been applied to block_store yet, rank 1
                // must NOT produce a competing block.
                self.seen_blocks_for_slot.insert(block.header.slot);

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
                            self.last_producer_list_change = Some(Instant::now());
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
                            self.last_producer_list_change = Some(Instant::now());
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
                debug!("Sync request from {}: {:?}", peer_id, request);
                // Spawn sync request handling in background to avoid blocking
                // the event loop. Without this, sync requests from 40+ peers
                // starve the production timer via the biased select!, causing
                // producers to miss their rank 0 window and create forks.
                let block_store = self.block_store.clone();
                let chain_state = self.chain_state.clone();
                let utxo_set = self.utxo_set.clone();
                let producer_set = self.producer_set.clone();
                let network_cmd_tx = self.network.as_ref().map(|n| n.command_sender());
                tokio::spawn(async move {
                    if let Err(e) = handle_sync_request_bg(
                        block_store,
                        chain_state,
                        utxo_set,
                        producer_set,
                        network_cmd_tx,
                        request,
                        channel,
                    )
                    .await
                    {
                        warn!("Background sync request error: {}", e);
                    }
                });
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
                    // Track block seen for gossip-aware fallback production
                    self.seen_blocks_for_slot.insert(block.header.slot);
                    // Route through handle_new_block for orphan/fork detection.
                    // For normal sync blocks that build on tip, this falls through
                    // to apply_block unchanged. For orphan blocks (e.g., peer's tip
                    // when we're on a fork), they get cached and trigger fork recovery.
                    self.handle_new_block(block, peer_id).await?;
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

            NetworkEvent::ProducersAnnounced(remote_list) => {
                // LEGACY ANTI-ENTROPY GOSSIP: Merge remote producer list with our local list
                // This is STATE-BASED (not event-based) - we receive the sender's full view
                // and merge using CRDT union semantics: Union(Local, Remote)
                // This guarantees convergence even with packet loss or network partitions.
                let genesis_active = {
                    let state = self.chain_state.read().await;
                    self.config.network.is_in_genesis(state.best_height + 1)
                };
                if self.config.network == Network::Testnet
                    || self.config.network == Network::Devnet
                    || genesis_active
                {
                    let changed = {
                        let mut known = self.known_producers.write().await;
                        let mut changed = false;

                        // CRDT MERGE: Add any producers we don't already know about
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

                        // Keep sorted for deterministic round-robin ordering
                        if changed {
                            known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                            info!(
                                "Producer set updated via anti-entropy: {} total known producers",
                                known.len()
                            );
                        }
                        changed
                    };

                    // Mark stability timer reset (outside the lock)
                    if changed {
                        self.last_producer_list_change = Some(Instant::now());
                    }
                }
            }

            NetworkEvent::ProducerAnnouncementsReceived(announcements) => {
                // NEW PRODUCER DISCOVERY: Merge signed announcements into the GSet CRDT
                // Each announcement is cryptographically verified before merging
                let merge_result = {
                    let mut gset = self.producer_gset.write().await;
                    gset.merge(announcements)
                };

                // Update adaptive gossip controller with merge result
                let peer_count = self.sync_manager.read().await.peer_count();
                {
                    let mut gossip = self.adaptive_gossip.write().await;
                    gossip.on_gossip_result(&merge_result, peer_count);
                }

                // Log significant changes
                if merge_result.added > 0 {
                    info!(
                        "Producer announcements: added={}, new_producers={}, rejected={}, duplicates={}",
                        merge_result.added, merge_result.new_producers, merge_result.rejected, merge_result.duplicates
                    );
                    // Only reset stability timer when truly NEW producers are discovered
                    // Sequence updates (liveness proofs) should not reset stability
                    if merge_result.new_producers > 0 {
                        self.last_producer_list_change = Some(Instant::now());
                    }

                    // Also sync to legacy known_producers for compatibility
                    let gset = self.producer_gset.read().await;
                    let producers = gset.sorted_producers();
                    let mut known = self.known_producers.write().await;
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
            }

            NetworkEvent::ProducerDigestReceived { peer_id, digest } => {
                // Peer sent us their bloom filter - compute delta and send missing announcements
                debug!(
                    "Received producer digest from {} ({} elements)",
                    peer_id,
                    digest.element_count()
                );

                // Get delta announcements (producers we know that peer doesn't)
                let delta = {
                    let gset = self.producer_gset.read().await;
                    gset.delta_for_peer(&digest)
                };

                if !delta.is_empty() {
                    debug!("Sending {} producers as delta to {}", delta.len(), peer_id);
                    if let Some(ref network) = self.network {
                        let _ = network.send_producer_delta(peer_id, delta).await;
                    }
                }
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
        }

        Ok(())
    }
}

/// Handle sync requests in a background task, outside the main event loop.
///
/// This prevents sync request I/O (reading headers/bodies from RocksDB) from
/// blocking block production. With 40+ peers syncing, the biased select! in the
/// event loop would process sync requests indefinitely, starving the production
/// timer and causing producers to miss their rank 0 window.
async fn handle_sync_request_bg(
    block_store: Arc<storage::BlockStore>,
    chain_state: Arc<tokio::sync::RwLock<storage::ChainState>>,
    utxo_set: Arc<tokio::sync::RwLock<storage::UtxoSet>>,
    producer_set: Arc<tokio::sync::RwLock<storage::ProducerSet>>,
    network_cmd_tx: Option<tokio::sync::mpsc::Sender<network::service::NetworkCommand>>,
    request: network::protocols::SyncRequest,
    channel: network::ResponseChannel<network::protocols::SyncResponse>,
) -> Result<()> {
    use network::protocols::{SyncRequest, SyncResponse};

    let response = match request {
        SyncRequest::GetHeaders {
            start_hash,
            max_count,
        } => {
            let mut headers = Vec::new();
            let state = chain_state.read().await;
            let genesis_hash = state.genesis_hash;
            let best_height = state.best_height;
            drop(state);

            let start_height = if start_hash == genesis_hash {
                0
            } else {
                match block_store.get_height_by_hash(&start_hash).ok().flatten() {
                    Some(h) => h,
                    None => {
                        if let Some(tx) = network_cmd_tx {
                            let _ = tx
                                .send(network::service::NetworkCommand::SendSyncResponse {
                                    channel,
                                    response: SyncResponse::Headers(vec![]),
                                })
                                .await;
                        }
                        return Ok(());
                    }
                }
            };

            let end_height = (start_height + max_count as u64).min(best_height);
            for height in (start_height + 1)..=end_height {
                if let Ok(Some(hash)) = block_store.get_hash_by_height(height) {
                    if let Ok(Some(header)) = block_store.get_header(&hash) {
                        headers.push(header);
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            SyncResponse::Headers(headers)
        }

        SyncRequest::GetBodies { hashes } => {
            let mut bodies = Vec::new();
            for hash in hashes {
                if let Ok(Some(block)) = block_store.get_block(&hash) {
                    bodies.push(block);
                }
            }
            SyncResponse::Bodies(bodies)
        }

        SyncRequest::GetBlockByHeight { height } => match block_store.get_block_by_height(height) {
            Ok(Some(block)) => SyncResponse::Block(Box::new(Some(block))),
            _ => SyncResponse::Block(Box::new(None)),
        },

        SyncRequest::GetBlockByHash { hash } => match block_store.get_block(&hash) {
            Ok(Some(block)) => SyncResponse::Block(Box::new(Some(block))),
            _ => SyncResponse::Block(Box::new(None)),
        },

        SyncRequest::GetStateAtCheckpoint { height } => {
            let best_height = chain_state.read().await.best_height;
            if best_height < height || best_height < 10 {
                if let Some(tx) = network_cmd_tx {
                    let _ = tx
                        .send(network::service::NetworkCommand::SendSyncResponse {
                            channel,
                            response: SyncResponse::Error(format!(
                                "Cannot serve checkpoint: local_h={}, requested_h={}",
                                best_height, height
                            )),
                        })
                        .await;
                }
                return Ok(());
            }

            let cs = chain_state.read().await;
            let utxo = utxo_set.read().await;
            let ps = producer_set.read().await;

            match storage::StateSnapshot::create(&cs, &utxo, &ps) {
                Ok(snap) => SyncResponse::StateAtCheckpoint {
                    block_hash: snap.block_hash,
                    block_height: snap.block_height,
                    chain_state: snap.chain_state_bytes,
                    utxo_set: snap.utxo_set_bytes,
                    producer_set: snap.producer_set_bytes,
                    state_root: snap.state_root,
                },
                Err(e) => SyncResponse::Error(format!("Checkpoint state error: {}", e)),
            }
        }

        SyncRequest::GetBlocksByHeightRange {
            start_height,
            count,
        } => {
            let mut blocks = Vec::new();
            let end_height = start_height.saturating_add(count as u64).saturating_sub(1);
            let best_height = chain_state.read().await.best_height;
            let actual_end = end_height.min(best_height);
            for h in start_height..=actual_end {
                if let Ok(Some(block)) = block_store.get_block_by_height(h) {
                    blocks.push(block);
                } else {
                    break;
                }
            }
            SyncResponse::Bodies(blocks)
        }
    };

    if let Some(tx) = network_cmd_tx {
        let _ = tx
            .send(network::service::NetworkCommand::SendSyncResponse { channel, response })
            .await;
    }

    Ok(())
}
