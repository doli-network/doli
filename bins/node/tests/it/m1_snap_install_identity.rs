//! UTXO-scalability M1 [F1] — snap-install identity + rejection lock.
//!
//! This file must PASS before and after the install refactor. It is the contract
//! the single-decode rewrite may not break.
//!
//! REQ-SCALE-001 — Decision: a failure reveals the installed 3-state no longer
//! hashes to the root the quorum agreed on, i.e. a snap-synced node forks.
//! REQ-SCALE-002 — Decision: a failure reveals a snap-synced node's utxoCount or
//! utxoHash diverged from the set the snapshot bytes carried.
//! REQ-SCALE-006 — Decision: a failure reveals INV-SYNC-007 (cross-backend byte
//! identity) or INV-SYNC-014 (backend swap after install) regressed.
//! REQ-SCALE-014 — Decision: a failure here alongside a lower peak reveals the RAM
//! reduction was bought by installing less state, not by streaming it.
//!
//! OUTPUT CONTRACT: fn Node::apply_snap_snapshot(&mut self, VerifiedSnapshot) -> Result<()>
//!   Outputs: O1 self.chain_state | O2 self.utxo_set content | O2b self.utxo_set backend
//!            O3 self.producer_set | O4 self.cached_state_root | O5 state_db durable
//!            O8 return value
//!   Paths:   P5 success | P3 root mismatch (reject) | P4 envelope mismatch (reject)
//!   MATRIX:
//!            P5 x O1  -> m1_installed_state_root_is_true_of_the_installed_backend
//!            P5 x O2  -> m1_installed_utxo_count_and_hash_match_the_snapshot_bytes
//!            P5 x O2b -> m1_installed_utxo_set_is_state_db_backed
//!            P5 x O3  -> m1_installed_state_root_is_true_of_the_installed_backend
//!            P5 x O4  -> m1_cached_state_root_equals_snapshot_state_root
//!            P5 x O5  -> m1_installed_utxo_set_is_state_db_backed (read routes to state_db)
//!            P5 x O8  -> every test (Ok(()) required)
//!            P3 x O1,O2,O4,O5,O8 -> m1_root_mismatch_snapshot_is_rejected_and_installs_nothing
//!            P4 x O1,O2,O4,O5,O8 -> m1_envelope_mismatch_snapshot_is_rejected_and_installs_nothing
//!   P1 (recovery_mode) and P2 (undecodable bytes) are out of this milestone's scope.
//! INPUT PARTITIONS: IP1 a valid snapshot over a non-trivial chain; IP2 the same
//!   snapshot with a corrupted state_root; IP3 the same snapshot with a shifted envelope.

use crate::inc_i_156_m1_harness as h;
use crypto::Hash;
use doli_node::node::Node;
use network::VerifiedSnapshot;
use std::sync::atomic::Ordering;
use tempfile::TempDir;

const CHAIN_LEN: u64 = 6;
const N_PRODUCERS: usize = 3;
/// UTXOs added on top of the chain's own state, so the set is non-trivial.
const EXTRA_UTXOS: usize = 64;

struct Fixture {
    node: Node,
    snapshot: VerifiedSnapshot,
    _temp: TempDir,
}

fn canonical_hash(bytes: &[u8]) -> Hash {
    crypto::hash::hash(bytes)
}

fn extra_utxos(n: usize) -> Vec<(storage::Outpoint, storage::UtxoEntry)> {
    (0..n)
        .map(|i| {
            let v = i as u64;
            let mut tx_hash = [0u8; 32];
            tx_hash[0..8].copy_from_slice(&v.to_le_bytes());
            tx_hash[8..16].copy_from_slice(&(v.wrapping_mul(0x9E37_79B9_7F4A_7C15)).to_le_bytes());
            let mut pkh = [0u8; 32];
            pkh[0..8].copy_from_slice(&(v ^ 0x5555_5555_5555_5555).to_le_bytes());
            (
                storage::Outpoint::new(Hash::from_bytes(tx_hash), (i % 4) as u32),
                storage::UtxoEntry {
                    output: doli_core::transaction::Output::normal(
                        7_000 + v,
                        Hash::from_bytes(pkh),
                    ),
                    height: 1,
                    is_coinbase: false,
                    is_epoch_reward: false,
                },
            )
        })
        .collect()
}

