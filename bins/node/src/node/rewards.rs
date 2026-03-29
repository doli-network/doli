use super::*;

impl Node {
    /// Calculate epoch rewards using on-chain attestation bitfield qualification.
    ///
    /// Scans blocks in the completed epoch, decodes each block's `presence_root`
    /// bitfield, and counts unique attestation minutes per producer. Producers
    /// with ≥54/60 minutes qualify. Pool is distributed bond-weighted among qualifiers.
    ///
    /// Formula: reward[i] = pool × bonds[i] / Σ(qualifying_bonds)
    /// Non-qualifiers' share is redistributed to qualifiers (not burned).
    ///
    /// Returns a vector of (amount, pubkey_hash) tuples for the EpochReward transaction.
    pub async fn calculate_epoch_rewards(&self, epoch: u64) -> Vec<(u64, Hash)> {
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let epoch_start_height = epoch * blocks_per_epoch;
        let epoch_end_height = (epoch + 1) * blocks_per_epoch;

        // Get active producers at epoch start, sorted by pubkey (same order as bitfield encoding)
        let sorted_producers: Vec<storage::producer::ProducerInfo> = {
            let producers = self.producer_set.read().await;
            let mut ps: Vec<storage::producer::ProducerInfo> = producers
                .active_producers_at_height(epoch_start_height)
                .iter()
                .map(|p| (*p).clone())
                .collect();
            ps.sort_by(|a, b| a.public_key.as_bytes().cmp(b.public_key.as_bytes()));
            ps
        };

        if sorted_producers.is_empty() {
            return Vec::new();
        }

        let producer_count = sorted_producers.len();

        // Scan all blocks in epoch, decode presence_root bitfield, track attested minutes per producer
        // Key: producer index → set of attestation minutes they were attested in
        let mut attested_minutes: HashMap<usize, HashSet<u32>> = HashMap::new();

        for h in epoch_start_height..epoch_end_height {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(h) {
                // Skip blocks with zero presence_root (no attestation data)
                if block.header.presence_root.is_zero() {
                    continue;
                }

                let minute = attestation_minute(block.header.slot);
                let indices =
                    decode_attestation_bitfield(&block.header.presence_root, producer_count);

                // Union: for each producer index attested in this block, add the minute
                for idx in indices {
                    attested_minutes.entry(idx).or_default().insert(minute);
                }
            }
        }

        // Qualify producers: attested in ≥ ATTESTATION_QUALIFICATION_THRESHOLD minutes
        // Genesis epoch (epoch 0): all active producers qualify — no attestation data exists yet
        //
        // Never-burn fallback tiers:
        //   Tier 1: 90% threshold (54/60 minutes)
        //   Tier 2: 80% of median attendance (floor of 1 minute)
        //   Tier 3: All producers have 0 attendance — pool accumulates to next epoch
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let threshold =
            doli_core::attestation::attestation_qualification_threshold(blocks_per_epoch);
        let qualified: Vec<&storage::producer::ProducerInfo> = if epoch == 0 {
            info!("Epoch 0 (genesis): all active producers qualify for rewards");
            sorted_producers.iter().collect()
        } else {
            // Tier 1: standard 90% threshold
            let tier1: Vec<&storage::producer::ProducerInfo> = sorted_producers
                .iter()
                .enumerate()
                .filter(|(idx, _)| {
                    let minutes = attested_minutes.get(idx).map(|s| s.len()).unwrap_or(0);
                    minutes as u32 >= threshold
                })
                .map(|(_, p)| p)
                .collect();

            if !tier1.is_empty() {
                tier1
            } else {
                // Tier 2: fallback — 80% of median attendance, floor of 1 minute
                let mut all_minutes: Vec<u32> = sorted_producers
                    .iter()
                    .enumerate()
                    .map(|(idx, _)| {
                        attested_minutes
                            .get(&idx)
                            .map(|s| s.len() as u32)
                            .unwrap_or(0)
                    })
                    .collect();
                all_minutes.sort();

                let median = if all_minutes.is_empty() {
                    0
                } else {
                    let mid = all_minutes.len() / 2;
                    if all_minutes.len().is_multiple_of(2) {
                        (all_minutes[mid - 1] + all_minutes[mid]) / 2
                    } else {
                        all_minutes[mid]
                    }
                };

                let fallback_threshold = (median * 80 / 100).max(1);

                // Tier 3: if all producers have 0 attendance, accumulate
                let max_attendance = all_minutes.last().copied().unwrap_or(0);
                if max_attendance == 0 {
                    warn!(
                        "Epoch {}: all producers have 0 attendance — pool accumulates to next epoch",
                        epoch
                    );
                    return Vec::new();
                }

                warn!(
                    "Epoch {}: no producers met 90% threshold, using fallback: median={}, threshold={}",
                    epoch, median, fallback_threshold
                );

                sorted_producers
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| {
                        let minutes = attested_minutes.get(idx).map(|s| s.len()).unwrap_or(0);
                        minutes as u32 >= fallback_threshold
                    })
                    .map(|(_, p)| p)
                    .collect()
            }
        };

