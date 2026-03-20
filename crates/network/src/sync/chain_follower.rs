//! Unified chain follower: single state machine for all fork recovery.
//!
//! Replaces: ForkRecoveryTracker (parent walk), ForkSync (binary search),
//! resolve_shallow_fork (sequential rollbacks).
//!
//! State machine: Idle → FindAncestor → Download → Ready
//! On any failure: try next peer. No cooldowns, no grace periods.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::{Block, BlockHeader};
use libp2p::PeerId;
use tracing::{debug, info, warn};

use super::fork_sync::ProbeResult;
use crate::protocols::SyncRequest;

/// Maximum reorg depth for chain follower (matches fork_sync)
const MAX_SEARCH_DEPTH: u64 = 1000;

/// Timeout per session (doubled from ForkSync's 60s)
const SESSION_TIMEOUT: Duration = Duration::from_secs(120);

/// Wait time after all peers exhausted before retrying
const RETRY_WAIT: Duration = Duration::from_secs(10);

/// Phase of header/body download
#[derive(Clone, Debug)]
pub enum DownloadPhase {
    Headers,
    Bodies,
}

/// Chain follower state machine
#[derive(Clone, Debug)]
pub enum FollowerState {
    /// No fork detected, following chain normally
    Idle,
    /// Binary searching for common ancestor with a peer
    FindAncestor {
        peer: PeerId,
        low: u64,
        high: u64,
        low_verified: bool,
        pending_height: Option<u64>,
        peer_probe_hash: Option<Hash>,
        store_floor: u64,
        store_limited: bool,
        floor_probed: bool,
        started_at: Instant,
    },
    /// Downloading canonical chain from ancestor to peer tip
    Download {
        peer: PeerId,
        ancestor_height: u64,
        ancestor_hash: Hash,
        headers: Vec<BlockHeader>,
        blocks: Vec<Block>,
        phase: DownloadPhase,
        started_at: Instant,
    },
    /// Blocks ready for node to apply (rollback + apply)
    Ready {
        ancestor_height: u64,
        ancestor_hash: Hash,
        blocks: Vec<Block>,
    },
    /// All peers exhausted, waiting before retry
    WaitingRetry { started_at: Instant },
}

/// Result of a completed chain follow — common ancestor + canonical blocks to apply
pub struct FollowerResult {
    pub ancestor_height: u64,
    pub ancestor_hash: Hash,
    pub canonical_blocks: Vec<Block>,
}

/// Unified chain follower replacing ForkRecoveryTracker + ForkSync + resolve_shallow_fork
pub struct ChainFollower {
    state: FollowerState,
    /// Peers already tried in this recovery session
    tried_peers: HashSet<PeerId>,
    /// Last cancellation reason (for diagnostics)
    last_cancel_reason: Option<String>,
}

impl ChainFollower {
    pub fn new() -> Self {
        Self {
            state: FollowerState::Idle,
            tried_peers: HashSet::new(),
            last_cancel_reason: None,
        }
    }

    /// Start fork recovery. Called when fork is detected (empty headers, hash mismatch).
    /// Returns false if already active (not Idle or WaitingRetry).
    pub fn start(
        &mut self,
        local_height: u64,
        finality_floor: u64,
        store_floor: u64,
        peer: PeerId,
    ) -> bool {
        if self.is_active() {
            return false;
        }

        let high = local_height;
        let depth_floor = local_height.saturating_sub(MAX_SEARCH_DEPTH);
        let low = depth_floor.max(store_floor).max(finality_floor);

        info!(
            "Chain follower: starting binary search (low={}, high={}, finality_floor={}, store_floor={}) with peer {}",
            low, high, finality_floor, store_floor, peer
        );

        self.tried_peers.clear();
        self.tried_peers.insert(peer);
        self.last_cancel_reason = None;
        self.state = FollowerState::FindAncestor {
            peer,
            low,
            high,
            low_verified: false,
            pending_height: None,
            peer_probe_hash: None,
            store_floor,
            store_limited: false,
            floor_probed: false,
            started_at: Instant::now(),
        };
        true
    }

