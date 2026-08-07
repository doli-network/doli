//! INC-I-156 / M2 QA-iteration-1 — F2 / OBS-003: the REQ-I156-007 dense-path oracle must
//! reconstruct a NON-EMPTY ProducerSet.
//!
//! covers: bins/node/src/node/rewards.rs (rebuild_producer_set_from_blocks — replay loop)
//!
//! ## Why this file exists (the trap QA measured)
//!
//! `inc_i_156_m2_rebuild_guard.rs` pins the guard hoist over a `CHAIN_LEN = 14` fixture. Devnet
//! `genesis_blocks = 40` (`crates/core/src/network_params/defaults.rs:450`), and the replay
//! loop SKIPS every `Registration` tx while `height <= genesis_blocks`
//! (`rewards.rs:1177-1179`) — the genesis-boundary registration that actually populates the
//! set runs only at `height == genesis_blocks + 1` (`rewards.rs:1381-1401`). A 14-block chain
//! therefore sits entirely inside the genesis phase, so EVERY dense rebuild in that suite
//! legitimately returns an EMPTY set: QA measured `live.count=3 / 742 canonical bytes` vs
//! `rebuilt.count=0 / 16 bytes`. Its "byte-identical pure-function rebuild" assertion was
//! consequently comparing empty to empty and never exercised the replay loop's producer
//! reconstruction at all.
//!
//! Two facts make the trap easy to re-set, so both are named here:
//!   * raising `CHAIN_LEN` past `genesis_blocks` is NOT sufficient on its own —
//!     `derive_genesis_producers_from_chain` (`genesis.rs:24-45`) scans blocks
//!     `1..=genesis_blocks` for `Registration` txs, and the devnet fallback is empty
//!     (`genesis.rs:65`), so a coinbase-only chain still crosses the boundary with nothing to
//!     register;
//!   * the fixture must therefore carry REAL genesis-phase `Registration` txs (VDF-proof
//!     containers, zero-bond — `validation/registration.rs:33-63`) AND run past
//!     `genesis_blocks + 1`.
//!
//! ## OUTPUT CONTRACT — `Node::rebuild_producer_set_from_blocks(&self, producers: &mut ProducerSet, target_height: u64) -> Result<()>`
//!
//! Same function as `inc_i_156_m2_rebuild_guard.rs`; this file covers the partitions that file
//! cannot reach. Outputs, restated:
//!   O1  `producers` — the `&mut ProducerSet`. Asserted as CONTENT: canonical bytes, count,
//!       sorted `(pubkey, bond_amount, bond_count)`, `pending_update_count()`.
//!   O2  receiver `self` — the one interior-mutability surface is
//!       `self.cached_genesis_producers` (`OnceLock`, `genesis.rs:14`). Unlike the sibling
//!       file's partitions this one DOES reach it (that is the point), so it is asserted for
//!       DETERMINISM — two rebuilds over the same range must agree, cached or not — rather
//!       than for unreachability.
//!   O3  return value `Result<()>`.
//!   O4  persistent store writes — NONE. Read back from `state_db.load_producer_set()`.
//!
//! INPUT PARTITIONS (of `target_height` relative to the genesis boundary — the axis the
//! sibling file collapses):
//!   K1  `target_height == genesis_blocks` — boundary NOT crossed. The rebuild is EMPTY, and
//!       that is CORRECT, not a bug. This is the partition the whole M2 suite was stuck in;
//!       it is asserted explicitly here so the emptiness is a measured property with a named
//!       cause instead of an accident.                                          [PASS-LOCK]
//!   K2  `target_height > genesis_blocks` — boundary crossed. The rebuild MUST be NON-EMPTY
//!       and must equal the on-chain genesis registrations, byte for byte.       [THE F2 LOCK]
//!   K3  = K2 over a HOLED store — the refusal must leave a caller set intact whose rebuild
//!       WOULD have been non-empty, so "intact" cannot be satisfied by empty-in/empty-out.
//!                                                                                 [PASS-LOCK]
//!
//! MATRIX (outputs x partitions):
//!   K1: O1 ✓  O2 -   O3 ✓  O4 -
//!   K2: O1 ✓  O2 ✓   O3 ✓  O4 ✓
//!   K3: O1 ✓  O2 -   O3 ✓  O4 ✓

