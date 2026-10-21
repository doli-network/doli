//! INC-I-236: a sync body at or above the AH must carry a valid producer signature.

use std::collections::{HashMap, HashSet};

use crypto::Hash;
use doli_core::validation::verify_block_signature_at;
use doli_core::Block;
use libp2p::PeerId;

use crate::sync::manager::SyncManager;

impl SyncManager {
    /// Requested bodies only; one at height >= AH whose signature fails stays needed.
    pub(super) fn take_requested_bodies(&mut self, peer: PeerId, bodies: Vec<Block>) -> Vec<Block> {
        let ah = self.config.inc_i_236_block_signature_activation_height;
        let heights = self.pending_heights_from_ah(&bodies);
        self.pipeline
            .body_downloader
            .process_response_refusing(peer, bodies, |hash, b| {
                heights
                    .get(hash)
                    .is_some_and(|h| verify_block_signature_at(&b.header, *h, ah).is_err())
            })
    }

    /// Heights of the pending headers named by `bodies` at or above the AH.
    /// The pending list ends at the header downloader's continuation height.
    fn pending_heights_from_ah(&self, bodies: &[Block]) -> HashMap<Hash, u64> {
        let pending = &self.pipeline.pending_headers;
        let first = match self.pipeline.header_downloader.next_height() {
            Some(next) => next.saturating_sub(pending.len() as u64),
            None => self.local_height.saturating_add(1),
        };
        let skip = self
            .config
            .inc_i_236_block_signature_activation_height
            .saturating_sub(first);
        let mut out = HashMap::new();
        if skip >= pending.len() as u64 {
            return out;
        }
        let mut wanted: HashSet<Hash> = bodies.iter().map(Block::hash).collect();
        for (i, header) in pending.iter().enumerate().skip(skip as usize) {
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
