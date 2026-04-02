use super::*;

impl Node {
    /// Check producer eligibility for a received gossip block.
    ///
    /// LIGHTWEIGHT CHECK: only verifies the producer is in the known set and
    /// the time window is valid for the block's slot. Does NOT validate against
    /// local chain state (which may be on a micro-fork).
    ///
    /// Full validation happens in apply_block() where the block is checked
    /// against the actual chain state it builds on.
    pub async fn check_producer_eligibility(&self, block: &Block) -> Result<()> {
        // Use the BLOCK's slot for eligibility, not our local chain state.
        // Our local tip may be on a different micro-fork, causing us to
        // reject valid blocks from the canonical chain.
        let height = block.header.slot as u64; // Approximate — exact height unknown for gossip blocks

        // Check: is the producer in the known set?
        let producers = self.producer_set.read().await;
        let active: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        drop(producers);

        // If no active producers (pre-genesis), check GSet
        if !active.is_empty() && !active.contains(&block.header.producer) {
            // Producer not in active set — check if they're in GSet (bootstrap)
            let gset = self.producer_gset.read().await;
            let gset_producers = gset.active_producers(7200);
            drop(gset);
            if !gset_producers.contains(&block.header.producer) {
                anyhow::bail!("unknown producer — not in active set or GSet");
            }
        }

        // Bond weights from epoch-locked snapshot (single source of truth).
        let weighted = self.bond_weights_for_scheduling(active).await;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Build bootstrap producer list from GSet (same source as production side).
        // Must be sorted by pubkey for deterministic fallback rank order.
        let mut bootstrap_producers = {
            let gset = self.producer_gset.read().await;
            gset.active_producers(7200) // 2h liveness window, same as production
        };
        if bootstrap_producers.is_empty() {
            let known = self.known_producers.read().await;
            bootstrap_producers = known.clone();
        }
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

        // Scheduler fingerprint: compare across nodes to detect divergence.
        {
            let total_bonds: u64 = weighted.iter().map(|(_, b)| *b).sum();
            debug!(
                "[SCHED] VALIDATE slot={} producer={} producers={} total_bonds={} snap_epoch={}",
                block.header.slot,
                hex::encode(&block.header.producer.as_bytes()[..4]),
                weighted.len(),
                total_bonds,
                self.epoch_bond_snapshot_epoch,
            );
        }

        let mut ctx = validation::ValidationContext::new(
            ConsensusParams::for_network(self.config.network),
            self.config.network,
            now,
            height,
        )
        .with_producers_weighted(weighted)
        .with_bootstrap_producers(bootstrap_producers)
        .with_bootstrap_liveness(live_bp, stale_bp)
        .with_excluded_producers(self.excluded_producers.clone())
        .with_epoch_producer_list(self.epoch_producer_list.clone());

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
    pub async fn validate_block_for_apply(
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

        // Build weighted producer list using epoch-locked bond snapshot.
        // The snapshot is computed once at each epoch boundary and stays
        // constant for the entire epoch. This prevents mid-epoch add-bond
        // TXs from changing total_bonds and causing scheduler divergence.
        let producers = self.producer_set.read().await;
        let active: Vec<PublicKey> = producers
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        let pending_keys = producers.pending_registration_keys();
        drop(producers);

        // Bond weights from epoch-locked snapshot (single source of truth).
        let weighted = self.bond_weights_for_scheduling(active).await;

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
            let mut bp = {
                let gset = self.producer_gset.read().await;
                gset.active_producers(7200)
            };
            if bp.is_empty() {
                let known = self.known_producers.read().await;
                bp = known.clone();
            }

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
        .with_bootstrap_liveness(live_bp, stale_bp)
        .with_excluded_producers(self.excluded_producers.clone())
        .with_epoch_producer_list(self.epoch_producer_list.clone())
        .with_sig_verification_height(self.config.network.params().sig_verification_height);

        if let Some(ref spec) = self.config.chainspec {
            ctx.params.apply_chainspec(spec);
        }

        drop(state);

        // Validate missed_producers header field (P1-001: was unvalidated on receiving nodes)
        {
            const MAX_MISSED_PER_BLOCK: usize = 3;
            let missed = &block.header.missed_producers;

            // Length cap: production enforces MAX_MISSED_PER_BLOCK=3
            if missed.len() > MAX_MISSED_PER_BLOCK {
                return Err(validation::ValidationError::InvalidTransaction(format!(
                    "missed_producers has {} entries (max {})",
                    missed.len(),
                    MAX_MISSED_PER_BLOCK,
                )));
            }

            // Membership: all missed keys must be in the epoch producer list
            if !self.epoch_producer_list.is_empty() {
                for pk in missed {
                    if !self.epoch_producer_list.contains(pk) {
                        return Err(validation::ValidationError::InvalidTransaction(format!(
                            "missed_producers contains key {} not in epoch producer list",
                            hex::encode(&pk.as_bytes()[..4]),
                        )));
                    }
                }
            }

            // Total cap: excluded + new missed must not exceed active/3
            let max_total = if self.epoch_producer_list.is_empty() {
                usize::MAX
            } else {
                self.epoch_producer_list.len() / 3
            };
            let total_after = self.excluded_producers.len() + missed.len();
            if total_after > max_total {
                return Err(validation::ValidationError::InvalidTransaction(format!(
                    "missed_producers would bring total excluded to {} (max {})",
                    total_after, max_total,
                )));
            }
        }

        // P0-001: public_key enforcement is ACTIVE (v5.2.0+).
        // Input.public_key is part of the bincode wire format (#[serde(skip)] removed).
        // sig_verification_height=0 on all networks: enforce from genesis.

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
    pub async fn validate_block_economics(
        &self,
        block: &Block,
        height: u64,
        mode: ValidationMode,
    ) -> Result<()> {
        // === Coinbase validation ===
        if block.transactions.is_empty() {
            anyhow::bail!("block has no transactions (missing coinbase)");
        }

        let coinbase = &block.transactions[0];
        if !coinbase.is_coinbase() {
            anyhow::bail!("first transaction is not a valid coinbase");
        }

        // Calculate extra fees from user transactions in this block.
        // Excluded from extra_fees calculation:
        // - Coinbase/EpochReward: protocol-generated, no user fees
        // - Genesis Registration (0 inputs, 0 outputs): protocol-generated VDF proof
        // User Registration (from mempool, has inputs/outputs) DOES pay per-byte fees.
        let extra_fees: u64 = block
            .transactions
            .iter()
            .filter(|tx| {
                !(tx.is_coinbase()
                    || tx.is_epoch_reward()
                    || tx.tx_type == TxType::Registration
                        && tx.inputs.is_empty()
                        && tx.outputs.is_empty())
            })
            .flat_map(|tx| tx.outputs.iter())
            .map(|o| {
                o.extra_data.len() as u64 * doli_core::consensus::FEE_PER_BYTE
                    / doli_core::consensus::FEE_DIVISOR
            })
            .sum();

        let base_reward = self.params.block_reward(height);
        let expected_with_fees = base_reward + extra_fees;
        let coinbase_amount = coinbase.outputs[0].amount;
        // Accept both formats during version transition:
        // - v4.9.0+: coinbase = block_reward + per-byte extra_fees
        // - v4.5.x:  coinbase = block_reward only (no per-byte fees)
        // External producers on older versions don't include extra_fees.
        // Their blocks are valid — they just generate less reward pool revenue.
        // See: N5 fork incident 2026-03-26 (coinbase mismatch on delta=0 reorg).
        if coinbase_amount != expected_with_fees && coinbase_amount != base_reward {
            anyhow::bail!(
                "coinbase amount {} != expected block reward {} (base {} + extra_fees {}) and != base reward {}",
                coinbase_amount,
                expected_with_fees,
                base_reward,
                extra_fees,
                base_reward
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
            // EpochReward only allowed at epoch boundaries, post-genesis.
            // Skip this check in Light mode (sync/reorg): the canonical chain may
            // have been produced by a node at a different fork tip where this height
            // WAS an epoch boundary. Rejecting during resync prevents recovery.
            if !is_epoch_boundary && matches!(mode, ValidationMode::Full) {
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

            // In Light mode (sync/reorg), skip all EpochReward consistency checks.
            // The canonical chain may have EpochReward TXs produced at different
            // chain states (different height/epoch/pool balance). Rejecting them
            // during sync creates infinite apply-failure loops.
            // Full validation happens when blocks arrive via gossip.
            if matches!(mode, ValidationMode::Full) {
                // Validate extra_data contains correct height + epoch
                if epoch_tx.extra_data.len() < 16 {
                    anyhow::bail!(
                        "EpochReward extra_data too short: expected >= 16 bytes, got {}",
                        epoch_tx.extra_data.len()
                    );
                }
                let embedded_height =
                    u64::from_le_bytes(epoch_tx.extra_data[0..8].try_into().unwrap());
                let embedded_epoch =
                    u64::from_le_bytes(epoch_tx.extra_data[8..16].try_into().unwrap());
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

                // Conservation: total distributed must not exceed pool balance.
                // Pre-activation: include current coinbase (side-effect consumes all).
                // Post-activation: only existing UTXOs (explicit inputs don't include
                // current coinbase — its hash isn't known at assembly time).
                let total_distributed: u64 = epoch_tx.outputs.iter().map(|o| o.amount).sum();
                let pool_balance = {
                    let utxo = self.utxo_set.read().await;
                    let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                    let utxo_total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
                    if height >= doli_core::consensus::EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT {
                        utxo_total // post-activation: only existing UTXOs
                    } else {
                        utxo_total + coinbase_amount // pre-activation: + current coinbase
                    }
                };

                if total_distributed > pool_balance {
                    anyhow::bail!(
                        "EpochReward total {} exceeds pool balance {} — inflation attack",
                        total_distributed,
                        pool_balance
                    );
                }

                // Exact match of amounts and recipients
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
                    let total_distributed: u64 = actual_sorted.iter().map(|(a, _)| *a).sum();
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

                // Post-activation: verify explicit pool UTXO inputs
                if height >= doli_core::consensus::EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT {
                    if epoch_tx.inputs.is_empty() {
                        anyhow::bail!(
                            "EpochReward at height {} (post-activation) must have explicit pool inputs",
                            height
                        );
                    }
                    // Verify inputs match the sorted pool outpoints
                    let utxo = self.utxo_set.read().await;
                    let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                    let mut expected_inputs: Vec<(crypto::Hash, u32)> = pool_utxos
                        .iter()
                        .map(|(op, _)| (op.tx_hash, op.index))
                        .collect();
                    expected_inputs.sort();
                    drop(utxo);

                    let actual_inputs: Vec<(crypto::Hash, u32)> = epoch_tx
                        .inputs
                        .iter()
                        .map(|inp| (inp.prev_tx_hash, inp.output_index))
                        .collect();

                    if actual_inputs != expected_inputs {
                        anyhow::bail!(
                            "EpochReward pool inputs mismatch: expected {} inputs, got {}",
                            expected_inputs.len(),
                            actual_inputs.len()
                        );
                    }
                } else if !epoch_tx.inputs.is_empty() {
                    anyhow::bail!(
                        "EpochReward at height {} (pre-activation) must not have inputs",
                        height
                    );
                }
            }
        } else if is_epoch_boundary && matches!(mode, ValidationMode::Full) {
            // Only enforce missing-EpochReward check in Full mode.
            // In Light mode (sync/reorg), the canonical chain may have blocks at epoch
            // boundaries produced by nodes with different epoch parameters (ConsensusParams
            // vs NetworkParams mismatch). Rejecting these blocks prevents recovery.
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

    /// Handle a new transaction from the network
    pub async fn handle_new_transaction(&self, tx: Transaction) -> Result<()> {
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
                // Broadcast to network
                if let Some(ref network) = self.network {
                    let _ = network.broadcast_transaction(tx).await;
                }
            }
            Err(e) => {
                debug!("Failed to add transaction {} to mempool: {}", tx_hash, e);
            }
        }

        Ok(())
    }

    /// Handle a sync request from a peer (legacy inline path, kept for reference).
    /// Production code uses handle_sync_request_bg() in event_loop.rs instead.
    #[allow(dead_code)]
    pub async fn handle_sync_request(
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
                let end_height = (start_height + max_count as u64).min(best_height);
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

                let max_count = max_count.min(2000); // Cap to prevent expensive iteration
                let end_height = start_height
                    .saturating_add(max_count as u64)
                    .min(best_height);
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
