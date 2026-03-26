//! Protocol activation data for on-chain consensus upgrades.

use serde::{Deserialize, Serialize};

use crate::maintainer::MaintainerSignature;

/// Data for protocol activation transactions (on-chain consensus upgrade)
///
/// When maintainers want to activate new consensus rules, they create a
/// ProtocolActivation transaction with 3/5 multisig. The activation is
/// scheduled for a future epoch, giving all nodes time to process it.
/// At the target epoch boundary, ALL nodes switch simultaneously.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolActivationData {
    /// Protocol version to activate (must be > current active version)
    pub protocol_version: u32,
    /// Epoch at which activation occurs (must be in the future)
    pub activation_epoch: u64,
    /// Human-readable description of consensus changes
    pub description: String,
    /// Signatures from current maintainers authorizing activation
    pub signatures: Vec<MaintainerSignature>,
}

impl ProtocolActivationData {
    /// Create new protocol activation data
    pub fn new(
        protocol_version: u32,
        activation_epoch: u64,
        description: String,
        signatures: Vec<MaintainerSignature>,
    ) -> Self {
        Self {
            protocol_version,
            activation_epoch,
            description,
            signatures,
        }
    }

    /// Get the message bytes that should be signed for this activation
    ///
    /// Format: "activate:{version}:{epoch}"
    pub fn signing_message(&self) -> Vec<u8> {
        format!(
            "activate:{}:{}",
            self.protocol_version, self.activation_epoch
        )
        .into_bytes()
    }

    /// Serialize to bytes for storage in transaction extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }
}
