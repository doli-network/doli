//! ProducerSet core operations: constructors, lookups, queries, cache, pending updates

use std::collections::HashMap;

use crypto::hash::hash as crypto_hash;
use crypto::{Hash, PublicKey};

use super::constants::ACTIVATION_DELAY;
use super::types::{PendingProducerUpdate, ProducerInfo, ProducerSet, ProducerStatus};

impl ProducerSet {
    /// Create a new empty producer set
    pub fn new() -> Self {
        Self {
            producers: HashMap::new(),
            exit_history: HashMap::new(),
            active_cache: None,
            unbonding_index: std::collections::BTreeMap::new(),
            pending_updates: Vec::new(),
        }
    }

    /// Reconstruct a ProducerSet from raw parts (used by StateDb loading).
    pub fn from_parts(
        producers: HashMap<Hash, ProducerInfo>,
        exit_history: HashMap<Hash, u64>,
        pending_updates: Vec<PendingProducerUpdate>,
    ) -> Self {
        let mut set = Self {
            producers,
            exit_history,
            active_cache: None,
            unbonding_index: std::collections::BTreeMap::new(),
            pending_updates,
        };
        set.rebuild_unbonding_index();
        set
    }

    /// Borrow the raw parts for serialization into StateDb.
    pub fn as_parts(
        &self,
    ) -> (
        &HashMap<Hash, ProducerInfo>,
        &HashMap<Hash, u64>,
        &Vec<PendingProducerUpdate>,
    ) {
        (&self.producers, &self.exit_history, &self.pending_updates)
    }

    /// Rebuild the unbonding index from producers map.
    /// Called after deserialization to restore the skip-serialized index.
    pub fn rebuild_unbonding_index(&mut self) {
        self.unbonding_index.clear();
        for (key, info) in &self.producers {
            if let ProducerStatus::Unbonding { started_at } = info.status {
                self.unbonding_index
                    .entry(started_at)
                    .or_default()
                    .push(*key);
            }
        }
    }

    /// Clear all producers (used during chain reorganization)
    pub fn clear(&mut self) {
        self.producers.clear();
        self.active_cache = None;
        self.pending_updates.clear();
        // Keep exit_history as it's needed for anti-sybil checks
    }

    /// Queue a deferred producer mutation for application at the next epoch boundary.
    pub fn queue_update(&mut self, update: PendingProducerUpdate) {
        self.pending_updates.push(update);
    }

    /// Apply all pending producer mutations (called at epoch boundaries).
    /// After applying, the pending queue is cleared and caches invalidated.
    pub fn apply_pending_updates(&mut self) {
        if self.pending_updates.is_empty() {
            return;
        }
        let updates = std::mem::take(&mut self.pending_updates);
        for update in updates {
            match update {
                PendingProducerUpdate::Register { info, height } => {
                    if let Err(e) = self.register(*info, height) {
                        tracing::warn!("Deferred register failed: {}", e);
                    }
                }
                PendingProducerUpdate::Exit { pubkey, height } => {
                    if let Err(e) = self.request_exit(&pubkey, height) {
                        tracing::warn!("Deferred exit failed: {}", e);
                    }
                }
                PendingProducerUpdate::Slash { pubkey, height } => {
                    if let Err(e) = self.slash_producer(&pubkey, height) {
                        tracing::warn!("Deferred slash failed: {}", e);
                    }
                }
                PendingProducerUpdate::AddBond {
                    pubkey,
                    outpoints,
                    bond_unit,
                    creation_slot,
                } => {
                    if let Some(producer_info) = self.get_by_pubkey_mut(&pubkey) {
                        producer_info.add_bonds(outpoints, bond_unit, creation_slot);
                    }
                }
                PendingProducerUpdate::DelegateBond {
                    delegator,
                    delegate,
                    bond_count,
                } => {
                    let _ = self.delegate_bonds(&delegator, &delegate, bond_count);
                }
                PendingProducerUpdate::RevokeDelegation { delegator } => {
                    let _ = self.revoke_delegation(&delegator);
                }
                PendingProducerUpdate::RequestWithdrawal {
                    pubkey,
                    bond_count,
                    bond_unit,
                } => {
                    if let Some(producer_info) = self.get_by_pubkey_mut(&pubkey) {
                        producer_info.apply_withdrawal(bond_count, bond_unit);
                    }
                }
            }
        }
        self.active_cache = None;
    }

    /// Check if there are pending updates waiting to be applied.
    pub fn has_pending_updates(&self) -> bool {
        !self.pending_updates.is_empty()
    }

    /// Get the count of pending updates.
    pub fn pending_update_count(&self) -> usize {
        self.pending_updates.len()
    }

    /// Get pending updates for a specific producer (by public key).
    pub fn pending_updates_for(&self, pubkey: &PublicKey) -> Vec<&PendingProducerUpdate> {
        self.pending_updates
            .iter()
            .filter(|u| match u {
                PendingProducerUpdate::Register { info, .. } => info.public_key == *pubkey,
                PendingProducerUpdate::Exit { pubkey: pk, .. } => pk == pubkey,
                PendingProducerUpdate::Slash { pubkey: pk, .. } => pk == pubkey,
                PendingProducerUpdate::AddBond { pubkey: pk, .. } => pk == pubkey,
                PendingProducerUpdate::DelegateBond { delegator, .. } => delegator == pubkey,
                PendingProducerUpdate::RevokeDelegation { delegator } => delegator == pubkey,
                PendingProducerUpdate::RequestWithdrawal { pubkey: pk, .. } => pk == pubkey,
            })
            .collect()
    }

