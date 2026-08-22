//! INC-I-180 M2 / reviewer F5 (QA OBS-006) — the empty-snapshot early return in
//! `HoldingsSources::lookup` is the branch that decides fail-open vs
//! fail-closed, and nothing in the tree drove it.
//!
//! covers: crates/mempool/src/holdings.rs (`HoldingsSources::lookup`)
//!
//! ---------------------------------------------------------------------------
//! WHY THIS FILE EXISTS
//! ---------------------------------------------------------------------------
//! `Node::new` seeds the published holdings snapshot at construction
//! (`init.rs:740-752`); `Node::new_for_test` (`init.rs:1198`) and
//! `Node::new_for_replay` (`init.rs:1415`) do not. Without the `is_empty()`
//! arm, a write-contended `try_read()` on the live `ProducerSet` under those
//! two constructors makes absence from an EMPTY snapshot read as `Unregistered`
//! — so every producer is refused at admission. `new_for_replay` backs the
//! operator reindex tool, so that is fail-CLOSED censorship on a real path.
//!
//! Both legs offer the SAME transaction to the SAME node at the SAME height.
//! Only the availability of the live handle differs, which is what makes the
//! admitting leg non-vacuous: the contrast leg proves any source that ANSWERS
//! refuses this transaction outright.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT:
//! ---------------------------------------------------------------------------
//! Function under test:
//!   `Mempool::add_transaction(Transaction, &UtxoSet, BlockHeight)
//!        -> Result<Vec<Hash>, MempoolError>`
//!   reached through `Mempool::withdrawal_holdings_verdict` ->
//!   `HoldingsSources::lookup`, which is `pub(crate)` and not callable directly.
//!
//! Observable outputs:
//!   O1  the `Result` discriminant of `add_transaction`
//!   O2  the bracketed error code when it is `Err`
//!   NOT outputs: no block is built, no producer state is mutated.
//!
//! PATHS
//!   PL-LIVE  `lookup` resolves through the live handle (`try_read` succeeds)
//!   PL-EMPTY `try_read` fails, the snapshot is wired and EMPTY -> `Unavailable`
//!
//! INPUT PARTITIONS
//!   IP-OVER  one `RequestWithdrawal` declaring 99 bonds against a ledger
//!            holding 1 — refused by any source that answers
//!
//! MATRIX
//!   O1,O2 x PL-LIVE  x IP-OVER -> inc_i180_m2_f5_empty_snapshot_admits_when_the_live_handle_is_contended
//!   O1    x PL-EMPTY x IP-OVER -> inc_i180_m2_f5_empty_snapshot_admits_when_the_live_handle_is_contended

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, Transaction};
use doli_node::node::Node;

use crate::inc_i_180_common::{bond_unit, build_ledger, make_node, seed_owned_bond_utxos, POST_AH};

/// Flushed bonds in the ledger. The declared count is far above it, so the R1
/// allowance refuses the transaction on any source that answers.
const HELD: u32 = 1;
const DECLARED: u32 = 99;
const TAG: u8 = 0x7E;

/// A `RequestWithdrawal` spending `DECLARED` Bond UTXOs owned by `kp`, each
/// input signed by that key so admission reaches the holdings verdict.
fn signed_withdrawal(node: &Node, kp: &KeyPair, producer: &PublicKey) -> Transaction {
    let unit = bond_unit(node);
    let h = Hash::from_bytes([TAG; 32]);
    let inputs: Vec<Input> = (0..DECLARED)
        .map(|i| {
            let mut inp = Input::new(h, i);
            inp.public_key = Some(*kp.public_key());
            inp
        })
        .collect();
    let dest = crypto::hash::hash(b"inc-i-180-m2-f5-destination");
    let net = unit * DECLARED as u64 - unit / 100;
    let mut tx = Transaction::new_request_withdrawal(inputs, *producer, DECLARED, dest, net);
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
    }
    tx
}

async fn offer(node: &Node, tx: &Transaction) -> Result<(), String> {
    let utxo = node.utxo_set.read().await;
    let mut mempool = node.mempool.write().await;
    mempool
        .add_transaction(tx.clone(), &utxo, POST_AH)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// O1,O2 × PL-LIVE,PL-EMPTY × IP-OVER — **GREEN today, must STAY green.**
///
/// RED if the `is_empty()` arm is removed: the second leg then resolves
/// `Unregistered` out of the empty snapshot and refuses with
/// `[ECON_WITHDRAWAL_UNKNOWN_PRODUCER]`.
#[tokio::test]
async fn inc_i180_m2_f5_empty_snapshot_admits_when_the_live_handle_is_contended() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    {
        let mut guard = node.producer_set.write().await;
        *guard = build_ledger(&node, &pk, HELD, 0);
    }
    seed_owned_bond_utxos(&node, &pk, TAG, DECLARED).await;
    let tx = signed_withdrawal(&node, &kp, &pk);

    // PL-LIVE — the live handle is free, so a source answers and refuses.
    let answered = offer(&node, &tx).await;
    let err = answered.expect_err(
        "harness: a withdrawal declaring 99 bonds against 1 held must be refused \
         while the live handle answers, otherwise the admitting leg below proves \
         nothing about WHICH source answered",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_OVER_HOLDINGS]"),
        "harness: expected the R1 code from the live handle, got {err}"
    );

    // PL-EMPTY — the same transaction, with the live handle write-held. Under
    // `new_for_test` the published snapshot is never seeded, so it is wired and
    // empty: no source answers and admission must NOT refuse.
    let contended = node.producer_set.write().await;
    let verdict = offer(&node, &tx).await;
    drop(contended);

    assert!(
        verdict.is_ok(),
        "FAIL-CLOSED: with the live handle contended and the published snapshot \
         empty, no source can answer, so admission must fall open. Refusing here \
         censors EVERY producer under write contention on any node built by \
         `new_for_test` / `new_for_replay` (the operator reindex path). got: {}",
        verdict.unwrap_err()
    );
}
