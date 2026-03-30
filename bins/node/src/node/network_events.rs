use super::*;

use network::protocols::{StatusRequest, StatusResponse, TxFetchResponse};
use network::ResponseChannel;

impl Node {
    /// Handle a newly connected peer: enable bootstrap gate and request status.
    pub async fn on_peer_connected(&mut self, peer_id: PeerId) {
        info!("Peer connected: {}", peer_id);

        if self.first_peer_connected.is_none() {
            self.first_peer_connected = Some(Instant::now());
            info!("First peer connected - starting discovery grace period");
        }

        self.sync_manager.write().await.set_peer_connected();

        let genesis_hash = self.chain_state.read().await.genesis_hash;
        let status_request = if let Some(ref key) = self.producer_key {
            StatusRequest::with_producer(self.config.network.id(), genesis_hash, *key.public_key())
        } else {
            StatusRequest::new(self.config.network.id(), genesis_hash)
        };

        if let Some(ref network) = self.network {
            if let Err(e) = network.request_status(peer_id, status_request).await {
                warn!("Failed to request status from peer {}: {}", peer_id, e);
            } else {
                debug!("Requested status from peer {}", peer_id);
            }
        }
    }

    /// Handle a disconnected peer: remove from sync manager and attempt bootstrap reconnect.
    pub async fn on_peer_disconnected(&mut self, peer_id: PeerId) {
        info!("Peer disconnected: {}", peer_id);
        self.sync_manager.write().await.remove_peer(&peer_id);

        // Rate-limited reconnect: only do an immediate dial if we haven't tried
        // recently — prevents spin loop that floods the event queue.
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
    /// Handle a new gossip block: slot sanity, update network tip, delegate to handle_new_block.
    pub async fn on_new_block_event(&mut self, block: Block, source_peer: PeerId) -> Result<()> {
        // Skip gossip blocks during snap sync — they cause spurious fork recovery.
        if self.sync_manager.read().await.is_snap_syncing() {
            debug!("Ignoring gossip block {} during snap sync", block.hash());
            return Ok(());
        }

        debug!("Received new block: {} from {}", block.hash(), source_peer);

        // INC-I-014: Skip blocks extending from rejected fork tips.
        // When finality rejects a reorg, the fork tip hash is cached. Future blocks
        // with prev_hash in this set are dead ends — processing them wastes CPU and RAM.
        // At 166 nodes, 3 competing fork blocks caused 92GB RAM via gossip amplification.
        {
            if self.rejected_fork_tips.contains(&block.header.prev_hash) {
                debug!(
                    "Skipping block {} — parent {} is a rejected fork tip",
                    block.hash(),
                    block.header.prev_hash
                );
                // Also mark THIS block as rejected so its children are skipped too
                self.rejected_fork_tips.insert(block.hash());
                // Cap the rejection cache at 1000 entries
                if self.rejected_fork_tips.len() > 1000 {
                    self.rejected_fork_tips.clear();
                }
                return Ok(());
            }
        }

        // Slot sanity — reject gossip blocks with wildly wrong slots.
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

        // Update network tip slot from gossip — critical for "behind peers" production safety.
        {
            let mut sync = self.sync_manager.write().await;
            sync.note_block_received_from_peer(source_peer);
            sync.update_network_tip_slot(block.header.slot);
            sync.note_block_received_via_gossip();
        }
        self.handle_new_block(block, source_peer).await?;
        Ok(())
    }

    /// Handle a peer's status response: update sync manager, discover bootstrap producers.
    ///
    /// Uses `update_peer()` for already-known peers to preserve `last_block_received`
    /// tracking. Uses `add_peer()` only for the initial handshake.
    pub async fn on_peer_status(&mut self, peer_id: PeerId, status: StatusResponse) {
        debug!(
            "Peer {} status: height={}, slot={}",
            peer_id, status.best_height, status.best_slot
        );
        {
            let mut sync = self.sync_manager.write().await;
            if sync.has_peer(&peer_id) {
                sync.update_peer(
                    peer_id,
                    status.best_height,
                    status.best_hash,
                    status.best_slot,
                );
            } else {
                sync.add_peer(
                    peer_id,
                    status.best_height,
                    status.best_hash,
                    status.best_slot,
                );
            }
            sync.note_peer_status_received();
        }

        if let Some(ref producer_pubkey) = status.producer_pubkey {
            self.maybe_add_bootstrap_producer(producer_pubkey, "status")
                .await;
        }
    }

    /// Handle an inbound status request: discover requester's identity, respond with our state.
    pub async fn on_status_request(
        &mut self,
        peer_id: PeerId,
        channel: ResponseChannel<StatusResponse>,
        request: StatusRequest,
    ) {
        debug!("Status request from {}", peer_id);

        if let Some(ref producer_pubkey) = request.producer_pubkey {
            self.maybe_add_bootstrap_producer(producer_pubkey, "status request")
                .await;
        }

        let state = self.chain_state.read().await;
        let response = StatusResponse {
            version: 1,
            network_id: self.config.network.id(),
            genesis_hash: state.genesis_hash,
            best_height: state.best_height,
            best_hash: state.best_hash,
            best_slot: state.best_slot,
            producer_pubkey: self.producer_key.as_ref().map(|k| *k.public_key()),
        };

        if let Some(ref network) = self.network {
            let _ = network.send_status_response(channel, response).await;
        }
    }

    /// Add a producer discovered via status exchange to known_producers (testnet/devnet/genesis).
    async fn maybe_add_bootstrap_producer(&mut self, producer_pubkey: &PublicKey, source: &str) {
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
                info!(
                    "Bootstrap producer discovered via {}: {} (now {} known)",
                    source,
                    &pubkey_hash.to_hex()[..16],
                    known.len()
                );
                drop(known);
                self.last_producer_list_change = Some(Instant::now());
            }
        }
    }

    /// Handle an inbound sync request with global rate limiting (INC-I-012 F6: 24/interval).
    pub async fn on_sync_request(
        &mut self,
        peer_id: PeerId,
        request: SyncRequest,
        channel: ResponseChannel<SyncResponse>,
    ) -> Result<()> {
        const MAX_SYNC_REQUESTS_PER_INTERVAL: u32 = 24;
        if self.sync_requests_this_interval >= MAX_SYNC_REQUESTS_PER_INTERVAL {
            debug!(
                "Sync request from {} deferred — serving limit reached ({}/{})",
                peer_id, self.sync_requests_this_interval, MAX_SYNC_REQUESTS_PER_INTERVAL
            );
            if let Some(ref network) = self.network {
                let _ = network
                    .send_sync_response(
                        channel,
                        network::protocols::SyncResponse::Error(
                            "busy: sync serving limit reached".to_string(),
                        ),
                    )
                    .await;
            }
        } else {
            debug!("Sync request from {}: {:?}", peer_id, request);
            self.sync_requests_this_interval += 1;
            self.handle_sync_request(request, channel).await?;
        }
        Ok(())
    }

    /// Handle a sync response: process returned blocks and check for snap snapshots.
    pub async fn on_sync_response(
        &mut self,
        peer_id: PeerId,
        response: SyncResponse,
    ) -> Result<()> {
        debug!("Sync response from {}", peer_id);

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
            self.handle_new_block(block, peer_id).await?;
        }

        let snap = self.sync_manager.write().await.take_snap_snapshot();
        if let Some(snapshot) = snap {
            self.apply_snap_snapshot(snapshot).await?;
        }
        Ok(())
    }

    /// Handle legacy producer list broadcast via anti-entropy.
    pub fn on_producers_announced(&self, remote_list: Vec<PublicKey>) {
        let chain_state = self.chain_state.clone();
        let network_type = self.config.network;
        let known_producers = self.known_producers.clone();

        tokio::spawn(async move {
            let genesis_active = {
                let state = chain_state.read().await;
                network_type.is_in_genesis(state.best_height + 1)
            };
            if network_type == Network::Testnet || network_type == Network::Devnet || genesis_active
            {
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
            }
        });
    }

    /// Handle GSet CRDT announcement merge (CPU-intensive, spawned).
    pub fn on_producer_announcements(&self, announcements: Vec<ProducerAnnouncement>) {
        let gset = self.producer_gset.clone();
        let adaptive = self.adaptive_gossip.clone();
        let sync_mgr = self.sync_manager.clone();
        let known_producers = self.known_producers.clone();

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
                    merge_result.added, merge_result.new_producers,
                    merge_result.rejected, merge_result.duplicates
                );

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

    /// Handle bloom filter delta sync request (spawned).
    pub fn on_producer_digest(&self, peer_id: PeerId, digest: doli_core::ProducerBloomFilter) {
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

    /// Handle a vote message received via gossip (for the auto-update system).
    pub fn on_new_vote(&self, vote_data: Vec<u8>) {
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

    /// Handle a new attestation: verify, accumulate finality weight, flush to archive.
    pub async fn on_new_attestation(&mut self, data: Vec<u8>) {
        if let Some(attestation) = doli_core::Attestation::from_bytes(&data) {
            if attestation.verify().is_ok() {
                let mut sync = self.sync_manager.write().await;
                sync.add_attestation_weight(&attestation.block_hash, attestation.attester_weight);
                drop(sync);

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

                self.flush_finalized_to_archive().await;
            } else {
                debug!("Received invalid attestation signature");
            }
        }
    }

    /// Handle tx hash announcements: record unknown hashes, fetch missing txs.
    pub async fn on_tx_announcement(&mut self, peer_id: PeerId, hashes: Vec<Hash>) {
        let mempool = self.mempool.read().await;
        let mut new_hashes = Vec::new();
        for hash in &hashes {
            if !mempool.contains(hash) {
                let entry = self.pending_tx_announcements.entry(peer_id).or_default();
                if !entry.contains(hash) {
                    entry.push(*hash);
                    new_hashes.push(*hash);
                }
            }
        }
        drop(mempool);

        if !new_hashes.is_empty() {
            // Take all pending batches and fetch them
            let batches: Vec<(PeerId, Vec<Hash>)> = self.pending_tx_announcements.drain().collect();
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

    /// Handle an inbound tx fetch request: respond with matching mempool txs.
    pub async fn on_tx_fetch_request(
        &self,
        peer_id: PeerId,
        hashes: Vec<Hash>,
        channel: ResponseChannel<TxFetchResponse>,
    ) {
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

    /// Handle a tx fetch response: complete pending announcements and process txs.
    pub async fn on_tx_fetch_response(
        &mut self,
        peer_id: PeerId,
        transactions: Vec<Transaction>,
    ) -> Result<()> {
        debug!(
            "TxFetch response from {} with {} txs",
            peer_id,
            transactions.len()
        );
        for tx in transactions {
            let tx_hash = tx.hash();
            // Remove completed tx hash from all pending peer entries
            self.pending_tx_announcements.retain(|_peer, hashes| {
                hashes.retain(|h| h != &tx_hash);
                !hashes.is_empty()
            });
            self.handle_new_transaction(tx).await?;
        }
        Ok(())
    }
}
