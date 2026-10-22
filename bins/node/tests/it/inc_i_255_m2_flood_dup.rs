//! INC-I-255 M2 — one slash per accused: a second slash naming the same
//! accused is refused in one block, by the builder, at admission, after the
//! first is mined, and evicted by revalidation once the accused is queued.
//!
// OUTPUT CONTRACT: fn Node::validate_block_for_apply(&self, &Block, u64, ValidationMode) -> Result<(), ValidationError>
//   O1 return (InvalidSlash message) per mode
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return per mode; O2 chain tip; O3 producer-set slash record
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 SlashProducer transactions naming the accused that the builder selected
// OUTPUT CONTRACT: fn Node::handle_new_transaction(&self, Transaction) -> bool
//   O1 return; O2 mempool slash residency per accused
// OUTPUT CONTRACT: fn Mempool::revalidate(&mut self, &UtxoSet, u64, u32)
//   O1 mempool residency of a slash whose accused changed state
// PATHS: block validation x 3 modes | apply x 3 modes | builder | admission | revalidate
// MATRIX
//   validate/apply O1,O2 x two slashes, same accused, one block -> InvalidSlash, one message, tip kept (probe, *_every_path)
//   builder O1 x both offered -> exactly one selected (inc_i_255_m2_dup_builder_one_per_accused)
//   handle_new_transaction O1,O2 x second evidence, first resident -> false, [first]; first evicted -> second admissible
//   admission + validate O1 x accused queued by a mined slash -> InvalidSlash (inc_i_255_m2_dup_refused_after_mined)
//   revalidate O1 x {queued, Slashed} -> evicted; x unchanged -> kept (inc_i_255_m2_dup_revalidate_*)
// INPUT PARTITIONS:
//   evidence for one accused: pair A | pair B (different headers, same slot)
//   accused state: Active | slash resident | Slash queued | Slashed

use doli_core::validation::ValidationMode;
use storage::PendingProducerUpdate;

use super::inc_i_255_m1_common::*;
use super::inc_i_255_m2_flood_common::*;

fn second_slash(w: &World) -> doli_core::transaction::Transaction {
    slash_of(&w.accused, 0xC3, 0xD4)
}

/// World whose reporter key sorts before the accused key, so the reporter's
/// attendance bit stays index 0 once the slashed accused leaves the active set
/// at epoch 0 (pre-AH stray-bit width is the active count).
async fn world_reporter_first() -> World {
    let (a, b) = (crypto::KeyPair::generate(), crypto::KeyPair::generate());
    let (reporter, accused) = if a.public_key().as_bytes() < b.public_key().as_bytes() {
        (a, b)
    } else {
        (b, a)
    };
    world_with(&reporter, &accused).await
}

async fn revalidate_now(w: &World) {
    let (h, s) = {
        let st = w.node.chain_state.read().await;
        (st.best_height, st.best_slot)
    };
    let utxo = w.node.utxo_set.read().await;
    w.node.mempool.write().await.revalidate(&utxo, h, s);
}

// REQ-SLASH-004 — Decision: a failure means two slashes for one accused can sit resident together, so the duplicate reaches the builder and the wire.
#[tokio::test]
async fn inc_i_255_m2_probe_duplicate() {
    let w = world().await;
    let (first, second) = (valid_slash(&w), second_slash(&w));
    assert_ne!(
        first.hash(),
        second.hash(),
        "FIXTURE: two distinct evidence pairs"
    );
    assert!(
        w.node.handle_new_transaction(first.clone()).await,
        "PRECONDITION: the first slash is admitted"
    );
    let admitted = w.node.handle_new_transaction(second).await;
    assert!(
        !admitted,
        "a second slash naming a resident accused must be refused at admission"
    );
    assert_eq!(
        resident_slashes(&w.node, &w.accused_pk()).await,
        vec![first.hash()]
    );
}

