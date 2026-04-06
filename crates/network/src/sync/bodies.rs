//! Parallel block body download
//!
//! Downloads block bodies from multiple peers concurrently
//! to maximize throughput during initial sync.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::{debug, warn};

use crypto::Hash;
use doli_core::Block;

use crate::protocols::SyncRequest;

/// Request state for body downloads
#[derive(Debug)]
struct BodyRequest {
    /// Hashes being requested
    hashes: Vec<Hash>,
    /// Peer the request was sent to
    #[allow(dead_code)]
    peer: PeerId,
    /// When the request was sent
    sent_at: Instant,
}

/// Body downloader for parallel block retrieval
pub struct BodyDownloader {
    /// Maximum bodies per request
    max_bodies_per_request: usize,
    /// Maximum concurrent requests
    max_concurrent_requests: usize,
    /// Request timeout
    request_timeout: Duration,
    /// Active requests (peer -> request info)
    active_requests: HashMap<PeerId, BodyRequest>,
    /// Hashes currently being requested
    in_flight: HashSet<Hash>,
    /// Downloaded blocks waiting to be processed
    downloaded: HashMap<Hash, Block>,
    /// Failed request hashes (need retry)
    failed: VecDeque<Hash>,
    /// Per-hash failure count — give up after MAX_BODY_FAILURES
    failure_count: HashMap<Hash, u32>,
    /// Peers that returned empty for a specific hash — don't ask again
    failed_peers: HashMap<Hash, HashSet<PeerId>>,
    /// Permanently failed hashes with timestamp (gave up after too many retries).
    /// INC-I-012 F12: Entries expire after 60s and move back to retry queue,
    /// allowing the downloader to try peers that may have come online since.
    permanently_failed: HashMap<Hash, Instant>,
    /// Round-robin peer index
    next_peer_index: usize,
    /// Total bodies downloaded
    total_downloaded: usize,
}

impl BodyDownloader {
    /// Create a new body downloader
    pub fn new(
        max_bodies_per_request: usize,
        max_concurrent_requests: usize,
        request_timeout: Duration,
    ) -> Self {
        Self {
            max_bodies_per_request,
            max_concurrent_requests,
            request_timeout,
            active_requests: HashMap::new(),
            in_flight: HashSet::new(),
            downloaded: HashMap::new(),
            failed: VecDeque::new(),
            failure_count: HashMap::new(),
            failed_peers: HashMap::new(),
            permanently_failed: HashMap::new(),
            next_peer_index: 0,
            total_downloaded: 0,
        }
    }

