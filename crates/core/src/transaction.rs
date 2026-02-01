//! Transaction types and operations

use crypto::{Hash, PublicKey, Signature};
use serde::{Deserialize, Serialize};

use crate::types::{Amount, BlockHeight};

/// Transaction type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum TxType {
    /// Regular transfer transaction
    Transfer = 0,
    /// Producer registration transaction
    Registration = 1,
    /// Producer exit transaction (starts unbonding period)
    Exit = 2,
    /// Claim accumulated rewards
    ClaimReward = 3,
    /// Claim bond after unbonding period completes
    ClaimBond = 4,
    /// Slash a misbehaving producer (with evidence)
    SlashProducer = 5,
    /// Coinbase transaction (block reward to producer)
    Coinbase = 6,
    /// Add bonds to increase stake (bond stacking)
    AddBond = 7,
    /// Request withdrawal of bonds (starts 7-day delay)
    RequestWithdrawal = 8,
    /// Claim withdrawal after 7-day delay
    ClaimWithdrawal = 9,
    /// Epoch reward transaction (fair share distribution at epoch boundary)
    /// DEPRECATED: Use ClaimEpochReward instead
    EpochReward = 10,
    /// Claim weighted presence rewards for a completed epoch
    ClaimEpochReward = 11,
}

impl TxType {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Transfer),
            1 => Some(Self::Registration),
            2 => Some(Self::Exit),
            3 => Some(Self::ClaimReward),
            4 => Some(Self::ClaimBond),
            5 => Some(Self::SlashProducer),
            6 => Some(Self::Coinbase),
            7 => Some(Self::AddBond),
            8 => Some(Self::RequestWithdrawal),
            9 => Some(Self::ClaimWithdrawal),
            10 => Some(Self::EpochReward),
            11 => Some(Self::ClaimEpochReward),
            _ => None,
        }
    }
}

/// Output type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum OutputType {
    /// Normal spendable output
    Normal = 0,
    /// Bond output (time-locked)
    Bond = 1,
}

impl OutputType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Normal),
            1 => Some(Self::Bond),
            _ => None,
        }
    }
}

/// Transaction input (reference to a previous output)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// Hash of the transaction containing the output
    pub prev_tx_hash: Hash,
    /// Index of the output in that transaction
    pub output_index: u32,
    /// Signature proving ownership
    pub signature: Signature,
}

impl Input {
    /// Create a new input
    pub fn new(prev_tx_hash: Hash, output_index: u32) -> Self {
        Self {
            prev_tx_hash,
            output_index,
            signature: Signature::default(),
        }
    }

    /// Create an outpoint identifier
    pub fn outpoint(&self) -> (Hash, u32) {
        (self.prev_tx_hash, self.output_index)
    }

    /// Serialize for signing
    pub fn serialize_for_signing(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.prev_tx_hash.as_bytes());
        bytes.extend_from_slice(&self.output_index.to_le_bytes());
        bytes
    }
}

/// Transaction output
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    /// Type of output
    pub output_type: OutputType,
    /// Amount in base units
    pub amount: Amount,
    /// Hash of the recipient's public key
    pub pubkey_hash: Hash,
    /// Lock until height (0 for normal, >0 for bonds)
    pub lock_until: BlockHeight,
}

impl Output {
    /// Create a normal output
    pub fn normal(amount: Amount, pubkey_hash: Hash) -> Self {
        Self {
            output_type: OutputType::Normal,
            amount,
            pubkey_hash,
            lock_until: 0,
        }
    }

    /// Create a bond output
    pub fn bond(amount: Amount, pubkey_hash: Hash, lock_until: BlockHeight) -> Self {
        Self {
            output_type: OutputType::Bond,
            amount,
            pubkey_hash,
            lock_until,
        }
    }

    /// Check if the output is spendable at a given height
    pub fn is_spendable_at(&self, height: BlockHeight) -> bool {
        height >= self.lock_until
    }

    /// Serialize for hashing
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.push(self.output_type as u8);
        bytes.extend_from_slice(&self.amount.to_le_bytes());
        bytes.extend_from_slice(self.pubkey_hash.as_bytes());
        bytes.extend_from_slice(&self.lock_until.to_le_bytes());
        bytes
    }
}

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

    /// Calculate total amount required
    pub fn total_amount(&self) -> Amount {
        use crate::consensus::BOND_UNIT;
        self.bond_count as Amount * BOND_UNIT
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

/// Claim withdrawal data for completing bond withdrawal after delay
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimWithdrawalData {
    /// Producer's public key
    pub producer_pubkey: PublicKey,
    /// Index of the pending withdrawal to claim (if multiple exist)
    pub withdrawal_index: u32,
}

impl ClaimWithdrawalData {
    /// Create new claim withdrawal data
    pub fn new(producer_pubkey: PublicKey, withdrawal_index: u32) -> Self {
        Self {
            producer_pubkey,
            withdrawal_index,
        }
    }

    /// Serialize to bytes for storage in extra_data
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.producer_pubkey.as_bytes());
        bytes.extend_from_slice(&self.withdrawal_index.to_le_bytes());
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 36 {
            return None;
        }
        let pubkey_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
        let withdrawal_index = u32::from_le_bytes(bytes[32..36].try_into().ok()?);
        Some(Self {
            producer_pubkey: PublicKey::from_bytes(pubkey_bytes),
            withdrawal_index,
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

// ==================== Weighted Presence Reward Claims ====================
//
// ClaimEpochReward is the new on-demand claim system that replaces EpochReward.
// Producers prove presence via VDF heartbeats each slot and claim rewards at
// their convenience after an epoch completes.

/// Data for ClaimEpochReward transaction
///
/// This transaction allows a producer to claim their weighted presence rewards
/// for a completed epoch. The reward amount is calculated based on presence
/// (VDF heartbeats with witness signatures) and bond weight during the epoch.
///
/// Layout in extra_data:
/// - bytes 0-7:   epoch (u64 LE)
/// - bytes 8-39:  producer_pubkey (32 bytes)
/// - bytes 40-71: recipient_hash (32 bytes)
/// - bytes 72-135: signature (64 bytes) [appended by new_claim_epoch_reward]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimEpochRewardData {
    /// Epoch number being claimed
    pub epoch: u64,
    /// Claiming producer's public key
    pub producer_pubkey: PublicKey,
    /// Recipient address (can differ from producer)
    pub recipient_hash: Hash,
}

impl ClaimEpochRewardData {
    /// Create new claim epoch reward data
    pub fn new(epoch: u64, producer_pubkey: PublicKey, recipient_hash: Hash) -> Self {
        Self {
            epoch,
            producer_pubkey,
            recipient_hash,
        }
    }

    /// Serialize to bytes for storage in extra_data
    ///
    /// Returns 72 bytes: epoch (8) + producer_pubkey (32) + recipient_hash (32)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(72);
        bytes.extend_from_slice(&self.epoch.to_le_bytes()); // 8 bytes
        bytes.extend_from_slice(self.producer_pubkey.as_bytes()); // 32 bytes
        bytes.extend_from_slice(self.recipient_hash.as_bytes()); // 32 bytes
        bytes
    }

    /// Deserialize from bytes
    ///
    /// Expects at least 72 bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 72 {
            return None;
        }
        let epoch = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let pubkey_bytes: [u8; 32] = bytes[8..40].try_into().ok()?;
        let hash_bytes: [u8; 32] = bytes[40..72].try_into().ok()?;
        Some(Self {
            epoch,
            producer_pubkey: PublicKey::from_bytes(pubkey_bytes),
            recipient_hash: Hash::from_bytes(hash_bytes),
        })
    }

    /// Compute the message to sign for this claim
    ///
    /// Includes epoch, producer, recipient, and amount to prevent any manipulation.
    pub fn signing_message(&self, amount: Amount) -> Hash {
        crypto::hash::hash_concat(&[
            b"DOLI_CLAIM_SIGN_V1",
            &self.epoch.to_le_bytes(),
            self.producer_pubkey.as_bytes(),
            self.recipient_hash.as_bytes(),
            &amount.to_le_bytes(),
        ])
    }
}