mod inc_i_156_m1_harness;
use inc_i_156_m1_harness as h;

use crypto::{Hash, KeyPair};
use doli_core::transaction::{RegistrationData, Transaction, TxType};
use doli_node::node::Node;
use storage::ProducerSet;
use tempfile::TempDir;

// ==================== Scenario geometry ====================

/// Devnet `genesis_blocks` (`crates/core/src/network_params/defaults.rs:450`). Asserted
/// against the live config in the fixture — never trusted as a literal.
const GENESIS_BLOCKS: u64 = 40;

/// Genesis-phase heights carrying one `Registration` VDF-proof container each. Must be
/// `<= GENESIS_BLOCKS` so `derive_genesis_producers_from_chain` (`genesis.rs:24`) sees them.
const REG_HEIGHTS: [u64; 3] = [5, 6, 7];

/// Chain tip. Must exceed `GENESIS_BLOCKS + 1` so the rebuild's boundary crossing at
/// `height == genesis_blocks + 1` (`rewards.rs:1381`) falls INSIDE the replay range.
const CHAIN_LEN: u64 = 44;

/// The rebuild range for the K2/K3 partitions. `> GENESIS_BLOCKS` — this is the single
/// property that separates a meaningful oracle from the vacuous one F2 reported.
const TARGET_HEIGHT: u64 = CHAIN_LEN - 1;

/// A hole strictly inside `1..=TARGET_HEIGHT` and ABOVE the genesis boundary, so the K3
/// refusal is over a range whose successful rebuild would have been non-empty.
const HOLE_HEIGHT: u64 = 42;

// ==================== Observation surface ====================

/// Full CONTENT snapshot — REQ-I156-006 forbids count-only assertions, and
/// `serialize_canonical` alone is blind to `pending_updates`
/// (`producer/set_persistence.rs`), which `ProducerSet::clear()` also drops.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProducerContent {
    canonical: Vec<u8>,
    count: usize,
    bonds: Vec<(Vec<u8>, u64, u32)>,
    pending: usize,
}

impl ProducerContent {
    fn of(ps: &ProducerSet) -> Self {
        let mut bonds: Vec<(Vec<u8>, u64, u32)> = ps
            .all_producers()
            .iter()
            .map(|p| {
                (
                    p.public_key.as_bytes().to_vec(),
                    p.bond_amount,
                    p.bond_count,
                )
            })
            .collect();
        bonds.sort();
        Self {
            canonical: ps.serialize_canonical(),
            count: ps.total_count(),
            bonds,
            pending: ps.pending_update_count(),
        }
    }

    async fn live(node: &Node) -> Self {
        Self::of(&*node.producer_set.read().await)
    }

    fn persisted(node: &Node) -> Self {
        Self::of(&node.state_db.load_producer_set())
    }

    fn summary(&self) -> String {
        format!(
            "count={} pending={} canonical={}B",
            self.count,
            self.pending,
            self.canonical.len()
        )
    }
}

/// The canonical encoding of an EMPTY set — the value the whole M2 suite was unknowingly
/// comparing against on both sides.
fn empty_canonical() -> Vec<u8> {
    ProducerSet::new().serialize_canonical()
}

// ==================== Fixture ====================

/// A genesis-phase `Registration` transaction: a VDF-proof container with NO inputs and NO
/// outputs (`validation/transaction.rs:40-87` exempts registrations from both requirements,
/// and `validation/registration.rs:33-63` takes the genesis early-return once
/// `Network::is_in_genesis(height)` holds — bond, registration chain and fee are all handled
/// at GENESIS PHASE COMPLETE instead).
///
/// `vdf_output` MUST be exactly 32 bytes: `derive_genesis_producers_from_chain` filters on
/// that length (`genesis.rs:32`), so a shorter one silently yields an empty genesis set and
/// re-creates the very trap this file exists to close. The BLS PoP is real
/// (`validate_bls_pop`, `validation/registration.rs:259-275` verifies it).
fn genesis_registration_tx(producer: &KeyPair) -> Transaction {
    let bls = crypto::BlsKeyPair::generate();
    let pop = bls
        .proof_of_possession()
        .expect("fixture: BLS proof-of-possession must be producible");
    let reg = RegistrationData {
        public_key: *producer.public_key(),
        epoch: 0,
        vdf_output: vec![0u8; 32],
        vdf_proof: Vec::new(),
        prev_registration_hash: Hash::ZERO,
        sequence_number: 0,
        bond_count: 1,
        bls_pubkey: bls.public_key().as_bytes().to_vec(),
        bls_pop: pop.as_bytes().to_vec(),
    };
    Transaction {
        version: 1,
        tx_type: TxType::Registration,
        inputs: Vec::new(),
        outputs: Vec::new(),
        extra_data: bincode::serialize(&reg).expect("fixture: RegistrationData must serialize"),
    }
}

