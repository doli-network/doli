use super::*;

impl Node {
    /// Apply a block to the chain
    ///
    /// `mode`: `Full` for gossip/production (checks MAX_PAST_SLOTS, VDF),
    ///         `Light` for sync/reorg (skips time-based checks that reject old blocks).
    pub(super) async fn apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()> {
        let block_hash = block.hash();

        let height = self.chain_state.read().await.best_height + 1;

        // Defense-in-depth: skip blocks we already have AND have applied.
        // Prevents height double-counting if sync delivers stored blocks (ISSUE-5).
        //
        // Critical: We must check that the block was actually APPLIED (chain state
        // advanced past it), not just stored. A block can be in the store but not
        // applied if a previous apply_block() wrote the block then failed during
        // UTXO validation (block store poisoning — see N4 incident 2026-03-13).
        // In that case, we allow re-apply — put_block() will overwrite harmlessly.
        if self.block_store.has_block(&block_hash)? {
            if let Some(stored_height) = self.block_store.get_height_by_hash(&block_hash)? {
                if stored_height < height {
                    // Block is in store but chain state didn't advance past it.
                    // This is a poisoned block from a failed apply — proceed with
                    // re-validation and re-apply (put_block overwrites safely).
                    warn!(
                        "Block {} in store at height {} but chain at height {} — \
                         poisoned block, re-applying",
                        block_hash,
                        stored_height,
                        height - 1
                    );
                } else {
                    // Block is in store AND chain state is at or past it — truly applied.
                    debug!(
                        "Block {} already applied at height {}, skipping",
                        block_hash, stored_height
                    );
                    return Ok(());
                }
            } else {
                // Block in store but no height index — proceed with apply.
                warn!(
                    "Block {} in store but no height index — re-applying",
                    block_hash
                );
            }
        }

        self.validate_block_for_apply(&block, height, mode).await?;
        self.validate_block_economics(&block, height, mode).await?;

        info!("Applying block {} at height {}", block_hash, height);

        // NOTE: Block store write is deferred to AFTER all transaction validation.
        // Writing here would poison the store if UTXO validation fails later —
        // the "already in store" check would then skip the block forever.
        // See: N4 stuck-at-22888 incident (2026-03-13).

        // Update producer liveness tracker (for scheduling filter)
        self.producer_liveness.insert(block.header.producer, height);

        // Track newly registered producers to add to known_producers after lock release
        let mut new_registrations: Vec<PublicKey> = Vec::new();

        // Deferred protocol activation (verified in tx loop, applied when chain_state lock acquired)
        let mut pending_protocol_activation_data: Option<(u32, u64)> = None;

        // Create BlockBatch for atomic persistence (all state changes in one WriteBatch)
        let mut batch = self.state_db.begin_batch();

        // Track dirty producers for efficient batch writes
        let mut dirty_producer_keys: std::collections::HashSet<Hash> =
            std::collections::HashSet::new();
        let removed_producer_keys: std::collections::HashSet<Hash> =
            std::collections::HashSet::new();
        let dirty_exit_keys: std::collections::HashSet<Hash> = std::collections::HashSet::new();

        // Undo log: snapshot ProducerSet BEFORE any mutations
        let undo_producer_snapshot = {
            let producers = self.producer_set.read().await;
            bincode::serialize(&*producers).unwrap_or_default()
        };

        // Undo log: track UTXO changes for this block
        let mut undo_spent_utxos: Vec<(storage::Outpoint, storage::UtxoEntry)> = Vec::new();
        let mut undo_created_utxos: Vec<storage::Outpoint> = Vec::new();

        // Apply transactions to UTXO set and process special transactions
        {
            let mut utxo = self.utxo_set.write().await;
            let mut producers = self.producer_set.write().await;

            for (tx_index, tx) in block.transactions.iter().enumerate() {
                // Apply UTXO changes (in-memory for reads + batch for atomic persistence)
                let is_reward_tx = tx_index == 0 && tx.is_reward_minting();

                // Undo log: capture UTXOs BEFORE spending them
                if !is_reward_tx {
                    for input in &tx.inputs {
                        let outpoint =
                            storage::Outpoint::new(input.prev_tx_hash, input.output_index);
                        if let Some(entry) = utxo.get(&outpoint) {
                            undo_spent_utxos.push((outpoint, entry));
                        }
                    }
                }

                // EpochReward TX: consume all pool UTXOs (the pool collected per-block coinbase)
                if tx.tx_type == TxType::EpochReward {
                    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
                    let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                    for (outpoint, entry) in &pool_utxos {
                        undo_spent_utxos.push((*outpoint, entry.clone()));
                        utxo.remove(outpoint)?;
                        let _ = batch.spend_utxo(outpoint);
                    }
                    if !pool_utxos.is_empty() {
                        let total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
                        info!(
                            "Consumed {} pool UTXOs ({} units) for epoch reward distribution",
                            pool_utxos.len(),
                            total
                        );
                    }
                }

                // Validate UTXO-level spending conditions (signatures, covenant evaluation,
                // lock times, balance). Coinbase/EpochReward skip internally.
                if !is_reward_tx {
                    let utxo_ctx = validation::ValidationContext::new(
                        ConsensusParams::for_network(self.config.network),
                        self.config.network,
                        0, // wall-clock not needed for UTXO validation
                        height,
                    );
                    validation::validate_transaction_with_utxos(tx, &utxo_ctx, &*utxo).map_err(
                        |e| anyhow::anyhow!("UTXO validation failed for tx {}: {}", tx.hash(), e),
                    )?;
                }

                let _ = utxo.spend_transaction(tx); // In-memory
                utxo.add_transaction(tx, height, is_reward_tx, block.header.slot)?; // In-memory

                // Undo log: track created UTXOs
                let tx_hash = tx.hash();
                for (index, _) in tx.outputs.iter().enumerate() {
                    undo_created_utxos.push(storage::Outpoint::new(tx_hash, index as u32));
                }

                // Batch: track the same UTXO changes for atomic persistence
                if !is_reward_tx {
                    let _ = batch.spend_transaction_utxos(tx);
                }
                batch.add_transaction_utxos(tx, height, is_reward_tx, block.header.slot);

                // Process registration transactions — deferred to epoch boundary
                if tx.tx_type == TxType::Registration {
                    // During genesis, Registration TXs are VDF proof containers only.
                    // Actual producer registration happens at GENESIS PHASE COMPLETE (block 361).
                    if self.config.network.is_in_genesis(height) {
                        if let Some(reg_data) = tx.registration_data() {
                            info!(
                                "Genesis VDF proof submitted by {} (block {})",
                                hex::encode(&reg_data.public_key.as_bytes()[..8]),
                                height
                            );
                        }
                        // Skip immediate registration — handled at genesis end
                    } else if let Some(reg_data) = tx.registration_data() {
                        // Find all bond outputs and use the first as the primary outpoint
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
                            info!(
                                "Queued producer registration {} at height {} (deferred to epoch boundary)",
                                crypto_hash(reg_data.public_key.as_bytes()),
                                height
                            );
                            // Track for known_producers update after lock release
                            new_registrations.push(reg_data.public_key);
                        }
                    }
                }

                // Process exit transactions — withdraw ALL bonds (deferred to epoch boundary)
                if tx.tx_type == TxType::Exit {
                    if let Some(exit_data) = tx.exit_data() {
                        if let Some(info) = producers.get_by_pubkey(&exit_data.public_key) {
                            let all_bonds = info.bond_count;
                            if all_bonds > 0 {
                                let bond_unit = self.config.network.bond_unit();
                                // Mark all bonds as pending withdrawal
                                if let Some(producer) =
                                    producers.get_by_pubkey_mut(&exit_data.public_key)
                                {
                                    producer.withdrawal_pending_count += all_bonds;
                                    dirty_producer_keys
                                        .insert(crypto_hash(exit_data.public_key.as_bytes()));
                                }
                                producers.queue_update(PendingProducerUpdate::RequestWithdrawal {
                                    pubkey: exit_data.public_key,
                                    bond_count: all_bonds,
                                    bond_unit,
                                });
                                info!(
                                    "Queued exit (withdraw all {} bonds) for producer {} at height {} (deferred to epoch boundary)",
                                    all_bonds,
                                    crypto_hash(exit_data.public_key.as_bytes()),
                                    height
                                );
                            } else {
                                // No bonds — just mark as exited directly
                                producers.queue_update(PendingProducerUpdate::Exit {
                                    pubkey: exit_data.public_key,
                                    height,
                                });
                            }
                        } else {
                            warn!("Exit: producer not found");
                        }
                    }
                }

                // Process slash transactions — deferred to epoch boundary
                if tx.tx_type == TxType::SlashProducer {
                    if let Some(slash_data) = tx.slash_data() {
                        producers.queue_update(PendingProducerUpdate::Slash {
                            pubkey: slash_data.producer_pubkey,
                            height,
                        });
                        warn!(
                            "Queued SLASH for producer {} at height {} (deferred to epoch boundary)",
                            crypto_hash(slash_data.producer_pubkey.as_bytes()),
                            height
                        );
                    }
                }

                // Process AddBond transactions — deferred to epoch boundary
                if tx.tx_type == TxType::AddBond {
                    if let Some(add_bond_data) = tx.add_bond_data() {
                        // Guard: producer must be registered (or have a pending registration)
                        let pubkey = &add_bond_data.producer_pubkey;
                        let is_registered = producers.get_by_pubkey(pubkey).is_some();
                        let has_pending_reg = producers
                            .pending_updates_for(pubkey)
                            .iter()
                            .any(|u| matches!(u, PendingProducerUpdate::Register { .. }));

                        if !is_registered && !has_pending_reg {
                            warn!(
                                "AddBond ignored: producer {} is not registered (Bond UTXO orphaned at height {})",
                                crypto_hash(pubkey.as_bytes()),
                                height
                            );
                        } else {
                            let tx_hash = tx.hash();
                            let bond_count = add_bond_data.bond_count;

                            // Lock/unlock: Bond output is a real UTXO in the UTXO set.
                            // Find Bond output indices for outpoint references.
                            let bond_outpoints: Vec<(crypto::Hash, u32)> = tx
                                .outputs
                                .iter()
                                .enumerate()
                                .filter(|(_, o)| {
                                    o.output_type == doli_core::transaction::OutputType::Bond
                                })
                                .map(|(i, _)| (tx_hash, i as u32))
                                .collect();

                            let bond_unit = self.config.network.bond_unit();
                            producers.queue_update(PendingProducerUpdate::AddBond {
                                pubkey: add_bond_data.producer_pubkey,
                                outpoints: bond_outpoints,
                                bond_unit,
                                creation_slot: block.header.slot,
                            });
                            info!(
                                "Queued AddBond ({} bonds, lock/unlock) for producer {} at height {} (deferred to epoch boundary)",
                                bond_count,
                                crypto_hash(pubkey.as_bytes()),
                                height
                            );
                        }
                    }
                }

                // Process WithdrawalRequest transactions — deferred to epoch boundary
                if tx.tx_type == TxType::RequestWithdrawal {
                    if let Some(data) = tx.withdrawal_request_data() {
                        // Validate: producer exists and has enough bonds
                        if let Some(info) = producers.get_by_pubkey(&data.producer_pubkey) {
                            let available = info
                                .bond_count
                                .saturating_sub(info.withdrawal_pending_count);
                            if data.bond_count > available {
                                warn!(
                                    "WithdrawalRequest: not enough bonds (requested {}, available {})",
                                    data.bond_count, available
                                );
                            } else {
                                // Validate: output amount <= FIFO net calculation (from UTXO bonds)
                                let pubkey_hash = hash_with_domain(
                                    ADDRESS_DOMAIN,
                                    data.producer_pubkey.as_bytes(),
                                );
                                let bond_entries = utxo.get_bond_entries(&pubkey_hash);
                                let expected = storage::producer::calculate_withdrawal_from_bonds(
                                    &bond_entries,
                                    data.bond_count,
                                    block.header.slot,
                                    self.config.network.params().vesting_quarter_slots,
                                );
                                let tx_output = tx.outputs.first().map(|o| o.amount).unwrap_or(0);
                                if let Some((expected_net, _penalty)) = expected {
                                    if tx_output > expected_net {
                                        warn!(
                                            "WithdrawalRequest: output {} exceeds FIFO net {}",
                                            tx_output, expected_net
                                        );
                                    } else {
                                        // Increment pending count (prevents double-withdrawal in same epoch)
                                        if let Some(producer) =
                                            producers.get_by_pubkey_mut(&data.producer_pubkey)
                                        {
                                            producer.withdrawal_pending_count += data.bond_count;
                                            // Track as dirty for atomic batch write
                                            dirty_producer_keys.insert(crypto_hash(
                                                data.producer_pubkey.as_bytes(),
                                            ));
                                        }

                                        let bond_unit = self.config.network.bond_unit();
                                        producers.queue_update(
                                            PendingProducerUpdate::RequestWithdrawal {
                                                pubkey: data.producer_pubkey,
                                                bond_count: data.bond_count,
                                                bond_unit,
                                            },
                                        );
                                        info!(
                                            "Queued WithdrawalRequest ({} bonds) for producer {} at height {} (deferred to epoch boundary)",
                                            data.bond_count,
                                            crypto_hash(data.producer_pubkey.as_bytes()),
                                            height
                                        );
                                    }
                                } else {
                                    warn!(
                                        "WithdrawalRequest: FIFO calculation failed for producer"
                                    );
                                }
                            }
                        } else {
                            warn!("WithdrawalRequest: producer not found");
                        }
                    }
                }

                // Process DelegateBond transactions — deferred to epoch boundary
                if tx.tx_type == TxType::DelegateBond {
                    if let Some(data) = tx.delegate_bond_data() {
                        producers.queue_update(PendingProducerUpdate::DelegateBond {
                            delegator: data.delegator,
                            delegate: data.delegate,
                            bond_count: data.bond_count,
                        });
                        info!(
                            "Queued DelegateBond ({} bonds) from {} to {} at height {} (deferred to epoch boundary)",
                            data.bond_count,
                            crypto_hash(data.delegator.as_bytes()),
                            crypto_hash(data.delegate.as_bytes()),
                            height
                        );
                    }
                }

                // Process RevokeDelegation transactions — deferred to epoch boundary
                if tx.tx_type == TxType::RevokeDelegation {
                    if let Some(data) = tx.revoke_delegation_data() {
                        producers.queue_update(PendingProducerUpdate::RevokeDelegation {
                            delegator: data.delegator,
                        });
                        info!(
                            "Queued RevokeDelegation for {} at height {} (deferred to epoch boundary)",
                            crypto_hash(data.delegator.as_bytes()),
                            height
                        );
                    }
                }

                // Process MaintainerAdd transactions — applied immediately (governance, not epoch-deferred)
                if tx.tx_type == TxType::AddMaintainer {
                    if let Some(maintainer_state) = &self.maintainer_state {
                        if let Some(data) =
                            doli_core::maintainer::MaintainerChangeData::from_bytes(&tx.extra_data)
                        {
                            let mut ms = maintainer_state.write().await;
                            let message = data.signing_message(true);
                            if ms.set.verify_multisig(&data.signatures, &message) {
                                match ms.set.add_maintainer(data.target, height) {
                                    Ok(()) => {
                                        ms.last_derived_height = height;
                                        if let Err(e) = ms.save(&self.config.data_dir) {
                                            warn!("Failed to persist maintainer state: {}", e);
                                        }
                                        info!(
                                            "[MAINTAINER] Added maintainer {} at height {}",
                                            data.target.to_hex(),
                                            height
                                        );
                                    }
                                    Err(e) => warn!("[MAINTAINER] Add failed: {}", e),
                                }
                            } else {
                                warn!(
                                    "[MAINTAINER] Rejected AddMaintainer: insufficient signatures"
                                );
                            }
                        }
                    }
                }

                // Process MaintainerRemove transactions — applied immediately
                if tx.tx_type == TxType::RemoveMaintainer {
                    if let Some(maintainer_state) = &self.maintainer_state {
                        if let Some(data) =
                            doli_core::maintainer::MaintainerChangeData::from_bytes(&tx.extra_data)
                        {
                            let mut ms = maintainer_state.write().await;
                            let message = data.signing_message(false);
                            if ms.set.verify_multisig_excluding(
                                &data.signatures,
                                &message,
                                &data.target,
                            ) {
                                match ms.set.remove_maintainer(&data.target, height) {
                                    Ok(()) => {
                                        ms.last_derived_height = height;
                                        if let Err(e) = ms.save(&self.config.data_dir) {
                                            warn!("Failed to persist maintainer state: {}", e);
                                        }
                                        info!(
                                            "[MAINTAINER] Removed maintainer {} at height {}",
                                            data.target.to_hex(),
                                            height
                                        );
                                    }
                                    Err(e) => warn!("[MAINTAINER] Remove failed: {}", e),
                                }
                            } else {
                                warn!("[MAINTAINER] Rejected RemoveMaintainer: insufficient signatures");
                            }
                        }
                    }
                }

                // Process ProtocolActivation transactions — verified against on-chain maintainer set
                if tx.tx_type == TxType::ProtocolActivation {
                    if let Some(data) = tx.protocol_activation_data() {
                        // Use on-chain MaintainerSet if available, fall back to ad-hoc derivation
                        let mset = if let Some(maintainer_state) = &self.maintainer_state {
                            let ms = maintainer_state.read().await;
                            if ms.set.is_fully_bootstrapped() {
                                ms.set.clone()
                            } else {
                                // Not yet bootstrapped — derive ad-hoc
                                let mut sorted = producers.all_producers().to_vec();
                                sorted.sort_by_key(|p| p.registered_at);
                                let keys: Vec<crypto::PublicKey> = sorted
                                    .iter()
                                    .take(doli_core::maintainer::INITIAL_MAINTAINER_COUNT)
                                    .map(|p| p.public_key)
                                    .collect();
                                doli_core::MaintainerSet::with_members(keys, height)
                            }
                        } else {
                            let mut sorted = producers.all_producers().to_vec();
                            sorted.sort_by_key(|p| p.registered_at);
                            let keys: Vec<crypto::PublicKey> = sorted
                                .iter()
                                .take(doli_core::maintainer::INITIAL_MAINTAINER_COUNT)
                                .map(|p| p.public_key)
                                .collect();
                            doli_core::MaintainerSet::with_members(keys, height)
                        };

                        let message = data.signing_message();
                        if mset.verify_multisig(&data.signatures, &message) {
                            pending_protocol_activation_data =
                                Some((data.protocol_version, data.activation_epoch));
                            info!(
                                "[PROTOCOL] Verified activation tx: v{} at epoch {}",
                                data.protocol_version, data.activation_epoch
                            );
                        } else {
                            warn!("[PROTOCOL] Rejected activation: insufficient maintainer signatures");
                        }
                    }
                }
            }

            // Process completed unbonding periods
            let completed = producers.process_unbonding(height, UNBONDING_PERIOD);
            for producer in &completed {
                info!(
                    "Producer {} completed unbonding, bond released",
                    crypto_hash(producer.public_key.as_bytes())
                );
            }
        }

