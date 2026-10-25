//! The chainspec a node of a network runs on: used by `run` and the replay node.

use std::path::Path;

use doli_core::chainspec::ChainSpec;
use doli_core::Network;
use storage::ProducerSet;
use tracing::{info, warn};

const EMBEDDED_MAINNET_CHAINSPEC: &str = include_str!("../../../chainspec.mainnet.json");
const EMBEDDED_TESTNET_CHAINSPEC: &str = include_str!("../../../chainspec.testnet.json");

/// Mainnet and testnet: the embedded chainspec (`explicit` and disk files are
/// ignored). Otherwise `explicit`, else `<data_dir>/chainspec.json` when it
/// exists (a file that does not load gives `None`). `Err` is fatal.
pub(crate) fn network_chainspec(
    network: Network,
    explicit: Option<&Path>,
    data_dir: &Path,
) -> Result<Option<ChainSpec>, String> {
    let embedded = match network {
        Network::Mainnet => Some(("Mainnet", "mainnet", EMBEDDED_MAINNET_CHAINSPEC)),
        Network::Testnet => Some(("Testnet", "testnet", EMBEDDED_TESTNET_CHAINSPEC)),
        Network::Devnet => None,
    };
    if let Some((title, name, json)) = embedded {
        let spec = serde_json::from_str::<ChainSpec>(json)
            .map_err(|e| format!("BUG: embedded {name} chainspec is invalid: {e}"))?;
        info!(
            "{title} chainspec: embedded (genesis={}, slot={}s) — disk/CLI overrides disabled",
            spec.genesis.timestamp, spec.consensus.slot_duration
        );
        if explicit.is_some() {
            warn!("SECURITY: --chainspec flag ignored for {name} — embedded chainspec is authoritative");
        }
        let disk_path = data_dir.join("chainspec.json");
        if network == Network::Mainnet && disk_path.exists() {
            warn!(
                "SECURITY: Ignoring {:?} — mainnet uses embedded chainspec only",
                disk_path
            );
        }
        return Ok(Some(spec));
    }
    if let Some(path) = explicit {
        info!("Loading chainspec from {:?}", path);
        let spec = ChainSpec::load(path).map_err(|e| format!("Failed to load chainspec: {e}"))?;
        info!("Loaded chainspec: {} ({})", spec.name, spec.id);
        return Ok(Some(spec));
    }
    let default_path = data_dir.join("chainspec.json");
    if !default_path.exists() {
        return Ok(None);
    }
    match ChainSpec::load(&default_path) {
        Ok(spec) => {
            info!(
                "Loaded chainspec from {:?}: {} ({})",
                default_path, spec.name, spec.id
            );
            Ok(Some(spec))
        }
        Err(e) => {
            warn!("Could not load default chainspec: {}", e);
            Ok(None)
        }
    }
}

/// The producer set a node seeds on a fresh start: the chainspec's genesis
/// producers, else the mainnet/testnet constants, else none.
pub(crate) fn genesis_producer_set(network: Network, chainspec: Option<&ChainSpec>) -> ProducerSet {
    let from_spec = chainspec.map_or_else(Vec::new, |spec| spec.get_genesis_producers());
    if !from_spec.is_empty() {
        info!(
            "Initializing {} with {} genesis producers from chainspec",
            network.name(),
            from_spec.len()
        );
        for (pk, bonds) in &from_spec {
            info!("  - {} (bonds: {})", pk.to_hex(), bonds);
        }
        return ProducerSet::with_genesis_producers(from_spec, network.bond_unit());
    }
    let producers = match network {
        Network::Mainnet => doli_core::genesis::mainnet_genesis_producers(),
        Network::Testnet => doli_core::genesis::testnet_genesis_producers(),
        Network::Devnet => Vec::new(),
    };
    let (name, count) = (network.name(), producers.len());
    match (chainspec.is_some(), network) {
        (true, Network::Devnet) => {
            info!("Chainspec has no genesis producers, using bootstrap mode")
        }
        (true, _) => {
            info!("Chainspec has no genesis producers, using {count} hardcoded {name} producers")
        }
        (false, Network::Devnet) => {}
        (false, _) => {
            info!("No chainspec provided, using {count} hardcoded {name} genesis producers")
        }
    }
    ProducerSet::with_genesis_producers(producers, network.bond_unit())
}
