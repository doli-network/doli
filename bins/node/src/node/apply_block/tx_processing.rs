use super::*;

impl Node {
    /// Process UTXO changes for a single transaction (in-memory + batch for atomic persistence).
    ///
    /// Returns the transaction hash (needed for undo log tracking of created UTXOs).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn process_transaction_utxos(
        &self,
        tx: &Transaction,
        tx_index: usize,
        height: u64,
        block_slot: u32,
        utxo: &mut UtxoSet,
        batch: &mut storage::BlockBatch<'_>,
        undo_spent_utxos: &mut Vec<(storage::Outpoint, storage::UtxoEntry)>,
        undo_created_utxos: &mut Vec<storage::Outpoint>,
    ) -> Result<()> {
        let is_reward_tx = tx_index == 0 && tx.is_reward_minting();

        // Undo log: capture UTXOs BEFORE spending them
        if !is_reward_tx {
            for input in &tx.inputs {
                let outpoint = storage::Outpoint::new(input.prev_tx_hash, input.output_index);
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
            validation::validate_transaction_with_utxos(tx, &utxo_ctx, utxo).map_err(|e| {
                warn!(
                    "[UTXO] FAIL h={} tx={} type={:?} inputs={} outputs={} error={}",
                    height,
                    tx.hash(),
                    tx.tx_type,
                    tx.inputs.len(),
                    tx.outputs.len(),
                    e,
                );
                anyhow::anyhow!("UTXO validation failed for tx {}: {}", tx.hash(), e)
            })?;
        }

        // Reject outputs with duplicate unique IDs
        for output in &tx.outputs {
            match output.output_type {
                doli_core::OutputType::NFT => {
                    if let Some((token_id, _)) = output.nft_metadata() {
                        if utxo.has_unique_id(storage::UID_PREFIX_NFT, &token_id) {
                            anyhow::bail!("NFT token_id {} already exists", token_id.to_hex());
                        }
                    }
                }
                doli_core::OutputType::Pool if tx.tx_type == TxType::CreatePool => {
                    if let Some(meta) = output.pool_metadata() {
                        if utxo.has_unique_id(storage::UID_PREFIX_POOL, &meta.pool_id) {
                            anyhow::bail!(
                                "Pool {} already exists — cannot create duplicate",
                                meta.pool_id.to_hex()
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        let _ = utxo.spend_transaction(tx); // In-memory
        utxo.add_transaction(tx, height, is_reward_tx, block_slot)?; // In-memory

        // Undo log: track created UTXOs
        let tx_hash = tx.hash();
        for (index, _) in tx.outputs.iter().enumerate() {
            undo_created_utxos.push(storage::Outpoint::new(tx_hash, index as u32));
        }

        // Batch: track the same UTXO changes for atomic persistence
        if !is_reward_tx {
            let _ = batch.spend_transaction_utxos(tx);
        }
        batch.add_transaction_utxos(tx, height, is_reward_tx, block_slot);

        Ok(())
    }

    /// Process producer-related effects for a single transaction.
    ///
    /// Handles: Registration, Exit, SlashProducer, AddBond, RequestWithdrawal,
    /// DelegateBond, RevokeDelegation — all epoch-deferred.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn process_transaction_producer_effects(
        &self,
        tx: &Transaction,
        height: u64,
        block_slot: u32,
        _utxo: &UtxoSet,
        producers: &mut ProducerSet,
        dirty_producer_keys: &mut std::collections::HashSet<Hash>,
        new_registrations: &mut Vec<PublicKey>,
    ) {
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
                    .filter(|(_, o)| o.output_type == doli_core::transaction::OutputType::Bond)
                    .collect();

                if let Some(&(bond_index, _)) = bond_outputs.first() {
                    let tx_hash = tx.hash();
                    let era = self.params.height_to_era(height);
                    let total_bond_amount: u64 = bond_outputs.iter().map(|(_, o)| o.amount).sum();

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
                        if let Some(producer) = producers.get_by_pubkey_mut(&exit_data.public_key) {
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
                        .filter(|(_, o)| o.output_type == doli_core::transaction::OutputType::Bond)
                        .map(|(i, _)| (tx_hash, i as u32))
                        .collect();

                    let bond_unit = self.config.network.bond_unit();
                    producers.queue_update(PendingProducerUpdate::AddBond {
                        pubkey: add_bond_data.producer_pubkey,
                        outpoints: bond_outpoints,
                        bond_unit,
                        creation_slot: block_slot,
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
                        // TX is on-chain (passed consensus validation). Always enqueue
                        // the deferred update. Previous code had a redundant FIFO check
                        // here that silently dropped the update when output included
                        // accumulated rewards, causing permanent ProducerSet inconsistency.
                        // See: N12 testnet incident 2026-03-26.

                        // Increment pending count (prevents double-withdrawal in same epoch)
                        if let Some(producer) =
                            producers.get_by_pubkey_mut(&data.producer_pubkey)
                        {
                            producer.withdrawal_pending_count += data.bond_count;
                            dirty_producer_keys
                                .insert(crypto_hash(data.producer_pubkey.as_bytes()));
                        }

                        let bond_unit = self.config.network.bond_unit();
                        producers.queue_update(PendingProducerUpdate::RequestWithdrawal {
                            pubkey: data.producer_pubkey,
                            bond_count: data.bond_count,
                            bond_unit,
                        });
                        info!(
                            "Queued WithdrawalRequest ({} bonds) for producer {} at height {} (deferred to epoch boundary)",
                            data.bond_count,
                            crypto_hash(data.producer_pubkey.as_bytes()),
                            height
                        );
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
    }

    /// Process completed unbonding periods for producers.
    pub(super) fn process_unbonding(producers: &mut ProducerSet, height: u64) {
        let completed = producers.process_unbonding(height, UNBONDING_PERIOD);
        for producer in &completed {
            info!(
                "Producer {} completed unbonding, bond released",
                crypto_hash(producer.public_key.as_bytes())
            );
        }
    }
}
