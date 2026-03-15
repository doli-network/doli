use crypto::{Hash, PublicKey, Signature};
use serde::{Deserialize, Serialize};

use crate::types::Amount;

/// Registration data for producer registration transactions
///
/// Registration uses a chained VDF system for anti-Sybil protection.
/// Each registration must reference the previous registration's hash,
/// creating a sequential chain that prevents parallel registration attacks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationData {
    /// Producer's public key
    pub public_key: PublicKey,
    /// Target epoch
    pub epoch: u32,
    /// VDF output
    pub vdf_output: Vec<u8>,
    /// VDF proof
    pub vdf_proof: Vec<u8>,
    /// Hash of the previous registration transaction (Hash::ZERO for first registration)
    ///
    /// This creates a chain that prevents parallel registration attacks.
    /// An attacker cannot register multiple nodes simultaneously because
    /// each registration must wait for the previous one to be confirmed.
    pub prev_registration_hash: Hash,
    /// Global sequence number for this registration
    ///
    /// Starts at 0 for the first registration, increments by 1 for each
    /// subsequent registration. Used to verify registration ordering.
    pub sequence_number: u64,
    /// Number of bonds being staked (each bond = 1 bond_unit).
    ///
    /// This is consensus-critical: all nodes must agree on bond_count for
    /// deterministic producer selection (WHITEPAPER Section 7).
    /// Stored on-chain to avoid re-deriving from bond_amount / local_bond_unit,
    /// which would break consensus if nodes have different bond_unit configs.
    pub bond_count: u32,
    /// BLS12-381 public key for aggregate attestation signatures (48 bytes).
    ///
    /// Required for epoch reward qualification. Verified at registration time
    /// via proof-of-possession to prevent rogue public key attacks.
    #[serde(default)]
    pub bls_pubkey: Vec<u8>,
    /// BLS proof-of-possession: signature over the BLS pubkey using the `PoP` DST (96 bytes).
    ///
    /// Proves the registrant possesses the BLS secret key, preventing
    /// rogue public key attacks on aggregate signature verification.
    #[serde(default)]
    pub bls_pop: Vec<u8>,
}

/// Exit data for producer exit transactions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitData {
    /// Producer's public key (to identify which producer is exiting)
    pub public_key: PublicKey,
}

/// Claim data for reward claim transactions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimData {
    /// Producer's public key (to identify which producer is claiming)
    pub public_key: PublicKey,
}

/// Claim bond data for bond claim transactions (after unbonding)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimBondData {
    /// Producer's public key (to identify which producer is claiming their bond)
    pub public_key: PublicKey,
}

/// Evidence of producer misbehavior for slashing
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlashingEvidence {
    /// Producer created two different blocks for the same slot
    /// This is the ONLY slashable offense - it's unambiguously intentional
    ///
    /// The evidence includes full block headers so validators can verify:
    /// 1. Both headers have the same producer
    /// 2. Both headers have the same slot
    /// 3. Both headers have different hashes
    /// 4. Both headers have valid VDFs (proving the producer actually created them)
    DoubleProduction {
        /// First block header (complete, for VDF verification)
        block_header_1: crate::BlockHeader,
        /// Second block header (complete, for VDF verification)
        block_header_2: crate::BlockHeader,
    },
    // Note: Invalid blocks are NOT slashable. The network simply rejects them.
    // This follows Bitcoin's philosophy: natural consequences (lost slot/reward)
    // are sufficient for honest mistakes. Slashing is reserved for fraud.
}

/// Slash data for slash producer transactions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashData {
    /// Producer being slashed
    pub producer_pubkey: PublicKey,
    /// Evidence of misbehavior
    pub evidence: SlashingEvidence,
    /// Signature from the reporter (to prevent spam)
    pub reporter_signature: Signature,
}

// ==================== Bond Stacking Transactions ====================
//
// Producers can stake multiple bonds (up to 100) to increase their
// selection weight. Each bond is 1,000 DOLI and has its own vesting timer.

/// Add bond data for increasing stake
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddBondData {
    /// Producer's public key
    pub producer_pubkey: PublicKey,
    /// Number of bonds to add (each bond = 1,000 DOLI)
    pub bond_count: u32,
}

impl AddBondData {
    /// Create new add bond data
    pub fn new(producer_pubkey: PublicKey, bond_count: u32) -> Self {
        Self {
            producer_pubkey,
            bond_count,
        }
    }

    /// Calculate total amount required for a specific network
    ///
    /// Uses NetworkParams to get the correct bond_unit for the network.
    pub fn total_amount_for_network(&self, network: crate::Network) -> Amount {
        let params = crate::network_params::NetworkParams::load(network);
        self.bond_count as Amount * params.bond_unit
    }

