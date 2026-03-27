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

// ============================================================
// GOSSIP SIMULATION — realistic block propagation with delay
// ============================================================

/// Simulate realistic gossip propagation where blocks arrive at different
/// nodes at different times, causing temporary excluded_producers divergence.
/// This is the test that reveals whether the network can grow.
impl TestNetwork {
    /// Propagate a block with random delay to simulate gossip.
    /// Some nodes receive the block, others don't (simulating network latency).
    /// Returns (received_count, missed_count).
    pub async fn gossip_propagate(
        &self,
        block: Block,
        delivery_probability: f64,
    ) -> (usize, usize) {
        use std::collections::HashSet;

        let n = self.nodes.len();
        let mut received = 0;
        let mut missed = 0;

        // Determine which nodes receive this block (random subset)
        let mut receivers = Vec::new();
        for i in 0..n {
            // Simple deterministic "random" based on block hash + node id
            let hash_byte = block.hash().as_bytes()[(i % 32)] as f64 / 255.0;
            if hash_byte < delivery_probability {
                receivers.push(i);
            }
        }

        // Apply to receivers in parallel
        let futs: Vec<_> = receivers
            .iter()
            .map(|&node_id| {
                let node = self.nodes[node_id].clone();
                let block = block.clone();
                async move {
                    let mut n = node.lock().await;
                    n.apply_block(block, ValidationMode::Light).await.is_ok()
                }
            })
            .collect();

        let results = futures::future::join_all(futs).await;
        received = results.iter().filter(|ok| **ok).count();
        missed = n - received;

        (received, missed)
    }

    /// Simulate gossip with check_producer_eligibility — the REAL path.
    /// Blocks go through eligibility check before apply_block, just like production.
    /// Returns (accepted, rejected_eligibility, rejected_apply).
    pub async fn gossip_with_eligibility(
        &self,
        block: Block,
    ) -> (usize, usize, usize) {
        let n = self.nodes.len();
        let mut accepted = 0;
        let mut rejected_elig = 0;
        let mut rejected_apply = 0;

        let futs: Vec<_> = (0..n)
            .map(|node_id| {
                let node = self.nodes[node_id].clone();
                let block = block.clone();
                async move {
                    let mut n = node.lock().await;
                    // Step 1: check_producer_eligibility (gossip gate)
                    match n.check_producer_eligibility(&block).await {
                        Ok(()) => {
                            // Step 2: apply_block (consensus)
                            match n.apply_block(block, ValidationMode::Light).await {
                                Ok(()) => 0u8,     // accepted
                                Err(_) => 2u8,     // rejected at apply
                            }
                        }
                        Err(_) => 1u8, // rejected at eligibility
                    }
                }
            })
            .collect();

        let results = futures::future::join_all(futs).await;
        for r in results {
            match r {
                0 => accepted += 1,
                1 => rejected_elig += 1,
                _ => rejected_apply += 1,
            }
        }

        (accepted, rejected_elig, rejected_apply)
    }

    /// Add a producer to excluded_producers on a specific node
    pub async fn exclude_producer(&self, node_id: usize, producer: &PublicKey) {
        let mut n = self.nodes[node_id].lock().await;
        n.excluded_producers.insert(*producer);
    }

    /// Get excluded_producers count for a node
    pub async fn excluded_count(&self, node_id: usize) -> usize {
        let n = self.nodes[node_id].lock().await;
        n.excluded_producers.len()
    }

    /// Clear excluded_producers on a specific node
    pub async fn clear_excluded(&self, node_id: usize) {
        let mut n = self.nodes[node_id].lock().await;
        n.excluded_producers.clear();
    }
}

// ============================================================
// TEST: Gossip divergence with growing producer count
// ============================================================