// ==================== Proof of Time ====================
//
// In Proof of Time, there are NO multi-signature attestations.
// Each block has exactly one producer who receives 100% of the block reward.
//
// Time is proven by producing blocks with valid VDF when selected:
// - One producer per slot (10 seconds)
// - Selection based on bond count (deterministic round-robin)
// - VDF provides anti-grinding protection (~7s computation)
// - Producer receives full block reward via coinbase transaction
//
// This eliminates:
// - Attestation structs and signatures
// - Multi-signature overhead
// - Delegation incentives (giving keys away means losing all rewards)
//
// With EpochPool reward mode, rewards accumulate and are distributed
// fairly at epoch boundaries via EpochReward transactions.

/// A transaction
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    /// Protocol version
    pub version: u32,
    /// Transaction type
    pub tx_type: TxType,
    /// Inputs (references to previous outputs)
    pub inputs: Vec<Input>,
    /// Outputs (new coins being created)
    pub outputs: Vec<Output>,
    /// Extra data (type-specific)
    pub extra_data: Vec<u8>,
}

impl Transaction {
    /// Create a new transfer transaction
    pub fn new_transfer(inputs: Vec<Input>, outputs: Vec<Output>) -> Self {
        Self {
            version: 1,
            tx_type: TxType::Transfer,
            inputs,
            outputs,
            extra_data: Vec::new(),
        }
    }

    /// Create a coinbase transaction
    pub fn new_coinbase(amount: Amount, pubkey_hash: Hash, height: BlockHeight) -> Self {
        Self {
            version: 1,
            tx_type: TxType::Transfer,
            inputs: Vec::new(),
            outputs: vec![Output::normal(amount, pubkey_hash)],
            extra_data: height.to_le_bytes().to_vec(),
        }
    }

    /// Create an epoch reward coinbase with multiple outputs.
    ///
    /// This is used at epoch boundaries to automatically distribute rewards
    /// to all producers who were present during the completed epoch.
    /// Each output pays the calculated reward to a producer's address.
    ///
    /// # Arguments
    /// * `outputs` - Vector of (amount, pubkey_hash) pairs for each producer
    /// * `height` - Block height (used as extra_data for uniqueness)
    /// * `epoch` - The completed epoch number (stored in extra_data)
    pub fn new_epoch_reward_coinbase(
        outputs: Vec<(Amount, Hash)>,
        height: BlockHeight,
        epoch: u64,
    ) -> Self {
        let tx_outputs: Vec<Output> = outputs
            .into_iter()
            .map(|(amount, pubkey_hash)| Output::normal(amount, pubkey_hash))
            .collect();

        // Store both height and epoch in extra_data for auditability
        let mut extra_data = height.to_le_bytes().to_vec();
        extra_data.extend_from_slice(&epoch.to_le_bytes());

        Self {
            version: 1,
            tx_type: TxType::EpochReward, // Use EpochReward type for automatic distribution
            inputs: Vec::new(),
            outputs: tx_outputs,
            extra_data,
        }
    }

    /// Check if this is a coinbase transaction
    ///
    /// A coinbase is a Transfer transaction with no inputs and one output.
    /// Note: ClaimReward transactions also have no inputs and one output,
    /// but they have a different tx_type.
    pub fn is_coinbase(&self) -> bool {
        self.tx_type == TxType::Transfer && self.inputs.is_empty() && self.outputs.len() == 1
    }

    /// Check if this is an epoch reward coinbase (automatic distribution at epoch boundaries)
    ///
    /// Epoch reward coinbase transactions have:
    /// - TxType::EpochReward
    /// - No inputs (minted coins)
    /// - One or more outputs (rewards distributed to present producers)
    pub fn is_epoch_reward_coinbase(&self) -> bool {
        self.tx_type == TxType::EpochReward && self.inputs.is_empty() && !self.outputs.is_empty()
    }

    /// Check if this is any type of reward-minting transaction (coinbase or epoch reward)
    ///
    /// Returns true for both regular single-output coinbase and epoch reward coinbase.
    /// Use this when validating blocks to check for reward transactions.
    pub fn is_reward_minting(&self) -> bool {
        self.is_coinbase() || self.is_epoch_reward_coinbase()
    }

    /// Check if this is an exit transaction
    pub fn is_exit(&self) -> bool {
        self.tx_type == TxType::Exit
    }

