//! INC-I-180 M2 / S1 — the builder's IN-BLOCK allowance terms.
//!
//! covers: production/withdrawal_holdings.rs, assembly.rs, validation_checks.rs,
//!         holdings.rs, withdrawal_holdings.rs, pool.rs
//!
//! ---------------------------------------------------------------------------
//! THE DEFECT THIS FILE REPRODUCES (QA round 1, ISSUE-001)
//! ---------------------------------------------------------------------------
//! The builder and the gate computed R1 from the SAME terms in a DIFFERENT
//! saturating order:
//!
//!   gate    : bond .sat_add(pending) .sat_add(in_block_add) .sat_sub(wp) .sat_sub(in_block_wd)
//!   builder : (bond .sat_add(pending) .sat_sub(wp)) .sat_add(in_block_add) .sat_sub(in_block_wd)
//!
//! `saturating_sub` does not commute with `saturating_add` across the clamp.
//! Subtracting `wp` first pins a negative intermediate at 0, so a later
//! `in_block_addbond` credit starts from 0 instead of from the true deficit and
//! the builder's allowance exceeds the gate's by `wp - (bond + pending)`. The
//! builder then selects a withdrawal its OWN gate rejects — INV-PROD-003
//! verbatim, and the free `rollback_one_block()` poison M2 exists to close.
//!
//! The `wp > bond + pending` ledger is reachable on chain, not synthetic:
//! apply's Exit arm does `withdrawal_pending_count += bond_count` against an
//! UNCHANGED `bond_count`, so two Exits for one producer charge it twice. This
//! file DERIVES that ledger by applying two real `Exit` transactions rather
//! than asserting it, so the reachability claim is executed, not stated.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT:
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   `Node::build_block_content(&mut self, Hash, u32, u64, u32, PublicKey)
//!        -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>`
//!   `Node::validate_block_economics(&self, &Block, u64, ValidationMode)
//!        -> Result<()>`
//!   `Mempool::add_transaction(&mut self, Transaction, &UtxoSet, BlockHeight)
//!        -> Result<AddTransactionResult, MempoolError>`
//!   `Node::process_transaction_producer_effects(...)` (ledger derivation only)
//!
//! Observable outputs:
//!   O1  the built transaction list — membership of the candidate withdrawal
//!       and of the in-block `AddBond`
//!   O2  the `Result` discriminant of `build_block_content`: INV-PROD-002
//!       forbids `Err` as a response to an unwanted transaction
//!   O3  `validate_block_economics(built_block)` at the SAME height — a block
//!       this node built must be one this node accepts
//!   O4  the admission verdict for the same transaction
//!   O5  `withdrawal_pending_count` after applying two `Exit`s (derivation)
//!   NOT outputs: no block is stored and no gossip is emitted by
//!   `build_block_content`.
//!
//! PATHS
//!   PB-POST  build at a height AT/ABOVE AH #23
//!   PB-PRE   build BELOW AH #23 (devnet gate = 20; PRE_AH = 5)
//!   PV       `validate_block_economics` on what was built, or hand-built
//!   PM       `Mempool::add_transaction`
//!   PA       `process_transaction_producer_effects` (Exit arm)
//!
//! INPUT PARTITIONS
//!   IP-SAT    `wp = 2 x bond_count` (DERIVED from two Exits) AND an in-block
//!             `AddBond` at a lower index. Gate allowance clamps to 0; the
//!             pre-fix builder read 10.
//!   IP-CREDIT `wp = bond_count` (one Exit, no clamp) AND an in-block `AddBond`
//!             that legitimately RAISES the allowance. The withdrawal must be
//!             SELECTED: a builder that simply drops `in_block_addbond` passes
//!             IP-SAT and fails here.
//!   IP-EXIT   an `Exit(P)` sharing the candidate set with a withdrawal for P.
//!   IP-CONT   `[AddBond(P,+n), RequestWithdrawal(P,d)]`, `d` above the FLUSHED
//!             allowance — the one shape admission rejects and the gate accepts.
//!   IP-LOWER  a plain over-holdings withdrawal, no allowance-raising in-block
//!             term — the containment half that DOES hold.
//!
//! MATRIX (every enumerated cell has an assertion)
//!   O5 x PA x IP-SAT                  -> inc_i180_m2_two_exits_charge_the_allowance_twice
//!   O1,O2,O3 x PB-POST,PV x IP-SAT    -> inc_i180_m2_builder_skips_when_the_allowance_clamps
//!   O3 x PV x IP-SAT                  -> inc_i180_m2_the_gate_rejects_the_clamped_withdrawal
//!   O1 x PB-PRE x IP-SAT              -> inc_i180_m2_pre_activation_still_selects_the_clamped_withdrawal
//!   O1,O2,O3 x PB-POST,PV x IP-CREDIT -> inc_i180_m2_builder_credits_the_in_block_addbond
//!   O1,O2,O3 x PB-POST,PV x IP-EXIT   -> inc_i180_m2_an_in_block_exit_never_splits_builder_from_gate
//!   O3,O4 x PM,PV x IP-CONT           -> inc_i180_m2_admission_over_rejects_the_addbond_window
//!   O4 x PM x IP-CONT                 -> inc_i180_m2_the_operator_resubmits_once_the_addbond_flushes
//!   O3,O4 x PM,PV x IP-LOWER          -> inc_i180_m2_admission_reject_implies_gate_reject_without_a_credit
//!
//! A bare `Exit` is not selectable by the builder at all: `TxType::Exit` is out
//! of the zero-flow exempt set, so `validate_transaction_with_utxos` refuses it
//! at the FIRST skip gate of the selection loop. IP-EXIT measures that and pins
//! the mechanism, so a change to `allows_empty_io` fails here instead of
//! diverging silently. The gate side is locked by `inc_i_180_gate_bindings.rs`.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, Output, Transaction, TxType};
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::Node;
use storage::{ProducerSet, UtxoSet};
use tempfile::TempDir;

