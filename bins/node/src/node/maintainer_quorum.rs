//! Maintainer trust-root peer quorum (INC-I-248): probe state machine and node wiring.
//!
//! The selection rule is `doli_core::maintainer::select_quorum_maintainer_state`.
//! The probe mutex is never held across an `.await`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use crypto::Hash;
use doli_core::maintainer::{
    maintainer_set_digest, select_quorum_maintainer_state, KeepReason, LocalRootView,
    PeerMaintainerAnswer, QuorumVerdict,
};
use doli_core::MaintainerSet;
use network::protocols::{SyncRequest, SyncResponse};
use network::PeerId;
use tracing::{debug, info, warn};

use super::Node;

/// Collection window of one probe attempt.
pub const MAINTAINER_QUORUM_WINDOW_SECS: u64 = 15;
/// Wait between a transient verdict and the next attempt.
pub const MAINTAINER_QUORUM_RETRY_SECS: u64 = 60;
/// Attempts before the probe gives up.
pub const MAINTAINER_QUORUM_MAX_ATTEMPTS: u32 = 3;
/// Process uptime before the startup probe may arm.
pub const MAINTAINER_STARTUP_MIN_UPTIME_SECS: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeReason {
    PostSnapInstall,
    Startup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeStep {
    Idle,
    Wait,
    Evaluate,
    Rearm,
}

#[derive(Debug)]
enum Phase {
    Idle,
    Collecting {
        started: Instant,
        answers: HashMap<PeerId, PeerMaintainerAnswer>,
    },
    RetryWait {
        since: Instant,
    },
    Done,
    GaveUp,
}

/// Pure probe state machine; time is always passed in.
#[derive(Debug)]
pub struct MaintainerQuorumProbe {
    phase: Phase,
    attempts: u32,
    reason: ProbeReason,
    created: Instant,
    startup_armed: bool,
}

impl Default for MaintainerQuorumProbe {
    fn default() -> Self {
        Self::new()
    }
}

/// Final verdicts end the probe; the others are retried.
fn is_final(verdict: &QuorumVerdict) -> bool {
    matches!(
        verdict,
        QuorumVerdict::Adopt { .. }
            | QuorumVerdict::Keep(KeepReason::AlreadyCanonical)
            | QuorumVerdict::Keep(KeepReason::PeersDisagree { .. })
    )
}

impl MaintainerQuorumProbe {
    pub fn new() -> Self {
        Self::new_at(Instant::now())
    }

    pub fn new_at(created: Instant) -> Self {
        Self {
            phase: Phase::Idle,
            attempts: 0,
            reason: ProbeReason::Startup,
            created,
            startup_armed: false,
        }
    }

    /// Start an attempt. Attempts count up across retries and restart at 1
    /// from an idle or finished probe.
    pub fn arm(&mut self, reason: ProbeReason, now: Instant) {
        self.attempts = match self.phase {
            Phase::Collecting { .. } | Phase::RetryWait { .. } => self.attempts.saturating_add(1),
            Phase::Idle | Phase::Done | Phase::GaveUp => 1,
        };
        self.startup_armed |= reason == ProbeReason::Startup;
        self.reason = reason;
        self.phase = Phase::Collecting {
            started: now,
            answers: HashMap::new(),
        };
    }

    /// Store a peer answer (a second answer from one peer replaces the first).
    /// Returns `false` when the probe is not collecting.
    pub fn record(&mut self, peer: PeerId, answer: PeerMaintainerAnswer) -> bool {
        match &mut self.phase {
            Phase::Collecting { answers, .. } => {
                answers.insert(peer, answer);
                true
            }
            _ => false,
        }
    }

    /// The startup probe arms once per process, from an idle probe, with a peer.
    pub fn startup_arming_due(&self, peer_count: usize, now: Instant) -> bool {
        !self.startup_armed
            && matches!(self.phase, Phase::Idle)
            && peer_count >= 1
            && now.saturating_duration_since(self.created)
                >= Duration::from_secs(MAINTAINER_STARTUP_MIN_UPTIME_SECS)
    }

    pub fn on_tick(&mut self, now: Instant) -> ProbeStep {
        let elapsed = |t: Instant| now.saturating_duration_since(t);
        match &self.phase {
            Phase::Collecting { started, .. } => {
                if elapsed(*started) >= Duration::from_secs(MAINTAINER_QUORUM_WINDOW_SECS) {
                    ProbeStep::Evaluate
                } else {
                    ProbeStep::Wait
                }
            }
            Phase::RetryWait { since } => {
                if elapsed(*since) >= Duration::from_secs(MAINTAINER_QUORUM_RETRY_SECS) {
                    ProbeStep::Rearm
                } else {
                    ProbeStep::Wait
                }
            }
            Phase::Idle | Phase::Done | Phase::GaveUp => ProbeStep::Idle,
        }
    }

    pub fn on_verdict(&mut self, verdict: &QuorumVerdict, now: Instant) {
        self.phase = if is_final(verdict) {
            Phase::Done
        } else if self.attempts < MAINTAINER_QUORUM_MAX_ATTEMPTS {
            Phase::RetryWait { since: now }
        } else {
            Phase::GaveUp
        };
    }

    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Idle => "idle",
            Phase::Collecting { .. } => "collecting",
            Phase::RetryWait { .. } => "retry-wait",
            Phase::Done => "done",
            Phase::GaveUp => "gave-up",
        }
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn is_collecting(&self) -> bool {
        matches!(self.phase, Phase::Collecting { .. })
    }

    pub fn answers(&self) -> Vec<PeerMaintainerAnswer> {
        match &self.phase {
            Phase::Collecting { answers, .. } => answers.values().cloned().collect(),
            _ => Vec::new(),
        }
    }

    pub fn reason(&self) -> ProbeReason {
        self.reason
    }
}

