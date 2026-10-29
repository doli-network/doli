//! A fork body copy is untrusted until apply succeeds. A copy that fails is
//! evicted, its supplier is recorded as an invalid-block source, and the body is
//! fetched again by hash from a different peer, at most `MAX_REFETCH` times per
//! height. A body failure never refuses its header or branch.

use std::collections::VecDeque;

use super::fork_walk::cached_branch_tip;
use super::reorg_refusals::content_key;
use super::*;

mod sites;
#[cfg(test)]
mod tests;
mod walk;

const CAP: usize = 256;
const MAX_REFETCH: u32 = 3;
const ASK_TIMEOUT: Duration = Duration::from_secs(10);

/// `Junk`: the copy failed the commitment pre-filter (refused on content, no
/// rollback). `Failed`: it passed the pre-filter and failed apply or replays a
/// refused content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fault {
    Junk,
    Failed,
}

/// A failed copy of a fork block; `source` = (supplier, slot) when known.
pub(super) struct BodyFault {
    pub(super) hash: Hash,
    pub(super) key: Hash,
    pub(super) height: Option<u64>,
    pub(super) source: Option<(PeerId, u32)>,
    pub(super) kind: Fault,
}

impl BodyFault {
    pub(super) fn of(
        block: &Block,
        height: Option<u64>,
        supplier: Option<PeerId>,
        kind: Fault,
    ) -> Self {
        Self {
            hash: block.hash(),
            key: content_key(block),
            height,
            source: supplier.map(|p| (p, block.header.slot)),
            kind,
        }
    }
}

