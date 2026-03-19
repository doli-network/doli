//! Network command handling.
//!
//! Dispatches `NetworkCommand` variants to the appropriate libp2p swarm
//! operations (gossipsub publish, request-response send, dial, etc.).

use libp2p::gossipsub::IdentTopic;
use libp2p::Swarm;
use tracing::{debug, info, warn};

use doli_core::encode_producer_set;

use crate::behaviour::DoliBehaviour;
use crate::config::NetworkConfig;
use crate::gossip::{
    BLOCKS_TOPIC, HEADERS_TOPIC, HEARTBEATS_TOPIC, PRODUCERS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

use super::types::NetworkCommand;

/// Handle network commands
pub(super) async fn handle_command(
    command: NetworkCommand,
    swarm: &mut Swarm<DoliBehaviour>,
    config: &NetworkConfig,
) {
    match command {
        NetworkCommand::BroadcastBlock(block) => {
            let data = block.serialize();
            let topic = IdentTopic::new(BLOCKS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                warn!("Failed to broadcast block: {}", e);
            }
        }

        NetworkCommand::BroadcastHeader(header) => {
            let data = header.serialize();
            let topic = IdentTopic::new(HEADERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                warn!("Failed to broadcast header: {}", e);
            }
        }

        NetworkCommand::BroadcastTransaction(tx) => {
            let data = tx.serialize();
            let topic = IdentTopic::new(TRANSACTIONS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                warn!("Failed to broadcast transaction: {}", e);
            }
        }

        NetworkCommand::RequestStatus { peer_id, request } => {
            swarm.behaviour_mut().status.send_request(&peer_id, request);
        }

        NetworkCommand::RequestSync { peer_id, request } => {
            info!(
                "[SYNC_DEBUG] Sending sync request to peer={}, request={:?}",
                peer_id, request
            );
            swarm.behaviour_mut().sync.send_request(&peer_id, request);
        }

        NetworkCommand::SendStatusResponse { channel, response } => {
            let _ = swarm
                .behaviour_mut()
                .status
                .send_response(channel, response);
        }

        NetworkCommand::SendSyncResponse { channel, response } => {
            info!(
                "[SYNC_DEBUG] Sending sync response via channel, response_type={}",
                response.type_name()
            );
            let _ = swarm.behaviour_mut().sync.send_response(channel, response);
        }

        NetworkCommand::Connect(addr) => {
            // Skip dial if we're already connected to this peer (defense-in-depth)
            let already_connected = addr
                .iter()
                .find_map(|proto| match proto {
                    libp2p::multiaddr::Protocol::P2p(peer_id) => Some(peer_id),
                    _ => None,
                })
                .is_some_and(|peer_id| swarm.is_connected(&peer_id));

            if already_connected {
                debug!("Skipping dial to {} — already connected", addr);
            } else if let Err(e) = swarm.dial(addr.clone()) {
                warn!("Failed to dial {}: {}", addr, e);
            }
        }

        NetworkCommand::Disconnect(peer_id) => {
            let _ = swarm.disconnect_peer_id(peer_id);
        }

        NetworkCommand::Bootstrap => {
            if config.no_dht {
                debug!("DHT bootstrap skipped (--no-dht enabled)");
            } else if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
                warn!("Failed to bootstrap kademlia: {:?}", e);
            }
        }

        NetworkCommand::BroadcastProducers(data) => {
            let topic = IdentTopic::new(PRODUCERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                // Duplicate is expected if we broadcast the same list frequently
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast producer list: {}", e);
                }
            }
        }

        NetworkCommand::BroadcastProducerAnnouncements(announcements) => {
            let data = encode_producer_set(&announcements);
            let topic = IdentTopic::new(PRODUCERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast producer announcements: {}", e);
                }
            }
        }

        NetworkCommand::BroadcastProducerDigest(digest) => {
            // For now, we use gossipsub for digest broadcast
            // In the future, this could use a dedicated request-response protocol
            let data = doli_core::encode_digest(&digest);
            let topic = IdentTopic::new(PRODUCERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast producer digest: {}", e);
                }
            }
        }

        NetworkCommand::SendProducerDelta {
            peer_id,
            announcements,
        } => {
            // For direct peer communication, we use a direct gossip message
            // In the future, this could use request-response protocol
            let data = encode_producer_set(&announcements);
            debug!(
                "Sending producer delta ({} announcements) to {}",
                announcements.len(),
                peer_id
            );
            // Publish to gossip (all peers receive, but we log the intended target)
            let topic = IdentTopic::new(PRODUCERS_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to send producer delta to {}: {}", peer_id, e);
                }
            }
        }

        NetworkCommand::BroadcastVote(vote_data) => {
            let topic = IdentTopic::new(VOTES_TOPIC);
            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, vote_data) {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast vote: {}", e);
                }
            }
        }

        NetworkCommand::BroadcastHeartbeat(heartbeat_data) => {
            let topic = IdentTopic::new(HEARTBEATS_TOPIC);
            if let Err(e) = swarm
                .behaviour_mut()
                .gossipsub
                .publish(topic, heartbeat_data)
            {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast heartbeat: {}", e);
                }
            }
        }

        NetworkCommand::BroadcastAttestation(attestation_data) => {
            let topic = IdentTopic::new(crate::gossip::ATTESTATION_TOPIC);
            if let Err(e) = swarm
                .behaviour_mut()
                .gossipsub
                .publish(topic, attestation_data)
            {
                if !matches!(e, libp2p::gossipsub::PublishError::Duplicate) {
                    warn!("Failed to broadcast attestation: {}", e);
                }
            }
        }

        NetworkCommand::RequestTxFetch { peer_id, hashes } => {
            debug!("Requesting {} txs from peer {}", hashes.len(), peer_id);
            let request = crate::protocols::TxFetchRequest { hashes };
            swarm
                .behaviour_mut()
                .txfetch
                .send_request(&peer_id, request);
        }

        NetworkCommand::SendTxFetchResponse { channel, response } => {
            let _ = swarm
                .behaviour_mut()
                .txfetch
                .send_response(channel, response);
        }
    }
}
