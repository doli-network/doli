//! TestNetwork — simulated P2P network of real DOLI nodes.
//!
//! Each node is a real Node::new_for_test() with real RocksDB, real ProducerSet,
//! real SyncManager. No mocks. Blocks propagate via direct apply_block() calls
//! simulating gossip.
//!
//! Start with an exact replica of what we have in production.
//! Find the ceiling. Then optimize.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use tempfile::TempDir;
use tokio::sync::Mutex;
use vdf::{VdfOutput, VdfProof};

/// A simulated P2P network of real DOLI nodes.
/// Nodes are wrapped in Arc<Mutex> for parallel block propagation.
pub struct TestNetwork {
    /// Real nodes, each with its own RocksDB in a temp directory
    pub nodes: Vec<Arc<Mutex<Node>>>,
    /// Temp directories (must outlive nodes)
    _temps: Vec<TempDir>,
    /// Producer keypairs (shared across all nodes — same producer set)
    pub producers: Vec<KeyPair>,
    /// Consensus params (shared)
    pub params: ConsensusParams,
    /// Network topology: node_id → set of connected peer node_ids
    pub connections: HashMap<usize, HashSet<usize>>,
    /// Partitioned links: (a, b) pairs that are disconnected
    pub partitions: HashSet<(usize, usize)>,
    /// Genesis hash
    pub genesis_hash: Hash,
}

impl TestNetwork {
    /// Create a network with `n_nodes` nodes, each having the same `n_producers` registered.
    /// All nodes start connected to all other nodes (full mesh).
    pub async fn new(n_nodes: usize, n_producers: usize) -> Self {
        let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
        let mut nodes = Vec::with_capacity(n_nodes);
        let mut temps = Vec::with_capacity(n_nodes);

        for i in 0..n_nodes {
            let temp = TempDir::new().unwrap();
            let node = Node::new_for_test(temp.path().to_path_buf(), producers.clone())
                .await
                .unwrap_or_else(|e| panic!("Node {} init failed: {}", i, e));
            nodes.push(Arc::new(Mutex::new(node)));
            temps.push(temp);
        }

        let (params, genesis_hash) = {
            let n = nodes[0].lock().await;
            let gh = n.chain_state.read().await.best_hash;
            (n.params.clone(), gh)
        };

        // Full mesh topology
        let mut connections: HashMap<usize, HashSet<usize>> = HashMap::new();
        for i in 0..n_nodes {
            let mut peers = HashSet::new();
            for j in 0..n_nodes {
                if i != j {
                    peers.insert(j);
                }
            }
            connections.insert(i, peers);
        }

        Self {
            nodes,
            _temps: temps,
            producers,
            params,
            connections,
            partitions: HashSet::new(),
            genesis_hash,
        }
    }

    /// Number of nodes in the network
    pub fn size(&self) -> usize {
        self.nodes.len()
    }

    /// Build a block for a specific height/slot/producer
    pub fn build_block(
        &self,
        height: u64,
        slot: u32,
        prev_hash: Hash,
        producer: &KeyPair,
    ) -> Block {
        let reward = self.params.block_reward(height);
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let coinbase = Transaction::new_coinbase(reward, pool_hash, height);
        let timestamp = self.params.genesis_time + (slot as u64 * self.params.slot_duration);
        let merkle_root = doli_core::block::compute_merkle_root(&[coinbase.clone()]);
        let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

        let header = BlockHeader {
            version: 2,
            prev_hash,
            merkle_root,
            presence_root: Hash::ZERO,
            genesis_hash,
            timestamp,
            slot,
            producer: *producer.public_key(),
            vdf_output: VdfOutput {
                value: vec![0u8; 32],
            },
            vdf_proof: VdfProof::empty(),
        };

        Block::new(header, vec![coinbase])
    }

