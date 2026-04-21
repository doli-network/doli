use super::*;

impl Node {
    /// Actions performed around batch commit: active-list recompute, epoch snapshot,
    /// attestation, archive buffering, and websocket broadcast.
    /// Epoch state writes go into the passed batch (atomic with block commit).
    pub async fn post_commit_actions(
        &mut self,
        block: &Block,
        block_hash: Hash,
        height: u64,
        batch: &mut storage::BlockBatch<'_>,
    ) {
        // Recompute whether we are in the active production list at epoch boundaries
        self.recompute_active_status(height).await;
        let blocks_per_epoch = self.config.network.blocks_per_reward_epoch();

        // INCREMENTAL ATTESTATION TRACKING via EpochState::accumulate_block().
        // Decode the bitfield first (caller's responsibility since it depends on
        // producer_list ordering and block body data).
        {
            let has_attestation_data =
                !block.header.presence_root.is_zero() && !self.epoch_state.producer_list.is_empty();

            // After FULL_BITFIELD_DECODE_HEIGHT: decode ALL indices (base + extra)
            // so filtered producers can re-enter via 3-epoch lookback.
            // Before: only base indices (epoch_state.producer_list.len()).
            let use_full_decode =
                height >= self.config.network.params().full_bitfield_decode_height;
            let base_len = self.epoch_state.producer_list.len();

            // Build full decode list [base | extra sorted] when needed
            let (decode_len, extra_pks) = if use_full_decode && has_attestation_data {
                let producers = self.producer_set.read().await;
                let all_active: Vec<crypto::PublicKey> = producers
                    .active_producers_at_height(height)
                    .iter()
                    .map(|p| p.public_key)
                    .collect();
                drop(producers);
                let base_set: HashSet<&crypto::PublicKey> =
                    self.epoch_state.producer_list.iter().collect();
                let mut extra: Vec<crypto::PublicKey> = all_active
                    .iter()
                    .filter(|pk| !base_set.contains(pk))
                    .copied()
                    .collect();
                extra.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
                let total = base_len + extra.len();
                (total, extra)
            } else {
                (base_len, Vec::new())
            };

            let indices = if has_attestation_data {
                let idx = if !block.attestation_bitfield.is_empty() {
                    doli_core::decode_attestation_bitfield_vec(
                        &block.attestation_bitfield,
                        decode_len,
                    )
                } else if height < doli_core::consensus::BITFIELD_BODY_ACTIVATION_HEIGHT {
                    decode_attestation_bitfield(&block.header.presence_root, decode_len)
                } else {
                    vec![]
                };
                info!(
                    "[ATTEST_DECODE] h={} epoch_list={} decode_len={} indices={} bitfield_len={}",
                    height,
                    base_len,
                    decode_len,
                    idx.len(),
                    if !block.attestation_bitfield.is_empty() {
                        block.attestation_bitfield.len()
                    } else {
                        32
                    }
                );
                // Log producers MISSING from the attestation bitfield (only when partial)
                if !idx.is_empty() && idx.len() < base_len {
                    let attested: HashSet<usize> = idx.iter().copied().collect();
                    let missing: Vec<String> = (0..base_len)
                        .filter(|i| !attested.contains(i))
                        .filter_map(|i| {
                            self.epoch_state.producer_list.get(i).map(|pk| {
                                let h = hex::encode(pk.as_bytes());
                                h[..8].to_string()
                            })
                        })
                        .collect();
                    if !missing.is_empty() {
                        let minute = attestation_minute(block.header.slot);
                        warn!(
                            "[ATTEST_MISS] h={} minute={} missing={} producers=[{}]",
                            height,
                            minute,
                            missing.len(),
                            missing.join(",")
                        );
                    }
                }
                idx
            } else {
                vec![]
            };

            // Split indices: base (0..base_len) go to accumulate_block,
            // extra (base_len..) resolved manually into epoch_state.
            let base_indices: Vec<usize> =
                indices.iter().filter(|&&i| i < base_len).copied().collect();

            self.epoch_state
                .accumulate_block(&doli_core::BlockAccumulationInput {
                    producer: block.header.producer,
                    slot: block.header.slot,
                    has_attestation_data: has_attestation_data && !base_indices.is_empty(),
                    attested_indices: base_indices,
                });

            // Track extra producers directly in epoch_state
            if use_full_decode && has_attestation_data {
                let minute = attestation_minute(block.header.slot);
                for &idx in &indices {
                    if idx >= base_len {
                        let extra_idx = idx - base_len;
                        if let Some(pk) = extra_pks.get(extra_idx) {
                            self.epoch_state.attested_sets[0].insert(*pk);
                            self.epoch_state.attestation_accum[0]
                                .entry(*pk)
                                .or_default()
                                .insert(minute);
                        }
                    }
                }
            }
        }

        // Persist epoch_state after every block (not just epoch boundaries).
        // Without this, mid-epoch accumulator changes are RAM-only — lost on restart,
        // causing sched divergence until the next epoch boundary. Latent fork trigger
        // at 50+ producers when tier promotion reads attestation_accum[0].
        batch.put_epoch_state(&self.epoch_state.serialize());
        batch.put_epoch_state_version(CURRENT_PROTOCOL_VERSION);

        // Chain commitment: computed periodically via full scan in periodic.rs.
        // Incremental computation was removed — it corrupted on every code path
        // that modified the chain without updating the commitment (fork replacement,
        // sync, rsync, snap sync). Periodic full scan is always correct by construction.

        if doli_core::EpochSnapshot::is_epoch_boundary_with(height, blocks_per_epoch) {
            let epoch = doli_core::EpochSnapshot::epoch_from_height_with(height, blocks_per_epoch);

            // EpochSnapshot (separate concern — merkle root for the epoch).
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

            // Build EpochDerivationInput from external sources.
            // Bond counts: guard against overwriting a newer snapshot with stale data.
            let bond_counts = if self.epoch_state.epoch >= epoch {
                info!(
                    "[EPOCH] Bond snapshot: KEEPING existing epoch={} (incoming epoch={}) producers={} total_bonds={}",
                    self.epoch_state.epoch, epoch,
                    self.epoch_state.bond_snapshot.len(),
                    self.epoch_state.bond_snapshot.values().sum::<u64>()
                );
                self.epoch_state.bond_snapshot.clone()
            } else {
                let utxo = self.utxo_set.read().await;
                let producers = self.producer_set.read().await;
                let active = producers.active_producers_at_height(height);
                let mut snap = HashMap::new();
                for p in &active {
                    let pubkey_hash = hash_with_domain(ADDRESS_DOMAIN, p.public_key.as_bytes());
                    let count = utxo
                        .count_bonds(&pubkey_hash, self.config.network.bond_unit())
                        .max(1) as u64;
                    snap.insert(pubkey_hash, count);
                }
                let total: u64 = snap.values().sum();
                info!(
                    "Epoch bond snapshot rebuilt: epoch={}, producers={}, total_bonds={}",
                    epoch,
                    snap.len(),
                    total
                );
                // Log fingerprint for divergence diagnosis
                {
                    let mut keys: Vec<_> = snap.keys().collect();
                    keys.sort();
                    let fp_bytes: Vec<u8> = keys
                        .iter()
                        .flat_map(|k| k.as_bytes())
                        .copied()
                        .chain(snap.values().flat_map(|v| v.to_le_bytes()))
                        .collect();
                    let fp = crypto::hash::hash(&fp_bytes);
                    info!(
                        "[EPOCH] Bond snapshot fingerprint: epoch={} producers={} total_bonds={} fp={:.16}",
                        epoch, snap.len(), total, fp
                    );
                }
                snap
            };

            // Active producers + registered_at for tier system
            let producers = self.producer_set.read().await;
            let active_producers: Vec<PublicKey> = producers
                .active_producers_at_height(height)
                .iter()
                .map(|p| p.public_key)
                .collect();
            let registered_at: HashMap<PublicKey, u64> = producers
                .active_producers_at_height(height)
                .iter()
                .map(|p| (p.public_key, p.registered_at))
                .collect();
            let active_count = active_producers.len();
            drop(producers);

            // Log pre-derivation state
            info!(
                "[EPOCH] Producer list filter: epoch={} active={} attested_e0={} attested_e1={} attested_e2={}",
                epoch, active_count,
                self.epoch_state.attested_sets[0].len(),
                self.epoch_state.attested_sets[1].len(),
                self.epoch_state.attested_sets[2].len()
            );
            debug!(
                "[EPOCH] Accum pre-rotation: e0_attested={} e1_attested={} e2_attested={} e0_minutes={} e1_minutes={} blocks_produced={}",
                self.epoch_state.attested_sets[0].len(),
                self.epoch_state.attested_sets[1].len(),
                self.epoch_state.attested_sets[2].len(),
                self.epoch_state.attestation_accum[0].len(),
                self.epoch_state.attestation_accum[1].len(),
                self.epoch_state.blocks_produced.len()
            );

            let derivation_input = doli_core::EpochDerivationInput {
                active_producers,
                bond_counts,
                blocks_per_epoch,
                snap_attestation_skip_height: self
                    .config
                    .network
                    .params()
                    .snap_attestation_skip_height,
                height,
                epoch,
                registered_at,
            };

            // THE canonical derivation — one function, one path, compile-time guarantee.
            let new_state =
                doli_core::EpochState::derive_at_boundary(&self.epoch_state, &derivation_input);

            info!(
                "[EPOCH] Frozen producer list for epoch {}: {} producers, active_list={} (was: {} producers)",
                epoch,
                new_state.producer_list.len(),
                new_state.active_list.len(),
                self.epoch_state.producer_list.len(),
            );

            // Persist epoch state atomically with the block commit (M5: no crash window).
            batch.put_epoch_producer_list(&new_state.producer_list);
            batch.put_active_production_list(&new_state.active_list);
            batch.put_attestation_accumulators(
                &new_state.attested_sets,
                &new_state.attestation_accum,
                &new_state.blocks_produced,
            );
            batch.put_epoch_bond_snapshot(&new_state.bond_snapshot, new_state.epoch);
            batch.put_epoch_state(&new_state.serialize());
            batch.put_epoch_state_version(CURRENT_PROTOCOL_VERSION);

            // Apply the new epoch state
            self.epoch_state = new_state;

            // Reset minute tracker for the new epoch
            self.minute_tracker.reset();

            // INC-I-010 layer 3: epoch_producer_list is now rebuilt with
            // attestation filtering — end the post-snap Light-mode window.
            if self.snap_sync_height.is_some() {
                info!(
                    "[SNAP_SYNC] Epoch boundary reached — switching gossip validation to Full mode"
                );
                self.snap_sync_height = None;
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
