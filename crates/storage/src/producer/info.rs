//! ProducerInfo implementation methods

use crypto::Hash;
use crypto::PublicKey;
use doli_core::consensus::{
    withdrawal_penalty_rate_with_quarter, BOND_UNIT as CORE_BOND_UNIT, MAX_BONDS_PER_PRODUCER,
    VESTING_QUARTER_SLOTS,
};
use doli_core::network::Network;

use super::constants::*;
use super::seniority::*;
use super::types::{ActivityStatus, ProducerInfo, ProducerStatus, StoredBondEntry};

/// Calculate FIFO withdrawal from UTXO-derived bond entries.
///
/// `bonds` must be sorted by creation_slot ascending (oldest first) —
/// as returned by `UtxoSet::get_bond_entries()`.
///
/// Returns `Some((net_payout, total_penalty))` or `None` if count exceeds available.
pub fn calculate_withdrawal_from_bonds(
    bonds: &[(crate::utxo::Outpoint, u32, u64)],
    count: u32,
    current_slot: u32,
    quarter_slots: u64,
) -> Option<(u64, u64)> {
    if count == 0 || count as usize > bonds.len() {
        return None;
    }

    let mut total_net: u64 = 0;
    let mut total_penalty: u64 = 0;

    for &(_, creation_slot, amount) in bonds.iter().take(count as usize) {
        let age = (current_slot as u64).saturating_sub(creation_slot as u64) as u32;
        let penalty_pct =
            doli_core::consensus::withdrawal_penalty_rate_with_quarter(age, quarter_slots as u32);
        let penalty = (amount * penalty_pct as u64) / 100;
        let net = amount - penalty;
        total_net += net;
        total_penalty += penalty;
    }

    Some((total_net, total_penalty))
}

impl ProducerInfo {
    /// Create a new active producer with a single bond
    ///
    /// # Arguments
    /// - `public_key`: The producer's public key
    /// - `registered_at`: Block height at registration
    /// - `bond_amount`: Total bond amount in base units
    /// - `bond_outpoint`: UTXO reference for the bond
    /// - `registration_era`: Era at registration
    /// - `bond_unit`: Amount per bond (use `bond_unit_for_network(network)`)
    #[allow(deprecated)]
    pub fn new(
        public_key: PublicKey,
        registered_at: u64,
        bond_amount: u64,
        bond_outpoint: (Hash, u32),
        registration_era: u32,
        bond_unit: u64,
    ) -> Self {
        // Calculate bond count from amount using network-specific bond unit
        // For devnet: bond_unit = 100_000_000 (1 DOLI per bond)
        // For mainnet/testnet: bond_unit = 1_000_000_000 (10 DOLI per bond)
        let bond_unit = if bond_unit == 0 { BOND_UNIT } else { bond_unit };
        let bond_count = (bond_amount / bond_unit).min(MAX_BONDS_PER_PRODUCER as u64) as u32;
        let bond_count = bond_count.max(1); // Minimum 1 bond

        let bond_entries = (0..bond_count)
            .map(|_| StoredBondEntry {
                creation_slot: registered_at as u32,
                amount: bond_unit,
            })
            .collect();

        Self {
            public_key,
            registered_at,
            bond_amount,
            bond_outpoint,
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era,
            pending_rewards: 0,
            has_prior_exit: false,
            last_activity: registered_at,
            activity_gaps: 0,
            bond_count,
            additional_bonds: Vec::new(),
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: Vec::new(),
            bond_entries,
            withdrawal_pending_count: 0,
            bls_pubkey: Vec::new(),
            scheduled: true,
        }
    }