    /// Build a chain of blocks
    pub fn build_chain(
        &self,
        start_height: u64,
        start_slot: u32,
        prev_hash: Hash,
        producer: &KeyPair,
        count: usize,
    ) -> Vec<Block> {
        let mut blocks = Vec::with_capacity(count);
        let mut prev = prev_hash;
        for i in 0..count {
            let block = self.build_block(
                start_height + i as u64,
                start_slot + i as u32,
                prev,
                producer,
            );
            prev = block.hash();
            blocks.push(block);
        }
        blocks
    }

    /// Apply a block to a specific node
    pub async fn apply_to_node(&self, node_id: usize, block: Block) -> Result<(), String> {
        self.nodes[node_id]
            .lock()
            .await
            .apply_block(block, ValidationMode::Light)
            .await
            .map_err(|e| format!("Node {} apply_block failed: {}", node_id, e))
    }

    /// Propagate a block from source node to all connected peers — IN PARALLEL.
    /// Each node locks independently, utilizing all available cores.
    /// Returns the number of nodes that accepted the block.
    pub async fn propagate(&self, source: usize, block: Block) -> usize {
        let peers: Vec<usize> = self
            .connections
            .get(&source)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|peer| !self.is_partitioned(source, *peer))
            .collect();

        let futs: Vec<_> = peers
            .into_iter()
            .map(|peer| {
                let node = self.nodes[peer].clone();
                let block = block.clone();
                async move {
                    let mut n = node.lock().await;
                    n.apply_block(block, ValidationMode::Light).await.is_ok()
                }
            })
            .collect();

        let results = futures::future::join_all(futs).await;
        results.into_iter().filter(|ok| *ok).count()
    }

    /// Produce a block on a specific node and propagate to all peers IN PARALLEL.
    /// Returns the block and number of peers that accepted it.
    pub async fn produce_and_propagate(
        &self,
        producer_idx: usize,
    ) -> Result<(Block, usize), String> {
        // Get state from first node (all should be synced)
        let (height, prev_hash) = {
            let n = self.nodes[0].lock().await;
            let cs = n.chain_state.read().await;
            (cs.best_height + 1, cs.best_hash)
        };
        let slot = height as u32;

        let block = self.build_block(height, slot, prev_hash, &self.producers[producer_idx]);

        // Apply to the producing node first
        {
            let mut n = self.nodes[0].lock().await;
            n.apply_block(block.clone(), ValidationMode::Light)
                .await
                .map_err(|e| format!("Producer apply failed: {}", e))?;
        }

        // Propagate to all other nodes in parallel
        let accepted = self.propagate(0, block.clone()).await;

        Ok((block, accepted))
    }

    /// Produce N blocks and propagate each one to the network in parallel.
    /// Returns the final height.
    pub async fn produce_blocks(&self, count: usize, producer_idx: usize) -> u64 {
        for _ in 0..count {
            self.produce_and_propagate(producer_idx).await.unwrap();
        }
        self.height(0).await
    }

    /// Check if two nodes are partitioned (disconnected)
    fn is_partitioned(&self, a: usize, b: usize) -> bool {
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        self.partitions.contains(&(lo, hi))
    }

    /// Partition the network: disconnect node group A from group B.
    pub fn partition(&mut self, group_a: &[usize], group_b: &[usize]) {
        for &a in group_a {
            for &b in group_b {
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                self.partitions.insert((lo, hi));
            }
        }
    }

    /// Heal all partitions
    pub fn heal(&mut self) {
        self.partitions.clear();
    }

    /// Get height of a specific node
    pub async fn height(&self, node_id: usize) -> u64 {
        let n = self.nodes[node_id].lock().await;
        let h = n.chain_state.read().await.best_height;
        h
    }

    /// Get hash of a specific node
    pub async fn hash(&self, node_id: usize) -> Hash {
        let n = self.nodes[node_id].lock().await;
        let h = n.chain_state.read().await.best_hash;
        h
    }

    /// Check if all nodes are synced (same height and hash) — parallel query
    pub async fn is_synced(&self) -> bool {
        if self.nodes.is_empty() {
            return true;
        }
        let futs: Vec<_> = self
            .nodes
            .iter()
            .map(|node| {
                let node = node.clone();
                async move {
                    let n = node.lock().await;
                    let cs = n.chain_state.read().await;
                    (cs.best_height, cs.best_hash)
                }
            })
            .collect();

        let states = futures::future::join_all(futs).await;
        let (h0, hash0) = states[0];
        states.iter().all(|(h, hash)| *h == h0 && *hash == hash0)
    }

    /// Print status of all nodes
    pub async fn status(&self) {
        for i in 0..self.nodes.len() {
            let h = self.height(i).await;
            let hash = self.hash(i).await;
            eprintln!(
                "  Node {}: h={} hash={:.16}",
                i,
                h,
                hash.to_string()
            );
        }
    }
}

