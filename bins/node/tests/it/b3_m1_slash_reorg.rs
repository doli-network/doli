//! B3 M1 / R-1 — a slash this node created and saw mined stays deliverable
//! until the boundary: a rollback or a reorg of its including block brings the
//! same tx back to the pool, and it is never resident while it is queued.
//!
// covers: slash_redeliver.rs slash_reported.rs slash_submit.rs periodic.rs rollback.rs block_handling.rs
//
// OUTPUT CONTRACT: fn Node::run_periodic_tasks(&mut self) -> Result<()> (-> redeliver_slashes)
//   O1 mempool: SlashProducer entries naming the accused (ids, tx bytes)
//   O2 self.slash_reported ledger (crate-private; observed through what a later tick re-offers)
//   O3 return: Ok
// OUTPUT CONTRACT: fn Node::rollback_one_block / execute_reorg (the including block undone)
//   O1 producer set: Slash for the accused no longer queued
//   O2 chain tip; O3 canonical chain: SlashProducer txs naming the accused
// PATHS:
//   P-Q   mined and queued -> ticks re-admit nothing
//   P-RB  rollback of the including block, then a tick -> same tx resident, mineable, one on chain
//   P-RG  execute_reorg onto a sibling without the slash, then a tick -> same, mined on the sibling
// INPUT PARTITIONS:
//   undo: rollback_one_block | execute_reorg with a sibling; ticks while queued: 2
//   the tick while queued is the partition that empties the ledger at 6977585c
// MATRIX: P-Q x O1 (x2 ticks, in both setups) | P-RB x {O1 ids, O3 count} | P-RG x {O1 ids, O2, O3 count}
//   | P-RB x O1 bytes (same id as the mined tx) = 3 tests

use crypto::{Hash, PublicKey};
use doli_core::transaction::Transaction;
use doli_core::Block;
use doli_node::node::Node;
use network::sync::ReorgResult;

use super::inc_i_255_m2_common::{queued_slash, rig, Keys, Rig, EPOCH};
use super::inc_i_256_m1_slash_fx::{offer, resident, tick, Offence};

/// The slash mined mid-epoch at EPOCH + 1 and two ticks run while it is queued.
struct Mined {
    r: Rig,
    o: Offence,
    including: Block,
    mined_tx: Transaction,
    sibling: Block,
}

async fn resident_tx(node: &Node, id: &Hash) -> Transaction {
    let pool = node.mempool.read().await;
    pool.get(id)
        .map(|e| e.tx.clone())
        .unwrap_or_else(|| panic!("FIXTURE: slash {id} resident"))
}

/// SlashProducer txs naming `pk` on the canonical chain 1..=tip.
async fn mined_on_chain(node: &Node, pk: &PublicKey) -> usize {
    let tip = node.chain_state.read().await.best_height;
    let mut n = 0;
    for h in 1..=tip {
        let b = node
            .block_store
            .get_block_by_height(h)
            .expect("FIXTURE: block store read")
            .unwrap_or_else(|| panic!("FIXTURE: canonical block {h} stored"));
        n += b
            .transactions
            .iter()
            .filter(|t| t.slash_data().is_some_and(|d| d.producer_pubkey == *pk))
            .count();
    }
    n
}

async fn mined_and_queued() -> Mined {
    let mut r = rig(&Keys::new()).await;
    r.advance_to(EPOCH).await;
    let o = Offence::by(&r.keys.accused);
    offer(&mut r.node, &o).await;
    assert_eq!(
        resident(&r.node, &o).await,
        vec![o.id()],
        "PRECONDITION: the node's own slash path admits the canonical slash"
    );
    let sibling = r.next_block(Vec::new(), 1).await;
    let mined_tx = resident_tx(&r.node, &o.id()).await;
    let including = r.step(vec![mined_tx.clone()]).await;
    assert!(
        queued_slash(&r.node, &o.accused).await,
        "PRECONDITION: mid-epoch the mined slash is queued"
    );
    for round in 0..2 {
        tick(&mut r.node).await;
        let now = resident(&r.node, &o).await;
        assert!(
            now.is_empty(),
            "tick {round} while queued: the mined slash must not be re-admitted (double mining); got {now:?}"
        );
    }
    Mined {
        r,
        o,
        including,
        mined_tx,
        sibling,
    }
}

