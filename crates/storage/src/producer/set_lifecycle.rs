//! ProducerSet lifecycle: exit, slash, unbonding, renew, cleanup + deprecated methods

use crypto::hash::hash as crypto_hash;
use crypto::{Hash, PublicKey};

use super::types::{ProducerInfo, ProducerSet, ProducerStatus};
use crate::StorageError;

impl ProducerSet {
    /// Process exit request for a producer (starts unbonding period)
    pub fn request_exit(
        &mut self,
        pubkey: &PublicKey,
        current_height: u64,
    ) -> Result<(), StorageError> {
        let info = self.get_by_pubkey_mut(pubkey).ok_or_else(|| {
            StorageError::NotFound(format!(
                "[STOR001] producer not found for exit request (pubkey={})",
                pubkey
            ))
        })?;

        let key = crypto_hash(pubkey.as_bytes());
        match info.status {
            ProducerStatus::Active => {
                info.start_unbonding(current_height);
                self.active_cache = None;
                // Maintain unbonding index
                self.unbonding_index
                    .entry(current_height)
                    .or_default()
                    .push(key);
                Ok(())
            }
            ProducerStatus::Unbonding { .. } => Err(StorageError::AlreadyExists(format!(
                "[STOR002] producer already in unbonding (pubkey={})",
                pubkey
            ))),
            ProducerStatus::Exited => Err(StorageError::AlreadyExists(format!(
                "[STOR003] producer already exited (pubkey={})",
                pubkey
            ))),
            ProducerStatus::Slashed { .. } => Err(StorageError::AlreadyExists(format!(
                "[STOR004] producer was slashed (pubkey={})",
                pubkey
            ))),
        }
    }

    /// Cancel an exit request during unbonding period
    ///
    /// Allows a producer to change their mind before exit is final.
    /// This is a safety feature for:
    /// - Accidental exit requests
    /// - Compromised wallet recovery
    /// - Changed circumstances
    ///
    /// Returns producer to Active status. No penalty is applied.
    /// Seniority (registered_at) is preserved.
    ///
    /// # Errors
    /// - NotFound: Producer doesn't exist
    /// - InvalidState: Producer is not in unbonding (already active, exited, or slashed)
    pub fn cancel_exit(&mut self, pubkey: &PublicKey) -> Result<(), StorageError> {
        let key = crypto_hash(pubkey.as_bytes());
        let info = self.get_by_pubkey_mut(pubkey).ok_or_else(|| {
            StorageError::NotFound(format!(
                "[STOR005] producer not found for cancel_exit (pubkey={})",
                pubkey
            ))
        })?;

        match info.status {
            ProducerStatus::Unbonding { started_at } => {
                info.status = ProducerStatus::Active;
                self.active_cache = None;
                // Remove from unbonding index
                if let Some(entries) = self.unbonding_index.get_mut(&started_at) {
                    entries.retain(|k| k != &key);
                    if entries.is_empty() {
                        self.unbonding_index.remove(&started_at);
                    }
                }
                Ok(())
            }
            ProducerStatus::Active => Err(StorageError::AlreadyExists(format!(
                "[STOR006] cannot cancel exit — producer already active (pubkey={})",
                pubkey
            ))),
            ProducerStatus::Exited => Err(StorageError::NotFound(format!(
                "[STOR007] cannot cancel exit — producer already exited (pubkey={})",
                pubkey
            ))),
            ProducerStatus::Slashed { .. } => Err(StorageError::NotFound(format!(
                "[STOR008] cannot cancel exit — producer was slashed (pubkey={})",
                pubkey
            ))),
        }
    }

    /// Process completed unbonding periods and mark as exited.
    ///
    /// Uses the unbonding index for O(k) lookup where k = completed producers,
    /// instead of O(n) scan of all producers.
    ///
    /// Returns producers that have completed unbonding and are ready for bond claim.
    /// Also records the exit in `exit_history` with the exit height for expiration tracking.
    pub fn process_unbonding(
        &mut self,
        current_height: u64,
        unbonding_duration: u64,
    ) -> Vec<ProducerInfo> {
        // Completion threshold: producers who started unbonding at or before this height are done
        let threshold = current_height.saturating_sub(unbonding_duration);

        // Collect all keys from the index with started_at ≤ threshold
        let mut candidate_keys: Vec<(u64, Hash)> = Vec::new();
        for (&started_at, keys) in self.unbonding_index.range(..=threshold) {
            for key in keys {
                candidate_keys.push((started_at, *key));
            }
        }

        let mut completed = Vec::new();

        for (started_at, key) in &candidate_keys {
            if let Some(info) = self.producers.get_mut(key) {
                if info.is_unbonding_complete(current_height, unbonding_duration) {
                    info.complete_exit();
                    completed.push(info.clone());
                    self.exit_history.insert(*key, current_height);
                }
            }
            // Remove from index
            if let Some(entries) = self.unbonding_index.get_mut(started_at) {
                entries.retain(|k| k != key);
            }
        }

        // Clean up empty index entries
        self.unbonding_index.retain(|_, v| !v.is_empty());

        if !completed.is_empty() {
            self.active_cache = None;
        }

        completed
    }

