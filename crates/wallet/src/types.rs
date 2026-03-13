//! Shared response types for RPC communication.
//!
//! These types mirror the JSON-RPC responses from DOLI nodes.
//! They are intentionally kept compatible with the types in `bins/cli/src/rpc_client.rs`.

use serde::{Deserialize, Serialize};

/// Balance information returned by `getBalance` RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    /// Confirmed balance (in base units) - spendable
    pub confirmed: u64,
    /// Unconfirmed balance (in base units) - in mempool
    pub unconfirmed: u64,
    /// Immature balance (in base units) - coinbase/rewards pending maturity
    #[serde(default)]
    pub immature: u64,
    /// Total balance (confirmed + unconfirmed + immature)
    pub total: u64,
}

/// UTXO information returned by `getUtxos` RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Utxo {
    /// Transaction hash
    pub tx_hash: String,
    /// Output index
    pub output_index: u32,
    /// Amount in base units
    pub amount: u64,
    /// Output type (normal, bond, coinbase, etc.)
    pub output_type: String,
    /// Lock until height (0 if not locked)
    pub lock_until: u64,
    /// Block height where this UTXO was created
    pub height: u64,
    /// Whether this UTXO is currently spendable
    pub spendable: bool,
}

/// Chain information returned by `getChainInfo` RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChainInfo {
    /// Network name (mainnet, testnet, devnet)
    pub network: String,
    /// Best block hash
    pub best_hash: String,
    /// Best block height
    pub best_height: u64,
    /// Best block slot
    pub best_slot: u64,
    /// Genesis block hash
    pub genesis_hash: String,
}

/// History entry from `getHistory` RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// Transaction hash
    pub hash: String,
    /// Transaction type (string)
    pub tx_type: String,
    /// Block hash
    pub block_hash: String,
    /// Block height
    pub height: u64,
    /// Timestamp
    pub timestamp: u64,
    /// Amount received (base units)
    pub amount_received: u64,
    /// Amount sent (base units)
    pub amount_sent: u64,
    /// Fee (base units)
    pub fee: u64,
    /// Confirmations
    pub confirmations: u64,
}

/// Producer information from `getProducers` RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProducerInfo {
    /// Public key (hex)
    pub public_key: String,
    /// Registration height
    pub registration_height: u64,
    /// Bond amount (total, base units)
    pub bond_amount: u64,
    /// Number of bonds staked
    #[serde(default = "default_bond_count")]
    pub bond_count: u32,
    /// Status (active, unbonding, exited, slashed)
    pub status: String,
    /// Current era
    pub era: u64,
    /// Pending withdrawals
    #[serde(default)]
    pub pending_withdrawals: Vec<PendingWithdrawalInfo>,
    /// Pending epoch-deferred updates
    #[serde(default)]
    pub pending_updates: Vec<PendingUpdateInfo>,
}

fn default_bond_count() -> u32 {
    1
}

/// Pending withdrawal information.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingWithdrawalInfo {
    /// Number of bonds being withdrawn
    pub bond_count: u32,
    /// Slot when withdrawal was requested
    pub request_slot: u32,
    /// Net amount after penalty
    pub net_amount: u64,
    /// Whether this withdrawal can be claimed now
    pub claimable: bool,
}

/// Pending producer update info (epoch-deferred).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdateInfo {
    /// Update type (register, exit, add_bond, withdrawal, etc.)
    pub update_type: String,
    /// Bond count affected (if applicable)
    pub bond_count: Option<u32>,
}

/// Bond details response (per-bond granularity).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BondDetailsInfo {
    /// Total bond count
    pub bond_count: u32,
    /// Total staked amount (base units)
    pub total_staked: u64,
    /// Summary by vesting quarter
    pub summary: BondsSummaryInfo,
    /// Per-bond details (sorted oldest first)
    #[serde(default)]
    pub bonds: Vec<BondEntryInfo>,
    /// Bonds pending withdrawal this epoch
    #[serde(default)]
    pub withdrawal_pending_count: u32,
    /// Vesting quarter duration in slots
    #[serde(default = "default_vesting_quarter_slots")]
    pub vesting_quarter_slots: u64,
    /// Full vesting period in slots
    #[serde(default = "default_vesting_period_slots")]
    pub vesting_period_slots: u64,
}