    /// Get the next body request to send
    pub fn next_request(
        &mut self,
        needed: &VecDeque<Hash>,
        available_peers: &[PeerId],
    ) -> Option<(PeerId, SyncRequest)> {
        // Check if we can make more requests
        if self.active_requests.len() >= self.max_concurrent_requests {
            warn!(
                "[BODY_DL] Blocked: active_requests={} >= max={}",
                self.active_requests.len(),
                self.max_concurrent_requests
            );
            return None;
        }

        // Filter out peers with active requests
        let free_peers: Vec<&PeerId> = available_peers
            .iter()
            .filter(|p| !self.active_requests.contains_key(*p))
            .collect();

        if free_peers.is_empty() {
            warn!(
                "[BODY_DL] No free peers: available={}, active_requests={}",
                available_peers.len(),
                self.active_requests.len()
            );
            return None;
        }

        // Invariant: in_flight should only contain hashes tracked by active_requests.
        // Compute the expected in-flight count from active requests and release any excess.
        // This handles all stall scenarios: peer disconnect, lost responses, timeout races.
        let expected_in_flight: usize = self.active_requests.values().map(|r| r.hashes.len()).sum();
        let actual_in_flight = self.in_flight.len();
        if actual_in_flight > expected_in_flight {
            // Rebuild in_flight from active_requests to drop orphaned hashes
            let valid: HashSet<Hash> = self
                .active_requests
                .values()
                .flat_map(|r| r.hashes.iter().copied())
                .collect();
            let orphaned: Vec<Hash> = self
                .in_flight
                .iter()
                .filter(|h| !valid.contains(h))
                .copied()
                .collect();
            for hash in &orphaned {
                self.in_flight.remove(hash);
                self.failed.push_back(*hash);
            }
            if !orphaned.is_empty() {
                warn!(
                    "Released {} orphaned in-flight hashes ({} active, {} were in-flight)",
                    orphaned.len(),
                    expected_in_flight,
                    actual_in_flight
                );
            }
        }

        // Collect hashes to request (not already in flight)
        let mut hashes = Vec::new();

        // INC-I-012 F12: Expire permanently_failed entries after 60s.
        // Allows retrying with peers that may have come online since the failure.
        let now_for_expiry = Instant::now();
        let expired: Vec<Hash> = self
            .permanently_failed
            .iter()
            .filter(|(_, failed_at)| now_for_expiry.duration_since(**failed_at).as_secs() >= 60)
            .map(|(h, _)| *h)
            .collect();
        for hash in expired {
            self.permanently_failed.remove(&hash);
            self.failure_count.remove(&hash);
            self.failed_peers.remove(&hash);
            self.failed.push_back(hash);
        }

        // First try failed hashes (skip in-flight and permanently failed)
        while hashes.len() < self.max_bodies_per_request && !self.failed.is_empty() {
            if let Some(hash) = self.failed.pop_front() {
                if !self.in_flight.contains(&hash) && !self.permanently_failed.contains_key(&hash) {
                    hashes.push(hash);
                }
            }
        }

        // Then new hashes (skip permanently failed)
        for hash in needed.iter() {
            if hashes.len() >= self.max_bodies_per_request {
                break;
            }
            if !self.in_flight.contains(hash)
                && !self.downloaded.contains_key(hash)
                && !self.permanently_failed.contains_key(hash)
            {
                hashes.push(*hash);
            }
        }

        if hashes.is_empty() {
            warn!(
                "[BODY_DL] No hashes to request: needed={}, failed={}, in_flight={}, downloaded={}",
                needed.len(),
                self.failed.len(),
                self.in_flight.len(),
                self.downloaded.len()
            );
            return None;
        }

        // Select peer — prefer peers that haven't failed for these hashes
        let first_hash = hashes[0];
        let failed_for_hash = self.failed_peers.get(&first_hash);
        let preferred: Vec<&PeerId> = free_peers
            .iter()
            .filter(|p| failed_for_hash.map(|f| !f.contains(*p)).unwrap_or(true))
            .copied()
            .collect();
        let candidates = if preferred.is_empty() {
            &free_peers
        } else {
            &preferred
        };
        let peer_index = self.next_peer_index % candidates.len();
        let peer = *candidates[peer_index];
        self.next_peer_index = self.next_peer_index.wrapping_add(1);

        // Mark hashes as in-flight
        for hash in &hashes {
            self.in_flight.insert(*hash);
        }

        // Record the request
        self.active_requests.insert(
            peer,
            BodyRequest {
                hashes: hashes.clone(),
                peer,
                sent_at: Instant::now(),
            },
        );

        debug!(
            "Requesting {} bodies from {} ({} active requests)",
            hashes.len(),
            peer,
            self.active_requests.len()
        );

        Some((peer, SyncRequest::GetBodies { hashes }))
    }

    /// Process received bodies
    pub fn process_response(&mut self, peer: PeerId, bodies: Vec<Block>) -> Vec<Block> {
        // Get the request info
        let request = match self.active_requests.remove(&peer) {
            Some(r) => r,
            None => {
                warn!("Received bodies from {} but no active request", peer);
                return bodies;
            }
        };

        let mut received_hashes = HashSet::new();

        // Process received bodies
        for block in &bodies {
            let hash = block.hash();
            received_hashes.insert(hash);
            self.in_flight.remove(&hash);
            self.downloaded.insert(hash, block.clone());
            self.total_downloaded += 1;
        }

        // Mark missing hashes as failed — track per-hash failure count
        for hash in &request.hashes {
            if !received_hashes.contains(hash) {
                self.in_flight.remove(hash);

                // Track which peer failed for this hash
                self.failed_peers.entry(*hash).or_default().insert(peer);

                let count = self.failure_count.entry(*hash).or_insert(0);
                *count += 1;

                if *count >= 3 {
                    warn!(
                        "Giving up on body {} after {} failures — marking permanently failed (60s expiry)",
                        hash, count
                    );
                    self.permanently_failed.insert(*hash, Instant::now());
                } else {
                    warn!(
                        "Missing body {} from peer {} (attempt {}/3)",
                        hash, peer, count
                    );
                    self.failed.push_back(*hash);
                }
            }
        }

        debug!(
            "Received {} bodies from {}, total downloaded: {}",
            bodies.len(),
            peer,
            self.total_downloaded
        );

        bodies
    }