/// A dense chain that CROSSES the genesis boundary and carries real on-chain genesis
/// registrations — the shape the sibling M2 fixture cannot produce.
async fn build_node_crossing_genesis() -> (Node, Vec<KeyPair>, TempDir) {
    let (mut node, producers, temp) = h::make_node(3).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;

    // Every height constant here is derived from the LIVE config value, never from the
    // literal — the literal is only pinned so a config change surfaces here instead of
    // silently collapsing this file back into the vacuous genesis-phase partition.
    let live_genesis_blocks = node.config.network.genesis_blocks();
    assert_eq!(
        live_genesis_blocks, GENESIS_BLOCKS,
        "fixture: the GENESIS_BLOCKS literal must match the live devnet config"
    );
    assert!(
        TARGET_HEIGHT > live_genesis_blocks,
        "fixture: TARGET_HEIGHT ({TARGET_HEIGHT}) must exceed genesis_blocks \
         ({live_genesis_blocks}); this is the entire point of the file"
    );

    for (i, reg_height) in REG_HEIGHTS.iter().enumerate() {
        assert!(
            *reg_height <= live_genesis_blocks,
            "fixture: registration at h={reg_height} must be inside the genesis phase"
        );
        h::apply_plain_up_to(&mut node, &producers, reg_height - 1, &params).await;
        h::apply_block_with_transfer(
            &mut node,
            &producers,
            *reg_height,
            &params,
            genesis_registration_tx(&producers[i]),
        )
        .await;
    }
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    assert_eq!(
        node.chain_state.read().await.best_height,
        CHAIN_LEN,
        "fixture: the chain must reach CHAIN_LEN"
    );
    node.block_store
        .ensure_blocks_present(1, TARGET_HEIGHT)
        .expect("fixture: the block store must be DENSE over the rebuild range");

    (node, producers, temp)
}

fn punch_hole(node: &Node, height: u64) {
    let hash = node
        .block_store
        .get_hash_by_height(height)
        .expect("block_store get_hash_by_height failed")
        .unwrap_or_else(|| panic!("setup: expected a canonical entry at h={height}"));
    node.block_store
        .remove_canonical_entry(height, hash)
        .expect("remove_canonical_entry failed");
}

// ==========================================================================
//  K1 — PASS-LOCK: below the boundary the rebuild is EMPTY, and that is correct.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — names the cause of the emptiness F2 measured.
///
/// This is the partition `inc_i_156_m2_rebuild_guard.rs` is permanently in (`CHAIN_LEN = 14`).
/// Asserting it explicitly turns "the oracle happened to compare empty to empty" into a
/// measured, attributed property, so the trap cannot be re-set silently: if someone later
/// lowers `genesis_blocks`, or removes the `height <= genesis_blocks` skip at
/// `rewards.rs:1177-1179`, THIS test fails and points at the reason.
#[tokio::test]
async fn inc_i156_f2_rebuild_below_genesis_boundary_is_empty_by_design() {
    let (node, _keys, _tmp) = build_node_crossing_genesis().await;

    let mut scratch = ProducerSet::new();
    node.rebuild_producer_set_from_blocks(&mut scratch, GENESIS_BLOCKS)
        .expect("O3: a dense range must return Ok(())");

    let after = ProducerContent::of(&scratch);
    assert_eq!(
        after.canonical,
        empty_canonical(),
        "F2 / O1: a rebuild that stops AT the genesis boundary must be EMPTY. The replay loop \
         skips every Registration tx while `height <= genesis_blocks` (rewards.rs:1177-1179) \
         and the boundary registration runs only at `height == genesis_blocks + 1` \
         (rewards.rs:1381). This is the sole reason the CHAIN_LEN=14 fixture in \
         inc_i_156_m2_rebuild_guard.rs rebuilds nothing — recorded here so the cause is \
         measured, not inferred. Got {}",
        after.summary()
    );
    assert_eq!(
        (after.count, after.pending),
        (0, 0),
        "F2 / O1: the below-boundary rebuild must have no producers and no pending updates"
    );
}

