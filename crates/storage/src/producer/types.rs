//! Producer type definitions

use std::collections::HashMap;

use crypto::{Hash, PublicKey};
use serde::{Deserialize, Serialize};

/// Producer status in the network
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProducerStatus {
    /// Producer is actively participating in block production
    Active,
    /// Producer has requested exit, in unbonding period (7 days)
    /// During this period, producer can still produce blocks
    Unbonding {
        /// Block height when unbonding started
        started_at: u64,
    },
    /// Producer has completed unbonding and can claim their bond
    Exited,
    /// Producer was slashed for misbehavior (100% bond burned)
    /// They are permanently excluded from the network
    Slashed {
        /// Block height when slashing occurred
        slashed_at: u64,
    },
}

// ==================== Activity Status System ====================

/// Activity status of a producer
///
/// Determines governance power and quorum participation.
/// **Principle: "El silencio no bloquea"** - only active producers
/// can block governance changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityStatus {
    /// Producing blocks regularly. Full governance power.
    /// - Can vote on proposals
    /// - Counts for quorum denominator
    /// - Receives rewards for blocks produced
    Active,

    /// Recently stopped producing (< 2 weeks). Grace period.
    /// - Cannot vote on proposals
    /// - Does NOT count for quorum
    /// - Can reactivate by producing blocks
    RecentlyInactive,

    /// Dormant for extended period (>= 2 weeks). No governance power.
    /// - Cannot vote on proposals
    /// - Does NOT count for quorum
    /// - Bond remains locked (auto-renew)
    /// - Can reactivate by producing blocks for 24h
    Dormant,
}

/// A single bond entry with its creation time, stored in ProducerInfo.
///
/// Each bond tracks when it was created for per-bond vesting/penalty calculation.
/// Sorted by `creation_slot` ascending (oldest first) for FIFO withdrawal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredBondEntry {
    /// Slot when this bond was created
    pub creation_slot: u32,
    /// Amount staked (always BOND_UNIT)
    pub amount: u64,
}

/// Information about a registered producer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProducerInfo {
    /// Producer's public key
    pub public_key: PublicKey,
    /// Block height when registered
    pub registered_at: u64,
    /// Bond amount locked (total across all bonds)
    pub bond_amount: u64,
    /// Bond output location (tx_hash, output_index) - primary bond
    pub bond_outpoint: (Hash, u32),
    /// Current status
    pub status: ProducerStatus,
    /// DEPRECATED: Blocks produced counter (never updated by EpochPool rewards)
    ///
    /// This field is vestigial from the Pull/Claim reward model. The active
    /// EpochPool system calculates block counts from the BlockStore on-demand.
    /// Field retained for serialization backwards compatibility only.
    #[deprecated(
        note = "Block counts are calculated from BlockStore - this field is never updated"
    )]
    #[serde(default)]
    pub blocks_produced: u64,
    /// Number of missed slots
    pub slots_missed: u64,
    /// Era when registered (for renewal tracking)
    pub registration_era: u32,
    /// DEPRECATED: Pending rewards counter (never updated by EpochPool rewards)
    ///
    /// This field is vestigial from the Pull/Claim reward model. The active
    /// EpochPool system distributes rewards directly to UTXOs via EpochReward
    /// transactions. Field retained for serialization backwards compatibility only.
    #[deprecated(
        note = "Rewards go directly to UTXOs via EpochReward txs - this field is never updated"
    )]
    #[serde(default)]
    pub pending_rewards: u64,
    /// Whether this producer has previously exited (anti-Sybil: maturity cooldown)
    ///
    /// When true, this producer has previously exited the network and re-registered.
    /// They start fresh with weight 1, losing all prior seniority.
    /// This prevents "hit-and-run" attacks where a producer votes for something
    /// malicious and then exits before consequences.
    #[serde(default)]
    pub has_prior_exit: bool,
    /// Last block height where this producer showed activity (produced block or voted)
    ///
    /// Used for uptime tracking. Dormant nodes have reduced effective weight.
    #[serde(default)]
    pub last_activity: u64,
    /// DEPRECATED: Activity gaps are no longer tracked or penalized.
    /// Per alignment decision: No activity penalty. Producers who miss slots
    /// simply miss rewards - no weight reduction. Only slashable offense is
    /// double production (equivocation).
    /// Field retained for serialization backwards compatibility only.
    #[serde(default)]
    #[deprecated(
        note = "Activity gaps no longer affect consensus - field kept for backwards compatibility"
    )]
    pub activity_gaps: u64,
    /// Number of bonds staked (1-100)
    ///
    /// Bond Stacking System: More bonds = more chances to produce blocks.
    /// Each bond is 1,000 DOLI. Maximum 100 bonds per producer.
    /// In selection, producers are expanded by their bond_count for weighted lottery.
    /// Example: [Alice:1, Bob:5] → [Alice, Bob, Bob, Bob, Bob, Bob]
    /// This ensures ROI is proportional: 10 bonds cost 10x, earn ~10x.
    #[serde(default = "default_bond_count")]
    pub bond_count: u32,
    /// Additional bond outpoints (for bond stacking)
    /// First bond uses bond_outpoint, additional bonds stored here
    #[serde(default)]
    pub additional_bonds: Vec<(Hash, u32)>,
    /// Who this producer delegates bond weight to (Tier 3 → Tier 1/2)
    #[serde(default)]
    pub delegated_to: Option<PublicKey>,
    /// How many bonds are delegated
    #[serde(default)]
    pub delegated_bonds: u32,
    /// Delegations received from other producers: (delegator pubkey hash, bond count)
    #[serde(default)]
    pub received_delegations: Vec<(Hash, u32)>,
    /// Per-bond creation timestamps for FIFO withdrawal penalty calculation.
    /// Sorted by creation_slot ascending (oldest first).
    /// Auto-populated from bond_count + registered_at on first load if empty.
    #[serde(default)]
    pub bond_entries: Vec<StoredBondEntry>,
    /// Bonds pending withdrawal this epoch (prevents double-withdrawal).
    /// Reset to 0 when RequestWithdrawal is applied at epoch boundary.
    #[serde(default)]
    pub withdrawal_pending_count: u32,
    /// BLS12-381 public key for aggregate attestation signatures (48 bytes).
    ///
    /// Set at registration time (with `PoP` verification). Empty for pre-BLS producers.
    /// Used to verify aggregate attestation signatures in block validation.
    #[serde(default)]
    pub bls_pubkey: Vec<u8>,
    /// Whether this producer is currently in the active round-robin schedule.
    ///
    /// Reactive Round-Robin Scheduling:
    /// - `true` (default): producer is in the round-robin and gets assigned slots
    /// - `false`: producer missed their slot and is temporarily excluded
    ///
    /// Removal: when a block is applied, any producer who was rank 0 for a missed
    /// slot (or who was rank 0 but a fallback produced instead) gets `scheduled = false`.
    ///
    /// Re-entry: when a producer's attestation appears in a block's `presence_root`
    /// bitfield, they get `scheduled = true` again.
    ///
    /// IMPORTANT: This field is SKIPPED during bincode serialization to maintain
    /// backward compatibility with existing RocksDB data. Bincode is positional
    /// and does NOT support `#[serde(default)]` — adding a field breaks deserialization
    /// of old data. Instead, `scheduled` defaults to `true` on load and is recomputed
    /// by the reactive scheduling logic in `apply_block()` from on-chain data.
    #[serde(skip, default = "default_scheduled")]
    pub scheduled: bool,
}