/// A node on the production RocksDb backend plus a valid snapshot whose UTXO set is
/// the node's own state plus `EXTRA_UTXOS` synthetic entries.
async fn fixture() -> Fixture {
    let (mut node, producers, _temp) = h::make_node(N_PRODUCERS).await;
    let params = node.params.clone();
    h::install_production_utxo_backend(&node).await;
    h::apply_plain_up_to(&mut node, &producers, CHAIN_LEN, &params).await;

    assert!(
        !node.recovery_mode.load(Ordering::Relaxed),
        "fixture: recovery_mode must be false or the install path is never reached"
    );

    let snapshot = {
        let cs = node.chain_state.read().await;
        let utxo = node.utxo_set.read().await;
        let ps = node.producer_set.read().await;
        let base = storage::StateSnapshot::create(&cs, &utxo, &ps)
            .expect("fixture: StateSnapshot::create must succeed");

        let mut widened = storage::UtxoSet::deserialize_canonical(&base.utxo_set_bytes)
            .expect("fixture: the node's own canonical bytes must decode");
        for (op, entry) in extra_utxos(EXTRA_UTXOS) {
            widened
                .insert(op, entry)
                .expect("fixture: synthetic UTXO insert must succeed");
        }
        let utxo_bytes = widened.serialize_canonical();
        drop(widened);

        let root = storage::compute_state_root_from_bytes(
            &base.chain_state_bytes,
            &utxo_bytes,
            &base.producer_set_bytes,
        )
        .expect("fixture: root over the snapshot bytes must compute");

        VerifiedSnapshot {
            block_hash: base.block_hash,
            block_height: base.block_height,
            chain_state: base.chain_state_bytes,
            utxo_set: utxo_bytes,
            utxo_staged: None,
            producer_set: base.producer_set_bytes,
            state_root: root,
            block_header_bytes: None,
            epoch_bond_snapshot_bytes: None,
            epoch_accumulators_bytes: None,
            epoch_state_bytes: Some(node.epoch_state.serialize()),
        }
    };

    assert_eq!(
        snapshot.block_height, CHAIN_LEN,
        "fixture: the snapshot must be taken at the tip"
    );

    Fixture {
        node,
        snapshot,
        _temp,
    }
}

/// (utxo_count, canonical utxo hash, best_hash, best_height) read through the node.
async fn observe(node: &Node) -> (u64, Hash, Hash, u64) {
    let utxo = node.utxo_set.read().await;
    let cs = node.chain_state.read().await;
    (
        utxo.utxo_count(),
        canonical_hash(&utxo.serialize_canonical()),
        cs.best_hash,
        cs.best_height,
    )
}

// ==================== P5 — success ====================

/// REQ-SCALE-002 / REQ-SCALE-006 — Decision: a failure reveals the installed set is
/// not the set the snapshot bytes carried (INV-SYNC-007 cross-backend byte identity).
#[tokio::test]
async fn m1_installed_utxo_count_and_hash_match_the_snapshot_bytes() {
    let Fixture {
        mut node,
        snapshot,
        _temp,
    } = fixture().await;

    let wire = storage::UtxoSet::deserialize_canonical(&snapshot.utxo_set)
        .expect("independent decode of the snapshot bytes must succeed");
    let wire_count = wire.utxo_count();
    let wire_hash = canonical_hash(&wire.serialize_canonical());
    drop(wire);
    assert!(
        wire_count > EXTRA_UTXOS as u64,
        "non-vacuity: the snapshot must carry the chain's own UTXOs plus the {} synthetic \
         ones (got {})",
        EXTRA_UTXOS,
        wire_count
    );

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("apply_snap_snapshot must succeed on a valid snapshot");

    let utxo = node.utxo_set.read().await;
    assert_eq!(
        utxo.utxo_count(),
        wire_count,
        "installed utxo_count must equal the count the snapshot bytes contained"
    );
    assert_eq!(
        canonical_hash(&utxo.serialize_canonical()),
        wire_hash,
        "installed canonical UTXO hash must be bit-identical to the InMemory decode of \
         the same bytes (INV-SYNC-007)"
    );
}

/// REQ-SCALE-001 — Decision: a failure reveals the cached root a snap-synced node
/// serves diverged from the quorum-agreed root.
#[tokio::test]
async fn m1_cached_state_root_equals_snapshot_state_root() {
    let Fixture {
        mut node,
        snapshot,
        _temp,
    } = fixture().await;
    let expected = snapshot.state_root;

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("apply_snap_snapshot must succeed on a valid snapshot");

    let cached = *node.cached_state_root.read().await;
    let (root, hash, height) = cached.expect("the install must cache a state root");
    assert_eq!(
        root, expected,
        "cached state root must equal the snapshot's state root"
    );
    let cs = node.chain_state.read().await;
    assert_eq!(
        hash, cs.best_hash,
        "cached root must be bound to the new tip"
    );
    assert_eq!(
        height, cs.best_height,
        "cached root must be bound to the new height"
    );
}

