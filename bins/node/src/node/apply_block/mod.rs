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
    pub async fn apply_block(&mut self, block: Block, mode: ValidationMode) -> Result<()> {
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

        // NOTE: active status recompute + EpochSnapshot + attestation moved after batch commit
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
        let needs_full_producer_write = needs_full_producer_write || genesis_needs_full_write;

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

        // Reset cumulative rollback depth — a successful block application means
        // the chain is advancing, so any prior rollback sequence is resolved.
        self.cumulative_rollback_depth = 0;

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

        // Include undo data in the same atomic batch (avoids separate WAL entry)
        let undo = storage::UndoData {
            spent_utxos: undo_spent_utxos,
            created_utxos: undo_created_utxos,
            producer_snapshot: undo_producer_snapshot,
        };
        batch.put_undo(height, &undo);

        batch
            .commit()
            .map_err(|e| anyhow::anyhow!("StateDb batch commit failed: {}", e))?;

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
