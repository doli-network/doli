use anyhow::Result;
use crypto::{bls_sign_pop, signature, BlsSecretKey, Hash, PublicKey};
use doli_core::{Input, Output, Transaction};

use crate::rpc_client::{format_balance, RpcClient};
use crate::wallet::Wallet;

pub(super) async fn handle_register(
    wallet: &Wallet,
    keypair: &crypto::KeyPair,
    pubkey_hash: &str,
    rpc: &RpcClient,
    bonds: u32,
) -> Result<()> {
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

    // Get spendable normal UTXOs (exclude bonds, conditioned, NFTs, tokens, etc.)
    let utxos: Vec<_> = rpc
        .get_utxos(pubkey_hash, true)
        .await?
        .into_iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();
    let total_available: u64 = utxos.iter().map(|u| u.amount).sum();

    // Calculate fee: base + per-byte for Bond output extra_data (4 bytes each)
    let mut selected_utxos = Vec::new();
    let mut total_input = 0u64;
    let fee = {
        // Each bond output has 4 bytes extra_data (creation_slot)
        let extra_bytes = bonds as u64 * 4;
        doli_core::consensus::BASE_FEE + extra_bytes * doli_core::consensus::FEE_PER_BYTE
    };
    for utxo in &utxos {
        if total_input >= required_amount + fee {
            break;
        }
        selected_utxos.push(utxo.clone());
        total_input += utxo.amount;
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
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| anyhow::anyhow!("Invalid UTXO tx_hash"))?;
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
    let current_epoch = (chain_info.best_slot / network_params.blocks_per_reward_epoch) as u32;
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
        bls_pubkey: {
            let bls_priv_hex = wallet.addresses()[0]
                .bls_private_key
                .as_ref()
                .ok_or_else(|| {
                    anyhow::anyhow!("Wallet has no BLS key. Run: doli --wallet <wallet> add-bls")
                })?;
            let bls_sk = BlsSecretKey::from_hex(bls_priv_hex)
                .map_err(|e| anyhow::anyhow!("Invalid BLS secret key: {}", e))?;
            let bls_pk = bls_sk.public_key();
            bls_pk.as_bytes().to_vec()
        },
        bls_pop: {
            let bls_priv_hex = wallet.addresses()[0].bls_private_key.as_ref().unwrap(); // already validated above
            let bls_sk = BlsSecretKey::from_hex(bls_priv_hex).unwrap();
            let bls_pk = bls_sk.public_key();
            let pop = bls_sign_pop(&bls_sk, &bls_pk)
                .map_err(|e| anyhow::anyhow!("Failed to generate BLS PoP: {}", e))?;
            pop.as_bytes().to_vec()
        },
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
        let change_hash =
            Hash::from_hex(pubkey_hash).ok_or_else(|| anyhow::anyhow!("Invalid change address"))?;
        tx.outputs.push(Output::normal(change, change_hash));
    }

    // Sign each input (BIP-143: per-input signing hash)
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = signature::sign_hash(&signing_hash, keypair.private_key());
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

    Ok(())
}