    /// Create a new producer with specified bond count (for multi-bond registration)
    #[allow(deprecated)]
    pub fn new_with_bonds(
        public_key: PublicKey,
        registered_at: u64,
        bond_amount: u64,
        bond_outpoint: (Hash, u32),
        registration_era: u32,
        bond_count: u32,
    ) -> Self {
        let bond_count = bond_count.clamp(1, MAX_BONDS_PER_PRODUCER);
        let bond_unit = if bond_count > 0 {
            bond_amount / bond_count as u64
        } else {
            CORE_BOND_UNIT
        };

        let bond_entries = (0..bond_count)
            .map(|_| StoredBondEntry {
                creation_slot: registered_at as u32,
                amount: bond_unit,
            })
            .collect();

        Self {
            public_key,
            registered_at,
            bond_amount,
            bond_outpoint,
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era,
            pending_rewards: 0,
            has_prior_exit: false,
            last_activity: registered_at,
            activity_gaps: 0,
            bond_count,
            additional_bonds: Vec::new(),
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: Vec::new(),
            bond_entries,
            withdrawal_pending_count: 0,
            bls_pubkey: Vec::new(),
            scheduled: true,
        }
    }

    /// Create a new producer with prior exit flag (for re-registration)
    ///
    /// Used when a producer who previously exited is re-registering.
    /// They start fresh with weight 1 (registered_at = current height).
    ///
    /// # Arguments
    /// * `bond_unit` - Amount per bond (use `NetworkParams::bond_unit()` or `bond_unit_for_network()`)
    #[allow(deprecated)]
    pub fn new_with_prior_exit(
        public_key: PublicKey,
        registered_at: u64,
        bond_amount: u64,
        bond_outpoint: (Hash, u32),
        registration_era: u32,
        bond_unit: u64,
    ) -> Self {
        let bond_unit = if bond_unit == 0 { BOND_UNIT } else { bond_unit };
        let bond_count = (bond_amount / bond_unit).min(MAX_BONDS_PER_PRODUCER as u64) as u32;
        let bond_count = bond_count.max(1);

        let bond_entries = (0..bond_count)
            .map(|_| StoredBondEntry {
                creation_slot: registered_at as u32,
                amount: bond_unit,
            })
            .collect();

        Self {
            public_key,
            registered_at,
            bond_amount,
            bond_outpoint,
            status: ProducerStatus::Active,
            blocks_produced: 0,
            slots_missed: 0,
            registration_era,
            pending_rewards: 0,
            has_prior_exit: true,
            last_activity: registered_at,
            activity_gaps: 0,
            bond_count,
            additional_bonds: Vec::new(),
            delegated_to: None,
            delegated_bonds: 0,
            received_delegations: Vec::new(),
            bond_entries,
            withdrawal_pending_count: 0,
            bls_pubkey: Vec::new(),
            scheduled: true,
        }
    }

    /// Check if producer is active (can produce blocks)
    /// Producers in unbonding period are still considered active
    pub fn is_active(&self) -> bool {
        matches!(
            self.status,
            ProducerStatus::Active | ProducerStatus::Unbonding { .. }
        )
    }

    /// Check if producer can produce blocks in the current slot
    /// Producers in unbonding period can still produce blocks
    pub fn can_produce(&self) -> bool {
        matches!(
            self.status,
            ProducerStatus::Active | ProducerStatus::Unbonding { .. }
        )
    }

    /// Check if bond lock period has expired
    pub fn is_bond_unlocked(&self, current_height: u64, lock_duration: u64) -> bool {
        current_height >= self.registered_at + lock_duration
    }

    /// Start unbonding period for exit
    pub fn start_unbonding(&mut self, current_height: u64) {
        self.status = ProducerStatus::Unbonding {
            started_at: current_height,
        };
    }

    /// Check if unbonding period has completed
    pub fn is_unbonding_complete(&self, current_height: u64, unbonding_duration: u64) -> bool {
        match self.status {
            ProducerStatus::Unbonding { started_at } => {
                current_height >= started_at + unbonding_duration
            }
            _ => false,
        }
    }

    /// Get the height when unbonding started (if in unbonding)
    pub fn unbonding_started_at(&self) -> Option<u64> {
        match self.status {
            ProducerStatus::Unbonding { started_at } => Some(started_at),
            _ => None,
        }
    }

    /// Complete the exit process
    pub fn complete_exit(&mut self) {
        self.status = ProducerStatus::Exited;
    }

