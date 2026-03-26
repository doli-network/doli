use super::*;
use doli_core::transaction::{Input, Output, Transaction};
use storage::{Outpoint, UtxoEntry, UtxoSet};

/// Create a testnet mempool with min_fee_rate=0 for easier testing
fn test_mempool() -> Mempool {
    Mempool::testnet()
}

/// Create a funding UTXO and return (utxo_set, tx_hash, pubkey_hash)
fn funded_utxo(amount: u64) -> (UtxoSet, Hash, Hash) {
    let mut utxo_set = UtxoSet::new();
    let pubkey_hash = crypto::hash::hash(b"test_address");
    let tx_hash = crypto::hash::hash(b"funding_tx");
    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = UtxoEntry {
        output: Output::normal(amount, pubkey_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    utxo_set.insert(outpoint, entry).unwrap();
    (utxo_set, tx_hash, pubkey_hash)
}

/// Create a simple transfer spending from a funded UTXO
fn simple_transfer(prev_hash: Hash, amount: u64, fee: u64, dest: Hash) -> Transaction {
    Transaction::new_transfer(
        vec![Input::new(prev_hash, 0)],
        vec![Output::normal(amount - fee, dest)],
    )
}

#[test]
fn test_mempool_creation() {
    let mempool = Mempool::mainnet();
    assert!(mempool.is_empty());
    assert_eq!(mempool.len(), 0);
    assert_eq!(mempool.size(), 0);
}

#[test]
fn test_add_valid_transaction() {
    let mut mempool = test_mempool();
    let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
    let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

    let result = mempool.add_transaction(tx, &utxo_set, 100);
    assert!(result.is_ok());
    assert_eq!(mempool.len(), 1);
    assert!(!mempool.is_empty());
}

#[test]
fn test_add_duplicate_rejected() {
    let mut mempool = test_mempool();
    let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
    let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

    mempool.add_transaction(tx.clone(), &utxo_set, 100).unwrap();
    let result = mempool.add_transaction(tx, &utxo_set, 100);
    assert!(matches!(result, Err(MempoolError::AlreadyExists)));
}

#[test]
fn test_missing_input_rejected() {
    let mut mempool = test_mempool();
    let utxo_set = UtxoSet::new(); // Empty UTXO set
    let fake_hash = crypto::hash::hash(b"nonexistent");
    let dest = crypto::hash::hash(b"dest");
    let tx = simple_transfer(fake_hash, 5_000, 500, dest);

    let result = mempool.add_transaction(tx, &utxo_set, 100);
    assert!(matches!(result, Err(MempoolError::MissingInput(_, _))));
}

#[test]
fn test_insufficient_funds_rejected() {
    let mut mempool = test_mempool();
    let (utxo_set, tx_hash, _) = funded_utxo(1_000);
    let dest = crypto::hash::hash(b"dest");
    // Try to send more than available
    let tx = Transaction::new_transfer(
        vec![Input::new(tx_hash, 0)],
        vec![Output::normal(2_000, dest)],
    );

    let result = mempool.add_transaction(tx, &utxo_set, 100);
    assert!(matches!(result, Err(MempoolError::InvalidTransaction(_))));
}

#[test]
fn test_double_spend_rejected() {
    let mut mempool = test_mempool();
    let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);

    // First tx spends the UTXO
    let tx1 = Transaction::new_transfer(
        vec![Input::new(tx_hash, 0)],
        vec![Output::normal(9_000, pubkey_hash)],
    );
    mempool.add_transaction(tx1, &utxo_set, 100).unwrap();

    // Second tx tries to spend the same UTXO
    let dest2 = crypto::hash::hash(b"other_dest");
    let tx2 = Transaction::new_transfer(
        vec![Input::new(tx_hash, 0)],
        vec![Output::normal(8_000, dest2)],
    );
    let result = mempool.add_transaction(tx2, &utxo_set, 100);
    assert!(matches!(result, Err(MempoolError::DoubleSpend)));
}

#[test]
fn test_remove_transaction() {
    let mut mempool = test_mempool();
    let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
    let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

    let hash = mempool.add_transaction(tx, &utxo_set, 100).unwrap();
    assert_eq!(mempool.len(), 1);

    let removed = mempool.remove_transaction(&hash);
    assert!(removed.is_some());
    assert_eq!(mempool.len(), 0);
    assert!(mempool.is_empty());
}

#[test]
fn test_remove_for_block() {
    let mut mempool = test_mempool();
    let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
    let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

    mempool.add_transaction(tx.clone(), &utxo_set, 100).unwrap();
    assert_eq!(mempool.len(), 1);

    mempool.remove_for_block(&[tx]);
    assert_eq!(mempool.len(), 0);
}

#[test]
fn test_select_for_block_respects_size() {
    let mut mempool = test_mempool();

    // Create two funded UTXOs
    let mut utxo_set = UtxoSet::new();
    let pubkey_hash = crypto::hash::hash(b"addr");
    for i in 0..2u8 {
        let tx_hash = crypto::hash::hash(&[i, 1, 2, 3]);
        let outpoint = Outpoint::new(tx_hash, 0);
        utxo_set
            .insert(
                outpoint,
                UtxoEntry {
                    output: Output::normal(50_000, pubkey_hash),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
            .unwrap();
        let tx = simple_transfer(tx_hash, 50_000, 5_000, pubkey_hash);
        mempool.add_transaction(tx, &utxo_set, 100).unwrap();
    }

    assert_eq!(mempool.len(), 2);

    // Select with tiny size limit — should get at most 1
    let selected = mempool.select_for_block(1);
    assert!(selected.len() <= 1);

    // Select with large limit — should get both
    let selected = mempool.select_for_block(1_000_000);
    assert_eq!(selected.len(), 2);
}

#[test]
fn test_cpfp_effective_fee_rate() {
    let mut mempool = test_mempool();
    let pubkey_hash = crypto::hash::hash(b"addr");

    // Create parent UTXO (confirmed)
    let mut utxo_set = UtxoSet::new();
    let parent_funding = crypto::hash::hash(b"parent_fund");
    utxo_set
        .insert(
            Outpoint::new(parent_funding, 0),
            UtxoEntry {
                output: Output::normal(100_000, pubkey_hash),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .unwrap();

    // Parent tx: low fee (fee = 100)
    let parent_tx = Transaction::new_transfer(
        vec![Input::new(parent_funding, 0)],
        vec![Output::normal(99_900, pubkey_hash)],
    );
    let parent_hash = parent_tx.hash();
    mempool.add_transaction(parent_tx, &utxo_set, 100).unwrap();

    // Child tx: high fee (fee = 50_000)
    let child_tx = Transaction::new_transfer(
        vec![Input::new(parent_hash, 0)],
        vec![Output::normal(49_900, pubkey_hash)],
    );
    let child_hash = child_tx.hash();
    mempool.add_transaction(child_tx, &utxo_set, 100).unwrap();

    // Verify CPFP: child's effective_fee_rate includes ancestor package
    let child_entry = mempool.get(&child_hash).unwrap();
    assert!(child_entry.ancestor_fee > child_entry.fee);
    assert!(child_entry.ancestor_size > child_entry.size);
    // effective_fee_rate should be the package rate
    assert!(child_entry.effective_fee_rate() > 0);
}

#[test]
fn test_get_balance_tracking() {
    let mut mempool = test_mempool();
    let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);

    // Spend UTXO in mempool, send to different address
    let other = crypto::hash::hash(b"other");
    let tx = Transaction::new_transfer(
        vec![Input::new(tx_hash, 0)],
        vec![Output::normal(9_000, other)],
    );
    mempool.add_transaction(tx, &utxo_set, 100).unwrap();

    // Original address should show outgoing
    let (incoming, outgoing) = mempool.calculate_unconfirmed_balance(&pubkey_hash, &utxo_set);
    assert_eq!(outgoing, 10_000); // Spent UTXO

    // Destination address should show incoming
    let (incoming_other, _) = mempool.calculate_unconfirmed_balance(&other, &utxo_set);
    assert_eq!(incoming_other, 9_000);
    assert_eq!(incoming, 0);
}

#[test]
fn test_revalidate_removes_stale() {
    let mut mempool = test_mempool();
    let (utxo_set, tx_hash, pubkey_hash) = funded_utxo(10_000);
    let tx = simple_transfer(tx_hash, 10_000, 1_000, pubkey_hash);

    mempool.add_transaction(tx, &utxo_set, 100).unwrap();
    assert_eq!(mempool.len(), 1);

    // Revalidate against an empty UTXO set (simulates reorg removing the UTXO)
    let empty_utxo = UtxoSet::new();
    mempool.revalidate(&empty_utxo, 200);
    assert_eq!(mempool.len(), 0);
}
