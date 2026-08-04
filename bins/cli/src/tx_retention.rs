//! Post-submit retention verification, shared by every CLI submit command.
//!
//! INV-CLI-002 (INC-I-148): a CLI submit command MUST NOT report success
//! (exit 0 + a "submitted successfully" line + an activation ETA) for a
//! transaction that was not retained by the node. A bare OK from
//! `sendTransaction` is NOT evidence of retention — the node can accept a
//! transaction into its mempool and drop it moments later (`Mempool::revalidate`
//! evicts a same-input duplicate as soon as the first copy mines).
//!
//! Reporting must distinguish three outcomes:
//!
//! * **accepted-and-retained** — the node still holds the tx, or has mined it.
//! * **rejected-at-submit** — `sendTransaction` itself returned an error; the
//!   caller handles that before ever getting here.
//! * **accepted-then-dropped** — `sendTransaction` said OK and the node no
//!   longer has the transaction.
//!
//! # Why the probe polls instead of asking once
//!
//! `bins/node/src/node/apply_block/mod.rs` removes a block's transactions from
//! the mempool at line 266 (`mempool.remove_for_block`) but does not write the
//! block — and therefore the tx index that `getTransaction` falls back on — until
//! line 305 (`block_store.put_block`). Between those two points a *perfectly
//! mined* transaction is in neither the mempool nor the block index, and
//! `getTransaction` answers `-32001 Transaction not found`. A single-shot probe
//! would report a healthy registration as DROPPED, pushing the operator toward
//! exactly the duplicate resubmission that caused INC-I-147.
//!
//! That window spans a handful of in-memory operations plus one RocksDB write,
//! so [`PROBE_ATTEMPTS`] probes spaced [`PROBE_INTERVAL`] apart cover it by
//! orders of magnitude. A retained transaction is recognised on the FIRST probe
//! (the node inserts it under a write lock before `sendTransaction` returns,
//! `crates/rpc/src/methods/transaction.rs:201-215`), so the poll adds latency
//! only on the path that is about to report a failure anyway.

use std::time::Duration;

use anyhow::{bail, Result};

use crate::rpc_client::{RpcClient, TxPresence};

/// Probes attempted before concluding that a transaction was dropped.
const PROBE_ATTEMPTS: u32 = 4;

/// Delay between probes. Total added latency on the dropped path is
/// `(PROBE_ATTEMPTS - 1) * PROBE_INTERVAL`; the retained path pays nothing.
const PROBE_INTERVAL: Duration = Duration::from_millis(400);

/// What the node says about a transaction after it was submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retention {
    /// The node still holds the transaction in its mempool, unmined.
    InMempool,
    /// The node has the transaction in a block.
    Mined {
        /// Confirmation depth reported by the node.
        confirmations: u64,
    },
    /// The node accepted the submission and no longer has the transaction.
    Dropped,
    /// The node could not be asked. NOT evidence in either direction.
    Unverified(String),
}

impl Retention {
    /// One-line operator-facing description of the retention state.
    pub fn describe(&self) -> String {
        match self {
            Retention::InMempool => "held in the node mempool, not yet mined".to_string(),
            Retention::Mined { confirmations } => {
                format!("mined ({} confirmation(s))", confirmations)
            }
            Retention::Dropped => "not retained by the node".to_string(),
            Retention::Unverified(reason) => format!("unverified ({})", reason),
        }
    }
}

/// Ask the node that received the submission whether it still holds `tx_hash`.
///
/// Never returns [`Retention::Dropped`] on the strength of a single answer, and
/// never turns a transport failure into a drop verdict — an unreachable node
/// yields [`Retention::Unverified`].
pub async fn probe_retention(rpc: &RpcClient, tx_hash: &str) -> Retention {
    let mut probe_error: Option<String> = None;

    for attempt in 0..PROBE_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(PROBE_INTERVAL).await;
        }
        match rpc.get_transaction_presence(tx_hash).await {
            Ok(TxPresence::Mempool) => return Retention::InMempool,
            Ok(TxPresence::Mined { confirmations }) => return Retention::Mined { confirmations },
            // Not visible YET, or genuinely gone. Only the full probe budget
            // can tell those apart — keep asking.
            Ok(TxPresence::Absent) => {}
            Err(e) => probe_error = Some(e.to_string()),
        }
    }

    // A single failed probe anywhere in the budget is enough to disqualify a
    // "dropped" verdict: we cannot claim the node lost a transaction we were
    // unable to ask it about.
    match probe_error {
        Some(reason) => Retention::Unverified(reason),
        None => Retention::Dropped,
    }
}

/// Verify a just-submitted transaction was retained, and turn every
/// non-retained outcome into an error so the caller cannot print success.
///
/// `subject` names the operation for the operator, e.g. `"Registration"`.
pub async fn require_retained(rpc: &RpcClient, tx_hash: &str, subject: &str) -> Result<Retention> {
    let retention = probe_retention(rpc, tx_hash).await;
    match retention {
        Retention::InMempool | Retention::Mined { .. } => Ok(retention),
        Retention::Dropped => bail!(
            "{subject} was NOT retained by the node.\n\
             The node accepted TX {tx_hash} at submit but no longer holds it — \
             getTransaction reports it in neither the mempool nor a block after {PROBE_ATTEMPTS} probes.\n\
             Nothing was changed on-chain. Confirm the transaction's fate before retrying: \
             a blind resubmission can create a duplicate."
        ),
        Retention::Unverified(reason) => bail!(
            "{subject} could NOT be verified.\n\
             The node accepted TX {tx_hash} at submit, but the follow-up getTransaction probe failed: {reason}\n\
             The transaction may or may not have been retained. Query TX {tx_hash} before retrying."
        ),
    }
}