/// Test that N nodes with different excluded_producers sets can still
/// sync after rollback clears the exclusions.
/// This simulates the real production scenario where nodes see different
/// blocks first and accumulate different exclusion sets.
#[tokio::test]
async fn test_gossip_divergence_and_recovery() {
    let n_nodes = 20;
    let n_producers = 10;
    let mut net = TestNetwork::new(n_nodes, n_producers).await;
    let params = net.params.clone();
    let genesis = net.genesis_hash;

    // Phase 1: Build 50 blocks (past genesis period for devnet)
    // All nodes receive all blocks — everyone synced
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 50);
    for block in &base {
        net.propagate(0, block.clone()).await;
        // Also apply to node 0 (propagate skips source)
        net.apply_to_node(0, block.clone()).await.ok();
    }
    assert!(net.is_synced().await);
    assert_eq!(net.height(0).await, 50);

    // Phase 2: Simulate gossip divergence
    // Half the nodes miss some blocks → they develop different excluded_producers
    let fork_point = base[49].hash();

    // Nodes 0-9 receive blocks 51-55 (by producer[0])
    // Nodes 10-19 DON'T receive them → they fall behind
    let chain_a = net.build_chain(51, 51, fork_point, &net.producers[0].clone(), 5);
    for block in &chain_a {
        for node_id in 0..10 {
            net.apply_to_node(node_id, block.clone()).await.ok();
        }
    }

    // Nodes 0-9 are at height 55, nodes 10-19 at height 50
    assert_eq!(net.height(0).await, 55);
    assert_eq!(net.height(15).await, 50);

    // Nodes 10-19 develop excluded_producers because they think producer[0]
    // missed slots (they didn't see the blocks)
    for node_id in 10..20 {
        net.exclude_producer(node_id, net.producers[0].public_key()).await;
    }

    // Phase 3: Try to sync nodes 10-19 with blocks 51-55
    // With the fix (clear excluded on rollback), nodes should accept after
    // rolling back from their stale state.
    // Without the fix, check_producer_eligibility would reject these blocks.
    let mut all_synced_after = true;
    for block in &chain_a {
        let (accepted, rejected_elig, rejected_apply) =
            net.gossip_with_eligibility(block.clone()).await;
        if rejected_elig > 0 {
            eprintln!(
                "  Block slot={}: accepted={}, rejected_eligibility={}, rejected_apply={}",
                block.header.slot, accepted, rejected_elig, rejected_apply
            );
            all_synced_after = false;
        }
    }

    // Verify all nodes converged
    let synced = net.is_synced().await;
    let h0 = net.height(0).await;

    eprintln!(
        "  Result: synced={}, height={}, all_accepted={}",
        synced, h0, all_synced_after
    );

    // All nodes should be at height 55 and synced
    assert!(
        synced,
        "Network should converge after gossip divergence"
    );
    assert_eq!(h0, 55);
}

/// Test convergence at scale: 50 nodes, 20 producers, with realistic
/// gossip delays where blocks are delivered to random subsets.
#[tokio::test]
async fn test_gossip_convergence_50_nodes() {
    let n_nodes = 50;
    let n_producers = 20;
    let net = TestNetwork::new(n_nodes, n_producers).await;
    let genesis = net.genesis_hash;

    // Build 45 blocks to exit genesis period — all nodes receive all
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }
    assert!(net.is_synced().await);

    // Now produce 20 more blocks with 80% delivery probability
    // Some nodes will miss some blocks
    let mut prev = base[44].hash();
    let mut total_rejected = 0;
    for i in 0..20 {
        let producer_idx = i % n_producers;
        let height = 46 + i as u64;
        let slot = 46 + i as u32;
        let block = net.build_block(height, slot, prev, &net.producers[producer_idx]);
        prev = block.hash();

        // Apply to node 0 first (always receives)
        net.apply_to_node(0, block.clone()).await.ok();

        // Gossip with 80% delivery — some nodes miss some blocks
        let (received, missed) = net.gossip_propagate(block.clone(), 0.80).await;
        if missed > 0 {
            total_rejected += missed;
        }
    }

    // Check how many nodes are synced
    let target_height = net.height(0).await;
    let mut synced_count = 0;
    let mut behind_count = 0;
    for i in 0..n_nodes {
        let h = net.height(i).await;
        if h == target_height {
            synced_count += 1;
        } else {
            behind_count += 1;
        }
    }

    eprintln!(
        "  After gossip: {} synced at h={}, {} behind, {} total missed deliveries",
        synced_count, target_height, behind_count, total_rejected
    );

    // Now send ALL blocks to ALL nodes (simulating eventual consistency)
    for i in 0..20 {
        let height = 46 + i as u64;
        if let Ok(Some(block)) = {
            let n = net.nodes[0].lock().await;
            n.block_store.get_block_by_height(height)
        } {
            for node_id in 1..n_nodes {
                net.apply_to_node(node_id, block.clone()).await.ok();
            }
        }
    }

    // All should converge now
    let final_synced = net.is_synced().await;
    let final_height = net.height(0).await;

    eprintln!(
        "  After sync: synced={}, height={}",
        final_synced, final_height
    );

    assert!(
        final_synced,
        "All nodes should converge after receiving all blocks"
    );
    assert_eq!(final_height, 65);
}

