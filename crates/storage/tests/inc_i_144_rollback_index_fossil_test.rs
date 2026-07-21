//! INC-I-144 — Block-store height-index rollback fossils (RUN_ID=469).
//!
//! STRICT-TDD reproduction. These tests FAIL on current code and MUST pass
//! once the rollback/reorg rewind path is made atomic with height-index
//! maintenance (diagnosis-report.md VERDICT conf 0.95; architecture-context §6).
//!
//! ROOT CAUSE (not the planned fix): the canonical by-height projection
//! (`height_index` / `hash_to_height`) is written ONLY on the APPLY path
//! (`set_canonical_chain`, the sole live index writer). Its healing walk runs
//! tip→down and STOPS at the first already-matching height (writes.rs:111-118),
//! so it heals only the range it walks and NEVER removes entries ABOVE the new
//! canonical tip. No rollback/reorg path deletes above-tip entries. Therefore a
//! reorg that moves the canonical tip DOWN to a shorter branch leaves the
//! rewound orphan heights as permanent "fossils": `get_block_by_height(h)` /
//! `get_hash_by_height(h)` keep serving the rolled-back orphan instead of None.
//! This is fail-silent-wrong (E1/E5, minimal-scenario steps 2-3).
//!
//! Both tests reproduce the mainnet seed2 mechanism with ONLY the two existing
//! live primitives (`put_block` + `set_canonical_chain`) — no node, fully
//! deterministic. The reorg is a real `set_canonical_chain` call to a lower
//! tip; today it genuinely runs and genuinely leaves the fossil.

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::{Block, BlockHeader};
use storage::BlockStore;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

/// Create a test BlockStore in a temporary directory.
fn create_test_store() -> (BlockStore, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let store = BlockStore::open(temp_dir.path()).unwrap();
    (store, temp_dir)
}

/// Build a header with a given slot, producer, and parent hash.
fn header(slot: u32, producer: &PublicKey, prev_hash: Hash) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 1000 + slot as u64 * 10,
        slot,
        producer: *producer,
        vdf_output: VdfOutput { value: Vec::new() },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

/// Build a block with a given slot and parent, no transactions.
fn block(slot: u32, producer: &PublicKey, prev_hash: Hash) -> Block {
    Block::new(header(slot, producer, prev_hash), vec![])
}

// ===========================================================================
// INC-I-144 — reorg-to-lower-tip leaves height-index fossils above the new tip
// ===========================================================================
//
// OUTPUT CONTRACT: fn reorg_to_lower_tip_purges_multi_height_index_fossils
//   Function under test (symptom surface): the canonical-index rewind contract
//   exercised via `set_canonical_chain(new_tip, new_height)` when a reorg moves
//   the canonical tip DOWN. Observed via the by-height read methods + reverse map.
//   O3: get_hash_by_height(h)  — must be None for every REWOUND height h > new_tip
//   O3: get_block_by_height(h) — must be None for every rewound height h > new_tip
//   O3: get_height_by_hash(orphan) — must be None for every rewound orphan hash
//                                    (orphan is not on the canonical chain)
//   O4: CF_HEIGHT_INDEX / CF_HASH_TO_HEIGHT — must NOT retain rewound-height→orphan
//   (O1 mutable params: none. O2 receiver: &self store — covered via O4 reads.
//    O5 globals: none. O6 channels: none.)
// PATHS:
//   P1: reorg to a strictly lower tip (orphan branch longer than new canonical tip)
//       — heights above the new tip must be unserved by the by-height index.
// INPUT PARTITIONS:
//   P1b: MULTIPLE rewound heights (orphan tip = new_tip + 2). Exercises whether the
//        cleanup purges the whole above-tip range, not just the boundary height.
// MATRIX: 3 read-outputs × 1 path × 1 partition, over 2 rewound heights (2,3)
//   P1b: O3(get_hash_by_height 2,3)✓ O3(get_block_by_height 2)✓
//        O3(get_height_by_hash O2,O3)✓  + positive canonical checks (0,1)