fn default_vesting_quarter_slots() -> u64 {
    2_160
}

fn default_vesting_period_slots() -> u64 {
    8_640
}

/// Bond summary by vesting quarter.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BondsSummaryInfo {
    /// Bonds in Q1 (0-1yr, 75% penalty)
    pub q1: u32,
    /// Bonds in Q2 (1-2yr, 50% penalty)
    pub q2: u32,
    /// Bonds in Q3 (2-3yr, 25% penalty)
    pub q3: u32,
    /// Fully vested bonds (3yr+, 0% penalty)
    pub vested: u32,
}

/// Individual bond entry info.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BondEntryInfo {
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
    /// Slot when this bond fully vests
    #[serde(default)]
    pub maturation_slot: u64,
}

/// Reward epoch information.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RewardEpoch {
    /// Epoch number
    pub epoch: u64,
    /// Estimated reward amount (base units)
    pub estimated_reward: u64,
    /// Whether the producer is qualified for this epoch
    pub qualified: bool,
    /// Whether this epoch's reward has been claimed
    pub claimed: bool,
}

/// Epoch info from `getEpochInfo` RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EpochInfo {
    /// Current epoch number
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

/// Network parameters from `getNetworkParams` RPC.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NetworkParams {
    /// Network name (mainnet, testnet, devnet)
    pub network: String,
    /// Bond unit in base units (amount per bond)
    pub bond_unit: u64,
    /// Slot duration in seconds
    pub slot_duration: u64,
    /// Slots per epoch
    pub slots_per_epoch: u32,
    /// Blocks per reward epoch
    pub blocks_per_reward_epoch: u64,
    /// Coinbase maturity (blocks before spendable)
    pub coinbase_maturity: u64,
    /// Initial block reward
    pub initial_reward: u64,
    /// Genesis time
    pub genesis_time: u64,
}

/// Withdrawal simulation result.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawalSimulation {
    /// Number of bonds being withdrawn
    pub bond_count: u32,
    /// Total staked amount (before penalty)
    pub total_staked: u64,
    /// Total penalty amount
    pub total_penalty: u64,
    /// Net amount after penalty
    pub net_amount: u64,
    /// Per-bond breakdown
    pub bonds: Vec<BondWithdrawalDetail>,
}

/// Per-bond detail in a withdrawal simulation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BondWithdrawalDetail {
    /// Creation slot of this bond
    pub creation_slot: u32,
    /// Amount staked
    pub amount: u64,
    /// Penalty percentage applied
    pub penalty_pct: u8,
    /// Penalty amount
    pub penalty_amount: u64,
    /// Net amount returned
    pub net_amount: u64,
}

// ============================================================================
// Constants
// ============================================================================

/// 1 DOLI = 100,000,000 base units (8 decimal places)
pub const UNITS_PER_DOLI: u64 = 100_000_000;

/// Bond unit: 1,000,000,000 base units = 10 DOLI per bond
pub const BOND_UNIT: u64 = 1_000_000_000;

/// Maximum bonds per producer
pub const MAX_BONDS_PER_PRODUCER: u32 = 3_000;

/// Blocks per reward epoch
pub const BLOCKS_PER_REWARD_EPOCH: u64 = 360;

/// Coinbase maturity (blocks before reward/coinbase is spendable)
pub const COINBASE_MATURITY: u64 = 6;

/// Unbonding period in blocks (~7 days)
pub const UNBONDING_PERIOD: u64 = 60_480;

/// Base registration fee in base units (0.001 DOLI)
pub const BASE_REGISTRATION_FEE: u64 = 100_000;

/// Maximum registration fee in base units (0.01 DOLI)
pub const MAX_REGISTRATION_FEE: u64 = 1_000_000;

/// Vesting quarter duration in mainnet slots (~1 year)
pub const VESTING_QUARTER_SLOTS: u64 = 3_153_600;

// ============================================================================
// Unit conversion utilities
// ============================================================================

/// Convert base units to DOLI (floating point, for display only).
pub fn units_to_coins(units: u64) -> f64 {
    units as f64 / UNITS_PER_DOLI as f64
}

