//! Producer management Tauri commands.
//!
//! Handles producer registration, status, bond management, withdrawal,
//! and exit operations.

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
/// **Limitation**: Producer registration requires VDF (Verifiable Delay Function)
/// proof computation, which depends on `doli-core` and the GMP library. The GUI
/// wallet crate intentionally does not depend on `doli-core` to keep cross-platform
/// builds simple (no MSYS2/GMP needed on Windows).
///
/// Registration MUST be done via the DOLI CLI (`doli producer register`) or a
/// running node. The GUI supports all other producer operations: add-bond,
/// request-withdrawal, simulate-withdrawal, exit, and status queries.
#[tauri::command]
pub async fn register_producer(
    _bond_count: u32,
    _state: State<'_, AppState>,
) -> Result<TxResponse, String> {
    Err(
        "Producer registration requires VDF proof computation, which is not available in the GUI. \
         Please use the DOLI CLI: `doli producer register`"
            .to_string(),
    )
}

/// Add bonds to an existing producer.
#[tauri::command]
pub async fn add_bonds(count: u32, state: State<'_, AppState>) -> Result<TxResponse, String> {
    let (pubkey_hash, sender_hash) = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        let pkh = w.primary_pubkey_hash().map_err(|e| e.to_string())?;
        let bytes: [u8; 32] = hex::decode(&pkh)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "Invalid pubkey hash length".to_string())?;
        (pkh, bytes)
    };

    let utxos = {
        let rpc = state.rpc_client.read().await;
        rpc.get_utxos(&pubkey_hash, true)
            .await
            .map_err(|e| e.to_string())?
    };

    let fee = 1u64;
    let mut builder = wallet::TxBuilder::build_add_bond(&utxos, count, sender_hash, fee)
        .map_err(|e| e.to_string())?;

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
        tx_type: "AddBond".to_string(),
        message: format!(
            "Added {} bonds ({} DOLI)",
            count,
            wallet::units_to_coins(count as u64 * wallet::BOND_UNIT)
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

    let mut builder =
        wallet::TxBuilder::build_request_withdrawal(bond_count, sender_hash, dest_hash)
            .map_err(|e| e.to_string())?;

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
