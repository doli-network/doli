//! DOLI Node - Full node implementation
//!
//! This binary runs a full DOLI node that:
//! - Validates and stores blocks
//! - Maintains the UTXO set
//! - Participates in P2P network
//! - Optionally produces blocks (if registered as producer)
//! - Auto-updates with community veto (7 days, 40% threshold)

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::Result;
use clap::Parser;
use doli_core::{network_params, Network};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod cli;
mod commands;
mod config;
mod devnet;
mod keys;
mod metrics;
mod node;
mod operations;
mod producer;
mod run;
mod updater;

use cli::{expand_tilde_path, Cli, Commands};
use updater::UpdateConfig;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let level = match cli.log_level.as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        other => {
            anyhow::bail!(
                "Invalid log level '{}'. Valid options: trace, debug, info, warn, error",
                other
            );
        }
    };

    let subscriber = FmtSubscriber::builder().with_max_level(level).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Parse network (not used for devnet subcommands)
    let network = Network::from_str(&cli.network).map_err(|e| anyhow::anyhow!("{}", e))?;

    // Skip network logging for devnet subcommands (they manage their own environment)
    let is_devnet_command = matches!(cli.command, Some(Commands::Devnet { .. }));

    // Get data directory: --data-dir flag > DOLI_DATA_DIR env > platform default > legacy fallback
    let data_dir = if let Some(ref dir) = cli.data_dir {
        expand_tilde_path(dir)
    } else if let Ok(dir) = std::env::var("DOLI_DATA_DIR") {
        PathBuf::from(dir)
    } else {
        let net_name = network.data_dir_name();

        // Platform default
        #[cfg(target_os = "linux")]
        let platform_default = PathBuf::from("/var/lib/doli").join(net_name);
        #[cfg(target_os = "macos")]
        let platform_default = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("Library/Application Support/doli")
            .join(net_name);
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let platform_default = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".doli")
            .join(net_name);

        if platform_default.exists() {
            platform_default
        } else {
            // Legacy fallback
            let legacy = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".doli")
                .join(net_name);
            if legacy.exists() {
                eprintln!(
                    "Note: Using legacy data directory {}. Consider moving to {}.",
                    legacy.display(),
                    platform_default.display()
                );
                legacy
            } else {
                // Return platform default (will be created later)
                platform_default
            }
        }
    };

    // Load environment variables from data directory BEFORE using network params
    // This allows overriding network defaults via ~/.doli/{network}/.env
    if !is_devnet_command {
        network_params::load_env_for_network(network.name(), &data_dir);

        // Apply chainspec consensus defaults (lowest priority: only sets vars not already set)
        // Must happen BEFORE any code triggers NetworkParams OnceLock
        //
        // SECURITY: For mainnet, ALWAYS use the embedded chainspec compiled into the binary.
        // Never read from disk, never write to disk. A stale or tampered chainspec.json
        // on disk could change genesis_timestamp, causing slot schedule divergence and
        // chain forks. The embedded chainspec is the single source of truth.
        let chainspec_for_defaults: Option<std::path::PathBuf> =
            if matches!(network, Network::Mainnet | Network::Testnet) {
                // Mainnet/Testnet: skip disk — embedded chainspec applied later in run_node()
                None
            } else {
                match &cli.command {
                    Some(Commands::Run {
                        chainspec: Some(ref path),
                        ..
                    }) => Some(expand_tilde_path(path)),
                    Some(Commands::Run { .. }) => {
                        let default = data_dir.join("chainspec.json");
                        if default.exists() {
                            Some(default)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            };
        if let Some(ref path) = chainspec_for_defaults {
            network_params::apply_chainspec_defaults(path);
        }
    }

    info!("DOLI Node v{}", env!("CARGO_PKG_VERSION"));
    if !is_devnet_command {
        info!("Network: {} (id={})", network.name(), network.id());
    }

    match cli.command {
        Some(Commands::Run {
            producer,
            producer_key,
            no_auto_update,
            update_notify_only,
            no_auto_rollback,
            p2p_port,
            external_address,
            rpc_port,
            rpc_bind,
            metrics_port,
            bootstrap,
            bootnode_enr,
            no_discv5,
            discv5_port,
            no_dht,
            relay_server,
            no_snap_sync,
            force_start,
            yes,
            chainspec,
            auto_checkpoint,
            archive_to,
            checkpoint_height,
            checkpoint_hash,
        }) => {
            let update_config = UpdateConfig {
                enabled: !no_auto_update,
                notify_only: update_notify_only,
                auto_rollback: !no_auto_rollback,
                check_interval_secs: network.update_check_interval_secs(),
                veto_period_secs: network.veto_period_secs(),
                grace_period_secs: network.grace_period_secs(),
                custom_url: None,
            };
            run::run_node(
                network,
                &data_dir,
                producer,
                producer_key,
                update_config,
                p2p_port,
                external_address,
                rpc_port,
                rpc_bind,
                metrics_port,
                bootstrap,
                bootnode_enr,
                no_discv5,
                discv5_port,
                no_dht,
                relay_server,
                no_snap_sync,
                force_start,
                yes,
                chainspec,
                auto_checkpoint,
                archive_to,
                checkpoint_height,
                checkpoint_hash,
            )
            .await?;
        }
        Some(Commands::Init { network: net_str }) => {
            let init_network = Network::from_str(&net_str).map_err(|e| anyhow::anyhow!("{}", e))?;
            operations::init_data_dir(&data_dir, init_network)?;
        }
        Some(Commands::Status) => {
            operations::show_status(&data_dir)?;
        }
        Some(Commands::Import { path }) => {
            operations::import_blocks(&data_dir, &path)?;
        }
        Some(Commands::Export { path, from, to }) => {
            operations::export_blocks(&data_dir, &path, from, to)?;
        }
        Some(Commands::Update { action }) => {
            commands::handle_update_command(action, &data_dir, network).await?;
        }
        Some(Commands::Maintainer { action }) => {
            commands::handle_maintainer_command(action, &data_dir, network).await?;
        }
        Some(Commands::Truncate { blocks, yes }) => {
            operations::truncate_chain(network, &data_dir, blocks, yes)?;
        }
        Some(Commands::Recover { yes }) => {
            operations::recover_chain_state(network, &data_dir, yes)?;
        }
        Some(Commands::Restore {
            from,
            from_rpc,
            yes,
            backfill,
            skip_genesis_check,
        }) => {
            if let Some(rpc_url) = from_rpc {
                operations::restore_from_rpc(
                    network,
                    &data_dir,
                    &rpc_url,
                    backfill,
                    yes,
                    skip_genesis_check,
                )
                .await?;
            } else if let Some(path) = from {
                if backfill {
                    operations::backfill_from_archive(
                        network,
                        &data_dir,
                        &expand_tilde_path(&path),
                        yes,
                    )?;
                } else {
                    operations::restore_from_archive(
                        network,
                        &data_dir,
                        &expand_tilde_path(&path),
                        yes,
                    )?;
                }
            } else {
                anyhow::bail!("Either --from <PATH> or --from-rpc <URL> is required");
            }
        }
        Some(Commands::Reindex) => {
            operations::reindex_canonical_chain(&data_dir)?;
        }
        Some(Commands::Devnet { action }) => {
            commands::handle_devnet_command(action).await?;
        }
        Some(Commands::Release { action }) => {
            commands::handle_release_command(action).await?;
        }
        Some(Commands::Upgrade { version, yes }) => {
            commands::handle_upgrade_command(version, yes).await?;
        }
        Some(Commands::CheckpointInfo) => {
            use doli_core::consensus::{CHECKPOINT_HASH, CHECKPOINT_HEIGHT, CHECKPOINT_STATE_ROOT};
            println!("DOLI Checkpoint Info (compiled into binary)");
            println!("  Height:     {}", CHECKPOINT_HEIGHT);
            println!("  Hash:       {}", CHECKPOINT_HASH);
            println!("  State Root: {}", CHECKPOINT_STATE_ROOT);
        }
        None => {
            // Default: run the node with auto-updates enabled
            run::run_node(
                network,
                &data_dir,
                false,
                None,
                UpdateConfig {
                    check_interval_secs: network.update_check_interval_secs(),
                    veto_period_secs: network.veto_period_secs(),
                    grace_period_secs: network.grace_period_secs(),
                    ..Default::default()
                },
                None,
                None, // external_address
                None,
                None, // rpc_bind
                9000,
                vec![],
                vec![], // bootnode_enr
                false,  // no_discv5
                None,   // discv5_port
                false,
                false, // relay_server
                false, // no_snap_sync
                false,
                false, // yes
                None,  // chainspec
                None,  // auto_checkpoint
                None,  // archive_to
                None,  // checkpoint_height
                None,  // checkpoint_hash
            )
            .await?;
        }
    }

    Ok(())
}