// ============================================================
// REALISTIC GOSSIP — delay, partial delivery, slot gaps,
// excluded_producers divergence, sync backfill
// ============================================================

/// Result of a gossip round for a single block
#[derive(Debug)]
pub struct GossipRoundResult {
    pub slot: u32,
    pub delivered: usize,
    pub missed: usize,
    pub rejected_eligibility: usize,
    pub rejected_apply: usize,
    pub accepted: usize,
}

/// Accumulated stats across all gossip rounds
#[derive(Debug, Default)]
pub struct GossipStats {
    pub total_blocks: usize,
    pub total_delivered: usize,
    pub total_missed: usize,
    pub total_rejected_elig: usize,
    pub total_rejected_apply: usize,
    pub total_accepted: usize,
    pub rounds: Vec<GossipRoundResult>,
}

impl GossipStats {
    pub fn delivery_rate(&self) -> f64 {
        if self.total_delivered + self.total_missed == 0 {
            return 1.0;
        }
        self.total_delivered as f64 / (self.total_delivered + self.total_missed) as f64
    }

    pub fn acceptance_rate(&self) -> f64 {
        if self.total_delivered == 0 {
            return 0.0;
        }
        self.total_accepted as f64 / self.total_delivered as f64
    }

    pub fn eligibility_rejection_rate(&self) -> f64 {
        if self.total_delivered == 0 {
            return 0.0;
        }
        self.total_rejected_elig as f64 / self.total_delivered as f64
    }
}

impl TestNetwork {
    /// Realistic gossip propagation with:
    /// - Configurable delivery probability (simulates network loss)
    /// - Deterministic "randomness" based on block hash + node id (reproducible)
    /// - check_producer_eligibility gate (the real gossip path)
    /// - Slot gap tracking that causes excluded_producers divergence
    ///
    /// Returns per-round stats.
    pub async fn gossip_realistic(
        &self,
        block: &Block,
        delivery_probability: f64,
    ) -> GossipRoundResult {
        let n = self.nodes.len();
        let slot = block.header.slot;

        // Determine delivery per node (deterministic pseudo-random)
        let block_hash = block.hash(); let block_bytes = block_hash.as_bytes();
        let mut delivered_to = Vec::new();
        let mut missed_by = Vec::new();

        for i in 0..n {
            // Mix block hash with node index for deterministic but varied delivery
            let seed = block_bytes[(i * 7) % 32] as u16
                + block_bytes[(i * 13 + 3) % 32] as u16;
            let prob = (seed as f64) / 510.0; // 0.0 to 1.0
            if prob < delivery_probability {
                delivered_to.push(i);
            } else {
                missed_by.push(i);
            }
        }

        // Deliver to nodes in parallel — through check_producer_eligibility
        let mut accepted = 0usize;
        let mut rejected_elig = 0usize;
        let mut rejected_apply = 0usize;

        let futs: Vec<_> = delivered_to
            .iter()
            .map(|&node_id| {
                let node = self.nodes[node_id].clone();
                let block = block.clone();
                async move {
                    let mut n = node.lock().await;

                    // Gate 1: check_producer_eligibility (gossip filter)
                    match n.check_producer_eligibility(&block).await {
                        Ok(()) => {
                            // Gate 2: apply_block
                            match n.apply_block(block, ValidationMode::Light).await {
                                Ok(()) => 0u8,  // accepted
                                Err(_) => 2u8,  // rejected at apply (duplicate, etc)
                            }
                        }
                        Err(_) => 1u8, // rejected at eligibility
                    }
                }
            })
            .collect();

        let results = futures::future::join_all(futs).await;
        for r in results {
            match r {
                0 => accepted += 1,
                1 => rejected_elig += 1,
                _ => rejected_apply += 1,
            }
        }

        GossipRoundResult {
            slot,
            delivered: delivered_to.len(),
            missed: missed_by.len(),
            rejected_eligibility: rejected_elig,
            rejected_apply: rejected_apply,
            accepted,
        }
    }

    /// Simulate sync/backfill: send ALL blocks from node 0 to nodes that are behind.
    /// This is what happens when a node requests missing blocks from peers.
    /// Returns how many blocks were successfully backfilled across all nodes.
    pub async fn backfill_from_leader(&self) -> usize {
        let leader_height = self.height(0).await;
        let n = self.nodes.len();
        let mut total_backfilled = 0;

        for node_id in 1..n {
            let node_height = self.height(node_id).await;
            if node_height < leader_height {
                // This node is behind — send missing blocks
                for h in (node_height + 1)..=leader_height {
                    let block = {
                        let n = self.nodes[0].lock().await;
                        n.block_store.get_block_by_height(h).ok().flatten()
                    };
                    if let Some(block) = block {
                        // Backfill bypasses check_producer_eligibility — it's sync, not gossip
                        let mut n = self.nodes[node_id].lock().await;
                        if n.apply_block(block, ValidationMode::Light).await.is_ok() {
                            total_backfilled += 1;
                        }
                    }
                }
            }
        }

        total_backfilled
    }

