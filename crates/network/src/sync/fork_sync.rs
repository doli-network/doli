//! Fork-aware sync via binary search for common ancestor.
//!
//! When header sync detects a fork (3+ consecutive failures), this module
//! binary-searches for the common ancestor using `GetBlockByHeight`, then
//! downloads the canonical chain and triggers a reorg in one atomic operation.
//!
//! O(log N) instead of O(N): a 50-block fork = ~6 round-trips, not 50 UTXO rebuilds.

use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::{Block, BlockHeader};
use libp2p::PeerId;
use tracing::{debug, info};

use crate::protocols::SyncRequest;

/// Maximum reorg depth for fork sync (matches MAX_REORG_DEPTH in reorg.rs)
const MAX_FORK_SYNC_DEPTH: u64 = 100;

/// Timeout for entire fork sync session
const FORK_SYNC_TIMEOUT: Duration = Duration::from_secs(60);

/// Phase of the fork sync process
#[derive(Clone, Debug)]
pub enum ForkSyncPhase {
    /// Binary searching for common ancestor
    Searching,
    /// Downloading canonical headers from ancestor
    DownloadingHeaders {
        ancestor_height: u64,
        ancestor_hash: Hash,
    },
    /// Downloading canonical block bodies
    DownloadingBodies {
        ancestor_height: u64,
        ancestor_hash: Hash,
    },
    /// Complete — result ready for Node to consume
    Complete {
        ancestor_height: u64,
        ancestor_hash: Hash,
    },
}

/// Result of a completed fork sync — common ancestor + canonical blocks to apply
pub struct ForkSyncResult {
    pub ancestor_height: u64,
    pub ancestor_hash: Hash,
    pub canonical_blocks: Vec<Block>,
}

/// Fork sync state machine. Follows the `ForkRecoveryTracker` pattern:
/// state lives in SyncManager, Node drives comparisons.
pub struct ForkSync {
    /// Peer we're syncing from
    peer: PeerId,
    /// Our local chain height when fork sync started (for logging)
    _local_height: u64,
    /// Binary search: lowest height known to match (genesis always matches)
    low: u64,
    /// Binary search: highest height known to differ (our fork tip)
    high: u64,
    /// Current phase
    phase: ForkSyncPhase,
    /// Height of the current probe waiting for response
    pending_height: Option<u64>,
    /// Peer's block hash at the probed height (from GetBlockByHeight response)
    probe_block_hash: Option<Hash>,
    /// Canonical headers downloaded from peer
    canonical_headers: Vec<BlockHeader>,
    /// Canonical blocks downloaded from peer
    canonical_blocks: Vec<Block>,
    /// When this session started
    started_at: Instant,
}

impl ForkSync {
    /// Create a new fork sync targeting the given peer.
    pub fn new(peer: PeerId, local_height: u64) -> Self {
        let high = local_height;
        // Cap search depth: don't binary search deeper than MAX_FORK_SYNC_DEPTH
        let low = local_height.saturating_sub(MAX_FORK_SYNC_DEPTH);

        info!(
            "Fork sync: starting binary search (low={}, high={}) with peer {}",
            low, high, peer
        );

        Self {
            peer,
            _local_height: local_height,
            low,
            high,
            phase: ForkSyncPhase::Searching,
            pending_height: None,
            probe_block_hash: None,
            canonical_headers: Vec::new(),
            canonical_blocks: Vec::new(),
            started_at: Instant::now(),
        }
    }

    /// Peer this fork sync is targeting
    pub fn peer(&self) -> PeerId {
        self.peer
    }

    /// Get the next request to send, if any.
    pub fn next_request(&mut self) -> Option<SyncRequest> {
        if self.is_timed_out() {
            return None;
        }

        match &self.phase {
            ForkSyncPhase::Searching => {
                // Don't send a new probe while one is pending
                if self.pending_height.is_some() {
                    return None;
                }
                // Binary search: probe the midpoint
                if self.high <= self.low + 1 {
                    // Search complete — ancestor is at `low`
                    return None;
                }
                let mid = self.low + (self.high - self.low) / 2;
                self.pending_height = Some(mid);
                debug!(
                    "Fork sync: probing height {} (low={}, high={})",
                    mid, self.low, self.high
                );
                Some(SyncRequest::GetBlockByHeight { height: mid })
            }
            ForkSyncPhase::DownloadingHeaders { ancestor_hash, .. } => {
                // Request headers starting from the ancestor
                let start_hash = if self.canonical_headers.is_empty() {
                    *ancestor_hash
                } else {
                    self.canonical_headers.last().unwrap().hash()
                };
                Some(SyncRequest::GetHeaders {
                    start_hash,
                    max_count: 500,
                })
            }
            ForkSyncPhase::DownloadingBodies { .. } => {
                // Request bodies for collected headers
                let hashes: Vec<Hash> = self.canonical_headers.iter().map(|h| h.hash()).collect();
                if hashes.is_empty() {
                    return None;
                }
                Some(SyncRequest::GetBodies { hashes })
            }
            ForkSyncPhase::Complete { .. } => None,
        }
    }

