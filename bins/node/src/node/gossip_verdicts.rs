//! Node relay verdicts on deferred attestation, vote and producer gossip
//! (INC-I-237 R2); every verdict goes through the M2 `send_gossip_verdict` seam.

use super::*;

use super::attestation::ingress::AttestSeen;
use network::service::{send_gossip_verdict, Verdict};
use network::GossipHandle;

impl Node {
    /// Hand one relay verdict to the swarm without waiting.
    pub(super) fn relay_verdict(&self, handle: GossipHandle, verdict: Verdict) {
        if let Some(net) = &self.network {
            send_gossip_verdict(&net.command_sender(), handle, verdict);
        }
    }

    /// Ingest a gossip attestation and return its relay verdict: Accept iff it
    /// decodes, its Ed25519 signature verifies, the attester is in the producer
    /// set (pending activation included) and either its `(attester, slot)` is
    /// first seen or this copy is the first with a valid BLS half for that
    /// `(attester, slot)` (AUDIT-P1-506, REQ-GRV-023). An unknown block is
    /// fetched only for members (F-9).
    pub async fn on_new_attestation(&mut self, data: Vec<u8>, source_peer: PeerId) -> Verdict {
        let Some(attestation) = Attestation::from_bytes(&data).filter(|a| a.verify().is_ok())
        else {
            debug!("Received undecodable or invalid-signature attestation");
            return Verdict::Ignore;
        };
        let member = self
            .producer_set
            .read()
            .await
            .get_by_pubkey(&attestation.attester)
            .is_some();
        if !member {
            debug!(
                "[ATTEST_INGEST] not relaying attestation from non-member {:.8}",
                attestation.attester
            );
            return Verdict::Ignore;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.attest_seen
            .prune_wall(self.params.timestamp_to_slot(now));
        let key = (attestation.attester, attestation.slot);
        let relay = |seen: &mut AttestSeen, bls_valid: bool| {
            if seen.admit(key, bls_valid) {
                Verdict::Accept
            } else {
                Verdict::Ignore
            }
        };

        // Authority is derived at the attested block's LOCAL height (D4).
        let local_height = match self.block_store.get_height_by_hash(&attestation.block_hash) {
            Ok(Some(h)) => h,
            _ => {
                self.maybe_fetch_attested_block(&attestation.block_hash, source_peer)
                    .await;
                let bls_valid =
                    self.attest_seen.awaits_bls(&key) && self.relay_bls_valid(&attestation).await;
                return relay(&mut self.attest_seen, bls_valid);
            }
        };
        let Some((weight, bls_valid)) = self
            .ingest_attestation(&attestation, local_height, source_peer)
            .await
        else {
            return relay(&mut self.attest_seen, false);
        };

        // Finality numerator: only a positive locally-derived weight counts.
        if weight > 0 {
            let mut sync = self.sync_manager.write().await;
            sync.add_attestation_weight(&attestation.block_hash, attestation.attester, weight);
            drop(sync);
        }
        self.flush_finalized_to_archive().await;
        relay(&mut self.attest_seen, bls_valid)
    }

    /// Hand a gossip vote to the updater and return its relay verdict: Accept
    /// iff it decodes, its signature verifies against `producer_id` and the
    /// voter is an active producer (the set the updater tally counts, X6).
    /// A vote whose voter id or signature field is malformed is Ignored and
    /// recorded against `source` on the record-only `vote_ingress_scorer`.
    pub async fn on_new_vote(&mut self, vote_data: Vec<u8>, source: Option<PeerId>) -> Verdict {
        debug!("Received vote message ({} bytes)", vote_data.len());
        let vote_msg = match serde_json::from_slice::<node_updater::VoteMessage>(&vote_data) {
            Ok(v) => v,
            Err(e) => {
                debug!("Failed to decode vote message: {}", e);
                return Verdict::Ignore;
            }
        };
        let Some(voter) = super::vote_ingress::parse_voter(&vote_msg) else {
            self.report_malformed_vote(source);
            return Verdict::Ignore;
        };
        if !vote_msg.verify(&vote_msg.producer_id) {
            debug!(
                "Rejected gossip vote with invalid signature from {:.16}",
                voter.to_hex()
            );
            return Verdict::Ignore;
        }
        let active = self
            .producer_set
            .read()
            .await
            .get_by_pubkey(&voter)
            .is_some_and(|p| p.is_active());
        info!(
            "Vote received via gossip: {} vote for v{} from {:.16}",
            if vote_msg.vote == node_updater::Vote::Veto {
                "VETO"
            } else {
                "APPROVE"
            },
            vote_msg.version,
            voter.to_hex()
        );
        if let Some(ref vote_tx) = self.vote_tx {
            let _ = vote_tx.try_send(vote_msg);
        }
        if active {
            Verdict::Accept
        } else {
            Verdict::Ignore
        }
    }

    /// Record and log a malformed vote from `source`, at most once per peer per window.
    fn report_malformed_vote(&mut self, source: Option<PeerId>) {
        let window = super::vote_ingress::MALFORMED_VOTE_WINDOW;
        if let Some(peer) = source.filter(|p| self.vote_ingress_throttle.should_report(*p, window))
        {
            self.vote_ingress_scorer.record_malformed(&peer);
            warn!("Malformed gossip vote dropped from peer {}", peer);
        }
    }

    /// GSet announcement merge (CPU-heavy, spawned); at most one merge in flight.
    /// The verdict follows the merge: Accept iff it added a verified entry. A
    /// batch dropped while a merge runs is Ignored (the digest exchange repairs it).
    pub fn on_producer_announcements(
        &self,
        announcements: Vec<ProducerAnnouncement>,
        handle: GossipHandle,
    ) {
        let Some(slot) = super::producer_merge::MergeSlot::acquire(&self.announcement_merge_busy)
        else {
            debug!("Producer announcements dropped: a GSet merge is in flight");
            self.relay_verdict(handle, Verdict::Ignore);
            return;
        };
        let cmd = self.network.as_ref().map(|n| n.command_sender());
        let gset = self.producer_gset.clone();
        let adaptive = self.adaptive_gossip.clone();
        let sync_mgr = self.sync_manager.clone();
        let known_producers = self.known_producers.clone();

        tokio::spawn(async move {
            let _slot = slot;
            let merge_result = gset.write().await.merge(announcements);
            if let Some(cmd) = &cmd {
                let verdict = if merge_result.added > 0 {
                    Verdict::Accept
                } else {
                    Verdict::Ignore
                };
                send_gossip_verdict(cmd, handle, verdict);
            }
            let peer_count = sync_mgr.read().await.peer_count();
            adaptive
                .write()
                .await
                .on_gossip_result(&merge_result, peer_count);

            if merge_result.added > 0 {
                info!(
                    "Producer announcements: added={}, new_producers={}, rejected={}, duplicates={}",
                    merge_result.added, merge_result.new_producers,
                    merge_result.rejected, merge_result.duplicates
                );
                let producers = gset.read().await.sorted_producers();
                let mut known = known_producers.write().await;
                for pubkey in producers {
                    if !known.contains(&pubkey) {
                        known.push(pubkey);
                    }
                }
                known.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            } else if merge_result.rejected > 0 {
                debug!(
                    "Producer announcements rejected: {} (invalid signature, stale, or wrong network)",
                    merge_result.rejected
                );
            }
        });
    }
}