    /// Restart with a different peer (called on timeout or failure).
    /// Returns false if no untried peer was provided.
    pub fn retry_with_peer(
        &mut self,
        local_height: u64,
        finality_floor: u64,
        store_floor: u64,
        peer: PeerId,
    ) -> bool {
        if self.tried_peers.contains(&peer) {
            return false;
        }

        let high = local_height;
        let depth_floor = local_height.saturating_sub(MAX_SEARCH_DEPTH);
        let low = depth_floor.max(store_floor).max(finality_floor);

        info!(
            "Chain follower: retrying with peer {} (low={}, high={})",
            peer, low, high
        );

        self.tried_peers.insert(peer);
        self.state = FollowerState::FindAncestor {
            peer,
            low,
            high,
            low_verified: false,
            pending_height: None,
            peer_probe_hash: None,
            store_floor,
            store_limited: false,
            floor_probed: false,
            started_at: Instant::now(),
        };
        true
    }

    /// Get next request to send to peer
    pub fn next_request(&mut self) -> Option<(PeerId, SyncRequest)> {
        match &self.state {
            FollowerState::FindAncestor {
                peer,
                low,
                high,
                low_verified,
                pending_height,
                store_limited,
                floor_probed,
                started_at,
                ..
            } => {
                if started_at.elapsed() > SESSION_TIMEOUT {
                    let peer = *peer;
                    self.cancel(&format!("session timeout with peer {}", peer));
                    return None;
                }

                // Don't send a new probe while one is pending
                if pending_height.is_some() {
                    return None;
                }

                let (low, high) = (*low, *high);
                let peer = *peer;

                if high <= low + 1 {
                    if !low_verified && !floor_probed && low > 0 && !store_limited {
                        // Verify floor before declaring bottomed out
                        debug!(
                            "Chain follower: verifying floor at height {} before concluding",
                            low
                        );
                        if let FollowerState::FindAncestor {
                            pending_height: ref mut ph,
                            floor_probed: ref mut fp,
                            ..
                        } = self.state
                        {
                            *fp = true;
                            *ph = Some(low);
                        }
                        return Some((peer, SyncRequest::GetBlockByHeight { height: low }));
                    }
                    // Search complete
                    return None;
                }

                let mid = low + (high - low) / 2;
                if let FollowerState::FindAncestor {
                    pending_height: ref mut ph,
                    ..
                } = self.state
                {
                    *ph = Some(mid);
                }
                debug!(
                    "Chain follower: probing height {} (low={}, high={})",
                    mid, low, high
                );
                Some((peer, SyncRequest::GetBlockByHeight { height: mid }))
            }
            FollowerState::Download {
                peer,
                ancestor_hash,
                headers,
                phase,
                started_at,
                ..
            } => {
                if started_at.elapsed() > SESSION_TIMEOUT {
                    let peer = *peer;
                    self.cancel(&format!("download timeout with peer {}", peer));
                    return None;
                }

                let peer = *peer;
                match phase {
                    DownloadPhase::Headers => {
                        let start_hash = if headers.is_empty() {
                            *ancestor_hash
                        } else {
                            headers.last().unwrap().hash()
                        };
                        Some((
                            peer,
                            SyncRequest::GetHeaders {
                                start_hash,
                                max_count: 500,
                            },
                        ))
                    }
                    DownloadPhase::Bodies => {
                        let hashes: Vec<Hash> = headers.iter().map(|h| h.hash()).collect();
                        if hashes.is_empty() {
                            return None;
                        }
                        Some((peer, SyncRequest::GetBodies { hashes }))
                    }
                }
            }
            _ => None,
        }
    }

    /// Handle block response (for binary search probes)
    pub fn handle_block_response(&mut self, block: Option<Block>) {
        if let FollowerState::FindAncestor {
            ref mut pending_height,
            ref mut peer_probe_hash,
            ref mut high,
            ..
        } = self.state
        {
            match block {
                Some(blk) => {
                    *peer_probe_hash = Some(blk.hash());
                    debug!(
                        "Chain follower: peer has block {} at height {:?}",
                        &blk.hash().to_string()[..16],
                        pending_height
                    );
                }
                None => {
                    if let Some(height) = pending_height.take() {
                        debug!(
                            "Chain follower: peer has no block at height {} — narrowing high",
                            height
                        );
                        *high = height;
                        *peer_probe_hash = None;
                    }
                }
            }
        }
    }

