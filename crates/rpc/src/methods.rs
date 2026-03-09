//! RPC method handlers

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crypto::Hash;
use doli_core::Transaction;
use storage::{BlockStore, ChainState, ProducerSet, UtxoSet};

/// Shared state for live backfill progress tracking.
pub struct BackfillState {
    pub running: AtomicBool,
    pub imported: AtomicU64,
    pub total: AtomicU64,
    pub error: RwLock<Option<String>>,
}

use crate::error::RpcError;
use crate::types::*;
use mempool::{Mempool, MempoolError};

use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::network_params::NetworkParams;

/// Convert a PendingProducerUpdate to its RPC representation.
fn pending_update_to_info(update: &storage::PendingProducerUpdate) -> PendingUpdateInfo {
    match update {
        storage::PendingProducerUpdate::Register { .. } => PendingUpdateInfo {
            update_type: "register".to_string(),
            bond_count: None,
        },
        storage::PendingProducerUpdate::Exit { .. } => PendingUpdateInfo {
            update_type: "exit".to_string(),
            bond_count: None,
        },
        storage::PendingProducerUpdate::Slash { .. } => PendingUpdateInfo {
            update_type: "slash".to_string(),
            bond_count: None,
        },
        storage::PendingProducerUpdate::AddBond { outpoints, .. } => PendingUpdateInfo {
            update_type: "add_bond".to_string(),
            bond_count: Some(outpoints.len() as u32),
        },
        storage::PendingProducerUpdate::DelegateBond { bond_count, .. } => PendingUpdateInfo {
            update_type: "delegate_bond".to_string(),
            bond_count: Some(*bond_count),
        },
        storage::PendingProducerUpdate::RevokeDelegation { .. } => PendingUpdateInfo {
            update_type: "revoke_delegation".to_string(),
            bond_count: None,
        },
        storage::PendingProducerUpdate::RequestWithdrawal { bond_count, .. } => PendingUpdateInfo {
            update_type: "withdrawal".to_string(),
            bond_count: Some(*bond_count),
        },
    }
}

/// Map a network name string to its bech32m address prefix.
///
/// Returns `Some` for known networks, `None` otherwise (which makes
/// `resolve` accept any valid prefix).
fn network_name_to_prefix(name: &str) -> Option<&str> {
    match name {
        "mainnet" => Some("doli"),
        "testnet" => Some("tdoli"),
        "devnet" => Some("ddoli"),
        _ => None,
    }
}

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
    /// Mempool
    pub mempool: Arc<RwLock<Mempool>>,
    /// Consensus params
    pub params: ConsensusParams,
    /// Network name
    pub network: String,
    /// Blocks per reward epoch (network-specific: mainnet/testnet=360, devnet=60)
    pub blocks_per_reward_epoch: u64,
    /// Coinbase maturity (network-specific: mainnet/testnet=100, devnet=10)
    pub coinbase_maturity: u64,
    /// Bond unit in base units (network-specific: mainnet/testnet=10B, devnet=100M)
    pub bond_unit: u64,
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
    /// Update status callback (reads live state from UpdateService)
    pub update_status: Arc<dyn Fn() -> Value + Send + Sync>,
    /// On-chain maintainer set (shared with Node — bootstrapped from first 5 producers,
    /// updated via MaintainerAdd/Remove governance txs)
    pub maintainer_state: Option<Arc<RwLock<storage::MaintainerState>>>,
    /// Vesting quarter duration in slots (network-specific: mainnet=3,153,600, testnet=2,160)
    pub vesting_quarter_slots: u64,
    /// Shared backfill state for live hot-backfill
    pub backfill_state: Arc<BackfillState>,
}

impl RpcContext {
    /// Create a new RPC context for a specific network (recommended)
    ///
    /// Uses NetworkParams to get network-specific values for blocks_per_reward_epoch,
    /// coinbase_maturity, and bond_unit.
    pub fn new_for_network(
        chain_state: Arc<RwLock<ChainState>>,
        block_store: Arc<BlockStore>,
        utxo_set: Arc<RwLock<UtxoSet>>,
        mempool: Arc<RwLock<Mempool>>,
        params: ConsensusParams,
        network: Network,
    ) -> Self {
        let net_params = NetworkParams::load(network);
        Self {
            chain_state,
            block_store,
            utxo_set,
            producer_set: None,
            mempool,
            params,
            network: network.name().to_string(),
            blocks_per_reward_epoch: net_params.blocks_per_reward_epoch,
            coinbase_maturity: net_params.coinbase_maturity,
            bond_unit: net_params.bond_unit,
            peer_id: "unknown".to_string(),
            peer_count: Arc::new(|| 0),
            broadcast_tx: Arc::new(|_| {}),
            sync_status: Arc::new(SyncStatus::default),
            broadcast_vote: Arc::new(|_| {}),
            update_status: Arc::new(|| {
                serde_json::json!({
                    "pending_update": null,
                    "veto_period_active": false,
                    "veto_count": 0,
                    "veto_percent": 0.0
                })
            }),
            maintainer_state: None,
            vesting_quarter_slots: net_params.vesting_quarter_slots,
            backfill_state: Arc::new(BackfillState {
                running: AtomicBool::new(false),
                imported: AtomicU64::new(0),
                total: AtomicU64::new(0),
                error: RwLock::new(None),
            }),
        }
    }

