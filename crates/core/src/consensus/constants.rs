use crate::types::{Amount, BlockHeight, Slot};

// ==================== Protocol Versioning ====================

/// Initial protocol version at genesis. All existing chains start at v1.
/// Incremented via on-chain ProtocolActivation transactions (3/5 maintainer multisig).
pub const INITIAL_PROTOCOL_VERSION: u32 = 1;

/// Check if a protocol version is active given the current active version.
///
/// Used to gate consensus-critical code behind protocol version checks:
/// ```ignore
/// if is_protocol_active(2, state.active_protocol_version) {
///     new_consensus_rules();
/// } else {
///     old_consensus_rules();
/// }
/// ```
pub fn is_protocol_active(required_version: u32, active_version: u32) -> bool {
    active_version >= required_version
}

/// Genesis timestamp — must match chainspec.mainnet.json
/// Guarded by `test_genesis_time_matches_chainspec` test.
pub const GENESIS_TIME: u64 = 1773694559;

/// Deterministic reward pool address.
/// All per-block coinbase goes here; distributed to qualified producers at epoch end.
/// No private key exists for this address — only the consensus engine can move funds.
pub fn reward_pool_pubkey_hash() -> crypto::Hash {
    crypto::hash::hash_with_domain(b"REWARD_POOL", b"doli")
}

// ==================== Proof of Time Parameters ====================

/// Slot duration in seconds.
///
/// # Proof of Time Timing
///
/// With 10-second slots:
/// - VDF computation: ~7s (T_BLOCK iterations)
/// - Block construction and broadcast: ~3s
/// - Total: 10 seconds
///
/// VDF is required for anti-grinding protection.
pub const SLOT_DURATION: u64 = 10;

/// Slots per epoch (1 hour = 360 slots at 10s each)
/// Used for consensus-level producer set stability.
pub const SLOTS_PER_EPOCH: u32 = 360;

/// Slots per reward epoch (1 hour = 360 slots at 10s each)
/// Each producer receives 100% of the block reward when they produce a block.
pub const SLOTS_PER_REWARD_EPOCH: u32 = 360;

/// Blocks per reward epoch (360 blocks for mainnet/testnet)
///
/// This is the primary constant for the weighted presence reward system.
/// Reward epochs are defined by block height (not slot), making calculation
/// simpler since block heights are sequential with no gaps.
///
/// Examples:
/// - 360 blocks ≈ 1 hour at 10s blocks (mainnet)
/// - 60 blocks  ≈ 1 minute at 1s blocks (devnet)
/// - 8640 blocks ≈ 24 hours (daily rewards)
pub const BLOCKS_PER_REWARD_EPOCH: BlockHeight = 360;

/// Slots per year (365 days * 24 hours * 360 slots/hour)
/// Used for seniority weight calculations.
pub const SLOTS_PER_YEAR: u32 = 3_153_600;

/// Minimum presence rate to remain in the active producer set (percentage).
/// Producers must successfully produce at least this percentage of their
/// assigned blocks to maintain good standing.
/// 50% = must produce at least half of assigned slots
pub const MIN_PRESENCE_RATE: u32 = 50;

/// Minimum attestation rate - alias for backward compatibility
pub const MIN_ATTESTATION_RATE: u32 = MIN_PRESENCE_RATE;

/// Attestation interval - kept for backward compatibility.
/// In Proof of Time, there are no attestations; each block
/// production with valid VDF IS the proof of time spent.
#[deprecated(note = "Attestations not used in Proof of Time")]
pub const ATTESTATION_INTERVAL: u32 = 1;

// ==================== Epoch Reward Qualification ====================

/// Slots per era (~4 years at 10-second slots)
/// 4 years = 12,614,400 slots (halving interval)
pub const SLOTS_PER_ERA: BlockHeight = 12_614_400;

/// Blocks per era - alias for SLOTS_PER_ERA
pub const BLOCKS_PER_ERA: BlockHeight = SLOTS_PER_ERA;

