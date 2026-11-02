//! INC-I-293 U-2: local outbound sync request id -> the request sent. The id is
//! libp2p's local `OutboundRequestId`; it never crosses the wire.

use std::collections::BTreeMap;

use libp2p::request_response::OutboundRequestId;

use crate::protocols::SyncRequest;

/// Entries past this bound drop the oldest (ids grow), so the map cannot grow
/// without bound if a response or failure event is lost.
pub(super) const MAX_TRACKED_SYNC_REQUESTS: usize = 4096;

/// Filled on send, drained on the response or on the outbound failure.
pub(super) struct OutboundSyncRequests<K: Ord = OutboundRequestId> {
    sent: BTreeMap<K, SyncRequest>,
}

impl<K: Ord> Default for OutboundSyncRequests<K> {
    fn default() -> Self {
        Self {
            sent: BTreeMap::new(),
        }
    }
}

impl<K: Ord> OutboundSyncRequests<K> {
    pub(super) fn insert(&mut self, id: K, request: SyncRequest) {
        self.sent.insert(id, request);
        while self.sent.len() > MAX_TRACKED_SYNC_REQUESTS {
            self.sent.pop_first();
        }
    }

    /// The request `id` answered; `None` when unknown (dropped by the bound).
    pub(super) fn take(&mut self, id: &K) -> Option<SyncRequest> {
        self.sent.remove(id)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.sent.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crypto::Hash;

    fn by_height(height: u64) -> SyncRequest {
        SyncRequest::GetBlockByHeight { height }
    }

    #[test]
    fn take_returns_the_sent_request_once() {
        let mut m = OutboundSyncRequests::<u64>::default();
        let hash = Hash::from_bytes([7; 32]);
        m.insert(1, by_height(5));
        m.insert(2, SyncRequest::GetBlockByHash { hash });
        assert!(matches!(m.take(&2), Some(SyncRequest::GetBlockByHash { hash: h }) if h == hash));
        assert!(m.take(&2).is_none(), "a response drains its entry");
        assert!(matches!(
            m.take(&1),
            Some(SyncRequest::GetBlockByHeight { height: 5 })
        ));
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn the_bound_drops_the_oldest_entry() {
        let mut m = OutboundSyncRequests::<u64>::default();
        for id in 0..=MAX_TRACKED_SYNC_REQUESTS as u64 {
            m.insert(id, by_height(id));
        }
        assert_eq!(m.len(), MAX_TRACKED_SYNC_REQUESTS);
        assert!(
            m.take(&0).is_none(),
            "the oldest id is dropped past the bound"
        );
        assert!(m.take(&(MAX_TRACKED_SYNC_REQUESTS as u64)).is_some());
    }
}
