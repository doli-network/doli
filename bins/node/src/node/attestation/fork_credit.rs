//! INC-I-289 K-1: verified gossip attesters of hashes the node holds only as fork
//! blocks (fork cache or retained tips). Read by the reorg doors only: never
//! attendance, the BLS pool or finality.

use std::collections::{HashMap, HashSet, VecDeque};

use crypto::{Hash, PublicKey};

/// Hashes tracked at most; the oldest hash is evicted first.
pub(crate) const FORK_CREDIT_CAP: usize = super::super::wedge_escape::WEDGE_RETAINED_CAP;

#[derive(Debug, Default)]
pub(crate) struct ForkCredit {
    attesters: HashMap<Hash, HashSet<PublicKey>>,
    order: VecDeque<Hash>,
}

impl ForkCredit {
    /// Record that `attester` attested `hash`.
    pub(crate) fn credit(&mut self, hash: Hash, attester: PublicKey) {
        if !self.attesters.contains_key(&hash) {
            if self.order.len() >= FORK_CREDIT_CAP {
                if let Some(old) = self.order.pop_front() {
                    self.attesters.remove(&old);
                }
            }
            self.order.push_back(hash);
        }
        self.attesters.entry(hash).or_default().insert(attester);
    }

    /// Every tracked hash with its attesters.
    pub(crate) fn snapshot(&self) -> Vec<(Hash, Vec<PublicKey>)> {
        self.attesters
            .iter()
            .map(|(h, v)| (*h, v.iter().copied().collect()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(i: u8) -> Hash {
        Hash::from_bytes([i; 32])
    }

    #[test]
    fn the_oldest_hash_is_evicted_at_the_cap() {
        let pk = *crypto::KeyPair::generate().public_key();
        let mut c = ForkCredit::default();
        for i in 0..=FORK_CREDIT_CAP as u8 {
            c.credit(h(i), pk);
        }
        c.credit(h(1), pk);
        let held: HashSet<Hash> = c.snapshot().into_iter().map(|(x, _)| x).collect();
        assert_eq!(held.len(), FORK_CREDIT_CAP);
        assert!(!held.contains(&h(0)), "the first hash is evicted");
        assert!(held.contains(&h(FORK_CREDIT_CAP as u8)));
    }
}
