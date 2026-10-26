//! INC-I-255 M2 — exclusion point of a mined slash: in epoch > 0 the accused
//! leaves selection at the NEXT EPOCH BOUNDARY, in epoch 0 at the NEXT BLOCK;
//! a second flush against a Slashed producer changes nothing.
//!
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()> (slash block, then later blocks)
//   O1 accused status / is_active / record kept as a tombstone
//   O2 scheduling list (active_producers_for_scheduling_at_height)
//   O3 bond weight at later heights (selection_weight_at)
// OUTPUT CONTRACT: fn ProducerSet::slash_producer(&mut self, &PublicKey, u64) -> Result<u64, StorageError>
//   O1 return; O2 self: status, exit history (direct call and through apply_pending_updates)
// PATHS: P-MID slash mined mid-epoch | P-E0 slash mined in epoch 0 | P-AGAIN slash of an already Slashed producer
// MATRIX
//   O1-O3 x P-MID   -> selectable and Active until the boundary; from the boundary: Slashed, not selectable, weight 0, record kept
//                      (inc_i_255_m2_exclusion_epoch_gt0_from_next_epoch_boundary)
//   O1-O3 x P-E0    -> from the next block: Slashed, not selectable, weight 0 (inc_i_255_m2_exclusion_epoch0_from_next_block)
//   O1,O2 x P-AGAIN -> Err; status and exit history unchanged; queued Slash flush is a no-op (inc_i_255_m2_second_slash_*)
//   probe = P-MID + P-AGAIN (inc_i_255_m2_probe_exclusion)
// INPUT PARTITIONS:
//   height of the slash block: epoch 0 | first block after a boundary; check heights: before | at | after the boundary
//   second slash: direct call | queued update flushed

use crypto::KeyPair;
use storage::{PendingProducerUpdate, ProducerSet, ProducerStatus};

use super::inc_i_255_m2_common::*;

fn is_slashed(s: &Option<ProducerStatus>) -> bool {
    matches!(s, Some(ProducerStatus::Slashed { .. }))
}

async fn assert_excluded(r: &Rig, from: u64, what: &str) {
    let pk = r.accused_pk();
    let honest = *r.keys.honest.public_key();
    let st = status(&r.node, &pk).await;
    assert!(
        is_slashed(&st),
        "{what}: status must be Slashed; got {st:?}"
    );
    let ps = r.node.producer_set.read().await;
    let info = ps
        .get_by_pubkey(&pk)
        .unwrap_or_else(|| panic!("{what}: the record must be kept as a tombstone"));
    assert!(!info.is_active(), "{what}: is_active() must be false");
    drop(ps);
    for h in [from, from + 1, from + EPOCH, from + 3 * EPOCH] {
        assert!(
            !schedulable(&r.node, &pk, h).await,
            "{what}: accused still selectable at h={h}"
        );
        assert_eq!(
            weight(&r.node, &pk, h).await,
            0,
            "{what}: bond weight at h={h} must be 0"
        );
        assert!(
            schedulable(&r.node, &honest, h).await,
            "NON-VACUITY: the honest producer stays selectable at h={h}"
        );
    }
}

async fn excluded_from_next_boundary() {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    let pk = r.accused_pk();
    assert!(
        schedulable(&r.node, &pk, EPOCH + 1).await,
        "PRECONDITION: selectable before the slash"
    );
    r.step(vec![r.slash()]).await;
    while r.best().await.0 < 2 * EPOCH - 1 {
        let h = r.best().await.0;
        assert!(
            !is_slashed(&status(&r.node, &pk).await),
            "epoch > 0: excluded before the boundary at h={h}"
        );
        assert!(
            schedulable(&r.node, &pk, h + 1).await,
            "epoch > 0: the accused stays selectable until the next epoch boundary (h={})",
            h + 1
        );
        r.step(Vec::new()).await;
    }
    assert!(schedulable(&r.node, &pk, 2 * EPOCH - 1).await);
    r.step(Vec::new()).await;
    assert_eq!(r.best().await.0, 2 * EPOCH);
    assert_excluded(&r, 2 * EPOCH, "epoch > 0, from the next epoch boundary").await;
}

fn second_slash_refused() {
    let unit = 1_000;
    let (pk, other) = (
        *KeyPair::generate().public_key(),
        *KeyPair::generate().public_key(),
    );
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(pk, 2, unit).unwrap();
    ps.register_genesis_producer(other, 2, unit).unwrap();
    ps.slash_producer(&pk, 10)
        .expect("PRECONDITION: first slash");
    let first = ps.serialize_canonical();
    let exits = ps.as_parts().1.clone();

    let again = ps.slash_producer(&pk, 20);
    assert!(
        again.is_err(),
        "slashing an already Slashed producer must be refused; got {again:?}"
    );
    assert_eq!(
        ps.get_by_pubkey(&pk).map(|p| p.status),
        Some(ProducerStatus::Slashed { slashed_at: 10 }),
        "a refused second slash must not move slashed_at"
    );
    assert_eq!(ps.as_parts().1, &exits, "exit history changed");

    ps.queue_update(PendingProducerUpdate::Slash {
        pubkey: pk,
        height: 30,
    });
    ps.apply_pending_updates();
    assert_eq!(
        ps.serialize_canonical(),
        first,
        "a second Slash flush must leave the producer set unchanged"
    );
    assert_eq!(ps.as_parts().1, &exits, "flush changed the exit history");
}

// REQ-SLASH-003 — Decision: a failure means a slashed producer keeps or regains authority, or a re-flushed slash rewrites its record, so nodes diverge on its state.
#[tokio::test]
async fn inc_i_255_m2_probe_exclusion() {
    excluded_from_next_boundary().await;
    second_slash_refused();
}

// REQ-SLASH-003 — Decision: a failure means exclusion lands at another height than the next epoch boundary, changing scheduler inputs mid-epoch.
#[tokio::test]
async fn inc_i_255_m2_exclusion_epoch_gt0_from_next_epoch_boundary() {
    excluded_from_next_boundary().await;
}

// REQ-SLASH-003 — Decision: a failure means a slash mined in epoch 0 is not excluded from the next block on.
#[tokio::test]
async fn inc_i_255_m2_exclusion_epoch0_from_next_block() {
    let mut r = rig(&Keys::new()).await;
    let pk = r.accused_pk();
    assert!(
        schedulable(&r.node, &pk, 1).await,
        "PRECONDITION: selectable before the slash"
    );
    r.step(vec![r.slash()]).await;
    assert_excluded(&r, 2, "epoch 0, from the next block").await;
}

// REQ-SLASH-003 — Decision: a failure means a second flush re-slashes a Slashed producer and rewrites slashed_at and exit history.
#[test]
fn inc_i_255_m2_second_slash_of_slashed_producer_is_refused() {
    second_slash_refused();
}
