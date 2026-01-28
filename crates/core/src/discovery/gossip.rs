//! AdaptiveGossip - Smart Gossip Interval Controller
//!
//! This module implements an adaptive gossip controller that adjusts
//! the gossip interval based on network conditions to optimize bandwidth
//! while maintaining fast convergence.

use std::time::Duration;

use super::MergeResult;

/// Default initial gossip interval (5 seconds).
const DEFAULT_INTERVAL_SECS: u64 = 5;

/// Minimum gossip interval (1 second).
const MIN_INTERVAL_SECS: u64 = 1;

/// Maximum gossip interval (60 seconds).
const MAX_INTERVAL_SECS: u64 = 60;

/// Number of stable rounds before starting to back off.
const BACKOFF_THRESHOLD: u32 = 3;

/// Backoff multiplier for exponential backoff.
const BACKOFF_MULTIPLIER: f64 = 1.5;

/// Network size threshold for enabling delta sync.
const DELTA_SYNC_THRESHOLD: usize = 20;

/// Base stability period in seconds.
const BASE_STABILITY_SECS: u64 = 10;

/// Stability period scaling factor per node.
const STABILITY_SCALE_MS_PER_NODE: u64 = 100;

/// Maximum stability period in seconds.
const MAX_STABILITY_SECS: u64 = 300;

/// An adaptive gossip controller that adjusts intervals based on network activity.
///
/// The controller implements the following strategy:
/// - When new producers are discovered, speed up gossip (min interval)
/// - When the network is stable, slow down gossip (exponential backoff)
/// - When the network is large, enable delta sync for efficiency
///
/// # Example
///
/// ```rust
/// use doli_core::discovery::{AdaptiveGossip, MergeResult};
/// use std::time::Duration;
///
/// let mut gossip = AdaptiveGossip::new();
/// assert_eq!(gossip.interval(), Duration::from_secs(5));
///
/// // Simulate discovering new producers
/// gossip.on_gossip_result(&MergeResult { added: 1, new_producers: 1, rejected: 0, duplicates: 0 }, 10);
/// assert_eq!(gossip.interval(), Duration::from_secs(1));
/// ```
#[derive(Debug, Clone)]
pub struct AdaptiveGossip {
    /// Current gossip interval.
    interval: Duration,

    /// Minimum allowed interval.
    min_interval: Duration,

    /// Maximum allowed interval.
    max_interval: Duration,

    /// Number of consecutive rounds without new producers.
    rounds_without_change: u32,

    /// Estimated number of nodes in the network.
    estimated_network_size: usize,
}

