use super::*;

impl Node {
    /// Run the node
    pub async fn run(&mut self) -> Result<()> {
        info!("Node starting...");

        // Check for placeholder maintainer keys
        if self.config.network == Network::Mainnet && is_using_placeholder_keys(Network::Mainnet) {
            error!("CRITICAL: Placeholder maintainer keys detected!");
            error!("This node is NOT suitable for mainnet operation.");
            error!("Replace BOOTSTRAP_MAINTAINER_KEYS_MAINNET in doli-updater/src/lib.rs with real Ed25519 keys.");
            return Err(anyhow::anyhow!(
                "Cannot start mainnet node with placeholder maintainer keys"
            ));
        } else if is_using_placeholder_keys(self.config.network) {
            warn!("Using placeholder maintainer keys - this is OK for testnet/devnet");
        }

        // Start network service
        self.start_network().await?;

        // BOOTSTRAP PRODUCER SELF-REGISTRATION: Register ourselves as a producer immediately.
        // This is critical for round-robin slot assignment - if we don't know about ourselves,
        // we might produce blocks in slots assigned to other producers.
        if let Some(ref key) = self.producer_key {
            let genesis_active = {
                let state = self.chain_state.read().await;
                self.config.network.is_in_genesis(state.best_height + 1)
            };
            if self.config.network == Network::Testnet
                || self.config.network == Network::Devnet
                || genesis_active
            {
                let our_pubkey = *key.public_key();

                // Read our stored sequence from the persisted GSet so we resume
                // from where we left off. Without this, a restart resets to 0 and
                // every peer silently rejects our announcements as Duplicate until
                // our counter exceeds their stored sequence — causing us to vanish
                // from their active_producers() for potentially hours.
                let stored_seq = {
                    let gset = self.producer_gset.read().await;
                    gset.sequence_for(&our_pubkey)
                };
                let start_seq = stored_seq + 1;
                self.announcement_sequence
                    .store(start_seq, std::sync::atomic::Ordering::SeqCst);

                let gh = self.chain_state.read().await.genesis_hash;
                let announcement =
                    ProducerAnnouncement::new(key, self.config.network.id(), start_seq, gh);

                // Add to GSet (new format)
                {
                    let mut gset = self.producer_gset.write().await;
                    let _ = gset.merge_one(announcement.clone());
                }
                *self.our_announcement.write().await = Some(announcement);

                // Also add to legacy known_producers for compatibility during transition
                let mut known = self.known_producers.write().await;
                if !known.contains(&our_pubkey) {
                    known.push(our_pubkey);
                    known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                    let pubkey_hash = crypto_hash(our_pubkey.as_bytes());
                    info!(
                        "Registered self as bootstrap producer: {} (now {} known)",
                        &pubkey_hash.to_hex()[..16],
                        known.len()
                    );
                    self.last_producer_list_change = Some(Instant::now());
                }
                drop(known);

                // IMMEDIATE ANTI-ENTROPY: Broadcast our announcement via new format
                if let Some(ref network) = self.network {
                    let announcements = {
                        let gset = self.producer_gset.read().await;
                        gset.export()
                    };
                    if !announcements.is_empty() {
                        info!(
                            "Broadcasting initial producer announcements ({} producers)",
                            announcements.len()
                        );
                        let _ = network
                            .broadcast_producer_announcements(announcements)
                            .await;
                    }
                }
            }
        }

        // Compute genesis VDF proof in background during genesis phase.
        // This takes ~30s for 5M iterations. The proof will be embedded in a
        // Registration TX in the next block this producer creates.
        if let Some(ref key) = self.producer_key {
            let genesis_active = {
                let state = self.chain_state.read().await;
                self.config.network.is_in_genesis(state.best_height + 1)
            };
            if genesis_active {
                let our_pubkey = *key.public_key();
                let iterations = self.config.network.vdf_register_iterations();
                let vdf_input = vdf::registration_input(&our_pubkey, 0);
                info!(
                    "Computing genesis registration VDF proof ({} iterations)...",
                    iterations
                );
                let output =
                    tokio::task::spawn_blocking(move || hash_chain_vdf(&vdf_input, iterations))
                        .await?;
                self.genesis_vdf_output = Some(output);
                info!(
                    "Genesis VDF proof computed for {}",
                    hex::encode(&our_pubkey.as_bytes()[..8])
                );
            }
        }

        // NOTE: Do NOT call recompute_tier() here. At startup the on-chain ProducerSet
        // is incomplete (not synced yet). producer_tier() would default to Tier 3
        // (header-only), causing reconfigure_topics_for_tier(3) to unsubscribe from
        // BLOCKS_TOPIC — the node would stop receiving blocks and get stuck.
        // Tier computation runs safely at epoch boundaries (after sync completes).

        // Start RPC server if enabled
        if self.config.rpc.enabled {
            self.start_rpc().await?;
        }

        // Main event loop
        self.run_event_loop().await?;

        // Graceful shutdown: save all state before exiting
        self.shutdown().await?;

        Ok(())
    }

