//! Chainspec-based parameter defaults
//!
//! Applies consensus parameters from a chainspec JSON file as lowest-priority
//! environment variable defaults. Skipped entirely for mainnet (defense-in-depth).

use std::path::Path;

use tracing::{debug, info, warn};

use crate::Network;

/// Apply chainspec consensus parameters as environment variable defaults.
///
/// Reads a chainspec JSON file and sets environment variables for consensus
/// parameters that are not already set. This makes the chainspec the lowest
/// priority source (below parent env and .env file) but above hardcoded defaults.
///
/// Priority hierarchy: Parent ENV > .env file > Chainspec > consensus.rs defaults
///
/// Skipped entirely for mainnet (defense-in-depth: mainnet params are locked).
pub fn apply_chainspec_defaults(chainspec_path: &Path) {
    use crate::chainspec::ChainSpec;

    let spec = match ChainSpec::load(chainspec_path) {
        Ok(spec) => spec,
        Err(e) => {
            warn!(
                "Could not load chainspec from {:?} for defaults: {}",
                chainspec_path, e
            );
            return;
        }
    };

    // Defense-in-depth: never override mainnet params from chainspec
    if matches!(spec.network, Network::Mainnet) {
        debug!("Skipping chainspec defaults for mainnet (locked parameters)");
        return;
    }

    let mut applied = Vec::new();

    if set_env_if_absent(
        "DOLI_SLOT_DURATION",
        &spec.consensus.slot_duration.to_string(),
    ) {
        applied.push(format!(
            "DOLI_SLOT_DURATION={}",
            spec.consensus.slot_duration
        ));
    }
    if set_env_if_absent("DOLI_BOND_UNIT", &spec.consensus.bond_amount.to_string()) {
        applied.push(format!("DOLI_BOND_UNIT={}", spec.consensus.bond_amount));
    }
    if set_env_if_absent(
        "DOLI_SLOTS_PER_REWARD_EPOCH",
        &spec.consensus.slots_per_epoch.to_string(),
    ) {
        applied.push(format!(
            "DOLI_SLOTS_PER_REWARD_EPOCH={}",
            spec.consensus.slots_per_epoch
        ));
    }
    if set_env_if_absent(
        "DOLI_INITIAL_REWARD",
        &spec.genesis.initial_reward.to_string(),
    ) {
        applied.push(format!(
            "DOLI_INITIAL_REWARD={}",
            spec.genesis.initial_reward
        ));
    }
    if spec.genesis.timestamp != 0
        && set_env_if_absent("DOLI_GENESIS_TIME", &spec.genesis.timestamp.to_string())
    {
        applied.push(format!("DOLI_GENESIS_TIME={}", spec.genesis.timestamp));
    }

    if applied.is_empty() {
        debug!("Chainspec defaults: all vars already set, nothing applied");
    } else {
        info!(
            "Applied {} chainspec defaults: {}",
            applied.len(),
            applied.join(", ")
        );
    }
}

/// Set an environment variable only if it is not already set.
/// Returns true if the variable was set (i.e., it was absent).
fn set_env_if_absent(key: &str, value: &str) -> bool {
    if std::env::var(key).is_err() {
        std::env::set_var(key, value);
        true
    } else {
        false
    }
}
