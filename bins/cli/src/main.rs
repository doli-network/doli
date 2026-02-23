//! DOLI CLI - Wallet command-line interface
//!
//! This binary provides wallet functionality:
//! - Key generation and management
//! - Transaction creation and signing
//! - Balance queries
//! - Network interaction

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

mod rpc_client;
mod wallet;

use rpc_client::{coins_to_units, format_balance, RpcClient};
use wallet::Wallet;

#[derive(Parser)]
#[command(name = "doli")]
#[command(about = "DOLI wallet CLI", long_about = None)]
struct Cli {
    /// Wallet file path
    #[arg(short, long, default_value = "~/.doli/wallet.json")]
    wallet: PathBuf,

    /// Node RPC endpoint
    #[arg(short, long, default_value = "http://127.0.0.1:8545")]
    rpc: String,

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

    /// Request withdrawal of bonds (starts 7-day delay)
    RequestWithdrawal {
        /// Number of bonds to withdraw
        #[arg(short, long)]
        count: u32,

        /// Destination address for withdrawn funds (hex pubkey_hash)
        #[arg(short, long)]
        destination: Option<String>,
    },

    /// Claim withdrawal after 7-day delay
    ClaimWithdrawal {
        /// Withdrawal index (use 'producer status' to see pending withdrawals)
        #[arg(short, long, default_value = "0")]
        index: u32,
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => {
            cmd_new(&cli.wallet, name)?;
        }
        Commands::Address { label } => {
            cmd_address(&cli.wallet, label)?;
        }
        Commands::Addresses => {
            cmd_addresses(&cli.wallet)?;
        }
        Commands::Balance { address } => {
            cmd_balance(&cli.wallet, &cli.rpc, address).await?;
        }
        Commands::Send { to, amount, fee } => {
            cmd_send(&cli.wallet, &cli.rpc, &to, &amount, fee).await?;
        }
        Commands::History { limit } => {
            cmd_history(&cli.wallet, &cli.rpc, limit).await?;
        }
        Commands::Export { output } => {
            cmd_export(&cli.wallet, &output)?;
        }
        Commands::Import { input } => {
            cmd_import(&cli.wallet, &input)?;
        }
        Commands::Info => {
            cmd_info(&cli.wallet)?;
        }
        Commands::Sign { message, address } => {
            cmd_sign(&cli.wallet, &message, address)?;
        }
        Commands::Verify {
            message,
            signature,
            pubkey,
        } => {
            cmd_verify(&message, &signature, &pubkey)?;
        }
        Commands::Producer { command } => {
            cmd_producer(&cli.wallet, &cli.rpc, command).await?;
        }
        Commands::Rewards { command } => {
            cmd_rewards(&cli.wallet, &cli.rpc, command).await?;
        }
        Commands::Chain => {
            cmd_chain(&cli.rpc).await?;
        }
        Commands::Update { command } => {
            cmd_update(&cli.wallet, &cli.rpc, command).await?;
        }
        Commands::Maintainer { command } => {
            cmd_maintainer(&cli.rpc, command).await?;
        }
    }

    Ok(())
}

/// Default address prefix (mainnet). Used when no node connection is available.
const DEFAULT_PREFIX: &str = "doli";

fn cmd_new(wallet_path: &PathBuf, name: Option<String>) -> Result<()> {
    let name = name.unwrap_or_else(|| "default".to_string());
    println!("Creating new wallet: {}", name);

    let wallet = Wallet::new(&name);
    wallet.save(wallet_path)?;

    let bech32_addr = wallet.primary_bech32_address(DEFAULT_PREFIX);

    println!("Wallet created at: {:?}", wallet_path);
    println!();
    println!("  Address: {}", bech32_addr);
    println!();
    println!("Back up your wallet!");

    Ok(())
}

