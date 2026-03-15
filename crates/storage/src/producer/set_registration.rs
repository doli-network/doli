//! ProducerSet registration: register, genesis producers, exit history

use crypto::hash::hash as crypto_hash;
use crypto::{Hash, PublicKey};
use doli_core::network::Network;

use super::constants::EXIT_HISTORY_RETENTION;
use super::types::{ProducerInfo, ProducerSet, ProducerStatus, StoredBondEntry};
use crate::StorageError;

impl ProducerSet {
    /// Register a new producer
    ///
    /// If the producer has previously exited (in exit_history and not expired),
    /// they will have `has_prior_exit = true` and start with weight 1 (maturity cooldown).
    ///
    /// Exit history expires after EXIT_HISTORY_RETENTION (~8 years).
    pub fn register(
        &mut self,
        mut info: ProducerInfo,
        current_height: u64,
    ) -> Result<(), StorageError> {
        let key = crypto_hash(info.public_key.as_bytes());
        if let Some(existing) = self.producers.get(&key) {
            if existing.status != ProducerStatus::Exited {
                return Err(StorageError::AlreadyExists(
                    "Producer already registered".to_string(),
                ));
            }
            // Allow re-registration after exit — replace the old entry
        }

        // Anti-Sybil: Check if this producer has recently exited (not expired)
        // If so, mark them as having prior exit (they start fresh with weight 1)
        if self.exited_recently(&key, current_height) {
            info.has_prior_exit = true;
        }

        self.producers.insert(key, info);
        self.active_cache = None;
        Ok(())
    }

    /// Register a new producer with network-specific exit history retention
    pub fn register_for_network(
        &mut self,
        mut info: ProducerInfo,
        current_height: u64,
        network: Network,
    ) -> Result<(), StorageError> {
        let key = crypto_hash(info.public_key.as_bytes());
        if let Some(existing) = self.producers.get(&key) {
            if existing.status != ProducerStatus::Exited {
                return Err(StorageError::AlreadyExists(
                    "Producer already registered".to_string(),
                ));
            }
            // Allow re-registration after exit — replace the old entry
        }

        // Anti-Sybil: Check if this producer has recently exited (not expired for this network)
        if self.exited_recently_for_network(&key, current_height, network) {
            info.has_prior_exit = true;
        }

        self.producers.insert(key, info);
        self.active_cache = None;
        Ok(())
    }

    /// Check if a pubkey hash has recently exited (within retention period)
    fn exited_recently(&self, pubkey_hash: &Hash, current_height: u64) -> bool {
        match self.exit_history.get(pubkey_hash) {
            Some(&exit_height) => {
                current_height.saturating_sub(exit_height) < EXIT_HISTORY_RETENTION
            }
            None => false,
        }
    }

    /// Check if a pubkey hash has recently exited for a specific network
    fn exited_recently_for_network(
        &self,
        pubkey_hash: &Hash,
        current_height: u64,
        network: Network,
    ) -> bool {
        match self.exit_history.get(pubkey_hash) {
            Some(&exit_height) => {
                current_height.saturating_sub(exit_height) < network.exit_history_retention()
            }
            None => false,
        }
    }

    /// Check if a pubkey has previously exited (for anti-Sybil verification)
    ///
    /// Note: This checks if the exit is still within the retention period.
    /// After 8 years, the exit record expires and this returns false.
    pub fn has_prior_exit(&self, pubkey: &PublicKey, current_height: u64) -> bool {
        let key = crypto_hash(pubkey.as_bytes());
        self.exited_recently(&key, current_height)
    }

    /// Check if a pubkey has previously exited for a specific network
    pub fn has_prior_exit_for_network(
        &self,
        pubkey: &PublicKey,
        current_height: u64,
        network: Network,
    ) -> bool {
        let key = crypto_hash(pubkey.as_bytes());
        self.exited_recently_for_network(&key, current_height, network)
    }

    /// Prune expired exit history entries
    ///
    /// Call this periodically (e.g., once per era) to clean up old entries
    /// and bound the exit_history size.
    pub fn prune_exit_history(&mut self, current_height: u64) {
        self.exit_history.retain(|_, &mut exit_height| {
            current_height.saturating_sub(exit_height) < EXIT_HISTORY_RETENTION
        });
    }

    /// Prune expired exit history entries for a specific network
    pub fn prune_exit_history_for_network(&mut self, current_height: u64, network: Network) {
        let retention = network.exit_history_retention();
        self.exit_history
            .retain(|_, &mut exit_height| current_height.saturating_sub(exit_height) < retention);
    }

    /// Register a genesis producer (special registration without VDF/UTXO verification)
    ///
    /// Genesis producers are registered at height 0 with synthetic bond outpoints
    /// (Hash::ZERO). They cannot unbond their genesis bonds but can participate
    /// in block production from the start.
    ///
    /// # Arguments
    /// - `pubkey`: Producer's public key
    /// - `bond_count`: Number of bonds (typically 1)
    ///
    /// # Returns
    /// `Ok(())` if registered, `Err` if already exists
    #[allow(deprecated)]
    pub fn register_genesis_producer(
        &mut self,
        pubkey: PublicKey,
        bond_count: u32,
        bond_unit: u64,
    ) -> Result<(), StorageError> {
        let key = crypto_hash(pubkey.as_bytes());
        if self.producers.contains_key(&key) {
            return Err(StorageError::AlreadyExists(
                "Genesis producer already registered".to_string(),
            ));
        }

        // Use network-specific bond_unit instead of hardcoded BOND_UNIT
        let bond_amount = (bond_count as u64) * bond_unit;
        let bond_entries = (0..bond_count)
            .map(|_| StoredBondEntry {
                creation_slot: 0,
                amount: bond_unit,
            })
            .collect();
        let info = ProducerInfo {
            public_key: pubkey,
            registered_at: 0, // Genesis registration
            bond_amount,
            bond_outpoint: (Hash::ZERO, 0), // Synthetic - cannot unbond
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era: 0,
            pending_rewards: 0,
            has_prior_exit: false,
            last_activity: 0,
            activity_gaps: 0,
            bond_count,
            additional_bonds: Vec::new(),
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: Vec::new(),
            bond_entries,
            withdrawal_pending_count: 0,
            bls_pubkey: Vec::new(),
        };

        self.producers.insert(key, info);
        self.active_cache = None;
        Ok(())
    }

    /// Create a ProducerSet with genesis producers pre-registered
    ///
    /// # Arguments
    /// - `producers`: Vec of (PublicKey, bond_count) tuples
    /// - `bond_unit`: Network-specific bond unit (e.g., 1 DOLI for devnet)
    pub fn with_genesis_producers(producers: Vec<(PublicKey, u32)>, bond_unit: u64) -> Self {
        let mut set = Self::new();
        for (pubkey, bond_count) in producers {
            // Ignore errors (shouldn't happen for fresh set)
            let _ = set.register_genesis_producer(pubkey, bond_count, bond_unit);
        }
        set
    }

    /// Get the size of the exit history (for monitoring)
    pub fn exit_history_size(&self) -> usize {
        self.exit_history.len()
    }
}
