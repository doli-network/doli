//! INC-I-180 — the n11 defect executed through the REAL `apply_block` path.
//!
//! covers: apply_block/mod.rs, apply_block/tx_processing.rs, validation_checks.rs,
//!         crates/storage/src/utxo/set.rs
//!
//! ---------------------------------------------------------------------------
//! THE INFERENCE THIS FILE CONVERTS INTO AN EXECUTED ASSERTION
//! ---------------------------------------------------------------------------
//! Every other pre-activation test in this suite proves only the LEDGER half of
//! the n11 shape: the ProducerSet keeps its weight. The UTXO half — "and the
//! Bond UTXOs are destroyed" — was never executed. The shared harness
//! `inc_i_180_common::run_block_case_unseeded` (:313-373) calls ONLY
//! `process_transaction_producer_effects`, against a throwaway
//! `UtxoSet::new()` (:334). It never calls `apply_block`, so it never reaches
//! `process_transaction_utxos` and never spends anything. The destruction half
//! is documented there as inferred observable "O3"
//! (`inc_i_180_withdrawal_holdings_gate.rs:69-72`): "validation Ok ⇒ the Bond
//! UTXOs WILL be spent".
//!
//! This file removes the "⇒". It drives the real `Node::apply_block` and then
//! reads the real `UtxoSet` back, so BOTH halves of the defect are observed in
//! one execution:
//!
//!   pre-AH  : apply succeeds, Bond UTXOs GONE, selection weight RETAINED
//!             → unbacked producer weight, collateral destroyed  (the defect)
//!   post-AH : apply REJECTS, and NOT ONE Bond UTXO is spent
//!             → the anti-theft property, also previously unexecuted at this layer
//!
//! ---------------------------------------------------------------------------
//! WHY THE UTXO BACKEND IS SWAPPED FIRST
//! ---------------------------------------------------------------------------
//! Three views of the UTXO set must agree or this test proves nothing:
//!   * the gate reads `self.utxo_set`            (`validation_checks.rs:635`)
//!   * the spend path reads the `BlockBatch`     (`apply_block/mod.rs:148`)
//!   * writes land in `state_db`
//!
//! A node built by `Node::new_for_test` carries an in-memory `utxo_set` that the
//! spend path cannot see, so a bond seeded there yields `OutputNotFound` instead
//! of a spend. `UtxoSet::from_state_db` (`crates/storage/src/utxo/set.rs:46`)
//! routes inserts straight into RocksDB, which the batch reads through — one
//! swap and all three views agree.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT:
//! ---------------------------------------------------------------------------
//! Function under test:
//!   `Node::apply_block(&mut self, Block, ValidationMode) -> Result<()>`
//!   (`bins/node/src/node/apply_block/mod.rs:42`). Height is derived as
//!   `chain_state.best_height + 1` (:52), so the fixture parks the tip one below.
//!
//! Observables:
//!   O1 — the apply verdict (Ok, or Err carrying a rejection code).
//!   O2 — live Bond UTXO count for the owner, via `UtxoSet::count_bonds`.
//!   O3 — per-outpoint survival of each seeded Bond UTXO, via `UtxoSet::get`.
//!   O4 — ProducerSet `bond_count` / `status` / `selection_weight` /
//!        `withdrawal_pending_count`.
//!   O5 — queued `PendingProducerUpdate::RequestWithdrawal` count.
//!
//! ---------------------------------------------------------------------------
//! INPUT PARTITIONS:
//! ---------------------------------------------------------------------------
//! The ledger is held fixed at the n11 relationship `U(3) > P(2)`, declared
//! count == owned count == 3, allowance == 2. The partitioning variable is
//! block height relative to the devnet gate (pinned 20,
//! `network_params/defaults.rs:679`), because that is the only input that
//! changes which mathematical relationship the assertions hold:
//!
//!   P1 — height BELOW the gate (`APPLY_PRE_AH` = 1).
//!        `declared > allowance` is NOT evaluated; the block applies. Spend
//!        precedes the ledger effect, and the enqueue guard
//!        (`tx_processing.rs:442`) fails, so O2/O3 go to zero while O4 is
//!        unchanged. Relationship: `utxo_after == 0 AND weight_after == P`.
//!        → `req_i180_003_pre_ah_apply_block_destroys_bonds_and_keeps_weight`
//!
//!   P2 — height AT/ABOVE the gate (`APPLY_POST_AH` = 25, deliberately not a
//!        multiple of devnet `blocks_per_reward_epoch = 4`, so no epoch-boundary
//!        side effects confound the result). `declared > allowance` is evaluated
//!        and rejects at `apply_block/mod.rs:113`, ahead of the spend at :202.
//!        Relationship: `utxo_after == utxo_before AND weight_after == P`.
//!        → `req_i180_003_post_ah_apply_block_rejects_and_spends_nothing`
//!
//!   P3 — the fixture itself (degenerate partition, no apply). Guards against a
//!        seeder that stops emitting Bond-typed outputs, under which P1 and P2
//!        would both pass VACUOUSLY (0 bonds before and after in P1; equal
//!        non-Bond counts in P2). Relationship: `utxo_before == 3 AND all Bond`.
//!        → `req_i180_003_fixture_actually_seeds_bond_utxos`
//!
//! Partitions deliberately NOT taken: `declared < allowance` and
//! `declared == allowance` are the partial/full-drain shapes, already covered by
//! `inc_i_180_drain_everything.rs`; the delegation term is covered by
//! `inc_i_180_allowance_parity.rs`. This file exists only for the
//! spend-vs-ledger interaction those files cannot observe.

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::validation::ValidationMode;
use doli_core::{Input, Transaction};
use storage::{Outpoint, ProducerStatus, UtxoSet};

