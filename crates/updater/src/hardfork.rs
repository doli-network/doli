//! Hard fork support — deterministic consensus upgrades by height.
//!
//! Hard forks activate at a specific block height. All nodes MUST update
//! before activation_height or they stop producing (safety measure).
//!
//! At 150K nodes, this works because activation is deterministic —
//! every node independently checks `current_height >= activation_height`
//! with zero coordination overhead.

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// A hard fork that activates at a specific block height.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardForkInfo {
    /// Block height at which the fork activates
    pub activation_height: u64,
    /// Minimum binary version required to participate after activation
    pub min_version: String,
    /// Human-readable list of consensus changes in this fork
    pub consensus_changes: Vec<String>,
}

impl HardForkInfo {
    /// Check if this hard fork has activated at the given height.
    pub fn is_active(&self, current_height: u64) -> bool {
        current_height >= self.activation_height
    }

    /// Check if the given version meets the minimum requirement.
    pub fn version_is_compatible(&self, current_version: &str) -> bool {
        !crate::is_newer_version(&self.min_version, current_version)
    }

    /// Check if a node should stop producing at this height with its version.
    ///
    /// Returns `true` if the fork is active and the node's version is too old.
    pub fn should_stop_producing(&self, current_height: u64, current_version: &str) -> bool {
        self.is_active(current_height) && !self.version_is_compatible(current_version)
    }

    /// Blocks remaining until activation (0 if already active).
    pub fn blocks_until_activation(&self, current_height: u64) -> u64 {
        self.activation_height.saturating_sub(current_height)
    }
}

/// Manages a list of known hard forks (sorted by activation height).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HardForkSchedule {
    forks: Vec<HardForkInfo>,
}

impl HardForkSchedule {
    /// Create an empty schedule.
    pub fn new() -> Self {
        Self { forks: Vec::new() }
    }

    /// Add a hard fork to the schedule (maintains sorted order).
    pub fn add(&mut self, fork: HardForkInfo) {
        // Avoid duplicates at same height
        if self
            .forks
            .iter()
            .any(|f| f.activation_height == fork.activation_height)
        {
            warn!(
                "Hard fork at height {} already scheduled, replacing",
                fork.activation_height
            );
            self.forks
                .retain(|f| f.activation_height != fork.activation_height);
        }
        info!(
            "Scheduled hard fork at height {}: min_version={}, changes={:?}",
            fork.activation_height, fork.min_version, fork.consensus_changes
        );
        self.forks.push(fork);
        self.forks.sort_by_key(|f| f.activation_height);
    }

    /// Check if ANY pending fork blocks production at this height/version.
    pub fn should_stop_producing(&self, current_height: u64, current_version: &str) -> bool {
        self.forks
            .iter()
            .any(|f| f.should_stop_producing(current_height, current_version))
    }

    /// Get the next upcoming fork (not yet activated).
    pub fn next_pending(&self, current_height: u64) -> Option<&HardForkInfo> {
        self.forks.iter().find(|f| !f.is_active(current_height))
    }

    /// Get all forks that are active at the given height.
    pub fn active_forks(&self, current_height: u64) -> Vec<&HardForkInfo> {
        self.forks
            .iter()
            .filter(|f| f.is_active(current_height))
            .collect()
    }

    /// Get all scheduled forks.
    pub fn all(&self) -> &[HardForkInfo] {
        &self.forks
    }

    /// Check if schedule is empty.
    pub fn is_empty(&self) -> bool {
        self.forks.is_empty()
    }

