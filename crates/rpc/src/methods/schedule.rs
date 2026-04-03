//! Schedule and attestation handlers: getSlotSchedule, getProducerSchedule, getAttestationStats

use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

impl RpcContext {
    /// Build the active producer set with UTXO-derived bond weights.
    ///
    /// Returns `(producers_with_bonds, total_bonds)` where each entry is
    /// `(PublicKey, bond_count)` with a minimum of 1 bond per producer.
    pub(super) async fn build_producers_with_bonds(
        &self,
    ) -> Result<(Vec<(crypto::PublicKey, u64)>, u64), RpcError> {
        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let producers = producer_set.read().await;
        let active = producers.active_producers();
        let utxo_set = self.utxo_set.read().await;

        let mut result = Vec::with_capacity(active.len());
        let mut total_bonds: u64 = 0;

        for info in &active {
            let pubkey_hash =
                crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, info.public_key.as_bytes());
            let bonds = utxo_set.count_bonds(&pubkey_hash, self.bond_unit) as u64;
            let effective = bonds.max(1);
            result.push((info.public_key, effective));
            total_bonds += effective;
        }

        Ok((result, total_bonds))
    }

    /// Get slot schedule for upcoming slots
    pub(super) async fn get_slot_schedule(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetSlotScheduleParams =
            serde_json::from_value(params).unwrap_or(GetSlotScheduleParams {
                from_slot: None,
                count: None,
            });

        let chain_state = self.chain_state.read().await;
        let current_slot = chain_state.best_slot;
        drop(chain_state);

        let from_slot = params.from_slot.unwrap_or(current_slot);
        let count = params.count.unwrap_or(20).min(360);

        let (producers_with_bonds, total_bonds) = self.build_producers_with_bonds().await?;

        if producers_with_bonds.is_empty() {
            return Err(RpcError::internal_error("No active producers"));
        }

        let mut slots = Vec::with_capacity(count as usize);
        for i in 0..count {
            let slot = from_slot.saturating_add(i);
            #[allow(deprecated)]
            let ranked =
                doli_core::consensus::select_producer_for_slot(slot, &producers_with_bonds);
            if let Some(pk) = ranked.first() {
                slots.push(SlotScheduleEntry {
                    slot,
                    producer: hex::encode(pk.as_bytes()),
                    rank: 0,
                });
            }
        }

        let epoch = self.params.slot_to_epoch(current_slot) as u64;
        let epoch_start = epoch * self.params.slots_per_epoch as u64;
        let epoch_end = epoch_start + self.params.slots_per_epoch as u64;
        let slots_remaining = epoch_end.saturating_sub(current_slot as u64);

        let response = SlotScheduleResponse {
            slots,
            current_slot,
            epoch,
            slots_remaining_in_epoch: slots_remaining,
            total_bonds,
            slot_duration: self.params.slot_duration,
            genesis_time: self.params.genesis_time,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get schedule and performance info for a specific producer
    pub(super) async fn get_producer_schedule(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetProducerScheduleParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let target_pubkey = crypto::PublicKey::from_hex(&params.public_key)
            .map_err(|_| RpcError::invalid_params("Invalid public key format"))?;

        let chain_state = self.chain_state.read().await;
        let current_slot = chain_state.best_slot;
        let current_height = chain_state.best_height;
        drop(chain_state);

        let (producers_with_bonds, total_bonds) = self.build_producers_with_bonds().await?;

        if producers_with_bonds.is_empty() {
            return Err(RpcError::internal_error("No active producers"));
        }

        // Get this producer's bond count
        let pubkey_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, target_pubkey.as_bytes());
        let utxo_set = self.utxo_set.read().await;
        let bond_count = utxo_set.count_bonds(&pubkey_hash, self.bond_unit) as u64;
        drop(utxo_set);
        let effective_bonds = bond_count.max(1);

        // Epoch boundaries
        let epoch = self.params.slot_to_epoch(current_slot) as u64;
        let spe = self.params.slots_per_epoch as u64;
        let epoch_start_slot = epoch * spe;
        let epoch_end_slot = epoch_start_slot + spe;

        // Find all slots in this epoch where target is primary (rank 0)
        let mut slots_this_epoch = Vec::new();
        for slot in epoch_start_slot..epoch_end_slot {
            let slot32 = slot as u32;
            #[allow(deprecated)]
            let ranked =
                doli_core::consensus::select_producer_for_slot(slot32, &producers_with_bonds);
            if ranked.first() == Some(&target_pubkey) {
                slots_this_epoch.push(slot32);
            }
        }

        // Find next slot >= current_slot
        let next_slot = slots_this_epoch
            .iter()
            .find(|&&s| s > current_slot)
            .copied();
        let seconds_until_next = next_slot.map(|ns| {
            let slot_diff = ns.saturating_sub(current_slot) as u64;
            slot_diff * self.params.slot_duration
        });

        // Count produced blocks this epoch by scanning block store
        let mut produced_count: u32 = 0;
        for &slot in &slots_this_epoch {
            if slot <= current_slot {
                if let Ok(Some(block)) = self.block_store.get_block_by_slot(slot) {
                    if block.header.producer == target_pubkey {
                        produced_count += 1;
                    }
                }
            }
        }

        let assigned_count = slots_this_epoch.len() as u32;
        let past_assigned = slots_this_epoch
            .iter()
            .filter(|&&s| s <= current_slot)
            .count() as u32;
        let fill_rate = if past_assigned > 0 {
            produced_count as f64 / past_assigned as f64
        } else {
            0.0
        };

        // Economics
        let block_reward = self.params.block_reward(current_height);
        let slots_per_week: u64 = 60480; // 7 * 24 * 360
        let weekly_earnings = if total_bonds > 0 {
            slots_per_week * block_reward * effective_bonds / total_bonds
        } else {
            0
        };
        let doubling_weeks = if weekly_earnings > 0 {
            (self.bond_unit as f64 * effective_bonds as f64) / weekly_earnings as f64
        } else {
            f64::INFINITY
        };

        let response = ProducerScheduleResponse {
            public_key: params.public_key,
            current_slot,
            epoch,
            next_slot,
            seconds_until_next,
            slots_this_epoch,
            assigned_count,
            produced_count,
            fill_rate,
            bond_count: effective_bonds as u32,
            total_network_bonds: total_bonds,
            weekly_earnings,
            doubling_weeks,
            block_reward,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get attestation statistics for the current epoch.
    ///
    /// Scans all blocks in the current epoch, decodes presence_root bitfields,
    /// and reports per-producer attestation minute counts.
    pub(super) async fn get_attestation_stats(&self) -> Result<Value, RpcError> {
        use doli_core::attestation::{
            attestation_minute, attestation_qualification_threshold, decode_attestation_bitfield,
        };
        use doli_core::consensus::reward_epoch;

        let blocks_per_epoch = self.blocks_per_reward_epoch;
        let chain_state = self.chain_state.read().await;
        let current_height = chain_state.best_height;
        drop(chain_state);

        let current_epoch = reward_epoch::from_height_with(current_height, blocks_per_epoch);
        let (epoch_start, _epoch_end) =
            reward_epoch::boundaries_with(current_epoch, blocks_per_epoch);

        // Get sorted producer list (same order as bitfield)
        let sorted_producers: Vec<(crypto::PublicKey, bool)> =
            if let Some(ref ps) = self.producer_set {
                let producers = ps.read().await;
                let mut list: Vec<_> = producers
                    .active_producers_at_height(epoch_start)
                    .iter()
                    .map(|p| (p.public_key, !p.bls_pubkey.is_empty()))
                    .collect();
                list.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
                list
            } else {
                Vec::new()
            };

        let producer_count = sorted_producers.len();

        // Scan epoch blocks for attestation data
        let mut blocks_with_attestations = 0u64;
        let mut blocks_with_bls = 0u64;
        let mut per_producer_minutes: HashMap<usize, HashSet<u32>> = HashMap::new();

        let scan_start = epoch_start.max(1);
        for h in scan_start..=current_height {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(h) {
                let pr = block.header.presence_root;
                if pr != crypto::Hash::ZERO {
                    blocks_with_attestations += 1;
                    let slot = block.header.slot;
                    let minute = attestation_minute(slot);
                    let indices = if !block.attestation_bitfield.is_empty() {
                        doli_core::decode_attestation_bitfield_vec(
                            &block.attestation_bitfield,
                            producer_count,
                        )
                    } else {
                        decode_attestation_bitfield(&pr, producer_count)
                    };
                    for idx in indices {
                        per_producer_minutes.entry(idx).or_default().insert(minute);
                    }
                }
                if !block.aggregate_bls_signature.is_empty() {
                    blocks_with_bls += 1;
                }
            }
        }

        let blocks_in_epoch = current_height.saturating_sub(epoch_start) + 1;
        let slots_elapsed = blocks_in_epoch as u32;
        let current_min = if slots_elapsed > 0 {
            attestation_minute(slots_elapsed - 1)
        } else {
            0
        };
        let total_minutes = current_min + 1;

        let producer_stats: Vec<AttestationProducerResp> = sorted_producers
            .iter()
            .enumerate()
            .map(|(idx, (pk, has_bls))| {
                let attested = per_producer_minutes
                    .get(&idx)
                    .map(|s| s.len() as u32)
                    .unwrap_or(0);
                AttestationProducerResp {
                    public_key: hex::encode(pk.as_bytes()),
                    attested_minutes: attested,
                    total_minutes,
                    threshold: attestation_qualification_threshold(blocks_per_epoch),
                    qualified: attested >= attestation_qualification_threshold(blocks_per_epoch),
                    has_bls: *has_bls,
                }
            })
            .collect();

        let response = AttestationStatsResp {
            epoch: current_epoch as u32,
            epoch_start,
            current_height,
            blocks_in_epoch,
            blocks_with_attestations,
            blocks_with_bls,
            current_minute: current_min,
            producers: producer_stats,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}

// Inline types for getAttestationStats (avoid name collision with types.rs)
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AttestationStatsResp {
    epoch: u32,
    epoch_start: u64,
    current_height: u64,
    blocks_in_epoch: u64,
    blocks_with_attestations: u64,
    blocks_with_bls: u64,
    current_minute: u32,
    producers: Vec<AttestationProducerResp>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AttestationProducerResp {
    public_key: String,
    attested_minutes: u32,
    total_minutes: u32,
    threshold: u32,
    qualified: bool,
    has_bls: bool,
}
