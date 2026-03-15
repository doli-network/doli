//! ProducerSet delegation: delegate_bonds, revoke_delegation

use crypto::hash::hash as crypto_hash;
use crypto::PublicKey;

use super::types::ProducerSet;

impl ProducerSet {
    /// Delegate bonds from one producer to another.
    ///
    /// The delegator's `delegated_to` and `delegated_bonds` are set.
    /// The delegatee's `received_delegations` list is updated.
    /// Returns an error string if either producer doesn't exist or isn't active.
    pub fn delegate_bonds(
        &mut self,
        delegator_pubkey: &PublicKey,
        delegatee_pubkey: &PublicKey,
        bond_count: u32,
    ) -> Result<(), String> {
        let delegator_hash = crypto_hash(delegator_pubkey.as_bytes());
        let delegatee_hash = crypto_hash(delegatee_pubkey.as_bytes());

        // Validate delegator exists and is active
        let delegator = self
            .producers
            .get(&delegator_hash)
            .ok_or("delegator not found")?;
        if !delegator.is_active() {
            return Err("delegator is not active".into());
        }
        if bond_count > delegator.bond_count {
            return Err(format!(
                "insufficient bonds: has {}, delegating {}",
                delegator.bond_count, bond_count
            ));
        }
        if delegator.delegated_to.is_some() {
            return Err("delegator already has an active delegation".into());
        }

        // Validate delegatee exists and is active
        let delegatee = self
            .producers
            .get(&delegatee_hash)
            .ok_or("delegatee not found")?;
        if !delegatee.is_active() {
            return Err("delegatee is not active".into());
        }

        // Apply delegation
        if let Some(delegator) = self.producers.get_mut(&delegator_hash) {
            delegator.delegated_to = Some(*delegatee_pubkey);
            delegator.delegated_bonds = bond_count;
        }
        if let Some(delegatee) = self.producers.get_mut(&delegatee_hash) {
            delegatee
                .received_delegations
                .push((delegator_hash, bond_count));
        }

        self.active_cache = None;
        Ok(())
    }

    /// Revoke delegation from a producer.
    ///
    /// Clears the delegator's delegation state and removes the entry
    /// from the delegatee's received_delegations list.
    pub fn revoke_delegation(&mut self, delegator_pubkey: &PublicKey) -> Result<(), String> {
        let delegator_hash = crypto_hash(delegator_pubkey.as_bytes());

        let delegator = self
            .producers
            .get(&delegator_hash)
            .ok_or("delegator not found")?;
        let delegatee_pubkey = delegator
            .delegated_to
            .ok_or("no active delegation to revoke")?;

        let delegatee_hash = crypto_hash(delegatee_pubkey.as_bytes());

        // Clear delegator state
        if let Some(delegator) = self.producers.get_mut(&delegator_hash) {
            delegator.delegated_to = None;
            delegator.delegated_bonds = 0;
        }

        // Remove from delegatee's received list
        if let Some(delegatee) = self.producers.get_mut(&delegatee_hash) {
            delegatee
                .received_delegations
                .retain(|(hash, _)| hash != &delegator_hash);
        }

        self.active_cache = None;
        Ok(())
    }
}
