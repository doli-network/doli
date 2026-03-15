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