    /// Simulate sync/backfill that goes through check_producer_eligibility.
    /// This is the path that BREAKS when excluded_producers diverges.
    /// Returns (backfilled, rejected_by_eligibility).
    pub async fn backfill_with_eligibility(&self) -> (usize, usize) {
        let leader_height = self.height(0).await;
        let n = self.nodes.len();
        let mut total_backfilled = 0;
        let mut total_rejected = 0;

        for node_id in 1..n {
            let node_height = self.height(node_id).await;
            if node_height < leader_height {
                for h in (node_height + 1)..=leader_height {
                    let block = {
                        let n = self.nodes[0].lock().await;
                        n.block_store.get_block_by_height(h).ok().flatten()
                    };
                    if let Some(block) = block {
                        let mut n = self.nodes[node_id].lock().await;
                        match n.check_producer_eligibility(&block).await {
                            Ok(()) => {
                                if n.apply_block(block, ValidationMode::Light).await.is_ok() {
                                    total_backfilled += 1;
                                }
                            }
                            Err(_) => {
                                total_rejected += 1;
                                break; // Can't continue — chain is broken
                            }
                        }
                    }
                }
            }
        }

        (total_backfilled, total_rejected)
    }

    /// Full sync simulation: for each behind node, find common ancestor
    /// with leader, rollback to ancestor, apply leader's chain forward.
    /// This is what the real sync protocol does (header-first + block download).
    /// Returns (nodes_synced, total_rollbacks, total_applied).
    pub async fn sync_from_leader(&self) -> (usize, usize, usize) {
        let leader_height = self.height(0).await;
        let leader_hash = self.hash(0).await;
        let n = self.nodes.len();
        let mut nodes_synced = 0;
        let mut total_rollbacks = 0;
        let mut total_applied = 0;

        for node_id in 1..n {
            let node_height = self.height(node_id).await;
            let node_hash = self.hash(node_id).await;

            // Already synced?
            if node_height == leader_height && node_hash == leader_hash {
                nodes_synced += 1;
                continue;
            }

            // Step 1: Find common ancestor by walking back the leader's chain
            // and checking if the behind node has each block
            let mut ancestor_height = None;
            {
                let leader = self.nodes[0].lock().await;
                let node = self.nodes[node_id].lock().await;

                // Walk leader's chain backwards to find a block both have
                for h in (1..=std::cmp::min(node_height, leader_height)).rev() {
                    let leader_block = leader.block_store.get_block_by_height(h).ok().flatten();
                    let node_block = node.block_store.get_block_by_height(h).ok().flatten();

                    if let (Some(lb), Some(nb)) = (leader_block, node_block) {
                        if lb.hash() == nb.hash() {
                            ancestor_height = Some(h);
                            break;
                        }
                    }
                }
            }

            // If no common ancestor found, try height 0 (genesis)
            let ancestor_h = ancestor_height.unwrap_or(0);

            // Step 2: Reset behind node to ancestor state.
            // In production, this is what snap sync or checkpoint sync does:
            // wipe state to a known-good point, then re-apply from there.
            // We simulate by rolling back as far as possible, then rebuilding.
            {
                let mut node = self.nodes[node_id].lock().await;
                let current_h = node.chain_state.read().await.best_height;

                // Reset chain state directly to ancestor
                {
                    let mut cs = node.chain_state.write().await;
                    cs.best_height = ancestor_h;
                    // Get the ancestor's hash from leader's block store
                    let ancestor_hash = if ancestor_h == 0 {
                        cs.genesis_hash
                    } else {
                        let leader = self.nodes[0].lock().await;
                        leader.block_store.get_block_by_height(ancestor_h)
                            .ok().flatten()
                            .map(|b| b.hash())
                            .unwrap_or(cs.genesis_hash)
                    };
                    cs.best_hash = ancestor_hash;
                    cs.best_slot = ancestor_h as u32;
                }

                // Reset sync manager tip to match
                {
                    let cs = node.chain_state.read().await;
                    node.sync_manager.write().await.update_local_tip(
                        cs.best_height, cs.best_hash, cs.best_slot,
                    );
                }

                // Clear stale fork recovery state
                node.excluded_producers.clear();
                node.cumulative_rollback_depth = 0;
                node.cached_scheduler = None;

                total_rollbacks += (current_h - ancestor_h) as usize;
            }

            // Step 3: Apply leader's blocks from ancestor+1 to leader_height
            let mut applied = 0;
            for h in (ancestor_h + 1)..=leader_height {
                let block = {
                    let leader = self.nodes[0].lock().await;
                    leader.block_store.get_block_by_height(h).ok().flatten()
                };
                if let Some(block) = block {
                    let mut node = self.nodes[node_id].lock().await;
                    match node.apply_block(block, ValidationMode::Light).await {
                        Ok(()) => applied += 1,
                        Err(_) => break,
                    }
                } else {
                    break;
                }
            }
            total_applied += applied;

            // Check if synced now
            let final_h = self.height(node_id).await;
            let final_hash = self.hash(node_id).await;
            if final_h == leader_height && final_hash == leader_hash {
                nodes_synced += 1;
            }
        }

        (nodes_synced, total_rollbacks, total_applied)
    }