    /// Handle a Block response (from GetBlockByHeight probe during search).
    /// Stores the peer's block hash for Node to compare with our block_store.
    pub fn handle_block_response(&mut self, block: Option<Block>) {
        if !matches!(self.phase, ForkSyncPhase::Searching) {
            return;
        }
        match block {
            Some(blk) => {
                self.probe_block_hash = Some(blk.hash());
                debug!(
                    "Fork sync: peer has block {} at height {:?}",
                    &blk.hash().to_string()[..16],
                    self.pending_height
                );
            }
            None => {
                // Peer doesn't have a block at this height — treat as "different"
                if let Some(height) = self.pending_height {
                    debug!(
                        "Fork sync: peer has no block at height {} — narrowing high",
                        height
                    );
                    self.high = height;
                    self.pending_height = None;
                    self.probe_block_hash = None;
                }
            }
        }
    }

    /// Returns the pending probe for Node to compare with block_store.
    /// Returns (height, peer_hash) when a probe response is ready for comparison.
    pub fn pending_probe(&self) -> Option<(u64, Hash)> {
        if !matches!(self.phase, ForkSyncPhase::Searching) {
            return None;
        }
        let height = self.pending_height?;
        let peer_hash = self.probe_block_hash?;
        Some((height, peer_hash))
    }

    /// Feed the result of the local block_store comparison.
    /// `matches` = true means our block at that height has the same hash as the peer's.
    pub fn handle_probe_result(&mut self, matches: bool) {
        let height = match self.pending_height.take() {
            Some(h) => h,
            None => return,
        };
        self.probe_block_hash = None;

        if matches {
            // Same block at this height — ancestor is at or above this height
            self.low = height;
            debug!("Fork sync: match at height {} — low={}", height, self.low);
        } else {
            // Different block — ancestor is below this height
            self.high = height;
            debug!(
                "Fork sync: mismatch at height {} — high={}",
                height, self.high
            );
        }

        // Check if search is complete — Node will call set_ancestor() with the hash
        if self.high <= self.low + 1 {
            info!(
                "Fork sync: common ancestor found at height {} — waiting for Node to confirm hash",
                self.low,
            );
        }
    }

    /// Handle headers response during the DownloadingHeaders phase.
    pub fn handle_headers_response(&mut self, headers: Vec<BlockHeader>) {
        match &self.phase {
            ForkSyncPhase::Searching => {
                // Ignore headers during search phase
            }
            ForkSyncPhase::DownloadingHeaders {
                ancestor_height,
                ancestor_hash,
            } => {
                if headers.is_empty() {
                    // No more headers — transition to body download
                    info!(
                        "Fork sync: received {} canonical headers, downloading bodies",
                        self.canonical_headers.len()
                    );
                    self.phase = ForkSyncPhase::DownloadingBodies {
                        ancestor_height: *ancestor_height,
                        ancestor_hash: *ancestor_hash,
                    };
                    return;
                }
                // Only keep headers above the ancestor
                let keep: Vec<BlockHeader> = headers
                    .into_iter()
                    .filter(|h| h.slot > 0) // Skip genesis-like
                    .collect();
                let count = keep.len();
                self.canonical_headers.extend(keep);
                info!(
                    "Fork sync: received {} headers (total {})",
                    count,
                    self.canonical_headers.len()
                );
                // If we got fewer than expected, we have all headers
                if count < 500 {
                    info!(
                        "Fork sync: all {} canonical headers received, downloading bodies",
                        self.canonical_headers.len()
                    );
                    self.phase = ForkSyncPhase::DownloadingBodies {
                        ancestor_height: *ancestor_height,
                        ancestor_hash: *ancestor_hash,
                    };
                }
            }
            _ => {}
        }
    }

    /// Handle bodies response during the DownloadingBodies phase.
    pub fn handle_bodies_response(&mut self, blocks: Vec<Block>) {
        if let ForkSyncPhase::DownloadingBodies {
            ancestor_height,
            ancestor_hash,
        } = &self.phase
        {
            let ancestor_height = *ancestor_height;
            let ancestor_hash = *ancestor_hash;
            let count = blocks.len();
            self.canonical_blocks = blocks;
            info!(
                "Fork sync: received {} canonical blocks — reorg ready",
                count
            );
            self.phase = ForkSyncPhase::Complete {
                ancestor_height,
                ancestor_hash,
            };
        }
    }