/// After the undo: one tick re-offers the same id, a block mines it, one slash on chain.
async fn assert_reoffered_and_mined_once(m: &mut Mined, what: &str) {
    let pk = m.o.accused;
    assert!(
        !queued_slash(&m.r.node, &pk).await,
        "{what}: PRECONDITION: the undo removed the queued Slash"
    );
    tick(&mut m.r.node).await;
    let after = resident(&m.r.node, &m.o).await;
    println!(
        "B3M1_METRIC SLASH_RESIDENT_AFTER_UNDO={}",
        u8::from(after.contains(&m.o.id()))
    );
    assert_eq!(
        after,
        vec![m.o.id()],
        "{what}: one tick after the including block is undone must re-offer the slash"
    );
    let tx = resident_tx(&m.r.node, &m.o.id()).await;
    m.r.step(vec![tx]).await;
    assert!(
        queued_slash(&m.r.node, &pk).await,
        "{what}: the re-offered slash is mined and queued again"
    );
    tick(&mut m.r.node).await;
    assert!(
        resident(&m.r.node, &m.o).await.is_empty(),
        "{what}: queued again, the slash is not re-admitted"
    );
    assert_eq!(
        mined_on_chain(&m.r.node, &pk).await,
        1,
        "{what}: exactly one mined slash for the offence on the final chain"
    );
}

// REQ-B3-001 — Decision: a failure means a rollback of the block that mined this node's slash, before the boundary, loses the slash for good and the offence goes unpunished.
#[tokio::test]
async fn b3_m1_slash_reoffered_after_rollback_of_including_block() {
    let mut m = mined_and_queued().await;
    m.r.rollback().await;
    assert_eq!(
        m.r.best().await.0,
        EPOCH,
        "PRECONDITION: the including block is rolled back"
    );
    assert_reoffered_and_mined_once(&mut m, "rollback").await;
}

// REQ-B3-001 — Decision: a failure means a reorg onto a sibling without the slash, before the boundary, loses this node's slash for good.
#[tokio::test]
async fn b3_m1_slash_reoffered_after_reorg_to_sibling() {
    let mut m = mined_and_queued().await;
    let reorg = ReorgResult {
        rollback: vec![m.including.hash()],
        common_ancestor: m.including.header.prev_hash,
        new_blocks: vec![m.sibling.hash()],
        weight_delta: 1,
    };
    m.r.node
        .execute_reorg(reorg, m.sibling.clone())
        .await
        .unwrap_or_else(|e| panic!("FIXTURE: execute_reorg failed: {e:#}"));
    let (h, _, hash) = m.r.best().await;
    assert_eq!(
        (h, hash),
        (EPOCH + 1, m.sibling.hash()),
        "PRECONDITION: the reorg replaced the including block by the sibling"
    );
    assert_reoffered_and_mined_once(&mut m, "reorg to sibling").await;
}

// REQ-B3-001 — Decision: a failure means the re-offer mints a different slash (new id) for an offence already mined once, so two ids can be mined for one offence.
#[tokio::test]
async fn b3_m1_reoffered_slash_keeps_tx_id() {
    let mut m = mined_and_queued().await;
    m.r.rollback().await;
    tick(&mut m.r.node).await;
    let after = resident(&m.r.node, &m.o).await;
    assert_eq!(
        after,
        vec![m.mined_tx.hash()],
        "the re-offered slash must carry the id of the tx the undone block mined"
    );
    let back = resident_tx(&m.r.node, &m.mined_tx.hash()).await;
    assert_eq!(
        back.serialize(),
        m.mined_tx.serialize(),
        "the re-offered slash is byte-identical to the mined tx"
    );
}
