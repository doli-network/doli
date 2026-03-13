//! HTLC lifecycle management on commitment transactions.
//!
//! Manages the state of in-flight HTLCs across commitment updates:
//! add → fulfill/fail → settle into balance.

use doli_core::{Amount, BlockHeight};
use serde::{Deserialize, Serialize};

use crate::error::{ChannelError, Result};
use crate::types::{HtlcState, InFlightHtlc, PaymentDirection};

/// HTLC manager for a single channel.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HtlcManager {
    /// Next HTLC ID to assign.
    next_id: u64,
    /// All in-flight HTLCs.
    htlcs: Vec<InFlightHtlc>,
}

impl HtlcManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new outgoing HTLC. Returns the HTLC ID.
    pub fn add_outgoing(
        &mut self,
        payment_hash: [u8; 32],
        amount: Amount,
        expiry_height: BlockHeight,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.htlcs.push(InFlightHtlc {
            htlc_id: id,
            payment_hash,
            amount,
            expiry_height,
            direction: PaymentDirection::Outgoing,
            state: HtlcState::Pending,
            preimage: None,
        });
        id
    }

    /// Add a new incoming HTLC. Returns the HTLC ID.
    pub fn add_incoming(
        &mut self,
        htlc_id: u64,
        payment_hash: [u8; 32],
        amount: Amount,
        expiry_height: BlockHeight,
    ) {
        self.htlcs.push(InFlightHtlc {
            htlc_id,
            payment_hash,
            amount,
            expiry_height,
            direction: PaymentDirection::Incoming,
            state: HtlcState::Pending,
            preimage: None,
        });
        if htlc_id >= self.next_id {
            self.next_id = htlc_id + 1;
        }
    }

    /// Fulfill an HTLC with a preimage.
    pub fn fulfill(&mut self, htlc_id: u64, preimage: [u8; 32]) -> Result<Amount> {
        let htlc = self
            .htlcs
            .iter_mut()
            .find(|h| h.htlc_id == htlc_id && h.state == HtlcState::Pending)
            .ok_or_else(|| ChannelError::HtlcNotFound(htlc_id.to_string()))?;

        // Verify preimage matches payment hash
        let hash = crypto::hash::hash_with_domain(doli_core::HASHLOCK_DOMAIN, &preimage);
        if *hash.as_bytes() != htlc.payment_hash {
            return Err(ChannelError::InvalidRevocation);
        }

        htlc.state = HtlcState::Fulfilled;
        htlc.preimage = Some(preimage);
        Ok(htlc.amount)
    }

    /// Mark an HTLC as expired.
    pub fn expire(&mut self, htlc_id: u64, current_height: BlockHeight) -> Result<Amount> {
        let htlc = self
            .htlcs
            .iter_mut()
            .find(|h| h.htlc_id == htlc_id && h.state == HtlcState::Pending)
            .ok_or_else(|| ChannelError::HtlcNotFound(htlc_id.to_string()))?;

        if current_height < htlc.expiry_height {
            return Err(ChannelError::Protocol(format!(
                "HTLC {} not yet expired (current: {}, expiry: {})",
                htlc_id, current_height, htlc.expiry_height
            )));
        }

        htlc.state = HtlcState::Expired;
        Ok(htlc.amount)
    }

    /// Resolve a fulfilled or expired HTLC (remove from commitment).
    pub fn resolve(&mut self, htlc_id: u64) -> Result<()> {
        let htlc = self
            .htlcs
            .iter_mut()
            .find(|h| h.htlc_id == htlc_id)
            .ok_or_else(|| ChannelError::HtlcNotFound(htlc_id.to_string()))?;

        if !matches!(htlc.state, HtlcState::Fulfilled | HtlcState::Expired) {
            return Err(ChannelError::Protocol(format!(
                "HTLC {} cannot be resolved in state {:?}",
                htlc_id, htlc.state
            )));
        }

        htlc.state = HtlcState::Resolved;
        Ok(())
    }

    /// Get all pending HTLCs.
    pub fn pending(&self) -> Vec<&InFlightHtlc> {
        self.htlcs
            .iter()
            .filter(|h| h.state == HtlcState::Pending)
            .collect()
    }

    /// Get total amount locked in outgoing pending HTLCs.
    pub fn total_outgoing_pending(&self) -> Amount {
        self.htlcs
            .iter()
            .filter(|h| h.state == HtlcState::Pending && h.direction == PaymentDirection::Outgoing)
            .map(|h| h.amount)
            .sum()
    }

    /// Get total amount locked in incoming pending HTLCs.
    pub fn total_incoming_pending(&self) -> Amount {
        self.htlcs
            .iter()
            .filter(|h| h.state == HtlcState::Pending && h.direction == PaymentDirection::Incoming)
            .map(|h| h.amount)
            .sum()
    }

    /// Get all HTLCs (for commitment construction).
    pub fn all(&self) -> &[InFlightHtlc] {
        &self.htlcs
    }

    /// Remove all resolved HTLCs.
    pub fn gc_resolved(&mut self) {
        self.htlcs.retain(|h| h.state != HtlcState::Resolved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_payment_hash(val: u8) -> [u8; 32] {
        let preimage = [val; 32];
        let hash = crypto::hash::hash_with_domain(doli_core::HASHLOCK_DOMAIN, &preimage);
        *hash.as_bytes()
    }

    #[test]
    fn add_and_fulfill_htlc() {
        let mut mgr = HtlcManager::new();
        let preimage = [42u8; 32];
        let payment_hash = test_payment_hash(42);

        let id = mgr.add_outgoing(payment_hash, 1000, 100);
        assert_eq!(mgr.pending().len(), 1);

        let amount = mgr.fulfill(id, preimage).unwrap();
        assert_eq!(amount, 1000);
        assert_eq!(mgr.pending().len(), 0);
    }

    #[test]
    fn fulfill_wrong_preimage_fails() {
        let mut mgr = HtlcManager::new();
        let payment_hash = test_payment_hash(42);
        let id = mgr.add_outgoing(payment_hash, 1000, 100);

        let wrong_preimage = [99u8; 32];
        assert!(mgr.fulfill(id, wrong_preimage).is_err());
    }

    #[test]
    fn expire_htlc() {
        let mut mgr = HtlcManager::new();
        let payment_hash = test_payment_hash(42);
        let id = mgr.add_outgoing(payment_hash, 1000, 100);

        // Can't expire before expiry height
        assert!(mgr.expire(id, 99).is_err());
        // Can expire at or after
        let amount = mgr.expire(id, 100).unwrap();
        assert_eq!(amount, 1000);
    }

    #[test]
    fn pending_totals() {
        let mut mgr = HtlcManager::new();
        mgr.add_outgoing(test_payment_hash(1), 100, 50);
        mgr.add_outgoing(test_payment_hash(2), 200, 60);
        mgr.add_incoming(10, test_payment_hash(3), 300, 70);

        assert_eq!(mgr.total_outgoing_pending(), 300);
        assert_eq!(mgr.total_incoming_pending(), 300);
    }
}
