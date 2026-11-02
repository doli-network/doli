//! INC-I-289 K-1: verified gossip attesters of hashes the node holds only as fork
//! blocks (fork cache or retained tips). Read by the reorg doors only: never
//! attendance, the BLS pool or finality.

use std::collections::{HashMap, HashSet, VecDeque};

use crypto::{Hash, PublicKey};

/// Hashes tracked at most; the oldest hash is evicted first.
pub(crate) const FORK_CREDIT_CAP: usize = super::super::wedge_escape::WEDGE_RETAINED_CAP;

/// Each attester holds at most one credit (its latest by slot), so one key can
/// never push another key's credit out of the bounded map.
#[derive(Debug, Default)]
pub(crate) struct ForkCredit {
    attesters: HashMap<Hash, HashSet<PublicKey>>,
    order: VecDeque<Hash>,
    latest: HashMap<PublicKey, (Hash, u32)>,
}

impl ForkCredit {
    /// Record that `attester` attested `hash` at `slot`. The key's credit moves
    /// only to a strictly higher slot; a same-slot conflict keeps the first.
    pub(crate) fn credit(&mut self, hash: Hash, attester: PublicKey, slot: u32) {
        if let Some(&(held, held_slot)) = self.latest.get(&attester) {
            if held == hash || slot <= held_slot {
                return;
            }
            self.release(held, &attester);
        }
        if !self.attesters.contains_key(&hash) {
            if self.order.len() >= FORK_CREDIT_CAP {
                if let Some(old) = self.order.pop_front() {
                    for k in self.attesters.remove(&old).unwrap_or_default() {
                        self.latest.remove(&k);
                    }
                }
            }
            self.order.push_back(hash);
        }
        self.attesters.entry(hash).or_default().insert(attester);
        self.latest.insert(attester, (hash, slot));
    }

    fn release(&mut self, hash: Hash, attester: &PublicKey) {
        let Some(set) = self.attesters.get_mut(&hash) else {
            return;
        };
        set.remove(attester);
        if set.is_empty() {
            self.attesters.remove(&hash);
            self.order.retain(|h| *h != hash);
        }
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

    fn key() -> PublicKey {
        *crypto::KeyPair::generate().public_key()
    }

    fn held(c: &ForkCredit, x: Hash) -> Vec<PublicKey> {
        c.snapshot()
            .into_iter()
            .find(|(y, _)| *y == x)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }

    #[test]
    fn the_oldest_hash_is_evicted_at_the_cap() {
        let keys: Vec<PublicKey> = (0..=FORK_CREDIT_CAP).map(|_| key()).collect();
        let mut c = ForkCredit::default();
        for (i, k) in keys.iter().enumerate() {
            c.credit(h(i as u8), *k, 1);
        }
        c.credit(h(1), keys[1], 1);
        let held: HashSet<Hash> = c.snapshot().into_iter().map(|(x, _)| x).collect();
        assert_eq!(held.len(), FORK_CREDIT_CAP);
        assert!(!held.contains(&h(0)), "the first hash is evicted");
        assert!(held.contains(&h(FORK_CREDIT_CAP as u8)));
    }

    #[test]
    fn one_key_cannot_evict_another_keys_credit() {
        let (honest, junk) = (key(), key());
        let mut c = ForkCredit::default();
        c.credit(h(200), honest, 10);
        for i in 0..=FORK_CREDIT_CAP as u8 {
            c.credit(h(i), junk, 11 + i as u32);
        }
        assert_eq!(held(&c, h(200)), vec![honest], "S still holds H");
        assert_eq!(c.snapshot().len(), 2, "the junk key holds one hash");
    }

    #[test]
    fn a_credit_moves_only_to_a_strictly_higher_slot() {
        let k = key();
        let mut c = ForkCredit::default();
        c.credit(h(1), k, 10);
        c.credit(h(2), k, 9);
        c.credit(h(3), k, 10);
        assert_eq!(
            held(&c, h(1)),
            vec![k],
            "a lower or equal slot keeps the first"
        );
        assert!(held(&c, h(2)).is_empty() && held(&c, h(3)).is_empty());
        c.credit(h(4), k, 11);
        assert_eq!(c.snapshot(), vec![(h(4), vec![k])], "the old hash is freed");
        assert_eq!(c.order.len(), 1);
    }
}
