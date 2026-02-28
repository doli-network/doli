//! VDF Calibration System
//!
//! This module provides dynamic calibration of VDF iterations to maintain
//! consistent timing across different hardware. It measures actual VDF
//! computation times and adjusts iterations to match the target duration.
//!
//! ## Why Calibration?
//!
//! VDF computation speed varies significantly across hardware:
//! - CPU speed differences
//! - Cache sizes
//! - Thermal throttling
//! - Background system load
//!
//! Dynamic calibration ensures all nodes produce blocks with consistent timing,
//! regardless of their underlying hardware performance.

use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tracing::{debug, info};

use super::heartbeat::hash_chain_vdf;
use crate::network::Network;
use crate::network_params::NetworkParams;
use crypto::hash::hash;

/// Target VDF time in milliseconds (~700ms for heartbeat proof)
/// With Epoch Lookahead selection, VDF only needs to prove presence, not prevent grinding.
pub const TARGET_VDF_TIME_MS: u64 = 700;

/// Default iterations for VDF (mainnet default: ~700ms on reference hardware)
/// Reference: ~14,285 iterations/ms on modern CPU
/// 700ms * 14,285 = ~10M iterations
///
/// **Deprecated**: Use `vdf_iterations_for_network(network)` for network-aware calculations.
/// The calibrator will adjust at runtime, but initial value should come from NetworkParams.
#[deprecated(note = "Use vdf_iterations_for_network(network) for network-aware initial value")]
pub const DEFAULT_VDF_ITERATIONS: u64 = 10_000_000;

/// Get VDF iterations for a specific network (for initial calibration)
pub fn vdf_iterations_for_network(network: Network) -> u64 {
    NetworkParams::load(network).vdf_iterations
}

/// Minimum allowed iterations (safety floor)
pub const MIN_VDF_ITERATIONS: u64 = 100_000;

/// Maximum allowed iterations (safety ceiling)
pub const MAX_VDF_ITERATIONS: u64 = 100_000_000;

/// Number of samples to average for calibration
const CALIBRATION_SAMPLES: usize = 5;

/// Minimum samples before adjusting iterations
const MIN_SAMPLES_FOR_ADJUSTMENT: usize = 3;

/// Acceptable tolerance from target (10%)
const TOLERANCE_PERCENT: f64 = 0.10;

/// Maximum adjustment per calibration cycle (20%)
const MAX_ADJUSTMENT_PERCENT: f64 = 0.20;

/// A single timing measurement
#[derive(Clone, Debug)]
struct TimingSample {
    /// Number of iterations used
    iterations: u64,
    /// Actual time taken
    duration_ms: u64,
    /// When this sample was taken
    #[allow(dead_code)]
    timestamp: Instant,
}

/// VDF Calibrator for maintaining consistent timing
///
/// This struct tracks VDF computation times and dynamically adjusts
/// the number of iterations to maintain the target computation time.
#[derive(Debug)]
pub struct VdfCalibrator {
    /// Current number of iterations
    current_iterations: u64,

    /// Recent timing samples
    samples: VecDeque<TimingSample>,

    /// Target computation time in milliseconds
    target_time_ms: u64,

    /// Whether calibration is enabled
    enabled: bool,

    /// Last calibration time
    last_calibration: Option<Instant>,

    /// Minimum time between calibrations
    calibration_interval: Duration,
}

impl Default for VdfCalibrator {
    #[allow(deprecated)]
    fn default() -> Self {
        Self::new(DEFAULT_VDF_ITERATIONS, TARGET_VDF_TIME_MS)
    }
}

impl VdfCalibrator {
    /// Create a new calibrator with default iterations and target time
    pub fn new(initial_iterations: u64, target_time_ms: u64) -> Self {
        Self {
            current_iterations: initial_iterations.clamp(MIN_VDF_ITERATIONS, MAX_VDF_ITERATIONS),
            samples: VecDeque::with_capacity(CALIBRATION_SAMPLES + 1),
            target_time_ms,
            enabled: true,
            last_calibration: None,
            calibration_interval: Duration::from_secs(60), // Recalibrate at most every 60 seconds
        }
    }

    /// Create a calibrator with calibration disabled (uses fixed iterations)
    pub fn disabled(iterations: u64) -> Self {
        let mut cal = Self::new(iterations, TARGET_VDF_TIME_MS);
        cal.enabled = false;
        cal
    }

