//! Maintainer quorum probe (INC-I-248): reply routing and the one-line verdict log.

use std::time::Instant;

use doli_core::maintainer::{maintainer_set_digest, QuorumVerdict};
use network::protocols::SyncResponse;
use network::PeerId;
use tracing::{debug, info, warn};

use super::Node;

impl Node {
    /// Take a reply to the node's own `GetMaintainerState`; any other response is
    /// handed back for the sync manager.
    pub(super) async fn route_maintainer_probe_reply(
        &self,
        peer: PeerId,
        response: SyncResponse,
    ) -> Option<SyncResponse> {
        match response {
            SyncResponse::MaintainerStateAt { .. } => {
                self.maintainer_probe().take_outstanding(&peer);
                self.on_maintainer_state_answer(peer, response).await;
                None
            }
            SyncResponse::Error(e) if self.maintainer_probe().take_outstanding(&peer) => {
                debug!("[MAINTAINER] probe error from {}: {}", peer, e);
                None
            }
            other => Some(other),
        }
    }

    /// Close one probe attempt with `verdict` and log it on one line.
    pub(super) fn finish_maintainer_quorum(
        &self,
        verdict: &QuorumVerdict,
        answers: usize,
        now: Instant,
    ) {
        let attempts = {
            let mut probe = self.maintainer_probe();
            probe.on_verdict(verdict, now);
            probe.attempts()
        };
        let phase = self.maintainer_probe_phase();
        let detail = match verdict {
            QuorumVerdict::Adopt { set, agreeing } => format!(
                "Adopt(agreeing={}) MAINTAINER_SET_DIGEST={} members={} threshold={} last_updated={}",
                agreeing,
                hex::encode(maintainer_set_digest(set, self.params.genesis_hash.as_bytes())),
                set.member_count(),
                set.threshold,
                set.last_updated
            ),
            QuorumVerdict::Keep(reason) => format!("Keep({reason:?})"),
        };
        if phase == "gave-up" {
            warn!(
                "[MAINTAINER] quorum verdict {} answers={} attempt={} phase={}; keeping the current trust root",
                detail, answers, attempts, phase
            );
        } else {
            info!(
                "[MAINTAINER] quorum verdict {} answers={} attempt={} phase={}",
                detail, answers, attempts, phase
            );
        }
    }
}