    /// Get pending probe for Node to compare with block_store.
    /// Returns (height, peer_hash) when a probe response is ready for comparison.
    pub fn pending_probe(&self) -> Option<(u64, Hash)> {
        if let FollowerState::FindAncestor {
            pending_height,
            peer_probe_hash,
            ..
        } = &self.state
        {
            let height = (*pending_height)?;
            let peer_hash = (*peer_probe_hash)?;
            Some((height, peer_hash))
        } else {
            None
        }
    }

    /// Feed the result of the local block_store comparison.
    pub fn handle_probe_result(&mut self, result: ProbeResult) {
        if let FollowerState::FindAncestor {
            ref mut pending_height,
            ref mut peer_probe_hash,
            ref mut low,
            ref mut high,
            ref mut low_verified,
            ref mut store_limited,
            store_floor,
            ..
        } = self.state
        {
            let height = match pending_height.take() {
                Some(h) => h,
                None => return,
            };
            *peer_probe_hash = None;

            match result {
                ProbeResult::Match => {
                    *low = height;
                    *low_verified = true;
                    debug!("Chain follower: match at height {} — low={}", height, low);
                }
                ProbeResult::Mismatch => {
                    *high = height;
                    debug!(
                        "Chain follower: mismatch at height {} — high={}",
                        height, high
                    );
                }
                ProbeResult::NotInStore => {
                    info!(
                        "Chain follower: height {} not in block store (store_floor={})",
                        height, store_floor
                    );
                    *low = height;
                    *store_limited = true;
                }
            }

            if *high <= *low + 1 {
                if *store_limited {
                    info!(
                        "Chain follower: search stopped at height {} — store limited",
                        *low,
                    );
                } else {
                    info!(
                        "Chain follower: common ancestor found at height {} — waiting for Node to confirm hash",
                        *low,
                    );
                }
            }
        }
    }

    /// Handle headers response during Download phase
    pub fn handle_headers_response(&mut self, headers: Vec<BlockHeader>) {
        if let FollowerState::Download {
            headers: ref mut stored_headers,
            ref mut phase,
            ..
        } = self.state
        {
            if headers.is_empty() {
                // No more headers — transition to body download
                if stored_headers.is_empty() {
                    // Peer has no headers past ancestor — fork resolved
                    debug!("Chain follower: peer has no headers past ancestor, completing");
                }
                *phase = DownloadPhase::Bodies;
            } else {
                stored_headers.extend(headers);
                // Transition to body download after receiving headers
                *phase = DownloadPhase::Bodies;
            }
        }
    }

    /// Handle bodies response during Download phase
    pub fn handle_bodies_response(&mut self, new_blocks: Vec<Block>) {
        // Extract state to avoid borrow conflicts
        let old_state = std::mem::replace(&mut self.state, FollowerState::Idle);
        if let FollowerState::Download {
            mut blocks,
            ancestor_height,
            ancestor_hash,
            ..
        } = old_state
        {
            blocks.extend(new_blocks);
            let count = blocks.len();
            info!(
                "Chain follower: download complete, {} blocks ready for reorg from ancestor height {}",
                count, ancestor_height
            );
            self.state = FollowerState::Ready {
                ancestor_height,
                ancestor_hash,
                blocks,
            };
        } else {
            // Put the state back if it wasn't Download
            self.state = old_state;
        }
    }

    /// Take completed result (blocks to apply after rollback)
    pub fn take_result(&mut self) -> Option<FollowerResult> {
        if matches!(self.state, FollowerState::Ready { .. }) {
            let old_state = std::mem::replace(&mut self.state, FollowerState::Idle);
            if let FollowerState::Ready {
                ancestor_height,
                ancestor_hash,
                blocks,
            } = old_state
            {
                return Some(FollowerResult {
                    ancestor_height,
                    ancestor_hash,
                    canonical_blocks: blocks,
                });
            }
        }
        None
    }