    /// Create a calibrator for a specific network's target VDF time
    ///
    /// With Epoch Lookahead selection, VDF only proves presence (heartbeat).
    /// The target time is ~700ms for all networks.
    ///
    /// Reference rate: ~14,285 iterations/ms on modern CPU
    pub fn for_network(target_time_ms: u64) -> Self {
        // Estimate initial iterations based on reference rate (~14,285 iter/ms)
        const REFERENCE_RATE: u64 = 14_285;
        let estimated_iterations = target_time_ms * REFERENCE_RATE;
        let initial_iterations = estimated_iterations.clamp(MIN_VDF_ITERATIONS, MAX_VDF_ITERATIONS);
        Self::new(initial_iterations, target_time_ms)
    }

    /// Get the current number of iterations to use
    pub fn iterations(&self) -> u64 {
        self.current_iterations
    }

    /// Get the target computation time
    pub fn target_time_ms(&self) -> u64 {
        self.target_time_ms
    }

    /// Check if calibration is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable dynamic calibration
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Record a VDF computation timing
    ///
    /// Call this after each VDF computation with the actual duration.
    /// The calibrator will adjust iterations based on accumulated samples.
    pub fn record_timing(&mut self, iterations: u64, duration: Duration) {
        if !self.enabled {
            return;
        }

        let duration_ms = duration.as_millis() as u64;

        let sample = TimingSample {
            iterations,
            duration_ms,
            timestamp: Instant::now(),
        };

        // Add sample
        self.samples.push_back(sample);

        // Keep only recent samples
        while self.samples.len() > CALIBRATION_SAMPLES {
            self.samples.pop_front();
        }

        // Try to recalibrate
        self.maybe_recalibrate();
    }

    /// Attempt to recalibrate based on collected samples
    fn maybe_recalibrate(&mut self) {
        // Need enough samples
        if self.samples.len() < MIN_SAMPLES_FOR_ADJUSTMENT {
            return;
        }

        // Check calibration interval
        if let Some(last) = self.last_calibration {
            if last.elapsed() < self.calibration_interval {
                return;
            }
        }

        // Calculate average iterations per millisecond from samples
        let mut total_rate = 0f64;
        let mut valid_samples = 0;

        for sample in &self.samples {
            if sample.duration_ms > 0 {
                let rate = sample.iterations as f64 / sample.duration_ms as f64;
                total_rate += rate;
                valid_samples += 1;
            }
        }

        if valid_samples == 0 {
            return;
        }

        let avg_rate = total_rate / valid_samples as f64;

        // Calculate ideal iterations for target time
        let ideal_iterations = (avg_rate * self.target_time_ms as f64) as u64;

        // Check if adjustment is needed (outside tolerance)
        let current_time_estimate = self.current_iterations as f64 / avg_rate;
        let deviation =
            (current_time_estimate - self.target_time_ms as f64).abs() / self.target_time_ms as f64;

        if deviation <= TOLERANCE_PERCENT {
            debug!(
                "VDF calibration within tolerance: estimated {}ms, target {}ms, deviation {:.1}%",
                current_time_estimate as u64,
                self.target_time_ms,
                deviation * 100.0
            );
            return;
        }

        // Calculate adjustment (limited to MAX_ADJUSTMENT_PERCENT)
        let adjustment_ratio = ideal_iterations as f64 / self.current_iterations as f64;
        let clamped_ratio =
            adjustment_ratio.clamp(1.0 - MAX_ADJUSTMENT_PERCENT, 1.0 + MAX_ADJUSTMENT_PERCENT);

        let new_iterations = (self.current_iterations as f64 * clamped_ratio) as u64;
        let new_iterations = new_iterations.clamp(MIN_VDF_ITERATIONS, MAX_VDF_ITERATIONS);

        if new_iterations != self.current_iterations {
            info!(
                "VDF calibration: {} -> {} iterations (target {}ms, estimated {}ms)",
                self.current_iterations,
                new_iterations,
                self.target_time_ms,
                (new_iterations as f64 / avg_rate) as u64
            );

            self.current_iterations = new_iterations;
        }

        self.last_calibration = Some(Instant::now());
    }