// ============================================================
// TESTS
// ============================================================

#[tokio::test]
async fn test_network_creates_and_syncs() {
    let mut net = TestNetwork::new(3, 3).await;
    assert_eq!(net.size(), 3);
    assert!(net.is_synced().await);

    // Produce 5 blocks
    let final_h = net.produce_blocks(5, 0).await;
    assert_eq!(final_h, 5);
    assert!(net.is_synced().await);
}

#[tokio::test]
async fn test_network_partition_and_heal() {
    let mut net = TestNetwork::new(5, 3).await;

    // Produce 5 blocks — all synced
    net.produce_blocks(5, 0).await;
    assert!(net.is_synced().await);

    // Partition: nodes [0,1] disconnected from [2,3,4]
    net.partition(&[0, 1], &[2, 3, 4]);

    // Produce 3 more blocks on node 0 — only node 1 receives
    for _ in 0..3 {
        let (h, prev) = {
            let n = net.nodes[0].lock().await;
            let cs = n.chain_state.read().await;
            (cs.best_height + 1, cs.best_hash)
        };
        let block = net.build_block(h, h as u32, prev, &net.producers[0]);
        net.apply_to_node(0, block.clone()).await.unwrap();
        net.propagate(0, block).await;
    }

    // Nodes 0,1 at height 8; nodes 2,3,4 at height 5
    assert_eq!(net.height(0).await, 8);
    assert_eq!(net.height(1).await, 8);
    assert_eq!(net.height(2).await, 5);
    assert!(!net.is_synced().await);

    // Heal partition and propagate missing blocks
    net.heal();

    // Send blocks 6,7,8 to nodes 2,3,4
    for h in 6..=8u64 {
        let block = {
            let n = net.nodes[0].lock().await;
            n.block_store.get_block_by_height(h).ok().flatten()
        };
        if let Some(block) = block {
            for node_id in [2, 3, 4] {
                let _ = net.apply_to_node(node_id, block.clone()).await;
            }
        }
    }

    assert!(net.is_synced().await);
    assert_eq!(net.height(0).await, 8);
}

#[tokio::test]
async fn test_network_10_nodes() {
    let mut net = TestNetwork::new(10, 5).await;
    let final_h = net.produce_blocks(20, 0).await;
    assert_eq!(final_h, 20);
    assert!(net.is_synced().await);
}

#[tokio::test]
async fn test_network_scale_ceiling() {
    // Find how many real nodes we can create
    // Start small, increase until it gets slow
    let start = std::time::Instant::now();
    let net = TestNetwork::new(25, 5).await;
    let init_time = start.elapsed();

    let start = std::time::Instant::now();
    // Produce 10 blocks across 25 nodes
    let mut net = net;
    net.produce_blocks(10, 0).await;
    let produce_time = start.elapsed();

    assert!(net.is_synced().await);

    eprintln!(
        "Scale test: {} nodes, init={:?}, 10 blocks={:?}",
        net.size(),
        init_time,
        produce_time
    );
}

#[tokio::test]
async fn test_network_50_nodes() {
    let start = std::time::Instant::now();
    let mut net = TestNetwork::new(50, 5).await;
    let init = start.elapsed();
    let start = std::time::Instant::now();
    net.produce_blocks(10, 0).await;
    let produce = start.elapsed();
    assert!(net.is_synced().await);
    eprintln!("50 nodes: init={:?}, 10 blocks={:?}", init, produce);
}

