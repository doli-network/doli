//! DOLI CLI - Wallet command-line interface
//!
//! This binary provides wallet functionality:
//! - Key generation and management
//! - Transaction creation and signing
//! - Balance queries
//! - Network interaction

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod rpc_client;
mod wallet;

use rpc_client::{coins_to_units, format_balance, RpcClient};
use wallet::Wallet;

#[derive(Parser)]
#[command(name = "doli")]
#[command(about = "DOLI wallet CLI", long_about = None)]
#[command(version = env!("DOLI_VERSION_STRING"))]
struct Cli {
    /// Wallet file path
    #[arg(short, long, default_value = "~/.doli/wallet.json")]
    wallet: String,

    /// Node RPC endpoint (auto-detected from --network if not set)
    #[arg(short, long)]
    rpc: Option<String>,

    /// Network (mainnet, testnet, devnet)
    #[arg(short, long, default_value = "mainnet")]
    network: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new wallet
    New {
        /// Wallet name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Restore a wallet from a 24-word seed phrase
    Restore {
        /// Wallet name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Generate a new address
    Address {
        /// Label for the address
        #[arg(short, long)]
        label: Option<String>,
    },

    /// List all addresses
    Addresses,

    /// Show wallet balance
    Balance {
        /// Specific address (default: all)
        #[arg(short, long)]
        address: Option<String>,
    },

    /// Send coins
    Send {
        /// Recipient address
        to: String,

        /// Amount to send
        amount: String,

        /// Fee (default: auto)
        #[arg(short, long)]
        fee: Option<String>,
    },

    /// Show transaction history
    History {
        /// Maximum number of transactions
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Export wallet
    Export {
        /// Output file path
        output: PathBuf,
    },

    /// Import wallet
    Import {
        /// Input file path
        input: PathBuf,
    },

    /// Show wallet info
    Info,

    /// Add BLS attestation key to an existing wallet
    AddBls,

    /// Sign a message
    Sign {
        /// Message to sign
        message: String,

        /// Address to sign with
        #[arg(short, long)]
        address: Option<String>,
    },

    /// Verify a signature
    Verify {
        /// Message that was signed
        message: String,

        /// Signature (hex)
        signature: String,

        /// Public key or address
        pubkey: String,
    },

    /// Producer commands
    Producer {
        #[command(subcommand)]
        command: ProducerCommands,
    },

    /// Rewards commands (epoch presence rewards)
    Rewards {
        #[command(subcommand)]
        command: RewardsCommands,
    },

    /// Show chain information
    Chain,

    /// Update governance commands
    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },

    /// Maintainer governance commands
    Maintainer {
        #[command(subcommand)]
        command: MaintainerCommands,
    },

    /// Upgrade doli binaries to the latest release
    Upgrade {
        /// Target version (default: latest)
        #[arg(long)]
        version: Option<String>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,

        /// Custom path to doli-node binary (skip auto-detection)
        #[arg(long)]
        doli_node_path: Option<std::path::PathBuf>,

        /// Restart only this systemd service (e.g. doli-mainnet-node3)
        #[arg(long)]
        service: Option<String>,
    },

    /// Release management commands (maintainer signing)
    Release {
        #[command(subcommand)]
        command: ReleaseCommands,
    },

    /// Protocol activation commands (on-chain consensus upgrades)
    Protocol {
        #[command(subcommand)]
        command: ProtocolCommands,
    },

    /// Wipe chain data for a fresh resync (preserves keys/ and .env)
    Wipe {
        /// Network (mainnet, testnet, devnet)
        #[arg(short, long, default_value = "mainnet")]
        network: String,

        /// Data directory (overrides network default)
        #[arg(short, long)]
        data_dir: Option<PathBuf>,

        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ProducerCommands {
    /// Register as a block producer
    Register {
        /// Number of bonds to stake (1-10000, each bond = 1 bond_unit)
        #[arg(short, long, default_value = "1")]
        bonds: u32,
    },

    /// Check producer status
    Status {
        /// Public key (optional, uses wallet if not specified)
        #[arg(short, long)]
        pubkey: Option<String>,
    },

    /// Show per-bond vesting details
    Bonds {
        /// Public key (optional, uses wallet if not specified)
        #[arg(short, long)]
        pubkey: Option<String>,
    },

    /// List all producers in the network
    List {
        /// Show only active producers
        #[arg(short, long)]
        active: bool,
    },

    /// Add more bonds to increase stake (bond stacking)
    AddBond {
        /// Number of bonds to add (1-10000)
        #[arg(short, long)]
        count: u32,
    },

    /// Withdraw bonds instantly (FIFO, vesting penalty applies)
    RequestWithdrawal {
        /// Number of bonds to withdraw
        #[arg(short, long)]
        count: u32,

        /// Destination address for withdrawn funds (hex pubkey_hash)
        #[arg(short, long)]
        destination: Option<String>,
    },

    /// Simulate bond withdrawal (dry run, no transaction)
    SimulateWithdrawal {
        /// Number of bonds to simulate withdrawing
        #[arg(short, long)]
        count: u32,
    },

    /// Exit the producer set (early exit incurs penalty)
    Exit {
        /// Force early exit with penalty
        #[arg(long)]
        force: bool,
    },

    /// Submit slashing evidence for double production (equivocation)
    Slash {
        /// Block 1 hash (first conflicting block)
        #[arg(long)]
        block1: String,

        /// Block 2 hash (second conflicting block for same slot)
        #[arg(long)]
        block2: String,
    },
}

#[derive(Subcommand)]
enum RewardsCommands {
    /// List all claimable epochs with estimated rewards
    List,

    /// Claim rewards for a specific epoch
    Claim {
        /// Epoch number to claim
        epoch: u64,

        /// Recipient address (defaults to wallet address)
        #[arg(short, long)]
        recipient: Option<String>,
    },

    /// Claim all available rewards (one tx per epoch)
    ClaimAll {
        /// Recipient address (defaults to wallet address)
        #[arg(short, long)]
        recipient: Option<String>,
    },

    /// Show claim history
    History {
        /// Maximum number of entries to show
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Show current epoch info and BLOCKS_PER_REWARD_EPOCH
    Info,
}

#[derive(Subcommand)]
enum UpdateCommands {
    /// Check for available updates
    Check,

    /// Show pending update status, veto progress, deadlines
    Status,

    /// Vote on a pending update
    Vote {
        /// Version to vote on (e.g., "1.0.1")
        #[arg(long)]
        version: String,

        /// Cast a veto vote
        #[arg(long, conflicts_with = "approve")]
        veto: bool,

        /// Cast an approval vote
        #[arg(long, conflicts_with = "veto")]
        approve: bool,
    },

    /// Show current votes for a version
    Votes {
        /// Version to check (e.g., "1.0.1")
        #[arg(long)]
        version: String,
    },

    /// Manually apply an approved update
    Apply,

    /// Rollback to previous version
    Rollback,
}

#[derive(Subcommand)]
enum MaintainerCommands {
    /// List current maintainers
    List,
}

#[derive(Subcommand)]
enum ReleaseCommands {
    /// Sign a release (maintainer workflow)
    ///
    /// Signs the message "{version}:{sha256(CHECKSUMS.txt)}" with a producer key.
    /// Output is a JSON signature block for inclusion in SIGNATURES.json.
    Sign {
        /// Release version to sign (e.g., v1.0.27)
        #[arg(long)]
        version: String,

        /// Path to producer key file (overrides -w wallet)
        #[arg(long)]
        key: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ProtocolCommands {
    /// Sign a protocol activation (one maintainer at a time)
    ///
    /// Signs "activate:{version}:{epoch}" with a producer key.
    /// Output is a JSON signature block. Collect 3/5 and pass to `protocol activate`.
    Sign {
        /// Protocol version to activate (e.g., 2)
        #[arg(long)]
        version: u32,

        /// Epoch at which activation occurs
        #[arg(long)]
        epoch: u64,

        /// Path to producer key file (overrides -w wallet)
        #[arg(long)]
        key: Option<PathBuf>,
    },

    /// Submit a protocol activation transaction (requires 3/5 signatures)
    ///
    /// Reads signature blocks from a JSON file, builds the ProtocolActivation tx,
    /// and submits it to the network.
    Activate {
        /// Protocol version to activate (e.g., 2)
        #[arg(long)]
        version: u32,

        /// Epoch at which activation occurs
        #[arg(long)]
        epoch: u64,

        /// Description of consensus changes
        #[arg(long)]
        description: String,

        /// Path to JSON file containing collected signatures
        #[arg(long)]
        signatures: PathBuf,
    },
}

/// Expand `~` or `~/...` to the user's home directory.
/// Shell tilde expansion doesn't happen inside Rust — clap default values
/// like `~/.doli/wallet.json` arrive as literal strings.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest)
    } else if path == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    } else {
        PathBuf::from(path)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let wallet = expand_tilde(&cli.wallet);

    // Set address prefix from network
    let _ = ADDRESS_PREFIX.set(prefix_for_network(&cli.network).to_string());

    // Resolve RPC endpoint: explicit flag > network default
    let rpc_endpoint = cli
        .rpc
        .unwrap_or_else(|| default_rpc_for_network(&cli.network).to_string());

    match cli.command {
        Commands::New { name } => {
            cmd_new(&wallet, name)?;
        }
        Commands::Restore { name } => {
            cmd_restore(&wallet, name)?;
        }
        Commands::Address { label } => {
            cmd_address(&wallet, label)?;
        }
        Commands::Addresses => {
            cmd_addresses(&wallet)?;
        }
        Commands::Balance { address } => {
            cmd_balance(&wallet, &rpc_endpoint, address).await?;
        }
        Commands::Send { to, amount, fee } => {
            cmd_send(&wallet, &rpc_endpoint, &to, &amount, fee).await?;
        }
        Commands::History { limit } => {
            cmd_history(&wallet, &rpc_endpoint, limit).await?;
        }
        Commands::Export { output } => {
            cmd_export(&wallet, &output)?;
        }
        Commands::Import { input } => {
            cmd_import(&wallet, &input)?;
        }
        Commands::Info => {
            cmd_info(&wallet)?;
        }
        Commands::AddBls => {
            cmd_add_bls(&wallet)?;
        }
        Commands::Sign { message, address } => {
            cmd_sign(&wallet, &message, address)?;
        }
        Commands::Verify {
            message,
            signature,
            pubkey,
        } => {
            cmd_verify(&message, &signature, &pubkey)?;
        }
        Commands::Producer { command } => {
            cmd_producer(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Rewards { command } => {
            cmd_rewards(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Chain => {
            cmd_chain(&rpc_endpoint).await?;
        }
        Commands::Update { command } => {
            cmd_update(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Maintainer { command } => {
            cmd_maintainer(&rpc_endpoint, command).await?;
        }
        Commands::Upgrade {
            version,
            yes,
            doli_node_path,
            service,
        } => {
            cmd_upgrade(version, yes, doli_node_path, service).await?;
        }
        Commands::Release { command } => {
            cmd_release(&wallet, command).await?;
        }
        Commands::Protocol { command } => {
            cmd_protocol(&wallet, &rpc_endpoint, command).await?;
        }
        Commands::Wipe {
            network,
            data_dir,
            yes,
        } => {
            cmd_wipe(&network, data_dir, yes)?;
        }
    }

    Ok(())
}

/// Address prefix resolved from --network flag at startup.
static ADDRESS_PREFIX: OnceLock<String> = OnceLock::new();

fn address_prefix() -> &'static str {
    ADDRESS_PREFIX.get().map(|s| s.as_str()).unwrap_or("doli")
}

fn prefix_for_network(network: &str) -> &'static str {
    match network {
        "testnet" => "tdoli",
        "devnet" => "ddoli",
        _ => "doli",
    }
}

fn default_rpc_for_network(network: &str) -> &'static str {
    match network {
        "testnet" => "http://127.0.0.1:18545",
        "devnet" => "http://127.0.0.1:28545",
        _ => "http://127.0.0.1:8545",
    }
}

fn cmd_new(wallet_path: &PathBuf, name: Option<String>) -> Result<()> {
    if wallet_path.exists() {
        anyhow::bail!(
            "Wallet already exists at {:?}. Use a different path with -w.",
            wallet_path
        );
    }

    let name = name.unwrap_or_else(|| "default".to_string());

    let (wallet, phrase) = Wallet::new(&name);
    wallet.save(wallet_path)?;

    // Write seed phrase to a separate file
    let seed_path = wallet_path.with_extension("seed.txt");
    let mut seed_content = String::new();
    let words: Vec<&str> = phrase.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        seed_content.push_str(&format!("{}. {}\n", i + 1, word));
    }
    std::fs::write(&seed_path, &seed_content)?;

    let bech32_addr = wallet.primary_bech32_address(address_prefix());

    println!();
    println!("  Your wallet has been created.");
    println!();
    println!("  Recovery phrase:");
    println!();
    for (i, chunk) in words.chunks(6).enumerate() {
        let numbered: Vec<String> = chunk
            .iter()
            .enumerate()
            .map(|(j, w)| format!("{:>2}. {}", i * 6 + j + 1, w))
            .collect();
        println!("    {}", numbered.join("  "));
    }
    println!();
    println!("  Address: {}", bech32_addr);
    println!();
    println!("  Wallet saved to: {:?}", wallet_path);
    println!("  Seed phrase saved to: {:?}", seed_path);
    println!();
    println!("  WARNING: This is the ONLY time your recovery phrase is shown.");
    println!("  If you lose both the wallet file and these 24 words, your DOLI is gone forever.");
    println!("  Write them down on paper and store in a safe place.");
    println!("  Then delete the seed file:");
    println!("    rm {}", seed_path.display());
    println!();

    Ok(())
}

fn cmd_restore(wallet_path: &PathBuf, name: Option<String>) -> Result<()> {
    if wallet_path.exists() {
        anyhow::bail!(
            "Wallet already exists at {:?}. Use a different path with -w.",
            wallet_path
        );
    }

    let name = name.unwrap_or_else(|| "default".to_string());

    // Read phrase from stdin to avoid shell history exposure
    println!("Enter your 24-word recovery phrase:");
    let mut phrase = String::new();
    std::io::stdin().read_line(&mut phrase)?;
    let phrase = phrase.trim();

    if phrase.is_empty() {
        anyhow::bail!("No seed phrase provided.");
    }

    let wallet = Wallet::from_seed_phrase(&name, phrase)?;
    wallet.save(wallet_path)?;

    let bech32_addr = wallet.primary_bech32_address(address_prefix());

    println!();
    println!("  Wallet restored successfully.");
    println!();
    println!("  Address: {}", bech32_addr);
    println!("  Wallet saved to: {:?}", wallet_path);
    println!();

    Ok(())
}

fn cmd_address(wallet_path: &Path, label: Option<String>) -> Result<()> {
    let mut wallet = Wallet::load(wallet_path)?;

    let _hex_address = wallet.generate_address(label.as_deref())?;
    wallet.save(wallet_path)?;

    // Display bech32m for the newly generated address (last in list)
    let new_addr = wallet.addresses().last().expect("just added");
    let pubkey_bytes = hex::decode(&new_addr.public_key)?;
    let bech32 = crypto::address::from_pubkey(&pubkey_bytes, address_prefix())
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("New address: {}", bech32);

    Ok(())
}

fn cmd_addresses(wallet_path: &Path) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;

    println!("Addresses:");
    for (i, addr) in wallet.addresses().iter().enumerate() {
        let label = addr.label.as_deref().unwrap_or("");
        let pubkey_bytes = hex::decode(&addr.public_key)?;
        let bech32 = crypto::address::from_pubkey(&pubkey_bytes, address_prefix())
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        let label_str = if label.is_empty() {
            String::new()
        } else {
            format!(" ({})", label)
        };
        println!("  {}. {}{}", i + 1, bech32, label_str);
    }

    Ok(())
}

async fn cmd_balance(
    wallet_path: &Path,
    rpc_endpoint: &str,
    address: Option<String>,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection first
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}. Make sure a DOLI node is running and the RPC endpoint is correct.", rpc_endpoint);
    }

    let mut total_spendable = 0u64;
    let mut total_bonded = 0u64;
    let mut total_immature = 0u64;
    let mut total_unconfirmed = 0u64;
    let mut total_activating = 0u64;

    // Build pubkey_hash → bonded_value map from producer set.
    // Bonds live in ProducerSet (consumed on registration), not as UTXOs.
    // Use bond_amount from ProducerSet — the actual amount locked on-chain.
    let (bond_map, pending_activation_map): (
        std::collections::HashMap<String, u64>,
        std::collections::HashMap<String, u64>,
    ) = match rpc.get_producers(false).await {
        Ok(producers) => {
            let bonds = producers
                .iter()
                .filter(|p| p.bond_amount > 0 && p.status == "active")
                .filter_map(|p| {
                    let pubkey_bytes = hex::decode(&p.public_key).ok()?;
                    let pubkey_hash =
                        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &pubkey_bytes);
                    Some((pubkey_hash.to_hex(), p.bond_amount))
                })
                .collect();
            let pending = producers
                .iter()
                .filter(|p| p.bond_amount > 0 && p.status == "pending")
                .filter_map(|p| {
                    let pubkey_bytes = hex::decode(&p.public_key).ok()?;
                    let pubkey_hash =
                        crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &pubkey_bytes);
                    Some((pubkey_hash.to_hex(), p.bond_amount))
                })
                .collect();
            (bonds, pending)
        }
        Err(_) => (
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ),
    };

    println!("Balances:");
    println!("{:-<60}", "");

    // Collect addresses to query
    let addresses: Vec<(String, String)> = if let Some(addr) = &address {
        let pubkey_hash =
            crypto::address::resolve(addr, None).map_err(|e| anyhow::anyhow!("{}", e))?;
        let pubkey_hash_hex = pubkey_hash.to_hex();
        let display_addr = crypto::address::encode(&pubkey_hash, address_prefix())
            .unwrap_or_else(|_| pubkey_hash_hex.clone());
        vec![(pubkey_hash_hex, display_addr)]
    } else {
        wallet
            .addresses()
            .iter()
            .filter_map(|wallet_addr| {
                let pubkey_bytes = hex::decode(&wallet_addr.public_key).ok()?;
                let pubkey_hash =
                    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &pubkey_bytes);
                let pubkey_hash_hex = pubkey_hash.to_hex();
                let label = wallet_addr.label.as_deref().unwrap_or("");
                let bech32 = crypto::address::encode(&pubkey_hash, address_prefix())
                    .unwrap_or_else(|_| pubkey_hash_hex.clone());
                let display = if !label.is_empty() {
                    format!("{} ({})", bech32, label)
                } else {
                    bech32
                };
                Some((pubkey_hash_hex, display))
            })
            .collect()
    };

    for (pubkey_hash_hex, display_addr) in &addresses {
        match rpc.get_balance(pubkey_hash_hex).await {
            Ok(balance) => {
                // Bonds live in ProducerSet (consumed on registration), not as UTXOs.
                // So `confirmed` (spendable UTXOs) does NOT include bonds.
                let bonded = bond_map.get(pubkey_hash_hex.as_str()).copied().unwrap_or(0);
                let activating = pending_activation_map
                    .get(pubkey_hash_hex.as_str())
                    .copied()
                    .unwrap_or(0);
                let spendable = balance.confirmed;
                let total = balance
                    .total
                    .saturating_add(bonded)
                    .saturating_add(activating);

                println!("{}", display_addr);
                println!("  Spendable: {}", format_balance(spendable));
                if bonded > 0 {
                    println!("  Bonded:    {}  (producer bond)", format_balance(bonded));
                }
                if activating > 0 {
                    println!(
                        "  Activating: {}  (pending epoch)",
                        format_balance(activating)
                    );
                }
                if balance.immature > 0 {
                    println!("  Immature:  {}", format_balance(balance.immature));
                }
                if balance.unconfirmed > 0 {
                    println!("  Pending:   {}", format_balance(balance.unconfirmed));
                }
                println!("  Total:     {}", format_balance(total));
                println!();

                total_spendable += spendable;
                total_bonded += bonded;
                total_activating += activating;
                total_immature += balance.immature;
                total_unconfirmed += balance.unconfirmed;
            }
            Err(e) => {
                println!("{}: Error - {}", display_addr, e);
            }
        }
    }

    if address.is_none() && addresses.len() > 1 {
        let grand_total =
            total_spendable + total_bonded + total_activating + total_immature + total_unconfirmed;
        println!("{:-<60}", "");
        println!("Total Spendable: {}", format_balance(total_spendable));
        if total_bonded > 0 {
            println!("Total Bonded:    {}", format_balance(total_bonded));
        }
        if total_activating > 0 {
            println!("Total Activating: {}", format_balance(total_activating));
        }
        if total_immature > 0 {
            println!("Total Immature:  {}", format_balance(total_immature));
        }
        if total_unconfirmed > 0 {
            println!("Total Pending:   {}", format_balance(total_unconfirmed));
        }
        println!("Total:           {}", format_balance(grand_total));
    }

    Ok(())
}

async fn cmd_send(
    wallet_path: &Path,
    rpc_endpoint: &str,
    to: &str,
    amount: &str,
    fee: Option<String>,
) -> Result<()> {
    use crypto::{signature, Hash};
    use doli_core::{Input, Output, Transaction};

    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    // Parse recipient address (doli1... or 64-char hex pubkey_hash)
    let recipient_hash = crypto::address::resolve(to, None)
        .map_err(|e| anyhow::anyhow!("Invalid recipient address: {}", e))?;

    // Parse amount
    let amount_coins: f64 = amount
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid amount: {}", amount))?;

    if amount_coins <= 0.0 {
        anyhow::bail!("Amount must be greater than zero");
    }
    if amount_coins > 25_200_000.0 {
        anyhow::bail!("Amount exceeds maximum supply (25.2M DOLI)");
    }

    let amount_units = coins_to_units(amount_coins);

    // Parse explicit fee if provided; otherwise auto-calculate after UTXO selection
    let explicit_fee: Option<u64> = if let Some(f) = &fee {
        let fee_coins: f64 = f
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid fee: {}", f))?;
        Some(coins_to_units(fee_coins))
    } else {
        None
    };

    let recipient_display = crypto::address::encode(&recipient_hash, address_prefix())
        .unwrap_or_else(|_| recipient_hash.to_hex());

    // Get the sender's pubkey_hash for UTXO lookup
    let from_pubkey_hash = wallet.primary_pubkey_hash();

    // Get spendable UTXOs
    let utxos = rpc.get_utxos(&from_pubkey_hash, true).await?;

    if utxos.is_empty() {
        anyhow::bail!("No spendable UTXOs available. Note: Coinbase outputs require {} confirmations before they can be spent.", doli_core::consensus::COINBASE_MATURITY);
    }

    // Select UTXOs with a preliminary fee estimate, then recalculate
    let preliminary_fee = explicit_fee.unwrap_or(1000);
    let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

    let mut selected_utxos = Vec::new();
    let mut total_input = 0u64;
    for utxo in &utxos {
        if total_input >= amount_units + preliminary_fee {
            break;
        }
        selected_utxos.push(utxo.clone());
        total_input += utxo.amount;
    }

    // Auto-calculate fee: max(1000, inputs * 500)
    let fee_units = explicit_fee.unwrap_or_else(|| 1000u64.max(selected_utxos.len() as u64 * 500));

    // Re-select if auto fee increased the requirement
    if explicit_fee.is_none() && total_input < amount_units + fee_units {
        selected_utxos.clear();
        total_input = 0;
        for utxo in &utxos {
            if total_input >= amount_units + fee_units {
                break;
            }
            selected_utxos.push(utxo.clone());
            total_input += utxo.amount;
        }
    }

    let required = amount_units + fee_units;

    if total_available < required {
        anyhow::bail!(
            "Insufficient balance. Available: {}, Required: {}",
            format_balance(total_available),
            format_balance(required)
        );
    }

    if total_input < required {
        anyhow::bail!(
            "Insufficient balance. Selected: {}, Required: {}",
            format_balance(total_input),
            format_balance(required)
        );
    }

    println!("Preparing transaction:");
    println!("  To:     {}", recipient_display);
    println!("  Amount: {} DOLI", amount_coins);
    println!("  Fee:    {}", format_balance(fee_units));

    println!();
    println!("Using {} UTXO(s):", selected_utxos.len());
    for utxo in &selected_utxos {
        println!(
            "  {}:{} - {}",
            &utxo.tx_hash[..16],
            utxo.output_index,
            format_balance(utxo.amount)
        );
    }

    // Build transaction inputs (without signatures yet)
    let mut inputs: Vec<Input> = Vec::new();
    for utxo in &selected_utxos {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
        inputs.push(Input::new(prev_tx_hash, utxo.output_index));
    }

    // Build transaction outputs
    let mut outputs: Vec<Output> = Vec::new();

    // Recipient output
    outputs.push(Output::normal(amount_units, recipient_hash));

    // Change output (if needed)
    let change = total_input - required;
    if change > 0 {
        let change_hash = Hash::from_hex(&from_pubkey_hash)
            .ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
        outputs.push(Output::normal(change, change_hash));
        println!("  Change: {}", format_balance(change));
    }

    // Create unsigned transaction
    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign each input
    let keypair = wallet.primary_keypair()?;
    let signing_message = tx.signing_message();

    for input in &mut tx.inputs {
        input.signature = signature::sign_hash(&signing_message, keypair.private_key());
    }

    // Serialize transaction
    let tx_bytes = tx.serialize();
    let tx_hex = hex::encode(&tx_bytes);
    let tx_hash = tx.hash();

    println!();
    println!("Transaction hash: {}", tx_hash.to_hex());
    println!("Transaction size: {} bytes", tx_bytes.len());

    // Submit to network
    println!();
    println!("Broadcasting transaction...");
    match rpc.send_transaction(&tx_hex).await {
        Ok(result_hash) => {
            println!("Transaction submitted successfully!");
            println!("TX Hash: {}", result_hash);
        }
        Err(e) => {
            println!("Error submitting transaction: {}", e);
            return Err(anyhow::anyhow!("Transaction submission failed: {}", e));
        }
    }

    Ok(())
}

async fn cmd_history(wallet_path: &Path, rpc_endpoint: &str, limit: usize) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    println!("Transaction History (limit: {})", limit);
    println!("{:-<80}", "");

