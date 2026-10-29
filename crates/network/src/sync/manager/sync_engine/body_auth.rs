//! A sync body must match its header commitments (merkle root, presence root) and,
//! at or above the INC-I-236 AH, carry a valid producer signature.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crypto::Hash;
use doli_core::validation::verify_block_signature_at;
use doli_core::Block;
use libp2p::PeerId;

use crate::sync::manager::SyncManager;

const REFUSED_SUPPLIERS_CAP: usize = 64;

impl SyncManager {
    /// Requested bodies only; a refused one (bad signature at h >= AH, or a body
    /// that does not match its header at its height) stays needed. With the BLS AH
    /// unset (`u64::MAX`) the body is judged at an unknown height (either arm).
    pub(super) fn take_requested_bodies(&mut self, peer: PeerId, bodies: Vec<Block>) -> Vec<Block> {
        let sig_ah = self.config.inc_i_236_block_signature_activation_height;
        let bls_ah = self.config.inc_i_178_attestation_bls_activation_height;
        let heights = self.pending_heights(&bodies);
        let refused = RefCell::new(Vec::new());
        let accepted =
            self.pipeline
                .body_downloader
                .process_response_refusing(peer, bodies, |hash, b| {
                    let height = heights.get(hash).copied();
                    let body_height = height.filter(|_| bls_ah != u64::MAX);
                    let bad = height
                        .is_some_and(|h| verify_block_signature_at(&b.header, h, sig_ah).is_err())
                        || !b.body_matches_header(bls_ah, body_height);
                    if bad {
                        refused.borrow_mut().push(b.header.slot);
                    }
                    bad
                });
        let queue = &mut self.pipeline.refused_body_suppliers;
        for slot in refused.into_inner() {
            queue.push_back((peer, slot));
            if queue.len() > REFUSED_SUPPLIERS_CAP {
                queue.pop_front();
            }
        }
        accepted
    }

    /// (supplier, slot) of every sync body refused since the last call.
    pub fn take_refused_body_suppliers(&mut self) -> Vec<(PeerId, u32)> {
        self.pipeline.refused_body_suppliers.drain(..).collect()
    }

    /// Heights of the pending headers named by `bodies`. The pending list ends at
    /// the header downloader's continuation height.
    fn pending_heights(&self, bodies: &[Block]) -> HashMap<Hash, u64> {
        let pending = &self.pipeline.pending_headers;
        let first = match self.pipeline.header_downloader.next_height() {
            Some(next) => next.saturating_sub(pending.len() as u64),
            None => self.local_height.saturating_add(1),
        };
        let mut out = HashMap::new();
        let mut wanted: HashSet<Hash> = bodies.iter().map(Block::hash).collect();
        for (i, header) in pending.iter().enumerate() {
            if wanted.is_empty() {
                break;
            }
            let hash = header.hash();
            if wanted.remove(&hash) {
                out.insert(hash, first.saturating_add(i as u64));
            }
        }
        out
    }
}