    /// Apply slashing penalty (100% bond burned)
    /// Slashing is always complete - the full bond is destroyed
    pub fn slash(&mut self, current_height: u64) {
        self.status = ProducerStatus::Slashed {
            slashed_at: current_height,
        };
    }

    /// Get the slashed bond amount (100% for slashed producers)
    pub fn slashed_amount(&self) -> u64 {
        match self.status {
            ProducerStatus::Slashed { .. } => self.bond_amount,
            _ => 0,
        }
    }

    /// Get the remaining bond amount after slashing (always 0 for slashed)
    pub fn remaining_bond(&self) -> u64 {
        match self.status {
            ProducerStatus::Slashed { .. } => 0,
            _ => self.bond_amount,
        }
    }

    /// Check if producer has been slashed
    pub fn is_slashed(&self) -> bool {
        matches!(self.status, ProducerStatus::Slashed { .. })
    }

    // ==================== Bond Stacking Methods ====================

    /// Get the number of bonds staked
    pub fn bonds(&self) -> u32 {
        self.bond_count
    }

    /// Check if producer can add more bonds
    pub fn can_add_bonds(&self, count: u32) -> bool {
        self.is_active() && self.bond_count + count <= MAX_BONDS_PER_PRODUCER
    }

    /// Add additional bonds to this producer
    ///
    /// # Arguments
    /// * `bond_outpoints` - List of (tx_hash, output_index) for the new bonds
    /// * `amount_per_bond` - Amount per bond (should be BOND_UNIT)
    /// * `creation_slot` - Slot when these bonds were created (for vesting)
    ///
    /// # Returns
    /// Number of bonds actually added (may be less if hitting MAX_BONDS_PER_PRODUCER)
    pub fn add_bonds(
        &mut self,
        bond_outpoints: Vec<(Hash, u32)>,
        amount_per_bond: u64,
        creation_slot: u32,
    ) -> u32 {
        if !self.is_active() {
            return 0;
        }

        let available_slots = MAX_BONDS_PER_PRODUCER.saturating_sub(self.bond_count);
        let bonds_to_add = (bond_outpoints.len() as u32).min(available_slots);

        for outpoint in bond_outpoints.into_iter().take(bonds_to_add as usize) {
            self.additional_bonds.push(outpoint);
            self.bond_amount = self.bond_amount.saturating_add(amount_per_bond);
            self.bond_entries.push(StoredBondEntry {
                creation_slot,
                amount: amount_per_bond,
            });
        }

        self.bond_count += bonds_to_add;
        bonds_to_add
    }

    /// Remove bonds from this producer (for partial withdrawal)
    ///
    /// # Arguments
    /// * `count` - Number of bonds to remove
    /// * `bond_unit` - Amount per bond (use `NetworkParams::bond_unit()` or `bond_unit_for_network()`)
    ///
    /// # Returns
    /// List of bond outpoints that were removed (for unlocking)
    pub fn remove_bonds(&mut self, count: u32, bond_unit: u64) -> Vec<(Hash, u32)> {
        if count == 0 || !self.is_active() {
            return Vec::new();
        }

        // Can't remove all bonds - must keep at least 1
        let removable = self.bond_count.saturating_sub(1).min(count);
        if removable == 0 {
            return Vec::new();
        }

        let bond_unit = if bond_unit == 0 { BOND_UNIT } else { bond_unit };
        let mut removed = Vec::new();

        // Remove from additional_bonds first
        for _ in 0..removable {
            if let Some(outpoint) = self.additional_bonds.pop() {
                removed.push(outpoint);
                self.bond_amount = self.bond_amount.saturating_sub(bond_unit);
            }
        }

        self.bond_count -= removed.len() as u32;
        removed
    }

    /// Get all bond outpoints (primary + additional)
    pub fn all_bond_outpoints(&self) -> Vec<(Hash, u32)> {
        let mut all = vec![self.bond_outpoint];
        all.extend(self.additional_bonds.iter().cloned());
        all
    }

