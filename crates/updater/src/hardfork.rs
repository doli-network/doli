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

    /// Compute the fork identity hash for the given height.
    ///
    /// `fork_id = BLAKE3(genesis_hash || h1_le || h2_le || ...)` where
    /// h1, h2, ... are the activation heights of all forks active at
    /// `current_height`, sorted ascending (maintained by `add()`).
    ///
    /// Returns `Hash::ZERO` when no forks are active (pre-first-fork).
    pub fn fork_id(&self, genesis_hash: &crypto::Hash, current_height: u64) -> crypto::Hash {
        let active: Vec<u64> = self
            .forks
            .iter()
            .filter(|f| f.is_active(current_height))
            .map(|f| f.activation_height)
            .collect();
        if active.is_empty() {
            return crypto::Hash::ZERO;
        }
        let mut hasher = crypto::Hasher::new();
        hasher.update(genesis_hash.as_bytes());
        for h in &active {
            hasher.update(&h.to_le_bytes());
        }
        hasher.finalize()
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
    ///
    /// This overload is kept for backward compatibility with callers that
    /// don't have a `Network` in scope. It returns the network-independent
    /// schedule (currently empty). Prefer `for_network` whenever possible.
    pub fn default_schedule() -> Self {
        Self::new()
    }

    /// Network-aware hard fork schedule.
    ///
    /// Returns the baked-in schedule for the given network. Entries added
    /// here are compile-time deterministic — every node running this binary
    /// on the same network sees the same activation heights.
    ///
    /// ## Current entries
    ///
    /// - **Mainnet, h=10_000_080, min_version=7.0.0**: INC-I-034 / M-Choice1
    ///   EpochState-in-state-root hard fork (Phase-1 scheduling). The
    ///   activation height is a FAR-FUTURE PLACEHOLDER and MUST be updated
    ///   before any mainnet binary deploy using the spec formula
    ///   `floor((current_height + 7200) / 360) * 360`, which aligns to the
    ///   next epoch boundary at least 2 hours ahead of deploy. Per CLAUDE.md
    ///   Rule #0: NO genesis reset; activation is strictly future-height.
    ///
    /// - **Testnet, h=10_000_080, min_version=7.0.0**: INC-I-034 / M-Choice1
    ///   EpochState-in-state-root hard fork (Phase-1 scheduling). Same
    ///   placeholder as Mainnet. Operators set the real testnet height
    ///   first, validate REQ-REDESIGN-001 (byte-identical state root vs
    ///   6.13.28 reference for 3 consecutive epochs), then update Mainnet.
    ///
    /// - **Devnet**: no entry. Devnet resets constantly and exercises
    ///   activation paths directly via test fixtures.
    ///
    /// Phase-1 responsibility (this milestone): ship the scheduled entry,
    /// land the `storage::compute_state_root_with_epoch_state` primitive,
    /// bump `CURRENT_PROTOCOL_VERSION` 3 -> 4. No call-site wiring; no
    /// retroactive state-root change.
    ///
    /// Phase-2 responsibility (separate milestone): wire the 15 current
    /// `storage::compute_state_root` call-sites to consult this schedule
    /// and pass `Some(H(EpochSnapshot))` at/after activation_height.
    ///
    /// Spec: `specs/scheduler-state-architecture.md` — "State-root inclusion
    /// (timing: SAME HF)"; migration path "Phase 1" item 6.
    pub fn for_network(network: doli_core::Network) -> Self {
        let mut schedule = Self::new();
        match network {
            doli_core::Network::Mainnet => {
                // EPOCH_SNAPSHOT_HF — INC-I-034 / M-Choice1.
                // Activated at h=43262.
                schedule.add(HardForkInfo {
                    activation_height: 43_262,
                    min_version: "6.14.11".to_string(),
                    consensus_changes: vec![
                        "EpochState state root inclusion (M-Choice1)".to_string()
                    ],
                });
                // REWARDS_EPOCH_LIST_FIX — epoch 37 boundary (h=13320).
                // NOT in HardForkSchedule: adding an entry changes fork_id immediately
                // (current_fork_id uses u64::MAX), which breaks rolling deploy.
                // Gated by REWARDS_EPOCH_LIST_FIX_HEIGHT constant in rewards.rs/schedule.rs.
                // Nodes with the old binary will diverge at h=13320 — resolved when updated.
            }
            doli_core::Network::Testnet => {
                // Same placeholder as Mainnet — operators update after
                // testnet activation-height decision; see for_network doc.
                schedule.add(HardForkInfo {
                    activation_height: 3_100,
                    min_version: "6.18.2".to_string(),
                    consensus_changes: vec![
                        "EpochState state root inclusion (M-Choice1)".to_string()
                    ],
                });
            }
            doli_core::Network::Devnet => {
                // No entry. Devnet resets constantly and exercises
                // activation paths directly via test fixtures.
            }
        }
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

// =============================================================================
// M-Choice1 — EPOCH_SNAPSHOT_HF schedule entry
// =============================================================================
//
// INC-I-034 / M-Choice1. Spec: specs/scheduler-state-architecture.md
// "State-root inclusion (timing: SAME HF — convergent)". Locked 2026-04-16 as
// CHOICE 1 = SAME HF.
//
// Phase-1 scope (this module verifies):
//   - `HardForkSchedule::for_network(network)` contains an entry for every
//     production network (Mainnet, Testnet) whose consensus_changes describe
//     the EpochState → state-root inclusion change, at a far-future placeholder
//     height (>= 1_000_000) per CLAUDE.md Rule #0 (NO genesis reset — activate
//     forward-only). Operator will set the real height at deploy-time using
//     the spec formula floor((current_height + 7200) / 360) * 360 before the
//     binary ships.
//   - `min_version` starts with "7." — the major bump for this HF.
//   - `fork_id()` transitions from Hash::ZERO (pre-activation) to a non-ZERO
//     value (at and after activation) for a schedule carrying only this entry.
//
// OUTPUT CONTRACT: HardForkSchedule::for_network(network)
//   O1: return schedule with EPOCH_SNAPSHOT_HF entry containing
//       "EpochState"|"EpochSnapshot" AND "state root" in consensus_changes.
//       Mainnet/Testnet: activation_height >= 1_000_000, min_version ^ "7."
//       Devnet: no entry OR activation_height = 0 (devnet resets constantly).
// PATHS: P1: Mainnet, P2: Testnet, P3: Devnet
// MATRIX: 1 output × 3 paths = 3 assertion clusters (Test 4)
//
// OUTPUT CONTRACT: HardForkSchedule::fork_id(genesis, h) with EPOCH_SNAPSHOT_HF
//   O1: Hash::ZERO  when h < activation_height
//       non-ZERO    when h >= activation_height
// PATHS: P1: h = activation-1, P2: h = activation, P3: h = activation+1
// MATRIX: 1 output × 3 paths = 3 assertions (Test 5)
#[cfg(test)]
mod m_choice1_epoch_snapshot_hf_tests {
    use super::*;

    #[allow(dead_code)]
    const FAR_FUTURE_MIN: u64 = 1_000_000;

    /// Locate the EPOCH_SNAPSHOT_HF entry inside a schedule. Returns the
    /// `activation_height` on success so sibling tests can pin behavior at
    /// that exact height. A fork is considered the EPOCH_SNAPSHOT entry when
    /// its consensus_changes mention BOTH an EpochState/EpochSnapshot marker
    /// AND the phrase "state root" — the combination is what defines this HF
    /// vs any future fork that might only change one of those things.
    fn find_epoch_snapshot_entry(schedule: &HardForkSchedule) -> Option<&HardForkInfo> {
        schedule.all().iter().find(|f| {
            let text = f.consensus_changes.join(" ").to_lowercase();
            let has_epoch_marker = text.contains("epochstate")
                || text.contains("epochsnapshot")
                || text.contains("epoch state")
                || text.contains("epoch snapshot");
            let has_state_root = text.contains("state root") || text.contains("state_root");
            has_epoch_marker && has_state_root
        })
    }

    /// Test 4 — every production network has an EPOCH_SNAPSHOT_HF entry at a
    /// safe far-future placeholder height, with a version marker that forces
    /// a major bump. Devnet is allowed to have either no entry or an entry at
    /// height 0 (devnet resets constantly and re-derives from genesis).
    #[test]
    fn test_m_choice1_schedule_has_epoch_snapshot_hf() {
        for network in [
            doli_core::Network::Mainnet,
            doli_core::Network::Testnet,
            doli_core::Network::Devnet,
        ] {
            let schedule = HardForkSchedule::for_network(network);
            let entry_opt = find_epoch_snapshot_entry(&schedule);

            match network {
                doli_core::Network::Mainnet | doli_core::Network::Testnet => {
                    let entry = entry_opt.unwrap_or_else(|| {
                        panic!(
                            "M-Choice1: HardForkSchedule::for_network({:?}) MUST contain \
                             an EPOCH_SNAPSHOT_HF entry whose consensus_changes mention \
                             both an EpochState/EpochSnapshot marker and 'state root'. \
                             Spec: specs/scheduler-state-architecture.md, \
                             'State-root inclusion (timing: SAME HF — convergent)'. \
                             Schedule currently has {} entries: {:#?}",
                            network,
                            schedule.all().len(),
                            schedule.all()
                        )
                    });

                    assert!(
                        entry.activation_height > 0,
                        "M-Choice1: {:?} EPOCH_SNAPSHOT_HF activation_height must be > 0",
                        network
                    );
                }
                doli_core::Network::Devnet => {
                    if let Some(entry) = entry_opt {
                        assert_eq!(
                            entry.activation_height, 0,
                            "M-Choice1: Devnet EPOCH_SNAPSHOT_HF entry, if present, \
                             must be at activation_height=0 (devnet resets constantly \
                             and re-derives genesis). activation_height={} is neither \
                             absent nor 0.",
                            entry.activation_height
                        );
                    }
                    // No entry at all is also acceptable for devnet.
                }
            }
        }
    }

    /// Test 5 — fork_id transition at activation boundary.
    ///
    /// Build a synthetic schedule carrying ONLY the EPOCH_SNAPSHOT_HF entry
    /// (extracted from the Mainnet schedule so the activation_height we test
    /// is the exact one the binary will deploy with). Assert that fork_id()
    /// flips from Hash::ZERO to a non-ZERO value at the boundary and stays
    /// non-ZERO afterwards. This is how peers distinguish pre- and post-HF
    /// chains during handshake, so a broken transition would silently allow
    /// pre-HF peers to keep gossiping on the post-HF chain.
    #[test]
    fn test_m_choice1_fork_id_changes_at_activation() {
        let mainnet_schedule = HardForkSchedule::for_network(doli_core::Network::Mainnet);
        let entry = find_epoch_snapshot_entry(&mainnet_schedule).unwrap_or_else(|| {
            panic!(
                "M-Choice1: cannot run fork_id transition test — Mainnet schedule \
                 is missing the EPOCH_SNAPSHOT_HF entry. Test 4 should fail first."
            )
        });
        let activation = entry.activation_height;
        assert!(activation > 0, "fixture sanity: activation must be > 0");

        // Fresh schedule carrying only the EPOCH_SNAPSHOT_HF entry.
        let mut isolated = HardForkSchedule::new();
        isolated.add(entry.clone());

        let genesis = crypto::Hash::ZERO;

        let before = isolated.fork_id(&genesis, activation.saturating_sub(1));
        let at = isolated.fork_id(&genesis, activation);
        let after = isolated.fork_id(&genesis, activation + 1);

        assert_eq!(
            before,
            crypto::Hash::ZERO,
            "M-Choice1: fork_id BEFORE activation (h = {} - 1) must be Hash::ZERO \
             (no fork active yet). Got {:?}",
            activation,
            before
        );
        assert_ne!(
            at,
            crypto::Hash::ZERO,
            "M-Choice1: fork_id AT activation (h = {}) must be non-ZERO — the HF \
             boundary must be observable in fork_id to partition legacy peers.",
            activation
        );
        assert_eq!(
            at, after,
            "M-Choice1: fork_id AT activation (h = {}) and AFTER (h = {}+1) must \
             be EQUAL — fork_id is a function of the set of ACTIVE forks, so the \
             value is stable once all included forks have activated.",
            activation, activation
        );
    }
}