    /// Return the canonical genesis hash from the embedded chainspec.
    /// Always correct regardless of state_db corruption.
    pub(super) fn canonical_genesis_hash(&self) -> Hash {
        let spec = match self.config.network {
            Network::Mainnet => doli_core::chainspec::ChainSpec::mainnet(),
            Network::Testnet => doli_core::chainspec::ChainSpec::testnet(),
            Network::Devnet => doli_core::chainspec::ChainSpec::devnet(),
        };
        spec.genesis_hash()
    }

    /// Start the network service
    pub(super) async fn start_network(&mut self) -> Result<()> {
        let listen_addr: SocketAddr = self.config.listen_addr.parse()?;
        let genesis_hash = self.chain_state.read().await.genesis_hash;

        let mut network_config = NetworkConfig::for_network(self.config.network, genesis_hash);
        network_config.listen_addr = listen_addr;
        network_config.bootstrap_nodes = self.config.bootstrap_nodes.clone();
        network_config.max_peers = self.config.max_peers;
        network_config.no_dht = self.config.no_dht;
        // Store node_key in parent of data_dir so chain resets (which wipe data_dir)
        // don't regenerate the peer ID. Stable peer IDs prevent Kademlia mismatch storms.
        // Falls back to data_dir if parent doesn't exist (e.g., data_dir is root-level).
        let node_key_dir = self
            .config
            .data_dir
            .parent()
            .filter(|p| p.exists())
            .unwrap_or(&self.config.data_dir);
        network_config.node_key_path = Some(node_key_dir.join("node_key"));
        network_config.peer_cache_path = Some(self.config.data_dir.join("peers.cache"));

        // Dynamic gossip mesh: scale with total peer count (producers + seeds)
        // so ALL nodes are in each other's eager-push mesh. Seeds are not producers
        // but participate in gossip — excluding them leaves mesh gaps that cause
        // propagation delays and sync oscillation.
        let active_producers = self.producer_set.read().await.active_count();
        let seed_count = network_config.bootstrap_nodes.len();
        let total_peers = active_producers + seed_count;
        let mesh = network::gossip::compute_dynamic_mesh(total_peers);
        info!(
            "Gossip mesh: mesh_n={} mesh_n_low={} mesh_n_high={} gossip_lazy={} (producers={}, seeds={}, total={}, cap=20)",
            mesh.mesh_n, mesh.mesh_n_low, mesh.mesh_n_high, mesh.gossip_lazy, active_producers, seed_count, total_peers
        );
        network_config.mesh_n = mesh.mesh_n;
        network_config.mesh_n_low = mesh.mesh_n_low;
        network_config.mesh_n_high = mesh.mesh_n_high;
        network_config.gossip_lazy = mesh.gossip_lazy;

        // REQ-OPS-001: Warn when --no-dht used with many producers
        if network_config.no_dht && active_producers > 5 {
            warn!("════════════════════════════════════════════════════════════════");
            warn!(
                "  WARNING: --no-dht is set but {} active producers detected.",
                active_producers
            );
            warn!("  Peer discovery will be limited to bootstrap nodes only.");
            warn!("  Snap sync requires 3+ peers. Gossip mesh requires 6+ peers.");
            warn!("  Consider enabling DHT for networks with >5 nodes.");
            warn!("════════════════════════════════════════════════════════════════");
        }

        // NAT traversal: enable relay server if configured (for public/bootstrap nodes)
        if self.config.relay_server {
            network_config.nat_config = network::NatConfig::relay_server();
        }

        // External address: advertise a specific public address to peers
        if let Some(ref addr_str) = self.config.external_address {
            match addr_str.parse::<network::Multiaddr>() {
                Ok(addr) => {
                    network_config.external_address = Some(addr);
                }
                Err(e) => {
                    warn!("Invalid --external-address '{}': {}", addr_str, e);
                }
            }
        }

        info!(
            "Starting network service on {} (network={}, id={})",
            listen_addr,
            self.config.network.name(),
            self.config.network.id()
        );
        let network = NetworkService::new(network_config).await?;
        self.network = Some(network);

        info!("Network service started");
        Ok(())
    }