use crate::inc_i_180_common::{
    block_with, bond_unit, build_ledger, seed_owned_bond_utxos, POST_AH, PRE_AH, SLOT,
};

// ──────────────────────────────────────────────────────────────── fixture

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn sign_input(tx: &mut Transaction, i: usize, kp: &KeyPair) {
    let m = tx.signing_message_for_input(i);
    tx.inputs[i].signature = crypto::signature::sign_hash(&m, kp.private_key());
}

fn outpoints(tag: u8, count: u32) -> Vec<(Hash, u32)> {
    let h = Hash::from_bytes([tag; 32]);
    (0..count).map(|i| (h, i)).collect()
}

fn signed_withdrawal(
    node: &Node,
    producer: &PublicKey,
    declared: u32,
    spends: &[((Hash, u32), &KeyPair)],
) -> Transaction {
    let unit = bond_unit(node);
    let inputs: Vec<Input> = spends
        .iter()
        .map(|((h, idx), owner)| {
            let mut inp = Input::new(*h, *idx);
            inp.public_key = Some(*owner.public_key());
            inp
        })
        .collect();
    let dest = crypto::hash::hash(b"inc-i-180-m2-in-block-destination");
    let net = unit * spends.len() as u64 - unit / 100;
    let mut tx = Transaction::new_request_withdrawal(inputs, *producer, declared, dest, net);
    for (i, (_, owner)) in spends.iter().enumerate() {
        sign_input(&mut tx, i, owner);
    }
    tx
}

