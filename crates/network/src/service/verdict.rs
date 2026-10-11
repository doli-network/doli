//! Node relay verdicts on deferred gossip (INC-I-237 D1 / R5): carried back on
//! the command channel and reported once on the original message id.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use libp2p::gossipsub::{MessageAcceptance, MessageId};
use libp2p::PeerId;
use tokio::sync::mpsc;

use crate::gossip::staleness::GossipTopic;

use super::types::NetworkCommand;
use super::GossipHandle;

/// Last moment after receipt a verdict is reported; inside the ~4 s mcache window.
pub const VERDICT_DEADLINE: Duration = Duration::from_millis(3_500);

/// The node's relay verdict; there is no Reject (SEC-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    Ignore,
}

/// Per-topic count of verdicts that arrived after [`VERDICT_DEADLINE`].
pub struct LateVerdicts([AtomicU64; 5]);

/// Process-wide late-verdict counters the node exports.
pub static LATE_VERDICTS: LateVerdicts = LateVerdicts([const { AtomicU64::new(0) }; 5]);

impl LateVerdicts {
    pub fn record(&self, topic: GossipTopic) {
        self.slot(topic).fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(&self, topic: GossipTopic) -> u64 {
        self.slot(topic).load(Ordering::Relaxed)
    }

    fn slot(&self, topic: GossipTopic) -> &AtomicU64 {
        &self.0[match topic {
            GossipTopic::Blocks => 0,
            GossipTopic::Producers => 1,
            GossipTopic::Attestations => 2,
            GossipTopic::Votes => 3,
            GossipTopic::Transactions => 4,
        }]
    }
}

/// Report `verdict` on the handle's origin unless it is late (counted) or
/// unstamped; true iff `report` was called.
pub(crate) fn apply_gossip_verdict(
    handle: GossipHandle,
    verdict: Verdict,
    report: impl FnOnce(&MessageId, &PeerId, MessageAcceptance),
) -> bool {
    let Some((topic, msg_id, source)) = handle.origin() else {
        return false;
    };
    if handle.recv_at.elapsed() > VERDICT_DEADLINE {
        LATE_VERDICTS.record(*topic);
        return false;
    }
    let acceptance = match verdict {
        Verdict::Accept => MessageAcceptance::Accept,
        Verdict::Ignore => MessageAcceptance::Ignore,
    };
    report(msg_id, source, acceptance);
    true
}

/// Hand a verdict to the swarm without waiting; false when the channel is
/// full or the swarm is gone (the message then expires unreported).
pub fn send_gossip_verdict(
    cmd: &mpsc::Sender<NetworkCommand>,
    handle: GossipHandle,
    verdict: Verdict,
) -> bool {
    cmd.try_send(NetworkCommand::GossipVerdict { handle, verdict })
        .is_ok()
}
