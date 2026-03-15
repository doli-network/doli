//! Producer, bond, schedule, epoch, and governance types.

use serde::{Deserialize, Serialize};

// ==================== Request Parameters ====================

/// Parameters for getProducer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProducerParams {
    /// Producer public key (hex)
    #[serde(alias = "publicKey")]
    pub public_key: String,
}

/// Parameters for getProducers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProducersParams {
    /// Only return active producers
    #[serde(default)]
    pub active_only: bool,
}

/// Vote message for governance veto system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteMessageParams {
    /// Version being voted on
    pub version: String,
    /// Vote type: "approve" or "veto"
    pub vote: String,
    /// Producer's public key (hex)
    pub producer_id: String,
    /// Unix timestamp of the vote
    pub timestamp: u64,
    /// Signature over "version:vote:timestamp" (hex)
    pub signature: String,
}

/// Parameters for submitVote
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitVoteParams {
    /// The signed vote message
    pub vote: VoteMessageParams,
}

// ==================== Response Types ====================

/// Producer response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerResponse {
    /// Public key (hex)
    pub public_key: String,
    /// Address hash (hex) — BLAKE3(ADDRESS_DOMAIN || pubkey), used for balance lookups
    pub address_hash: String,
    /// Registration height
    pub registration_height: u64,
    /// Bond amount (total)
    pub bond_amount: u64,
    /// Number of bonds staked
    pub bond_count: u32,
    /// Status (active, unbonding, exited, slashed)
    pub status: String,
    /// Current era
    pub era: u64,
    /// Pending withdrawals
    #[serde(default)]
    pub pending_withdrawals: Vec<PendingWithdrawalResponse>,
    /// Pending epoch-deferred updates
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_updates: Vec<PendingUpdateInfo>,
    /// BLS12-381 public key for aggregate attestation (hex, 96 chars = 48 bytes).
    /// Empty string if producer registered before BLS was available.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bls_pubkey: String,
}

/// Pending withdrawal response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingWithdrawalResponse {
    /// Number of bonds being withdrawn
    pub bond_count: u32,
    /// Slot when withdrawal was requested
    pub request_slot: u32,
    /// Net amount after penalty
    pub net_amount: u64,
    /// Whether this withdrawal can be claimed now
    pub claimable: bool,
}

/// Pending producer update info (epoch-deferred)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdateInfo {
    /// Update type (register, exit, add_bond, withdrawal, etc.)
    pub update_type: String,
    /// Bond count affected (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bond_count: Option<u32>,
}

// ==================== Epoch ====================

/// Response for getEpochInfo
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpochInfoResponse {
    /// Current block height
    pub current_height: u64,
    /// Current reward epoch
    pub current_epoch: u64,
    /// Last complete epoch (if any)
    pub last_complete_epoch: Option<u64>,
    /// Blocks per reward epoch
    pub blocks_per_epoch: u64,
    /// Blocks remaining in current epoch
    pub blocks_remaining: u64,
    /// Current epoch start height
    pub epoch_start_height: u64,
    /// Current epoch end height (exclusive)
    pub epoch_end_height: u64,
    /// Current block reward rate
    pub block_reward: u64,
}

// ==================== Bond Details ====================

/// Parameters for getBondDetails
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBondDetailsParams {
    /// Producer public key (hex)
    #[serde(alias = "publicKey")]
    pub public_key: String,
}

