//! DOLI GUI Desktop Wallet -- Tauri 2.x application entry point.
//!
//! This is the Rust backend for the DOLI desktop wallet. It provides Tauri command
//! handlers that bridge the Svelte frontend to the `crates/wallet` library.
//!
//! Key security property: Private keys NEVER cross the IPC boundary (GUI-NF-004).
//! All signing operations happen in Rust. The frontend only receives public data.

// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use state::AppState;

fn main() {
    let app_state = AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            // Wallet commands
            commands::wallet::create_wallet,
            commands::wallet::restore_wallet,
            commands::wallet::load_wallet,
            commands::wallet::generate_address,
            commands::wallet::list_addresses,
            commands::wallet::export_wallet,
            commands::wallet::import_wallet,
            commands::wallet::wallet_info,
            commands::wallet::add_bls_key,
            // Transaction commands
            commands::transaction::get_balance,
            commands::transaction::send_doli,
            commands::transaction::get_history,
            // Producer commands
            commands::producer::producer_status,
            commands::producer::register_producer,
            commands::producer::add_bonds,
            commands::producer::request_withdrawal,
            commands::producer::simulate_withdrawal,
            commands::producer::exit_producer,
            // Rewards commands
            commands::rewards::list_rewards,
            commands::rewards::claim_reward,
            commands::rewards::claim_all_rewards,
            // Network commands
            commands::network::get_chain_info,
            commands::network::set_rpc_endpoint,
            commands::network::set_network,
            commands::network::test_connection,
            commands::network::get_connection_status,
            // NFT/Token commands
            commands::nft::mint_nft,
            commands::nft::transfer_nft,
            commands::nft::nft_info,
            commands::nft::issue_token,
            commands::nft::token_info,
            // Bridge commands
            commands::bridge::bridge_lock,
            commands::bridge::bridge_claim,
            commands::bridge::bridge_refund,
            // Governance commands
            commands::governance::check_updates,
            commands::governance::update_status,
            commands::governance::vote_update,
            commands::governance::sign_message,
            commands::governance::verify_signature,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DOLI GUI");
}