    /// Count how many nodes have divergent excluded_producers
    pub async fn count_divergent_exclusions(&self) -> (usize, usize, usize) {
        let n = self.nodes.len();
        let mut exclusion_counts: Vec<usize> = Vec::new();

        for i in 0..n {
            let node = self.nodes[i].lock().await;
            exclusion_counts.push(node.excluded_producers.len());
        }

        let with_exclusions = exclusion_counts.iter().filter(|c| **c > 0).count();
        let max_exclusions = exclusion_counts.iter().max().copied().unwrap_or(0);
        let total_exclusions: usize = exclusion_counts.iter().sum();

        (with_exclusions, max_exclusions, total_exclusions)
    }

    /// Count how many nodes are at each height
    pub async fn height_distribution(&self) -> HashMap<u64, usize> {
        let mut dist = HashMap::new();
        for i in 0..self.nodes.len() {
            let h = self.height(i).await;
            *dist.entry(h).or_insert(0) += 1;
        }
        dist
    }
}

// ============================================================
// THE DEFINITIVE SCALE TEST
// ============================================================

/// 20 nodes, 10 producers, 100 blocks with 90% gossip delivery.
/// Measures divergence and tests convergence after backfill.
/// This is the test that reveals whether the network can grow.
#[tokio::test]
async fn test_realistic_gossip_20_nodes_100_blocks() {
    let n_nodes = 20;
    let n_producers = 10;
    let delivery_rate = 0.90;
    let total_blocks = 100;

    let net = TestNetwork::new(n_nodes, n_producers).await;
    let genesis = net.genesis_hash;

    // Phase 1: Build 45 blocks to exit genesis period — 100% delivery
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }
    assert!(net.is_synced().await);

    // DIAGNOSTIC: Check what node 1 expects for slot 46 vs what we produce
    {
        let first_block = net.build_block(46, 46, base[44].hash(), &net.producers[0]);
        let actual_producer = first_block.header.producer;

        // What does node 1 compute as expected producer for slot 46?
        let n1 = net.nodes[1].lock().await;
        let ps = n1.producer_set.read().await;
        let active: Vec<PublicKey> = ps.active_producers_at_height(46)
            .iter().map(|p| p.public_key).collect();
        let n_active = active.len();
        drop(ps);

        let weighted = n1.bond_weights_for_scheduling(active.clone()).await;
        let n_weighted = weighted.len();

        let mut sorted: Vec<PublicKey> = weighted.iter().map(|(pk, _)| *pk).collect();
        sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

        let expected_idx = (46usize) % sorted.len();
        let expected_producer = sorted[expected_idx];

        let node1_height = n1.chain_state.read().await.best_height;
        let excluded_count = n1.excluded_producers.len();
        let snapshot_len = n1.epoch_bond_snapshot.len();

        eprintln!();
        eprintln!("  === DIAGNOSTIC: Node 1 eligibility for slot 46 ===");
        eprintln!("  Node 1 height:      {}", node1_height);
        eprintln!("  Active producers:   {}", n_active);
        eprintln!("  Weighted producers: {}", n_weighted);
        eprintln!("  Sorted list len:    {}", sorted.len());
        eprintln!("  Excluded count:     {}", excluded_count);
        eprintln!("  Bond snapshot len:  {}", snapshot_len);
        eprintln!("  Slot 46 % {} = idx {}", sorted.len(), expected_idx);
        eprintln!("  Expected producer:  {:?}", &expected_producer.as_bytes()[..4]);
        eprintln!("  Actual producer:    {:?}", &actual_producer.as_bytes()[..4]);
        eprintln!("  Match: {}", expected_producer == actual_producer);

        // Also check: does the ValidationContext path match?
        let err = n1.check_producer_eligibility(&first_block).await;
        eprintln!("  check_producer_eligibility result: {:?}", err.is_ok());
        if let Err(e) = &err {
            eprintln!("  Error: {}", e);
        }
        drop(n1);
    }

    // Phase 2: Produce blocks with realistic gossip (90% delivery)
    let mut stats = GossipStats::default();
    let mut prev = base[44].hash();

    // Build sorted producer list for round-robin (same order as validation)
    let mut rr_producers: Vec<(usize, PublicKey)> = net
        .producers
        .iter()
        .enumerate()
        .map(|(i, kp)| (i, *kp.public_key()))
        .collect();
    rr_producers.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));

    for i in 0..total_blocks {
        let height = 46 + i as u64;
        let slot = 46 + i as u32;
        // Use the CORRECT producer for this slot (round-robin)
        let rr_idx = (slot as usize) % rr_producers.len();
        let (orig_idx, _) = rr_producers[rr_idx];
        let block = net.build_block(height, slot, prev, &net.producers[orig_idx]);
        prev = block.hash();

        // Node 0 always receives (it's the "leader")
        net.apply_to_node(0, block.clone()).await.ok();

        // Gossip to others with partial delivery
        let round = net.gossip_realistic(&block, delivery_rate).await;

        stats.total_delivered += round.delivered;
        stats.total_missed += round.missed;
        stats.total_rejected_elig += round.rejected_eligibility;
        stats.total_rejected_apply += round.rejected_apply;
        stats.total_accepted += round.accepted;
        stats.total_blocks += 1;
        stats.rounds.push(round);
    }

    // Phase 2 results
    let dist = net.height_distribution().await;
    let (nodes_with_excl, max_excl, total_excl) = net.count_divergent_exclusions().await;

    eprintln!();
    eprintln!("  === Realistic Gossip Test ({} nodes, {} producers, {} blocks) ===", n_nodes, n_producers, total_blocks);
    eprintln!("  Delivery rate:     {:.1}%", stats.delivery_rate() * 100.0);
    eprintln!("  Acceptance rate:   {:.1}%", stats.acceptance_rate() * 100.0);
    eprintln!("  Elig rejections:   {:.1}%", stats.eligibility_rejection_rate() * 100.0);
    eprintln!("  Height distribution: {:?}", dist);
    eprintln!("  Nodes with exclusions: {}/{}", nodes_with_excl, n_nodes);
    eprintln!("  Max exclusions on one node: {}", max_excl);

    // Phase 3: Backfill — nodes that fell behind request missing blocks
    // First try WITH eligibility check (the buggy path)
    let (backfilled_elig, rejected_elig) = net.backfill_with_eligibility().await;
    let dist_after_elig = net.height_distribution().await;
    let synced_after_elig = net.is_synced().await;

    eprintln!();
    eprintln!("  === After backfill WITH eligibility check ===");
    eprintln!("  Backfilled: {}, Rejected: {}", backfilled_elig, rejected_elig);
    eprintln!("  Height distribution: {:?}", dist_after_elig);
    eprintln!("  All synced: {}", synced_after_elig);

    // DIAGNOSTIC: check what node 5 has in block_store
    {
        let n5 = net.nodes[5].lock().await;
        let h5 = n5.chain_state.read().await.best_height;
        let hash5 = n5.chain_state.read().await.best_hash;

        // Check if node 5 and leader share blocks at various heights
        let leader = net.nodes[0].lock().await;
        let mut common_h = 0u64;
        for h in (1..=std::cmp::min(h5, 145)).rev() {
            let lb = leader.block_store.get_block_by_height(h).ok().flatten();
            let nb = n5.block_store.get_block_by_height(h).ok().flatten();
            match (lb, nb) {
                (Some(lb), Some(nb)) if lb.hash() == nb.hash() => {
                    common_h = h;
                    break;
                }
                (Some(_), Some(_)) => {
                    eprintln!("  Height {}: DIFFERENT hashes (fork)", h);
                }
                (Some(_), None) => {}
                (None, Some(_)) => {
                    eprintln!("  Height {}: node5 has block but leader doesn't?", h);
                }
                (None, None) => {}
            }
        }
        eprintln!();
        eprintln!("  === DIAGNOSTIC: Node 5 vs Leader ===");
        eprintln!("  Node 5 height: {}, hash: {:.16}", h5, hash5.to_string());
        eprintln!("  Common ancestor: h={}", common_h);
        eprintln!("  Blocks to rollback: {}", h5 - common_h);
        eprintln!("  Blocks to apply: {}", 145 - common_h);
        drop(leader);
        drop(n5);
    }

    // Phase 4: Full sync simulation (find ancestor, rollback, apply leader's chain)
    let (synced_count, total_rollbacks, total_sync_applied) = net.sync_from_leader().await;
    let synced_after_sync = net.is_synced().await;
    let final_height = net.height(0).await;
    let final_dist = net.height_distribution().await;

    eprintln!();
    eprintln!("  === After sync simulation ===");
    eprintln!("  Nodes synced: {}/{}", synced_count, n_nodes - 1);
    eprintln!("  Total rollbacks: {}", total_rollbacks);
    eprintln!("  Total blocks applied: {}", total_sync_applied);
    eprintln!("  All synced: {}", synced_after_sync);
    eprintln!("  Height distribution: {:?}", final_dist);
    eprintln!("  Final height: {}", final_height);

    assert!(
        synced_after_sync,
        "Network must converge after sync simulation. \
         {} of {} nodes synced. Distribution: {:?}",
        synced_count, n_nodes - 1, final_dist
    );
    assert_eq!(final_height, 145); // 45 base + 100 gossip
}

