use super::*;

mod assembly;
mod gates;
mod scheduling;

impl Node {
    // NOTE: try_broadcast_heartbeat removed in deterministic scheduler model
    // Rewards go 100% to block producer via coinbase, no heartbeats needed

    /// Try to produce a block if we're an eligible producer
    pub(super) async fn try_produce_block(&mut self) -> Result<()> {
        let our_pubkey = match &self.producer_key {
            Some(k) => *k.public_key(),
            None => return Ok(()),
        };

        // VERSION ENFORCEMENT CHECK
        // If an update has been approved and grace period has passed,
        // outdated nodes cannot produce blocks.
        if let Err(blocked) = node_updater::is_production_allowed(&self.config.data_dir) {
            // Log once per minute to avoid spam
            static LAST_WARNING: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let last = LAST_WARNING.load(std::sync::atomic::Ordering::Relaxed);
            if now_secs - last >= 60 {
                LAST_WARNING.store(now_secs, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!("{}", blocked);
            }
            return Ok(());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let current_slot = self.params.timestamp_to_slot(now);

        // Already produced for this slot — skip before any eligibility/scheduler work.
        // The signed_slots DB is the authoritative slashing guard; this is a fast path
        // that avoids wasted eligibility checks, VDF, and block building when the
        // production timer fires again within the same 10s slot.
        if self.last_produced_slot == Some(current_slot as u64) {
            return Ok(());
        }

        // PRODUCTION GATE CHECK
        if !self.handle_production_authorization(current_slot).await {
            return Ok(());
        }

        // Log slot info periodically (every ~60 seconds)
        if current_slot.is_multiple_of(60) {
            info!(
                "Production check: now={}, genesis={}, current_slot={}, last_produced={:?}",
                now, self.params.genesis_time, current_slot, self.last_produced_slot
            );
        }

        // EARLY BLOCK EXISTENCE CHECK (optimization)
        // Check if a block already exists for this slot before spending time on eligibility
        // checks and VDF computation. This is safe because:
        // 1. If we see a block, another producer already succeeded
        // 2. We'll check again after VDF to catch blocks that appeared during computation
        if self.block_store.has_block_for_slot(current_slot as u64) {
            debug!(
                "Block already exists for slot {} - skipping production",
                current_slot
            );
            return Ok(());
        }

        // Get chain state
        let state = self.chain_state.read().await;
        let prev_hash = state.best_hash;
        let prev_slot = state.best_slot;
        let height = state.best_height + 1;
        drop(state);

        // Can't produce if slot hasn't advanced
        if current_slot <= prev_slot {
            debug!(
                "Slot not advanced: current_slot={} <= prev_slot={}",
                current_slot, prev_slot
            );
            return Ok(());
        }

        // =========================================================================
        // DEFENSE-IN-DEPTH: Peer-Aware Behind-Network Check
        //
        // Prevents producing orphan blocks when we're significantly behind the
        // network. A node at height 0 should never produce for slot 92 if peers
        // are at height 90 — the block would be an orphan.
        //
        // Key insight: Compare HEIGHTS, not slot-height gap. In sparse chains
        // (genesis phase, network downtime), slots advance via wall-clock while
        // blocks are produced infrequently. A large slot-height gap (e.g.,
        // slot 100,000 at height 175) is normal and does NOT indicate we're
        // behind — it means the chain has been producing sparsely. What matters
        // is whether peers have blocks we're missing.
        //
        // Previous bug: Using current_slot - height as the gap metric caused a
        // permanent deadlock where joining nodes could never produce because
        // the slot-height gap always exceeded max_gap in sparse chains.
        //
        // Thresholds:
        // - Height < 10: max 3 blocks behind (tight - prevent orphan forks)
        // - Height 10+:  max 5 blocks behind (allow propagation delay)
        // =========================================================================
        let network_tip_height = {
            let sync = self.sync_manager.read().await;
            sync.best_peer_height()
        };

        let network_height_ahead = network_tip_height > height.saturating_sub(1);

        if height > 1 && network_height_ahead {
            let blocks_behind = network_tip_height.saturating_sub(height.saturating_sub(1));
            let max_behind: u64 = if height < 10 {
                3 // Tight during early chain - prevent orphan forks
            } else {
                5 // Normal operation - allow some propagation delay
            };

            if blocks_behind > max_behind {
                debug!(
                    "Behind network by {} blocks (next_height={}, network_tip={}) - deferring production",
                    blocks_behind, height, network_tip_height
                );
                return Ok(());
            }
        }

        // Get SCHEDULED active producers for round-robin scheduling.
        //
        // Reactive Round-Robin: each producer gets exactly 1 scheduling ticket.
        // Producers who missed their slot (scheduled=false) are excluded.
        // Bond weight determines REWARDS, not scheduling.
        //
        // This is deterministic: `scheduled` flag is on-chain state derived from
        // block headers (missed slots + attestation re-entry), identical on all nodes.
        let producers = self.producer_set.read().await;
        let active_with_weights: Vec<(PublicKey, u64)> = producers
            .scheduled_producers_at_height(height)
            .iter()
            .map(|p| (p.public_key, 1u64)) // Equal weight: 1 ticket per producer
            .collect();
        let total_producers = producers.total_count();
        drop(producers);

        // Check if we're in genesis phase (bond-free production)
        let in_genesis = self.config.network.is_in_genesis(height);

        let _we_are_active = active_with_weights.iter().any(|(pk, _)| pk == &our_pubkey);

        // =========================================================================
        // INVARIANT CHECK: Detect inconsistent state after resync
        //
        // If we're at a very low height (< 10) but have many active producers with
        // weights, AND we're not in genesis phase, this indicates an inconsistent
        // state - possibly a failed or incomplete resync.
        //
        // After a proper resync via recover_from_peers(), the producer_set
        // should be cleared. If it's not, we're in a dangerous state where
        // production could create orphan blocks.
        //
        // This check catches edge cases like:
        // - Interrupted resync
        // - State corruption
        // - Race conditions in state updates
        // =========================================================================
        if height < 10 && !in_genesis && active_with_weights.len() > 5 {
            error!(
                "INVARIANT VIOLATION: height {} < 10 but {} active producers (total: {}) \
                 outside genesis phase. This indicates inconsistent state - blocking production.",
                height,
                active_with_weights.len(),
                total_producers
            );
            self.sync_manager.write().await.block_production(&format!(
                "invariant violation: height {} with {} active producers outside genesis",
                height,
                active_with_weights.len()
            ));
            return Ok(());
        }

        // Use bootstrap mode if:
        // 1. Still in genesis phase (no bond required), OR
        // 2. No active producers registered (transition block or testnet/devnet)
        let (eligible, our_bootstrap_rank) = if in_genesis || active_with_weights.is_empty() {
            match self
                .resolve_bootstrap_eligibility(current_slot, height, our_pubkey, in_genesis)
                .await
            {
                Some(result) => result,
                None => return Ok(()),
            }
        } else {
            // Use raw bond weights for scheduler — must match validation path exactly.
            // INC-003/INC-005: emergency equalization used LOCAL producer_liveness to
            // filter producers when slot_gap>3, but validation had no equivalent,
            // causing catastrophic scheduler mismatch and cascade forks. Removed permanently.
            let eligible =
                self.resolve_epoch_eligibility(current_slot, height, &active_with_weights);
            (eligible, None)
        };

        if eligible.is_empty() {
            return Ok(());
        }

        // Calculate slot offset in milliseconds for eligibility window
        let slot_start = self.params.slot_to_timestamp(current_slot);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let slot_start_ms = slot_start * 1000;
        let slot_offset_ms = now_ms.saturating_sub(slot_start_ms);

        // Check if we're eligible at this time
        // For bootstrap mode, use continuous time-based scheduling
        // For normal mode, use the standard eligibility check (ms-precision)
        let is_eligible = if let Some(score) = our_bootstrap_rank {
            // Bootstrap mode: continuous time-based scheduling
            // The score (0-255) determines when we should produce within the slot.
            // We can produce when the current time offset exceeds our target offset.
            let slot_duration_ms = self.params.slot_duration * 1000;
            let max_offset_percent = 80; // Leave 20% for propagation
            let target_offset_percent = (score as u64 * max_offset_percent) / 255;
            let target_offset_ms = (slot_duration_ms * target_offset_percent) / 100;

            // We're eligible if current offset >= our target offset
            slot_offset_ms >= target_offset_ms
        } else {
            // Normal mode: ms-precision sequential eligibility check
            consensus::is_producer_eligible_ms(&our_pubkey, &eligible, slot_offset_ms)
        };

        if !is_eligible {
            return Ok(());
        }

        // For devnet, add a minimum delay to allow heartbeat collection
        // Scale delay to 20% of slot duration (capped at 700ms for long slots)
        // This prevents the delay from consuming too much of short slots
        if self.config.network == Network::Devnet {
            let slot_duration_ms = self.params.slot_duration * 1000;
            let heartbeat_collection_ms = std::cmp::min(slot_duration_ms / 5, 700);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let slot_start_ms = slot_start * 1000;
            let offset_ms = now_ms.saturating_sub(slot_start_ms);
            if offset_ms < heartbeat_collection_ms {
                return Ok(()); // Too early, wait for heartbeats
            }
        }

        // We're eligible - produce a block!
        info!(
            "Producing block for slot {} at height {} (offset {}ms)",
            current_slot, height, slot_offset_ms
        );

        // =========================================================================
        // PROPAGATION RACE MITIGATION: Yield before VDF to catch in-flight blocks
        //
        // Problem: During VDF computation (~55ms), the event loop is blocked and
        // cannot process incoming gossip blocks. If another producer's block for
        // slot S is in-flight while we start producing for slot S+1, we won't see
        // it until after we've already broadcast our block, creating a fork.
        //
        // Solution: Before starting VDF computation, yield control briefly to allow
        // any pending network events to be processed. This gives in-flight blocks
        // a chance to arrive before we commit to production.
        //
        // We yield if:
        // 1. Network tip slot suggests there might be a recent block we haven't seen
        // 2. We're not too far into the slot (leave time for our own production)
        //
        // This is a lightweight "micro-sync" that doesn't require protocol changes.
        // =========================================================================
        {
            let network_tip_slot = self.sync_manager.read().await.best_peer_slot();

            // If network tip is at current_slot-1 or current_slot, there might be
            // an in-flight block we should wait for before producing
            if network_tip_slot >= prev_slot && current_slot > prev_slot + 1 {
                // We're potentially missing a block - yield briefly
                debug!(
                    "Propagation race mitigation: yielding before VDF (prev_slot={}, network_tip={}, current={})",
                    prev_slot, network_tip_slot, current_slot
                );

                // Yield for a short time to allow pending network events to be processed
                // This is much shorter than a full slot - just enough for gossip propagation
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

                // Re-check: did a block arrive for a more recent slot?
                let new_state = self.chain_state.read().await;
                let new_prev_slot = new_state.best_slot;
                drop(new_state);

                if new_prev_slot > prev_slot {
                    debug!(
                        "Block arrived during yield (prev_slot: {} -> {}) - restarting production check",
                        prev_slot, new_prev_slot
                    );
                    // A block arrived - abort this production attempt
                    // Next production tick will re-evaluate with updated chain state
                    return Ok(());
                }
            }
        }

        // SIGNED SLOTS PROTECTION: Check if we already signed this slot
        // This prevents double-signing if we restart quickly
        if let Some(ref signed_slots) = self.signed_slots_db {
            if let Err(e) = signed_slots.check_and_mark(current_slot as u64) {
                error!("SLASHING PROTECTION: {}", e);
                return Ok(());
            }
        }

        // Build block content (coinbase, rewards, genesis VDF, mempool, attestations)
        let (header, transactions) = match self
            .build_block_content(prev_hash, prev_slot, height, current_slot, our_pubkey)
            .await?
        {
            Some(result) => result,
            None => return Ok(()),
        };

        // Drain pending network events before VDF
        if self.drain_pending_events(prev_slot, height).await {
            return Ok(());
        }

        // Compute VDF
        let (vdf_output, vdf_proof) = self.compute_block_vdf(&prev_hash, &header).await?;

        // SAFETY CHECK: Verify chain state hasn't changed during VDF computation
        //
        // The VDF takes ~700ms during which the event loop is blocked. Other
        // producers' blocks may have arrived and been queued. We must check:
        // 1. No block exists for our current slot (same-slot duplicate)
        // 2. Chain tip hasn't advanced (stale parent detection)
        //
        // Without check #2, we'd build on a stale parent and create a fork.
        if self.block_store.has_block_for_slot(current_slot as u64) {
            debug!(
                "Block appeared during VDF computation for slot {} - aborting production",
                current_slot
            );
            return Ok(());
        }

        // Check if chain tip advanced during VDF (stale parent detection)
        {
            let post_vdf_state = self.chain_state.read().await;
            if post_vdf_state.best_height >= height || post_vdf_state.best_hash != prev_hash {
                info!(
                    "Chain advanced during VDF computation (tip moved from height {} to {}) - aborting to avoid fork",
                    height - 1, post_vdf_state.best_height
                );
                return Ok(());
            }
        }

        // Create final block header with VDF
        let final_header = BlockHeader {
            version: header.version,
            prev_hash: header.prev_hash,
            merkle_root: header.merkle_root,
            presence_root: header.presence_root,
            genesis_hash: header.genesis_hash,
            timestamp: header.timestamp,
            slot: header.slot,
            producer: header.producer,
            vdf_output,
            vdf_proof,
        };

        // Use the transactions from the builder — same list used for merkle root computation.
        // No duplicate transaction assembly needed.
        // Aggregate BLS signatures from minute tracker for on-chain proof.
        let aggregate_bls_signature = self.aggregate_bls_signatures(current_slot);

        let block = Block {
            header: final_header,
            transactions,
            aggregate_bls_signature,
        };

        let block_hash = block.hash();
        info!(
            "[BLOCK_PRODUCED] hash={} height={} slot={} parent={}",
            block_hash, height, current_slot, block.header.prev_hash
        );

        // Apply the block locally
        self.apply_block(block.clone(), ValidationMode::Full)
            .await?;

        // NOTE: Do NOT call note_block_received_via_gossip() here.
        // Self-produced blocks must not reset the solo production watchdog —
        // otherwise a node that loses gossip connectivity will produce solo
        // indefinitely, deepening a fork without any circuit breaker firing.
        //
        // Similarly, do NOT call refresh_all_peers() — refreshing all peer
        // timestamps after self-production masks actually-stale peers, preventing
        // stale chain detection from triggering.

        // Mark that we produced for this slot
        self.last_produced_slot = Some(current_slot as u64);

        // Broadcast the block to the network
        // This is only done for blocks we produce ourselves - received blocks
        // are already on the network and don't need to be re-broadcast.
        // Header is sent first so peers can pre-validate before the full block arrives.
        if let Some(ref network) = self.network {
            let _ = network.broadcast_header(block.header.clone()).await;
            let _ = network.broadcast_block(block).await;
        }

        // Attest our own block for finality gadget + record in minute tracker.
        self.attest_own_block(block_hash, current_slot, height)
            .await;

        // Flush any blocks that just reached finality
        self.flush_finalized_to_archive().await;

        Ok(())
    }
}
