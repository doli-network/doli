//! RPC method handlers

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use tracing::debug;

use crypto::Hash;
use doli_core::Transaction;
use storage::{BlockStore, ChainState, ClaimRegistry, ProducerSet, UtxoSet};

use crate::error::RpcError;
use crate::types::*;
use mempool::{Mempool, MempoolError};

use doli_core::consensus::ConsensusParams;

/// Sync status information
#[derive(Debug, Clone, Default)]
pub struct SyncStatus {
    /// Whether actively syncing
    pub is_syncing: bool,
    /// Sync progress as percentage (0-100)
    pub progress: Option<f64>,
}

/// RPC handler context
pub struct RpcContext {
    /// Block store
    pub block_store: Arc<BlockStore>,
    /// UTXO set
    pub utxo_set: Arc<RwLock<UtxoSet>>,
    /// Chain state
    pub chain_state: Arc<RwLock<ChainState>>,
    /// Producer set
    pub producer_set: Option<Arc<RwLock<ProducerSet>>>,
    /// Claim registry
    pub claim_registry: Option<Arc<RwLock<ClaimRegistry>>>,
    /// Mempool
    pub mempool: Arc<RwLock<Mempool>>,
    /// Consensus params
    pub params: ConsensusParams,
    /// Network name
    pub network: String,
    /// Blocks per reward epoch (network-specific: mainnet/testnet=360, devnet=60)
    pub blocks_per_reward_epoch: u64,
    /// Local peer ID
    pub peer_id: String,
    /// Peer count function
    pub peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
    /// Broadcast transaction function
    pub broadcast_tx: Arc<dyn Fn(Transaction) + Send + Sync>,
    /// Sync status function
    pub sync_status: Arc<dyn Fn() -> SyncStatus + Send + Sync>,
    /// Broadcast vote function (for governance veto system)
    pub broadcast_vote: Arc<dyn Fn(Vec<u8>) + Send + Sync>,
}

impl RpcContext {
    /// Create a new RPC context
    pub fn new(
        chain_state: Arc<RwLock<ChainState>>,
        block_store: Arc<BlockStore>,
        utxo_set: Arc<RwLock<UtxoSet>>,
        mempool: Arc<RwLock<Mempool>>,
        params: ConsensusParams,
    ) -> Self {
        use doli_core::consensus::BLOCKS_PER_REWARD_EPOCH;
        Self {
            chain_state,
            block_store,
            utxo_set,
            producer_set: None,
            claim_registry: None,
            mempool,
            params,
            network: "mainnet".to_string(),
            blocks_per_reward_epoch: BLOCKS_PER_REWARD_EPOCH,
            peer_id: "unknown".to_string(),
            peer_count: Arc::new(|| 0),
            broadcast_tx: Arc::new(|_| {}),
            sync_status: Arc::new(|| SyncStatus::default()),
            broadcast_vote: Arc::new(|_| {}),
        }
    }

    /// Set peer count callback
    pub fn with_peer_count(mut self, f: impl Fn() -> usize + Send + Sync + 'static) -> Self {
        self.peer_count = Arc::new(f);
        self
    }

    /// Set broadcast callback
    pub fn with_broadcast(mut self, f: impl Fn(Transaction) + Send + Sync + 'static) -> Self {
        self.broadcast_tx = Arc::new(f);
        self
    }

    /// Set network name
    pub fn with_network(mut self, network: String) -> Self {
        self.network = network;
        self
    }

    /// Set blocks per reward epoch (network-specific)
    pub fn with_blocks_per_reward_epoch(mut self, blocks: u64) -> Self {
        self.blocks_per_reward_epoch = blocks;
        self
    }

    /// Set peer ID
    pub fn with_peer_id(mut self, peer_id: String) -> Self {
        self.peer_id = peer_id;
        self
    }

    /// Set sync status callback
    pub fn with_sync_status(mut self, f: impl Fn() -> SyncStatus + Send + Sync + 'static) -> Self {
        self.sync_status = Arc::new(f);
        self
    }

    /// Set producer set
    pub fn with_producer_set(mut self, producer_set: Arc<RwLock<ProducerSet>>) -> Self {
        self.producer_set = Some(producer_set);
        self
    }