    /// Is the follower active? (not Idle, not WaitingRetry)
    pub fn is_active(&self) -> bool {
        !matches!(
            self.state,
            FollowerState::Idle | FollowerState::WaitingRetry { .. }
        )
    }

    /// Check if the follower is waiting for retry after all peers exhausted
    pub fn is_waiting_retry(&self) -> bool {
        if let FollowerState::WaitingRetry { started_at } = &self.state {
            started_at.elapsed() < RETRY_WAIT
        } else {
            false
        }
    }

    /// Check if retry wait has expired and we should try again
    pub fn retry_wait_expired(&self) -> bool {
        if let FollowerState::WaitingRetry { started_at } = &self.state {
            started_at.elapsed() >= RETRY_WAIT
        } else {
            false
        }
    }

    /// Cancel current session
    pub fn cancel(&mut self, reason: &str) {
        warn!("Chain follower: cancelled — {}", reason);
        self.last_cancel_reason = Some(reason.to_string());
        self.state = FollowerState::Idle;
    }

    /// Signal that all peers are exhausted — enter waiting state
    pub fn enter_waiting_retry(&mut self) {
        info!(
            "Chain follower: all peers exhausted ({} tried), waiting {}s before retry",
            self.tried_peers.len(),
            RETRY_WAIT.as_secs()
        );
        self.tried_peers.clear();
        self.state = FollowerState::WaitingRetry {
            started_at: Instant::now(),
        };
    }

    /// Reset to idle (e.g., when fork is resolved via normal sync)
    pub fn reset(&mut self) {
        self.state = FollowerState::Idle;
        self.tried_peers.clear();
        self.last_cancel_reason = None;
    }

    /// Set the ancestor hash when search completes (Node provides from block_store)
    pub fn set_ancestor(&mut self, height: u64, hash: Hash) {
        if let FollowerState::FindAncestor {
            peer, started_at, ..
        } = &self.state
        {
            let peer = *peer;
            let started_at = *started_at;
            info!(
                "Chain follower: ancestor confirmed at height {} — downloading canonical chain",
                height
            );
            self.state = FollowerState::Download {
                peer,
                ancestor_height: height,
                ancestor_hash: hash,
                headers: Vec::new(),
                blocks: Vec::new(),
                phase: DownloadPhase::Headers,
                started_at,
            };
        }
    }

    /// Check if search found an ancestor (returns height when search converged)
    pub fn search_complete(&self) -> Option<u64> {
        if let FollowerState::FindAncestor {
            low,
            high,
            pending_height,
            store_limited,
            ..
        } = &self.state
        {
            if *high <= *low + 1 && pending_height.is_none() && !store_limited {
                return Some(*low);
            }
        }
        None
    }

    /// Check if search bottomed out (no common ancestor within MAX_SEARCH_DEPTH)
    pub fn search_bottomed_out(&self) -> bool {
        if let FollowerState::FindAncestor {
            low,
            high,
            low_verified,
            pending_height,
            floor_probed,
            store_limited,
            ..
        } = &self.state
        {
            *high <= *low + 1
                && pending_height.is_none()
                && !*low_verified
                && *low > 0
                && *floor_probed
                && !*store_limited
        } else {
            false
        }
    }

    /// Check if search was limited by block store range
    pub fn search_store_limited(&self) -> bool {
        if let FollowerState::FindAncestor {
            high,
            low,
            store_limited,
            pending_height,
            ..
        } = &self.state
        {
            *high <= *low + 1 && *store_limited && pending_height.is_none()
        } else {
            false
        }
    }

    /// Get the peer currently being used
    pub fn current_peer(&self) -> Option<PeerId> {
        match &self.state {
            FollowerState::FindAncestor { peer, .. } => Some(*peer),
            FollowerState::Download { peer, .. } => Some(*peer),
            _ => None,
        }
    }

    /// Check if a peer has already been tried
    pub fn peer_tried(&self, peer: &PeerId) -> bool {
        self.tried_peers.contains(peer)
    }

    /// Get the current state (for diagnostics)
    pub fn state(&self) -> &FollowerState {
        &self.state
    }

