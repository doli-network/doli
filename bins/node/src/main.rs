//! DOLI Node - Full node implementation
//!
//! This binary runs a full DOLI node that:
//! - Validates and stores blocks
//! - Maintains the UTXO set
//! - Participates in P2P network
//! - Optionally produces blocks (if registered as producer)
//! - Auto-updates with community veto (7 days, 33% threshold)

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use crypto::{KeyPair, PrivateKey};
use doli_core::Network;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

mod config;
mod metrics;
mod node;
mod updater;

use config::NodeConfig;
use updater::UpdateConfig;

#[derive(Parser)]
#[command(name = "doli-node")]
#[command(about = "DOLI full node", long_about = None)]
struct Cli {
    /// Network to connect to (mainnet, testnet, devnet)
    #[arg(short, long, default_value = "mainnet", global = true)]
    network: String,

    /// Configuration file path
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Data directory (overrides network default)
    #[arg(short, long)]
    data_dir: Option<PathBuf>,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the node
    Run {
        /// Enable block production
        #[arg(long)]
        producer: bool,

        /// Producer key file
        #[arg(long)]
        producer_key: Option<PathBuf>,

        /// Disable auto-updates
        #[arg(long)]
        no_auto_update: bool,

        /// Only notify about updates, don't apply
        #[arg(long)]
        update_notify_only: bool,

        /// P2P listen port (overrides network default)
        #[arg(long)]
        p2p_port: Option<u16>,

        /// RPC listen port (overrides network default)
        #[arg(long)]
        rpc_port: Option<u16>,

        /// Metrics server port (default: 9090)
        #[arg(long, default_value = "9090")]
        metrics_port: u16,

        /// Bootstrap node to connect to (e.g., /ip4/127.0.0.1/tcp/50303)
        #[arg(long)]
        bootstrap: Option<String>,

        /// Disable DHT discovery (only connect to explicitly provided bootstrap nodes)
        /// Use this to isolate test networks from external peers
        #[arg(long)]
        no_dht: bool,
    },

    /// Initialize a new data directory
    Init {
        /// Network (mainnet, testnet)
        #[arg(long, default_value = "mainnet")]
        network: String,
    },

    /// Show node status
    Status,

    /// Import blocks from file
    Import {
        /// Path to blocks file
        path: PathBuf,
    },

    /// Export blocks to file
    Export {
        /// Path to output file
        path: PathBuf,

        /// Start height
        #[arg(long, default_value = "0")]
        from: u64,

        /// End height (default: latest)
        #[arg(long)]
        to: Option<u64>,
    },

    /// Update management commands
    Update {
        #[command(subcommand)]
        action: UpdateCommands,
    },
}

#[derive(Subcommand)]
enum UpdateCommands {
    /// Check for available updates
    Check,

    /// Show pending update status
    Status,