#[test]
fn reorg_to_lower_tip_purges_multi_height_index_fossils() {
    let (store, _dir) = create_test_store();
    let producer = *KeyPair::generate().public_key();

    // --- Canonical genesis at height 0 ---
    let a = block(1, &producer, Hash::ZERO);
    let hash_a = a.hash();
    store.put_block(&a, 0).unwrap();
    store.set_canonical_chain(hash_a, 0).unwrap();

    // --- Orphan (loser) branch applied to heights 1,2,3 ---
    let o1 = block(2, &producer, hash_a);
    let hash_o1 = o1.hash();
    let o2 = block(3, &producer, hash_o1);
    let hash_o2 = o2.hash();
    let o3 = block(4, &producer, hash_o2);
    let hash_o3 = o3.hash();
    store.put_block(&o1, 1).unwrap();
    store.put_block(&o2, 2).unwrap();
    store.put_block(&o3, 3).unwrap();
    store.set_canonical_chain(hash_o3, 3).unwrap();

    // Sanity: the orphan branch is the current by-height view.
    assert_eq!(store.get_hash_by_height(3).unwrap(), Some(hash_o3));
    assert_eq!(store.get_hash_by_height(2).unwrap(), Some(hash_o2));

    // --- Reorg / rollback: canonical winner is a DIFFERENT block at height 1.
    // The new canonical tip is height 1; heights 2 and 3 are rewound. ---
    let c1 = block(5, &producer, hash_a);
    let hash_c1 = c1.hash();
    store.put_block(&c1, 1).unwrap();
    store.set_canonical_chain(hash_c1, 1).unwrap();

    // Positive: the walked range is canonical.
    assert_eq!(
        store.get_hash_by_height(0).unwrap(),
        Some(hash_a),
        "genesis at height 0 must remain canonical"
    );
    assert_eq!(
        store.get_hash_by_height(1).unwrap(),
        Some(hash_c1),
        "height 1 must be healed to the reorg winner C1"
    );

    // CONTRACT (fails today): heights above the new tip must be unserved.
    // Today the by-height index still serves the rolled-back orphans O2/O3.
    assert_eq!(
        store.get_hash_by_height(2).unwrap(),
        None,
        "FOSSIL: height 2 above new tip must return None, not rewound orphan {:.16}",
        hash_o2
    );
    assert_eq!(
        store.get_hash_by_height(3).unwrap(),
        None,
        "FOSSIL: height 3 above new tip must return None, not rewound orphan {:.16}",
        hash_o3
    );
    assert!(
        store.get_block_by_height(2).unwrap().is_none(),
        "FOSSIL: get_block_by_height(2) must return None after rewind"
    );

    // Reverse map must not map rewound orphans as canonical.
    assert_eq!(
        store.get_height_by_hash(&hash_o2).unwrap(),
        None,
        "FOSSIL: hash_to_height must not map rewound orphan O2 → 2"
    );
    assert_eq!(
        store.get_height_by_hash(&hash_o3).unwrap(),
        None,
        "FOSSIL: hash_to_height must not map rewound orphan O3 → 3"
    );
}

// OUTPUT CONTRACT: fn reorg_to_lower_tip_purges_single_height_index_fossil
//   Same function under test and outputs as above.
// PATHS:
//   P1: reorg to a strictly lower tip.
// INPUT PARTITIONS:
//   P1a: SINGLE rewound height (orphan tip = new_tip + 1) — the degenerate
//        boundary case; a fix that only handles a multi-height range would still
//        need to purge exactly one entry here.
// MATRIX: 3 read-outputs × 1 path × 1 partition, over 1 rewound height (2)
//   P1a: O3(get_hash_by_height 2)✓ O3(get_block_by_height 2)✓
//        O3(get_height_by_hash O2)✓  + positive canonical checks (0,1)