    /// Group all pending updates by their target public key in a single O(M) pass.
    ///
    /// Use this instead of calling `pending_updates_for()` N times (which is O(N×M)).
    pub fn pending_updates_by_pubkey(&self) -> HashMap<PublicKey, Vec<&PendingProducerUpdate>> {
        let mut map: HashMap<PublicKey, Vec<&PendingProducerUpdate>> =
            HashMap::with_capacity(self.pending_updates.len());
        for update in &self.pending_updates {
            let pk = match update {
                PendingProducerUpdate::Register { info, .. } => info.public_key,
                PendingProducerUpdate::Exit { pubkey, .. } => *pubkey,
                PendingProducerUpdate::Slash { pubkey, .. } => *pubkey,
                PendingProducerUpdate::AddBond { pubkey, .. } => *pubkey,
                PendingProducerUpdate::DelegateBond { delegator, .. } => *delegator,
                PendingProducerUpdate::RevokeDelegation { delegator } => *delegator,
                PendingProducerUpdate::RequestWithdrawal { pubkey, .. } => *pubkey,
            };
            map.entry(pk).or_default().push(update);
        }
        map
    }

    /// Get public keys of all pending registrations (for duplicate detection).
    pub fn pending_registration_keys(&self) -> Vec<crypto::PublicKey> {
        self.pending_updates
            .iter()
            .filter_map(|u| {
                if let PendingProducerUpdate::Register { info, .. } = u {
                    Some(info.public_key)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all pending registrations (producers not yet in the set).
    pub fn pending_registrations(&self) -> Vec<&ProducerInfo> {
        self.pending_updates
            .iter()
            .filter_map(|u| {
                if let PendingProducerUpdate::Register { info, .. } = u {
                    if !self
                        .producers
                        .contains_key(&crypto_hash(info.public_key.as_bytes()))
                    {
                        return Some(info.as_ref());
                    }
                }
                None
            })
            .collect()
    }

    /// Get producer info by public key hash
    pub fn get(&self, pubkey_hash: &Hash) -> Option<&ProducerInfo> {
        self.producers.get(pubkey_hash)
    }

    /// Get mutable producer info
    pub fn get_mut(&mut self, pubkey_hash: &Hash) -> Option<&mut ProducerInfo> {
        self.producers.get_mut(pubkey_hash)
    }

    /// Get producer by public key
    pub fn get_by_pubkey(&self, pubkey: &PublicKey) -> Option<&ProducerInfo> {
        self.producers.get(&crypto_hash(pubkey.as_bytes()))
    }

    /// Get mutable producer by public key
    pub fn get_by_pubkey_mut(&mut self, pubkey: &PublicKey) -> Option<&mut ProducerInfo> {
        self.producers.get_mut(&crypto_hash(pubkey.as_bytes()))
    }

    /// Get all active producers
    pub fn active_producers(&self) -> Vec<&ProducerInfo> {
        self.producers.values().filter(|p| p.is_active()).collect()
    }

    /// Get active producers that are eligible for scheduling at the given height.
    ///
    /// A producer is eligible for scheduling if:
    /// 1. They are in active status
    /// 2. They have passed the ACTIVATION_DELAY since registration
    ///
    /// Genesis producers (registered_at == 0) are exempt from ACTIVATION_DELAY
    /// because they are pre-registered in the chainspec and don't need network
    /// propagation time.
    ///
    /// This ensures all nodes have the same view of the producer set for a given
    /// height, preventing scheduling conflicts when new producers join.
    ///
    /// Uses epoch-based cache when available (call `ensure_active_cache()` to populate).
    pub fn active_producers_at_height(&self, current_height: u64) -> Vec<&ProducerInfo> {
        // Fast path: use cached keys if available and valid for this height
        if let Some((cached_height, ref keys)) = self.active_cache {
            if cached_height == current_height {
                return keys.iter().filter_map(|k| self.producers.get(k)).collect();
            }
        }

        // Slow path: full scan
        self.producers
            .values()
            .filter(|p| {
                p.is_active()
                    && (p.registered_at == 0
                        || current_height >= p.registered_at.saturating_add(ACTIVATION_DELAY))
            })
            .collect()
    }

    /// Pre-build the active producer cache for the given height.
    ///
    /// Call once per slot before the hot path to avoid repeated O(n) scans.
    /// The cache is automatically invalidated on any producer state mutation.
    pub fn ensure_active_cache(&mut self, current_height: u64) {
        if let Some((cached_height, _)) = &self.active_cache {
            if *cached_height == current_height {
                return; // Cache already valid
            }
        }

        let keys: Vec<Hash> = self
            .producers
            .iter()
            .filter(|(_, p)| {
                p.is_active()
                    && (p.registered_at == 0
                        || current_height >= p.registered_at.saturating_add(ACTIVATION_DELAY))
            })
            .map(|(k, _)| *k)
            .collect();

        self.active_cache = Some((current_height, keys));
    }

    /// Get all producers (all states)
    pub fn all_producers(&self) -> Vec<&ProducerInfo> {
        self.producers.values().collect()
    }

    /// Get count of active producers
    pub fn active_count(&self) -> usize {
        self.producers.values().filter(|p| p.is_active()).count()
    }

    /// Get count of active producers eligible for scheduling at the given height.
    pub fn active_count_at_height(&self, current_height: u64) -> usize {
        self.active_producers_at_height(current_height).len()
    }

    /// Get total producer count (all states)
    pub fn total_count(&self) -> usize {
        self.producers.len()
    }
}
