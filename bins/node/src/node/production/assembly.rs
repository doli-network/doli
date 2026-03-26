use super::*;

impl Node {
    /// Build block content: coinbase, epoch rewards, genesis VDF registration, mempool txs,
    /// attestation bitfield, and header. Returns `None` if build failed (slot monotonicity
    /// violation or slot boundary crossed during assembly).
    pub(super) async fn build_block_content(
        &mut self,
        prev_hash: Hash,
        prev_slot: u32,
        height: u64,
        current_slot: u32,
        our_pubkey: PublicKey,
    ) -> Result<Option<(BlockHeader, Vec<Transaction>)>> {
        // Build the block — single transaction list used for both merkle root and block body.
        // All transactions MUST be added to the builder BEFORE build(), which computes the
        // merkle root from exactly the transactions that will be in the final block.
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let mut builder =
            BlockBuilder::new(prev_hash, prev_slot, our_pubkey).with_params(self.params.clone());

        // Coinbase is deferred until after mempool tx inclusion, so that
        // per-byte extra fees from user transactions can be routed to the
        // reward pool via an inflated coinbase amount.
        // See add_coinbase_with_extra() call below.

        if !self.config.network.is_in_genesis(height) {
            // POST-GENESIS: distribute pool at epoch boundaries.
            let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
            if height > 0 && reward_epoch::is_epoch_start_with(height, blocks_per_epoch) {
                let completed_epoch = (height / blocks_per_epoch) - 1;

                // Skip epoch 0: genesis bonds consumed pool funds, remainder carries to E1.
                // Distributing E0 would create coins from nothing (pool drained by bonds).
                if completed_epoch == 0 {
                    info!("Epoch 0 (genesis): pool remainder carries to E1, no distribution");
                } else {
                    info!(
                        "Epoch {} completed at height {}, distributing pool rewards...",
                        completed_epoch, height
                    );

                    let epoch_outputs = self.calculate_epoch_rewards(completed_epoch).await;

                    if !epoch_outputs.is_empty() {
                        let total_reward: u64 = epoch_outputs.iter().map(|(amt, _)| *amt).sum();
                        info!(
                            "Distributing {} total reward to {} qualified producers for epoch {}",
                            total_reward,
                            epoch_outputs.len(),
                            completed_epoch
                        );

                        let coinbase = Transaction::new_epoch_reward_coinbase(
                            epoch_outputs,
                            height,
                            completed_epoch,
                        );
                        builder.add_transaction(coinbase);
                    } else {
                        debug!(
                            "No qualified producers in epoch {} — pool accumulates to next epoch",
                            completed_epoch
                        );
                    }
                } // else completed_epoch != 0
            }
        }

        // During genesis: include VDF proof Registration TX in EVERY block we produce.
        // Idempotent — derive_genesis_producers_from_chain() deduplicates by public key.
        // Must not use a one-shot flag: orphaned blocks would set it permanently,
        // preventing retry in the canonical chain.
        if let Some(vdf_output_bytes) = self.genesis_vdf_output {
            if self.config.network.is_in_genesis(height) {
                let (bls_pubkey_bytes, bls_pop_bytes) = if let Some(ref bls_kp) = self.bls_key {
                    let pop = bls_kp
                        .proof_of_possession()
                        .expect("PoP signing cannot fail");
                    (
                        bls_kp.public_key().as_bytes().to_vec(),
                        pop.as_bytes().to_vec(),
                    )
                } else {
                    (vec![], vec![])
                };
                let reg_data = RegistrationData {
                    public_key: our_pubkey,
                    epoch: 0,
                    vdf_output: vdf_output_bytes.to_vec(),
                    vdf_proof: vec![],
                    prev_registration_hash: Hash::ZERO,
                    sequence_number: 0,
                    bond_count: 0, // Zero bond — handled at genesis end
                    bls_pubkey: bls_pubkey_bytes,
                    bls_pop: bls_pop_bytes,
                };
                let extra_data = bincode::serialize(&reg_data)
                    .expect("RegistrationData serialization cannot fail");
                let reg_tx = Transaction {
                    version: 1,
                    tx_type: TxType::Registration,
                    inputs: vec![],
                    outputs: vec![],
                    extra_data,
                };
                builder.add_transaction(reg_tx);
                info!(
                    "Included genesis VDF proof Registration TX in block {} for {}",
                    height,
                    hex::encode(&our_pubkey.as_bytes()[..8])
                );
            }
        }

        // Add transactions from mempool (validate covenant conditions before inclusion)
        // REQ-PROD-001: Time-bounded block building — 60% of slot for TX validation,
        // leaving 40% for VDF, signing, and gossip broadcast.
        let mempool_txs: Vec<Transaction> = {
            let mempool = self.mempool.read().await;
            mempool.select_for_block(1_000_000) // Up to ~1MB of transactions per block
        };
        {
            let deadline = Instant::now() + Duration::from_millis(self.params.slot_duration * 600); // 60% of slot
            let utxo = self.utxo_set.read().await;
            // Use self.params which already has chainspec overrides applied
            // (ConsensusParams::for_network() ignores chainspec env vars for bond_unit)
            let utxo_ctx = validation::ValidationContext::new(
                self.params.clone(),
                self.config.network,
                0,
                height,
            );
            let total_mempool = mempool_txs.len();
            let mut included_count = 0usize;
            let mut included_txs: Vec<&Transaction> = Vec::new();
            for tx in &mempool_txs {
                if Instant::now() > deadline {
                    warn!(
                        "Block building deadline reached: including {}/{} mempool TXs",
                        included_count, total_mempool
                    );
                    break;
                }
                if let Err(e) = validation::validate_transaction_with_utxos(tx, &utxo_ctx, &*utxo) {
                    warn!(
                        "Skipping mempool tx {} — UTXO validation failed: {}",
                        tx.hash(),
                        e
                    );
                    continue;
                }
                // Check UniqueIdIndex for NFT/Pool duplicate IDs.
                // validate_transaction_with_utxos doesn't check this — the check
                // only existed in apply_block, causing a fatal mismatch where the
                // builder includes a TX that apply_block rejects, freezing the chain.
                // See: testnet incident 2026-03-25, NFT token_id poisoned mempool.
                let mut unique_conflict = false;
                for output in &tx.outputs {
                    match output.output_type {
                        doli_core::OutputType::NFT => {
                            if let Some((token_id, _)) = output.nft_metadata() {
                                if utxo.has_unique_id(storage::UID_PREFIX_NFT, &token_id) {
                                    warn!(
                                        "Skipping mempool tx {} — NFT token_id {} already exists",
                                        tx.hash(),
                                        token_id.to_hex()
                                    );
                                    unique_conflict = true;
                                    break;
                                }
                            }
                        }
                        doli_core::OutputType::Pool
                            if tx.tx_type == doli_core::TxType::CreatePool =>
                        {
                            if let Some(meta) = output.pool_metadata() {
                                if utxo.has_unique_id(storage::UID_PREFIX_POOL, &meta.pool_id) {
                                    warn!(
                                        "Skipping mempool tx {} — Pool {} already exists",
                                        tx.hash(),
                                        meta.pool_id.to_hex()
                                    );
                                    unique_conflict = true;
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if unique_conflict {
                    continue;
                }
                included_count += 1;
                included_txs.push(tx);
                builder.add_transaction(tx.clone());
            }

            // Calculate extra fees from included user transactions only.
            // Protocol-generated transactions (coinbase, epoch rewards, registrations)
            // are NOT counted — only mempool TXs contribute per-byte fees.
            let extra_fees: u64 = included_txs
                .iter()
                .flat_map(|tx| tx.outputs.iter())
                .map(|o| o.extra_data.len() as u64 * doli_core::consensus::FEE_PER_BYTE)
                .sum();

            // Now add coinbase with block reward + extra fees.
            // insert(0, ...) places it at position 0 regardless of when called.
            builder.add_coinbase_with_extra(height, pool_hash, extra_fees);
        }

        // Recapture timestamp just before building the block.
        // The original `now` can be 1-2 seconds stale due to async work between capture
        // and here (GSet reads, liveness filtering, VDF, etc). If the stale timestamp
        // maps to a different 2s rank window than the one we passed eligibility for,
        // validation rejects our own block as "invalid producer for slot". Using a fresh
        // timestamp ensures the block's header.timestamp matches the rank window we were
        // actually eligible in.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Sanity: abort if we crossed into a different slot during production
        if self.params.timestamp_to_slot(now) != current_slot {
            warn!(
                "Slot boundary crossed during block production (started slot {}, now slot {}) - aborting",
                current_slot,
                self.params.timestamp_to_slot(now)
            );
            return Ok(None);
        }

        // Build attestation bitfield for presence_root.
        // Records which producers attested the current minute (from gossip).
        // 6 blocks per minute from different producers -> union mitigates censorship.
        let current_minute = attestation_minute(current_slot);
        let attested_pks = self.minute_tracker.attested_in_minute(current_minute);
        let presence_root = if attested_pks.is_empty() {
            Hash::ZERO
        } else {
            // Build sorted producer list (same ordering as DeterministicScheduler)
            // Use height-aware method to match the decoding path in calculate_epoch_rewards()
            let sorted_producers: Vec<storage::producer::ProducerInfo> = {
                let producers = self.producer_set.read().await;
                let mut ps: Vec<storage::producer::ProducerInfo> = producers
                    .active_producers_at_height(height)
                    .iter()
                    .map(|p| (*p).clone())
                    .collect();
                ps.sort_by(|a, b| a.public_key.as_bytes().cmp(b.public_key.as_bytes()));
                ps
            };
            // Map attesting pubkeys to sorted indices
            let mut attested_indices = Vec::new();
            for pk in &attested_pks {
                if let Some(idx) = sorted_producers.iter().position(|p| &p.public_key == *pk) {
                    attested_indices.push(idx);
                }
            }
            encode_attestation_bitfield(&attested_indices)
        };
        let builder = builder.with_presence_root(presence_root);

        // Build header + finalized transaction list. The merkle root is computed from
        // exactly these transactions, guaranteeing header-body consistency.
        let (header, transactions) = match builder.build(now) {
            Some(result) => result,
            None => {
                warn!("Failed to build block - slot monotonicity violation");
                return Ok(None);
            }
        };

        Ok(Some((header, transactions)))
    }

    // =========================================================================
    // DRAIN PENDING BLOCKS: Process all queued network events before VDF
    //
    // This is the key fix for the propagation race bug. The problem:
    // - VDF computation takes ~550-700ms
    // - During this time, the event loop is blocked (we await spawn_blocking)
    // - Blocks from other producers may arrive in the network channel queue
    // - Without draining, we don't see them until after we've already produced
    //
    // Solution: Non-blocking drain of all pending NewBlock events from the
    // network channel BEFORE starting VDF. This ensures we have the latest
    // chain state before committing to production.
    //
    // The try_next_event() method uses try_recv() which returns immediately
    // if no events are queued, so this adds negligible latency.
    // =========================================================================

    /// Drain pending network events before VDF computation.
    /// Returns `true` if chain advanced (caller should abort production).
    pub(super) async fn drain_pending_events(&mut self, prev_slot: u32, height: u64) -> bool {
        // First, collect pending events (releases borrow of self.network quickly)
        let pending_events: Vec<NetworkEvent> = {
            if let Some(ref mut network) = self.network {
                let mut events = Vec::new();
                // Drain ALL pending events (not just 10)
                // With 20+ nodes, limiting to 10 can leave blocks unprocessed
                while let Some(event) = network.try_next_event() {
                    events.push(event);
                }
                events
            } else {
                Vec::new()
            }
        };

        // Now process collected events (self.network borrow is released)
        if !pending_events.is_empty() {
            let mut block_count = 0;
            for event in pending_events {
                if matches!(event, NetworkEvent::NewBlock(_, _)) {
                    block_count += 1;
                }
                if let Err(e) = self.handle_network_event(event).await {
                    warn!("Error handling drained event: {}", e);
                }
            }

            if block_count > 0 {
                debug!("Drained {} pending block events before VDF", block_count);

                // Check if chain advanced during drain
                let new_state = self.chain_state.read().await;
                if new_state.best_slot > prev_slot || new_state.best_height >= height {
                    debug!(
                        "Chain advanced during drain (new tip after prev_slot={}) - aborting production",
                        prev_slot
                    );
                    return true; // Chain advanced — abort production
                }
            }
        }

        false // Chain did not advance — continue production
    }

    /// Compute VDF proof for the block header.
    /// Uses hash-chain with dynamically calibrated iterations.
    /// For devnet, VDF is disabled to enable rapid development.
    pub(super) async fn compute_block_vdf(
        &mut self,
        prev_hash: &Hash,
        header: &BlockHeader,
    ) -> Result<(VdfOutput, VdfProof)> {
        if self.config.network.vdf_enabled() {
            // Construct VDF input from block context: prev_hash || tx_root || slot || producer_key
            let vdf_input = construct_vdf_input(
                prev_hash,
                &header.merkle_root,
                header.slot,
                &header.producer,
            );

            // Get network-specific fixed iterations (must match validation)
            let iterations = self.config.network.heartbeat_vdf_iterations();
            info!(
                "Computing hash-chain VDF with {} iterations (network={:?})...",
                iterations, self.config.network
            );

            // Use hash-chain VDF with calibrated iterations
            let vdf_input_clone = vdf_input;
            let vdf_start = std::time::Instant::now();
            let output_bytes =
                tokio::task::spawn_blocking(move || hash_chain_vdf(&vdf_input_clone, iterations))
                    .await
                    .map_err(|e| anyhow::anyhow!("VDF task failed: {}", e))?;
            let vdf_duration = vdf_start.elapsed();

            // Record timing for calibration
            self.vdf_calibrator
                .write()
                .await
                .record_timing(iterations, vdf_duration);

            info!("VDF computed in {:?} (target: ~55ms)", vdf_duration);
            Ok((
                VdfOutput {
                    value: output_bytes.to_vec(),
                },
                VdfProof::empty(), // Hash-chain VDF is self-verifying
            ))
        } else {
            // VDF disabled for this network - use placeholder values
            info!(
                "VDF disabled for {:?} network, using placeholder",
                self.config.network
            );
            Ok((
                VdfOutput {
                    value: prev_hash.as_bytes().to_vec(),
                },
                VdfProof::empty(),
            ))
        }
    }

    /// Aggregate BLS signatures from minute tracker for on-chain proof.
    /// Only includes producers that have BLS sigs for this minute.
    pub(super) fn aggregate_bls_signatures(&self, current_slot: u32) -> Vec<u8> {
        let current_minute = attestation_minute(current_slot);
        let bls_sigs = self.minute_tracker.bls_sigs_for_minute(current_minute);
        info!(
            "BLS aggregate: minute={} sigs_count={} tracker_bls_total={}",
            current_minute,
            bls_sigs.len(),
            self.minute_tracker.bls_sig_count()
        );
        if bls_sigs.is_empty() {
            Vec::new()
        } else {
            let sigs: Vec<crypto::BlsSignature> = bls_sigs
                .iter()
                .filter_map(|(_, raw)| crypto::BlsSignature::try_from_slice(raw).ok())
                .collect();
            if sigs.is_empty() {
                Vec::new()
            } else {
                match crypto::bls_aggregate(&sigs) {
                    Ok(agg) => agg.as_bytes().to_vec(),
                    Err(e) => {
                        warn!("BLS aggregation failed: {}", e);
                        Vec::new()
                    }
                }
            }
        }
    }

    /// Attest our own block for finality gadget + record in minute tracker.
    pub(super) async fn attest_own_block(
        &mut self,
        block_hash: Hash,
        current_slot: u32,
        height: u64,
    ) {
        self.create_and_broadcast_attestation(block_hash, current_slot, height)
            .await;
        if let Some(ref kp) = self.producer_key {
            let minute = attestation_minute(current_slot);
            if let Some(ref bls_kp) = self.bls_key {
                let bls_msg = crypto::attestation_message(&block_hash, current_slot);
                let bls_sig = crypto::bls_sign(&bls_msg, bls_kp.secret_key())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();
                self.minute_tracker
                    .record_with_bls(*kp.public_key(), minute, bls_sig);
            } else {
                self.minute_tracker.record(*kp.public_key(), minute);
            }
        }
    }
}