    /// Handle a request timeout
    pub fn handle_timeout(&mut self, peer: &PeerId) {
        if let Some(request) = self.active_requests.remove(peer) {
            warn!("Body request to {} timed out", peer);

            // Move hashes back to failed queue
            for hash in request.hashes {
                self.in_flight.remove(&hash);
                self.failed.push_back(hash);
            }
        }
    }

    /// Get a downloaded block by hash
    pub fn take_block(&mut self, hash: &Hash) -> Option<Block> {
        self.downloaded.remove(hash)
    }

    /// Check if a block has been downloaded
    pub fn has_block(&self, hash: &Hash) -> bool {
        self.downloaded.contains_key(hash)
    }

    /// Get count of downloaded blocks
    pub fn downloaded_count(&self) -> usize {
        self.downloaded.len()
    }

    /// Get count of in-flight requests
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    /// Get total bodies downloaded
    pub fn total_downloaded(&self) -> usize {
        self.total_downloaded
    }

    /// Clean up timed out requests
    pub fn cleanup_timeouts(&mut self) {
        let now = Instant::now();
        let timed_out: Vec<PeerId> = self
            .active_requests
            .iter()
            .filter(|(_, req)| now.duration_since(req.sent_at) > self.request_timeout)
            .map(|(peer, _)| *peer)
            .collect();

        for peer in timed_out {
            self.handle_timeout(&peer);
        }
    }

    /// Cancel a peer's active request and move its hashes back to the failed queue.
    /// Called when a peer disconnects to prevent hashes from being stuck in in_flight.
    pub fn cancel_peer(&mut self, peer: &PeerId) {
        if let Some(request) = self.active_requests.remove(peer) {
            for hash in request.hashes {
                self.in_flight.remove(&hash);
                self.failed.push_back(hash);
            }
        }
    }

    /// Number of permanently failed bodies (active, non-expired)
    pub fn permanently_failed_count(&self) -> usize {
        self.permanently_failed.len()
    }

    /// Check if all needed hashes are permanently failed (no progress possible).
    /// This detects the deadlock where headers_needing_bodies has hashes but
    /// all of them are in permanently_failed — next_request() filters them all
    /// out, returning None forever. The sync manager should transition to Idle.
    pub fn all_needed_permanently_failed(&self, needed: &VecDeque<Hash>) -> bool {
        if needed.is_empty() {
            return false;
        }
        // All needed hashes must be either permanently_failed or already downloaded
        needed
            .iter()
            .all(|h| self.permanently_failed.contains_key(h) || self.downloaded.contains_key(h))
    }

    /// Expire permanently_failed entries and move them back to the retry queue.
    /// Called from cleanup to ensure expiration happens even when next_request()
    /// is not called (e.g., when headers_needing_bodies is effectively empty
    /// because all hashes are filtered by permanently_failed).
    pub fn expire_permanently_failed(&mut self) {
        let now = Instant::now();
        let expired: Vec<Hash> = self
            .permanently_failed
            .iter()
            .filter(|(_, failed_at)| now.duration_since(**failed_at).as_secs() >= 60)
            .map(|(h, _)| *h)
            .collect();
        for hash in expired {
            self.permanently_failed.remove(&hash);
            self.failure_count.remove(&hash);
            self.failed_peers.remove(&hash);
            self.failed.push_back(hash);
        }
    }

