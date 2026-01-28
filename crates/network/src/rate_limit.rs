//! Rate limiting for peer connections
//!
//! This module implements rate limiting to protect the node from
//! being overwhelmed by excessive requests from peers.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use libp2p::PeerId;
use tracing::debug;

/// Token bucket for rate limiting
#[derive(Clone, Debug)]
pub struct TokenBucket {
    /// Maximum tokens in the bucket
    capacity: u64,
    /// Current number of tokens
    tokens: u64,
    /// Time of last token refill
    last_refill: Instant,
    /// Rate of token refill (tokens per second)
    refill_rate: f64,
}

impl TokenBucket {
    /// Create a new token bucket
    pub fn new(capacity: u64, refill_rate: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            last_refill: Instant::now(),
            refill_rate,
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        let new_tokens = (elapsed * self.refill_rate) as u64;
        if new_tokens > 0 {
            self.tokens = (self.tokens + new_tokens).min(self.capacity);
            self.last_refill = Instant::now();
        }
    }

    /// Try to consume a token, returns true if successful
    pub fn try_consume(&mut self, amount: u64) -> bool {
        self.refill();
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    /// Check if a token can be consumed without actually consuming it
    pub fn can_consume(&mut self, amount: u64) -> bool {
        self.refill();
        self.tokens >= amount
    }

    /// Get current token count
    pub fn tokens(&mut self) -> u64 {
        self.refill();
        self.tokens
    }
}

/// Rate limits for a single peer
#[derive(Debug)]
pub struct PeerLimits {
    /// Block rate limiter
    pub blocks: TokenBucket,
    /// Transaction rate limiter
    pub transactions: TokenBucket,
    /// General request rate limiter
    pub requests: TokenBucket,
    /// Bandwidth rate limiter (bytes)
    pub bandwidth: TokenBucket,
}

impl PeerLimits {
    /// Create peer limits from config
    pub fn new(config: &RateLimitConfig) -> Self {
        Self {
            blocks: TokenBucket::new(
                config.max_blocks_per_minute as u64,
                config.max_blocks_per_minute as f64 / 60.0,
            ),
            transactions: TokenBucket::new(
                config.max_txs_per_second as u64 * 10,
                config.max_txs_per_second as f64,
            ),
            requests: TokenBucket::new(
                config.max_requests_per_second as u64 * 10,
                config.max_requests_per_second as f64,
            ),
            bandwidth: TokenBucket::new(
                config.max_bytes_per_second * 10,
                config.max_bytes_per_second as f64,
            ),
        }
    }
}

/// Configuration for rate limiting
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    /// Maximum blocks per minute from a single peer
    pub max_blocks_per_minute: u32,
    /// Maximum transactions per second from a single peer
    pub max_txs_per_second: u32,
    /// Maximum general requests per second from a single peer
    pub max_requests_per_second: u32,
    /// Maximum bytes per second from a single peer
    pub max_bytes_per_second: u64,
    /// Whether rate limiting is enabled
    pub enabled: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_blocks_per_minute: 10,
            max_txs_per_second: 50,
            max_requests_per_second: 20,
            max_bytes_per_second: 1_048_576, // 1 MB
            enabled: true,
        }
    }
}

