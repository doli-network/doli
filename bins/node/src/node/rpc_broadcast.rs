//! RPC broadcast reservers (INC-I-237 R3): reserve a node->swarm command slot
//! before mempool admission and answer Busy when none is free.

use std::sync::{Arc, Mutex};

use network::rate_limit::TokenBucket;
use network::NetworkCommand;
use tokio::sync::mpsc;
use tracing::debug;

use crate::updater::VoteMessage;

/// Own-origin tx budget; mirrors a receiver's per-peer tx bucket (AUDIT-P2-511).
pub(crate) fn new_origin_tx_bucket() -> Arc<Mutex<TokenBucket>> {
    Arc::new(Mutex::new(TokenBucket::new(2_000, 500.0)))
}

/// Take one `origin` token, then reserve one command slot; `None` (Busy) when
/// the origin budget is spent or the channel is full or closed.
pub(crate) fn reserve_tx_broadcast(
    cmd_tx: &mpsc::Sender<NetworkCommand>,
    origin: &Mutex<TokenBucket>,
) -> Option<rpc::TxBroadcastPermit> {
    if !origin
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .try_consume(1)
    {
        return None;
    }
    let permit = cmd_tx.clone().try_reserve_owned().ok()?;
    Some(Box::new(move |tx| {
        permit.send(NetworkCommand::BroadcastTransaction(tx));
    }))
}

/// Send a vote to the swarm now and hand it to the local update service.
/// `false` when the command channel is full or closed.
pub(crate) fn broadcast_vote_now(
    cmd_tx: &mpsc::Sender<NetworkCommand>,
    local: Option<&mpsc::Sender<VoteMessage>>,
    vote_data: Vec<u8>,
) -> bool {
    let Ok(permit) = cmd_tx.try_reserve() else {
        return false;
    };
    let parsed = local.and_then(|l| {
        serde_json::from_slice::<VoteMessage>(&vote_data)
            .ok()
            .map(|v| (l, v))
    });
    permit.send(NetworkCommand::BroadcastVote(vote_data));
    if let Some((local, vote)) = parsed {
        if local.try_send(vote).is_err() {
            debug!("local vote channel full or closed: vote not delivered locally");
        }
    }
    true
}