    /// Return the compile-time schedule of known hard forks.
    ///
    /// Add entries here when scheduling a consensus-breaking upgrade.
    /// All nodes with the same binary share the same schedule, so
    /// activation is deterministic — no coordination needed.
    ///
    /// Example (uncomment when scheduling a real hard fork):
    /// ```ignore
    /// schedule.add(HardForkInfo {
    ///     activation_height: 100_000,
    ///     min_version: "5.0.0".to_string(),
    ///     consensus_changes: vec!["New reward curve".to_string()],
    /// });
    /// ```
    pub fn default_schedule() -> Self {
        let mut schedule = Self::new();
        schedule.add(HardForkInfo {
            activation_height: 8_450,
            min_version: "5.5.0".to_string(),
            consensus_changes: vec![
                "EpochReward TX requires explicit pool UTXO inputs (UTXO auditability)".to_string(),
            ],
        });
        schedule
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fork(height: u64, version: &str) -> HardForkInfo {
        HardForkInfo {
            activation_height: height,
            min_version: version.to_string(),
            consensus_changes: vec!["test change".to_string()],
        }
    }

    #[test]
    fn test_hardfork_activation() {
        let fork = sample_fork(100, "2.0.0");

        assert!(!fork.is_active(99));
        assert!(fork.is_active(100));
        assert!(fork.is_active(101));
    }

    #[test]
    fn test_hardfork_version_check() {
        let fork = sample_fork(100, "2.0.0");

        // 2.0.0 meets 2.0.0
        assert!(fork.version_is_compatible("2.0.0"));
        // 3.0.0 exceeds 2.0.0
        assert!(fork.version_is_compatible("3.0.0"));
        // 1.9.9 does NOT meet 2.0.0
        assert!(!fork.version_is_compatible("1.9.9"));
    }

    #[test]
    fn test_should_stop_producing() {
        let fork = sample_fork(100, "2.0.0");

        // Before activation: never stop
        assert!(!fork.should_stop_producing(99, "1.0.0"));
        // At activation with old version: stop
        assert!(fork.should_stop_producing(100, "1.9.9"));
        // At activation with correct version: don't stop
        assert!(!fork.should_stop_producing(100, "2.0.0"));
        // After activation with old version: stop
        assert!(fork.should_stop_producing(200, "1.0.0"));
        // After activation with new version: don't stop
        assert!(!fork.should_stop_producing(200, "2.1.0"));
    }

    #[test]
    fn test_blocks_until_activation() {
        let fork = sample_fork(100, "2.0.0");

        assert_eq!(fork.blocks_until_activation(50), 50);
        assert_eq!(fork.blocks_until_activation(99), 1);
        assert_eq!(fork.blocks_until_activation(100), 0);
        assert_eq!(fork.blocks_until_activation(200), 0);
    }

    #[test]
    fn test_schedule_add_and_order() {
        let mut schedule = HardForkSchedule::new();
        schedule.add(sample_fork(200, "3.0.0"));
        schedule.add(sample_fork(100, "2.0.0"));

        // Should be sorted by height
        assert_eq!(schedule.all().len(), 2);
        assert_eq!(schedule.all()[0].activation_height, 100);
        assert_eq!(schedule.all()[1].activation_height, 200);
    }

    #[test]
    fn test_schedule_duplicate_height_replaces() {
        let mut schedule = HardForkSchedule::new();
        schedule.add(sample_fork(100, "2.0.0"));
        schedule.add(sample_fork(100, "2.1.0"));

        assert_eq!(schedule.all().len(), 1);
        assert_eq!(schedule.all()[0].min_version, "2.1.0");
    }

    #[test]
    fn test_schedule_stop_producing() {
        let mut schedule = HardForkSchedule::new();
        schedule.add(sample_fork(100, "2.0.0"));
        schedule.add(sample_fork(200, "3.0.0"));

        // Before any fork: fine
        assert!(!schedule.should_stop_producing(50, "1.0.0"));
        // At first fork with v1: stop
        assert!(schedule.should_stop_producing(100, "1.0.0"));
        // At first fork with v2: fine (meets 2.0.0 but not at 200 yet)
        assert!(!schedule.should_stop_producing(100, "2.0.0"));
        // At second fork with v2: stop (doesn't meet 3.0.0)
        assert!(schedule.should_stop_producing(200, "2.0.0"));
        // At second fork with v3: fine
        assert!(!schedule.should_stop_producing(200, "3.0.0"));
    }

    #[test]
    fn test_schedule_next_pending() {
        let mut schedule = HardForkSchedule::new();
        schedule.add(sample_fork(100, "2.0.0"));
        schedule.add(sample_fork(200, "3.0.0"));

        // Before all forks
        let next = schedule.next_pending(50).unwrap();
        assert_eq!(next.activation_height, 100);

        // After first fork
        let next = schedule.next_pending(150).unwrap();
        assert_eq!(next.activation_height, 200);

        // After all forks
        assert!(schedule.next_pending(300).is_none());
    }

    #[test]
    fn test_schedule_active_forks() {
        let mut schedule = HardForkSchedule::new();
        schedule.add(sample_fork(100, "2.0.0"));
        schedule.add(sample_fork(200, "3.0.0"));

        assert_eq!(schedule.active_forks(50).len(), 0);
        assert_eq!(schedule.active_forks(100).len(), 1);
        assert_eq!(schedule.active_forks(200).len(), 2);
    }
}