fn cmd_address(wallet_path: &Path, label: Option<String>) -> Result<()> {
    let mut wallet = Wallet::load(wallet_path)?;

    let _hex_address = wallet.generate_address(label.as_deref())?;
    wallet.save(wallet_path)?;

    // Display bech32m for the newly generated address (last in list)
    let new_addr = wallet.addresses().last().expect("just added");
    let pubkey_bytes = hex::decode(&new_addr.public_key)?;
    let bech32 = crypto::address::from_pubkey(&pubkey_bytes, DEFAULT_PREFIX)
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
        let bech32 = crypto::address::from_pubkey(&pubkey_bytes, DEFAULT_PREFIX)
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
        println!("Warning: Cannot connect to node at {}", rpc_endpoint);
        println!("Make sure a DOLI node is running and the RPC endpoint is correct.");
        return Ok(());
    }

    let mut total_confirmed = 0u64;
    let mut total_unconfirmed = 0u64;

    println!("Balances:");
    println!("{:-<60}", "");

    let mut total_immature: u64 = 0;

    if let Some(addr) = &address {
        // Resolve user-supplied address (doli1... or hex)
        let pubkey_hash =
            crypto::address::resolve(addr, None).map_err(|e| anyhow::anyhow!("{}", e))?;
        let pubkey_hash_hex = pubkey_hash.to_hex();
        let display_addr = crypto::address::encode(&pubkey_hash, DEFAULT_PREFIX)
            .unwrap_or_else(|_| pubkey_hash_hex.clone());

        match rpc.get_balance(&pubkey_hash_hex).await {
            Ok(balance) => {
                println!("{}", display_addr);
                println!("  Confirmed:   {}", format_balance(balance.confirmed));
                println!("  Unconfirmed: {}", format_balance(balance.unconfirmed));
                println!("  Immature:    {}", format_balance(balance.immature));
                println!("  Total:       {}", format_balance(balance.total));
                println!();

                total_confirmed += balance.confirmed;
                total_unconfirmed += balance.unconfirmed;
                total_immature += balance.immature;
            }
            Err(e) => {
                println!("{}: Error - {}", display_addr, e);
            }
        }
    } else {
        for wallet_addr in wallet.addresses() {
            let pubkey_bytes = hex::decode(&wallet_addr.public_key)?;
            let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, &pubkey_bytes);
            let pubkey_hash_hex = pubkey_hash.to_hex();

            let label = wallet_addr.label.as_deref().unwrap_or("");
            let bech32 = crypto::address::encode(&pubkey_hash, DEFAULT_PREFIX)
                .unwrap_or_else(|_| pubkey_hash_hex.clone());

            match rpc.get_balance(&pubkey_hash_hex).await {
                Ok(balance) => {
                    println!(
                        "{} {}",
                        bech32,
                        if !label.is_empty() {
                            format!("({})", label)
                        } else {
                            String::new()
                        }
                    );
                    println!("  Confirmed:   {}", format_balance(balance.confirmed));
                    println!("  Unconfirmed: {}", format_balance(balance.unconfirmed));
                    println!("  Immature:    {}", format_balance(balance.immature));
                    println!("  Total:       {}", format_balance(balance.total));
                    println!();

                    total_confirmed += balance.confirmed;
                    total_unconfirmed += balance.unconfirmed;
                    total_immature += balance.immature;
                }
                Err(e) => {
                    println!("{}: Error - {}", bech32, e);
                }
            }
        }
    }

    if address.is_none() && wallet.addresses().len() > 1 {
        println!("{:-<60}", "");
        println!("Total Confirmed:   {}", format_balance(total_confirmed));
        println!("Total Unconfirmed: {}", format_balance(total_unconfirmed));
        println!("Total Immature:    {}", format_balance(total_immature));
        println!(
            "Total:             {}",
            format_balance(total_confirmed + total_unconfirmed + total_immature)
        );
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
        println!("Error: Cannot connect to node at {}", rpc_endpoint);
        return Ok(());
    }

    // Parse recipient address (doli1... or 64-char hex pubkey_hash)
    let recipient_hash = crypto::address::resolve(to, None)
        .map_err(|e| anyhow::anyhow!("Invalid recipient address: {}", e))?;

    // Parse amount
    let amount_coins: f64 = amount
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid amount: {}", amount))?;
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

    let recipient_display = crypto::address::encode(&recipient_hash, DEFAULT_PREFIX)
        .unwrap_or_else(|_| recipient_hash.to_hex());

    // Get the sender's pubkey_hash for UTXO lookup
    let from_pubkey_hash = wallet.primary_pubkey_hash();

    // Get spendable UTXOs
    let utxos = rpc.get_utxos(&from_pubkey_hash, true).await?;

    if utxos.is_empty() {
        println!("Error: No spendable UTXOs available");
        println!("Note: Coinbase outputs require 100 confirmations before they can be spent.");
        return Ok(());
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
        println!("Error: Insufficient balance");
        println!("  Available: {}", format_balance(total_available));
        println!("  Required:  {}", format_balance(required));
        return Ok(());
    }

    if total_input < required {
        println!("Error: Insufficient balance");
        println!("  Selected:  {}", format_balance(total_input));
        println!("  Required:  {}", format_balance(required));
        return Ok(());
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
        println!("Error: Cannot connect to node at {}", rpc_endpoint);
        return Ok(());
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
            println!("Error fetching history: {}", e);
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

    let bech32_addr = wallet.primary_bech32_address(DEFAULT_PREFIX);

    println!("Wallet: {}", wallet.name());
    println!("Addresses: {}", wallet.addresses().len());
    println!();
    println!("Primary Address:");
    println!("  Address:    {}", bech32_addr);
    println!("  Public Key: {}", wallet.primary_public_key());
    println!();
    println!("Use the address above for sending and receiving DOLI.");

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
        println!("Error: Cannot connect to node at {}", rpc_endpoint);
        return Ok(());
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
                println!("Error: Bond count must be between 1 and 10000");
                return Ok(());
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
            let fee: u64 = 10000; // 0.0001 DOLI fee

            if total_available < required_amount + fee {
                println!("Error: Insufficient balance for bond");
                println!("  Required:  {} DOLI", bonds as u64 * bond_display);
                println!("  Available: {}", format_balance(total_available));
                return Ok(());
            }

            // Select UTXOs
            let mut selected_utxos = Vec::new();
            let mut total_input = 0u64;
            for utxo in &utxos {
                if total_input >= required_amount + fee {
                    break;
                }
                selected_utxos.push(utxo.clone());
                total_input += utxo.amount;
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
            let lock_until = chain_info.best_height + blocks_per_era;

            // Create registration transaction with bonds
            let mut tx = Transaction::new_registration(
                inputs,
                producer_pubkey,
                required_amount,
                lock_until,
                bonds,
            );

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
                    println!("Note: Registration requires VDF proof validation.");
                    println!("Use 'doli producer status' to check registration status.");
                }
                Err(e) => {
                    println!("Error submitting registration: {}", e);
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
                        .and_then(|bytes| crypto::address::from_pubkey(&bytes, DEFAULT_PREFIX).ok())
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

                    // Show pending withdrawals if any
                    if !info.pending_withdrawals.is_empty() {
                        println!();
                        println!("Pending Withdrawals:");
                        for (i, w) in info.pending_withdrawals.iter().enumerate() {
                            let claimable = if w.claimable { " [READY]" } else { "" };
                            println!(
                                "  [{}] {} bonds, {} net{} (requested slot {})",
                                i,
                                w.bond_count,
                                format_balance(w.net_amount),
                                claimable,
                                w.request_slot
                            );
                        }
                    }
                }
                Err(e) => {
                    if e.to_string().contains("not found") {
                        let pk_display = hex::decode(&pk)
                            .ok()
                            .and_then(|bytes| {
                                crypto::address::from_pubkey(&bytes, DEFAULT_PREFIX).ok()
                            })
                            .unwrap_or_else(|| {
                                format!("{}...{}", &pk[..16], &pk[pk.len().saturating_sub(8)..])
                            });
                        println!("Producer not found: {}", pk_display);
                        println!();
                        println!("This key is not registered as a producer.");
                        println!("Use 'doli producer register' to register.");
                    } else {
                        println!("Error: {}", e);
                    }
                }
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
                                crypto::address::from_pubkey(&pubkey_bytes, DEFAULT_PREFIX)
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
                    println!("Error fetching producers: {}", e);
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
                println!("Error: Bond count must be between 1 and 10000");
                return Ok(());
            }

            // Get network parameters from node (bond_unit is network-specific)
            let network_params = rpc.get_network_params().await?;
            let bond_unit = network_params.bond_unit;
            let bond_display = bond_unit / 100_000_000; // Convert to DOLI per bond
            let required_amount = bond_unit * count as u64;
            let fee: u64 = 10000;

            println!(
                "Adding {} bond(s) = {} DOLI",
                count,
                count as u64 * bond_display
            );
            println!();

            // Get spendable UTXOs
            let utxos = rpc.get_utxos(&pubkey_hash, true).await?;
            let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

            if total_available < required_amount + fee {
                println!("Error: Insufficient balance");
                println!("  Required:  {} DOLI", count as u64 * bond_display);
                println!("  Available: {}", format_balance(total_available));
                return Ok(());
            }

            // Select UTXOs
            let mut selected_utxos = Vec::new();
            let mut total_input = 0u64;
            for utxo in &utxos {
                if total_input >= required_amount + fee {
                    break;
                }
                selected_utxos.push(utxo.clone());
                total_input += utxo.amount;
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

            // Create add-bond transaction
            let mut tx = Transaction::new_add_bond(inputs, producer_pubkey, count);

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
                }
                Err(e) => {
                    println!("Error adding bonds: {}", e);
                }
            }
        }

        ProducerCommands::RequestWithdrawal { count, destination } => {
            let wallet = Wallet::load(wallet_path)?;
            let _keypair = wallet.primary_keypair()?;
            let pubkey_hash = wallet.primary_pubkey_hash();

            println!("Request Withdrawal");
            println!("{:-<60}", "");
            println!();

            if count < 1 {
                println!("Error: Must withdraw at least 1 bond");
                return Ok(());
            }

            // Destination defaults to wallet address (accept doli1... or hex)
            let dest_hash = match &destination {
                Some(d) => crypto::address::resolve(d, None)
                    .map_err(|e| anyhow::anyhow!("Invalid destination address: {}", e))?,
                None => Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid wallet address"))?,
            };

            let dest_display = crypto::address::encode(&dest_hash, DEFAULT_PREFIX)
                .unwrap_or_else(|_| dest_hash.to_hex());

            println!("Requesting withdrawal of {} bond(s)", count);
            println!("Destination: {}", dest_display);
            println!();
            println!("Note: Withdrawals have a 7-day delay period.");
            println!("      Use 'doli producer claim-withdrawal' after delay.");
            println!();

            // Parse producer public key
            let pubkey_bytes = hex::decode(&wallet.addresses()[0].public_key)?;
            let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

            // Create request-withdrawal transaction
            let tx = Transaction::new_request_withdrawal(producer_pubkey, count, dest_hash);

            // Sign (request-withdrawal is state-only, but still needs signature for authorization)
            let tx_hex = hex::encode(tx.serialize());
            println!("Submitting withdrawal request...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => {
                    println!("Withdrawal request submitted!");
                    println!("TX Hash: {}", hash);
                    println!();
                    println!("Use 'doli producer status' to track withdrawal status.");
                }
                Err(e) => {
                    println!("Error requesting withdrawal: {}", e);
                }
            }
        }

        ProducerCommands::ClaimWithdrawal { index } => {
            let wallet = Wallet::load(wallet_path)?;
            let pubkey_hash = wallet.primary_pubkey_hash();

            println!("Claim Withdrawal");
            println!("{:-<60}", "");
            println!();

            // First check producer status to get pending withdrawals
            let pk = wallet.addresses()[0].public_key.clone();
            let producer_info = rpc.get_producer(&pk).await?;

            if producer_info.pending_withdrawals.is_empty() {
                println!("No pending withdrawals found.");
                return Ok(());
            }

            let withdrawal = producer_info
                .pending_withdrawals
                .get(index as usize)
                .ok_or_else(|| anyhow::anyhow!("Withdrawal index {} not found", index))?;

            if !withdrawal.claimable {
                println!("Error: Withdrawal [{}] is not yet claimable.", index);
                println!("       Wait for the 7-day delay period to complete.");
                return Ok(());
            }

            println!("Claiming withdrawal [{}]:", index);
            println!("  Bonds:      {}", withdrawal.bond_count);
            println!("  Net Amount: {}", format_balance(withdrawal.net_amount));
            println!();

            // Parse producer public key
            let pubkey_bytes = hex::decode(&wallet.addresses()[0].public_key)?;
            let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

            // Destination is the wallet's pubkey hash
            let dest_hash = Hash::from_hex(&pubkey_hash)
                .ok_or_else(|| anyhow::anyhow!("Invalid destination"))?;

            // Create claim-withdrawal transaction
            let tx = Transaction::new_claim_withdrawal(
                producer_pubkey,
                index,
                withdrawal.net_amount,
                dest_hash,
            );

            let tx_hex = hex::encode(tx.serialize());
            println!("Submitting claim transaction...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => {
                    println!("Withdrawal claimed successfully!");
                    println!("TX Hash: {}", hash);
                    println!();
                    println!("Funds will be available in your wallet shortly.");
                }
                Err(e) => {
                    println!("Error claiming withdrawal: {}", e);
                }
            }
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
                println!("Producer has already exited.");
                return Ok(());
            }

            // Calculate early exit penalty based on time active
            let current_info = rpc.get_chain_info().await?;
            let blocks_active = current_info
                .best_height
                .saturating_sub(producer_info.registration_height);
            let slots_per_year = 365 * 24 * 60; // Approximate

            let years_active = blocks_active as f64 / slots_per_year as f64;
            let penalty_pct = if years_active < 1.0 {
                75
            } else if years_active < 2.0 {
                50
            } else if years_active < 3.0 {
                25
            } else {
                0
            };

            if penalty_pct > 0 && !force {
                println!("Warning: Early exit penalty applies!");
                println!();
                println!("  Time active:   {:.1} years", years_active);
                println!("  Penalty:       {}%", penalty_pct);
                println!(
                    "  Bond amount:   {}",
                    format_balance(producer_info.bond_amount)
                );
                println!(
                    "  Penalty loss:  {}",
                    format_balance(producer_info.bond_amount * penalty_pct / 100)
                );
                println!(
                    "  You receive:   {}",
                    format_balance(producer_info.bond_amount * (100 - penalty_pct) / 100)
                );
                println!();
                println!("Use --force to proceed with early exit.");
                println!(
                    "Or wait {} more years to exit without penalty.",
                    4.0 - years_active
                );
                return Ok(());
            }

            // Parse producer public key
            let pubkey_bytes = hex::decode(&pk)?;
            let producer_pubkey = PublicKey::try_from_slice(&pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key: {}", e))?;

            // Create exit transaction
            let tx = Transaction::new_exit(producer_pubkey);
            let tx_hex = hex::encode(tx.serialize());

            println!("Submitting exit transaction...");

            match rpc.send_transaction(&tx_hex).await {
                Ok(hash) => {
                    println!("Exit submitted successfully!");
                    println!("TX Hash: {}", hash);
                    if penalty_pct > 0 {
                        println!();
                        println!(
                            "Penalty of {}% applied. Remaining bond will be returned.",
                            penalty_pct
                        );
                    }
                }
                Err(e) => {
                    println!("Error submitting exit: {}", e);
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
                println!();
                println!("Error: Blocks are for different slots!");
                println!("  Block 1 slot: {}", block1_resp.slot);
                println!("  Block 2 slot: {}", block2_resp.slot);
                println!();
                println!("Slashing requires two different blocks for the SAME slot.");
                return Ok(());
            }

            // Verify they're from the same producer
            if block1_resp.producer != block2_resp.producer {
                println!();
                println!("Error: Blocks are from different producers!");
                println!("  Block 1 producer: {}", block1_resp.producer);
                println!("  Block 2 producer: {}", block2_resp.producer);
                println!();
                println!("Slashing requires blocks from the SAME producer.");
                return Ok(());
            }

            // Verify blocks are different
            if block1_resp.hash == block2_resp.hash {
                println!();
                println!("Error: Both hashes refer to the same block!");
                println!("Slashing requires two DIFFERENT blocks for the same slot.");
                return Ok(());
            }

            let producer_addr = hex::decode(&block1_resp.producer)
                .ok()
                .and_then(|bytes| crypto::address::from_pubkey(&bytes, DEFAULT_PREFIX).ok())
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
            println!("Error: Cannot connect to node at {}", rpc_endpoint);
            println!("Details: {}", e);
            println!();
            println!("Make sure a DOLI node is running and accessible.");
        }
    }

    Ok(())
}

