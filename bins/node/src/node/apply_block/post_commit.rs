use super::*;

impl Node {
    /// Actions performed after the batch commit: active-list recompute, epoch snapshot,
    /// attestation, archive buffering, and websocket broadcast.
    pub async fn post_commit_actions(&mut self, block: &Block, block_hash: Hash, height: u64) {
        // Recompute whether we are in the active production list at epoch boundaries
        self.recompute_active_status(height).await;
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();

        // INCREMENTAL ATTESTATION TRACKING: decode this block's bitfield and
        // accumulate into current epoch tracker. Distributes O(N) per block
        // instead of O(N × blocks_per_epoch) at epoch boundary.
        {
            let minute = doli_core::attestation::attestation_minute(block.header.slot);
            self.epoch_attested_set[0].insert(block.header.producer);
            *self
                .epoch_blocks_produced_accum
                .entry(block.header.producer)
                .or_insert(0) += 1;
            self.epoch_attestation_accum[0]
                .entry(block.header.producer)
                .or_default()
                .insert(minute);

            if !block.header.presence_root.is_zero() && !self.epoch_producer_list.is_empty() {
                let indices = if !block.attestation_bitfield.is_empty() {
                    doli_core::decode_attestation_bitfield_vec(
                        &block.attestation_bitfield,
                        self.epoch_producer_list.len(),
                    )
                } else if height < doli_core::consensus::BITFIELD_BODY_ACTIVATION_HEIGHT {
                    decode_attestation_bitfield(
                        &block.header.presence_root,
                        self.epoch_producer_list.len(),
                    )
                } else {
                    vec![]
                };
                // INFO so the decode path is symmetric with [ATTEST_ENCODE]
                // and visible in production. Encoder/decoder mismatch is the
                // class of bug that took multiple bisects to isolate (3a1e64ee,
                // 69b4755e, ee99546f) — at INFO they self-diagnose.
                info!(
                    "[ATTEST_DECODE] h={} epoch_list={} indices={} bitfield_len={}",
                    height,
                    self.epoch_producer_list.len(),
                    indices.len(),
                    if !block.attestation_bitfield.is_empty() {
                        block.attestation_bitfield.len()
                    } else {
                        32
                    }
                );
                for idx in indices {
                    if let Some(pk) = self.epoch_producer_list.get(idx) {
                        self.epoch_attested_set[0].insert(*pk);
                        self.epoch_attestation_accum[0]
                            .entry(*pk)
                            .or_default()
                            .insert(minute);
                    }
                }
            }
        }
        if doli_core::EpochSnapshot::is_epoch_boundary_with(height, blocks_per_epoch) {
            let epoch = doli_core::EpochSnapshot::epoch_from_height_with(height, blocks_per_epoch);
            let producers = self.producer_set.read().await;
            let active = producers.active_producers_at_height(height);
            let pws: Vec<(PublicKey, u64)> = active
                .iter()
                .map(|p| (p.public_key, p.selection_weight()))
                .collect();
            let total_w: u64 = pws.iter().map(|(_, w)| *w).sum();
            let mut all_pks: Vec<PublicKey> = pws.into_iter().map(|(pk, _)| pk).collect();
            all_pks.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            drop(producers);

            let snapshot = doli_core::EpochSnapshot::new(epoch, &all_pks, total_w);
            info!(
                "EpochSnapshot built: epoch={}, producers={}, weight={}, merkle={}",
                epoch, snapshot.total_producers, snapshot.total_weight, snapshot.merkle_root
            );

            // Rebuild epoch bond snapshot from UTXO set.
            // This snapshot is used by the scheduler for the ENTIRE next epoch.
            // All nodes compute this at the same height → deterministic scheduling.
            //
            // Guard: during header-first catch-up, the node processes old epoch
            // boundaries. If a correct snapshot from a later epoch exists (from
            // snap sync payload or persisted), don't overwrite with stale data.
            if self.epoch_bond_snapshot_epoch >= epoch {
                info!(
                    "[EPOCH] Bond snapshot: KEEPING existing epoch={} (incoming epoch={}) producers={} total_bonds={}",
                    self.epoch_bond_snapshot_epoch, epoch,
                    self.epoch_bond_snapshot.len(),
                    self.epoch_bond_snapshot.values().sum::<u64>()
                );
            } else {
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
                {
                    let mut keys: Vec<_> = self.epoch_bond_snapshot.keys().collect();
                    keys.sort();
                    let fp_bytes: Vec<u8> = keys
                        .iter()
                        .flat_map(|k| k.as_bytes())
                        .copied()
                        .chain(
                            self.epoch_bond_snapshot
                                .values()
                                .flat_map(|v| v.to_le_bytes()),
                        )
                        .collect();
                    let fp = crypto::hash::hash(&fp_bytes);
                    info!(
                        "[EPOCH] Bond snapshot fingerprint: epoch={} producers={} total_bonds={} fp={:.16}",
                        epoch, self.epoch_bond_snapshot.len(), total, fp
                    );
                }
            }

            // Reset minute tracker for the new epoch
            self.minute_tracker.reset();

            // EPOCH PRODUCER LIST: Freeze the schedule for the new epoch.
            // Read accumulators BEFORE rotation (they contain the just-completed epoch's data).
            // [0] = just-completed epoch, [1] = prev epoch, [2] = prev-prev epoch.
            {
                let producers = self.producer_set.read().await;
                let active: Vec<PublicKey> = producers
                    .active_producers_at_height(height)
                    .iter()
                    .map(|p| p.public_key)
                    .collect();
                drop(producers);

                // Read by reference — no clone needed, same scope as self.
                let attestation_minutes = &self.epoch_attestation_accum[0];
                let blocks_produced = &self.epoch_blocks_produced_accum;
                let mut new_list: Vec<PublicKey> = if epoch <= 1 {
                    active
                } else {
                    // 3-epoch lookback: producer retained if attested in ANY of last 3 epochs.
                    let mut attested: HashSet<PublicKey> = HashSet::new();
                    for i in 0..3 {
                        attested.extend(&self.epoch_attested_set[i]);
                    }

                    let have_full_history = !self.epoch_attested_set[0].is_empty();
                    let skip_height = self.config.network.params().snap_attestation_skip_height;
                    if have_full_history || height < skip_height {
                        active
                            .into_iter()
                            .filter(|pk| attested.contains(pk))
                            .collect()
                    } else {
                        info!(
                            "[EPOCH] Empty attestation accumulators — using all {} active producers",
                            active.len()
                        );
                        active
                    }
                };

                // FIX 2 (v6.13.5-fix12): Deadlock safety floor tightened from 1/3 to 2/3.
                // If attestation filter leaves less than 2/3 of active producers, it's
                // a mass event (restart, deploy, network outage), not individual
                // inactivity. Include everyone to prevent chain death.
                //
                // Previous 1/3 floor (v6.8.8) allowed erosion cascade past deadlock
                // point. 2/3 is the canonical BFT majority threshold — if >1/3 of
                // producers are simultaneously un-attested, assume outage not liveness.
                {
                    let producers = self.producer_set.read().await;
                    let active_count = producers.active_producers_at_height(height).len();
                    if new_list.len() < (active_count * 2 / 3) || new_list.is_empty() {
                        warn!(
                            "[EPOCH] Attestation filter left {}/{} producers (<2/3) — mass event detected, including all",
                            new_list.len(), active_count
                        );
                        new_list = producers
                            .active_producers_at_height(height)
                            .iter()
                            .map(|p| p.public_key)
                            .collect();
                    }
                }

                // Log filter summary: how many retained vs active, with attestation window sizes.
                {
                    let producers = self.producer_set.read().await;
                    let active_count = producers.active_producers_at_height(height).len();
                    info!(
                        "[EPOCH] Producer list filter: epoch={} retained={} active={} attested_e0={} attested_e1={} attested_e2={}",
                        epoch,
                        new_list.len(),
                        active_count,
                        self.epoch_attested_set[0].len(),
                        self.epoch_attested_set[1].len(),
                        self.epoch_attested_set[2].len()
                    );
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
                // After activation: first ACTIVE_PRODUCERS_CAP by registered_at (earliest first),
                // with promotion: producers below MIN_ATTESTATION_MINUTES are demoted to attestor
                // and replaced by the next qualifying attestor.
                use doli_core::consensus::{
                    ACTIVE_PRODUCERS_CAP, MIN_ATTESTATION_MINUTES,
                    TIER_PROMOTION_ACTIVATION_HEIGHT, TIER_SYSTEM_ACTIVATION_HEIGHT,
                };
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

                    if height >= TIER_PROMOTION_ACTIVATION_HEIGHT && epoch > 1 {
                        // Promotion: filter out underperformers before selecting top 50.
                        // Two criteria — must meet BOTH:
                        // 1. Attestation: >= MIN_ATTESTATION_MINUTES (30/60)
                        // 2. Production: >= 80% of expected blocks produced
                        let expected_per_producer =
                            blocks_per_epoch / self.epoch_producer_list.len().max(1) as u64;
                        let min_produced = (expected_per_producer * 80 / 100).max(1);

                        let before = with_reg.len();
                        with_reg.retain(|(pk, _)| {
                            let mins = attestation_minutes.get(pk).map(|s| s.len()).unwrap_or(0);
                            let produced = blocks_produced.get(pk).copied().unwrap_or(0) as u64;
                            mins >= MIN_ATTESTATION_MINUTES && produced >= min_produced
                        });
                        let demoted = before - with_reg.len();
                        if demoted > 0 {
                            info!(
                                "[TIER] Demoted {} producers (min_att={}, min_prod={}/{})",
                                demoted,
                                MIN_ATTESTATION_MINUTES,
                                min_produced,
                                expected_per_producer
                            );
                        }
                    }

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
                        "[TIER] Active production list: {}/{} producers (by registered_at, promotion={})",
                        self.active_production_list.len(),
                        self.epoch_producer_list.len(),
                        height >= TIER_PROMOTION_ACTIVATION_HEIGHT,
                    );
                } else {
                    self.active_production_list = self.epoch_producer_list.clone();
                }

                // Deadlock safety: if tier filter left < 1/3, mass event — include all.
                if self.active_production_list.len() < self.epoch_producer_list.len() / 3
                    || self.active_production_list.is_empty()
                {
                    warn!(
                        "[TIER] Filter left {}/{} producers — mass event, falling back to full epoch list",
                        self.active_production_list.len(), self.epoch_producer_list.len()
                    );
                    self.active_production_list = self.epoch_producer_list.clone();
                }

                // Persist lists + attestation accumulators + bond snapshot to RocksDB.
                {
                    let mut batch = self.state_db.begin_batch();
                    batch.put_epoch_producer_list(&self.epoch_producer_list);
                    batch.put_active_production_list(&self.active_production_list);
                    batch.put_attestation_accumulators(
                        &self.epoch_attested_set,
                        &self.epoch_attestation_accum,
                        &self.epoch_blocks_produced_accum,
                    );
                    batch.put_epoch_bond_snapshot(
                        &self.epoch_bond_snapshot,
                        self.epoch_bond_snapshot_epoch,
                    );
                    if let Err(e) = batch.commit() {
                        warn!("[EPOCH] Failed to persist epoch state: {}", e);
                    }
                }

                // INC-I-010 layer 3: epoch_producer_list is now rebuilt with
                // attestation filtering — end the post-snap Light-mode window.
                if self.snap_sync_height.is_some() {
                    info!("[SNAP_SYNC] Epoch boundary reached — switching gossip validation to Full mode");
                    self.snap_sync_height = None;
                }

                // Rotate attestation accumulators AFTER reading them.
                // [0]=just-completed → [1]=prev, [1]=prev → [2]=prev-prev, [2] discarded.
                debug!(
                    "[EPOCH] Accum pre-rotation: e0_attested={} e1_attested={} e2_attested={} e0_minutes={} e1_minutes={} blocks_produced={}",
                    self.epoch_attested_set[0].len(),
                    self.epoch_attested_set[1].len(),
                    self.epoch_attested_set[2].len(),
                    self.epoch_attestation_accum[0].len(),
                    self.epoch_attestation_accum[1].len(),
                    self.epoch_blocks_produced_accum.len()
                );
                self.epoch_attested_set[2] = std::mem::take(&mut self.epoch_attested_set[1]);
                self.epoch_attested_set[1] = std::mem::take(&mut self.epoch_attested_set[0]);
                self.epoch_attestation_accum[2] =
                    std::mem::take(&mut self.epoch_attestation_accum[1]);
                self.epoch_attestation_accum[1] =
                    std::mem::take(&mut self.epoch_attestation_accum[0]);
                self.epoch_blocks_produced_accum.clear();
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
