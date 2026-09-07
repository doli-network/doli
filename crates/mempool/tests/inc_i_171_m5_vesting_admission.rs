//! INC-I-171 M5 — the vesting payout bound at MEMPOOL ADMISSION (REQ-VEST-008).
//!
//! OUTPUT CONTRACT — `Mempool::add_transaction(&mut self, Transaction,
//! &UtxoSet, BlockHeight, Slot) -> Result<AddTransactionResult, MempoolError>`:
//!   O1 return                  `Ok(_)` | `Err(MempoolError::InvalidTransaction(msg))`
//!                              whose `msg` carries a bracketed `[CODE]`
//!   O2 receiver `self.entries` membership of the offered tx afterwards
//!   O3 receiver `self.slot_watermark` — non-regressing, read via `slot_watermark()`
//!   O4 `utxo_set` UNMUTATED (`&UtxoSet`, enforced by the type)
//! OUTPUT CONTRACT — `Mempool::revalidate(&mut self, &UtxoSet, BlockHeight, Slot)`:
//!   O5 no return; O6 `self.entries` membership after the pass; O3 as above.
//! PATHS: P-BELOW (h < activation) | P-ENFORCED | P-SLOT0 (watermark 0) |
//!   P-MALFORMED (Bond `extra_data.len() != 4`) | P-REGRESS (slot moves backwards).
//! INPUT PARTITIONS: tier Q1/Q2/Q3/Q4; payout at-bound / bound+1 / full raw
//!   value; Bond-only / mixed Bond + non-Bond inputs.
//! MATRIX: O1,O2 x each path x each partition (one test per cell below);
//!   O3 x P-REGRESS (`req_vest_011_the_mempool_slot_is_a_non_regressing_watermark`);
//!   O6 x reorg (`req_vest_011_reorg_revalidate_keeps_the_honest_withdrawal`).
//!
//! ===========================================================================
//! TARGET API — the developer replaces the `target_api` shims below
//! ===========================================================================
//! Everything else in this file compiles and runs against the CURRENT tree, so
//! the M5 probe prints a value on the RED tree as well as the GREEN one. The
//! shims are the ONLY lines that move.
//!
//! INV-GOV-001 — the shipped window is `[u64::MAX, u64::MAX)`, empty at every
//! reachable height, so the gate is armed through the ENV OVERRIDE. Params are
//! a per-network `OnceLock` frozen on the first `params()` call in the PROCESS,
//! so this must be its own test binary.

use std::sync::Once;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::consensus::ConsensusParams;
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::vesting::penalized_bond_net;
use mempool::{Mempool, MempoolPolicy};
use storage::{Outpoint, UtxoEntry, UtxoSet};

static INIT_ENV: Once = Once::new();

/// Called FIRST by every test, before any `params()` touch. All tests set
/// identical values, so the `Once` race is benign.
fn init_vesting_env() {
    INIT_ENV.call_once(|| {
        std::env::set_var("DOLI_INC_I_171_VESTING_PENALTY_ACTIVATION_HEIGHT", "101");
        std::env::set_var("DOLI_INC_I_171_VESTING_PENALTY_DISABLE_HEIGHT", "201");
    });
}

const H_BELOW: u64 = 99;
const H_ENFORCED: u64 = 101;

/// Devnet `vesting_quarter_slots`. Read back from params in `assert_armed`.
const QUARTER: u32 = 60;

/// Evaluation slot. Ages are centred inside their quarter so no rounding of the
/// tier boundary is load-bearing here — M4 owns the arithmetic.
const SLOT: u32 = 1_234;
/// A LOWER slot, used to prove the watermark does not regress.
const SLOT_OLD: u32 = 1_184;

/// `(creation_slot, label)` per tier at `SLOT` with `QUARTER = 60`.
const TIERS: [(u32, &str); 4] = [
    (SLOT - 30, "Q1 75%"),
    (SLOT - 90, "Q2 50%"),
    (SLOT - 150, "Q3 25%"),
    (SLOT - 210, "Q4 0%"),
];

