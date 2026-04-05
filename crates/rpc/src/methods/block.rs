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
            .ok_or_else(RpcError::block_not_found)?;

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
            .ok_or_else(RpcError::block_not_found)?;

        let block = self
            .block_store
            .get_block(&hash)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(RpcError::block_not_found)?;

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
            .ok_or_else(RpcError::block_not_found)?;

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

    /// Get blob data from a specific output in a block.
    /// Used by light nodes and clients to retrieve NFT/document content from archivers.
    /// Returns the raw extra_data bytes (base64-encoded) for the specified output.
    pub(super) async fn get_block_data(&self, params: Value) -> Result<Value, RpcError> {
        #[derive(serde::Deserialize)]
        struct Params {
            hash: String,
            output_index: u32,
        }
        let params: Params =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let hash = crypto::Hash::from_hex(&params.hash)
            .ok_or_else(|| RpcError::invalid_params("Invalid hash format"))?;

        let block = self
            .block_store
            .get_block(&hash)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(RpcError::block_not_found)?;

        // Find the transaction and output
        let mut global_output_idx = 0u32;
        for tx in &block.transactions {
            for output in &tx.outputs {
                if global_output_idx == params.output_index {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&output.extra_data);
                    let blob_hash = crypto::hash::hash(&output.extra_data).to_string();
                    return serde_json::to_value(serde_json::json!({
                        "data": b64,
                        "size": output.extra_data.len(),
                        "blob_hash": blob_hash,
                        "output_type": format!("{:?}", output.output_type),
                    }))
                    .map_err(|e| RpcError::internal_error(e.to_string()));
                }
                global_output_idx += 1;
            }
        }

        Err(RpcError::invalid_params(format!(
            "Output index {} not found in block",
            params.output_index
        )))
    }
}