        if qualified.is_empty() {
            warn!(
                "Epoch {}: no producers qualified — pool accumulates to next epoch",
                epoch
            );
            return Vec::new();
        }

        let disqualified_count = sorted_producers.len() - qualified.len();
        if disqualified_count > 0 {
            info!(
                "Epoch {}: {}/{} producers qualified, {} disqualified (rewards redistributed)",
                epoch,
                qualified.len(),
                sorted_producers.len(),
                disqualified_count
            );
        }

        // Sum qualifying bonds using epoch-locked snapshot (deterministic across all nodes).
        // CRITICAL: Do NOT use selection_weight() here — it reads live bond_count which can
        // differ between nodes after mid-epoch withdrawals (N5 testnet incident 2026-03-26).
        // The epoch_bond_snapshot is computed from the UTXO set at the epoch boundary,
        // identical on all nodes.
        let bond_for = |p: &storage::producer::ProducerInfo| -> u64 {
            let pkh = hash_with_domain(ADDRESS_DOMAIN, p.public_key.as_bytes());
            self.epoch_bond_snapshot.get(&pkh).copied().unwrap_or(1)
        };
        let qualifying_bonds: u64 = qualified.iter().map(|p| bond_for(p)).sum();

        if qualifying_bonds == 0 {
            return Vec::new();
        }

        // Calculate total pool from accumulated coinbase UTXOs in the reward pool.
        // Include current block's coinbase (not yet in UTXO set during block production)
        // so the distributed amount matches what the consume step will remove.
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let pool = {
            let utxo = self.utxo_set.read().await;
            let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
            let utxo_total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
            utxo_total + self.params.block_reward(epoch_end_height)
        };
        info!(
            "Epoch {} reward pool: {} units from accumulated coinbase",
            epoch, pool
        );

        // Bond-weighted distribution among qualifiers: pool × bonds[i] / qualifying_bonds
        // Uses u128 intermediates to prevent overflow
        let mut reward_outputs = Vec::new();
        let mut distributed: u64 = 0;

        for producer_info in &qualified {
            let bonds = bond_for(producer_info);
            let reward = (pool as u128 * bonds as u128 / qualifying_bonds as u128) as u64;

            if reward == 0 {
                continue;
            }

            let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, producer_info.public_key.as_bytes());

            // Find this producer's index in sorted list for attestation minute count
            let att_minutes = sorted_producers
                .iter()
                .position(|p| p.public_key == producer_info.public_key)
                .and_then(|idx| attested_minutes.get(&idx))
                .map(|s| s.len())
                .unwrap_or(0);

            info!(
                "Producer {} earned {} in epoch {} (attested: {}/60 minutes, bonds: {})",
                producer_info.public_key, reward, epoch, att_minutes, bonds
            );

