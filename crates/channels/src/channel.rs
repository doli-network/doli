//! Channel record — the main state container persisted to disk.

use chrono::{DateTime, Utc};
use doli_core::Amount;
use serde::{Deserialize, Serialize};

use crate::commitment::RevocationStore;
use crate::types::{
    ChannelBalance, ChannelId, ChannelState, CommitmentNumber, FundingOutpoint, InFlightHtlc,
};

/// Persistent record of a single payment channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelRecord {
    /// Unique channel identifier.
    pub channel_id: ChannelId,
    /// Current lifecycle state.
    pub state: ChannelState,
    /// Our pubkey hash.
    pub local_pubkey_hash: [u8; 32],
    /// Counterparty's pubkey hash.
    pub remote_pubkey_hash: [u8; 32],
    /// Funding outpoint on L1.
    pub funding_outpoint: FundingOutpoint,
    /// Total channel capacity.
    pub capacity: Amount,
    /// Current balance distribution.
    pub balance: ChannelBalance,
    /// Current commitment number.
    pub commitment_number: CommitmentNumber,
    /// Seed for deriving revocation preimages.
    pub channel_seed: [u8; 32],
    /// Stored revocation preimages from counterparty (for penalty enforcement).
    pub revocation_store: RevocationStore,
    /// Dispute window in blocks.
    pub dispute_window: u64,
    /// In-flight HTLCs.
    pub htlcs: Vec<InFlightHtlc>,
    /// Funding tx confirmation count.
    pub funding_confirmations: u32,
    /// Timestamps.
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Close transaction hash (if closing/closed).
    pub close_tx_hash: Option<String>,
    /// Penalty transaction hash (if revoked commitment detected).
    pub penalty_tx_hash: Option<String>,
}

impl ChannelRecord {
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// Transition to a new state.
    ///
    /// AUDIT-CHAN-002: Now enforces validate_transition() — previously
    /// the state machine validation existed but was never called.
    pub fn transition(&mut self, new_state: ChannelState) -> crate::error::Result<()> {
        crate::state_machine::validate_transition(&self.state, &new_state)?;
        self.state = new_state;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Lazily advance a funding-broadcast channel to `Active` once its funding
    /// transaction has reached the required confirmation depth.
    ///
    /// INC-I-097: nothing else in the client observes on-chain funding
    /// confirmation, so callers (the CLI channel commands) invoke this after
    /// querying the node for the funding tx's confirmation count. Pure and
    /// I/O-free so the activation rule is unit-testable without a node.
    ///
    /// Records the observed confirmation count while in `FundingBroadcast`, and
    /// requires at least one confirmation regardless of `required` to avoid
    /// activating an unconfirmed channel under a misconfigured `required == 0`.
    /// Returns `true` iff the state changed to `Active`.
    pub fn try_activate(&mut self, confirmations: u32, required: u32) -> bool {
        if self.state != ChannelState::FundingBroadcast {
            return false;
        }
        self.funding_confirmations = confirmations;
        self.updated_at = Utc::now();
        if confirmations >= required.max(1) {
            // validate_transition() permits FundingBroadcast -> Active.
            self.transition(ChannelState::Active).is_ok()
        } else {
            false
        }
    }

    /// Advance the commitment number and return the new number.
    pub fn advance_commitment(&mut self) -> CommitmentNumber {
        self.commitment_number += 1;
        self.updated_at = Utc::now();
        self.commitment_number
    }

    /// Update the balance after a successful off-chain payment.
    pub fn update_balance(&mut self, new_balance: ChannelBalance) {
        self.balance = new_balance;
        self.updated_at = Utc::now();
    }

    /// Store a revocation preimage received from the counterparty.
    pub fn store_revocation(&mut self, number: CommitmentNumber, preimage: [u8; 32]) {
        self.revocation_store.add(number, preimage);
        self.updated_at = Utc::now();
    }
}