// ==========================================================================
//  K2 — THE F2 LOCK: past the boundary the rebuild is NON-EMPTY and byte-compared.
// ==========================================================================

/// Requirement: REQ-I156-007 (Must) — the "byte-identical pure-function rebuild" half of the
/// contract, over a result that is genuinely NON-EMPTY.
///
/// This is the assertion F2 reported as vacuous in the sibling file. Here the replay loop
/// really does reconstruct producers: `derive_genesis_producers_from_chain` (`genesis.rs:24`)
/// picks the three on-chain registrations out of blocks `1..=genesis_blocks`, the crossing at
/// `rewards.rs:1381-1401` registers them, and `apply_pending_updates_with_cap` /
/// `process_unbonding` (`rewards.rs:1404-1422`) run over the remainder of the range.
///
/// The oracle has three independent parts, none of which an empty result can satisfy:
///   (a) the rebuilt set is NON-EMPTY and differs from the empty encoding;
///   (b) its CONTENT is exactly the three registered pubkeys, each with `bond_amount ==
///       bond_unit` and `bond_count == 1` — observed against the keys the fixture put on
///       chain, not against a reimplementation of the registration logic;
///   (c) rebuilding into the LIVE set and into a FRESH one yields BYTE-IDENTICAL canonical
///       encodings — the purity statement, now over 3 producers instead of 0.
#[tokio::test]
async fn inc_i156_f2_rebuild_past_genesis_boundary_reconstructs_non_empty_set() {
    let (node, keys, _tmp) = build_node_crossing_genesis().await;
    let before = ProducerContent::live(&node).await;
    let bond_unit = node.config.network.bond_unit();

    // (c1) into the LIVE, non-empty set.
    let from_live = {
        let mut producers = node.producer_set.write().await;
        node.rebuild_producer_set_from_blocks(&mut producers, TARGET_HEIGHT)
            .expect("O3: a DENSE range must return Ok(()) — the hoisted guard must not refuse");
        ProducerContent::of(&producers)
    };

    // (a) NON-EMPTY — the property F2 says the sibling suite never established.
    assert_ne!(
        from_live.canonical,
        empty_canonical(),
        "F2 / O1: the rebuilt ProducerSet must be NON-EMPTY past the genesis boundary. If it \
         is empty, the replay loop's producer reconstruction was never exercised and every \
         'byte-identical rebuild' assertion in the M2 suite is comparing empty to empty (the \
         exact defect QA measured: live.count=3/742B vs rebuilt.count=0/16B). Got {}",
        from_live.summary()
    );
    assert_eq!(
        from_live.count,
        REG_HEIGHTS.len(),
        "F2 / O1: the rebuild must reconstruct exactly the {} producers the fixture registered \
         on chain. Got {}",
        REG_HEIGHTS.len(),
        from_live.summary()
    );

    // (b) CONTENT — the exact pubkeys and bonds, observed against the fixture's own keys.
    let mut expected_bonds: Vec<(Vec<u8>, u64, u32)> = keys
        .iter()
        .map(|kp| (kp.public_key().as_bytes().to_vec(), bond_unit, 1u32))
        .collect();
    expected_bonds.sort();
    assert_eq!(
        from_live.bonds, expected_bonds,
        "F2 / O1: the rebuilt set's (pubkey, bond_amount, bond_count) triples must be exactly \
         the genesis producers the fixture registered on chain, each holding one bond_unit \
         (rewards.rs:1386-1400 registers them via ProducerInfo::new_with_bonds(pubkey, 0, \
         bond_unit, (genesis_bond_hash, 0), era, 1))"
    );

    // (c2) into a FRESH set — same blocks, same range, different starting content.
    let from_empty = {
        let mut scratch = ProducerSet::new();
        node.rebuild_producer_set_from_blocks(&mut scratch, TARGET_HEIGHT)
            .expect("O3: the same dense range must return Ok(()) for any input");
        ProducerContent::of(&scratch)
    };

    // (c3) BYTE comparison — now over a non-empty value, so it actually constrains the loop.
    assert_eq!(
        from_live.canonical, from_empty.canonical,
        "F2 / O1 + O2: `rebuild_producer_set_from_blocks` must be a PURE FUNCTION of \
         `1..=target_height`. Both sides now carry 3 reconstructed producers, so this compares \
         real canonical bytes rather than two empty encodings. O2 is covered too: the second \
         call reads `cached_genesis_producers` (genesis.rs:14) where the first initialised it, \
         and the results must be identical either way."
    );
    assert_eq!(
        (from_live.count, from_live.pending),
        (from_empty.count, from_empty.pending),
        "F2 / O1: producer count and pending_updates length must also be input-independent \
         (`pending_updates` is outside `serialize_canonical`)"
    );

    // O4 — a completed rebuild still persists nothing by itself.
    assert_eq!(
        ProducerContent::persisted(&node).canonical,
        before.canonical,
        "O4: `rebuild_producer_set_from_blocks` only READS block_store and must never write \
         the ProducerSet to state_db"
    );
}

