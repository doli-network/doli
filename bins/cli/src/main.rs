//! DOLI CLI - Wallet command-line interface
//!
//! This binary provides wallet functionality:
//! - Key generation and management
//! - Transaction creation and signing
//! - Balance queries
//! - Network interaction

use std::path::PathBuf;

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

    /// Show chain information
    Chain,
}

#[derive(Subcommand)]
enum ProducerCommands {
    /// Register as a block producer
    Register {
        /// Number of bonds to stake (1-100, each bond = 1,000 DOLI)
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
        /// Number of bonds to add (1-100)
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
        Commands::Chain => {
            cmd_chain(&cli.rpc).await?;
        }
    }

    Ok(())
}

fn cmd_new(wallet_path: &PathBuf, name: Option<String>) -> Result<()> {
    let name = name.unwrap_or_else(|| "default".to_string());
    println!("Creating new wallet: {}", name);

    let wallet = Wallet::new(&name);
    wallet.save(wallet_path)?;

    println!("Wallet created at: {:?}", wallet_path);
    println!();
    println!("Primary Address:");
    println!("  Address (20-byte):    {}", wallet.primary_address());
    println!("  Pubkey Hash (32-byte): {}", wallet.primary_pubkey_hash());
    println!();
    println!("Use the Pubkey Hash for RPC queries and sending coins.");

    Ok(())
}

fn cmd_address(wallet_path: &PathBuf, label: Option<String>) -> Result<()> {
    let mut wallet = Wallet::load(wallet_path)?;

    let address = wallet.generate_address(label.as_deref())?;
    wallet.save(wallet_path)?;

    println!("New address: {}", address);

    Ok(())
}

fn cmd_addresses(wallet_path: &PathBuf) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;

    println!("Addresses:");
    for (i, addr) in wallet.addresses().iter().enumerate() {
        let label = addr.label.as_deref().unwrap_or("");
        println!("  {}. {} {}", i + 1, addr.address, label);
    }

    Ok(())
}

async fn cmd_balance(
    wallet_path: &PathBuf,
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

    // Use pubkey_hash (32-byte) for RPC queries instead of address (20-byte)
    if let Some(addr) = &address {
        // If user provided an address, use it directly (assume it's already a pubkey_hash)
        match rpc.get_balance(addr).await {
            Ok(balance) => {
                println!("{}", addr);
                println!("  Confirmed:   {}", format_balance(balance.confirmed));
                println!("  Unconfirmed: {}", format_balance(balance.unconfirmed));
                println!("  Total:       {}", format_balance(balance.total));
                println!();

                total_confirmed += balance.confirmed;
                total_unconfirmed += balance.unconfirmed;
            }
            Err(e) => {
                println!("{}: Error - {}", addr, e);
            }
        }
    } else {
        // Query balance for each wallet address using pubkey_hash
        for wallet_addr in wallet.addresses() {
            // Compute pubkey_hash from the public key
            let pubkey_bytes = hex::decode(&wallet_addr.public_key)?;
            let pubkey_hash = crypto::hash::hash(&pubkey_bytes).to_hex();

            let label = wallet_addr.label.as_deref().unwrap_or("");

            match rpc.get_balance(&pubkey_hash).await {
                Ok(balance) => {
                    println!(
                        "{} {}",
                        &pubkey_hash[..16],
                        if !label.is_empty() {
                            format!("({})", label)
                        } else {
                            String::new()
                        }
                    );
                    println!("  Pubkey Hash: {}", pubkey_hash);
                    println!("  Confirmed:   {}", format_balance(balance.confirmed));
                    println!("  Unconfirmed: {}", format_balance(balance.unconfirmed));
                    println!("  Total:       {}", format_balance(balance.total));
                    println!();

                    total_confirmed += balance.confirmed;
                    total_unconfirmed += balance.unconfirmed;
                }
                Err(e) => {
                    println!("{}: Error - {}", &pubkey_hash[..16], e);
                }
            }
        }
    }

    if address.is_none() && wallet.addresses().len() > 1 {
        println!("{:-<60}", "");
        println!("Total Confirmed:   {}", format_balance(total_confirmed));
        println!("Total Unconfirmed: {}", format_balance(total_unconfirmed));
        println!(
            "Total:             {}",
            format_balance(total_confirmed + total_unconfirmed)
        );
    }

    Ok(())
}