    /// Vote on a pending update (requires producer key)
    Vote {
        /// Vote to veto the update
        #[arg(long)]
        veto: bool,

        /// Vote to approve the update
        #[arg(long)]
        approve: bool,

        /// Path to producer key file
        #[arg(long)]
        key: PathBuf,
    },
}

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
        _ => Level::INFO,
    };

    let subscriber = FmtSubscriber::builder().with_max_level(level).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Parse network
    let network = Network::from_str(&cli.network).map_err(|e| anyhow::anyhow!("{}", e))?;

    info!("DOLI Node v{}", env!("CARGO_PKG_VERSION"));
    info!("Network: {} (id={})", network.name(), network.id());

    // Get data directory (use override or network default)
    let data_dir = cli.data_dir.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".doli")
            .join(network.data_dir_name())
    });

    match cli.command {
        Some(Commands::Run {
            producer,
            producer_key,
            no_auto_update,
            update_notify_only,
            p2p_port,
            rpc_port,
            metrics_port,
            bootstrap,
            no_dht,
        }) => {
            let update_config = UpdateConfig {
                enabled: !no_auto_update,
                notify_only: update_notify_only,
                ..Default::default()
            };
            run_node(
                network,
                &data_dir,
                producer,
                producer_key,
                update_config,
                p2p_port,
                rpc_port,
                metrics_port,
                bootstrap,
                no_dht,
            )
            .await?;
        }
        Some(Commands::Init { network: net_str }) => {
            let init_network = Network::from_str(&net_str).map_err(|e| anyhow::anyhow!("{}", e))?;
            init_data_dir(&data_dir, init_network)?;
        }
        Some(Commands::Status) => {
            show_status(&data_dir)?;
        }
        Some(Commands::Import { path }) => {
            import_blocks(&data_dir, &path)?;
        }
        Some(Commands::Export { path, from, to }) => {
            export_blocks(&data_dir, &path, from, to)?;
        }
        Some(Commands::Update { action }) => {
            handle_update_command(action).await?;
        }
        None => {
            // Default: run the node with auto-updates enabled
            run_node(
                network,
                &data_dir,
                false,
                None,
                UpdateConfig::default(),
                None,
                None,
                9090,
                None,
                false,
            )
            .await?;
        }
    }

    Ok(())
}

async fn run_node(
    network: Network,
    data_dir: &PathBuf,
    producer: bool,
    producer_key_path: Option<PathBuf>,
    update_config: UpdateConfig,
    p2p_port: Option<u16>,
    rpc_port: Option<u16>,
    metrics_port: u16,
    bootstrap: Option<String>,
    no_dht: bool,
) -> Result<()> {
    info!("Starting node with data directory: {:?}", data_dir);

    // Expand tilde in path
    let data_dir = if data_dir.starts_with("~") {
        if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(data_dir.strip_prefix("~").unwrap())
        } else {
            data_dir.clone()
        }
    } else {
        data_dir.clone()
    };

    // Ensure data directory exists
    std::fs::create_dir_all(&data_dir)?;

    // Load producer key if production is enabled
    let producer_key = if producer {
        let key_path = producer_key_path
            .ok_or_else(|| anyhow!("--producer-key required when --producer is enabled"))?;

        info!("Loading producer key from {:?}", key_path);
        let key = load_producer_key(&key_path)?;
        info!(
            "Producer key loaded: {}",
            crypto::hash::hash(key.public_key().as_bytes())
        );
        Some(key)
    } else {
        None
    };

    // Load or create configuration for the network
    let mut config = NodeConfig::for_network(network);
    config.data_dir = data_dir.clone();

    // Override ports if specified
    if let Some(port) = p2p_port {
        config.listen_addr = format!("0.0.0.0:{}", port);
    }
    if let Some(port) = rpc_port {
        config.rpc.listen_addr = format!("127.0.0.1:{}", port);
    }
    if no_dht {
        config.no_dht = true;
        // Clear default bootstrap nodes - only use explicitly provided ones
        config.bootstrap_nodes.clear();
        info!("DHT discovery disabled - cleared default bootstrap nodes, only connecting to explicit bootstrap addresses");
    }
    // Add explicit bootstrap after clearing defaults (so it's preserved with --no-dht)
    if let Some(ref addr) = bootstrap {
        config.bootstrap_nodes.push(addr.clone());
    }

    // Start metrics server
    let metrics_addr: std::net::SocketAddr = format!("0.0.0.0:{}", metrics_port).parse()?;
    metrics::spawn_metrics_server(metrics_addr);
    info!("Metrics server started on {}", metrics_addr);

    // Start auto-update service
    if update_config.enabled {
        info!("Auto-updates enabled (7-day veto period, 33% threshold)");
        if update_config.notify_only {
            info!("Notify-only mode: updates will not be applied automatically");
        }
    } else {
        info!("Auto-updates disabled");
    }

    // Spawn update service with placeholder functions
    // In production, these would be connected to the actual producer registry
    let _vote_tx = updater::spawn_update_service(
        update_config,
        || 100,    // TODO: Connect to actual producer count
        |_| false, // TODO: Connect to actual producer registry
    );

    // Create and run the node
    let mut node = node::Node::new(config, producer_key).await?;

    info!("Node running. Press Ctrl+C to stop.");

    // Run node in a separate task
    let node_handle = tokio::spawn(async move {
        if let Err(e) = node.run().await {
            error!("Node error: {}", e);
        }
    });

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    info!("Shutting down...");

    // Cancel the node task
    node_handle.abort();

    Ok(())
}

