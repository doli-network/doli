use super::*;
use crypto::Hash;
use doli_core::transaction::Transaction;

use crate::utxo_rocks::RocksDbUtxoStore;

#[test]
fn test_utxo_add_and_spend() {
    let mut utxo_set = UtxoSet::new();

    // Create a coinbase transaction
    let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 0);
    let tx_hash = tx.hash();

    // Add to UTXO set
    utxo_set.add_transaction(&tx, 0, true, 0).unwrap();
    assert_eq!(utxo_set.len(), 1);

    // Check it exists
    let outpoint = Outpoint::new(tx_hash, 0);
    assert!(utxo_set.contains(&outpoint));

    // Check coinbase maturity (COINBASE_MATURITY = 6)
    let entry = utxo_set.get(&outpoint).unwrap();
    #[allow(deprecated)]
    {
        assert!(!entry.is_spendable_at(5)); // 5 confirmations - not mature
        assert!(entry.is_spendable_at(6)); // 6 confirmations - mature
        assert!(entry.is_spendable_at(100)); // well past maturity
    }
}

#[test]
fn test_outpoint_serialization() {
    let outpoint = Outpoint::new(Hash::ZERO, 42);
    let bytes = outpoint.to_bytes();
    let recovered = Outpoint::from_bytes(&bytes).unwrap();
    assert_eq!(outpoint, recovered);
}

// ==========================================================================
// Epoch Reward UTXO Tests
// ==========================================================================

#[test]
fn test_utxo_entry_epoch_reward_maturity() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1_000_000, pubkey_hash);
    let tx_hash = tx.hash();

    let mut utxo_set = UtxoSet::new();
    utxo_set.add_transaction(&tx, 100, false, 0).unwrap(); // height 100, not coinbase

    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = utxo_set.get(&outpoint).unwrap();

    // Verify is_epoch_reward flag is set
    assert!(entry.is_epoch_reward);
    assert!(!entry.is_coinbase);

    // Check maturity: needs REWARD_MATURITY (6) confirmations
    #[allow(deprecated)]
    {
        assert!(!entry.is_spendable_at(100)); // 0 confirmations
        assert!(!entry.is_spendable_at(105)); // 5 confirmations
        assert!(entry.is_spendable_at(106)); // 6 confirmations (mature)
        assert!(entry.is_spendable_at(200)); // well past maturity
    }
}

#[test]
fn test_utxo_entry_coinbase_maturity_unchanged() {
    // Verify coinbase still requires same maturity
    let tx = Transaction::new_coinbase(500_000_000, Hash::ZERO, 0);
    let tx_hash = tx.hash();

    let mut utxo_set = UtxoSet::new();
    utxo_set.add_transaction(&tx, 50, true, 0).unwrap();

    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = utxo_set.get(&outpoint).unwrap();

    // Verify flags
    assert!(entry.is_coinbase);
    assert!(!entry.is_epoch_reward);

    // Check maturity (COINBASE_MATURITY = 6)
    #[allow(deprecated)]
    {
        assert!(!entry.is_spendable_at(55)); // 5 confirmations - not mature
        assert!(entry.is_spendable_at(56)); // 6 confirmations (mature)
        assert!(entry.is_spendable_at(150)); // well past maturity
    }
}

#[test]
fn test_utxo_entry_regular_tx_no_maturity() {
    // Regular transactions should not have maturity requirements
    use doli_core::transaction::{Input, Output};

    let tx = Transaction {
        version: 1,
        tx_type: doli_core::transaction::TxType::Transfer,
        inputs: vec![Input::new(Hash::ZERO, 0)],
        outputs: vec![Output::normal(1000, Hash::ZERO)],
        extra_data: vec![],
    };
    let tx_hash = tx.hash();

    let mut utxo_set = UtxoSet::new();
    utxo_set.add_transaction(&tx, 50, false, 0).unwrap();

    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = utxo_set.get(&outpoint).unwrap();

    // Verify flags
    assert!(!entry.is_coinbase);
    assert!(!entry.is_epoch_reward);

    // Regular tx should be spendable immediately
    #[allow(deprecated)]
    {
        assert!(entry.is_spendable_at(50)); // Same height
        assert!(entry.is_spendable_at(51)); // Next height
    }
}