    /// Create a new RPC context with mainnet defaults
    ///
    /// **Deprecated**: Use `new_for_network()` for network-aware initialization.
    #[deprecated(
        note = "Use new_for_network(chain_state, block_store, utxo_set, mempool, params, network) for network-aware initialization"
    )]
    pub fn new(
        chain_state: Arc<RwLock<ChainState>>,
        block_store: Arc<BlockStore>,
        utxo_set: Arc<RwLock<UtxoSet>>,
        mempool: Arc<RwLock<Mempool>>,
        params: ConsensusParams,
    ) -> Self {
        #[allow(deprecated)]
        {
            use doli_core::consensus::BLOCKS_PER_REWARD_EPOCH;
            use storage::utxo::DEFAULT_REWARD_MATURITY;
            Self {
                chain_state,
                block_store,
                utxo_set,
                producer_set: None,
                mempool,
                params,
                network: "mainnet".to_string(),
                blocks_per_reward_epoch: BLOCKS_PER_REWARD_EPOCH,
                coinbase_maturity: DEFAULT_REWARD_MATURITY,
                bond_unit: 10_000_000_000, // 100 DOLI default (mainnet/testnet)
                peer_id: "unknown".to_string(),
                peer_count: Arc::new(|| 0),
                broadcast_tx: Arc::new(|_| {}),
                sync_status: Arc::new(SyncStatus::default),
                broadcast_vote: Arc::new(|_| {}),
                update_status: Arc::new(|| {
                    serde_json::json!({
                        "pending_update": null,
                        "veto_period_active": false,
                        "veto_count": 0,
                        "veto_percent": 0.0
                    })
                }),
                maintainer_state: None,
                vesting_quarter_slots: doli_core::consensus::VESTING_QUARTER_SLOTS as u64,
                backfill_state: Arc::new(BackfillState {
                    running: AtomicBool::new(false),
                    imported: AtomicU64::new(0),
                    total: AtomicU64::new(0),
                    error: RwLock::new(None),
                }),
            }
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

    /// Set coinbase maturity (network-specific)
    pub fn with_coinbase_maturity(mut self, maturity: u64) -> Self {
        self.coinbase_maturity = maturity;
        self
    }

    /// Set bond unit (network-specific)
    pub fn with_bond_unit(mut self, bond_unit: u64) -> Self {
        self.bond_unit = bond_unit;
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

    /// Set broadcast vote callback (for governance veto system)
    pub fn with_broadcast_vote(mut self, f: impl Fn(Vec<u8>) + Send + Sync + 'static) -> Self {
        self.broadcast_vote = Arc::new(f);
        self
    }

    /// Set update status callback (for getUpdateStatus RPC)
    pub fn with_update_status(mut self, f: impl Fn() -> Value + Send + Sync + 'static) -> Self {
        self.update_status = Arc::new(f);
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
            "getBlockRaw" => self.get_block_raw(request.params).await,
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
            .ok_or_else(RpcError::block_not_found)?;

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
    async fn get_block_raw(&self, params: Value) -> Result<Value, RpcError> {
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

        // Look up confirmed transaction via tx index
        let height = self
            .block_store
            .get_tx_block_height(&hash)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(RpcError::tx_not_found)?;

        let block = self
            .block_store
            .get_block_by_height(height)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(RpcError::tx_not_found)?;

        let block_hash = block.hash().to_hex();
        let best_height = self.chain_state.read().await.best_height;
        let confirmations = best_height.saturating_sub(height) + 1;

        for tx in &block.transactions {
            if tx.hash() == hash {
                let mut response = TransactionResponse::from(tx);
                response.block_hash = Some(block_hash);
                response.block_height = Some(height);
                response.confirmations = Some(confirmations);
                return serde_json::to_value(response)
                    .map_err(|e| RpcError::internal_error(e.to_string()));
            }
        }

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

        // Add to mempool — state-only txs (Exit, RequestWithdrawal, etc.) bypass
        // UTXO fee accounting since they have no inputs by design. Their spam
        // protection comes from requiring a registered producer bond.
        {
            let chain_state = self.chain_state.read().await;
            let current_height = chain_state.best_height;
            drop(chain_state);

            let mut mempool = self.mempool.write().await;

            if tx.is_state_only() {
                mempool
                    .add_system_transaction(tx.clone(), current_height)
                    .map_err(|e| match e {
                        MempoolError::AlreadyExists => RpcError::tx_already_known(),
                        MempoolError::Full => RpcError::mempool_full(),
                        MempoolError::InvalidTransaction(msg) => RpcError::invalid_tx(msg),
                        _ => RpcError::internal_error(e.to_string()),
                    })?;
            } else {
                let utxo_set = self.utxo_set.read().await;
                mempool
                    .add_transaction(tx.clone(), &utxo_set, current_height)
                    .map_err(|e| match e {
                        MempoolError::AlreadyExists => RpcError::tx_already_known(),
                        MempoolError::Full => RpcError::mempool_full(),
                        MempoolError::InvalidTransaction(msg) => RpcError::invalid_tx(msg),
                        _ => RpcError::internal_error(e.to_string()),
                    })?;
            }
        }

        // Broadcast to network
        (self.broadcast_tx)(tx);

        Ok(Value::String(tx_hash.to_hex()))
    }

    /// Resolve an address string (bech32m `doli1...` or 64-char hex) to a Hash.
    fn resolve_address(&self, addr: &str) -> Result<Hash, RpcError> {
        let prefix = network_name_to_prefix(&self.network);
        crypto::address::resolve(addr, prefix).map_err(|e| RpcError::invalid_params(e.to_string()))
    }

    /// Get balance for address
    async fn get_balance(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetBalanceParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = self.resolve_address(&params.address)?;

        let utxo_set = self.utxo_set.read().await;
        let chain_state = self.chain_state.read().await;
        let mempool = self.mempool.read().await;

        // Use network-specific coinbase maturity
        let maturity = self.coinbase_maturity;
        let confirmed =
            utxo_set.get_balance_with_maturity(&pubkey_hash, chain_state.best_height, maturity);
        let immature = utxo_set.get_immature_balance_with_maturity(
            &pubkey_hash,
            chain_state.best_height,
            maturity,
        );
        let bonded = utxo_set.get_bonded_balance(&pubkey_hash);

        // Get incoming (change outputs) and outgoing (spent UTXOs) from mempool
        let (incoming, outgoing) = mempool.calculate_unconfirmed_balance(&pubkey_hash, &utxo_set);

        // Spendable = confirmed minus mempool-spent UTXOs (matches getUtxos filtering)
        let spendable = confirmed.saturating_sub(outgoing);

        // Total = all assets: spendable + pending change + immature rewards + bonded
        let total = spendable
            .saturating_add(incoming)
            .saturating_add(immature)
            .saturating_add(bonded);

        let response = BalanceResponse {
            confirmed: spendable,
            unconfirmed: incoming,
            immature,
            bonded,
            total,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get UTXOs for address
    async fn get_utxos(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetUtxosParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = self.resolve_address(&params.address)?;

        let utxo_set = self.utxo_set.read().await;
        let chain_state = self.chain_state.read().await;
        let current_height = chain_state.best_height;
        let mempool = self.mempool.read().await;

        let utxos = utxo_set.get_by_pubkey_hash(&pubkey_hash);
        let maturity = self.coinbase_maturity;

        let responses: Vec<UtxoResponse> = utxos
            .into_iter()
            .filter(|(outpoint, entry)| {
                // Exclude UTXOs being spent by mempool transactions
                if mempool.is_outpoint_spent(outpoint) {
                    return false;
                }
                !params.spendable_only
                    || entry.is_spendable_at_with_maturity(current_height, maturity)
            })
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
                    spendable: entry.is_spendable_at_with_maturity(current_height, maturity),
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
            version: env!("CARGO_PKG_VERSION").to_string(),
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
            .ok_or_else(RpcError::producer_not_found)?;

        let status = match &info.status {
            storage::ProducerStatus::Active => "active",
            storage::ProducerStatus::Unbonding { .. } => "unbonding",
            storage::ProducerStatus::Exited => "exited",
            storage::ProducerStatus::Slashed { .. } => "slashed",
        };

        // Derive bond data from UTXO set (source of truth)
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());
        let utxo_set = self.utxo_set.read().await;
        let utxo_bond_count = utxo_set.count_bonds(&pubkey_hash);
        let utxo_bonds = utxo_set.get_bond_entries(&pubkey_hash);
        let utxo_bond_amount: u64 = utxo_bonds.iter().map(|(_, _, amt)| *amt).sum();
        drop(utxo_set);

        // Use UTXO values when available, fall back to ProducerInfo (genesis)
        let effective_bond_count = if utxo_bond_count > 0 {
            utxo_bond_count
        } else {
            info.bond_count
        };
        let effective_bond_amount = if utxo_bond_amount > 0 {
            utxo_bond_amount
        } else {
            info.bond_amount
        };

        // Calculate current era
        let era = chain_state.best_height / self.params.blocks_per_era;

        // Build pending withdrawals from ProducerStatus::Unbonding
        let pending_withdrawals =
            if let storage::ProducerStatus::Unbonding { started_at } = &info.status {
                let current_height = chain_state.best_height;
                let unbonding_period = doli_core::consensus::UNBONDING_PERIOD;
                let claimable = current_height >= started_at + unbonding_period;
                vec![PendingWithdrawalResponse {
                    bond_count: effective_bond_count,
                    request_slot: *started_at as u32,
                    net_amount: effective_bond_amount,
                    claimable,
                }]
            } else {
                Vec::new()
            };

        // Collect pending epoch-deferred updates for this producer
        let pending_updates: Vec<PendingUpdateInfo> = producers
            .pending_updates_for(&pubkey)
            .into_iter()
            .map(pending_update_to_info)
            .collect();

        let response = ProducerResponse {
            public_key: params.public_key,
            registration_height: info.registered_at,
            bond_amount: effective_bond_amount,
            bond_count: effective_bond_count,
            status: status.to_string(),
            era,
            pending_withdrawals,
            pending_updates,
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

        let current_height = chain_state.best_height;
        let unbonding_period = doli_core::consensus::UNBONDING_PERIOD;

        // Build pending updates index once — O(M) instead of O(N×M)
        let pending_by_pubkey = producers.pending_updates_by_pubkey();

        let responses: Vec<ProducerResponse> = producer_list
            .iter()
            .map(|info| {
                let status = match &info.status {
                    storage::ProducerStatus::Active => "active",
                    storage::ProducerStatus::Unbonding { .. } => "unbonding",
                    storage::ProducerStatus::Exited => "exited",
                    storage::ProducerStatus::Slashed { .. } => "slashed",
                };

                let pending_withdrawals =
                    if let storage::ProducerStatus::Unbonding { started_at } = &info.status {
                        let claimable = current_height >= started_at + unbonding_period;
                        vec![PendingWithdrawalResponse {
                            bond_count: info.bond_count,
                            request_slot: *started_at as u32,
                            net_amount: info.bond_amount,
                            claimable,
                        }]
                    } else {
                        Vec::new()
                    };

                let pending_updates: Vec<PendingUpdateInfo> = pending_by_pubkey
                    .get(&info.public_key)
                    .map(|updates| updates.iter().map(|u| pending_update_to_info(u)).collect())
                    .unwrap_or_default();

                ProducerResponse {
                    public_key: hex::encode(info.public_key.as_bytes()),
                    registration_height: info.registered_at,
                    bond_amount: info.bond_amount,
                    bond_count: info.bond_count,
                    status: status.to_string(),
                    era,
                    pending_withdrawals,
                    pending_updates,
                }
            })
            .collect();

        // Append pending registrations (not yet in producer set)
        let mut responses = responses;
        for info in producers.pending_registrations() {
            responses.push(ProducerResponse {
                public_key: hex::encode(info.public_key.as_bytes()),
                registration_height: info.registered_at,
                bond_amount: info.bond_amount,
                bond_count: info.bond_count,
                status: "pending".to_string(),
                era,
                pending_withdrawals: Vec::new(),
                pending_updates: vec![PendingUpdateInfo {
                    update_type: "register".to_string(),
                    bond_count: Some(info.bond_count),
                }],
            });
        }

        serde_json::to_value(responses).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get transaction history for an address (uses addr_tx_index for O(1) lookup)
    async fn get_history(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetHistoryParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let pubkey_hash = self.resolve_address(&params.address)?;

        let chain_state = self.chain_state.read().await;
        let best_height = chain_state.best_height;
        drop(chain_state);

        let limit = params.limit.min(100);
        let before_height = params
            .before_height
            .map(|h| h.min(best_height + 1))
            .unwrap_or(best_height + 1);

        // Use address index to find relevant block heights (descending)
        let heights = self
            .block_store
            .get_address_heights(&pubkey_hash, Some(before_height), limit * 2)
            .map_err(|e| RpcError::internal_error(e.to_string()))?;

        // Build tx output cache for these blocks (for input resolution)
        let mut tx_output_cache: std::collections::HashMap<
            crypto::Hash,
            Vec<doli_core::transaction::Output>,
        > = std::collections::HashMap::new();

        for &height in &heights {
            if let Ok(Some(block)) = self.block_store.get_block_by_height(height) {
                for tx in &block.transactions {
                    tx_output_cache.insert(tx.hash(), tx.outputs.clone());
                    // Also cache parent tx outputs for input resolution
                    for input in &tx.inputs {
                        if !tx_output_cache.contains_key(&input.prev_tx_hash) {
                            if let Ok(Some(parent_height)) =
                                self.block_store.get_tx_block_height(&input.prev_tx_hash)
                            {
                                if let Ok(Some(parent_block)) =
                                    self.block_store.get_block_by_height(parent_height)
                                {
                                    for ptx in &parent_block.transactions {
                                        tx_output_cache.insert(ptx.hash(), ptx.outputs.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Build history entries from indexed blocks
        let mut history: Vec<HistoryEntryResponse> = Vec::new();

        for &height in &heights {
            if history.len() >= limit {
                break;
            }

            let block = match self.block_store.get_block_by_height(height) {
                Ok(Some(b)) => b,
                _ => continue,
            };

            let block_hash = block.hash();
            let timestamp = block.header.timestamp;
            let confirmations = best_height.saturating_sub(height) + 1;

            for tx in &block.transactions {
                let mut amount_received: u64 = 0;
                let mut amount_sent: u64 = 0;
                let mut total_input: u64 = 0;
                let mut is_relevant = false;

                for output in &tx.outputs {
                    if output.pubkey_hash == pubkey_hash {
                        amount_received += output.amount;
                        is_relevant = true;
                    }
                }

                for input in &tx.inputs {
                    if let Some(prev_outputs) = tx_output_cache.get(&input.prev_tx_hash) {
                        if let Some(prev_output) = prev_outputs.get(input.output_index as usize) {
                            total_input += prev_output.amount;
                            if prev_output.pubkey_hash == pubkey_hash {
                                amount_sent += prev_output.amount;
                                is_relevant = true;
                            }
                        }
                    }
                }

                if !is_relevant {
                    continue;
                }

                let total_output: u64 = tx.outputs.iter().map(|o| o.amount).sum();
                let fee = if total_input > 0 && total_input >= total_output {
                    total_input - total_output
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
                    doli_core::TxType::RemoveMaintainer => "remove_maintainer",
                    doli_core::TxType::AddMaintainer => "add_maintainer",
                    doli_core::TxType::DelegateBond => "delegate_bond",
                    doli_core::TxType::RevokeDelegation => "revoke_delegation",
                    doli_core::TxType::ProtocolActivation => "protocol_activation",
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
    ///
    /// Calls the update status callback to read live state from UpdateService.
    async fn get_update_status(&self) -> Result<Value, RpcError> {
        Ok((self.update_status)())
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

    /// Get network parameters
    ///
    /// Returns network-specific configuration parameters like bond_unit, slot_duration, etc.
    /// CLI tools should use this to get the correct values instead of hardcoding.
    async fn get_network_params(&self) -> Result<Value, RpcError> {
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

    /// Get current maintainer set
    ///
    /// Returns the maintainer set derived from the blockchain.
    /// First 5 registered producers become maintainers automatically.
    async fn get_maintainer_set(&self) -> Result<Value, RpcError> {
        use doli_core::maintainer::{INITIAL_MAINTAINER_COUNT, MAX_MAINTAINERS, MIN_MAINTAINERS};

        // Read from on-chain MaintainerState if available
        if let Some(ms) = &self.maintainer_state {
            let state = ms.read().await;
            let maintainers: Vec<_> = state
                .set
                .members
                .iter()
                .map(|pk| {
                    serde_json::json!({
                        "pubkey": pk.to_hex(),
                    })
                })
                .collect();

            return Ok(serde_json::json!({
                "maintainers": maintainers,
                "threshold": state.set.threshold,
                "member_count": state.set.members.len(),
                "max_maintainers": MAX_MAINTAINERS,
                "min_maintainers": MIN_MAINTAINERS,
                "initial_maintainer_count": INITIAL_MAINTAINER_COUNT,
                "last_change_block": state.set.last_updated,
                "source": "on-chain"
            }));
        }

        // Fallback: derive ad-hoc from producer set (pre-bootstrap or no MaintainerState)
        let producer_set = match &self.producer_set {
            Some(ps) => ps,
            None => {
                return Ok(serde_json::json!({
                    "maintainers": [],
                    "threshold": 0,
                    "member_count": 0,
                    "source": "none"
                }));
            }
        };

        let producers = producer_set.read().await;
        let mut sorted_producers = producers.all_producers();
        sorted_producers.sort_by_key(|p| p.registered_at);

        let maintainers: Vec<_> = sorted_producers
            .into_iter()
            .take(INITIAL_MAINTAINER_COUNT)
            .map(|p| {
                serde_json::json!({
                    "pubkey": p.public_key.to_hex(),
                    "registered_at_block": p.registered_at,
                    "is_active_producer": p.is_active()
                })
            })
            .collect();

        let member_count = maintainers.len();
        let threshold = doli_core::maintainer::MaintainerSet::calculate_threshold(member_count);

        Ok(serde_json::json!({
            "maintainers": maintainers,
            "threshold": threshold,
            "member_count": member_count,
            "max_maintainers": MAX_MAINTAINERS,
            "min_maintainers": MIN_MAINTAINERS,
            "initial_maintainer_count": INITIAL_MAINTAINER_COUNT,
            "last_change_block": 0,
            "source": "derived"
        }))
    }

    /// Submit a maintainer change (add or remove)
    ///
    /// Requires 3/5 signatures from current maintainers.
    async fn submit_maintainer_change(&self, params: Value) -> Result<Value, RpcError> {
        #[derive(serde::Deserialize)]
        struct SubmitMaintainerChangeParams {
            action: String,        // "add" or "remove"
            target_pubkey: String, // Hex-encoded public key
            signatures: Vec<SignatureEntry>,
            reason: Option<String>,
        }

        #[derive(serde::Deserialize)]
        struct SignatureEntry {
            pubkey: String,
            signature: String,
        }

        let params: SubmitMaintainerChangeParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        // Validate action
        if params.action != "add" && params.action != "remove" {
            return Err(RpcError::invalid_params("action must be 'add' or 'remove'"));
        }

        // Parse target public key
        let target = crypto::PublicKey::from_hex(&params.target_pubkey)
            .map_err(|e| RpcError::invalid_params(format!("invalid target pubkey: {}", e)))?;

        // Parse and validate signatures
        let mut signatures = Vec::new();
        for entry in params.signatures {
            let pubkey = crypto::PublicKey::from_hex(&entry.pubkey)
                .map_err(|e| RpcError::invalid_params(format!("invalid signer pubkey: {}", e)))?;
            let signature = crypto::Signature::from_hex(&entry.signature)
                .map_err(|e| RpcError::invalid_params(format!("invalid signature: {}", e)))?;
            signatures.push(doli_core::maintainer::MaintainerSignature { pubkey, signature });
        }

        // Check signature count (need at least 3)
        if signatures.len() < doli_core::maintainer::MAINTAINER_THRESHOLD {
            return Err(RpcError::invalid_params(format!(
                "insufficient signatures: need {}, got {}",
                doli_core::maintainer::MAINTAINER_THRESHOLD,
                signatures.len()
            )));
        }

        // Create the transaction
        let tx = if params.action == "add" {
            doli_core::Transaction::new_add_maintainer(target, signatures)
        } else {
            doli_core::Transaction::new_remove_maintainer(target, signatures, params.reason)
        };

        let tx_hash = tx.hash();

        // Get current height for mempool validation
        let current_height = {
            let chain_state = self.chain_state.read().await;
            chain_state.best_height
        };

        // Submit to mempool (maintainer txs are state-only, no UTXO inputs)
        let mut mempool = self.mempool.write().await;
        mempool
            .add_system_transaction(tx, current_height)
            .map_err(|e| RpcError::internal_error(format!("mempool error: {}", e)))?;

        Ok(serde_json::json!({
            "status": "accepted",
            "tx_hash": tx_hash.to_hex(),
            "message": format!("Maintainer {} transaction submitted", params.action)
        }))
    }

    /// Get bond vesting details for a producer (per-bond granularity)
    async fn get_bond_details(&self, params: Value) -> Result<Value, RpcError> {
        use doli_core::consensus::withdrawal_penalty_rate_with_quarter;

        let quarter = self.vesting_quarter_slots;
        let period = 4 * quarter;

        let params: GetBondDetailsParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let pubkey = crypto::PublicKey::from_hex(&params.public_key)
            .map_err(|_| RpcError::invalid_params("Invalid public key format"))?;

        let producers = producer_set.read().await;
        let chain_state = self.chain_state.read().await;
        let current_slot = chain_state.best_slot;

        let info = producers
            .get_by_pubkey(&pubkey)
            .ok_or_else(RpcError::producer_not_found)?;

        // Derive bond data from UTXO set (source of truth)
        let pubkey_hash = crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pubkey.as_bytes());
        let utxo_set = self.utxo_set.read().await;
        let utxo_bonds = utxo_set.get_bond_entries(&pubkey_hash);
        let bond_count = utxo_bonds.len() as u32;
        let total_staked: u64 = utxo_bonds.iter().map(|(_, _, amt)| *amt).sum();
        drop(utxo_set);

        // Build per-bond response from UTXO entries (already FIFO-sorted)
        let bonds: Vec<BondEntryResponse> = utxo_bonds
            .iter()
            .map(|(_, creation_slot, amount)| {
                let age = (current_slot as u64).saturating_sub(*creation_slot as u64);
                let penalty = withdrawal_penalty_rate_with_quarter(age as u32, quarter as u32);
                BondEntryResponse {
                    creation_slot: *creation_slot,
                    amount: *amount,
                    age_slots: age,
                    penalty_pct: penalty,
                    vested: age >= period,
                    maturation_slot: *creation_slot as u64 + period,
                }
            })
            .collect();

        // Compute summary from UTXO bonds
        let mut summary = BondsSummaryResponse {
            q1: 0,
            q2: 0,
            q3: 0,
            vested: 0,
        };
        for (_, creation_slot, _) in &utxo_bonds {
            let age = (current_slot as u64).saturating_sub(*creation_slot as u64);
            let quarters_elapsed = age / quarter;
            match quarters_elapsed {
                0 => summary.q1 += 1,
                1 => summary.q2 += 1,
                2 => summary.q3 += 1,
                _ => summary.vested += 1,
            }
        }

        // Overall vesting based on oldest bond
        let oldest_age = utxo_bonds
            .first()
            .map(|(_, cs, _)| (current_slot as u64).saturating_sub(*cs as u64))
            .unwrap_or(0);
        let overall_penalty =
            withdrawal_penalty_rate_with_quarter(oldest_age as u32, quarter as u32);
        let all_vested = utxo_bonds
            .iter()
            .all(|(_, cs, _)| (current_slot as u64).saturating_sub(*cs as u64) >= period);

        let response = BondDetailsResponse {
            public_key: params.public_key,
            bond_count,
            total_staked,
            registration_slot: info.registered_at,
            age_slots: oldest_age,
            penalty_pct: overall_penalty,
            vested: all_vested,
            maturation_slot: utxo_bonds
                .last()
                .map(|(_, cs, _)| *cs as u64 + period)
                .unwrap_or(0),
            vesting_quarter_slots: quarter,
            vesting_period_slots: period,
            summary,
            bonds,
            withdrawal_pending_count: info.withdrawal_pending_count,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Start a live backfill from a peer's RPC endpoint.
    async fn backfill_from_peer(&self, params: Value) -> Result<Value, RpcError> {
        let params: BackfillParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        // Check if already running
        if self.backfill_state.running.load(Ordering::SeqCst) {
            return Err(RpcError::internal_error(
                "Backfill already in progress. Use backfillStatus to check progress.",
            ));
        }

        // Find the gap: look for the lowest existing block
        let tip_height = self.chain_state.read().await.best_height;
        if tip_height == 0 {
            return Err(RpcError::internal_error("Node has no blocks yet"));
        }

        // Check if block 1 exists — if so, no gap
        let has_block_1 = self
            .block_store
            .get_block_by_height(1)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .is_some();
        if has_block_1 {
            return Ok(serde_json::json!({
                "started": false,
                "message": "No gap detected — block 1 exists. Chain is complete."
            }));
        }

        // Find lowest existing block (binary search)
        let mut lo = 1u64;
        let mut hi = tip_height;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let exists = self
                .block_store
                .get_block_by_height(mid)
                .map_err(|e| RpcError::internal_error(e.to_string()))?
                .is_some();
            if exists {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let lowest_existing = lo;
        let gap_end = lowest_existing - 1; // blocks 1..=gap_end are missing

        // Reset state
        self.backfill_state.imported.store(0, Ordering::SeqCst);
        self.backfill_state.total.store(gap_end, Ordering::SeqCst);
        *self.backfill_state.error.write().await = None;
        self.backfill_state.running.store(true, Ordering::SeqCst);

        // Fetch the lowest existing block to verify chain linking at the end
        let anchor_block = self
            .block_store
            .get_block_by_height(lowest_existing)
            .map_err(|e| RpcError::internal_error(e.to_string()))?
            .ok_or_else(|| {
                RpcError::internal_error(format!("Anchor block {} not found", lowest_existing))
            })?;
        let anchor_prev_hash = anchor_block.header.prev_hash;
        let genesis_hash = self.chain_state.read().await.genesis_hash;

        // Spawn background task
        let block_store = self.block_store.clone();
        let state = self.backfill_state.clone();
        let rpc_url = params.rpc_url.clone();

        tokio::spawn(async move {
            info!(
                "Backfill started: fetching blocks 1..={} from {}",
                gap_end, rpc_url
            );

            let client = reqwest::Client::new();
            let mut imported = 0u64;
            let mut prev_hash = genesis_hash; // block 1's parent is the genesis hash

            for h in 1..=gap_end {
                // Check if block already exists (idempotent)
                if let Ok(Some(existing)) = block_store.get_block_by_height(h) {
                    prev_hash = existing.hash();
                    imported += 1;
                    state.imported.store(imported, Ordering::SeqCst);
                    continue;
                }

                // Fetch block from peer
                let resp = match client
                    .post(&rpc_url)
                    .json(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "getBlockRaw",
                        "params": { "height": h },
                        "id": h
                    }))
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let msg = format!("HTTP error at height {}: {}", h, e);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        let msg = format!("JSON parse error at height {}: {}", h, e);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                let block_result = match body.get("result") {
                    Some(r) => r,
                    None => {
                        let msg = format!("No result for block {} (peer may not have it)", h);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                let b64_data = match block_result.get("block").and_then(|v| v.as_str()) {
                    Some(s) => s,
                    None => {
                        let msg = format!("Missing block data at height {}", h);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                let expected_checksum = block_result
                    .get("blake3")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Decode and verify
                use base64::Engine;
                let data = match base64::engine::general_purpose::STANDARD.decode(b64_data) {
                    Ok(d) => d,
                    Err(e) => {
                        let msg = format!("Base64 decode error at height {}: {}", h, e);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                if !expected_checksum.is_empty() {
                    let actual = crypto::hash::hash(&data).to_string();
                    if actual != expected_checksum {
                        let msg = format!("BLAKE3 mismatch at height {}", h);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                }

                let block: doli_core::Block = match bincode::deserialize(&data) {
                    Ok(b) => b,
                    Err(e) => {
                        let msg = format!("Deserialize error at height {}: {}", h, e);
                        warn!("Backfill failed: {}", msg);
                        *state.error.write().await = Some(msg);
                        state.running.store(false, Ordering::SeqCst);
                        return;
                    }
                };

                // Chain linking: verify parent_hash matches previous block
                if block.header.prev_hash != prev_hash {
                    let msg = format!(
                        "Chain link broken at height {}: expected parent {}, got {}",
                        h,
                        hex::encode(prev_hash.as_bytes()),
                        hex::encode(block.header.prev_hash.as_bytes()),
                    );
                    warn!("Backfill failed: {}", msg);
                    *state.error.write().await = Some(msg);
                    state.running.store(false, Ordering::SeqCst);
                    return;
                }
                prev_hash = block.hash();

                if let Err(e) = block_store.put_block_canonical(&block, h) {
                    let msg = format!("Store error at height {}: {}", h, e);
                    warn!("Backfill failed: {}", msg);
                    *state.error.write().await = Some(msg);
                    state.running.store(false, Ordering::SeqCst);
                    return;
                }

                imported += 1;
                state.imported.store(imported, Ordering::SeqCst);

                if imported.is_multiple_of(500) {
                    info!("Backfill progress: {}/{} blocks", imported, gap_end);
                }

                // Yield every 100 blocks to avoid starving other tasks
                if imported.is_multiple_of(100) {
                    tokio::task::yield_now().await;
                }
            }

            // Verify the last backfilled block connects to the anchor
            if prev_hash != anchor_prev_hash {
                let msg = format!(
                    "Anchor mismatch: last backfilled hash {} != anchor block {} parent {}",
                    hex::encode(prev_hash.as_bytes()),
                    lowest_existing,
                    hex::encode(anchor_prev_hash.as_bytes()),
                );
                warn!("Backfill failed: {}", msg);
                *state.error.write().await = Some(msg);
                state.running.store(false, Ordering::SeqCst);
                return;
            }

            info!(
                "Backfill complete: {} blocks imported (1..={}), chain verified",
                imported, gap_end
            );
            state.running.store(false, Ordering::SeqCst);
        });

        Ok(serde_json::json!({
            "started": true,
            "gaps": format!("1-{}", gap_end),
            "total": gap_end
        }))
    }

    /// Get backfill progress status.
    async fn backfill_status(&self) -> Result<Value, RpcError> {
        let running = self.backfill_state.running.load(Ordering::SeqCst);
        let imported = self.backfill_state.imported.load(Ordering::SeqCst);
        let total = self.backfill_state.total.load(Ordering::SeqCst);
        let pct = if total > 0 { imported * 100 / total } else { 0 };
        let error = self.backfill_state.error.read().await.clone();

        let response = BackfillStatusResponse {
            running,
            imported,
            total,
            pct,
            error,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Verify chain integrity by scanning every height from 1 to tip.
    /// Returns missing heights (gaps) and chain-linking errors.
    async fn verify_chain_integrity(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;
        let tip_height = chain_state.best_height;
        drop(chain_state);

        if tip_height == 0 {
            return Ok(serde_json::json!({
                "complete": true,
                "tip": 0,
                "scanned": 0,
                "missing": [],
                "missing_count": 0
            }));
        }

        let block_store = self.block_store.clone();
        let tip = tip_height;

        // Run scan in blocking task to avoid starving the async runtime
        let result = tokio::task::spawn_blocking(move || {
            let mut missing: Vec<String> = Vec::new();
            let mut range_start: Option<u64> = None;
            let mut range_end: u64 = 0;

            for h in 1..=tip {
                let exists = block_store
                    .get_hash_by_height(h)
                    .map(|opt| opt.is_some())
                    .unwrap_or(false);

                if !exists {
                    if range_start.is_none() {
                        range_start = Some(h);
                    }
                    range_end = h;
                } else if let Some(start) = range_start.take() {
                    // Flush completed gap range
                    if start == range_end {
                        missing.push(format!("{}", start));
                    } else {
                        missing.push(format!("{}-{}", start, range_end));
                    }
                }
            }
            // Flush final range if chain ends with a gap
            if let Some(start) = range_start {
                if start == range_end {
                    missing.push(format!("{}", start));
                } else {
                    missing.push(format!("{}-{}", start, range_end));
                }
            }

            // Count total missing
            let missing_count: u64 = missing
                .iter()
                .map(|s| {
                    if let Some((a, b)) = s.split_once('-') {
                        b.parse::<u64>().unwrap_or(0) - a.parse::<u64>().unwrap_or(0) + 1
                    } else {
                        1
                    }
                })
                .sum();

            (missing, missing_count)
        })
        .await
        .map_err(|e| RpcError::internal_error(format!("Scan failed: {}", e)))?;

        let (missing, missing_count) = result;

        Ok(serde_json::json!({
            "complete": missing_count == 0,
            "tip": tip_height,
            "scanned": tip_height,
            "missing": missing,
            "missing_count": missing_count
        }))
    }

    /// Get chain statistics (supply, address count, UTXO count, staking info)
    async fn get_chain_stats(&self) -> Result<Value, RpcError> {
        let chain_state = self.chain_state.read().await;
        let height = chain_state.best_height;
        drop(chain_state);

        let utxo_set = self.utxo_set.read().await;
        let total_supply = utxo_set.total_supply();
        let address_count = utxo_set.address_count();
        let utxo_count = utxo_set.utxo_count();
        drop(utxo_set);

        let (active_producers, total_staked) = if let Some(ref ps) = self.producer_set {
            let producers = ps.read().await;
            let active_list = producers.active_producers();
            let staked: u64 = active_list.iter().map(|p| p.bond_amount).sum();
            (active_list.len(), staked)
        } else {
            (0, 0)
        };

        let response = ChainStatsResponse {
            total_supply,
            address_count,
            utxo_count,
            active_producers,
            total_staked,
            height,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get pending mempool transactions
    async fn get_mempool_transactions(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetMempoolTxsParams =
            serde_json::from_value(params).unwrap_or(GetMempoolTxsParams { limit: 100 });
        let limit = params.limit.min(500);

        let mempool = self.mempool.read().await;
        let mut txs: Vec<MempoolTxResponse> = mempool
            .iter()
            .take(limit)
            .map(|(_hash, entry)| MempoolTxResponse {
                hash: entry.tx_hash.to_hex(),
                tx_type: format!("{:?}", entry.tx.tx_type).to_lowercase(),
                size: entry.size,
                fee: entry.fee,
                fee_rate: entry.fee_rate,
                added_time: entry.added_time,
            })
            .collect();

        // Sort by fee rate descending (highest-fee first)
        txs.sort_by(|a, b| b.fee_rate.cmp(&a.fee_rate));

        serde_json::to_value(txs).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Build the active producer set with UTXO-derived bond weights.
    ///
    /// Returns `(producers_with_bonds, total_bonds)` where each entry is
    /// `(PublicKey, bond_count)` with a minimum of 1 bond per producer.
    async fn build_producers_with_bonds(
        &self,
    ) -> Result<(Vec<(crypto::PublicKey, u64)>, u64), RpcError> {
        let producer_set = self
            .producer_set
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("Producer set not available"))?;

        let producers = producer_set.read().await;
        let active = producers.active_producers();
        let utxo_set = self.utxo_set.read().await;

        let mut result = Vec::with_capacity(active.len());
        let mut total_bonds: u64 = 0;

        for info in &active {
            let pubkey_hash =
                crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, info.public_key.as_bytes());
            let bonds = utxo_set.count_bonds(&pubkey_hash) as u64;
            let effective = bonds.max(1);
            result.push((info.public_key, effective));
            total_bonds += effective;
        }

        Ok((result, total_bonds))
    }

    /// Get slot schedule for upcoming slots
    async fn get_slot_schedule(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetSlotScheduleParams =
            serde_json::from_value(params).unwrap_or(GetSlotScheduleParams {
                from_slot: None,
                count: None,
            });

        let chain_state = self.chain_state.read().await;
        let current_slot = chain_state.best_slot;
        drop(chain_state);

        let from_slot = params.from_slot.unwrap_or(current_slot);
        let count = params.count.unwrap_or(20).min(360);

        let (producers_with_bonds, total_bonds) = self.build_producers_with_bonds().await?;

        if producers_with_bonds.is_empty() {
            return Err(RpcError::internal_error("No active producers"));
        }

        let mut slots = Vec::with_capacity(count as usize);
        for i in 0..count {
            let slot = from_slot.saturating_add(i);
            let ranked =
                doli_core::consensus::select_producer_for_slot(slot, &producers_with_bonds);
            if let Some(pk) = ranked.first() {
                slots.push(SlotScheduleEntry {
                    slot,
                    producer: hex::encode(pk.as_bytes()),
                    rank: 0,
                });
            }
        }

        let epoch = self.params.slot_to_epoch(current_slot) as u64;
        let epoch_start = epoch * self.params.slots_per_epoch as u64;
        let epoch_end = epoch_start + self.params.slots_per_epoch as u64;
        let slots_remaining = epoch_end.saturating_sub(current_slot as u64);

        let response = SlotScheduleResponse {
            slots,
            current_slot,
            epoch,
            slots_remaining_in_epoch: slots_remaining,
            total_bonds,
            slot_duration: self.params.slot_duration,
            genesis_time: self.params.genesis_time,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Get schedule and performance info for a specific producer
    async fn get_producer_schedule(&self, params: Value) -> Result<Value, RpcError> {
        let params: GetProducerScheduleParams =
            serde_json::from_value(params).map_err(|e| RpcError::invalid_params(e.to_string()))?;

        let target_pubkey = crypto::PublicKey::from_hex(&params.public_key)
            .map_err(|_| RpcError::invalid_params("Invalid public key format"))?;

        let chain_state = self.chain_state.read().await;
        let current_slot = chain_state.best_slot;
        let current_height = chain_state.best_height;
        drop(chain_state);

        let (producers_with_bonds, total_bonds) = self.build_producers_with_bonds().await?;

        if producers_with_bonds.is_empty() {
            return Err(RpcError::internal_error("No active producers"));
        }

        // Get this producer's bond count
        let pubkey_hash =
            crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, target_pubkey.as_bytes());
        let utxo_set = self.utxo_set.read().await;
        let bond_count = utxo_set.count_bonds(&pubkey_hash) as u64;
        drop(utxo_set);
        let effective_bonds = bond_count.max(1);

        // Epoch boundaries
        let epoch = self.params.slot_to_epoch(current_slot) as u64;
        let spe = self.params.slots_per_epoch as u64;
        let epoch_start_slot = epoch * spe;
        let epoch_end_slot = epoch_start_slot + spe;

        // Find all slots in this epoch where target is primary (rank 0)
        let mut slots_this_epoch = Vec::new();
        for slot in epoch_start_slot..epoch_end_slot {
            let slot32 = slot as u32;
            let ranked =
                doli_core::consensus::select_producer_for_slot(slot32, &producers_with_bonds);
            if ranked.first() == Some(&target_pubkey) {
                slots_this_epoch.push(slot32);
            }
        }

        // Find next slot >= current_slot
        let next_slot = slots_this_epoch
            .iter()
            .find(|&&s| s > current_slot)
            .copied();
        let seconds_until_next = next_slot.map(|ns| {
            let slot_diff = ns.saturating_sub(current_slot) as u64;
            slot_diff * self.params.slot_duration
        });

        // Count produced blocks this epoch by scanning block store
        let mut produced_count: u32 = 0;
        for &slot in &slots_this_epoch {
            if slot <= current_slot {
                if let Ok(Some(block)) = self.block_store.get_block_by_slot(slot) {
                    if block.header.producer == target_pubkey {
                        produced_count += 1;
                    }
                }
            }
        }

        let assigned_count = slots_this_epoch.len() as u32;
        let past_assigned = slots_this_epoch
            .iter()
            .filter(|&&s| s <= current_slot)
            .count() as u32;
        let fill_rate = if past_assigned > 0 {
            produced_count as f64 / past_assigned as f64
        } else {
            0.0
        };

        // Economics
        let block_reward = self.params.block_reward(current_height);
        let slots_per_week: u64 = 60480; // 7 * 24 * 360
        let weekly_earnings = if total_bonds > 0 {
            slots_per_week * block_reward * effective_bonds / total_bonds
        } else {
            0
        };
        let doubling_weeks = if weekly_earnings > 0 {
            (self.bond_unit as f64 * effective_bonds as f64) / weekly_earnings as f64
        } else {
            f64::INFINITY
        };

        let response = ProducerScheduleResponse {
            public_key: params.public_key,
            current_slot,
            epoch,
            next_slot,
            seconds_until_next,
            slots_this_epoch,
            assigned_count,
            produced_count,
            fill_rate,
            bond_count: effective_bonds as u32,
            total_network_bonds: total_bonds,
            weekly_earnings,
            doubling_weeks,
            block_reward,
        };

        serde_json::to_value(response).map_err(|e| RpcError::internal_error(e.to_string()))
    }
}