/// Default scheduled status for backwards compatibility (all producers start scheduled)
pub(crate) fn default_scheduled() -> bool {
    true
}

/// Default bond count for backwards compatibility
pub(crate) fn default_bond_count() -> u32 {
    1
}

/// A deferred producer set mutation, queued during block application and
/// applied at the next epoch boundary. This prevents scheduler divergence
/// between forks: all nodes apply the same mutations at deterministic heights.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PendingProducerUpdate {
    Register {
        info: Box<ProducerInfo>,
        height: u64,
    },
    Exit {
        pubkey: PublicKey,
        height: u64,
    },
    Slash {
        pubkey: PublicKey,
        height: u64,
    },
    AddBond {
        pubkey: PublicKey,
        outpoints: Vec<(Hash, u32)>,
        bond_unit: u64,
        creation_slot: u32,
    },
    DelegateBond {
        delegator: PublicKey,
        delegate: PublicKey,
        bond_count: u32,
    },
    RevokeDelegation {
        delegator: PublicKey,
    },
    RequestWithdrawal {
        pubkey: PublicKey,
        bond_count: u32,
        bond_unit: u64,
    },
}

/// Set of producers with their states
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProducerSet {
    /// All producers indexed by public key hash
    pub(super) producers: HashMap<Hash, ProducerInfo>,
    /// Exit history: pubkey hash -> exit block height (anti-Sybil with expiration)
    ///
    /// Used to track producers who have previously exited the network.
    /// When they re-register within EXIT_HISTORY_RETENTION, they start fresh
    /// with weight 1 (maturity cooldown). After 8 years, the record expires
    /// and they can re-register without penalty.
    ///
    /// This bounds the exit_history size while maintaining anti-Sybil protection
    /// beyond any realistic attack horizon.
    #[serde(default)]
    pub(super) exit_history: HashMap<Hash, u64>,
    /// Epoch-based cache of active producer keys at a given height.
    /// Avoids O(n) scan of all producers every slot.
    /// Invalidated on any mutation (register, exit, slash, etc.).
    #[serde(skip)]
    pub(super) active_cache: Option<(u64, Vec<Hash>)>,
    /// Index of unbonding producers by their unbonding start height.
    /// Enables O(k) lookup in `process_unbonding()` instead of O(n) full scan.
    /// Rebuilt on deserialization from `producers` map.
    #[serde(skip)]
    pub(super) unbonding_index: std::collections::BTreeMap<u64, Vec<Hash>>,
    /// Deferred producer mutations queued during block application.
    /// Applied at epoch boundaries to ensure scheduler consistency across forks.
    #[serde(default)]
    pub(super) pending_updates: Vec<PendingProducerUpdate>,
}
