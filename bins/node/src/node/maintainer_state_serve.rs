//! Serve seam for `SyncRequest::GetMaintainerState` (INC-I-248).

use doli_core::maintainer::maintainer_set_digest;
use doli_core::MaintainerSet;
use network::protocols::SyncResponse;
use tracing::warn;

use super::Node;

impl Node {
    /// This node's maintainer trust root at its tip; an EMPTY set when it holds none.
    pub async fn serve_maintainer_state(&self) -> SyncResponse {
        let (set, last_derived_height) = match &self.maintainer_state {
            Some(ms) => {
                let state = ms.read().await;
                (state.set.clone(), state.last_derived_height)
            }
            None => (MaintainerSet::new(), 0),
        };
        let (block_hash, block_height) = {
            let cs = self.chain_state.read().await;
            (cs.best_hash, cs.best_height)
        };
        let digest = maintainer_set_digest(&set, self.params.genesis_hash.as_bytes());
        let set_bytes = bincode::serialize(&set).unwrap_or_else(|e| {
            warn!("[MAINTAINER] failed to encode the maintainer set: {}", e);
            Vec::new()
        });
        SyncResponse::MaintainerStateAt {
            block_hash,
            block_height,
            digest: crypto::Hash::from_bytes(digest),
            set_bytes,
            last_derived_height,
        }
    }
}
