//! Producer query handlers: getProducer, getProducers, getBondDetails

use serde_json::Value;

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

/// Convert a PendingProducerUpdate to its RPC representation.
fn pending_update_to_info(update: &storage::PendingProducerUpdate) -> PendingUpdateInfo {
    match update {
        storage::PendingProducerUpdate::Register { .. } => PendingUpdateInfo {
            update_type: "register".to_string(),
            bond_count: None,
        },
        storage::PendingProducerUpdate::Exit { .. } => PendingUpdateInfo {
            update_type: "exit".to_string(),
            bond_count: None,
        },
        storage::PendingProducerUpdate::Slash { .. } => PendingUpdateInfo {
            update_type: "slash".to_string(),
            bond_count: None,
        },
        storage::PendingProducerUpdate::AddBond { outpoints, .. } => PendingUpdateInfo {
            update_type: "add_bond".to_string(),
            bond_count: Some(outpoints.len() as u32),
        },
        storage::PendingProducerUpdate::DelegateBond { bond_count, .. } => PendingUpdateInfo {
            update_type: "delegate_bond".to_string(),
            bond_count: Some(*bond_count),
        },
        storage::PendingProducerUpdate::RevokeDelegation { .. } => PendingUpdateInfo {
            update_type: "revoke_delegation".to_string(),
            bond_count: None,
        },
        storage::PendingProducerUpdate::RequestWithdrawal { bond_count, .. } => PendingUpdateInfo {
            update_type: "withdrawal".to_string(),
            bond_count: Some(*bond_count),
        },
    }
}

impl RpcContext {
    /// Get producer information by public key
    pub(super) async fn get_producer(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetProducerParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let pubkey = crypto::PublicKey::from_hex(&params.public_key)
            .map_err(|_| RpcError::invalid_params("Invalid public key format"))?;

        let producers = producer_set.read().await;
        let chain_state = self.chain_state.read().await;

        let info = producers
            .get_by_pubkey(&pubkey)
            .ok_or_else(RpcError::producer_not_found)?;

        let status = match &info.status {
            storage::ProducerStatus::Active => "active",
            storage::ProducerStatus::Unbonding { .. } => "unbonding",
            storage::ProducerStatus::Exited => "exited",
            storage::ProducerStatus::Slashed { .. } => "slashed",
        };

        // Derive bond data from UTXO set (source of truth)
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());
        let utxo_set = self.utxo_set.read().await;
        let utxo_bond_count = utxo_set.count_bonds(&pubkey_hash, self.bond_unit);
        let utxo_bonds = utxo_set.get_bond_entries(&pubkey_hash);
        let utxo_bond_amount: u64 = utxo_bonds.iter().map(|(_, _, amt)| *amt).sum();
        drop(utxo_set);

        // Use UTXO values when available, fall back to ProducerInfo (genesis)
        let effective_bond_count = if utxo_bond_count > 0 {
            utxo_bond_count
        } else {
            info.bond_count
        };
        let effective_bond_amount = if utxo_bond_amount > 0 {
            utxo_bond_amount
        } else {
            info.bond_amount
        };

        // Calculate current era
        let era = chain_state.best_height / self.params.blocks_per_era;

        // Withdrawal is instant -- pending_withdrawals always empty (kept for API compat)
        let pending_withdrawals: Vec<PendingWithdrawalResponse> = Vec::new();

        // Collect pending epoch-deferred updates for this producer
        let pending_updates: Vec<PendingUpdateInfo> = producers
            .pending_updates_for(&pubkey)
            .into_iter()
            .map(pending_update_to_info)
            .collect();

