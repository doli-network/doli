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
                        // INC-I-012: Drain pending network events before producing.
                        // During connection storms (50+ nodes joining), block events
                        // queue behind hundreds of Identify/Kademlia/GSet events in
                        // the swarm loop. Without draining, the escape hatch produces
                        // on a stale chain tip → competing blocks → fork cascade.
                        // Cap scales with max_peers to handle larger networks:
                        // at max_peers=50, drain_cap=150 (~150ms worst case).
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
                    // INC-I-012: Drain pending events before producing (same as escape hatch).
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

    /// Handle network events — dispatcher to `on_*` methods in `network_events.rs`.
    pub(super) async fn handle_network_event(&mut self, event: NetworkEvent) -> Result<()> {
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

            // NOTE: Heartbeats removed in deterministic scheduler model
            // Rewards go 100% to block producer via coinbase
            NetworkEvent::NewHeartbeat(_) => {
                // Ignored - deterministic scheduler model doesn't use heartbeats
            }

            NetworkEvent::NewAttestation(data) => {
                self.on_new_attestation(data).await;
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
        }

        Ok(())
    }
}