/// Scale test: 100 nodes, 20 producers, 200 blocks, 95% delivery
#[tokio::test]
#[ignore] // ~30s, run with --test-threads=1
async fn test_realistic_gossip_100_nodes_200_blocks() {
    let n_nodes = 100;
    let n_producers = 20;
    let delivery_rate = 0.95;
    let total_blocks = 200;

    let net = TestNetwork::new(n_nodes, n_producers).await;
    let genesis = net.genesis_hash;

    // Exit genesis period
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }

    // Produce with realistic gossip
    let mut prev = base[44].hash();
    let mut total_rejected_elig = 0;

    // Build sorted producer list for round-robin (same order as validation)
    let mut rr_producers: Vec<(usize, PublicKey)> = net
        .producers
        .iter()
        .enumerate()
        .map(|(i, kp)| (i, *kp.public_key()))
        .collect();
    rr_producers.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));

    for i in 0..total_blocks {
        let height = 46 + i as u64;
        let slot = 46 + i as u32;
        // Use the CORRECT producer for this slot (round-robin)
        let rr_idx = (slot as usize) % rr_producers.len();
        let (orig_idx, _) = rr_producers[rr_idx];
        let block = net.build_block(height, slot, prev, &net.producers[orig_idx]);
        prev = block.hash();

        net.apply_to_node(0, block.clone()).await.ok();
        let round = net.gossip_realistic(&block, delivery_rate).await;
        total_rejected_elig += round.rejected_eligibility;
    }

    let dist = net.height_distribution().await;
    let (nodes_with_excl, max_excl, _) = net.count_divergent_exclusions().await;

    eprintln!();
    eprintln!("  === Scale Gossip ({} nodes, {} producers, {} blocks, {:.0}% delivery) ===",
        n_nodes, n_producers, total_blocks, delivery_rate * 100.0);
    eprintln!("  Eligibility rejections during gossip: {}", total_rejected_elig);
    eprintln!("  Height distribution: {:?}", dist);
    eprintln!("  Nodes with exclusions: {}/{}", nodes_with_excl, n_nodes);
    eprintln!("  Max exclusions: {}", max_excl);

    // Full sync simulation
    let (synced_count, rollbacks, applied) = net.sync_from_leader().await;
    let synced = net.is_synced().await;
    let final_height = net.height(0).await;

    eprintln!("  Synced: {}/{}, Rollbacks: {}, Applied: {}, Height: {}",
        synced_count, n_nodes - 1, rollbacks, applied, final_height);

    assert!(synced, "100 nodes should converge after sync");
    assert_eq!(final_height, 245);
}

