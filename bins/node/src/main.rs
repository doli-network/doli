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
use doli_core::{network_params, Network};
use storage::ProducerSet;
use tokio::sync::RwLock;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

mod config;
mod devnet;
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

        /// Disable automatic rollback on update failures
        #[arg(long)]
        no_auto_rollback: bool,

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

        /// Skip all interactive confirmations (for automation/scripts).
        /// Implies acceptance of --force-start warning when used together.
        #[arg(long)]
        yes: bool,

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

    /// Maintainer management commands
    Maintainer {
        #[command(subcommand)]
        action: MaintainerCommands,
    },

    /// Recover chain state from existing block data
    ///
    /// Use this if nodes fail to load after ungraceful shutdown.
    /// Scans BlockStore for blocks and rebuilds UTXO set and chain state.
    Recover {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Local devnet management commands
    Devnet {
        #[command(subcommand)]
        action: DevnetCommands,
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

    /// View current vote status for an update
    Votes {
        /// Version to check votes for (optional, uses pending update if not specified)
        #[arg(long)]
        version: Option<String>,
    },

    /// Apply a pending approved update
    Apply {
        /// Force apply even if not in enforcement period
        #[arg(long)]
        force: bool,
    },

    /// Rollback to previous version backup
    Rollback,

    /// Verify release signatures
    Verify {
        /// Version to verify
        #[arg(long)]
        version: String,
    },
}

#[derive(Subcommand)]
enum MaintainerCommands {
    /// List current maintainer set
    List,

    /// Propose removing a maintainer (requires 3/5 multisig)
    Remove {
        /// Public key of maintainer to remove
        #[arg(long)]
        target: String,

        /// Path to maintainer key file for signing
        #[arg(long)]
        key: PathBuf,

        /// Reason for removal (optional)
        #[arg(long)]
        reason: Option<String>,
    },

    /// Propose adding a new maintainer (requires 3/5 multisig)
    Add {
        /// Public key of producer to add as maintainer
        #[arg(long)]
        target: String,

        /// Path to maintainer key file for signing
        #[arg(long)]
        key: PathBuf,
    },

    /// Sign a pending maintainer change proposal
    Sign {
        /// Proposal ID to sign
        #[arg(long)]
        proposal_id: String,

        /// Path to maintainer key file for signing
        #[arg(long)]
        key: PathBuf,
    },

    /// Verify if a public key is a maintainer
    Verify {
        /// Public key to check
        #[arg(long)]
        pubkey: String,
    },
}

#[derive(Subcommand)]
enum DevnetCommands {
    /// Initialize a local devnet with N producer nodes
    Init {
        /// Number of nodes to create (1-20)
        #[arg(long, default_value = "3")]
        nodes: u32,
    },

    /// Start all devnet nodes
    Start,

    /// Stop all devnet nodes
    Stop,

    /// Show devnet status
    Status,

    /// Remove devnet data
    Clean {
        /// Keep wallet keys when cleaning
        #[arg(long)]
        keep_keys: bool,
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

    // Parse network (not used for devnet subcommands)
    let network = Network::from_str(&cli.network).map_err(|e| anyhow::anyhow!("{}", e))?;

    // Skip network logging for devnet subcommands (they manage their own environment)
    let is_devnet_command = matches!(cli.command, Some(Commands::Devnet { .. }));

    // Get data directory (use override or network default)
    let data_dir = cli.data_dir.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".doli")
            .join(network.data_dir_name())
    });

    // Load environment variables from data directory BEFORE using network params
    // This allows overriding network defaults via ~/.doli/{network}/.env
    if !is_devnet_command {
        network_params::load_env_for_network(network.name(), &data_dir);
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
            rpc_port,
            metrics_port,
            bootstrap,
            no_dht,
            force_start,
            yes,
            chainspec,
        }) => {
            let update_config = UpdateConfig {
                enabled: !no_auto_update,
                notify_only: update_notify_only,
                auto_rollback: !no_auto_rollback,
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
                yes,
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
        Some(Commands::Maintainer { action }) => {
            handle_maintainer_command(action, &data_dir, network).await?;
        }
        Some(Commands::Recover { yes }) => {
            recover_chain_state(network, &data_dir, yes)?;
        }
        Some(Commands::Devnet { action }) => {
            handle_devnet_command(action).await?;
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
                false, // yes
                None,  // chainspec
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
    yes: bool,
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
        let pubkey_hex = key.public_key().to_hex();
        info!(
            "Producer key loaded: {}...{} (hash: {})",
            &pubkey_hex[..16],
            &pubkey_hex[pubkey_hex.len() - 8..],
            &crypto::hash::hash(key.public_key().as_bytes()).to_hex()[..16]
        );

        // PRODUCER SAFETY CHECKS
        // These protect against accidental slashing by:
        // 1. Preventing two instances on the same machine (lock file)
        // 2. Preventing double-signing after restart (signed slots db)
        // 3. Detecting if another node is using this key (duplicate detection)

        // If --force-start is specified, require interactive confirmation (unless --yes)
        let force_start = if force_start {
            if yes {
                // --yes flag bypasses interactive confirmation (for automation)
                warn!("--yes specified: skipping force-start confirmation");
                true
            } else if !producer::confirm_force_start() {
                return Err(anyhow!("Force start cancelled by user"));
            } else {
                true
            }
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

        // Get genesis producers and genesis time from chainspec or fall back to built-in (legacy)
        if let Some(ref spec) = chainspec {
            // Set genesis time override if specified in chainspec (non-zero)
            // This ensures all nodes use the same genesis time for coordinated startup
            if spec.genesis.timestamp != 0 {
                config.genesis_time_override = Some(spec.genesis.timestamp);
                info!(
                    "Genesis time from chainspec: {} (all nodes will use this time)",
                    spec.genesis.timestamp
                );
            }

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
                ProducerSet::with_genesis_producers(genesis_producers, network.bond_unit())
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
                        ProducerSet::with_genesis_producers(genesis_producers, network.bond_unit())
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
        UpdateCommands::Votes { version } => {
            // Get version from argument or pending update
            let target_version = version.or_else(|| updater::get_pending_version(data_dir));

            let target_version = match target_version {
                Some(v) => v,
                None => {
                    println!("No version specified and no pending update found.");
                    println!("Use --version <ver> to check votes for a specific version.");
                    return Ok(());
                }
            };

            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!(
                "║                    VOTE STATUS: v{}                              ║",
                target_version
            );
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Check for votes in pending update
            if let Some(pending) = updater::get_pending_update(data_dir) {
                if pending.release.version == target_version {
                    let tracker = &pending.vote_tracker;
                    let veto_count = tracker.veto_count();
                    let approve_count = tracker.approval_count();
                    let total = tracker.total_votes();
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Veto votes:    {}                                               ║",
                        veto_count
                    );
                    println!(
                        "║  Approve votes: {}                                               ║",
                        approve_count
                    );
                    println!(
                        "║  Total votes:   {}                                               ║",
                        total
                    );
                    println!(
                        "║  Threshold:     40% of weighted votes                            ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    if pending.approved {
                        println!(
                            "║  Status: APPROVED                                                ║"
                        );
                    } else {
                        println!(
                            "║  Status: Voting in progress                                      ║"
                        );
                    }
                } else {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  No votes found for this version.                                ║"
                    );
                }
            } else {
                println!("║                                                                  ║");
                println!("║  No pending update found.                                         ║");
            }
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }
        UpdateCommands::Rollback => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    MANUAL ROLLBACK                               ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            match updater::rollback().await {
                Ok(()) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ✅ Rollback successful                                          ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!("║  Backup restored. Restart the node to use the previous version. ║");
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                }
                Err(e) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Rollback failed: {}                                          ║",
                        e
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!("║  No backup found or backup is corrupted.                        ║");
                    println!("║  You may need to reinstall manually.                            ║");
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                }
            }
        }
        UpdateCommands::Verify { version } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    VERIFY RELEASE                                ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // First check if we have a pending update matching the version
            let release = if let Some(pending) = updater::get_pending_update(data_dir) {
                if pending.release.version == version {
                    Some(pending.release)
                } else {
                    None
                }
            } else {
                None
            };

            // If no pending update, try to fetch from network
            let release = match release {
                Some(r) => Some(r),
                None => {
                    info!("Fetching latest release to verify...");
                    match updater::fetch_latest_release(None).await {
                        Ok(Some(r)) if r.version == version => Some(r),
                        Ok(Some(r)) => {
                            println!("║                                                                  ║");
                            println!("║  ❌ Requested version {} not found                               ║", version);
                            println!("║     Latest available: {}                                         ║", r.version);
                            println!("║                                                                  ║");
                            println!("╚══════════════════════════════════════════════════════════════════╝");
                            return Ok(());
                        }
                        Ok(None) => None,
                        Err(e) => {
                            println!("║                                                                  ║");
                            println!("║  ❌ Failed to fetch release: {}                                  ║", e);
                            println!("║                                                                  ║");
                            println!("╚══════════════════════════════════════════════════════════════════╝");
                            return Ok(());
                        }
                    }
                }
            };

            match release {
                Some(release) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Version: {}                                                     ║",
                        release.version
                    );
                    if release.binary_sha256.len() >= 16 {
                        println!(
                            "║  SHA256:  {}...                                                  ║",
                            &release.binary_sha256[..16]
                        );
                    }
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  MAINTAINER SIGNATURES                                           ║"
                    );

                    match updater::verify_release_signatures(&release) {
                        Ok(()) => {
                            for sig in &release.signatures {
                                if sig.public_key.len() >= 16 {
                                    println!(
                                        "║  ✓ {}...                                              ║",
                                        &sig.public_key[..16]
                                    );
                                }
                            }
                            println!("║                                                                  ║");
                            println!("║  ✅ Release signatures verified (3/5 threshold met)             ║");
                        }
                        Err(e) => {
                            println!("║  ❌ Verification failed: {}                                      ║", e);
                        }
                    }
                    println!(
                        "║                                                                  ║"
                    );
                }
                None => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Release v{} not found                                        ║",
                        version
                    );
                    println!(
                        "║                                                                  ║"
                    );
                }
            }
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }
    }
    Ok(())
}

