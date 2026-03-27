use super::*;

impl Node {
    /// Handle genesis phase completion — derive and register producers with REAL bonds.
    ///
    /// This runs when applying the FIRST post-genesis block (genesis_blocks + 1).
    /// Each genesis producer's coinbase reward UTXOs are consumed to back their bond,
    /// creating a proper bond-backed registration (no phantom bonds).
    ///
    /// Returns `true` if a full producer write is needed for the batch.
    pub async fn maybe_complete_genesis(
        &mut self,
        height: u64,
        batch: &mut storage::BlockBatch<'_>,
        undo_spent_utxos: &mut Vec<(storage::Outpoint, storage::UtxoEntry)>,
        undo_created_utxos: &mut Vec<storage::Outpoint>,
    ) -> Result<bool> {
        let genesis_blocks = self.config.network.genesis_blocks();
        if genesis_blocks == 0 || height != genesis_blocks + 1 {
            return Ok(false);
        }

        info!("=== GENESIS PHASE COMPLETE at height {} ===", height);

        let genesis_producers = self.derive_genesis_producers_from_chain();
        let genesis_bls = self.genesis_bls_pubkeys();
        let producer_count = genesis_producers.len();

        if producer_count == 0 {
            warn!("Genesis ended with no producers found in blockchain!");
            return Ok(false);
        }

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

        info!(
            "Genesis complete: {} producers active, {} DOLI bonded, {} DOLI in pool",
            producers.active_producers().len(),
            bonds_created / UNITS_PER_COIN,
            remainder / UNITS_PER_COIN,
        );

        Ok(true)
    }
}