    /// Perform a calibration run (measures VDF time with test iterations)
    ///
    /// This can be called during node startup to establish baseline performance.
    /// It performs a quick VDF computation and adjusts iterations accordingly.
    pub fn calibrate_now(&mut self) -> Duration {
        if !self.enabled {
            return Duration::from_millis(0);
        }

        // Use a smaller number of test iterations for quick calibration
        let test_iterations = 1_000_000; // ~70ms on reference hardware

        // Generate random input
        let input = hash(b"calibration_test");

        // Time the VDF computation
        let start = Instant::now();
        let _ = hash_chain_vdf(&input, test_iterations);
        let duration = start.elapsed();

        // Record this timing
        self.samples.push_back(TimingSample {
            iterations: test_iterations,
            duration_ms: duration.as_millis() as u64,
            timestamp: Instant::now(),
        });

        // Calculate rate and adjust
        let rate = test_iterations as f64 / duration.as_millis().max(1) as f64;
        let ideal_iterations = (rate * self.target_time_ms as f64) as u64;
        let new_iterations = ideal_iterations.clamp(MIN_VDF_ITERATIONS, MAX_VDF_ITERATIONS);

        if new_iterations != self.current_iterations {
            info!(
                "VDF initial calibration: {} -> {} iterations (rate: {:.0} iter/ms)",
                self.current_iterations, new_iterations, rate
            );
            self.current_iterations = new_iterations;
        }

        self.last_calibration = Some(Instant::now());
        duration
    }

    /// Get calibration statistics
    pub fn stats(&self) -> CalibrationStats {
        let avg_duration_ms = if self.samples.is_empty() {
            0
        } else {
            self.samples.iter().map(|s| s.duration_ms).sum::<u64>() / self.samples.len() as u64
        };

        CalibrationStats {
            current_iterations: self.current_iterations,
            target_time_ms: self.target_time_ms,
            sample_count: self.samples.len(),
            avg_duration_ms,
            enabled: self.enabled,
        }
    }

    /// Load calibration state from saved iterations
    ///
    /// This can be called during node startup to restore the last known
    /// good iteration count from persistent storage.
    pub fn load_iterations(&mut self, saved_iterations: u64) {
        let clamped = saved_iterations.clamp(MIN_VDF_ITERATIONS, MAX_VDF_ITERATIONS);
        self.current_iterations = clamped;
        info!("Loaded VDF iterations from storage: {}", clamped);
    }
}

/// Statistics about calibration state
#[derive(Clone, Debug)]
pub struct CalibrationStats {
    /// Current number of iterations
    pub current_iterations: u64,
    /// Target computation time
    pub target_time_ms: u64,
    /// Number of timing samples collected
    pub sample_count: usize,
    /// Average measured duration
    pub avg_duration_ms: u64,
    /// Whether calibration is enabled
    pub enabled: bool,
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[test]
    fn test_default_calibrator() {
        let cal = VdfCalibrator::default();
        assert_eq!(cal.iterations(), DEFAULT_VDF_ITERATIONS);
        assert_eq!(cal.target_time_ms(), TARGET_VDF_TIME_MS);
        assert!(cal.is_enabled());
    }

    #[test]
    fn test_disabled_calibrator() {
        // Use a value within valid range
        let cal = VdfCalibrator::disabled(10_000_000);
        assert_eq!(cal.iterations(), 10_000_000);
        assert!(!cal.is_enabled());
    }

    #[test]
    fn test_iterations_clamped() {
        // Test minimum clamping
        let cal = VdfCalibrator::new(100, TARGET_VDF_TIME_MS);
        assert_eq!(cal.iterations(), MIN_VDF_ITERATIONS);

        // Test maximum clamping
        let cal = VdfCalibrator::new(2_000_000_000, TARGET_VDF_TIME_MS);
        assert_eq!(cal.iterations(), MAX_VDF_ITERATIONS);
    }

    #[test]
    fn test_record_timing() {
        let mut cal = VdfCalibrator::default();

        // Record some timings (use values matching the default iterations ~700ms)
        cal.record_timing(DEFAULT_VDF_ITERATIONS, Duration::from_millis(700));
        cal.record_timing(DEFAULT_VDF_ITERATIONS, Duration::from_millis(710));
        cal.record_timing(DEFAULT_VDF_ITERATIONS, Duration::from_millis(690));

        let stats = cal.stats();
        assert_eq!(stats.sample_count, 3);
        assert!(stats.avg_duration_ms > 0);
    }

    #[test]
    fn test_calibration_adjusts_for_slow_hardware() {
        // Use values within valid range (10M iterations, 700ms target)
        let mut cal = VdfCalibrator::new(10_000_000, 700);
        cal.calibration_interval = Duration::from_millis(0); // Allow immediate recalibration

        // Simulate slow hardware (taking 1400ms instead of 700ms)
        for _ in 0..5 {
            cal.record_timing(10_000_000, Duration::from_millis(1400));
        }

        // Should reduce iterations since hardware is slow
        assert!(cal.iterations() < 10_000_000);
    }

