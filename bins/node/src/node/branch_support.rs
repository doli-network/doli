//! INC-I-289 K-1: who supports a block, in bond, for the reorg doors at/above the
//! INC-I-288 gate. Supporters = the block's producer, the attesters its bitfield
//! names (they attest the PARENT; counted only when the block is at/above the
//! INC-I-178 gate and its aggregate verifies), and the verified gossip attesters
//! held for it from OTHER producers. Bond = `selection_weight_at`, the finality unit.

use std::collections::HashMap;
use std::sync::Mutex;

use doli_core::presence_root_matches;
use network::BlockSupport;

use super::attestation::verify::{decide_attestation, Outcome};
use super::attestation::{commit, keys};
use super::*;

/// The facts one block contributes to its support.
struct Facts {
    parent: Hash,
    slot: u32,
    producer: PublicKey,
    bitfield: Vec<u8>,
    aggregate: Vec<u8>,
    presence_root: Hash,
}

impl Facts {
    fn of(b: &Block) -> Self {
        Self {
            parent: b.header.prev_hash,
            slot: b.header.slot,
            producer: b.header.producer,
            bitfield: b.attestation_bitfield.clone(),
            aggregate: b.aggregate_bls_signature.clone(),
            presence_root: b.header.presence_root,
        }
    }
}

/// A lock-free snapshot the doors' support closure reads.
pub(super) struct SupportView {
    ah178: u64,
    base: Vec<PublicKey>,
    producers: ProducerSet,
    bonds: HashMap<PublicKey, u64>,
    gossip: HashMap<Hash, Vec<PublicKey>>,
    /// Shared, so a support call never copies a bitfield or aggregate.
    cached: HashMap<Hash, Arc<Facts>>,
    /// Per block: the attesters its verified bits name (empty when unverified).
    verified: Mutex<HashMap<Hash, Vec<PublicKey>>>,
}

impl SupportView {
    fn bond(&self, pk: &PublicKey) -> (network::SupporterId, u64) {
        (*pk.as_bytes(), self.bonds.get(pk).copied().unwrap_or(0))
    }

    /// Adds a block the cache snapshot does not hold (a body taken out of the cache).
    pub(super) fn include(&mut self, block: &Block) {
        self.cached.insert(block.hash(), Arc::new(Facts::of(block)));
    }

    /// The same snapshot weighed with `table` (no block bytes copied).
    pub(super) fn with_table(&self, table: storage::EpochTable) -> Self {
        Self {
            ah178: self.ah178,
            producers: table.producer_set().unwrap_or_default(),
            bonds: table.bonds.into_iter().collect(),
            base: table.producer_list,
            gossip: self.gossip.clone(),
            cached: self.cached.clone(),
            verified: Mutex::new(HashMap::new()),
        }
    }

    fn facts(&self, store: &BlockStore, hash: &Hash) -> Option<Arc<Facts>> {
        match self.cached.get(hash) {
            Some(f) => Some(Arc::clone(f)),
            None => Some(Arc::new(Facts::of(&store.get_block(hash).ok().flatten()?))),
        }
    }

    /// The block's support; `None` when neither the store nor the cache holds it.
    pub(super) fn support(&self, store: &BlockStore, hash: &Hash) -> Option<BlockSupport> {
        let facts = self.facts(store, hash)?;
        let gossip = self.gossip.get(hash).into_iter().flatten();
        let backers = std::iter::once(&facts.producer)
            .chain(gossip)
            .map(|pk| self.bond(pk))
            .collect();
        let parent_attesters = self
            .verified_bits(store, hash, &facts)
            .iter()
            .map(|pk| self.bond(pk))
            .collect();
        let parent_slot = match self.cached.get(&facts.parent) {
            Some(p) => Some(p.slot),
            None => store
                .get_header(&facts.parent)
                .ok()
                .flatten()
                .map(|h| h.slot),
        };
        Some(BlockSupport {
            parent: facts.parent,
            backers,
            parent_attesters,
            slot: Some(facts.slot),
            parent_slot,
        })
    }

