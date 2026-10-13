//! INC-I-237 M2 — a fake node loop on a real `NetworkService`: admits txs into
//! a set keyed on `tx.hash()` (as the mempool is) and answers each gossip
//! message with ONE verdict through `send_gossip_verdict` (D1 / D5 shape).
//!
//! OUTPUT CONTRACT: N/A — fixture file (helpers only, asserts nothing).
//! INPUT PARTITIONS: N/A — fixture file.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crypto::Hash;
use network::service::{send_gossip_verdict, Verdict};
use network::{GossipHandle, NetworkCommand, NetworkEvent, NetworkService};
use tokio::sync::mpsc;

use super::inc_i_237_m2_common::tag;

#[derive(Clone, Copy, Debug)]
pub enum Policy {
    /// D5: Accept iff every tx is admissible and newly admitted by this message.
    Judge,
    /// Judge, but send the verdict this long after receipt.
    Hold(Duration),
    /// Never send a verdict (handles are kept for the test).
    Never,
    /// Send these two verdicts on the same handle, in order.
    Twice(Verdict, Verdict),
}

/// One delivered gossip message: tx tags and the local receive time.
pub type Deliveries = Arc<Mutex<Vec<(Vec<u64>, Instant)>>>;

pub struct Fake {
    pub delivered: Deliveries,
    pub kept: Arc<Mutex<Vec<GossipHandle>>>,
    pub policy: Arc<Mutex<Policy>>,
    pub cmd: mpsc::Sender<NetworkCommand>,
    pub peers: Arc<
        tokio::sync::RwLock<std::collections::HashMap<network::PeerId, network::peer::PeerInfo>>,
    >,
}

impl Fake {
    pub fn set(&self, p: Policy) {
        *self.policy.lock().unwrap() = p;
    }

    /// Delivered messages that carry any tag in `r`.
    pub fn count(&self, r: std::ops::Range<u64>) -> usize {
        count(&self.delivered, r)
    }

    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }
}

pub fn count(d: &Deliveries, r: std::ops::Range<u64>) -> usize {
    d.lock()
        .unwrap()
        .iter()
        .filter(|(tags, _)| tags.iter().any(|t| r.contains(t)))
        .count()
}

/// `bad(tag)` = the tx can never be admitted (wrong signature, unfunded, ...).
pub fn spawn_fake(mut svc: NetworkService, policy: Policy, bad: fn(u64) -> bool) -> Fake {
    let cmd = svc.command_sender();
    let peers = svc.peers_arc();
    let fake = Fake {
        delivered: Arc::default(),
        kept: Arc::default(),
        policy: Arc::new(Mutex::new(policy)),
        cmd: cmd.clone(),
        peers,
    };
    let (delivered, kept, pol) = (
        fake.delivered.clone(),
        fake.kept.clone(),
        fake.policy.clone(),
    );
    tokio::spawn(async move {
        let mut mempool: HashSet<Hash> = HashSet::new();
        while let Some(ev) = svc.next_event().await {
            let NetworkEvent::NewTransaction(txs, handle) = ev else {
                continue;
            };
            delivered
                .lock()
                .unwrap()
                .push((txs.iter().map(tag).collect(), Instant::now()));
            let mut all_new = !txs.is_empty();
            for t in &txs {
                let admitted = !bad(tag(t)) && mempool.insert(t.hash());
                all_new &= admitted;
            }
            let verdict = if all_new {
                Verdict::Accept
            } else {
                Verdict::Ignore
            };
            let policy = *pol.lock().unwrap();
            match policy {
                Policy::Judge => {
                    send_gossip_verdict(&cmd, handle, verdict);
                }
                Policy::Hold(d) => {
                    let cmd = cmd.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(d).await;
                        send_gossip_verdict(&cmd, handle, verdict);
                    });
                }
                Policy::Never => kept.lock().unwrap().push(handle),
                Policy::Twice(first, second) => {
                    send_gossip_verdict(&cmd, handle.clone(), first);
                    send_gossip_verdict(&cmd, handle, second);
                }
            }
        }
    });
    fake
}