/// A signature-valid `AddBond` creating `n` Bond outputs, funded by a Normal
/// UTXO seeded at `(tag, 0)`. Its fee rate exceeds any withdrawal here, so
/// `select_for_block` places it FIRST — which is what makes `in_block_addbond`
/// non-zero at the builder at all.
async fn funded_add_bond(node: &Node, kp: &KeyPair, n: u32, tag: u8) -> Transaction {
    let unit = bond_unit(node);
    let funding = Hash::from_bytes([tag; 32]);
    {
        let mut utxo = node.utxo_set.write().await;
        utxo.insert(
            storage::Outpoint::new(funding, 0),
            storage::UtxoEntry {
                output: Output::normal(unit * u64::from(n + 1), addr(kp.public_key())),
                height: 1,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: fund the AddBond");
    }
    let mut inp = Input::new(funding, 0);
    inp.public_key = Some(*kp.public_key());
    let mut tx = Transaction::new_add_bond(
        vec![inp],
        *kp.public_key(),
        n,
        unit * u64::from(n),
        u64::MAX,
    );
    sign_input(&mut tx, 0, kp);
    tx
}

/// Apply `exits` real `Exit(P)` transactions to a fresh `held`-bond ledger and
/// return what apply produced. Nothing here is hand-set: `withdrawal_pending`
/// is whatever `tx_processing.rs`'s Exit arm computed.
async fn ledger_after_exits(node: &Node, pk: &PublicKey, held: u32, exits: usize) -> ProducerSet {
    let mut applied = build_ledger(node, pk, held, 0);
    let utxo = UtxoSet::new();
    let mut dirty: HashSet<Hash> = HashSet::new();
    let mut regs: Vec<PublicKey> = Vec::new();
    for _ in 0..exits {
        node.process_transaction_producer_effects(
            &Transaction::new_exit(*pk),
            POST_AH,
            SLOT,
            &utxo,
            &mut applied,
            &mut dirty,
            &mut regs,
        );
    }
    applied
}

async fn install(node: &Node, set: ProducerSet) {
    *node.producer_set.write().await = set;
}

async fn build_at(node: &mut Node, kp: &KeyPair, height: u64) -> Block {
    let our = *kp.public_key();
    for _ in 0..12 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        let slot = node.params.timestamp_to_slot(now);
        let prev = slot.saturating_sub(1);
        let built = node
            .build_block_content(Hash::ZERO, prev, height, slot, our)
            .await
            .expect("O2/INV-PROD-002: build_block_content must never return Err");
        if let Some((header, txs, _bf)) = built {
            return Block::new(header, txs);
        }
    }
    panic!("fixture: 12 slot-boundary aborts at h={height}");
}

fn carries(block: &Block, tx: &Transaction) -> bool {
    let h = tx.hash();
    block.transactions.iter().any(|t| t.hash() == h)
}

/// One node holding an admitted `[AddBond(P,+add), RequestWithdrawal(P,declared)]`
/// pair, with the ledger then moved to whatever `exits` real Exits produce.
///
/// Admission runs against the PRE-Exit ledger on purpose: post-Exit the mempool
/// would refuse the withdrawal, and then no builder could ever be handed the
/// shape under test. A withdrawal admitted before the Exits confirm and still
/// resident afterwards is the ordinary mempool timeline.
struct Scenario {
    node: Node,
    kp: KeyPair,
    addbond: Transaction,
    withdrawal: Transaction,
    _temp: TempDir,
}

async fn scenario(held: u32, exits: usize, add: u32, declared: u32, owned: u32) -> Scenario {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    let pk = *kp.public_key();

    install(&node, build_ledger(&node, &pk, held, 0)).await;
    seed_owned_bond_utxos(&node, &pk, 0x70, owned).await;
    let addbond = funded_add_bond(&node, &kp, add, 0x71).await;
    let spends: Vec<((Hash, u32), &KeyPair)> = outpoints(0x70, declared.min(owned))
        .into_iter()
        .map(|o| (o, &kp))
        .collect();
    let withdrawal = signed_withdrawal(&node, &pk, declared, &spends);
    {
        let utxo = node.utxo_set.read().await;
        let mut mp = node.mempool.write().await;
        mp.add_transaction(addbond.clone(), &utxo, POST_AH)
            .expect("fixture: the AddBond must be admitted");
        mp.add_transaction(withdrawal.clone(), &utxo, POST_AH)
            .expect("fixture: the withdrawal must be admitted at the PRE-Exit ledger");
    }
    let moved = ledger_after_exits(&node, &pk, held, exits).await;
    install(&node, moved).await;

    Scenario {
        node,
        kp,
        addbond,
        withdrawal,
        _temp: temp,
    }
}

// ═════════════════════════════════════════════════════════════════════════
// IP-SAT — the clamp
// ═════════════════════════════════════════════════════════════════════════

/// O5 × PA × IP-SAT. Without this row the whole saturation partition rests on a
/// hand-written `withdrawal_pending`, and a reader cannot tell a real ledger
/// from a fixture convenience.
#[tokio::test]
async fn inc_i180_m2_two_exits_charge_the_allowance_twice() {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    let pk = *kp.public_key();

    let one = ledger_after_exits(&node, &pk, 12, 1).await;
    let two = ledger_after_exits(&node, &pk, 12, 2).await;

    let wp = |s: &ProducerSet| {
        s.get_by_pubkey(&pk)
            .expect("producer registered")
            .withdrawal_pending_count
    };
    assert_eq!(12, wp(&one), "O5: one Exit charges bond_count once");
    assert_eq!(
        24,
        wp(&two),
        "O5: apply's Exit arm re-reads an UNCHANGED bond_count and uses `+=`, so \
         two Exits in one epoch drive withdrawal_pending to 2 x bond_count. This \
         is what makes `wp > bond_count + pending_addbond` an on-chain ledger and \
         not a fixture invention."
    );
    assert_eq!(
        12,
        one.get_by_pubkey(&pk).expect("registered").bond_count,
        "O5: bond_count is untouched — the flush is deferred to the epoch boundary"
    );
}

/// O1,O2,O3 × PB-POST,PV × IP-SAT — **the ISSUE-001 regression.**
///
/// gate    = 12 .add(0) .add(10) .sub(24) .sub(0) = 0  -> 8 > 0   REJECT
/// pre-fix = (12 .add(0) .sub(24) = 0) .add(10) .sub(0) = 10 -> 8 <= 10  SELECT
#[tokio::test]
async fn inc_i180_m2_builder_skips_when_the_allowance_clamps() {
    let mut s = scenario(12, 2, 10, 8, 8).await;
    let block = build_at(&mut s.node, &s.kp, POST_AH).await;
    let verdict = s
        .node
        .validate_block_economics(&block, POST_AH, ValidationMode::Full)
        .await;

    assert!(
        verdict.is_ok(),
        "O3/INV-PROD-003: the builder assembled a block this same node's gate \
         rejects. The two layers must compute R1 through ONE function, in the \
         gate's saturating order: {verdict:?}"
    );
    assert!(
        !carries(&block, &s.withdrawal),
        "O1: with the gate allowance clamped to 0 the withdrawal must be SKIPPED"
    );
    assert!(
        carries(&block, &s.addbond),
        "O1: only the withdrawal is refused — the AddBond that raises the term is \
         a perfectly valid transaction and must still be selected"
    );
}

/// O3 × PV × IP-SAT. Harness self-check: the builder test above asserts the
/// withdrawal is absent, which a builder that skips EVERY withdrawal would also
/// satisfy. This row proves the gate really does reject the partition.
#[tokio::test]
async fn inc_i180_m2_the_gate_rejects_the_clamped_withdrawal() {
    let s = scenario(12, 2, 10, 8, 8).await;
    let pk = *s.kp.public_key();
    let block = block_with(
        &s.node,
        POST_AH,
        pk,
        vec![s.addbond.clone(), s.withdrawal.clone()],
    );
    let msg = s
        .node
        .validate_block_economics(&block, POST_AH, ValidationMode::Full)
        .await
        .map(|_| String::new())
        .unwrap_or_else(|e| e.to_string());

    assert!(
        msg.contains("[ECON_WITHDRAWAL_OVER_HOLDINGS]"),
        "O3: the hand-built block must be rejected for R1, or the builder row \
         above is vacuous; got {msg:?}"
    );
    assert!(
        msg.contains("allowance is 0"),
        "O3: and the gate's allowance must be the CLAMPED 0, not 10: {msg:?}"
    );
}

/// O1 × PB-PRE × IP-SAT. Below AH #23 the same ledger must not censor.
#[tokio::test]
async fn inc_i180_m2_pre_activation_still_selects_the_clamped_withdrawal() {
    let mut s = scenario(12, 2, 10, 8, 8).await;
    let block = build_at(&mut s.node, &s.kp, PRE_AH).await;
    assert!(
        carries(&block, &s.withdrawal),
        "O1: pre-AH censorship — below AH #23 the withdrawal is a valid \
         transaction and the predicate must be a strict no-op"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// IP-CREDIT — the in-block AddBond RAISES the allowance
// ═════════════════════════════════════════════════════════════════════════

/// O1,O2,O3 × PB-POST,PV × IP-CREDIT.
///
/// gate = 6 .add(0) .add(5) .sub(6) .sub(0) = 5 -> 4 <= 5, partial, 4 == 4 spent
/// A builder that drops `in_block_addbond` reads 0 and skips: this row is the
/// mutation-detector that keeps the ISSUE-001 fix from becoming an over-fix.
#[tokio::test]
async fn inc_i180_m2_builder_credits_the_in_block_addbond() {
    let mut s = scenario(6, 1, 5, 4, 4).await;
    let block = build_at(&mut s.node, &s.kp, POST_AH).await;
    let verdict = s
        .node
        .validate_block_economics(&block, POST_AH, ValidationMode::Full)
        .await;

    assert!(
        verdict.is_ok(),
        "O3/INV-PROD-003: the built block must satisfy this node's own gate: {verdict:?}"
    );
    assert!(
        carries(&block, &s.addbond),
        "O1: the AddBond outranks the withdrawal on fee rate and must be selected \
         first, or the in-block credit is never exercised"
    );
    assert!(
        carries(&block, &s.withdrawal),
        "O1: the gate accepts this withdrawal because the in-block AddBond raises \
         the allowance to 5. A builder that ignores the credit censors it."
    );
}

// ═════════════════════════════════════════════════════════════════════════
// IP-EXIT — an Exit sharing the candidate set
// ═════════════════════════════════════════════════════════════════════════

/// O1,O2,O3 × PB-POST,PV × IP-EXIT.
///
/// MEASURED, not assumed: a bare `Exit` is 0-in/0-out and `TxType::Exit` is
/// deliberately OUT of the zero-flow exempt set (`validation/utxo.rs`, C1 — the
/// set is curated by authorization and `ExitData` carries no signature), so
/// `validate_transaction_with_utxos` refuses it at `assembly.rs:259`, before the
/// withdrawal-holdings gate at `:324` ever runs. The mempool admits it through
/// `add_system_transaction`, the builder then drops it, and the builder's Exit
/// accounting is therefore unreachable through selection today.
///
/// The accounting stays in the builder because it must match the gate term for
/// term if `allows_empty_io` ever changes; the assertion below is what turns
/// that change into a test failure instead of a silent divergence. The gate side
/// of `[Exit(P), RequestWithdrawal(P,d)]` is locked by `inc_i_180_gate_bindings`.
#[tokio::test]
async fn inc_i180_m2_an_in_block_exit_never_splits_builder_from_gate() {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let mut node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    let pk = *kp.public_key();

    install(&node, build_ledger(&node, &pk, 4, 0)).await;
    seed_owned_bond_utxos(&node, &pk, 0x80, 4).await;
    let spends: Vec<((Hash, u32), &KeyPair)> =
        outpoints(0x80, 2).into_iter().map(|o| (o, &kp)).collect();
    let withdrawal = signed_withdrawal(&node, &pk, 2, &spends);
    let exit = Transaction::new_exit(pk);
    {
        let utxo = node.utxo_set.read().await;
        let mut mp = node.mempool.write().await;
        mp.add_transaction(withdrawal.clone(), &utxo, POST_AH)
            .expect("fixture: partial withdrawal admitted against allowance 4");
        mp.add_system_transaction(exit.clone(), POST_AH)
            .expect("fixture: a zero-flow Exit routes through add_system_transaction");
    }

    let block = build_at(&mut node, &kp, POST_AH).await;
    let verdict = node
        .validate_block_economics(&block, POST_AH, ValidationMode::Full)
        .await;

    assert!(
        verdict.is_ok(),
        "O3/INV-PROD-003: an Exit in the candidate set must not desynchronise the \
         builder's in-block accounting from the gate's: {verdict:?}"
    );
    assert!(
        !TxType::Exit.allows_empty_io(),
        "O1 mechanism: `Exit` is OUT of the zero-flow exempt set, which is why \
         the builder drops it below. If this ever flips, the builder's Exit \
         charge becomes reachable through selection and needs a partition that \
         drives `[Exit(P), RequestWithdrawal(P,d)]` end to end."
    );
    assert!(
        !carries(&block, &exit),
        "O1: a bare Exit fails `validate_transaction_with_utxos` at the FIRST \
         skip gate of the selection loop, before the withdrawal-holdings gate"
    );
    assert!(
        carries(&block, &withdrawal),
        "O1: and an unminable Exit sitting in the mempool must not cost the \
         withdrawal its slot — the two skips are independent"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// IP-CONT / IP-LOWER — what containment actually is (QA round 1, ISSUE-002)
// ═════════════════════════════════════════════════════════════════════════

/// O3,O4 × PM,PV × IP-CONT. `mempool-reject ⊆ builder-skip` is FALSE, and this
/// row drives BOTH layers to pin the one shape that breaks it rather than
/// comparing admission against a model of admission.
///
/// Admission substitutes mempool-wide state for the block's terms, so it
/// over-rejects whenever the substitute raises the block's allowance. This row
/// is the `in_block_addbond → 0` instance of that rule.
#[tokio::test]
async fn inc_i180_m2_admission_over_rejects_the_addbond_window() {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    let pk = *kp.public_key();

    install(&node, build_ledger(&node, &pk, 1, 0)).await;
    seed_owned_bond_utxos(&node, &pk, 0x60, 4).await;
    let addbond = funded_add_bond(&node, &kp, 5, 0x61).await;
    let spends: Vec<((Hash, u32), &KeyPair)> =
        outpoints(0x60, 4).into_iter().map(|o| (o, &kp)).collect();
    let withdrawal = signed_withdrawal(&node, &pk, 4, &spends);

    let admitted = {
        let utxo = node.utxo_set.read().await;
        let mut mp = node.mempool.write().await;
        mp.add_transaction(addbond.clone(), &utxo, POST_AH)
            .expect("fixture: the AddBond itself is admissible");
        mp.add_transaction(withdrawal.clone(), &utxo, POST_AH)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    let block = block_with(&node, POST_AH, pk, vec![addbond, withdrawal]);
    let gate = node
        .validate_block_economics(&block, POST_AH, ValidationMode::Full)
        .await;

    let msg = admitted.expect_err(
        "O4: admission evaluates the rule table with in_block_addbond at ZERO, so \
         a withdrawal covered only by an in-block AddBond reads as over-holdings",
    );
    assert!(
        msg.contains("[ECON_WITHDRAWAL_OVER_HOLDINGS]"),
        "O4: and it must be the allowance rule that refuses it: {msg:?}"
    );
    assert!(
        gate.is_ok(),
        "O3: while a real block carrying the same pair is LEGAL — which is what \
         makes this over-rejection and not containment: {gate:?}"
    );
}

/// O4 × PM × IP-CONT. The consequence the docs now name: bounded censorship,
/// not poison. Once the AddBond confirms, `pending_addbond_count` covers the
/// resubmission.
#[tokio::test]
async fn inc_i180_m2_the_operator_resubmits_once_the_addbond_flushes() {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    let pk = *kp.public_key();

    install(&node, build_ledger(&node, &pk, 1, 5)).await;
    seed_owned_bond_utxos(&node, &pk, 0x62, 4).await;
    let spends: Vec<((Hash, u32), &KeyPair)> =
        outpoints(0x62, 4).into_iter().map(|o| (o, &kp)).collect();
    let withdrawal = signed_withdrawal(&node, &pk, 4, &spends);

    let admitted = {
        let utxo = node.utxo_set.read().await;
        let mut mp = node.mempool.write().await;
        mp.add_transaction(withdrawal, &utxo, POST_AH)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    assert!(
        admitted.is_ok(),
        "O4: with the AddBond mined and queued, pending_addbond_count = 5 lifts \
         the admission allowance to 6 and the resubmission goes through. The \
         over-rejection above lasts exactly one confirmation: {admitted:?}"
    );
}

/// O3,O4 × PM,PV × IP-LOWER. The half of the relation that DOES hold: with no
/// allowance-raising in-block term, admission-reject implies gate-reject. Both
/// layers are driven; neither is a model of the other.
#[tokio::test]
async fn inc_i180_m2_admission_reject_implies_gate_reject_without_a_credit() {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    let pk = *kp.public_key();

    install(&node, build_ledger(&node, &pk, 4, 0)).await;
    seed_owned_bond_utxos(&node, &pk, 0x63, 6).await;
    let spends: Vec<((Hash, u32), &KeyPair)> =
        outpoints(0x63, 5).into_iter().map(|o| (o, &kp)).collect();
    let withdrawal = signed_withdrawal(&node, &pk, 5, &spends);

    let admitted = {
        let utxo = node.utxo_set.read().await;
        let mut mp = node.mempool.write().await;
        mp.add_transaction(withdrawal.clone(), &utxo, POST_AH)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    let block = block_with(&node, POST_AH, pk, vec![withdrawal]);
    let gate = node
        .validate_block_economics(&block, POST_AH, ValidationMode::Full)
        .await;

    assert!(
        admitted.is_err(),
        "O4: declared 5 against a flushed allowance of 4 must be refused at \
         admission: {admitted:?}"
    );
    assert!(
        gate.is_err(),
        "O3: and the gate must refuse it too. Every in-block term available to \
         the gate here can only LOWER the allowance, so admission cannot be \
         stricter than the block rule for this shape: {gate:?}"
    );
    assert_eq!(
        TxType::RequestWithdrawal,
        block.transactions[1].tx_type,
        "O3: guard against a fixture that stopped carrying the withdrawal"
    );
}