// ==========================================================================
//  K3 — PASS-LOCK: the refusal contract, over a range whose rebuild is non-empty.
// ==========================================================================

/// Requirement: REQ-I156-006 (Must) — "refusal leaves the ProducerSet byte-for-byte
/// unchanged", re-verified on the partition where the rebuild has something to reconstruct.
///
/// On the sibling file's fixture, "the set is unchanged" and "the rebuild produces nothing"
/// were indistinguishable in one direction: an implementation that cleared the set and
/// returned `Err` on a range whose correct output is EMPTY would still be caught (the live
/// set is non-empty), but the pairing was never exercised where a SUCCESSFUL rebuild would
/// have replaced the content with different non-empty content. This test closes that.
#[tokio::test]
async fn inc_i156_f2_refusal_past_genesis_boundary_leaves_producer_set_intact() {
    let (node, _keys, _tmp) = build_node_crossing_genesis().await;

    // Ground truth: over the intact store this range rebuilds to a NON-EMPTY set.
    let would_have_been = {
        let mut scratch = ProducerSet::new();
        node.rebuild_producer_set_from_blocks(&mut scratch, TARGET_HEIGHT)
            .expect("precondition: the intact range must rebuild cleanly");
        ProducerContent::of(&scratch)
    };
    assert!(
        would_have_been.count > 0,
        "precondition: the successful rebuild of this range must be NON-EMPTY, otherwise this \
         test degrades into the empty-in/empty-out shape F2 reported. Got {}",
        would_have_been.summary()
    );

    punch_hole(&node, HOLE_HEIGHT);
    let before = ProducerContent::live(&node).await;
    assert!(
        before.count > 0,
        "precondition: the live set must be non-empty before the refusal"
    );

    let err = {
        let mut producers = node.producer_set.write().await;
        node.rebuild_producer_set_from_blocks(&mut producers, TARGET_HEIGHT)
            .expect_err("O3: a holed range must be REFUSED by the hoisted guard at rewards.rs:1147")
    };
    assert!(
        err.to_string().contains(&format!("height {HOLE_HEIGHT}")),
        "O3: the refusal must name the FIRST missing height ({HOLE_HEIGHT}); got {err}"
    );

    let after = ProducerContent::live(&node).await;
    assert_eq!(
        after.canonical, before.canonical,
        "REQ-I156-006 / O1: a refused rebuild must leave the caller's ProducerSet \
         byte-identical — asserted here on a range whose SUCCESSFUL output would have been \
         different non-empty content, so 'unchanged' cannot be satisfied by an empty rebuild"
    );
    assert_eq!(
        (after.count, after.pending),
        (before.count, before.pending),
        "REQ-I156-006 / O1: count and pending_updates length must be unchanged too"
    );
    assert_eq!(
        ProducerContent::persisted(&node).canonical,
        before.canonical,
        "O4: the refusal must not have written the ProducerSet to state_db"
    );
}