    // Use pubkey_hash instead of address for RPC call
    let primary_pubkey_hash = wallet.primary_pubkey_hash();

    match rpc.get_history(&primary_pubkey_hash, limit).await {
        Ok(entries) => {
            if entries.is_empty() {
                println!("No transactions found.");
            } else {
                for entry in entries {
                    let status = if entry.confirmations > 0 {
                        format!("{} confirmations", entry.confirmations)
                    } else {
                        "pending".to_string()
                    };

                    // Capitalize first letter of tx_type
                    let tx_type = entry
                        .tx_type
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default()
                        + &entry.tx_type.chars().skip(1).collect::<String>();

                    println!("Hash:     {}", entry.hash);
                    println!("Type:     {}", tx_type);
                    println!("Block:    {} ({})", entry.height, status);
                    if entry.amount_received > 0 {
                        println!("Received: +{}", format_balance(entry.amount_received));
                    }
                    if entry.amount_sent > 0 {
                        println!("Sent:     -{}", format_balance(entry.amount_sent));
                    }
                    if entry.fee > 0 {
                        println!("Fee:      {}", format_balance(entry.fee));
                    }
                    println!();
                }
            }
        }
        Err(e) => {
            anyhow::bail!("Error fetching history: {}", e);
        }
    }

    Ok(())
}

