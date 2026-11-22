//! Anchor rule for `backfillFromPeer` (INC-I-308 R-1): a peer block is stored at h only if our chain commits to its hash.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crypto::Hash;
use doli_core::validation::verify_block_signature_at;
use doli_core::Block;
use storage::{BlockStore, ChainState};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::super::context::BackfillState;

/// Hash our chain commits to at `h`: `best_hash` at the tip, else `prev_hash` of the stored block at h+1.
pub(super) fn anchor_above(
    store: &BlockStore,
    h: u64,
    tip_height: u64,
    tip_hash: Hash,
) -> Option<Hash> {
    if h == tip_height {
        return Some(tip_hash);
    }
    if h > tip_height {
        return None;
    }
    store
        .get_block_by_height(h + 1)
        .ok()
        .flatten()
        .map(|b| b.header.prev_hash)
}

/// Blocks this walk verified by hash but did not store: height -> (hash, prev_hash).
pub(super) type Rejected = HashMap<u64, (Hash, Hash)>;

/// Like `anchor_above`, but the commitment may pass through rejected blocks of this walk: a rejected block
/// at k+1 gives its prev_hash only while the CURRENT chain still commits to its hash at k+1.
pub(super) fn anchor_now(
    store: &BlockStore,
    h: u64,
    tip_height: u64,
    tip_hash: Hash,
    rejected: &Rejected,
) -> Option<Hash> {
    let mut k = h;
    let mut through = Vec::new();
    let mut a = loop {
        if k >= tip_height {
            break anchor_above(store, k, tip_height, tip_hash)?;
        }
        if let Some(b) = store.get_block_by_height(k + 1).ok().flatten() {
            break b.header.prev_hash;
        }
        through.push(*rejected.get(&(k + 1))?);
        k += 1;
    };
    for (hash, prev) in through.into_iter().rev() {
        if a != hash {
            return None;
        }
        a = prev;
    }
    Some(a)
}

/// True when the stored block at `h` (header and body present) has exactly this hash.
pub(super) fn holds(store: &BlockStore, h: u64, want: &Hash) -> bool {
    matches!(store.get_block_by_height(h), Ok(Some(b)) if b.hash() == *want)
}

#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Store,
    SkipKeepChain(String),
    Break(String),
}

pub(super) fn admit(block: &Block, h: u64, want: &Hash, sig_ah: u64, bls_ah: u64) -> Verdict {
    let hash = block.hash();
    if hash != *want {
        return Verdict::Break(format!("peer block {hash} is not our chain's {want}"));
    }
    if let Err(e) = verify_block_signature_at(&block.header, h, sig_ah) {
        return Verdict::SkipKeepChain(e.to_string());
    }
    if !block.body_matches_header(bls_ah, Some(h)) {
        return Verdict::SkipKeepChain("body does not match its header".to_string());
    }
    Verdict::Store
}

async fn fetch(client: &reqwest::Client, url: &str, h: u64) -> Result<Block, String> {
    use base64::Engine;
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "method": "getBlockRaw", "params": { "height": h }, "id": h
        }))
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("JSON parse error: {e}"))?;
    let b64 = body
        .pointer("/result/block")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "peer has no block".to_string())?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("Base64 decode error: {e}"))?;
    let want = body
        .pointer("/result/blake3")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !want.is_empty() && crypto::hash::hash(&data).to_string() != want {
        return Err("BLAKE3 mismatch".to_string());
    }
    doli_core::transaction::legacy::deserialize_block_compat(&data)
        .ok_or_else(|| "deserialize error (tried current and legacy formats)".to_string())
}

pub(super) struct Walk {
    pub store: Arc<BlockStore>,
    pub chain: Arc<RwLock<ChainState>>,
    pub state: Arc<BackfillState>,
    pub url: String,
    pub tip_height: u64,
    pub tip_hash: Hash,
    pub sig_ah: u64,
    pub bls_ah: u64,
}

impl Walk {
    /// Fill `todo` (ascending) top-down from our tip; the first chain break stops the pass.
    pub(super) async fn run(self, todo: Vec<u64>) {
        let mut imported = 0u64;
        let mut skipped = 0u64;
        let result = self.walk(&todo, &mut imported, &mut skipped).await;
        if let Err((h, why)) = result {
            let total = self.state.total.load(Ordering::SeqCst);
            let msg = format!("stopped at height {h}: {why}; {imported} of {total} stored");
            warn!("Backfill failed: {msg}");
            *self.state.error.write().await = Some(msg);
        } else if skipped > 0 {
            warn!("Backfill done: {imported} imported, {skipped} skipped (signature or body rejected)");
        } else {
            info!("Backfill complete: {imported} blocks imported, chain filled");
        }
        self.state.running.store(false, Ordering::SeqCst);
    }

    async fn walk(
        &self,
        todo: &[u64],
        imported: &mut u64,
        skipped: &mut u64,
    ) -> Result<(), (u64, String)> {
        let client = reqwest::Client::new();
        let mut rejected = Rejected::new();
        let mut i = todo.len();
        while i > 0 {
            let b = todo[i - 1];
            let mut want = anchor_above(&self.store, b, self.tip_height, self.tip_hash)
                .ok_or_else(|| (b, format!("no stored block at {}", b + 1)))?;
            let mut h = b;
            while h > 0 {
                if i > 0 && todo[i - 1] == h {
                    i -= 1;
                } else if holds(&self.store, h, &want) {
                    break;
                } else {
                    self.state.total.fetch_add(1, Ordering::SeqCst);
                }
                let block = fetch(&client, &self.url, h).await.map_err(|e| (h, e))?;
                match admit(&block, h, &want, self.sig_ah, self.bls_ah) {
                    Verdict::Break(why) => return Err((h, why)),
                    Verdict::SkipKeepChain(why) => {
                        error!("Backfill: rejected block at height {h}: {why}");
                        *skipped += 1;
                        rejected.insert(h, (want, block.header.prev_hash));
                    }
                    Verdict::Store => {
                        self.store_at(&block, h, &rejected).await?;
                        *imported += 1;
                        self.state.imported.store(*imported, Ordering::SeqCst);
                        if imported.is_multiple_of(100) {
                            tokio::task::yield_now().await;
                        }
                    }
                }
                want = block.header.prev_hash;
                h -= 1;
            }
        }
        Ok(())
    }

