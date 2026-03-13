//! Watchtower client/server (Phase 3).
//!
//! A watchtower monitors the chain on behalf of offline channel participants
//! and broadcasts penalty transactions when revoked commitments are detected.
//!
//! Design:
//! - Client sends encrypted penalty blobs to the watchtower
//! - Each blob is keyed by the revoked commitment's tx hash prefix
//! - When the watchtower sees a matching tx on-chain, it decrypts and broadcasts the penalty
//! - The watchtower never learns channel balances until a breach occurs

use serde::{Deserialize, Serialize};

/// An encrypted penalty blob stored by the watchtower.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PenaltyBlob {
    /// Hint: first 16 bytes of the revoked commitment tx hash.
    /// The watchtower matches this against on-chain transactions.
    pub tx_hint: [u8; 16],
    /// Encrypted penalty transaction. Decryption key is derived from
    /// the full revoked tx hash (which the watchtower only learns when
    /// the breach appears on-chain).
    pub encrypted_data: Vec<u8>,
}

/// Watchtower registration for a channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchtowerSession {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// Watchtower endpoint URL.
    pub endpoint: String,
    /// Session token.
    pub session_token: String,
    /// Number of penalty blobs uploaded.
    pub blobs_uploaded: u64,
}

impl WatchtowerSession {
    pub fn new(channel_id: [u8; 32], endpoint: &str, session_token: &str) -> Self {
        Self {
            channel_id,
            endpoint: endpoint.to_string(),
            session_token: session_token.to_string(),
            blobs_uploaded: 0,
        }
    }
}
