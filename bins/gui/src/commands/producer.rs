//! Producer management Tauri commands.
//!
//! Handles producer registration, status, bond management, withdrawal,
//! and exit operations.

use crypto::{bls_sign_pop, BlsSecretKey, Hash, PublicKey};
use doli_producer_tx::producer_tx::{
    build_signed_registration, chain_genesis_hash, RegistrationSpec,
};
use tauri::State;

use crate::commands::{ProducerStatusResponse, SimulateResponse, TxResponse};
use crate::state::AppState;

/// Get producer status for the current wallet's public key.
#[tauri::command]
pub async fn producer_status(state: State<'_, AppState>) -> Result<ProducerStatusResponse, String> {
    let public_key = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        w.primary_public_key().to_string()
    };

    let producers = {
        let rpc = state.rpc_client.read().await;
        rpc.get_producers().await.map_err(|e| e.to_string())?
    };

    match producers.iter().find(|p| p.public_key == public_key) {
        Some(producer) => Ok(ProducerStatusResponse {
            is_registered: true,
            status: producer.status.clone(),
            bond_count: producer.bond_count,
            bond_amount: producer.bond_amount,
            formatted_bond_amount: wallet::format_balance(producer.bond_amount),
            registration_height: producer.registration_height,
            era: producer.era,
        }),
        None => Ok(ProducerStatusResponse {
            is_registered: false,
            status: "not_registered".to_string(),
            bond_count: 0,
            bond_amount: 0,
            formatted_bond_amount: wallet::format_balance(0),
            registration_height: 0,
            era: 0,
        }),
    }
}

/// Register as a producer.
///
/// Computes a hash-chain VDF proof, builds a Registration transaction with bond
/// outputs, BLS key + proof-of-possession, and submits it via RPC.
#[tauri::command]
pub async fn register_producer(
    bond_count: u32,
    state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    if bond_count == 0 || bond_count > 10_000 {
        return Err("Bond count must be between 1 and 10,000".to_string());
    }

    // Extract wallet data (pubkey, BLS key, keypair)
    let (pk_hex, pubkey_bytes, bls_priv_hex) = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let addr = &w.addresses()[0];
        let bls = addr
            .bls_private_key
            .as_ref()
            .ok_or("Wallet has no BLS key. Use wallet settings to add one.")?
            .clone();
        (addr.public_key.clone(), addr.public_key.clone(), bls)
    };

    // Check if already registered
    {
        let rpc = state.rpc_client.read().await;
        let producers = rpc.get_producers().await.map_err(|e| e.to_string())?;
        if let Some(p) = producers.iter().find(|p| p.public_key == pk_hex) {
            match p.status.to_lowercase().as_str() {
                "active" => {
                    return Err(format!(
                    "Already registered as active producer (bonds: {}). Use add-bond to increase.",
                    p.bond_count
                ))
                }
                "pending" => return Err(
                    "Registration already pending. It will activate at the next epoch boundary."
                        .to_string(),
                ),
                _ => {} // exited/slashed — allow re-registration
            }
        }
    }

    // Get chain info and network params
    let (chain_info, network_params) = {
        let rpc = state.rpc_client.read().await;
        let ci = rpc.get_chain_info().await.map_err(|e| e.to_string())?;
        let np = rpc.get_network_params().await.map_err(|e| e.to_string())?;
        (ci, np)
    };

    let bond_unit = network_params.bond_unit;
    let required_amount = bond_unit * bond_count as u64;

    // Get spendable UTXOs
    let pubkey_hash = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        w.primary_pubkey_hash().map_err(|e| e.to_string())?
    };

    let utxos: Vec<wallet::Utxo> = {
        let rpc = state.rpc_client.read().await;
        rpc.get_utxos(&pubkey_hash, true)
            .await
            .map_err(|e| e.to_string())?
    };

    let spendable: Vec<_> = utxos
        .iter()
        .filter(|u| u.output_type == "normal" && u.spendable)
        .collect();
    let total_available: u64 = spendable.iter().map(|u| u.amount).sum();

    // Calculate fee
    let extra_bytes = bond_count as u64 * 4;
    let fee = doli_core::consensus::BASE_FEE
        + extra_bytes * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR;

    if total_available < required_amount + fee {
        return Err(format!(
            "Insufficient balance: need {} DOLI + fee, have {}",
            wallet::units_to_coins(required_amount),
            wallet::format_balance(total_available)
        ));
    }

    // Select UTXOs
    let mut selected = Vec::new();
    let mut total_input = 0u64;
    for utxo in &spendable {
        if total_input >= required_amount + fee {
            break;
        }
        selected.push(utxo);
        total_input += utxo.amount;
    }

    // Build inputs
    let mut inputs: Vec<(Hash, u32)> = Vec::new();
    for utxo in &selected {
        let prev_tx_hash =
            Hash::from_hex(&utxo.tx_hash).ok_or_else(|| "Invalid UTXO tx_hash".to_string())?;
        inputs.push((prev_tx_hash, utxo.output_index));
    }

    // Parse producer public key
    let producer_pubkey = {
        let bytes = hex::decode(&pubkey_bytes).map_err(|e| e.to_string())?;
        PublicKey::try_from_slice(&bytes).map_err(|e| format!("Invalid public key: {}", e))?
    };

    let lock_until = bond_lock_until(&chain_info);

    // Compute hash-chain VDF proof
    let current_epoch = (chain_info.best_slot / network_params.blocks_per_reward_epoch) as u32;
    let vdf_input = vdf::registration_input(&producer_pubkey, current_epoch);
    let vdf_output = doli_core::tpop::heartbeat::hash_chain_vdf(&vdf_input, vdf::T_REGISTER_BASE);

    // Build BLS key + proof-of-possession
    let bls_sk = BlsSecretKey::from_hex(&bls_priv_hex)
        .map_err(|e| format!("Invalid BLS secret key: {}", e))?;
    let bls_pk = bls_sk.public_key();
    let bls_pop =
        bls_sign_pop(&bls_sk, &bls_pk).map_err(|e| format!("Failed to generate BLS PoP: {}", e))?;

    // Build registration data
    let reg_data = doli_core::transaction::RegistrationData {
        public_key: producer_pubkey,
        epoch: current_epoch,
        vdf_output: vdf_output.to_vec(),
        vdf_proof: vec![],
        prev_registration_hash: Hash::ZERO,
        sequence_number: 0,
        bond_count,
        bls_pubkey: bls_pk.as_bytes().to_vec(),
        bls_pop: bls_pop.as_bytes().to_vec(),
    };

    let change_to =
        Hash::from_hex(&pubkey_hash).ok_or_else(|| "Invalid change address".to_string())?;
    let spec = RegistrationSpec {
        inputs,
        reg_data,
        bond_unit,
        lock_until,
        change: total_input - required_amount - fee,
        change_to,
    };

    // Inputs and the producer-auth signature over the chain genesis.
    let genesis = chain_genesis_hash(&chain_info.genesis_hash)?;
    let keypair = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        w.primary_keypair().map_err(|e| e.to_string())?
    };
    let tx = build_signed_registration(&spec, &keypair, &genesis)?;

    // Submit
    let tx_hex = hex::encode(tx.serialize());
    let tx_hash = {
        let rpc = state.rpc_client.read().await;
        rpc.send_transaction(&tx_hex)
            .await
            .map_err(|e| format!("Registration failed: {}", e))?
    };

    Ok(TxResponse {
        tx_hash,
        tx_type: "Registration".to_string(),
        message: format!(
            "Registered with {} bond(s) ({} DOLI)",
            bond_count,
            wallet::units_to_coins(bond_count as u64 * bond_unit)
        ),
    })
}

