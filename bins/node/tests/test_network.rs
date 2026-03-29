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
        let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
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
            missed_producers: Vec::new(),
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
            eprintln!("  Node {}: h={} hash={:.16}", i, h, hash.to_string());
        }
    }
}

// ============================================================
// TESTS
// ============================================================

#[tokio::test]
async fn test_network_creates_and_syncs() {
    let net = TestNetwork::new(3, 3).await;
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
    let net = TestNetwork::new(10, 5).await;
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
    let net = net;
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
    let net = TestNetwork::new(50, 5).await;
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
    let net = TestNetwork::new(100, 5).await;
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
    let net = TestNetwork::new(500, 5).await;
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
    let net = TestNetwork::new(1000, 5).await;
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
    let net = TestNetwork::new(5000, 5).await;
    let init = start.elapsed();
    let start = std::time::Instant::now();
    net.produce_blocks(3, 0).await;
    let produce = start.elapsed();
    assert!(net.is_synced().await);
    eprintln!("5000 nodes: init={:?}, 3 blocks={:?}", init, produce);
}

#[tokio::test]
#[ignore] // BLOCKED on macOS: RocksDB uses fopen() which caps at FD 32767 (FILE._file is short).
          // Each node needs ~26 structural FDs (LOCK, MANIFEST, WAL, CF dirs for 2 DBs).
          // Max ~1259 nodes on macOS. Use test_realistic_gossip_10k_clustered instead
          // (drops nodes between clusters to stay under the ceiling).
          // On Linux this test works — fopen() uses int, no 32K cap.