            // Split rewards if producer has received delegations
            if producer_info.received_delegations.is_empty() {
                reward_outputs.push((reward, pubkey_hash));
            } else {
                let utxo = self.utxo_set.read().await;
                let own_bonds = utxo
                    .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                    .max(1) as u64;
                drop(utxo);
                let delegated: u64 = producer_info
                    .received_delegations
                    .iter()
                    .map(|(_, c)| *c as u64)
                    .sum();
                let total_bonds = own_bonds + delegated;

                let own_share = reward * own_bonds / total_bonds;
                let delegated_share = reward - own_share;
                let delegate_fee = delegated_share * DELEGATE_REWARD_PCT as u64 / 100;
                let staker_pool = delegated_share - delegate_fee; // remainder to stakers, no dust

                // Distribute staker pool to delegators, last gets remainder
                let mut staker_distributed = 0u64;
                let delegators: Vec<_> = producer_info.received_delegations.iter().collect();
                for (i, (delegator_hash, bond_count)) in delegators.iter().enumerate() {
                    let delegator_reward = if i == delegators.len() - 1 {
                        staker_pool - staker_distributed // last delegator gets remainder
                    } else {
                        staker_pool * (*bond_count as u64) / delegated
                    };
                    staker_distributed += delegator_reward;
                    if delegator_reward > 0 {
                        reward_outputs.push((delegator_reward, *delegator_hash));
                    }
                }

                let producer_total = own_share + delegate_fee;
                if producer_total > 0 {
                    reward_outputs.push((producer_total, pubkey_hash));
                }
            }

