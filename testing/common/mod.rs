#![allow(dead_code)]
//! Common test utilities for DOLI integration and E2E tests

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::{
    consensus::ConsensusParams, Amount, Block, BlockHeader, BlockHeight, Output, Slot, Transaction,
};
use storage::{ChainState, Outpoint, UtxoEntry, UtxoSet};
use tempfile::TempDir;
use tokio::sync::RwLock;
use vdf::{VdfOutput, VdfProof};

/// Test node configuration
#[derive(Clone)]
pub struct TestNodeConfig {
    pub data_dir: PathBuf,
    pub listen_port: u16,
    pub rpc_port: u16,
    pub bootstrap_nodes: Vec<String>,
    pub producer_key: Option<KeyPair>,
}

impl TestNodeConfig {
    pub fn new(temp_dir: &TempDir, port_offset: u16) -> Self {
        Self {
            data_dir: temp_dir.path().join(format!("node_{}", port_offset)),
            listen_port: 30303 + port_offset,
            rpc_port: 8545 + port_offset,
            bootstrap_nodes: Vec::new(),
            producer_key: None,
        }
    }

    pub fn with_producer_key(mut self, keypair: KeyPair) -> Self {
        self.producer_key = Some(keypair);
        self
    }

    pub fn with_bootstrap(mut self, addr: String) -> Self {
        self.bootstrap_nodes.push(addr);
        self
    }
}

/// In-memory test node for integration testing
pub struct TestNode {
    pub config: TestNodeConfig,
    pub chain_state: Arc<RwLock<ChainState>>,
    pub utxo_set: Arc<RwLock<UtxoSet>>,
    pub blocks: Arc<RwLock<HashMap<Hash, Block>>>,
    pub mempool: Arc<RwLock<Vec<Transaction>>>,
    pub keypair: KeyPair,
    pub params: ConsensusParams,
}

impl TestNode {
    pub fn new(config: TestNodeConfig) -> Self {
        let genesis_hash = Hash::ZERO;
        let keypair = config
            .producer_key
            .clone()
            .unwrap_or_else(KeyPair::generate);

        Self {
            config,
            chain_state: Arc::new(RwLock::new(ChainState::new(genesis_hash))),
            utxo_set: Arc::new(RwLock::new(UtxoSet::new())),
            blocks: Arc::new(RwLock::new(HashMap::new())),
            mempool: Arc::new(RwLock::new(Vec::new())),
            keypair,
            params: ConsensusParams::mainnet(),
        }
    }

    /// Get current chain height
    pub async fn height(&self) -> BlockHeight {
        self.chain_state.read().await.best_height
    }

    /// Get best block hash
    pub async fn best_hash(&self) -> Hash {
        self.chain_state.read().await.best_hash
    }

    /// Get a block by hash
    pub async fn get_block(&self, hash: &Hash) -> Option<Block> {
        self.blocks.read().await.get(hash).cloned()
    }

    /// Add a block to the chain
    pub async fn add_block(&self, block: Block) -> Result<(), String> {
        let hash = block.hash();
        let height = block.header.slot as BlockHeight;

        // Validate prev_hash (except for genesis)
        if height > 0 {
            let blocks = self.blocks.read().await;
            if !blocks.contains_key(&block.header.prev_hash) && !block.header.prev_hash.is_zero() {
                return Err("Invalid prev_hash: parent block not found".to_string());
            }
        }

        {
            let mut blocks = self.blocks.write().await;
            if blocks.contains_key(&hash) {
                return Err("Block already exists".to_string());
            }
            blocks.insert(hash, block.clone());
        }

        {
            let mut state = self.chain_state.write().await;
            state.best_hash = hash;
            state.best_height = height;
        }

        // Update UTXO set
        {
            let mut utxos = self.utxo_set.write().await;
            for (tx_idx, tx) in block.transactions.iter().enumerate() {
                let tx_hash = tx.hash();
                for (out_idx, output) in tx.outputs.iter().enumerate() {
                    let outpoint = Outpoint {
                        tx_hash,
                        index: out_idx as u32,
                    };
                    let entry = UtxoEntry {
                        output: output.clone(),
                        height,
                        is_coinbase: tx_idx == 0,
                        is_epoch_reward: tx.is_epoch_reward(),
                    };
                    utxos.insert(outpoint, entry);
                }

                // Remove spent UTXOs
                for input in &tx.inputs {
                    let outpoint = Outpoint {
                        tx_hash: input.prev_tx_hash,
                        index: input.output_index,
                    };
                    utxos.remove(&outpoint);
                }
            }
        }

        Ok(())
    }

    /// Revert the last n blocks
    pub async fn revert_blocks(&self, count: u64) -> Result<Vec<Block>, String> {
        let mut reverted = Vec::new();

        for _ in 0..count {
            let current_hash = self.best_hash().await;
            let block = self
                .get_block(&current_hash)
                .await
                .ok_or("Block not found")?;

            // Revert UTXO changes
            {
                let mut utxos = self.utxo_set.write().await;

                // Remove outputs created by this block
                for tx in &block.transactions {
                    let tx_hash = tx.hash();
                    for out_idx in 0..tx.outputs.len() {
                        let outpoint = Outpoint {
                            tx_hash,
                            index: out_idx as u32,
                        };
                        utxos.remove(&outpoint);
                    }
                }
            }

            // Update chain state to previous block
            {
                let mut state = self.chain_state.write().await;
                state.best_hash = block.header.prev_hash;
                state.best_height = state.best_height.saturating_sub(1);
            }

            reverted.push(block);
        }

        Ok(reverted)
    }