use doli_node::node::Node;

use crate::inc_i_180_common::{
    block_with, bond_unit, build_ledger, make_node, queued_withdrawal_count, snapshot,
};

// The suite-wide `PRE_AH`/`POST_AH` constants are tuned for callers that invoke
// `validate_block_economics` in isolation. `apply_block` runs the FULL pipeline,
// which adds two constraints they do not satisfy:
//
//   * `POST_AH = 1_000_007` is far past the emission decay, so
//     `params.block_reward(height)` is 0 there and the fixture coinbase trips
//     `[ERRTX003] output 0 has zero amount` inside `validate_block_for_apply`,
//     BEFORE the holdings gate is ever consulted.
//   * `PRE_AH = 5` makes `update_height_index` walk back from the new block via
//     `prev_hash` (`block_store/writes.rs:162`). The fixture parent is
//     `Hash::ZERO` and the store is empty, so the walk dies at
//     `[STOR020] header 000…0 missing during chain walk`. The loop only
//     terminates on `height == 0` (:149), which is reached from height 1.
//
// Hence the two local heights below. Both keep the devnet gate (20) on the
// correct side, which is the only property the partitioning depends on.

/// Below the devnet gate (20) AND low enough that the height-index chain walk
/// terminates at `height == 0` instead of chasing a missing parent header.
const APPLY_PRE_AH: u64 = 1;

/// Above the devnet gate (20), inside the live emission band so the coinbase is
/// non-zero, and not a multiple of devnet `blocks_per_reward_epoch = 4`.
/// Storage is never reached here — the block is rejected at
/// `apply_block/mod.rs:113`, before `process_transaction_utxos` at :202.
const APPLY_POST_AH: u64 = 25;

// ──────────────────────────────────────────────────────────────── fixture

/// The ledger half: the ProducerSet believes this producer holds 2 bonds.
const LEDGER_BONDS: u32 = 2;
/// The UTXO half: 3 Bond UTXOs actually exist on chain. `U(3) > P(2)`.
const OWNED_BOND_UTXOS: u32 = 3;
/// Outpoint tag for the seeded bonds. No other fixture in this file uses it.
const BOND_TAG: u8 = 0xD1;

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn sign_input(tx: &mut Transaction, i: usize, kp: &KeyPair) {
    let m = tx.signing_message_for_input(i);
    tx.inputs[i].signature = crypto::signature::sign_hash(&m, kp.private_key());
}

fn bond_outpoints(count: u32) -> Vec<Outpoint> {
    let h = Hash::from_bytes([BOND_TAG; 32]);
    (0..count).map(|i| Outpoint::new(h, i)).collect()
}

/// Repoint the node's `utxo_set` at its own `state_db` so the gate, the spend
/// path and this test's assertions all read one store. MUST run before seeding.
async fn back_utxos_with_state_db(node: &Node) {
    let mut guard = node.utxo_set.write().await;
    *guard = UtxoSet::from_state_db(node.state_db.clone());
}