/// Halving interval - same as SLOTS_PER_ERA
pub const HALVING_INTERVAL: BlockHeight = SLOTS_PER_ERA;

/// Bootstrap phase duration in blocks (~1 week at 10-second slots)
/// 7 days = 60,480 blocks
pub const BOOTSTRAP_BLOCKS: BlockHeight = 60_480;

/// Liveness window: producers who haven't produced within this many blocks
/// are excluded from primary scheduling (but eligible for re-entry slots).
/// Dynamic formula: `max(LIVENESS_WINDOW_MIN, total_producers * 3)` ensures
/// every producer gets ~3 primary opportunities before being classified stale.
pub const LIVENESS_WINDOW_MIN: u64 = 500;

/// Re-entry interval in slots. Every K slots per stale producer, that producer
/// gets rank 0 (exclusive 2s window) to produce a block and rejoin the live rotation.
/// K=50 → 2% overhead per stale producer. Capped at 20% total (K/5 stale max).
pub const REENTRY_INTERVAL: u32 = 50;

/// Inactivity leak: after INACTIVITY_LEAK_START missed consecutive slots,
/// a producer's effective bond weight decays by INACTIVITY_LEAK_RATE% per epoch.
/// Matches Ethereum's inactivity leak: offline validators lose stake
/// quadratically until online validators control >2/3.
pub const INACTIVITY_LEAK_START: u64 = 360; // 1 epoch of missed slots
pub const INACTIVITY_LEAK_RATE: u64 = 10; // 10% per epoch
pub const INACTIVITY_LEAK_FLOOR: u64 = 1; // Minimum 1 bond (never fully zeroed)

/// Bootstrap grace period in seconds.
///
/// At genesis startup, when all peers are at height 0, the node waits this
/// duration before allowing block production. This distinguishes between:
/// - **True genesis**: We're the first producer, safe to start
/// - **Network partition**: We're isolated from the real chain, dangerous!
///
/// Waiting gives time to connect to the real network if we're partitioned.
/// Default: 15 seconds for mainnet/testnet, configurable for devnet.
pub const BOOTSTRAP_GRACE_PERIOD_SECS: u64 = 15;

/// Maximum clock drift allowed (seconds).
/// Nodes with clocks drifting more than this are considered out of sync.
/// Tightened from 10s to 1s for 55ms VDF + 2s sequential fallback windows.
pub const MAX_DRIFT: u64 = 1;

/// Network margin for block timing (milliseconds).
/// Time reserved for presence signature collection.
/// With 1-second slots, we allocate 200ms buffer at the end.
pub const NETWORK_MARGIN_MS: u64 = 200;

/// Network margin in seconds (for backward compatibility).
/// Rounded up from NETWORK_MARGIN_MS.
pub const NETWORK_MARGIN: u64 = 1;

/// Maximum slots in the future a block can be accepted.
/// Prevents clock manipulation attacks where a node with a fast clock
/// produces blocks for future slots.
/// With 10-second slots, 1 slot = 10 seconds into the future.
pub const MAX_FUTURE_SLOTS: u64 = 1;

/// Maximum slots in the past a block can be accepted.
/// Allows for late blocks due to network delays, but prevents
/// producers from mining old slots indefinitely.
/// With 10-second slots, 192 slots = 32 minutes of history.
pub const MAX_PAST_SLOTS: u64 = 192;

/// Initial block reward (1 DOLI = 100,000,000 base units per block)
///
/// # Emission Schedule
///
/// With 10-second slots:
/// - 1 DOLI per block × 12,614,400 blocks/era = 12,614,400 DOLI/era
/// - Halves every era (~4 years)
/// - Total supply converges to 25,228,800 DOLI
///
/// Per epoch (1 hour = 360 blocks): 360 DOLI distributed
pub const INITIAL_REWARD: Amount = 100_000_000;

/// Initial block reward - alias for INITIAL_REWARD
pub const INITIAL_BLOCK_REWARD: Amount = INITIAL_REWARD;

