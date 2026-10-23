//! Ledger of the slashes this node created, one per accused, kept for re-offer
//! until the accused is Slashed or slash-queued or the slash is refused for good.

use std::collections::{HashMap, VecDeque};

use crypto::PublicKey;
use doli_core::transaction::Transaction;

/// Most accused keys remembered; the oldest is forgotten first.
pub(crate) const MAX_REPORTED_SLASH_ACCUSED: usize = 1024;

#[derive(Default)]
pub(crate) struct SlashReported {
    txs: HashMap<PublicKey, Transaction>,
    order: VecDeque<PublicKey>,
    cursor: usize,
}

impl SlashReported {
    pub(crate) fn contains(&self, accused: &PublicKey) -> bool {
        self.txs.contains_key(accused)
    }

    /// Keep `tx` for `accused`; an existing entry is never replaced.
    pub(crate) fn mark(&mut self, accused: PublicKey, tx: Transaction) {
        if self.txs.contains_key(&accused) {
            return;
        }
        self.txs.insert(accused, tx);
        self.order.push_back(accused);
        while self.order.len() > MAX_REPORTED_SLASH_ACCUSED {
            if let Some(oldest) = self.order.pop_front() {
                self.txs.remove(&oldest);
            }
        }
    }

    pub(crate) fn retained(&self, accused: &PublicKey) -> Option<Transaction> {
        self.txs.get(accused).cloned()
    }

    pub(crate) fn forget(&mut self, accused: &PublicKey) {
        if self.txs.remove(accused).is_some() {
            self.order.retain(|k| k != accused);
        }
    }

    /// Up to `n` entries, round-robin from where the previous call stopped.
    pub(crate) fn next_batch(&mut self, n: usize) -> Vec<(PublicKey, Transaction)> {
        let len = self.order.len();
        if len == 0 {
            return Vec::new();
        }
        let start = self.cursor % len;
        let take = n.min(len);
        self.cursor = start + take;
        (0..take)
            .map(|i| self.order[(start + i) % len])
            .filter_map(|pk| self.retained(&pk).map(|tx| (pk, tx)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;

    fn tx() -> Transaction {
        Transaction::new_transfer(Vec::new(), Vec::new())
    }

    #[test]
    fn oldest_accused_forgotten_past_the_bound() {
        let mut r = SlashReported::default();
        let keys: Vec<PublicKey> = (0..=MAX_REPORTED_SLASH_ACCUSED)
            .map(|_| *KeyPair::generate().public_key())
            .collect();
        for k in &keys {
            r.mark(*k, tx());
            r.mark(*k, tx());
        }
        assert!(!r.contains(&keys[0]));
        assert!(r.contains(&keys[1]));
        assert!(r.contains(&keys[MAX_REPORTED_SLASH_ACCUSED]));
        assert_eq!(r.order.len(), MAX_REPORTED_SLASH_ACCUSED);
        assert_eq!(r.txs.len(), MAX_REPORTED_SLASH_ACCUSED);
    }
}