    /// Check if session has timed out
    pub fn is_timed_out(&self) -> bool {
        match &self.state {
            FollowerState::FindAncestor { started_at, .. } => {
                started_at.elapsed() > SESSION_TIMEOUT
            }
            FollowerState::Download { started_at, .. } => started_at.elapsed() > SESSION_TIMEOUT,
            _ => false,
        }
    }
}

impl Default for ChainFollower {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_peer(_seed: u8) -> PeerId {
        PeerId::random()
    }

    fn make_hash(seed: u8) -> Hash {
        crypto::hash::hash(&[seed])
    }

    #[test]
    fn test_new_is_idle() {
        let follower = ChainFollower::new();
        assert!(!follower.is_active());
        assert!(matches!(follower.state(), FollowerState::Idle));
    }

    #[test]
    fn test_start_makes_active() {
        let mut follower = ChainFollower::new();
        let peer = make_peer(1);
        assert!(follower.start(100, 0, 1, peer));
        assert!(follower.is_active());
    }

    #[test]
    fn test_start_fails_when_active() {
        let mut follower = ChainFollower::new();
        let peer1 = make_peer(1);
        let peer2 = make_peer(2);
        assert!(follower.start(100, 0, 1, peer1));
        assert!(!follower.start(100, 0, 1, peer2)); // Already active
    }

    #[test]
    fn test_binary_search_converges() {
        let mut follower = ChainFollower::new();
        let peer = make_peer(1);
        // Local height 100, finality at 0, store floor 1
        follower.start(100, 0, 1, peer);

        // First probe should be midpoint: (1 + 100) / 2 = 50
        // Actually: low = max(0, 1, 0) = 1, high = 100, mid = 1 + (100-1)/2 = 50
        let req = follower.next_request();
        assert!(req.is_some());
        let (_, request) = req.unwrap();
        if let SyncRequest::GetBlockByHeight { height } = request {
            assert_eq!(height, 50);
        } else {
            panic!("Expected GetBlockByHeight");
        }

        // Simulate: peer has block at height 50, give it back
        let block_hash = make_hash(50);
        follower.handle_block_response(Some(Block::new(
            doli_core::BlockHeader {
                version: 1,
                prev_hash: Hash::ZERO,
                merkle_root: Hash::ZERO,
                presence_root: Hash::ZERO,
                genesis_hash: Hash::ZERO,
                timestamp: 1000,
                slot: 50,
                producer: crypto::PublicKey::from_bytes([0u8; 32]),
                vdf_output: vdf::VdfOutput { value: vec![0; 32] },
                vdf_proof: vdf::VdfProof::empty(),
            },
            vec![],
        )));

        // Pending probe should be available
        let probe = follower.pending_probe();
        assert!(probe.is_some());

        // Simulate: local store matches at height 50
        follower.handle_probe_result(ProbeResult::Match);

        // Now low=50, high=100, mid=75
        let req = follower.next_request();
        assert!(req.is_some());
    }

    #[test]
    fn test_search_bounded_by_finality() {
        let mut follower = ChainFollower::new();
        let peer = make_peer(1);
        // Local height 100, finality at 80 — search should never go below 80
        follower.start(100, 80, 1, peer);

        if let FollowerState::FindAncestor { low, .. } = &follower.state {
            assert!(*low >= 80, "low should be bounded by finality floor");
        } else {
            panic!("Expected FindAncestor state");
        }
    }

    #[test]
    fn test_no_cooldown_between_sessions() {
        let mut follower = ChainFollower::new();
        let peer1 = make_peer(1);
        follower.start(100, 0, 1, peer1);
        follower.cancel("test cancel");
        assert!(!follower.is_active());

        // Can immediately start again — no cooldown
        let peer2 = make_peer(2);
        assert!(follower.start(100, 0, 1, peer2));
        assert!(follower.is_active());
    }

