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
        }

        // LIVE LIVENESS FILTER: exclude on missed slot, re-include on attestation.
        {
            let prev_slot = if height > 1 {
                if let Ok(Some(prev_block)) = self.block_store.get_block_by_height(height - 1) {
                    prev_block.header.slot
                } else {
                    block.header.slot.saturating_sub(1)
                }
            } else {
                0
            };

            let slot_gap = block.header.slot.saturating_sub(prev_slot);

            // EXCLUDE: producers who missed skipped slots
            //
            // Safety caps (INC-I-017):
            // 1. MAX_EXCLUSIONS_PER_BLOCK: A single large slot gap (e.g., 24 slots
            //    during a gossip partition) must not mass-exclude the entire producer
            //    set. Capped at 3 per block — genuine offline producers accumulate
            //    exclusions over multiple blocks; a gossip hiccup can't nuke the set.
            // 2. MAX_EXCLUDED_FRACTION (1/3): Even accumulated exclusions can't push
            //    the round-robin denominator below 2/3 of normal. This bounds how far
            //    the schedule can diverge between nodes with slightly different views.
            //
            // Without these caps, a single 24-slot gap excluded 23/32 producers,
            // changed slot%30 → slot%8, permanently diverging the scheduler.
            const MAX_EXCLUSIONS_PER_BLOCK: usize = 3;

            if slot_gap > 1 && height > 36 {
                let producers = self.producer_set.read().await;
                let active: Vec<PublicKey> = producers
                    .active_producers_at_height(height)
                    .iter()
                    .map(|p| p.public_key)
                    .collect();
                drop(producers);

                let active_count = active.len();
                let max_excluded_total = active_count / 3; // never exclude >1/3

                let mut sorted = active;
                sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                let rr_list: Vec<&PublicKey> = sorted
                    .iter()
                    .filter(|pk| !self.excluded_producers.contains(pk))
                    .collect();
                let rr_count = rr_list.len();

                if rr_count > 0 {
                    let mut exclusions_this_block = 0usize;
                    for skipped_slot in (prev_slot + 1)..block.header.slot {
                        if exclusions_this_block >= MAX_EXCLUSIONS_PER_BLOCK {
                            if exclusions_this_block == MAX_EXCLUSIONS_PER_BLOCK {
                                warn!(
                                    "[LIVENESS] Exclusion cap hit ({}/block) — {} more skipped slots ignored (gap={})",
                                    MAX_EXCLUSIONS_PER_BLOCK,
                                    block.header.slot.saturating_sub(skipped_slot),
                                    slot_gap
                                );
                            }
                            break;
                        }
                        if self.excluded_producers.len() >= max_excluded_total {
                            warn!(
                                "[LIVENESS] Total exclusion cap hit ({}/{} active) — skipping remaining",
                                self.excluded_producers.len(),
                                active_count
                            );
                            break;
                        }
                        let idx = (skipped_slot as usize) % rr_count;
                        let missed = *rr_list[idx];
                        if !self.excluded_producers.contains(&missed) {
                            self.excluded_producers.insert(missed);
                            exclusions_this_block += 1;
                            info!(
                                "[LIVENESS] EXCLUDED {} — missed slot {}",
                                hex::encode(&missed.as_bytes()[..4]),
                                skipped_slot
                            );
                        }
                    }
                }
            }

            // RE-INCLUDE: producers who attested in this block
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
