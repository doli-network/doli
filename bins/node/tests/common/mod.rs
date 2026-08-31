//! TestNetwork — simulated P2P network of real DOLI nodes (shared test harness).
//!
//! Each node is a real Node::new_for_test() with real RocksDB, real ProducerSet,
//! real SyncManager. No mocks. Blocks propagate via direct apply_block() calls
//! simulating gossip.
//!
//! Start with an exact replica of what we have in production.
//! Find the ceiling. Then optimize.
//!
//! INC-I-198 — THIS FILE LIVES IN `tests/common/` ON PURPOSE. Cargo treats every
//! `tests/*.rs` file as its OWN test binary, but a file in a SUBDIRECTORY is only
//! a module. When the harness lived in `tests/test_network.rs` and
//! `tests/checkpoint_rotation.rs` pulled it in with `mod test_network;`, all 25
//! `test_network` tests were compiled into BOTH binaries and RUN TWICE — roughly
//! 150 s of duplicated work per suite run, for the sake of one
//! `TestNetwork::new(3, 3)` call. The harness now lives here and both binaries
//! declare `mod common;`, so the tests run exactly once, in their own binary.
//
// OUTPUT CONTRACT: N/A — test infrastructure (INC-I-065)

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

// ============================================================
// INC-I-198 — LIVE-NODE BUDGET
// ============================================================
//
// A test node costs exactly 28 file descriptors: 16 column families (9 in
// BlockStore, 7 in StateDb) plus ~6 per-DB files each. MEASURED on this
// harness — `test_network_10_nodes` peaks at 289 open fds and
// `test_network_100_nodes` at 2809, a slope of (2809-289)/(100-10) = 28.0 with
// an intercept of 9. That cost is structural: `max_open_files` is already 256
// on both DBs and is not binding, because these DBs are tiny and hold almost
// no SSTs.
//
// The ceiling is NOT a resource budget. On Darwin `struct __sFILE` stores the
// descriptor in a `short _file` (`/usr/include/_stdio.h:136`), so `fdopen()`
// fails for any fd NUMBER >= 32768 and sets EMFILE — whose `strerror` text,
// "Too many open files", describes a limit that is nowhere near reached.
// RocksDB hits this at `env/fs_posix.cc:203`, where `open()` has already
// SUCCEEDED and only the `fdopen()` wrapper fails. RLIMIT_NOFILE on the
// affected machine is 1048576 and `kern.maxfilesperproc` is 245760 — three
// orders of magnitude above the point of failure. Raising `ulimit -n`
// therefore CANNOT help, and the pre-existing `#[ignore]` note claiming this
// test "requires ulimit -n 65536" was wrong on that point.
//
// The defect this fixes is that the suite had NO BOUND on concurrent demand:
// peak fd use was the sum of whatever tests the harness happened to run in
// parallel. `test_onchain_liveness_10k_nodes` alone holds 1000 live nodes
// (28009 fds); beside the other twelve live tests on eight threads the binary
// peaked at fd number 32750 — seventeen short of the wall — and
// `test_cluster_10x100` was the test that happened to ask for the descriptor
// that did not exist.
//
// So the harness now meters itself. Every `TestNetwork` acquires one permit
// per node before it creates any, and releases them on drop. Nothing is
// skipped, serialized by name, or shrunk: a test that wants more nodes than
// the whole budget takes the whole budget and runs alone, which is both
// deadlock-free and exactly the desired behaviour for the big ones.
//
// SCALE NOTE: the constants are for the tightest platform (macOS). On Linux
// `FILE` stores the fd in an `int` and no such ceiling exists, so the budget
// is generous rather than binding there. It is derived, never hand-tuned — if
// a schema change alters the per-node cost, re-measure FDS_PER_TEST_NODE
// rather than nudging the reserve.

