//! Network, chain, node, epoch, and params info handlers

use serde_json::Value;

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

impl RpcContext {
    /// Get mempool info
    pub(super) async fn get_mempool_info(&self) -> Result<Value, RpcError> {
        let mempool = self.mempool.read().await;

        let response = MempoolInfoResponse {
            tx_count: mempool.len(),
            total_size: mempool.size(),
            min_fee_rate: mempool.min_fee_rate(),
            max_size: mempool.max_size(),
            max_count: mempool.max_count(),
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get network info
    pub(super) async fn get_network_info(&self) -> Result<Value, RpcError> {
        let sync_status = (self.sync_status)();
        let response = NetworkInfoResponse {
            peer_id: self.peer_id.clone(),
            peer_count: (self.peer_count)(),
            syncing: sync_status.is_syncing,
            sync_progress: sync_status.progress,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get detailed peer info
    pub(super) async fn get_peer_info(&self) -> Result<Value, RpcError> {
        let peers = (self.peer_list)();
        serde_json::to_value(&peers).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get chain info
    pub(super) async fn get_chain_info(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;

        let response = ChainInfoResponse {
            network: self.network.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            best_hash: chain_state.best_hash.to_hex(),
            best_height: chain_state.best_height,
            best_slot: chain_state.best_slot,
            genesis_hash: chain_state.genesis_hash.to_hex(),
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get node information including version
    pub(super) async fn get_node_info(&self) -> Result<Value, RpcError> {
        Ok(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "network": self.network,
            "peerId": self.peer_id,
            "peerCount": (self.peer_count)(),
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH
        }))
    }

    /// Get current reward epoch information
    pub(super) async fn get_epoch_info(&self) -> Result<Value, RpcError> {
        use doli_core::consensus::reward_epoch;

        let blocks_per_epoch = self.blocks_per_reward_epoch;
        let chain_state = self.chain_state.read().await;
        let current_height = chain_state.best_height;

        // Use network-aware epoch functions
        let current_epoch = reward_epoch::from_height_with(current_height, blocks_per_epoch);
        let (epoch_start, epoch_end) =
            reward_epoch::boundaries_with(current_epoch, blocks_per_epoch);
        let blocks_remaining = epoch_end.saturating_sub(current_height);

        let last_complete_epoch =
            reward_epoch::last_complete_with(current_height, blocks_per_epoch);

        let block_reward = self.params.block_reward(current_height);

        let response = EpochInfoResponse {
            current_height,
            current_epoch,
            last_complete_epoch,
            blocks_per_epoch,
            blocks_remaining,
            epoch_start_height: epoch_start,
            epoch_end_height: epoch_end,
            block_reward,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get network parameters
    ///
    /// Returns network-specific configuration parameters like bond_unit, slot_duration, etc.
    /// CLI tools should use this to get the correct values instead of hardcoding.
    pub(super) async fn get_network_params(&self) -> Result<Value, RpcError> {
        Ok(serde_json::json!({
            "network": self.network,
            "bondUnit": self.bond_unit,
            "slotDuration": self.params.slot_duration,
            "slotsPerEpoch": self.params.slots_per_epoch,
            "blocksPerRewardEpoch": self.blocks_per_reward_epoch,
            "coinbaseMaturity": self.coinbase_maturity,
            "initialReward": self.params.initial_reward,
            "genesisTime": self.params.genesis_time
        }))
    }
}
