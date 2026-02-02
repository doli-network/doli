//! Configuration validation for network parameters
//!
//! This module validates that critical network parameters are not overridden
//! for mainnet. Security-critical parameters must remain at their hardcoded
//! values to ensure consensus compatibility.

use tracing::warn;

use crate::network_params::NetworkParams;
use crate::Network;

/// Parameters that are locked for mainnet and cannot be overridden
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockedParam {
    SlotDuration,
    GenesisTime,
    BondUnit,
    InitialReward,
    UnbondingPeriod,
    VdfIterations,
    HeartbeatVdfIterations,
    VdfRegisterIterations,
    BlocksPerYear,
    BlocksPerRewardEpoch,
    CoinbaseMaturity,
    AutomaticGenesisBond,
    GenesisBlocks,
}

impl LockedParam {
    /// Get the environment variable name for this parameter
    pub fn env_var(&self) -> &'static str {
        match self {
            LockedParam::SlotDuration => "DOLI_SLOT_DURATION",
            LockedParam::GenesisTime => "DOLI_GENESIS_TIME",
            LockedParam::BondUnit => "DOLI_BOND_UNIT",
            LockedParam::InitialReward => "DOLI_INITIAL_REWARD",
            LockedParam::UnbondingPeriod => "DOLI_UNBONDING_PERIOD",
            LockedParam::VdfIterations => "DOLI_VDF_ITERATIONS",
            LockedParam::HeartbeatVdfIterations => "DOLI_HEARTBEAT_VDF_ITERATIONS",
            LockedParam::VdfRegisterIterations => "DOLI_VDF_REGISTER_ITERATIONS",
            LockedParam::BlocksPerYear => "DOLI_BLOCKS_PER_YEAR",
            LockedParam::BlocksPerRewardEpoch => "DOLI_BLOCKS_PER_REWARD_EPOCH",
            LockedParam::CoinbaseMaturity => "DOLI_COINBASE_MATURITY",
            LockedParam::AutomaticGenesisBond => "DOLI_AUTOMATIC_GENESIS_BOND",
            LockedParam::GenesisBlocks => "DOLI_GENESIS_BLOCKS",
        }
    }
}

/// All locked parameters for mainnet
pub const MAINNET_LOCKED_PARAMS: &[LockedParam] = &[
    LockedParam::SlotDuration,
    LockedParam::GenesisTime,
    LockedParam::BondUnit,
    LockedParam::InitialReward,
    LockedParam::UnbondingPeriod,
    LockedParam::VdfIterations,
    LockedParam::HeartbeatVdfIterations,
    LockedParam::VdfRegisterIterations,
    LockedParam::BlocksPerYear,
    LockedParam::BlocksPerRewardEpoch,
    LockedParam::CoinbaseMaturity,
    LockedParam::AutomaticGenesisBond,
    LockedParam::GenesisBlocks,
];

/// Check if any locked parameters have been attempted to be overridden
///
/// This function logs warnings for any locked parameters that have
/// environment variables set. The values are NOT applied (they are
/// ignored by NetworkParams for mainnet), but users should be warned.
///
/// # Arguments
///
/// * `network` - The network being configured
///
/// # Returns
///
/// A list of locked parameters that have been attempted to be overridden
pub fn check_locked_params(network: Network) -> Vec<LockedParam> {
    if !matches!(network, Network::Mainnet) {
        return vec![];
    }

    let mut overridden = vec![];

    for param in MAINNET_LOCKED_PARAMS {
        if std::env::var(param.env_var()).is_ok() {
            warn!(
                "Ignoring {} for mainnet - this parameter is locked for security",
                param.env_var()
            );
            overridden.push(*param);
        }
    }

    if !overridden.is_empty() {
        warn!(
            "Note: {} locked parameter(s) were set via environment but ignored for mainnet",
            overridden.len()
        );
    }

    overridden
}

/// Validate that loaded parameters are within acceptable ranges
///
/// This function checks that parameters are reasonable (e.g., slot duration > 0).
/// Returns a list of validation errors.
pub fn validate_params(params: &NetworkParams, network: Network) -> Vec<String> {
    let mut errors = vec![];

    // Slot duration must be positive
    if params.slot_duration == 0 {
        errors.push("slot_duration must be > 0".to_string());
    }

    // Ports must be valid
    if params.default_p2p_port == 0 {
        errors.push("default_p2p_port must be > 0".to_string());
    }
    if params.default_rpc_port == 0 {
        errors.push("default_rpc_port must be > 0".to_string());
    }

    // Economic parameters must be positive
    if params.bond_unit == 0 {
        errors.push("bond_unit must be > 0".to_string());
    }
    if params.initial_reward == 0 && !matches!(network, Network::Devnet) {
        errors.push("initial_reward must be > 0 for non-devnet".to_string());
    }

    // Time structure must be consistent
    if params.blocks_per_year == 0 {
        errors.push("blocks_per_year must be > 0".to_string());
    }
    if params.blocks_per_reward_epoch == 0 {
        errors.push("blocks_per_reward_epoch must be > 0".to_string());
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locked_param_env_vars() {
        // Verify all locked params have env var names
        for param in MAINNET_LOCKED_PARAMS {
            let env_var = param.env_var();
            assert!(env_var.starts_with("DOLI_"));
        }
    }

    #[test]
    fn test_check_locked_params_devnet() {
        // For devnet, no params should be reported as locked
        let overridden = check_locked_params(Network::Devnet);
        assert!(overridden.is_empty());
    }

    #[test]
    fn test_validate_params_defaults() {
        // Default params should pass validation
        let mainnet = NetworkParams::defaults(Network::Mainnet);
        let errors = validate_params(&mainnet, Network::Mainnet);
        assert!(errors.is_empty(), "Mainnet defaults should be valid: {:?}", errors);

        let devnet = NetworkParams::defaults(Network::Devnet);
        let errors = validate_params(&devnet, Network::Devnet);
        assert!(errors.is_empty(), "Devnet defaults should be valid: {:?}", errors);
    }
}