/// Response for getBondDetails
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondDetailsResponse {
    /// Producer public key (hex)
    pub public_key: String,
    /// Total bond count
    pub bond_count: u32,
    /// Total staked amount (base units)
    pub total_staked: u64,
    /// Registration slot (proxy for bond creation)
    pub registration_slot: u64,
    /// Age in slots since oldest bond
    pub age_slots: u64,
    /// Penalty percentage for oldest bond (0-75)
    pub penalty_pct: u8,
    /// Whether all bonds are fully vested
    pub vested: bool,
    /// Slot when newest bond becomes fully vested (0% penalty)
    pub maturation_slot: u64,
    /// Vesting quarter duration in slots
    pub vesting_quarter_slots: u64,
    /// Full vesting period in slots
    pub vesting_period_slots: u64,
    /// Summary by vesting quarter
    pub summary: BondsSummaryResponse,
    /// Per-bond details (sorted oldest first)
    #[serde(default)]
    pub bonds: Vec<BondEntryResponse>,
    /// Bonds pending withdrawal this epoch
    #[serde(default)]
    pub withdrawal_pending_count: u32,
}

/// Individual bond entry response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondEntryResponse {
    /// Slot when this bond was created
    pub creation_slot: u32,
    /// Amount staked (base units)
    pub amount: u64,
    /// Age in slots
    pub age_slots: u64,
    /// Current penalty percentage (0-75)
    pub penalty_pct: u8,
    /// Whether this bond is fully vested
    pub vested: bool,
    /// Slot when this bond becomes fully vested
    pub maturation_slot: u64,
}

/// Bond summary by vesting quarter
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondsSummaryResponse {
    /// Bonds in Q1 (0-6h, 75% penalty)
    pub q1: u32,
    /// Bonds in Q2 (6-12h, 50% penalty)
    pub q2: u32,
    /// Bonds in Q3 (12-18h, 25% penalty)
    pub q3: u32,
    /// Fully vested bonds (18h+, 0% penalty)
    pub vested: u32,
}

// ==================== Schedule ====================

/// Parameters for getSlotSchedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSlotScheduleParams {
    /// Starting slot (default = current slot)
    #[serde(default, alias = "fromSlot")]
    pub from_slot: Option<u32>,
    /// Number of slots to return (default = 20, max 360)
    #[serde(default, alias = "count")]
    pub count: Option<u32>,
}

/// Slot schedule entry
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotScheduleEntry {
    /// Slot number
    pub slot: u32,
    /// Producer public key (hex)
    pub producer: String,
    /// Rank (0 = primary producer)
    pub rank: usize,
}

/// Response for getSlotSchedule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotScheduleResponse {
    /// Scheduled slots
    pub slots: Vec<SlotScheduleEntry>,
    /// Current slot
    pub current_slot: u32,
    /// Current epoch
    pub epoch: u64,
    /// Slots remaining in current epoch
    pub slots_remaining_in_epoch: u64,
    /// Total bonds across all producers
    pub total_bonds: u64,
    /// Slot duration in seconds
    pub slot_duration: u64,
    /// Genesis time (unix timestamp)
    pub genesis_time: u64,
}

/// Parameters for getProducerSchedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProducerScheduleParams {
    /// Producer public key (hex)
    #[serde(alias = "publicKey")]
    pub public_key: String,
}

/// Response for getProducerSchedule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerScheduleResponse {
    /// Producer public key (hex)
    pub public_key: String,
    /// Current slot
    pub current_slot: u32,
    /// Current epoch
    pub epoch: u64,
    /// Next slot where this producer is primary
    pub next_slot: Option<u32>,
    /// Seconds until next assigned slot
    pub seconds_until_next: Option<u64>,
    /// All slots this epoch where producer is primary
    pub slots_this_epoch: Vec<u32>,
    /// Number of assigned slots this epoch
    pub assigned_count: u32,
    /// Number of blocks actually produced this epoch
    pub produced_count: u32,
    /// Fill rate (produced / assigned)
    pub fill_rate: f64,
    /// Producer's bond count
    pub bond_count: u32,
    /// Total network bonds
    pub total_network_bonds: u64,
    /// Estimated weekly earnings (base units)
    pub weekly_earnings: u64,
    /// Weeks until bond investment doubles from rewards
    pub doubling_weeks: f64,
    /// Current block reward (base units)
    pub block_reward: u64,
}
