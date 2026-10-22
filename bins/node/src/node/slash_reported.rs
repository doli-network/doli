//! Accused producers this node already submitted an admitted slash for.

use std::collections::{HashSet, VecDeque};

use crypto::PublicKey;

/// Most accused keys remembered; the oldest is forgotten first.
pub(crate) const MAX_REPORTED_SLASH_ACCUSED: usize = 1024;

#[derive(Default)]
pub(crate) struct SlashReported {
    keys: HashSet<PublicKey>,
    order: VecDeque<PublicKey>,
}

impl SlashReported {
    pub(crate) fn contains(&self, accused: &PublicKey) -> bool {
        self.keys.contains(accused)
    }

    pub(crate) fn mark(&mut self, accused: PublicKey) {
        if !self.keys.insert(accused) {
            return;
        }
        self.order.push_back(accused);
        while self.order.len() > MAX_REPORTED_SLASH_ACCUSED {
            if let Some(oldest) = self.order.pop_front() {
                self.keys.remove(&oldest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::KeyPair;

    #[test]
    fn oldest_accused_forgotten_past_the_bound() {
        let mut r = SlashReported::default();
        let keys: Vec<PublicKey> = (0..=MAX_REPORTED_SLASH_ACCUSED)
            .map(|_| *KeyPair::generate().public_key())
            .collect();
        for k in &keys {
            r.mark(*k);
            r.mark(*k);
        }
        assert!(!r.contains(&keys[0]));
        assert!(r.contains(&keys[1]));
        assert!(r.contains(&keys[MAX_REPORTED_SLASH_ACCUSED]));
        assert_eq!(r.order.len(), MAX_REPORTED_SLASH_ACCUSED);
        assert_eq!(r.keys.len(), MAX_REPORTED_SLASH_ACCUSED);
    }
}