// REQ-SLASH-004 — Decision: a failure names the block path that accepts two slashes for one accused in one block (double queue, double burn attempt) or refuses it differently.
#[tokio::test]
async fn inc_i_255_m2_dup_same_block_refused_on_every_path() {
    let mut w = world().await;
    let pair = [valid_slash(&w), second_slash(&w)];
    let (block, _) = full_block_with(&mut w.node, &pair).await;
    refused_everywhere(&mut w.node, &block, "two slashes, one accused").await;
    assert!(
        !slash_recorded(&w.node, &w.accused_pk()).await,
        "a refused block must record no slash"
    );
    let (single, _) = full_block_with(&mut w.node, &pair[..1]).await;
    let r = w
        .node
        .validate_block_for_apply(&single, 1, ValidationMode::Full)
        .await;
    assert!(
        r.is_ok(),
        "NON-VACUITY: one of the two alone validates; got {r:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means the builder packs two slashes for one accused and produces a block every node rejects.
#[tokio::test]
async fn inc_i_255_m2_dup_builder_one_per_accused() {
    let mut w = world().await;
    assert!(
        inject(&w.node, &valid_slash(&w)).await,
        "PRECONDITION: first resident"
    );
    inject(&w.node, &second_slash(&w)).await;
    let (block, selected) = full_block_with(&mut w.node, &[]).await;
    assert_eq!(
        slashes_naming(&block, &selected, &w.accused_pk()),
        1,
        "the builder must select exactly one slash for the accused"
    );
}

// REQ-SLASH-004 — Decision: a failure means residency, not evidence, is not what refuses the second slash (it stays refused after the first leaves), or it never is refused.
#[tokio::test]
async fn inc_i_255_m2_dup_second_admissible_once_first_leaves() {
    let w = world().await;
    let (first, second) = (valid_slash(&w), second_slash(&w));
    assert!(
        w.node.handle_new_transaction(first.clone()).await,
        "PRECONDITION: first admitted"
    );
    let refused = admit(&w.node, &second).await;
    assert!(
        refused.is_err(),
        "second slash for a resident accused must be refused; got {refused:?}"
    );
    w.node
        .mempool
        .write()
        .await
        .remove_transaction(&first.hash());
    let r = admit(&w.node, &second).await;
    assert!(
        r.is_ok(),
        "NON-VACUITY: with no resident slash the second evidence is admissible; got {r:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means a mined (queued) slash can be followed by a second mined slash for the same accused, the per-offender flood.
#[tokio::test]
async fn inc_i_255_m2_dup_refused_after_mined() {
    let mut w = world_reporter_first().await;
    let (first, second) = (valid_slash(&w), second_slash(&w));
    let (block, _) = full_block_with(&mut w.node, std::slice::from_ref(&first)).await;
    let r = w.node.apply_block(block, ValidationMode::Full).await;
    assert!(
        r.is_ok(),
        "PRECONDITION: the first slash is mined; got {:?}",
        r.err()
    );
    assert!(
        slash_recorded(&w.node, &w.accused_pk()).await,
        "PRECONDITION: queued"
    );

    let a = admit(&w.node, &second).await;
    assert!(
        mempool_slash_msg(&a).is_some(),
        "admission must refuse a slash for an already-queued accused; got {a:?}"
    );
    let (next, _) = full_block_with(&mut w.node, std::slice::from_ref(&second)).await;
    let v = w
        .node
        .validate_block_for_apply(&next, 2, ValidationMode::Full)
        .await;
    assert!(
        block_slash_msg(&v).is_some(),
        "block validation must refuse a second slash for a queued accused; got {v:?}"
    );
}

// REQ-SLASH-004 — Decision: a failure means a resident slash outlives the accused's queued slash and is re-offered to the builder every slot.
#[tokio::test]
async fn inc_i_255_m2_dup_revalidate_evicts_when_accused_queued() {
    let w = world().await;
    let tx = valid_slash(&w);
    assert!(
        w.node.handle_new_transaction(tx.clone()).await,
        "PRECONDITION: resident"
    );
    revalidate_now(&w).await;
    assert!(
        w.node.mempool.read().await.contains(&tx.hash()),
        "NON-VACUITY: revalidate keeps the slash while the accused is unchanged"
    );
    w.node
        .producer_set
        .write()
        .await
        .queue_update(PendingProducerUpdate::Slash {
            pubkey: w.accused_pk(),
            height: 0,
        });
    revalidate_now(&w).await;
    assert!(
        !w.node.mempool.read().await.contains(&tx.hash()),
        "revalidate must evict a slash whose accused already has a queued slash"
    );
}

// REQ-SLASH-004 — Decision: a failure means a resident slash outlives the accused becoming Slashed and is re-offered to the builder every slot.
#[tokio::test]
async fn inc_i_255_m2_dup_revalidate_evicts_when_accused_slashed() {
    let w = world().await;
    let tx = valid_slash(&w);
    assert!(
        w.node.handle_new_transaction(tx.clone()).await,
        "PRECONDITION: resident"
    );
    w.node
        .producer_set
        .write()
        .await
        .slash_producer(&w.accused_pk(), 0)
        .expect("FIXTURE: slash the accused");
    revalidate_now(&w).await;
    assert!(
        !w.node.mempool.read().await.contains(&tx.hash()),
        "revalidate must evict a slash whose accused is already Slashed"
    );
}
