//! Memory watchdog: shed gossip blocks under memory pressure (INC-I-114 M2).
//!
//! Complements M1's queue-bound load shedding with a process-level memory
//! pressure signal. When resident memory crosses a configurable SOFT threshold
//! (set well below the kernel OOM ceiling), the watchdog trips a shared
//! `AtomicBool` flag that the gossip hot path reads to shed ALL inbound blocks
//! until memory recovers.
//!
//! ## Design
//!
//! - **Pure decision core** (`evaluate`) — no I/O, unit-testable via injected
//!   byte values.
//! - **Injectable sampler** — `MemoryWatchdog` accepts a closure returning
//!   `Option<u64>` (resident bytes). `None` = unavailable → fail-open (never
//!   shed). Real sampler reads `/proc/self/statm` on Linux; returns `None`
//!   on non-Linux (macOS dev).
//! - **Edge-triggered logging** — WARN on Healthy→Shedding transition, INFO
//!   on recovery. No per-tick log spam.
//! - **Fail-open** — if the sampler is unavailable, the watchdog never trips.
//!   This mirrors the genesis_time=0 fail-open pattern in gossip validation.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tracing::{info, warn};

/// Watchdog evaluation result — pure enum, no side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogState {
    /// Memory usage is below the soft threshold.
    Healthy,
    /// Memory usage is at or above the soft threshold — shed gossip blocks.
    Shedding,
}

/// Pure decision function: returns `Shedding` iff `current_bytes >= soft_threshold_bytes`.
///
/// Boundary semantics: the threshold value itself triggers shedding. This is
/// intentional — the threshold is a SOFT limit set below the OOM ceiling, so
/// "at threshold = already too high" is the conservative choice.
pub fn evaluate(current_bytes: u64, soft_threshold_bytes: u64) -> WatchdogState {
    if current_bytes >= soft_threshold_bytes {
        WatchdogState::Shedding
    } else {
        WatchdogState::Healthy
    }
}

/// Memory watchdog that periodically samples process RSS and trips a shared
/// shed flag when memory pressure is detected.
pub struct MemoryWatchdog {
    /// Shared flag: true = shed all gossip blocks in the hot path.
    shed_flag: Arc<AtomicBool>,
    /// Count of Healthy→Shedding transitions (edge-triggered).
    trip_count: Arc<AtomicU64>,
    /// Soft memory threshold in bytes. 0 = disabled (never trips).
    soft_threshold_bytes: u64,
    /// Injectable sampler: returns `Some(resident_bytes)` or `None` if unavailable.
    sample: Arc<dyn Fn() -> Option<u64> + Send + Sync>,
    /// Previous state for edge-triggered logging.
    was_shedding: bool,
}

impl MemoryWatchdog {
    /// Create a new watchdog with an injectable sampler.
    ///
    /// - `soft_threshold_bytes`: 0 = disabled (tick is a no-op).
    /// - `sampler`: returns `Some(resident_bytes)` or `None`.
    pub fn new(
        soft_threshold_bytes: u64,
        sampler: Arc<dyn Fn() -> Option<u64> + Send + Sync>,
    ) -> Self {
        Self {
            shed_flag: Arc::new(AtomicBool::new(false)),
            trip_count: Arc::new(AtomicU64::new(0)),
            soft_threshold_bytes,
            sample: sampler,
            was_shedding: false,
        }
    }

    /// Create a watchdog wired to the real platform sampler.
    ///
    /// On Linux: reads `/proc/self/statm` for resident pages × page_size.
    /// On non-Linux: sampler returns `None` (fail-open, logs once).
    pub fn with_real_sampler(soft_threshold_bytes: u64) -> Self {
        Self::new(soft_threshold_bytes, Arc::new(sample_resident_bytes))
    }