/// Block reward pool per slot (alias for clarity)
pub const BLOCK_REWARD_POOL: Amount = INITIAL_REWARD;

/// Epoch reward pool (360 DOLI per hour)
/// 360 slots × 100,000,000 base units = 36,000,000,000 base units = 360 DOLI
pub const EPOCH_REWARD_POOL: Amount = SLOTS_PER_REWARD_EPOCH as u64 * INITIAL_REWARD;

/// Coinbase maturity (confirmations required before spending)
/// 6 blocks = ~60 seconds at 10s slots
pub const COINBASE_MATURITY: BlockHeight = 6;

// ==================== Bond Stacking System ====================
//
// Producers stake bonds to participate in block production.
// More bonds = more selection weight = more block production opportunities.
// Each bond has its own vesting timer (4 years to full maturity).

/// Bond unit: 0.1 DOLI = 1 slot per cycle (local dev)
/// This is the atomic unit for staking. You can only stake in multiples of this.
/// With 10 DOLI per bond unit:
/// - Producer with 100 DOLI = 10 slots per cycle
/// - Maximum 3,000 bonds = 30,000 DOLI maximum per producer
pub const BOND_UNIT: Amount = 1_000_000_000; // 10 DOLI in base units

/// Initial bond amount - alias for backward compatibility
pub const INITIAL_BOND: Amount = BOND_UNIT;

/// Maximum bonds per producer
/// 3,000 bonds × 10 DOLI = 30,000 DOLI maximum stake per node
pub const MAX_BONDS_PER_PRODUCER: u32 = 3_000;

/// One year in slots (used for seniority weight calculation — NOT vesting)
/// 365 days * 24 hours * 360 slots/hour = 3,153,600 slots
pub const YEAR_IN_SLOTS: Slot = 3_153_600;

/// One vesting quarter (1 year at 10s slots = 3,153,600 slots)
/// 4 quarters = 4 years to full maturity on mainnet.
/// Testnet/devnet override via NetworkParams.vesting_quarter_slots.
pub const VESTING_QUARTER_SLOTS: Slot = 3_153_600;

/// Full vesting period (4 years = 4 × 3,153,600 = 12,614,400 slots)
pub const VESTING_PERIOD_SLOTS: Slot = 4 * VESTING_QUARTER_SLOTS;

/// Commitment period for full vesting (4 years)
/// After 4 years, bonds can be withdrawn with 0% penalty
pub const COMMITMENT_PERIOD: BlockHeight = VESTING_PERIOD_SLOTS as BlockHeight;

/// Unbonding period for exit (~7 days at 10-second slots)
/// After requesting exit, producers must wait this long before bond is released
pub const UNBONDING_PERIOD: BlockHeight = 60_480;

/// Lock duration for bonds (4 years for full vesting on mainnet)
pub const BOND_LOCK_BLOCKS: BlockHeight = COMMITMENT_PERIOD;

/// Calculate withdrawal penalty rate based on bond age.
///
/// # Vesting Schedule (4-year, year-based — mainnet)
/// - Y1 (0-1yr): 75% penalty
/// - Y2 (1-2yr): 50% penalty
/// - Y3 (2-3yr): 25% penalty
/// - Y4+ (3yr+): 0% penalty (fully vested)
///
/// Testnet uses 1-day schedule (6h quarters) via NetworkParams.
///
/// # Arguments
/// - `bond_age_slots`: How many slots since the bond was created
///
/// # Returns
/// Penalty percentage (0-75)
pub fn withdrawal_penalty_rate(bond_age_slots: Slot) -> u8 {
    withdrawal_penalty_rate_with_quarter(bond_age_slots, VESTING_QUARTER_SLOTS)
}

