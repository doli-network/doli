//! RpcContext struct, constructors, and builder methods

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crypto::Hash;
use doli_core::Transaction;
use storage::{BlockStore, ChainState, ProducerSet, UtxoSet};

use crate::error::RpcError;
use crate::types::PeerInfoEntry;
use mempool::Mempool;

use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::network_params::NetworkParams;

/// Shared state for live backfill progress tracking.
pub struct BackfillState {
    pub running: AtomicBool,
    pub imported: AtomicU64,
    pub total: AtomicU64,
    pub error: RwLock<Option<String>>,
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
    /// Bond unit in base units (network-specific: mainnet/testnet=1B [10 DOLI], devnet=100M [1 DOLI])
    pub bond_unit: u64,
    /// Local peer ID
    pub peer_id: String,
    /// Peer count function
    pub peer_count: Arc<dyn Fn() -> usize + Send + Sync>,
    /// Peer list function (returns detailed peer info)
    pub peer_list: Arc<dyn Fn() -> Vec<PeerInfoEntry> + Send + Sync>,
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
            peer_list: Arc::new(std::vec::Vec::new),
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
                bond_unit: 1_000_000_000, // 10 DOLI default (mainnet/testnet)
                peer_id: "unknown".to_string(),
                peer_count: Arc::new(|| 0),
                peer_list: Arc::new(std::vec::Vec::new),
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

    /// Set peer list callback
    pub fn with_peer_list(
        mut self,
        f: impl Fn() -> Vec<PeerInfoEntry> + Send + Sync + 'static,
    ) -> Self {
        self.peer_list = Arc::new(f);
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

    /// Map a network name string to its bech32m address prefix.
    pub(super) fn network_prefix(&self) -> Option<&str> {
        match self.network.as_str() {
            "mainnet" => Some("doli"),
            "testnet" => Some("tdoli"),
            "devnet" => Some("ddoli"),
            _ => None,
        }
    }

    /// Resolve an address string (bech32m `doli1...` or 64-char hex) to a Hash.
    pub(super) fn resolve_address(&self, addr: &str) -> Result<Hash, RpcError> {
        let prefix = self.network_prefix();
        crypto::address::resolve(addr, prefix).map_err(|e| RpcError::invalid_params(e.to_string()))
    }
}
