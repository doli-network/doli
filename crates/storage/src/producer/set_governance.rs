//! ProducerSet governance: weight, veto, governance, weighted rewards

use crypto::{Hash, PublicKey};
use doli_core::network::Network;

use super::constants::VETO_THRESHOLD_PERCENT;
use super::types::ProducerSet;

impl ProducerSet {
    // ==================== Seniority Weight Methods ====================

    /// Get total weight of all active producers
    ///
    /// Weight is based on seniority (time active in the network).
    /// Used for weighted veto calculations and reward distribution.
    pub fn total_weight(&self, current_height: u64) -> u64 {
        self.producers
            .values()
            .filter(|p| p.is_active())
            .map(|p| p.weight(current_height))
            .sum()
    }

    /// Get total weight for a specific network
    pub fn total_weight_for_network(&self, current_height: u64, network: Network) -> u64 {
        self.producers
            .values()
            .filter(|p| p.is_active())
            .map(|p| p.weight_for_network(current_height, network))
            .sum()
    }

    /// Get the weighted veto threshold
    ///
    /// Returns the minimum total weight required for a veto to pass (40%).
    pub fn weighted_veto_threshold(&self, current_height: u64) -> u64 {
        let total = self.total_weight(current_height);
        // 40% threshold, rounded up
        (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
    }

    /// Get the weighted veto threshold for a specific network
    pub fn weighted_veto_threshold_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> u64 {
        let total = self.total_weight_for_network(current_height, network);
        // 40% threshold, rounded up
        (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
    }

    /// Get total effective weight of producers with governance power
    ///
    /// **Only counts Active producers** (recently producing blocks).
    /// Inactive/dormant producers do NOT count - "el silencio no bloquea".
    ///
    /// Effective weight also accounts for activity gap penalties.
    pub fn total_effective_weight(&self, current_height: u64) -> u64 {
        self.producers
            .values()
            .filter(|p| p.has_governance_power(current_height))
            .map(|p| p.effective_weight(current_height))
            .sum()
    }

    /// Get total effective weight for a specific network
    ///
    /// **Only counts Active producers** for that network's threshold.
    pub fn total_effective_weight_for_network(&self, current_height: u64, network: Network) -> u64 {
        self.producers
            .values()
            .filter(|p| p.has_governance_power_for_network(current_height, network))
            .map(|p| p.effective_weight_for_network(current_height, network))
            .sum()
    }

    /// Get the effective veto threshold
    ///
    /// Uses effective weight of **only Active producers** as the base.
    /// Inactive/dormant producers don't affect the threshold denominator.
    /// Requires 40% of total effective weight to veto.
    pub fn effective_veto_threshold(&self, current_height: u64) -> u64 {
        let total = self.total_effective_weight(current_height);
        // 40% threshold, rounded up
        (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
    }

    /// Get the effective veto threshold for a specific network
    pub fn effective_veto_threshold_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> u64 {
        let total = self.total_effective_weight_for_network(current_height, network);
        // 40% threshold, rounded up
        (total * VETO_THRESHOLD_PERCENT).div_ceil(100)
    }

    /// Check if veto votes have reached the weighted threshold
    ///
    /// **Only Active producers can vote and count for quorum.**
    /// This implements "el silencio no bloquea" - inactive/dormant
    /// producers cannot block governance changes.
    ///
    /// # Arguments
    /// - `veto_pubkeys`: Public keys of producers who have voted to veto
    /// - `current_height`: Current block height for weight calculation
    ///
    /// # Returns
    /// `true` if the total effective weight of Active veto voters >= 40% of total Active weight
    pub fn has_weighted_veto(&self, veto_pubkeys: &[PublicKey], current_height: u64) -> bool {
        // Only count veto votes from producers with governance power (Active status)
        let veto_weight: u64 = veto_pubkeys
            .iter()
            .filter_map(|pk| self.get_by_pubkey(pk))
            .filter(|p| p.has_governance_power(current_height))
            .map(|p| p.effective_weight(current_height))
            .sum();

        let threshold = self.effective_veto_threshold(current_height);
        veto_weight >= threshold
    }

    /// Check if veto votes have reached the weighted threshold for a specific network
    ///
    /// **Only Active producers can vote and count for quorum.**
    pub fn has_weighted_veto_for_network(
        &self,
        veto_pubkeys: &[PublicKey],
        current_height: u64,
        network: Network,
    ) -> bool {
        // Only count veto votes from producers with governance power
        let veto_weight: u64 = veto_pubkeys
            .iter()
            .filter_map(|pk| self.get_by_pubkey(pk))
            .filter(|p| p.has_governance_power_for_network(current_height, network))
            .map(|p| p.effective_weight_for_network(current_height, network))
            .sum();

        let threshold = self.effective_veto_threshold_for_network(current_height, network);
        veto_weight >= threshold
    }

    /// Count producers with governance power (Active status)
    pub fn governance_participant_count(&self, current_height: u64) -> usize {
        self.producers
            .values()
            .filter(|p| p.has_governance_power(current_height))
            .count()
    }

    /// Count producers with governance power for a specific network
    pub fn governance_participant_count_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> usize {
        self.producers
            .values()
            .filter(|p| p.has_governance_power_for_network(current_height, network))
            .count()
    }

    /// Get all active producers with their weights
    ///
    /// Returns a vector of (PublicKey, weight) tuples for weighted selection.
    pub fn weighted_active_producers(&self, current_height: u64) -> Vec<(PublicKey, u64)> {
        self.producers
            .values()
            .filter(|p| p.is_active())
            .map(|p| (p.public_key, p.weight(current_height)))
            .collect()
    }

    /// Get all active producers with their weights for a specific network
    pub fn weighted_active_producers_for_network(
        &self,
        current_height: u64,
        network: Network,
    ) -> Vec<(PublicKey, u64)> {
        self.producers
            .values()
            .filter(|p| p.is_active())
            .map(|p| (p.public_key, p.weight_for_network(current_height, network)))
            .collect()
    }

    /// Distribute rewards proportionally by weight
    ///
    /// Instead of equal distribution, producers with higher seniority
    /// receive proportionally larger rewards.
    ///
    /// # Arguments
    /// - `total_reward`: Total reward to distribute
    /// - `current_height`: Current block height for weight calculation
    ///
    /// # Returns
    /// Number of producers who received rewards
    #[allow(deprecated)]
    pub fn distribute_weighted_rewards(&mut self, total_reward: u64, current_height: u64) -> usize {
        let total_weight = self.total_weight(current_height);
        if total_weight == 0 {
            return 0;
        }

        // Collect pubkeys and weights first to avoid borrow issues
        let distributions: Vec<(Hash, u64)> = self
            .producers
            .iter()
            .filter(|(_, p)| p.is_active())
            .map(|(hash, p)| {
                let weight = p.weight(current_height);
                let share = (total_reward * weight) / total_weight;
                (*hash, share)
            })
            .collect();

        let count = distributions.len();

        // Apply the distributions
        for (hash, share) in distributions {
            if let Some(producer) = self.producers.get_mut(&hash) {
                producer.credit_reward(share);
            }
        }

        count
    }

    /// Distribute rewards proportionally by weight for a specific network
    #[allow(deprecated)]
    pub fn distribute_weighted_rewards_for_network(
        &mut self,
        total_reward: u64,
        current_height: u64,
        network: Network,
    ) -> usize {
        let total_weight = self.total_weight_for_network(current_height, network);
        if total_weight == 0 {
            return 0;
        }

        // Collect pubkeys and weights first to avoid borrow issues
        let distributions: Vec<(Hash, u64)> = self
            .producers
            .iter()
            .filter(|(_, p)| p.is_active())
            .map(|(hash, p)| {
                let weight = p.weight_for_network(current_height, network);
                let share = (total_reward * weight) / total_weight;
                (*hash, share)
            })
            .collect();

        let count = distributions.len();

        // Apply the distributions
        for (hash, share) in distributions {
            if let Some(producer) = self.producers.get_mut(&hash) {
                producer.credit_reward(share);
            }
        }

        count
    }
}
