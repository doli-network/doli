//! RPC broadcast reservers (INC-I-237 R3): reserve a node->swarm command slot
//! before mempool admission and answer Busy when none is free.

use network::NetworkCommand;
use tokio::sync::mpsc;
use tracing::debug;

use crate::updater::VoteMessage;

/// Reserve one command slot; `None` when the channel is full or closed.
pub(crate) fn reserve_tx_broadcast(
    cmd_tx: &mpsc::Sender<NetworkCommand>,
) -> Option<rpc::TxBroadcastPermit> {
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