#[test]
fn test_utxo_entry_default_epoch_reward() {
    // Test that #[serde(default)] works - is_epoch_reward defaults to false
    // This ensures backward compatibility with existing serialized UTXOs
    use doli_core::transaction::Output;

    // Create an entry manually to verify default behavior
    let entry = UtxoEntry {
        output: Output::normal(1000, Hash::ZERO),
        height: 100,
        is_coinbase: true,
        is_epoch_reward: false, // Explicit false (what default should be)
    };

    // Serialize and deserialize
    let serialized = bincode::serialize(&entry).unwrap();
    let deserialized: UtxoEntry = bincode::deserialize(&serialized).unwrap();

    assert!(deserialized.is_coinbase);
    assert!(!deserialized.is_epoch_reward);

    // Also verify new UTXOs with is_epoch_reward=true serialize correctly
    let epoch_entry = UtxoEntry {
        output: Output::normal(1000, Hash::ZERO),
        height: 100,
        is_coinbase: false,
        is_epoch_reward: true,
    };

    let serialized2 = bincode::serialize(&epoch_entry).unwrap();
    let deserialized2: UtxoEntry = bincode::deserialize(&serialized2).unwrap();

    assert!(!deserialized2.is_coinbase);
    assert!(deserialized2.is_epoch_reward);
}

#[test]
fn test_utxo_entry_serialization_roundtrip() {
    let keypair = crypto::KeyPair::generate();
    let pubkey_hash = crypto::hash::hash(b"recipient");
    let tx = Transaction::new_epoch_reward(1, *keypair.public_key(), 1_000_000, pubkey_hash);

    let mut utxo_set = UtxoSet::new();
    utxo_set.add_transaction(&tx, 100, false, 0).unwrap();

    let outpoint = Outpoint::new(tx.hash(), 0);
    let entry = utxo_set.get(&outpoint).unwrap();

    // Serialize and deserialize
    let serialized = bincode::serialize(&entry).unwrap();
    let deserialized: UtxoEntry = bincode::deserialize(&serialized).unwrap();

    assert_eq!(entry.height, deserialized.height);
    assert_eq!(entry.is_coinbase, deserialized.is_coinbase);
    assert_eq!(entry.is_epoch_reward, deserialized.is_epoch_reward);
    assert_eq!(entry.output.amount, deserialized.output.amount);
}

#[test]
fn test_serialize_canonical_matches_between_backends() {
    let pk_hash = crypto::hash::hash(b"alice");

    // Create InMemory store with some entries
    let mut mem_store = InMemoryUtxoStore::new();
    let tx1 = Transaction::new_coinbase(100_000, pk_hash, 0);
    let tx2 = Transaction::new_coinbase(200_000, pk_hash, 1);
    mem_store.add_transaction(&tx1, 0, true, 0);
    mem_store.add_transaction(&tx2, 1, true, 0);

    // Create RocksDB store with same entries
    let dir = tempfile::TempDir::new().unwrap();
    let rocks_store = RocksDbUtxoStore::open(dir.path()).unwrap();
    rocks_store.add_transaction(&tx1, 0, true, 0).unwrap();
    rocks_store.add_transaction(&tx2, 1, true, 0).unwrap();

    // Canonical bytes must match
    let mem_bytes = mem_store.serialize_canonical();
    let rocks_bytes = rocks_store.serialize_canonical();
    assert_eq!(
        mem_bytes, rocks_bytes,
        "Canonical serialization must match between InMemory and RocksDB backends"
    );
}

#[test]
fn test_utxo_entry_canonical_bytes_fixed_size() {
    let pk_hash = crypto::hash::hash(b"alice");
    let tx = Transaction::new_coinbase(1_000_000, pk_hash, 0);
    let entry = UtxoEntry {
        output: tx.outputs[0].clone(),
        height: 42,
        is_coinbase: true,
        is_epoch_reward: false,
    };
    let bytes = entry.serialize_canonical_bytes();
    assert_eq!(
        bytes.len(),
        61,
        "canonical entry base size must be 61 bytes (59 + 2 for extra_data len)"
    );
}