    /// Start the RPC server
    pub(super) async fn start_rpc(&self) -> Result<()> {
        let listen_addr: SocketAddr = self.config.rpc.listen_addr.parse()?;

        let rpc_config = RpcServerConfig {
            listen_addr,
            enable_cors: false,
            allowed_origins: vec![],
        };

        // Create sync status callback
        let sync_manager_for_rpc = self.sync_manager.clone();
        let chain_state_for_rpc = self.chain_state.clone();
        let sync_status_fn = move || {
            // Get sync state synchronously by creating a small runtime
            // This is acceptable for RPC as it's called infrequently
            let sync_manager = sync_manager_for_rpc.clone();
            let chain_state = chain_state_for_rpc.clone();

            // Use try_read to avoid blocking if lock is held
            let is_syncing = match sync_manager.try_read() {
                Ok(guard) => guard.state().is_syncing(),
                Err(_) => false, // Default to not syncing if lock unavailable
            };

            // Calculate progress if syncing
            let progress = if is_syncing {
                match (sync_manager.try_read(), chain_state.try_read()) {
                    (Ok(sync_guard), Ok(chain_guard)) => {
                        let local_height = chain_guard.best_height;
                        let target_height = sync_guard.best_peer_height();
                        if target_height > 0 {
                            let progress = (local_height as f64 / target_height as f64) * 100.0;
                            Some(progress.min(100.0))
                        } else {
                            Some(0.0)
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };

            SyncStatus {
                is_syncing,
                progress,
            }
        };

        // Create RPC context with references to node state
        let mut context = RpcContext::new_for_network(
            self.chain_state.clone(),
            self.block_store.clone(),
            self.utxo_set.clone(),
            self.mempool.clone(),
            self.params.clone(),
            self.config.network,
        )
        .with_blocks_per_reward_epoch(self.config.network.blocks_per_reward_epoch())
        .with_coinbase_maturity(self.config.network.coinbase_maturity())
        .with_bond_unit(self.config.network.bond_unit())
        .with_producer_set(self.producer_set.clone())
        .with_sync_status(sync_status_fn);

        // Wire up peer info so getNetworkInfo reports real values
        if let Some(ref network) = self.network {
            let peers = network.peers_arc();
            let peers_for_list = peers.clone();
            context = context
                .with_peer_id(network.local_peer_id().to_string())
                .with_peer_count(move || peers.try_read().map(|p| p.len()).unwrap_or(0))
                .with_peer_list(move || {
                    peers_for_list
                        .try_read()
                        .map(|p| {
                            p.values()
                                .map(|info| rpc::PeerInfoEntry {
                                    peer_id: info.id.clone(),
                                    address: info.address.clone(),
                                    best_height: info.best_height,
                                    connected_secs: info.connected_at.elapsed().as_secs(),
                                    last_seen_secs: info.last_seen.elapsed().as_secs(),
                                    latency_ms: info.latency.map(|d| d.as_millis() as u64),
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                });
        }

        // Wire up transaction broadcast so RPC-submitted txs are gossiped to peers
        if let Some(ref network) = self.network {
            let cmd_tx = network.command_sender();
            context = context.with_broadcast(move |tx: Transaction| {
                let cmd_tx = cmd_tx.clone();
                tokio::spawn(async move {
                    let _ = cmd_tx.send(NetworkCommand::BroadcastTransaction(tx)).await;
                });
            });
        }

        // Wire up vote broadcast so RPC-submitted votes are gossiped to peers
        // AND processed locally (gossip doesn't echo back to publisher)
        if let Some(ref network) = self.network {
            let cmd_tx = network.command_sender();
            let local_vote_tx = self.vote_tx.clone();
            context = context.with_broadcast_vote(move |vote_data: Vec<u8>| {
                let cmd_tx = cmd_tx.clone();
                let local_tx = local_vote_tx.clone();
                tokio::spawn(async move {
                    // Broadcast to network
                    let _ = cmd_tx
                        .send(NetworkCommand::BroadcastVote(vote_data.clone()))
                        .await;
                    // Also deliver to local update service (gossip won't echo back)
                    if let Some(tx) = local_tx {
                        if let Ok(vote_msg) =
                            serde_json::from_slice::<node_updater::VoteMessage>(&vote_data)
                        {
                            let _ = tx.send(vote_msg).await;
                        }
                    }
                });
            });
        }

        // Wire up update status callback to read live state from UpdateService
        if let Some(ref pending) = self.pending_update {
            let pending = pending.clone();
            let producer_set = self.producer_set.clone();
            context = context.with_update_status(move || {
                let pending_guard = pending.try_read();
                match pending_guard {
                    Ok(guard) => match guard.as_ref() {
                        Some(p) => {
                            let total_producers = producer_set
                                .try_read()
                                .map(|set| set.active_count())
                                .unwrap_or(0);
                            let veto_active =
                                !p.approved && !node_updater::veto_period_ended(&p.release);
                            serde_json::json!({
                                "pending_update": {
                                    "version": p.release.version,
                                    "published_at": p.release.published_at,
                                    "changelog": p.release.changelog,
                                    "approved": p.approved,
                                    "days_remaining": p.days_remaining(),
                                    "hours_remaining": p.hours_remaining(),
                                },
                                "veto_period_active": veto_active,
                                "veto_count": p.vote_tracker.veto_count(),
                                "veto_percent": p.vote_tracker.veto_percent(total_producers) as f64,
                                "enforcement": p.enforcement.as_ref().map(|e| serde_json::json!({
                                    "active": e.active,
                                    "min_version": e.min_version,
                                })),
                            })
                        }
                        None => serde_json::json!({
                            "pending_update": null,
                            "veto_period_active": false,
                            "veto_count": 0,
                            "veto_percent": 0.0
                        }),
                    },
                    Err(_) => serde_json::json!({
                        "pending_update": null,
                        "veto_period_active": false,
                        "veto_count": 0,
                        "veto_percent": 0.0
                    }),
                }
            });
        }

        // Wire up on-chain maintainer set for getMaintainerSet RPC
        if let Some(ref ms) = self.maintainer_state {
            context.maintainer_state = Some(ms.clone());
        }

        let (ws_tx, _ws_rx) = rpc::ws::broadcast_channel();
        *self.ws_sender.write().await = Some(ws_tx.clone());

        let server = RpcServer::new(rpc_config, context, ws_tx);
        info!("Starting RPC server on {} (WebSocket at /ws)", listen_addr);
        server.spawn();

        Ok(())
    }

    /// Recompute our tier classification from the active producer set.
    /// Called once per epoch boundary (when `height / SLOTS_PER_EPOCH` changes).
    pub(super) async fn recompute_tier(&mut self, height: u64) {
        let current_epoch = height / SLOTS_PER_EPOCH as u64;
        if self.last_tier_epoch == Some(current_epoch) {
            return; // Already computed for this epoch
        }

        let our_pubkey = match &self.producer_key {
            Some(kp) => *kp.public_key(),
            None => {
                self.our_tier = 0; // Non-producer
                self.last_tier_epoch = Some(current_epoch);
                return;
            }
        };

        let producers = self.producer_set.read().await;
        let active = producers.active_producers_at_height(height);
        let producers_with_weights: Vec<(PublicKey, u64)> = active
            .iter()
            .map(|p| (p.public_key, p.selection_weight()))
            .collect();
        drop(producers);

        let tier1_set = compute_tier1_set(&producers_with_weights);
        // Build sorted-by-weight list for producer_tier()
        // Reuse same sort order as compute_tier1_set (weight desc, pubkey asc)
        let mut all_sorted = producers_with_weights.clone();
        all_sorted.sort_unstable_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
        });
        let all_sorted_pks: Vec<PublicKey> = all_sorted.into_iter().map(|(pk, _)| pk).collect();

        let new_tier = producer_tier(&our_pubkey, &tier1_set, &all_sorted_pks);

        if self.our_tier != new_tier {
            info!(
                "Tier classification changed: {} -> {} (epoch {})",
                self.our_tier, new_tier, current_epoch
            );
            let mut sync = self.sync_manager.write().await;
            sync.set_tier(new_tier, producers_with_weights.len());
            // During genesis, keep min_peers=1 to allow bootstrapping with fewer nodes
            if self.config.network.is_in_genesis(height) {
                sync.set_min_peers_for_production(1);
            }
            drop(sync);

            // Reconfigure gossipsub topic subscriptions for the new tier
            // Tier 2 nodes get a deterministic region assignment
            let region = if new_tier == 2 {
                Some(doli_core::consensus::producer_region(&our_pubkey))
            } else {
                None
            };
            if let Some(ref network) = self.network {
                let _ = network.reconfigure_tier(new_tier, region).await;
            }
        }

        self.our_tier = new_tier;
        self.last_tier_epoch = Some(current_epoch);
    }

    /// Create an attestation for a block and broadcast it to the network.
    ///
    /// Adds attestation weight for the finality gadget.
    pub(super) async fn create_and_broadcast_attestation(
        &self,
        block_hash: Hash,
        slot: u32,
        height: u64,
    ) {
        let (private_key, public_key, weight) = match &self.producer_key {
            Some(kp) => {
                let pk = *kp.public_key();
                let producers = self.producer_set.read().await;
                let w = producers
                    .get_by_pubkey(&pk)
                    .map(|p| p.selection_weight())
                    .unwrap_or(0);
                (kp.private_key().clone(), pk, w)
            }
            None => return, // Non-producer can't attest
        };

        if weight == 0 {
            return; // Not active, skip attestation
        }

        let attestation = if let Some(ref bls_kp) = self.bls_key {
            Attestation::new_with_bls(
                block_hash,
                slot,
                height,
                weight,
                &private_key,
                public_key,
                bls_kp,
            )
        } else {
            Attestation::new(block_hash, slot, height, weight, &private_key, public_key)
        };

        // Add our own weight to finality tracker
        {
            let mut sync = self.sync_manager.write().await;
            sync.add_attestation_weight(&block_hash, weight);
        }

        // Broadcast to network
        if let Some(ref network) = self.network {
            let _ = network.broadcast_attestation(attestation.to_bytes()).await;
        }
    }
}
