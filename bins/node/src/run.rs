use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use crypto::PublicKey;
use doli_core::Network;
use storage::ProducerSet;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::cli::expand_tilde_path;
use crate::config::NodeConfig;
use crate::keys::{load_bls_key, load_producer_key};
use crate::updater::UpdateConfig;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_node(
    network: Network,
    data_dir: &Path,
    producer: bool,
    producer_key_path: Option<PathBuf>,
    update_config: UpdateConfig,
    p2p_port: Option<u16>,
    external_address: Option<String>,
    rpc_port: Option<u16>,
    rpc_bind: Option<String>,
    metrics_port: u16,
    bootstrap: Vec<String>,
    no_dht: bool,
    relay_server: bool,
    force_start: bool,
    yes: bool,
    chainspec_path: Option<PathBuf>,
    archive_to: Option<PathBuf>,
    seed: bool,
    no_snap_sync: bool,
    snap_sync: bool,
) -> Result<()> {
    // Expand tilde in all paths (shell expansion doesn't happen in Rust)
    let data_dir = expand_tilde_path(data_dir);
    let producer_key_path = producer_key_path.map(|p| expand_tilde_path(&p));
    let archive_to = archive_to.map(|p| expand_tilde_path(&p));
    let chainspec_path = chainspec_path.map(|p| expand_tilde_path(&p));

    info!("Starting node with data directory: {:?}", data_dir);

    // Ensure data directory exists
    std::fs::create_dir_all(&data_dir)?;

    // Load producer key if production is enabled
    let (producer_key, bls_key, producer_guard, signed_slots_db) = if producer {
        let key_path = producer_key_path
            .ok_or_else(|| anyhow!("--producer-key required when --producer is enabled"))?;

        info!("Loading producer key from {:?}", key_path);
        let key = load_producer_key(&key_path)?;
        let bls = load_bls_key(&key_path);
        if bls.is_none() {
            return Err(anyhow!(
                "Producer key missing BLS keypair. BLS is mandatory for all producers.\n\
                 Fix: doli --wallet {} add-bls",
                key_path.display()
            ));
        }
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
            } else if !crate::producer::confirm_force_start() {
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
        match crate::producer::startup_checks(&data_dir, key.public_key(), force_start, None).await
        {
            Ok((guard, db)) => {
                info!("Producer safety checks passed");
                (Some(key), bls, Some(guard), Some(db))
            }
            Err(e) => {
                error!("Producer startup blocked: {}", e);
                return Err(anyhow!("{}", e));
            }
        }
    } else {
        (None, None, None, None)
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
    {
        let bind = rpc_bind.as_deref().unwrap_or("127.0.0.1");
        let port = rpc_port.unwrap_or(config.network.default_rpc_port());
        config.rpc.listen_addr = format!("{}:{}", bind, port);
    }
    if no_dht {
        config.no_dht = true;
        // Clear default bootstrap nodes - only use explicitly provided ones
        config.bootstrap_nodes.clear();
        info!("DHT discovery disabled - cleared default bootstrap nodes, only connecting to explicit bootstrap addresses");
    }
    if seed {
        config.seed_mode = true;
        info!("Seed mode enabled — only serving confirmed blocks to prevent fork propagation");
    }
    if no_snap_sync {
        config.no_snap_sync = true;
        info!("Snap sync disabled — node will only use header-first sync");
    }
    // REQ-SYNC-002: --snap-sync explicitly enables snap sync (overrides mainnet default)
    if snap_sync {
        config.no_snap_sync = false;
        info!("Snap sync explicitly enabled via --snap-sync");
    }
    if relay_server {
        config.relay_server = true;
        info!("Relay server enabled — NAT'd peers can relay through this node");
    }
    if let Some(ref addr) = external_address {
        config.external_address = Some(addr.clone());
        info!("External address: {}", addr);
    }
    // Add explicit bootstrap(s) after clearing defaults (so they're preserved with --no-dht)
    for addr in &bootstrap {
        config.bootstrap_nodes.push(addr.clone());
    }

    // Start metrics server
    let metrics_addr: std::net::SocketAddr = format!("0.0.0.0:{}", metrics_port).parse()?;
    crate::metrics::spawn_metrics_server(metrics_addr);
    info!("Metrics server started on {}", metrics_addr);

    // Start auto-update service
    if update_config.enabled {
        info!(
            "Auto-updates enabled (check: {}m, veto: {}m, grace: {}m, threshold: 40%)",
            update_config.check_interval_secs / 60,
            update_config.veto_period_secs / 60,
            update_config.grace_period_secs / 60,
        );
        if update_config.notify_only {
            info!("Notify-only mode: updates will not be applied automatically");
        }
    } else {
        info!("Auto-updates disabled");
    }

    // Load chainspec ALWAYS (consensus-critical overrides must apply on every start)
    // Previously this was inside the first-init branch, causing slot_duration divergence on restarts
    use doli_core::chainspec::ChainSpec;

    // SECURITY: For mainnet, ALWAYS use the embedded chainspec. The genesis_timestamp,
    // slot_duration, and other consensus parameters are compiled into the binary and
    // cannot be overridden by disk files or CLI flags. This prevents:
    // - Stale chainspec.json from a previous binary causing slot schedule divergence
    // - Malicious modification of chainspec.json to hijack genesis time
    // - Accidental deployment with mismatched parameters
    //
    // For testnet/devnet, disk files and CLI flags are allowed for flexibility.
    let chainspec: Option<ChainSpec> = if matches!(network, Network::Mainnet) {
        const EMBEDDED_MAINNET_CHAINSPEC: &str = include_str!("../../../chainspec.mainnet.json");
        match serde_json::from_str::<ChainSpec>(EMBEDDED_MAINNET_CHAINSPEC) {
            Ok(spec) => {
                info!(
                    "Mainnet chainspec: embedded (genesis={}, slot={}s) — disk/CLI overrides disabled",
                    spec.genesis.timestamp, spec.consensus.slot_duration
                );
                if chainspec_path.is_some() {
                    warn!("SECURITY: --chainspec flag ignored for mainnet — embedded chainspec is authoritative");
                }
                let disk_path = data_dir.join("chainspec.json");
                if disk_path.exists() {
                    warn!(
                        "SECURITY: Ignoring {:?} — mainnet uses embedded chainspec only",
                        disk_path
                    );
                }
                Some(spec)
            }
            Err(e) => {
                error!("BUG: embedded mainnet chainspec is invalid: {}", e);
                std::process::exit(1);
            }
        }
    } else if matches!(network, Network::Testnet) {
        const EMBEDDED_TESTNET_CHAINSPEC: &str = include_str!("../../../chainspec.testnet.json");
        match serde_json::from_str::<ChainSpec>(EMBEDDED_TESTNET_CHAINSPEC) {
            Ok(spec) => {
                info!(
                    "Testnet chainspec: embedded (genesis={}, slot={}s) — disk/CLI overrides disabled",
                    spec.genesis.timestamp, spec.consensus.slot_duration
                );
                if chainspec_path.is_some() {
                    warn!("SECURITY: --chainspec flag ignored for testnet — embedded chainspec is authoritative");
                }
                Some(spec)
            }
            Err(e) => {
                error!("BUG: embedded testnet chainspec is invalid: {}", e);
                std::process::exit(1);
            }
        }
    } else if let Some(ref path) = chainspec_path {
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
        // Devnet: try disk, then none
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

    // ======================== CONSENSUS-CRITICAL GUARD ========================
    // Producer nodes MUST have a chainspec to ensure all nodes agree on:
    // - genesis_timestamp (determines slot schedule)
    // - slot_duration
    // - bond_amount, initial_reward
    //
    // For mainnet this is always satisfied (embedded chainspec).
    // For testnet/devnet, a disk file or --chainspec flag is required.
    if producer && chainspec.is_none() {
        error!("FATAL: --producer requires a chainspec file for consensus safety");
        error!("Without chainspec, genesis_timestamp may differ between nodes,");
        error!("causing slot schedule divergence and chain forks.");
        error!("");
        error!("Fix: specify --chainspec /path/to/chainspec.json");
        std::process::exit(1);
    }

    // Pass chainspec to config so Node::new() can apply overrides
    config.chainspec = chainspec.clone();

    // Apply consensus-critical overrides from chainspec (MUST run on every start, not just first init)
    if let Some(ref spec) = chainspec {
        if spec.genesis.timestamp != 0 {
            config.genesis_time_override = Some(spec.genesis.timestamp);
            info!(
                "Genesis time from chainspec: {} (all nodes will use this time)",
                spec.genesis.timestamp
            );
        }
        if spec.consensus.slot_duration > 0 {
            config.slot_duration_override = Some(spec.consensus.slot_duration);
            info!(
                "Slot duration from chainspec: {}s (overrides local config)",
                spec.consensus.slot_duration
            );
        }
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
    } else if let Some(ref spec) = chainspec {
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
            // Chainspec has no genesis producers — fall back to hardcoded constants
            match network {
                Network::Mainnet => {
                    use doli_core::genesis::mainnet_genesis_producers;
                    let genesis_producers = mainnet_genesis_producers();
                    info!(
                        "Chainspec has no genesis producers, using {} hardcoded mainnet producers",
                        genesis_producers.len()
                    );
                    ProducerSet::with_genesis_producers(genesis_producers, network.bond_unit())
                }
                Network::Testnet => {
                    use doli_core::genesis::testnet_genesis_producers;
                    let genesis_producers = testnet_genesis_producers();
                    info!(
                        "Chainspec has no genesis producers, using {} hardcoded testnet producers",
                        genesis_producers.len()
                    );
                    ProducerSet::with_genesis_producers(genesis_producers, network.bond_unit())
                }
                _ => {
                    info!("Chainspec has no genesis producers, using bootstrap mode");
                    ProducerSet::new()
                }
            }
        }
    } else {
        // No chainspec at all — fall back to hardcoded genesis producers
        match network {
            Network::Mainnet => {
                use doli_core::genesis::mainnet_genesis_producers;
                let genesis_producers = mainnet_genesis_producers();
                info!(
                    "No chainspec provided, using {} hardcoded mainnet genesis producers",
                    genesis_producers.len()
                );
                ProducerSet::with_genesis_producers(genesis_producers, network.bond_unit())
            }
            Network::Testnet => {
                use doli_core::genesis::testnet_genesis_producers;
                let genesis_producers = testnet_genesis_producers();
                info!(
                    "No chainspec provided, using {} hardcoded testnet genesis producers",
                    genesis_producers.len()
                );
                ProducerSet::with_genesis_producers(genesis_producers, network.bond_unit())
            }
            Network::Devnet => ProducerSet::new(),
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

    // Load on-chain maintainer set from disk (or default empty).
    // Shared between: maintainer_keys_fn (UpdateService), Node (apply_block), RPC.
    // Bootstrapped from first 5 producers at epoch boundary in Node::apply_block.
    let maintainer_state = Arc::new(RwLock::new(
        storage::MaintainerState::load(&data_dir).unwrap_or_default(),
    ));
    // Return empty → updater falls back to network-specific bootstrap maintainer keys.
    // On-chain ProducerInfo stores BLAKE3 pubkey hashes, not raw Ed25519 keys,
    // so MaintainerState members can't be used for signature verification.
    // TODO: Store raw Ed25519 keys on-chain to enable dynamic maintainer key lookup.
    let _ms_for_keys = maintainer_state.clone();
    let maintainer_keys_fn = move || -> Vec<String> { Vec::new() };

    // Spawn update service with real producer registry
    let (vote_tx, pending_update) = crate::updater::spawn_update_service(
        update_config,
        data_dir.clone(),
        network,
        producer_count_fn,
        is_producer_fn,
        maintainer_keys_fn,
    );

    // Create shared shutdown flag for graceful shutdown signaling
    let shutdown_flag = Arc::new(RwLock::new(false));
    let shutdown_flag_for_node = shutdown_flag.clone();

    // Create and run the node (share the producer_set, signed_slots_db, and shutdown flag)
    let mut node = crate::node::Node::new(
        config,
        producer_key,
        bls_key,
        Some(producer_set),
        signed_slots_db,
        Some(shutdown_flag_for_node),
    )
    .await?;

    // Connect vote forwarding: gossip votes → UpdateService
    node.set_vote_tx(vote_tx);

    // Connect pending update state: UpdateService → RPC (live status reads)
    node.set_pending_update(pending_update);

    // Connect on-chain maintainer set: Node (apply_block) → UpdateService (signature verification)
    node.set_maintainer_state(maintainer_state);

    // Block archiver: streams blocks to filesystem for disaster recovery
    if let Some(ref archive_dir) = archive_to {
        let archive_dir = expand_tilde_path(archive_dir);
        let tip = node.best_height().await;

        // Catch up: archive any blocks we missed while down
        if tip > 0 {
            info!("Archiver: catching up to height {}...", tip);
            match storage::archiver::BlockArchiver::catch_up(&archive_dir, node.block_store(), tip)
            {
                Ok(n) if n > 0 => info!("Archiver: caught up {} blocks", n),
                Ok(_) => info!("Archiver: already up to date"),
                Err(e) => warn!("Archiver catch-up error: {}", e),
            }
        }

        let (archive_tx, archive_rx) = tokio::sync::mpsc::channel(256);
        node.set_archive_tx(archive_tx, archive_dir.clone());
        let archiver = storage::archiver::BlockArchiver::new(archive_rx, archive_dir);
        tokio::spawn(async move { archiver.run().await });
        info!("Block archiver enabled");
    }

    info!("Node running. Press Ctrl+C to stop.");

    // Run node in a separate task
    let node_handle = tokio::spawn(async move {
        if let Err(e) = node.run().await {
            error!("Node error: {}", e);
        }
    });

    // Wait for shutdown signal (SIGINT via Ctrl+C or SIGTERM via kill)
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
    }

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