async fn handle_maintainer_command(
    action: MaintainerCommands,
    data_dir: &PathBuf,
    network: Network,
) -> Result<()> {
    use doli_core::maintainer::{
        derive_maintainer_set, MaintainerChangeData, MaintainerSet, MaintainerSignature,
        INITIAL_MAINTAINER_COUNT, MAINTAINER_THRESHOLD, MAX_MAINTAINERS, MIN_MAINTAINERS,
    };

    match action {
        MaintainerCommands::List => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    MAINTAINER SET                                ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Load maintainer set from storage or derive from chain
            // For now, show the expected structure
            let producers_path = data_dir.join("producers.bin");
            if producers_path.exists() {
                // In a full implementation, we would derive the maintainer set
                // from the blockchain by scanning registration transactions
                println!("║                                                                  ║");
                println!(
                    "║  Maintainer set derived from first {} registrations            ║",
                    INITIAL_MAINTAINER_COUNT
                );
                println!(
                    "║  Threshold: {} of {} signatures required                        ║",
                    MAINTAINER_THRESHOLD, MAX_MAINTAINERS
                );
                println!(
                    "║  Min maintainers: {}                                             ║",
                    MIN_MAINTAINERS
                );
                println!(
                    "║  Max maintainers: {}                                             ║",
                    MAX_MAINTAINERS
                );
                println!("║                                                                  ║");
                println!("║  Note: Full maintainer list requires blockchain scan.           ║");
                println!("║  Use RPC 'getMaintainerSet' for complete information.           ║");
            } else {
                println!("║                                                                  ║");
                println!("║  No producer data found. Start the node to sync.                ║");
            }
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }

        MaintainerCommands::Remove {
            target,
            key,
            reason,
        } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    PROPOSE MAINTAINER REMOVAL                    ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Load maintainer key
            let keypair = load_producer_key(&key)?;
            let signer_pubkey = keypair.public_key().clone();

            // Parse target public key
            let target_pubkey = match PublicKey::from_hex(&target) {
                Ok(pk) => pk,
                Err(e) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Invalid target public key: {}                               ║",
                        e
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                    return Ok(());
                }
            };

            // Create the change data
            let change_data = MaintainerChangeData::with_reason(
                target_pubkey.clone(),
                vec![], // Signatures will be collected separately
                reason.unwrap_or_default(),
            );

            // Sign the proposal
            let message = change_data.signing_message(false); // false = removal
            let signature = crypto::signature::sign(&message, keypair.private_key());

            let sig = MaintainerSignature {
                pubkey: signer_pubkey.clone(),
                signature,
            };

            println!("║                                                                  ║");
            println!("║  Proposal created:                                               ║");
            println!(
                "║  Target:  {}...                                                  ║",
                &target[..16.min(target.len())]
            );
            println!(
                "║  Signer:  {}...                                                  ║",
                &signer_pubkey.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!(
                "║  Signature: {}...                                                ║",
                &sig.signature.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!("║  Next steps:                                                     ║");
            println!("║  1. Share proposal with other maintainers                        ║");
            println!("║  2. Collect 3/5 signatures                                       ║");
            println!("║  3. Submit via RPC 'submitMaintainerChange'                      ║");
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }

        MaintainerCommands::Add { target, key } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    PROPOSE MAINTAINER ADDITION                   ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Load maintainer key
            let keypair = load_producer_key(&key)?;
            let signer_pubkey = keypair.public_key().clone();

            // Parse target public key
            let target_pubkey = match PublicKey::from_hex(&target) {
                Ok(pk) => pk,
                Err(e) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Invalid target public key: {}                               ║",
                        e
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "╚══════════════════════════════════════════════════════════════════╝"
                    );
                    return Ok(());
                }
            };

            // Create the change data
            let change_data = MaintainerChangeData::new(target_pubkey.clone(), vec![]);

            // Sign the proposal
            let message = change_data.signing_message(true); // true = addition
            let signature = crypto::signature::sign(&message, keypair.private_key());

            let sig = MaintainerSignature {
                pubkey: signer_pubkey.clone(),
                signature,
            };

            println!("║                                                                  ║");
            println!("║  Proposal created:                                               ║");
            println!(
                "║  Target:  {}...                                                  ║",
                &target[..16.min(target.len())]
            );
            println!(
                "║  Signer:  {}...                                                  ║",
                &signer_pubkey.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!(
                "║  Signature: {}...                                                ║",
                &sig.signature.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!("║  Next steps:                                                     ║");
            println!("║  1. Share proposal with other maintainers                        ║");
            println!("║  2. Collect 3/5 signatures                                       ║");
            println!("║  3. Submit via RPC 'submitMaintainerChange'                      ║");
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }

        MaintainerCommands::Sign { proposal_id, key } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    SIGN MAINTAINER PROPOSAL                      ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Load maintainer key
            let keypair = load_producer_key(&key)?;
            let signer_pubkey = keypair.public_key().clone();

            println!("║                                                                  ║");
            println!(
                "║  Proposal ID: {}                                                 ║",
                proposal_id
            );
            println!(
                "║  Signer:      {}...                                              ║",
                &signer_pubkey.to_hex()[..16]
            );
            println!("║                                                                  ║");
            println!("║  Note: Proposal signing requires the full proposal data.         ║");
            println!("║  Use RPC to fetch proposal and sign interactively.               ║");
            println!("║                                                                  ║");
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }

        MaintainerCommands::Verify { pubkey } => {
            println!();
            println!("╔══════════════════════════════════════════════════════════════════╗");
            println!("║                    VERIFY MAINTAINER STATUS                      ║");
            println!("╠══════════════════════════════════════════════════════════════════╣");

            // Parse public key
            match PublicKey::from_hex(&pubkey) {
                Ok(pk) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Public key: {}...                                               ║",
                        &pubkey[..16.min(pubkey.len())]
                    );
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  Note: Full verification requires blockchain access.             ║"
                    );
                    println!(
                        "║  Use RPC 'getMaintainerSet' to check current maintainers.        ║"
                    );
                    println!(
                        "║                                                                  ║"
                    );
                }
                Err(e) => {
                    println!(
                        "║                                                                  ║"
                    );
                    println!(
                        "║  ❌ Invalid public key: {}                                       ║",
                        e
                    );
                    println!(
                        "║                                                                  ║"
                    );
                }
            }
            println!("╚══════════════════════════════════════════════════════════════════╝");
        }
    }

    Ok(())
}

