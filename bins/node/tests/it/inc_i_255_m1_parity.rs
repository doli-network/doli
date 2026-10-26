//! INC-I-255 M1 — one slash verdict on every path: each invalid evidence or
//! state variant is refused by admission, the builder, block validation and
//! apply (Full, Light, Replay) with the same `InvalidSlash` message, while the
//! valid evidence is admitted.
//!
// OUTPUT CONTRACT: fn Mempool::add_transaction(&mut self, Transaction, &UtxoSet, u64, u32) -> Result<AddTransactionResult, MempoolError>
//   O1 return (InvalidSlash message)
// OUTPUT CONTRACT: fn Node::build_block_content(&mut self, ..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>
//   O1 transactions the builder selected
// OUTPUT CONTRACT: fn Node::validate_block_for_apply(&self, &Block, u64, ValidationMode) -> Result<(), ValidationError>
//   O1 return (InvalidSlash message) per mode
// OUTPUT CONTRACT: fn Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>
//   O1 return (InvalidSlash message) per mode; O2 chain tip unchanged
// PATHS: admission | builder selection | block validation x3 modes | apply x3 modes
//        | admission with the live producer set write-locked (fails closed, transient Busy)
// MATRIX: every variant below x every path -> rejected, one identical message naming the rule;
//         control: the valid evidence on a fresh node -> admitted
//         set locked x {valid -> Busy, unregistered accused -> Busy, trailing byte -> refused}
//           x {return text, pool len, pool membership, resident slashes naming the accused}
//         released x valid -> admitted by the node intake (not known), same id resident;
//                    unregistered -> InvalidSlash again (the state check ran, nothing cached)
//           (inc_i_255_admission_fails_closed_busy_when_producer_set_locked)
// INPUT PARTITIONS:
//   evidence: one bad signature | same header twice | different slots | wrong genesis |
//             encoded > 64 KiB | trailing byte | descending header order | non-default reporter signature
//   accused state: not registered | already Slashed | Slash queued
// covers: slash_submit slash_redeliver periodic mod block_handling network_events header_auth equivocation slash_admission pool transaction validation_checks init

use crypto::{Hash, KeyPair, Signature};
use doli_core::transaction::Transaction;
use storage::PendingProducerUpdate;

use super::inc_i_255_m1_common::*;

#[derive(Clone, Copy)]
enum State {
    Unchanged,
    Slashed,
    Queued,
}

async fn set_state(w: &World, state: State) {
    let pk = w.accused_pk();
    let mut ps = w.node.producer_set.write().await;
    match state {
        State::Unchanged => {}
        State::Slashed => {
            ps.slash_producer(&pk, 0)
                .expect("FIXTURE: slash the accused");
        }
        State::Queued => ps.queue_update(PendingProducerUpdate::Slash {
            pubkey: pk,
            height: 0,
        }),
    }
}

async fn control_admits_valid_evidence(case: &str) {
    let c = world().await;
    let tx = valid_slash(&c);
    let r = admit(&c.node, &tx).await;
    assert!(
        r.is_ok(),
        "{case}: NON-VACUITY: valid evidence must be admitted, else every refusal above is the fee floor; got {r:?}"
    );
}

/// Drives `build`'s tx through every path; asserts one shared InvalidSlash message.
async fn assert_rejected_everywhere(
    case: &str,
    state: State,
    expect: &[&str],
    build: impl FnOnce(&World) -> Transaction,
) {
    let mut w = world().await;
    let tx = build(&w);
    let resident = inject(&w.node, &tx).await;
    set_state(&w, state).await;

    let (block, selected) = full_block_with(&mut w.node, std::slice::from_ref(&tx)).await;
    assert!(
        !selected.contains(&tx.hash()),
        "{case}: builder selected the slash (resident before the state change: {resident})"
    );
    w.node.mempool.write().await.remove_transaction(&tx.hash());

    let mut seen: Vec<(String, Option<String>, String)> = Vec::new();
    let r = admit(&w.node, &tx).await;
    seen.push(("mempool".into(), mempool_slash_msg(&r), format!("{r:?}")));
    let height = tip(&w.node).await.0 + 1;
    for mode in MODES {
        let r = w.node.validate_block_for_apply(&block, height, mode).await;
        seen.push((
            format!("validate {mode:?}"),
            block_slash_msg(&r),
            format!("{r:?}"),
        ));
    }
    for mode in MODES {
        let before = tip(&w.node).await;
        let r = w.node.apply_block(block.clone(), mode).await;
        assert_eq!(
            tip(&w.node).await,
            before,
            "{case}: apply {mode:?} moved the tip"
        );
        let raw = format!("{:?}", r.as_ref().err().map(|e| format!("{e:#}")));
        seen.push((format!("apply {mode:?}"), apply_slash_msg(&r), raw));
    }

    for (path, msg, raw) in &seen {
        assert!(
            msg.is_some(),
            "{case}: {path} must refuse with InvalidSlash; got {raw}"
        );
    }
    let first = seen[0].1.clone().unwrap_or_default();
    for (path, msg, _) in &seen {
        assert_eq!(
            msg.as_deref(),
            Some(first.as_str()),
            "{case}: {path} message differs from admission's"
        );
    }
    assert!(
        expect.iter().any(|s| first.contains(s)),
        "{case}: message {first:?} must name the rule ({expect:?})"
    );
    control_admits_valid_evidence(case).await;
}