    /// Writes only if the CURRENT chain still commits to `block` at h; the chain_state read lock spans re-check and write.
    async fn store_at(
        &self,
        block: &Block,
        h: u64,
        rejected: &Rejected,
    ) -> Result<(), (u64, String)> {
        let store_err = |e: storage::StorageError| (h, format!("store error: {e}"));
        let hash = block.hash();
        let cs = self.chain.read().await;
        let now = anchor_now(&self.store, h, cs.best_height, cs.best_hash, rejected);
        if now != Some(hash) {
            let why = match now {
                Some(x) => {
                    format!("chain moved at h {h}: our chain now commits to {x}, not {hash}")
                }
                None => format!("no stored block at {} or tip commits to {hash}", h + 1),
            };
            return Err((h, why));
        }
        if let Some(x) = self.store.get_hash_by_height(h).map_err(store_err)? {
            if x != hash {
                if self.store.get_block(&x).map_err(store_err)?.is_none() {
                    return Err((
                        h,
                        format!("our index at h holds {x}, our chain commits to {hash}"),
                    ));
                }
                self.store.remove_canonical_entry(h, x).map_err(store_err)?;
            }
        }
        let r = self.store.put_block_canonical(block, h).map_err(store_err);
        drop(cs);
        r
    }
}

/// Height list as "a-b, c" ranges for the reply.
pub(super) fn format_ranges(heights: &[u64]) -> String {
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < heights.len() {
        let start = heights[i];
        let mut end = start;
        while i + 1 < heights.len() && heights[i + 1] == end + 1 {
            i += 1;
            end = heights[i];
        }
        ranges.push(if start == end {
            format!("{start}")
        } else {
            format!("{start}-{end}")
        });
        i += 1;
    }
    ranges.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;
    use doli_core::BlockHeader;
    use vdf::{VdfOutput, VdfProof};

    fn blk(h: u64, prev: Hash, kp: &KeyPair) -> Block {
        let mut header = BlockHeader {
            version: 2,
            prev_hash: prev,
            merkle_root: doli_core::block::compute_merkle_root(&[]),
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 40_000 + h,
            slot: h as u32,
            producer: *kp.public_key(),
            vdf_output: VdfOutput {
                value: vec![h as u8; 32],
            },
            vdf_proof: VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        let msg = header.signing_message();
        header.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), kp.private_key()).to_vec();
        Block::new(header, vec![])
    }

    fn kp() -> KeyPair {
        KeyPair::from_seed([0x5A; 32])
    }

    #[test]
    fn admit_three_verdicts() {
        let b = blk(1, Hash::ZERO, &kp());
        assert_eq!(admit(&b, 1, &b.hash(), 0, u64::MAX), Verdict::Store);
        let other = Hash::from_bytes([1; 32]);
        assert!(matches!(
            admit(&b, 1, &other, 0, u64::MAX),
            Verdict::Break(_)
        ));
        let mut unsigned = b.clone();
        unsigned.header.vdf_proof.pi = vec![0; 64];
        let want = unsigned.hash();
        assert!(matches!(
            admit(&unsigned, 1, &want, 0, u64::MAX),
            Verdict::SkipKeepChain(_)
        ));
    }

    #[test]
    fn anchor_above_tip_stored_none_and_holds() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = BlockStore::open(tmp.path()).unwrap();
        let b1 = blk(1, Hash::ZERO, &kp());
        let b2 = blk(2, b1.hash(), &kp());
        store.put_block_canonical(&b2, 2).unwrap();
        let tip = Hash::from_bytes([9; 32]);
        assert_eq!(anchor_above(&store, 3, 3, tip), Some(tip));
        assert_eq!(anchor_above(&store, 1, 3, tip), Some(b1.hash()));
        assert_eq!(anchor_above(&store, 2, 3, tip), None);
        assert!(holds(&store, 2, &b2.hash()));
        assert!(!holds(&store, 2, &b1.hash()));
        assert!(!holds(&store, 1, &b1.hash()));
    }

    #[test]
    fn anchor_now_passes_rejected_only_while_chain_commits() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = BlockStore::open(tmp.path()).unwrap();
        let b1 = blk(1, Hash::ZERO, &kp());
        let b2 = blk(2, b1.hash(), &kp());
        let b3 = blk(3, b2.hash(), &kp());
        let b4 = blk(4, b3.hash(), &kp());
        store.put_block_canonical(&b4, 4).unwrap();
        let mut rej = Rejected::new();
        rej.insert(3, (b3.hash(), b2.hash()));
        rej.insert(2, (b2.hash(), b1.hash()));
        assert_eq!(anchor_now(&store, 1, 4, b4.hash(), &rej), Some(b1.hash()));
        let moved = Hash::from_bytes([7; 32]);
        assert_eq!(anchor_now(&store, 1, 3, moved, &rej), None);
        assert_eq!(anchor_now(&store, 1, 4, b4.hash(), &Rejected::new()), None);
        assert_eq!(anchor_now(&store, 5, 4, b4.hash(), &rej), None);
    }
}