    /// Periodic tick — call from the swarm loop's interval branch.
    ///
    /// Calls the sampler, evaluates against the threshold, and updates the
    /// shed flag. Edge-triggered: logs only on state transitions.
    ///
    /// ## Branches (Path-Coverage)
    ///
    /// 1. `threshold == 0` → no-op (disabled).
    /// 2. `sampler returns None` → ensure flag=false (fail-open), return.
    /// 3. `evaluate → Healthy` AND was_shedding → flag=false, log recovery.
    /// 4. `evaluate → Healthy` AND !was_shedding → flag=false (idempotent).
    /// 5. `evaluate → Shedding` AND !was_shedding → flag=true, trip_count++, log WARN.
    /// 6. `evaluate → Shedding` AND was_shedding → flag stays true (no log, no trip_count++).
    pub fn tick(&mut self) {
        // Branch 1: disabled
        if self.soft_threshold_bytes == 0 {
            return;
        }

        // Sample resident bytes
        let bytes = match (self.sample)() {
            Some(b) => b,
            None => {
                // Branch 2: sampler unavailable → fail-open
                self.shed_flag.store(false, Ordering::Relaxed);
                self.was_shedding = false;
                return;
            }
        };

        match evaluate(bytes, self.soft_threshold_bytes) {
            WatchdogState::Healthy => {
                if self.was_shedding {
                    // Branch 3: recovery transition
                    self.shed_flag.store(false, Ordering::Relaxed);
                    self.was_shedding = false;
                    info!(
                        "[MEM_WATCHDOG] Memory recovered: {} MB < {} MB threshold — gossip shedding OFF",
                        bytes / (1024 * 1024),
                        self.soft_threshold_bytes / (1024 * 1024),
                    );
                } else {
                    // Branch 4: still healthy (common path, no log)
                    self.shed_flag.store(false, Ordering::Relaxed);
                }
            }
            WatchdogState::Shedding => {
                if !self.was_shedding {
                    // Branch 5: new trip (Healthy → Shedding transition)
                    self.shed_flag.store(true, Ordering::Relaxed);
                    self.trip_count.fetch_add(1, Ordering::Relaxed);
                    self.was_shedding = true;
                    warn!(
                        "[MEM_WATCHDOG] Memory pressure: {} MB >= {} MB threshold — gossip shedding ON (trip #{})",
                        bytes / (1024 * 1024),
                        self.soft_threshold_bytes / (1024 * 1024),
                        self.trip_count.load(Ordering::Relaxed),
                    );
                }
                // Branch 6: already shedding — no log, no trip_count++
                // (shed_flag already true from Branch 5)
            }
        }
    }

    /// Returns true if the watchdog has tripped (shed all gossip blocks).
    pub fn should_shed(&self) -> bool {
        self.shed_flag.load(Ordering::Relaxed)
    }

    /// Total number of Healthy→Shedding transitions since creation.
    pub fn trips(&self) -> u64 {
        self.trip_count.load(Ordering::Relaxed)
    }

    /// Get the shared shed flag for wiring into the gossip hot path.
    pub fn shed_flag(&self) -> Arc<AtomicBool> {
        self.shed_flag.clone()
    }
}

/// Real platform sampler: returns resident memory in bytes.
///
/// - **Linux**: reads `/proc/self/statm` (field 2 = resident pages) × page_size.
/// - **Non-Linux (macOS, etc.)**: returns `None` (fail-open). A rate-limited
///   log is emitted on first call so operators know the watchdog is inactive.
pub fn sample_resident_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        sample_resident_bytes_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        use std::sync::atomic::AtomicBool;
        static LOGGED_ONCE: AtomicBool = AtomicBool::new(false);
        if !LOGGED_ONCE.swap(true, Ordering::Relaxed) {
            info!(
                "[MEM_WATCHDOG] Memory sampler unavailable on this platform — watchdog inactive (fail-open)"
            );
        }
        None
    }
}