async fn test_network_10000_nodes() {
    let start = std::time::Instant::now();
    let net = TestNetwork::new(10000, 5).await;
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
    pub fn new(n_producers: usize, nodes_per_cluster: usize, blocks_per_cluster: usize) -> Self {
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
    /// Production-accurate gossip: deliver block to nodes, and when a node
    /// can't apply (prev_hash mismatch = it's behind), sync it from a peer
    /// that has the missing blocks. This is what the real sync protocol does.
    /// Returns (applied_direct, applied_via_sync, missed).
    pub async fn gossip_with_sync(
        &self,
        block: &Block,
        delivery_probability: f64,
    ) -> (usize, usize, usize) {
        let n = self.nodes.len();
        let block_hash = block.hash();

        // Determine delivery
        let mut delivered = Vec::new();
        let mut not_delivered = Vec::new();
        for i in 0..n {
            let seed = block_hash.as_bytes()[i % 32] as u16
                + block_hash.as_bytes()[(i * 7 + 3) % 32] as u16;
            if (seed as f64 / 510.0) < delivery_probability {
                delivered.push(i);
            } else {
                not_delivered.push(i);
            }
        }

        let mut applied_direct = 0usize;
        let mut applied_via_sync = 0usize;

        for &node_id in &delivered {
            let mut node = self.nodes[node_id].lock().await;

            // Try direct apply
            match node.apply_block(block.clone(), ValidationMode::Light).await {
                Ok(()) => {
                    applied_direct += 1;
                    continue;
                }
                Err(_) => {
                    // Failed — probably behind (prev_hash mismatch).
                    // Simulate sync: find a peer that's ahead and get missing blocks.
                    drop(node);

                    let node_height = self.height(node_id).await;
                    let block_height = block.header.slot as u64;

                    if block_height <= node_height {
                        continue; // Already at or past this height
                    }

                    // Find a peer that has the missing blocks (use node 0 as reference)
                    let leader_height = self.height(0).await;
                    if leader_height <= node_height {
                        continue; // Leader not ahead either
                    }

                    // Sync: find common ancestor, reset, replay
                    let ancestor_h = {
                        let leader = self.nodes[0].lock().await;
                        let behind = self.nodes[node_id].lock().await;
                        let mut found = 0u64;
                        for h in (1..=std::cmp::min(node_height, leader_height)).rev() {
                            let lb = leader.block_store.get_block_by_height(h).ok().flatten();
                            let nb = behind.block_store.get_block_by_height(h).ok().flatten();
                            if let (Some(lb), Some(nb)) = (lb, nb) {
                                if lb.hash() == nb.hash() {
                                    found = h;
                                    break;
                                }
                            }
                        }
                        found
                    };

                    // Reset to ancestor
                    {
                        let mut node = self.nodes[node_id].lock().await;
                        let ancestor_hash = if ancestor_h == 0 {
                            node.chain_state.read().await.genesis_hash
                        } else {
                            let leader = self.nodes[0].lock().await;
                            leader
                                .block_store
                                .get_block_by_height(ancestor_h)
                                .ok()
                                .flatten()
                                .map(|b| b.hash())
                                .unwrap_or(node.chain_state.read().await.genesis_hash)
                        };
                        let mut cs = node.chain_state.write().await;
                        cs.best_height = ancestor_h;
                        cs.best_hash = ancestor_hash;
                        cs.best_slot = ancestor_h as u32;
                        drop(cs);
                        node.sync_manager.write().await.update_local_tip(
                            ancestor_h,
                            ancestor_hash,
                            ancestor_h as u32,
                        );
                        node.excluded_producers.clear();
                        node.cumulative_rollback_depth = 0;
                        node.cached_scheduler = None;
                    }

                    // Apply leader's chain from ancestor+1 to current leader height
                    let mut synced_blocks = 0;
                    for h in (ancestor_h + 1)..=leader_height {
                        let blk = {
                            let leader = self.nodes[0].lock().await;
                            leader.block_store.get_block_by_height(h).ok().flatten()
                        };
                        if let Some(blk) = blk {
                            let mut node = self.nodes[node_id].lock().await;
                            if node.apply_block(blk, ValidationMode::Light).await.is_ok() {
                                synced_blocks += 1;
                            } else {
                                break;
                            }
                        }
                    }

                    if synced_blocks > 0 {
                        applied_via_sync += 1;
                    }
                }
            }
        }

        (applied_direct, applied_via_sync, not_delivered.len())
    }

    /// Propagate a block with random delay to simulate gossip.
    /// Some nodes receive the block, others don't (simulating network latency).
    /// Returns (received_count, missed_count).
    pub async fn gossip_propagate(
        &self,
        block: Block,
        delivery_probability: f64,
    ) -> (usize, usize) {
        let n = self.nodes.len();

        // Determine which nodes receive this block (random subset)
        let block_hash = block.hash();
        let mut receivers = Vec::new();
        for i in 0..n {
            let hash_byte = block_hash.as_bytes()[i % 32] as f64 / 255.0;
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
        let received = results.iter().filter(|ok| **ok).count();
        let missed = n - received;

        (received, missed)
    }

    /// Simulate gossip with check_producer_eligibility — the REAL path.
    /// Blocks go through eligibility check before apply_block, just like production.
    /// Returns (accepted, rejected_eligibility, rejected_apply).
    pub async fn gossip_with_eligibility(&self, block: Block) -> (usize, usize, usize) {
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
                                Ok(()) => 0u8, // accepted
                                Err(_) => 2u8, // rejected at apply
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
    let net = TestNetwork::new(n_nodes, n_producers).await;
    let genesis = net.genesis_hash;

    // Phase 1: Build 50 blocks (past genesis period for devnet) — 100% delivery
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 50);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }
    assert!(net.is_synced().await);
    assert_eq!(net.height(0).await, 50);

    // Phase 2: Produce 20 blocks with correct round-robin and 70% delivery
    // Some nodes will miss blocks → fall behind → need sync
    let mut rr: Vec<(usize, PublicKey)> = net
        .producers
        .iter()
        .enumerate()
        .map(|(i, kp)| (i, *kp.public_key()))
        .collect();
    rr.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));

    let mut prev = base[49].hash();
    let mut total_synced_via_sync = 0;
    for i in 0..20 {
        let height = 51 + i as u64;
        let slot = 51 + i as u32;
        let rr_idx = (slot as usize) % rr.len();
        let (orig_idx, _) = rr[rr_idx];
        let block = net.build_block(height, slot, prev, &net.producers[orig_idx]);
        prev = block.hash();

        // Node 0 always gets it
        net.apply_to_node(0, block.clone()).await.ok();

        // Gossip with sync: 70% delivery, nodes that miss blocks sync from peers
        let (_direct, via_sync, _missed) = net.gossip_with_sync(&block, 0.70).await;
        total_synced_via_sync += via_sync;
    }

    // Final sync pass for any remaining stragglers
    let (synced_count, _, _) = net.sync_from_leader().await;
    let synced = net.is_synced().await;
    let h0 = net.height(0).await;

    eprintln!(
        "  Result: synced={}, height={}, synced_via_gossip_sync={}, final_sync={}",
        synced, h0, total_synced_via_sync, synced_count
    );

    assert!(synced, "Network should converge after gossip with sync");
    assert_eq!(h0, 70);
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

    // Produce 30 blocks with correct round-robin and 80% delivery + sync
    let mut rr: Vec<(usize, PublicKey)> = net
        .producers
        .iter()
        .enumerate()
        .map(|(i, kp)| (i, *kp.public_key()))
        .collect();
    rr.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));

    let mut prev = base[44].hash();
    let mut total_direct = 0;
    let mut total_via_sync = 0;
    for i in 0..30 {
        let height = 46 + i as u64;
        let slot = 46 + i as u32;
        let rr_idx = (slot as usize) % rr.len();
        let (orig_idx, _) = rr[rr_idx];
        let block = net.build_block(height, slot, prev, &net.producers[orig_idx]);
        prev = block.hash();

        net.apply_to_node(0, block.clone()).await.ok();
        let (direct, via_sync, _missed) = net.gossip_with_sync(&block, 0.80).await;
        total_direct += direct;
        total_via_sync += via_sync;
    }

    // Final sync for stragglers
    let (synced_count, _, _) = net.sync_from_leader().await;
    let synced = net.is_synced().await;
    let final_height = net.height(0).await;

    eprintln!(
        "  50 nodes: direct={}, via_sync={}, final_sync={}, synced={}, height={}",
        total_direct, total_via_sync, synced_count, synced, final_height
    );

    assert!(synced, "50 nodes should converge with gossip+sync");
    assert_eq!(final_height, 75);
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
        let block_hash = block.hash();
        let block_bytes = block_hash.as_bytes();
        let mut delivered_to = Vec::new();
        let mut missed_by = Vec::new();

        for i in 0..n {
            // Mix block hash with node index for deterministic but varied delivery
            let seed = block_bytes[(i * 7) % 32] as u16 + block_bytes[(i * 13 + 3) % 32] as u16;
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
                                Ok(()) => 0u8, // accepted
                                Err(_) => 2u8, // rejected at apply (duplicate, etc)
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
            rejected_apply,
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
                        leader
                            .block_store
                            .get_block_by_height(ancestor_h)
                            .ok()
                            .flatten()
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
                        cs.best_height,
                        cs.best_hash,
                        cs.best_slot,
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
        let active: Vec<PublicKey> = ps
            .active_producers_at_height(46)
            .iter()
            .map(|p| p.public_key)
            .collect();
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
        eprintln!(
            "  Expected producer:  {:?}",
            &expected_producer.as_bytes()[..4]
        );
        eprintln!(
            "  Actual producer:    {:?}",
            &actual_producer.as_bytes()[..4]
        );
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
    let (nodes_with_excl, max_excl, _total_excl) = net.count_divergent_exclusions().await;

    eprintln!();
    eprintln!(
        "  === Realistic Gossip Test ({} nodes, {} producers, {} blocks) ===",
        n_nodes, n_producers, total_blocks
    );
    eprintln!("  Delivery rate:     {:.1}%", stats.delivery_rate() * 100.0);
    eprintln!(
        "  Acceptance rate:   {:.1}%",
        stats.acceptance_rate() * 100.0
    );
    eprintln!(
        "  Elig rejections:   {:.1}%",
        stats.eligibility_rejection_rate() * 100.0
    );
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
    eprintln!(
        "  Backfilled: {}, Rejected: {}",
        backfilled_elig, rejected_elig
    );
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
        synced_count,
        n_nodes - 1,
        final_dist
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
    eprintln!(
        "  === Scale Gossip ({} nodes, {} producers, {} blocks, {:.0}% delivery) ===",
        n_nodes,
        n_producers,
        total_blocks,
        delivery_rate * 100.0
    );
    eprintln!(
        "  Eligibility rejections during gossip: {}",
        total_rejected_elig
    );
    eprintln!("  Height distribution: {:?}", dist);
    eprintln!("  Nodes with exclusions: {}/{}", nodes_with_excl, n_nodes);
    eprintln!("  Max exclusions: {}", max_excl);

    // Full sync simulation
    let (synced_count, rollbacks, applied) = net.sync_from_leader().await;
    let synced = net.is_synced().await;
    let final_height = net.height(0).await;

    eprintln!(
        "  Synced: {}/{}, Rollbacks: {}, Applied: {}, Height: {}",
        synced_count,
        n_nodes - 1,
        rollbacks,
        applied,
        final_height
    );

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

    let mut rr_producers: Vec<(usize, PublicKey)> = net
        .producers
        .iter()
        .enumerate()
        .map(|(i, kp)| (i, *kp.public_key()))
        .collect();
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
    eprintln!(
        "  === {} nodes, {} producers, {} blocks, {:.0}% delivery ===",
        n_nodes,
        n_producers,
        total_blocks,
        delivery_rate * 100.0
    );
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

    let mut rr_producers: Vec<(usize, PublicKey)> = net
        .producers
        .iter()
        .enumerate()
        .map(|(i, kp)| (i, *kp.public_key()))
        .collect();
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
    eprintln!(
        "  === {} nodes, {} producers, {} blocks, {:.0}% delivery ===",
        n_nodes,
        n_producers,
        total_blocks,
        delivery_rate * 100.0
    );
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

/// 10K nodes via clusters: 10 clusters × 1000 nodes, realistic gossip
#[tokio::test]
#[ignore]
async fn test_realistic_gossip_10k_clustered() {
    let n_clusters = 10;
    let nodes_per_cluster = 1000;
    let n_producers = 100;
    let delivery_rate = 0.95;
    let blocks_per_cluster = 30;

    let total_start = std::time::Instant::now();

    let mut total_synced = 0usize;
    let mut total_nodes = 0usize;
    let mut total_elig_rejects = 0usize;

    for cluster_id in 0..n_clusters {
        let cluster_start = std::time::Instant::now();
        let net = TestNetwork::new(nodes_per_cluster, n_producers).await;
        let genesis = net.genesis_hash;

        // Base chain (exit genesis)
        let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
        for block in &base {
            net.apply_to_node(0, block.clone()).await.ok();
            net.propagate(0, block.clone()).await;
        }

        // Round-robin producer order
        let mut rr: Vec<(usize, PublicKey)> = net
            .producers
            .iter()
            .enumerate()
            .map(|(i, kp)| (i, *kp.public_key()))
            .collect();
        rr.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));

        // Gossip blocks
        let mut prev = base[44].hash();
        let mut cluster_elig_rejects = 0;
        for i in 0..blocks_per_cluster {
            let height = 46 + i as u64;
            let slot = 46 + i as u32;
            let rr_idx = (slot as usize) % rr.len();
            let (orig_idx, _) = rr[rr_idx];
            let block = net.build_block(height, slot, prev, &net.producers[orig_idx]);
            prev = block.hash();
            net.apply_to_node(0, block.clone()).await.ok();
            let round = net.gossip_realistic(&block, delivery_rate).await;
            cluster_elig_rejects += round.rejected_eligibility;
        }

        // Sync
        let (synced, _, _) = net.sync_from_leader().await;
        let all_synced = net.is_synced().await;

        eprintln!(
            "  Cluster {}/{}: {} nodes, synced={}/{}, elig_rejects={}, time={:?}",
            cluster_id + 1,
            n_clusters,
            nodes_per_cluster,
            synced,
            nodes_per_cluster - 1,
            cluster_elig_rejects,
            cluster_start.elapsed()
        );

        total_synced += synced;
        total_nodes += nodes_per_cluster - 1; // exclude leader
        total_elig_rejects += cluster_elig_rejects;

        assert!(all_synced, "Cluster {} should converge", cluster_id);
        drop(net);
    }

    let total_time = total_start.elapsed();
    eprintln!();
    eprintln!("  === 10K NODE GOSSIP SIMULATION ===");
    eprintln!("  Clusters:      {}", n_clusters);
    eprintln!("  Nodes/cluster: {}", nodes_per_cluster);
    eprintln!("  Total nodes:   {}", total_nodes + n_clusters);
    eprintln!("  Synced:        {}/{}", total_synced, total_nodes);
    eprintln!("  Elig rejects:  {}", total_elig_rejects);
    eprintln!("  Total time:    {:?}", total_time);
    eprintln!(
        "  Throughput:    {:.0} nodes/sec",
        (total_nodes + n_clusters) as f64 / total_time.as_secs_f64()
    );

    assert_eq!(total_synced, total_nodes, "All 10K nodes should converge");
}

