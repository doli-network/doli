//! INC-I-256 M1 / R-2 — a valid slash created by this node comes back after a
//! loss or a transient refusal, never for a Slashed, queued or permanently
//! refused accused, and one offence never yields more than one slash.
//!
// OUTPUT CONTRACT: fn Node::handle_equivocation(&mut self, EquivocationProof)
//   (-> submit_equivocation_slash)
//   O1 mempool: SlashProducer entries naming the accused (ids)
//   O2 self.slash_reported ledger (crate-private; observed through what a later tick re-offers)
//   O3 return: () — none
// OUTPUT CONTRACT: fn Node::run_periodic_tasks(&mut self) -> Result<()>
//   O1 mempool: SlashProducer entries naming each accused (ids)
//   O2 ledger entries kept or dropped (observed through later ticks)
//   O3 return: Ok
// OUTPUT CONTRACT: fn Node::build_block_content + apply_block(Full) on a short chain
//   O1 slashes naming the accused the builder selected, per block
// PATHS:
//   P1 admitted, then lost -> one tick re-offers the same id
//   P2 offered with the producer set write-held -> refused, nothing resident; release + tick -> same id
//   P3 lost, accused then Slashed | slash-queued -> ticks re-offer nothing; an untouched accused is re-offered
//   P4 refused for a permanent reason (unregistered accused), then registered -> ticks re-offer nothing
//   P5 6 rounds of loss + tick + re-detection, then 3 builder blocks -> <= 1 resident at any time, <= 1 mined
// INPUT PARTITIONS:
//   accused state: active | Slashed | slash-queued | unregistered then registered
//   lock: free | write-held during the offer
//   rounds: 1 | 6 with re-detection between ticks
// MATRIX: P1 x O1(same id) | P2 x {O1 locked, O1 after tick, never waits on the lock}
//         | P3 x O1 x {X, Q, C} x 2 ticks | P4 x O1 x {Y, C} | P5 x {O1 per round, O1 builder per block, total mined}
// covers: slash_submit slash_redeliver periodic mod block_handling network_events header_auth equivocation slash_admission pool transaction validation_checks init

use std::time::Duration;

use crypto::KeyPair;
use doli_core::validation::ValidationMode;
use storage::PendingProducerUpdate;

use super::inc_i_255_m1_common::{admit, full_block_with, mempool_slash_msg};
use super::inc_i_255_m2_flood_common::{flock, resident_slash_total, slashes_naming};
use super::inc_i_256_m1_slash_fx::*;

const ROUNDS: usize = 6;
const CHAIN: usize = 3;
const LOCK_WAIT: Duration = Duration::from_secs(30);

// REQ-R-2 — Decision: a failure means a lost slash (evicted, expired, purged) is never re-offered by its creator, so the offence goes unpunished (P2-537).
#[tokio::test]
async fn inc_i_256_m1_slash_reoffered_same_id_after_loss() {
    let mut f = flock(1).await;
    let o = Offence::by(&f.accused[0]);
    offer(&mut f.node, &o).await;
    assert_eq!(
        resident(&f.node, &o).await,
        vec![o.id()],
        "PRECONDITION: the node's own slash path admits the canonical slash"
    );
    lose(&f.node, &o.id()).await;
    tick(&mut f.node).await;
    let after = resident(&f.node, &o).await;
    metric_resident(&after, &o.id());
    assert_eq!(
        after,
        vec![o.id()],
        "one periodic tick after the loss must re-offer the SAME slash id"
    );
}

// REQ-R-2 — Decision: a failure means a slash refused while the producer set was busy is admitted fail-open, or is never re-offered once the lock is free (P2-538).
#[tokio::test]
async fn inc_i_256_m1_slash_reoffered_after_busy_refusal() {
    let mut f = flock(1).await;
    let o = Offence::by(&f.accused[0]);
    let set = f.node.producer_set.clone();
    let guard = set.write().await;
    let offered = tokio::time::timeout(LOCK_WAIT, offer(&mut f.node, &o)).await;
    let while_locked = resident(&f.node, &o).await;
    drop(guard);
    assert!(
        offered.is_ok(),
        "the slash path must refuse while the producer set is write-held, never wait on the lock"
    );
    assert!(
        while_locked.is_empty(),
        "set write-held: the slash must be refused (fail closed), nothing resident; got {while_locked:?}"
    );
    tick(&mut f.node).await;
    let after = resident(&f.node, &o).await;
    metric_resident(&after, &o.id());
    assert_eq!(
        after,
        vec![o.id()],
        "after the lock is released, one tick must re-offer the SAME slash id"
    );
}

