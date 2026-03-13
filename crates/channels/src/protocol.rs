//! Off-chain channel protocol messages.
//!
//! These messages are exchanged between channel peers to negotiate opens,
//! updates, and closes. In Phase 1 they are relayed via RPC; Phase 2 adds
//! libp2p request-response.

use doli_core::{Amount, BlockHeight};
use serde::{Deserialize, Serialize};

/// Channel protocol messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChannelMessage {
    OpenChannel(OpenChannel),
    AcceptChannel(AcceptChannel),
    FundingCreated(FundingCreated),
    FundingSigned(FundingSigned),
    UpdateCommitment(UpdateCommitment),
    RevokeAndAck(RevokeAndAck),
    AddHtlc(AddHtlc),
    FulfillHtlc(FulfillHtlc),
    FailHtlc(FailHtlc),
    CloseChannel(CloseChannel),
    CloseAccepted(CloseAccepted),
    Error(ErrorMessage),
}

/// Propose opening a new channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenChannel {
    /// Temporary channel ID (random, replaced after funding).
    pub temp_channel_id: [u8; 32],
    /// Proposed capacity.
    pub capacity: Amount,
    /// Opener's initial balance (push amount goes to remote).
    pub push_amount: Amount,
    /// Opener's pubkey hash.
    pub funder_pubkey_hash: [u8; 32],
    /// Proposed dispute window in blocks.
    pub dispute_window: BlockHeight,
    /// Opener's revocation hash for commitment #0.
    pub first_revocation_hash: [u8; 32],
}

/// Accept a channel open proposal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcceptChannel {
    /// Echo back the temp channel ID.
    pub temp_channel_id: [u8; 32],
    /// Acceptor's pubkey hash.
    pub acceptor_pubkey_hash: [u8; 32],
    /// Accepted dispute window (may differ from proposed).
    pub dispute_window: BlockHeight,
    /// Acceptor's revocation hash for commitment #0.
    pub first_revocation_hash: [u8; 32],
}

/// Notify that the funding transaction has been created.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FundingCreated {
    /// Temp channel ID.
    pub temp_channel_id: [u8; 32],
    /// Funding tx hash.
    pub funding_tx_hash: [u8; 32],
    /// Funding output index.
    pub funding_output_index: u32,
    /// Funder's signature on the initial commitment tx (hex-encoded).
    pub signature: String,
}

/// Acknowledge the funding transaction and provide signature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FundingSigned {
    /// Channel ID (derived from funding outpoint).
    pub channel_id: [u8; 32],
    /// Acceptor's signature on the initial commitment tx (hex-encoded).
    pub signature: String,
}

/// Propose a new commitment (off-chain payment).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateCommitment {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// New commitment number.
    pub commitment_number: u64,
    /// Updated local balance (sender's perspective).
    pub local_balance: Amount,
    /// Updated remote balance (sender's perspective).
    pub remote_balance: Amount,
    /// Sender's revocation hash for this new commitment.
    pub revocation_hash: [u8; 32],
    /// Sender's signature on the new commitment tx (hex-encoded).
    pub signature: String,
}

/// Revoke the previous commitment and acknowledge the new one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevokeAndAck {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// Revocation preimage for the previous commitment.
    pub revocation_preimage: [u8; 32],
    /// Sender's revocation hash for the next commitment.
    pub next_revocation_hash: [u8; 32],
}

/// Offer a new HTLC to the counterparty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddHtlc {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// HTLC ID (unique within channel).
    pub htlc_id: u64,
    /// Payment hash.
    pub payment_hash: [u8; 32],
    /// Amount locked in the HTLC.
    pub amount: Amount,
    /// Expiry height.
    pub expiry_height: BlockHeight,
}

/// Fulfill an HTLC by revealing the preimage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FulfillHtlc {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// HTLC ID being fulfilled.
    pub htlc_id: u64,
    /// Payment preimage.
    pub preimage: [u8; 32],
}

/// Fail an HTLC (reject the payment).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FailHtlc {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// HTLC ID being failed.
    pub htlc_id: u64,
    /// Reason for failure.
    pub reason: String,
}

/// Propose cooperative close.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloseChannel {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// Proposed final local balance.
    pub local_balance: Amount,
    /// Proposed final remote balance.
    pub remote_balance: Amount,
    /// Signature on the close tx (hex-encoded).
    pub signature: String,
}

/// Accept cooperative close.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloseAccepted {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// Signature on the close tx (hex-encoded).
    pub signature: String,
}

/// Protocol error message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorMessage {
    /// Channel ID (or temp channel ID).
    pub channel_id: [u8; 32],
    /// Error description.
    pub message: String,
}