const PAYOUT_EXCEEDS: &str = "ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET";
const EXTRA_DATA_MALFORMED: &str = "ECON_WITHDRAWAL_BOND_EXTRA_DATA_MALFORMED";

// ============================================================
// TARGET API — the three shims the developer retargets
// ============================================================

mod target_api {
    use super::*;

    /// TARGET: `pool.add_transaction(tx, utxo, height, slot)`.
    pub fn admit(
        pool: &mut Mempool,
        tx: Transaction,
        utxo: &UtxoSet,
        height: u64,
        slot: u32,
    ) -> Result<(), String> {
        pool.add_transaction(tx, utxo, height, slot)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// TARGET: `pool.revalidate(utxo, height, slot)`.
    pub fn revalidate(pool: &mut Mempool, utxo: &UtxoSet, height: u64, slot: u32) {
        pool.revalidate(utxo, height, slot);
    }

    /// TARGET: `pool.slot_watermark()`.
    pub fn watermark(pool: &Mempool) -> u32 {
        pool.slot_watermark()
    }
}

// ============================================================
// FIXTURE
// ============================================================

fn mempool() -> Mempool {
    Mempool::new(
        MempoolPolicy::testnet(),
        ConsensusParams::devnet(),
        Network::Devnet,
    )
}

fn unit() -> u64 {
    Network::Devnet.bond_unit()
}

/// Value of the extra Normal input. It funds the fee on the tiers whose penalty
/// is zero, so every fixture below clears `Transaction::minimum_fee()`.
fn fee_input() -> u64 {
    unit() / 100
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

/// Harness integrity. Without it, "nothing rejected" reads the same whether the
/// rule is unimplemented or the env override never reached `NetworkParams`.
fn assert_armed() {
    let params = Network::Devnet.params();
    assert_eq!(
        (
            params.inc_i_171_vesting_penalty_activation_height,
            params.inc_i_171_vesting_penalty_disable_height,
            params.vesting_quarter_slots,
        ),
        (H_ENFORCED, 201, QUARTER as u64),
        "the env override must arm the window BEFORE the first params() call in this process"
    );
}

fn entry(output: Output) -> UtxoEntry {
    UtxoEntry {
        output,
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

/// One Bond UTXO per `bonds` entry plus one Normal of `fee_input()`. Returns
/// the matching inputs, each carrying the owner key so signing is possible.
/// A direct `UtxoSet` insert bypasses `[ERRTX007]`, which is exactly the
/// snap-sync ingress hole INV-VEST-013 names.
fn seed(
    utxo: &mut UtxoSet,
    kp: &KeyPair,
    bonds: &[(u64, u32)],
    malformed_at: Option<usize>,
    tag: u8,
) -> Vec<Input> {
    let pk = *kp.public_key();
    let h = Hash::from_bytes([tag; 32]);
    let mut inputs = Vec::new();
    for (i, (amount, creation_slot)) in bonds.iter().enumerate() {
        let mut output = Output::bond(*amount, addr(&pk), u64::MAX, *creation_slot);
        if malformed_at == Some(i) {
            output.extra_data = vec![0u8; 3];
        }
        let index = inputs.len() as u32;
        utxo.insert(Outpoint::new(h, index), entry(output))
            .expect("fixture: seed Bond UTXO");
        let mut inp = Input::new(h, index);
        inp.public_key = Some(pk);
        inputs.push(inp);
    }
    let index = inputs.len() as u32;
    utxo.insert(
        Outpoint::new(h, index),
        entry(Output::normal(fee_input(), addr(&pk))),
    )
    .expect("fixture: seed the fee input");
    let mut inp = Input::new(h, index);
    inp.public_key = Some(pk);
    inputs.push(inp);
    inputs
}

fn signed_withdrawal(kp: &KeyPair, inputs: Vec<Input>, declared: u32, payout: u64) -> Transaction {
    let pk = *kp.public_key();
    let dest = crypto::hash::hash(b"inc-i-171-m5-admission-destination");
    let mut tx = Transaction::new_request_withdrawal(inputs, pk, declared, dest, payout);
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
    }
    tx
}

/// The bound the three sites must agree on: penalized net per Bond, summed
/// after truncation, plus the non-Bond input value.
fn bound_at(bonds: &[(u64, u32)], slot: u32) -> u64 {
    bonds
        .iter()
        .fold(0u64, |acc, (amount, creation)| {
            acc.saturating_add(penalized_bond_net(*amount, *creation, slot, QUARTER))
        })
        .saturating_add(fee_input())
}

/// One Bond of `unit()` at tier `creation_slot`, with the payout the caller asks
/// for. `declared` is always 1: the INC-I-180 rules are not armed in this
/// binary (no holdings source is wired, so the lookup is `Unavailable`), but
/// keeping the declared count honest keeps the fixture legal for the gate too.
fn case(kp: &KeyPair, utxo: &mut UtxoSet, creation_slot: u32, tag: u8, payout: u64) -> Transaction {
    let inputs = seed(utxo, kp, &[(unit(), creation_slot)], None, tag);
    signed_withdrawal(kp, inputs, 1, payout)
}

fn is_vesting_rejection(verdict: &Result<(), String>, code: &str) -> bool {
    matches!(verdict, Err(msg) if msg.contains(code))
}

// ============================================================
// MUST — REQ-VEST-008: admission reaches the block-validation verdict
// ============================================================

// REQ-VEST-008 (Should) — Decision: a failure here means a producer can park an
// unvested full-value withdrawal in every mempool on the network and have each
// builder waste a slot assembling a block the whole fleet rejects; it is also
// the direct read on whether admission evaluates the bound at all.
#[test]
fn req_vest_008_m5_full_value_q1_withdrawal_is_rejected_at_admission() {
    init_vesting_env();
    assert_armed();
    let kp = KeyPair::generate();
    let mut utxo = UtxoSet::new();
    let mut pool = mempool();

    // Payout takes the WHOLE raw bond; the Q1 bound is 26% of it plus the fee input.
    let tx = case(&kp, &mut utxo, TIERS[0].0, 0xA1, unit());
    let verdict = target_api::admit(&mut pool, tx.clone(), &utxo, H_ENFORCED, SLOT);

    println!(
        "M5-PROBE-VERDICT-ADMISSION: {}",
        match &verdict {
            Ok(()) => "ACCEPTED".to_string(),
            Err(msg) if msg.contains(PAYOUT_EXCEEDS) => format!("REJECTED({PAYOUT_EXCEEDS})"),
            Err(msg) => format!("REJECTED(other): {msg}"),
        }
    );

    assert!(
        is_vesting_rejection(&verdict, PAYOUT_EXCEEDS),
        "REQ-VEST-008: a {} payout against a {} bound must be refused at admission; got {:?}",
        unit(),
        bound_at(&[(unit(), TIERS[0].0)], SLOT),
        verdict
    );
    assert!(
        !pool.contains(&tx.hash()),
        "REQ-VEST-008 (O2): a refused withdrawal must not be resident"
    );
}

// REQ-VEST-008 (Should) — Decision: reveals admission and block validation
// disagreeing on the bound itself — an over-rejection censors an honest CLI
// withdrawal at every tier, which is the failure mode that makes a shipped
// admission rule worse than none.
#[test]
fn req_vest_008_honest_payout_is_admitted_at_every_tier() {
    init_vesting_env();
    assert_armed();
    for (tag, (creation_slot, label)) in TIERS.iter().enumerate() {
        let kp = KeyPair::generate();
        let mut utxo = UtxoSet::new();
        let mut pool = mempool();
        let bonds = [(unit(), *creation_slot)];
        let bound = bound_at(&bonds, SLOT);
        // One fee-input below the bound: an exact-bound Q4 payout would leave a
        // zero fee, which `Transaction::minimum_fee()` refuses for its own
        // reason and would make this row agree for the wrong cause.
        let payout = bound - fee_input();
        let tx = case(&kp, &mut utxo, *creation_slot, 0xB0 + tag as u8, payout);
        let verdict = target_api::admit(&mut pool, tx.clone(), &utxo, H_ENFORCED, SLOT);
        assert_eq!(
            verdict,
            Ok(()),
            "REQ-VEST-008 {label}: honest payout {payout} against bound {bound} must be admitted"
        );
        assert!(
            pool.contains(&tx.hash()),
            "REQ-VEST-008 {label} (O2): an admitted withdrawal must be resident"
        );
    }
}

// REQ-VEST-008 (Should) — Decision: reveals an off-by-one between the admission
// bound and the block bound; one unit either way is the difference between a
// censored honest exit and an admitted unvested one.
#[test]
fn req_vest_008_one_unit_over_the_bound_is_refused_at_admission() {
    init_vesting_env();
    assert_armed();
    let kp = KeyPair::generate();
    let mut utxo = UtxoSet::new();
    let mut pool = mempool();
    let bonds = [(unit(), TIERS[0].0)];
    let bound = bound_at(&bonds, SLOT);
    let tx = case(&kp, &mut utxo, TIERS[0].0, 0xC1, bound + 1);

    let verdict = target_api::admit(&mut pool, tx, &utxo, H_ENFORCED, SLOT);
    assert!(
        is_vesting_rejection(&verdict, PAYOUT_EXCEEDS),
        "REQ-VEST-008: payout {} against bound {bound} must be refused; got {verdict:?}",
        bound + 1
    );
}

// REQ-VEST-008 (Should) — Decision: reveals the admission gate leaking outside
// its own window, which would refuse withdrawals that every node on the
// pre-activation chain still accepts (INV-VEST-007) — a self-inflicted
// censorship split with no consensus change behind it.
#[test]
fn req_vest_008_below_the_activation_height_admission_is_unchanged() {
    init_vesting_env();
    assert_armed();
    let kp = KeyPair::generate();
    let mut utxo = UtxoSet::new();
    let mut pool = mempool();
    let tx = case(&kp, &mut utxo, TIERS[0].0, 0xD1, unit());

    assert_eq!(
        target_api::admit(&mut pool, tx.clone(), &utxo, H_BELOW, SLOT),
        Ok(()),
        "INV-VEST-007: below the activation height the full-value withdrawal must still be \
         admitted"
    );
    assert!(
        pool.contains(&tx.hash()),
        "INV-VEST-007 (O2): still resident"
    );
}

// ============================================================
// MUST — INV-VEST-013: fail closed on an undecodable Bond slot
// ============================================================

// REQ-VEST-008 / INV-VEST-013 — Decision: reveals admission reusing the
// fail-OPEN `bond_creation_slot().unwrap_or(0)` default, which reads a
// snap-synced malformed Bond as fully vested and hands back 100% of it.
#[test]
fn req_vest_013_malformed_bond_extra_data_fails_closed_at_admission() {
    init_vesting_env();
    assert_armed();
    let kp = KeyPair::generate();
    let mut utxo = UtxoSet::new();
    let mut pool = mempool();

    // A payout of 1 is below ANY plausible bound, so only a fail-CLOSED rule
    // can refuse this transaction.
    let inputs = seed(&mut utxo, &kp, &[(unit(), TIERS[0].0)], Some(0), 0xE1);
    let tx = signed_withdrawal(&kp, inputs, 1, 1);

    let verdict = target_api::admit(&mut pool, tx.clone(), &utxo, H_ENFORCED, SLOT);
    assert!(
        is_vesting_rejection(&verdict, EXTRA_DATA_MALFORMED),
        "INV-VEST-013: a 3-byte Bond `extra_data` must refuse the withdrawal, never read as \
         fully vested; got {verdict:?}"
    );
    assert!(
        !pool.contains(&tx.hash()),
        "INV-VEST-013 (O2): a refused withdrawal must not be resident"
    );
}

// ============================================================
// MUST — INV-VEST-011: the slot is a non-regressing watermark
// ============================================================

// REQ-VEST-008 / INV-VEST-011 — Decision: reveals the mempool taking the slot
// it was last handed rather than the highest it has seen. `best_slot` regresses
// on every reorg (`block_handling.rs:803,875`), and a regressed slot RAISES
// every bond's penalty, so an honest exit already legal would start being
// refused at exactly the moment the chain is least stable. Admission is
// liveness-only; over-rejection there is censorship (INC-I-057, INC-I-203,
// INC-I-147) and the builder, on the block's own slot, is the exact gate.
#[test]
fn req_vest_011_the_mempool_slot_is_a_non_regressing_watermark() {
    init_vesting_env();
    assert_armed();
    let kp = KeyPair::generate();
    let mut utxo = UtxoSet::new();
    let mut pool = mempool();

    // A bond created at `SLOT_OLD - 1`: at SLOT_OLD its age is 1 (Q1, 75%), at
    // SLOT its age is 51 — still Q1 — so the DIFFERING term is the tier below.
    // Chosen so the verdict differs between the two slots:
    //   creation at SLOT - 70 → age 20 at SLOT_OLD (Q1, bound 26%),
    //                           age 70 at SLOT     (Q2, bound 51%).
    let creation = SLOT - 70;
    let bonds = [(unit(), creation)];
    let bound_old = bound_at(&bonds, SLOT_OLD);
    let bound_new = bound_at(&bonds, SLOT);
    assert!(
        bound_old < bound_new,
        "fixture: the two slots must give DIFFERENT bounds, got {bound_old} and {bound_new}"
    );

    // First call at the HIGH slot, second at the LOW one.
    let warm = case(&kp, &mut utxo, creation, 0xF1, 1);
    target_api::admit(&mut pool, warm, &utxo, H_ENFORCED, SLOT).expect("fixture: warm-up admit");
    assert_eq!(
        target_api::watermark(&pool),
        SLOT,
        "INV-VEST-011 (O3): the watermark must record the highest slot seen"
    );

    let kp2 = KeyPair::generate();
    let mut utxo2 = UtxoSet::new();
    let probe = case(&kp2, &mut utxo2, creation, 0xF2, bound_new);
    let verdict = target_api::admit(&mut pool, probe, &utxo2, H_ENFORCED, SLOT_OLD);
    assert_eq!(
        target_api::watermark(&pool),
        SLOT,
        "INV-VEST-011 (O3): a LOWER slot must not move the watermark back to {SLOT_OLD}"
    );
    assert_eq!(
        verdict,
        Ok(()),
        "INV-VEST-011: a payout of {bound_new} is legal at the watermark {SLOT} and must STAY \
         legal after the slot regresses to {SLOT_OLD} (bound there is only {bound_old}); a reorg \
         must never turn an honest in-flight withdrawal into a rejection; got {verdict:?}"
    );

    // The negative half: the arm must still be live at the watermark, not merely
    // disabled. One unit over the watermark bound is refused at the same slot.
    let kp3 = KeyPair::generate();
    let mut utxo3 = UtxoSet::new();
    let over = case(&kp3, &mut utxo3, creation, 0xF3, bound_new + 1);
    let over_verdict = target_api::admit(&mut pool, over, &utxo3, H_ENFORCED, SLOT_OLD);
    assert!(
        is_vesting_rejection(&over_verdict, PAYOUT_EXCEEDS),
        "INV-VEST-011: {} is one over the watermark bound {bound_new} and must be refused even \
         though the call slot regressed to {SLOT_OLD}; got {over_verdict:?}",
        bound_new + 1
    );
}

// REQ-VEST-008 / INV-VEST-011 — Decision: reveals admission evaluating the bound
// against slot 0, where every bond reads as age 0 (the harshest 75% tier) and
// every honest withdrawal on the network is refused at once. A cold mempool has
// no slot, so the skip is what keeps a restarted node from censoring.
#[test]
fn req_vest_011_a_zero_watermark_skips_the_check_at_admission() {
    init_vesting_env();
    assert_armed();
    let kp = KeyPair::generate();
    let mut utxo = UtxoSet::new();
    let mut pool = mempool();

    assert_eq!(
        target_api::watermark(&pool),
        0,
        "fixture: a fresh mempool has never seen a slot"
    );
    let honest_bound = bound_at(&[(unit(), TIERS[0].0)], SLOT);
    let honest = case(&kp, &mut utxo, TIERS[0].0, 0x11, honest_bound - fee_input());
    let full_value = case(&kp, &mut utxo, TIERS[0].0, 0x12, unit());

    for (label, tx) in [("honest", honest), ("full-value", full_value)] {
        let verdict = target_api::admit(&mut pool, tx, &utxo, H_ENFORCED, 0);
        assert_eq!(
            verdict,
            Ok(()),
            "INV-VEST-011 {label}: at watermark 0 the vesting check must be SKIPPED — the \
             builder is the gate. Got {verdict:?}"
        );
    }
}

// REQ-VEST-008 / INV-VEST-011 — Decision: reveals a vesting `Err` reaching the
// `revalidate` evict-on-Err loop. A reorg drops `best_slot` to the common
// ancestor; if the lowered slot re-evaluates residents, an honest withdrawal is
// silently dropped from every mempool on the network mid-reorg and the producer
// has no signal at all. The second half guards the eviction that must SURVIVE.
#[test]
fn req_vest_011_reorg_revalidate_keeps_the_honest_withdrawal() {
    init_vesting_env();
    assert_armed();
    let kp = KeyPair::generate();
    let mut utxo = UtxoSet::new();
    let mut pool = mempool();

    let bonds = [(unit(), TIERS[0].0)];
    let honest = case(
        &kp,
        &mut utxo,
        TIERS[0].0,
        0x21,
        bound_at(&bonds, SLOT) - fee_input(),
    );
    let honest_hash = honest.hash();
    target_api::admit(&mut pool, honest, &utxo, H_ENFORCED, SLOT).expect("fixture: admit honest");

    // A second withdrawal whose input the reorg removes from the UTXO set. It
    // must STILL be evicted: input existence is a rule the vesting exemption
    // does not touch.
    let kp2 = KeyPair::generate();
    let doomed = case(&kp2, &mut utxo, TIERS[0].0, 0x22, 1);
    let doomed_hash = doomed.hash();
    target_api::admit(&mut pool, doomed, &utxo, H_ENFORCED, SLOT).expect("fixture: admit doomed");
    for index in 0..2u32 {
        utxo.remove(&Outpoint::new(Hash::from_bytes([0x22; 32]), index))
            .expect("fixture: reorg removes the doomed transaction's inputs");
    }

    // The reorg: height and slot both move BACKWARDS to the common ancestor.
    target_api::revalidate(&mut pool, &utxo, H_ENFORCED - 5, SLOT - 50);

    assert!(
        pool.contains(&honest_hash),
        "INV-VEST-011 (O6): a withdrawal that was honest at slot {SLOT} must survive a \
         revalidate at slot {}",
        SLOT - 50
    );
    assert!(
        !pool.contains(&doomed_hash),
        "INV-VEST-011 (O6, regression guard): revalidate must still evict a transaction whose \
         inputs the reorg removed"
    );
    assert_eq!(
        target_api::watermark(&pool),
        SLOT,
        "INV-VEST-011 (O3): revalidate must not walk the watermark back either"
    );
}