    /// Check if this is a registration transaction
    pub fn is_registration(&self) -> bool {
        self.tx_type == TxType::Registration
    }

    /// Check if this is a claim reward transaction
    pub fn is_claim_reward(&self) -> bool {
        self.tx_type == TxType::ClaimReward
    }

    /// Check if this is an epoch reward transaction
    pub fn is_epoch_reward(&self) -> bool {
        self.tx_type == TxType::EpochReward
    }

    /// Check if this is a claim epoch reward transaction (new weighted presence system)
    pub fn is_claim_epoch_reward(&self) -> bool {
        self.tx_type == TxType::ClaimEpochReward
    }

    /// Create a claim epoch reward transaction
    ///
    /// This is the new on-demand claim system for weighted presence rewards.
    /// The producer claims their share of rewards for a completed epoch based
    /// on their presence (VDF heartbeats) and bond weight.
    ///
    /// # Arguments
    /// * `epoch` - The epoch number being claimed
    /// * `producer_pubkey` - The claiming producer's public key
    /// * `amount` - The calculated reward amount (validated by network)
    /// * `recipient_hash` - Where to send the reward (can differ from producer)
    /// * `signature` - Producer's signature over the claim data + amount
    pub fn new_claim_epoch_reward(
        epoch: u64,
        producer_pubkey: PublicKey,
        amount: Amount,
        recipient_hash: Hash,
        signature: Signature,
    ) -> Self {
        let data = ClaimEpochRewardData::new(epoch, producer_pubkey, recipient_hash);
        let mut extra_data = data.to_bytes(); // 72 bytes
        extra_data.extend_from_slice(signature.as_bytes()); // 64 bytes = 136 total

        Self {
            version: 1,
            tx_type: TxType::ClaimEpochReward,
            inputs: Vec::new(), // Minted - no inputs
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data,
        }
    }

    /// Parse claim epoch reward data from extra_data
    pub fn claim_epoch_reward_data(&self) -> Option<ClaimEpochRewardData> {
        if self.tx_type != TxType::ClaimEpochReward {
            return None;
        }
        ClaimEpochRewardData::from_bytes(&self.extra_data)
    }

    /// Get signature from claim epoch reward transaction
    ///
    /// Returns None if not a ClaimEpochReward tx or if extra_data is too short.
    pub fn claim_epoch_reward_signature(&self) -> Option<Signature> {
        if self.tx_type != TxType::ClaimEpochReward {
            return None;
        }
        if self.extra_data.len() < 136 {
            return None;
        }
        let sig_bytes: [u8; 64] = self.extra_data[72..136].try_into().ok()?;
        Some(Signature::from_bytes(sig_bytes))
    }

    /// Create an epoch reward transaction for fair distribution
    ///
    /// This transaction type is used at epoch boundaries to distribute
    /// accumulated rewards fairly among all producers who participated
    /// in the epoch.
    pub fn new_epoch_reward(
        epoch: u64,
        recipient_pubkey: PublicKey,
        amount: Amount,
        recipient_hash: Hash,
    ) -> Self {
        let data = EpochRewardData::new(epoch, recipient_pubkey);
        Self {
            version: 1,
            tx_type: TxType::EpochReward,
            inputs: Vec::new(), // Minted, no inputs
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data: data.to_bytes(),
        }
    }

    /// Parse epoch reward data from extra_data
    pub fn epoch_reward_data(&self) -> Option<EpochRewardData> {
        if self.tx_type != TxType::EpochReward {
            return None;
        }
        EpochRewardData::from_bytes(&self.extra_data)
    }

    /// Create a registration transaction
    ///
    /// This creates a transaction to register as a block producer.
    /// Inputs must cover the bond amount. The bond is locked for the specified duration.
    ///
    /// # Arguments
    /// - `inputs`: UTXOs to spend for the bond
    /// - `public_key`: Producer's public key
    /// - `bond_amount`: Total amount to lock as bond
    /// - `lock_until`: Block height until which the bond is locked (must be >= current_height + blocks_per_era)
    ///
    /// Note: For simplicity, this creates a basic registration. For full
    /// registration with VDF proofs, use the node's registration flow.
    pub fn new_registration(
        inputs: Vec<Input>,
        public_key: PublicKey,
        bond_amount: Amount,
        lock_until: BlockHeight,
    ) -> Self {
        // Create registration data without VDF (for CLI use)
        // Full VDF registration happens through node's registration flow
        let reg_data = RegistrationData {
            public_key,
            epoch: 0,
            vdf_output: Vec::new(),
            vdf_proof: Vec::new(),
            prev_registration_hash: Hash::ZERO,
            sequence_number: 0,
        };
        let extra_data = bincode::serialize(&reg_data).unwrap_or_default();

        // Create bond output (locked to producer's pubkey)
        let pubkey_hash = crypto::hash::hash(public_key.as_bytes());
        let bond_output = Output::bond(bond_amount, pubkey_hash, lock_until);

        Self {
            version: 1,
            tx_type: TxType::Registration,
            inputs,
            outputs: vec![bond_output],
            extra_data,
        }
    }

    /// Create an exit transaction
    pub fn new_exit(public_key: PublicKey) -> Self {
        let exit_data = ExitData { public_key };
        let extra_data = bincode::serialize(&exit_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::Exit,
            inputs: Vec::new(),  // No inputs needed for exit
            outputs: Vec::new(), // No outputs - bond released after cooldown
            extra_data,
        }
    }