    fn verified_bits(&self, store: &BlockStore, hash: &Hash, f: &Facts) -> Vec<PublicKey> {
        let mut memo = self.verified.lock().unwrap_or_else(|e| e.into_inner());
        memo.entry(*hash)
            .or_insert_with(|| self.check_bits(store, hash, f).unwrap_or_default())
            .clone()
    }

    /// The attesters `f`'s bits name, or `None` (fail-closed): height unknown,
    /// below the INC-I-178 gate, or the aggregate check does not accept.
    fn check_bits(&self, store: &BlockStore, hash: &Hash, f: &Facts) -> Option<Vec<PublicKey>> {
        let height = self.height_of(store, hash)?;
        if height < self.ah178 {
            return None;
        }
        let active: Vec<PublicKey> = (self.producers)
            .active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect();
        let universe = commit::post_commit_universe_at(self.ah178, height, &self.base, &active);
        let (bf, agg) = (&f.bitfield, &f.aggregate);
        let root = presence_root_matches(self.ah178, Some(height), &f.presence_root, bf, agg);
        let set_bit_keys = keys::set_bit_bls_pubkeys(&universe, bf, &self.producers);
        if decide_attestation(root, bf, agg, &f.parent, false, set_bit_keys)
            != Ok(Outcome::Accepted)
        {
            return None;
        }
        let named = doli_core::decode_attestation_bitfield_vec(bf, universe.len());
        Some(
            named
                .into_iter()
                .filter_map(|i| universe.get(i))
                .copied()
                .collect(),
        )
    }

    /// Indexed height, else one more than the parent's (cache, then stored headers).
    fn height_of(&self, store: &BlockStore, hash: &Hash) -> Option<u64> {
        let mut at = *hash;
        for depth in 0..=(self.cached.len() as u64 + 64) {
            if let Some(h) = store.get_height_by_hash(&at).ok().flatten() {
                return h.checked_add(depth);
            }
            at = match self.cached.get(&at) {
                Some(f) => f.parent,
                None => store.get_header(&at).ok().flatten()?.prev_hash,
            };
        }
        None
    }
}

impl Node {
    /// The doors' INC-I-288 gate predicate at the local tip.
    pub(super) async fn weighs_by_support(&self) -> bool {
        let sync = self.sync_manager.read().await;
        let tip_h = sync.fork_choice_finality().local_tip_height;
        sync.reorg_handler().weighs_from_ancestor(tip_h)
    }

    /// INC-I-293 F1: the attested block is on the node (stored, cached or retained).
    pub(super) async fn attested_block_held(&self, hash: &Hash) -> bool {
        matches!(self.block_store.has_block(hash), Ok(true))
            || self.wedge_retained_tips.contains(hash)
            || self.fork_block_cache.read().await.contains_key(hash)
    }

    /// INC-I-293 F1/103: the recorded relayer while it is in the peer table, else
    /// `None`: no peer that showed the hash is connected, so no one is asked.
    pub(super) async fn attest_fetch_peer(&self, recorded: PeerId) -> Option<PeerId> {
        let sync = self.sync_manager.read().await;
        sync.has_peer(&recorded).then_some(recorded)
    }

    /// INC-I-293 103: a peer other than `skip` whose reported tip is off our chain
    /// (and is `tip` when given); lowest id first, never chosen by height.
    pub(super) fn branch_peer(
        &self,
        sm: &SyncManager,
        skip: &PeerId,
        tip: Option<Hash>,
    ) -> Option<PeerId> {
        (sm.peer_tips().into_iter())
            .filter(|(p, h)| p != skip && tip.is_none_or(|t| t == *h) && !self.is_canonical(h))
            .map(|(p, _)| p)
            .min()
    }

