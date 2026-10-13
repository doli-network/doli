//! Zero-flow admission sequence and the canonical mined check (INC-I-237 M12.2):
//! the mined check runs after every cheaper refusal, holds no mempool lock, and
//! remembers its refusals in a bounded cache pinned to the carrying block.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crypto::Hash;
use doli_core::{BlockHeight, Transaction, TxType};
use storage::{BlockStore, MaintainerState, ProducerSet};
use tokio::sync::RwLock;

use crate::pool::{Mempool, MempoolError};
use crate::zero_flow_auth::snapshot_for;
use crate::zero_flow_policy::refuse;

/// Most txids the negative cache holds; the oldest is evicted first.
pub const MINED_CACHE_CAP: usize = 10_000;

/// Pinned `(height, block_hash)` by txid; `order` holds the same txids, oldest first.
#[derive(Debug, Default)]
struct Pins {
    by_txid: HashMap<Hash, (u64, Hash)>,
    order: VecDeque<Hash>,
}

/// The node's canonical mined check with its negative cache of txids refused as mined.
#[derive(Debug, Default)]
pub struct MinedCheck {
    pins: Mutex<Pins>,
    body_reads: AtomicU64,
    index_gets: AtomicU64,
}

impl MinedCheck {
    /// `[MPTX014]` iff the canonical chain carries `tx`.
    pub(crate) fn refuse_if_mined(
        &self,
        tx: &Transaction,
        blocks: &BlockStore,
    ) -> Result<(), MempoolError> {
        if self.is_mined(tx.hash(), blocks) {
            return refuse("MPTX014", tx, "is already mined in the canonical chain");
        }
        Ok(())
    }

    fn is_mined(&self, txid: Hash, blocks: &BlockStore) -> bool {
        let pinned = self.pins().by_txid.get(&txid).copied();
        if let Some((height, block)) = pinned {
            self.index_gets.fetch_add(1, Ordering::Relaxed);
            if blocks.get_hash_by_height(height).ok().flatten() == Some(block) {
                return true;
            }
            let mut pins = self.pins();
            if pins.by_txid.get(&txid) == Some(&(height, block)) {
                pins.by_txid.remove(&txid);
                pins.order.retain(|t| *t != txid);
            }
        }
        match self.carrier(&txid, blocks) {
            Some((height, block)) => {
                self.record(txid, height, block);
                true
            }
            None => false,
        }
    }

    /// Height and hash of the canonical block at the tx-index height, iff its body carries `txid`.
    fn carrier(&self, txid: &Hash, blocks: &BlockStore) -> Option<(u64, Hash)> {
        let height = blocks.get_tx_block_height(txid).ok().flatten()?;
        let block = blocks.get_hash_by_height(height).ok().flatten()?;
        self.body_reads.fetch_add(1, Ordering::Relaxed);
        let body = blocks.get_block(&block).ok().flatten()?;
        body.transactions
            .iter()
            .any(|t| t.hash() == *txid)
            .then_some((height, block))
    }

    /// Pin `txid` to `block` at `height`; past the cap the oldest pin goes.
    #[doc(hidden)]
    pub fn record(&self, txid: Hash, height: u64, block: Hash) {
        let mut pins = self.pins();
        if pins.by_txid.insert(txid, (height, block)).is_none() {
            pins.order.push_back(txid);
        }
        if pins.order.len() > MINED_CACHE_CAP {
            if let Some(oldest) = pins.order.pop_front() {
                pins.by_txid.remove(&oldest);
            }
        }
    }

    #[doc(hidden)]
    pub fn body_reads(&self) -> u64 {
        self.body_reads.load(Ordering::Relaxed)
    }

    #[doc(hidden)]
    pub fn index_gets(&self) -> u64 {
        self.index_gets.load(Ordering::Relaxed)
    }

    #[doc(hidden)]
    pub fn cached_len(&self) -> usize {
        self.pins().by_txid.len()
    }

    #[doc(hidden)]
    pub fn is_cached(&self, txid: &Hash) -> bool {
        self.pins().by_txid.contains_key(txid)
    }

    fn pins(&self) -> MutexGuard<'_, Pins> {
        self.pins.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Mempool {
    /// The node's one mined check, shared by every admission site.
    pub fn mined_check(&self) -> Arc<MinedCheck> {
        Arc::clone(&self.mined_check)
    }
}

/// Zero-flow admission for every site: cheaper refusals under the read lock, the
/// DelegateBond mined check under no mempool lock, then insertion under the write lock.
pub async fn admit_zero_flow(
    pool: &RwLock<Mempool>,
    tx: Transaction,
    current_height: BlockHeight,
    maintainer_state: Option<&Arc<RwLock<MaintainerState>>>,
    producers: Option<&Arc<RwLock<ProducerSet>>>,
    blocks: &BlockStore,
) -> Result<Hash, MempoolError> {
    let snapshot = snapshot_for(&tx, maintainer_state, producers).await;
    if tx.tx_type == TxType::DelegateBond {
        let check = {
            let guard = pool.read().await;
            guard.check_system_transaction(&tx, current_height, Some(&snapshot))?;
            guard.mined_check()
        };
        check.refuse_if_mined(&tx, blocks)?;
    }
    pool.write()
        .await
        .add_system_transaction(tx, current_height, Some(&snapshot))
}