/// Add bonds to an existing producer.
#[tauri::command]
pub async fn add_bonds(count: u32, state: State<'_, AppState>) -> Result<TxResponse, String> {
    let pubkey_hash = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        w.primary_pubkey_hash().map_err(|e| e.to_string())?
    };

    let (utxos, chain_info, network_params) = {
        let rpc = state.rpc_client.read().await;
        let utxos = rpc
            .get_utxos(&pubkey_hash, true)
            .await
            .map_err(|e| e.to_string())?;
        let ci = rpc.get_chain_info().await.map_err(|e| e.to_string())?;
        let np = rpc.get_network_params().await.map_err(|e| e.to_string())?;
        (utxos, ci, np)
    };

    let bond_unit = network_params.bond_unit;
    let fee = doli_core::consensus::BASE_FEE
        + count as u64 * 4 * doli_core::consensus::FEE_PER_BYTE / doli_core::consensus::FEE_DIVISOR;

    let tx_hex = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let keypair = w.primary_keypair().map_err(|e| e.to_string())?;
        let signer = wallet::ProducerSigner {
            keypair: &keypair,
            genesis_hash: &chain_info.genesis_hash,
        };
        wallet::TxBuilder::build_add_bond(
            &utxos,
            count,
            bond_unit,
            bond_lock_until(&chain_info),
            fee,
            &signer,
        )
        .map_err(|e| e.to_string())?
    };

    let tx_hash = {
        let rpc = state.rpc_client.read().await;
        rpc.send_transaction(&tx_hex)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(TxResponse {
        tx_hash,
        tx_type: "AddBond".to_string(),
        message: format!(
            "Added {} bonds ({} DOLI)",
            count,
            wallet::units_to_coins(count as u64 * bond_unit)
        ),
    })
}