// REQ-R-2 — Decision: a failure means the re-offer path brings back a slash whose accused is already Slashed or slash-queued, or re-offers nothing at all.
#[tokio::test]
async fn inc_i_256_m1_slash_tick_skips_slashed_and_queued_accused() {
    let mut f = flock(3).await;
    let offs: Vec<Offence> = f.accused.iter().map(Offence::by).collect();
    for o in &offs {
        offer(&mut f.node, o).await;
        assert_eq!(
            resident(&f.node, o).await,
            vec![o.id()],
            "PRECONDITION: each offence is admitted"
        );
        lose(&f.node, &o.id()).await;
    }
    let (slashed, queued, control) = (&offs[0], &offs[1], &offs[2]);
    {
        let mut ps = f.node.producer_set.write().await;
        ps.slash_producer(&slashed.accused, 0)
            .expect("FIXTURE: slash X");
        ps.queue_update(PendingProducerUpdate::Slash {
            pubkey: queued.accused,
            height: 0,
        });
    }
    for round in 0..2 {
        tick(&mut f.node).await;
        let x = resident(&f.node, slashed).await;
        let q = resident(&f.node, queued).await;
        let c = resident(&f.node, control).await;
        assert!(
            x.is_empty(),
            "tick {round}: a Slashed accused must never get its slash back; got {x:?}"
        );
        assert!(
            q.is_empty(),
            "tick {round}: a slash-queued accused must never get its slash back; got {q:?}"
        );
        assert_eq!(
            c,
            vec![control.id()],
            "tick {round}: NON-VACUITY: the untouched accused's slash is re-offered with the same id"
        );
    }
}

// REQ-R-2 — Decision: a failure means a slash refused for a permanent reason stays in the re-offer ledger and is retried on every tick.
#[tokio::test]
async fn inc_i_256_m1_slash_tick_drops_permanently_refused_offer() {
    let mut f = flock(1).await;
    let stranger = KeyPair::generate();
    let refused = Offence::by(&stranger);
    let control = Offence::by(&f.accused[0]);
    offer(&mut f.node, &refused).await;
    assert!(
        resident(&f.node, &refused).await.is_empty(),
        "PRECONDITION: a slash against a key outside the producer set is refused"
    );
    let why = admit(&f.node, &refused.tx).await;
    assert!(
        mempool_slash_msg(&why).is_some_and(|m| m.contains("not a registered producer")),
        "PRECONDITION: the refusal is the permanent target rule; got {why:?}"
    );
    offer(&mut f.node, &control).await;
    lose(&f.node, &control.id()).await;
    {
        let unit = f.node.config.network.bond_unit();
        f.node
            .producer_set
            .write()
            .await
            .register_genesis_producer(refused.accused, 1, unit)
            .expect("FIXTURE: register the stranger after the refusal");
    }
    for round in 0..2 {
        tick(&mut f.node).await;
        let y = resident(&f.node, &refused).await;
        assert!(
            y.is_empty(),
            "tick {round}: a slash refused for a permanent reason must leave the re-offer ledger; got {y:?}"
        );
        assert_eq!(
            resident(&f.node, &control).await,
            vec![control.id()],
            "tick {round}: NON-VACUITY: the lost valid slash is re-offered with the same id"
        );
    }
}

// REQ-R-2 — Decision: a failure means re-delivery floods the pool or the chain with more than one slash for one offence.
#[tokio::test]
async fn inc_i_256_m1_slash_redelivery_never_floods() {
    let mut f = flock(1).await;
    let o = Offence::by(&f.accused[0]);
    offer(&mut f.node, &o).await;
    assert_eq!(
        resident(&f.node, &o).await,
        vec![o.id()],
        "PRECONDITION: the offence is admitted"
    );
    let mut peak = 1;
    for round in 0..ROUNDS {
        if resident(&f.node, &o).await.contains(&o.id()) {
            lose(&f.node, &o.id()).await;
        }
        tick(&mut f.node).await;
        offer(&mut f.node, &o).await;
        let now = resident(&f.node, &o).await;
        assert!(
            now.len() <= 1 && now.iter().all(|h| *h == o.id()),
            "round {round}: at most one resident slash for the offence, always the canonical id; got {now:?}"
        );
        assert_eq!(
            resident_slash_total(&f.node).await,
            now.len(),
            "round {round}: no other slash tx in the pool"
        );
        peak = peak.max(now.len());
    }
    let mut mined = 0;
    for n in 0..CHAIN {
        let (block, selected) = full_block_with(&mut f.node, &[]).await;
        mined += slashes_naming(&block, &selected, &o.accused);
        let r = f.node.apply_block(block, ValidationMode::Full).await;
        assert!(
            r.is_ok(),
            "FIXTURE: builder block {n} applies; got {:?}",
            r.err().map(|e| format!("{e:#}"))
        );
        tick(&mut f.node).await;
        offer(&mut f.node, &o).await;
        let now = resident(&f.node, &o).await;
        assert!(
            now.len() <= 1,
            "block {n}: at most one resident slash for the offence; got {now:?}"
        );
    }
    println!("B2M1_METRIC SLASH_PEAK_RESIDENT={peak} SLASH_MINED={mined}");
    assert!(
        mined <= 1,
        "one offence must yield at most one mined slash on the chain; mined {mined}"
    );
}