/// Highest file descriptor number Darwin stdio can wrap (`short _file`).
const FD_CEILING: usize = 32_767;
/// Measured file descriptors per live test node (`fds = 28 * nodes + 9`).
const FDS_PER_TEST_NODE: usize = 28;
/// Descriptors left for the harness, the tokio runtime and non-node files.
const FD_RESERVE: usize = 4_000;
/// Live test nodes permitted across the whole process at any one moment.
pub const MAX_LIVE_TEST_NODES: usize = (FD_CEILING - FD_RESERVE) / FDS_PER_TEST_NODE;

/// Process-wide budget of concurrently-live test nodes.
static NODE_BUDGET: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();

fn node_budget() -> &'static Arc<tokio::sync::Semaphore> {
    NODE_BUDGET.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_LIVE_TEST_NODES)))
}

/// Reserve capacity for `n_nodes` live nodes, waiting if the process is full.
///
/// Clamped to the whole budget so a network larger than the budget cannot
/// deadlock: it simply runs alone.
async fn reserve_node_capacity(n_nodes: usize) -> tokio::sync::OwnedSemaphorePermit {
    let want = n_nodes.clamp(1, MAX_LIVE_TEST_NODES) as u32;
    Arc::clone(node_budget())
        .acquire_many_owned(want)
        .await
        .expect("node budget semaphore is never closed")
}

/// A simulated P2P network of real DOLI nodes.
/// Nodes are wrapped in Arc<Mutex> for parallel block propagation.
pub struct TestNetwork {
    /// Real nodes, each with its own RocksDB in a temp directory
    pub nodes: Vec<Arc<Mutex<Node>>>,
    /// Temp directories (must outlive nodes)
    _temps: Vec<TempDir>,
    /// INC-I-198 live-node budget permit; released when this network drops.
    _fd_permit: tokio::sync::OwnedSemaphorePermit,
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
        // INC-I-198: reserve descriptor capacity BEFORE opening any RocksDB.
        // Held for the lifetime of this network and released on drop.
        let _fd_permit = reserve_node_capacity(n_nodes).await;
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
            _fd_permit,
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
        let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
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
            data_root: crypto::Hash::ZERO,
            fork_id: crypto::Hash::ZERO,
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

// ============================================================
// GOSSIP SIMULATION — realistic block propagation with delay
// ============================================================

/// Simulate realistic gossip propagation where blocks arrive at different
/// nodes at different times. Tests whether the network can grow.
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

                    // Rollback to ancestor using undo data directly (bypasses production guards)
                    {
                        let mut node = self.nodes[node_id].lock().await;
                        let current_h = node.chain_state.read().await.best_height;
                        for roll_h in (0..(current_h.saturating_sub(ancestor_h))).rev() {
                            let h = ancestor_h + roll_h + 1;
                            if let Some(undo) = node.state_db.get_undo(h) {
                                let mut utxo = node.utxo_set.write().await;
                                for outpoint in &undo.created_utxos {
                                    utxo.remove(outpoint).ok();
                                }
                                for (outpoint, entry) in &undo.spent_utxos {
                                    utxo.insert(*outpoint, entry.clone()).ok();
                                }
                                drop(utxo);
                                if let Ok(restored) = bincode::deserialize::<storage::ProducerSet>(
                                    &undo.producer_snapshot,
                                ) {
                                    let mut producers = node.producer_set.write().await;
                                    *producers = restored;
                                }
                            }
                        }
                        let ancestor_hash = if ancestor_h == 0 {
                            node.chain_state.read().await.genesis_hash
                        } else {
                            node.block_store
                                .get_block_by_height(ancestor_h)
                                .ok()
                                .flatten()
                                .map(|b| b.hash())
                                .unwrap_or(node.chain_state.read().await.genesis_hash)
                        };
                        let ancestor_slot = if ancestor_h == 0 {
                            0u32
                        } else {
                            node.block_store
                                .get_block_by_height(ancestor_h)
                                .ok()
                                .flatten()
                                .map(|b| b.header.slot)
                                .unwrap_or(ancestor_h as u32)
                        };
                        let mut cs = node.chain_state.write().await;
                        cs.best_height = ancestor_h;
                        cs.best_hash = ancestor_hash;
                        cs.best_slot = ancestor_slot;
                        drop(cs);
                        node.cumulative_rollback_depth = 0;
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
}

// ============================================================
// TEST: Gossip divergence with growing producer count
// ============================================================

// ============================================================
// REALISTIC GOSSIP — delay, partial delivery, slot gaps, sync backfill
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
    /// - Slot gap tracking
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