    /// Set claim registry
    pub fn with_claim_registry(mut self, claim_registry: Arc<RwLock<ClaimRegistry>>) -> Self {
        self.claim_registry = Some(claim_registry);
        self
    }

    /// Set broadcast vote callback (for governance veto system)
    pub fn with_broadcast_vote(mut self, f: impl Fn(Vec<u8>) + Send + Sync + 'static) -> Self {
        self.broadcast_vote = Arc::new(f);
        self
    }
}

impl RpcContext {
    /// Handle an RPC request
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        debug!("RPC request: method={}", request.method);

        let result = match request.method.as_str() {
            "getBlockByHash" => self.get_block_by_hash(request.params).await,
            "getBlockByHeight" => self.get_block_by_height(request.params).await,
            "getTransaction" => self.get_transaction(request.params).await,
            "sendTransaction" => self.send_transaction(request.params).await,
            "getBalance" => self.get_balance(request.params).await,
            "getUtxos" => self.get_utxos(request.params).await,
            "getMempoolInfo" => self.get_mempool_info().await,
            "getNetworkInfo" => self.get_network_info().await,
            "getChainInfo" => self.get_chain_info().await,
            "getProducer" => self.get_producer(request.params).await,
            "getProducers" => self.get_producers(request.params).await,
            "getHistory" => self.get_history(request.params).await,
            "submitVote" => self.submit_vote(request.params).await,
            "getUpdateStatus" => self.get_update_status().await,
            "getNodeInfo" => self.get_node_info().await,
            // Reward/claim methods
            "getClaimableRewards" => self.get_claimable_rewards(request.params).await,
            "getClaimHistory" => self.get_claim_history(request.params).await,
            "estimateEpochReward" => self.estimate_epoch_reward(request.params).await,
            "buildClaimTx" => self.build_claim_tx(request.params).await,
            "getEpochInfo" => self.get_epoch_info().await,
            _ => Err(RpcError::method_not_found(&request.method)),
        };