/// Request withdrawal of bonds.
#[tauri::command]
pub async fn request_withdrawal(
    bond_count: u32,
    dest: Option<String>,
    state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    let (pubkey_hash, public_key) = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let pkh = w.primary_pubkey_hash().map_err(|e| e.to_string())?;
        (pkh, w.primary_public_key().to_string())
    };

    let dest_hash = match dest {
        Some(d) => {
            let bytes: [u8; 32] = hex::decode(&d)
                .map_err(|e| e.to_string())?
                .try_into()
                .map_err(|_| "Invalid destination hash".to_string())?;
            Some(bytes)
        }
        None => None,
    };

    let (chain_info, sim, utxos) = {
        let rpc = state.rpc_client.read().await;
        let ci = rpc.get_chain_info().await.map_err(|e| e.to_string())?;
        let sim = rpc
            .simulate_withdrawal(&public_key, bond_count)
            .await
            .map_err(|e| e.to_string())?;
        let utxos = rpc
            .get_utxos(&pubkey_hash, false)
            .await
            .map_err(|e| e.to_string())?;
        (ci, sim, utxos)
    };

    // Exactly `bond_count` Bond inputs, oldest first as the node lists them.
    let bonds: Vec<wallet::Utxo> = utxos
        .iter()
        .filter(|u| u.output_type == "bond")
        .take(bond_count as usize)
        .cloned()
        .collect();

    let tx_hex = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let keypair = w.primary_keypair().map_err(|e| e.to_string())?;
        let signer = wallet::ProducerSigner {
            keypair: &keypair,
            genesis_hash: &chain_info.genesis_hash,
        };
        let req = wallet::WithdrawalRequest {
            network: &chain_info.network,
            best_height: chain_info.best_height,
            bond_count,
            bond_utxos: &bonds,
            total_net: sim.net_amount,
            fee_utxos: &utxos,
            fee: 1,
            destination: dest_hash,
        };
        wallet::TxBuilder::build_request_withdrawal(&req, &signer).map_err(|e| e.to_string())?
    };

    let tx_hash = {
        let rpc = state.rpc_client.read().await;
        rpc.send_transaction(&tx_hex)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(TxResponse {
        tx_hash,
        tx_type: "RequestWithdrawal".to_string(),
        message: format!("Requested withdrawal of {} bonds", bond_count),
    })
}

/// Simulate a withdrawal to preview penalty amounts.
#[tauri::command]
pub async fn simulate_withdrawal(
    bond_count: u32,
    state: State<'_, AppState>,
) -> Result<SimulateResponse, String> {
    let public_key = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        w.primary_public_key().to_string()
    };

    let sim = {
        let rpc = state.rpc_client.read().await;
        rpc.simulate_withdrawal(&public_key, bond_count)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(SimulateResponse {
        bond_count: sim.bond_count,
        total_staked: sim.total_staked,
        total_penalty: sim.total_penalty,
        net_amount: sim.net_amount,
        formatted_total_staked: wallet::format_balance(sim.total_staked),
        formatted_penalty: wallet::format_balance(sim.total_penalty),
        formatted_net: wallet::format_balance(sim.net_amount),
    })
}

/// Exit as a producer (voluntary exit).
#[tauri::command]
pub async fn exit_producer(_force: bool, state: State<'_, AppState>) -> Result<TxResponse, String> {
    let sender_hash = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let pkh = w.primary_pubkey_hash().map_err(|e| e.to_string())?;
        let bytes: [u8; 32] = hex::decode(&pkh)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "Invalid pubkey hash length".to_string())?;
        bytes
    };

    let mut builder = wallet::TxBuilder::new(wallet::TxType::ProducerExit);
    builder.add_input(sender_hash, 0);
    builder.add_output(0, sender_hash, 0, 0, Vec::new());

    let tx_hex = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let keypair = w.primary_keypair().map_err(|e| e.to_string())?;
        builder
            .sign_and_build(&keypair)
            .map_err(|e| e.to_string())?
    };

    let tx_hash = {
        let rpc = state.rpc_client.read().await;
        rpc.send_transaction(&tx_hex)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(TxResponse {
        tx_hash,
        tx_type: "ProducerExit".to_string(),
        message: "Producer exit requested".to_string(),
    })
}

/// Bond lock height: the tip plus one era plus a margin.
fn bond_lock_until(chain_info: &wallet::ChainInfo) -> u64 {
    let blocks_per_era: u64 = match chain_info.network.as_str() {
        "devnet" => 576,
        _ => 12_614_400,
    };
    chain_info.best_height + blocks_per_era + 1000
}