    /// Calculate total amount required (mainnet default)
    ///
    /// **Deprecated**: Use `total_amount_for_network(network)` instead for
    /// network-aware calculations.
    #[deprecated(note = "Use total_amount_for_network(network) for network-aware bond calculation")]
    pub fn total_amount(&self) -> Amount {
        // Fallback to mainnet bond_unit for backward compatibility
        let params = crate::network_params::NetworkParams::load(crate::Network::Mainnet);
        self.bond_count as Amount * params.bond_unit
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.producer_pubkey.as_bytes());
        bytes.extend_from_slice(&self.bond_count.to_le_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 36 {
            return None;
        }
        let pubkey_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let bond_count = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
        Some(Self {
            producer_pubkey: PublicKey::from_bytes(pubkey_bytes),
            bond_count,
        })
    }
}

// ==================== Bond Delegation (Tier 3 → Tier 1/2) ====================

/// Delegate bond weight to a Tier 1/2 validator.
///
/// Stored in Transaction.extra_data. The delegator's bond weight is added
/// to the delegate's effective_weight for producer selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegateBondData {
    /// Public key of the delegator (the producer delegating weight)
    pub delegator: PublicKey,
    /// Public key of the delegate (Tier 1/2 validator receiving weight)
    pub delegate: PublicKey,
    /// Number of bonds to delegate
    pub bond_count: u32,
}

impl DelegateBondData {
    pub fn new(delegator: PublicKey, delegate: PublicKey, bond_count: u32) -> Self {
        Self {
            delegator,
            delegate,
            bond_count,
        }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.delegator.as_bytes());
        bytes.extend_from_slice(self.delegate.as_bytes());
        bytes.extend_from_slice(&self.bond_count.to_le_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 68 {
            return None;
        }
        let delegator_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let delegate_bytes: [u8; 32] = bytes[32..64].try_into().ok()?;
        let bond_count = u32::from_le_bytes(bytes[64..68].try_into().ok()?);
        Some(Self {
            delegator: PublicKey::from_bytes(delegator_bytes),
            delegate: PublicKey::from_bytes(delegate_bytes),
            bond_count,
        })
    }
}

/// Revoke a previously delegated bond.
///
/// DELEGATION_UNBONDING_SLOTS delay applies before weight is removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeDelegationData {
    /// Public key of the delegator revoking
    pub delegator: PublicKey,
    /// Public key of the delegate to revoke from
    pub delegate: PublicKey,
}

impl RevokeDelegationData {
    pub fn new(delegator: PublicKey, delegate: PublicKey) -> Self {
        Self {
            delegator,
            delegate,
        }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.delegator.as_bytes());
        bytes.extend_from_slice(self.delegate.as_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 64 {
            return None;
        }
        let delegator_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let delegate_bytes: [u8; 32] = bytes[32..64].try_into().ok()?;
        Some(Self {
            delegator: PublicKey::from_bytes(delegator_bytes),
            delegate: PublicKey::from_bytes(delegate_bytes),
        })
    }
}

/// Withdrawal request data for starting bond withdrawal
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WithdrawalRequestData {
    /// Producer's public key
    pub producer_pubkey: PublicKey,
    /// Number of bonds to withdraw
    pub bond_count: u32,
    /// Destination address for the withdrawal
    pub destination: Hash,
}

impl WithdrawalRequestData {
    /// Create new withdrawal request data
    pub fn new(producer_pubkey: PublicKey, bond_count: u32, destination: Hash) -> Self {
        Self {
            producer_pubkey,
            bond_count,
            destination,
        }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.producer_pubkey.as_bytes());
        bytes.extend_from_slice(&self.bond_count.to_le_bytes());
        bytes.extend_from_slice(self.destination.as_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 68 {
            return None;
        }
        let pubkey_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let bond_count = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
        let dest_bytes: [u8; 32] = bytes[36..68].try_into().ok()?;
        Some(Self {
            producer_pubkey: PublicKey::from_bytes(pubkey_bytes),
            bond_count,
            destination: Hash::from_bytes(dest_bytes),
        })
    }
}

// ==================== Epoch Reward Distribution ====================
//
// For fair reward distribution, rewards accumulate in a pool during an epoch
// and are distributed equally to all producers at epoch boundaries.

/// Epoch reward data for fair distribution transactions
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochRewardData {
    /// The epoch number this reward is for
    pub epoch: u64,
    /// The recipient producer's public key
    pub recipient: PublicKey,
}

impl EpochRewardData {
    /// Create new epoch reward data
    pub fn new(epoch: u64, recipient: PublicKey) -> Self {
        Self { epoch, recipient }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.epoch.to_le_bytes().to_vec();
        bytes.extend_from_slice(self.recipient.as_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 40 {
            // 8 bytes for epoch + 32 bytes for public key
            return None;
        }
        let epoch = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let pubkey_bytes: [u8; 32] = bytes[8..40].try_into().ok()?;
        Some(Self {
            epoch,
            recipient: PublicKey::from_bytes(pubkey_bytes),
        })
    }
}