/// REQ-SCALE-001 / REQ-SCALE-006 — Decision: a failure reveals the root is true only
/// of the wire bytes and not of the durable state actually installed (filter F-10).
#[tokio::test]
async fn m1_installed_state_root_is_true_of_the_installed_backend() {
    let Fixture {
        mut node,
        snapshot,
        _temp,
    } = fixture().await;
    let expected = snapshot.state_root;

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("apply_snap_snapshot must succeed on a valid snapshot");

    let cs = node.chain_state.read().await;
    let utxo = node.utxo_set.read().await;
    let ps = node.producer_set.read().await;
    let recomputed = storage::compute_state_root(&cs, &utxo, &ps)
        .expect("root over the installed 3-state must compute");
    assert_eq!(
        recomputed, expected,
        "the root recomputed from the INSTALLED chain_state/utxo_set/producer_set must \
         equal the snapshot root — a wire-bytes hash proves transport, not storage"
    );
}

/// REQ-SCALE-006 — Decision: a failure reveals INV-SYNC-014 regressed and the node is
/// left on a frozen InMemory set that post-snap apply_block writes never reach.
#[tokio::test]
async fn m1_installed_utxo_set_is_state_db_backed() {
    let Fixture {
        mut node,
        snapshot,
        _temp,
    } = fixture().await;

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("apply_snap_snapshot must succeed on a valid snapshot");

    assert!(
        node.utxo_set.read().await.is_rocksdb(),
        "after install self.utxo_set must be the state_db-backed variant (INV-SYNC-014)"
    );

    // Behavioural confirmation: a write made directly to state_db is visible through
    // self.utxo_set, which is only true if the read routes to state_db.
    let probe_pkh = Hash::from_bytes([0xA7; 32]);
    let probe_op = storage::Outpoint::new(Hash::from_bytes([0xB3; 32]), 0);
    let probe_entry = storage::UtxoEntry {
        output: doli_core::transaction::Output::normal(4_242, probe_pkh),
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    };
    {
        let cs = node.chain_state.read().await;
        let ps = node.producer_set.read().await;
        let utxo = node.utxo_set.read().await;
        let mut pairs = utxo.iter_all();
        pairs.push((probe_op, probe_entry));
        node.state_db
            .atomic_replace(&cs, &ps, pairs.into_iter())
            .expect("probe write to state_db must succeed");
    }

    let seen: u64 = node
        .utxo_set
        .read()
        .await
        .get_by_pubkey_hash(&probe_pkh)
        .iter()
        .map(|(_, e)| e.output.amount)
        .sum();
    assert_eq!(
        seen, 4_242,
        "a state_db write made underneath must be visible through self.utxo_set — \
         an InMemory set would report 0"
    );
}

// ==================== P3 / P4 — rejection ====================

/// REQ-SCALE-001 — Decision: a failure reveals the refactor widened the admission
/// gate so a snapshot whose bytes do not hash to its root can be installed.
#[tokio::test]
async fn m1_root_mismatch_snapshot_is_rejected_and_installs_nothing() {
    let Fixture {
        mut node,
        mut snapshot,
        _temp,
    } = fixture().await;

    let before = observe(&node).await;
    assert!(
        before.0 > 0,
        "non-vacuity: the node must already hold state, or 'unchanged' proves nothing"
    );
    snapshot.state_root = Hash::from_bytes([0xEE; 32]);

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("a rejected snapshot must not propagate an error");

    let after = observe(&node).await;
    assert_eq!(
        after, before,
        "a root-mismatched snapshot must install nothing: utxo_count, utxo hash, \
         best_hash and best_height must all be unchanged"
    );
}

/// REQ-SCALE-001 — Decision: a failure reveals the C3 envelope defence was lost, so a
/// peer can bind a valid state to a block hash/height it does not belong to.
#[tokio::test]
async fn m1_envelope_mismatch_snapshot_is_rejected_and_installs_nothing() {
    let Fixture {
        mut node,
        mut snapshot,
        _temp,
    } = fixture().await;

    let before = observe(&node).await;
    assert!(
        before.0 > 0,
        "non-vacuity: the node must already hold state, or 'unchanged' proves nothing"
    );
    // The root is computed over the component BYTES, so shifting the envelope alone
    // leaves the root check passing and isolates the C3 check.
    snapshot.block_height += 1;

    node.apply_snap_snapshot(snapshot)
        .await
        .expect("a rejected snapshot must not propagate an error");

    let after = observe(&node).await;
    assert_eq!(
        after, before,
        "an envelope-mismatched snapshot must install nothing: utxo_count, utxo hash, \
         best_hash and best_height must all be unchanged"
    );
}
