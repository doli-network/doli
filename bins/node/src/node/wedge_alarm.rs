//! INC-I-204 M0 / D7 — the wedge detector.
//!
//! INC-I-204's signature: a node's own tip frozen while the fleet holds more than
//! one tip and FORK_GUARD refusals keep accumulating. It ran 7 days before a
//! human noticed. All three conditions must hold across a whole rolling window —
//! refusals alone are the guard working correctly (LB-1), and a stalled tip with
//! a unanimous fleet is a different incident with a different runbook.
//!
//! Time is injected, never read from a clock here, so the production window can
//! be evaluated in a test without sleeping.

use std::collections::VecDeque;

/// Hard cap on retained samples. Time eviction is the real bound; this only
/// protects against a caller whose clock never advances.
const MAX_SAMPLES: usize = 1024;

/// One health observation. `refusals_total` is CUMULATIVE, as a Prometheus
/// counter is; the alarm differences it inside the window.
#[derive(Clone, Copy, Debug)]
pub struct WedgeSample {
    pub at_secs: u64,
    pub tip_height: u64,
    pub refusals_total: u64,
    pub unique_chain_tips: usize,
    /// Reported to the operator, deliberately NOT a fire condition: a same-height
    /// fork leaves peers level with us, which is exactly the case that must alarm.
    #[allow(dead_code)]
    pub best_peer_height: u64,
}

/// Detector thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WedgeAlarmConfig {
    pub window_secs: u64,
    pub min_refusals_in_window: u64,
}

impl Default for WedgeAlarmConfig {
    fn default() -> Self {
        Self {
            window_secs: 300,
            min_refusals_in_window: 3,
        }
    }
}

/// The verdict for the latest sample. Never latches: recovery is observable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WedgeVerdict {
    Clear,
    Wedged {
        stalled_secs: u64,
        refusals_in_window: u64,
        unique_chain_tips: usize,
    },
}

/// Rolling, bounded window of samples.
pub struct WedgeAlarm {
    cfg: WedgeAlarmConfig,
    samples: VecDeque<WedgeSample>,
}

impl WedgeAlarm {
    pub fn new(cfg: WedgeAlarmConfig) -> Self {
        Self {
            cfg,
            samples: VecDeque::new(),
        }
    }

    /// Record a sample and judge the window ending at it.
    ///
    /// Fires only when all three hold for the FULL window: the tip never moved,
    /// refusals grew by at least the threshold, and the fleet stayed split.
    pub fn observe(&mut self, s: WedgeSample) -> WedgeVerdict {
        self.samples.push_back(s);
        let cutoff = s.at_secs.saturating_sub(self.cfg.window_secs);
        while self.samples.front().is_some_and(|f| f.at_secs < cutoff) {
            self.samples.pop_front();
        }
        while self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }

        let Some(oldest) = self.samples.front().copied() else {
            return WedgeVerdict::Clear;
        };
        let stalled_secs = s.at_secs.saturating_sub(oldest.at_secs);
        if stalled_secs < self.cfg.window_secs {
            return WedgeVerdict::Clear;
        }
        if self.samples.iter().any(|x| x.tip_height != s.tip_height) {
            return WedgeVerdict::Clear;
        }
        // `unique_chain_tips == 0` is an ISOLATED node: checkpoint_health returns
        // (0,0,0) with no peers, which is no evidence, not "fewer than one tip".
        // `> 1` excludes it deliberately — a peerless node cannot witness a fork.
        if self.samples.iter().any(|x| x.unique_chain_tips <= 1) {
            return WedgeVerdict::Clear;
        }
        let refusals_in_window = s.refusals_total.saturating_sub(oldest.refusals_total);
        if refusals_in_window < self.cfg.min_refusals_in_window {
            return WedgeVerdict::Clear;
        }
        WedgeVerdict::Wedged {
            stalled_secs,
            refusals_in_window,
            unique_chain_tips: s.unique_chain_tips,
        }
    }
}