/// Load a producer key from a JSON wallet file
fn load_producer_key(path: &PathBuf) -> Result<KeyPair> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| anyhow!("Failed to read key file: {}", e))?;

    // Try to parse as a wallet JSON (same format as doli-cli)
    #[derive(serde::Deserialize)]
    struct WalletAddress {
        private_key: String,
    }

    #[derive(serde::Deserialize)]
    struct Wallet {
        addresses: Vec<WalletAddress>,
    }

    let wallet: Wallet =
        serde_json::from_str(&contents).map_err(|e| anyhow!("Failed to parse key file: {}", e))?;

    if wallet.addresses.is_empty() {
        return Err(anyhow!("Wallet file contains no addresses"));
    }

    let private_key = PrivateKey::from_hex(&wallet.addresses[0].private_key)
        .map_err(|e| anyhow!("Invalid private key: {}", e))?;

    Ok(KeyPair::from_private_key(private_key))
}

async fn handle_update_command(action: UpdateCommands) -> Result<()> {
    match action {
        UpdateCommands::Check => {
            info!("Checking for updates...");
            match updater::fetch_latest_release(None).await {
                Ok(Some(release)) => {
                    let current = updater::current_version();
                    if updater::is_newer_version(&release.version, current) {
                        println!("Update available: {} -> {}", current, release.version);
                        println!("Changelog: {}", release.changelog);
                    } else {
                        println!("Already on latest version ({})", current);
                    }
                }
                Ok(None) => {
                    println!("Could not fetch release information");
                }
                Err(e) => {
                    error!("Failed to check for updates: {}", e);
                }
            }
        }
        UpdateCommands::Status => {
            // This would need access to node state, simplified for now
            println!("Current version: {}", updater::current_version());
            println!("Use 'doli-node run' to see pending update status");
        }
        UpdateCommands::Vote { veto, approve, key } => {
            if veto == approve {
                error!("Specify either --veto or --approve, not both or neither");
                return Ok(());
            }
            let vote = if veto {
                updater::Vote::Veto
            } else {
                updater::Vote::Approve
            };
            info!(
                "Vote {} with key {:?}",
                if veto { "VETO" } else { "APPROVE" },
                key
            );
            // TODO: Load key, sign vote, broadcast
            println!("Vote command not fully implemented yet");
        }
    }
    Ok(())
}

fn init_data_dir(data_dir: &PathBuf, network: Network) -> Result<()> {
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

fn show_status(data_dir: &PathBuf) -> Result<()> {
    info!("Showing status for: {:?}", data_dir);

    // TODO: Load chain state and show status
    println!("Status: Not implemented yet");

    Ok(())
}

fn import_blocks(data_dir: &PathBuf, path: &PathBuf) -> Result<()> {
    info!("Importing blocks from {:?} to {:?}", path, data_dir);

    // TODO: Import blocks
    println!("Import: Not implemented yet");

    Ok(())
}

fn export_blocks(data_dir: &PathBuf, path: &PathBuf, from: u64, to: Option<u64>) -> Result<()> {
    info!(
        "Exporting blocks from {:?} ({}..{:?}) to {:?}",
        data_dir, from, to, path
    );

    // TODO: Export blocks
    println!("Export: Not implemented yet");

    Ok(())
}