        let response = ProducerResponse {
            public_key: params.public_key,
            address_hash: hex::encode(pubkey_hash.as_bytes()),
            registration_height: info.registered_at,
            bond_amount: effective_bond_amount,
            bond_count: effective_bond_count,
            status: status.to_string(),
            era,
            pending_withdrawals,
            pending_updates,
            bls_pubkey: if info.bls_pubkey.is_empty() {
                String::new()
            } else {
                hex::encode(&info.bls_pubkey)
            },
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get all producers in the network
    pub(super) async fn get_producers(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetProducersParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let producers = producer_set.read().await;
        let chain_state = self.chain_state.read().await;
        let era = chain_state.best_height / self.params.blocks_per_era;

        let producer_list: Vec<&storage::ProducerInfo> = if params.active_only {
            producers.active_producers()
        } else {
            producers.all_producers()
        };

        // Build pending updates index once — O(M) instead of O(N×M)
        let pending_by_pubkey = producers.pending_updates_by_pubkey();

        // Derive bond data from UTXO set (source of truth)
        let utxo_set = self.utxo_set.read().await;

        let responses: Vec<ProducerResponse> = producer_list
            .iter()
            .map(|info| {
                let status = match &info.status {
                    storage::ProducerStatus::Active => "active",
                    storage::ProducerStatus::Unbonding { .. } => "unbonding",
                    storage::ProducerStatus::Exited => "exited",
                    storage::ProducerStatus::Slashed { .. } => "slashed",
                };

                // Use UTXO-derived bond count/amount when available, fall back to ProducerInfo
                let addr_hash = crypto::hash::hash_with_domain(
                    crypto::ADDRESS_DOMAIN,
                    info.public_key.as_bytes(),
                );
                let utxo_bond_count = utxo_set.count_bonds(&addr_hash, self.bond_unit);
                let utxo_bond_amount = utxo_set.get_bonded_balance(&addr_hash);
                let effective_bond_count = if utxo_bond_count > 0 {
                    utxo_bond_count
                } else {
                    info.bond_count
                };
                let effective_bond_amount = if utxo_bond_amount > 0 {
                    utxo_bond_amount
                } else {
                    info.bond_amount
                };

                // Withdrawal is instant -- pending_withdrawals always empty (kept for API compat)
                let pending_withdrawals: Vec<PendingWithdrawalResponse> = Vec::new();

                let pending_updates: Vec<PendingUpdateInfo> = pending_by_pubkey
                    .get(&info.public_key)
                    .map(|updates| updates.iter().map(|u| pending_update_to_info(u)).collect())
                    .unwrap_or_default();

                ProducerResponse {
                    public_key: hex::encode(info.public_key.as_bytes()),
                    address_hash: hex::encode(addr_hash.as_bytes()),
                    registration_height: info.registered_at,
                    bond_amount: effective_bond_amount,
                    bond_count: effective_bond_count,
                    status: status.to_string(),
                    era,
                    pending_withdrawals,
                    pending_updates,
                    bls_pubkey: if info.bls_pubkey.is_empty() {
                        String::new()
                    } else {
                        hex::encode(&info.bls_pubkey)
                    },
                }
            })
            .collect();

        drop(utxo_set);

        // Append pending registrations (not yet in producer set)
        let mut responses = responses;
        for info in producers.pending_registrations() {
            let addr_hash =
                crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, info.public_key.as_bytes());
            responses.push(ProducerResponse {
                public_key: hex::encode(info.public_key.as_bytes()),
                address_hash: hex::encode(addr_hash.as_bytes()),
                registration_height: info.registered_at,
                bond_amount: info.bond_amount,
                bond_count: info.bond_count,
                status: "pending".to_string(),
                era,
                pending_withdrawals: Vec::new(),
                pending_updates: vec![PendingUpdateInfo {
                    update_type: "register".to_string(),
                    bond_count: Some(info.bond_count),
                }],
                bls_pubkey: if info.bls_pubkey.is_empty() {
                    String::new()
                } else {
                    hex::encode(&info.bls_pubkey)
                },
            });
        }

        serde_json::to_value(responses).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get bond vesting details for a producer (per-bond granularity)
    pub(super) async fn get_bond_details(&self, params: Value) -> Result<Value, RpcError> {
        use doli_core::consensus::withdrawal_penalty_rate_with_quarter;

        let quarter = self.vesting_quarter_slots;
        let period = 4 * quarter;

        let params: GetBondDetailsParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let pubkey = crypto::PublicKey::from_hex(&params.public_key)
            .map_err(|_| RpcError::invalid_params("Invalid public key format"))?;

        let producers = producer_set.read().await;
        let chain_state = self.chain_state.read().await;
        let current_slot = chain_state.best_slot;

        let info = producers
            .get_by_pubkey(&pubkey)
            .ok_or_else(RpcError::producer_not_found)?;

        // Derive bond data from UTXO set (source of truth)
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());
        let utxo_set = self.utxo_set.read().await;
        let utxo_bonds = utxo_set.get_bond_entries(&pubkey_hash);
        let total_staked: u64 = utxo_bonds.iter().map(|(_, _, amt)| *amt).sum();
        let bond_count = if self.bond_unit > 0 {
            (total_staked / self.bond_unit) as u32
        } else {
            0
        };
        drop(utxo_set);

        // Build per-bond response from UTXO entries (already FIFO-sorted)
        let bonds: Vec<BondEntryResponse> = utxo_bonds
            .iter()
            .map(|(_, creation_slot, amount)| {
                let age = (current_slot as u64).saturating_sub(*creation_slot as u64);
                let penalty = withdrawal_penalty_rate_with_quarter(age as u32, quarter as u32);
                BondEntryResponse {
                    creation_slot: *creation_slot,
                    amount: *amount,
                    age_slots: age,
                    penalty_pct: penalty,
                    vested: age >= period,
                    maturation_slot: *creation_slot as u64 + period,
                }
            })
            .collect();

        // Compute summary from UTXO bonds
        let mut summary = BondsSummaryResponse {
            q1: 0,
            q2: 0,
            q3: 0,
            vested: 0,
        };
        for (_, creation_slot, _) in &utxo_bonds {
            let age = (current_slot as u64).saturating_sub(*creation_slot as u64);
            let quarters_elapsed = age / quarter;
            match quarters_elapsed {
                0 => summary.q1 += 1,
                1 => summary.q2 += 1,
                2 => summary.q3 += 1,
                _ => summary.vested += 1,
            }
        }

        // Overall vesting based on oldest bond
        let oldest_age = utxo_bonds
            .first()
            .map(|(_, cs, _)| (current_slot as u64).saturating_sub(*cs as u64))
            .unwrap_or(0);
        let overall_penalty =
            withdrawal_penalty_rate_with_quarter(oldest_age as u32, quarter as u32);
        let all_vested = utxo_bonds
            .iter()
            .all(|(_, cs, _)| (current_slot as u64).saturating_sub(*cs as u64) >= period);

        let response = BondDetailsResponse {
            public_key: params.public_key,
            bond_count,
            total_staked,
            registration_slot: info.registered_at,
            age_slots: oldest_age,
            penalty_pct: overall_penalty,
            vested: all_vested,
            maturation_slot: utxo_bonds
                .last()
                .map(|(_, cs, _)| *cs as u64 + period)
                .unwrap_or(0),
            vesting_quarter_slots: quarter,
            vesting_period_slots: period,
            summary,
            bonds,
            withdrawal_pending_count: info.withdrawal_pending_count,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}
