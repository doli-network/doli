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
                    let prev_epoch_start = (epoch - 1) * blocks_per_epoch;
                    let prev_epoch_end = epoch * blocks_per_epoch;
                    let mut attested: HashSet<PublicKey> = HashSet::new();

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
                        }
                    }

                    // Include: attested producers + newly registered (not in prev epoch)
                    active
                        .into_iter()
                        .filter(|pk| {
                            attested.contains(pk) || {
                                // Newly registered producers get a free pass
                                // (they weren't in the previous epoch's active set)
                                // Check by seeing if they were active at prev epoch start
                                false // Conservative: rely on attestation
                            }
                        })
                        .collect()
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
                // Clear mid-epoch exclusions — fresh start for new epoch
                self.excluded_producers.clear();
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

            // RE-INCLUDE: producers who attested in this block via presence_root
            if !self.excluded_producers.is_empty() && !block.header.presence_root.is_zero() {
                let sorted_pks: Vec<PublicKey> = {
                    let producers = self.producer_set.read().await;
                    let active = producers.active_producers_at_height(height);
                    let mut pks: Vec<PublicKey> = active.iter().map(|p| p.public_key).collect();
                    pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                    pks
                };

                let indices =
                    decode_attestation_bitfield(&block.header.presence_root, sorted_pks.len());
                for idx in indices {
                    if let Some(pk) = sorted_pks.get(idx) {
                        if self.excluded_producers.remove(pk) {
                            info!(
                                "[LIVENESS] RE-INCLUDED {} — attested at h={}",
                                hex::encode(&pk.as_bytes()[..4]),
                                height
                            );
                        }
                    }
                }
            }
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
