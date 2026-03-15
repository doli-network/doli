use super::*;

impl Node {
    /// Add newly registered producers to known_producers for bootstrap round-robin.
    pub(super) async fn update_known_producers(
        &mut self,
        new_registrations: Vec<PublicKey>,
        height: u64,
    ) {
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
    }

    /// Update chain state after transaction processing.
    ///
    /// Handles: height/hash/slot update, snap sync clear, protocol activation,
    /// genesis timestamp, and state root caching.
    pub(super) async fn update_chain_state_for_block(
        &mut self,
        block: &Block,
        block_hash: Hash,
        height: u64,
        pending_protocol_activation_data: Option<(u32, u64)>,
    ) {
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

    /// Track block for finality and apply deferred producer updates at epoch boundaries.
    ///
    /// Returns whether a full producer write is needed for the batch.
    pub(super) async fn track_finality_and_apply_deferred(
        &mut self,
        block_hash: Hash,
        height: u64,
        block_slot: u32,
    ) -> bool {
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
                sync.track_block_for_finality(block_hash, height, block_slot, total_weight);
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

        needs_full_producer_write
    }
}
