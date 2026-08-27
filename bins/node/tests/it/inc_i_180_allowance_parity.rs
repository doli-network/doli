//! INC-I-180 M2 / QA round 2 (ISSUE-003) — the gate's R1 expression is locked
//! to `ProducerHoldings::allowance_with`.
//!
//! covers: validation_checks.rs (R1), holdings.rs
//!
//! ---------------------------------------------------------------------------
//! WHY THIS FILE EXISTS
//! ---------------------------------------------------------------------------
//! The builder and the mempool call `ProducerHoldings::allowance_with`. The
//! consensus gate does NOT: it holds a second, inline transcription of the same
//! five terms at `validation_checks.rs`, and routing it through the mempool
//! crate would make consensus validation depend on the mempool. So the safety
//! property is agreement between two expressions, and agreement decays unless
//! something asserts it. QA measured the existing protection: drifting the
//! gate's transcription into the pre-fix order turned exactly ONE row red, and
//! that row is a harness self-check, not a parity assertion.
//!
//! Each row here drives the REAL gate and reads back the allowance the gate
//! itself reports, then requires it to equal `allowance_with` evaluated on the
//! terms the gate's own message echoes. That is two-sided: a gate allowance
//! BELOW the shared function reports a different number, and one ABOVE it does
//! not reject the probe at all.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT:
//! ---------------------------------------------------------------------------
//! Function under test:
//!   `Node::validate_block_economics(&self, &Block, u64, ValidationMode)
//!        -> Result<()>`
//! Reference under test:
//!   `mempool::holdings::ProducerHoldings::allowance_with(u32, u32) -> u32`
//!
//! Observable outputs:
//!   O1  the `Err` discriminant of `validate_block_economics`
//!   O2  the allowance the gate REPORTS in `[ECON_WITHDRAWAL_OVER_HOLDINGS]`
//!   O3  the five terms the same message echoes (`held`, `pending_addbond`,
//!       `in_block_addbond`, `withdrawal_pending`, `in_block_withdrawn`)
//!   NOT outputs: no block is stored, no producer state is mutated by the gate.
//!
//! PATHS
//!   PV-R1  `validate_block_economics` bails at R1 (every probe declares
//!          `allowance + 1`, so R2/R3/R4 are never reached)
//!
//! INPUT PARTITIONS (one table row each unless noted)
//!   IP-ZERO    allowance lands exactly on 0 with no in-block term
//!   IP-DEFICIT `withdrawal_pending > bond_count + pending_addbond`, NO
//!              in-block `AddBond` — the lower clamp on its own
//!   IP-DEFICIT-CREDIT the same deficit WITH a lower-index same-producer
//!              `AddBond` — the ISSUE-001 shape, and the only row that turns
//!              red when the gate re-orders its terms
//!   IP-BOTH    both in-block terms non-zero and neither clamp engaged
//!   IP-PENDING credit carried entirely by `pending_addbond`
//!   IP-EXIT    `in_block_withdrawn` charged by an in-block `Exit`
//!   IP-DEBIT-CLAMP  ledger debit plus in-block debit drive the result below 0
//!   IP-CEILING `bond_count + pending_addbond + in_block_addbond` saturates at
//!              `u32::MAX` (own test — see its doc comment for why the ledger
//!              is written directly there and derived everywhere else)
//!
//! MATRIX
//!   O1,O2,O3 x PV-R1 x {IP-ZERO, IP-DEFICIT, IP-DEFICIT-CREDIT, IP-BOTH,
//!                       IP-PENDING, IP-EXIT, IP-DEBIT-CLAMP}
//!        -> inc_i180_m2_the_gate_allowance_equals_the_shared_function
//!   O1,O2,O3 x PV-R1 x IP-CEILING
//!        -> inc_i180_m2_the_gate_allowance_equals_the_shared_function_at_the_ceiling

use std::collections::HashSet;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::Block;
use doli_node::node::Node;
use mempool::holdings::{of_producer_set, HoldingsLookup, ProducerHoldings};
use storage::{ProducerSet, UtxoSet};

use crate::inc_i_180_common::{
    add_bond_tx, block_with, build_ledger, exit_tx, make_node, seed_bond_utxos, withdrawal_tx,
    withdrawal_tx_with_inputs, POST_AH, SLOT,
};