        match result {
            Ok(value) => JsonRpcResponse::success(request.id, value),
            Err(error) => JsonRpcResponse::error(request.id, error),
        }
    }

    /// Get block by hash
    async fn get_block_by_hash(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetBlockByHashParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let hash = Hash::from_hex(&params.hash)
            .ok_or_else(|| RpcError::invalid_params("Invalid hash format"))?;

        let block = self
            .block_store
            .get_block(&hash)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::block_not_found())?;

        // Height not directly available from hash lookup
        // Would require reverse index - for now return 0
        let height = 0;

        let mut response = BlockResponse::from(&block);
        response.height = height;

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get block by height
    async fn get_block_by_height(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetBlockByHeightParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let hash = self
            .block_store
            .get_hash_by_height(params.height)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::block_not_found())?;

        let block = self
            .block_store
            .get_block(&hash)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| RpcError::block_not_found())?;

        let mut response = BlockResponse::from(&block);
        response.height = params.height;

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get transaction by hash
    async fn get_transaction(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetTransactionParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let hash = Hash::from_hex(&params.hash)
            .ok_or_else(|| RpcError::invalid_params("Invalid hash format"))?;

        // First check mempool
        {
            let mempool = self.mempool.read().await;
            if let Some(entry) = mempool.get(&hash) {
                let mut response = TransactionResponse::from(&entry.tx);
                response.fee = Some(entry.fee);
                return serde_json::to_value(response)
                    .map_err(|e| RpcError::internal_error(e.to_string()));
            }
        }

        // Confirmed transaction lookup requires a transaction index
        // which is not yet implemented in BlockStore
        // For now, return not found for confirmed transactions
        Err(RpcError::tx_not_found())
    }

    /// Send transaction
    async fn send_transaction(&self, params: Value) -> Result<Value, RpcError> {
        let params: SendTransactionParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let tx_bytes = hex::decode(&params.tx)
            .map_err(|e| RpcError::invalid_params(format!("Invalid hex: {}", e)))?;

        let tx = Transaction::deserialize(&tx_bytes)
            .ok_or_else(|| RpcError::invalid_params("Failed to deserialize transaction"))?;

        let tx_hash = tx.hash();

        // Add to mempool
        {
            let utxo_set = self.utxo_set.read().await;
            let chain_state = self.chain_state.read().await;
            let mut mempool = self.mempool.write().await;

            mempool
                .add_transaction(tx.clone(), &utxo_set, chain_state.best_height)
                .map_err(|e| match e {
                    MempoolError::AlreadyExists => RpcError::tx_already_known(),
                    MempoolError::Full => RpcError::mempool_full(),
                    MempoolError::InvalidTransaction(msg) => RpcError::invalid_tx(msg),
                    _ => RpcError::internal_error(e.to_string()),
                })?;
        }

        // Broadcast to network
        (self.broadcast_tx)(tx);

        Ok(Value::String(tx_hash.to_hex()))
    }

    /// Get balance for address
    async fn get_balance(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetBalanceParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = Hash::from_hex(&params.address)
            .ok_or_else(|| RpcError::invalid_params("Invalid address format"))?;

        let utxo_set = self.utxo_set.read().await;
        let chain_state = self.chain_state.read().await;
        let mempool = self.mempool.read().await;

        let confirmed = utxo_set.get_balance(&pubkey_hash, chain_state.best_height);
        let immature = utxo_set.get_immature_balance(&pubkey_hash, chain_state.best_height);

        // Calculate unconfirmed balance change from mempool
        // This gives the net change (incoming - outgoing) from pending transactions
        let unconfirmed_change = mempool.get_unconfirmed_balance(&pubkey_hash, &utxo_set);

        // For the response, we show:
        // - confirmed: current spendable balance
        // - unconfirmed: incoming amounts in mempool (always positive, represents pending credits)
        // - immature: coinbase/epoch rewards pending maturity (100 blocks)
        // - total: confirmed + unconfirmed + immature
        let (incoming, _outgoing) = mempool.calculate_unconfirmed_balance(&pubkey_hash, &utxo_set);
        let unconfirmed = incoming;

        // Total includes confirmed, unconfirmed mempool changes, and immature rewards
        let total = if unconfirmed_change >= 0 {
            confirmed
                .saturating_add(unconfirmed_change as u64)
                .saturating_add(immature)
        } else {
            confirmed
                .saturating_sub((-unconfirmed_change) as u64)
                .saturating_add(immature)
        };

        let response = BalanceResponse {
            confirmed,
            unconfirmed,
            immature,
            total,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get UTXOs for address
    async fn get_utxos(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetUtxosParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = Hash::from_hex(&params.address)
            .ok_or_else(|| RpcError::invalid_params("Invalid address format"))?;

        let utxo_set = self.utxo_set.read().await;
        let chain_state = self.chain_state.read().await;
        let current_height = chain_state.best_height;

        let utxos = utxo_set.get_by_pubkey_hash(&pubkey_hash);

        let responses: Vec<UtxoResponse> = utxos
            .into_iter()
            .filter(|(_, entry)| !params.spendable_only || entry.is_spendable_at(current_height))
            .map(|(outpoint, entry)| {
                let output_type = match entry.output.output_type {
                    doli_core::OutputType::Normal => "normal",
                    doli_core::OutputType::Bond => "bond",
                };

                UtxoResponse {
                    tx_hash: outpoint.tx_hash.to_hex(),
                    output_index: outpoint.index,
                    amount: entry.output.amount,
                    output_type: output_type.to_string(),
                    lock_until: entry.output.lock_until,
                    height: entry.height,
                    spendable: entry.is_spendable_at(current_height),
                }
            })
            .collect();

        serde_json::to_value(responses).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get mempool info
    async fn get_mempool_info(&self) -> Result<Value, RpcError> {
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
    async fn get_network_info(&self) -> Result<Value, RpcError> {
        let sync_status = (self.sync_status)();
        let response = NetworkInfoResponse {
            peer_id: self.peer_id.clone(),
            peer_count: (self.peer_count)(),
            syncing: sync_status.is_syncing,
            sync_progress: sync_status.progress,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get chain info
    async fn get_chain_info(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;

        let response = ChainInfoResponse {
            network: self.network.clone(),
            best_hash: chain_state.best_hash.to_hex(),
            best_height: chain_state.best_height,
            best_slot: chain_state.best_slot,
            genesis_hash: chain_state.genesis_hash.to_hex(),
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get producer information by public key
    async fn get_producer(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetProducerParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let pubkey = crypto::PublicKey::from_hex(&params.public_key)
            .map_err(|_| RpcError::invalid_params("Invalid public key format"))?;

        let producers = producer_set.read().await;
        let chain_state = self.chain_state.read().await;

        let info = producers
            .get_by_pubkey(&pubkey)
            .ok_or_else(|| RpcError::producer_not_found())?;

        let status = match &info.status {
            storage::ProducerStatus::Active => "active",
            storage::ProducerStatus::Unbonding { .. } => "unbonding",
            storage::ProducerStatus::Exited => "exited",
            storage::ProducerStatus::Slashed { .. } => "slashed",
        };

        // Calculate current era
        let era = chain_state.best_height / self.params.blocks_per_era;

        let response = ProducerResponse {
            public_key: params.public_key,
            registration_height: info.registered_at,
            bond_amount: info.bond_amount,
            bond_count: info.bond_count,
            status: status.to_string(),
            era,
            pending_withdrawals: Vec::new(), // TODO: Add pending withdrawals when ProducerBonds is integrated
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get all producers in the network
    async fn get_producers(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetProducersParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let producers = producer_set.read().await;
        let chain_state = self.chain_state.read().await;
        let era = chain_state.best_height / self.params.blocks_per_era;

        let producer_list: Vec<&storage::ProducerInfo> = if params.active_only {
            producers.active_producers()
        } else {
            producers.all_producers()
        };

        let responses: Vec<ProducerResponse> = producer_list
            .iter()
            .map(|info| {
                let status = match &info.status {
                    storage::ProducerStatus::Active => "active",
                    storage::ProducerStatus::Unbonding { .. } => "unbonding",
                    storage::ProducerStatus::Exited => "exited",
                    storage::ProducerStatus::Slashed { .. } => "slashed",
                };

                ProducerResponse {
                    public_key: hex::encode(info.public_key.as_bytes()),
                    registration_height: info.registered_at,
                    bond_amount: info.bond_amount,
                    bond_count: info.bond_count,
                    status: status.to_string(),
                    era,
                    pending_withdrawals: Vec::new(),
                }
            })
            .collect();

        serde_json::to_value(responses).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get transaction history for an address
    async fn get_history(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetHistoryParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = Hash::from_hex(&params.address)
            .ok_or_else(|| RpcError::invalid_params("Invalid address format"))?;

        let chain_state = self.chain_state.read().await;
        let best_height = chain_state.best_height;
        let utxo_set = self.utxo_set.read().await;

        let mut history: Vec<HistoryEntryResponse> = Vec::new();
        let limit = params.limit.min(100); // Cap at 100 entries
        let max_blocks_to_scan = 1000; // Limit scanning for performance

        // Scan backwards through blocks
        let start_height = best_height;
        let end_height = start_height.saturating_sub(max_blocks_to_scan);

        for height in (end_height..=start_height).rev() {
            if history.len() >= limit {
                break;
            }

            let block = match self.block_store.get_block_by_height(height) {
                Ok(Some(b)) => b,
                Ok(None) => continue,
                Err(_) => continue,
            };

            let block_hash = block.hash();
            let timestamp = block.header.timestamp;
            let confirmations = best_height.saturating_sub(height) + 1;

            for tx in &block.transactions {
                let mut amount_received: u64 = 0;
                let mut amount_sent: u64 = 0;
                let mut is_relevant = false;

                // Check outputs for received amounts
                for output in &tx.outputs {
                    if output.pubkey_hash == pubkey_hash {
                        amount_received += output.amount;
                        is_relevant = true;
                    }
                }

                // Check inputs for sent amounts (by looking up spent UTXOs)
                // For inputs, we need to check if the spent output belonged to this address
                for input in &tx.inputs {
                    // Look up the spent output in our UTXO set or block store
                    // Since the UTXO is already spent, we need to look at the original tx
                    if let Ok(Some(prev_block)) = self.block_store.get_block(&input.prev_tx_hash) {
                        // The input.prev_tx_hash is actually a tx hash, not block hash
                        // We need a different approach - check if any input signature matches
                        continue;
                    }

                    // Alternative: scan for the transaction that created this output
                    // This is expensive, so for now we rely on the output check
                }

                // For sent transactions, also check if any output goes back to us (change)
                // and calculate what was sent to others
                if !is_relevant {
                    // Check if this tx has inputs from our address by checking outputs
                    // This is a heuristic - if we have outputs to this address and it's a transfer,
                    // it might be relevant
                    continue;
                }

                // Calculate fee for outgoing transactions
                let fee = if amount_sent > 0 {
                    // Fee calculation would require summing input values
                    // For now, report 0 unless we can calculate it
                    0
                } else {
                    0
                };

                let tx_type = match tx.tx_type {
                    doli_core::TxType::Transfer => "transfer",
                    doli_core::TxType::Registration => "registration",
                    doli_core::TxType::Exit => "exit",
                    doli_core::TxType::ClaimReward => "claim_reward",
                    doli_core::TxType::ClaimBond => "claim_bond",
                    doli_core::TxType::SlashProducer => "slash_producer",
                    doli_core::TxType::Coinbase => "coinbase",
                    doli_core::TxType::AddBond => "add_bond",
                    doli_core::TxType::RequestWithdrawal => "request_withdrawal",
                    doli_core::TxType::ClaimWithdrawal => "claim_withdrawal",
                    doli_core::TxType::EpochReward => "epoch_reward",
                    doli_core::TxType::ClaimEpochReward => "claim_epoch_reward",
                    doli_core::TxType::ClaimEpochRewardV2 => "claim_epoch_reward_v2",
                };

                history.push(HistoryEntryResponse {
                    hash: tx.hash().to_hex(),
                    tx_type: tx_type.to_string(),
                    block_hash: block_hash.to_hex(),
                    height,
                    timestamp,
                    amount_received,
                    amount_sent,
                    fee,
                    confirmations,
                });

                if history.len() >= limit {
                    break;
                }
            }
        }

        serde_json::to_value(history).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Submit a vote for a pending update (governance veto system)
    async fn submit_vote(&self, params: Value) -> Result<Value, RpcError> {
        let params: SubmitVoteParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        // Serialize the vote message for broadcasting
        let vote_data = serde_json::to_vec(&params.vote)
            .map_err(|e| RpcError::internal_error(format!("Failed to serialize vote: {}", e)))?;

        // Broadcast via gossip
        (self.broadcast_vote)(vote_data);

        Ok(serde_json::json!({
            "status": "submitted",
            "message": "Vote submitted and broadcast to network"
        }))
    }

    /// Get the current update status (pending updates, votes, etc.)
    async fn get_update_status(&self) -> Result<Value, RpcError> {
        // For now, return a placeholder - full implementation would query the update service
        Ok(serde_json::json!({
            "pending_update": null,
            "veto_period_active": false,
            "veto_count": 0,
            "veto_percent": 0,
            "message": "Update status tracking not yet integrated with RPC"
        }))
    }

    /// Get node information including version
    async fn get_node_info(&self) -> Result<Value, RpcError> {
        Ok(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "network": self.network,
            "peerId": self.peer_id,
            "peerCount": (self.peer_count)(),
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH
        }))
    }

    // ==================== Reward/Claim Methods ====================

    /// Get claimable rewards for a producer
    async fn get_claimable_rewards(&self, params: Value) -> Result<Value, RpcError> {
        use doli_core::consensus::reward_epoch;
        use doli_core::rewards::WeightedRewardCalculator;

        let params: GetClaimableRewardsParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_pubkey = crypto::PublicKey::from_hex(&params.producer)
            .map_err(|_| RpcError::invalid_params("Invalid producer public key format"))?;

        // Get producer set to find producer index
        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let claim_registry = self
            .claim_registry
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Claim registry not available"))?;

        let producers = producer_set.read().await;
        let registry = claim_registry.read().await;
        let chain_state = self.chain_state.read().await;

        // Verify producer exists
        let _ = producers
            .get_by_pubkey(&producer_pubkey)
            .ok_or_else(|| RpcError::producer_not_found())?;

        // Find producer index in sorted list (must match block production ordering)
        let active_producers = producers.active_producers();
        let mut sorted_pubkeys: Vec<crypto::PublicKey> = active_producers
            .iter()
            .map(|p| p.public_key.clone())
            .collect();
        sorted_pubkeys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        let producer_index = sorted_pubkeys
            .iter()
            .position(|p| *p == producer_pubkey)
            .ok_or_else(|| RpcError::producer_not_found())?;

        let current_height = chain_state.best_height;
        let current_epoch =
            reward_epoch::from_height_with(current_height, self.blocks_per_reward_epoch);

        // Get unclaimed epochs
        let unclaimed_epochs = registry.get_unclaimed_epochs(&producer_pubkey, 0, current_epoch);

        // Calculate rewards for each unclaimed epoch
        let calculator = WeightedRewardCalculator::with_blocks_per_epoch(
            &*self.block_store,
            &self.params,
            self.blocks_per_reward_epoch,
        );

        let mut epochs = Vec::new();
        let mut total_estimated_reward: u64 = 0;

        for epoch in unclaimed_epochs {
            match calculator.calculate_producer_reward(&producer_pubkey, producer_index, epoch) {
                Ok(calc) if calc.reward_amount > 0 => {
                    total_estimated_reward += calc.reward_amount;
                    epochs.push(ClaimableEpochEntry {
                        epoch,
                        blocks_present: calc.blocks_present,
                        total_blocks: calc.total_blocks,
                        presence_rate: calc.presence_rate(),
                        estimated_reward: calc.reward_amount,
                    });
                }
                _ => continue,
            }
        }

        let response = ClaimableRewardsResponse {
            producer: params.producer,
            current_height,
            current_epoch,
            claimable_count: epochs.len(),
            total_estimated_reward,
            epochs,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get claim history for a producer
    async fn get_claim_history(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetClaimHistoryParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_pubkey = crypto::PublicKey::from_hex(&params.producer)
            .map_err(|_| RpcError::invalid_params("Invalid producer public key format"))?;

        let claim_registry = self
            .claim_registry
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Claim registry not available"))?;

        let registry = claim_registry.read().await;

        let mut claims: Vec<(u64, &storage::ClaimRecord)> =
            registry.get_producer_claims(&producer_pubkey);

        // Sort by epoch descending (most recent first)
        claims.sort_by(|a, b| b.0.cmp(&a.0));

        // Apply limit
        let claims: Vec<_> = claims.into_iter().take(params.limit).collect();

        let total_claimed: u64 = claims.iter().map(|(_, r)| r.amount).sum();

        let claim_entries: Vec<ClaimHistoryEntry> = claims
            .iter()
            .map(|(epoch, record)| ClaimHistoryEntry {
                epoch: *epoch,
                tx_hash: record.tx_hash.to_hex(),
                height: record.height,
                amount: record.amount,
                timestamp: record.timestamp,
            })
            .collect();

        let response = ClaimHistoryResponse {
            producer: params.producer,
            total_claims: claim_entries.len(),
            total_claimed,
            claims: claim_entries,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Estimate reward for a specific epoch
    async fn estimate_epoch_reward(&self, params: Value) -> Result<Value, RpcError> {
        use doli_core::consensus::reward_epoch;
        use doli_core::rewards::WeightedRewardCalculator;

        let params: EstimateEpochRewardParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_pubkey = crypto::PublicKey::from_hex(&params.producer)
            .map_err(|_| RpcError::invalid_params("Invalid producer public key format"))?;

        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let claim_registry = self
            .claim_registry
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Claim registry not available"))?;

        let producers = producer_set.read().await;
        let registry = claim_registry.read().await;
        let chain_state = self.chain_state.read().await;

        // Find producer index in sorted list (must match block production ordering)
        let active_producers = producers.active_producers();
        let mut sorted_pubkeys: Vec<crypto::PublicKey> = active_producers
            .iter()
            .map(|p| p.public_key.clone())
            .collect();
        sorted_pubkeys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        let producer_index = sorted_pubkeys
            .iter()
            .position(|p| *p == producer_pubkey)
            .ok_or_else(|| RpcError::producer_not_found())?;

        let current_height = chain_state.best_height;
        let is_complete = reward_epoch::is_complete_with(
            params.epoch,
            current_height,
            self.blocks_per_reward_epoch,
        );
        let is_claimed = registry.is_claimed(&producer_pubkey, params.epoch);

        // Calculate reward
        let calculator = WeightedRewardCalculator::with_blocks_per_epoch(
            &*self.block_store,
            &self.params,
            self.blocks_per_reward_epoch,
        );
        let calc = calculator
            .calculate_producer_reward(&producer_pubkey, producer_index, params.epoch)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        let response = RewardEstimateResponse {
            producer: params.producer,
            epoch: params.epoch,
            is_complete,
            is_claimed,
            blocks_present: calc.blocks_present,
            total_blocks: calc.total_blocks,
            presence_rate: calc.presence_rate(),
            block_reward: calc.block_reward,
            estimated_reward: calc.reward_amount,
            average_weight: calc.average_weight(),
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Build an unsigned claim transaction
    async fn build_claim_tx(&self, params: Value) -> Result<Value, RpcError> {
        use doli_core::consensus::reward_epoch;
        use doli_core::rewards::WeightedRewardCalculator;
        use doli_core::ClaimEpochRewardData;

        let params: BuildClaimTxParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_pubkey = crypto::PublicKey::from_hex(&params.producer)
            .map_err(|_| RpcError::invalid_params("Invalid producer public key format"))?;

        // Recipient defaults to producer's pubkey hash
        let recipient_hash = if let Some(ref recipient) = params.recipient {
            Hash::from_hex(recipient)
                .ok_or_else(|| RpcError::invalid_params("Invalid recipient address format"))?
        } else {
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, producer_pubkey.as_bytes())
        };

        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let claim_registry = self
            .claim_registry
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Claim registry not available"))?;

        let producers = producer_set.read().await;
        let registry = claim_registry.read().await;
        let chain_state = self.chain_state.read().await;

        // Verify epoch is complete
        let current_height = chain_state.best_height;
        if !reward_epoch::is_complete_with(
            params.epoch,
            current_height,
            self.blocks_per_reward_epoch,
        ) {
            return Err(RpcError::invalid_params(format!(
                "Epoch {} is not yet complete",
                params.epoch
            )));
        }

        // Verify not already claimed
        if registry.is_claimed(&producer_pubkey, params.epoch) {
            return Err(RpcError::invalid_params(format!(
                "Epoch {} already claimed",
                params.epoch
            )));
        }

        // Find producer index in sorted list (must match block production ordering)
        let active_producers = producers.active_producers();
        let mut sorted_pubkeys: Vec<crypto::PublicKey> = active_producers
            .iter()
            .map(|p| p.public_key.clone())
            .collect();
        sorted_pubkeys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        let producer_index = sorted_pubkeys
            .iter()
            .position(|p| *p == producer_pubkey)
            .ok_or_else(|| RpcError::producer_not_found())?;

        // Calculate reward
        let calculator = WeightedRewardCalculator::with_blocks_per_epoch(
            &*self.block_store,
            &self.params,
            self.blocks_per_reward_epoch,
        );
        let calc = calculator
            .calculate_producer_reward(&producer_pubkey, producer_index, params.epoch)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        if calc.reward_amount == 0 {
            return Err(RpcError::invalid_params(
                "No reward to claim for this epoch",
            ));
        }

        // Build claim data
        let claim_data = ClaimEpochRewardData::new(params.epoch, producer_pubkey, recipient_hash);

        // Create the signing message
        let signing_message = claim_data.signing_message(calc.reward_amount);

        // Build unsigned transaction (without signature)
        // The extra_data will be: claim_data (72 bytes) + signature placeholder (64 bytes of zeros)
        let mut extra_data = claim_data.to_bytes();
        extra_data.extend_from_slice(&[0u8; 64]); // Placeholder for signature

        let unsigned_tx = Transaction {
            version: 1,
            tx_type: doli_core::TxType::ClaimEpochReward,
            inputs: Vec::new(),
            outputs: vec![doli_core::Output::normal(
                calc.reward_amount,
                recipient_hash,
            )],
            extra_data,
        };

        let response = BuildClaimTxResponse {
            epoch: params.epoch,
            amount: calc.reward_amount,
            recipient: recipient_hash.to_hex(),
            unsigned_tx: hex::encode(unsigned_tx.serialize()),
            signing_message: signing_message.to_hex(),
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get current reward epoch information
    async fn get_epoch_info(&self) -> Result<Value, RpcError> {
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
}