impl Node {
    fn maintainer_probe(&self) -> MutexGuard<'_, MaintainerQuorumProbe> {
        self.maintainer_quorum
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Arm the probe and ask every sync peer for its maintainer state.
    pub async fn arm_maintainer_quorum(&self, reason: ProbeReason) {
        self.maintainer_probe().arm(reason, Instant::now());
        let Some(network) = &self.network else {
            return;
        };
        let peers: Vec<PeerId> = self.sync_manager.read().await.peer_ids().collect();
        info!(
            "[MAINTAINER] quorum probe armed ({:?}): asking {} peers",
            reason,
            peers.len()
        );
        for peer in peers {
            let _ = network
                .request_sync(peer, SyncRequest::GetMaintainerState)
                .await;
        }
    }

    /// Run the quorum rule over `answers`; on `Adopt`, replace and persist the local set.
    pub async fn apply_maintainer_quorum_answers(
        &self,
        answers: Vec<PeerMaintainerAnswer>,
    ) -> QuorumVerdict {
        let Some(ms) = self.maintainer_state.clone() else {
            return QuorumVerdict::Keep(KeepReason::NoAnswers);
        };
        let (snap_synced, tip_height) = {
            let cs = self.chain_state.read().await;
            (cs.is_snap_synced(), cs.best_height)
        };
        let quorum = self.sync_manager.read().await.snap_quorum();
        let genesis = self.params.genesis_hash;

        let mut state = ms.write().await;
        let local = LocalRootView {
            digest: (!state.set.members.is_empty())
                .then(|| maintainer_set_digest(&state.set, genesis.as_bytes())),
            snap_synced,
            usable: crate::updater::resolve_trust_root(&state, self.config.network).is_usable(),
            tip_height,
        };
        let verdict = select_quorum_maintainer_state(&local, &answers, quorum);
        if let QuorumVerdict::Adopt { set, agreeing } = &verdict {
            state.set = set.clone();
            state.last_derived_height = tip_height;
            if let Err(e) = state.save(&self.config.data_dir) {
                warn!(
                    "[MAINTAINER] failed to persist the adopted maintainer set: {}",
                    e
                );
            }
            info!(
                "[MAINTAINER] MAINTAINER_SET_DIGEST={} members={} threshold={} last_updated={} height={} (peer quorum, {} agreeing)",
                hex::encode(maintainer_set_digest(&state.set, genesis.as_bytes())),
                state.set.member_count(),
                state.set.threshold,
                state.set.last_updated,
                tip_height,
                agreeing
            );
            // Released first: the update service resolves the root with `try_read`.
            drop(state);
            if let Some(recheck) = &self.update_recheck {
                info!("[MAINTAINER] trust root replaced: release re-check requested");
                recheck.notify_one();
            }
        }
        verdict
    }

    /// Decode a peer frame; the digest is recomputed with the LOCAL genesis hash.
    fn decode_maintainer_answer(
        &self,
        block_height: u64,
        digest: Hash,
        set_bytes: &[u8],
        last_derived_height: u64,
    ) -> Result<PeerMaintainerAnswer, String> {
        let set: MaintainerSet = bincode::deserialize(set_bytes).map_err(|e| e.to_string())?;
        storage::validate_persisted_set(Path::new("peer:MaintainerStateAt"), &set)
            .map_err(|e| e.to_string())?;
        let local = maintainer_set_digest(&set, self.params.genesis_hash.as_bytes());
        if local != *digest.as_bytes() {
            return Err("digest does not match the body under the local genesis".into());
        }
        Ok(PeerMaintainerAnswer {
            set,
            last_derived_height,
            digest: local,
            tip_height: block_height,
        })
    }