#[test]
fn reorg_to_lower_tip_purges_single_height_index_fossil() {
    let (store, _dir) = create_test_store();
    let producer = *KeyPair::generate().public_key();

    // Canonical genesis at height 0.
    let a = block(1, &producer, Hash::ZERO);
    let hash_a = a.hash();
    store.put_block(&a, 0).unwrap();
    store.set_canonical_chain(hash_a, 0).unwrap();

    // Orphan branch to heights 1,2.
    let o1 = block(2, &producer, hash_a);
    let hash_o1 = o1.hash();
    let o2 = block(3, &producer, hash_o1);
    let hash_o2 = o2.hash();
    store.put_block(&o1, 1).unwrap();
    store.put_block(&o2, 2).unwrap();
    store.set_canonical_chain(hash_o2, 2).unwrap();
    assert_eq!(store.get_hash_by_height(2).unwrap(), Some(hash_o2));

    // Reorg: winner C1 at height 1 → new tip height 1, height 2 rewound.
    let c1 = block(5, &producer, hash_a);
    let hash_c1 = c1.hash();
    store.put_block(&c1, 1).unwrap();
    store.set_canonical_chain(hash_c1, 1).unwrap();

    // Positive.
    assert_eq!(store.get_hash_by_height(0).unwrap(), Some(hash_a));
    assert_eq!(store.get_hash_by_height(1).unwrap(), Some(hash_c1));

    // CONTRACT (fails today): the single rewound height must be unserved.
    assert_eq!(
        store.get_hash_by_height(2).unwrap(),
        None,
        "FOSSIL: height 2 above new tip must return None, not rewound orphan {:.16}",
        hash_o2
    );
    assert!(
        store.get_block_by_height(2).unwrap().is_none(),
        "FOSSIL: get_block_by_height(2) must return None after rewind"
    );
    assert_eq!(
        store.get_height_by_hash(&hash_o2).unwrap(),
        None,
        "FOSSIL: hash_to_height must not map rewound orphan O2 → 2"
    );
}

// ===========================================================================
// INC-I-144 — direct coverage of the deleter primitive `remove_canonical_entry`
// ===========================================================================
//
// OUTPUT CONTRACT: fn remove_canonical_entry_purges_matching_height
//   Function under test: BlockStore::remove_canonical_entry(height, expected_hash)
//   — the guarded index deleter wired into rollback_one_block / execute_reorg.
//   O3: get_hash_by_height(height)   — must become None once the guard fires
//   O3: get_height_by_hash(expected) — must become None (reverse map cleared)
//   O4: CF_HEIGHT_INDEX / CF_HASH_TO_HEIGHT — the height→hash and hash→height
//       entries for (height, expected_hash) must be removed in ONE WriteBatch
//   O3: get_hash_by_height(below)    — heights below the rewind must be untouched
//   (O1 mutable params: none. O2 receiver: &self store — observed via O4 reads.
//    O5 globals: none. O6 channels: none. Return: Ok(()) — non-fallible here.)
// PATHS:
//   P-fire: get_hash_by_height(height) == Some(expected_hash) → guard TRUE →
//           batch delete of both column-family entries executes.
// INPUT PARTITIONS:
//   Pf: the current index entry MATCHES expected_hash (the delete-firing case).
// MATRIX: 3 read-outputs × 1 path × 1 partition
//   Pf: O3(get_hash_by_height 1 → None)✓ O3(get_height_by_hash O1 → None)✓
//       O3(get_hash_by_height 0 → Some(A), untouched)✓