    #[test]
    fn test_peer_exhaustion_then_retry() {
        let mut follower = ChainFollower::new();
        let peer1 = make_peer(1);
        follower.start(100, 0, 1, peer1);
        follower.cancel("test");

        // Try to retry with same peer — should fail
        assert!(!follower.retry_with_peer(100, 0, 1, peer1));

        // All peers exhausted, enter waiting
        follower.enter_waiting_retry();
        assert!(!follower.is_active());
        assert!(follower.is_waiting_retry());

        // After retry wait, tried_peers should be clear
        // (In real code, retry_wait_expired() would be checked)
    }

    #[test]
    fn test_full_state_machine_transition() {
        let mut follower = ChainFollower::new();
        let peer = make_peer(1);

        // Start: Idle → FindAncestor
        assert!(follower.start(10, 0, 1, peer));
        assert!(matches!(
            follower.state(),
            FollowerState::FindAncestor { .. }
        ));

        // Simulate quick search: ancestor at height 5
        // low=1, high=10, mid=5
        let _ = follower.next_request(); // probe height 5

        // Simulate match at 5
        follower.handle_block_response(Some(Block::new(
            doli_core::BlockHeader {
                version: 1,
                prev_hash: Hash::ZERO,
                merkle_root: Hash::ZERO,
                presence_root: Hash::ZERO,
                genesis_hash: Hash::ZERO,
                timestamp: 1000,
                slot: 5,
                producer: crypto::PublicKey::from_bytes([0u8; 32]),
                vdf_output: vdf::VdfOutput { value: vec![0; 32] },
                vdf_proof: vdf::VdfProof::empty(),
            },
            vec![],
        )));
        follower.handle_probe_result(ProbeResult::Match);

        // low=5, high=10, mid=7
        let _ = follower.next_request(); // probe height 7
        follower.handle_block_response(Some(Block::new(
            doli_core::BlockHeader {
                version: 1,
                prev_hash: Hash::ZERO,
                merkle_root: Hash::ZERO,
                presence_root: Hash::ZERO,
                genesis_hash: Hash::ZERO,
                timestamp: 1000,
                slot: 7,
                producer: crypto::PublicKey::from_bytes([0u8; 32]),
                vdf_output: vdf::VdfOutput { value: vec![0; 32] },
                vdf_proof: vdf::VdfProof::empty(),
            },
            vec![],
        )));
        follower.handle_probe_result(ProbeResult::Mismatch);

        // low=5, high=7, mid=6
        let _ = follower.next_request(); // probe height 6
        follower.handle_block_response(Some(Block::new(
            doli_core::BlockHeader {
                version: 1,
                prev_hash: Hash::ZERO,
                merkle_root: Hash::ZERO,
                presence_root: Hash::ZERO,
                genesis_hash: Hash::ZERO,
                timestamp: 1000,
                slot: 6,
                producer: crypto::PublicKey::from_bytes([0u8; 32]),
                vdf_output: vdf::VdfOutput { value: vec![0; 32] },
                vdf_proof: vdf::VdfProof::empty(),
            },
            vec![],
        )));
        follower.handle_probe_result(ProbeResult::Mismatch);

        // low=5, high=6 → search complete
        assert!(follower.search_complete().is_some());
        assert_eq!(follower.search_complete().unwrap(), 5);

        // FindAncestor → Download
        follower.set_ancestor(5, make_hash(5));
        assert!(matches!(follower.state(), FollowerState::Download { .. }));

        // Download → Ready
        follower.handle_headers_response(vec![]);
        follower.handle_bodies_response(vec![]);
        assert!(matches!(follower.state(), FollowerState::Ready { .. }));

        // Ready → Idle (via take_result)
        let result = follower.take_result();
        assert!(result.is_some());
        assert!(!follower.is_active());
    }

    #[test]
    fn test_bottomed_out_signals_deep_fork() {
        let mut follower = ChainFollower::new();
        let peer = make_peer(1);
        // Height 1000, store_floor=1, finality=0 → low starts at 0 (capped by max(0,1,0)=1)
        follower.start(1000, 0, 1, peer);

        // Simulate search converging at floor without ever matching
        if let FollowerState::FindAncestor {
            ref mut low,
            ref mut high,
            ref mut floor_probed,
            ..
        } = follower.state
        {
            *low = 1;
            *high = 2;
            *floor_probed = true;
        }

        assert!(follower.search_bottomed_out());
    }
}