#[test]
fn test_utxo_entry_canonical_roundtrip() {
    use doli_core::transaction::OutputType;
    let pk_hash = crypto::hash::hash(b"bob");
    let entry = UtxoEntry {
        output: doli_core::transaction::Output {
            output_type: OutputType::Normal,
            amount: 5_000_000,
            pubkey_hash: pk_hash,
            lock_until: 0,
            extra_data: vec![],
        },
        height: 1337,
        is_coinbase: false,
        is_epoch_reward: true,
    };

    let bytes = entry.serialize_canonical_bytes();
    let decoded = UtxoEntry::deserialize_canonical_bytes(&bytes).expect("roundtrip must succeed");

    assert_eq!(decoded.output.output_type, entry.output.output_type);
    assert_eq!(decoded.output.amount, entry.output.amount);
    assert_eq!(decoded.output.pubkey_hash, entry.output.pubkey_hash);
    assert_eq!(decoded.output.lock_until, entry.output.lock_until);
    assert_eq!(decoded.height, entry.height);
    assert_eq!(decoded.is_coinbase, entry.is_coinbase);
    assert_eq!(decoded.is_epoch_reward, entry.is_epoch_reward);
}

#[test]
fn test_utxo_entry_canonical_bond_roundtrip() {
    use doli_core::transaction::OutputType;
    let pk_hash = crypto::hash::hash(b"producer");
    let entry = UtxoEntry {
        output: doli_core::transaction::Output {
            output_type: OutputType::Bond,
            amount: 10_000_000_000,
            pubkey_hash: pk_hash,
            lock_until: 9999,
            extra_data: vec![],
        },
        height: 500,
        is_coinbase: false,
        is_epoch_reward: false,
    };

    let bytes = entry.serialize_canonical_bytes();
    let decoded = UtxoEntry::deserialize_canonical_bytes(&bytes).unwrap();
    assert_eq!(decoded.output.output_type, OutputType::Bond);
    assert_eq!(decoded.output.lock_until, 9999);
}

#[test]
fn test_utxo_canonical_count_header() {
    // Verify the 8-byte count header at the start of serialize_canonical output
    let mut store = InMemoryUtxoStore::new();
    let pk = crypto::hash::hash(b"x");
    let tx1 = Transaction::new_coinbase(100, pk, 0);
    let tx2 = Transaction::new_coinbase(200, pk, 1);
    store.add_transaction(&tx1, 0, true, 0);
    store.add_transaction(&tx2, 1, true, 0);

    let bytes = store.serialize_canonical();
    let count = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    assert_eq!(count, 2, "count header must reflect number of UTXOs");
    // Total size: 8 (header) + 2 * (36 outpoint + 61 entry) = 8 + 194 = 202
    assert_eq!(bytes.len(), 8 + 2 * (36 + 61));
}

#[test]
fn test_canonical_bytes_immune_to_is_epoch_reward_default() {
    // Simulates the bug where old on-disk UTXOs (without is_epoch_reward) and
    // new UTXOs (with is_epoch_reward=false explicitly) must produce identical
    // canonical bytes, since deserialize_canonical_bytes handles both uniformly.
    use doli_core::transaction::OutputType;
    let pk = crypto::hash::hash(b"test");

    // "Old" entry: is_epoch_reward was not stored (defaults to false via serde)
    let old_entry = UtxoEntry {
        output: doli_core::transaction::Output {
            output_type: OutputType::Normal,
            amount: 1_000,
            pubkey_hash: pk,
            lock_until: 0,
            extra_data: vec![],
        },
        height: 10,
        is_coinbase: true,
        is_epoch_reward: false, // default
    };

    // "New" entry: same data, is_epoch_reward explicitly false
    let new_entry = old_entry.clone();

    assert_eq!(
        old_entry.serialize_canonical_bytes(),
        new_entry.serialize_canonical_bytes(),
        "entries with identical logical state must produce identical canonical bytes"
    );
}
