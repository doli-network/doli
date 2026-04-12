//! INC-I-028 regression test — body-gap recovery must not leak partial undo mutations
//!
//! ## The bug (pre-fix)
//!
//! When a snap-synced node restarts after applying some blocks, the startup
//! body-gap detection finds missing block bodies below the snap floor and enters
//! an undo loop to rewind. The loop removes created UTXOs for each height it
//! can undo — but at the snap anchor, `get_undo()` returns `None` (undo data
//! was never stored for pre-snap blocks). The old code did `break`, leaving the
//! partial mutations permanently committed to `utxo_store/` (RocksDB-backed,
//! writes persist immediately). This makes the reward pool short by
//! N × coinbase_reward, causing `ECON_EPOCH_OVERFLOW` at the next epoch boundary.
//!
//! ## The fix
//!
//! When undo data is missing, rebuild `utxo_store` from `state_db` (the
//! authoritative source) — the same self-heal pattern proven by INC-I-027.
//!
//! ## What this test verifies
//!
//! Simulates the exact N38 scenario: utxo_store has 5 entries (3 pre-snap +
//! 2 post-snap coinbases), undo data exists for the 2 post-snap blocks but NOT
//! for the snap anchor. Body gap exists below the snap floor.
//!
//! Expected: after `recover_body_gaps`, all 5 UTXOs are preserved.
//! BUG: the undo loop removes the 2 coinbase UTXOs and breaks → only 3 remain.
//! FIX: rebuild from state_db → all 5 restored.

use crypto::hash::hash as crypto_hash;
use crypto::PublicKey;
use doli_core::block::{Block, BlockBuilder};
use doli_core::consensus::ConsensusParams;
use doli_core::transaction::Output;
use doli_node::node::recover_body_gaps;
use storage::{BlockStore, ChainState, Outpoint, StateDb, UndoData, UtxoEntry, UtxoSet};
use tempfile::TempDir;

/// Build a deterministic test UTXO keyed by `tag`.
fn make_utxo(tag: &str, amount: u64) -> (Outpoint, UtxoEntry) {
    let pk_hash = crypto_hash(b"inc_i_028_owner");
    let outpoint = Outpoint::new(crypto_hash(tag.as_bytes()), 0);
    let entry = UtxoEntry {
        output: Output::normal(amount, pk_hash),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    (outpoint, entry)
}

/// Build a minimal block at the given height with deterministic hashing.
fn make_block(height: u64, prev_hash: crypto::Hash) -> Block {
    let pk_bytes = *crypto_hash(b"test_producer").as_bytes();
    let producer = PublicKey::from_bytes(pk_bytes);
    let params = ConsensusParams::mainnet();
    // Timestamp must be genesis_time + slot * slot_duration for valid slot derivation
    let timestamp = params.genesis_time + height * params.slot_duration;
    let builder = BlockBuilder::new(prev_hash, (height - 1) as u32, producer).with_params(params);
    let (header, txs) = builder.build(timestamp).expect("build test block");
    Block::new(header, txs)
}

#[test]
fn inc_i_028_body_gap_recovery_preserves_utxo_integrity() {
    // === Setup: simulate a snap-synced node that applied blocks h=6,7 then restarted ===
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path();

    let state_db = StateDb::open(&data_dir.join("state_db")).expect("open state_db");
    let block_store = BlockStore::open(&data_dir.join("blocks")).expect("open block_store");

    // 5 authoritative UTXOs (3 pre-snap + 2 post-snap coinbases)
    let pre_snap = vec![
        make_utxo("pre-1", 1000),
        make_utxo("pre-2", 2000),
        make_utxo("pre-3", 3000),
    ];
    let post_snap_coinbases = vec![
        make_utxo("coinbase-h6", 100_000_000),
        make_utxo("coinbase-h7", 100_000_000),
    ];

    // Insert all 5 into state_db (authoritative at h=7)
    for (o, e) in pre_snap.iter().chain(post_snap_coinbases.iter()) {
        state_db.insert_utxo(o, e);
    }
    assert_eq!(state_db.utxo_len(), 5);

    // Create utxo_store with same 5 UTXOs (matching state — steady state)
    let utxo_store_path = data_dir.join("utxo_store");
    let mut utxo = UtxoSet::open_rocksdb(&utxo_store_path).expect("open utxo_store");
    for (o, e) in pre_snap.iter().chain(post_snap_coinbases.iter()) {
        utxo.insert(o.clone(), e.clone()).expect("insert utxo");
    }
    assert_eq!(utxo.len(), 5, "utxo_store should start with 5 UTXOs");

    // Undo data: h=7 created coinbase-h7, h=6 created coinbase-h6.
    // NO undo for h=5 (snap anchor — never existed).
    state_db.put_undo(
        7,
        &UndoData {
            created_utxos: vec![post_snap_coinbases[1].0.clone()],
            spent_utxos: vec![],
            producer_snapshot: vec![],
        },
    );
    state_db.put_undo(
        6,
        &UndoData {
            created_utxos: vec![post_snap_coinbases[0].0.clone()],
            spent_utxos: vec![],
            producer_snapshot: vec![],
        },
    );

    // Block store: blocks exist for h=6,7 (post-snap) but NOT h=1-5 (body gap).
    let genesis_hash = crypto_hash(b"test_genesis");
    let block_5_hash = crypto_hash(b"block_5_phantom");
    let block_6 = make_block(6, block_5_hash);
    let block_7 = make_block(7, block_6.hash());
    block_store
        .put_block_canonical(&block_6, 6)
        .expect("store block 6");
    block_store
        .put_block_canonical(&block_7, 7)
        .expect("store block 7");

    // Chain state: tip is block 7
    let mut chain_state = ChainState::new(genesis_hash);
    chain_state.best_height = 7;
    chain_state.best_hash = block_7.hash();
    chain_state.best_slot = 6;

    // === Act: run body-gap recovery ===
    let result = recover_body_gaps(&mut chain_state, &block_store, &state_db, &mut utxo);
    assert!(
        result.is_ok(),
        "recover_body_gaps should not error: {:?}",
        result.err()
    );

    // === Assert: all 5 UTXOs must be preserved ===
    //
    // BUG path (pre-fix):
    //   Undo loop removes coinbase-h7, then coinbase-h6, then hits h=5 (no undo)
    //   and breaks. utxo_store now has only 3 entries (pre-snap). Pool is short 2×100M.
    //
    // FIXED path:
    //   At h=5 (no undo), rebuild utxo_store from state_db → 5 entries preserved.
    assert_eq!(
        utxo.len(),
        5,
        "INC-I-028: UTXO set should preserve all 5 entries after body-gap recovery \
         (got {} — partial undo leak detected!)",
        utxo.len()
    );

    // Verify every authoritative UTXO is present
    for (i, (outpoint, _)) in pre_snap.iter().enumerate() {
        assert!(
            utxo.contains(outpoint),
            "INC-I-028: missing pre-snap UTXO #{} after body-gap recovery",
            i
        );
    }
    for (i, (outpoint, _)) in post_snap_coinbases.iter().enumerate() {
        assert!(
            utxo.contains(outpoint),
            "INC-I-028: missing post-snap coinbase #{} after body-gap recovery \
             — partial undo leaked!",
            i
        );
    }
}