    #[test]
    fn test_calibration_adjusts_for_fast_hardware() {
        // Use values within valid range (10M iterations, 700ms target)
        let mut cal = VdfCalibrator::new(10_000_000, 700);
        cal.calibration_interval = Duration::from_millis(0); // Allow immediate recalibration

        // Simulate fast hardware (taking 350ms instead of 700ms)
        for _ in 0..5 {
            cal.record_timing(10_000_000, Duration::from_millis(350));
        }

        // Should increase iterations since hardware is fast
        assert!(cal.iterations() > 10_000_000);
    }

    #[test]
    fn test_calibration_within_tolerance_no_change() {
        // Use values within valid range
        let mut cal = VdfCalibrator::new(10_000_000, 700);
        cal.calibration_interval = Duration::from_millis(0);

        // Simulate timings within 10% tolerance (~3% over)
        for _ in 0..5 {
            cal.record_timing(10_000_000, Duration::from_millis(721));
        }

        // Should NOT adjust since we're within tolerance
        assert_eq!(cal.iterations(), 10_000_000);
    }

    #[test]
    fn test_max_adjustment_limit() {
        // Use values within valid range
        let mut cal = VdfCalibrator::new(10_000_000, 700);
        // Keep calibration interval high so only one recalibration happens
        cal.calibration_interval = Duration::from_secs(3600);

        // Add just enough samples to trigger one calibration
        // Simulate extremely slow hardware (taking 10x longer)
        cal.record_timing(10_000_000, Duration::from_millis(7000));
        cal.record_timing(10_000_000, Duration::from_millis(7000));

        // The 3rd sample will trigger calibration
        let initial = cal.iterations();
        cal.record_timing(10_000_000, Duration::from_millis(7000));

        // Verify only one adjustment happened
        assert_eq!(initial, 10_000_000);

        // Should only reduce by MAX_ADJUSTMENT_PERCENT per cycle (20%)
        // Expected: 10M * 0.8 = 8M (minimum after one adjustment cycle)
        let expected_min = (10_000_000.0 * (1.0 - MAX_ADJUSTMENT_PERCENT)) as u64;
        assert!(
            cal.iterations() >= expected_min,
            "iterations {} should be >= {} (max 20% reduction per cycle)",
            cal.iterations(),
            expected_min
        );
        // Also verify it did reduce (not stuck at 10M)
        assert!(
            cal.iterations() < 10_000_000,
            "should have reduced from 10M"
        );
    }

    #[test]
    fn test_calibrate_now() {
        // Use default target (~700ms)
        let mut cal = VdfCalibrator::for_network(700);

        // This actually runs a VDF, so we check it returns reasonable duration
        let duration = cal.calibrate_now();

        // Should complete in reasonable time
        assert!(duration < Duration::from_secs(5));

        // Should have updated iterations
        assert!(cal.iterations() >= MIN_VDF_ITERATIONS);
        assert!(cal.iterations() <= MAX_VDF_ITERATIONS);
    }

    #[test]
    fn test_load_iterations() {
        let mut cal = VdfCalibrator::default();

        // Load valid iterations (within range)
        cal.load_iterations(10_000_000);
        assert_eq!(cal.iterations(), 10_000_000);

        // Load iterations below minimum
        cal.load_iterations(1000);
        assert_eq!(cal.iterations(), MIN_VDF_ITERATIONS);

        // Load iterations above maximum
        cal.load_iterations(500_000_000);
        assert_eq!(cal.iterations(), MAX_VDF_ITERATIONS);
    }

    #[test]
    fn test_stats() {
        // Use values within valid range
        let mut cal = VdfCalibrator::new(10_000_000, 700);
        cal.record_timing(10_000_000, Duration::from_millis(700));
        cal.record_timing(10_000_000, Duration::from_millis(720));

        let stats = cal.stats();
        assert_eq!(stats.current_iterations, 10_000_000);
        assert_eq!(stats.target_time_ms, 700);
        assert_eq!(stats.sample_count, 2);
        assert!(stats.avg_duration_ms > 0);
        assert!(stats.enabled);
    }

    #[test]
    fn test_for_network() {
        // All networks use ~700ms VDF target (heartbeat proof)
        let cal_700 = VdfCalibrator::for_network(700);
        assert_eq!(cal_700.target_time_ms(), 700);
        assert!(cal_700.iterations() >= MIN_VDF_ITERATIONS);

        // Smaller target for faster tests
        let cal_500 = VdfCalibrator::for_network(500);
        assert_eq!(cal_500.target_time_ms(), 500);
        assert!(cal_500.iterations() >= MIN_VDF_ITERATIONS);

        // Larger target
        let cal_1000 = VdfCalibrator::for_network(1000);
        assert_eq!(cal_1000.target_time_ms(), 1000);
        assert!(cal_1000.iterations() >= MIN_VDF_ITERATIONS);
    }
}