    /// Record a `MaintainerStateAt` answer and evaluate all recorded answers at once.
    pub(crate) async fn on_maintainer_state_answer(&self, peer: PeerId, response: SyncResponse) {
        let SyncResponse::MaintainerStateAt {
            block_height,
            digest,
            set_bytes,
            last_derived_height,
            ..
        } = response
        else {
            return;
        };
        if !self.maintainer_probe().is_collecting() {
            debug!("[MAINTAINER] dropped answer from {}: not collecting", peer);
            return;
        }
        let answer = match self.decode_maintainer_answer(
            block_height,
            digest,
            &set_bytes,
            last_derived_height,
        ) {
            Ok(a) => a,
            Err(e) => {
                debug!("[MAINTAINER] dropped answer from {}: {}", peer, e);
                return;
            }
        };
        let answers = {
            let mut probe = self.maintainer_probe();
            if !probe.record(peer, answer) {
                debug!("[MAINTAINER] dropped answer from {}: not collecting", peer);
                return;
            }
            probe.answers()
        };
        if self.defers_to_window_end().await {
            return;
        }
        let verdict = self.apply_maintainer_quorum_answers(answers).await;
        if is_final(&verdict) {
            info!(
                "[MAINTAINER] quorum probe finished: {}",
                verdict_label(&verdict)
            );
            self.maintainer_probe().on_verdict(&verdict, Instant::now());
        }
    }

    /// Replay node with a non-empty usable root: only the window-end tick evaluates,
    /// so every answer of the window counts as possible dissent.
    async fn defers_to_window_end(&self) -> bool {
        let snap_synced = self.chain_state.read().await.is_snap_synced();
        match &self.maintainer_state {
            Some(ms) if !snap_synced => {
                let state = ms.read().await;
                !state.set.members.is_empty()
                    && crate::updater::resolve_trust_root(&state, self.config.network).is_usable()
            }
            _ => false,
        }
    }

    pub(crate) async fn maintainer_quorum_tick(&self) {
        self.maintainer_quorum_tick_at(Instant::now()).await;
    }

    /// Periodic driver: arm the startup probe once, evaluate at the end of a window,
    /// re-arm after a retry wait.
    pub async fn maintainer_quorum_tick_at(&self, now: Instant) {
        let peers = self.sync_manager.read().await.peer_count();
        let startup_due = self.maintainer_probe().startup_arming_due(peers, now);
        if startup_due {
            self.arm_maintainer_quorum(ProbeReason::Startup).await;
            return;
        }
        let (step, reason) = {
            let mut probe = self.maintainer_probe();
            (probe.on_tick(now), probe.reason())
        };
        match step {
            ProbeStep::Idle | ProbeStep::Wait => {}
            ProbeStep::Rearm => self.arm_maintainer_quorum(reason).await,
            ProbeStep::Evaluate => {
                let answers = self.maintainer_probe().answers();
                let verdict = self.apply_maintainer_quorum_answers(answers).await;
                let mut probe = self.maintainer_probe();
                probe.on_verdict(&verdict, now);
                if probe.phase_name() == "gave-up" {
                    warn!(
                        "[MAINTAINER] quorum probe gave up after {} attempts ({}); keeping the current trust root",
                        probe.attempts(),
                        verdict_label(&verdict)
                    );
                }
            }
        }
    }

    /// Snap-synced node at or above the derivation gate: hold an EMPTY root stamped
    /// at the snap height instead of seeding from producers. Returns `true` when the
    /// caller must not seed.
    pub(crate) async fn hold_snap_root_empty(&self, height: u64) -> bool {
        let Some(maintainer_state) = &self.maintainer_state else {
            return false;
        };
        let gate = self
            .config
            .network
            .params()
            .maintainer_derivation_activation_height;
        if height < gate {
            return false;
        }
        let Some(snap_h) = self.chain_state.read().await.snap_sync_height() else {
            return false;
        };
        let mut state = maintainer_state.write().await;
        if !Self::maintainer_seed_is_done(&state, true) {
            state.last_derived_height = snap_h;
            if let Err(e) = state.save(&self.config.data_dir) {
                warn!("Failed to persist maintainer state: {}", e);
            }
            warn!(
                "[MAINTAINER] snap-synced node: trust root stored EMPTY at height {}; waiting for a peer quorum (INC-I-248)",
                snap_h
            );
        }
        true
    }
}

fn verdict_label(verdict: &QuorumVerdict) -> String {
    match verdict {
        QuorumVerdict::Adopt { agreeing, .. } => format!("Adopt(agreeing={agreeing})"),
        QuorumVerdict::Keep(reason) => format!("Keep({reason:?})"),
    }
}