        // Add newly registered producers to known_producers for bootstrap round-robin
        // This ensures they can produce blocks immediately (without waiting for gossip discovery)
        let genesis_active = self.config.network.is_in_genesis(height);
        if !new_registrations.is_empty()
            && (self.config.network == Network::Testnet
                || self.config.network == Network::Devnet
                || genesis_active)
        {
            let mut known = self.known_producers.write().await;
            let mut added_any = false;
            for pubkey in new_registrations {
                if !known.contains(&pubkey) {
                    known.push(pubkey);
                    added_any = true;
                    let pubkey_hash = crypto_hash(pubkey.as_bytes());
                    info!(
                        "Added registered producer to known_producers: {} (now {} known)",
                        &pubkey_hash.to_hex()[..16],
                        known.len()
                    );
                }
            }
            // Sort for deterministic ordering (all nodes compute same order)
            known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

            // Trigger stability check so round-robin waits for convergence
            if added_any {
                self.last_producer_list_change = Some(std::time::Instant::now());
            }
        }

        // Update chain state
        {
            let mut state = self.chain_state.write().await;
            state.update(block_hash, height, block.header.slot);
            // Clear snap sync marker: block store now has at least one real block,
            // so the next restart's integrity check will find it normally.
            state.clear_snap_sync();

            // Apply deferred protocol activation (verified during tx processing)
            if let Some((version, activation_epoch)) = pending_protocol_activation_data {
                let current_epoch = height / SLOTS_PER_EPOCH as u64;
                if activation_epoch > current_epoch && version > state.active_protocol_version {
                    state.pending_protocol_activation = Some((version, activation_epoch));
                    info!(
                        "[PROTOCOL] Scheduled activation: v{} at epoch {} (current epoch {})",
                        version, activation_epoch, current_epoch
                    );
                } else {
                    warn!(
                        "[PROTOCOL] Rejected activation: v{} at epoch {} (current v{}, epoch {})",
                        version, activation_epoch, state.active_protocol_version, current_epoch
                    );
                }
            }

            // Check pending protocol activation at epoch boundaries
            if doli_core::EpochSnapshot::is_epoch_boundary(height) {
                if let Some((version, activation_epoch)) = state.pending_protocol_activation {
                    let current_epoch = height / SLOTS_PER_EPOCH as u64;
                    if current_epoch >= activation_epoch {
                        state.active_protocol_version = version;
                        state.pending_protocol_activation = None;
                        info!(
                            "[PROTOCOL] Activated protocol version {} at epoch {} (height {})",
                            version, current_epoch, height
                        );
                    }
                }
            }

            // For devnet: set genesis_timestamp from first block if not already set from chainspec
            // When chainspec has a fixed genesis time, we MUST use it (all nodes must agree)
            // Only fall back to deriving from block timestamp if no chainspec genesis was provided
            if state.genesis_timestamp == 0 && height <= 1 {
                // Check if we have a chainspec genesis time override
                if let Some(override_time) = self.config.genesis_time_override {
                    // Use the chainspec genesis time (already set in params)
                    state.genesis_timestamp = override_time;
                    info!(
                        "Genesis timestamp set from chainspec: {} (block {} received)",
                        state.genesis_timestamp, height
                    );
                } else {
                    // No chainspec override - derive from block timestamp (legacy behavior)
                    let block_timestamp = block.header.timestamp;
                    let new_genesis_time =
                        block_timestamp - (block_timestamp % self.params.slot_duration);
                    state.genesis_timestamp = new_genesis_time;

                    // Also update params.genesis_time for devnet so slot calculations use synced value
                    if self.config.network == Network::Devnet
                        && self.params.genesis_time != new_genesis_time
                    {
                        info!(
                            "Devnet genesis time synced from block {}: {} (was {})",
                            height, new_genesis_time, self.params.genesis_time
                        );
                        self.params.genesis_time = new_genesis_time;
                    } else {
                        info!(
                            "Genesis timestamp set from block {}: {}",
                            height, state.genesis_timestamp
                        );
                    }
                }
            }

            // Cache state root atomically: chain_state is still write-locked,
            // utxo and producer_set were already updated earlier in apply_block.
            // No TOCTOU race window possible.
            let utxo = self.utxo_set.read().await;
            let ps = self.producer_set.read().await;
            if let Ok(root) = storage::compute_state_root(&state, &utxo, &ps) {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, state.best_hash, state.best_height));
            }
        }

        // Track this block for finality (attestation-based finality gadget).
        // The total_weight is the sum of all active producer bonds at this height.
        {
            let producers = self.producer_set.read().await;
            let total_weight: u64 = producers
                .active_producers_at_height(height)
                .iter()
                .map(|p| p.selection_weight())
                .sum();
            drop(producers);

            if total_weight > 0 {
                let mut sync = self.sync_manager.write().await;
                sync.track_block_for_finality(block_hash, height, block.header.slot, total_weight);
            }
        }

        // Apply deferred producer updates at epoch boundaries.
        // During epoch 0 (blocks 1-359), apply every block so genesis producers work immediately.
        // After epoch 0, apply only at epoch boundaries (height % 360 == 0).
        let mut needs_full_producer_write = false;
        {
            let is_epoch_0 = height < SLOTS_PER_EPOCH as u64;
            let is_boundary = doli_core::EpochSnapshot::is_epoch_boundary(height);
            if is_epoch_0 || is_boundary {
                let mut producers = self.producer_set.write().await;
                if producers.has_pending_updates() {
                    let count = producers.pending_update_count();
                    producers.apply_pending_updates();
                    self.cached_scheduler = None; // Force scheduler rebuild
                    needs_full_producer_write = true; // Many producers may have changed
                    info!(
                        "Applied {} deferred producer updates at height {} (epoch_0={}, boundary={})",
                        count, height, is_epoch_0, is_boundary
                    );
                }
            }
        }

        // Bootstrap maintainer set once we have enough producers.
        // Must run after apply_pending_updates so newly registered producers are visible.
        self.maybe_bootstrap_maintainer_set(height).await;

        // NOTE: recompute_tier + EpochSnapshot + attestation moved after batch commit
        // to avoid borrow conflict with BlockBatch (which borrows self.state_db).

        // GENESIS END: Derive and register producers with REAL bonds from the blockchain.
        // This runs when applying the FIRST post-genesis block (genesis_blocks + 1).
        // Each genesis producer's coinbase reward UTXOs are consumed to back their bond,
        // creating a proper bond-backed registration (no phantom bonds).
        let genesis_blocks = self.config.network.genesis_blocks();
        if genesis_blocks > 0 && height == genesis_blocks + 1 {
            info!("=== GENESIS PHASE COMPLETE at height {} ===", height);

            let genesis_producers = self.derive_genesis_producers_from_chain();
            let genesis_bls = self.genesis_bls_pubkeys();
            let producer_count = genesis_producers.len();

            if producer_count > 0 {
                info!(
                    "Derived {} genesis producers from blockchain (blocks 1-{}), {} with BLS keys",
                    producer_count,
                    genesis_blocks,
                    genesis_bls.len()
                );

                let bond_unit = self.config.network.bond_unit();
                let era = self.params.height_to_era(height);
                let mut utxo = self.utxo_set.write().await;
                let mut producers = self.producer_set.write().await;

                // Clear bootstrap phantom producers — replace with real bond-backed ones
                producers.clear();

                // Consume pool UTXOs for genesis bonds.
                // All per-block coinbase went to the reward pool — draw bonds from there.
                let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
                let mut pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
                pool_utxos.sort_by(|(a_op, a_entry), (b_op, b_entry)| {
                    a_entry
                        .height
                        .cmp(&b_entry.height)
                        .then_with(|| a_op.tx_hash.cmp(&b_op.tx_hash))
                        .then_with(|| a_op.index.cmp(&b_op.index))
                });

                let total_pool: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
                let total_bonds_needed = bond_unit * producer_count as u64;
                info!(
                    "Genesis bond migration: pool={} DOLI, bonds_needed={} DOLI ({} producers × {} bond)",
                    total_pool / 100_000_000,
                    total_bonds_needed / 100_000_000,
                    producer_count,
                    bond_unit / 100_000_000
                );

                if total_pool < total_bonds_needed {
                    warn!(
                        "Pool has insufficient funds for all bonds ({} < {}), some producers may not bond",
                        total_pool, total_bonds_needed
                    );
                }

                // Consume all pool UTXOs (they'll be redistributed as bonds + remainder back to pool)
                let mut pool_consumed: u64 = 0;
                for (outpoint, entry) in &pool_utxos {
                    undo_spent_utxos.push((*outpoint, entry.clone()));
                    utxo.remove(outpoint)?;
                    let _ = batch.spend_utxo(outpoint);
                    pool_consumed += entry.output.amount;
                }

                // Create bonds for each producer from pool funds
                let mut bonds_created: u64 = 0;
                for pubkey in &genesis_producers {
                    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey.as_bytes());

                    if pool_consumed - bonds_created < bond_unit {
                        warn!(
                            "Insufficient remaining pool for producer {} bond",
                            hex::encode(&pubkey.as_bytes()[..8])
                        );
                        continue;
                    }

                    // Create deterministic bond hash for tracking
                    let bond_hash = hash_with_domain(b"genesis_bond", pubkey.as_bytes());

                    // Create Bond UTXO
                    let bond_outpoint = storage::Outpoint::new(bond_hash, 0);
                    let bond_entry = storage::UtxoEntry {
                        output: doli_core::transaction::Output::bond(
                            bond_unit,
                            pubkey_hash,
                            u64::MAX, // locked until withdrawal
                            0,        // genesis bond — slot 0
                        ),
                        height,
                        is_coinbase: false,
                        is_epoch_reward: false,
                    };
                    undo_created_utxos.push(bond_outpoint);
                    utxo.insert(bond_outpoint, bond_entry.clone())?;
                    batch.add_utxo(bond_outpoint, bond_entry);
                    bonds_created += bond_unit;

                    // Register producer with real bond outpoint
                    // registered_at = 0: Genesis producers are exempt from ACTIVATION_DELAY
                    // (they've been producing since block 1, no propagation delay needed)
                    let mut producer_info = storage::ProducerInfo::new_with_bonds(
                        *pubkey,
                        0,
                        bond_unit,
                        (bond_hash, 0),
                        era,
                        1, // 1 bond
                    );
                    if let Some(bls) = genesis_bls.get(pubkey) {
                        producer_info.bls_pubkey = bls.clone();
                    }

                    match producers.register(producer_info, height) {
                        Ok(()) => {
                            info!(
                                "  Registered genesis producer {} with bond from pool ({} DOLI bonded)",
                                hex::encode(&pubkey.as_bytes()[..8]),
                                bond_unit / UNITS_PER_COIN,
                            );
                        }
                        Err(e) => {
                            warn!(
                                "  Failed to register genesis producer {}: {}",
                                hex::encode(&pubkey.as_bytes()[..8]),
                                e
                            );
                        }
                    }
                }

                // Return remainder to pool (pool funds minus bonds)
                let remainder = pool_consumed.saturating_sub(bonds_created);
                if remainder > 0 {
                    let remainder_hash = hash_with_domain(b"genesis_pool_remainder", b"doli");
                    let remainder_outpoint = storage::Outpoint::new(remainder_hash, 0);
                    let remainder_entry = storage::UtxoEntry {
                        output: doli_core::transaction::Output::normal(remainder, pool_hash),
                        height,
                        is_coinbase: false,
                        is_epoch_reward: false,
                    };
                    undo_created_utxos.push(remainder_outpoint);
                    utxo.insert(remainder_outpoint, remainder_entry.clone())?;
                    batch.add_utxo(remainder_outpoint, remainder_entry);
                    info!(
                        "Genesis pool remainder: {} DOLI returned to pool",
                        remainder / UNITS_PER_COIN
                    );
                }

                // Full producer write needed — clear + rebuild happened
                needs_full_producer_write = true;

                info!(
                    "Genesis complete: {} producers active, {} DOLI bonded, {} DOLI in pool",
                    producers.active_producers().len(),
                    bonds_created / UNITS_PER_COIN,
                    remainder / UNITS_PER_COIN,
                );
            } else {
                warn!("Genesis ended with no producers found in blockchain!");
            }
        }

        // Remove transactions from mempool and prune any with now-spent inputs
        {
            let mut mempool = self.mempool.write().await;
            mempool.remove_for_block(&block.transactions);
            let utxo = self.utxo_set.read().await;
            let height = self.chain_state.read().await.best_height;
            mempool.revalidate(&utxo, height);
        }

        // Get producer's effective weight for fork choice rule
        let producer_weight = {
            let producers = self.producer_set.read().await;
            producers
                .get_by_pubkey(&block.header.producer)
                .map(|p| p.effective_weight(height))
                .unwrap_or(1) // Default weight 1 for unknown producers (bootstrap)
        };

        // Update sync manager with weight for fork choice rule
        self.sync_manager.write().await.block_applied_with_weight(
            block_hash,
            height,
            block.header.slot,
            producer_weight,
            block.header.prev_hash,
        );

        // Store the block AFTER all transaction validation has passed.
        // This prevents block store poisoning: if UTXO validation fails, the block
        // is NOT in the store, so sync can retry it cleanly on the next attempt.
        // Previously this was before the tx loop, causing the N4 stuck-at-22888 bug.
        self.block_store.put_block(&block, height)?;
        self.block_store.set_canonical_chain(block_hash, height)?;

        // Atomic state persistence via StateDb WriteBatch.
        // All state changes (UTXOs, chain_state, producers) committed in one batch.
        // Crash between any two writes is impossible — it's all-or-nothing.
        {
            let state = self.chain_state.read().await;
            batch.put_chain_state(&state);
            batch.set_last_applied(height, block_hash, state.best_slot);

            let producers = self.producer_set.read().await;
            if needs_full_producer_write {
                batch.write_full_producer_set(&producers);
            } else {
                batch.write_dirty_producers(
                    &producers,
                    &dirty_producer_keys,
                    &removed_producer_keys,
                    &dirty_exit_keys,
                );
            }
        }
        batch
            .commit()
            .map_err(|e| anyhow::anyhow!("StateDb batch commit failed: {}", e))?;

        // Persist undo data for this block (enables O(depth) rollbacks)
        let undo = storage::UndoData {
            spent_utxos: undo_spent_utxos,
            created_utxos: undo_created_utxos,
            producer_snapshot: undo_producer_snapshot,
        };
        self.state_db.put_undo(height, &undo);

        // Prune old undo data (keep last MAX_REORG_DEPTH blocks)
        const UNDO_KEEP_DEPTH: u64 = 2000; // 2x MAX_REORG_DEPTH for safety margin
        if height > UNDO_KEEP_DEPTH {
            self.state_db.prune_undo_before(height - UNDO_KEEP_DEPTH);
        }

        // Recompute our tier at epoch boundaries and build EpochSnapshot
        // (moved after batch commit to avoid borrow conflict with BlockBatch)
        self.recompute_tier(height).await;
        if doli_core::EpochSnapshot::is_epoch_boundary(height) {
            let epoch = doli_core::EpochSnapshot::epoch_from_height(height);
            let producers = self.producer_set.read().await;
            let active = producers.active_producers_at_height(height);
            let pws: Vec<(PublicKey, u64)> = active
                .iter()
                .map(|p| (p.public_key, p.selection_weight()))
                .collect();
            let total_w: u64 = pws.iter().map(|(_, w)| *w).sum();
            let tier1 = compute_tier1_set(&pws);
            let mut all_pks: Vec<PublicKey> = pws.into_iter().map(|(pk, _)| pk).collect();
            all_pks.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            drop(producers);

            let snapshot = doli_core::EpochSnapshot::new(epoch, tier1, &all_pks, total_w);
            info!(
                "EpochSnapshot built: epoch={}, producers={}, weight={}, merkle={}",
                epoch, snapshot.total_producers, snapshot.total_weight, snapshot.merkle_root
            );

            // Reset minute tracker for the new epoch
            self.minute_tracker.reset();
        }

        // Per-block attestation: sign chain tip for finality gadget + record in tracker.
        self.create_and_broadcast_attestation(block_hash, block.header.slot, height)
            .await;
        if let Some(ref kp) = self.producer_key {
            let minute = attestation_minute(block.header.slot);
            if let Some(ref bls_kp) = self.bls_key {
                let bls_msg = crypto::attestation_message(&block_hash, block.header.slot);
                let bls_sig = crypto::bls_sign(&bls_msg, bls_kp.secret_key())
                    .map(|s| s.as_bytes().to_vec())
                    .unwrap_or_default();
                self.minute_tracker
                    .record_with_bls(*kp.public_key(), minute, bls_sig);
            } else {
                self.minute_tracker.record(*kp.public_key(), minute);
            }
        }

        // Buffer block for archiving (will be flushed when finalized)
        if self.archive_tx.is_some() {
            if let Ok(data) = bincode::serialize(&block) {
                self.pending_archive.push_back(ArchiveBlock {
                    height,
                    hash: block_hash,
                    data,
                });
            }
            // Flush any blocks that just reached finality
            self.flush_finalized_to_archive().await;
        }

        // Broadcast new block event to WebSocket subscribers
        if let Some(ref ws_tx) = *self.ws_sender.read().await {
            let _ = ws_tx.send(rpc::WsEvent::NewBlock {
                hash: block_hash.to_hex(),
                height,
                slot: block.header.slot,
                timestamp: block.header.timestamp,
                producer: hex::encode(block.header.producer.as_bytes()),
                tx_count: block.transactions.len(),
            });
        }

        // Note: Don't broadcast here. Blocks received from the network should not be
        // re-broadcast (they already came from the network). Locally produced blocks
        // should be broadcast explicitly by the caller (produce_block).

        Ok(())
    }
}