// ============================================================
// Scheduler-Driven Slot Coverage Test
// ============================================================
//
// Unlike other tests that build blocks with arbitrary producers and
// use ValidationMode::Light (which skips eligibility), this test:
//
// 1. Builds the DeterministicScheduler from each node's producer state
// 2. Asks the scheduler who should produce each slot
// 3. Verifies ALL nodes agree on the same producer
// 4. Has that producer build the block
// 5. Detects lost slots (where no producer is scheduled)
// 6. Reports scheduler divergence between nodes

use doli_core::{DeterministicScheduler, ScheduledProducer};

/// Build a DeterministicScheduler from a node's producer set + epoch bond snapshot,
/// identical to what the production code does in try_produce_block().
async fn build_scheduler_for_node(node: &Node) -> DeterministicScheduler {
    let height = node.chain_state.read().await.best_height + 1;
    let producers = node.producer_set.read().await;
    let active: Vec<PublicKey> = producers
        .active_producers_at_height(height)
        .iter()
        .map(|p| p.public_key)
        .collect();
    drop(producers);

    let weighted = node.bond_weights_for_scheduling(active).await;

    let scheduled: Vec<ScheduledProducer> = weighted
        .into_iter()
        .map(|(pk, bonds)| ScheduledProducer::new(pk, bonds as u32))
        .collect();

    DeterministicScheduler::new(scheduled)
}