#[test]
fn remove_canonical_entry_purges_matching_height() {
    let (store, _dir) = create_test_store();
    let producer = *KeyPair::generate().public_key();

    // Canonical genesis at height 0.
    let a = block(1, &producer, Hash::ZERO);
    let hash_a = a.hash();
    store.put_block(&a, 0).unwrap();
    store.set_canonical_chain(hash_a, 0).unwrap();

    // Canonical block O1 at height 1.
    let o1 = block(2, &producer, hash_a);
    let hash_o1 = o1.hash();
    store.put_block(&o1, 1).unwrap();
    store.set_canonical_chain(hash_o1, 1).unwrap();
    assert_eq!(store.get_hash_by_height(1).unwrap(), Some(hash_o1));

    // Guard fires: current index[1] == expected_hash → both entries deleted.
    store.remove_canonical_entry(1, hash_o1).unwrap();

    // The by-height entry and its reverse mapping are gone.
    assert_eq!(
        store.get_hash_by_height(1).unwrap(),
        None,
        "guarded delete must clear height_index[1]"
    );
    assert_eq!(
        store.get_height_by_hash(&hash_o1).unwrap(),
        None,
        "guarded delete must clear hash_to_height[O1]"
    );
    // Below the rewind is untouched.
    assert_eq!(
        store.get_hash_by_height(0).unwrap(),
        Some(hash_a),
        "genesis at height 0 must remain canonical (below the rewind)"
    );
}

// OUTPUT CONTRACT: fn remove_canonical_entry_guards_against_clobber
//   Function under test: BlockStore::remove_canonical_entry(height, expected_hash)
//   with a STALE expected_hash after a newer branch overwrote the height.
//   O3: get_hash_by_height(height)   — must be UNCHANGED (still the newer hash)
//   O3: get_height_by_hash(newer)    — must be UNCHANGED (still maps to height)
//   O4: CF_HEIGHT_INDEX / CF_HASH_TO_HEIGHT — NO deletes issued (no-op batch)
//   (O1 mutable params: none. O2 receiver: &self store — observed via O4 reads.
//    O5 globals: none. O6 channels: none. Return: Ok(()) — no-op is idempotent.)
// PATHS:
//   P-skip: get_hash_by_height(height) == Some(NEWER) != expected_hash(OLD) →
//           guard FALSE → nothing deleted.
// INPUT PARTITIONS:
//   Ps: the current index entry does NOT match expected_hash (clobber-guard case).
// MATRIX: 2 read-outputs × 1 path × 1 partition
//   Ps: O3(get_hash_by_height 1 → Some(NEWER), unchanged)✓
//       O3(get_height_by_hash NEWER → Some(1), unchanged)✓

#[test]
fn remove_canonical_entry_guards_against_clobber() {
    let (store, _dir) = create_test_store();
    let producer = *KeyPair::generate().public_key();

    // Canonical genesis at height 0.
    let a = block(1, &producer, Hash::ZERO);
    let hash_a = a.hash();
    store.put_block(&a, 0).unwrap();
    store.set_canonical_chain(hash_a, 0).unwrap();

    // OLD occupies height 1 first...
    let old = block(2, &producer, hash_a);
    let hash_old = old.hash();
    store.put_block(&old, 1).unwrap();
    store.set_canonical_chain(hash_old, 1).unwrap();

    // ...then a DIFFERENT block NEWER clobbers height 1 (a newer branch won).
    let newer = block(7, &producer, hash_a);
    let hash_newer = newer.hash();
    assert_ne!(hash_old, hash_newer, "OLD and NEWER must differ");
    store.put_block(&newer, 1).unwrap();
    store.set_canonical_chain(hash_newer, 1).unwrap();
    assert_eq!(store.get_hash_by_height(1).unwrap(), Some(hash_newer));

    // Guard holds: expected_hash(OLD) != current(NEWER) → no-op.
    store.remove_canonical_entry(1, hash_old).unwrap();

    // NEWER's entry survives untouched — the stale delete did NOT clobber it.
    assert_eq!(
        store.get_hash_by_height(1).unwrap(),
        Some(hash_newer),
        "clobber guard must leave height_index[1] = NEWER untouched"
    );
    assert_eq!(
        store.get_height_by_hash(&hash_newer).unwrap(),
        Some(1),
        "clobber guard must leave hash_to_height[NEWER] = 1 untouched"
    );
}
