//! Block answers for the GetBlockByHeight / GetBlockByHash / GetBodies sync requests.

use super::*;

/// False for a stored header whose body does not match it (the snap-anchor stub).
fn servable(block: &Block) -> bool {
    if block.transactions.is_empty() && !block.verify_merkle_root() {
        debug!("sync serve: withholding header-only block {}", block.hash());
        return false;
    }
    true
}

impl Node {
    /// Answer to GetBlockByHeight.
    pub fn serve_block_by_height(&self, height: u64) -> SyncResponse {
        match self.block_store.get_block_by_height(height) {
            Ok(Some(block)) if servable(&block) => SyncResponse::Block(Some(block)),
            _ => SyncResponse::Block(None),
        }
    }

    /// Answer to GetBlockByHash.
    pub fn serve_block_by_hash(&self, hash: &Hash) -> SyncResponse {
        match self.block_store.get_block(hash) {
            Ok(Some(block)) if servable(&block) => SyncResponse::Block(Some(block)),
            _ => SyncResponse::Block(None),
        }
    }

    /// Answer to GetBodies; hashes not stored or not servable are omitted.
    pub fn serve_bodies(&self, hashes: Vec<Hash>) -> SyncResponse {
        let mut bodies = Vec::new();
        for hash in hashes {
            if let Ok(Some(block)) = self.block_store.get_block(&hash) {
                if servable(&block) {
                    bodies.push(block);
                }
            }
        }
        SyncResponse::Bodies(bodies)
    }
}
