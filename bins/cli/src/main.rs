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
    Register,

    /// Check producer status
    Status {
        /// Public key (optional, uses wallet if not specified)
        #[arg(short, long)]
        pubkey: Option<String>,
    },

    /// Withdraw from producer set
    Withdraw,
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
    let rpc = RpcClient::new(rpc_endpoint);

    // Check connection
    if !rpc.ping().await? {
        println!("Error: Cannot connect to node at {}", rpc_endpoint);
        return Ok(());
    }

    match command {
        ProducerCommands::Register => {
            let wallet = Wallet::load(wallet_path)?;
            println!("Producer Registration");
            println!("{:-<60}", "");
            println!();
            println!("To register as a producer, you need:");
            println!("  1. Complete a VDF proof (takes ~10 minutes)");
            println!("  2. Have sufficient bond (check current era requirements)");
            println!("  3. Submit a registration transaction");
            println!();
            println!(
                "Your wallet public key: {}",
                wallet.addresses()[0].public_key
            );
            println!();
            println!("Full registration flow requires additional implementation.");
            println!("See docs/BECOMING_A_PRODUCER.md for manual steps.");
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
                    println!("Public Key:    {}", info.public_key);
                    println!("Status:        {}", info.status);
                    println!("Registered at: block {}", info.registration_height);
                    println!("Bond Amount:   {}", format_balance(info.bond_amount));
                    println!("Blocks Made:   {}", info.blocks_produced);
                    println!("Current Era:   {}", info.era);
                }
                Err(e) => {
                    if e.to_string().contains("not found") {
                        println!("Producer not found: {}", pk);
                        println!();
                        println!("This key is not registered as a producer.");
                        println!("Use 'doli producer register' to register.");
                    } else {
                        println!("Error: {}", e);
                    }
                }
            }
        }

        ProducerCommands::Withdraw => {
            let wallet = Wallet::load(wallet_path)?;
            println!("Producer Withdrawal");
            println!("{:-<60}", "");
            println!();
            println!("Requesting withdrawal will:");
            println!("  1. Start a cooldown period (~1 week)");
            println!("  2. You continue producing during cooldown");
            println!("  3. After cooldown, your bond is released");
            println!();
            println!("Your public key: {}", wallet.addresses()[0].public_key);
            println!();
            println!("Full withdrawal flow requires additional implementation.");
            println!("See docs/BECOMING_A_PRODUCER.md for details.");
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