#[tokio::test]
async fn test_network_100_nodes() {
    let start = std::time::Instant::now();
    let mut net = TestNetwork::new(100, 5).await;
    let init = start.elapsed();
    let start = std::time::Instant::now();
    net.produce_blocks(10, 0).await;
    let produce = start.elapsed();
    assert!(net.is_synced().await);
    eprintln!("100 nodes: init={:?}, 10 blocks={:?}", init, produce);
}

#[tokio::test]
#[ignore] // Run with --test-threads=1 to avoid FD exhaustion
async fn test_network_500_nodes() {
    let start = std::time::Instant::now();
    let mut net = TestNetwork::new(500, 5).await;
    let init = start.elapsed();
    let start = std::time::Instant::now();
    net.produce_blocks(5, 0).await;
    let produce = start.elapsed();
    assert!(net.is_synced().await);
    eprintln!("500 nodes: init={:?}, 5 blocks={:?}", init, produce);
}

#[tokio::test]
#[ignore] // Run with --test-threads=1 to avoid FD exhaustion
async fn test_network_1000_nodes() {
    let start = std::time::Instant::now();
    let mut net = TestNetwork::new(1000, 5).await;
    let init = start.elapsed();
    let start = std::time::Instant::now();
    net.produce_blocks(5, 0).await;
    let produce = start.elapsed();
    assert!(net.is_synced().await);
    eprintln!("1000 nodes: init={:?}, 5 blocks={:?}", init, produce);
}

#[tokio::test]
#[ignore] // Requires ulimit -n 65536
async fn test_network_5000_nodes() {
    let start = std::time::Instant::now();
    let mut net = TestNetwork::new(5000, 5).await;
    let init = start.elapsed();
    let start = std::time::Instant::now();
    net.produce_blocks(3, 0).await;
    let produce = start.elapsed();
    assert!(net.is_synced().await);
    eprintln!("5000 nodes: init={:?}, 3 blocks={:?}", init, produce);
}

#[tokio::test]
#[ignore] // Requires ulimit -n 65536
async fn test_network_10000_nodes() {
    let start = std::time::Instant::now();
    let mut net = TestNetwork::new(10000, 5).await;
    let init = start.elapsed();
    let start = std::time::Instant::now();
    net.produce_blocks(2, 0).await;
    let produce = start.elapsed();
    assert!(net.is_synced().await);
    eprintln!("10000 nodes: init={:?}, 2 blocks={:?}", init, produce);
}

// ============================================================
// ClusterNetwork — sequential clusters for massive scale
// ============================================================

/// Results from a single cluster run
pub struct ClusterResult {
    pub cluster_id: usize,
    pub nodes: usize,
    pub init_time: std::time::Duration,
    pub produce_time: std::time::Duration,
    pub blocks_produced: usize,
    pub all_synced: bool,
    pub final_height: u64,
    pub final_hash: Hash,
}

/// Run N clusters of M nodes sequentially, reusing memory.
/// Each cluster shares the same genesis and producer set.
/// Total simulated nodes = clusters × nodes_per_cluster.
pub struct ClusterNetwork {
    pub producers: Vec<KeyPair>,
    pub n_producers: usize,
    pub nodes_per_cluster: usize,
    pub blocks_per_cluster: usize,
    pub results: Vec<ClusterResult>,
}

impl ClusterNetwork {
    pub fn new(
        n_producers: usize,
        nodes_per_cluster: usize,
        blocks_per_cluster: usize,
    ) -> Self {
        let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
        Self {
            producers,
            n_producers,
            nodes_per_cluster,
            blocks_per_cluster,
            results: Vec::new(),
        }
    }

