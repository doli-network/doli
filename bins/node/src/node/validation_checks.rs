use super::*;

impl Node {
    /// Check producer eligibility for a received block.
    ///
    /// Builds a lightweight ValidationContext and calls validate_producer_eligibility.
    /// During bootstrap, validates using fallback rank windows from the GSet producer list.
    pub(super) async fn check_producer_eligibility(&self, block: &Block) -> Result<()> {
        let state = self.chain_state.read().await;
        let height = state.best_height + 1;

        // Build weighted producer list using Reactive Round-Robin:
        // equal weight (1 ticket each), filtered by scheduled flag.
        let producers = self.producer_set.read().await;
        let weighted: Vec<(PublicKey, u64)> = producers
            .scheduled_producers_at_height(height)
            .iter()
            .map(|p| (p.public_key, 1u64)) // Equal weight: 1 ticket per producer
            .collect();
        drop(producers);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Build bootstrap producer list for validation.
        // INC-001 RC-10: During genesis, use HARDCODED genesis producers — identical
        // to the production side (scheduling.rs RC-5A). The GSet hasn't converged
        // during genesis and returns 1 producer instead of 5, causing "invalid
        // producer for slot" rejections for 4 out of 5 valid producers.
        let mut bootstrap_producers = if self.config.network.is_in_genesis(height) {
            match self.config.network {
                Network::Testnet => doli_core::genesis::testnet_genesis_producers()
                    .into_iter()
                    .map(|(pk, _)| pk)
                    .collect::<Vec<_>>(),
                Network::Mainnet => doli_core::genesis::mainnet_genesis_producers()
                    .into_iter()
                    .map(|(pk, _)| pk)
                    .collect::<Vec<_>>(),
                Network::Devnet => {
                    // Devnet: no hardcoded list, use GSet
                    let gset = self.producer_gset.read().await;
                    let mut bp = gset.active_producers(7200);
                    if bp.is_empty() {
                        let known = self.known_producers.read().await;
                        bp = known.clone();
                    }
                    bp
                }
            }
        } else {
            let gset = self.producer_gset.read().await;
            let mut bp = gset.active_producers(7200);
            if bp.is_empty() {
                let known = self.known_producers.read().await;
                bp = known.clone();
            }
            bp
        };
        bootstrap_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        // Build liveness split for bootstrap validation (must match production side).
        let num_bp = bootstrap_producers.len();
        let liveness_window = std::cmp::max(
            consensus::LIVENESS_WINDOW_MIN,
            (num_bp as u64).saturating_mul(3),
        );
        let chain_height = height.saturating_sub(1);
        let cutoff = chain_height.saturating_sub(liveness_window);
        let (live_bp, stale_bp): (Vec<PublicKey>, Vec<PublicKey>) = {
            let (live, stale): (Vec<_>, Vec<_>) = bootstrap_producers.iter().partition(|pk| {
                match self.producer_liveness.get(pk) {
                    Some(&last_h) => last_h >= cutoff,
                    // No chain record: live if chain is young, stale otherwise
                    None => chain_height < liveness_window,
                }
            });
            (
                live.into_iter().copied().collect(),
                stale.into_iter().copied().collect(),
            )
        };
        // Deadlock safety: if all stale, treat all as live (filter disabled)
        let (live_bp, stale_bp) = if live_bp.is_empty() {
            (bootstrap_producers.clone(), Vec::new())
        } else {
            (live_bp, stale_bp)
        };

        let mut ctx = validation::ValidationContext::new(
            ConsensusParams::for_network(self.config.network),
            self.config.network,
            now,
            height,
        )
        .with_producers_weighted(weighted)
        .with_bootstrap_producers(bootstrap_producers)
        .with_bootstrap_liveness(live_bp, stale_bp);

        // Apply chainspec if present
        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }

        validation::validate_producer_eligibility(&block.header, &ctx)?;
        Ok(())
    }

    /// Validate a block before applying it to the chain.
    ///
    /// Builds a full ValidationContext and calls `validate_block_with_mode`.
    /// In `Light` mode (gap blocks after snap sync), VDF is skipped.
    /// In `Full` mode (recent blocks near tip), VDF is verified.
    pub(super) async fn validate_block_for_apply(
        &self,
        block: &Block,
        height: u64,
        mode: ValidationMode,
    ) -> Result<(), validation::ValidationError> {
        let state = self.chain_state.read().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Build weighted producer list using Reactive Round-Robin:
        // equal weight (1 ticket each), filtered by scheduled flag.
        let producers = self.producer_set.read().await;
        let weighted: Vec<(PublicKey, u64)> = producers
            .scheduled_producers_at_height(height)
            .iter()
            .map(|p| (p.public_key, 1u64)) // Equal weight: 1 ticket per producer
            .collect();
        let pending_keys = producers.pending_registration_keys();
        drop(producers);

        // Build bootstrap producer list for validation.
        //
        // For Light mode (sync): the GSet reflects CURRENT network state
        // (includes producers that joined after genesis, e.g. N6/N8), but
        // historical blocks were produced with a DIFFERENT GSet composition.
        // bootstrap_fallback_order uses (slot + rank) % n — a different n
        // means completely different rank assignments → "invalid producer
        // for slot". Pass empty bootstrap_producers for ALL synced blocks:
        // - Genesis-phase blocks: accepted via empty-bootstrap-list fallback
        // - Transition block (361): same — producer_set not yet populated
        // - Post-genesis blocks: validated by deterministic bond-weighted
        //   scheduler (on-chain data), bypassing bootstrap path entirely
        // This is safe: header chain continuity is verified during header
        // download, and blocks were already validated by the network.
        let (bootstrap_producers, live_bp, stale_bp) = if mode == ValidationMode::Light {
            (Vec::new(), Vec::new(), Vec::new())
        } else {
            // INC-001 RC-10: During genesis, use hardcoded genesis producers (same as
            // production/scheduling.rs and check_producer_eligibility above).
            let mut bp = if self.config.network.is_in_genesis(height) {
                match self.config.network {
                    Network::Testnet => doli_core::genesis::testnet_genesis_producers()
                        .into_iter()
                        .map(|(pk, _)| pk)
                        .collect::<Vec<_>>(),
                    Network::Mainnet => doli_core::genesis::mainnet_genesis_producers()
                        .into_iter()
                        .map(|(pk, _)| pk)
                        .collect::<Vec<_>>(),
                    Network::Devnet => {
                        let gset = self.producer_gset.read().await;
                        let mut gp = gset.active_producers(7200);
                        if gp.is_empty() {
                            let known = self.known_producers.read().await;
                            gp = known.clone();
                        }
                        gp
                    }
                }
            } else {
                let gset = self.producer_gset.read().await;
                let mut gp = gset.active_producers(7200);
                if gp.is_empty() {
                    let known = self.known_producers.read().await;
                    gp = known.clone();
                }
                gp
            };

            // ACTIVATION_DELAY filter: mirror the production code's filtering
            // (node.rs try_produce_block lines 4993-5014). Without this, the
            // validation path may compute a different producer count N than
            // production, causing slot % N mismatches → "invalid producer for slot".
            {
                let producers = self.producer_set.read().await;
                bp.retain(|pk| match producers.get_by_pubkey(pk) {
                    Some(info) => {
                        if !info.is_active() {
                            return false;
                        }
                        // Genesis producers: always eligible
                        if info.registered_at == 0 {
                            return true;
                        }
                        // Late joiners: must wait activation delay
                        height >= info.registered_at + storage::ACTIVATION_DELAY
                    }
                    None => {
                        // Not registered (gossip-discovered): include in bootstrap
                        true
                    }
                });
            }

            bp.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

            // Build liveness split
            let num_bp = bp.len();
            let liveness_window = std::cmp::max(
                consensus::LIVENESS_WINDOW_MIN,
                (num_bp as u64).saturating_mul(3),
            );
            let chain_height = height.saturating_sub(1);
            let cutoff = chain_height.saturating_sub(liveness_window);
            let (live, stale): (Vec<PublicKey>, Vec<PublicKey>) = {
                let (l, s): (Vec<_>, Vec<_>) =
                    bp.iter()
                        .partition(|pk| match self.producer_liveness.get(pk) {
                            Some(&last_h) => last_h >= cutoff,
                            None => chain_height < liveness_window,
                        });
                (
                    l.into_iter().copied().collect(),
                    s.into_iter().copied().collect(),
                )
            };
            if live.is_empty() {
                (bp.clone(), bp, Vec::new())
            } else {
                (bp, live, stale)
            }
        };

        // Get previous block timestamp from block store for header validation
        let prev_timestamp = self
            .block_store
            .get_header(&state.best_hash)
            .ok()
            .flatten()
            .map(|h| h.timestamp)
            .unwrap_or(0);

        let mut ctx = validation::ValidationContext::new(
            ConsensusParams::for_network(self.config.network),
            self.config.network,
            now,
            height,
        )
        .with_prev_block(state.best_slot, prev_timestamp, state.best_hash)
        .with_producers_weighted(weighted)
        .with_pending_producer_keys(pending_keys)
        .with_bootstrap_producers(bootstrap_producers)
        .with_bootstrap_liveness(live_bp, stale_bp);

        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }

        drop(state);

        validation::validate_block_with_mode(block, &ctx, mode)
    }

    /// Validate block economics — prevents inflation and reward theft.
    ///
    /// Checks that cannot be done in the core validation crate because they
    /// require access to the UTXO set, producer registry, and block store.
    ///
    /// ## Coinbase validation (every block)
    /// - First TX must be coinbase (Transfer, no inputs, 1 output)
    /// - Amount must equal `block_reward(height)`
    /// - Recipient must be `reward_pool_pubkey_hash()`
    ///
    /// ## EpochReward validation (epoch boundary blocks)
    /// - EpochReward TX only allowed at epoch boundaries, post-genesis, epoch > 0
    /// - At most one EpochReward TX per block
    /// - Total distributed must not exceed pool balance (conservation)
    /// - Exact match of amounts and recipients (both Full and Light modes)
    pub(super) async fn validate_block_economics(
        &self,
        block: &Block,
        height: u64,
        _mode: ValidationMode,
    ) -> Result<()> {
        // === Coinbase validation ===
        if block.transactions.is_empty() {
            anyhow::bail!("block has no transactions (missing coinbase)");
        }

        let coinbase = &block.transactions[0];
        if !coinbase.is_coinbase() {
            anyhow::bail!("first transaction is not a valid coinbase");
        }

        let expected_reward = self.params.block_reward(height);
        if coinbase.outputs[0].amount != expected_reward {
            anyhow::bail!(
                "coinbase amount {} != expected block reward {}",
                coinbase.outputs[0].amount,
                expected_reward
            );
        }

        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        if coinbase.outputs[0].pubkey_hash != pool_hash {
            anyhow::bail!("coinbase recipient is not the reward pool — possible theft attempt");
        }

        // === EpochReward validation ===
        let epoch_reward_txs: Vec<&Transaction> = block
            .transactions
            .iter()
            .filter(|tx| tx.tx_type == TxType::EpochReward)
            .collect();

        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let is_epoch_boundary = height > 0
            && !self.config.network.is_in_genesis(height)
            && reward_epoch::is_epoch_start_with(height, blocks_per_epoch);

        if !epoch_reward_txs.is_empty() {
            // EpochReward only allowed at epoch boundaries, post-genesis
            if !is_epoch_boundary {
                anyhow::bail!(
                    "EpochReward transaction at non-epoch-boundary height {}",
                    height
                );
            }

            let completed_epoch = (height / blocks_per_epoch) - 1;

            // No EpochReward at epoch 0 (genesis bonds drained the pool)
            if completed_epoch == 0 {
                anyhow::bail!("EpochReward not allowed at epoch 0 (genesis pool used for bonds)");
            }

            // Exactly one EpochReward TX per block
            if epoch_reward_txs.len() != 1 {
                anyhow::bail!(
                    "expected at most 1 EpochReward TX, got {}",
                    epoch_reward_txs.len()
                );
            }
            let epoch_tx = epoch_reward_txs[0];

            // Validate extra_data contains correct height + epoch
            if epoch_tx.extra_data.len() < 16 {
                anyhow::bail!(
                    "EpochReward extra_data too short: expected >= 16 bytes, got {}",
                    epoch_tx.extra_data.len()
                );
            }
            let embedded_height = u64::from_le_bytes(epoch_tx.extra_data[0..8].try_into().unwrap());
            let embedded_epoch = u64::from_le_bytes(epoch_tx.extra_data[8..16].try_into().unwrap());
            if embedded_height != height {
                anyhow::bail!(
                    "EpochReward embedded height {} != block height {}",
                    embedded_height,
                    height
                );
            }
            if embedded_epoch != completed_epoch {
                anyhow::bail!(
                    "EpochReward embedded epoch {} != completed epoch {}",
                    embedded_epoch,
                    completed_epoch
                );
            }

            // Conservation: total distributed must not exceed pool balance
            let total_distributed: u64 = epoch_tx.outputs.iter().map(|o| o.amount).sum();
            let pool_balance = {
                let utxo = self.utxo_set.read().await;
                let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                let utxo_total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
                // Include current block's coinbase (not yet in UTXO set)
                utxo_total + self.params.block_reward(height)
            };

            if total_distributed > pool_balance {
                anyhow::bail!(
                    "EpochReward total {} exceeds pool balance {} — inflation attack",
                    total_distributed,
                    pool_balance
                );
            }

            // Exact match of amounts and recipients (both Full and Light modes)
            let expected = self.calculate_epoch_rewards(completed_epoch).await;

            let mut expected_sorted: Vec<(u64, crypto::Hash)> = expected;
            expected_sorted.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

            let mut actual_sorted: Vec<(u64, crypto::Hash)> = epoch_tx
                .outputs
                .iter()
                .map(|o| (o.amount, o.pubkey_hash))
                .collect();
            actual_sorted.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

            if expected_sorted != actual_sorted {
                let expected_total: u64 = expected_sorted.iter().map(|(a, _)| *a).sum();
                anyhow::bail!(
                    "EpochReward distribution mismatch: expected {} outputs totaling {}, \
                     got {} outputs totaling {} — possible reward theft",
                    expected_sorted.len(),
                    expected_total,
                    actual_sorted.len(),
                    total_distributed
                );
            }
        } else if is_epoch_boundary {
            let completed_epoch = (height / blocks_per_epoch) - 1;
            if completed_epoch > 0 {
                let expected = self.calculate_epoch_rewards(completed_epoch).await;
                if !expected.is_empty() {
                    anyhow::bail!(
                        "epoch boundary block at height {} missing EpochReward TX for epoch {} ({} qualified producers)",
                        height, completed_epoch, expected.len()
                    );
                }
            }
        }

        Ok(())
    }

    /// Handle a new transaction received from the network (gossip or txfetch).
    ///
    /// NOTE: This does NOT re-broadcast the tx. GossipSub already handles
    /// multi-hop forwarding through the mesh (eager push to mesh peers,
    /// IHAVE/IWANT to non-mesh peers). Application-level re-broadcast would
    /// create new gossipsub messages with different batch compositions,
    /// bypassing gossipsub's dedup and causing amplification at scale.
    ///
    /// RPC-submitted txs are broadcast separately via the `broadcast_tx`
    /// callback wired in startup.rs, which sends directly to gossipsub.
    pub(super) async fn handle_new_transaction(&self, tx: Transaction) -> Result<()> {
        let tx_hash = tx.hash();

        // Check if we already have this transaction
        {
            let mempool = self.mempool.read().await;
            if mempool.contains(&tx_hash) {
                debug!("Transaction {} already in mempool", tx_hash);
                return Ok(());
            }
        }

        // Add to mempool
        let current_height = self.chain_state.read().await.best_height;
        let result = {
            let utxo = self.utxo_set.read().await;
            let mut mempool = self.mempool.write().await;
            mempool.add_transaction(tx.clone(), &utxo, current_height)
        };

        match result {
            Ok(_) => {
                info!("Added transaction {} to mempool", tx_hash);
                // Broadcast to WebSocket subscribers
                if let Some(ref ws_tx) = *self.ws_sender.read().await {
                    let tx_type = format!("{:?}", tx.tx_type).to_lowercase();
                    let _ = ws_tx.send(rpc::WsEvent::NewTx {
                        hash: tx_hash.to_hex(),
                        tx_type,
                        size: tx.size(),
                        fee: 0,
                    });
                }
                // No network re-broadcast — gossipsub handles forwarding.
            }
            Err(e) => {
                debug!("Failed to add transaction {} to mempool: {}", tx_hash, e);
            }
        }

        Ok(())
    }

    /// Handle a sync request from a peer
    pub(super) async fn handle_sync_request(
        &self,
        request: network::protocols::SyncRequest,
        channel: network::ResponseChannel<network::protocols::SyncResponse>,
    ) -> Result<()> {
        let response = match request {
            SyncRequest::GetHeaders {
                start_hash,
                max_count,
            } => {
                let mut headers = Vec::new();
                let state = self.chain_state.read().await;
                let genesis_hash = state.genesis_hash;
                let best_height = state.best_height;
                drop(state);

                // Determine starting height via O(1) hash→height index.
                // The hash_to_height index is populated by:
                // 1. rebuild_canonical_index (one-time migration on startup)
                // 2. Normal block insertion during sync/production
                // No linear fallback — avoids O(n) scans that caused timeouts.
                let start_height = if start_hash == genesis_hash {
                    0
                } else {
                    match self
                        .block_store
                        .get_height_by_hash(&start_hash)
                        .ok()
                        .flatten()
                    {
                        Some(h) => h,
                        None => {
                            // Unknown hash — respond empty so requester doesn't timeout
                            debug!(
                                "GetHeaders: unknown start_hash {} (responding with empty)",
                                start_hash
                            );
                            if let Some(ref network) = self.network {
                                let _ = network
                                    .send_sync_response(channel, SyncResponse::Headers(vec![]))
                                    .await;
                            }
                            return Ok(());
                        }
                    }
                };

                // Return headers from start_height+1 up to max_count
                // Use get_hash_by_height → get_header to avoid deserializing full blocks
                // Seed mode: only serve confirmed blocks (N blocks deep) to prevent
                // fork propagation to syncing peers.
                let serve_height = if self.config.seed_mode {
                    best_height.saturating_sub(consensus::SEED_CONFIRMATION_DEPTH)
                } else {
                    best_height
                };
                let end_height = (start_height + max_count as u64).min(serve_height);
                for height in (start_height + 1)..=end_height {
                    if let Ok(Some(hash)) = self.block_store.get_hash_by_height(height) {
                        if let Ok(Some(header)) = self.block_store.get_header(&hash) {
                            headers.push(header);
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                debug!(
                    "GetHeaders: returning {} headers (heights {}..={})",
                    headers.len(),
                    start_height + 1,
                    end_height
                );
                SyncResponse::Headers(headers)
            }

            SyncRequest::GetBodies { hashes } => {
                let mut bodies = Vec::new();
                for hash in hashes {
                    if let Ok(Some(block)) = self.block_store.get_block(&hash) {
                        bodies.push(block);
                    }
                }
                SyncResponse::Bodies(bodies)
            }

            SyncRequest::GetBlockByHeight { height } => {
                match self.block_store.get_block_by_height(height) {
                    Ok(Some(block)) => SyncResponse::Block(Some(block)),
                    _ => SyncResponse::Block(None),
                }
            }

            SyncRequest::GetBlockByHash { hash } => match self.block_store.get_block(&hash) {
                Ok(Some(block)) => SyncResponse::Block(Some(block)),
                _ => SyncResponse::Block(None),
            },

            // INC-I-012 F1: Height-based header request. Used after snap sync
            // when the node's local_hash is unrecognizable by peers. The server
            // uses its OWN canonical chain at start_height, bypassing the hash
            // lookup that causes the deadlock.
            SyncRequest::GetHeadersByHeight {
                start_height,
                max_count,
            } => {
                let mut headers = Vec::new();
                let state = self.chain_state.read().await;
                let best_height = state.best_height;
                drop(state);

                let serve_height = if self.config.seed_mode {
                    best_height.saturating_sub(consensus::SEED_CONFIRMATION_DEPTH)
                } else {
                    best_height
                };
                let max_count = max_count.min(2000); // Cap to prevent expensive iteration
                let end_height = start_height
                    .saturating_add(max_count as u64)
                    .min(serve_height);
                for height in (start_height + 1)..=end_height {
                    if let Ok(Some(hash)) = self.block_store.get_hash_by_height(height) {
                        if let Ok(Some(header)) = self.block_store.get_header(&hash) {
                            headers.push(header);
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }

                debug!(
                    "GetHeadersByHeight: returning {} headers (heights {}..={})",
                    headers.len(),
                    start_height + 1,
                    end_height
                );
                SyncResponse::Headers(headers)
            }

            SyncRequest::GetStateRoot { block_hash: _ } => {
                // Use cached state root to avoid race conditions.
                // The cache is updated atomically after each apply_block, so all
                // three components (ChainState, UTXO, ProducerSet) are guaranteed
                // to be at the same height.
                let cache = self.cached_state_root.read().await;
                if let Some((root, hash, height)) = *cache {
                    SyncResponse::StateRoot {
                        block_hash: hash,
                        block_height: height,
                        state_root: root,
                    }
                } else {
                    // Fallback: compute on-the-fly if cache not yet populated (pre-first-block)
                    drop(cache);
                    let chain_state = self.chain_state.read().await;
                    let current_hash = chain_state.best_hash;
                    let current_height = chain_state.best_height;
                    let utxo_set = self.utxo_set.read().await;
                    let ps = self.producer_set.read().await;
                    match storage::compute_state_root(&chain_state, &utxo_set, &ps) {
                        Ok(root) => SyncResponse::StateRoot {
                            block_hash: current_hash,
                            block_height: current_height,
                            state_root: root,
                        },
                        Err(e) => SyncResponse::Error(format!("State root error: {}", e)),
                    }
                }
            }

            SyncRequest::GetStateSnapshot { block_hash } => {
                let chain_state = self.chain_state.read().await;
                // Serve snapshot at current tip regardless of requested hash.
                // The requesting node verifies the state root against quorum votes.
                // Previously this rejected requests where best_hash != block_hash,
                // causing a race condition: the peer advances between vote and
                // download, making snap sync fail 100% of the time on active chains.
                if chain_state.best_hash != block_hash {
                    info!(
                        "[SNAP_SYNC] Requested hash {} differs from tip {} — serving current tip (client verifies root)",
                        block_hash, chain_state.best_hash
                    );
                }
                let utxo_set = self.utxo_set.read().await;
                let ps = self.producer_set.read().await;
                match storage::StateSnapshot::create(&chain_state, &utxo_set, &ps) {
                    Ok(snap) => {
                        info!(
                            "[SNAP_SYNC] Serving snapshot at height={}, size={}KB, root={}",
                            snap.block_height,
                            snap.total_bytes() / 1024,
                            snap.state_root
                        );
                        SyncResponse::StateSnapshot {
                            block_hash: snap.block_hash,
                            block_height: snap.block_height,
                            chain_state: snap.chain_state_bytes,
                            utxo_set: snap.utxo_set_bytes,
                            producer_set: snap.producer_set_bytes,
                            state_root: snap.state_root,
                        }
                    }
                    Err(e) => SyncResponse::Error(format!("Snapshot error: {}", e)),
                }
            }
        };

        if let Some(ref network) = self.network {
            let _ = network.send_sync_response(channel, response).await;
        }

        Ok(())
    }
}