/// 500 nodes, 50 producers, 100 blocks, 95% delivery
#[tokio::test]
#[ignore]
async fn test_realistic_gossip_500_nodes() {
    let n_nodes = 500;
    let n_producers = 50;
    let delivery_rate = 0.95;
    let total_blocks = 100;

    let start = std::time::Instant::now();
    let net = TestNetwork::new(n_nodes, n_producers).await;
    let init_time = start.elapsed();

    let genesis = net.genesis_hash;
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }

    let mut rr_producers: Vec<(usize, PublicKey)> = net.producers.iter().enumerate()
        .map(|(i, kp)| (i, *kp.public_key())).collect();
    rr_producers.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));

    let gossip_start = std::time::Instant::now();
    let mut prev = base[44].hash();
    let mut total_elig_reject = 0;
    for i in 0..total_blocks {
        let height = 46 + i as u64;
        let slot = 46 + i as u32;
        let rr_idx = (slot as usize) % rr_producers.len();
        let (orig_idx, _) = rr_producers[rr_idx];
        let block = net.build_block(height, slot, prev, &net.producers[orig_idx]);
        prev = block.hash();
        net.apply_to_node(0, block.clone()).await.ok();
        let round = net.gossip_realistic(&block, delivery_rate).await;
        total_elig_reject += round.rejected_eligibility;
    }
    let gossip_time = gossip_start.elapsed();

    let sync_start = std::time::Instant::now();
    let (synced_count, rollbacks, applied) = net.sync_from_leader().await;
    let sync_time = sync_start.elapsed();
    let synced = net.is_synced().await;
    let final_height = net.height(0).await;

    eprintln!();
    eprintln!("  === {} nodes, {} producers, {} blocks, {:.0}% delivery ===",
        n_nodes, n_producers, total_blocks, delivery_rate * 100.0);
    eprintln!("  Init:          {:?}", init_time);
    eprintln!("  Gossip:        {:?}", gossip_time);
    eprintln!("  Sync:          {:?}", sync_time);
    eprintln!("  Total:         {:?}", start.elapsed());
    eprintln!("  Elig rejects:  {}", total_elig_reject);
    eprintln!("  Synced:        {}/{}", synced_count, n_nodes - 1);
    eprintln!("  Rollbacks:     {}", rollbacks);
    eprintln!("  Applied:       {}", applied);
    eprintln!("  Final height:  {}", final_height);

    assert!(synced, "{} nodes should converge", n_nodes);
    assert_eq!(final_height, 145);
}