    /// Transition from search to download phase.
    /// Node provides the ancestor hash from its own block_store (both chains agree at `low`).
    pub fn set_ancestor(&mut self, ancestor_height: u64, ancestor_hash: Hash) {
        info!(
            "Fork sync: ancestor confirmed at height {} (hash={:.16}), downloading canonical chain",
            ancestor_height, ancestor_hash
        );
        self.phase = ForkSyncPhase::DownloadingHeaders {
            ancestor_height,
            ancestor_hash,
        };
    }

    /// Take the completed result. Returns None if not yet complete.
    pub fn take_result(&mut self) -> Option<ForkSyncResult> {
        if let ForkSyncPhase::Complete {
            ancestor_height,
            ancestor_hash,
        } = &self.phase
        {
            let result = ForkSyncResult {
                ancestor_height: *ancestor_height,
                ancestor_hash: *ancestor_hash,
                canonical_blocks: std::mem::take(&mut self.canonical_blocks),
            };
            Some(result)
        } else {
            None
        }
    }

    /// Check if the fork sync has timed out.
    pub fn is_timed_out(&self) -> bool {
        self.started_at.elapsed() > FORK_SYNC_TIMEOUT
    }

    /// Check if search is complete and we're waiting for Node to provide the ancestor hash.
    /// Returns Some(ancestor_height) when ready for transition.
    pub fn search_complete_ancestor_height(&self) -> Option<u64> {
        if matches!(self.phase, ForkSyncPhase::Searching)
            && self.high <= self.low + 1
            && self.pending_height.is_none()
        {
            Some(self.low)
        } else {
            None
        }
    }

    /// Current phase (for logging)
    pub fn phase_name(&self) -> &'static str {
        match &self.phase {
            ForkSyncPhase::Searching => "Searching",
            ForkSyncPhase::DownloadingHeaders { .. } => "DownloadingHeaders",
            ForkSyncPhase::DownloadingBodies { .. } => "DownloadingBodies",
            ForkSyncPhase::Complete { .. } => "Complete",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_search_converges() {
        let peer = PeerId::random();
        let mut fs = ForkSync::new(peer, 100);

        // Simulate: fork at height 90, so blocks 0-89 match, 90-100 differ
        let fork_point = 89;

        let mut rounds = 0;
        loop {
            rounds += 1;
            if rounds > 20 {
                panic!("Binary search did not converge in 20 rounds");
            }

            let req = fs.next_request();
            match req {
                Some(SyncRequest::GetBlockByHeight { height }) => {
                    // Simulate: peer has a block at every height
                    let _fake_hash = crypto::hash::hash(format!("block_{}", height).as_bytes());
                    fs.handle_block_response(Some(doli_core::Block::new(
                        doli_core::BlockHeader {
                            version: 1,
                            prev_hash: Hash::ZERO,
                            merkle_root: Hash::ZERO,
                            presence_root: Hash::ZERO,
                            timestamp: height * 10,
                            slot: height as u32,
                            producer: crypto::PublicKey::from_bytes([0u8; 32]),
                            vdf_output: vdf::VdfOutput { value: vec![] },
                            vdf_proof: vdf::VdfProof::empty(),
                        },
                        vec![],
                    )));

                    // Check probe
                    if let Some((h, _peer_hash)) = fs.pending_probe() {
                        let matches = h <= fork_point;
                        fs.handle_probe_result(matches);
                    }
                }
                None => break,
                _ => panic!("Unexpected request type"),
            }
        }

        // Search should find ancestor at fork_point or very close
        assert!(fs.low >= fork_point - 1 && fs.low <= fork_point);
        assert!(
            rounds <= 10,
            "Binary search took too many rounds: {}",
            rounds
        );
    }

    #[test]
    fn test_timeout() {
        let peer = PeerId::random();
        let fs = ForkSync {
            peer,
            _local_height: 100,
            low: 0,
            high: 100,
            phase: ForkSyncPhase::Searching,
            pending_height: None,
            probe_block_hash: None,
            canonical_headers: vec![],
            canonical_blocks: vec![],
            started_at: Instant::now() - Duration::from_secs(120),
        };
        assert!(fs.is_timed_out());
    }

    #[test]
    fn test_phase_transitions() {
        let peer = PeerId::random();
        let mut fs = ForkSync::new(peer, 10);
        assert_eq!(fs.phase_name(), "Searching");

        // Force search to complete
        fs.low = 5;
        fs.high = 6;
        fs.phase = ForkSyncPhase::DownloadingHeaders {
            ancestor_height: 5,
            ancestor_hash: Hash::ZERO,
        };
        assert_eq!(fs.phase_name(), "DownloadingHeaders");

        // Empty headers → transition to bodies
        fs.handle_headers_response(vec![]);
        assert_eq!(fs.phase_name(), "DownloadingBodies");

        // Bodies received → complete
        fs.handle_bodies_response(vec![]);
        assert_eq!(fs.phase_name(), "Complete");

        // Take result
        let result = fs.take_result();
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.ancestor_height, 5);
    }
}