/// Linux-specific: read resident pages from /proc/self/statm and multiply by page_size.
#[cfg(target_os = "linux")]
fn sample_resident_bytes_linux() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    // Format: "total_pages resident_pages shared_pages ..."
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return None;
    }
    Some(resident_pages * page_size as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    // -----------------------------------------------------------------
    // Pure decision function tests
    // -----------------------------------------------------------------

    /// TEST-WD-001: Below threshold → Healthy.
    #[test]
    fn evaluate_below_threshold_healthy() {
        assert_eq!(evaluate(99, 100), WatchdogState::Healthy);
        assert_eq!(evaluate(0, 100), WatchdogState::Healthy);
        assert_eq!(evaluate(0, 1), WatchdogState::Healthy);
    }

    /// TEST-WD-002: At threshold → Shedding (boundary: threshold itself triggers).
    #[test]
    fn evaluate_at_threshold_sheds() {
        // Boundary semantics: the threshold value ITSELF triggers shedding.
        // Rationale: threshold is a SOFT limit set below OOM ceiling,
        // so "at threshold = already too high" is the conservative choice.
        assert_eq!(evaluate(100, 100), WatchdogState::Shedding);
        assert_eq!(evaluate(1, 1), WatchdogState::Shedding);
    }

    /// TEST-WD-003: Above threshold → Shedding.
    #[test]
    fn evaluate_above_threshold_sheds() {
        assert_eq!(evaluate(101, 100), WatchdogState::Shedding);
        assert_eq!(evaluate(u64::MAX, 100), WatchdogState::Shedding);
    }

    // -----------------------------------------------------------------
    // MemoryWatchdog integration tests (injectable sampler)
    // -----------------------------------------------------------------

    /// Helper: create a sampler backed by an AtomicU64 that can be changed
    /// between tick() calls for deterministic state transitions.
    fn injectable_sampler() -> (Arc<AtomicU64>, Arc<dyn Fn() -> Option<u64> + Send + Sync>) {
        let value = Arc::new(AtomicU64::new(0));
        let v = value.clone();
        let sampler: Arc<dyn Fn() -> Option<u64> + Send + Sync> =
            Arc::new(move || Some(v.load(Ordering::Relaxed)));
        (value, sampler)
    }

    /// Helper: create a sampler that always returns None (unavailable).
    fn none_sampler() -> Arc<dyn Fn() -> Option<u64> + Send + Sync> {
        Arc::new(|| None)
    }

    /// TEST-WD-004: Watchdog trips when memory crosses threshold.
    /// Soft threshold = 1 GB. Memory starts below, then crosses.
    #[test]
    fn watchdog_trips_before_threshold() {
        let threshold = 1_000_000_000u64; // 1 GB — set BELOW OOM ceiling
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(threshold, sampler);

        // Below threshold: should NOT shed
        mem.store(500_000_000, Ordering::Relaxed);
        wd.tick();
        assert!(!wd.should_shed(), "below threshold must not shed");
        assert_eq!(wd.trips(), 0);

        // At threshold: should shed (boundary)
        mem.store(threshold, Ordering::Relaxed);
        wd.tick();
        assert!(wd.should_shed(), "at threshold must shed");
        assert_eq!(wd.trips(), 1, "first trip");
    }

    /// TEST-WD-005: Edge-triggered — multiple ticks above threshold increment
    /// trip_count only ONCE per Healthy→Shedding transition.
    #[test]
    fn watchdog_edge_triggered_logging() {
        let threshold = 100u64;
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(threshold, sampler);

        // Cross threshold
        mem.store(200, Ordering::Relaxed);
        wd.tick();
        assert_eq!(wd.trips(), 1, "first transition");

        // Stay above — tick 10 more times
        for _ in 0..10 {
            wd.tick();
        }
        assert_eq!(
            wd.trips(),
            1,
            "trip_count must NOT increment while already shedding"
        );
        assert!(wd.should_shed(), "flag must stay true");
    }

    /// TEST-WD-006: Watchdog recovers when memory drops below threshold.
    #[test]
    fn watchdog_recovers() {
        let threshold = 100u64;
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(threshold, sampler);

        // Trip
        mem.store(200, Ordering::Relaxed);
        wd.tick();
        assert!(wd.should_shed());
        assert_eq!(wd.trips(), 1);

        // Recover
        mem.store(50, Ordering::Relaxed);
        wd.tick();
        assert!(!wd.should_shed(), "must recover when below threshold");
        assert_eq!(wd.trips(), 1, "trip count unchanged on recovery");

        // Re-trip: should be trip #2
        mem.store(200, Ordering::Relaxed);
        wd.tick();
        assert!(wd.should_shed());
        assert_eq!(wd.trips(), 2, "second trip after recovery");
    }

    /// TEST-WD-007: Sampler returns None → fail-open, never sheds.
    #[test]
    fn sampler_none_fail_open() {
        let mut wd = MemoryWatchdog::new(100, none_sampler());

        // Multiple ticks with None sampler
        for _ in 0..10 {
            wd.tick();
        }
        assert!(
            !wd.should_shed(),
            "None sampler must never shed (fail-open)"
        );
        assert_eq!(wd.trips(), 0, "no trips with None sampler");
    }

    /// TEST-WD-008: Threshold 0 = disabled, tick is a no-op.
    #[test]
    fn disabled_threshold_zero_never_sheds() {
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(0, sampler);

        // Even with high memory, threshold=0 means disabled
        mem.store(u64::MAX, Ordering::Relaxed);
        for _ in 0..10 {
            wd.tick();
        }
        assert!(!wd.should_shed(), "threshold=0 must never shed");
        assert_eq!(wd.trips(), 0, "threshold=0 must never trip");
    }

    // -----------------------------------------------------------------
    // Path-coverage: every branch of tick()
    // -----------------------------------------------------------------

    /// TEST-WD-009: Path 1 — threshold=0 (disabled), returns immediately.
    #[test]
    fn path_disabled_noop() {
        let (mem, sampler) = injectable_sampler();
        mem.store(999, Ordering::Relaxed);
        let mut wd = MemoryWatchdog::new(0, sampler);
        wd.tick();
        assert!(!wd.should_shed());
        assert_eq!(wd.trips(), 0);
    }

    /// TEST-WD-010: Path 2 — sampler returns None, flag forced false (fail-open).
    #[test]
    fn path_sampler_none_forces_healthy() {
        let mut wd = MemoryWatchdog::new(100, none_sampler());
        // Even if we manually set the flag (shouldn't happen, but defensive):
        wd.shed_flag.store(true, Ordering::Relaxed);
        wd.was_shedding = true;
        wd.tick();
        assert!(
            !wd.should_shed(),
            "None sampler must force flag=false (fail-open)"
        );
    }

    /// TEST-WD-011: Path 3 — recovery transition (was_shedding=true → Healthy).
    #[test]
    fn path_recovery_transition() {
        let threshold = 100u64;
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(threshold, sampler);

        // Enter shedding state
        mem.store(200, Ordering::Relaxed);
        wd.tick();
        assert!(wd.should_shed());

        // Recover
        mem.store(50, Ordering::Relaxed);
        wd.tick();
        assert!(!wd.should_shed(), "recovery transition must clear flag");
    }

    /// TEST-WD-012: Path 4 — already healthy, stays healthy (common path).
    #[test]
    fn path_healthy_stays_healthy() {
        let threshold = 100u64;
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(threshold, sampler);

        mem.store(50, Ordering::Relaxed);
        wd.tick();
        assert!(!wd.should_shed());
        // Tick again — still healthy
        wd.tick();
        assert!(!wd.should_shed());
        assert_eq!(wd.trips(), 0);
    }

    /// TEST-WD-013: Path 5 — Healthy→Shedding transition (new trip).
    #[test]
    fn path_new_trip_transition() {
        let threshold = 100u64;
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(threshold, sampler);

        mem.store(50, Ordering::Relaxed);
        wd.tick(); // healthy
        assert_eq!(wd.trips(), 0);

        mem.store(100, Ordering::Relaxed);
        wd.tick(); // trip
        assert!(wd.should_shed());
        assert_eq!(wd.trips(), 1);
    }

    /// TEST-WD-014: Path 6 — already shedding, stays shedding (no extra trip).
    #[test]
    fn path_already_shedding_no_extra_trip() {
        let threshold = 100u64;
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(threshold, sampler);

        mem.store(200, Ordering::Relaxed);
        wd.tick(); // trip
        assert_eq!(wd.trips(), 1);

        // Stay above
        mem.store(300, Ordering::Relaxed);
        wd.tick();
        assert_eq!(wd.trips(), 1, "must not increment when already shedding");
        assert!(wd.should_shed());
    }

    /// TEST-WD-015: shed_flag() getter returns a shared Arc that reflects
    /// watchdog state changes.
    #[test]
    fn shed_flag_getter_shares_state() {
        let threshold = 100u64;
        let (mem, sampler) = injectable_sampler();
        let mut wd = MemoryWatchdog::new(threshold, sampler);

        let flag = wd.shed_flag();
        assert!(!flag.load(Ordering::Relaxed), "initial: not shedding");

        mem.store(200, Ordering::Relaxed);
        wd.tick();
        assert!(flag.load(Ordering::Relaxed), "flag reflects shedding");

        mem.store(50, Ordering::Relaxed);
        wd.tick();
        assert!(!flag.load(Ordering::Relaxed), "flag reflects recovery");
    }

    /// TEST-WD-016: with_real_sampler constructor works (non-Linux: returns None = fail-open).
    #[test]
    fn real_sampler_constructor_works() {
        let mut wd = MemoryWatchdog::with_real_sampler(100);
        // On macOS (CI/dev), this returns None → fail-open → no shed
        wd.tick();
        // On Linux, it reads actual RSS — either way, should not panic
        // We can't assert the exact value, but we verify no panic
    }
}