fn cmd_export(wallet_path: &Path, output: &PathBuf) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    wallet.export(output)?;

    println!("Wallet exported to: {:?}", output);

    Ok(())
}

fn cmd_import(wallet_path: &PathBuf, input: &PathBuf) -> Result<()> {
    let wallet = Wallet::import(input)?;
    wallet.save(wallet_path)?;

    println!("Wallet imported from: {:?}", input);
    println!("Saved to: {:?}", wallet_path);

    Ok(())
}

fn cmd_info(wallet_path: &Path) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;

    let bech32_addr = wallet.primary_bech32_address(address_prefix());

    println!("Wallet: {}", wallet.name());
    println!("Addresses: {}", wallet.addresses().len());
    println!();
    println!("Primary Address:");
    println!("  Address:    {}", bech32_addr);
    println!("  Public Key: {}", wallet.primary_public_key());
    if let Some(bls_pub) = wallet.primary_bls_public_key() {
        println!("  BLS Key:    {}", bls_pub);
    } else {
        println!("  BLS Key:    none (run 'doli add-bls' to generate)");
    }
    println!();
    println!("Use the address above for sending and receiving DOLI.");

    Ok(())
}

fn cmd_add_bls(wallet_path: &Path) -> Result<()> {
    let mut wallet = Wallet::load(wallet_path)?;

    if wallet.has_bls_key() {
        println!("BLS key already exists in this wallet.");
        println!(
            "  BLS Public Key: {}",
            wallet.primary_bls_public_key().unwrap_or("?")
        );
        return Ok(());
    }

    let bls_pub = wallet.add_bls_key()?;
    wallet.save(wallet_path)?;

    println!("BLS attestation key added.");
    println!("  BLS Public Key: {}", bls_pub);
    println!();
    println!("Restart the node to load the new BLS key.");

    Ok(())
}

fn cmd_sign(wallet_path: &Path, message: &str, address: Option<String>) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;

    let signature = wallet.sign_message(message, address.as_deref())?;

    println!("Message: {}", message);
    println!("Signature: {}", signature);

    Ok(())
}

fn cmd_verify(message: &str, signature: &str, pubkey: &str) -> Result<()> {
    let valid = wallet::verify_message(message, signature, pubkey)?;

    println!("Message: {}", message);
    println!("Signature: {}", signature);
    println!("Public Key: {}", pubkey);
    println!("Valid: {}", valid);

    Ok(())
}

async fn cmd_upgrade(
    version: Option<String>,
    yes: bool,
    doli_node_path: Option<std::path::PathBuf>,
    service: Option<String>,
) -> Result<()> {
    let current = updater::current_version();
    println!("Current version: v{}", current);
    println!("Checking for updates...");

    let release = updater::fetch_github_release(version.as_deref())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch release: {}", e))?;

    if !updater::is_newer_version(&release.version, current) {
        if let Some(ref svc) = service {
            // Binary already updated (e.g. by a prior run on this server),
            // but the caller wants a specific service restarted.
            println!(
                "Binary already at v{}, restarting service: {}",
                current, svc
            );
            restart_specific_service(svc);
            return Ok(());
        }
        println!("Already up to date (v{}).", current);
        return Ok(());
    }

    println!();
    println!(
        "New version available: v{} -> v{}",
        current, release.version
    );
    if !release.changelog.is_empty() {
        println!();
        // Show first 20 lines of changelog
        for line in release.changelog.lines().take(20) {
            println!("  {}", line);
        }
        println!();
    }

    if !yes {
        print!("Proceed with upgrade? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Upgrade cancelled.");
            return Ok(());
        }
    }

    // Download tarball
    println!("Downloading v{}...", release.version);
    let tarball = updater::download_from_url(&release.tarball_url)
        .await
        .map_err(|e| anyhow::anyhow!("Download failed: {}", e))?;

    // Verify hash
    println!("Verifying checksum...");
    updater::verify_hash(&tarball, &release.expected_hash)
        .map_err(|e| anyhow::anyhow!("Hash verification failed: {}", e))?;
    println!("Checksum OK.");

    // Check maintainer signatures (informational — never blocks manual upgrade)
    match updater::download_signatures_json(&release.version).await {
        Ok(Some(sf)) => {
            let sig_release = updater::Release {
                version: sf.version.clone(),
                binary_sha256: sf.checksums_sha256.clone(),
                signatures: sf.signatures,
                binary_url_template: String::new(),
                changelog: String::new(),
                published_at: 0,
                target_networks: vec![],
            };
            match updater::verify_release_signatures(&sig_release, doli_core::Network::Mainnet) {
                Ok(()) => {
                    println!("Verified: 3/5 maintainer signatures on CHECKSUMS.txt");
                }
                Err(updater::UpdateError::InsufficientSignatures { found, required }) => {
                    println!(
                        "Warning: only {}/{} required maintainer signatures found",
                        found, required
                    );
                }
                Err(e) => {
                    println!("Warning: signature verification failed: {}", e);
                }
            }
        }
        Ok(None) => {
            println!("Note: no maintainer signatures (SIGNATURES.json not found)");
        }
        Err(e) => {
            println!("Note: could not check signatures: {}", e);
        }
    }

    // Extract and install doli (CLI binary — ourselves)
    let cli_binary = updater::extract_named_binary_from_tarball(&tarball, "doli")
        .map_err(|e| anyhow::anyhow!("Failed to extract doli binary: {}", e))?;
    let cli_path = std::env::current_exe()?;
    println!("Installing doli to {:?}...", cli_path);
    if let Err(e) = updater::install_binary(&cli_binary, &cli_path).await {
        if e.to_string().contains("Permission denied") || e.to_string().contains("os error 13") {
            return Err(anyhow::anyhow!(
                "Permission denied writing to {:?}.\n  Try: sudo doli upgrade{}",
                cli_path,
                if yes { " --yes" } else { "" }
            ));
        }
        return Err(anyhow::anyhow!("Failed to install doli: {}", e));
    }

    // Extract and install doli-node (if found in tarball)
    let mut installed_node_path: Option<std::path::PathBuf> = None;
    match updater::extract_named_binary_from_tarball(&tarball, "doli-node") {
        Ok(node_binary) => {
            // Use custom path if provided, otherwise auto-detect
            let node_path = doli_node_path.or_else(find_doli_node_path);
            if let Some(path) = node_path {
                println!("Installing doli-node to {:?}...", path);
                if let Err(e) = updater::install_binary(&node_binary, &path).await {
                    if e.to_string().contains("Permission denied")
                        || e.to_string().contains("os error 13")
                    {
                        return Err(anyhow::anyhow!(
                            "Permission denied writing to {:?}.\n  Try: sudo doli upgrade{}",
                            path,
                            if yes { " --yes" } else { "" }
                        ));
                    }
                    return Err(anyhow::anyhow!("Failed to install doli-node: {}", e));
                }
                installed_node_path = Some(path);
            } else {
                println!("doli-node not found on system, skipping node binary install.");
                println!("  Hint: use --doli-node-path <PATH> to specify the doli-node location.");
            }
        }
        Err(_) => {
            println!("doli-node not in tarball, skipping node binary install.");
        }
    }

    // Restart only the service that owns the installed binary
    if let Some(ref svc) = service {
        restart_specific_service(svc);
    } else {
        restart_doli_service(installed_node_path.as_deref());
    }

    println!();
    println!("Upgrade to v{} complete!", release.version);

    Ok(())
}

async fn cmd_release(wallet_path: &Path, command: ReleaseCommands) -> Result<()> {
    match command {
        ReleaseCommands::Sign { version, key } => {
            cmd_release_sign(wallet_path, &version, key).await?;
        }
    }
    Ok(())
}

async fn cmd_release_sign(
    wallet_path: &Path,
    version: &str,
    key_path: Option<PathBuf>,
) -> Result<()> {
    // Load wallet/key
    let key_file = key_path.as_deref().unwrap_or(wallet_path);
    let w = Wallet::load(key_file)?;
    let keypair = w.primary_keypair()?;
    let pubkey_hex = keypair.public_key().to_hex();

    let version_bare = version.strip_prefix('v').unwrap_or(version);

    // Download CHECKSUMS.txt from GitHub and compute its SHA-256
    println!("Fetching CHECKSUMS.txt for v{}...", version_bare);
    let (_checksums_content, checksums_sha256) = updater::download_checksums_txt(version_bare)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch CHECKSUMS.txt: {}", e))?;

    // Sign "version:checksums_sha256"
    let sig = updater::sign_release_hash(&keypair, version_bare, &checksums_sha256);

    // Output JSON signature block
    let output = serde_json::json!({
        "public_key": sig.public_key,
        "signature": sig.signature
    });
    println!("{}", serde_json::to_string_pretty(&output)?);

    eprintln!();
    eprintln!(
        "Signed v{} with key {}...{}",
        version_bare,
        &pubkey_hex[..8],
        &pubkey_hex[pubkey_hex.len() - 8..]
    );
    eprintln!("Message: \"{}:{}\"", version_bare, checksums_sha256);
    eprintln!();
    eprintln!("To assemble SIGNATURES.json, collect 3+ signatures and upload:");
    eprintln!("  gh release upload v{} SIGNATURES.json", version_bare);

    Ok(())
}

async fn cmd_protocol(wallet_path: &Path, rpc_url: &str, command: ProtocolCommands) -> Result<()> {
    match command {
        ProtocolCommands::Sign {
            version,
            epoch,
            key,
        } => {
            cmd_protocol_sign(wallet_path, version, epoch, key)?;
        }
        ProtocolCommands::Activate {
            version,
            epoch,
            description,
            signatures,
        } => {
            cmd_protocol_activate(rpc_url, version, epoch, description, signatures).await?;
        }
    }
    Ok(())
}

fn cmd_protocol_sign(
    wallet_path: &Path,
    version: u32,
    epoch: u64,
    key_path: Option<PathBuf>,
) -> Result<()> {
    let key_file = key_path.as_deref().unwrap_or(wallet_path);
    let w = Wallet::load(key_file)?;
    let keypair = w.primary_keypair()?;
    let pubkey_hex = keypair.public_key().to_hex();

    // Build signing message: "activate:{version}:{epoch}"
    let message = format!("activate:{}:{}", version, epoch);
    let msg_bytes = message.as_bytes();

    // Sign with Ed25519
    let sig = crypto::signature::sign(msg_bytes, keypair.private_key());

    let output = serde_json::json!({
        "public_key": pubkey_hex,
        "signature": hex::encode(sig.as_bytes()),
    });
    println!("{}", serde_json::to_string_pretty(&output)?);

    eprintln!();
    eprintln!(
        "Signed protocol v{} activation at epoch {} with key {}...{}",
        version,
        epoch,
        &pubkey_hex[..8],
        &pubkey_hex[pubkey_hex.len() - 8..]
    );
    eprintln!("Message: \"{}\"", message);
    eprintln!();
    eprintln!("Collect 3/5 signatures into a JSON array file, then run:");
    eprintln!(
        "  doli protocol activate --version {} --epoch {} --description \"...\" --signatures sigs.json",
        version, epoch
    );

    Ok(())
}

