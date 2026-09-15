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

                    // PRODUCTION ESCAPE HATCH: If production hasn't run for a full
                    // interval, force it now. This guarantees production runs at least
                    // once per interval even under infinite event load.
                    if last_production_check.elapsed() >= production_interval {
                        // INC-I-012: Drain pending network events before producing.
                        {
                            let drain_cap = self.config.max_peers * 3;
                            let mut pending = Vec::new();
                            if let Some(ref mut network) = self.network {
                                while pending.len() < drain_cap {
                                    match network.try_next_event() {
                                        Some(ev) => pending.push(ev),
                                        None => break,
                                    }
                                }
                            }
                            for ev in pending {
                                if let Err(e) = self.handle_network_event(ev).await {
                                    warn!("Error handling drained event: {}", e);
                                }
                            }
                        }
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
                    // INC-I-012: Drain pending events before producing.
                    {
                        let drain_cap = self.config.max_peers * 3;
                        let mut pending = Vec::new();
                        if let Some(ref mut network) = self.network {
                            while pending.len() < drain_cap {
                                match network.try_next_event() {
                                    Some(ev) => pending.push(ev),
                                    None => break,
                                }
                            }
                        }
                        for ev in pending {
                            if let Err(e) = self.handle_network_event(ev).await {
                                warn!("Error handling drained event: {}", e);
                            }
                        }
                    }

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

                            // SCALE-T2-002: Always use delta gossip when producers are known.
                            let producer_count = {
                                let gset = self.producer_gset.read().await;
                                gset.len()
                            };

                            if producer_count > 0 {
                                // DELTA SYNC: Send bloom filter, peers respond with missing
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
                                // FULL SYNC FALLBACK: Only when GSet is empty (fresh startup)
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

    /// Handle network events — delegates to network_events.rs handlers.
    pub async fn handle_network_event(&mut self, event: NetworkEvent) -> Result<()> {
        match event {
            NetworkEvent::PeerConnected(peer_id) => {
                self.on_peer_connected(peer_id).await;
            }
            NetworkEvent::PeerDisconnected(peer_id) => {
                self.on_peer_disconnected(peer_id).await;
            }
            NetworkEvent::NewBlock(block, source_peer) => {
                self.on_new_block_event(block, source_peer).await?;
            }
            NetworkEvent::NewHeader(header) => {
                debug!("Received new header: {}", header.hash());
            }
            NetworkEvent::NewTransaction(tx) => {
                self.handle_new_transaction(tx).await?;
            }
            NetworkEvent::PeerStatus { peer_id, status } => {
                self.on_peer_status(peer_id, status).await;
            }
            NetworkEvent::StatusRequest {
                peer_id,
                channel,
                request,
            } => {
                self.on_status_request(peer_id, channel, request).await;
            }
            NetworkEvent::SyncRequest {
                peer_id,
                request,
                channel,
            } => {
                self.on_sync_request(peer_id, request, channel).await?;
            }
            NetworkEvent::SyncResponse { peer_id, response } => {
                self.on_sync_response(peer_id, response).await?;
            }
            NetworkEvent::NetworkMismatch {
                peer_id,
                our_network_id,
                their_network_id,
            } => {
                warn!(
                    "Network mismatch with peer {}: ours={}, theirs={}",
                    peer_id, our_network_id, their_network_id
                );
                if let Some(ref network) = self.network {
                    let _ = network.disconnect(peer_id).await;
                }
            }
            NetworkEvent::GenesisMismatch { peer_id } => {
                warn!(
                    "Genesis hash mismatch with peer {} — disconnecting (different chain)",
                    peer_id
                );
                if let Some(ref network) = self.network {
                    let _ = network.disconnect(peer_id).await;
                }
            }
            NetworkEvent::VersionMismatch {
                peer_id,
                our_version,
                their_version,
            } => {
                warn!(
                    "Protocol version mismatch with peer {}: ours={}, theirs={} (min required={}) — disconnecting",
                    peer_id, our_version, their_version,
                    network::protocols::status::MIN_PEER_PROTOCOL_VERSION,
                );
                if let Some(ref network) = self.network {
                    let _ = network.disconnect(peer_id).await;
                }
            }
            NetworkEvent::ProducersAnnounced(remote_list) => {
                self.on_producers_announced(remote_list);
            }
            NetworkEvent::ProducerAnnouncementsReceived(announcements) => {
                self.on_producer_announcements(announcements);
            }
            NetworkEvent::ProducerDigestReceived { peer_id, digest } => {
                self.on_producer_digest(peer_id, digest);
            }
            NetworkEvent::NewVote(vote_data) => {
                self.on_new_vote(vote_data);
            }
            NetworkEvent::NewHeartbeat(_) => {
                // Heartbeats handled by the presence system
            }
            NetworkEvent::TxAnnouncement { peer_id, hashes } => {
                self.on_tx_announcement(peer_id, hashes).await;
            }
            NetworkEvent::TxFetchRequest {
                peer_id,
                hashes,
                channel,
            } => {
                self.on_tx_fetch_request(peer_id, hashes, channel).await;
            }
            NetworkEvent::TxFetchResponse {
                peer_id,
                transactions,
            } => {
                self.on_tx_fetch_response(peer_id, transactions).await?;
            }
            NetworkEvent::NewAttestation(data, source_peer) => {
                self.on_new_attestation(data, source_peer).await;
            }
        }
        Ok(())
    }
}