/// Faults for the `blocks` (at `base + 1 + i`, served by `suppliers[i]`) whose hash is in `hashes`.
pub(super) fn faults_of(
    blocks: &[Block],
    hashes: &[Hash],
    base: Option<u64>,
    suppliers: &[Option<PeerId>],
    kind: Fault,
) -> Vec<BodyFault> {
    blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| hashes.contains(&b.hash()))
        .map(|(i, b)| {
            let height = base.map(|h| h.saturating_add(i as u64 + 1));
            BodyFault::of(b, height, suppliers.get(i).copied().flatten(), kind)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetryKey {
    Height(u64),
    Hash(Hash),
}

/// Who supplied each cached fork body, who failed which hash, and the by-hash
/// refetches in flight. Every list is FIFO-bounded at `CAP`.
#[derive(Debug, Default)]
pub struct BodyRetry {
    intake: Option<(Hash, PeerId)>,
    supplied: VecDeque<Supplied>,
    /// `Failed` suppliers: their copies of that hash are refused and they are never asked.
    failed: VecDeque<(Hash, PeerId)>,
    /// `Junk` suppliers: never asked for that hash.
    junk: VecDeque<(Hash, PeerId)>,
    attempts: VecDeque<(RetryKey, u32)>,
    /// (hash, peer, when, gate): a gate ask refuses other peers' copies until `ASK_TIMEOUT`.
    asked: VecDeque<(Hash, PeerId, Instant, bool)>,
}

fn push_capped<T>(q: &mut VecDeque<T>, v: T) {
    q.push_back(v);
    if q.len() > CAP {
        q.pop_front();
    }
}

/// (hash, content key, supplier, slot) of a cached copy.
pub(super) type Supplied = (Hash, Hash, PeerId, u32);

impl BodyRetry {
    pub(super) fn note_cached(&mut self, (hash, key, peer, slot): Supplied) {
        self.note_supplier(hash, key, peer, slot);
    }

    fn note_supplier(&mut self, hash: Hash, key: Hash, peer: PeerId, slot: u32) {
        self.supplied.retain(|e| e.0 != hash);
        push_capped(&mut self.supplied, (hash, key, peer, slot));
    }

    fn supplier_of(&self, hash: &Hash, key: &Hash) -> Option<(PeerId, u32)> {
        self.supplied
            .iter()
            .find(|e| e.0 == *hash && e.1 == *key)
            .map(|e| (e.2, e.3))
    }

    fn fail(&mut self, hash: Hash, peer: PeerId, kind: Fault) {
        let list = match kind {
            Fault::Junk => &mut self.junk,
            Fault::Failed => &mut self.failed,
        };
        if !list.contains(&(hash, peer)) {
            push_capped(list, (hash, peer));
        }
    }

    fn has_failed(&self, hash: &Hash, peer: &PeerId) -> bool {
        self.failed.contains(&(*hash, *peer))
    }

    fn never_ask(&self, hash: &Hash, peer: &PeerId) -> bool {
        self.has_failed(hash, peer) || self.junk.contains(&(*hash, *peer))
    }

    /// True when `peer`'s copy of `hash` may not fill the cache slot: its copy of
    /// `hash` failed after the pre-filter, or another peer is asked under a gate.
    fn refuses(&self, hash: &Hash, peer: &PeerId, now: Instant) -> bool {
        self.has_failed(hash, peer)
            || self.asked.iter().any(|(h, p, t, gate)| {
                *gate && h == hash && p != peer && now.saturating_duration_since(*t) < ASK_TIMEOUT
            })
    }

    fn asking(&self, hash: &Hash, now: Instant) -> bool {
        self.asked
            .iter()
            .any(|(h, _, t, _)| h == hash && now.saturating_duration_since(*t) < ASK_TIMEOUT)
    }

    /// Counts and returns the next attempt for `key`; `None` past `MAX_REFETCH`.
    fn take_attempt(&mut self, key: RetryKey) -> Option<u32> {
        if let Some(e) = self.attempts.iter_mut().find(|e| e.0 == key) {
            if e.1 >= MAX_REFETCH {
                return None;
            }
            e.1 += 1;
            return Some(e.1);
        }
        push_capped(&mut self.attempts, (key, 1));
        Some(1)
    }

    fn ask(&mut self, hash: Hash, peer: PeerId, now: Instant, gate: bool) {
        self.asked.retain(|e| e.0 != hash);
        push_capped(&mut self.asked, (hash, peer, now, gate));
    }

    /// True (and cleared) when `peer` is the one asked for `hash`.
    fn answered(&mut self, hash: &Hash, peer: &PeerId) -> bool {
        let before = self.asked.len();
        self.asked.retain(|(h, p, _, _)| !(h == hash && p == peer));
        self.asked.len() != before
    }
}

/// The peer to ask: not `excluded`; at or above `height` first, then the highest, then the lowest id.
fn pick_peer(
    peers: Vec<(PeerId, u64)>,
    height: Option<u64>,
    excluded: impl Fn(&PeerId) -> bool,
) -> Option<PeerId> {
    let h = height.unwrap_or(0);
    peers
        .into_iter()
        .filter(|(p, _)| !excluded(p))
        .min_by_key(|(p, ph)| (*ph < h, std::cmp::Reverse(*ph), p.to_bytes()))
        .map(|(p, _)| p)
}

impl Node {
    /// Handle a new block from the network. A refetched body that its asked peer
    /// delivers re-evaluates the cached branch it heads.
    pub async fn handle_new_block(&mut self, block: Block, source_peer: PeerId) -> Result<()> {
        let hash = block.hash();
        let answered = self.body_retry.answered(&hash, &source_peer);
        self.body_retry.intake = Some((hash, source_peer));
        let result = self.process_block(block, source_peer).await;
        self.body_retry.intake = None;
        if answered {
            self.retry_branch(hash).await;
        }
        result
    }

    /// Fork choice for the tip of the cached chain that descends from `hash`.
    async fn retry_branch(&mut self, hash: Hash) {
        let tip = {
            let cache = self.fork_block_cache.read().await;
            cache.get(&cached_branch_tip(&cache, hash)).cloned()
        };
        let Some(tip) = tip.filter(|t| t.hash() != hash) else {
            return;
        };
        info!(
            "[BODY_RETRY] refetched body {:.16} cached — re-evaluating branch tip {:.16}",
            hash,
            tip.hash()
        );
        if let Err(e) = self.retain_sibling_and_try_escape(tip).await {
            warn!("[BODY_RETRY] re-evaluation after refetch failed: {}", e);
        }
    }

    /// The gate a peer copy passes before `fork_cache::admit`: false when its
    /// supplier may not fill the slot, or when it does not match its header (a
    /// junk body of its supplier).
    pub(super) async fn screen_peer_copy(&mut self, block: &Block, height: Option<u64>) -> bool {
        let hash = block.hash();
        let supplier = self.intake_supplier(&hash);
        if let Some(p) = supplier.filter(|p| self.body_retry.refuses(&hash, p, Instant::now())) {
            debug!(
                "[BODY_RETRY] copy of {:.16} from {} not cached: that peer failed it, or another peer is asked (INC-I-281)",
                hash, p
            );
            return false;
        }
        let ah = self.inc_i_178_attestation_bls_activation_height;
        if !block.body_matches_header(ah, height) {
            self.body_failed(BodyFault::of(block, height, supplier, Fault::Junk))
                .await;
            return false;
        }
        true
    }

    /// The handle_new_block peer of `hash`, if that block is the one being handled.
    fn intake_supplier(&self, hash: &Hash) -> Option<PeerId> {
        self.body_retry
            .intake
            .filter(|(h, _)| h == hash)
            .map(|(_, p)| p)
    }

    /// (hash, content key, peer, slot) of `block` when it is the block being handled.
    pub(super) fn intake_copy(&self, block: &Block) -> Option<Supplied> {
        let hash = block.hash();
        let peer = self.intake_supplier(&hash)?;
        Some((hash, content_key(block), peer, block.header.slot))
    }

    /// A failed copy of a fork block: evict it, record its supplier in the
    /// scorer, and ask a different peer for the body by hash. A junk copy is
    /// fetched again only when no copy is held and no ask is open.
    pub(super) async fn body_failed(&mut self, fault: BodyFault) {
        let BodyFault {
            hash,
            key,
            height,
            source,
            kind,
        } = fault;
        self.evict_cached_body(&hash, key).await;
        let source = source.or_else(|| self.body_retry.supplier_of(&hash, &key));
        if let Some((peer, slot)) = source {
            self.bls_ingress_scorer.record_invalid_block(&peer, slot);
            self.body_retry.fail(hash, peer, kind);
        }
        let gate = kind == Fault::Failed;
        if !gate && (self.body_held(&hash).await || self.body_retry.asking(&hash, Instant::now())) {
            return;
        }
        self.refetch_body(hash, height, gate).await;
    }

    /// True when the fork cache or the block store holds a body for `hash`.
    async fn body_held(&self, hash: &Hash) -> bool {
        self.fork_block_cache.read().await.contains_key(hash)
            || matches!(self.block_store.get_header(hash), Ok(Some(_)))
    }

    /// Asks a peer that never failed `hash` for its body by hash, at most
    /// `MAX_REFETCH` times per height (per hash when the height is unknown).
    async fn refetch_body(&mut self, hash: Hash, height: Option<u64>, gate: bool) {
        let at = height.map_or_else(|| "unknown".to_string(), |h| h.to_string());
        let peers = self.sync_manager.read().await.peer_heights();
        let Some(peer) = pick_peer(peers, height, |p| self.body_retry.never_ask(&hash, p)) else {
            warn!(
                "[BODY_RETRY] no other peer to ask for {:.16} (height {}) — left to fork recovery (INC-I-281)",
                hash, at
            );
            return;
        };
        let retry = height.map_or(RetryKey::Hash(hash), RetryKey::Height);
        let Some(attempt) = self.body_retry.take_attempt(retry) else {
            warn!(
                "[BODY_RETRY] {:.16} (height {}): {} refetches done — left to fork recovery (INC-I-281)",
                hash, at, MAX_REFETCH
            );
            return;
        };
        info!(
            "[BODY_REFETCH] asking {} for {} by hash (height {}, attempt {}/{})",
            peer, hash, at, attempt, MAX_REFETCH
        );
        if let Some(ref network) = self.network {
            let request = network::protocols::SyncRequest::get_block_by_hash(hash);
            let _ = network.request_sync(peer, request).await;
        }
        self.body_retry.ask(hash, peer, Instant::now(), gate);
    }
}