async fn cmd_protocol_activate(
    rpc_url: &str,
    version: u32,
    epoch: u64,
    description: String,
    signatures_path: PathBuf,
) -> Result<()> {
    use crypto::{PublicKey, Signature};
    use doli_core::maintainer::{MaintainerSignature, ProtocolActivationData};
    use doli_core::Transaction;

    println!("Protocol Activation");
    println!("{:-<60}", "");
    println!("  Version: {}", version);
    println!("  Epoch:   {}", epoch);
    println!("  Desc:    {}", description);
    println!();

    // Read signatures file
    let sigs_json = std::fs::read_to_string(&signatures_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", signatures_path.display(), e))?;

    let sigs: Vec<serde_json::Value> = serde_json::from_str(&sigs_json)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in signatures file: {}", e))?;

    if sigs.len() < 3 {
        anyhow::bail!(
            "Need at least 3 signatures, got {}. Collect more with `doli protocol sign`.",
            sigs.len()
        );
    }

    // Parse signatures
    let mut maintainer_sigs = Vec::new();
    for (i, sig_val) in sigs.iter().enumerate() {
        let pk_hex = sig_val["public_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Signature {} missing public_key", i))?;
        let sig_hex = sig_val["signature"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Signature {} missing signature", i))?;

        let pubkey = PublicKey::from_hex(pk_hex)
            .map_err(|e| anyhow::anyhow!("Invalid public key in signature {}: {}", i, e))?;
        let sig_bytes =
            hex::decode(sig_hex).map_err(|e| anyhow::anyhow!("Invalid sig hex {}: {}", i, e))?;
        let signature = Signature::try_from_slice(&sig_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid signature {}: {}", i, e))?;

        println!(
            "  Sig {}: {}...{}",
            i + 1,
            &pk_hex[..8],
            &pk_hex[pk_hex.len() - 8..]
        );
        maintainer_sigs.push(MaintainerSignature::new(pubkey, signature));
    }
    println!();

    // Build ProtocolActivationData
    let data = ProtocolActivationData::new(version, epoch, description, maintainer_sigs);

    // Verify signing message matches what signers signed
    let expected_msg = data.signing_message();
    println!(
        "Signing message: \"{}\"",
        String::from_utf8_lossy(&expected_msg)
    );

    // Build transaction
    let tx = Transaction::new_protocol_activation(data);
    let tx_hex = hex::encode(tx.serialize());

    println!("Submitting ProtocolActivation transaction...");

    let rpc = RpcClient::new(rpc_url);
    match rpc.send_transaction(&tx_hex).await {
        Ok(hash) => {
            println!();
            println!("Protocol activation submitted!");
            println!("TX Hash: {}", hash);
            println!();
            println!(
                "All nodes will activate protocol v{} at epoch {}.",
                version, epoch
            );
        }
        Err(e) => {
            anyhow::bail!("Error submitting activation: {}", e);
        }
    }

    Ok(())
}

/// Find the path to an installed doli-node binary
fn find_doli_node_path() -> Option<std::path::PathBuf> {
    // Tier 1: Check running doli-node process to find its binary path
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = std::process::Command::new("pgrep")
            .args(["-a", "doli-node"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Take the first match's binary path (first token after PID)
                if let Some(line) = stdout.lines().next() {
                    if let Some(bin_path) = line.split_whitespace().nth(1) {
                        let p = std::path::PathBuf::from(bin_path);
                        if p.exists() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }

    // Tier 2: Check `which doli-node`
    if let Ok(output) = std::process::Command::new("which")
        .arg("doli-node")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(std::path::PathBuf::from(path));
            }
        }
    }

    // Tier 3: Check common install paths (most specific first)
    for path in &[
        "/mainnet/bin/doli-node",
        "/testnet/bin/doli-node",
        "/usr/local/bin/doli-node",
        "/opt/doli/target/release/doli-node",
    ] {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    None
}

/// Restart a specific systemd service by name
fn restart_specific_service(service: &str) {
    println!("Restarting service: {}", service);
    let result = std::process::Command::new("sudo")
        .args(["systemctl", "restart", service])
        .status();
    match result {
        Ok(s) if s.success() => println!("  Restarted {}.", service),
        _ => println!(
            "  Failed to restart {}. Run: sudo systemctl restart {}",
            service, service
        ),
    }
}

/// Detect and restart the doli-node service that owns the installed binary.
/// If `installed_path` is provided, only restart the service whose ExecStart
/// references that path. Otherwise fall back to process detection.
fn restart_doli_service(#[allow(unused_variables)] installed_path: Option<&std::path::Path>) {
    // Tier 1: systemd (Linux) — find and restart only the owning service
    #[cfg(target_os = "linux")]
    {
        if try_restart_systemd(installed_path) {
            return;
        }
    }

    // Tier 2: launchd (macOS) — launchctl list shows all loaded agents
    #[cfg(target_os = "macos")]
    {
        if try_restart_launchd() {
            return;
        }
    }

    // Tier 3: Process fallback (all platforms)
    if try_restart_process() {
        return;
    }

    println!("No doli-node service or process found. Restart manually if needed.");
}

/// Tier 1: Detect and restart doli-node via systemd.
/// If `installed_path` is provided, only restart the service whose ExecStart
/// references that binary — prevents restarting unrelated services on multi-node servers.
#[cfg(target_os = "linux")]
fn try_restart_systemd(installed_path: Option<&std::path::Path>) -> bool {
    let output = match std::process::Command::new("systemctl")
        .args(["list-unit-files", "--type=service", "--no-pager"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let all_units: Vec<String> = stdout
        .lines()
        .filter(|line| line.contains("doli"))
        .filter_map(|line| line.split_whitespace().next().map(|s| s.to_string()))
        .collect();

    if all_units.is_empty() {
        return false;
    }

    // Filter to only services whose ExecStart matches the installed binary path
    let units: Vec<String> = if let Some(bin_path) = installed_path {
        let bin_str = bin_path.to_string_lossy();
        all_units
            .into_iter()
            .filter(|unit| {
                // Read the service's ExecStart to check if it uses our binary
                if let Ok(cat_output) = std::process::Command::new("systemctl")
                    .args(["cat", unit, "--no-pager"])
                    .output()
                {
                    let cat_str = String::from_utf8_lossy(&cat_output.stdout);
                    cat_str.contains(bin_str.as_ref())
                } else {
                    false
                }
            })
            .collect()
    } else {
        // No path context — only restart services that are currently running
        all_units
            .into_iter()
            .filter(|unit| {
                if let Ok(is_active) = std::process::Command::new("systemctl")
                    .args(["is-active", "--quiet", unit])
                    .status()
                {
                    is_active.success()
                } else {
                    false
                }
            })
            .collect()
    };

    if units.is_empty() {
        return false;
    }

    let mut any_ok = false;
    for unit in &units {
        println!("Restarting service: {}", unit);
        let result = std::process::Command::new("sudo")
            .args(["systemctl", "restart", unit])
            .status();
        match result {
            Ok(s) if s.success() => {
                println!("  Restarted {}.", unit);
                any_ok = true;
            }
            _ => println!(
                "  Failed to restart {}. Run: sudo systemctl restart {}",
                unit, unit
            ),
        }
    }
    any_ok
}

/// Tier 2: Detect and restart doli-node via launchd
#[cfg(target_os = "macos")]
fn try_restart_launchd() -> bool {
    let output = match std::process::Command::new("launchctl")
        .args(["list"])
        .output()
    {
        Ok(o) => o,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let services: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("doli"))
        .filter_map(|line| line.split_whitespace().last())
        .collect();

    if services.is_empty() {
        return false;
    }

    let mut any_ok = false;
    for label in &services {
        println!("Restarting service: {}", label);
        let result = std::process::Command::new("launchctl")
            .args(["kickstart", "-k", &format!("gui/{}/{}", get_uid(), label)])
            .status();
        match result {
            Ok(s) if s.success() => {
                println!("  Restarted {}.", label);
                any_ok = true;
            }
            _ => {
                // Fallback: stop + start
                let _ = std::process::Command::new("launchctl")
                    .args(["stop", label])
                    .status();
                let _ = std::process::Command::new("launchctl")
                    .args(["start", label])
                    .status();
                println!("  Restarted {} (stop+start).", label);
                any_ok = true;
            }
        }
    }
    any_ok
}

/// Tier 3: Detect doli-node process via pgrep and restart it
fn try_restart_process() -> bool {
    // pgrep -a on Linux shows PID + full cmdline; pgrep -fl on macOS does the same
    let pgrep_args = if cfg!(target_os = "macos") {
        vec!["-fl", "doli-node"]
    } else {
        vec!["-a", "doli-node"]
    };

    let output = match std::process::Command::new("pgrep")
        .args(&pgrep_args)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return false;
    }

    let mut any_ok = false;
    for line in &lines {
        // Format: "<PID> <full command line>"
        let mut parts = line.splitn(2, char::is_whitespace);
        let pid_str = match parts.next() {
            Some(p) => p,
            None => continue,
        };
        let cmdline = match parts.next() {
            Some(c) => c.trim(),
            None => continue,
        };

        println!("Found doli-node process (PID {}): {}", pid_str, cmdline);
        println!("  Sending SIGTERM...");

        // Kill the process gracefully
        let kill_ok = std::process::Command::new("kill")
            .arg(pid_str)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !kill_ok {
            println!("  Failed to kill PID {}. May need sudo.", pid_str);
            continue;
        }

        // Wait for clean shutdown
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Respawn with the same command line, detached
        println!("  Respawning: {}", cmdline);
        let spawn_result = std::process::Command::new("sh")
            .args(["-c", &format!("nohup {} > /dev/null 2>&1 &", cmdline)])
            .status();

        match spawn_result {
            Ok(s) if s.success() => {
                println!("  Respawned doli-node.");
                any_ok = true;
            }
            _ => println!("  Failed to respawn. Start manually: {}", cmdline),
        }
    }
    any_ok
}

/// Get current user's UID (for launchctl kickstart)
#[cfg(target_os = "macos")]
fn get_uid() -> String {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "501".to_string())
}

/// Format a slot duration as human-readable time (SLOT_DURATION = 10s)
fn format_slot_duration(slots: u64) -> String {
    let seconds = slots * 10;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("~{}h {}m", hours, minutes)
    } else {
        format!("~{}m", minutes)
    }
}

/// FIFO breakdown tier: (count, penalty_pct, gross_amount, net_amount)
struct FifoBreakdown {
    total_net: u64,
    total_penalty: u64,
    tiers: Vec<(u32, u8, u64, u64)>,
}

/// Compute FIFO breakdown for withdrawing `count` bonds (oldest first)
fn compute_fifo_breakdown(details: &rpc_client::BondDetailsInfo, count: u32) -> FifoBreakdown {
    let mut total_net: u64 = 0;
    let mut total_penalty: u64 = 0;
    let mut tiers: Vec<(u32, u8, u64, u64)> = Vec::new();

    let mut current_tier_pct: Option<u8> = None;
    let mut tier_count: u32 = 0;
    let mut tier_gross: u64 = 0;
    let mut tier_net: u64 = 0;

    for entry in details.bonds.iter().take(count as usize) {
        let pct = entry.penalty_pct;
        let penalty = (entry.amount * pct as u64) / 100;
        let net = entry.amount - penalty;
        total_net += net;
        total_penalty += penalty;

        if current_tier_pct == Some(pct) {
            tier_count += 1;
            tier_gross += entry.amount;
            tier_net += net;
        } else {
            if let Some(prev_pct) = current_tier_pct {
                tiers.push((tier_count, prev_pct, tier_gross, tier_net));
            }
            current_tier_pct = Some(pct);
            tier_count = 1;
            tier_gross = entry.amount;
            tier_net = net;
        }
    }
    if let Some(pct) = current_tier_pct {
        tiers.push((tier_count, pct, tier_gross, tier_net));
    }

    FifoBreakdown {
        total_net,
        total_penalty,
        tiers,
    }
}

/// Display FIFO breakdown table
fn display_fifo_breakdown(breakdown: &FifoBreakdown) {
    for (cnt, pct, gross, net) in &breakdown.tiers {
        let tier_label = match pct {
            0 => "vested (0% penalty)".to_string(),
            p => format!("Q{} ({}% penalty)", (4 - p / 25), p),
        };
        println!(
            "  {} x {}: {} -> {} ({} burned)",
            cnt,
            tier_label,
            format_balance(*gross),
            format_balance(*net),
            format_balance(gross - net)
        );
    }
    if breakdown.tiers.len() > 1 {
        let total_gross = breakdown.total_net + breakdown.total_penalty;
        println!("  {:-<50}", "");
        println!(
            "  Total: {} -> {} ({} burned)",
            format_balance(total_gross),
            format_balance(breakdown.total_net),
            format_balance(breakdown.total_penalty)
        );
    }
}

async fn cmd_producer(
    wallet_path: &Path,
    rpc_endpoint: &str,
    command: ProducerCommands,
) -> Result<()> {
    use crypto::{signature, Hash, PublicKey};
    use doli_core::{Input, Output, Transaction};

    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    match command {
        ProducerCommands::Register { bonds } => {
            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let pubkey_hash = wallet.primary_pubkey_hash();

            println!("Producer Registration");
            println!("{:-<60}", "");
            println!();

            // Validate bond count (WHITEPAPER Section 6.3: max 1000 bonds)
            if !(1..=10000).contains(&bonds) {
                anyhow::bail!("Bond count must be between 1 and 10000");
            }

            // Check if already registered or pending (WHITEPAPER: "public key is not already registered")
            let pk_hex = &wallet.addresses()[0].public_key;
            if let Ok(info) = rpc.get_producer(pk_hex).await {
                match info.status.to_lowercase().as_str() {
                    "active" => {
                        anyhow::bail!("This key is already registered as an active producer (pubkey: {}, bonds: {}). Use 'doli producer add-bond' to increase your bond count.", pk_hex, info.bond_count);
                    }
                    "pending" => {
                        anyhow::bail!("This key already has a pending registration (pubkey: {}). It will activate at the next epoch boundary.", pk_hex);
                    }
                    _ => {} // exited/slashed — allow re-registration
                }
            }

            // Get network parameters from node (bond_unit is network-specific)
            let chain_info = rpc.get_chain_info().await?;
            let network_params = rpc.get_network_params().await?;
            let bond_unit = network_params.bond_unit;
            let bond_display = bond_unit / 100_000_000; // Convert to DOLI per bond
            let required_amount = bond_unit * bonds as u64;

            println!(
                "Registering with {} bond(s) = {} DOLI",
                bonds,
                bonds as u64 * bond_display
            );
            println!();

            // Get spendable UTXOs
            let utxos = rpc.get_utxos(&pubkey_hash, true).await?;
            let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

            // Initial UTXO selection with minimum fee estimate
            let mut selected_utxos = Vec::new();
            let mut total_input = 0u64;
            let mut fee = 1000u64.max(utxos.len() as u64 * 500);
            for utxo in &utxos {
                if total_input >= required_amount + fee {
                    break;
                }
                selected_utxos.push(utxo.clone());
                total_input += utxo.amount;
            }

            // Auto-calculate fee: max(1000, inputs * 500) — same as send
            fee = 1000u64.max(selected_utxos.len() as u64 * 500);

            // Re-select if fee increased the requirement
            if total_input < required_amount + fee {
                selected_utxos.clear();
                total_input = 0;
                for utxo in &utxos {
                    selected_utxos.push(utxo.clone());
                    total_input += utxo.amount;
                    fee = 1000u64.max(selected_utxos.len() as u64 * 500);
                    if total_input >= required_amount + fee {
                        break;
                    }
                }
            }

            if total_available < required_amount + fee {
                anyhow::bail!(
                    "Insufficient balance for bond. Required: {} DOLI + {} fee, Available: {}",
                    bonds as u64 * bond_display,
                    format_balance(fee),
                    format_balance(total_available)
                );
            }

            // Build registration transaction
            let mut inputs: Vec<Input> = Vec::new();
            for utxo in &selected_utxos {
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
                inputs.push(Input::new(prev_tx_hash, utxo.output_index));
            }

            // Parse public key for registration
            let pubkey_bytes = hex::decode(&wallet.addresses()[0].public_key)?;
            let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

            // Calculate lock_until based on network
            // Lock must be at least current_height + blocks_per_era
            let blocks_per_era: u64 = match chain_info.network.as_str() {
                "devnet" => 576, // ~10 minutes at 1s slots
                _ => 12_614_400, // ~4 years at 10s slots (mainnet/testnet)
            };
            let lock_until = chain_info.best_height + blocks_per_era + 1000;

            // Compute hash-chain VDF proof for registration (~5 seconds)
            let current_epoch = (chain_info.best_slot / 360) as u32;
            let vdf_input = vdf::registration_input(&producer_pubkey, current_epoch);
            let vdf_iterations = vdf::T_REGISTER_BASE;
            println!(
                "Computing registration VDF ({} iterations)...",
                vdf_iterations
            );
            let vdf_output = doli_core::tpop::heartbeat::hash_chain_vdf(&vdf_input, vdf_iterations);
            println!("VDF proof computed.");
            println!();

            // Create registration transaction with bonds and hash-chain VDF output
            // Hash-chain VDF is self-verifying (recompute to verify), no separate proof needed
            let reg_data = doli_core::transaction::RegistrationData {
                public_key: producer_pubkey,
                epoch: current_epoch,
                vdf_output: vdf_output.to_vec(),
                vdf_proof: vec![], // Hash-chain VDF: no separate proof, output IS the proof
                prev_registration_hash: crypto::Hash::ZERO,
                sequence_number: 0,
                bond_count: bonds,
                bls_pubkey: Vec::new(), // BLS key added via node wallet (Layer 4)
                bls_pop: Vec::new(),
            };
            let extra_data = bincode::serialize(&reg_data)
                .map_err(|e| anyhow::anyhow!("Failed to serialize registration data: {}", e))?;

            // Create one Bond UTXO per bond (each with bond_unit amount)
            let pubkey_hash_for_bond =
                crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, producer_pubkey.as_bytes());
            let outputs: Vec<Output> = (0..bonds)
                .map(|_| {
                    doli_core::transaction::Output::bond(
                        bond_unit,
                        pubkey_hash_for_bond,
                        lock_until,
                        0, // creation_slot stamped by node at block application
                    )
                })
                .collect();

            let mut tx = Transaction {
                version: 1,
                tx_type: doli_core::transaction::TxType::Registration,
                inputs,
                outputs,
                extra_data,
            };

            // Add change output if needed
            let change = total_input - required_amount - fee;
            if change > 0 {
                let change_hash = Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
                tx.outputs.push(Output::normal(change, change_hash));
            }

            // Sign each input
            let signing_message = tx.signing_message();
            for input in &mut tx.inputs {
                input.signature = signature::sign_hash(&signing_message, keypair.private_key());
            }

            // Submit transaction
            let tx_hex = hex::encode(tx.serialize());
            println!("Submitting registration transaction...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => {
                    println!("Registration submitted successfully!");
                    println!("TX Hash: {}", hash);
                    println!();
                    // Show activation epoch ETA
                    if let Ok(epoch) = rpc.get_epoch_info().await {
                        let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                        println!("Status: Pending activation (epoch-deferred).",);
                        println!(
                            "Estimated activation: ~{} minutes (Epoch {}, block {}).",
                            eta_minutes,
                            epoch.current_epoch + 1,
                            epoch.epoch_end_height
                        );
                    }
                    println!("Use 'doli producer status' to check registration status.");
                }
                Err(e) => {
                    anyhow::bail!("Error submitting registration: {}", e);
                }
            }
        }

        ProducerCommands::Status { pubkey } => {
            let pk = match pubkey {
                Some(pk) => pk,
                None => {
                    let wallet = Wallet::load(wallet_path)?;
                    wallet.addresses()[0].public_key.clone()
                }
            };

            println!("Producer Status");
            println!("{:-<60}", "");
            println!();

            match rpc.get_producer(&pk).await {
                Ok(info) => {
                    let addr_display = hex::decode(&info.public_key)
                        .ok()
                        .and_then(|bytes| {
                            crypto::address::from_pubkey(&bytes, address_prefix()).ok()
                        })
                        .unwrap_or_else(|| {
                            format!(
                                "{}...{}",
                                &info.public_key[..16],
                                &info.public_key[info.public_key.len() - 8..]
                            )
                        });
                    println!("Address:       {}", addr_display);
                    println!("Status:        {}", info.status);
                    println!("Registered at: block {}", info.registration_height);
                    println!("Bond Count:    {}", info.bond_count);
                    println!("Bond Amount:   {}", format_balance(info.bond_amount));
                    println!("Current Era:   {}", info.era);

                    // Show per-bond vesting info from getBondDetails
                    if let Ok(details) = rpc.get_bond_details(&pk).await {
                        let vq = details.vesting_quarter_slots;

                        // Find oldest bond per tier for time-to-next display
                        let mut oldest_q3: Option<u64> = None; // oldest age in Q3
                        let mut oldest_q2: Option<u64> = None;
                        let mut oldest_q1: Option<u64> = None;
                        for bond in &details.bonds {
                            if bond.vested {
                                continue;
                            }
                            let quarters = bond.age_slots / vq;
                            match quarters {
                                0 => {
                                    oldest_q1 = Some(
                                        oldest_q1.map_or(bond.age_slots, |v| v.max(bond.age_slots)),
                                    );
                                }
                                1 => {
                                    oldest_q2 = Some(
                                        oldest_q2.map_or(bond.age_slots, |v| v.max(bond.age_slots)),
                                    );
                                }
                                2 => {
                                    oldest_q3 = Some(
                                        oldest_q3.map_or(bond.age_slots, |v| v.max(bond.age_slots)),
                                    );
                                }
                                _ => {}
                            }
                        }

                        println!();
                        println!(
                            "Bonds: {} ({}):",
                            details.bond_count,
                            format_balance(details.total_staked)
                        );
                        let s = &details.summary;
                        if s.vested > 0 {
                            let label = if s.vested == 1 { "bond " } else { "bonds" };
                            println!("  Vested (0% penalty):   {} {}", s.vested, label);
                        }
                        if s.q3 > 0 {
                            let label = if s.q3 == 1 { "bond " } else { "bonds" };
                            let eta = oldest_q3
                                .map(|age| {
                                    let remaining = (3 * vq).saturating_sub(age);
                                    format!(" — {} to 0%", format_slot_duration(remaining))
                                })
                                .unwrap_or_default();
                            println!("  Q3 (25% penalty):      {} {}{}", s.q3, label, eta);
                        }
                        if s.q2 > 0 {
                            let label = if s.q2 == 1 { "bond " } else { "bonds" };
                            let eta = oldest_q2
                                .map(|age| {
                                    let remaining = (2 * vq).saturating_sub(age);
                                    format!(" — {} to 25%", format_slot_duration(remaining))
                                })
                                .unwrap_or_default();
                            println!("  Q2 (50% penalty):      {} {}{}", s.q2, label, eta);
                        }
                        if s.q1 > 0 {
                            let label = if s.q1 == 1 { "bond " } else { "bonds" };
                            let eta = oldest_q1
                                .map(|age| {
                                    let remaining = vq.saturating_sub(age);
                                    format!(" — {} to 50%", format_slot_duration(remaining))
                                })
                                .unwrap_or_default();
                            println!("  Q1 (75% penalty):      {} {}{}", s.q1, label, eta);
                        }
                        if details.withdrawal_pending_count > 0 {
                            println!();
                            println!(
                                "  Withdrawal pending: {} bonds (applied at next epoch boundary)",
                                details.withdrawal_pending_count
                            );
                        }
                    }

                    // Show pending epoch-deferred updates
                    if !info.pending_updates.is_empty() {
                        println!();
                        println!("Pending updates (applied at next epoch boundary):");
                        for pu in &info.pending_updates {
                            match pu.update_type.as_str() {
                                "add_bond" => {
                                    println!("  + Add {} bond(s)", pu.bond_count.unwrap_or(0))
                                }
                                "withdrawal" => {
                                    println!("  - Withdraw {} bond(s)", pu.bond_count.unwrap_or(0))
                                }
                                "exit" => println!("  - Exit producer set"),
                                "register" => println!("  + Registration pending"),
                                other => println!("  ? {}", other),
                            }
                        }
                        if let Ok(epoch) = rpc.get_epoch_info().await {
                            let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                            println!(
                                "  ETA: ~{} minutes (Epoch {}, block {})",
                                eta_minutes,
                                epoch.current_epoch + 1,
                                epoch.epoch_end_height
                            );
                        }
                    }
                }
                Err(e) => {
                    if e.to_string().contains("not found") {
                        let pk_display = hex::decode(&pk)
                            .ok()
                            .and_then(|bytes| {
                                crypto::address::from_pubkey(&bytes, address_prefix()).ok()
                            })
                            .unwrap_or_else(|| {
                                format!("{}...{}", &pk[..16], &pk[pk.len().saturating_sub(8)..])
                            });
                        println!("Producer Status");
                        println!("{:-<60}", "");
                        println!();
                        println!("  Address: {}", pk_display);
                        println!("  Status:  Not registered");
                        println!();
                        println!("If you recently registered, your registration may be pending");
                        println!("activation at the next epoch boundary. Check again in a few");
                        println!("minutes, or use 'doli producer register' to register.");
                    } else {
                        anyhow::bail!("Error: {}", e);
                    }
                }
            }
        }

        ProducerCommands::Bonds { pubkey } => {
            let pk = match pubkey {
                Some(pk) => pk,
                None => {
                    let wallet = Wallet::load(wallet_path)?;
                    wallet.addresses()[0].public_key.clone()
                }
            };

            let details = rpc.get_bond_details(&pk).await?;
            let vq = details.vesting_quarter_slots;

            println!(
                "Bond Details ({} bonds, {} staked)",
                details.bond_count,
                format_balance(details.total_staked)
            );
            println!("{:-<60}", "");

            if details.bonds.is_empty() {
                println!("No bonds found.");
            } else {
                println!(
                    " {:<4} {:<16} {:<12} {:<10} {:<8} Time to Next",
                    "#", "Created (slot)", "Age", "Quarter", "Penalty"
                );
                println!(" {:-<70}", "");

                for (i, bond) in details.bonds.iter().enumerate() {
                    let age_str = format_slot_duration(bond.age_slots);

                    let quarter_label = if bond.vested {
                        "Q4+".to_string()
                    } else {
                        let quarters = bond.age_slots / vq;
                        format!("Q{}", quarters + 1)
                    };

                    let penalty_str = format!("{}%", bond.penalty_pct);

                    let time_to_next = if bond.vested {
                        "Fully vested".to_string()
                    } else {
                        // Slots until next quarter boundary
                        let quarters_done = bond.age_slots / vq;
                        let next_quarter_age = (quarters_done + 1) * vq;
                        let slots_remaining = next_quarter_age.saturating_sub(bond.age_slots);
                        let next_penalty = match quarters_done + 1 {
                            1 => "50%",
                            2 => "25%",
                            _ => "0%",
                        };
                        format!(
                            "{} to {}",
                            format_slot_duration(slots_remaining),
                            next_penalty
                        )
                    };

                    println!(
                        " {:<4} {:<16} {:<12} {:<10} {:<8} {}",
                        i + 1,
                        format!("slot {}", bond.creation_slot),
                        age_str,
                        quarter_label,
                        penalty_str,
                        time_to_next
                    );
                }
            }

            if details.withdrawal_pending_count > 0 {
                println!();
                println!(
                    "Withdrawal pending: {} bonds (applied at next epoch boundary)",
                    details.withdrawal_pending_count
                );
            }
        }

        ProducerCommands::List { active } => {
            println!("Network Producers");
            println!("{:-<80}", "");
            println!();

            match rpc.get_producers(active).await {
                Ok(producers) => {
                    if producers.is_empty() {
                        println!("No producers found.");
                    } else {
                        println!(
                            "{:<10} {:<20} {:<10} {:<10}",
                            "Status", "Address", "Bonds", "Era"
                        );
                        println!("{:-<52}", "");

                        for p in &producers {
                            let addr_display = if let Ok(pubkey_bytes) = hex::decode(&p.public_key)
                            {
                                crypto::address::from_pubkey(&pubkey_bytes, address_prefix())
                                    .map(|a| format!("{}...", &a[..16]))
                                    .unwrap_or_else(|_| {
                                        format!(
                                            "{}...{}",
                                            &p.public_key[..8],
                                            &p.public_key[p.public_key.len() - 4..]
                                        )
                                    })
                            } else {
                                format!(
                                    "{}...{}",
                                    &p.public_key[..8],
                                    &p.public_key[p.public_key.len() - 4..]
                                )
                            };
                            println!(
                                "{:<10} {:<20} {:<10} {:<10}",
                                p.status, addr_display, p.bond_count, p.era
                            );
                        }

                        println!();
                        println!("Total: {} producers", producers.len());
                    }
                }
                Err(e) => {
                    anyhow::bail!("Error fetching producers: {}", e);
                }
            }
        }

        ProducerCommands::AddBond { count } => {
            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let pubkey_hash = wallet.primary_pubkey_hash();

            println!("Add Bond");
            println!("{:-<60}", "");
            println!();

            if !(1..=10000).contains(&count) {
                anyhow::bail!("Bond count must be between 1 and 10000");
            }

            // Get network parameters from node (bond_unit is network-specific)
            let network_params = rpc.get_network_params().await?;
            let bond_unit = network_params.bond_unit;
            let bond_display = bond_unit / 100_000_000; // Convert to DOLI per bond
            let required_amount = bond_unit * count as u64;

            // Verify producer is registered before proceeding
            let pk_hex = hex::encode(keypair.public_key().as_bytes());
            let producers = rpc.get_producers(false).await?;
            let is_registered = producers.iter().any(|p| p.public_key == pk_hex);
            if !is_registered {
                anyhow::bail!(
                    "This key is not registered as a producer. Register first with 'doli producer register'.\n\
                     Public key: {}",
                    pk_hex
                );
            }

            println!(
                "Adding {} bond(s) = {} DOLI",
                count,
                count as u64 * bond_display
            );
            println!();

            // Get spendable UTXOs
            let utxos = rpc.get_utxos(&pubkey_hash, true).await?;
            let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

            // Initial UTXO selection with minimum fee estimate
            let mut selected_utxos = Vec::new();
            let mut total_input = 0u64;
            let mut fee = 1000u64.max(utxos.len() as u64 * 500);
            for utxo in &utxos {
                if total_input >= required_amount + fee {
                    break;
                }
                selected_utxos.push(utxo.clone());
                total_input += utxo.amount;
            }

            // Auto-calculate fee: max(1000, inputs * 500) — same as send
            fee = 1000u64.max(selected_utxos.len() as u64 * 500);

            // Re-select if fee increased the requirement
            if total_input < required_amount + fee {
                selected_utxos.clear();
                total_input = 0;
                for utxo in &utxos {
                    selected_utxos.push(utxo.clone());
                    total_input += utxo.amount;
                    fee = 1000u64.max(selected_utxos.len() as u64 * 500);
                    if total_input >= required_amount + fee {
                        break;
                    }
                }
            }

            if total_available < required_amount + fee {
                anyhow::bail!(
                    "Insufficient balance. Required: {} DOLI + {} fee, Available: {}",
                    count as u64 * bond_display,
                    format_balance(fee),
                    format_balance(total_available)
                );
            }

            // Build inputs
            let mut inputs: Vec<Input> = Vec::new();
            for utxo in &selected_utxos {
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
                inputs.push(Input::new(prev_tx_hash, utxo.output_index));
            }

            // Parse producer public key
            let pubkey_bytes = hex::decode(&wallet.addresses()[0].public_key)?;
            let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

            // Calculate lock_until for Bond UTXO (same as registration)
            let chain_info = rpc.get_chain_info().await?;
            let blocks_per_era: u64 = match chain_info.network.as_str() {
                "devnet" => 576,
                _ => 12_614_400,
            };
            let lock_until = chain_info.best_height + blocks_per_era + 1000;

            // Create add-bond transaction (lock/unlock: creates Bond UTXO)
            let mut tx = Transaction::new_add_bond(
                inputs,
                producer_pubkey,
                count,
                required_amount,
                lock_until,
            );

            // Add change output
            let change = total_input - required_amount - fee;
            if change > 0 {
                let change_hash = Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
                tx.outputs.push(Output::normal(change, change_hash));
            }

            // Sign
            let signing_message = tx.signing_message();
            for input in &mut tx.inputs {
                input.signature = signature::sign_hash(&signing_message, keypair.private_key());
            }

            // Submit
            let tx_hex = hex::encode(tx.serialize());
            println!("Submitting add-bond transaction...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => {
                    println!("Bonds added successfully!");
                    println!("TX Hash: {}", hash);
                    println!();
                    // Show activation epoch ETA
                    if let Ok(epoch) = rpc.get_epoch_info().await {
                        let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                        println!("Status: Pending activation (epoch-deferred).",);
                        println!(
                            "Estimated activation: ~{} minutes (Epoch {}, block {}).",
                            eta_minutes,
                            epoch.current_epoch + 1,
                            epoch.epoch_end_height
                        );
                    }
                }
                Err(e) => {
                    anyhow::bail!("Error adding bonds: {}", e);
                }
            }
        }

        ProducerCommands::RequestWithdrawal { count, destination } => {
            let wallet = Wallet::load(wallet_path)?;
            let pubkey_hash = wallet.primary_pubkey_hash();

            println!("Request Withdrawal");
            println!("{:-<60}", "");
            println!();

            if count < 1 {
                anyhow::bail!("Must withdraw at least 1 bond");
            }

            // Destination defaults to wallet address (accept doli1... or hex)
            let dest_hash = match &destination {
                Some(d) => crypto::address::resolve(d, None)
                    .map_err(|e| anyhow::anyhow!("Invalid destination address: {}", e))?,
                None => Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid wallet address"))?,
            };

            let dest_display = crypto::address::encode(&dest_hash, address_prefix())
                .unwrap_or_else(|_| dest_hash.to_hex());

            // Fetch per-bond details for FIFO breakdown
            let pk = wallet.addresses()[0].public_key.clone();
            let details = rpc.get_bond_details(&pk).await?;

            let available = details.bond_count - details.withdrawal_pending_count;
            if count > available {
                anyhow::bail!(
                    "--count must be between 1 and {} (your available bonds)",
                    available
                );
            }

            // Show bond inventory
            let s = &details.summary;
            println!("Your bonds ({} total):", details.bond_count);
            if s.vested > 0 {
                println!("  {} bonds — vested (0% penalty)", s.vested);
            }
            if s.q3 > 0 {
                println!("  {} bonds — Q3 (25% penalty)", s.q3);
            }
            if s.q2 > 0 {
                println!("  {} bonds — Q2 (50% penalty)", s.q2);
            }
            if s.q1 > 0 {
                println!("  {} bonds — Q1 (75% penalty)", s.q1);
            }
            println!();

            // Calculate and display FIFO breakdown (oldest first)
            let breakdown = compute_fifo_breakdown(&details, count);

            println!("Withdrawing {} bonds (FIFO — oldest first):", count);
            println!("Destination: {}", dest_display);
            display_fifo_breakdown(&breakdown);
            println!();
            println!("You receive: {}", format_balance(breakdown.total_net));
            if breakdown.total_penalty > 0 {
                println!(
                    "Penalty burned: {}",
                    format_balance(breakdown.total_penalty)
                );
                let pct = (breakdown.total_penalty * 100)
                    / (breakdown.total_net + breakdown.total_penalty);
                if pct >= 50 {
                    println!(
                        "WARNING: High penalty — {}% of bond value will be burned.",
                        pct
                    );
                }
            } else {
                println!("No penalty — all bonds fully vested.");
            }
            let total_net = breakdown.total_net;
            println!("Bonds remaining: {}", details.bond_count - count);
            println!();

            // Parse producer public key
            let pubkey_bytes = hex::decode(&wallet.addresses()[0].public_key)?;
            let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

            // Lock/unlock: find Bond UTXOs to consume as inputs
            let all_utxos = rpc.get_utxos(&pubkey_hash, false).await?;
            let bond_utxos: Vec<_> = all_utxos
                .iter()
                .filter(|u| u.output_type == "bond")
                .collect();

            let bond_unit = {
                let params = rpc.get_network_params().await?;
                params.bond_unit
            };
            let required_bond_value = bond_unit * count as u64;
            let available_bond_value: u64 = bond_utxos.iter().map(|u| u.amount).sum();

            if available_bond_value < required_bond_value {
                anyhow::bail!(
                    "Insufficient Bond UTXOs. Need {} DOLI in bonds, have {}",
                    format_balance(required_bond_value),
                    format_balance(available_bond_value)
                );
            }

            // Select Bond UTXOs to cover bond_count × bond_unit (any order)
            let mut bond_inputs: Vec<Input> = Vec::new();
            let mut bond_input_total = 0u64;
            for utxo in &bond_utxos {
                if bond_input_total >= required_bond_value {
                    break;
                }
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid Bond UTXO tx_hash"))?;
                bond_inputs.push(Input::new(prev_tx_hash, utxo.output_index));
                bond_input_total += utxo.amount;
            }

            // Select a normal UTXO to cover the tx fee
            let normal_utxos: Vec<_> = all_utxos
                .iter()
                .filter(|u| u.output_type == "normal")
                .collect();
            let mut fee_input_total = 0u64;
            let mut fee_inputs: Vec<Input> = Vec::new();
            let fee_estimate = 1000u64.max((bond_inputs.len() as u64 + 2) * 500);
            for utxo in &normal_utxos {
                if fee_input_total >= fee_estimate {
                    break;
                }
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
                fee_inputs.push(Input::new(prev_tx_hash, utxo.output_index));
                fee_input_total += utxo.amount;
            }

            // Combine bond + fee inputs
            let mut all_inputs = bond_inputs;
            all_inputs.extend(fee_inputs);

            // Auto-calculate fee: max(1000, inputs * 500)
            let fee = 1000u64.max(all_inputs.len() as u64 * 500);
            if fee_input_total < fee {
                anyhow::bail!(
                    "Insufficient spendable balance for tx fee. Need {}, have {}",
                    format_balance(fee),
                    format_balance(fee_input_total)
                );
            }

            // Create request-withdrawal transaction with Bond + fee inputs
            let mut tx = Transaction::new_request_withdrawal(
                all_inputs,
                producer_pubkey,
                count,
                dest_hash,
                total_net,
            );

            // Add change output for fee UTXO remainder
            let fee_change = fee_input_total - fee;
            if fee_change > 0 {
                let change_hash = Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
                tx.outputs.push(Output::normal(fee_change, change_hash));
            }

            // Sign all inputs
            let keypair = wallet.primary_keypair()?;
            let signing_message = tx.signing_message();
            for input in &mut tx.inputs {
                input.signature = signature::sign_hash(&signing_message, keypair.private_key());
            }

            let tx_hex = hex::encode(tx.serialize());
            println!("Submitting withdrawal request...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => {
                    println!("Withdrawal submitted. TX: {}", hash);
                    println!("Funds available now.");
                    // Show epoch boundary ETA for bond removal
                    if let Ok(epoch) = rpc.get_epoch_info().await {
                        let eta_minutes = (epoch.blocks_remaining * 10) / 60;
                        println!(
                            "Bonds removed at next epoch boundary (~{} minutes, Epoch {}).",
                            eta_minutes,
                            epoch.current_epoch + 1,
                        );
                    }
                }
                Err(e) => {
                    anyhow::bail!("Error requesting withdrawal: {}", e);
                }
            }
        }

        ProducerCommands::SimulateWithdrawal { count } => {
            let wallet = Wallet::load(wallet_path)?;

            println!("Simulated Withdrawal (dry run — no transaction submitted)");
            println!("{:-<60}", "");
            println!();

            if count < 1 {
                anyhow::bail!("Must simulate at least 1 bond");
            }

            let pk = wallet.addresses()[0].public_key.clone();
            let details = rpc.get_bond_details(&pk).await?;

            let available = details.bond_count - details.withdrawal_pending_count;
            if count > available {
                anyhow::bail!(
                    "--count must be between 1 and {} (your available bonds)",
                    available
                );
            }

            // Show bond inventory
            let s = &details.summary;
            println!("Your bonds ({} total):", details.bond_count);
            if s.vested > 0 {
                println!("  {} bonds — vested (0% penalty)", s.vested);
            }
            if s.q3 > 0 {
                println!("  {} bonds — Q3 (25% penalty)", s.q3);
            }
            if s.q2 > 0 {
                println!("  {} bonds — Q2 (50% penalty)", s.q2);
            }
            if s.q1 > 0 {
                println!("  {} bonds — Q1 (75% penalty)", s.q1);
            }
            println!();

            // Calculate and display FIFO breakdown
            let breakdown = compute_fifo_breakdown(&details, count);

            println!("Withdrawing {} bonds (FIFO — oldest first):", count);
            display_fifo_breakdown(&breakdown);
            println!();
            println!("You would receive: {}", format_balance(breakdown.total_net));
            if breakdown.total_penalty > 0 {
                println!(
                    "Penalty burned: {}",
                    format_balance(breakdown.total_penalty)
                );
            }
            println!("Bonds remaining: {}", details.bond_count - count);
        }

        ProducerCommands::Exit { force } => {
            let wallet = Wallet::load(wallet_path)?;

            println!("Producer Exit");
            println!("{:-<60}", "");
            println!();

            // Check current status
            let pk = wallet.addresses()[0].public_key.clone();
            let producer_info = rpc.get_producer(&pk).await?;

            if producer_info.status == "exited" {
                anyhow::bail!("Producer has already exited");
            }
            if producer_info.status == "unbonding" {
                anyhow::bail!(
                    "Producer is in unbonding state. Use 'request-withdrawal' to withdraw bonds."
                );
            }

            let bond_count = producer_info.bond_count;
            if bond_count == 0 {
                anyhow::bail!("Producer has no bonds to withdraw");
            }

            // Fetch per-bond details for FIFO breakdown
            let details = rpc.get_bond_details(&pk).await?;
            let available = details.bond_count - details.withdrawal_pending_count;
            if available == 0 {
                anyhow::bail!("All bonds already have pending withdrawals");
            }
            let withdraw_count = available; // withdraw ALL available bonds

            // Show bond inventory
            let s = &details.summary;
            println!(
                "Exiting producer set — withdrawing ALL {} bonds:",
                withdraw_count
            );
            if s.vested > 0 {
                println!("  {} bonds — vested (0% penalty)", s.vested);
            }
            if s.q3 > 0 {
                println!("  {} bonds — Q3 (25% penalty)", s.q3);
            }
            if s.q2 > 0 {
                println!("  {} bonds — Q2 (50% penalty)", s.q2);
            }
            if s.q1 > 0 {
                println!("  {} bonds — Q1 (75% penalty)", s.q1);
            }
            println!();

            // Calculate FIFO breakdown
            let breakdown = compute_fifo_breakdown(&details, withdraw_count);
            display_fifo_breakdown(&breakdown);
            println!();
            println!("You receive: {}", format_balance(breakdown.total_net));
            if breakdown.total_penalty > 0 {
                println!(
                    "Penalty burned: {}",
                    format_balance(breakdown.total_penalty)
                );
                let pct = (breakdown.total_penalty * 100)
                    / (breakdown.total_net + breakdown.total_penalty);
                if pct >= 50 {
                    println!(
                        "WARNING: High penalty — {}% of bond value will be burned.",
                        pct
                    );
                }
            } else {
                println!("No penalty — all bonds fully vested.");
            }
            println!();

            if !force {
                println!("Use --force to proceed with exit.");
                return Ok(());
            }

            // Parse producer public key
            let pubkey_bytes = hex::decode(&pk)?;
            let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

            // Destination: wallet's own address
            let pubkey_hash = wallet.primary_pubkey_hash();
            let dest_hash = Hash::from_hex(&pubkey_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid wallet address"))?;

            // Lock/unlock: find Bond UTXOs to consume as inputs
            let all_utxos = rpc.get_utxos(&pubkey_hash, false).await?;
            let bond_utxos: Vec<_> = all_utxos
                .iter()
                .filter(|u| u.output_type == "bond")
                .collect();

            let bond_unit = {
                let params = rpc.get_network_params().await?;
                params.bond_unit
            };
            let required_bond_value = bond_unit * withdraw_count as u64;
            let available_bond_value: u64 = bond_utxos.iter().map(|u| u.amount).sum();

            if available_bond_value < required_bond_value {
                anyhow::bail!(
                    "Insufficient Bond UTXOs for exit. Need {} DOLI, have {}",
                    format_balance(required_bond_value),
                    format_balance(available_bond_value)
                );
            }

            let mut bond_inputs: Vec<Input> = Vec::new();
            let mut bond_input_total = 0u64;
            for utxo in &bond_utxos {
                if bond_input_total >= required_bond_value {
                    break;
                }
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid Bond UTXO tx_hash"))?;
                bond_inputs.push(Input::new(prev_tx_hash, utxo.output_index));
                bond_input_total += utxo.amount;
            }

            // Select a normal UTXO to cover the tx fee
            let normal_utxos: Vec<_> = all_utxos
                .iter()
                .filter(|u| u.output_type == "normal")
                .collect();
            let mut fee_input_total = 0u64;
            let mut fee_inputs: Vec<Input> = Vec::new();
            let fee_estimate = 1000u64.max((bond_inputs.len() as u64 + 2) * 500);
            for utxo in &normal_utxos {
                if fee_input_total >= fee_estimate {
                    break;
                }
                let prev_tx_hash = Hash::from_hex(&utxo.tx_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
                fee_inputs.push(Input::new(prev_tx_hash, utxo.output_index));
                fee_input_total += utxo.amount;
            }

            // Combine bond + fee inputs
            let mut all_inputs = bond_inputs;
            all_inputs.extend(fee_inputs);

            // Auto-calculate fee: max(1000, inputs * 500)
            let fee = 1000u64.max(all_inputs.len() as u64 * 500);
            if fee_input_total < fee {
                anyhow::bail!(
                    "Insufficient spendable balance for tx fee. Need {}, have {}",
                    format_balance(fee),
                    format_balance(fee_input_total)
                );
            }

            // Create RequestWithdrawal for ALL bonds with Bond + fee inputs
            let total_net = breakdown.total_net;
            let mut tx = Transaction::new_request_withdrawal(
                all_inputs,
                producer_pubkey,
                withdraw_count,
                dest_hash,
                total_net,
            );

            // Add change output for fee UTXO remainder
            let fee_change = fee_input_total - fee;
            if fee_change > 0 {
                let change_hash = Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
                tx.outputs.push(Output::normal(fee_change, change_hash));
            }

            // Sign all inputs
            let keypair = wallet.primary_keypair()?;
            let signing_message = tx.signing_message();
            for input in &mut tx.inputs {
                input.signature = signature::sign_hash(&signing_message, keypair.private_key());
            }

            let tx_hex = hex::encode(tx.serialize());

            println!("Submitting exit (withdrawal of all bonds)...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => {
                    println!("Exit submitted successfully!");
                    println!("TX Hash: {}", hash);
                    println!();
                    println!("Bonds withdrawn. Producer will be removed at next epoch boundary.");
                }
                Err(e) => {
                    anyhow::bail!("Error submitting exit: {}", e);
                }
            }
        }

        ProducerCommands::Slash { block1, block2 } => {
            println!("Submit Slashing Evidence");
            println!("{:-<60}", "");
            println!();

            println!("Checking blocks for equivocation evidence...");
            println!("  Block 1: {}", block1);
            println!("  Block 2: {}", block2);

            // Get block info from RPC to verify it's a valid equivocation
            let block1_resp = rpc.get_block(&block1).await?;
            let block2_resp = rpc.get_block(&block2).await?;

            // Verify they're for the same slot (equivocation)
            if block1_resp.slot != block2_resp.slot {
                anyhow::bail!("Blocks are for different slots (block 1 slot: {}, block 2 slot: {}). Slashing requires two different blocks for the SAME slot.", block1_resp.slot, block2_resp.slot);
            }

            // Verify they're from the same producer
            if block1_resp.producer != block2_resp.producer {
                anyhow::bail!("Blocks are from different producers (block 1: {}, block 2: {}). Slashing requires blocks from the SAME producer.", block1_resp.producer, block2_resp.producer);
            }

            // Verify blocks are different
            if block1_resp.hash == block2_resp.hash {
                anyhow::bail!("Both hashes refer to the same block. Slashing requires two DIFFERENT blocks for the same slot.");
            }

            let producer_addr = hex::decode(&block1_resp.producer)
                .ok()
                .and_then(|bytes| crypto::address::from_pubkey(&bytes, address_prefix()).ok())
                .unwrap_or_else(|| block1_resp.producer.clone());

            println!();
            println!("Equivocation confirmed!");
            println!("  Producer: {}", producer_addr);
            println!("  Slot:     {}", block1_resp.slot);
            println!(
                "  Block 1:  {} (height {})",
                block1_resp.hash, block1_resp.height
            );
            println!(
                "  Block 2:  {} (height {})",
                block2_resp.hash, block2_resp.height
            );
            println!();

            println!(
                "Note: Slashing evidence submission requires full block headers with VDF proofs."
            );
            println!();
            println!("Nodes automatically detect and submit slashing evidence when they");
            println!("receive conflicting blocks for the same slot. If you have raw block");
            println!("data with VDF proofs, use the node's internal submission mechanism.");
            println!();
            println!("The equivocating producer ({}) will be:", producer_addr);
            println!("  - Have their entire bond burned (100% penalty)");
            println!("  - Excluded from the producer set immediately");
            println!("  - Can re-register like any new producer (standard registration VDF)");
        }
    }

    Ok(())
}

