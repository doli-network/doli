use super::*;

impl Node {
    /// Actions performed after the batch commit: tier recompute, epoch snapshot,
    /// attestation, archive buffering, and websocket broadcast.
    pub async fn post_commit_actions(&mut self, block: &Block, block_hash: Hash, height: u64) {
        // Recompute our tier at epoch boundaries and build EpochSnapshot
        // (moved after batch commit to avoid borrow conflict with BlockBatch)
        self.recompute_tier(height).await;
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();
        if doli_core::EpochSnapshot::is_epoch_boundary_with(height, blocks_per_epoch) {
            let epoch = doli_core::EpochSnapshot::epoch_from_height_with(height, blocks_per_epoch);
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

            // Rebuild epoch bond snapshot from UTXO set.
            // This snapshot is used by the scheduler for the ENTIRE next epoch.
            // All nodes compute this at the same height → deterministic scheduling.
            {
                let utxo = self.utxo_set.read().await;
                let producers = self.producer_set.read().await;
                let active = producers.active_producers_at_height(height);
                let mut snapshot = HashMap::new();
                for p in &active {
                    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, p.public_key.as_bytes());
                    let count = utxo
                        .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                        .max(1) as u64;
                    snapshot.insert(pubkey_hash, count);
                }
                let total: u64 = snapshot.values().sum();
                info!(
                    "Epoch bond snapshot rebuilt: epoch={}, producers={}, total_bonds={}",
                    epoch,
                    snapshot.len(),
                    total
                );
                self.epoch_bond_snapshot = snapshot;
                self.epoch_bond_snapshot_epoch = epoch;
                self.cached_scheduler = None; // Force scheduler rebuild with new bonds
            }

            // Reset minute tracker for the new epoch
            self.minute_tracker.reset();

            // EPOCH PRODUCER LIST: Freeze the schedule for the new epoch.
            // Includes active producers who attested in the previous epoch
            // (or all active if this is epoch 0/1). This is the base denominator
            // for slot % N scheduling — it never changes mid-epoch.
            {
                let producers = self.producer_set.read().await;
                let active: Vec<PublicKey> = producers
                    .active_producers_at_height(height)
                    .iter()
                    .map(|p| p.public_key)
                    .collect();
                drop(producers);

                let mut new_list: Vec<PublicKey> = if epoch <= 1 {
                    // Genesis/early epochs: all active producers qualify
                    active
                } else {
                    // Producers who attested in the previous epoch are included.
                    // Scan previous epoch blocks for presence_root attestation data.
                    // INC-I-010: If ANY block is missing (e.g., after snap sync),
                    // skip filtering — use all active, same as epoch 0/1.
                    // Self-healing: once a full epoch of blocks exists, filtering resumes.
                    let prev_epoch_start = (epoch - 1) * blocks_per_epoch;
                    let prev_epoch_end = epoch * blocks_per_epoch;
                    let mut attested: HashSet<PublicKey> = HashSet::new();
                    let mut have_full_epoch = true;

                    let mut sorted_for_decode = active.clone();
                    sorted_for_decode.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                    for h in prev_epoch_start..prev_epoch_end {
                        if let Ok(Some(blk)) = self.block_store.get_block_by_height(h) {
                            // Block producer attested by producing
                            attested.insert(blk.header.producer);
                            // Decode presence_root for explicit attestations
                            if !blk.header.presence_root.is_zero() {
                                let indices = decode_attestation_bitfield(
                                    &blk.header.presence_root,
                                    sorted_for_decode.len(),
                                );
                                for idx in indices {
                                    if let Some(pk) = sorted_for_decode.get(idx) {
                                        attested.insert(*pk);
                                    }
                                }
                            }
                        } else {
                            // Missing block — incomplete epoch history (snap sync).
                            // Cannot reliably filter by attestation.
                            have_full_epoch = false;
                            break;
                        }
                    }

                    let skip_height = self.config.network.params().snap_attestation_skip_height;
                    if have_full_epoch || height < skip_height {
                        // Full history: filter by attestation (normal path).
                        // Before activation height: always filter (old behavior).
                        active
                            .into_iter()
                            .filter(|pk| attested.contains(pk))
                            .collect()
                    } else {
                        info!(
                            "[EPOCH] Incomplete block history for epoch {} — using all {} active producers",
                            epoch - 1,
                            active.len()
                        );
                        active
                    }
                };

                // Deadlock safety: if no one attested, include everyone
                if new_list.is_empty() {
                    let producers = self.producer_set.read().await;
                    new_list = producers
                        .active_producers_at_height(height)
                        .iter()
                        .map(|p| p.public_key)
                        .collect();
                }

                new_list.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                info!(
                    "[EPOCH] Frozen producer list for epoch {}: {} producers",
                    epoch,
                    new_list.len()
                );
                self.epoch_producer_list = new_list;

                // Tier system: build active production list.
                // Before activation: all producers in round-robin (identical to epoch_producer_list).
                // After activation: first ACTIVE_PRODUCERS_CAP by registered_at (earliest first).
                // Attestors outside the cap still receive bond-weighted epoch rewards.
                use doli_core::consensus::{TIER_SYSTEM_ACTIVATION_HEIGHT, ACTIVE_PRODUCERS_CAP};
                if height >= TIER_SYSTEM_ACTIVATION_HEIGHT
                    && self.epoch_producer_list.len() > ACTIVE_PRODUCERS_CAP
                {
                    let producers = self.producer_set.read().await;
                    let mut with_reg: Vec<(PublicKey, u64)> = self
                        .epoch_producer_list
                        .iter()
                        .filter_map(|pk| {
                            producers.get_by_pubkey(pk).map(|p| (*pk, p.registered_at))
                        })
                        .collect();
                    drop(producers);
                    // Sort by registered_at ascending (earliest first), pubkey tiebreak
                    with_reg.sort_by(|a, b| {
                        a.1.cmp(&b.1)
                            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
                    });
                    self.active_production_list = with_reg
                        .iter()
                        .take(ACTIVE_PRODUCERS_CAP)
                        .map(|(pk, _)| *pk)
                        .collect();
                    info!(
                        "[TIER] Active production list: {}/{} producers (by registered_at)",
                        self.active_production_list.len(),
                        self.epoch_producer_list.len()
                    );
                } else {
                    self.active_production_list = self.epoch_producer_list.clone();
                }

                // Clear mid-epoch exclusions — fresh start for new epoch
                self.excluded_producers.clear();
                // INC-I-010 layer 3: epoch_producer_list is now rebuilt with
                // attestation filtering — end the post-snap Light-mode window.
                if self.snap_sync_height.is_some() {
                    info!("[SNAP_SYNC] Epoch boundary reached — switching gossip validation to Full mode");
                    self.snap_sync_height = None;
                }
            }
        }

        // ON-CHAIN LIVENESS: read missed_producers from header (deterministic).
        // This replaces the old local gap analysis that could diverge between nodes.
        {
            // EXCLUDE: producers listed in header.missed_producers
            for pk in &block.header.missed_producers {
                if self.excluded_producers.insert(*pk) {
                    info!(
                        "[LIVENESS] EXCLUDED {} — missed slot (from header at h={})",
                        hex::encode(&pk.as_bytes()[..4]),
                        height
                    );
                }
            }

            // RE-INCLUSION: handled at epoch boundary only (line 136-138 above).
            // Mid-epoch re-inclusion by attestation was removed because it causes
            // an exclude→re-include→miss→exclude cycle that prevents the block rate
            // from reaching 100% when producers are offline. Excluded producers
            // stay excluded until the next epoch, where the frozen list is rebuilt
            // from attestation data. They still earn rewards if they attest.
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
            if let Ok(data) = bincode::serialize(block) {
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
    }
}
