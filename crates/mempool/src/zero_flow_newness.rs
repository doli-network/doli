//! Maintainer-change newness on signed content (INC-I-237 M11, AUDIT-P1-509):
//! at most one resident change per (action, target), at most MAX_MAINTAINERS per action.

use std::collections::HashMap;

use crypto::{Hash, PublicKey};
use doli_core::maintainer::MAX_MAINTAINERS;

use crate::entry::MempoolEntry;
use crate::pool::MempoolError;

/// Signed content of a maintainer change: (is_add, target).
pub(crate) type ChangeKey = (bool, PublicKey);

/// Live maintainer-change residents by signed content; self-prunes against the pool entries.
#[derive(Debug, Default)]
pub(crate) struct ChangeIndex(HashMap<ChangeKey, Hash>);

impl ChangeIndex {
    /// The key to record once the tx is inserted, or the refusal.
    pub(crate) fn admit(
        &mut self,
        key: Option<ChangeKey>,
        live: &HashMap<Hash, MempoolEntry>,
    ) -> Result<Option<ChangeKey>, MempoolError> {
        let Some(key) = key else {
            return Ok(None);
        };
        self.0.retain(|_, txid| live.contains_key(txid));
        if self.0.contains_key(&key) {
            return Err(MempoolError::AlreadyExists);
        }
        if self.0.keys().filter(|(is_add, _)| *is_add == key.0).count() >= MAX_MAINTAINERS {
            return Err(MempoolError::InvalidTransaction(format!(
                "[MPTX015] maintainer-change residents for this action at cap {MAX_MAINTAINERS}"
            )));
        }
        Ok(Some(key))
    }

    pub(crate) fn record(&mut self, key: ChangeKey, txid: Hash) {
        self.0.insert(key, txid);
    }
}