async fn cmd_send(
    wallet_path: &PathBuf,
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

    // Parse recipient address (should be a 32-byte pubkey_hash)
    let recipient_hash = Hash::from_hex(to).ok_or_else(|| {
        anyhow::anyhow!("Invalid recipient address: expected 64-character hex pubkey_hash")
    })?;

    // Parse amount
    let amount_coins: f64 = amount
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid amount: {}", amount))?;
    let amount_units = coins_to_units(amount_coins);

    // Parse fee if provided (default: 1000 units = 0.00001 DOLI)
    let fee_units = if let Some(f) = &fee {
        let fee_coins: f64 = f
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid fee: {}", f))?;
        coins_to_units(fee_coins)
    } else {
        1000 // Default fee
    };

    println!("Preparing transaction:");
    println!("  To:     {}", to);
    println!("  Amount: {} DOLI", amount_coins);
    println!("  Fee:    {}", format_balance(fee_units));

    // Get the sender's pubkey_hash for UTXO lookup
    let from_pubkey_hash = wallet.primary_pubkey_hash();

    // Get spendable UTXOs
    let utxos = rpc.get_utxos(&from_pubkey_hash, true).await?;

    if utxos.is_empty() {
        println!("Error: No spendable UTXOs available");
        println!("Note: Coinbase outputs require 100 confirmations before they can be spent.");
        return Ok(());
    }

    // Calculate total available
    let total_available: u64 = utxos.iter().map(|u| u.amount).sum();
    let required = amount_units + fee_units;

    if total_available < required {
        println!("Error: Insufficient balance");
        println!("  Available: {}", format_balance(total_available));
        println!("  Required:  {}", format_balance(required));
        return Ok(());
    }

    // Select UTXOs (greedy selection)
    let mut selected_utxos = Vec::new();
    let mut total_input = 0u64;
    for utxo in &utxos {
        if total_input >= required {
            break;
        }
        selected_utxos.push(utxo.clone());
        total_input += utxo.amount;
    }

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

async fn cmd_history(wallet_path: &PathBuf, rpc_endpoint: &str, limit: usize) -> Result<()> {
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
        Ok(transactions) => {
            if transactions.is_empty() {
                println!("No transactions found.");
            } else {
                for tx in transactions {
                    let status = if tx.confirmations > 0 {
                        format!("{} confirmations", tx.confirmations)
                    } else {
                        "pending".to_string()
                    };

                    let tx_type = match tx.tx_type {
                        0 => "Transfer",
                        1 => "Registration",
                        2 => "Exit",
                        _ => "Unknown",
                    };

                    println!("Hash:   {}", tx.hash);
                    println!("Type:   {}", tx_type);
                    println!("Status: {}", status);
                    println!("Fee:    {}", format_balance(tx.fee));
                    if let Some(height) = tx.block_height {
                        println!("Block:  {}", height);
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

fn cmd_export(wallet_path: &PathBuf, output: &PathBuf) -> Result<()> {
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

fn cmd_info(wallet_path: &PathBuf) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;

    println!("Wallet: {}", wallet.name());
    println!("Addresses: {}", wallet.addresses().len());
    println!();
    println!("Primary Address:");
    println!("  Address (20-byte):    {}", wallet.primary_address());
    println!("  Pubkey Hash (32-byte): {}", wallet.primary_pubkey_hash());
    println!("  Public Key:           {}", wallet.primary_public_key());
    println!();
    println!("Note: Use the Pubkey Hash for RPC queries (balance, UTXOs, send).");

    Ok(())
}

fn cmd_sign(wallet_path: &PathBuf, message: &str, address: Option<String>) -> Result<()> {
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
    wallet_path: &PathBuf,
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

            // Validate bond count
            if bonds < 1 || bonds > 100 {
                println!("Error: Bond count must be between 1 and 100");
                return Ok(());
            }

            // Each bond = 1,000 DOLI = 100,000,000,000 base units
            let bond_unit: u64 = 100_000_000_000;
            let required_amount = bond_unit * bonds as u64;

            println!("Registering with {} bond(s) = {} DOLI", bonds, bonds * 1000);
            println!();

            // Get spendable UTXOs
            let utxos = rpc.get_utxos(&pubkey_hash, true).await?;
            let total_available: u64 = utxos.iter().map(|u| u.amount).sum();
            let fee: u64 = 10000; // 0.0001 DOLI fee

            if total_available < required_amount + fee {
                println!("Error: Insufficient balance for bond");
                println!("  Required:  {} DOLI", bonds * 1000);
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

            // Create registration transaction with bonds
            let mut tx =
                Transaction::new_registration(inputs, producer_pubkey, required_amount, bonds);

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
                    println!("Public Key:    {}...{}", &info.public_key[..16], &info.public_key[info.public_key.len()-8..]);
                    println!("Status:        {}", info.status);
                    println!("Registered at: block {}", info.registration_height);
                    println!("Bond Count:    {}", info.bond_count);
                    println!("Bond Amount:   {}", format_balance(info.bond_amount));
                    println!("Blocks Made:   {}", info.blocks_produced);
                    println!("Rewards:       {}", format_balance(info.pending_rewards));
                    println!("Current Era:   {}", info.era);

                    // Show pending withdrawals if any
                    if !info.pending_withdrawals.is_empty() {
                        println!();
                        println!("Pending Withdrawals:");
                        for (i, w) in info.pending_withdrawals.iter().enumerate() {
                            let claimable = if w.claimable { " [READY]" } else { "" };
                            println!(
                                "  [{}] {} bonds, {} net{} (requested slot {})",
                                i, w.bond_count, format_balance(w.net_amount), claimable, w.request_slot
                            );
                        }
                    }
                }
                Err(e) => {
                    if e.to_string().contains("not found") {
                        println!("Producer not found: {}...{}", &pk[..16], &pk[pk.len().saturating_sub(8)..]);
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
                            "{:<8} {:<18} {:<10} {:<8} {:<12} {:<10}",
                            "Status", "Public Key", "Bonds", "Blocks", "Rewards", "Era"
                        );
                        println!("{:-<80}", "");

                        for p in &producers {
                            let pk_short = format!("{}...{}", &p.public_key[..8], &p.public_key[p.public_key.len()-4..]);
                            println!(
                                "{:<8} {:<18} {:<10} {:<8} {:<12} {:<10}",
                                p.status,
                                pk_short,
                                p.bond_count,
                                p.blocks_produced,
                                format_balance(p.pending_rewards).replace(" DOLI", ""),
                                p.era
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

            if count < 1 || count > 100 {
                println!("Error: Bond count must be between 1 and 100");
                return Ok(());
            }

            let bond_unit: u64 = 100_000_000_000;
            let required_amount = bond_unit * count as u64;
            let fee: u64 = 10000;

            println!("Adding {} bond(s) = {} DOLI", count, count * 1000);
            println!();

            // Get spendable UTXOs
            let utxos = rpc.get_utxos(&pubkey_hash, true).await?;
            let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

            if total_available < required_amount + fee {
                println!("Error: Insufficient balance");
                println!("  Required:  {} DOLI", count * 1000);
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
            let keypair = wallet.primary_keypair()?;
            let pubkey_hash = wallet.primary_pubkey_hash();

            println!("Request Withdrawal");
            println!("{:-<60}", "");
            println!();

            if count < 1 {
                println!("Error: Must withdraw at least 1 bond");
                return Ok(());
            }

            // Destination defaults to wallet address
            let dest_hash = match &destination {
                Some(d) => Hash::from_hex(d)
                    .ok_or_else(|| anyhow::anyhow!("Invalid destination address"))?,
                None => Hash::from_hex(&pubkey_hash)
                    .ok_or_else(|| anyhow::anyhow!("Invalid wallet address"))?,
            };

            println!("Requesting withdrawal of {} bond(s)", count);
            println!("Destination: {}", dest_hash.to_hex());
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
            let blocks_active = current_info.best_height.saturating_sub(producer_info.registration_height);
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
                println!("  Bond amount:   {}", format_balance(producer_info.bond_amount));
                println!("  Penalty loss:  {}", format_balance(producer_info.bond_amount * penalty_pct / 100));
                println!("  You receive:   {}", format_balance(producer_info.bond_amount * (100 - penalty_pct) / 100));
                println!();
                println!("Use --force to proceed with early exit.");
                println!("Or wait {} more years to exit without penalty.", 4.0 - years_active);
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
                        println!("Penalty of {}% applied. Remaining bond will be returned.", penalty_pct);
                    }
                }
                Err(e) => {
                    println!("Error submitting exit: {}", e);
                }
            }
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
