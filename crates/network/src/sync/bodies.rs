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
            return None;
        }

        // Filter out peers with active requests
        let free_peers: Vec<&PeerId> = available_peers
            .iter()
            .filter(|p| !self.active_requests.contains_key(*p))
            .collect();

        if free_peers.is_empty() {
            return None;
        }

        // Collect hashes to request (not already in flight)
        let mut hashes = Vec::new();

        // First try failed hashes
        while hashes.len() < self.max_bodies_per_request && !self.failed.is_empty() {
            if let Some(hash) = self.failed.pop_front() {
                if !self.in_flight.contains(&hash) {
                    hashes.push(hash);
                }
            }
        }

        // Then new hashes
        for hash in needed.iter() {
            if hashes.len() >= self.max_bodies_per_request {
                break;
            }
            if !self.in_flight.contains(hash) && !self.downloaded.contains_key(hash) {
                hashes.push(*hash);
            }
        }

        if hashes.is_empty() {
            return None;
        }

        // Select peer (round-robin)
        let peer_index = self.next_peer_index % free_peers.len();
        let peer = *free_peers[peer_index];
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

        // Mark missing hashes as failed
        for hash in &request.hashes {
            if !received_hashes.contains(hash) {
                warn!("Missing body {} from peer {}", hash, peer);
                self.in_flight.remove(hash);
                self.failed.push_back(*hash);
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

    /// Clear all state
    pub fn clear(&mut self) {
        self.active_requests.clear();
        self.in_flight.clear();
        self.downloaded.clear();
        self.failed.clear();
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
            merkle_root: Hash::zero(),
            presence_root: Hash::zero(),
            timestamp: now,
            slot,
            producer: PublicKey::from_bytes([0u8; 32]),
            vdf_output: VdfOutput { value: vec![0; 32] },
            vdf_proof: VdfProof::empty(),
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
    fn test_process_response() {
        let mut downloader = BodyDownloader::new(128, 8, Duration::from_secs(30));

        let hash1 = crypto::hash::hash(b"block1");
        let mut needed = VecDeque::new();
        needed.push_back(hash1);

        let peer = PeerId::random();
        let peers = vec![peer];

        let _ = downloader.next_request(&needed, &peers);

        let block = create_test_block(Hash::zero(), 1);
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
