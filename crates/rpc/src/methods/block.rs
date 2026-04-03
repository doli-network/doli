//! Block query handlers: getBlockByHash, getBlockByHeight, getBlockRaw

use serde_json::Value;

use crypto::Hash;

use crate::error::RpcError;
use crate::types::*;

use super::context::RpcContext;

impl RpcContext {
    /// Get block by hash
    pub(super) async fn get_block_by_hash(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetBlockByHashParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let hash = Hash::from_hex(&params.hash)
            .ok_or_else(|| RpcError::invalid_params("Invalid hash format"))?;

        let block = self
            .block_store
            .get_block(&hash)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::block_not_found_by_hash(&params.hash))?;

        // Height not directly available from hash lookup
        // Would require reverse index - for now return 0
        let height = 0;

        let mut response = BlockResponse::from(&block);
        response.height = height;

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get block by height
    pub(super) async fn get_block_by_height(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetBlockByHeightParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let hash = self
            .block_store
            .get_hash_by_height(params.height)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::block_not_found_by_height(params.height))?;

        let block = self
            .block_store
            .get_block(&hash)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::block_not_found_by_height(params.height))?;

        let mut response = BlockResponse::from(&block);
        response.height = params.height;

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get raw bincode-serialized block by height (for archiver backfill via RPC)
    pub(super) async fn get_block_raw(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetBlockRawParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let block = self
            .block_store
            .get_block_by_height(params.height)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::block_not_found_by_height(params.height))?;

        let data =
            bincode::serialize(&block).map_err(|e| RpcError::internal_error(e.to_string()))?;

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
        let checksum = crypto::hash::hash(&data).to_string();

        let response = BlockRawResponse {
            block: b64,
            blake3: checksum,
            height: params.height,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}
