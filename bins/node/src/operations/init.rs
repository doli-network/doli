use std::path::PathBuf;

use anyhow::Result;
use doli_core::Network;
use tracing::info;

use crate::config::NodeConfig;

pub(crate) fn init_data_dir(data_dir: &PathBuf, network: Network) -> Result<()> {
    info!("Initializing data directory: {:?}", data_dir);
    info!("Network: {} (id={})", network.name(), network.id());

    // Create directories
    std::fs::create_dir_all(data_dir)?;
    std::fs::create_dir_all(data_dir.join("blocks"))?;
    std::fs::create_dir_all(data_dir.join("utxo"))?;

    // Create network-specific config
    let mut config = NodeConfig::for_network(network);
    config.data_dir = data_dir.clone();
    let config_path = data_dir.join("config.toml");

    // Write config file
    let config_toml = toml::to_string_pretty(&config)?;
    std::fs::write(&config_path, config_toml)?;

    info!("Configuration written to {:?}", config_path);
    info!("Data directory initialized");
    info!("  P2P port: {}", network.default_p2p_port());
    info!("  RPC port: {}", network.default_rpc_port());
    info!("  Address prefix: {}", network.address_prefix());

    Ok(())
}

pub(crate) fn show_status(data_dir: &PathBuf) -> Result<()> {
    info!("Showing status for: {:?}", data_dir);

    // TODO: Load chain state and show status
    println!("Status: Not implemented yet");

    Ok(())
}

pub(crate) fn import_blocks(data_dir: &PathBuf, path: &PathBuf) -> Result<()> {
    info!("Importing blocks from {:?} to {:?}", path, data_dir);

    // TODO: Import blocks
    println!("Import: Not implemented yet");

    Ok(())
}

pub(crate) fn export_blocks(
    data_dir: &PathBuf,
    path: &PathBuf,
    from: u64,
    to: Option<u64>,
) -> Result<()> {
    info!(
        "Exporting blocks from {:?} ({}..{:?}) to {:?}",
        data_dir, from, to, path
    );

    // TODO: Export blocks
    println!("Export: Not implemented yet");

    Ok(())
}
