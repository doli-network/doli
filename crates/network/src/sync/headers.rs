//! Header-first download for chain synchronization
//!
//! Downloads and validates block headers before requesting bodies.
//! This allows for efficient validation of the VDF chain without
//! downloading full block data.

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use crypto::Hash;
use doli_core::BlockHeader;
use tracing::{debug, info, warn};

use crate::protocols::SyncRequest;

/// Header downloader state
pub struct HeaderDownloader {
    /// Maximum headers per request
    max_headers_per_request: u32,
    /// Request timeout
    #[allow(dead_code)]
    request_timeout: Duration,
    /// Validated header chain (in order)
    validated_headers: VecDeque<BlockHeader>,
    /// Set of known header hashes
    known_hashes: HashSet<Hash>,
    /// Expected next header's prev_hash
    expected_prev_hash: Option<Hash>,
    /// Total headers downloaded
    total_downloaded: usize,
}

impl HeaderDownloader {
    /// Create a new header downloader
    pub fn new(max_headers_per_request: u32, request_timeout: Duration) -> Self {
        Self {
            max_headers_per_request,
            request_timeout,
            validated_headers: VecDeque::new(),
            known_hashes: HashSet::new(),
            expected_prev_hash: None,
            total_downloaded: 0,
        }
    }

    /// Create a header request starting from the given hash
    /// Uses expected_prev_hash if set (for continuation), otherwise uses provided start_hash
    pub fn create_request(&self, start_hash: Hash) -> Option<SyncRequest> {
        // If we've already processed some headers, continue from where we left off
        let request_from = self.expected_prev_hash.unwrap_or(start_hash);
        Some(SyncRequest::GetHeaders {
            start_hash: request_from,
            max_count: self.max_headers_per_request,
        })
    }

    /// Process received headers, returning count of valid headers
    pub fn process_headers(&mut self, headers: &[BlockHeader], local_tip: Hash) -> usize {
        if headers.is_empty() {
            return 0;
        }

        let mut valid_count = 0;
        let mut prev_hash = self.expected_prev_hash.unwrap_or(local_tip);

        for header in headers {
            // Check chain linkage
            if header.prev_hash != prev_hash {
                warn!(
                    "[HEADER_DEBUG] Chain break: header.prev_hash={} expected={} header_slot={} valid_so_far={}",
                    header.prev_hash, prev_hash, header.slot, valid_count
                );
                break;
            }

            let hash = header.hash();

            // Skip if we already have this header
            if self.known_hashes.contains(&hash) {
                debug!("Skipping known header {}", hash);
                prev_hash = hash;
                continue;
            }

            // Basic header validation
            if !self.validate_header(header) {
                warn!("Invalid header {}", hash);
                break;
            }

            // Add to validated chain
            self.validated_headers.push_back(header.clone());
            self.known_hashes.insert(hash);
            prev_hash = hash;
            valid_count += 1;
        }

        if valid_count > 0 {
            self.expected_prev_hash = Some(prev_hash);
            self.total_downloaded += valid_count;
            debug!(
                "Validated {} headers, total: {}",
                valid_count, self.total_downloaded
            );
        }

        valid_count
    }

    /// Validate a single header
    fn validate_header(&self, header: &BlockHeader) -> bool {
        // Check version
        if header.version == 0 {
            return false;
        }

        // Check timestamp is not in the far future (1 hour tolerance)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if header.timestamp > now + 3600 {
            warn!("Header timestamp too far in future");
            return false;
        }

        // VDF validation would happen here, but requires the VDF verifier
        // For now, we trust the header and validate VDF when applying the block

        true
    }

    /// Get the next batch of validated headers for processing
    pub fn take_headers(&mut self, count: usize) -> Vec<BlockHeader> {
        let count = count.min(self.validated_headers.len());
        self.validated_headers.drain(..count).collect()
    }

    /// Get the total count of validated headers waiting
    pub fn pending_count(&self) -> usize {
        self.validated_headers.len()
    }

    /// Get total headers downloaded
    pub fn total_downloaded(&self) -> usize {
        self.total_downloaded
    }

    /// Clear the downloader state
    pub fn clear(&mut self) {
        self.validated_headers.clear();
        self.known_hashes.clear();
        self.expected_prev_hash = None;
        info!(
            "[HEADER_DEBUG] clear() called, expected_prev_hash=None (will use local_tip on next process)"
        );
    }

    /// Get the hash we expect next header to follow
    pub fn expected_prev_hash(&self) -> Option<Hash> {
        self.expected_prev_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::PublicKey;
    use std::time::Duration;
    use vdf::{VdfOutput, VdfProof};

    fn create_test_header(prev_hash: Hash, slot: u32) -> BlockHeader {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        BlockHeader {
            version: 1,
            prev_hash,
            merkle_root: Hash::zero(),
            presence_root: Hash::zero(),
            timestamp: now,
            slot,
            producer: PublicKey::from_bytes([0u8; 32]),
            vdf_output: VdfOutput { value: vec![0; 32] },
            vdf_proof: VdfProof::empty(),
        }
    }

    #[test]
    fn test_header_downloader_creation() {
        let downloader = HeaderDownloader::new(2000, Duration::from_secs(30));
        assert_eq!(downloader.pending_count(), 0);
        assert_eq!(downloader.total_downloaded(), 0);
    }

    #[test]
    fn test_process_valid_headers() {
        let mut downloader = HeaderDownloader::new(2000, Duration::from_secs(30));

        let genesis = Hash::zero();
        let header1 = create_test_header(genesis, 1);
        let hash1 = header1.hash();
        let header2 = create_test_header(hash1, 2);

        let valid = downloader.process_headers(&[header1, header2], genesis);
        assert_eq!(valid, 2);
        assert_eq!(downloader.pending_count(), 2);
    }

    #[test]
    fn test_process_broken_chain() {
        let mut downloader = HeaderDownloader::new(2000, Duration::from_secs(30));

        let genesis = Hash::zero();
        let header1 = create_test_header(genesis, 1);
        // header2 has wrong prev_hash
        let header2 = create_test_header(Hash::zero(), 2);

        let valid = downloader.process_headers(&[header1, header2], genesis);
        assert_eq!(valid, 1); // Only first header is valid
    }
}
