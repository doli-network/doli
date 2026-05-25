//! Pool UTXO contention detection for AMM transactions.
//!
//! When a Swap, AddLiquidity, or RemoveLiquidity transaction enters the mempool,
//! this module detects whether another pending transaction already references the
//! same Pool UTXO. A diagnostic warning is returned to the submitter so they know
//! their TX is likely to fail or be deferred.
//!
//! Design constraints:
//! - Pure read-only simulation: no state mutation outside the contention index
//! - Deviation-free: does not change which TXs the producer includes
//! - No MEV leak: diagnostic returned only to the submitter (no competing tx hashes)
//! - False-positive rate <= 0.1% (AC-12)

use crypto::Hash;
use serde::{Deserialize, Serialize};
use storage::Outpoint;

/// Contention information for a Pool UTXO.
///
/// Returned when a pending AMM transaction targets a Pool UTXO that other
/// pending transactions also reference. Does NOT include competing tx hashes
/// to avoid creating a MEV signal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentionInfo {
    /// Number of other pending transactions competing for this Pool UTXO.
    pub competing_count: usize,
    /// The contested Pool UTXO reference (tx_hash, output_index).
    pub pool_utxo_tx: Hash,
    pub pool_utxo_index: u32,
}

/// Diagnostic information returned alongside a mempool admission result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MempoolDiagnostic {
    /// Pool UTXO contention warning, if any.
    pub contention: Option<ContentionInfo>,
}

/// Result of successfully adding a transaction to the mempool.
#[derive(Clone, Debug)]
pub struct AddTransactionResult {
    /// Hash of the accepted transaction.
    pub tx_hash: Hash,
    /// Diagnostic information (contention warnings, etc.).
    pub diagnostic: MempoolDiagnostic,
}

/// Transaction types that participate in pool contention tracking.
pub(crate) fn is_pool_contention_type(tx_type: doli_core::TxType) -> bool {
    matches!(
        tx_type,
        doli_core::TxType::Swap
            | doli_core::TxType::AddLiquidity
            | doli_core::TxType::RemoveLiquidity
    )
}

/// Find the Pool UTXO input in a transaction by checking the UTXO set.
///
/// Returns the outpoint of the first input that references a Pool-type output.
/// For valid AMM transactions, there should be exactly one Pool input.
pub(crate) fn find_pool_input(
    tx: &doli_core::Transaction,
    utxo_set: &storage::UtxoSet,
) -> Option<Outpoint> {
    for input in &tx.inputs {
        let outpoint = Outpoint::new(input.prev_tx_hash, input.output_index);
        if let Some(entry) = utxo_set.get(&outpoint) {
            if entry.output.output_type == doli_core::OutputType::Pool {
                return Some(outpoint);
            }
        }
    }
    None
}