    /// INC-I-293 103: at/above the gate, before `gone` leaves the peer table, its
    /// tracker entries pass to a connected peer reporting the same off-chain tip
    /// (it showed the relayer's branch). No such peer: the entries stay unfetched.
    pub(super) async fn hand_off_attest_fetch(&mut self, gone: PeerId) {
        let named = self.attest_fetch_tracker.values().any(|e| e.2 == gone);
        if !named || !self.weighs_by_support().await {
            return;
        }
        let heir = {
            let sm = self.sync_manager.read().await;
            let tip = sm.peer_tips().into_iter().find(|(p, _)| *p == gone);
            tip.and_then(|(_, t)| self.branch_peer(&sm, &gone, Some(t)))
        };
        let Some(heir) = heir else { return };
        for e in self
            .attest_fetch_tracker
            .values_mut()
            .filter(|e| e.2 == gone)
        {
            e.2 = heir;
        }
    }

    /// [`Self::support_view`] at/above the gate toward the branch holding `from`;
    /// `None` below it (the doors read no support there) or when the common
    /// ancestor's epoch table is not kept (the doors then refuse the reorg).
    pub(super) async fn gated_support_view(&self, from: Hash) -> Option<SupportView> {
        if !self.weighs_by_support().await {
            return None;
        }
        let anchor = self.attach_point(from).await?;
        let table = self.ancestor_table(anchor).await?;
        Some(self.support_view(table).await)
    }

    /// Snapshot the inputs of [`SupportView::support`] (each lock taken and dropped).
    /// Bonds, decode list and BLS keys come from `table` (INC-I-289 L-1: the epoch
    /// of the common ancestor, one table for both branches).
    pub(super) async fn support_view(&self, table: storage::EpochTable) -> SupportView {
        crate::metrics::SUPPORT_VIEWS_BUILT.inc();
        let producers = table.producer_set().unwrap_or_default();
        let bonds = table.bonds.into_iter().collect();
        // The node's own attestation is its tip echoed back, not network evidence.
        let own = self.producer_key.as_ref().map(|k| *k.public_key());
        let mut gossip: HashMap<Hash, Vec<PublicKey>> = HashMap::new();
        let pending = (self.sync_manager.read().await).pending_attesters();
        let fork = self.attest_seen.fork_credit.snapshot();
        for (h, v) in pending.into_iter().chain(fork) {
            let others = v.into_iter().filter(|pk| Some(*pk) != own);
            gossip.entry(h).or_default().extend(others);
        }
        let cached = (self.fork_block_cache.read().await)
            .iter()
            .map(|(h, b)| (*h, Arc::new(Facts::of(b))))
            .collect();
        SupportView {
            ah178: self.inc_i_178_attestation_bls_activation_height,
            base: table.producer_list,
            producers,
            bonds,
            gossip,
            cached,
            verified: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OUTPUT CONTRACT: SupportView::facts(store, hash) -> Option<Arc<Facts>>, the
    //   handle support() reads. O1 two calls for one cached hash share one bitfield
    //   allocation (no per-call copy); O2 the handle holds the block's bitfield.
    //   PATHS: cached hash. PARTITIONS: a 64 KiB bitfield (the per-call copy the
    //   ranking repeats once per path offset).
    // P1b — Decision: a failure means the ranking copies each cached bitfield per call.
    #[test]
    fn p1b_support_facts_are_shared_not_cloned() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = BlockStore::open(tmp.path()).unwrap();
        let header = doli_core::BlockHeader {
            version: 1,
            prev_hash: Hash::ZERO,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: 10,
            slot: 1,
            producer: crypto::PublicKey::from_bytes([1; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        let mut b = Block::new(header, vec![]);
        b.attestation_bitfield = vec![0u8; 64 * 1024];
        let mut view = SupportView {
            ah178: u64::MAX,
            base: Vec::new(),
            producers: ProducerSet::new(),
            bonds: HashMap::new(),
            gossip: HashMap::new(),
            cached: HashMap::new(),
            verified: Mutex::new(HashMap::new()),
        };
        view.include(&b);
        let (a, c) = (view.facts(&store, &b.hash()), view.facts(&store, &b.hash()));
        let (a, c) = (a.expect("cached facts"), c.expect("cached facts"));
        assert!(
            std::ptr::eq(a.bitfield.as_ptr(), c.bitfield.as_ptr()),
            "P1b: support() copied the bitfield on a repeated call"
        );
        assert_eq!(a.bitfield, b.attestation_bitfield, "P1b: wrong facts");
    }
}
