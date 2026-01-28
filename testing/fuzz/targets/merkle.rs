//! Fuzz target for Merkle tree computation
//!
//! Tests that merkle root computation handles arbitrary transactions safely.

#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use doli_core::{Transaction, Output, block::compute_merkle_root};
use crypto::Hash;

#[derive(Debug, Arbitrary)]
struct FuzzTransaction {
    version: u32,
    tx_type: u8,
    num_outputs: u8,
    extra_data: Vec<u8>,
}

impl FuzzTransaction {
    fn to_transaction(&self) -> Transaction {
        let tx_type = match self.tx_type % 3 {
            0 => doli_core::transaction::TxType::Transfer,
            1 => doli_core::transaction::TxType::Registration,
            _ => doli_core::transaction::TxType::Exit,
        };

        let num_outputs = (self.num_outputs % 10) as usize; // Limit to reasonable number
        let outputs: Vec<Output> = (0..num_outputs)
            .map(|i| Output::normal(1000 + i as u64, Hash::ZERO))
            .collect();

        Transaction {
            version: self.version,
            tx_type,
            inputs: vec![],
            outputs,
            extra_data: self.extra_data.clone(),
        }
    }
}

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    transactions: Vec<FuzzTransaction>,
}

fuzz_target!(|data: FuzzInput| {
    // Convert to actual transactions
    let transactions: Vec<Transaction> = data.transactions
        .iter()
        .take(100) // Limit number of transactions
        .map(|ft| ft.to_transaction())
        .collect();

    // Compute merkle root - should not panic
    let root = compute_merkle_root(&transactions);

    // Root should be deterministic
    let root2 = compute_merkle_root(&transactions);
    assert_eq!(root, root2);

    // Non-empty transaction list should give non-zero root
    // (empty list gives a specific constant)
});