    /// Clear all state
    pub fn clear(&mut self) {
        self.active_requests.clear();
        self.in_flight.clear();
        self.downloaded.clear();
        self.failed.clear();
        self.failure_count.clear();
        self.failed_peers.clear();
        self.permanently_failed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::PublicKey;
    use doli_core::BlockHeader;
    use vdf::{VdfOutput, VdfProof};

    fn create_test_block(prev_hash: Hash, slot: u32) -> Block {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let header = BlockHeader {
            version: 1,
            prev_hash,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: now,
            slot,
            producer: PublicKey::from_bytes([0u8; 32]),
            vdf_output: VdfOutput { value: vec![0; 32] },
            vdf_proof: VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
        };

        Block::new(header, vec![])
    }

    #[test]
    fn test_body_downloader_creation() {
        let downloader = BodyDownloader::new(128, 8, Duration::from_secs(30));
        assert_eq!(downloader.downloaded_count(), 0);
        assert_eq!(downloader.in_flight_count(), 0);
    }

    #[test]
    fn test_next_request() {
        let mut downloader = BodyDownloader::new(128, 8, Duration::from_secs(30));

        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");
        let mut needed = VecDeque::new();
        needed.push_back(hash1);
        needed.push_back(hash2);

        let peer = PeerId::random();
        let peers = vec![peer];

        let (selected_peer, request) = downloader.next_request(&needed, &peers).unwrap();
        assert_eq!(selected_peer, peer);

        if let SyncRequest::GetBodies { hashes } = request {
            assert_eq!(hashes.len(), 2);
            assert!(hashes.contains(&hash1));
            assert!(hashes.contains(&hash2));
        } else {
            panic!("Expected GetBodies request");
        }

        assert_eq!(downloader.in_flight_count(), 2);
    }

    #[test]
    fn test_failed_hashes_requeued_when_in_flight() {
        let mut downloader = BodyDownloader::new(128, 8, Duration::from_secs(30));
        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        let mut needed = VecDeque::new();
        needed.push_back(hash1);
        needed.push_back(hash2);

        // Peer1 gets request for hash1+hash2
        let _ = downloader.next_request(&needed, &[peer1]);
        assert_eq!(downloader.in_flight_count(), 2);

        // Peer1 responds with only hash1 (hash2 missing → goes to failed)
        let _block1 = create_test_block(Hash::ZERO, 1);
        // We can't match hash exactly, so simulate via timeout
        downloader.handle_timeout(&peer1);
        // Now hash1 and hash2 are in failed, in_flight is empty

        // Peer2 gets the retry request
        let result = downloader.next_request(&needed, &[peer2]);
        assert!(
            result.is_some(),
            "Should generate retry request from failed queue"
        );
    }

    #[test]
    fn test_cancel_peer_releases_in_flight() {
        let mut downloader = BodyDownloader::new(128, 8, Duration::from_secs(30));
        let hash1 = crypto::hash::hash(b"block1");
        let hash2 = crypto::hash::hash(b"block2");

        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        let mut needed = VecDeque::new();
        needed.push_back(hash1);
        needed.push_back(hash2);

        // Peer1 gets request
        let _ = downloader.next_request(&needed, &[peer1]);
        assert_eq!(downloader.in_flight_count(), 2);

        // Peer1 disconnects — cancel releases hashes back to failed
        downloader.cancel_peer(&peer1);
        assert_eq!(downloader.in_flight_count(), 0);

        // Peer2 can now pick up the hashes
        let result = downloader.next_request(&needed, &[peer2]);
        assert!(result.is_some(), "Peer2 should get the released hashes");
    }

    #[test]
    fn test_process_response() {
        let mut downloader = BodyDownloader::new(128, 8, Duration::from_secs(30));

        let hash1 = crypto::hash::hash(b"block1");
        let mut needed = VecDeque::new();
        needed.push_back(hash1);

        let peer = PeerId::random();
        let peers = vec![peer];

        let _ = downloader.next_request(&needed, &peers);

        let block = create_test_block(Hash::ZERO, 1);
        let block_hash = block.hash();

        // Simulate receiving response with the block
        let mut needed2 = VecDeque::new();
        needed2.push_back(block_hash);

        // First, create a request for the actual block hash
        downloader.clear();
        let _ = downloader.next_request(&needed2, &peers);

        let bodies = downloader.process_response(peer, vec![block]);
        assert_eq!(bodies.len(), 1);
        assert!(downloader.has_block(&block_hash));
    }
}