    /// Run `n_clusters` clusters sequentially.
    /// Each cluster creates `nodes_per_cluster` real nodes, produces blocks,
    /// verifies sync, then drops everything (freeing FDs and memory).
    pub async fn run(&mut self, n_clusters: usize) {
        let total_start = std::time::Instant::now();

        for cluster_id in 0..n_clusters {
            let start = std::time::Instant::now();

            // Create cluster
            let net = TestNetwork::new(self.nodes_per_cluster, self.n_producers).await;
            let init_time = start.elapsed();

            // Produce blocks
            let prod_start = std::time::Instant::now();
            net.produce_blocks(self.blocks_per_cluster, cluster_id % self.n_producers)
                .await;
            let produce_time = prod_start.elapsed();

            // Verify sync
            let synced = net.is_synced().await;
            let final_height = net.height(0).await;
            let final_hash = net.hash(0).await;

            let result = ClusterResult {
                cluster_id,
                nodes: self.nodes_per_cluster,
                init_time,
                produce_time,
                blocks_produced: self.blocks_per_cluster,
                all_synced: synced,
                final_height,
                final_hash,
            };

            eprintln!(
                "  Cluster {}/{}: {} nodes, init={:?}, {} blocks={:?}, synced={}",
                cluster_id + 1,
                n_clusters,
                result.nodes,
                result.init_time,
                result.blocks_produced,
                result.produce_time,
                result.all_synced,
            );

            self.results.push(result);

            // Drop net — frees all RocksDB instances and temp dirs
            drop(net);
        }

        let total_time = total_start.elapsed();
        let total_nodes: usize = self.results.iter().map(|r| r.nodes).sum();
        let all_synced = self.results.iter().all(|r| r.all_synced);
        let avg_init: std::time::Duration = self
            .results
            .iter()
            .map(|r| r.init_time)
            .sum::<std::time::Duration>()
            / self.results.len() as u32;
        let avg_produce: std::time::Duration = self
            .results
            .iter()
            .map(|r| r.produce_time)
            .sum::<std::time::Duration>()
            / self.results.len() as u32;

        eprintln!();
        eprintln!("  === ClusterNetwork Summary ===");
        eprintln!("  Clusters:        {}", self.results.len());
        eprintln!("  Nodes/cluster:   {}", self.nodes_per_cluster);
        eprintln!("  Total nodes:     {}", total_nodes);
        eprintln!("  Blocks/cluster:  {}", self.blocks_per_cluster);
        eprintln!("  Avg init:        {:?}", avg_init);
        eprintln!("  Avg produce:     {:?}", avg_produce);
        eprintln!("  All synced:      {}", all_synced);
        eprintln!("  Total time:      {:?}", total_time);
        eprintln!(
            "  Throughput:      {:.0} nodes/sec",
            total_nodes as f64 / total_time.as_secs_f64()
        );

        assert!(all_synced, "Not all clusters synced!");
    }
}

// ============================================================
// CLUSTER TESTS
// ============================================================

/// 10 clusters × 100 nodes = 1,000 total (fast, runs in CI)
#[tokio::test]
async fn test_cluster_10x100() {
    let mut cluster = ClusterNetwork::new(5, 100, 5);
    cluster.run(10).await;
    assert_eq!(cluster.results.len(), 10);
}

/// 10 clusters × 500 nodes = 5,000 total
#[tokio::test]
#[ignore] // ~30s, run with --test-threads=1
async fn test_cluster_10x500() {
    let mut cluster = ClusterNetwork::new(5, 500, 3);
    cluster.run(10).await;
    assert_eq!(cluster.results.len(), 10);
}

/// 10 clusters × 1000 nodes = 10,000 total
#[tokio::test]
#[ignore] // ~90s, run with --test-threads=1
async fn test_cluster_10x1000() {
    let mut cluster = ClusterNetwork::new(5, 1000, 3);
    cluster.run(10).await;
    assert_eq!(cluster.results.len(), 10);
}

/// 100 clusters × 1000 nodes = 100,000 total
#[tokio::test]
#[ignore] // ~15min, run with --test-threads=1
async fn test_cluster_100x1000() {
    let mut cluster = ClusterNetwork::new(5, 1000, 2);
    cluster.run(100).await;
    assert_eq!(cluster.results.len(), 100);
}