/// Seed `count` live, owned, unspent Bond UTXOs through the state-db-backed set.
async fn seed_bonds(node: &Node, owner: &PublicKey, count: u32) {
    let pkh = addr(owner);
    let unit = bond_unit(node);
    let mut utxo = node.utxo_set.write().await;
    for op in bond_outpoints(count) {
        utxo.insert(
            op,
            storage::UtxoEntry {
                output: doli_core::Output::bond(unit, pkh, u64::MAX, 0),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: seed unspent Bond UTXO");
    }
}

/// `apply_block` derives `height = best_height + 1`, so park the tip one below.
async fn park_tip_below(node: &Node, target_height: u64) {
    let mut cs = node.chain_state.write().await;
    cs.best_height = target_height - 1;
}

/// A signature-valid `RequestWithdrawal` declaring `declared` bonds and spending
/// every seeded Bond UTXO. The fee is taken out of the bond value, so no extra
/// funding UTXO is needed (`RequestWithdrawal` is NOT fee-exempt —
/// `crates/core/src/validation/utxo.rs:292-296`).
fn signed_withdrawal(node: &Node, kp: &KeyPair, declared: u32, spends: u32) -> Transaction {
    let unit = bond_unit(node);
    let inputs: Vec<Input> = bond_outpoints(spends)
        .into_iter()
        .map(|op| {
            let mut inp = Input::new(op.tx_hash, op.index);
            inp.public_key = Some(*kp.public_key());
            inp
        })
        .collect();
    let dest = crypto::hash::hash(b"inc-i-180-apply-block-destination");
    let net = unit * u64::from(spends) - unit / 100;
    let mut tx = Transaction::new_request_withdrawal(inputs, *kp.public_key(), declared, dest, net);
    for i in 0..tx.inputs.len() {
        sign_input(&mut tx, i, kp);
    }
    tx
}

/// Build the whole `U > P` world at `height` and apply one block carrying the
/// withdrawal. Returns the apply verdict.
async fn apply_n11_shape(node: &mut Node, kp: &KeyPair, height: u64) -> Result<(), String> {
    let pk = *kp.public_key();

    back_utxos_with_state_db(node).await;
    seed_bonds(node, &pk, OWNED_BOND_UTXOS).await;
    {
        let mut guard = node.producer_set.write().await;
        *guard = build_ledger(node, &pk, LEDGER_BONDS, 0);
    }
    park_tip_below(node, height).await;

    // Precondition: the defect is only expressible while U > P.
    {
        let utxo = node.utxo_set.read().await;
        assert_eq!(
            utxo.count_bonds(&addr(&pk), bond_unit(node)),
            OWNED_BOND_UTXOS,
            "fixture: {} Bond UTXOs must be live before the block",
            OWNED_BOND_UTXOS
        );
    }

    let tx = signed_withdrawal(node, kp, OWNED_BOND_UTXOS, OWNED_BOND_UTXOS);
    let block = block_with(node, height, pk, vec![tx]);

    node.apply_block(block, ValidationMode::Light)
        .await
        .map_err(|e| e.to_string())
}

/// Live Bond UTXOs for `pk`, plus how many of the seeded outpoints survive.
async fn bonds_after(node: &Node, pk: &PublicKey) -> (u32, usize) {
    let utxo = node.utxo_set.read().await;
    let count = utxo.count_bonds(&addr(pk), bond_unit(node));
    let alive = bond_outpoints(OWNED_BOND_UTXOS)
        .into_iter()
        .filter(|op| utxo.get(op).is_some())
        .count();
    (count, alive)
}

// ──────────────────────────────────────────────────────────── P1: pre-activation

/// P1 — O1 × O2 × O3 × O4 × O5: the n11 defect, executed end-to-end.
///
/// BELOW the gate the block is accepted, the Bond UTXOs are really destroyed on
/// disk, and the producer really keeps its weight. That combination IS unbacked
/// producer weight. This is the assertion the shared harness cannot make,
/// because it never spends anything.
#[tokio::test]
async fn req_i180_003_pre_ah_apply_block_destroys_bonds_and_keeps_weight() {
    let (mut node, kp, _tmp) = make_node().await;
    let pk = *kp.public_key();

    let verdict = apply_n11_shape(&mut node, &kp, APPLY_PRE_AH).await;

    // O1: pre-activation the block is ACCEPTED — the legacy verdict.
    assert!(
        verdict.is_ok(),
        "O1: below the gate the n11 block must still apply (legacy behaviour), got {:?}",
        verdict
    );

    // O2 + O3: the collateral is GONE. This is the half that was only inferred.
    let (live_bonds, alive_outpoints) = bonds_after(&node, &pk).await;
    assert_eq!(
        live_bonds, 0,
        "O2: every Bond UTXO must be spent — this is the destruction half of the \
         n11 defect, and it is now EXECUTED, not inferred"
    );
    assert_eq!(
        alive_outpoints, 0,
        "O3: no seeded Bond outpoint may survive the apply"
    );

    // O4: yet the ProducerSet still pays this producer for bonds it no longer has.
    let ps = node.producer_set.read().await;
    let (bond_count, status, weight, withdrawal_pending) = snapshot(&ps, &pk);
    assert_eq!(
        bond_count, LEDGER_BONDS,
        "O4: the ledger keeps its bonds — the withdrawal was silently skipped \
         (tx_processing.rs:442 enqueue guard)"
    );
    assert_eq!(
        status,
        ProducerStatus::Active,
        "O4: the producer stays Active"
    );
    assert_eq!(
        weight, LEDGER_BONDS as u64,
        "O4: UNBACKED WEIGHT — {} weight units backed by 0 Bond UTXOs",
        LEDGER_BONDS
    );
    assert_eq!(
        withdrawal_pending, 0,
        "O4: no withdrawal was queued, so nothing will ever reconcile this"
    );

    // O5: and nothing is pending that would fix it at the epoch boundary.
    assert_eq!(
        queued_withdrawal_count(&ps, &pk),
        0,
        "O5: the shortfall path skips the enqueue entirely"
    );
}

// ─────────────────────────────────────────────────────────── P2: post-activation

/// P2 — O1 × O2 × O3 × O4: the anti-theft property at the apply layer.
///
/// AT/ABOVE the gate the same inputs are rejected BEFORE any mutation, so not
/// one Bond UTXO is spent. `validate_block_economics` runs at
/// `apply_block/mod.rs:113`, ahead of `process_transaction_utxos` at :202.
#[tokio::test]
async fn req_i180_003_post_ah_apply_block_rejects_and_spends_nothing() {
    let (mut node, kp, _tmp) = make_node().await;
    let pk = *kp.public_key();

    let verdict = apply_n11_shape(&mut node, &kp, APPLY_POST_AH).await;

    // O1: rejected, with the dedicated code.
    let err = verdict.expect_err("O1: above the gate the n11 block must be rejected");
    assert!(
        err.contains("ECON_WITHDRAWAL_OVER_HOLDINGS"),
        "O1: expected [ECON_WITHDRAWAL_OVER_HOLDINGS] (declared {} > allowance {}), got: {}",
        OWNED_BOND_UTXOS,
        LEDGER_BONDS,
        err
    );

    // O2 + O3: NOTHING was spent — rejection precedes mutation.
    let (live_bonds, alive_outpoints) = bonds_after(&node, &pk).await;
    assert_eq!(
        live_bonds, OWNED_BOND_UTXOS,
        "O2: a rejected block must not consume collateral — this is the \
         anti-theft property"
    );
    assert_eq!(
        alive_outpoints, OWNED_BOND_UTXOS as usize,
        "O3: every seeded Bond outpoint must survive a rejected block"
    );

    // O4: the producer is untouched.
    let ps = node.producer_set.read().await;
    let (bond_count, status, weight, withdrawal_pending) = snapshot(&ps, &pk);
    assert_eq!(bond_count, LEDGER_BONDS, "O4: ledger untouched");
    assert_eq!(status, ProducerStatus::Active, "O4: still Active");
    assert_eq!(weight, LEDGER_BONDS as u64, "O4: weight untouched");
    assert_eq!(withdrawal_pending, 0, "O4: nothing queued");
}

// ─────────────────────────────────────────────────────────────── P3: fixture

/// P3 — O2: guard against a fixture that silently stops exercising the Bond
/// path. If `seed_bonds` ever stopped producing Bond-typed outputs, P1 and P2
/// would both pass vacuously.
#[tokio::test]
async fn req_i180_003_fixture_actually_seeds_bond_utxos() {
    let (node, kp, _tmp) = make_node().await;
    let pk = *kp.public_key();

    back_utxos_with_state_db(&node).await;
    seed_bonds(&node, &pk, OWNED_BOND_UTXOS).await;

    let utxo = node.utxo_set.read().await;
    assert_eq!(
        utxo.count_bonds(&addr(&pk), bond_unit(&node)),
        OWNED_BOND_UTXOS,
        "the fixture must really create Bond-typed outputs, or both tests above \
         pass vacuously"
    );
    for op in bond_outpoints(OWNED_BOND_UTXOS) {
        let entry = utxo.get(&op).expect("seeded outpoint must exist");
        assert!(
            crate::inc_i_180_common::is_bond(&entry.output),
            "seeded output must be Bond-typed"
        );
    }
}