    /// Slash a producer for misbehavior (100% bond burned)
    ///
    /// Returns the amount burned. Also records the slash in exit_history
    /// so if they ever re-register, they start fresh (maturity cooldown).
    /// Exit record expires after EXIT_HISTORY_RETENTION (~8 years).
    pub fn slash_producer(
        &mut self,
        pubkey: &PublicKey,
        current_height: u64,
    ) -> Result<u64, StorageError> {
        let key = crypto_hash(pubkey.as_bytes());
        let info = self.get_by_pubkey_mut(pubkey).ok_or_else(|| {
            StorageError::NotFound(format!(
                "[STOR009] producer not found for slash (pubkey={})",
                pubkey
            ))
        })?;

        let slashed_amount = info.bond_amount;
        info.slash(current_height);

        // Record in exit history with current height (slashed producers lose all seniority)
        self.exit_history.insert(key, current_height);
        self.active_cache = None;

        Ok(slashed_amount)
    }

    /// Check if a producer can renew (bond lock expired)
    pub fn can_renew(&self, pubkey: &PublicKey, current_height: u64, lock_duration: u64) -> bool {
        self.get_by_pubkey(pubkey)
            .map(|info| info.is_bond_unlocked(current_height, lock_duration))
            .unwrap_or(false)
    }

    /// Renew a producer with a new bond (for era transition)
    pub fn renew(
        &mut self,
        pubkey: &PublicKey,
        new_bond_amount: u64,
        new_bond_outpoint: (Hash, u32),
        current_height: u64,
        new_era: u32,
    ) -> Result<u64, StorageError> {
        let info = self.get_by_pubkey_mut(pubkey).ok_or_else(|| {
            StorageError::NotFound(format!(
                "[STOR010] producer not found for renew (pubkey={})",
                pubkey
            ))
        })?;

        // Return old bond amount for release
        let old_bond = info.bond_amount;

        // Update to new bond
        info.bond_amount = new_bond_amount;
        info.bond_outpoint = new_bond_outpoint;
        info.registered_at = current_height;
        info.registration_era = new_era;
        info.status = ProducerStatus::Active;
        self.active_cache = None;

        Ok(old_bond)
    }

    /// Remove exited producers from the set (cleanup)
    pub fn cleanup_exited(&mut self) {
        let before = self.producers.len();
        self.producers
            .retain(|_, info| !matches!(info.status, ProducerStatus::Exited));
        if self.producers.len() != before {
            self.active_cache = None;
        }
    }

    /// Iterate over all producers
    pub fn iter(&self) -> impl Iterator<Item = (&Hash, &ProducerInfo)> {
        self.producers.iter()
    }

    /// DEPRECATED: Record a produced block for a producer
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// Block counts are now calculated from BlockStore on-demand.
    #[deprecated(note = "Block counts calculated from BlockStore - this is never called")]
    #[allow(dead_code)]
    pub fn record_block_produced(&mut self, pubkey: &PublicKey) {
        #[allow(deprecated)]
        if let Some(info) = self.get_by_pubkey_mut(pubkey) {
            info.blocks_produced += 1;
        }
    }

    /// Record a missed slot for a producer
    pub fn record_slot_missed(&mut self, pubkey: &PublicKey) {
        if let Some(info) = self.get_by_pubkey_mut(pubkey) {
            info.slots_missed += 1;
        }
    }

    /// DEPRECATED: Credit reward to a producer (Pull/Claim model)
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// The active EpochPool system distributes rewards directly to UTXOs.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn credit_reward(&mut self, pubkey: &PublicKey, amount: u64) {
        #[allow(deprecated)]
        if let Some(info) = self.get_by_pubkey_mut(pubkey) {
            info.credit_reward(amount);
        }
    }

    /// DEPRECATED: Process a reward claim from a producer
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn process_claim(&mut self, pubkey: &PublicKey) -> Option<u64> {
        #[allow(deprecated)]
        self.get_by_pubkey_mut(pubkey)?.claim_rewards()
    }

    /// DEPRECATED: Get pending rewards for a producer without claiming them
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn pending_rewards(&self, pubkey: &PublicKey) -> u64 {
        #[allow(deprecated)]
        self.get_by_pubkey(pubkey)
            .map(|info| info.pending_rewards)
            .unwrap_or(0)
    }

    /// DEPRECATED: Distribute block reward among producers (Pull/Claim model)
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// The active EpochPool system distributes rewards via EpochReward transactions
    /// calculated deterministically from the BlockStore.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - see calculate_epoch_rewards()")]
    #[allow(dead_code)]
    pub fn distribute_block_reward(&mut self, producer_pubkey: &PublicKey, reward: u64) {
        #[allow(deprecated)]
        {
            self.credit_reward(producer_pubkey, reward);
            self.record_block_produced(producer_pubkey);
        }
    }
}