async fn cmd_chain(rpc_endpoint: &str) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    println!("Chain Information");
    println!("{:-<60}", "");

    match rpc.get_chain_info().await {
        Ok(info) => {
            println!("Network:      {}", info.network);
            println!("Best Height:  {}", info.best_height);
            println!("Best Slot:    {}", info.best_slot);
            println!("Best Hash:    {}", info.best_hash);
            println!("Genesis Hash: {}", info.genesis_hash);
        }
        Err(e) => {
            anyhow::bail!("Cannot connect to node at {}. Details: {}. Make sure a DOLI node is running and accessible.", rpc_endpoint, e);
        }
    }

    Ok(())
}

async fn cmd_rewards(
    _wallet_path: &Path,
    rpc_endpoint: &str,
    command: RewardsCommands,
) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    match command {
        RewardsCommands::List => {
            println!("Rewards are distributed automatically via coinbase (1 DOLI per block).");
            println!("No claiming needed. Use 'doli balance' to see your rewards.");
            println!("Use 'doli rewards info' for current epoch details.");
        }

        RewardsCommands::Claim {
            epoch: _,
            recipient: _,
        } => {
            println!("Rewards are distributed automatically via coinbase (1 DOLI per block).");
            println!("No claiming needed. Use 'doli balance' to see your rewards.");
        }

        RewardsCommands::ClaimAll { recipient: _ } => {
            println!("Rewards are distributed automatically via coinbase (1 DOLI per block).");
            println!("No claiming needed. Use 'doli balance' to see your rewards.");
        }

        RewardsCommands::History { limit: _ } => {
            println!("Rewards are distributed automatically via coinbase (1 DOLI per block).");
            println!("No claim history — use 'doli history' to see received rewards.");
        }

        RewardsCommands::Info => {
            println!("Reward Epoch Information");
            println!("{:-<60}", "");
            println!();

            match rpc.get_epoch_info().await {
                Ok(info) => {
                    println!("Current Height:      {}", info.current_height);
                    println!("Current Epoch:       {}", info.current_epoch);
                    println!(
                        "Last Complete Epoch: {}",
                        info.last_complete_epoch
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "None".to_string())
                    );
                    println!();
                    println!("Blocks per Epoch:    {}", info.blocks_per_epoch);
                    println!("Blocks Remaining:    {}", info.blocks_remaining);
                    println!();
                    println!(
                        "Epoch {} Range:     {} - {} (exclusive)",
                        info.current_epoch, info.epoch_start_height, info.epoch_end_height
                    );
                    println!("Block Reward:        {}", format_balance(info.block_reward));
                    println!();
                    println!(
                        "Progress: [{}{}] {}%",
                        "=".repeat(
                            ((info.blocks_per_epoch - info.blocks_remaining) * 30
                                / info.blocks_per_epoch) as usize
                        ),
                        " ".repeat((info.blocks_remaining * 30 / info.blocks_per_epoch) as usize),
                        ((info.blocks_per_epoch - info.blocks_remaining) * 100
                            / info.blocks_per_epoch)
                    );
                }
                Err(e) => {
                    anyhow::bail!("Error fetching epoch info: {}", e);
                }
            }
        }
    }

    Ok(())
}

