//! Zero-flow newness on signed content (INC-I-237 M11 AUDIT-P1-509, M12.1 AUDIT-P1-515):
//! at most one resident change per (action, target), at most MAX_MAINTAINERS per action,
//! and at most one resident DelegateBond / RevokeDelegation per delegator.

use std::collections::HashMap;

use crypto::{Hash, PublicKey};
use doli_core::maintainer::MAX_MAINTAINERS;

use crate::entry::MempoolEntry;
use crate::pool::MempoolError;

/// Newness key: a maintainer change's (is_add, target), or a delegation tx's delegator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ChangeKey {
    Maintainer(bool, PublicKey),
    Delegator(PublicKey),
}

/// Live zero-flow residents by newness key; self-prunes against the pool entries.
#[derive(Debug, Default)]
pub(crate) struct ChangeIndex(HashMap<ChangeKey, Hash>);

impl ChangeIndex {
    /// The key to record once the tx is inserted, or the refusal.
    pub(crate) fn admit(
        &mut self,
        key: Option<ChangeKey>,
        live: &HashMap<Hash, MempoolEntry>,
    ) -> Result<Option<ChangeKey>, MempoolError> {
        if let Some(ChangeKey::Maintainer(..)) = key {
            self.0.retain(|_, txid| live.contains_key(txid));
        }
        self.probe(key, live)?;
        Ok(key)
    }

    /// The refusal `admit` would return, without pruning.
    pub(crate) fn probe(
        &self,
        key: Option<ChangeKey>,
        live: &HashMap<Hash, MempoolEntry>,
    ) -> Result<(), MempoolError> {
        let Some(key) = key else {
            return Ok(());
        };
        let resident = |k: &ChangeKey| self.0.get(k).is_some_and(|txid| live.contains_key(txid));
        if resident(&key) {
            return Err(MempoolError::AlreadyExists);
        }
        let ChangeKey::Maintainer(is_add, _) = key else {
            return Ok(());
        };
        let same_action = |k: &&ChangeKey| {
            matches!(k, ChangeKey::Maintainer(a, _) if *a == is_add) && resident(k)
        };
        if self.0.keys().filter(same_action).count() >= MAX_MAINTAINERS {
            return Err(MempoolError::InvalidTransaction(format!(
                "[MPTX015] maintainer-change residents for this action at cap {MAX_MAINTAINERS}"
            )));
        }
        Ok(())
    }

    pub(crate) fn record(&mut self, key: ChangeKey, txid: Hash) {
        self.0.insert(key, txid);
    }
}
