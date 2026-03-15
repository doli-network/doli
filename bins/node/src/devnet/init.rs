use std::fs;

use anyhow::{anyhow, Context, Result};
use crypto::KeyPair;
use doli_core::chainspec::{ChainSpec, ConsensusSpec, GenesisProducer, GenesisSpec};
use doli_core::Network;
use tracing::info;

use super::config::{devnet_dir, load_devnet_env, save_config};
use super::{DevnetConfig, Wallet, WalletAddress};

/// Initialize a new devnet with N producer nodes
pub fn init(node_count: u32) -> Result<()> {
    // Validate node count
    if node_count == 0 || node_count > 100 {
        return Err(anyhow!(
            "Node count must be between 1 and 100, got {}",
            node_count
        ));
    }

    let root = devnet_dir();

    // Check if already initialized
    if root.join("devnet.toml").exists() {
        return Err(anyhow!(
            "Devnet already initialized at {:?}. Run 'doli-node devnet clean' first.",
            root
        ));
    }

    // Create root directory first so we can load .env
    fs::create_dir_all(&root)?;

    // Create default .env file from .env.devnet in repo root
    let env_path = root.join(".env");
    if !env_path.exists() {
        let env_contents = load_devnet_env()?;
        fs::write(&env_path, &env_contents)
            .with_context(|| format!("Failed to create {:?}", env_path))?;
        info!("  Created default .env configuration: {:?}", env_path);
    }

    // Load environment variables from devnet root .env file
    // This allows customizing network parameters before init
    doli_core::network_params::load_env_for_network("devnet", &root);

    info!(
        "Initializing devnet with {} nodes at {:?}",
        node_count, root
    );

    // Create directory structure
    fs::create_dir_all(&root)?;
    fs::create_dir_all(root.join("keys"))?;
    fs::create_dir_all(root.join("data"))?;
    fs::create_dir_all(root.join("logs"))?;
    fs::create_dir_all(root.join("pids"))?;

    // Generate wallets for each producer
    let mut genesis_producers = Vec::with_capacity(node_count as usize);

    for i in 0..node_count {
        let keypair = KeyPair::generate();
        let wallet = Wallet {
            name: format!("producer_{}", i),
            version: 1,
            addresses: vec![WalletAddress {
                address: keypair.address().to_hex(),
                public_key: keypair.public_key().to_hex(),
                private_key: keypair.private_key().to_hex(),
                label: Some("primary".to_string()),
            }],
        };

        // Save wallet file
        let wallet_path = root.join("keys").join(format!("producer_{}.json", i));
        let wallet_json = serde_json::to_string_pretty(&wallet)?;
        fs::write(&wallet_path, wallet_json)?;
        info!("  Created wallet: {:?}", wallet_path);

        // Add to genesis producers
        genesis_producers.push(GenesisProducer {
            name: format!("producer_{}", i),
            public_key: keypair.public_key().to_hex(),
            bond_count: 1,
        });

        // Create node data directory
        fs::create_dir_all(root.join("data").join(format!("node{}", i)))?;
    }

    // Build chainspec using NetworkParams (which reads from .env)
    // This allows customizing devnet parameters via ~/.doli/devnet/.env
    let params = Network::Devnet.params();

    // CRITICAL: Set a fixed genesis time for ALL nodes to ensure they compute
    // the same current_slot at the same wall-clock time. Without this, each node
    // would use its own startup time as genesis, breaking round-robin consensus.
    //
    // Set genesis time a few seconds in the future to give nodes time to start
    // and discover each other before production begins.
    let genesis_delay_secs = 15; // 15 seconds from now
    let genesis_time = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        // Round to slot boundary and add delay
        (now / params.slot_duration + genesis_delay_secs / params.slot_duration + 1)
            * params.slot_duration
    };

    let chainspec = ChainSpec {
        name: "DOLI Local Devnet".into(),
        id: "local-devnet".into(),
        network: Network::Devnet,
        genesis: GenesisSpec {
            timestamp: genesis_time, // Fixed time for all nodes
            message: "DOLI Local Devnet - Development and Testing".into(),
            initial_reward: params.initial_reward,
        },
        consensus: ConsensusSpec {
            slot_duration: params.slot_duration,
            slots_per_epoch: params.slots_per_reward_epoch,
            bond_amount: params.bond_unit,
        },
        genesis_producers,
    };

    // Save chainspec
    let chainspec_path = root.join("chainspec.json");
    chainspec.save(&chainspec_path)?;
    info!("  Created chainspec: {:?}", chainspec_path);

    // Save devnet config
    let config = DevnetConfig {
        node_count,
        ..Default::default()
    };
    save_config(&config)?;
    info!("  Created config: {:?}", root.join("devnet.toml"));

    println!();
    println!("Devnet initialized successfully!");
    println!("  Nodes: {}", node_count);
    println!("  Directory: {:?}", root);
    println!();
    println!("Next steps:");
    println!("  doli-node devnet start   # Start all nodes");
    println!("  doli-node devnet status  # Check status");

    Ok(())
}