async fn cmd_update(wallet_path: &Path, rpc_endpoint: &str, command: UpdateCommands) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    match command {
        UpdateCommands::Check => {
            println!("Update Check");
            println!("{:-<60}", "");
            println!();

            // Show running node version
            if let Ok(info) = rpc.get_node_info().await {
                let node_version = info.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                let network = info.get("network").and_then(|v| v.as_str()).unwrap_or("?");
                println!("Node version: v{}", node_version);
                println!("Network:      {}", network);
                println!();
            }

            match rpc.get_update_status().await {
                Ok(status) => {
                    if let Some(pending) = status.get("pending_update") {
                        if pending.is_null() {
                            println!("No pending updates. Your node is up to date.");
                        } else {
                            println!("Pending update found:");
                            println!(
                                "  Version: {}",
                                pending
                                    .get("version")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?")
                            );
                            println!(
                                "  Published: {}",
                                pending
                                    .get("published_at")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0)
                            );
                        }
                    } else {
                        println!("No pending updates.");
                    }
                }
                Err(e) => anyhow::bail!("Error checking updates: {}", e),
            }
        }

        UpdateCommands::Status => {
            println!("Update Status");
            println!("{:-<60}", "");
            println!();

            match rpc.get_update_status().await {
                Ok(status) => {
                    let veto_active = status
                        .get("veto_period_active")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let veto_count = status
                        .get("veto_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let veto_pct = status
                        .get("veto_percent")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);

                    if let Some(pending) = status.get("pending_update") {
                        if !pending.is_null() {
                            let version = pending
                                .get("version")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            println!("Pending Update: v{}", version);
                            println!(
                                "Veto Period:    {}",
                                if veto_active { "ACTIVE" } else { "ended" }
                            );
                            println!("Veto Count:     {}", veto_count);
                            println!("Veto Percent:   {:.1}% (threshold: 40%)", veto_pct);
                        } else {
                            println!("No pending updates.");
                        }
                    } else {
                        println!("No pending updates.");
                    }
                }
                Err(e) => anyhow::bail!("Error fetching status: {}", e),
            }
        }

        UpdateCommands::Vote {
            version,
            veto,
            approve,
        } => {
            if !veto && !approve {
                anyhow::bail!("Must specify --veto or --approve");
            }

            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let vote_type = if veto { "Veto" } else { "Approve" };
            let vote_str = if veto { "veto" } else { "approve" };
            let producer_id = wallet.addresses()[0].public_key.clone();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            // Sign the vote: "version:vote:timestamp"
            let message = format!("{}:{}:{}", version, vote_str, timestamp);
            let sig = crypto::signature::sign(message.as_bytes(), keypair.private_key());

            println!(
                "Submitting {} vote for v{}...",
                vote_str.to_uppercase(),
                version
            );

            let vote_msg = serde_json::json!({
                "version": version,
                "vote": vote_type,
                "producerId": producer_id,
                "timestamp": timestamp,
                "signature": sig.to_hex(),
            });

            match rpc.submit_vote(vote_msg).await {
                Ok(result) => {
                    let status = result
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    println!("Vote {}: {}", vote_str, status);
                }
                Err(e) => anyhow::bail!("Error submitting vote: {}", e),
            }
        }

        UpdateCommands::Votes { version } => {
            println!("Votes for v{}", version);
            println!("{:-<60}", "");
            println!();

            match rpc.get_update_status().await {
                Ok(status) => {
                    let veto_count = status
                        .get("veto_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let veto_pct = status
                        .get("veto_percent")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);

                    println!("Veto votes:   {}", veto_count);
                    println!("Veto weight:  {:.1}%", veto_pct);
                    println!(
                        "Threshold:    40% ({})",
                        if veto_pct >= 40.0 {
                            "REJECTED"
                        } else {
                            "not reached"
                        }
                    );
                }
                Err(e) => anyhow::bail!("Error fetching votes: {}", e),
            }
        }

        UpdateCommands::Apply => {
            println!("Manual update apply requires a running node with an approved release.");
            println!("Updates are applied automatically after the 7-day veto period.");
            println!("Use 'doli update status' to check the current update state.");
        }

        UpdateCommands::Rollback => {
            // Check common backup locations
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let backup = PathBuf::from(&home).join(".doli").join("doli-node.backup");
            if backup.exists() {
                println!("Backup binary found at: {}", backup.display());
                println!("To rollback manually, stop the node and run:");
                println!("  cp {} $(which doli-node)", backup.display());
                println!("  systemctl restart doli-mainnet-nodeN");
            } else {
                println!("No backup binary found at {}", backup.display());
                println!("The watchdog automatically rolls back after 3 crashes.");
                println!("Use 'doli update status' to check the current update state.");
            }
        }
    }

    Ok(())
}

