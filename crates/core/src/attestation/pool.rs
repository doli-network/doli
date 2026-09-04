//! Bounded, node-local pool of BLS attestation signatures keyed by parent hash.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};

use crypto::{Hash, PublicKey};

/// Length of a BLS12-381 G2 signature in bytes.
const BLS_SIGNATURE_LEN: usize = 96;

/// Parent-hash keyed store of per-attester BLS signatures.
///
/// The parent window is a first-seen FIFO capped at [`Self::MAX_PARENTS`]: a new
/// parent beyond the cap evicts the oldest parent together with all of its
/// signatures. The first signature stored for a (parent, attester) pair is never
/// replaced, so a later relay cannot displace it.
///
/// Node-local scratch: never serialized, never persisted, never part of
/// `EpochState`.
#[derive(Debug, Default)]
pub struct ParentSignaturePool {
    pool: HashMap<Hash, HashMap<PublicKey, [u8; BLS_SIGNATURE_LEN]>>,
    recent: VecDeque<Hash>,
}

impl ParentSignaturePool {
    /// Number of parent hashes the window holds.
    pub const MAX_PARENTS: usize = 8;

    /// Create an empty pool.
    pub fn new() -> Self {
        Self {
            pool: HashMap::new(),
            recent: VecDeque::new(),
        }
    }

    /// Store `sig` under (`parent`, `attester`).
    ///
    /// Returns `true` if this call stored the signature, `false` if an entry for
    /// the pair already existed and was kept.
    pub fn insert(
        &mut self,
        parent: Hash,
        attester: PublicKey,
        sig: [u8; BLS_SIGNATURE_LEN],
    ) -> bool {
        if let Some(sigs) = self.pool.get_mut(&parent) {
            return match sigs.entry(attester) {
                Entry::Occupied(_) => false,
                Entry::Vacant(slot) => {
                    slot.insert(sig);
                    true
                }
            };
        }

        if self.recent.len() >= Self::MAX_PARENTS {
            if let Some(oldest) = self.recent.pop_front() {
                self.pool.remove(&oldest);
            }
        }

        let mut sigs = HashMap::new();
        sigs.insert(attester, sig);
        self.pool.insert(parent, sigs);
        self.recent.push_back(parent);
        true
    }

    /// Signature stored for (`parent`, `attester`), if any.
    pub fn get(&self, parent: &Hash, attester: &PublicKey) -> Option<&[u8; BLS_SIGNATURE_LEN]> {
        self.pool.get(parent)?.get(attester)
    }

    /// Every signature stored under `parent`.
    pub fn signatures_for(
        &self,
        parent: &Hash,
    ) -> Option<&HashMap<PublicKey, [u8; BLS_SIGNATURE_LEN]>> {
        self.pool.get(parent)
    }

    /// Whether `parent` is in the window.
    pub fn contains_parent(&self, parent: &Hash) -> bool {
        self.pool.contains_key(parent)
    }

    /// Number of parents in the window.
    pub fn parent_count(&self) -> usize {
        self.pool.len()
    }

    /// Total signatures held across every parent.
    pub fn total_signatures(&self) -> usize {
        self.pool.values().map(HashMap::len).sum()
    }

    /// Drop every parent and every signature.
    pub fn clear(&mut self) {
        self.pool.clear();
        self.recent.clear();
    }
}
