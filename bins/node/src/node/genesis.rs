use super::*;

impl Node {
    /// Derive genesis producers from on-chain VDF proof Registration TXs.
    ///
    /// Scans genesis blocks for Registration TXs and returns producers
    /// with valid registration data. VDF proofs were already verified
    /// at block acceptance time (validation.rs:validate_registration).
    ///
    /// Cached via OnceLock for the common path. Invalidated by
    /// execute_reorg/rollback_one_block when a reorg crosses the genesis
    /// boundary (target_height <= genesis_blocks).
    pub fn derive_genesis_producers_from_chain(&self) -> Vec<PublicKey> {
        self.cached_genesis_producers
            .get_or_init(|| {
                let genesis_blocks = self.config.network.genesis_blocks();
                if genesis_blocks == 0 {
                    return Vec::new();
                }

                let mut seen = std::collections::HashSet::new();
                let mut proven_producers = Vec::new();

                for height in 1..=genesis_blocks {
                    if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
                        for tx in &block.transactions {
                            if tx.tx_type == TxType::Registration {
                                if let Some(reg_data) = tx.registration_data() {
                                    // VDF proof already verified at block acceptance
                                    // (validate_registration in validation.rs).
                                    // Just check format and deduplicate.
                                    if reg_data.vdf_output.len() == 32
                                        && seen.insert(reg_data.public_key)
                                    {
                                        info!(
                                            "  Genesis producer {} (VDF pre-verified)",
                                            hex::encode(&reg_data.public_key.as_bytes()[..8])
                                        );
                                        proven_producers.push(reg_data.public_key);
                                    }
                                }
                            }
                        }
                    }
                }

                if proven_producers.is_empty() {
                    // Snap-synced nodes don't have genesis blocks — fall back to
                    // hardcoded chainspec producers to avoid breaking the scheduler.
                    let fallback: Vec<PublicKey> = match self.config.network {
                        Network::Mainnet => {
                            use doli_core::genesis::mainnet_genesis_producers;
                            mainnet_genesis_producers()
                                .into_iter()
                                .map(|(pk, _)| pk)
                                .collect()
                        }
                        Network::Testnet => {
                            use doli_core::genesis::testnet_genesis_producers;
                            testnet_genesis_producers()
                                .into_iter()
                                .map(|(pk, _)| pk)
                                .collect()
                        }
                        Network::Devnet => Vec::new(),
                    };
                    if !fallback.is_empty() {
                        warn!(
                            "[GENESIS] Blocks 1..={} missing (snap sync). Using {} chainspec \
                             genesis producers as fallback.",
                            genesis_blocks,
                            fallback.len()
                        );
                    }
                    return fallback;
                }

                info!(
                    "Derived {} genesis producers with valid VDF proofs (from {} genesis blocks)",
                    proven_producers.len(),
                    genesis_blocks
                );

                proven_producers
            })
            .clone()
    }

    /// Extract BLS pubkeys from genesis registration TXs.
    /// Returns a map from producer PublicKey → BLS pubkey bytes (48 bytes).
    /// Only includes producers that included a BLS pubkey in their registration.
    pub fn genesis_bls_pubkeys(&self) -> std::collections::HashMap<PublicKey, Vec<u8>> {
        let genesis_blocks = self.config.network.genesis_blocks();
        let mut bls_keys = std::collections::HashMap::new();

        for height in 1..=genesis_blocks {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
                for tx in &block.transactions {
                    if tx.tx_type == TxType::Registration {
                        if let Some(reg_data) = tx.registration_data() {
                            if !reg_data.bls_pubkey.is_empty()
                                && !bls_keys.contains_key(&reg_data.public_key)
                            {
                                bls_keys.insert(reg_data.public_key, reg_data.bls_pubkey.clone());
                            }
                        }
                    }
                }
            }
        }

        bls_keys
    }

    /// Consume pool UTXOs for genesis bond migration during UTXO rebuild.
    ///
    /// Must be called at height == genesis_blocks + 1 in any UTXO rebuild loop
    /// to match the apply_block() behavior. All per-block coinbase goes to the
    /// reward pool — this consumes pool UTXOs to create bonds for each producer
    /// and returns the remainder to the pool.
    pub fn consume_genesis_bond_utxos(
        utxo: &mut storage::UtxoSet,
        genesis_producers: &[PublicKey],
        bond_unit: u64,
        height: u64,
    ) -> Result<()> {
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();

        // Collect and sort all pool UTXOs (deterministic order)
        let mut pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
        pool_utxos.sort_by(|(a_op, a_entry), (b_op, b_entry)| {
            a_entry
                .height
                .cmp(&b_entry.height)
                .then_with(|| a_op.tx_hash.cmp(&b_op.tx_hash))
                .then_with(|| a_op.index.cmp(&b_op.index))
        });

        // Consume all pool UTXOs
        let mut pool_consumed: u64 = 0;
        for (outpoint, entry) in &pool_utxos {
            utxo.remove(outpoint)?;
            pool_consumed += entry.output.amount;
        }

        // Create bonds for each producer from pool funds
        let mut bonds_created: u64 = 0;
        for pubkey in genesis_producers {
            let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, pubkey.as_bytes());

            if pool_consumed - bonds_created < bond_unit {
                continue;
            }

            let bond_hash = hash_with_domain(b"genesis_bond", pubkey.as_bytes());
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
            utxo.insert(bond_outpoint, bond_entry)?;
            bonds_created += bond_unit;
        }

        // Return remainder to pool
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
            utxo.insert(remainder_outpoint, remainder_entry)?;
        }

        Ok(())
    }
}