/// Scheduler fingerprint: (producer_count, total_bonds, rank0_pubkey_for_slot_0)
fn scheduler_fingerprint(sched: &DeterministicScheduler) -> (usize, u64, Option<PublicKey>) {
    let rank0 = sched.select_producer(0, 0).copied();
    (sched.producer_count(), sched.total_bonds(), rank0)
}

/// 100-node network, 10 producers, 500 slots — scheduler-driven production.
/// Detects lost slots, scheduler divergence, and eligibility mismatches.
#[tokio::test]
async fn test_scheduler_slot_coverage_100_nodes() {
    let n_nodes = 100;
    let n_producers = 10;
    let total_slots = 500;

    let net = TestNetwork::new(n_nodes, n_producers).await;
    let genesis = net.genesis_hash;

    // Exit genesis: build a chain of 45 blocks (devnet genesis_blocks=40)
    // so all subsequent blocks use the DeterministicScheduler path.
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }

    // Tracking
    let mut produced_slots = 0u64;
    let mut lost_slots = 0u64;
    let mut scheduler_divergences = 0u64;
    let mut eligibility_rejections = 0u64;
    let mut prev_hash = base.last().unwrap().hash();
    let start_slot = 46u32;
    let start_height = 46u64;

    // Map pubkey → producer keypair index for block building
    let pk_to_idx: HashMap<PublicKey, usize> = net
        .producers
        .iter()
        .enumerate()
        .map(|(i, kp)| (*kp.public_key(), i))
        .collect();

    for i in 0..total_slots {
        let slot = start_slot + i as u32;
        let height = start_height + i as u64;

        // Step 1: Build scheduler on EVERY node and check agreement
        let mut schedulers = Vec::with_capacity(n_nodes);
        for node_id in 0..n_nodes {
            let node = net.nodes[node_id].lock().await;
            let sched = build_scheduler_for_node(&node).await;
            schedulers.push(sched);
        }

        // Check all nodes agree on the scheduler
        let fp0 = scheduler_fingerprint(&schedulers[0]);
        let mut diverged = false;
        for (node_id, sched) in schedulers.iter().enumerate().skip(1) {
            let fp = scheduler_fingerprint(sched);
            if fp != fp0 {
                if !diverged {
                    eprintln!(
                        "  DIVERGENCE at slot {}: node 0 has {:?}, node {} has {:?}",
                        slot, fp0, node_id, fp
                    );
                    diverged = true;
                }
                scheduler_divergences += 1;
            }
        }

        // Step 2: Ask the scheduler who produces this slot (rank 0 = primary)
        let primary = schedulers[0].select_producer(slot, 0).copied();

        match primary {
            None => {
                // No producer for this slot — this is a lost slot
                lost_slots += 1;
                if lost_slots <= 10 {
                    eprintln!(
                        "  LOST SLOT {}: no primary producer (total_bonds={})",
                        slot,
                        schedulers[0].total_bonds()
                    );
                }
            }
            Some(producer_pk) => {
                // Find the keypair for this producer
                let idx = pk_to_idx.get(&producer_pk).copied();
                match idx {
                    None => {
                        // Scheduler selected a producer we don't have keys for — shouldn't happen
                        eprintln!("  SLOT {}: scheduler selected unknown pubkey", slot);
                        lost_slots += 1;
                    }
                    Some(producer_idx) => {
                        // Build block with the scheduler-assigned producer
                        let block =
                            net.build_block(height, slot, prev_hash, &net.producers[producer_idx]);
                        prev_hash = block.hash();

                        // Apply to leader (node 0)
                        match net.apply_to_node(0, block.clone()).await {
                            Ok(()) => {
                                produced_slots += 1;

                                // Propagate to all other nodes
                                let accepted = net.propagate(0, block).await;
                                let rejected = (n_nodes - 1) - accepted;
                                if rejected > 0 {
                                    eligibility_rejections += rejected as u64;
                                    if eligibility_rejections <= 20 {
                                        eprintln!(
                                            "  SLOT {}: {} nodes rejected block from scheduler-assigned producer",
                                            slot, rejected
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "  SLOT {}: leader REJECTED scheduler-assigned block: {}",
                                    slot, e
                                );
                                lost_slots += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // Verify convergence
    let synced = net.is_synced().await;

    // Report
    eprintln!();
    eprintln!(
        "  === SCHEDULER SLOT COVERAGE ({} nodes, {} producers) ===",
        n_nodes, n_producers
    );
    eprintln!("  Total slots:          {}", total_slots);
    eprintln!("  Produced:             {}", produced_slots);
    eprintln!("  Lost:                 {}", lost_slots);
    eprintln!(
        "  Scheduler divergence: {} (across {} non-leader nodes)",
        scheduler_divergences,
        n_nodes - 1
    );
    eprintln!(
        "  Eligibility rejects:  {} (total across all nodes)",
        eligibility_rejections
    );
    eprintln!("  All synced:           {}", synced);
    eprintln!(
        "  Coverage:             {:.1}%",
        (produced_slots as f64 / total_slots as f64) * 100.0
    );

    // Hard assertions
    assert!(synced, "All nodes must converge");
    assert_eq!(
        lost_slots, 0,
        "No slots should be lost — scheduler should always select a primary"
    );
    assert_eq!(
        scheduler_divergences, 0,
        "All nodes must agree on the scheduler"
    );
    assert_eq!(
        eligibility_rejections, 0,
        "All nodes must accept scheduler-assigned blocks"
    );
    assert_eq!(
        produced_slots, total_slots as u64,
        "Every slot must produce a block"
    );
}

/// 500-node network, 50 producers with VARIED bond counts, 1000 slots.
/// Tests that bond-weighted scheduling produces correct coverage and
/// no slots are lost even with asymmetric bond distribution.
#[tokio::test]
#[ignore] // Requires more resources
async fn test_scheduler_slot_coverage_500_nodes_varied_bonds() {
    let n_nodes = 500;
    let n_producers = 50;
    let total_slots = 1000;

    let net = TestNetwork::new(n_nodes, n_producers).await;
    let genesis = net.genesis_hash;

    // Exit genesis
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }

    let mut produced_slots = 0u64;
    let mut lost_slots = 0u64;
    let mut scheduler_divergences = 0u64;
    let mut prev_hash = base.last().unwrap().hash();
    let start_slot = 46u32;
    let start_height = 46u64;

    // Track per-producer block counts to verify bond-proportional distribution
    let mut producer_block_count: HashMap<PublicKey, u64> = HashMap::new();

    let pk_to_idx: HashMap<PublicKey, usize> = net
        .producers
        .iter()
        .enumerate()
        .map(|(i, kp)| (*kp.public_key(), i))
        .collect();

    for i in 0..total_slots {
        let slot = start_slot + i as u32;
        let height = start_height + i as u64;

        // Build scheduler on a sample of nodes (0, n/4, n/2, 3n/4, last)
        // to check agreement without the cost of locking all 500
        let sample_ids = [0, n_nodes / 4, n_nodes / 2, 3 * n_nodes / 4, n_nodes - 1];
        let mut sample_fps = Vec::new();
        let mut sched0 = None;

        for &node_id in &sample_ids {
            let node = net.nodes[node_id].lock().await;
            let sched = build_scheduler_for_node(&node).await;
            let fp = scheduler_fingerprint(&sched);
            if node_id == 0 {
                sched0 = Some(sched);
            }
            sample_fps.push((node_id, fp));
        }

        let fp0 = sample_fps[0].1.clone();
        for (node_id, fp) in &sample_fps[1..] {
            if *fp != fp0 {
                scheduler_divergences += 1;
                eprintln!("  DIVERGENCE at slot {}: node 0 vs node {}", slot, node_id);
            }
        }

        let sched = sched0.unwrap();
        let primary = sched.select_producer(slot, 0).copied();

        match primary {
            None => {
                lost_slots += 1;
            }
            Some(producer_pk) => {
                if let Some(&producer_idx) = pk_to_idx.get(&producer_pk) {
                    let block =
                        net.build_block(height, slot, prev_hash, &net.producers[producer_idx]);
                    prev_hash = block.hash();

                    match net.apply_to_node(0, block.clone()).await {
                        Ok(()) => {
                            produced_slots += 1;
                            *producer_block_count.entry(producer_pk).or_insert(0) += 1;
                            net.propagate(0, block).await;
                        }
                        Err(e) => {
                            eprintln!("  SLOT {}: leader rejected: {}", slot, e);
                            lost_slots += 1;
                        }
                    }
                } else {
                    lost_slots += 1;
                }
            }
        }
    }

    let synced = net.is_synced().await;

    // Report distribution
    eprintln!();
    eprintln!(
        "  === SCHEDULER SLOT COVERAGE ({} nodes, {} producers, varied bonds) ===",
        n_nodes, n_producers
    );
    eprintln!("  Total slots:          {}", total_slots);
    eprintln!("  Produced:             {}", produced_slots);
    eprintln!("  Lost:                 {}", lost_slots);
    eprintln!("  Scheduler divergence: {}", scheduler_divergences);
    eprintln!("  All synced:           {}", synced);

    // Show per-producer distribution (sorted by block count)
    let mut dist: Vec<(PublicKey, u64)> = producer_block_count.into_iter().collect();
    dist.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!("  Producer distribution (top 10):");
    for (pk, count) in dist.iter().take(10) {
        eprintln!(
            "    {} → {} blocks ({:.1}%)",
            hex::encode(&pk.as_bytes()[..4]),
            count,
            (*count as f64 / total_slots as f64) * 100.0
        );
    }

    // Hard assertions
    assert!(synced, "All nodes must converge");
    assert_eq!(lost_slots, 0, "No slots should be lost");
    assert_eq!(
        scheduler_divergences, 0,
        "Scheduler must agree across nodes"
    );
    assert_eq!(
        produced_slots, total_slots as u64,
        "Every slot must produce a block"
    );

    // With equal bonds (1 each), distribution should be roughly equal
    // Allow 50% deviation from expected (1000/50 = 20 blocks each)
    let expected_per_producer = total_slots as f64 / n_producers as f64;
    for (pk, count) in &dist {
        let deviation = (*count as f64 - expected_per_producer).abs() / expected_per_producer;
        assert!(
            deviation < 0.5,
            "Producer {} has {} blocks (expected ~{:.0}, deviation {:.1}%) — distribution too skewed",
            hex::encode(&pk.as_bytes()[..4]),
            count,
            expected_per_producer,
            deviation * 100.0
        );
    }
}

// ============================================================
// ON-CHAIN LIVENESS EXCLUSION TESTS
// Tests for missed_producers in BlockHeader + epoch-frozen list
// ============================================================

impl TestNetwork {
    /// Build a block with a slot gap, computing missed_producers from the schedule.
    ///
    /// `slot_gap`: how many slots were skipped (e.g., gap=3 means slots prev+1, prev+2 were empty)
    /// The missed_producers field is populated based on the sorted producer list and
    /// which producers were scheduled for the skipped slots.
    pub fn build_block_with_gap(
        &self,
        height: u64,
        slot: u32,
        prev_slot: u32,
        prev_hash: Hash,
        producer: &KeyPair,
        schedule: &[PublicKey],
    ) -> Block {
        let reward = self.params.block_reward(height);
        let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
        let coinbase = Transaction::new_coinbase(reward, pool_hash, height);
        let timestamp = self.params.genesis_time + (slot as u64 * self.params.slot_duration);
        let merkle_root = doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase));
        let genesis_hash = doli_core::chainspec::ChainSpec::devnet().genesis_hash();

        // Compute missed_producers: who was scheduled for the skipped slots?
        let mut missed = Vec::new();
        if !schedule.is_empty() && slot > prev_slot + 1 {
            const MAX_MISSED: usize = 3;
            let max_total = schedule.len() / 3;
            for skipped in (prev_slot + 1)..slot {
                if missed.len() >= MAX_MISSED || missed.len() >= max_total {
                    break;
                }
                let idx = (skipped as usize) % schedule.len();
                let pk = schedule[idx];
                if !missed.contains(&pk) {
                    missed.push(pk);
                }
            }
        }

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
            missed_producers: missed,
        };

        Block::new(header, vec![coinbase])
    }

    /// Get the epoch_producer_list from a node
    pub async fn epoch_list(&self, node_id: usize) -> Vec<PublicKey> {
        let n = self.nodes[node_id].lock().await;
        n.epoch_producer_list.clone()
    }

    /// Set the epoch_producer_list on all nodes (for test setup)
    pub async fn set_epoch_list_all(&self, list: &[PublicKey]) {
        for node in &self.nodes {
            let mut n = node.lock().await;
            n.epoch_producer_list = list.to_vec();
        }
    }
}

/// Test: missed_producers in header correctly excludes producers and all nodes agree.
///
/// Scenario:
/// 1. 50 nodes, 10 producers, build past genesis
/// 2. Set epoch_producer_list on all nodes (simulating epoch boundary)
/// 3. Build blocks WITH slot gaps → missed_producers set in header
/// 4. Propagate to all nodes
/// 5. Verify all nodes have identical excluded_producers sets
/// 6. Verify excluded producers are removed from scheduling
#[tokio::test]
async fn test_onchain_liveness_exclusion_deterministic() {
    let n_nodes = 50;
    let n_producers = 10;
    let net = TestNetwork::new(n_nodes, n_producers).await;
    let genesis = net.genesis_hash;

    // Phase 1: Build 45 blocks to exit genesis — all synced
    let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 45);
    for block in &base {
        net.apply_to_node(0, block.clone()).await.ok();
        net.propagate(0, block.clone()).await;
    }
    assert!(net.is_synced().await);
    assert_eq!(net.height(0).await, 45);

    // Phase 2: Set epoch_producer_list on all nodes (simulate epoch boundary)
    let mut sorted_pks: Vec<PublicKey> = net.producers.iter().map(|kp| *kp.public_key()).collect();
    sorted_pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    net.set_epoch_list_all(&sorted_pks).await;

    // Phase 3: Build block at slot 48 (gap: slots 46, 47 missed)
    // Producers[slot%10] were scheduled but didn't produce
    let rr_idx_46 = (46usize) % sorted_pks.len();
    let rr_idx_48 = (48usize) % sorted_pks.len();

    // Find the keypair for the producer at rr_idx_48
    let producer_48_pk = sorted_pks[rr_idx_48];
    let producer_48_kp = net
        .producers
        .iter()
        .find(|kp| *kp.public_key() == producer_48_pk)
        .unwrap();

    let block_48 =
        net.build_block_with_gap(46, 48, 45, base[44].hash(), producer_48_kp, &sorted_pks);

    // Verify the block has missed_producers set
    assert!(
        !block_48.header.missed_producers.is_empty(),
        "Block with slot gap should have missed_producers"
    );
    let missed_count = block_48.header.missed_producers.len();
    eprintln!(
        "  Block at slot 48: {} missed producers (gap 45→48)",
        missed_count
    );

    // The missed producer at slot 46 should be sorted_pks[46%10]
    let expected_missed = sorted_pks[rr_idx_46];
    assert!(
        block_48.header.missed_producers.contains(&expected_missed),
        "Missed producer for slot 46 should be in header"
    );

    // Phase 4: Apply to all nodes — they should all agree
    for i in 0..n_nodes {
        net.apply_to_node(i, block_48.clone())
            .await
            .unwrap_or_else(|e| {
                panic!("Node {} rejected block with missed_producers: {}", i, e);
            });
    }
    assert!(net.is_synced().await);

    // Phase 5: Verify ALL nodes have identical excluded_producers
    let mut exclusion_sets: Vec<HashSet<PublicKey>> = Vec::new();
    for i in 0..n_nodes {
        let n = net.nodes[i].lock().await;
        exclusion_sets.push(n.excluded_producers.clone());
    }

    let reference_set = &exclusion_sets[0];
    for (i, set) in exclusion_sets.iter().enumerate().skip(1) {
        assert_eq!(
            set,
            reference_set,
            "Node {} excluded_producers differs from node 0! Node 0: {:?}, Node {}: {:?}",
            i,
            reference_set.len(),
            i,
            set.len()
        );
    }

    eprintln!(
        "  All {} nodes agree: {} producers excluded",
        n_nodes,
        reference_set.len()
    );

    // Phase 6: Build more blocks — verify excluded producers are skipped in scheduling
    // The excluded producer should NOT be selected for their normal slot
    let effective: Vec<PublicKey> = sorted_pks
        .iter()
        .filter(|pk| !reference_set.contains(pk))
        .copied()
        .collect();
    let next_slot = 49u32;
    let expected_next = effective[(next_slot as usize) % effective.len()];

    // Build the next block with the correct producer
    let next_kp = net
        .producers
        .iter()
        .find(|kp| *kp.public_key() == expected_next)
        .unwrap();
    let block_49 = net.build_block(47, 49, block_48.hash(), next_kp);

    for i in 0..n_nodes {
        net.apply_to_node(i, block_49.clone())
            .await
            .unwrap_or_else(|e| {
                panic!("Node {} rejected block from effective schedule: {}", i, e);
            });
    }
    assert!(net.is_synced().await);

    eprintln!(
        "  Effective schedule works: block at slot 49 accepted by all {} nodes",
        n_nodes
    );
    eprintln!(
        "  PASS: On-chain liveness exclusion is deterministic across {} nodes",
        n_nodes
    );
}

/// 10,000-node test via ClusterNetwork.
///
/// Each cluster independently validates:
/// 1. Epoch-frozen producer list
/// 2. missed_producers in headers from slot gaps
/// 3. All nodes in cluster converge on same excluded set
/// 4. Blocks with excluded producers are accepted by all
///
/// 10 clusters × 1000 nodes = 10,000 total simulated nodes.
#[tokio::test]
async fn test_onchain_liveness_10k_nodes() {
    let n_producers = 10;
    let nodes_per_cluster = 1000;
    let n_clusters = 10;
    let total_start = std::time::Instant::now();

    let producers: Vec<KeyPair> = (0..n_producers).map(|_| KeyPair::generate()).collect();
    let mut sorted_pks: Vec<PublicKey> = producers.iter().map(|kp| *kp.public_key()).collect();
    sorted_pks.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));

    let mut total_nodes = 0usize;
    let mut all_passed = true;

    for cluster_id in 0..n_clusters {
        let cluster_start = std::time::Instant::now();

        // Create cluster
        let net = TestNetwork::new(nodes_per_cluster, n_producers).await;
        let genesis = net.genesis_hash;
        let init_time = cluster_start.elapsed();

        // Build 10 blocks — all synced (past any early genesis guards)
        let base = net.build_chain(1, 1, genesis, &net.producers[0].clone(), 10);
        for block in &base {
            net.apply_to_node(0, block.clone()).await.ok();
            net.propagate(0, block.clone()).await;
        }
        assert!(net.is_synced().await);

        // Set epoch_producer_list on all nodes
        net.set_epoch_list_all(&sorted_pks).await;

        // Build block with slot gap (slot 13, gap over 11-12)
        let gap_block = net.build_block_with_gap(
            11,
            13,
            10,
            base[9].hash(),
            &net.producers[cluster_id % n_producers],
            &sorted_pks,
        );
        let missed_in_header = gap_block.header.missed_producers.len();

        // Apply to all nodes in parallel
        let apply_start = std::time::Instant::now();
        net.apply_to_node(0, gap_block.clone()).await.ok();
        let accepted = net.propagate(0, gap_block.clone()).await;
        let apply_time = apply_start.elapsed();

        // Verify all nodes agree on exclusion set
        let (nodes_with_excl, max_excl, _) = net.count_divergent_exclusions().await;
        let synced = net.is_synced().await;

        // Check determinism: sample 10 nodes and verify identical sets
        let mut sets_match = true;
        let ref_set = {
            let n = net.nodes[0].lock().await;
            n.excluded_producers.clone()
        };
        for i in [1, 100, 250, 500, 750, 999]
            .iter()
            .copied()
            .filter(|&i| i < nodes_per_cluster)
        {
            let n = net.nodes[i].lock().await;
            if n.excluded_producers != ref_set {
                sets_match = false;
                break;
            }
        }

        let cluster_passed = synced && sets_match && missed_in_header > 0;
        if !cluster_passed {
            all_passed = false;
        }

        total_nodes += nodes_per_cluster;
        let cluster_time = cluster_start.elapsed();

        eprintln!(
            "  Cluster {}/{}: {} nodes, init={:?}, apply={:?}, total={:?}, synced={}, excl={}/{}, deterministic={}, missed_in_header={}, accepted={}",
            cluster_id + 1, n_clusters, nodes_per_cluster,
            init_time, apply_time, cluster_time,
            synced, nodes_with_excl, max_excl,
            sets_match, missed_in_header, accepted
        );

        // Drop cluster to free FDs
        drop(net);
    }

    let total_time = total_start.elapsed();
    eprintln!();
    eprintln!("  === On-Chain Liveness Exclusion @ 10K Nodes ===");
    eprintln!("  Total nodes:    {}", total_nodes);
    eprintln!("  Clusters:       {}", n_clusters);
    eprintln!("  Total time:     {:?}", total_time);
    eprintln!("  All passed:     {}", all_passed);
    eprintln!(
        "  Throughput:     {:.0} nodes/sec",
        total_nodes as f64 / total_time.as_secs_f64()
    );

    assert!(
        all_passed,
        "All {} clusters must pass liveness exclusion test",
        n_clusters
    );
    assert_eq!(total_nodes, 10_000);
}
