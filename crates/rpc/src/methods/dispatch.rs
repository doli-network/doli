//! RPC request routing

use tracing::debug;

use crate::error::RpcError;
use crate::types::{JsonRpcRequest, JsonRpcResponse};

use super::context::RpcContext;

impl RpcContext {
    /// Handle an RPC request
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        debug!("RPC request: method={}", request.method);

        let result = match request.method.as_str() {
            "getBlockByHash" => self.get_block_by_hash(request.params).await,
            "getBlockByHeight" => self.get_block_by_height(request.params).await,
            "getBlockRaw" => self.get_block_raw(request.params).await,
            "getBlockData" => self.get_block_data(request.params).await,
            "getTransaction" => self.get_transaction(request.params).await,
            "sendTransaction" => self.send_transaction(request.params).await,
            "getBalance" => self.get_balance(request.params).await,
            "getUtxos" => self.get_utxos(request.params).await,
            "getMempoolInfo" => self.get_mempool_info().await,
            "getNetworkInfo" => self.get_network_info().await,
            "getPeerInfo" => self.get_peer_info().await,
            "getChainInfo" => self.get_chain_info().await,
            "getProducer" => self.get_producer(request.params).await,
            "getProducers" => self.get_producers(request.params).await,
            "getHistory" => self.get_history(request.params).await,
            "submitVote" => self.submit_vote(request.params).await,
            "getUpdateStatus" => self.get_update_status().await,
            "getMaintainerSet" => self.get_maintainer_set().await,
            "submitMaintainerChange" => self.submit_maintainer_change(request.params).await,
            "getNodeInfo" => self.get_node_info().await,
            "getEpochInfo" => self.get_epoch_info().await,
            "getNetworkParams" => self.get_network_params().await,
            "getBondDetails" => self.get_bond_details(request.params).await,
            "backfillFromPeer" => self.backfill_from_peer(request.params).await,
            "backfillStatus" => self.backfill_status().await,
            "verifyChainIntegrity" => self.verify_chain_integrity().await,
            "getChainStats" => self.get_chain_stats().await,
            "getMempoolTransactions" => self.get_mempool_transactions(request.params).await,
            "getSlotSchedule" => self.get_slot_schedule(request.params).await,
            "getProducerSchedule" => self.get_producer_schedule(request.params).await,
            "getAttestationStats" => self.get_attestation_stats().await,
            "getStateRootDebug" => self.get_state_root_debug().await,
            "getUtxoDiff" => self.get_utxo_diff(request.params).await,
            "getStateSnapshot" => self.get_state_snapshot().await,
            "getPoolInfo" => self.get_pool_info(request.params).await,
            "getPoolList" => self.get_pool_list(request.params).await,
            "getPoolPrice" => self.get_pool_price(request.params).await,
            "getSwapQuote" => self.get_swap_quote(request.params).await,
            "getLoanInfo" => self.get_loan_info(request.params).await,
            "getLoanList" => self.get_loan_list(request.params).await,
            // Storage management
            "pruneBlocks" => self.prune_blocks(Some(request.params)).await,
            "getStorageInfo" => self.get_storage_info().await,
            // Seed Guardian methods
            "pauseProduction" => self.pause_production().await,
            "resumeProduction" => self.resume_production().await,
            "createCheckpoint" => self.create_checkpoint(Some(request.params)).await,
            "getGuardianStatus" => self.get_guardian_status().await,
            _ => Err(RpcError::method_not_found(&request.method)),
        };

        match result {
            Ok(value) => JsonRpcResponse::success(request.id, value),
            Err(error) => JsonRpcResponse::error(request.id, error),
        }
    }
}
