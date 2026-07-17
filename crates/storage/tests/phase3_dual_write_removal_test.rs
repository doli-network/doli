//! Phase 3 tests: verify BlockBatch overlay reads (pending + disk) produce
//! correct results after removing per-tx utxo_store writes.
//!
//! OUTPUT CONTRACT:
//!   Functions under test (BlockBatch overlay methods):
//!     1. BlockBatch::get_utxo(outpoint) -> Option<UtxoEntry>
//!     2. BlockBatch::contains_utxo(outpoint) -> bool
//!     3. BlockBatch::get_utxos_by_pubkey(pkh) -> Vec<(Outpoint, UtxoEntry)>
//!     4. BlockBatch::has_unique_id_check(prefix, id) -> bool
//!     5. BlockBatch::add_transaction_utxos (unique_id side effects)
//!     6. BlockBatch::spend_utxo (unique_id side effects)
//!     7. UtxoProvider impl for BlockBatch
//!
//!   Code paths:
//!     P1: get_utxo — pending hit
//!     P2: get_utxo — pending miss, disk hit
//!     P3: get_utxo — pending miss, disk miss (None)
//!     P4: get_utxo — spent in batch (None even if on disk)
//!     P5: contains_utxo — mirrors P1-P4 with bool
//!     P6: get_utxos_by_pubkey — merges pending + disk, excludes spent
//!     P7: has_unique_id_check — pending, then removed, then disk
//!     P8: add_transaction_utxos auto-inserts unique IDs
//!     P9: spend_utxo auto-removes unique IDs
//!     P10: UtxoProvider::get_utxo dispatches to overlay
//!
//!   INPUT PARTITIONS:
//!     I1: Single UTXO added then spent in same batch (same-block-spend)
//!     I2: Disk-only UTXO read via overlay (no pending)
//!     I3: Nonexistent UTXO (neither pending nor disk)
//!     I4: Disk UTXO spent in batch (should be invisible)
//!     I5: 2 disk UTXOs + 1 pending + 1 spent — pubkey scan merge
//!     I6: NFT output via add_transaction_utxos — unique ID auto-insert
//!     I7: Pool output via add_transaction_utxos — unique ID auto-insert
//!     I8: FungibleAsset output via add_transaction_utxos — unique ID auto-insert
//!     I9: NFT UTXO spend — unique ID auto-remove from pending
//!     I10: Pool UTXO spend from disk — unique ID auto-remove overrides disk
//!     I11: Same-block duplicate NFT detection (uniqueness gate)
//!     I12: Same-block duplicate Pool detection (uniqueness gate)
//!     I13: UtxoProvider pending hit
//!     I14: UtxoProvider disk hit
//!     I15: UtxoProvider spent-in-batch returns None
//!     I16: Disk unique ID removed in batch stays removed
//!
//!   Matrix (test -> path x partition):
//!     test_same_block_spend_via_batch:                     P1,P4,P5 x I1
//!     test_get_utxo_falls_through_to_disk:                 P2,P5 x I2
//!     test_get_utxo_returns_none_for_missing:              P3,P5 x I3
//!     test_get_utxo_returns_none_for_spent_in_batch:       P4,P5 x I4
//!     test_get_utxos_by_pubkey_merges_pending_and_disk:    P6 x I5
//!     test_add_transaction_utxos_inserts_nft_unique_id:    P8 x I6
//!     test_add_transaction_utxos_inserts_pool_unique_id:   P8 x I7
//!     test_add_transaction_utxos_inserts_asset_unique_id:  P8 x I8
//!     test_spend_utxo_removes_nft_unique_id:               P9 x I9
//!     test_spend_utxo_removes_pool_unique_id:              P9,P7 x I10
//!     test_same_block_nft_uniqueness:                      P7 x I11
//!     test_same_block_pool_uniqueness:                     P7 x I12
//!     test_blockbatch_utxo_provider_pending:               P10 x I13
//!     test_blockbatch_utxo_provider_disk:                  P10 x I14
//!     test_blockbatch_utxo_provider_spent_returns_none:    P10 x I15
//!     test_unique_id_removed_in_batch_despite_disk:        P7 x I16

use crypto::hash::hash as crypto_hash;
use crypto::Hash;
use doli_core::conditions::Condition;
use doli_core::transaction::{Output, Transaction, TxType};
use storage::state_db::StateDb;
use storage::utxo::{Outpoint, UtxoEntry, UID_PREFIX_ASSET, UID_PREFIX_NFT, UID_PREFIX_POOL};
use tempfile::TempDir;