fn canonical_pair(
    w: &World,
) -> (
    crypto::PublicKey,
    doli_core::BlockHeader,
    doli_core::BlockHeader,
) {
    let (a, b) = pair(&w.accused, chain_genesis());
    let (lo, hi) = by_hash(a, b);
    (w.accused_pk(), lo, hi)
}

// REQ-SLASH-002 — Decision: a failure means a header the accused never signed is accepted, or refused differently, on some path (SEC-1).
#[tokio::test]
async fn inc_i_255_parity_bad_signature_rejected_on_every_path() {
    assert_rejected_everywhere(
        "bad signature",
        State::Unchanged,
        &["lacks a valid producer signature"],
        |w| {
            let (pk, lo, mut hi) = canonical_pair(w);
            hi.vdf_proof.pi = vec![0xA5; 64];
            slash_tx(pk, lo, hi, Signature::default())
        },
    )
    .await;
}

// REQ-SLASH-002 — Decision: a failure means one header presented twice passes as double production on some path (SEC-2).
#[tokio::test]
async fn inc_i_255_parity_same_hash_twice_rejected_on_every_path() {
    let expect = &["different block hashes", "header order"];
    assert_rejected_everywhere("same hash twice", State::Unchanged, expect, |w| {
        let (pk, lo, _) = canonical_pair(w);
        slash_tx(pk, lo.clone(), lo, Signature::default())
    })
    .await;
}

// REQ-SLASH-002 — Decision: a failure means headers from two slots pass as equivocation on some path (SEC-2).
#[tokio::test]
async fn inc_i_255_parity_different_slots_rejected_on_every_path() {
    assert_rejected_everywhere("different slots", State::Unchanged, &["same slot"], |w| {
        let g = chain_genesis();
        let a = evidence_header(&w.accused, g, SLOT, 0xA1, 0);
        let b = evidence_header(&w.accused, g, SLOT + 1, 0xB2, 0);
        canonical(w.accused_pk(), a, b)
    })
    .await;
}

// REQ-SLASH-002 — Decision: a failure means evidence from another chain slashes a producer here on some path (SEC-3).
#[tokio::test]
async fn inc_i_255_parity_wrong_genesis_rejected_on_every_path() {
    assert_rejected_everywhere(
        "wrong genesis",
        State::Unchanged,
        &["genesis mismatch"],
        |w| {
            let (a, b) = pair(&w.accused, Hash::from_bytes([0x77; 32]));
            canonical(w.accused_pk(), a, b)
        },
    )
    .await;
}

// REQ-SLASH-002 — Decision: a failure means unbounded evidence is decoded or admitted on some path (SEC-4).
#[tokio::test]
async fn inc_i_255_parity_oversize_rejected_on_every_path() {
    assert_rejected_everywhere("oversize", State::Unchanged, &["slash data is"], |w| {
        let g = chain_genesis();
        let a = evidence_header(&w.accused, g, SLOT, 0xA1, 1_100);
        let b = evidence_header(&w.accused, g, SLOT, 0xB2, 1_100);
        let tx = canonical(w.accused_pk(), a, b);
        assert!(
            tx.extra_data.len() > 64 * 1024,
            "FIXTURE: evidence must exceed 64 KiB"
        );
        tx
    })
    .await;
}

// REQ-SLASH-002 — Decision: a failure means padded bytes mint a second id for one evidence pair on some path (SEC-6).
#[tokio::test]
async fn inc_i_255_parity_trailing_bytes_rejected_on_every_path() {
    assert_rejected_everywhere(
        "trailing bytes",
        State::Unchanged,
        &["non-canonical"],
        |w| {
            let mut tx = valid_slash(w);
            tx.extra_data.push(0);
            tx
        },
    )
    .await;
}

// REQ-SLASH-002 — Decision: a failure means the swapped header order mints a second id for one evidence pair on some path (SEC-6).
#[tokio::test]
async fn inc_i_255_parity_wrong_header_order_rejected_on_every_path() {
    assert_rejected_everywhere(
        "wrong header order",
        State::Unchanged,
        &["header order"],
        |w| {
            let (pk, lo, hi) = canonical_pair(w);
            slash_tx(pk, hi, lo, Signature::default())
        },
    )
    .await;
}