            distributed += reward;
        }

        // Integer division remainder goes to first qualifier
        let remainder = pool.saturating_sub(distributed);
        if remainder > 0 && !reward_outputs.is_empty() {
            reward_outputs[0].0 += remainder;
        }

        reward_outputs
    }

    // NOTE: build_presence_commitment removed in deterministic scheduler model
    // Rewards go 100% to block producer via coinbase, no presence tracking needed

    /// Handle a detected equivocation (double signing)
    ///
    /// Creates and submits a slash transaction to the mempool when a producer
    /// is caught creating two different blocks for the same slot.
    pub async fn handle_equivocation(&mut self, proof: EquivocationProof) {
        // Log the equivocation with all details
        warn!(
            "SLASHING: Producer {} created two blocks for slot {}: {} and {}",
            crypto_hash(proof.producer.as_bytes()),
            proof.slot,
            proof.block_header_1.hash(),
            proof.block_header_2.hash()
        );

        // Check if the producer is actually registered (to avoid spam for unknown producers)
        let is_registered = {
            let producers = self.producer_set.read().await;
            producers.get_by_pubkey(&proof.producer).is_some()
        };

        if !is_registered {
            info!(
                "Equivocation by unregistered producer {} - not submitting slash tx",
                crypto_hash(proof.producer.as_bytes())
            );
            return;
        }

        // Create slash transaction using our producer key as reporter
        // If we don't have a producer key, we can't sign the slash tx
        let slash_tx = if let Some(ref reporter_key) = self.producer_key {
            proof.to_slash_transaction(reporter_key)
        } else {
            // Generate ephemeral keypair for reporting (anyone can report)
            let reporter_key = KeyPair::generate();
            proof.to_slash_transaction(&reporter_key)
        };

        let slash_tx_hash = slash_tx.hash();

        // Add to mempool for inclusion in next block
        // Use add_system_transaction since slash txs have no inputs/outputs
        let current_height = self.chain_state.read().await.best_height;
        let add_result = {
            let mut mempool = self.mempool.write().await;
            mempool.add_system_transaction(slash_tx.clone(), current_height)
        };

        match add_result {
            Ok(_hash) => {
                info!(
                    "Slash transaction {} submitted to mempool for producer {}",
                    slash_tx_hash,
                    crypto_hash(proof.producer.as_bytes())
                );

                // Broadcast the slash tx to the network
                if let Some(ref network) = self.network {
                    if let Err(e) = network.broadcast_transaction(slash_tx).await {
                        warn!("Failed to broadcast slash transaction: {}", e);
                    }
                }
            }
            Err(rpc::MempoolError::AlreadyExists) => {
                debug!("Slash transaction {} already in mempool", slash_tx_hash);
            }
            Err(e) => {
                warn!(
                    "Failed to add slash transaction {} to mempool: {}",
                    slash_tx_hash, e
                );
            }
        }
    }

    /// Rebuild the producer set by replaying all producer-modifying transactions
    /// from blocks 1 through `target_height`. Called by rollback/reorg paths.
    ///
    /// Processes: Registration, Exit, SlashProducer, AddBond, DelegateBond,
    /// RevokeDelegation, and unbonding transitions — mirroring `apply_block()`.
    /// Rebuild producer liveness map from canonical block_store.
    ///
    /// Scans the last LIVENESS_WINDOW_MIN blocks to determine which producers
    /// have been active recently. Must be called after any rollback to prevent
    /// divergent liveness views between nodes (fork block entries pollute the map).
    pub fn rebuild_producer_liveness(&mut self, tip_height: u64) {
        let window = consensus::LIVENESS_WINDOW_MIN;
        let start = tip_height.saturating_sub(window).max(1);
        self.producer_liveness.clear();
        for h in start..=tip_height {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(h) {
                self.producer_liveness.insert(block.header.producer, h);
            }
        }
        info!(
            "Rebuilt producer liveness after rollback from blocks {}-{}: {} producers tracked",
            start,
            tip_height,
            self.producer_liveness.len()
        );
    }

    /// Rebuild excluded_producers from block headers since epoch start.
    ///
    /// Reads `missed_producers` from each block header (on-chain source of truth)
    /// and `presence_root` for re-inclusions. Deterministic: all nodes with the
    /// same chain compute the same set.
    ///
    /// Must be called after any rollback to ensure the exclusion set matches
    /// the canonical chain.
    pub async fn rebuild_excluded_from_headers(&mut self) {
        let current_h = self.chain_state.read().await.best_height;
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let epoch = current_h / blocks_per_epoch;
        let epoch_start = epoch * blocks_per_epoch;
        let start_h = epoch_start.max(1);

        let mut excluded: HashSet<PublicKey> = HashSet::new();

        // Get sorted active producers for presence_root decoding
        let sorted_pks: Vec<PublicKey> = {
            let ps = self.producer_set.read().await;
            let mut pks: Vec<PublicKey> = ps
                .active_producers_at_height(current_h)
                .iter()
                .map(|p| p.public_key)
                .collect();
            pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            pks
        };

        for h in start_h..=current_h {
            if let Ok(Some(blk)) = self.block_store.get_block_by_height(h) {
                // EXCLUDE: from on-chain missed_producers header field
                for pk in &blk.header.missed_producers {
                    excluded.insert(*pk);
                }

                // RE-INCLUDE: from on-chain presence_root attestation bitfield
                if !blk.header.presence_root.is_zero() {
                    let indices =
                        decode_attestation_bitfield(&blk.header.presence_root, sorted_pks.len());
                    for idx in indices {
                        if let Some(pk) = sorted_pks.get(idx) {
                            excluded.remove(pk);
                        }
                    }
                }
            }
        }

        let old_count = self.excluded_producers.len();
        self.excluded_producers = excluded;
        let new_count = self.excluded_producers.len();

        if old_count > 0 || new_count > 0 {
            info!(
                "[LIVENESS] Rebuilt excluded set from headers (h={}-{}): {} excluded (was {})",
                start_h, current_h, new_count, old_count
            );
        }
    }

    pub fn rebuild_producer_set_from_blocks(
        &self,
        producers: &mut ProducerSet,
        target_height: u64,
    ) -> Result<()> {
        producers.clear();
        let bond_unit = self.config.network.bond_unit();
        let genesis_blocks = self.config.network.genesis_blocks();
        let epoch_len = self.config.network.blocks_per_reward_epoch();

        for height in 1..=target_height {
            let block = self
                .block_store
                .get_block_by_height(height)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Producer set rebuild: missing block at height {} (store corrupted)",
                        height
                    )
                })?;
            for tx in &block.transactions {
                match tx.tx_type {
                    TxType::Registration => {
                        // During genesis, Registration TXs are VDF proof containers
                        // (zero-bond). Skip — genesis producers are registered below
                        // when crossing the genesis boundary.
                        if genesis_blocks > 0 && height <= genesis_blocks {
                            continue;
                        }
                        if let Some(reg_data) = tx.registration_data() {
                            let bond_outputs: Vec<(usize, &doli_core::transaction::Output)> = tx
                                .outputs
                                .iter()
                                .enumerate()
                                .filter(|(_, o)| {
                                    o.output_type == doli_core::transaction::OutputType::Bond
                                })
                                .collect();

                            if let Some(&(bond_index, _)) = bond_outputs.first() {
                                let tx_hash = tx.hash();
                                let era = self.params.height_to_era(height);
                                let total_bond_amount: u64 =
                                    bond_outputs.iter().map(|(_, o)| o.amount).sum();
                                let mut producer_info = storage::ProducerInfo::new_with_bonds(
                                    reg_data.public_key,
                                    height,
                                    total_bond_amount,
                                    (tx_hash, bond_index as u32),
                                    era,
                                    reg_data.bond_count,
                                );
                                producer_info.bls_pubkey = reg_data.bls_pubkey.clone();
                                producers.queue_update(PendingProducerUpdate::Register {
                                    info: Box::new(producer_info),
                                    height,
                                });
                            }
                        }
                    }
                    TxType::Exit => {
                        if let Some(exit_data) = tx.exit_data() {
                            // Convert Exit to RequestWithdrawal for all bonds
                            if let Some(info) = producers.get_by_pubkey(&exit_data.public_key) {
                                let all_bonds = info.bond_count;
                                if all_bonds > 0 {
                                    if let Some(producer) =
                                        producers.get_by_pubkey_mut(&exit_data.public_key)
                                    {
                                        producer.withdrawal_pending_count += all_bonds;
                                    }
                                    producers.queue_update(
                                        PendingProducerUpdate::RequestWithdrawal {
                                            pubkey: exit_data.public_key,
                                            bond_count: all_bonds,
                                            bond_unit,
                                        },
                                    );
                                } else {
                                    producers.queue_update(PendingProducerUpdate::Exit {
                                        pubkey: exit_data.public_key,
                                        height,
                                    });
                                }
                            } else {
                                producers.queue_update(PendingProducerUpdate::Exit {
                                    pubkey: exit_data.public_key,
                                    height,
                                });
                            }
                        }
                    }
                    TxType::SlashProducer => {
                        if let Some(slash_data) = tx.slash_data() {
                            producers.queue_update(PendingProducerUpdate::Slash {
                                pubkey: slash_data.producer_pubkey,
                                height,
                            });
                        }
                    }
                    TxType::AddBond => {
                        if let Some(add_bond_data) = tx.add_bond_data() {
                            // Guard: skip if producer not registered (orphan Bond UTXO)
                            let pubkey = &add_bond_data.producer_pubkey;
                            let is_registered = producers.get_by_pubkey(pubkey).is_some();
                            let has_pending_reg = producers
                                .pending_updates_for(pubkey)
                                .iter()
                                .any(|u| matches!(u, PendingProducerUpdate::Register { .. }));

                            if is_registered || has_pending_reg {
                                let tx_hash = tx.hash();
                                // Lock/unlock: Bond output indices from actual TX outputs
                                let bond_outpoints: Vec<(crypto::Hash, u32)> = tx
                                    .outputs
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, o)| {
                                        o.output_type == doli_core::transaction::OutputType::Bond
                                    })
                                    .map(|(i, _)| (tx_hash, i as u32))
                                    .collect();
                                producers.queue_update(PendingProducerUpdate::AddBond {
                                    pubkey: add_bond_data.producer_pubkey,
                                    outpoints: bond_outpoints,
                                    bond_unit,
                                    creation_slot: block.header.slot,
                                });
                            }
                        }
                    }
                    TxType::RequestWithdrawal => {
                        if let Some(data) = tx.withdrawal_request_data() {
                            // During rebuild, validate and queue the withdrawal
                            if let Some(info) = producers.get_by_pubkey(&data.producer_pubkey) {
                                let available = info
                                    .bond_count
                                    .saturating_sub(info.withdrawal_pending_count);
                                if data.bond_count <= available {
                                    if let Some(producer) =
                                        producers.get_by_pubkey_mut(&data.producer_pubkey)
                                    {
                                        producer.withdrawal_pending_count += data.bond_count;
                                    }
                                    producers.queue_update(
                                        PendingProducerUpdate::RequestWithdrawal {
                                            pubkey: data.producer_pubkey,
                                            bond_count: data.bond_count,
                                            bond_unit,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    TxType::DelegateBond => {
                        if let Some(data) = tx.delegate_bond_data() {
                            producers.queue_update(PendingProducerUpdate::DelegateBond {
                                delegator: data.delegator,
                                delegate: data.delegate,
                                bond_count: data.bond_count,
                            });
                        }
                    }
                    TxType::RevokeDelegation => {
                        if let Some(data) = tx.revoke_delegation_data() {
                            producers.queue_update(PendingProducerUpdate::RevokeDelegation {
                                delegator: data.delegator,
                            });
                        }
                    }
                    // ProtocolActivation doesn't modify the producer set —
                    // it's processed in apply_block where chain_state is available.
                    _ => {}
                }
            }

            // Replicate GENESIS PHASE COMPLETE: register VDF-proven producers
            // when crossing the genesis boundary during rebuild.
            // Genesis producers are registered immediately (not deferred).
            if genesis_blocks > 0 && height == genesis_blocks + 1 {
                let genesis_producers = self.derive_genesis_producers_from_chain();
                let genesis_bls = self.genesis_bls_pubkeys();
                let era = self.params.height_to_era(height);
                for pubkey in &genesis_producers {
                    let bond_hash = hash_with_domain(b"genesis_bond", pubkey.as_bytes());
                    // registered_at = 0: Genesis producers exempt from ACTIVATION_DELAY
                    let mut producer_info = storage::ProducerInfo::new_with_bonds(
                        *pubkey,
                        0,
                        bond_unit,
                        (bond_hash, 0),
                        era,
                        1,
                    );
                    if let Some(bls) = genesis_bls.get(pubkey) {
                        producer_info.bls_pubkey = bls.clone();
                    }
                    let _ = producers.register(producer_info, height);
                }
            }

            // Apply deferred updates: every block in epoch 0, then at epoch boundaries
            let is_epoch_0 = height < epoch_len;
            let is_boundary = height > 0 && height.is_multiple_of(epoch_len);
            if is_epoch_0 || is_boundary {
                producers.apply_pending_updates();
            }

            // Process completed unbonding periods after each block
            producers.process_unbonding(height, UNBONDING_PERIOD);
        }

        // DO NOT apply remaining pending_updates here. If target_height doesn't
        // land on an epoch boundary, pending updates must stay deferred — they will
        // be applied when apply_block() processes the next epoch boundary. Applying
        // them early produces a producer set that never existed on-chain, causing
        // "invalid producer for slot" failures during reorg block validation.

        Ok(())
    }
}