async fn cmd_rewards(
    wallet_path: &Path,
    rpc_endpoint: &str,
    command: RewardsCommands,
) -> Result<()> {
    use crypto::{signature, Hash};

    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        println!("Error: Cannot connect to node at {}", rpc_endpoint);
        return Ok(());
    }

    match command {
        RewardsCommands::List => {
            let wallet = Wallet::load(wallet_path)?;
            let producer_pubkey = &wallet.addresses()[0].public_key;

            println!("Claimable Epoch Rewards");
            println!("{:-<70}", "");
            println!();

            match rpc.get_claimable_rewards(producer_pubkey).await {
                Ok(info) => {
                    let producer_addr = hex::decode(producer_pubkey)
                        .ok()
                        .and_then(|bytes| crypto::address::from_pubkey(&bytes, DEFAULT_PREFIX).ok())
                        .unwrap_or_else(|| {
                            format!(
                                "{}...{}",
                                &producer_pubkey[..16],
                                &producer_pubkey[producer_pubkey.len() - 8..]
                            )
                        });
                    println!("Producer: {}", producer_addr);
                    println!("Current Height: {}", info.current_height);
                    println!("Current Epoch:  {}", info.current_epoch);
                    println!();

                    if info.epochs.is_empty() {
                        println!("No claimable epochs found.");
                        println!();
                        println!("Note: Rewards accumulate each epoch (360 blocks).");
                        println!("      Only complete epochs can be claimed.");
                    } else {
                        println!(
                            "{:<8} {:<12} {:<12} {:<10} {:<18}",
                            "Epoch", "Blocks", "Presence", "Rate", "Estimated Reward"
                        );
                        println!("{:-<60}", "");

                        for entry in &info.epochs {
                            println!(
                                "{:<8} {:<12} {:<12} {:<10} {:<18}",
                                entry.epoch,
                                format!("{}/{}", entry.blocks_present, entry.total_blocks),
                                format!("{}%", entry.presence_rate),
                                "",
                                format_balance(entry.estimated_reward)
                            );
                        }

                        println!("{:-<60}", "");
                        println!(
                            "Claimable Epochs: {}   Total: {}",
                            info.claimable_count,
                            format_balance(info.total_estimated_reward)
                        );
                        println!();
                        println!("Use 'doli rewards claim <epoch>' to claim a specific epoch.");
                        println!("Use 'doli rewards claim-all' to claim all epochs.");
                    }
                }
                Err(e) => {
                    if e.to_string().contains("not found") {
                        println!("Producer not found in the network.");
                        println!("Use 'doli producer register' to become a producer first.");
                    } else {
                        println!("Error fetching claimable rewards: {}", e);
                    }
                }
            }
        }

        RewardsCommands::Claim { epoch, recipient } => {
            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let producer_pubkey = &wallet.addresses()[0].public_key;

            println!("Claim Epoch Reward");
            println!("{:-<60}", "");
            println!();
            println!("Epoch: {}", epoch);

            // Build the claim transaction
            match rpc
                .build_claim_tx(producer_pubkey, epoch, recipient.as_deref())
                .await
            {
                Ok(claim_info) => {
                    println!("Amount: {}", format_balance(claim_info.amount));
                    println!("Recipient: {}", claim_info.recipient);
                    println!();

                    // Sign the transaction
                    let signing_message = Hash::from_hex(&claim_info.signing_message)
                        .ok_or_else(|| anyhow::anyhow!("Invalid signing message"))?;

                    let sig = signature::sign_hash(&signing_message, keypair.private_key());

                    // Reconstruct the signed transaction
                    // The unsigned_tx has a 64-byte zero placeholder for signature at the end of extra_data
                    let mut tx_bytes = hex::decode(&claim_info.unsigned_tx)
                        .map_err(|e| anyhow::anyhow!("Invalid unsigned tx hex: {}", e))?;

                    // Replace the last 64 bytes with the actual signature
                    let sig_bytes = sig.as_bytes();
                    let tx_len = tx_bytes.len();
                    tx_bytes[tx_len - 64..].copy_from_slice(sig_bytes);

                    let tx_hex = hex::encode(&tx_bytes);

                    println!("Submitting claim transaction...");

                    match rpc.send_transaction(&tx_hex).await {
                        Ok(hash) => {
                            println!("Claim submitted successfully!");
                            println!("TX Hash: {}", hash);
                            println!();
                            println!("Reward will be available after confirmation.");
                        }
                        Err(e) => {
                            println!("Error submitting claim: {}", e);
                        }
                    }
                }
                Err(e) => {
                    if e.to_string().contains("not yet complete") {
                        println!("Error: Epoch {} is not yet complete.", epoch);
                        println!("Wait for the epoch to end before claiming.");
                    } else if e.to_string().contains("already claimed") {
                        println!("Error: Epoch {} has already been claimed.", epoch);
                    } else if e.to_string().contains("No reward") {
                        println!("Error: No reward to claim for epoch {}.", epoch);
                        println!("You may not have been present during this epoch.");
                    } else {
                        println!("Error building claim: {}", e);
                    }
                }
            }
        }

        RewardsCommands::ClaimAll { recipient } => {
            let wallet = Wallet::load(wallet_path)?;
            let keypair = wallet.primary_keypair()?;
            let producer_pubkey = &wallet.addresses()[0].public_key;

            println!("Claim All Epoch Rewards");
            println!("{:-<60}", "");
            println!();

            // Get all claimable epochs
            let claimable = match rpc.get_claimable_rewards(producer_pubkey).await {
                Ok(info) => info,
                Err(e) => {
                    println!("Error fetching claimable rewards: {}", e);
                    return Ok(());
                }
            };

            if claimable.epochs.is_empty() {
                println!("No claimable epochs found.");
                return Ok(());
            }

            println!(
                "Found {} claimable epoch(s), total: {}",
                claimable.claimable_count,
                format_balance(claimable.total_estimated_reward)
            );
            println!();

            let mut claimed = 0;
            let mut total_claimed: u64 = 0;

            for entry in &claimable.epochs {
                print!("Claiming epoch {}... ", entry.epoch);

                match rpc
                    .build_claim_tx(producer_pubkey, entry.epoch, recipient.as_deref())
                    .await
                {
                    Ok(claim_info) => {
                        // Sign the transaction
                        let signing_message = match Hash::from_hex(&claim_info.signing_message) {
                            Some(h) => h,
                            None => {
                                println!("FAILED (invalid signing message)");
                                continue;
                            }
                        };

                        let sig = signature::sign_hash(&signing_message, keypair.private_key());

                        // Reconstruct the signed transaction
                        let mut tx_bytes = match hex::decode(&claim_info.unsigned_tx) {
                            Ok(b) => b,
                            Err(_) => {
                                println!("FAILED (invalid tx hex)");
                                continue;
                            }
                        };

                        // Replace the last 64 bytes with the signature
                        let sig_bytes = sig.as_bytes();
                        let tx_len = tx_bytes.len();
                        tx_bytes[tx_len - 64..].copy_from_slice(sig_bytes);

                        let tx_hex = hex::encode(&tx_bytes);

                        match rpc.send_transaction(&tx_hex).await {
                            Ok(hash) => {
                                println!("OK ({})", &hash[..16]);
                                claimed += 1;
                                total_claimed += claim_info.amount;
                            }
                            Err(e) => {
                                println!("FAILED ({})", e);
                            }
                        }
                    }
                    Err(e) => {
                        println!("FAILED ({})", e);
                    }
                }
            }

            println!();
            println!("{:-<60}", "");
            println!(
                "Claimed {} of {} epochs, total: {}",
                claimed,
                claimable.claimable_count,
                format_balance(total_claimed)
            );
        }

        RewardsCommands::History { limit } => {
            let wallet = Wallet::load(wallet_path)?;
            let producer_pubkey = &wallet.addresses()[0].public_key;

            println!("Claim History");
            println!("{:-<70}", "");
            println!();

            match rpc.get_claim_history(producer_pubkey, limit).await {
                Ok(history) => {
                    if history.claims.is_empty() {
                        println!("No claim history found.");
                        println!();
                        println!("Use 'doli rewards list' to see claimable epochs.");
                    } else {
                        println!(
                            "{:<8} {:<18} {:<10} {:<20}",
                            "Epoch", "Amount", "Height", "TX Hash"
                        );
                        println!("{:-<60}", "");

                        for entry in &history.claims {
                            println!(
                                "{:<8} {:<18} {:<10} {}...{}",
                                entry.epoch,
                                format_balance(entry.amount),
                                entry.height,
                                &entry.tx_hash[..8],
                                &entry.tx_hash[entry.tx_hash.len() - 6..]
                            );
                        }

                        println!();
                        println!(
                            "Total Claims: {}   Total Claimed: {}",
                            history.total_claims,
                            format_balance(history.total_claimed)
                        );
                    }
                }
                Err(e) => {
                    println!("Error fetching claim history: {}", e);
                }
            }
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
                    println!("Error fetching epoch info: {}", e);
                }
            }
        }
    }

    Ok(())
}