/// 1000 nodes, 100 producers, 50 blocks, 95% delivery
#[tokio::test]
#[ignore]
async fn test_realistic_gossip_1000_nodes() {
    let n_nodes = 1000;
    let n_producers = 100;
    let delivery_rate = 0.95;
    let total_blocks = 50;

    let start = std::time::Instant::now();
    let net = TestNetwork::new(n_nodes, n_producers).await;
    let init_time = start.elapsed();

    let genesis = net.genesis_hash;
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }

    let mut rr_producers: Vec<(usize, PublicKey)> = net.producers.iter().enumerate()
        .map(|(i, kp)| (i, *kp.public_key())).collect();
    rr_producers.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));

    let gossip_start = std::time::Instant::now();
    let mut prev = base[44].hash();
    let mut total_elig_reject = 0;
    for i in 0..total_blocks {
        let height = 46 + i as u64;
        let slot = 46 + i as u32;
        let rr_idx = (slot as usize) % rr_producers.len();
        let (orig_idx, _) = rr_producers[rr_idx];
        let block = net.build_block(height, slot, prev, &net.producers[orig_idx]);
        prev = block.hash();
        net.apply_to_node(0, block.clone()).await.ok();
        let round = net.gossip_realistic(&block, delivery_rate).await;
        total_elig_reject += round.rejected_eligibility;
    }
    let gossip_time = gossip_start.elapsed();

    let sync_start = std::time::Instant::now();
    let (synced_count, rollbacks, applied) = net.sync_from_leader().await;
    let sync_time = sync_start.elapsed();
    let synced = net.is_synced().await;
    let final_height = net.height(0).await;

    eprintln!();
    eprintln!("  === {} nodes, {} producers, {} blocks, {:.0}% delivery ===",
        n_nodes, n_producers, total_blocks, delivery_rate * 100.0);
    eprintln!("  Init:          {:?}", init_time);
    eprintln!("  Gossip:        {:?}", gossip_time);
    eprintln!("  Sync:          {:?}", sync_time);
    eprintln!("  Total:         {:?}", start.elapsed());
    eprintln!("  Elig rejects:  {}", total_elig_reject);
    eprintln!("  Synced:        {}/{}", synced_count, n_nodes - 1);
    eprintln!("  Rollbacks:     {}", rollbacks);
    eprintln!("  Applied:       {}", applied);
    eprintln!("  Final height:  {}", final_height);

    assert!(synced, "{} nodes should converge", n_nodes);
    assert_eq!(final_height, 95);
}
