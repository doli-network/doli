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

/// D2 (INC-I-090): chain break info captured during header validation.
///
/// Contains the hashes and slot needed for the `ChainBreakDetected`
/// diagnostic event. The peer ID is added by the caller (sync engine)
/// since the downloader doesn't track which peer sent the headers.
#[derive(Debug, Clone)]
pub struct HeaderChainBreak {
    /// The hash we expected this header's prev_hash to be.
    pub expected: Hash,
    /// The actual prev_hash from the received header.
    pub actual: Hash,
    /// The slot of the header that caused the break.
    pub header_slot: u32,
    /// How many headers were validated before the break in this batch.
    pub valid_so_far: u32,
}

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
    /// D2 (INC-I-090): chain break detected in the most recent `process_headers` call.
    /// Cleared at the start of each call; populated if a break occurs.
    last_chain_break: Option<HeaderChainBreak>,
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
            last_chain_break: None,
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

    /// Process received headers, returning count of valid headers.
    ///
    /// D2 (INC-I-090): when a chain break is detected (header.prev_hash != expected),
    /// `self.last_chain_break` is populated with the break details. The caller
    /// should check `take_chain_break()` after this call.
    pub fn process_headers(&mut self, headers: &[BlockHeader], local_tip: Hash) -> usize {
        // D2: clear previous chain break info
        self.last_chain_break = None;

        if headers.is_empty() {
            return 0;
        }

        let mut valid_count = 0u32;
        let mut prev_hash = self.expected_prev_hash.unwrap_or(local_tip);

        for header in headers {
            // Check chain linkage
            if header.prev_hash != prev_hash {
                warn!(
                    "[HEADER_DEBUG] Chain break: header.prev_hash={} expected={} header_slot={} valid_so_far={}",
                    header.prev_hash, prev_hash, header.slot, valid_count
                );
                // D2 (INC-I-090): capture chain break info for diagnostic emission
                self.last_chain_break = Some(HeaderChainBreak {
                    expected: prev_hash,
                    actual: header.prev_hash,
                    header_slot: header.slot,
                    valid_so_far: valid_count,
                });
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
            self.total_downloaded += valid_count as usize;
            debug!(
                "Validated {} headers, total: {}",
                valid_count, self.total_downloaded
            );
        }

        valid_count as usize
    }

    /// D2 (INC-I-090): Take the chain break info from the last `process_headers` call.
    ///
    /// Returns `Some(HeaderChainBreak)` if a break was detected, `None` otherwise.
    /// Consumes the break info (subsequent calls return None until the next break).
    pub fn take_chain_break(&mut self) -> Option<HeaderChainBreak> {
        self.last_chain_break.take()
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
        self.last_chain_break = None;
        info!(
            "[HEADER_DEBUG] clear() called, expected_prev_hash=None (will use local_tip on next process)"
        );
    }

    /// Get the hash we expect next header to follow
    pub fn expected_prev_hash(&self) -> Option<Hash> {
        self.expected_prev_hash
    }

    /// Resume downloading from a specific hash (after preserving sync data).
    /// Sets expected_prev_hash so the next request continues from this point
    /// instead of re-downloading from local_tip.
    pub fn resume_from(&mut self, hash: Hash) {
        self.expected_prev_hash = Some(hash);
        // Clear internal buffers — validated_headers should already be drained
        // into pipeline.pending_headers, and known_hashes is stale after restart.
        self.validated_headers.clear();
        self.known_hashes.clear();
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
            fork_id: Hash::ZERO,
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

        let genesis = Hash::ZERO;
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

        let genesis = Hash::ZERO;
        let header1 = create_test_header(genesis, 1);
        // header2 has wrong prev_hash
        let header2 = create_test_header(Hash::ZERO, 2);

        let valid = downloader.process_headers(&[header1, header2], genesis);
        assert_eq!(valid, 1); // Only first header is valid
    }

    /// D2 (INC-I-090): verify chain break info is captured on break.
    #[test]
    fn test_chain_break_captured() {
        let mut downloader = HeaderDownloader::new(2000, Duration::from_secs(30));

        let genesis = Hash::ZERO;
        let header1 = create_test_header(genesis, 1);
        let _hash1 = header1.hash();
        // header2 has wrong prev_hash — points to genesis instead of hash1
        let header2 = create_test_header(Hash::from_bytes([0xAA; 32]), 2);

        let valid = downloader.process_headers(&[header1, header2], genesis);
        assert_eq!(valid, 1);

        let chain_break = downloader.take_chain_break();
        assert!(
            chain_break.is_some(),
            "chain break should be captured when header.prev_hash mismatches"
        );
        let cb = chain_break.unwrap();
        assert_eq!(cb.header_slot, 2);
        assert_eq!(cb.valid_so_far, 1);
        assert_ne!(cb.expected, cb.actual);
    }

    /// D2 (INC-I-090): no chain break on valid headers.
    #[test]
    fn test_no_chain_break_on_valid_headers() {
        let mut downloader = HeaderDownloader::new(2000, Duration::from_secs(30));

        let genesis = Hash::ZERO;
        let header1 = create_test_header(genesis, 1);
        let hash1 = header1.hash();
        let header2 = create_test_header(hash1, 2);

        let _valid = downloader.process_headers(&[header1, header2], genesis);
        assert!(
            downloader.take_chain_break().is_none(),
            "no chain break should be captured for valid headers"
        );
    }
}
