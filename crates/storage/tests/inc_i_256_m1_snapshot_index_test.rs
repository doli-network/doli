//! INC-I-256 M1 — a producer-set snapshot revived for a restore must carry its
//! unbonding index: plain `bincode::deserialize` leaves the skip-serialized
//! index empty, so an Unbonding producer never completes on the restored set;
//! `ProducerSet::from_snapshot_bytes` must rebuild it. `has_unbonding_due` is
//! the snapshot predicate for blocks where `process_unbonding` mutates the set.
//!
// covers: apply_block producer_undo helpers mod block_handling rollback periodic fork_cache set_core set_lifecycle set_persistence types chain snapshot_install
//
// OUTPUT CONTRACT: fn ProducerSet::from_snapshot_bytes(&[u8]) -> Result<ProducerSet, _>
//   O1 return: the set, byte-equal (serialize_canonical, pending queue) to the snapshotted one
//   O2 receiver state after process_unbonding(s + U, U) on the revived set: return [p], p Exited
// OUTPUT CONTRACT: fn ProducerSet::has_unbonding_due(&self, height, period) -> bool
//   O3 return per height
// PATHS: P-PLAIN bincode::deserialize (documents the gap: returns [] and p stays Unbonding)
//   | P-REVIVE from_snapshot_bytes | P-DUE has_unbonding_due
// INPUT PARTITIONS: p Unbonding from s, another producer Active; height s + U (due), s + U - 1 (not due);
//   a set with no Unbonding producer (never due)
// MATRIX: O1-O2 x {P-PLAIN, P-REVIVE} + O3 x 3 partitions = 2 tests

use crypto::{KeyPair, PublicKey};
use storage::{ProducerSet, ProducerStatus};

const UNIT: u64 = 1_000_000;
const STARTED_AT: u64 = 7;
const PERIOD: u64 = 60;

fn set_with_unbonding(p: &PublicKey, other: &PublicKey) -> ProducerSet {
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*p, 2, UNIT).unwrap();
    ps.register_genesis_producer(*other, 1, UNIT).unwrap();
    ps.request_exit(p, STARTED_AT).unwrap();
    ps
}

fn status_of(ps: &ProducerSet, p: &PublicKey) -> Option<ProducerStatus> {
    ps.get_by_pubkey(p).map(|i| i.status)
}

fn completed_keys(ps: &mut ProducerSet, height: u64) -> Vec<PublicKey> {
    ps.process_unbonding(height, PERIOD)
        .into_iter()
        .map(|i| i.public_key)
        .collect()
}

// R-1 (INC-I-256) — Decision: a failure means a restore of a producer snapshot (failed apply, rollback, reorg, snap install) loses the unbonding index, so an Unbonding producer never exits on the restored node and stays scheduled.
#[test]
fn inc_i_256_m1_snapshot_restore_rebuilds_unbonding_index() {
    let (p, other) = (KeyPair::generate(), KeyPair::generate());
    let (p, other) = (*p.public_key(), *other.public_key());
    let ps = set_with_unbonding(&p, &other);
    let bytes = bincode::serialize(&ps).unwrap();
    let due = STARTED_AT + PERIOD;

    let mut plain: ProducerSet = bincode::deserialize(&bytes).unwrap();
    let plain_done = completed_keys(&mut plain, due);
    let plain_status = status_of(&plain, &p);

    let mut revived = ProducerSet::from_snapshot_bytes(&bytes).expect("snapshot bytes decode");
    assert_eq!(
        revived.serialize_canonical(),
        ps.serialize_canonical(),
        "the revived set is byte-equal to the snapshotted one"
    );
    assert_eq!(
        revived.pending_update_count(),
        ps.pending_update_count(),
        "the revived set keeps the pending queue"
    );
    let revived_done = completed_keys(&mut revived, due);
    assert_eq!(
        (plain_done, plain_status),
        (
            Vec::new(),
            Some(ProducerStatus::Unbonding {
                started_at: STARTED_AT
            })
        ),
        "plain bincode revive: the index is empty, so p never completes"
    );
    assert_eq!(
        revived_done,
        vec![p],
        "from_snapshot_bytes: p completes its unbonding at {due}"
    );
    assert_eq!(
        status_of(&revived, &p),
        Some(ProducerStatus::Exited),
        "from_snapshot_bytes: p is Exited after the due height"
    );
}

// R-1 (INC-I-256) — Decision: a failure means the apply snapshot predicate misses a block where process_unbonding mutates the set (no undo to restore) or snapshots blocks where nothing is due.
#[test]
fn inc_i_256_m1_has_unbonding_due_matches_process_unbonding() {
    let (p, other) = (KeyPair::generate(), KeyPair::generate());
    let (p, other) = (*p.public_key(), *other.public_key());
    let ps = set_with_unbonding(&p, &other);
    let due = STARTED_AT + PERIOD;
    assert!(ps.has_unbonding_due(due, PERIOD), "p is due at {due}");
    assert!(
        !ps.has_unbonding_due(due - 1, PERIOD),
        "p is not due at {}",
        due - 1
    );
    let mut probe = ps.clone();
    let before = probe.serialize_canonical();
    assert!(
        completed_keys(&mut probe, due - 1).is_empty() && probe.serialize_canonical() == before,
        "NON-VACUITY: process_unbonding changes nothing where the predicate says not due"
    );
    let mut idle = ProducerSet::new();
    idle.register_genesis_producer(other, 1, UNIT).unwrap();
    assert!(
        !idle.has_unbonding_due(due, PERIOD),
        "a set without Unbonding producers is never due"
    );
}