/// Calculate withdrawal penalty rate with a custom quarter duration.
///
/// Used by `NetworkParams`-aware code paths (devnet can override quarter length).
pub fn withdrawal_penalty_rate_with_quarter(bond_age_slots: Slot, quarter_slots: Slot) -> u8 {
    let quarters = bond_age_slots / quarter_slots;
    match quarters {
        0 => 75, // Q1: 75% penalty
        1 => 50, // Q2: 50% penalty
        2 => 25, // Q3: 25% penalty
        _ => 0,  // Q4+: no penalty (fully vested)
    }
}

/// Maximum consecutive missed slots before considered inactive.
/// With 10-second slots, 50 missed slots = ~8 minutes of inactivity.
/// After this many misses, the producer is considered inactive.
pub const MAX_FAILURES: u32 = 50;

/// Inactivity threshold - same as MAX_FAILURES
pub const INACTIVITY_THRESHOLD: u32 = MAX_FAILURES;

/// Exclusion period for slashing (7 days in slots at 10-second slots)
/// 7 days = 60,480 slots
pub const EXCLUSION_SLOTS: Slot = 60_480;

/// Reward maturity (confirmations required)
pub const REWARD_MATURITY: BlockHeight = 6;

/// Base block size (Era 0) in bytes
pub const BASE_BLOCK_SIZE: usize = 2_000_000;

/// Maximum block size cap (Era 4+) in bytes
pub const MAX_BLOCK_SIZE_CAP: usize = 32_000_000;

/// Calculate max block size for a given height.
///
/// Block size doubles every era (~4 years):
/// - Era 0: 2 MB
/// - Era 1: 4 MB
/// - Era 2: 8 MB
/// - Era 3: 16 MB
/// - Era 4+: 32 MB (capped)
///
/// This growth is encoded in the protocol from genesis.
/// No hard forks or voting required.
#[must_use]
pub fn max_block_size(height: BlockHeight) -> usize {
    let era = height / BLOCKS_PER_ERA;
    if era >= 4 {
        MAX_BLOCK_SIZE_CAP
    } else {
        BASE_BLOCK_SIZE << era // shift left = multiply by 2^era
    }
}

/// Total supply (25,228,800 DOLI)
/// Calculated as: sum of geometric series with initial reward and halving
/// 25,228,800 DOLI * 100,000,000 base units = 2,522,880,000,000,000
pub const TOTAL_SUPPLY: Amount = 2_522_880_000_000_000;

// ==================== Proof of Time Types ====================

/// Presence score for a producer.
///
/// The score determines producer priority for block production.
/// Higher score = selected first. Score increases when you produce
/// blocks and decreases when you miss your assigned slots.
pub type PresenceScore = u64;

/// Minimum presence score to be eligible for block production
pub const MIN_PRESENCE_SCORE: PresenceScore = 1;

/// Maximum presence score (prevents overflow)
pub const MAX_PRESENCE_SCORE: PresenceScore = 10_000;

/// Initial presence score for new producers
pub const INITIAL_PRESENCE_SCORE: PresenceScore = 100;

/// Score bonus for producing a block when assigned
pub const SCORE_PRODUCE_BONUS: PresenceScore = 1;

/// Score penalty for missing an assigned slot
pub const SCORE_MISS_PENALTY: PresenceScore = 2;

// ==================== Producer Window Parameters ====================
//
// With 1-second slots, producer windows are tight:
// - Primary window: 0-300ms (only primary producer can submit)
// - Secondary window: 300-600ms (primary or secondary)
// - Tertiary window: 600-800ms (any of top 3)
// - Final 200ms reserved for signature collection
//
// The producer with highest presence_score builds the block.
// If they don't submit in time, fallbacks can step in.

/// Primary producer window in milliseconds - DEPRECATED.
/// Use FALLBACK_TIMEOUT_MS for sequential 2s windows instead.
#[deprecated(note = "Use FALLBACK_TIMEOUT_MS for sequential 2s windows")]
pub const PRIMARY_WINDOW_MS: u64 = 3_000;