// REQ-SLASH-001 — Decision: a failure means the unverifiable reporter field still varies the id or is accepted on some path (B5, SEC-6).
#[tokio::test]
async fn inc_i_255_parity_reporter_signature_rejected_on_every_path() {
    assert_rejected_everywhere(
        "reporter signature",
        State::Unchanged,
        &["reporter signature"],
        |w| {
            let (pk, lo, hi) = canonical_pair(w);
            slash_tx(pk, lo, hi, Signature::from_bytes([0x11; 64]))
        },
    )
    .await;
}

// REQ-SLASH-001 — Decision: a failure means a slash against a key outside the producer set is admitted or mined on some path (B7, SEC-5).
#[tokio::test]
async fn inc_i_255_parity_unknown_producer_rejected_on_every_path() {
    let expect = &["not a registered producer"];
    assert_rejected_everywhere("unknown producer", State::Unchanged, expect, |_| {
        let stranger = KeyPair::generate();
        let (a, b) = pair(&stranger, chain_genesis());
        canonical(*stranger.public_key(), a, b)
    })
    .await;
}

// REQ-SLASH-001 — Decision: a failure means an already slashed producer can be slashed again, a mineable no-op that re-opens the flood (B7, SEC-5).
#[tokio::test]
async fn inc_i_255_parity_already_slashed_rejected_on_every_path() {
    assert_rejected_everywhere(
        "already slashed",
        State::Slashed,
        &["already slashed"],
        valid_slash,
    )
    .await;
}

// REQ-SLASH-001 — Decision: a failure means a second slash is accepted while one is queued, so the builder or Replay diverges from admission (B7, B12).
#[tokio::test]
async fn inc_i_255_parity_pending_slash_rejected_on_every_path() {
    assert_rejected_everywhere(
        "pending slash",
        State::Queued,
        &["pending slash"],
        valid_slash,
    )
    .await;
}

fn busy(r: &Result<(), mempool::MempoolError>) -> bool {
    r.as_ref()
        .err()
        .is_some_and(|e| format!("{e:?}").contains("Busy") || e.to_string().contains("Busy"))
}

// REQ-SLASH-001 — Decision: a failure means a slash is admitted while the accused's producer-set state cannot be read (fail open), or the refusal leaves state that keeps the same tx out once the lock is released.
#[tokio::test]
async fn inc_i_255_admission_fails_closed_busy_when_producer_set_locked() {
    let w = world().await;
    let accused = w.accused_pk();
    let stranger = KeyPair::generate();
    let (sa, sb) = pair(&stranger, chain_genesis());
    let unknown = canonical(*stranger.public_key(), sa, sb);
    let readable = admit(&w.node, &unknown).await;
    assert!(
        mempool_slash_msg(&readable).is_some_and(|m| m.contains("not a registered producer")),
        "PRECONDITION: with the set readable the state check refuses; got {readable:?}"
    );
    let valid = valid_slash(&w);
    let mut trailing = valid.clone();
    trailing.extra_data.push(0);
    let before = w.node.mempool.read().await.len();

    let guard = w.node.producer_set.write().await;
    let locked_valid = admit(&w.node, &valid).await;
    let locked_unknown = admit(&w.node, &unknown).await;
    let locked_trailing = admit(&w.node, &trailing).await;
    drop(guard);

    assert!(
        busy(&locked_valid),
        "set locked: a valid slash must be refused with a transient Busy (fail closed); got {locked_valid:?}"
    );
    assert!(
        busy(&locked_unknown),
        "set locked: the unreadable state check must refuse with Busy, never be skipped; got {locked_unknown:?}"
    );
    assert!(
        locked_trailing.is_err(),
        "set locked: a non-canonical slash stays refused; got {locked_trailing:?}"
    );
    {
        let pool = w.node.mempool.read().await;
        assert_eq!(
            pool.len(),
            before,
            "a Busy refusal must leave the pool unchanged"
        );
        for (what, tx) in [
            ("valid", &valid),
            ("unknown", &unknown),
            ("trailing", &trailing),
        ] {
            assert!(
                !pool.contains(&tx.hash()),
                "set locked: the {what} slash became resident"
            );
        }
    }
    assert!(
        resident_slashes(&w.node, &accused).await.is_empty(),
        "set locked: no slash naming the accused may be resident"
    );

    assert!(
        w.node.handle_new_transaction(valid.clone()).await,
        "after release the same tx must be admitted by the node intake (never marked known or cached as refused)"
    );
    assert_eq!(
        resident_slashes(&w.node, &accused).await,
        vec![valid.hash()],
        "after release the resident slash must keep the same id"
    );
    let again = admit(&w.node, &unknown).await;
    assert!(
        mempool_slash_msg(&again).is_some_and(|m| m.contains("not a registered producer")),
        "after release the state check runs again for the unregistered accused; got {again:?}"
    );
}
