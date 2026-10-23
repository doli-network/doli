//! INC-I-236 block-signature gates (REQ-AUTH-007, REQ-AUTH-012, REQ-AUTH-020, REQ-AUTH-021).

use super::*;
use doli_core::BlockHeader;

/// `true` below the AH; at or above it, only if the producer's block signature verifies.
pub(super) fn gossip_header_authenticated(
    header: &BlockHeader,
    prospective_height: u64,
    activation_height: u64,
) -> bool {
    prospective_height < activation_height
        || doli_core::validation::verify_block_signature(header).is_ok()
}

/// Detector admission: every header below the slash gate; at or above it,
/// only a header that passes the consensus per-header evidence checks.
pub(super) fn detector_admits(
    header: &BlockHeader,
    prospective_height: u64,
    slashing_gate: u64,
    genesis_hash: &Hash,
    network: Network,
) -> bool {
    prospective_height < slashing_gate
        || doli_core::validation::evidence_header_admissible(header, genesis_hash, network).is_ok()
}

/// A fork block whose producer signature failed at its exact height.
#[derive(Debug)]
pub(crate) struct ForkSignatureRejected {
    hash: Hash,
    height: u64,
    reason: doli_core::validation::ValidationError,
}

impl std::fmt::Display for ForkSignatureRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "fork block {} at h={} rejected: {}",
            self.hash, self.height, self.reason
        )
    }
}

impl std::error::Error for ForkSignatureRejected {}

/// `true` when `result` failed on a fork block signature (the body is pi-blind to its hash).
pub(super) fn is_fork_signature_rejection(result: &Result<()>) -> bool {
    matches!(result, Err(e) if e.is::<ForkSignatureRejected>())
}

impl Node {
    /// Prospective-height gate for `handle_new_block`; logs and returns `false` on a drop.
    pub(super) fn gossip_block_authenticated(&self, block: &Block, best_height: u64) -> bool {
        let ah = self
            .config
            .network
            .params()
            .inc_i_236_block_signature_activation_height;
        let ok = gossip_header_authenticated(&block.header, best_height + 1, ah);
        if !ok {
            debug!("[AUTH] Dropping unauthenticated block {}", block.hash());
        }
        ok
    }

    /// Detector pass for `handle_new_block` at the prospective height and the local wall slot.
    pub(super) fn detector_check(
        &self,
        detector: &mut EquivocationDetector,
        block: &Block,
        best_height: u64,
    ) -> Option<EquivocationProof> {
        let gate = self
            .config
            .network
            .params()
            .effective_slashing_activation_height();
        let (chain, network) = (self.params.genesis_hash, self.config.network);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        detector.note_gate(best_height + 1, gate);
        detector.check_block_with(block, self.params.timestamp_to_slot(now), |h| {
            detector_admits(h, best_height + 1, gate, &chain, network)
        })
    }

    /// `on_new_block_event` pre-filter: the `handle_new_block` rule at the local best height + 1.
    pub(super) async fn gossip_event_authenticated(&self, block: &Block) -> bool {
        let best_height = self.chain_state.read().await.best_height;
        self.gossip_block_authenticated(block, best_height)
    }

    /// A present snapshot anchor must decode, hash to the quorum `block_hash`, and verify at
    /// `block_height`; an absent one passes. Logs the refusal.
    pub(super) fn snap_anchor_authenticated(&self, snapshot: &network::VerifiedSnapshot) -> bool {
        let Some(bytes) = &snapshot.block_header_bytes else {
            return true;
        };
        let ah = self
            .config
            .network
            .params()
            .inc_i_236_block_signature_activation_height;
        let verdict = match bincode::deserialize::<BlockHeader>(bytes) {
            Err(e) => Err(format!("undecodable anchor header: {e}")),
            Ok(h) if h.hash() != snapshot.block_hash => {
                Err(format!("anchor header hash {} != quorum hash", h.hash()))
            }
            Ok(h) => {
                doli_core::validation::verify_block_signature_at(&h, snapshot.block_height, ah)
                    .map_err(|e| e.to_string())
            }
        };
        if let Err(reason) = &verdict {
            warn!(
                "[SNAP_SYNC] Refusing snapshot h={} hash={}: {}",
                snapshot.block_height, snapshot.block_hash, reason
            );
        }
        verdict.is_ok()
    }

    /// Verify every fork block, `blocks[i]` at exact height `first_height + i`.
    pub(super) fn verify_fork_signed(&self, blocks: &[Block], first_height: u64) -> Result<()> {
        let ah = self
            .config
            .network
            .params()
            .inc_i_236_block_signature_activation_height;
        for (i, block) in blocks.iter().enumerate() {
            let height = first_height.saturating_add(i as u64);
            doli_core::validation::verify_block_signature_at(&block.header, height, ah).map_err(
                |reason| ForkSignatureRejected {
                    hash: block.hash(),
                    height,
                    reason,
                },
            )?;
        }
        Ok(())
    }
}