impl Default for AdaptiveGossip {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveGossip {
    /// Create a new adaptive gossip controller with default settings.
    ///
    /// Default settings:
    /// - Initial interval: 5 seconds
    /// - Min interval: 1 second
    /// - Max interval: 60 seconds
    #[must_use]
    pub fn new() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
            min_interval: Duration::from_secs(MIN_INTERVAL_SECS),
            max_interval: Duration::from_secs(MAX_INTERVAL_SECS),
            rounds_without_change: 0,
            estimated_network_size: 0,
        }
    }

    /// Create a new adaptive gossip controller with custom settings.
    ///
    /// # Arguments
    ///
    /// * `initial_interval` - Starting gossip interval
    /// * `min_interval` - Minimum allowed interval
    /// * `max_interval` - Maximum allowed interval
    #[must_use]
    pub fn with_config(
        initial_interval: Duration,
        min_interval: Duration,
        max_interval: Duration,
    ) -> Self {
        Self {
            interval: initial_interval,
            min_interval,
            max_interval,
            rounds_without_change: 0,
            estimated_network_size: 0,
        }
    }

    /// Update the controller based on gossip round results.
    ///
    /// This should be called after each gossip round with the merge result
    /// and the number of peers we gossiped with.
    ///
    /// # Behavior
    ///
    /// - If new producers were added: reset to minimum interval
    /// - If no changes for several rounds: exponential backoff
    /// - Updates estimated network size
    ///
    /// # Arguments
    ///
    /// * `result` - The merge result from the gossip round
    /// * `peer_count` - Number of peers we communicated with
    pub fn on_gossip_result(&mut self, result: &MergeResult, peer_count: usize) {
        // Update estimated network size (simple heuristic: max of peer_count and current)
        self.estimated_network_size = self.estimated_network_size.max(peer_count);

        if result.added > 0 {
            // New producers discovered - speed up gossip
            self.interval = self.min_interval;
            self.rounds_without_change = 0;
        } else {
            // No new producers - consider backing off
            self.rounds_without_change += 1;

            if self.rounds_without_change > BACKOFF_THRESHOLD {
                // Exponential backoff
                let new_interval_ms =
                    (self.interval.as_millis() as f64 * BACKOFF_MULTIPLIER) as u64;
                let new_interval = Duration::from_millis(new_interval_ms);
                self.interval = new_interval.min(self.max_interval);
            }
        }
    }

    /// Get the current recommended gossip interval.
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Check if delta sync should be used.
    ///
    /// Delta sync (using bloom filters) is more efficient for large networks
    /// but has overhead for small networks.
    ///
    /// Returns `true` if the estimated network size exceeds the threshold (20 nodes).
    #[must_use]
    pub fn use_delta_sync(&self) -> bool {
        self.estimated_network_size > DELTA_SYNC_THRESHOLD
    }

    /// Get the recommended stability period before starting block production.
    ///
    /// Larger networks need longer stability periods to ensure all nodes
    /// have converged on the same producer set.
    ///
    /// The formula is: base_stability + (network_size * scale_factor)
    /// Capped at a maximum to prevent excessive delays.
    #[must_use]
    pub fn stability_period(&self) -> Duration {
        let base_ms = BASE_STABILITY_SECS * 1000;
        let scale_ms = self.estimated_network_size as u64 * STABILITY_SCALE_MS_PER_NODE;
        let total_ms = base_ms + scale_ms;
        let max_ms = MAX_STABILITY_SECS * 1000;

        Duration::from_millis(total_ms.min(max_ms))
    }

    /// Get the estimated network size.
    #[must_use]
    pub fn estimated_network_size(&self) -> usize {
        self.estimated_network_size
    }

    /// Get the number of rounds without changes.
    #[must_use]
    pub fn rounds_without_change(&self) -> u32 {
        self.rounds_without_change
    }

    /// Reset the controller to initial state.
    pub fn reset(&mut self) {
        self.interval = Duration::from_secs(DEFAULT_INTERVAL_SECS);
        self.rounds_without_change = 0;
        // Keep estimated_network_size as it's still valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adaptive_initial_state() {
        let gossip = AdaptiveGossip::new();
        assert_eq!(gossip.interval(), Duration::from_secs(5));
        assert!(!gossip.use_delta_sync()); // Network size starts small
    }

    #[test]
    fn test_adaptive_speeds_up_on_changes() {
        let mut gossip = AdaptiveGossip::new();

        // Simulate stable network - should back off
        for _ in 0..5 {
            gossip.on_gossip_result(
                &MergeResult {
                    added: 0,
                    new_producers: 0,
                    rejected: 0,
                    duplicates: 0,
                },
                10,
            );
        }
        let backed_off = gossip.interval();
        assert!(backed_off > Duration::from_secs(5));

        // New producer discovered - should speed up
        gossip.on_gossip_result(
            &MergeResult {
                added: 1,
                new_producers: 1,
                rejected: 0,
                duplicates: 0,
            },
            10,
        );
        assert_eq!(gossip.interval(), Duration::from_secs(1));
    }

    #[test]
    fn test_adaptive_exponential_backoff() {
        let mut gossip = AdaptiveGossip::new();

        // Stable rounds
        for _ in 0..10 {
            gossip.on_gossip_result(&MergeResult::default(), 5);
        }

        // Should be backed off but capped at max
        assert!(gossip.interval() <= Duration::from_secs(60));
        assert!(gossip.interval() > Duration::from_secs(5));
    }

    #[test]
    fn test_adaptive_delta_sync_threshold() {
        let mut gossip = AdaptiveGossip::new();

        assert!(!gossip.use_delta_sync()); // Initially false

        // Simulate large network
        gossip.on_gossip_result(&MergeResult::default(), 50);
        assert!(gossip.use_delta_sync()); // Now true
    }

    #[test]
    fn test_adaptive_stability_period_scales() {
        let mut gossip = AdaptiveGossip::new();

        // Small network
        gossip.on_gossip_result(&MergeResult::default(), 5);
        let small = gossip.stability_period();

        // Large network
        gossip.on_gossip_result(&MergeResult::default(), 100);
        let large = gossip.stability_period();

        assert!(
            large > small,
            "Larger networks need longer stability periods"
        );
    }

    #[test]
    fn test_adaptive_backoff_threshold() {
        let mut gossip = AdaptiveGossip::new();
        let initial = gossip.interval();

        // First 3 rounds should not back off
        for _ in 0..3 {
            gossip.on_gossip_result(&MergeResult::default(), 5);
            assert_eq!(gossip.interval(), initial);
        }

        // 4th round should start backing off
        gossip.on_gossip_result(&MergeResult::default(), 5);
        assert!(gossip.interval() > initial);
    }

    #[test]
    fn test_adaptive_max_interval_cap() {
        let mut gossip = AdaptiveGossip::new();

        // Many stable rounds
        for _ in 0..100 {
            gossip.on_gossip_result(&MergeResult::default(), 5);
        }

        // Should be capped at max
        assert_eq!(gossip.interval(), Duration::from_secs(60));
    }

    #[test]
    fn test_adaptive_reset() {
        let mut gossip = AdaptiveGossip::new();

        // Make some changes
        for _ in 0..10 {
            gossip.on_gossip_result(&MergeResult::default(), 50);
        }
        assert!(gossip.interval() > Duration::from_secs(5));
        assert!(gossip.estimated_network_size() > 0);

        // Reset
        gossip.reset();
        assert_eq!(gossip.interval(), Duration::from_secs(5));
        assert_eq!(gossip.rounds_without_change(), 0);
        // Network size should be preserved
        assert!(gossip.estimated_network_size() > 0);
    }

    #[test]
    fn test_adaptive_default() {
        let gossip = AdaptiveGossip::default();
        assert_eq!(gossip.interval(), Duration::from_secs(5));
    }

    #[test]
    fn test_adaptive_with_config() {
        let gossip = AdaptiveGossip::with_config(
            Duration::from_secs(10),
            Duration::from_secs(2),
            Duration::from_secs(30),
        );
        assert_eq!(gossip.interval(), Duration::from_secs(10));
    }

    #[test]
    fn test_adaptive_network_size_tracking() {
        let mut gossip = AdaptiveGossip::new();

        gossip.on_gossip_result(&MergeResult::default(), 10);
        assert_eq!(gossip.estimated_network_size(), 10);

        // Should track max
        gossip.on_gossip_result(&MergeResult::default(), 5);
        assert_eq!(gossip.estimated_network_size(), 10);

        gossip.on_gossip_result(&MergeResult::default(), 20);
        assert_eq!(gossip.estimated_network_size(), 20);
    }

    #[test]
    fn test_adaptive_stability_period_cap() {
        let mut gossip = AdaptiveGossip::new();

        // Huge network
        gossip.on_gossip_result(&MergeResult::default(), 10000);

        // Should be capped at max
        assert!(gossip.stability_period() <= Duration::from_secs(MAX_STABILITY_SECS));
    }

    #[test]
    fn test_adaptive_rounds_without_change() {
        let mut gossip = AdaptiveGossip::new();
        assert_eq!(gossip.rounds_without_change(), 0);

        gossip.on_gossip_result(&MergeResult::default(), 5);
        assert_eq!(gossip.rounds_without_change(), 1);

        gossip.on_gossip_result(&MergeResult::default(), 5);
        assert_eq!(gossip.rounds_without_change(), 2);

        // Adding something resets the counter
        gossip.on_gossip_result(
            &MergeResult {
                added: 1,
                new_producers: 1,
                rejected: 0,
                duplicates: 0,
            },
            5,
        );
        assert_eq!(gossip.rounds_without_change(), 0);
    }
}