    /// Get the selection weight including bond delegations.
    ///
    /// This is used for weighted producer selection and tier computation.
    /// Includes own bonds plus all delegated bonds received from other producers.
    /// Returns 0 if producer is not active.
    pub fn selection_weight(&self) -> u64 {
        if self.is_active() {
            let own_bonds = self.bond_count as u64;
            let delegated: u64 = self
                .received_delegations
                .iter()
                .map(|(_, count)| *count as u64)
                .sum();
            own_bonds + delegated
        } else {
            0
        }
    }

    // ==================== Per-Bond Withdrawal Methods ====================

    /// Migrate legacy producers that have no bond_entries.
    ///
    /// If `bond_entries` is empty but `bond_count > 0`, populate with
    /// `bond_count` entries using `registered_at` as creation_slot and
    /// `bond_unit` as amount. Called after deserialization.
    pub fn migrate_bond_entries(&mut self, bond_unit: u64) {
        if self.bond_entries.is_empty() && self.bond_count > 0 {
            let unit = if bond_unit == 0 {
                CORE_BOND_UNIT
            } else {
                bond_unit
            };
            self.bond_entries = (0..self.bond_count)
                .map(|_| StoredBondEntry {
                    creation_slot: self.registered_at as u32,
                    amount: unit,
                })
                .collect();
        }
    }

    /// Calculate FIFO withdrawal amounts without mutating state.
    ///
    /// Returns `(net_amount, penalty_amount)` for withdrawing `count` oldest bonds.
    /// Returns `None` if not enough bonds available (considering pending withdrawals).
    /// Uses the mainnet vesting quarter (VESTING_QUARTER_SLOTS).
    pub fn calculate_withdrawal(&self, count: u32, current_slot: u32) -> Option<(u64, u64)> {
        self.calculate_withdrawal_with_quarter(count, current_slot, VESTING_QUARTER_SLOTS as u64)
    }

    /// Calculate FIFO withdrawal amounts with a custom quarter duration.
    ///
    /// Network-aware: pass `network_params.vesting_quarter_slots` for correct penalties.
    pub fn calculate_withdrawal_with_quarter(
        &self,
        count: u32,
        current_slot: u32,
        quarter_slots: u64,
    ) -> Option<(u64, u64)> {
        let available = self
            .bond_count
            .saturating_sub(self.withdrawal_pending_count);
        if count == 0 || count > available {
            return None;
        }

        let mut total_net: u64 = 0;
        let mut total_penalty: u64 = 0;

        for entry in self.bond_entries.iter().take(count as usize) {
            let age = (current_slot as u64).saturating_sub(entry.creation_slot as u64) as u32;
            let penalty_pct = withdrawal_penalty_rate_with_quarter(age, quarter_slots as u32);
            let penalty = (entry.amount * penalty_pct as u64) / 100;
            let net = entry.amount - penalty;
            total_net += net;
            total_penalty += penalty;
        }

        Some((total_net, total_penalty))
    }

    /// Apply a withdrawal at epoch boundary: remove oldest `count` entries.
    ///
    /// Decreases `bond_count`, `bond_amount`, and resets `withdrawal_pending_count`.
    /// Also removes corresponding entries from `additional_bonds`.
    pub fn apply_withdrawal(&mut self, count: u32, bond_unit: u64) {
        let unit = if bond_unit == 0 {
            CORE_BOND_UNIT
        } else {
            bond_unit
        };

        let to_remove = count.min(self.bond_entries.len() as u32);

        if to_remove > 0 {
            // Remove oldest entries (front of vec = FIFO)
            self.bond_entries.drain(..to_remove as usize);

            self.bond_count = self.bond_count.saturating_sub(to_remove);
            self.bond_amount = self.bond_amount.saturating_sub(to_remove as u64 * unit);
            self.withdrawal_pending_count = self.withdrawal_pending_count.saturating_sub(to_remove);

            // Remove corresponding additional_bonds (oldest first)
            for _ in 0..to_remove {
                if !self.additional_bonds.is_empty() {
                    self.additional_bonds.remove(0);
                }
            }
        }

        // Legacy fallback: bond_entries empty but bond_count > 0 (pre-StoredBondEntry producers)
        let remaining = count.saturating_sub(to_remove);
        if remaining > 0 && self.bond_count > 0 {
            let legacy_remove = remaining.min(self.bond_count);
            self.bond_count = self.bond_count.saturating_sub(legacy_remove);
            self.bond_amount = self.bond_amount.saturating_sub(legacy_remove as u64 * unit);
            self.withdrawal_pending_count =
                self.withdrawal_pending_count.saturating_sub(legacy_remove);
        }

        // Auto-exit when all bonds are withdrawn
        if self.bond_count == 0 {
            self.status = ProducerStatus::Exited;
        }
    }

