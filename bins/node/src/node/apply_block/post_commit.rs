use super::*;

impl Node {
    /// Actions performed after the batch commit: tier recompute, epoch snapshot,
    /// attestation, archive buffering, and websocket broadcast.
    pub(super) async fn post_commit_actions(
        &mut self,
        block: &Block,
        block_hash: Hash,
        height: u64,
    ) {
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
            // This snapshot is used for REWARDS (bond-weighted distribution).
            // Round-robin production uses active_producers directly.
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
                self.cached_scheduler = None;
            }

            // LIVENESS FILTER: scan completed epoch, find who produced blocks.
            // Producers who produced 0 blocks are excluded from round-robin
            // in the next epoch. Re-included when they produce or attest again.
            // Deterministic: all nodes read the same blocks on-chain.
            if epoch > 1 {
                let epoch_start = (epoch - 1) * blocks_per_epoch;
                let epoch_end = epoch * blocks_per_epoch;
                let mut producers_who_produced: HashSet<PublicKey> = HashSet::new();

                for h in epoch_start..epoch_end {
                    if let Ok(Some(blk)) = self.block_store.get_block_by_height(h) {
                        producers_who_produced.insert(blk.header.producer);
                    }
                }

                // Also count attestations (presence_root bitfield)
                let producers_read = self.producer_set.read().await;
                let active = producers_read.active_producers_at_height(height);
                let mut sorted_active: Vec<&storage::producer::ProducerInfo> = active.to_vec();
                sorted_active.sort_by(|a, b| a.public_key.as_bytes().cmp(b.public_key.as_bytes()));
                let producer_count = sorted_active.len();

                let mut attested_pks: HashSet<PublicKey> = HashSet::new();
                for h in epoch_start..epoch_end {
                    if let Ok(Some(blk)) = self.block_store.get_block_by_height(h) {
                        if !blk.header.presence_root.is_zero() {
                            let indices = decode_attestation_bitfield(
                                &blk.header.presence_root,
                                producer_count,
                            );
                            for idx in indices {
                                if let Some(p) = sorted_active.get(idx) {
                                    attested_pks.insert(p.public_key);
                                }
                            }
                        }
                    }
                }
                drop(producers_read);

                // Build excluded set
                let mut excluded: Vec<PublicKey> = Vec::new();
                let producers_read = self.producer_set.read().await;
                let active = producers_read.active_producers_at_height(height);
                for p in &active {
                    let produced = producers_who_produced.contains(&p.public_key);
                    let attested = attested_pks.contains(&p.public_key);
                    if !produced && !attested {
                        excluded.push(p.public_key);
                        info!(
                            "[LIVENESS] EXCLUDED {} from epoch {} round-robin (0 blocks, 0 attestations)",
                            hex::encode(&p.public_key.as_bytes()[..4]),
                            epoch
                        );
                    }
                }
                drop(producers_read);

                if !excluded.is_empty() {
                    warn!(
                        "[LIVENESS] {} producer(s) excluded from epoch {} for inactivity",
                        excluded.len(),
                        epoch
                    );
                }
                self.excluded_producers = excluded.into_iter().collect();
            }

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
