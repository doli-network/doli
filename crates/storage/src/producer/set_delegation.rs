//! ProducerSet delegation: delegate_bonds, revoke_delegation

use crypto::hash::hash as crypto_hash;
use crypto::{Hash, PublicKey};

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
        // AUDIT-PROD-002: Prevent self-delegation (always enforced — no
        // self-delegation transactions exist on chain prior to this fix).
        if bond_count == 0 {
            return Err("cannot delegate 0 bonds".into());
        }
        if delegator_pubkey == delegatee_pubkey {
            return Err("cannot delegate bonds to self".into());
        }
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
        // INC-I-056: Available bonds = total - pending_withdrawal - already_delegated
        let available = delegator
            .bond_count
            .saturating_sub(delegator.withdrawal_pending_count);
        if bond_count > available {
            return Err(format!(
                "insufficient bonds: has {}, pending_withdrawal={}, available={}, delegating {}",
                delegator.bond_count, delegator.withdrawal_pending_count, available, bond_count
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

    /// Clean up ALL delegation state for a producer being removed (exit/slash).
    ///
    /// Handles both directions:
    /// 1. **Incoming**: Resets each delegator's `delegated_to` and `delegated_bonds`,
    ///    then clears this producer's `received_delegations`.
    /// 2. **Outgoing**: If this producer delegated to someone, removes the entry from
    ///    the delegatee's `received_delegations` and clears own `delegated_to`/`delegated_bonds`.
    ///
    /// Idempotent: no-op if no delegations exist. Does NOT return errors.
    pub fn cleanup_all_delegations(&mut self, pubkey_hash: &Hash) {
        // --- Incoming: clean delegators who delegated TO this producer ---
        let delegator_hashes: Vec<(Hash, u32)> = self
            .producers
            .get(pubkey_hash)
            .map(|p| p.received_delegations.clone())
            .unwrap_or_default();

        for (delegator_hash, _) in &delegator_hashes {
            if let Some(delegator) = self.producers.get_mut(delegator_hash) {
                delegator.delegated_to = None;
                delegator.delegated_bonds = 0;
            }
        }

        // --- Outgoing: clean delegatee who received FROM this producer ---
        let delegatee_pubkey = self.producers.get(pubkey_hash).and_then(|p| p.delegated_to);

        if let Some(delegatee_pk) = delegatee_pubkey {
            let delegatee_hash = crypto_hash(delegatee_pk.as_bytes());
            if let Some(delegatee) = self.producers.get_mut(&delegatee_hash) {
                delegatee
                    .received_delegations
                    .retain(|(hash, _)| hash != pubkey_hash);
            }
        }

        // --- Clear this producer's own delegation fields ---
        if let Some(producer) = self.producers.get_mut(pubkey_hash) {
            producer.received_delegations.clear();
            producer.delegated_to = None;
            producer.delegated_bonds = 0;
        }

        self.active_cache = None;
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
