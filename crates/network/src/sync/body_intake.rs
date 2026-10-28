//! Body response intake: only requested hashes count, and a refused body stays needed.

use std::collections::HashSet;
use std::time::Instant;

use crypto::Hash;
use doli_core::Block;
use libp2p::PeerId;
use tracing::{debug, warn};

use super::BodyDownloader;

impl BodyDownloader {
    /// Bodies outside `peer`'s active request are dropped. A body `refuse(hash, body)`
    /// holds for is not stored, spends no retry budget, and is next asked of another peer.
    pub fn process_response_refusing(
        &mut self,
        peer: PeerId,
        bodies: Vec<Block>,
        refuse: impl Fn(&Hash, &Block) -> bool,
    ) -> Vec<Block> {
        let Some(request) = self.active_requests.remove(&peer) else {
            warn!(
                "Dropping {} bodies from {}: no active request",
                bodies.len(),
                peer
            );
            return Vec::new();
        };

        let total = bodies.len();
        let mut received_hashes = HashSet::new();
        let mut refused = HashSet::new();
        let mut accepted = Vec::with_capacity(total);
        for block in bodies {
            let hash = block.hash();
            if !request.hashes.contains(&hash) {
                warn!("Dropping unrequested body {} from {}", hash, peer);
                continue;
            }
            if refuse(&hash, &block) {
                warn!(
                    "Refusing body {} from {}: invalid block signature or a body that does not match its header",
                    hash, peer
                );
                refused.insert(hash);
                continue;
            }
            received_hashes.insert(hash);
            self.in_flight.remove(&hash);
            self.downloaded.insert(hash, block.clone());
            self.total_downloaded += 1;
            accepted.push(block);
        }

        // Mark missing hashes as failed — track per-hash failure count
        for hash in &request.hashes {
            if received_hashes.contains(hash) {
                continue;
            }
            self.in_flight.remove(hash);
            self.failed_peers.entry(*hash).or_default().insert(peer);
            if refused.contains(hash) {
                continue;
            }

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

        debug!(
            "Received {} bodies from {}, total downloaded: {}",
            total, peer, self.total_downloaded
        );

        accepted
    }
}