    /// Add transaction to mempool
    pub async fn add_to_mempool(&self, tx: Transaction) -> Result<(), String> {
        let mut mempool = self.mempool.write().await;
        if mempool.len() >= 10000 {
            return Err("Mempool full".to_string());
        }
        mempool.push(tx);
        Ok(())
    }

    /// Get mempool size
    pub async fn mempool_size(&self) -> usize {
        self.mempool.read().await.len()
    }

    /// Clear mempool
    pub async fn clear_mempool(&self) {
        self.mempool.write().await.clear();
    }
}

/// Create a test block at a given height
pub fn create_test_block(
    height: BlockHeight,
    prev_hash: Hash,
    producer: &PublicKey,
    transactions: Vec<Transaction>,
) -> Block {
    let params = ConsensusParams::mainnet();
    let slot = height as Slot;
    let timestamp = params.genesis_time + (slot as u64 * params.slot_duration);

    let merkle_root = doli_core::block::compute_merkle_root(&transactions);

    let header = BlockHeader {
        version: 1,
        prev_hash,
        merkle_root,
        presence_root: crypto::Hash::ZERO,
        genesis_hash: crypto::Hash::ZERO,
        timestamp,
        slot,
        producer: *producer,
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
    };

    Block::new(header, transactions)
}

/// Create a coinbase transaction
pub fn create_coinbase(height: BlockHeight, recipient: &Hash, amount: Amount) -> Transaction {
    Transaction::new_coinbase(amount, *recipient, height)
}

/// Create a transfer transaction
pub fn create_transfer(
    inputs: Vec<(Hash, u32)>,
    outputs: Vec<(Amount, Hash)>,
    keypair: &KeyPair,
) -> Transaction {
    use doli_core::transaction::Input;

    let inputs: Vec<Input> = inputs
        .into_iter()
        .map(|(hash, idx)| Input::new(hash, idx))
        .collect();

    let outputs: Vec<Output> = outputs
        .into_iter()
        .map(|(amount, pubkey_hash)| Output::normal(amount, pubkey_hash))
        .collect();

    let mut tx = Transaction::new_transfer(inputs, outputs);

    // Sign the transaction
    let msg = tx.signing_message();
    for input in &mut tx.inputs {
        input.signature = crypto::signature::sign(msg.as_bytes(), keypair.private_key());
    }

    tx
}

/// Wait for condition with timeout
pub async fn wait_for<F, Fut>(condition: F, timeout: Duration, poll_interval: Duration) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if condition().await {
            return true;
        }
        tokio::time::sleep(poll_interval).await;
    }
    false
}

/// Initialize test logging
pub fn init_test_logging() {
    use tracing_subscriber::EnvFilter;
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("doli=debug".parse().unwrap()))
        .try_init();
}

/// Generate a chain of test blocks
pub fn generate_test_chain(
    length: usize,
    producer: &KeyPair,
    initial_reward: Amount,
) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(length);
    let mut prev_hash = Hash::ZERO;
    let pubkey_hash = crypto::hash::hash(producer.public_key().as_bytes());

    for height in 0..length {
        let coinbase = create_coinbase(height as BlockHeight, &pubkey_hash, initial_reward);
        let block = create_test_block(
            height as BlockHeight,
            prev_hash,
            producer.public_key(),
            vec![coinbase],
        );
        prev_hash = block.hash();
        blocks.push(block);
    }

    blocks
}

/// Peer connection simulator
pub struct MockPeer {
    pub id: String,
    pub connected: bool,
    pub blocks_sent: Vec<Block>,
    pub blocks_received: Vec<Block>,
}

impl MockPeer {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            connected: false,
            blocks_sent: Vec::new(),
            blocks_received: Vec::new(),
        }
    }

    pub fn connect(&mut self) {
        self.connected = true;
    }

    pub fn disconnect(&mut self) {
        self.connected = false;
    }

    pub fn send_block(&mut self, block: Block) {
        if self.connected {
            self.blocks_sent.push(block);
        }
    }

    pub fn receive_block(&mut self, block: Block) {
        if self.connected {
            self.blocks_received.push(block);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_test_node_basics() {
        let temp_dir = TempDir::new().unwrap();
        let config = TestNodeConfig::new(&temp_dir, 0);
        let node = TestNode::new(config);

        assert_eq!(node.height().await, 0);
        assert_eq!(node.mempool_size().await, 0);
    }

    #[test]
    fn test_generate_chain() {
        let producer = KeyPair::generate();
        let chain = generate_test_chain(10, &producer, 5_000_000_000);

        assert_eq!(chain.len(), 10);

        // Verify chain links
        for i in 1..chain.len() {
            assert_eq!(chain[i].header.prev_hash, chain[i - 1].hash());
        }
    }
}