/// Rate limiter for all peers
#[derive(Debug)]
pub struct RateLimiter {
    /// Configuration
    config: RateLimitConfig,
    /// Per-peer limits
    limits: HashMap<PeerId, PeerLimits>,
    /// Global block rate limiter
    global_blocks: TokenBucket,
    /// Global transaction rate limiter
    global_transactions: TokenBucket,
    /// Global bandwidth rate limiter
    global_bandwidth: TokenBucket,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            global_blocks: TokenBucket::new(100, 100.0 / 60.0), // 100 blocks/min globally
            global_transactions: TokenBucket::new(1000, 200.0), // 200 tx/sec globally
            global_bandwidth: TokenBucket::new(
                config.max_bytes_per_second * 100,
                config.max_bytes_per_second as f64 * 10.0,
            ),
            limits: HashMap::new(),
            config,
        }
    }

    /// Get or create limits for a peer
    fn get_or_create_limits(&mut self, peer: &PeerId) -> &mut PeerLimits {
        let config = &self.config;
        self.limits
            .entry(*peer)
            .or_insert_with(|| PeerLimits::new(config))
    }

    /// Check if a block can be received from a peer
    pub fn check_block(&mut self, peer: &PeerId) -> bool {
        if !self.config.enabled {
            return true;
        }

        let limits = self.get_or_create_limits(peer);
        if !limits.blocks.can_consume(1) {
            debug!(peer = %peer, "Block rate limit exceeded");
            return false;
        }

        if !self.global_blocks.can_consume(1) {
            debug!("Global block rate limit exceeded");
            return false;
        }

        true
    }

    /// Record a block received from a peer
    pub fn record_block(&mut self, peer: &PeerId, size: usize) {
        if !self.config.enabled {
            return;
        }

        let limits = self.get_or_create_limits(peer);
        limits.blocks.try_consume(1);
        limits.bandwidth.try_consume(size as u64);

        self.global_blocks.try_consume(1);
        self.global_bandwidth.try_consume(size as u64);
    }

    /// Check if a transaction can be received from a peer
    pub fn check_transaction(&mut self, peer: &PeerId) -> bool {
        if !self.config.enabled {
            return true;
        }

        let limits = self.get_or_create_limits(peer);
        if !limits.transactions.can_consume(1) {
            debug!(peer = %peer, "Transaction rate limit exceeded");
            return false;
        }

        if !self.global_transactions.can_consume(1) {
            debug!("Global transaction rate limit exceeded");
            return false;
        }

        true
    }

    /// Record a transaction received from a peer
    pub fn record_transaction(&mut self, peer: &PeerId, size: usize) {
        if !self.config.enabled {
            return;
        }

        let limits = self.get_or_create_limits(peer);
        limits.transactions.try_consume(1);
        limits.bandwidth.try_consume(size as u64);

        self.global_transactions.try_consume(1);
        self.global_bandwidth.try_consume(size as u64);
    }

    /// Check if a request can be made to/from a peer
    pub fn check_request(&mut self, peer: &PeerId) -> bool {
        if !self.config.enabled {
            return true;
        }

        let limits = self.get_or_create_limits(peer);
        if !limits.requests.can_consume(1) {
            debug!(peer = %peer, "Request rate limit exceeded");
            return false;
        }

        true
    }

    /// Record a request from a peer
    pub fn record_request(&mut self, peer: &PeerId, size: usize) {
        if !self.config.enabled {
            return;
        }

        let limits = self.get_or_create_limits(peer);
        limits.requests.try_consume(1);
        limits.bandwidth.try_consume(size as u64);

        self.global_bandwidth.try_consume(size as u64);
    }

    /// Check if bandwidth is available from a peer
    pub fn check_bandwidth(&mut self, peer: &PeerId, bytes: usize) -> bool {
        if !self.config.enabled {
            return true;
        }

        let limits = self.get_or_create_limits(peer);
        if !limits.bandwidth.can_consume(bytes as u64) {
            debug!(peer = %peer, bytes, "Bandwidth rate limit exceeded");
            return false;
        }

        if !self.global_bandwidth.can_consume(bytes as u64) {
            debug!(bytes, "Global bandwidth rate limit exceeded");
            return false;
        }

        true
    }

    /// Remove a peer's rate limits
    pub fn remove_peer(&mut self, peer: &PeerId) {
        self.limits.remove(peer);
    }

    /// Clean up stale peer limits (peers we haven't seen in a while)
    pub fn cleanup(&mut self, max_age: Duration) {
        // This is a simple implementation - in production you'd track last activity
        // For now, we just cap the number of tracked peers
        const MAX_TRACKED_PEERS: usize = 1000;

        if self.limits.len() > MAX_TRACKED_PEERS {
            // Remove oldest entries (this is a simplification)
            let to_remove: Vec<_> = self.limits.keys().take(100).cloned().collect();
            for peer in to_remove {
                self.limits.remove(&peer);
            }
        }
        let _ = max_age; // Suppress unused warning
    }

    /// Get statistics
    pub fn stats(&self) -> RateLimiterStats {
        RateLimiterStats {
            tracked_peers: self.limits.len(),
            enabled: self.config.enabled,
        }
    }
}

/// Statistics from the rate limiter
#[derive(Clone, Debug)]
pub struct RateLimiterStats {
    /// Number of peers being tracked
    pub tracked_peers: usize,
    /// Whether rate limiting is enabled
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_peer_id() -> PeerId {
        PeerId::random()
    }

    #[test]
    fn test_token_bucket() {
        let mut bucket = TokenBucket::new(10, 1.0);

        // Should start full
        assert_eq!(bucket.tokens(), 10);

        // Consume tokens
        assert!(bucket.try_consume(5));
        assert_eq!(bucket.tokens(), 5);

        // Can't consume more than available
        assert!(!bucket.try_consume(6));
        assert_eq!(bucket.tokens(), 5);

        // Can consume exactly what's available
        assert!(bucket.try_consume(5));
        assert_eq!(bucket.tokens(), 0);
    }

    #[test]
    fn test_rate_limiter_blocks() {
        let config = RateLimitConfig {
            max_blocks_per_minute: 5,
            ..Default::default()
        };
        let mut limiter = RateLimiter::new(config);
        let peer = test_peer_id();

        // Should allow initial blocks
        for _ in 0..5 {
            assert!(limiter.check_block(&peer));
            limiter.record_block(&peer, 1000);
        }

        // Should reject after limit
        assert!(!limiter.check_block(&peer));
    }

    #[test]
    fn test_rate_limiter_transactions() {
        let config = RateLimitConfig {
            max_txs_per_second: 10,
            ..Default::default()
        };
        let mut limiter = RateLimiter::new(config);
        let peer = test_peer_id();

        // Should allow initial transactions
        for _ in 0..100 {
            assert!(limiter.check_transaction(&peer));
            limiter.record_transaction(&peer, 500);
        }

        // Should eventually reject
        // (need to consume all tokens in the bucket which has 10x burst capacity)
        let mut rejected = false;
        for _ in 0..200 {
            if !limiter.check_transaction(&peer) {
                rejected = true;
                break;
            }
            limiter.record_transaction(&peer, 500);
        }
        assert!(rejected);
    }

    #[test]
    fn test_rate_limiter_disabled() {
        let config = RateLimitConfig {
            enabled: false,
            ..Default::default()
        };
        let mut limiter = RateLimiter::new(config);
        let peer = test_peer_id();

        // Should always allow when disabled
        for _ in 0..1000 {
            assert!(limiter.check_block(&peer));
            assert!(limiter.check_transaction(&peer));
            assert!(limiter.check_request(&peer));
        }
    }

    #[test]
    fn test_remove_peer() {
        let mut limiter = RateLimiter::new(RateLimitConfig::default());
        let peer = test_peer_id();

        limiter.record_block(&peer, 1000);
        assert_eq!(limiter.stats().tracked_peers, 1);

        limiter.remove_peer(&peer);
        assert_eq!(limiter.stats().tracked_peers, 0);
    }
}