    /// DEPRECATED: Credit reward to this producer's pending balance
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// The active EpochPool system distributes rewards directly to UTXOs.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn credit_reward(&mut self, amount: u64) {
        #[allow(deprecated)]
        {
            self.pending_rewards = self.pending_rewards.saturating_add(amount);
        }
    }

    /// DEPRECATED: Claim all pending rewards
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    /// The active EpochPool system distributes rewards directly to UTXOs.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn claim_rewards(&mut self) -> Option<u64> {
        #[allow(deprecated)]
        {
            if self.pending_rewards == 0 {
                return None;
            }
            let amount = self.pending_rewards;
            self.pending_rewards = 0;
            Some(amount)
        }
    }

    /// DEPRECATED: Check if producer has pending rewards to claim
    ///
    /// This method is vestigial from the Pull/Claim reward model.
    #[deprecated(note = "Pull/Claim model replaced by EpochPool - rewards go directly to UTXOs")]
    #[allow(dead_code)]
    pub fn has_pending_rewards(&self) -> bool {
        #[allow(deprecated)]
        {
            self.pending_rewards > 0
        }
    }

    /// Get the producer's base weight based on seniority
    ///
    /// Uses discrete yearly steps: 0-1yr=1, 1-2yr=2, 2-3yr=3, 3+yr=4 (max)
    #[allow(deprecated)]
    pub fn weight(&self, current_height: u64) -> u64 {
        producer_weight(self.registered_at, current_height)
    }

    /// Get the producer's base weight for a specific network
    ///
    /// Uses network-specific time acceleration for devnet/testnet.
    pub fn weight_for_network(&self, current_height: u64, network: Network) -> u64 {
        producer_weight_for_network(self.registered_at, current_height, network)
    }

    /// Get the producer's effective weight
    ///
    /// Per alignment decision: NO activity penalty. Weight is purely seniority-based.
    /// Producers who miss slots simply miss rewards - no weight reduction.
    /// Only slashable offense is double production (equivocation).
    ///
    /// Note: activity_gaps field is retained for tracking purposes but does not
    /// affect weight calculation.
    #[allow(deprecated)]
    pub fn effective_weight(&self, current_height: u64) -> u64 {
        // No penalty - just return base seniority weight
        producer_weight(self.registered_at, current_height)
    }

    /// Get the producer's effective weight with decimal precision
    ///
    /// Per alignment decision: NO activity penalty.
    pub fn effective_weight_precise(&self, current_height: u64) -> f64 {
        // No penalty - just return base seniority weight
        producer_weight_precise(self.registered_at, current_height)
    }

    /// Get the producer's effective weight for a specific network
    ///
    /// Per alignment decision: NO activity penalty.
    pub fn effective_weight_for_network(&self, current_height: u64, network: Network) -> u64 {
        // No penalty - just return base seniority weight
        producer_weight_for_network(self.registered_at, current_height, network)
    }

    /// Get the producer's effective weight with decimal precision for a specific network
    ///
    /// Per alignment decision: NO activity penalty.
    pub fn effective_weight_precise_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> f64 {
        // No penalty - just return base seniority weight
        producer_weight_precise_for_network(self.registered_at, current_height, network)
    }

    /// Record activity (block production or voting)
    ///
    /// Call this when the producer produces a block or casts a vote.
    /// Updates last_activity timestamp for activity status tracking.
    /// Note: Activity gaps are no longer tracked or penalized per alignment decision.
    pub fn record_activity(&mut self, current_height: u64) {
        self.last_activity = current_height;
    }

    /// Record activity with network-specific inactivity threshold
    ///
    /// Note: Activity gaps are no longer tracked or penalized per alignment decision.
    pub fn record_activity_for_network(&mut self, current_height: u64, _network: Network) {
        self.last_activity = current_height;
    }

    /// Check if producer has been inactive for too long (mainnet default)
    ///
    /// **Deprecated**: Use `is_inactive_for_network()` for network-aware calculations.
    #[deprecated(
        note = "Use is_inactive_for_network(current_height, network) for network-aware calculations"
    )]
    pub fn is_inactive(&self, current_height: u64) -> bool {
        #[allow(deprecated)]
        let threshold = INACTIVITY_THRESHOLD;
        current_height.saturating_sub(self.last_activity) > threshold
    }

    /// Check if producer has been inactive for a specific network
    pub fn is_inactive_for_network(&self, current_height: u64, network: Network) -> bool {
        current_height.saturating_sub(self.last_activity) > network.inactivity_threshold()
    }

    /// Get the current inactivity duration in blocks
    pub fn inactivity_duration(&self, current_height: u64) -> u64 {
        current_height.saturating_sub(self.last_activity)
    }

    /// DEPRECATED: Activity gaps are no longer tracked or penalized.
    /// This method is kept for API compatibility but does nothing meaningful.
    #[deprecated(note = "Activity gaps no longer affect consensus")]
    pub fn reset_activity_gaps(&mut self) {
        #[allow(deprecated)]
        {
            self.activity_gaps = 0;
        }
    }

    // ==================== Activity Status System ====================

    /// Determine the producer's current activity status (mainnet default)
    ///
    /// **Deprecated**: Use `activity_status_for_network()` for network-aware calculations.
    ///
    /// Based on how long since last block production:
    /// - Active: < 1 week (INACTIVITY_THRESHOLD)
    /// - RecentlyInactive: 1-2 weeks (grace period)
    /// - Dormant: >= 2 weeks
    #[deprecated(
        note = "Use activity_status_for_network(current_height, network) for network-aware calculations"
    )]
    pub fn activity_status(&self, current_height: u64) -> ActivityStatus {
        let blocks_since_last = current_height.saturating_sub(self.last_activity);

        #[allow(deprecated)]
        let threshold = INACTIVITY_THRESHOLD;

        if blocks_since_last < threshold {
            ActivityStatus::Active
        } else if blocks_since_last < threshold * 2 {
            ActivityStatus::RecentlyInactive
        } else {
            ActivityStatus::Dormant
        }
    }

    /// Determine the producer's activity status for a specific network
    pub fn activity_status_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> ActivityStatus {
        let blocks_since_last = current_height.saturating_sub(self.last_activity);
        let threshold = network.inactivity_threshold();

        if blocks_since_last < threshold {
            ActivityStatus::Active
        } else if blocks_since_last < threshold * 2 {
            ActivityStatus::RecentlyInactive
        } else {
            ActivityStatus::Dormant
        }
    }

    /// Does this producer have governance power (can vote, counts for quorum)?
    ///
    /// **Only Active producers have governance power.**
    /// This implements "el silencio no bloquea" - inactive producers
    /// cannot block governance changes.
    #[allow(deprecated)]
    pub fn has_governance_power(&self, current_height: u64) -> bool {
        self.is_active() && self.activity_status(current_height) == ActivityStatus::Active
    }

    /// Does this producer have governance power for a specific network?
    pub fn has_governance_power_for_network(&self, current_height: u64, network: Network) -> bool {
        self.is_active()
            && self.activity_status_for_network(current_height, network) == ActivityStatus::Active
    }

    /// Does this producer count for quorum calculations?
    ///
    /// Same as has_governance_power - only Active status counts.
    /// Inactive/dormant producers don't affect quorum denominator.
    pub fn counts_for_quorum(&self, current_height: u64) -> bool {
        self.has_governance_power(current_height)
    }

    /// Does this producer count for quorum for a specific network?
    pub fn counts_for_quorum_for_network(&self, current_height: u64, network: Network) -> bool {
        self.has_governance_power_for_network(current_height, network)
    }
}
