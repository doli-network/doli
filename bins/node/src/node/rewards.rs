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
    ///
    /// # FAIL-FAST SEMANTICS (M-RC9, INC-I-034, 2026-04-16)
    ///
    /// This function is authoritative for EpochReward output construction. If the
    /// local block_store is incomplete within `[epoch_start, epoch_end)` — either
    /// because blocks are missing, or because post-activation blocks lack a body
    /// bitfield — any attestation-minute count we compute is a silent subset of
    /// the canonical count, and the resulting EpochReward will diverge from peers
    /// with complete stores. That divergence caused the 2026-04-16 live mainnet
    /// cascade (Santiago 39600–39628 gap).
    ///
    /// To prevent silent divergence, the function REFUSES TO PRODUCE OUTPUT when
    /// the input is incomplete: it returns an empty `Vec<(u64, Hash)>`, which the
    /// caller already handles as "no epoch reward distributable this epoch" (the
    /// pool accumulates into the next epoch — identical to Tier 3 fallback at
    /// line ~133 and the all-qualifiers-disqualified path at line ~158). The
    /// return type is intentionally unchanged from the pre-fix version.
    ///
    /// Epoch 0 is exempt from the incompleteness check because it has no
    /// attestation data by construction (genesis epoch short-circuit).
    pub async fn calculate_epoch_rewards(&self, epoch: u64) -> Vec<(u64, Hash)> {
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        let epoch_start_height = epoch * blocks_per_epoch;
        let epoch_end_height = (epoch + 1) * blocks_per_epoch;

        // Get producer list for bitfield decoding.
        //
        // Before REWARDS_EPOCH_LIST_FIX_HEIGHT: active_producers_at_height (all active, sorted).
        // After: epoch_state.producer_list only (same list post_commit uses to decode).
        //
        // The encoder uses epoch_state.producer_list for indices 0..N-1. Extra producers
        // (activated mid-epoch) occupy indices N+ but cannot qualify for rewards this epoch
        // (they can't reach 54/60 attestation minutes). Decoding only the first N indices
        // matches post_commit.rs exactly — proven correct in production.
        let use_epoch_list =
            epoch_start_height >= self.config.network.params().rewards_epoch_list_fix_height;
        let sorted_producers: Vec<storage::producer::ProducerInfo> = if use_epoch_list {
            // Post-fix: use epoch_state.producer_list (identical to post_commit decoder)
            let epl = &self.epoch_state.producer_list;
            let producers = self.producer_set.read().await;
            epl.iter()
                .filter_map(|pk| producers.get_by_pubkey(pk).cloned())
                .collect()
        } else {
            // Pre-fix: legacy behavior (active_producers_at_height sorted globally)
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

        // M-RC9 (INC-I-034): track any form of incompleteness in the block_store
        // window. A non-zero count in either bucket means we CANNOT compute a
        // canonical qualifier set, and must fail-fast to prevent divergent
        // EpochReward outputs across peers. Skip for epoch 0 (genesis has no
        // attestation data — the epoch-0 branch below includes all producers
        // unconditionally, so incompleteness is irrelevant).
        let mut missing_block_count: u64 = 0;
        let mut silent_bitfield_count: u64 = 0;

        for h in epoch_start_height..epoch_end_height {
            match self.block_store.get_block_by_height(h) {
                Ok(Some(block)) => {
                    // Skip blocks with zero presence_root (no attestation data)
                    if block.header.presence_root.is_zero() {
                        continue;
                    }

                    let minute = attestation_minute(block.header.slot);
                    let indices = if !block.attestation_bitfield.is_empty() {
                        // Body bitfield available: decode from body (no 256 cap)
                        doli_core::decode_attestation_bitfield_vec(
                            &block.attestation_bitfield,
                            producer_count,
                        )
                    } else if h < doli_core::consensus::BITFIELD_BODY_ACTIVATION_HEIGHT {
                        // Pre-activation: presence_root IS the raw bitfield
                        decode_attestation_bitfield(&block.header.presence_root, producer_count)
                    } else {
                        // Post-activation without body (snap sync gap or header-only
                        // store). M-RC9: record as incomplete instead of silently
                        // dropping. presence_root is BLAKE3 hash, NOT a bitfield —
                        // decoding it produces garbage indices.
                        silent_bitfield_count += 1;
                        vec![]
                    };

                    // Union: for each producer index attested in this block, add the minute
                    for idx in indices {
                        attested_minutes.entry(idx).or_default().insert(minute);
                    }
                }
                Ok(None) => {
                    // M-RC9: block missing from local store. Cannot be silently
                    // skipped — see function doc "FAIL-FAST SEMANTICS".
                    missing_block_count += 1;
                }
                Err(e) => {
                    // Treat a store read error as incompleteness — we cannot
                    // prove the block is present.
                    missing_block_count += 1;
                    warn!(
                        "[ECON_EPOCH_DISTRIBUTION] block_store read error at height={} \
                         during epoch={} scan: {} — treating as missing",
                        h, epoch, e
                    );
                }
            }
        }

        // Fail-fast: any incompleteness in the epoch window invalidates the
        // qualifier set for everyone. Returning empty is the same shape the
        // caller already handles for Tier 3 ("pool accumulates to next epoch").
        // Epoch 0 is exempt — the epoch-0 branch below auto-qualifies every
        // producer without reading `attested_minutes`, so incompleteness of
        // the scan cannot cause divergence for that epoch.
        if epoch > 0 && (missing_block_count > 0 || silent_bitfield_count > 0) {
            error!(
                "[ECON_EPOCH_DISTRIBUTION] incomplete_block_store: gap_count={} \
                 silent_bitfield_count={} — refusing to compute epoch rewards for \
                 epoch={} (range={}..{}). Pool accumulates to next epoch.",
                missing_block_count,
                silent_bitfield_count,
                epoch,
                epoch_start_height,
                epoch_end_height
            );
            return Vec::new();
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
            self.epoch_state
                .bond_snapshot
                .get(&pkh)
                .copied()
                .unwrap_or(1)
        };
        let qualifying_bonds: u64 = qualified.iter().map(|p| bond_for(p)).sum();

        if qualifying_bonds == 0 {
            return Vec::new();
        }

        // Calculate total pool from accumulated coinbase UTXOs in the reward pool.
        // Pre-activation: include current block's coinbase (not yet in UTXO set during
        // block production) so the distributed amount matches what the side-effect consume
        // step will remove (it removes ALL pool UTXOs including the new coinbase).
        // Post-activation: only count existing pool UTXOs — the current block's coinbase
        // is NOT referenced as an input (its hash isn't known yet at assembly time).
        // That 1 block reward stays in the pool and gets distributed next epoch.
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let pool = {
            let utxo = self.utxo_set.read().await;
            let pool_utxos = utxo.get_by_pubkey_hash(&pool_hash);
            let utxo_total: u64 = pool_utxos.iter().map(|(_, e)| e.output.amount).sum();
            if epoch_end_height >= doli_core::consensus::EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT {
                utxo_total // post-activation: only existing UTXOs
            } else {
                utxo_total + self.params.block_reward(epoch_end_height) // pre-activation: + current coinbase
            }
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
                // INC-I-056: bond_for() now returns the effective weight from bond_snapshot
                // (own - delegated_away + received). To split the reward correctly between
                // producer and delegators, extract the "own" portion by subtracting received.
                let effective_bonds = bond_for(producer_info).max(1);
                let delegated: u64 = producer_info
                    .received_delegations
                    .iter()
                    .map(|(_, c)| *c as u64)
                    .sum();
                let own_bonds = effective_bonds.saturating_sub(delegated).max(1);
                let total_bonds = effective_bonds; // own + received = effective

                let own_share = reward * own_bonds / total_bonds;
                let delegated_share = reward - own_share;
                let delegate_fee = delegated_share * DELEGATE_REWARD_PCT as u64 / 100;
                let staker_pool = delegated_share - delegate_fee; // remainder to stakers, no dust

                // Distribute staker pool to delegators, last gets remainder.
                // INC-I-061: received_delegations stores (crypto_hash(pubkey), bond_count)
                // which is the ProducerSet internal key — NOT the wallet address. Reward
                // outputs must use hash_with_domain(ADDRESS_DOMAIN, pubkey) to match the
                // delegator's wallet. Look up the delegator's public key via ProducerSet.
                let mut staker_distributed = 0u64;
                let delegators: Vec<_> = producer_info.received_delegations.iter().collect();
                let producers_read = self.producer_set.read().await;
                for (i, (delegator_hash, bond_count)) in delegators.iter().enumerate() {
                    let delegator_reward = if i == delegators.len() - 1 {
                        staker_pool - staker_distributed // last delegator gets remainder
                    } else {
                        staker_pool * (*bond_count as u64) / delegated
                    };
                    staker_distributed += delegator_reward;
                    if delegator_reward > 0 {
                        // Look up the delegator's pubkey to compute correct wallet address
                        let delegator_address = producers_read
                            .get(delegator_hash)
                            .map(|info| {
                                hash_with_domain(ADDRESS_DOMAIN, info.public_key.as_bytes())
                            })
                            .unwrap_or(*delegator_hash);
                        reward_outputs.push((delegator_reward, delegator_address));
                    }
                }
                drop(producers_read);

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

    /// Rebuild epoch_bond_snapshot and epoch_producer_list from block history.
    ///
    /// At startup, init.rs seeds these with "all active producers" (no attestation
    /// filtering) and "current UTXO bonds" (may include mid-epoch add-bonds).
    /// If an epoch boundary passed while the node was offline, these diverge from
    /// the network's frozen state → "invalid producer for slot" on gossip blocks.
    ///
    /// This function replays the same logic as post_commit_actions() at the last
    /// epoch boundary to reconstruct the exact scheduler state the network is using.
    /// DEPRECATED: backward-compatibility fallback for pre-upgrade undo data.
    /// Post-upgrade blocks carry epoch_state_snapshot in UndoData, making this
    /// function unnecessary. If this fires on a post-upgrade block, it indicates
    /// a persistence bug.
    pub async fn rebuild_epoch_state_from_blocks(&mut self) {
        warn!(
            "[EPOCH_REBUILD] rebuild_epoch_state_from_blocks called — should only fire for pre-upgrade undo data or reorg without epoch_state snapshot"
        );
        let current_h = self.chain_state.read().await.best_height;
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        if blocks_per_epoch == 0 || current_h == 0 {
            return;
        }

        let epoch = current_h / blocks_per_epoch;
        if epoch == 0 {
            return; // Genesis epoch — init.rs seeding is correct
        }

        let epoch_boundary_h = epoch * blocks_per_epoch;

        // INC-I-054 SAFETY CHECK: Detect incomplete block history upfront.
        // Snap-synced nodes only have blocks from the sync floor. If block 1 is
        // missing, the attestation scan below will produce non-deterministic results
        // that differ from nodes with full history → guaranteed fork.
        //
        // When detected: skip the block scan entirely, use all active producers,
        // and enable Light validation mode so we accept blocks we can't verify.
        let block_store_floor = self.block_store.get_block_by_height(1).ok().flatten();
        let lookback_start = epoch.saturating_sub(3) * blocks_per_epoch;
        let lookback_start_block = if lookback_start > 0 {
            self.block_store
                .get_block_by_height(lookback_start)
                .ok()
                .flatten()
        } else {
            block_store_floor.clone()
        };
        let has_incomplete_history = block_store_floor.is_none() || lookback_start_block.is_none();
        if has_incomplete_history {
            warn!(
                "[EPOCH_REBUILD] Incomplete block history detected (block_1={}, lookback_start_h={}={}). \
                 Attestation scan would produce non-deterministic results — using all active producers \
                 with Light validation mode. This is safe but suboptimal; next epoch boundary will \
                 rebuild correctly from post_commit.",
                block_store_floor.is_some(),
                lookback_start,
                lookback_start_block.is_some(),
            );
        }

        // 1. epoch_bond_snapshot: never overwrite with an older epoch.
        //    During header-first catch-up, this function is called at each epoch
        //    boundary the node processes. If the node already has a correct snapshot
        //    from a later epoch (loaded from persisted or received via snap sync),
        //    overwriting it with an earlier epoch's UTXO recalculation causes divergence.
        if self.epoch_state.epoch >= epoch {
            info!(
                "[STARTUP] Keeping epoch_bond_snapshot (epoch={} >= rebuild epoch={}, bonds={})",
                self.epoch_state.epoch,
                epoch,
                self.epoch_state.bond_snapshot.values().sum::<u64>()
            );
        } else if self.state_db.get_epoch_bond_snapshot().is_some() && self.epoch_state.epoch > 0 {
            info!(
                "[STARTUP] Keeping persisted epoch_bond_snapshot (epoch={}, bonds={})",
                self.epoch_state.epoch,
                self.epoch_state.bond_snapshot.values().sum::<u64>()
            );
        } else {
            let utxo = self.utxo_set.read().await;
            let producers = self.producer_set.read().await;
            let active = producers.active_producers_at_height(epoch_boundary_h);
            let mut snapshot = std::collections::HashMap::new();
            for p in &active {
                let pkh =
                    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, p.public_key.as_bytes());
                let count = utxo
                    .count_bonds(&pkh, self.config.network.bond_unit())
                    .max(1) as u64;
                snapshot.insert(pkh, count);
            }
            let total: u64 = snapshot.values().sum();
            warn!(
                "[STARTUP] No persisted bond snapshot — rebuilt from UTXO (may diverge): epoch={}, producers={}, total_bonds={}",
                epoch, snapshot.len(), total
            );
            self.epoch_state.bond_snapshot = snapshot;
            self.epoch_state.epoch = epoch;
        }

        // 2. Rebuild epoch_producer_list with attestation filtering
        //    (same logic as post_commit.rs:74-131)
        {
            let producers = self.producer_set.read().await;
            let active: Vec<crypto::PublicKey> = producers
                .active_producers_at_height(epoch_boundary_h)
                .iter()
                .map(|p| p.public_key)
                .collect();
            drop(producers);

            // Fix #4B (2026-04-15, synmgrefactor branch): tier-promotion accumulators
            // reconstructed from block scan below. Declared here so the tier logic
            // after epoch_producer_list assignment can use them. HashMaps are empty
            // when epoch <= 1 (no scan needed) and tier logic then won't promote.
            let mut just_completed_minutes: std::collections::HashMap<
                crypto::PublicKey,
                std::collections::HashSet<u32>,
            > = std::collections::HashMap::new();
            let mut just_completed_blocks: std::collections::HashMap<crypto::PublicKey, u32> =
                std::collections::HashMap::new();
            let mut scan_produced_data = false;
            // Fix #4B-edge (2026-04-15, synmgrefactor): track whether block scan
            // covered the full just-completed epoch. If the scan aborted early
            // because a block was missing from the store (historic gap post-
            // rollback), accumulators are PARTIAL and must NOT be used — tier
            // promotion could incorrectly demote producers whose contribution
            // fell in the gap. Moved out of the `else` branch so the guard
            // below can read it.
            let mut scan_covered_full_epoch = false;

            // Fix #6 (2026-04-15, synmgrefactor branch): if epoch_attested_set is
            // already populated (e.g. peer transferred accumulators via snap sync,
            // or persisted on disk from a prior run), use those directly for the
            // 3-epoch attested lookback. The block scan below is only needed when
            // we have NO attestation history at all (cold start on post-wipe node
            // with no peer accumulator payload).
            let have_inmem_accum = !self.epoch_state.attested_sets[0].is_empty()
                || !self.epoch_state.attested_sets[1].is_empty()
                || !self.epoch_state.attested_sets[2].is_empty();

            let mut new_list: Vec<crypto::PublicKey> = if epoch <= 1 {
                active
            } else if has_incomplete_history && !have_inmem_accum {
                // INC-I-054: Block history incomplete and no in-memory accumulators.
                // Skip block scan entirely — it would produce wrong results.
                // Use all active producers + Light validation until next epoch boundary.
                self.snap_sync_height = Some(current_h);
                active
            } else if have_inmem_accum {
                info!(
                    "[STARTUP] Using in-memory epoch_attested_set for filter (attested=[{},{},{}]) — no block scan needed",
                    self.epoch_state.attested_sets[0].len(),
                    self.epoch_state.attested_sets[1].len(),
                    self.epoch_state.attested_sets[2].len(),
                );
                let mut attested: std::collections::HashSet<crypto::PublicKey> =
                    std::collections::HashSet::new();
                for i in 0..3 {
                    attested.extend(&self.epoch_state.attested_sets[i]);
                }
                active
                    .into_iter()
                    .filter(|pk| attested.contains(pk))
                    .collect()
            } else {
                // Fix #4A (2026-04-15, synmgrefactor branch): attestation lookback
                // must be 3 epochs to match post_commit. Pre-fix used 1-epoch
                // window, producing a DIFFERENT producer list than the one
                // apply_block produces at epoch boundary. On any mid-epoch restart
                // (startup, snap sync, rollback), the node would freeze a wrong
                // scheduler list and diverge from the rest of the network until
                // the NEXT epoch boundary re-applied the correct filter.
                //
                // Scan up to 3 completed epochs. Union attested producers across
                // all of them. Matches post_commit line 163-168.
                let lookback_epochs: u64 = 3;
                let first_epoch = epoch.saturating_sub(lookback_epochs);
                let scan_start = first_epoch * blocks_per_epoch;
                let scan_end = epoch * blocks_per_epoch;
                // Just-completed epoch window, used for tier promotion accumulators.
                let just_completed_start = (epoch - 1) * blocks_per_epoch;
                let just_completed_end = scan_end;
                let mut attested: std::collections::HashSet<crypto::PublicKey> =
                    std::collections::HashSet::new();
                let mut have_full_epoch = true;

                let mut sorted_for_decode = active.clone();
                sorted_for_decode.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                let start_h = if scan_start == 0 { 1 } else { scan_start };
                for h in start_h..scan_end {
                    if let Ok(Some(blk)) = self.block_store.get_block_by_height(h) {
                        let in_just_completed = h >= just_completed_start && h < just_completed_end;
                        let minute = doli_core::attestation::attestation_minute(blk.header.slot);

                        attested.insert(blk.header.producer);
                        if in_just_completed {
                            scan_produced_data = true;
                            *just_completed_blocks
                                .entry(blk.header.producer)
                                .or_insert(0) += 1;
                            just_completed_minutes
                                .entry(blk.header.producer)
                                .or_default()
                                .insert(minute);
                        }

                        if !blk.header.presence_root.is_zero() {
                            let indices = if !blk.attestation_bitfield.is_empty() {
                                doli_core::decode_attestation_bitfield_vec(
                                    &blk.attestation_bitfield,
                                    sorted_for_decode.len(),
                                )
                            } else if h < doli_core::consensus::BITFIELD_BODY_ACTIVATION_HEIGHT {
                                doli_core::attestation::decode_attestation_bitfield(
                                    &blk.header.presence_root,
                                    sorted_for_decode.len(),
                                )
                            } else {
                                // Post-activation without body: skip
                                vec![]
                            };
                            for idx in indices {
                                if let Some(pk) = sorted_for_decode.get(idx) {
                                    attested.insert(*pk);
                                    if in_just_completed {
                                        just_completed_minutes
                                            .entry(*pk)
                                            .or_default()
                                            .insert(minute);
                                    }
                                }
                            }
                        }
                    } else {
                        have_full_epoch = false;
                        break;
                    }
                }

                // Propagate scan coverage to outer scope for the tier-accumulator
                // population guard (Fix #4B-edge).
                scan_covered_full_epoch = have_full_epoch;

                let skip_height = self.config.network.params().snap_attestation_skip_height;
                if have_full_epoch || epoch_boundary_h < skip_height {
                    active
                        .into_iter()
                        .filter(|pk| attested.contains(pk))
                        .collect()
                } else {
                    info!(
                        "[STARTUP] Incomplete block history for last {} epoch(s) — using all {} active producers, Light validation until next epoch boundary",
                        lookback_epochs,
                        active.len()
                    );
                    // Without full block history, our epoch_producer_list may differ
                    // from the network's attestation-filtered list. Use Light validation
                    // (skip producer eligibility on gossip) until the next epoch boundary
                    // rebuilds the list correctly. Same mechanism as snap sync runtime.
                    self.snap_sync_height = Some(current_h);
                    active
                }
            };

            // Fix #4A (2026-04-15, synmgrefactor): deadlock safety floor
            // tightened from 1/3 to 2/3 to match post_commit.rs:196. See that
            // file's explanation: >1/3 un-attested is assumed to be mass event
            // (outage), not individual inactivity. Canonical BFT threshold.
            //
            // INC-I-046: ghost exclusion — same logic as derive_at_boundary.
            {
                use doli_core::consensus::GHOST_EXCLUSION_GRACE_EPOCHS;
                let producers = self.producer_set.read().await;
                let active_at = producers.active_producers_at_height(epoch_boundary_h);
                let active_count = active_at.len();

                let ghost_exclusion_active = epoch_boundary_h
                    >= self
                        .config
                        .network
                        .params()
                        .ghost_exclusion_activation_height
                    && epoch > 1;

                let attested_union: std::collections::HashSet<&crypto::PublicKey> = self
                    .epoch_state
                    .attested_sets
                    .iter()
                    .flat_map(|s| s.iter())
                    .collect();

                let is_ghost = |pk: &crypto::PublicKey| -> bool {
                    if !ghost_exclusion_active || attested_union.contains(pk) {
                        return false;
                    }
                    match active_at.iter().find(|p| &p.public_key == pk) {
                        Some(p) => {
                            let reg_epoch =
                                p.registered_at.checked_div(blocks_per_epoch).unwrap_or(0);
                            epoch.saturating_sub(reg_epoch) > GHOST_EXCLUSION_GRACE_EPOCHS
                        }
                        None => false,
                    }
                };

                let ghost_count = if ghost_exclusion_active {
                    active_at.iter().filter(|p| is_ghost(&p.public_key)).count()
                } else {
                    0
                };
                let effective_active = active_count - ghost_count;

                if new_list.len() < (effective_active * 2 / 3)
                    || (new_list.is_empty() && effective_active > 0)
                {
                    if ghost_exclusion_active && ghost_count > 0 {
                        warn!(
                            "[STARTUP] Attestation filter left {}/{} (<2/3 of {} non-ghost) — including all non-ghosts (excluded {} ghosts)",
                            new_list.len(), active_count, effective_active, ghost_count
                        );
                        new_list = active_at
                            .iter()
                            .filter(|p| !is_ghost(&p.public_key))
                            .map(|p| p.public_key)
                            .collect();
                    } else {
                        warn!(
                            "[STARTUP] Attestation filter left {}/{} (<2/3) — mass event, including all",
                            new_list.len(),
                            active_count
                        );
                        new_list = active_at.iter().map(|p| p.public_key).collect();
                    }
                }
            }

            new_list.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            info!(
                "[STARTUP] Rebuilt epoch_producer_list: epoch={}, {} producers (attestation-filtered)",
                epoch,
                new_list.len()
            );
            self.epoch_state.producer_list = new_list;

            // Fix #4B (2026-04-15, synmgrefactor branch): populate tier-promotion
            // accumulators ONLY if in-memory state is empty. init.rs:806 already
            // loads persisted accumulators at startup — if present, they are more
            // authoritative than a block-scan reconstruction (they reflect the
            // exact state apply_block produced, including edge cases).
            //
            // Fix #4B-edge (2026-04-15, synmgrefactor): also require
            // scan_covered_full_epoch. If the block scan aborted mid-epoch due
            // to a missing block (historic gap post-rollback), accumulators are
            // PARTIAL and must not be used — tier promotion could demote
            // producers whose contribution fell in the gap.
            if self.epoch_state.attestation_accum[0].is_empty()
                && scan_produced_data
                && scan_covered_full_epoch
            {
                info!(
                    "[STARTUP] Tier accumulators empty — rebuilt from block scan: \
                     producers_with_minutes={}, producers_with_blocks={} (just-completed epoch={})",
                    just_completed_minutes.len(),
                    just_completed_blocks.len(),
                    epoch - 1
                );
                self.epoch_state.attestation_accum[0] = just_completed_minutes;
                self.epoch_state.blocks_produced = just_completed_blocks;
            } else if self.epoch_state.attestation_accum[0].is_empty()
                && scan_produced_data
                && !scan_covered_full_epoch
            {
                warn!(
                    "[STARTUP] Tier accumulators empty AND scan incomplete \
                     (historic gap in block store) — leaving accumulators empty, \
                     tier promotion will be inactive until next epoch boundary"
                );
            } else if !self.epoch_state.attestation_accum[0].is_empty() {
                info!(
                    "[STARTUP] Tier accumulators loaded from disk (pre-complete): \
                     minutes_entries={}, blocks_entries={}",
                    self.epoch_state.attestation_accum[0].len(),
                    self.epoch_state.blocks_produced.len()
                );
            }

            // Fix #4C (2026-04-15, synmgrefactor branch): COMPLETE the current-epoch
            // accumulators by replaying blocks between the last epoch boundary and
            // current_h.
            //
            // Rationale: post_commit persists epoch_attestation_accum +
            // epoch_blocks_produced_accum + epoch_attested_set to disk ONLY at
            // epoch boundaries. After an atomic deploy / restart mid-epoch, the
            // node loads disk state from the LAST boundary, so [0] reflects the
            // state AT that boundary (post-rotation: empty). Blocks already
            // applied between the boundary and the restart point had accumulated
            // into [0] on the OLD binary's in-memory state, which was LOST.
            //
            // Canary evidence (2026-04-15): n1 and n9 reached h=36021 with
            // identical state_root, bonds, epoch_producer_list, active_production_
            // list, and epoch_attested_set — but DIFFERENT sched= hash. The
            // delta was in the minute-level epoch_attestation_accum because
            // each node restarted at a different mid-epoch height, so they
            // accumulated into [0] from different starting points.
            //
            // This is cosmetic today (tier promotion doesn't fire with <50
            // producers), but a latent fork risk post-growth. Scan here is
            // bounded to at most blocks_per_epoch blocks (~360 for mainnet),
            // typically much less. Cost: ~50ms at startup.
            if current_h > epoch_boundary_h {
                let mut sorted_for_decode = self.epoch_state.producer_list.clone();
                sorted_for_decode.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                let mut replayed = 0u64;
                let mut attested_before = self.epoch_state.attested_sets[0].len();
                let minutes_before = self.epoch_state.attestation_accum[0].len();
                let blocks_before = self.epoch_state.blocks_produced.len();

                for h in (epoch_boundary_h + 1)..=current_h {
                    let Ok(Some(blk)) = self.block_store.get_block_by_height(h) else {
                        warn!(
                            "[STARTUP] Fix #4C accumulator replay: missing block at h={} \
                             (gap in store) — accumulators for current epoch may be partial",
                            h
                        );
                        break;
                    };
                    replayed += 1;
                    let minute = doli_core::attestation::attestation_minute(blk.header.slot);

                    self.epoch_state.attested_sets[0].insert(blk.header.producer);
                    *self
                        .epoch_state
                        .blocks_produced
                        .entry(blk.header.producer)
                        .or_insert(0) += 1;
                    self.epoch_state.attestation_accum[0]
                        .entry(blk.header.producer)
                        .or_default()
                        .insert(minute);

                    if !blk.header.presence_root.is_zero() && !sorted_for_decode.is_empty() {
                        let indices = if !blk.attestation_bitfield.is_empty() {
                            doli_core::decode_attestation_bitfield_vec(
                                &blk.attestation_bitfield,
                                sorted_for_decode.len(),
                            )
                        } else if h < doli_core::consensus::BITFIELD_BODY_ACTIVATION_HEIGHT {
                            doli_core::attestation::decode_attestation_bitfield(
                                &blk.header.presence_root,
                                sorted_for_decode.len(),
                            )
                        } else {
                            vec![]
                        };
                        for idx in indices {
                            if let Some(pk) = sorted_for_decode.get(idx) {
                                self.epoch_state.attested_sets[0].insert(*pk);
                                self.epoch_state.attestation_accum[0]
                                    .entry(*pk)
                                    .or_default()
                                    .insert(minute);
                            }
                        }
                    }
                }

                attested_before = self.epoch_state.attested_sets[0].len() - attested_before;
                info!(
                    "[STARTUP] Fix #4C accumulator replay: scanned {} blocks (h={}..={}) — \
                     +{} attested, +{} minutes entries, +{} blocks entries",
                    replayed,
                    epoch_boundary_h + 1,
                    current_h,
                    attested_before,
                    self.epoch_state.attestation_accum[0].len() - minutes_before,
                    self.epoch_state.blocks_produced.len() - blocks_before,
                );
            }

            // Fix #4B: apply tier system identical to post_commit.rs:237-310.
            // With current mainnet (24 producers < ACTIVE_PRODUCERS_CAP=50), this
            // is a no-op (active_production_list = epoch_producer_list.clone()),
            // but we include it for forward-compatibility when the network grows
            // past 50 producers.
            use doli_core::consensus::{
                ACTIVE_PRODUCERS_CAP, MIN_ATTESTATION_MINUTES, TIER_PROMOTION_ACTIVATION_HEIGHT,
                TIER_SYSTEM_ACTIVATION_HEIGHT,
            };
            if epoch_boundary_h >= TIER_SYSTEM_ACTIVATION_HEIGHT
                && self.epoch_state.producer_list.len() > ACTIVE_PRODUCERS_CAP
            {
                let producers = self.producer_set.read().await;
                let mut with_reg: Vec<(crypto::PublicKey, u64)> = self
                    .epoch_state
                    .producer_list
                    .iter()
                    .filter_map(|pk| producers.get_by_pubkey(pk).map(|p| (*pk, p.registered_at)))
                    .collect();
                drop(producers);

                if epoch_boundary_h >= TIER_PROMOTION_ACTIVATION_HEIGHT && epoch > 1 {
                    // Promotion: demote producers who failed to meet minimums.
                    let expected_per_producer =
                        blocks_per_epoch / self.epoch_state.producer_list.len().max(1) as u64;
                    let min_produced = (expected_per_producer * 80 / 100).max(1);
                    let attestation_minutes = &self.epoch_state.attestation_accum[0];
                    let blocks_produced = &self.epoch_state.blocks_produced;

                    let before = with_reg.len();
                    with_reg.retain(|(pk, _)| {
                        let mins = attestation_minutes.get(pk).map(|s| s.len()).unwrap_or(0);
                        let produced = blocks_produced.get(pk).copied().unwrap_or(0) as u64;
                        mins >= MIN_ATTESTATION_MINUTES && produced >= min_produced
                    });
                    let demoted = before - with_reg.len();
                    if demoted > 0 {
                        info!(
                            "[STARTUP][TIER] Demoted {} producers (min_att={}, min_prod={}/{})",
                            demoted, MIN_ATTESTATION_MINUTES, min_produced, expected_per_producer
                        );
                    }
                }

                with_reg.sort_by(|a, b| {
                    a.1.cmp(&b.1)
                        .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
                });
                self.epoch_state.active_list = with_reg
                    .iter()
                    .take(ACTIVE_PRODUCERS_CAP)
                    .map(|(pk, _)| *pk)
                    .collect();
                info!(
                    "[STARTUP][TIER] Active production list: {}/{} (by registered_at, promotion={})",
                    self.epoch_state.active_list.len(),
                    self.epoch_state.producer_list.len(),
                    epoch_boundary_h >= TIER_PROMOTION_ACTIVATION_HEIGHT,
                );
            } else {
                self.epoch_state.active_list = self.epoch_state.producer_list.clone();
            }

            // Deadlock safety: if tier filter left < 1/3, mass event — fall back.
            if self.epoch_state.active_list.len() < self.epoch_state.producer_list.len() / 3
                || self.epoch_state.active_list.is_empty()
            {
                warn!(
                    "[STARTUP][TIER] Filter left {}/{} — mass event, falling back to full epoch list",
                    self.epoch_state.active_list.len(),
                    self.epoch_state.producer_list.len()
                );
                self.epoch_state.active_list = self.epoch_state.producer_list.clone();
            }
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