fn make_state_db() -> (StateDb, TempDir) {
    let dir = TempDir::new().unwrap();
    let sdb = StateDb::open(dir.path()).unwrap();
    (sdb, dir)
}

fn make_normal_output(amount: u64, pkh: Hash) -> Output {
    Output::normal(amount, pkh)
}

fn make_utxo_entry(output: Output, height: u64) -> UtxoEntry {
    UtxoEntry {
        output,
        height,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

fn make_nft_output(pkh: Hash, token_id: Hash) -> Output {
    let content_hash = crypto_hash(b"content");
    let condition = Condition::Timelock(0);
    Output::nft(0, pkh, token_id, content_hash.as_bytes(), &condition)
        .expect("NFT output construction")
}

fn make_asset_output(pkh: Hash, asset_id: Hash) -> Output {
    let condition = Condition::Timelock(0);
    Output::fungible_asset(1000, pkh, asset_id, 1_000_000, "TEST", &condition)
        .expect("FungibleAsset output construction")
}

fn make_tx(tx_type: TxType, outputs: Vec<Output>) -> Transaction {
    Transaction {
        version: 1,
        tx_type,
        inputs: vec![],
        outputs,
        extra_data: vec![],
    }
}

// ============================================================================
// P1,P4,P5 x I1: same-block-spend via BlockBatch overlay
// ============================================================================

#[test]
fn test_same_block_spend_via_batch() {
    let (sdb, _dir) = make_state_db();
    let mut batch = sdb.begin_batch();

    let pkh = crypto_hash(b"alice");
    let tx_hash = crypto_hash(b"tx1");
    let outpoint = Outpoint::new(tx_hash, 0);

    let entry = make_utxo_entry(make_normal_output(1000, pkh), 100);
    batch.add_utxo(outpoint, entry.clone());

    assert!(batch.contains_utxo(&outpoint));
    let got = batch.get_utxo(&outpoint).expect("should find pending UTXO");
    assert_eq!(got.output.amount, 1000);

    let spent = batch
        .spend_utxo(&outpoint)
        .expect("should spend pending UTXO");
    assert_eq!(spent.output.amount, 1000);

    assert!(!batch.contains_utxo(&outpoint));
    assert!(batch.get_utxo(&outpoint).is_none());
}

// ============================================================================
// P2,P5 x I2: overlay reads fall through to disk
// ============================================================================

#[test]
fn test_get_utxo_falls_through_to_disk() {
    let (sdb, _dir) = make_state_db();

    let pkh = crypto_hash(b"bob");
    let tx_hash = crypto_hash(b"committed_tx");
    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = make_utxo_entry(make_normal_output(2000, pkh), 50);
    sdb.insert_utxo(&outpoint, &entry).unwrap();

    let batch = sdb.begin_batch();
    assert!(batch.contains_utxo(&outpoint));
    let got = batch.get_utxo(&outpoint).expect("should find disk UTXO");
    assert_eq!(got.output.amount, 2000);
}

// ============================================================================
// P3,P5 x I3: nonexistent UTXO
// ============================================================================

#[test]
fn test_get_utxo_returns_none_for_missing() {
    let (sdb, _dir) = make_state_db();
    let batch = sdb.begin_batch();

    let outpoint = Outpoint::new(crypto_hash(b"nonexistent"), 0);
    assert!(!batch.contains_utxo(&outpoint));
    assert!(batch.get_utxo(&outpoint).is_none());
}

// ============================================================================
// P4,P5 x I4: disk UTXO spent in batch
// ============================================================================

#[test]
fn test_get_utxo_returns_none_for_spent_in_batch() {
    let (sdb, _dir) = make_state_db();

    let pkh = crypto_hash(b"charlie");
    let tx_hash = crypto_hash(b"to_spend");
    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = make_utxo_entry(make_normal_output(3000, pkh), 60);
    sdb.insert_utxo(&outpoint, &entry).unwrap();

    let mut batch = sdb.begin_batch();
    batch.spend_utxo(&outpoint).expect("should spend");

    assert!(!batch.contains_utxo(&outpoint));
    assert!(batch.get_utxo(&outpoint).is_none());
}

// ============================================================================
// P6 x I5: pubkey scan with overlay
// ============================================================================

#[test]
fn test_get_utxos_by_pubkey_merges_pending_and_disk() {
    let (sdb, _dir) = make_state_db();

    let pkh = crypto_hash(b"dave");

    let op1 = Outpoint::new(crypto_hash(b"disk_tx1"), 0);
    let op2 = Outpoint::new(crypto_hash(b"disk_tx2"), 0);
    let e1 = make_utxo_entry(make_normal_output(100, pkh), 10);
    let e2 = make_utxo_entry(make_normal_output(200, pkh), 20);
    sdb.insert_utxo(&op1, &e1).unwrap();
    sdb.insert_utxo(&op2, &e2).unwrap();

    let mut batch = sdb.begin_batch();

    let op3 = Outpoint::new(crypto_hash(b"batch_tx"), 0);
    let e3 = make_utxo_entry(make_normal_output(300, pkh), 30);
    batch.add_utxo(op3, e3);

    batch.spend_utxo(&op1).expect("should spend");

    let results = batch.get_utxos_by_pubkey(&pkh);
    assert_eq!(results.len(), 2, "should have 2 UTXOs (1 disk + 1 pending)");

    let outpoints: Vec<Outpoint> = results.iter().map(|(o, _)| *o).collect();
    assert!(outpoints.contains(&op2));
    assert!(outpoints.contains(&op3));
    assert!(!outpoints.contains(&op1));
}

// ============================================================================
// P8 x I6,I7,I8: unique ID auto-insert via add_transaction_utxos
// ============================================================================

#[test]
fn test_add_transaction_utxos_inserts_nft_unique_id() {
    let (sdb, _dir) = make_state_db();
    let mut batch = sdb.begin_batch();

    let token_id = crypto_hash(b"nft_token_1");
    let nft_output = make_nft_output(crypto_hash(b"owner"), token_id);
    let tx = make_tx(TxType::Transfer, vec![nft_output]);

    batch.add_transaction_utxos(&tx, 100, false, 50);

    assert!(batch.has_unique_id_check(UID_PREFIX_NFT, &token_id));
}

#[test]
fn test_add_transaction_utxos_inserts_pool_unique_id() {
    let (sdb, _dir) = make_state_db();
    let mut batch = sdb.begin_batch();

    let pool_id = crypto_hash(b"pool_1");
    let asset_b = crypto_hash(b"asset_b");
    let pool_output = Output::pool(pool_id, asset_b, 1000, 500, 0, 0, 0, 30, 0);
    let tx = make_tx(TxType::CreatePool, vec![pool_output]);

    batch.add_transaction_utxos(&tx, 100, false, 50);

    assert!(batch.has_unique_id_check(UID_PREFIX_POOL, &pool_id));
}

#[test]
fn test_add_transaction_utxos_inserts_fungible_asset_unique_id() {
    let (sdb, _dir) = make_state_db();
    let mut batch = sdb.begin_batch();

    let asset_id = crypto_hash(b"asset_1");
    let asset_output = make_asset_output(crypto_hash(b"issuer"), asset_id);
    let tx = make_tx(TxType::Transfer, vec![asset_output]);

    batch.add_transaction_utxos(&tx, 100, false, 50);

    assert!(batch.has_unique_id_check(UID_PREFIX_ASSET, &asset_id));
}

// ============================================================================
// P9 x I9: NFT unique ID auto-removed on spend (from pending)
// ============================================================================

#[test]
fn test_spend_utxo_removes_nft_unique_id() {
    let (sdb, _dir) = make_state_db();
    let mut batch = sdb.begin_batch();

    let token_id = crypto_hash(b"nft_to_spend");
    let nft_output = make_nft_output(crypto_hash(b"owner"), token_id);
    let outpoint = Outpoint::new(crypto_hash(b"nft_tx"), 0);
    let entry = make_utxo_entry(nft_output, 100);

    batch.add_utxo(outpoint, entry);
    batch.add_pending_unique_id(UID_PREFIX_NFT, token_id);
    assert!(batch.has_unique_id_check(UID_PREFIX_NFT, &token_id));

    batch.spend_utxo(&outpoint).expect("should spend");

    assert!(!batch.has_unique_id_check(UID_PREFIX_NFT, &token_id));
}

// ============================================================================
// P9,P7 x I10: Pool unique ID auto-removed on spend (from disk)
// ============================================================================

#[test]
fn test_spend_utxo_removes_pool_unique_id() {
    let (sdb, _dir) = make_state_db();

    let pool_id = crypto_hash(b"pool_to_spend");
    let asset_b = crypto_hash(b"asset_b");
    let pool_output = Output::pool(pool_id, asset_b, 1000, 500, 100, 0, 10, 30, 5);
    let outpoint = Outpoint::new(crypto_hash(b"pool_tx"), 0);
    let entry = make_utxo_entry(pool_output, 100);

    sdb.insert_utxo(&outpoint, &entry).unwrap();
    sdb.add_unique_id(UID_PREFIX_POOL, &pool_id);

    let mut batch = sdb.begin_batch();
    assert!(batch.has_unique_id_check(UID_PREFIX_POOL, &pool_id));

    batch.spend_utxo(&outpoint).expect("should spend");

    assert!(!batch.has_unique_id_check(UID_PREFIX_POOL, &pool_id));
}

// ============================================================================
// P7 x I11,I12: same-block uniqueness rejection
// ============================================================================

#[test]
fn test_same_block_nft_uniqueness() {
    let (sdb, _dir) = make_state_db();
    let mut batch = sdb.begin_batch();

    let token_id = crypto_hash(b"unique_nft");
    let nft1 = make_nft_output(crypto_hash(b"owner1"), token_id);
    let tx1 = make_tx(TxType::Transfer, vec![nft1]);

    batch.add_transaction_utxos(&tx1, 100, false, 50);

    assert!(
        batch.has_unique_id_check(UID_PREFIX_NFT, &token_id),
        "duplicate NFT should be detected in same block"
    );
}

#[test]
fn test_same_block_pool_uniqueness() {
    let (sdb, _dir) = make_state_db();
    let mut batch = sdb.begin_batch();

    let pool_id = crypto_hash(b"unique_pool");
    let asset_b = crypto_hash(b"asset_b");
    let pool1 = Output::pool(pool_id, asset_b, 1000, 500, 0, 0, 0, 30, 0);
    let tx1 = make_tx(TxType::CreatePool, vec![pool1]);

    batch.add_transaction_utxos(&tx1, 100, false, 50);

    assert!(
        batch.has_unique_id_check(UID_PREFIX_POOL, &pool_id),
        "duplicate Pool should be detected in same block"
    );
}

// ============================================================================
// P10 x I13,I14,I15: UtxoProvider implementation on BlockBatch
// ============================================================================

#[test]
fn test_blockbatch_utxo_provider_pending() {
    use doli_core::validation::UtxoProvider;

    let (sdb, _dir) = make_state_db();
    let mut batch = sdb.begin_batch();

    let pkh = crypto_hash(b"provider_test");
    let tx_hash = crypto_hash(b"provider_tx");
    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = make_utxo_entry(make_normal_output(5000, pkh), 100);
    batch.add_utxo(outpoint, entry);

    let info = UtxoProvider::get_utxo(&batch, &tx_hash, 0).expect("should find via UtxoProvider");
    assert_eq!(info.output.amount, 5000);
    assert!(!info.spent);
}

#[test]
fn test_blockbatch_utxo_provider_disk() {
    use doli_core::validation::UtxoProvider;

    let (sdb, _dir) = make_state_db();

    let pkh = crypto_hash(b"provider_disk");
    let tx_hash = crypto_hash(b"disk_provider_tx");
    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = make_utxo_entry(make_normal_output(7000, pkh), 80);
    sdb.insert_utxo(&outpoint, &entry).unwrap();

    let batch = sdb.begin_batch();
    let info = UtxoProvider::get_utxo(&batch, &tx_hash, 0)
        .expect("should find disk UTXO via UtxoProvider");
    assert_eq!(info.output.amount, 7000);
}

#[test]
fn test_blockbatch_utxo_provider_spent_returns_none() {
    use doli_core::validation::UtxoProvider;

    let (sdb, _dir) = make_state_db();

    let pkh = crypto_hash(b"provider_spent");
    let tx_hash = crypto_hash(b"spent_provider_tx");
    let outpoint = Outpoint::new(tx_hash, 0);
    let entry = make_utxo_entry(make_normal_output(9000, pkh), 90);
    sdb.insert_utxo(&outpoint, &entry).unwrap();

    let mut batch = sdb.begin_batch();
    batch.spend_utxo(&outpoint).expect("should spend");

    assert!(
        UtxoProvider::get_utxo(&batch, &tx_hash, 0).is_none(),
        "spent UTXO should not be visible via UtxoProvider"
    );
}

// ============================================================================
// P7 x I16: unique ID removed in batch overrides disk
// ============================================================================

#[test]
fn test_unique_id_removed_in_batch_stays_removed_despite_disk() {
    let (sdb, _dir) = make_state_db();

    let token_id = crypto_hash(b"disk_nft");
    sdb.add_unique_id(UID_PREFIX_NFT, &token_id);

    let mut batch = sdb.begin_batch();
    assert!(batch.has_unique_id_check(UID_PREFIX_NFT, &token_id));

    batch.remove_pending_unique_id(UID_PREFIX_NFT, &token_id);

    assert!(
        !batch.has_unique_id_check(UID_PREFIX_NFT, &token_id),
        "removed unique ID should not be visible despite being on disk"
    );
}