/// Convert a DOLI string (e.g. "1.5") to base units using integer-only arithmetic.
///
/// Splits on `.`, parses integer and fractional parts separately, pads/truncates
/// the fractional part to 8 digits, then combines. No floating point involved,
/// so there is no precision loss for any valid DOLI amount.
///
/// Returns Err if the string is not a valid positive number.
pub fn coins_to_units(coins: &str) -> Result<u64, String> {
    let trimmed = coins.trim();
    if trimmed.is_empty() {
        return Err(format!("Invalid amount: '{}'", coins));
    }

    // Reject negative values
    if trimmed.starts_with('-') {
        return Err("Amount cannot be negative".to_string());
    }

    // Strip optional leading '+' (uncommon but valid)
    let trimmed = trimmed.strip_prefix('+').unwrap_or(trimmed);

    // Split on decimal point
    let (int_str, frac_str) = match trimmed.split_once('.') {
        Some((i, f)) => (i, f),
        None => (trimmed, ""),
    };

    // Reject multiple decimal points (e.g. "1.2.3" would have been split into "1" and "2.3")
    if frac_str.contains('.') {
        return Err(format!("Invalid amount: '{}'", coins));
    }

    // Parse integer part
    let int_str = if int_str.is_empty() { "0" } else { int_str };
    let integer_part: u64 = int_str.parse().map_err(|e: std::num::ParseIntError| {
        if e.kind() == &std::num::IntErrorKind::PosOverflow {
            "Amount too large".to_string()
        } else {
            format!("Invalid amount: '{}'", coins)
        }
    })?;

    // Parse fractional part: pad with trailing zeros or truncate to 8 digits
    const DECIMALS: usize = 8; // UNITS_PER_DOLI = 10^8
    let frac_padded = if frac_str.len() >= DECIMALS {
        &frac_str[..DECIMALS]
    } else {
        // Pad with trailing zeros
        // (we build a new string only if needed)
        &format!("{:0<width$}", frac_str, width = DECIMALS)
    };

    // Validate that fractional digits are all numeric
    let fractional_part: u64 = frac_padded
        .parse()
        .map_err(|_| format!("Invalid amount: '{}'", coins))?;

    // Combine: total = integer * UNITS_PER_DOLI + fractional
    let integer_units = integer_part
        .checked_mul(UNITS_PER_DOLI)
        .ok_or_else(|| "Amount too large".to_string())?;
    let total = integer_units
        .checked_add(fractional_part)
        .ok_or_else(|| "Amount too large".to_string())?;

    Ok(total)
}