                    // Gate 0: prev_hash linkage (Light mode skips this, but
                    // production gossip checks it — without this, blocks with
                    // wrong prev_hash get applied at wrong heights, corrupting
                    // chain_state.best_slot)
                    let best_hash = n.chain_state.read().await.best_hash;
                    if block.header.prev_hash != best_hash {
                        return 2u8; // rejected — prev_hash mismatch
                    }

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

            // Step 2: Rollback to ancestor using undo data directly.
            // Bypasses rollback_one_block's production guards (sync_manager dependency,
            // genesis guard, cumulative depth cap) that don't apply in test context.
            {
                let mut node = self.nodes[node_id].lock().await;
                let current_h = node.chain_state.read().await.best_height;
                let blocks_to_rollback = current_h.saturating_sub(ancestor_h);
                for roll_h in (0..blocks_to_rollback).rev() {
                    let h = ancestor_h + roll_h + 1;
                    if let Some(undo) = node.state_db.get_undo(h) {
                        let mut utxo = node.utxo_set.write().await;
                        for outpoint in &undo.created_utxos {
                            utxo.remove(outpoint).ok();
                        }
                        for (outpoint, entry) in &undo.spent_utxos {
                            utxo.insert(*outpoint, entry.clone()).ok();
                        }
                        drop(utxo);
                        if let Ok(restored) =
                            bincode::deserialize::<storage::ProducerSet>(&undo.producer_snapshot)
                        {
                            let mut producers = node.producer_set.write().await;
                            *producers = restored;
                        }
                    }
                }
                // Reset chain_state to ancestor
                let ancestor_hash = if ancestor_h == 0 {
                    node.chain_state.read().await.genesis_hash
                } else {
                    node.block_store
                        .get_block_by_height(ancestor_h)
                        .ok()
                        .flatten()
                        .map(|b| b.hash())
                        .unwrap_or(node.chain_state.read().await.genesis_hash)
                };
                let ancestor_slot = if ancestor_h == 0 {
                    0u32
                } else {
                    node.block_store
                        .get_block_by_height(ancestor_h)
                        .ok()
                        .flatten()
                        .map(|b| b.header.slot)
                        .unwrap_or(ancestor_h as u32)
                };
                {
                    let mut cs = node.chain_state.write().await;
                    cs.best_height = ancestor_h;
                    cs.best_hash = ancestor_hash;
                    cs.best_slot = ancestor_slot;
                }
                node.cumulative_rollback_depth = 0;
                total_rollbacks += blocks_to_rollback as usize;
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

    /// Count divergent exclusions (Fix #3 cleanup: field removed, now always 0,0,0).
    pub async fn count_divergent_exclusions(&self) -> (usize, usize, usize) {
        (0, 0, 0)
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
pub async fn build_scheduler_for_node(node: &Node) -> DeterministicScheduler {
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
pub fn scheduler_fingerprint(sched: &DeterministicScheduler) -> (usize, u64, Option<PublicKey>) {
    let rank0 = sched.select_producer(0, 0).copied();
    (sched.producer_count(), sched.total_bonds(), rank0)
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
        let coinbase = Transaction::new_coinbase(reward, pool_hash, height, 0);
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
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };

        Block::new(header, vec![coinbase])
    }

    /// Get the epoch_producer_list from a node
    pub async fn epoch_list(&self, node_id: usize) -> Vec<PublicKey> {
        let n = self.nodes[node_id].lock().await;
        n.epoch_state.producer_list.clone()
    }

    /// Set the epoch_producer_list on all nodes (for test setup)
    pub async fn set_epoch_list_all(&self, list: &[PublicKey]) {
        for node in &self.nodes {
            let mut n = node.lock().await;
            n.epoch_state.producer_list = list.to_vec();
        }
    }
}