async fn cmd_update(wallet_path: &Path, rpc_endpoint: &str, command: UpdateCommands) -> Result<()> {
    let rpc = RpcClient::new(rpc_endpoint);

    if !rpc.ping().await? {
        println!("Error: Cannot connect to node at {}", rpc_endpoint);
        return Ok(());
    }

    match command {
        UpdateCommands::Check => {
            println!("Update Check");
            println!("{:-<60}", "");
            println!();

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
                Err(e) => println!("Error checking updates: {}", e),
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
                Err(e) => println!("Error fetching status: {}", e),
            }
        }

        UpdateCommands::Vote {
            version,
            veto,
            approve,
        } => {
            if !veto && !approve {
                println!("Error: Must specify --veto or --approve");
                return Ok(());
            }

            let wallet = Wallet::load(wallet_path)?;
            let vote_type = if veto { "veto" } else { "approve" };
            let producer_id = wallet.addresses()[0].public_key.clone();

            println!(
                "Submitting {} vote for v{}...",
                vote_type.to_uppercase(),
                version
            );

            let vote_msg = serde_json::json!({
                "version": version,
                "vote": vote_type,
                "producer_id": producer_id,
            });

            match rpc.submit_vote(vote_msg).await {
                Ok(result) => {
                    let status = result
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    println!("Vote {}: {}", vote_type, status);
                }
                Err(e) => println!("Error submitting vote: {}", e),
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
                Err(e) => println!("Error fetching votes: {}", e),
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
        println!("Error: Cannot connect to node at {}", rpc_endpoint);
        return Ok(());
    }

    match command {
        MaintainerCommands::List => {
            println!("Maintainer Set");
            println!("{:-<60}", "");
            println!();

            match rpc.get_maintainer_set().await {
                Ok(set) => {
                    if let Some(members) = set.get("members").and_then(|m| m.as_array()) {
                        let threshold = set.get("threshold").and_then(|t| t.as_u64()).unwrap_or(3);
                        println!("Threshold: {} of {}", threshold, members.len());
                        println!();

                        for (i, member) in members.iter().enumerate() {
                            let key = member.as_str().unwrap_or("?");
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
                Err(e) => println!("Error fetching maintainer set: {}", e),
            }
        }
    }

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
        assert_eq!(cli.wallet, PathBuf::from("/tmp/test_wallet.json"));
        assert_eq!(cli.rpc, "http://localhost:9999");
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