/// Format base units as a DOLI string with 8 decimal places.
pub fn format_balance(units: u64) -> String {
    let coins = units_to_coins(units);
    format!("{:.8} DOLI", coins)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Requirement: GUI-FR-010 (Must)
    // Acceptance: Amounts in DOLI with 8 decimal places (1 DOLI = 100,000,000 base units)
    // ========================================================================

    #[test]
    fn test_fr010_units_to_coins_one_doli() {
        assert_eq!(units_to_coins(100_000_000), 1.0);
    }

    #[test]
    fn test_fr010_units_to_coins_fractional() {
        assert_eq!(units_to_coins(12_345_678), 0.12345678);
    }

    #[test]
    fn test_fr010_units_to_coins_zero() {
        assert_eq!(units_to_coins(0), 0.0);
    }

    #[test]
    fn test_fr010_units_to_coins_max_supply() {
        // Large value -- should not overflow
        let large_amount: u64 = 21_000_000 * UNITS_PER_DOLI; // 21M DOLI
        let result = units_to_coins(large_amount);
        assert!((result - 21_000_000.0).abs() < 0.01);
    }

    #[test]
    fn test_fr010_coins_to_units_valid() {
        assert_eq!(coins_to_units("1.0").unwrap(), 100_000_000);
        assert_eq!(coins_to_units("5").unwrap(), 500_000_000);
        assert_eq!(coins_to_units("0.12345678").unwrap(), 12_345_678);
    }

    #[test]
    fn test_fr010_coins_to_units_negative_rejected() {
        let result = coins_to_units("-1.0");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("negative"));
    }

    #[test]
    fn test_fr010_coins_to_units_invalid_string() {
        assert!(coins_to_units("abc").is_err());
        assert!(coins_to_units("").is_err());
        assert!(coins_to_units("1.2.3").is_err());
    }

    #[test]
    fn test_fr010_coins_to_units_overflow() {
        let result = coins_to_units("999999999999999999999");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too large"));
    }

    #[test]
    fn test_fr010_coins_to_units_zero() {
        assert_eq!(coins_to_units("0").unwrap(), 0);
        assert_eq!(coins_to_units("0.0").unwrap(), 0);
    }

    #[test]
    fn test_fr010_coins_to_units_whitespace() {
        assert_eq!(coins_to_units("  1.0  ").unwrap(), 100_000_000);
    }

    #[test]
    fn test_fr010_format_balance_one_doli() {
        assert_eq!(format_balance(100_000_000), "1.00000000 DOLI");
    }

    #[test]
    fn test_fr010_format_balance_fractional() {
        assert_eq!(format_balance(12_345_678), "0.12345678 DOLI");
    }

    #[test]
    fn test_fr010_format_balance_zero() {
        assert_eq!(format_balance(0), "0.00000000 DOLI");
    }

    // ========================================================================
    // Requirement: GUI-FR-020, GUI-FR-024, GUI-FR-025 (Must)
    // Acceptance: Bond cost calculations use correct constants
    // ========================================================================

    #[test]
    fn test_fr020_bond_unit_is_10_doli() {
        assert_eq!(BOND_UNIT, 10 * UNITS_PER_DOLI);
    }

    #[test]
    fn test_fr024_max_bonds_per_producer() {
        assert_eq!(MAX_BONDS_PER_PRODUCER, 3_000);
    }

    #[test]
    fn test_fr020_base_registration_fee() {
        // 0.001 DOLI = 100,000 base units
        assert_eq!(BASE_REGISTRATION_FEE, 100_000);
        assert!((units_to_coins(BASE_REGISTRATION_FEE) - 0.001).abs() < 1e-10);
    }

    // ========================================================================
    // Type deserialization tests (verifying compatibility with node responses)
    // ========================================================================

    #[test]
    fn test_balance_deserialization() {
        let json =
            r#"{"confirmed":100000000,"unconfirmed":50000000,"immature":0,"total":150000000}"#;
        let balance: Balance = serde_json::from_str(json).unwrap();
        assert_eq!(balance.confirmed, 100_000_000);
        assert_eq!(balance.unconfirmed, 50_000_000);
        assert_eq!(balance.immature, 0);
        assert_eq!(balance.total, 150_000_000);
    }

    #[test]
    fn test_balance_immature_defaults_to_zero() {
        // Old nodes may not send immature field
        let json = r#"{"confirmed":100,"unconfirmed":50,"total":150}"#;
        let balance: Balance = serde_json::from_str(json).unwrap();
        assert_eq!(balance.immature, 0);
    }

    #[test]
    fn test_chain_info_deserialization() {
        let json = r#"{
            "network": "mainnet",
            "bestHash": "abcd1234",
            "bestHeight": 12345,
            "bestSlot": 67890,
            "genesisHash": "genesis0000"
        }"#;
        let info: ChainInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.network, "mainnet");
        assert_eq!(info.best_height, 12345);
    }

    #[test]
    fn test_producer_info_default_bond_count() {
        // Old node responses may not include bond_count
        let json = r#"{
            "publicKey": "abcd",
            "registrationHeight": 100,
            "bondAmount": 1000000000,
            "status": "active",
            "era": 5
        }"#;
        let info: ProducerInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.bond_count, 1);
    }

    #[test]
    fn test_history_entry_deserialization() {
        let json = r#"{
            "hash": "tx123",
            "txType": "Transfer",
            "blockHash": "block456",
            "height": 1000,
            "timestamp": 1700000000,
            "amountReceived": 500000000,
            "amountSent": 0,
            "fee": 1000,
            "confirmations": 10
        }"#;
        let entry: HistoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.hash, "tx123");
        assert_eq!(entry.amount_received, 500_000_000);
    }

    #[test]
    fn test_reward_epoch_deserialization() {
        let json = r#"{
            "epoch": 42,
            "estimatedReward": 100000000,
            "qualified": true,
            "claimed": false
        }"#;
        let epoch: RewardEpoch = serde_json::from_str(json).unwrap();
        assert_eq!(epoch.epoch, 42);
        assert!(epoch.qualified);
        assert!(!epoch.claimed);
    }
}
