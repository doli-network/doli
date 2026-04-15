use super::*;

impl Node {
    /// Unconditionally roll back 1 block for fork recovery.
    ///
    /// Unlike `resolve_shallow_fork()`, this has no `empty_headers` or
    /// `shallow_rollback_count` preconditions — it just rolls back. Used by
    /// `maybe_auto_resync()` on mainnet where same-height forks never trigger
    /// empty header responses (peers are at the same height, not ahead).
    ///
    /// Returns `Ok(true)` if rollback succeeded, `Ok(false)` if at height 0.
    pub async fn rollback_one_block(&mut self) -> Result<bool> {
        let local_height = {
            let sync = self.sync_manager.read().await;
            sync.local_tip().0
        };

        // Log all context that led to this rollback being initiated.
        // The caller (resolve_shallow_fork, auto_resync) provides the trigger;
        // this captures the numeric state for post-incident root-cause analysis.
        {
            let sync = self.sync_manager.read().await;
            let empty_headers = sync.consecutive_empty_headers();
            let best_peer_h = sync.best_peer_height();
            let gap = best_peer_h.saturating_sub(local_height);
            info!(
                "[ROLLBACK] Initiating: depth={} local_h={} target_h={} gap={} empty_headers={} shallow_count={}",
                self.cumulative_rollback_depth + 1,
                local_height,
                local_height.saturating_sub(1),
                gap,
                empty_headers,
                self.shallow_rollback_count
            );
        }

        if local_height == 0 {
            return Ok(false);
        }

        let target_height = local_height - 1;

        // Fix 3: Never rollback to genesis from an established chain.
        // Rolling back to height 0 destroys all chain state and is never the right
        // recovery action for a running node. If we're at height 1, the chain is
        // effectively at genesis — there's nothing useful to rollback to.
        if target_height == 0 && local_height > 1 {
            warn!(
                "Refusing rollback to genesis from height {} — would destroy chain state. \
                 Manual intervention required (recover --yes).",
                local_height
            );
            return Ok(false);
        }

        // Fix 4: Cap cumulative rollback depth at 50 blocks.
        // Prevents cascading rollbacks from gradually eroding the chain back to genesis.
        // After 50 rollbacks without a successful block application, the fork is too
        // deep for rollback-based recovery — manual intervention or sync is needed.
        const MAX_CUMULATIVE_ROLLBACK: u32 = 50;
        if self.cumulative_rollback_depth >= MAX_CUMULATIVE_ROLLBACK {
            warn!(
                "Refusing rollback: cumulative depth {} reached limit {} — \
                 too deep for rollback recovery. Waiting for sync or manual intervention.",
                self.cumulative_rollback_depth, MAX_CUMULATIVE_ROLLBACK
            );
            return Ok(false);
        }

        // Invalidate genesis producer cache if rollback crosses genesis boundary
        let genesis_blocks = self.config.network.genesis_blocks();
        if genesis_blocks > 0 && target_height <= genesis_blocks {
            info!("[ROLLBACK] Crossing genesis boundary — invalidating genesis producer cache");
            self.cached_genesis_producers = std::sync::OnceLock::new();
        }

        let genesis_hash = self.chain_state.read().await.genesis_hash;

        let (parent_hash, parent_slot) = if target_height == 0 {
            (genesis_hash, 0u32)
        } else {
            match self.block_store.get_block_by_height(target_height)? {
                Some(parent_block) => (parent_block.hash(), parent_block.header.slot),
                None => {
                    error!("Cannot rollback: no block at height {}", target_height);
                    return Ok(false);
                }
            }
        };

        info!(
            "Rolling back from height {} to {} for fork recovery",
            local_height, target_height
        );

        // Try undo-based rollback first (O(1) for single block)
        if let Some(undo) = self.state_db.get_undo(local_height) {
            info!(
                "Undo-based rollback: reverting block at height {}",
                local_height
            );
            {
                let mut utxo = self.utxo_set.write().await;

                // Remove UTXOs created by this block
                for outpoint in &undo.created_utxos {
                    utxo.remove(outpoint)?;
                }

                // Restore UTXOs spent by this block
                for (outpoint, entry) in &undo.spent_utxos {
                    utxo.insert(*outpoint, entry.clone())?;
                }
            }

            // Restore ProducerSet from undo snapshot
            if let Ok(restored_producers) =
                bincode::deserialize::<storage::ProducerSet>(&undo.producer_snapshot)
            {
                let mut producers = self.producer_set.write().await;
                *producers = restored_producers;
            } else {
                warn!("Failed to deserialize producer snapshot, rebuilding from blocks");
                let mut producers = self.producer_set.write().await;
                self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
            }
        } else {
            // Legacy fallback: rebuild from genesis (no undo data)
            warn!(
                "No undo data for height {} — falling back to rebuild from genesis",
                local_height
            );

            // Pre-check: can't rebuild from genesis without block 1.
            // Do NOT snap sync — it destroys the block store further.
            // Skip the rollback and let header-first sync recover.
            if self.block_store.get_block_by_height(1)?.is_none() {
                warn!("Rollback: block 1 missing — cannot rebuild. Skipping rollback, header-first sync will recover.");
                return Ok(true);
            }

            let genesis_producers = if genesis_blocks > 0 && target_height > genesis_blocks {
                self.derive_genesis_producers_from_chain()
            } else {
                Vec::new()
            };
            let bond_unit = self.config.network.bond_unit();
            {
                let mut utxo = self.utxo_set.write().await;
                utxo.clear();
                for height in 1..=target_height {
                    let block = self
                        .block_store
                        .get_block_by_height(height)?
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Rollback UTXO rebuild: missing block at height {}",
                                height
                            )
                        })?;
                    for (tx_index, tx) in block.transactions.iter().enumerate() {
                        let is_reward_tx = tx_index == 0 && tx.is_reward_minting();
                        if !is_reward_tx {
                            let _ = utxo.spend_transaction(tx);
                        }
                        utxo.add_transaction(tx, height, is_reward_tx, block.header.slot)?;
                    }
                    if genesis_blocks > 0 && height == genesis_blocks + 1 {
                        Self::consume_genesis_bond_utxos(
                            &mut utxo,
                            &genesis_producers,
                            bond_unit,
                            height,
                        )?;
                    }
                }
            }

            {
                let mut producers = self.producer_set.write().await;
                self.rebuild_producer_set_from_blocks(&mut producers, target_height)?;
            }
        }

        // Rebuild producer liveness map from canonical block_store.
        // Critical: rollback does NOT undo liveness entries from fork blocks,
        // causing nodes to have divergent live_producers lists and conflicting
        // round-robin assignments. Rebuilding from block_store ensures all nodes
        // converge on the same liveness view.
        self.rebuild_producer_liveness(target_height);

        // Update chain state to parent
        {
            let mut state = self.chain_state.write().await;
            state.best_height = target_height;
            state.best_hash = parent_hash;
            state.best_slot = parent_slot;
        }

        // Update sync manager: local tip + reset fork signals
        {
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(target_height, parent_hash, parent_slot);
            sync.reset_sync_for_rollback();
            // Fix #2b-bis (2026-04-15, synmgrefactor): record post-rollback
            // height so that note_orphan_gossip_block can detect the
            // "applied since rollback → behind, not forked" case and skip
            // further rollback signals.
            sync.note_rollback_completed(target_height);
        }

        // Atomically persist the rolled-back state via StateDb.
        // Collects all UTXOs from in-memory set and writes everything in one WriteBatch.
        {
            let state = self.chain_state.read().await;
            let producers = self.producer_set.read().await;
            let utxo = self.utxo_set.read().await;
            let utxo_pairs: Vec<(storage::Outpoint, storage::UtxoEntry)> = match &*utxo {
                UtxoSet::InMemory(mem) => mem.iter().map(|(o, e)| (*o, e.clone())).collect(),
                // RocksDb variant shouldn't occur (we load InMemory from StateDb), but handle it
                UtxoSet::RocksDb(_) => self.state_db.iter_utxos(),
            };
            self.state_db
                .atomic_replace(&state, &producers, utxo_pairs.into_iter())
                .map_err(|e| anyhow::anyhow!("StateDb atomic_replace failed: {}", e))?;
        }

        // Restore epoch scheduler state from undo data (O(1) vs O(chain) rebuild).
        // The undo snapshot was taken BEFORE apply_block, so it reflects the correct
        // scheduler state at the pre-rollback height.
        if let Some(undo) = self.state_db.get_undo(local_height) {
            if let Some(ref epoch_bytes) = undo.epoch_state_snapshot {
                match doli_core::EpochState::deserialize_canonical(epoch_bytes) {
                    Ok(restored) => {
                        info!(
                            "[ROLLBACK] Restored epoch state from undo: epoch={} producers={} active={}",
                            restored.epoch, restored.producer_list.len(), restored.active_list.len()
                        );
                        self.epoch_state = restored;
                    }
                    Err(e) => {
                        warn!("[ROLLBACK] Failed to deserialize epoch state from undo: {} — rebuilding", e);
                        self.rebuild_epoch_state_from_blocks().await;
                    }
                }
            } else {
                // Pre-upgrade undo data (no epoch_state_snapshot) — fall back to rebuild
                info!("[ROLLBACK] No epoch state in undo (pre-upgrade block) — rebuilding");
                self.rebuild_epoch_state_from_blocks().await;
            }
        } else {
            // No undo data at all (legacy path already handled above) — rebuild
            self.rebuild_epoch_state_from_blocks().await;
        }

        // Track cumulative rollback depth (Fix 4)
        self.cumulative_rollback_depth += 1;

        info!(
            "[FORK] ROLLBACK_DONE h={} hash={:.8} cumulative_depth={}",
            target_height, parent_hash, self.cumulative_rollback_depth
        );

        Ok(true)
    }

    /// Detect and resolve shallow forks (1-block orphan tips from ungraceful shutdown).
    ///
    /// When a node is killed during block production, it may persist a block that
    /// peers never received. On restart, peers don't recognize our tip hash, causing
    /// consecutive empty header responses. If the gap to peers is small (<100 blocks),
    /// this is a shallow fork — not a deep fork needing genesis resync.
    ///
    /// Fix: roll back one block at a time until we reach a tip peers recognize.
    /// Capped at 10 rollbacks — if the fork is deeper than that, it's not shallow.
    /// Returns `true` if a rollback was performed (caller should skip other periodic tasks).
    pub async fn resolve_shallow_fork(&mut self) -> Result<bool> {
        let (empty_headers, local_height, gap, last_applied_secs, stuck_signal) = {
            let mut sync = self.sync_manager.write().await;
            let gap = sync.network_tip_height().saturating_sub(sync.local_tip().0);
            let last_applied = sync.last_block_applied_secs();
            // Fix #2c (2026-04-15): consume the stuck_fork_signal set by
            // note_orphan_gossip_block (Fix #2b). The signal is stronger
            // evidence than the empty_headers/last_applied heuristics —
            // peers.rs already confirmed we received 3+ orphan gossip
            // blocks (our tip is on a minority fork).
            let stuck = sync.take_stuck_fork_signal();
            (
                sync.consecutive_empty_headers(),
                sync.local_tip().0,
                gap,
                last_applied,
                stuck,
            )
        };

        if local_height == 0 {
            return Ok(false);
        }

        // Fix #2c: the orphan-path signal bypasses both heuristic gates.
        // Without this, Fix #2b sets the flag but no one consumes it —
        // observed in 2026-04-15 abraham/alessandro fork: signal fired at
        // 12:26:44 but rollback never ran, then snap sync escalated at
        // 12:27:30. resolve_shallow_fork's empty_headers>=3 and
        // last_applied_secs>=300 are for detecting stuckness WITHOUT an
        // external signal; when one exists, they should not block.
        if !stuck_signal {
            // Need at least 3 fork evidence signals before activating
            if empty_headers < 3 {
                return Ok(false);
            }

            // With INC-I-026 (deterministic scheduler) + fork_id, forks between
            // upgraded nodes are impossible. Only rollback if truly stuck for 5+ min.
            // Anything less is gossip timing, not a fork.
            if last_applied_secs < 300 {
                return Ok(false);
            }
        } else {
            info!(
                "[FORK] stuck_fork_signal consumed (gap={}, last_applied={}s, empty_headers={}) — \
                 bypassing heuristic guards (anti-cascade-orphan path)",
                gap, last_applied_secs, empty_headers
            );
        }

        // Don't rollback if we're making progress. If height advanced since
        // last rollback, sync is working — empty headers are just from peers
        // that haven't caught up yet. Only rollback when truly stuck.
        static LAST_ROLLBACK_HEIGHT: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let last_rb_h = LAST_ROLLBACK_HEIGHT.load(std::sync::atomic::Ordering::Relaxed);
        if local_height > last_rb_h && last_rb_h > 0 {
            // Height advanced since last rollback — sync is working, skip rollback
            debug!(
                "[FORK] Skipping rollback: height advanced {} → {} since last rollback",
                last_rb_h, local_height
            );
            LAST_ROLLBACK_HEIGHT.store(local_height, std::sync::atomic::Ordering::Relaxed);
            let mut sync = self.sync_manager.write().await;
            sync.reset_empty_headers();
            return Ok(false);
        }

        // Fast path: small gap (<= 12 blocks) — just rollback 1 block.
        // This changes local_hash to the parent, and the next sync attempt
        // will try GetHeaders with the parent hash. After 1-N rollbacks,
        // we find a hash peers recognize and sync resumes.
        // No binary search needed, no grace check — immediate recovery.
        //
        // Range expanded to 50 to prevent small-to-medium forks from
        // escalating to snap sync, which loses block history and creates
        // a cascade (snap → no block 1 → future rollback fails → re-snap).
        if gap <= 50 && self.shallow_rollback_count < 50 {
            info!(
                "[FORK] ROLLBACK gap={} empties={} h={} rollback_count={} — rolling back 1 block",
                gap,
                empty_headers,
                local_height,
                self.shallow_rollback_count + 1
            );
            let rolled_back = self.rollback_one_block().await?;
            if rolled_back {
                self.shallow_rollback_count += 1;
                LAST_ROLLBACK_HEIGHT.store(
                    local_height.saturating_sub(1),
                    std::sync::atomic::Ordering::Relaxed,
                );
                // Reset empty headers so sync retries with new tip hash
                let mut sync = self.sync_manager.write().await;
                sync.reset_empty_headers();
                return Ok(true);
            }
        }

        // Deep fork or rollback limit reached: use binary search.
        // BUT: if post_recovery_grace is active, fork sync JUST failed and cleared.
        // Re-triggering immediately creates a Sisyphean loop:
        //   fork_sync → bottoms out → grace → 1 header → empty → fork_sync again
        // Let header-first sync make progress during grace instead.
        let grace_active = self.sync_manager.read().await.post_recovery_grace_active();
        if grace_active {
            debug!(
                "resolve_shallow_fork: skipping fork sync escalation — \
                 post_recovery_grace active (gap={}, empty_headers={})",
                gap, empty_headers
            );
            // Do NOT reset empty_headers here — that creates a deadlock where
            // grace prevents fork sync AND resets the counter that would
            // eventually trigger it. Let the counter accumulate so fork sync
            // can activate once grace expires.
            return Ok(false);
        }

        // When fork sync has failed repeatedly, retry with different peers instead
        // of wiping state. A validated chain at height 400+ must NEVER be reset to
        // genesis based on peer behavior — this is a core safety invariant that
        // Ethereum and Bitcoin both enforce.
        if empty_headers >= 9 {
            warn!(
                "[FORK] RETRY empties={} local_h={} gap={} — retrying with different peers",
                empty_headers, local_height, gap
            );
            self.sync_manager.write().await.reset_empty_headers();
            self.sync_manager.write().await.set_post_recovery_grace();
            return Ok(false);
        }

        // Fork sync binary search removed — fork recovery handled by network crate.
        // At this point, rollback-based recovery and retry with different peers have
        // both been attempted. Let header-first sync handle further recovery.
        Ok(false)
    }

    /// State reset recovery: when hash-based sync and fork sync both fail repeatedly,
    /// reset chain state to genesis and let header-first sync rebuild from peers.
    ///
    /// This preserves the block store (blocks are still on disk) but resets the
    /// in-memory state (UTXO set, producer set, chain tip) to genesis. The node
    /// then syncs from height 0, replaying blocks from disk + downloading missing
    /// ones from peers. This always works because it doesn't depend on any peer
    /// recognizing our tip hash.
    #[allow(dead_code)]
    pub async fn state_reset_recovery(&mut self, local_height: u64) -> Result<bool> {
        warn!(
            "State reset recovery: hash-based sync failed after {} attempts. \
             Resetting to genesis for full resync (current h={}).",
            9, local_height
        );

        let genesis_hash = self.chain_state.read().await.genesis_hash;

        // Reset chain state to genesis
        {
            let mut state = self.chain_state.write().await;
            state.best_height = 0;
            state.best_hash = genesis_hash;
            state.best_slot = 0;
        }
        {
            let state = self.chain_state.read().await;
            self.state_db.clear_and_write_genesis(&state);
        }

        // Clear in-memory state — will be rebuilt from blocks during resync
        *self.utxo_set.write().await = storage::UtxoSet::new();
        *self.producer_set.write().await = storage::ProducerSet::new();

        // Reset sync manager
        {
            let mut sync = self.sync_manager.write().await;
            sync.update_local_tip(0, genesis_hash, 0);
            sync.reset_empty_headers();
            sync.reset_sync_for_rollback();
        }

        self.shallow_rollback_count = 0;
        self.cumulative_rollback_depth = 0;

        info!("State reset complete. Header-first sync will rebuild from genesis.");
        Ok(true)
    }
}