    /// Parse exit data from extra_data
    pub fn exit_data(&self) -> Option<ExitData> {
        if self.tx_type != TxType::Exit {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a claim reward transaction
    ///
    /// The producer claims their accumulated rewards as a single UTXO.
    /// The output amount is determined by the node based on pending_rewards.
    pub fn new_claim_reward(public_key: PublicKey, amount: Amount, recipient_hash: Hash) -> Self {
        let claim_data = ClaimData { public_key };
        let extra_data = bincode::serialize(&claim_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::ClaimReward,
            inputs: Vec::new(), // No inputs - rewards come from pending balance
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data,
        }
    }

    /// Parse claim data from extra_data
    pub fn claim_data(&self) -> Option<ClaimData> {
        if self.tx_type != TxType::ClaimReward {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a claim bond transaction
    ///
    /// After the unbonding period completes, the producer can claim their bond.
    /// The amount depends on whether it was a normal exit (100% returned) or
    /// early exit (proportional penalty applied).
    pub fn new_claim_bond(public_key: PublicKey, amount: Amount, recipient_hash: Hash) -> Self {
        let claim_bond_data = ClaimBondData { public_key };
        let extra_data = bincode::serialize(&claim_bond_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::ClaimBond,
            inputs: Vec::new(), // No inputs - bond comes from protocol
            outputs: vec![Output::normal(amount, recipient_hash)],
            extra_data,
        }
    }

    /// Check if this is a claim bond transaction
    pub fn is_claim_bond(&self) -> bool {
        self.tx_type == TxType::ClaimBond
    }

    /// Parse claim bond data from extra_data
    pub fn claim_bond_data(&self) -> Option<ClaimBondData> {
        if self.tx_type != TxType::ClaimBond {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Create a slash producer transaction
    ///
    /// Anyone can submit slashing evidence against a misbehaving producer.
    /// If valid, the producer's bond is burned (100%).
    pub fn new_slash_producer(slash_data: SlashData) -> Self {
        let extra_data = bincode::serialize(&slash_data).unwrap_or_default();

        Self {
            version: 1,
            tx_type: TxType::SlashProducer,
            inputs: Vec::new(),  // No inputs
            outputs: Vec::new(), // No outputs - bond is burned
            extra_data,
        }
    }

    /// Check if this is a slash producer transaction
    pub fn is_slash_producer(&self) -> bool {
        self.tx_type == TxType::SlashProducer
    }

    /// Parse slash data from extra_data
    pub fn slash_data(&self) -> Option<SlashData> {
        if self.tx_type != TxType::SlashProducer {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    // ==================== Bond Stacking Transactions ====================

    /// Create an add bond transaction.
    ///
    /// Inputs: bond amount (must be multiple of BOND_UNIT = 1,000 DOLI)
    /// Outputs: none (funds go into bond state)
    ///
    /// The producer must already be registered.
    pub fn new_add_bond(inputs: Vec<Input>, producer_pubkey: PublicKey, bond_count: u32) -> Self {
        let bond_data = AddBondData::new(producer_pubkey, bond_count);
        let extra_data = bond_data.to_bytes();

        Self {
            version: 1,
            tx_type: TxType::AddBond,
            inputs,
            outputs: Vec::new(), // No outputs - funds become bonds
            extra_data,
        }
    }

    /// Check if this is an add bond transaction
    pub fn is_add_bond(&self) -> bool {
        self.tx_type == TxType::AddBond
    }

    /// Parse add bond data from extra_data
    pub fn add_bond_data(&self) -> Option<AddBondData> {
        if self.tx_type != TxType::AddBond {
            return None;
        }
        AddBondData::from_bytes(&self.extra_data)
    }

    /// Create a withdrawal request transaction.
    ///
    /// This starts the 7-day withdrawal delay. Penalty is calculated at
    /// request time based on bond age (FIFO - oldest bonds first).
    ///
    /// The penalty amount is burned (100% burn, no treasury).
    pub fn new_request_withdrawal(
        producer_pubkey: PublicKey,
        bond_count: u32,
        destination: Hash,
    ) -> Self {
        let withdrawal_data = WithdrawalRequestData::new(producer_pubkey, bond_count, destination);
        let extra_data = withdrawal_data.to_bytes();

        Self {
            version: 1,
            tx_type: TxType::RequestWithdrawal,
            inputs: Vec::new(),  // No inputs - state-only operation
            outputs: Vec::new(), // No outputs - funds locked until claim
            extra_data,
        }
    }

    /// Check if this is a withdrawal request transaction
    pub fn is_request_withdrawal(&self) -> bool {
        self.tx_type == TxType::RequestWithdrawal
    }

    /// Parse withdrawal request data from extra_data
    pub fn withdrawal_request_data(&self) -> Option<WithdrawalRequestData> {
        if self.tx_type != TxType::RequestWithdrawal {
            return None;
        }
        WithdrawalRequestData::from_bytes(&self.extra_data)
    }

    /// Create a claim withdrawal transaction.
    ///
    /// This completes a pending withdrawal after the 7-day delay.
    /// The output is the net amount (after penalty was burned at request time).
    pub fn new_claim_withdrawal(
        producer_pubkey: PublicKey,
        withdrawal_index: u32,
        net_amount: Amount,
        destination: Hash,
    ) -> Self {
        let claim_data = ClaimWithdrawalData::new(producer_pubkey, withdrawal_index);
        let extra_data = claim_data.to_bytes();

        Self {
            version: 1,
            tx_type: TxType::ClaimWithdrawal,
            inputs: Vec::new(), // No inputs - funds come from pending withdrawal
            outputs: vec![Output::normal(net_amount, destination)],
            extra_data,
        }
    }

    /// Check if this is a claim withdrawal transaction
    pub fn is_claim_withdrawal(&self) -> bool {
        self.tx_type == TxType::ClaimWithdrawal
    }

    /// Parse claim withdrawal data from extra_data
    pub fn claim_withdrawal_data(&self) -> Option<ClaimWithdrawalData> {
        if self.tx_type != TxType::ClaimWithdrawal {
            return None;
        }
        ClaimWithdrawalData::from_bytes(&self.extra_data)
    }

    /// Parse registration data from extra_data
    pub fn registration_data(&self) -> Option<RegistrationData> {
        if self.tx_type != TxType::Registration {
            return None;
        }
        bincode::deserialize(&self.extra_data).ok()
    }

    /// Compute the transaction hash
    pub fn hash(&self) -> Hash {
        use crypto::Hasher;

        let mut hasher = Hasher::new();
        hasher.update(&self.version.to_le_bytes());
        hasher.update(&(self.tx_type as u32).to_le_bytes());

        // Hash inputs (without signatures for tx hash)
        hasher.update(&(self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            hasher.update(input.prev_tx_hash.as_bytes());
            hasher.update(&input.output_index.to_le_bytes());
            // Signature is NOT included in tx hash
        }

        // Hash outputs
        hasher.update(&(self.outputs.len() as u32).to_le_bytes());
        for output in &self.outputs {
            hasher.update(&output.serialize());
        }

        // Hash extra data
        hasher.update(&(self.extra_data.len() as u32).to_le_bytes());
        hasher.update(&self.extra_data);

        hasher.finalize()
    }

    /// Get the message to sign for a given input
    pub fn signing_message(&self) -> Hash {
        self.hash()
    }

    /// Calculate total input amount (requires UTXO lookup - returns 0 here)
    pub fn total_output(&self) -> Amount {
        self.outputs.iter().map(|o| o.amount).sum()
    }

    /// Serialize the transaction
    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap_or_default()
    }

    /// Deserialize a transaction
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        bincode::deserialize(bytes).ok()
    }

    /// Get the size in bytes
    pub fn size(&self) -> usize {
        self.serialize().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coinbase() {
        let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 0);

        assert!(tx.is_coinbase());
        assert_eq!(tx.inputs.len(), 0);
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, 500_000_000);
    }

    #[test]
    fn test_tx_hash_deterministic() {
        let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 100);

        let hash1 = tx.hash();
        let hash2 = tx.hash();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_output_spendability() {
        let normal = Output::normal(100, Hash::ZERO);
        assert!(normal.is_spendable_at(0));
        assert!(normal.is_spendable_at(100));

        let bond = Output::bond(100, Hash::ZERO, 1000);
        assert!(!bond.is_spendable_at(0));
        assert!(!bond.is_spendable_at(999));
        assert!(bond.is_spendable_at(1000));
        assert!(bond.is_spendable_at(1001));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 42);
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx, recovered);
    }

    #[test]
    fn test_tx_type_conversion() {
        assert_eq!(TxType::from_u32(0), Some(TxType::Transfer));
        assert_eq!(TxType::from_u32(1), Some(TxType::Registration));
        assert_eq!(TxType::from_u32(2), Some(TxType::Exit));
        assert_eq!(TxType::from_u32(3), Some(TxType::ClaimReward));
        assert_eq!(TxType::from_u32(4), Some(TxType::ClaimBond));
        assert_eq!(TxType::from_u32(5), Some(TxType::SlashProducer));
        assert_eq!(TxType::from_u32(6), Some(TxType::Coinbase));
        assert_eq!(TxType::from_u32(7), Some(TxType::AddBond));
        assert_eq!(TxType::from_u32(8), Some(TxType::RequestWithdrawal));
        assert_eq!(TxType::from_u32(9), Some(TxType::ClaimWithdrawal));
        assert_eq!(TxType::from_u32(10), Some(TxType::EpochReward));
        assert_eq!(TxType::from_u32(11), Some(TxType::ClaimEpochReward));
        assert_eq!(TxType::from_u32(12), None);
        assert_eq!(TxType::from_u32(u32::MAX), None);
    }

    #[test]
    fn test_output_type_conversion() {
        assert_eq!(OutputType::from_u8(0), Some(OutputType::Normal));
        assert_eq!(OutputType::from_u8(1), Some(OutputType::Bond));
        assert_eq!(OutputType::from_u8(2), None);
        assert_eq!(OutputType::from_u8(u8::MAX), None);
    }

    #[test]
    fn test_input_outpoint() {
        let hash = crypto::hash::hash(b"test");
        let input = Input::new(hash, 42);
        assert_eq!(input.outpoint(), (hash, 42));
    }

    #[test]
    fn test_transfer_not_coinbase() {
        let hash = crypto::hash::hash(b"prev");
        let tx = Transaction::new_transfer(
            vec![Input::new(hash, 0)],
            vec![Output::normal(100, Hash::ZERO)],
        );
        assert!(!tx.is_coinbase());
    }

    #[test]
    fn test_exit_transaction() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key();

        let tx = Transaction::new_exit(pubkey.clone());

        assert!(tx.is_exit());
        assert!(!tx.is_coinbase());
        assert!(!tx.is_registration());
        assert_eq!(tx.tx_type, TxType::Exit);
        assert!(tx.inputs.is_empty());
        assert!(tx.outputs.is_empty());

        // Verify exit data can be parsed
        let exit_data = tx.exit_data().unwrap();
        assert_eq!(exit_data.public_key, *pubkey);
    }

    #[test]
    fn test_exit_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key();

        let tx = Transaction::new_exit(pubkey.clone());
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        let recovered_data = recovered.exit_data().unwrap();
        assert_eq!(recovered_data.public_key, *pubkey);
    }

    #[test]
    fn test_claim_reward_transaction() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key();
        let recipient_hash = crypto::hash::hash(b"recipient");

        let tx = Transaction::new_claim_reward(pubkey.clone(), 500_000_000, recipient_hash);

        assert!(tx.is_claim_reward());
        assert!(!tx.is_coinbase());
        assert!(!tx.is_exit());
        assert!(!tx.is_registration());
        assert_eq!(tx.tx_type, TxType::ClaimReward);
        assert!(tx.inputs.is_empty());
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, 500_000_000);

        // Verify claim data can be parsed
        let claim_data = tx.claim_data().unwrap();
        assert_eq!(claim_data.public_key, *pubkey);
    }

