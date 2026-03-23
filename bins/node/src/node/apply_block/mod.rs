use super::*;

mod genesis_completion;
mod governance;
mod post_commit;
mod state_update;
mod tx_processing;

impl Node {
    /// Apply a block to the chain
    ///
    /// `mode`: `Full` for gossip/production (checks MAX_PAST_SLOTS, VDF),
    ///         `Light` for sync/reorg (skips time-based checks that reject old blocks).
    pub(super) async fn apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()> {
        let block_hash = block.hash();

        let height = self.chain_state.read().await.best_height + 1;

        // Defense-in-depth: skip blocks we already have AND have applied.
        // Prevents height double-counting if sync delivers stored blocks (ISSUE-5).
        //
        // Critical: We must check that the block was actually APPLIED (chain state
        // advanced past it), not just stored. A block can be in the store but not
        // applied if a previous apply_block() wrote the block then failed during
        // UTXO validation (block store poisoning — see N4 incident 2026-03-13).
        // In that case, we allow re-apply — put_block() will overwrite harmlessly.
        if self.block_store.has_block(&block_hash)? {
            if let Some(stored_height) = self.block_store.get_height_by_hash(&block_hash)? {
                if stored_height < height {
                    // Block is in store but chain state didn't advance past it.
                    // This is a poisoned block from a failed apply — proceed with
                    // re-validation and re-apply (put_block overwrites safely).
                    warn!(
                        "Block {} in store at height {} but chain at height {} — \
                         poisoned block, re-applying",
                        block_hash,
                        stored_height,
                        height - 1
                    );
                } else {
                    // Block is in store AND chain state is at or past it — truly applied.
                    debug!(
                        "Block {} already applied at height {}, skipping",
                        block_hash, stored_height
                    );
                    return Ok(());
                }
            } else {
                // Block in store but no height index — proceed with apply.
                warn!(
                    "Block {} in store but no height index — re-applying",
                    block_hash
                );
            }
        }

        self.validate_block_for_apply(&block, height, mode).await?;
        self.validate_block_economics(&block, height, mode).await?;

        info!("Applying block {} at height {}", block_hash, height);

        // NOTE: Block store write is deferred to AFTER all transaction validation.
        // Writing here would poison the store if UTXO validation fails later —
        // the "already in store" check would then skip the block forever.
        // See: N4 stuck-at-22888 incident (2026-03-13).

        // Update producer liveness tracker (for scheduling filter)
        self.producer_liveness.insert(block.header.producer, height);

        // Track newly registered producers to add to known_producers after lock release
        let mut new_registrations: Vec<PublicKey> = Vec::new();

        // Deferred protocol activation (verified in tx loop, applied when chain_state lock acquired)
        let mut pending_protocol_activation_data: Option<(u32, u64)> = None;

        // Create BlockBatch for atomic persistence (all state changes in one WriteBatch)
        // Clone the Arc so batch's borrow is independent of self (allows &mut self method calls)
        let state_db = Arc::clone(&self.state_db);
        let mut batch = state_db.begin_batch();

        // Track dirty producers for efficient batch writes
        let mut dirty_producer_keys: std::collections::HashSet<Hash> =
            std::collections::HashSet::new();
        let removed_producer_keys: std::collections::HashSet<Hash> =
            std::collections::HashSet::new();
        let dirty_exit_keys: std::collections::HashSet<Hash> = std::collections::HashSet::new();

        // Undo log: snapshot ProducerSet BEFORE any mutations
        let undo_producer_snapshot = {
            let producers = self.producer_set.read().await;
            bincode::serialize(&*producers).unwrap_or_default()
        };

        // Undo log: track UTXO changes for this block
        let mut undo_spent_utxos: Vec<(storage::Outpoint, storage::UtxoEntry)> = Vec::new();
        let mut undo_created_utxos: Vec<storage::Outpoint> = Vec::new();

        // Apply transactions to UTXO set and process special transactions
        {
            let mut utxo = self.utxo_set.write().await;
            let mut producers = self.producer_set.write().await;

            for (tx_index, tx) in block.transactions.iter().enumerate() {
                // Process UTXO changes (in-memory + batch for atomic persistence)
                self.process_transaction_utxos(
                    tx,
                    tx_index,
                    height,
                    block.header.slot,
                    &mut utxo,
                    &mut batch,
                    &mut undo_spent_utxos,
                    &mut undo_created_utxos,
                )?;

                // Process producer-related effects (registrations, exits, bonds, etc.)
                self.process_transaction_producer_effects(
                    tx,
                    height,
                    block.header.slot,
                    &utxo,
                    &mut producers,
                    &mut dirty_producer_keys,
                    &mut new_registrations,
                );

                // Process governance transactions (maintainer changes, protocol activation)
                if let Some(activation) = self
                    .process_transaction_governance(tx, height, &producers)
                    .await
                {
                    pending_protocol_activation_data = Some(activation);
                }
            }

            // Process completed unbonding periods
            Self::process_unbonding(&mut producers, height);
        }

        // Update known producers for bootstrap round-robin
        self.update_known_producers(new_registrations, height).await;

        // Capture previous slot BEFORE update_chain_state_for_block changes best_slot.
        // Used by reactive scheduling to detect missed slots.
        let prev_slot = if height <= 1 {
            0u32
        } else {
            match self
                .block_store
                .get_block_by_height(height - 1)
                .ok()
                .flatten()
                .map(|b| b.header.slot)
            {
                Some(slot) => slot,
                None => {
                    // After snap sync, block store is empty but chain_state.best_slot
                    // holds the correct slot from the snapshot. Defaulting to 0 would
                    // create a false gap of ~current_slot, triggering mass unscheduling
                    // of all producers via the missed-slot loop (INC-I-005).
                    self.chain_state.read().await.best_slot
                }
            }
        };

        // Update chain state (height, hash, slot, protocol activation, genesis time, state root)
        self.update_chain_state_for_block(
            &block,
            block_hash,
            height,
            pending_protocol_activation_data,
        )
        .await;

        // Track finality and apply deferred producer updates at epoch boundaries
        let needs_full_producer_write = self
            .track_finality_and_apply_deferred(block_hash, height, block.header.slot)
            .await;

        // NOTE: recompute_tier + EpochSnapshot + attestation moved after batch commit
        // to avoid borrow conflict with BlockBatch (which borrows self.state_db).

        // Genesis end: derive and register producers with REAL bonds from the blockchain
        let genesis_needs_full_write = self
            .maybe_complete_genesis(
                height,
                &mut batch,
                &mut undo_spent_utxos,
                &mut undo_created_utxos,
            )
            .await?;
        let mut needs_full_producer_write = needs_full_producer_write || genesis_needs_full_write;

        // =====================================================================
        // REACTIVE ROUND-ROBIN SCHEDULING
        //
        // Update the `scheduled` flag on producers based on on-chain evidence:
        // 1. Detect missed slots → unschedule rank 0 producers who didn't produce
        // 2. Block producer → schedule (proved alive by producing this block)
        // 3. Attestation bitfield → schedule attesters (proved alive by attesting)
        //
        // All data is from block headers (on-chain), so every node computes the
        // same result. This is NOT local gossip state — it's deterministic.
        // =====================================================================
        {
            let current_slot = block.header.slot;

            // Build the scheduler as it was BEFORE this block (same state that
            // validated this block). Used to determine who was rank 0 for this slot.
            let missed_slot_scheduler = {
                let producers = self.producer_set.read().await;
                let scheduled: Vec<(PublicKey, u64)> = producers
                    .scheduled_producers_at_height(height)
                    .iter()
                    .map(|p| (p.public_key, 1u64))
                    .collect();
                if scheduled.is_empty() {
                    None
                } else {
                    Some(doli_core::consensus::select_producer_for_slot(
                        current_slot,
                        &scheduled,
                    ))
                }
            };

            let mut producers = self.producer_set.write().await;
            let mut scheduling_changed = false;

            // 1. Detect missed slots: if current_slot > prev_slot + 1, slots were skipped.
            //    Cap at 100 iterations to prevent DoS from large slot gaps (e.g., after
            //    extended downtime). Beyond 100, mass-unscheduling is wrong anyway.
            if current_slot > prev_slot + 1 && height > 1 {
                let scheduled_list: Vec<(PublicKey, u64)> = producers
                    .scheduled_producers_at_height(height)
                    .iter()
                    .map(|p| (p.public_key, 1u64))
                    .collect();

                if !scheduled_list.is_empty() {
                    let gap = (current_slot - prev_slot - 1) as usize;
                    let max_missed = gap.min(100);
                    for i in 0..max_missed {
                        let missed_slot = prev_slot + 1 + i as u32;
                        let eligible = doli_core::consensus::select_producer_for_slot(
                            missed_slot,
                            &scheduled_list,
                        );
                        if let Some(rank0) = eligible.first() {
                            if producers.unschedule_producer(rank0) {
                                info!(
                                    "Reactive scheduling: unscheduling {} (missed slot {})",
                                    rank0, missed_slot
                                );
                                scheduling_changed = true;
                            }
                        }
                    }
                }
            }

            // 2. If this block was produced by a fallback (not rank 0), rank 0 missed
            if let Some(ref eligible) = missed_slot_scheduler {
                if let Some(expected_rank0) = eligible.first() {
                    if *expected_rank0 != block.header.producer
                        && producers.unschedule_producer(expected_rank0)
                    {
                        info!(
                            "Reactive scheduling: unscheduling {} (fallback produced slot {})",
                            expected_rank0, current_slot
                        );
                        scheduling_changed = true;
                    }
                }
            }

            // 3. Block producer proved alive → schedule them
            if producers.schedule_producer(&block.header.producer) {
                scheduling_changed = true;
            }

            // 4. Decode presence_root attestation bitfield → re-enter attesters
            if !block.header.presence_root.is_zero() {
                let mut sorted_producers: Vec<PublicKey> = producers
                    .active_producers_at_height(height)
                    .iter()
                    .map(|p| p.public_key)
                    .collect();
                sorted_producers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                let attested_indices = doli_core::attestation::decode_attestation_bitfield(
                    &block.header.presence_root,
                    sorted_producers.len(),
                );

                for idx in attested_indices {
                    if idx < sorted_producers.len() {
                        let attester = sorted_producers[idx];
                        if producers.schedule_producer(&attester) {
                            info!(
                                "Reactive scheduling: re-scheduling {} (attested in slot {})",
                                attester, current_slot
                            );
                            scheduling_changed = true;
                        }
                    }
                }
            }

            if scheduling_changed {
                needs_full_producer_write = true;
            }
            drop(producers);
        }

        // FIX C1: Recompute state root AFTER reactive scheduling mutations.
        // The initial state root (computed in update_chain_state_for_block) was cached
        // BEFORE scheduling flags changed. We must recompute so snap sync nodes get
        // a root that matches the persisted ProducerSet (which includes scheduled flags).
        if needs_full_producer_write {
            let state = self.chain_state.read().await;
            let utxo = self.utxo_set.read().await;
            let ps = self.producer_set.read().await;
            if let Ok(root) = storage::compute_state_root(&state, &utxo, &ps) {
                let mut cache = self.cached_state_root.write().await;
                *cache = Some((root, state.best_hash, state.best_height));
            }
        }

        // Remove transactions from mempool and prune any with now-spent inputs
        {
            let mut mempool = self.mempool.write().await;
            mempool.remove_for_block(&block.transactions);
            let utxo = self.utxo_set.read().await;
            let height = self.chain_state.read().await.best_height;
            mempool.revalidate(&utxo, height);
        }

        // Get producer's effective weight for fork choice rule
        let producer_weight = {
            let producers = self.producer_set.read().await;
            producers
                .get_by_pubkey(&block.header.producer)
                .map(|p| p.effective_weight(height))
                .unwrap_or(1) // Default weight 1 for unknown producers (bootstrap)
        };

        // Update sync manager with weight for fork choice rule
        self.sync_manager.write().await.block_applied_with_weight(
            block_hash,
            height,
            block.header.slot,
            producer_weight,
            block.header.prev_hash,
        );

        // Store the block AFTER all transaction validation has passed.
        // This prevents block store poisoning: if UTXO validation fails, the block
        // is NOT in the store, so sync can retry it cleanly on the next attempt.
        // Previously this was before the tx loop, causing the N4 stuck-at-22888 bug.
        self.block_store.put_block(&block, height)?;
        self.block_store.set_canonical_chain(block_hash, height)?;

        // Atomic state persistence via StateDb WriteBatch.
        // All state changes (UTXOs, chain_state, producers) committed in one batch.
        // Crash between any two writes is impossible — it's all-or-nothing.
        {
            let state = self.chain_state.read().await;
            batch.put_chain_state(&state);
            batch.set_last_applied(height, block_hash, state.best_slot);

            let producers = self.producer_set.read().await;
            if needs_full_producer_write {
                batch.write_full_producer_set(&producers);
            } else {
                batch.write_dirty_producers(
                    &producers,
                    &dirty_producer_keys,
                    &removed_producer_keys,
                    &dirty_exit_keys,
                );
            }
        }
        batch
            .commit()
            .map_err(|e| anyhow::anyhow!("StateDb batch commit failed: {}", e))?;

        // Persist undo data for this block (enables O(depth) rollbacks)
        let undo = storage::UndoData {
            spent_utxos: undo_spent_utxos,
            created_utxos: undo_created_utxos,
            producer_snapshot: undo_producer_snapshot,
        };
        self.state_db.put_undo(height, &undo);

        // Prune old undo data (keep last MAX_REORG_DEPTH blocks)
        const UNDO_KEEP_DEPTH: u64 = 2000; // 2x MAX_REORG_DEPTH for safety margin
        if height > UNDO_KEEP_DEPTH {
            self.state_db.prune_undo_before(height - UNDO_KEEP_DEPTH);
        }

        // Post-commit: tier recompute, epoch snapshot, attestation, archive, websocket
        self.post_commit_actions(&block, block_hash, height).await;

        // Note: Don't broadcast here. Blocks received from the network should not be
        // re-broadcast (they already came from the network). Locally produced blocks
        // should be broadcast explicitly by the caller (produce_block).

        Ok(())
    }
}
