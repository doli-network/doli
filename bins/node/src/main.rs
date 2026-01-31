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
use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use crypto::{KeyPair, PrivateKey, PublicKey};
use doli_core::Network;
use storage::ProducerSet;
use tokio::sync::RwLock;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

mod config;
mod metrics;
mod node;
mod producer;
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

        /// DANGEROUS: Skip duplicate key detection during producer startup.
        /// Only use if you are CERTAIN no other instance is running with this key.
        /// Using this incorrectly WILL cause slashing (100% bond loss).
        #[arg(long)]
        force_start: bool,

        /// Path to chainspec JSON file (overrides built-in network config)
        /// Use scripts/generate_chainspec.sh to create from wallet files
        #[arg(long)]
        chainspec: Option<PathBuf>,
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

    /// Apply a pending approved update
    Apply {
        /// Force apply even if not in enforcement period
        #[arg(long)]
        force: bool,
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
            force_start,
            chainspec,
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
                force_start,
                chainspec,
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
            handle_update_command(action, &data_dir).await?;
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
                false,
                None, // chainspec
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
    force_start: bool,
    chainspec_path: Option<PathBuf>,
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
    let (producer_key, producer_guard, signed_slots_db) = if producer {
        let key_path = producer_key_path
            .ok_or_else(|| anyhow!("--producer-key required when --producer is enabled"))?;

        info!("Loading producer key from {:?}", key_path);
        let key = load_producer_key(&key_path)?;
        info!(
            "Producer key loaded: {}",
            crypto::hash::hash(key.public_key().as_bytes())
        );

        // PRODUCER SAFETY CHECKS
        // These protect against accidental slashing by:
        // 1. Preventing two instances on the same machine (lock file)
        // 2. Preventing double-signing after restart (signed slots db)
        // 3. Detecting if another node is using this key (duplicate detection)

        // If --force-start is specified, require interactive confirmation
        let force_start = if force_start {
            if !producer::confirm_force_start() {
                return Err(anyhow!("Force start cancelled by user"));
            }
            true
        } else {
            false
        };

        // Perform startup checks (lock file + signed slots db)
        // Note: Duplicate key detection happens after network connection
        // when we have access to recent blocks from peers
        match producer::startup_checks(&data_dir, key.public_key(), force_start, None).await {
            Ok((guard, db)) => {
                info!("Producer safety checks passed");
                (Some(key), Some(guard), Some(db))
            }
            Err(e) => {
                error!("Producer startup blocked: {}", e);
                return Err(anyhow!("{}", e));
            }
        }
    } else {
        (None, None, None)
    };

    // Keep guard alive for the duration of the process
    let _producer_guard = producer_guard;

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
        info!("Auto-updates enabled (7-day veto period, 40% threshold)");
        if update_config.notify_only {
            info!("Notify-only mode: updates will not be applied automatically");
        }
    } else {
        info!("Auto-updates disabled");
    }

    // Load producer set for update service (shared with Node)
    let producers_path = data_dir.join("producers.bin");
    let producer_set = if producers_path.exists() {
        match ProducerSet::load(&producers_path) {
            Ok(set) => set,
            Err(e) => {
                info!("Could not load producer set: {}, starting fresh", e);
                ProducerSet::new()
            }
        }
    } else {
        // Initialize with genesis producers from chainspec (preferred) or built-in defaults
        use doli_core::chainspec::ChainSpec;

        // Try to load chainspec from provided path or default location
        let chainspec = if let Some(ref path) = chainspec_path {
            info!("Loading chainspec from {:?}", path);
            match ChainSpec::load(path) {
                Ok(spec) => {
                    info!("Loaded chainspec: {} ({})", spec.name, spec.id);
                    Some(spec)
                }
                Err(e) => {
                    error!("Failed to load chainspec: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            // Try default chainspec location
            let default_path = data_dir.join("chainspec.json");
            if default_path.exists() {
                match ChainSpec::load(&default_path) {
                    Ok(spec) => {
                        info!(
                            "Loaded chainspec from {:?}: {} ({})",
                            default_path, spec.name, spec.id
                        );
                        Some(spec)
                    }
                    Err(e) => {
                        warn!("Could not load default chainspec: {}", e);
                        None
                    }
                }
            } else {
                None
            }
        };

        // Get genesis producers from chainspec or fall back to built-in (legacy)
        if let Some(spec) = chainspec {
            let genesis_producers = spec.get_genesis_producers();
            if !genesis_producers.is_empty() {
                info!(
                    "Initializing {} with {} genesis producers from chainspec",
                    network.name(),
                    genesis_producers.len()
                );
                for (pk, bonds) in &genesis_producers {
                    info!("  - {} (bonds: {})", pk.to_hex(), bonds);
                }
                ProducerSet::with_genesis_producers(genesis_producers)
            } else {
                info!("Chainspec has no genesis producers, using bootstrap mode");
                ProducerSet::new()
            }
        } else {
            // Legacy fallback: use hardcoded genesis producers
            // This path is deprecated - prefer chainspec files
            match network {
                Network::Mainnet => {
                    warn!("No chainspec provided for mainnet - using bootstrap mode");
                    warn!("For production, use --chainspec with a validated chainspec file");
                    ProducerSet::new()
                }
                Network::Testnet => {
                    // Legacy testnet support - fall back to hardcoded producers
                    use doli_core::genesis::testnet_genesis_producers;
                    let genesis_producers = testnet_genesis_producers();
                    if !genesis_producers.is_empty() {
                        info!(
                            "Initializing testnet with {} genesis producers (legacy mode)",
                            genesis_producers.len()
                        );
                        ProducerSet::with_genesis_producers(genesis_producers)
                    } else {
                        ProducerSet::new()
                    }
                }
                Network::Devnet => ProducerSet::new(),
            }
        }
    };
    let producer_set = Arc::new(RwLock::new(producer_set));

    // Create closures for update service with actual producer registry
    let producers_for_count = producer_set.clone();
    let producer_count_fn = move || {
        // Use try_read to avoid blocking, fall back to 0 if locked
        producers_for_count
            .try_read()
            .map(|set| set.active_count())
            .unwrap_or(0)
    };

    let producers_for_check = producer_set.clone();
    let is_producer_fn = move |pubkey_hex: &str| {
        if let Ok(pubkey) = PublicKey::from_hex(pubkey_hex) {
            producers_for_check
                .try_read()
                .map(|set| {
                    set.get_by_pubkey(&pubkey)
                        .map(|p| p.is_active())
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        } else {
            false
        }
    };

    // Spawn update service with real producer registry
    let _vote_tx = updater::spawn_update_service(
        update_config,
        data_dir.clone(),
        producer_count_fn,
        is_producer_fn,
    );

    // Create shared shutdown flag for graceful shutdown signaling
    let shutdown_flag = Arc::new(RwLock::new(false));
    let shutdown_flag_for_node = shutdown_flag.clone();

    // Create and run the node (share the producer_set, signed_slots_db, and shutdown flag)
    let mut node = node::Node::new(
        config,
        producer_key,
        Some(producer_set),
        signed_slots_db,
        Some(shutdown_flag_for_node),
    )
    .await?;

    info!("Node running. Press Ctrl+C to stop.");

    // Run node in a separate task
    let node_handle = tokio::spawn(async move {
        if let Err(e) = node.run().await {
            error!("Node error: {}", e);
        }
    });

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    info!("Shutting down gracefully...");

    // Signal the node to shutdown (instead of aborting)
    *shutdown_flag.write().await = true;

    // Wait for node to complete graceful shutdown (with timeout)
    match tokio::time::timeout(std::time::Duration::from_secs(30), node_handle).await {
        Ok(Ok(())) => {
            info!("Node shutdown complete");
        }
        Ok(Err(e)) => {
            error!("Node task panicked: {}", e);
        }
        Err(_) => {
            warn!("Shutdown timeout - forcing exit");
        }
    }

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

async fn handle_update_command(action: UpdateCommands, data_dir: &PathBuf) -> Result<()> {
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
            // Show status from disk-persisted pending update
            updater::cli::show_status_from_disk(data_dir);
        }
        UpdateCommands::Vote { veto, approve, key } => {
            if veto == approve {
                error!("Specify either --veto or --approve, not both or neither");
                return Ok(());
            }

            let vote_type = if veto {
                updater::Vote::Veto
            } else {
                updater::Vote::Approve
            };

            // Load producer key
            let keypair = load_producer_key(&key)?;
            let pubkey_hex = keypair.public_key().to_hex();
            info!(
                "Voting {} with producer key: {}",
                if veto { "VETO" } else { "APPROVE" },
                &pubkey_hex[..16]
            );

            // Auto-detect version from pending update on disk
            let version = std::env::var("DOLI_VOTE_VERSION")
                .ok()
                .or_else(|| updater::get_pending_version(data_dir));

            let version = match version {
                Some(v) => v,
                None => {
                    println!("No pending update found.");
                    println!("Run 'doli-node update status' to check for pending updates.");
                    println!();
                    println!("If you want to vote on a specific version, set DOLI_VOTE_VERSION:");
                    println!("  DOLI_VOTE_VERSION=1.2.0 doli-node update vote --veto --key producer.json");
                    return Ok(());
                }
            };

            println!("Voting on update: v{}", version);

            let mut vote_msg = updater::VoteMessage::new(version.clone(), vote_type, pubkey_hex);

            // Sign the vote
            let message_bytes = vote_msg.message_bytes();
            let signature = crypto::signature::sign(&message_bytes, keypair.private_key());
            vote_msg.signature = signature.to_hex();

            // Serialize vote message
            let vote_json = serde_json::to_string_pretty(&vote_msg)?;
            println!();
            println!("Signed vote message:");
            println!("{}", vote_json);
            println!();
            println!("To submit this vote, send it to a running node via RPC:");
            println!("curl -X POST http://localhost:28545 \\");
            println!("  -H 'Content-Type: application/json' \\");
            println!("  -d '{{\"jsonrpc\":\"2.0\",\"method\":\"submitVote\",\"params\":{{\"vote\":{}}},\"id\":1}}'",
                     serde_json::to_string(&vote_msg)?);
        }
        UpdateCommands::Apply { force } => {
            // Get pending update
            let pending = match updater::get_pending_update(data_dir) {
                Some(p) => p,
                None => {
                    println!("No pending update to apply.");
                    println!("Run 'doli-node update check' to check for available updates.");
                    return Ok(());
                }
            };

            // Check if update is approved
            if !pending.approved && !force {
                println!("Update v{} is not yet approved.", pending.release.version);
                println!("Veto period: {} days remaining.", pending.days_remaining());
                println!();
                println!("Use --force to apply anyway (not recommended).");
                return Ok(());
            }

            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    Applying Update                               ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");
            println!(
                "║  Current version: {}                                            ║",
                updater::current_version()
            );
            println!(
                "║  Target version:  {}                                            ║",
                pending.release.version
            );
            println!("╚══════════════════════════════════════════════════════════════════╝");
            println!();

            println!("Downloading {}...", pending.release.version);

            // --force bypasses approval check but NEVER bypasses veto period check
            // The veto period check is a critical security measure to prevent
            // producers from applying potentially malicious updates before
            // the community has had time to review.
            let approved_or_forced = pending.approved || force;

            match updater::apply_update(&pending.release, approved_or_forced, None).await {
                Ok(()) => {
                    println!();
                    println!(
                        "╔══════════════════════════════════════════════════════════════════╗"
                    );
                    println!(
                        "║                    ✅ UPDATE SUCCESSFUL                          ║"
                    );
                    println!(
                        "╠══════════════════════════════════════════════════════════════════╣"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Version: {} → {}                                            ║",
                        updater::current_version(),
                        pending.release.version
                    );
                    println!(
                        "║  Status: Production enabled                                      ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Your node is now up to date.                                    ║"
                    );
                    println!(
                        "║  Restarting node to apply changes...                             ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                    println!();

                    // Remove pending update file
                    if let Err(e) = updater::PendingUpdate::remove(data_dir) {
                        error!("Failed to remove pending_update.json: {}", e);
                    }

                    // Restart node
                    updater::restart_node();
                }
                Err(e) => {
                    error!("Failed to apply update: {}", e);
                    println!();
                    println!("Update failed. Please try again or update manually.");
                }
            }
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