    #[test]
    fn test_claim_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key();
        let recipient_hash = crypto::hash::hash(b"recipient");

        let tx = Transaction::new_claim_reward(pubkey.clone(), 1_000_000_000, recipient_hash);
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        let recovered_data = recovered.claim_data().unwrap();
        assert_eq!(recovered_data.public_key, *pubkey);
    }

    #[test]
    fn test_claim_bond_transaction() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key();
        let recipient_hash = crypto::hash::hash(b"recipient");

        let tx = Transaction::new_claim_bond(pubkey.clone(), 100_000_000_000, recipient_hash);

        assert!(tx.is_claim_bond());
        assert!(!tx.is_coinbase());
        assert!(!tx.is_claim_reward());
        assert!(!tx.is_exit());
        assert_eq!(tx.tx_type, TxType::ClaimBond);
        assert!(tx.inputs.is_empty());
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, 100_000_000_000);

        // Verify claim bond data can be parsed
        let claim_bond_data = tx.claim_bond_data().unwrap();
        assert_eq!(claim_bond_data.public_key, *pubkey);
    }

    #[test]
    fn test_claim_bond_serialization() {
        let keypair = crypto::KeyPair::generate();
        let pubkey = keypair.public_key();
        let recipient_hash = crypto::hash::hash(b"recipient");

        let tx = Transaction::new_claim_bond(pubkey.clone(), 50_000_000_000, recipient_hash);
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        let recovered_data = recovered.claim_bond_data().unwrap();
        assert_eq!(recovered_data.public_key, *pubkey);
    }

    #[test]
    fn test_slash_producer_transaction() {
        use crate::BlockHeader;
        use vdf::{VdfOutput, VdfProof};

        let producer_keypair = crypto::KeyPair::generate();

        // Create test block headers with same producer and slot but different content
        let header1 = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: crypto::hash::hash(b"block1"),
            timestamp: 0,
            slot: 12345,
            producer: producer_keypair.public_key().clone(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };
        let header2 = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: crypto::hash::hash(b"block2"),
            timestamp: 0,
            slot: 12345,
            producer: producer_keypair.public_key().clone(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };

        let evidence = SlashingEvidence::DoubleProduction {
            block_header_1: header1,
            block_header_2: header2,
        };

        let slash_data = SlashData {
            producer_pubkey: producer_keypair.public_key().clone(),
            evidence,
            reporter_signature: Signature::default(),
        };

        let tx = Transaction::new_slash_producer(slash_data.clone());

        assert!(tx.is_slash_producer());
        assert!(!tx.is_coinbase());
        assert!(!tx.is_claim_reward());
        assert!(!tx.is_exit());
        assert_eq!(tx.tx_type, TxType::SlashProducer);
        assert!(tx.inputs.is_empty());
        assert!(tx.outputs.is_empty()); // No outputs - bond is burned

        // Verify slash data can be parsed
        let parsed_data = tx.slash_data().unwrap();
        assert_eq!(parsed_data.producer_pubkey, slash_data.producer_pubkey);
    }

    #[test]
    fn test_slash_producer_serialization() {
        use crate::BlockHeader;
        use vdf::{VdfOutput, VdfProof};

        let producer_keypair = crypto::KeyPair::generate();

        // Create test block headers with same producer and slot but different content
        let header1 = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: crypto::hash::hash(b"block_a"),
            timestamp: 0,
            slot: 99999,
            producer: producer_keypair.public_key().clone(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };
        let header2 = BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: crypto::hash::hash(b"block_b"),
            timestamp: 0,
            slot: 99999,
            producer: producer_keypair.public_key().clone(),
            vdf_output: VdfOutput { value: vec![] },
            vdf_proof: VdfProof::empty(),
        };

        let evidence = SlashingEvidence::DoubleProduction {
            block_header_1: header1,
            block_header_2: header2,
        };

        let slash_data = SlashData {
            producer_pubkey: producer_keypair.public_key().clone(),
            evidence,
            reporter_signature: Signature::default(),
        };

        let tx = Transaction::new_slash_producer(slash_data);
        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        let recovered_data = recovered.slash_data().unwrap();

        // Check evidence type matches
        match recovered_data.evidence {
            SlashingEvidence::DoubleProduction {
                block_header_1,
                block_header_2,
            } => {
                assert_eq!(block_header_1.slot, 99999);
                assert_eq!(block_header_2.slot, 99999);
            }
        }
    }

    // ==================== EpochReward Transaction Tests ====================

    #[test]
    fn test_tx_type_epoch_reward_value() {
        assert_eq!(TxType::EpochReward as u32, 10);
    }

    #[test]
    fn test_epoch_reward_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let data = EpochRewardData::new(42, keypair.public_key().clone());

        let bytes = data.to_bytes();
        let parsed = EpochRewardData::from_bytes(&bytes).unwrap();

        assert_eq!(data.epoch, parsed.epoch);
        assert_eq!(data.recipient, parsed.recipient);
    }

    #[test]
    fn test_epoch_reward_data_from_bytes_short() {
        // Less than 40 bytes should return None
        let short_bytes = vec![0u8; 39];
        assert!(EpochRewardData::from_bytes(&short_bytes).is_none());
    }

    #[test]
    fn test_new_epoch_reward_transaction() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, keypair.public_key().as_bytes());

        let tx = Transaction::new_epoch_reward(
            5,                            // epoch
            keypair.public_key().clone(), // recipient
            1_000_000,                    // amount
            pubkey_hash,                  // recipient hash
        );

        assert!(tx.is_epoch_reward());
        assert!(!tx.is_coinbase());
        assert!(tx.inputs.is_empty());
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, 1_000_000);
        assert_eq!(tx.outputs[0].output_type, OutputType::Normal);

        let data = tx.epoch_reward_data().unwrap();
        assert_eq!(data.epoch, 5);
        assert_eq!(data.recipient, *keypair.public_key());
    }

    #[test]
    fn test_epoch_reward_is_not_coinbase() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = Hash::ZERO;
        let tx = Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1000, pubkey_hash);

        assert!(!tx.is_coinbase());
        assert!(tx.is_epoch_reward());
    }

    #[test]
    fn test_epoch_reward_hash_deterministic() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = Hash::ZERO;

        let tx1 = Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1000, pubkey_hash);
        let tx2 = Transaction::new_epoch_reward(1, keypair.public_key().clone(), 1000, pubkey_hash);

        assert_eq!(tx1.hash(), tx2.hash());
    }

    #[test]
    fn test_epoch_reward_serialization_roundtrip() {
        let keypair = crypto::KeyPair::generate();
        let pubkey_hash = crypto::hash::hash(b"recipient");

        let tx = Transaction::new_epoch_reward(
            100,                          // epoch
            keypair.public_key().clone(), // recipient
            50_000_000,                   // amount
            pubkey_hash,                  // recipient hash
        );

        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        assert_eq!(tx, recovered);

        let recovered_data = recovered.epoch_reward_data().unwrap();
        assert_eq!(recovered_data.epoch, 100);
        assert_eq!(recovered_data.recipient, *keypair.public_key());
    }

    #[test]
    fn test_epoch_reward_data_none_for_non_epoch_reward() {
        let tx = Transaction::new_coinbase(1000, Hash::ZERO, 0);
        assert!(tx.epoch_reward_data().is_none());
    }

    // ==================== ClaimEpochReward Transaction Tests ====================

    #[test]
    fn test_tx_type_claim_epoch_reward_value() {
        assert_eq!(TxType::ClaimEpochReward as u32, 11);
        assert_eq!(TxType::from_u32(11), Some(TxType::ClaimEpochReward));
    }

    #[test]
    fn test_claim_epoch_reward_data_serialization() {
        let keypair = crypto::KeyPair::generate();
        let recipient_hash = crypto::hash::hash(b"recipient");

        let data = ClaimEpochRewardData::new(42, keypair.public_key().clone(), recipient_hash);

        let bytes = data.to_bytes();
        assert_eq!(bytes.len(), 72); // 8 + 32 + 32

        let parsed = ClaimEpochRewardData::from_bytes(&bytes).unwrap();
        assert_eq!(data.epoch, parsed.epoch);
        assert_eq!(data.producer_pubkey, parsed.producer_pubkey);
        assert_eq!(data.recipient_hash, parsed.recipient_hash);
    }

    #[test]
    fn test_claim_epoch_reward_data_from_bytes_short() {
        // Less than 72 bytes should return None
        let short_bytes = vec![0u8; 71];
        assert!(ClaimEpochRewardData::from_bytes(&short_bytes).is_none());
    }

    #[test]
    fn test_claim_epoch_reward_data_signing_message() {
        let keypair = crypto::KeyPair::generate();
        let recipient_hash = crypto::hash::hash(b"recipient");

        let data = ClaimEpochRewardData::new(42, keypair.public_key().clone(), recipient_hash);

        let msg1 = data.signing_message(1000);
        let msg2 = data.signing_message(1000);
        assert_eq!(msg1, msg2); // Deterministic

        let msg3 = data.signing_message(2000);
        assert_ne!(msg1, msg3); // Different amount = different message
    }

    #[test]
    fn test_new_claim_epoch_reward_transaction() {
        let keypair = crypto::KeyPair::generate();
        let recipient_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, keypair.public_key().as_bytes());

        let data = ClaimEpochRewardData::new(5, keypair.public_key().clone(), recipient_hash);
        let amount = 47_500_000_000u64;
        let signing_msg = data.signing_message(amount);

        // Sign the message
        let signature = crypto::signature::sign_hash(&signing_msg, keypair.private_key());

        let tx = Transaction::new_claim_epoch_reward(
            5,
            keypair.public_key().clone(),
            amount,
            recipient_hash,
            signature,
        );

        // Verify structure
        assert!(tx.is_claim_epoch_reward());
        assert!(!tx.is_coinbase());
        assert!(!tx.is_epoch_reward());
        assert_eq!(tx.tx_type, TxType::ClaimEpochReward);
        assert!(tx.inputs.is_empty());
        assert_eq!(tx.outputs.len(), 1);
        assert_eq!(tx.outputs[0].amount, amount);
        assert_eq!(tx.outputs[0].output_type, OutputType::Normal);
        assert_eq!(tx.outputs[0].pubkey_hash, recipient_hash);
        assert_eq!(tx.extra_data.len(), 136); // 72 + 64

        // Verify data parsing
        let parsed_data = tx.claim_epoch_reward_data().unwrap();
        assert_eq!(parsed_data.epoch, 5);
        assert_eq!(parsed_data.producer_pubkey, *keypair.public_key());
        assert_eq!(parsed_data.recipient_hash, recipient_hash);

        // Verify signature extraction
        let parsed_sig = tx.claim_epoch_reward_signature().unwrap();
        assert_eq!(parsed_sig, signature);

        // Verify signature is valid
        let verify_msg = parsed_data.signing_message(amount);
        assert!(crypto::signature::verify_hash(
            &verify_msg,
            &parsed_sig,
            &parsed_data.producer_pubkey
        )
        .is_ok());
    }

    #[test]
    fn test_claim_epoch_reward_not_coinbase() {
        let keypair = crypto::KeyPair::generate();
        let recipient_hash = Hash::ZERO;
        let signature = Signature::default();

        let tx = Transaction::new_claim_epoch_reward(
            1,
            keypair.public_key().clone(),
            1000,
            recipient_hash,
            signature,
        );

        assert!(!tx.is_coinbase());
        assert!(tx.is_claim_epoch_reward());
    }

    #[test]
    fn test_claim_epoch_reward_hash_deterministic() {
        let keypair = crypto::KeyPair::generate();
        let recipient_hash = Hash::ZERO;
        let signature = Signature::default();

        let tx1 = Transaction::new_claim_epoch_reward(
            1,
            keypair.public_key().clone(),
            1000,
            recipient_hash,
            signature.clone(),
        );
        let tx2 = Transaction::new_claim_epoch_reward(
            1,
            keypair.public_key().clone(),
            1000,
            recipient_hash,
            signature,
        );

        assert_eq!(tx1.hash(), tx2.hash());
    }

    #[test]
    fn test_claim_epoch_reward_serialization_roundtrip() {
        let keypair = crypto::KeyPair::generate();
        let recipient_hash = crypto::hash::hash(b"recipient");
        let signature = Signature::default();

        let tx = Transaction::new_claim_epoch_reward(
            100,
            keypair.public_key().clone(),
            50_000_000,
            recipient_hash,
            signature,
        );

        let bytes = tx.serialize();
        let recovered = Transaction::deserialize(&bytes).unwrap();

        assert_eq!(tx.tx_type, recovered.tx_type);
        assert_eq!(tx, recovered);

        let recovered_data = recovered.claim_epoch_reward_data().unwrap();
        assert_eq!(recovered_data.epoch, 100);
        assert_eq!(recovered_data.producer_pubkey, *keypair.public_key());
        assert_eq!(recovered_data.recipient_hash, recipient_hash);
    }

    #[test]
    fn test_claim_epoch_reward_data_none_for_non_claim_tx() {
        let tx = Transaction::new_coinbase(1000, Hash::ZERO, 0);
        assert!(tx.claim_epoch_reward_data().is_none());
        assert!(tx.claim_epoch_reward_signature().is_none());
    }

    #[test]
    fn test_claim_epoch_reward_signature_short_data() {
        // Create a tx with short extra_data
        let tx = Transaction {
            version: 1,
            tx_type: TxType::ClaimEpochReward,
            inputs: vec![],
            outputs: vec![Output::normal(1000, Hash::ZERO)],
            extra_data: vec![0u8; 100], // Less than 136 bytes
        };

        assert!(tx.claim_epoch_reward_signature().is_none());
    }

    #[test]
    fn test_claim_epoch_reward_different_epochs_different_hash() {
        let keypair = crypto::KeyPair::generate();
        let recipient_hash = Hash::ZERO;
        let signature = Signature::default();

        let tx1 = Transaction::new_claim_epoch_reward(
            1,
            keypair.public_key().clone(),
            1000,
            recipient_hash,
            signature.clone(),
        );
        let tx2 = Transaction::new_claim_epoch_reward(
            2, // Different epoch
            keypair.public_key().clone(),
            1000,
            recipient_hash,
            signature,
        );

        assert_ne!(tx1.hash(), tx2.hash());
    }

    // Property-based tests
    use proptest::prelude::*;

    #[allow(dead_code)]
    fn arb_hash() -> impl Strategy<Value = Hash> {
        any::<[u8; 32]>().prop_map(Hash::from_bytes)
    }

    #[allow(dead_code)]
    fn arb_output() -> impl Strategy<Value = Output> {
        (
            1u64..=u64::MAX / 2,
            arb_hash(),
            any::<bool>(),
            0u64..1_000_000u64,
        )
            .prop_map(|(amount, pubkey_hash, is_bond, lock)| {
                if is_bond {
                    Output::bond(amount, pubkey_hash, lock.max(1))
                } else {
                    Output::normal(amount, pubkey_hash)
                }
            })
    }

    #[allow(dead_code)]
    fn arb_input() -> impl Strategy<Value = Input> {
        (arb_hash(), 0u32..1000u32).prop_map(|(hash, idx)| Input::new(hash, idx))
    }

    proptest! {
        /// Transaction hash is deterministic
        #[test]
        fn prop_tx_hash_deterministic(amount in 1u64..u64::MAX/2, height: u64, seed: [u8; 32]) {
            let pubkey_hash = Hash::from_bytes(seed);
            let tx = Transaction::new_coinbase(amount, pubkey_hash, height);
            prop_assert_eq!(tx.hash(), tx.hash());
        }

        /// Different transactions have different hashes (with high probability)
        #[test]
        fn prop_different_tx_different_hash(amount1 in 1u64..u64::MAX/2, amount2 in 1u64..u64::MAX/2, height1: u64, height2: u64) {
            prop_assume!(amount1 != amount2 || height1 != height2);
            let pubkey_hash = Hash::ZERO;
            let tx1 = Transaction::new_coinbase(amount1, pubkey_hash, height1);
            let tx2 = Transaction::new_coinbase(amount2, pubkey_hash, height2);
            prop_assert_ne!(tx1.hash(), tx2.hash());
        }

        /// Serialization roundtrip preserves transaction
        #[test]
        fn prop_tx_serialization_roundtrip(amount in 1u64..u64::MAX/2, height: u64, seed: [u8; 32]) {
            let pubkey_hash = Hash::from_bytes(seed);
            let tx = Transaction::new_coinbase(amount, pubkey_hash, height);
            let bytes = tx.serialize();
            let recovered = Transaction::deserialize(&bytes);
            prop_assert!(recovered.is_some());
            prop_assert_eq!(tx, recovered.unwrap());
        }

        /// total_output sums correctly
        #[test]
        fn prop_total_output_sums(amounts in prop::collection::vec(1u64..1_000_000u64, 1..10)) {
            let outputs: Vec<Output> = amounts.iter()
                .map(|&a| Output::normal(a, Hash::ZERO))
                .collect();
            let tx = Transaction {
                version: 1,
                tx_type: TxType::Transfer,
                inputs: vec![Input::new(Hash::ZERO, 0)],
                outputs,
                extra_data: vec![],
            };
            let expected: Amount = amounts.iter().sum();
            prop_assert_eq!(tx.total_output(), expected);
        }

        /// Output spendability: normal outputs always spendable
        #[test]
        fn prop_normal_always_spendable(amount in 1u64..u64::MAX/2, height: u64) {
            let output = Output::normal(amount, Hash::ZERO);
            prop_assert!(output.is_spendable_at(height));
        }

        /// Output spendability: bond outputs respect lock time
        #[test]
        fn prop_bond_respects_lock(amount in 1u64..u64::MAX/2, lock_height in 1u64..u64::MAX/2) {
            let output = Output::bond(amount, Hash::ZERO, lock_height);
            // Not spendable before lock
            if lock_height > 0 {
                prop_assert!(!output.is_spendable_at(lock_height - 1));
            }
            // Spendable at and after lock
            prop_assert!(output.is_spendable_at(lock_height));
            if lock_height < u64::MAX {
                prop_assert!(output.is_spendable_at(lock_height + 1));
            }
        }

        /// Input serialization is deterministic
        #[test]
        fn prop_input_serialize_deterministic(seed: [u8; 32], idx: u32) {
            let hash = Hash::from_bytes(seed);
            let input = Input::new(hash, idx);
            prop_assert_eq!(input.serialize_for_signing(), input.serialize_for_signing());
        }

        /// Output serialization is deterministic
        #[test]
        fn prop_output_serialize_deterministic(amount in 1u64..u64::MAX/2, seed: [u8; 32]) {
            let output = Output::normal(amount, Hash::from_bytes(seed));
            prop_assert_eq!(output.serialize(), output.serialize());
        }

        /// Coinbase detection: empty inputs + single output = coinbase
        #[test]
        fn prop_coinbase_detection(amount in 1u64..u64::MAX/2, height: u64, seed: [u8; 32]) {
            let tx = Transaction::new_coinbase(amount, Hash::from_bytes(seed), height);
            prop_assert!(tx.is_coinbase());
            prop_assert!(tx.inputs.is_empty());
            prop_assert_eq!(tx.outputs.len(), 1);
        }

        /// Transfer with inputs is not coinbase
        #[test]
        fn prop_transfer_not_coinbase(seed: [u8; 32], idx: u32) {
            let hash = Hash::from_bytes(seed);
            let tx = Transaction::new_transfer(
                vec![Input::new(hash, idx)],
                vec![Output::normal(100, Hash::ZERO)],
            );
            prop_assert!(!tx.is_coinbase());
        }
    }
}
