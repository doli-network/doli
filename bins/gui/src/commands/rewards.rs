//! Rewards Tauri commands.
//!
//! Handles listing claimable rewards, claiming individual or all rewards.

use tauri::State;

use crate::commands::{RewardEpochResponse, TxResponse};
use crate::state::AppState;

/// List all reward epochs for the current producer.
#[tauri::command]
pub async fn list_rewards(state: State<'_, AppState>) -> Result<Vec<RewardEpochResponse>, String> {
    let public_key = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        w.primary_public_key().to_string()
    };

    let rewards = {
        let rpc = state.rpc_client.read().await;
        rpc.get_rewards_list(&public_key)
            .await
            .map_err(|e| e.to_string())?
    };

    let entries: Vec<RewardEpochResponse> = rewards
        .into_iter()
        .map(|r| RewardEpochResponse {
            epoch: r.epoch,
            estimated_reward: r.estimated_reward,
            formatted_reward: wallet::format_balance(r.estimated_reward),
            qualified: r.qualified,
            claimed: r.claimed,
        })
        .collect();

    Ok(entries)
}

/// Claim reward for a specific epoch.
#[tauri::command]
pub async fn claim_reward(
    epoch: u64,
    recipient: Option<String>,
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

    let recipient_hash = match recipient {
        Some(r) => {
            let bytes: [u8; 32] = hex::decode(&r)
                .map_err(|e| e.to_string())?
                .try_into()
                .map_err(|_| "Invalid recipient hash".to_string())?;
            Some(bytes)
        }
        None => None,
    };

    let mut builder = wallet::TxBuilder::build_reward_claim(epoch, sender_hash, recipient_hash)
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
        tx_type: "RewardClaim".to_string(),
        message: format!("Claimed reward for epoch {}", epoch),
    })
}

/// Claim all unclaimed rewards.
#[tauri::command]
pub async fn claim_all_rewards(state: State<'_, AppState>) -> Result<Vec<TxResponse>, String> {
    let (public_key, pubkey_hash) = {
        let wallet_guard = state.wallet.read().await;
        let w = wallet_guard.as_ref().ok_or("No wallet loaded")?;
        (
            w.primary_public_key().to_string(),
            w.primary_pubkey_hash().map_err(|e| e.to_string())?,
        )
    };
    let sender_hash: [u8; 32] = hex::decode(&pubkey_hash)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "Invalid pubkey hash length".to_string())?;

    let rewards = {
        let rpc = state.rpc_client.read().await;
        rpc.get_rewards_list(&public_key)
            .await
            .map_err(|e| e.to_string())?
    };

    let unclaimed: Vec<_> = rewards
        .into_iter()
        .filter(|r| r.qualified && !r.claimed)
        .collect();

    let mut results = Vec::new();
    for reward in unclaimed {
        let mut builder = wallet::TxBuilder::build_reward_claim(reward.epoch, sender_hash, None)
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

        results.push(TxResponse {
            tx_hash,
            tx_type: "RewardClaim".to_string(),
            message: format!("Claimed reward for epoch {}", reward.epoch),
        });
    }

    Ok(results)
}