async fn cmd_maintainer(rpc_endpoint: &str, command: MaintainerCommands) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        anyhow::bail!("Cannot connect to node at {}", rpc_endpoint);
    }

    match command {
        MaintainerCommands::List => {
            println!("Maintainer Set");
            println!("{:-<60}", "");
            println!();

            match rpc.get_maintainer_set().await {
                Ok(set) => {
                    if let Some(maintainers) = set.get("maintainers").and_then(|m| m.as_array()) {
                        let threshold = set.get("threshold").and_then(|t| t.as_u64()).unwrap_or(3);
                        println!("Threshold: {} of {}", threshold, maintainers.len());
                        println!();

                        for (i, member) in maintainers.iter().enumerate() {
                            let key = member.get("pubkey").and_then(|p| p.as_str()).unwrap_or("?");
                            let short = if key.len() > 24 {
                                format!("{}...{}", &key[..16], &key[key.len() - 8..])
                            } else {
                                key.to_string()
                            };
                            println!("  {}. {}", i + 1, short);
                        }
                        println!();
                        println!(
                            "These maintainers also verify release signatures for auto-updates."
                        );
                        println!("Derived from the first 5 registered producers on-chain.");
                    } else {
                        println!("No maintainers found (or unsupported response format).");
                    }
                }
                Err(e) => anyhow::bail!("Error fetching maintainer set: {}", e),
            }
        }
    }

    Ok(())
}