/// One (ledger, block) shape. `withdrawal_pending` is never written here: it is
/// whatever `process_transaction_producer_effects` produces from `ledger_exits`
/// real `Exit`s and one `ledger_withdrawal` of that size, which is how the chain
/// reaches those ledgers (`tx_processing.rs`, Exit and RequestWithdrawal arms).
struct Row {
    name: &'static str,
    bond_count: u32,
    pending_addbond: u32,
    ledger_exits: usize,
    ledger_withdrawal: u32,
    in_block_addbond: u32,
    in_block_withdrawal: u32,
    in_block_exit: bool,
}

const ROWS: &[Row] = &[
    Row {
        name: "IP-ZERO",
        bond_count: 4,
        pending_addbond: 0,
        ledger_exits: 0,
        ledger_withdrawal: 4,
        in_block_addbond: 0,
        in_block_withdrawal: 0,
        in_block_exit: false,
    },
    Row {
        name: "IP-DEFICIT",
        bond_count: 12,
        pending_addbond: 0,
        ledger_exits: 2,
        ledger_withdrawal: 0,
        in_block_addbond: 0,
        in_block_withdrawal: 0,
        in_block_exit: false,
    },
    Row {
        name: "IP-DEFICIT-CREDIT",
        bond_count: 12,
        pending_addbond: 0,
        ledger_exits: 2,
        ledger_withdrawal: 0,
        in_block_addbond: 10,
        in_block_withdrawal: 0,
        in_block_exit: false,
    },
    Row {
        name: "IP-BOTH",
        bond_count: 20,
        pending_addbond: 3,
        ledger_exits: 0,
        ledger_withdrawal: 5,
        in_block_addbond: 7,
        in_block_withdrawal: 4,
        in_block_exit: false,
    },
    Row {
        name: "IP-PENDING",
        bond_count: 1,
        pending_addbond: 6,
        ledger_exits: 0,
        ledger_withdrawal: 0,
        in_block_addbond: 0,
        in_block_withdrawal: 0,
        in_block_exit: false,
    },
    Row {
        name: "IP-EXIT",
        bond_count: 9,
        pending_addbond: 2,
        ledger_exits: 0,
        ledger_withdrawal: 0,
        in_block_addbond: 0,
        in_block_withdrawal: 0,
        in_block_exit: true,
    },
    Row {
        name: "IP-DEBIT-CLAMP",
        bond_count: 5,
        pending_addbond: 0,
        ledger_exits: 0,
        ledger_withdrawal: 5,
        in_block_addbond: 0,
        in_block_withdrawal: 0,
        in_block_exit: true,
    },
];

/// Charge `withdrawal_pending` the way the chain charges it: one
/// `RequestWithdrawal` (`+= declared`) then `exits` `Exit`s (`+= bond_count`).
async fn charged_ledger(node: &Node, pk: &PublicKey, row: &Row) -> ProducerSet {
    let mut ledger = build_ledger(node, pk, row.bond_count, row.pending_addbond);
    let utxo = UtxoSet::new();
    let mut dirty: HashSet<Hash> = HashSet::new();
    let mut regs: Vec<PublicKey> = Vec::new();
    let mut charges: Vec<Transaction> = Vec::new();
    if row.ledger_withdrawal > 0 {
        charges.push(withdrawal_tx_with_inputs(
            pk,
            row.ledger_withdrawal,
            0,
            0x24,
        ));
    }
    charges.extend((0..row.ledger_exits).map(|_| Transaction::new_exit(*pk)));
    for tx in &charges {
        node.process_transaction_producer_effects(
            tx,
            POST_AH,
            SLOT,
            &utxo,
            &mut ledger,
            &mut dirty,
            &mut regs,
        );
    }
    ledger
}

fn holdings_of(ledger: &ProducerSet, pk: &PublicKey, name: &str) -> ProducerHoldings {
    match of_producer_set(ledger, pk) {
        HoldingsLookup::Found(h) => h,
        other => panic!("{name}: fixture ledger does not resolve the producer: {other:?}"),
    }
}

/// The exact tail `[ECON_WITHDRAWAL_OVER_HOLDINGS]` must carry: the allowance
/// AND the five terms it came from, so a gate that reaches the right number
/// from the wrong terms fails here too.
fn expected_tail(h: &ProducerHoldings, in_block_addbond: u32, in_block_withdrawn: u32) -> String {
    format!(
        "but allowance is {} (held={}, pending_addbond={}, in_block_addbond={}, \
         withdrawal_pending={}, in_block_withdrawn={})",
        h.allowance_with(in_block_addbond, in_block_withdrawn),
        h.bond_count,
        h.pending_addbond,
        in_block_addbond,
        h.withdrawal_pending,
        in_block_withdrawn
    )
}