async fn handle_devnet_command(action: DevnetCommands) -> Result<()> {
    match action {
        DevnetCommands::Init { nodes } => {
            devnet::init(nodes)?;
        }
        DevnetCommands::Start => {
            devnet::start().await?;
        }
        DevnetCommands::Stop => {
            devnet::stop().await?;
        }
        DevnetCommands::Status => {
            devnet::status().await?;
        }
        DevnetCommands::Clean { keep_keys } => {
            devnet::clean(keep_keys)?;
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

/// Recover chain state from existing block data
///
/// This function scans the BlockStore to find all blocks and rebuilds:
/// - chain_state.bin (chain tip: height, hash, slot)
/// - UTXO set (all unspent outputs)
/// - producers.bin (registered producers)
fn recover_chain_state(network: Network, data_dir: &PathBuf, skip_confirm: bool) -> Result<()> {
    use doli_core::consensus::{ConsensusParams, RewardMode, UNBONDING_PERIOD};
    use doli_core::transaction::TxType;
    use storage::{BlockStore, ChainState, ProducerInfo, ProducerSet, UtxoSet};

    println!("=== DOLI Chain State Recovery ===");
    println!();
    println!("Data directory: {:?}", data_dir);
    println!("Network: {}", network.name());
    println!();

    // Check if data directory exists
    if !data_dir.exists() {
        return Err(anyhow!("Data directory does not exist: {:?}", data_dir));
    }

    // Check if blocks directory exists
    let blocks_path = data_dir.join("blocks");
    if !blocks_path.exists() {
        return Err(anyhow!(
            "Blocks directory not found: {:?}. Nothing to recover.",
            blocks_path
        ));
    }

    // Open BlockStore
    println!("Opening BlockStore...");
    let block_store = BlockStore::open(&blocks_path)?;

    // Scan to find chain tip (highest block)
    println!("Scanning for blocks...");
    let mut tip_height = 0u64;
    let mut tip_hash = crypto::hash::hash(b"DOLI Genesis");
    let mut tip_slot = 0u32;
    let mut block_count = 0u64;

    // Scan heights starting from 1
    for height in 1..=u64::MAX {
        match block_store.get_block_by_height(height) {
            Ok(Some(block)) => {
                tip_height = height;
                tip_hash = block.hash();
                tip_slot = block.header.slot;
                block_count += 1;

                if block_count % 1000 == 0 {
                    print!("\r  Scanned {} blocks (height {})...", block_count, height);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                }
            }
            Ok(None) => {
                // No more blocks at this height
                break;
            }
            Err(e) => {
                warn!("Error reading block at height {}: {}", height, e);
                break;
            }
        }
    }
    println!();

    if block_count == 0 {
        return Err(anyhow!(
            "No blocks found in BlockStore. Nothing to recover."
        ));
    }

    println!();
    println!("Found {} blocks:", block_count);
    println!("  Chain tip height: {}", tip_height);
    println!("  Chain tip hash:   {}", tip_hash);
    println!("  Chain tip slot:   {}", tip_slot);
    println!();

    // Check existing state files
    let state_path = data_dir.join("chain_state.bin");
    let utxo_path = data_dir.join("utxo");
    let producers_path = data_dir.join("producers.bin");

    let state_exists = state_path.exists();
    let utxo_exists = utxo_path.exists();
    let producers_exists = producers_path.exists();

    if state_exists || utxo_exists || producers_exists {
        println!("Existing state files:");
        if state_exists {
            println!("  - chain_state.bin (will be overwritten)");
        }
        if utxo_exists {
            println!("  - utxo/ (will be overwritten)");
        }
        if producers_exists {
            println!("  - producers.bin (will be overwritten)");
        }
        println!();
    }

    // Confirm with user
    if !skip_confirm {
        print!(
            "Proceed with recovery? This will rebuild state from {} blocks. [y/N] ",
            block_count
        );
        std::io::Write::flush(&mut std::io::stdout()).ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Recovery cancelled.");
            return Ok(());
        }
    }

    println!();
    println!("Rebuilding state from blocks...");

    // Initialize fresh state
    let genesis_hash = crypto::hash::hash(b"DOLI Genesis");
    let mut chain_state = ChainState::new(genesis_hash);
    let mut utxo_set = UtxoSet::new();
    let mut producer_set = ProducerSet::new();
    let params = ConsensusParams::for_network(network);

    // Replay all blocks
    for height in 1..=tip_height {
        let block = block_store
            .get_block_by_height(height)?
            .ok_or_else(|| anyhow!("Block at height {} disappeared during recovery", height))?;

        // Apply transactions to UTXO set
        for (tx_index, tx) in block.transactions.iter().enumerate() {
            let is_coinbase = tx_index == 0 && tx.is_coinbase();
            let _ = utxo_set.spend_transaction(tx); // Ignore errors for coinbase
            utxo_set.add_transaction(tx, height, is_coinbase);

            // Process registration transactions
            if tx.tx_type == TxType::Registration {
                if let Some(reg_data) = tx.registration_data() {
                    if let Some((bond_index, bond_output)) =
                        tx.outputs.iter().enumerate().find(|(_, o)| {
                            o.output_type == doli_core::transaction::OutputType::Bond
                        })
                    {
                        let tx_hash = tx.hash();
                        let era = params.height_to_era(height);

                        let producer_info = ProducerInfo::new(
                            reg_data.public_key.clone(),
                            height,
                            bond_output.amount,
                            (tx_hash, bond_index as u32),
                            era,
                            network.initial_bond(),
                        );

                        if let Err(e) = producer_set.register(producer_info, height) {
                            warn!("Failed to register producer at height {}: {}", height, e);
                        }
                    }
                }
            }

            // Process exit transactions
            if tx.tx_type == TxType::Exit {
                if let Some(exit_data) = tx.exit_data() {
                    if let Err(e) = producer_set.request_exit(&exit_data.public_key, height) {
                        warn!("Failed to process exit at height {}: {}", height, e);
                    }
                }
            }

            // Process slash transactions
            if tx.tx_type == TxType::SlashProducer {
                if let Some(slash_data) = tx.slash_data() {
                    if let Err(e) = producer_set.slash_producer(&slash_data.producer_pubkey, height)
                    {
                        warn!("Failed to slash producer at height {}: {}", height, e);
                    }
                }
            }
        }

        // Process completed unbonding periods
        let _ = producer_set.process_unbonding(height, UNBONDING_PERIOD);

        // Update chain state
        chain_state.update(block.hash(), height, block.header.slot);

        // Set genesis timestamp from first block if needed
        if chain_state.genesis_timestamp == 0 && height == 1 {
            let block_timestamp = block.header.timestamp;
            chain_state.genesis_timestamp =
                block_timestamp - (block_timestamp % params.slot_duration);
        }

        // Progress indicator
        if height % 500 == 0 || height == tip_height {
            let pct = (height as f64 / tip_height as f64) * 100.0;
            print!(
                "\r  Replayed {}/{} blocks ({:.1}%)...",
                height, tip_height, pct
            );
            std::io::Write::flush(&mut std::io::stdout()).ok();
        }
    }
    println!();

    // Save recovered state
    println!();
    println!("Saving recovered state...");

    chain_state.save(&state_path)?;
    println!(
        "  Saved chain_state.bin (height={}, slot={})",
        chain_state.best_height, chain_state.best_slot
    );

    utxo_set.save(&utxo_path)?;
    println!("  Saved utxo/ ({} UTXOs)", utxo_set.len());

    producer_set.save(&producers_path)?;
    println!(
        "  Saved producers.bin ({} producers)",
        producer_set.active_count()
    );

    println!();
    println!("=== Recovery Complete ===");
    println!();
    println!("Chain state recovered:");
    println!("  Height: {}", chain_state.best_height);
    println!("  Hash:   {}", chain_state.best_hash);
    println!("  Slot:   {}", chain_state.best_slot);
    println!();
    println!("You can now start the node normally.");

    Ok(())
}