fn cmd_wipe(network: &str, data_dir: Option<PathBuf>, yes: bool) -> Result<()> {
    println!("Wipe Chain Data");
    println!("{:-<60}", "");
    println!();

    // 1. Resolve data dir
    let data_dir = match data_dir {
        Some(d) => d,
        None => {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
            home.join(".doli").join(network)
        }
    };

    // 2. Verify directory exists
    if !data_dir.exists() {
        anyhow::bail!("Data directory does not exist: {:?}", data_dir);
    }

    println!("Network:   {}", network);
    println!("Data dir:  {:?}", data_dir);
    println!();

    // 3. Safety check: is doli-node running with this data dir?
    let data_dir_str = data_dir.to_string_lossy().to_string();
    let is_running = std::process::Command::new("pgrep")
        .args(["-f", &format!("doli-node.*{}", data_dir_str)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if is_running {
        anyhow::bail!(
            "A doli-node process is running with this data directory.\n\
             Stop the node first: sudo systemctl stop <service>"
        );
    }

    // 4. List what gets deleted vs preserved
    let delete_targets = [
        "blocks",
        "state_db",
        "signed_slots.db",
        "utxo_rocks",
        "chain_state.bin",
        "producers.bin",
        "utxo.bin",
        "producer_gset.bin",
        "peers.cache",
        "node_key",
        "maintainer_state.bin",
        "producer.lock",
    ];
    let preserve_targets = ["keys", ".env"];

    // Scan for items to delete (including node subdirectories like node1/, node2/)
    let mut found_items: Vec<PathBuf> = Vec::new();
    let mut scan_dir = |dir: &Path| {
        for name in &delete_targets {
            let path = dir.join(name);
            if path.exists() {
                found_items.push(path);
            }
        }
    };

    // Scan data_dir itself
    scan_dir(&data_dir);

    // Scan subdirectories (node1/, node2/, data/, etc.)
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                // Skip preserved directories
                if preserve_targets.contains(&name.as_ref()) {
                    continue;
                }
                scan_dir(&path);
                // Also scan data/ subdirectory if it exists
                let data_subdir = path.join("data");
                if data_subdir.is_dir() {
                    scan_dir(&data_subdir);
                }
            }
        }
    }

    if found_items.is_empty() {
        println!("Nothing to wipe — data directory is already clean.");
        return Ok(());
    }

    println!("Will DELETE:");
    for item in &found_items {
        let suffix = if item.is_dir() { "/" } else { "" };
        println!(
            "  - {}{}",
            item.strip_prefix(&data_dir).unwrap_or(item).display(),
            suffix
        );
    }
    println!();
    println!("Will PRESERVE:");
    for name in &preserve_targets {
        let path = data_dir.join(name);
        if path.exists() {
            println!("  - {}/", name);
        }
    }
    println!();

    // 5. Confirm
    if !yes {
        println!("This will delete all chain data. The node will resync from peers.");
        println!("Run with --yes to proceed.");
        return Ok(());
    }

    // 6. Delete
    let mut deleted = 0;
    for item in &found_items {
        let result = if item.is_dir() {
            std::fs::remove_dir_all(item)
        } else {
            std::fs::remove_file(item)
        };
        match result {
            Ok(()) => {
                deleted += 1;
            }
            Err(e) => {
                eprintln!("Warning: failed to remove {:?}: {}", item, e);
            }
        }
    }

    // 7. Summary
    println!();
    println!("Wiped {} items. Data directory is clean.", deleted);
    println!("Start the node to resync from peers.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_parse_new_wallet() {
        let cli = Cli::try_parse_from(["doli", "new"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert!(matches!(cli.command, Commands::New { .. }));
    }

    #[test]
    fn test_parse_balance() {
        let cli = Cli::try_parse_from(["doli", "balance"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_parse_send_with_args() {
        let cli = Cli::try_parse_from(["doli", "send", "doli1abc123", "100"]);
        assert!(cli.is_ok());
        if let Commands::Send { to, amount, .. } = &cli.unwrap().command {
            assert_eq!(to, "doli1abc123");
            assert_eq!(amount, "100");
        } else {
            panic!("Expected Send command");
        }
    }

    #[test]
    fn test_parse_custom_wallet_and_rpc() {
        let cli = Cli::try_parse_from([
            "doli",
            "-w",
            "/tmp/test_wallet.json",
            "-r",
            "http://localhost:9999",
            "balance",
        ]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert_eq!(cli.wallet, "/tmp/test_wallet.json");
        assert_eq!(cli.rpc, Some("http://localhost:9999".to_string()));
    }

    #[test]
    fn test_parse_producer_subcommands() {
        let cli = Cli::try_parse_from(["doli", "producer", "list"]);
        assert!(cli.is_ok());

        let cli = Cli::try_parse_from(["doli", "producer", "register", "--bonds", "5"]);
        assert!(cli.is_ok());
    }

    #[test]
    fn test_parse_missing_subcommand_fails() {
        let cli = Cli::try_parse_from(["doli"]);
        assert!(cli.is_err());
    }
}