/// `[AddBond]? [Exit]? [RequestWithdrawal]? probe` — the probe declares one bond
/// more than the shared function allows, so the gate must bail at R1.
async fn probe_block(node: &Node, pk: &PublicKey, row: &Row, declared: u32) -> Block {
    let mut txs = Vec::new();
    if row.in_block_addbond > 0 {
        txs.push(add_bond_tx(node, pk, row.in_block_addbond, 0x21));
    }
    if row.in_block_exit {
        txs.push(exit_tx(pk));
    }
    if row.in_block_withdrawal > 0 {
        let prior = withdrawal_tx(pk, row.in_block_withdrawal, 0x22);
        seed_bond_utxos(node, &prior, pk).await;
        txs.push(prior);
    }
    txs.push(withdrawal_tx_with_inputs(pk, declared, 0, 0x23));
    block_with(node, POST_AH, *pk, txs)
}

async fn assert_parity(row: &Row, ledger: ProducerSet, node: &Node, kp: &KeyPair) {
    let pk = *kp.public_key();
    let h = holdings_of(&ledger, &pk, row.name);
    let in_block_withdrawn =
        row.in_block_withdrawal + if row.in_block_exit { h.bond_count } else { 0 };
    let allowance = h.allowance_with(row.in_block_addbond, in_block_withdrawn);
    let tail = expected_tail(&h, row.in_block_addbond, in_block_withdrawn);

    *node.producer_set.write().await = ledger;
    let block = probe_block(node, &pk, row, allowance + 1).await;
    let verdict = node
        .validate_block_economics(&block, POST_AH, ValidationMode::Full)
        .await;

    let msg = match verdict {
        Err(e) => e.to_string(),
        Ok(()) => panic!(
            "{}: O1 — the gate ACCEPTED a withdrawal declaring {}, one above the \
             allowance `ProducerHoldings::allowance_with` computes ({}). The gate's \
             inline R1 at validation_checks.rs is a SECOND transcription of that \
             function and it has drifted upward.",
            row.name,
            allowance + 1,
            allowance
        ),
    };
    assert!(
        msg.contains(&tail),
        "{}: O2/O3 — the gate's R1 must agree with \
         `ProducerHoldings::allowance_with` term for term.\n  expected tail: {}\n  gate said:    {}",
        row.name,
        tail,
        msg
    );
}

/// O1,O2,O3 × PV-R1 × seven partitions. IP-DEFICIT-CREDIT is the row that goes
/// red when the gate's terms are re-ordered into the pre-fix shape.
#[tokio::test]
async fn inc_i180_m2_the_gate_allowance_equals_the_shared_function() {
    for row in ROWS {
        let (node, kp, _temp) = make_node().await;
        let ledger = charged_ledger(&node, kp.public_key(), row).await;
        assert_parity(row, ledger, &node, &kp).await;
    }
}

/// O1,O2,O3 × PV-R1 × IP-CEILING. `register_genesis_producer` allocates one
/// `StoredBondEntry` per bond, so a ceiling ledger cannot be derived through it;
/// this row writes `bond_count` and `withdrawal_pending_count` directly. Nothing
/// downstream reads the bond entries: every probe bails at R1, above R2/R3.
/// The credit chain saturates INSIDE the expression here —
/// `(MAX-5) + 4 + 3` exceeds `u32::MAX` — which is the clamp direction the
/// deficit rows cannot reach.
#[tokio::test]
async fn inc_i180_m2_the_gate_allowance_equals_the_shared_function_at_the_ceiling() {
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    let row = Row {
        name: "IP-CEILING",
        bond_count: u32::MAX - 5,
        pending_addbond: 4,
        ledger_exits: 0,
        ledger_withdrawal: 0,
        in_block_addbond: 3,
        in_block_withdrawal: 0,
        in_block_exit: false,
    };

    let mut ledger = build_ledger(&node, &pk, 1, row.pending_addbond);
    {
        let info = ledger
            .get_by_pubkey_mut(&pk)
            .expect("fixture: genesis producer is registered");
        info.bond_count = row.bond_count;
        info.withdrawal_pending_count = 10;
    }
    let h = holdings_of(&ledger, &pk, row.name);
    assert_eq!(
        u32::MAX - 10,
        h.allowance_with(row.in_block_addbond, 0),
        "IP-CEILING: the fixture must actually engage the upper clamp"
    );
    assert_parity(&row, ledger, &node, &kp).await;
}
