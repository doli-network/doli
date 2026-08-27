//! AUDIT-P3-005 — Node-level integration tests for the Phase 2.1 oracle.
//!
//! The audit observed that no integration test exercised the full
//! gossip → mempool → block → epoch-boundary aggregator path. The
//! existing unit tests in `crates/core/src/oracle/tests.rs`,
//! `crates/core/src/validation/tests_oracle.rs`, and
//! `crates/rpc/src/methods/tests_oracle*.rs` cover the pure functions
//! and per-method validation rules; this file adds **node-level**
//! coverage for the cross-component wiring that the audit specifically
//! flagged:
//!
//!   - AUDIT-P1-001: the mempool's
//!     `active_producers_weighted` snapshot must be populated after
//!     every successful block apply (otherwise
//!     `PriceAttestation` admission rejects every signer at oracle
//!     activation).
//!   - AUDIT-P1-003: the mempool ROUTING classification of
//!     `PriceAttestation`. INC-I-173 M3 / F4 replaced the type-based
//!     `Transaction::is_state_only()` with the shape-based
//!     `is_zero_flow()`, and `PriceAttestation` is NOT in
//!     `TxType::allows_empty_io`, so it no longer takes the 0-fee
//!     system lane. See the updated test below for the full delta.
//!
//! The AUDIT-P0-001 rollback test, AUDIT-P1-002 missing-block test,
//! and AUDIT-P2-004 dedup test require an end-to-end driver that
//! pins `oracle_activation_height` to a runtime value, builds blocks
//! with PriceAttestations signed by the test producers, and crosses
//! an epoch boundary. The minimum wiring for that driver (~200 LOC of
//! block-builder shim) is filed as `oracle_e2e_driver` follow-up;
//! once it exists, those three tests append to this file.

use doli_core::transaction::{PriceAttestationData, Transaction, TxType};
use doli_node::node::Node;
use tempfile::TempDir;

async fn make_node(n_producers: usize) -> (Node, TempDir) {
    let temp = TempDir::new().unwrap();
    let producers = (0..n_producers)
        .map(|_| crypto::KeyPair::generate())
        .collect();
    let node = Node::new_for_test(temp.path().to_path_buf(), producers)
        .await
        .expect("Node::new_for_test failed");
    (node, temp)
}

// OUTPUT CONTRACT: refresh_mempool_producer_snapshot wires Node ProducerSet
//                  through to mempool's active_producers_weighted snapshot
//   O1: after calling refresh_mempool_producer_snapshot(height) on a node
//       with N producers, the mempool's snapshot has exactly N entries
//   O2: each entry's PublicKey appears in the Node's ProducerSet
//       active_producers_at_height
//   O3: each entry's weight is > 0 (positive bond)
// PATHS:
//   P1: 3-producer node, refresh at height=1
// INPUT PARTITIONS:
//   P1: cold-start node (snapshot empty) → first refresh tick
// MATRIX:
//   P1×O1✓  P1×O2✓  P1×O3✓
//
// AUDIT-P1-001: pre-fix the snapshot was always empty Vec. This test
// pins the wiring: Node.refresh_mempool_producer_snapshot must read
// from producer_set + bond_weights_for_scheduling and write into the
// std::sync::RwLock that the mempool's admission path reads on every
// add_transaction / add_system_transaction.
#[tokio::test]
async fn audit_p1_001_mempool_producer_snapshot_populates_after_refresh() {
    let (node, _tmp) = make_node(3).await;

    // Pre-refresh: snapshot is empty (init.rs hardcodes empty Vec).
    {
        let snapshot = node
            .mempool_active_producers_snapshot
            .read()
            .expect("snapshot lock not poisoned");
        assert!(
            snapshot.is_empty(),
            "AUDIT-P1-001 baseline: snapshot must start empty pre-first-refresh"
        );
    }

    // Refresh — production code calls this after every apply_block commit.
    node.refresh_mempool_producer_snapshot(1).await;

    // Post-refresh: snapshot reflects the active producer set.
    let active = {
        let ps = node.producer_set.read().await;
        ps.active_producers_at_height(1)
            .iter()
            .map(|p| p.public_key)
            .collect::<Vec<_>>()
    };
    let snapshot = node
        .mempool_active_producers_snapshot
        .read()
        .expect("snapshot lock not poisoned");
    assert_eq!(
        snapshot.len(),
        active.len(),
        "AUDIT-P1-001: snapshot length must match active_producers_at_height — \
         got {} snapshot entries vs {} active producers",
        snapshot.len(),
        active.len()
    ); // O1
    for (pk, weight) in snapshot.iter() {
        assert!(
            active.contains(pk),
            "AUDIT-P1-001: every snapshot entry pubkey must be in the active set"
        ); // O2
        assert!(
            *weight > 0,
            "AUDIT-P1-001: every snapshot entry must have positive weight"
        ); // O3
    }
}

// OUTPUT CONTRACT: Transaction::is_zero_flow — PriceAttestation routing
//   O1: PriceAttestation.is_zero_flow() is FALSE (the mempool does NOT
//       route it via add_system_transaction)
// PATHS:
//   P1: a fresh PriceAttestation tx
// INPUT PARTITIONS:
//   P1: PriceAttestation built via Transaction::new_price_attestation
// MATRIX: P1×O1✓
//
// INC-I-173 M3 / F4 (AUDIT-P3-002) — UPDATED, not deleted. This test
// previously asserted `is_state_only() == true`. That predicate is
// DELETED: it routed on TYPE alone, so ClaimReward/ClaimBond — which
// carry OUTPUTS — took the 0-fee lane and gave free relay that
// evict_lowest_fee then used to push out legitimate 0-fee governance
// transactions (FM-10). Routing is now SHAPE-based.
//
// DELTA: PriceAttestation loses system routing, because
// TxType::allows_empty_io is false for it. NO LIVE IMPACT —
// oracle_activation_height is u64::MAX on every network, so no
// PriceAttestation can be mined anywhere. Reclassifying it `true` is
// spec Option B and is explicitly OUT of M3 scope: allows_empty_io's
// true-set is curated by AUTHORIZATION, not by wire shape, so widening
// it is its own decision with its own activation-height question.
//
// RENAMED at review iteration 1 (M3a / F7) from
// `audit_p1_003_price_attestation_classified_state_only`, which asserted the
// OPPOSITE of what its name said after the F4 update. Traceability:
// AUDIT-P1-003 (kept in the name) and AUDIT-P3-002 (this comment block).
#[tokio::test]
async fn audit_p1_003_price_attestation_is_not_system_routed() {
    let kp = crypto::KeyPair::generate();
    let mut data = PriceAttestationData {
        signer_pubkey: *kp.public_key(),
        price_cents: 100,
        pair_id: doli_core::oracle::phase_2_1_known_pair_id(),
        epoch_number: 1,
        signature: Default::default(),
    };
    data.signature = crypto::signature::sign_hash(&data.signing_message(), kp.private_key());
    let tx = Transaction::new_price_attestation(data);

    assert_eq!(tx.tx_type, TxType::PriceAttestation);
    assert!(
        tx.inputs.is_empty() && tx.outputs.is_empty(),
        "setup: a PriceAttestation is 0-in/0-out, so only the TYPE half of \
         is_zero_flow can decide its routing"
    );
    assert!(
        !tx.is_zero_flow(),
        "INC-I-173 M3 / F4: PriceAttestation is NOT in TxType::allows_empty_io, so it \
         must NOT take the 0-fee system lane. If this fires, the exempt type list was \
         widened without the activation-height decision that widening requires \
         (spec Option B, out of M3 scope)."
    );
}