/// Secondary producer window in milliseconds - DEPRECATED.
#[deprecated(note = "Use FALLBACK_TIMEOUT_MS for sequential 2s windows")]
pub const SECONDARY_WINDOW_MS: u64 = 6_000;

/// Tertiary producer window in milliseconds - DEPRECATED.
#[deprecated(note = "Use FALLBACK_TIMEOUT_MS for sequential 2s windows")]
pub const TERTIARY_WINDOW_MS: u64 = 10_000;

/// Signature collection window - deprecated with 10s slots.
/// Block propagation happens within the slot window.
pub const SIGNATURE_WINDOW_MS: u64 = 0;

/// Primary producer window in seconds - DEPRECATED.
#[deprecated(note = "Use FALLBACK_TIMEOUT_MS for sequential 2s windows")]
pub const PRIMARY_WINDOW_SECS: u64 = 3;

/// Secondary producer window in seconds - DEPRECATED.
#[deprecated(note = "Use FALLBACK_TIMEOUT_MS for sequential 2s windows")]
pub const SECONDARY_WINDOW_SECS: u64 = 6;

/// Tertiary producer window in seconds - DEPRECATED.
#[deprecated(note = "Use FALLBACK_TIMEOUT_MS for sequential 2s windows")]
pub const TERTIARY_WINDOW_SECS: u64 = 10;

/// Fast block threshold - reserved for future VDF timing optimizations
pub const FAST_THRESHOLD_MS: u64 = 0;
pub const FAST_THRESHOLD: u64 = 0;

/// Maximum number of fallback producers per slot.
/// With sequential 2s windows: 5 ranks (0-4) each get exclusive 2s,
/// filling the entire 10s slot with no emergency window.
pub const MAX_FALLBACK_PRODUCERS: usize = 5;

/// Sequential fallback timeout in milliseconds.
/// Each rank gets an exclusive 2s window before the next rank takes over.
/// 55ms VDF + ~600ms propagation = 655ms, leaving 1345ms margin per window.
pub const FALLBACK_TIMEOUT_MS: u64 = 2_000;

/// Maximum fallback ranks (0-4 = 5 ranks, each with exclusive 2s window).
/// 5 ranks × 2000ms = 10000ms = full slot. No emergency window.
pub const MAX_FALLBACK_RANKS: usize = 5;

/// Maximum clock drift in milliseconds for fine-grained NTP validation.
/// Nodes with drift > 200ms should enable NTP synchronization.
pub const MAX_DRIFT_MS: u64 = 200;

// ==================== Tiered Architecture Constants ====================

/// Maximum Tier 1 validator count. Top N producers by effective_weight.
/// 500 nodes: O(log 500) = ~9 comparisons, 2-hop gossip in ~120ms.
pub const TIER1_MAX_VALIDATORS: usize = 500;

/// Maximum Tier 2 attestor count. Validate blocks and produce attestations.
pub const TIER2_MAX_ATTESTORS: usize = 15_000;

/// Number of gossip regions for Tier 2 sharding.
/// Each region has ~1,000 attestors with its own mesh.
pub const NUM_REGIONS: u32 = 15;

/// Percentage of block reward kept by the delegate (Tier 1/2 node).
pub const DELEGATE_REWARD_PCT: u32 = 10;

/// Percentage of block reward distributed to stakers (Tier 3 delegators).
pub const STAKER_REWARD_PCT: u32 = 90;

/// Unbonding period for delegation revocation (in slots).
pub const DELEGATION_UNBONDING_SLOTS: u64 = 60_480; // ~7 days

/// Size of the eligible producer pool for weighted selection.
///
/// Anti-Grinding Selection:
/// - Producers are sorted by pubkey (deterministic)
/// - Selection uses consecutive tickets: slot % total_tickets
/// - Fallbacks use consecutive offsets: (base + 1), (base + 2)
///
/// This prevents grinding attacks: prev_hash is not used in selection,
/// making it impossible to influence future producer selection.
pub const ELIGIBLE_PRODUCER_POOL: usize = 5;
