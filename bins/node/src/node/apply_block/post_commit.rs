use super::*;

impl Node {
    /// Actions performed after the batch commit: active-list recompute, epoch snapshot,
    /// attestation, archive buffering, and websocket broadcast.
    pub async fn post_commit_actions(&mut self, block: &Block, block_hash: Hash, height: u64) {
        // Recompute whether we are in the active production list at epoch boundaries
        self.recompute_active_status(height).await;
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

                let mut attestation_minutes: HashMap<PublicKey, HashSet<u32>> = HashMap::new();
                let mut blocks_produced: HashMap<PublicKey, u32> = HashMap::new();
                let mut new_list: Vec<PublicKey> = if epoch <= 1 {
                    // Genesis/early epochs: all active producers qualify
                    active.clone()
                } else {
                    // Rolling 3-epoch lookback window. Producers who attested in
                    // ANY of the last 3 epochs are retained. A producer must be
                    // offline for 3 consecutive epochs (~3h) to be culled.
                    //
                    // Why 3 epochs: single-epoch filter caused chain death via
                    // attestation erosion cascade (2026-04-11 incident). One
                    // restart or glitch = producer dropped permanently due to
                    // chicken-and-egg (not in list → no bit → no attestation).
                    // Rolling window gives natural recovery: a producer back
                    // online can attest and stay in the next list.
                    //
                    // INC-I-010: If ANY block in any window is missing (e.g.,
                    // after snap sync), skip filtering — use all active.
                    let mut attested: HashSet<PublicKey> = HashSet::new();
                    let mut have_full_history = true;
                    const LOOKBACK_EPOCHS: u64 = 3;

                    for epoch_back in 1..=LOOKBACK_EPOCHS {
                        if epoch < epoch_back {
                            // Not enough history (early epochs) — skip this window.
                            continue;
                        }
                        let target_epoch = epoch - epoch_back;
                        let window_start = target_epoch * blocks_per_epoch;
                        let window_end = (target_epoch + 1) * blocks_per_epoch;

                        // Decoder list for this specific window: active at the
                        // window's epoch_start. Matches the encoder path (which
                        // queries active_at_epoch_start since BITFIELD_ENCODER_EPOCH_START_HEIGHT=0).
                        let producers = self.producer_set.read().await;
                        let mut sorted_for_decode: Vec<PublicKey> = producers
                            .active_producers_at_height(window_start)
                            .iter()
                            .map(|p| p.public_key)
                            .collect();
                        drop(producers);
                        sorted_for_decode.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

                        for h in window_start..window_end {
                            if let Ok(Some(blk)) = self.block_store.get_block_by_height(h) {
                                let minute =
                                    doli_core::attestation::attestation_minute(blk.header.slot);
                                attested.insert(blk.header.producer);
                                // Only track produced/minutes for the PREVIOUS epoch
                                // (used by tier promotion below, which reflects
                                // last-epoch performance specifically).
                                if epoch_back == 1 {
                                    *blocks_produced.entry(blk.header.producer).or_insert(0) += 1;
                                    attestation_minutes
                                        .entry(blk.header.producer)
                                        .or_default()
                                        .insert(minute);
                                }
                                if !blk.header.presence_root.is_zero() {
                                    let indices = if !blk.attestation_bitfield.is_empty() {
                                        doli_core::decode_attestation_bitfield_vec(
                                            &blk.attestation_bitfield,
                                            sorted_for_decode.len(),
                                        )
                                    } else if h
                                        < doli_core::consensus::BITFIELD_BODY_ACTIVATION_HEIGHT
                                    {
                                        decode_attestation_bitfield(
                                            &blk.header.presence_root,
                                            sorted_for_decode.len(),
                                        )
                                    } else {
                                        vec![]
                                    };
                                    for idx in indices {
                                        if let Some(pk) = sorted_for_decode.get(idx) {
                                            attested.insert(*pk);
                                            if epoch_back == 1 {
                                                attestation_minutes
                                                    .entry(*pk)
                                                    .or_default()
                                                    .insert(minute);
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Missing block — incomplete history.
                                have_full_history = false;
                                break;
                            }
                        }

                        if !have_full_history {
                            break;
                        }
                    }

                    let skip_height = self.config.network.params().snap_attestation_skip_height;
                    if have_full_history || height < skip_height {
                        // Full history across all lookback windows: filter by attestation.
                        active
                            .clone()
                            .into_iter()
                            .filter(|pk| attested.contains(pk))
                            .collect()
                    } else {
                        info!(
                            "[EPOCH] Incomplete block history in lookback window — using all {} active producers",
                            active.len()
                        );
                        active.clone()
                    }
                };

                // Deadlock safety: if attestation filter leaves less than 2/3 of
                // active producers, treat as a mass event (restart, deploy, network
                // outage) rather than individual inactivity, and include everyone.
                //
                // Tightened from 1/3 (v6.8.8) to 2/3 (v6.13.2): a legitimate
                // outage affecting more than a third of producers should NEVER
                // result in chain death. The previous 1/3 threshold allowed
                // erosion to continue past the deadlock point.
                let active_count = active.len();
                if new_list.len() < (active_count * 2 / 3) || new_list.is_empty() {
                    warn!(
                        "[EPOCH] Attestation filter left {}/{} producers (<2/3) — mass event detected, including all",
                        new_list.len(), active_count
                    );
                    new_list = active;
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

                // Clear mid-epoch exclusions — fresh start for new epoch
                self.excluded_producers.clear();
                // INC-I-010 layer 3: epoch_producer_list is now rebuilt with
                // attestation filtering — end the post-snap Light-mode window.
                if self.snap_sync_height.is_some() {
                    info!("[SNAP_SYNC] Epoch boundary reached — switching gossip validation to Full mode");
                    self.snap_sync_height = None;
                }
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

            // RE-INCLUSION: handled at epoch boundary only (line 136-138 above).
            // Mid-epoch re-inclusion by attestation was removed because it causes
            // an exclude→re-include→miss→exclude cycle that prevents the block rate
            // from reaching 100% when producers are offline. Excluded producers
            // stay excluded until the next epoch, where the frozen list is rebuilt
            // from attestation data. They still earn rewards if they attest.
        }

        // Sanity cap: prevent excluded_producers feedback loop (INC-I-016)
        if !self.epoch_producer_list.is_empty()
            && self.excluded_producers.len() > self.epoch_producer_list.len() / 3
        {
            warn!(
                "[LIVENESS] Excluded producers ({}) exceeds 33% cap — resetting",
                self.excluded_producers.len()
            );
            self.excluded_producers.clear();
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
