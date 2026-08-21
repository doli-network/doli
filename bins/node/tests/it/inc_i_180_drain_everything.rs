//! INC-I-180 M1 DRAIN-EVERYTHING — the R2 split and the R4 same-block rule.
//! Requirements: REQ-I180-001 (Must), REQ-I180-002 (Must), REQ-I180-003 (Must).
//!
//! covers: bins/node/src/node/validation_checks.rs (INC-I-180 gate, R2/R3/R4),
//!         specs/protocol.md (withdrawal rules), docs/error-codes.md
//!         (ECON_WITHDRAWAL_INCOMPLETE_DRAIN, ECON_WITHDRAWAL_SAME_BLOCK_INPUT)
//!
//! Normative source: `docs/.workflow/inc-i-180-M1-drain-everything-design.md`
//! (user decision on AUDIT-P1-002). Post-AH a `RequestWithdrawal` splits by
//! shape instead of always demanding `declared == bond_inputs`:
//!
//!   is_full_exit := declared == allowance && declared > 0
//!   full exit ⇒ require bond_inputs == owned_live_bonds(P)
//!               else [ECON_WITHDRAWAL_INCOMPLETE_DRAIN]
//!   partial   ⇒ require declared == bond_inputs   (today's rule, verbatim)
//!               else [ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]
//!
//! and, evaluated BEFORE both, R4: no input may reference a transaction at a
//! lower index in the same block, else [ECON_WITHDRAWAL_SAME_BLOCK_INPUT].
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT:
//! ---------------------------------------------------------------------------
//! Function under test:
//!   `Node::validate_block_economics(&Block, height, ValidationMode) -> Result<()>`
//! plus the apply/rebuild counterparts it must stay in parity with
//!   `Node::process_transaction_producer_effects(..) -> ()` (parameter mutations)
//!   `Node::rebuild_producer_set_from_blocks(&self, &mut ProducerSet, u64)`
//!
//!   O1: the accept/reject verdict, asserted on the BRACKETED ERROR TOKEN, not
//!       on `is_err()` — the three rules return three different codes and a bare
//!       `is_err()` cannot tell them apart.
//!   O2: post-flush `ProducerSet` state — `bond_count`, `status`, `weight`.
//!   O3: validation/apply parity — `Ok` at validation ⇒ the update was queued.
//!   O4: `withdrawal_pending_count` after apply, before the flush.
//!   O5: the queued `PendingProducerUpdate::RequestWithdrawal` count.
//!   NOT outputs: neither function writes a persistent store; no side channel;
//!       TERMINATION is not an output (every path is a bounded loop).
//!
//! PATHS
//!   PD1: height >= AH, full-exit branch (declared == allowance, declared > 0)
//!   PD2: height >= AH, partial branch (declared != allowance)
//!   PD3: height >= AH, R4 same-block-input rejection
//!   PD4: height <  AH, whole gate skipped — verdict bit-identical to today
//!   PD5: rebuild replay (no admission rules exist there at all)
//!
//! INPUT PARTITIONS:
//!   IP-D1  ledger 433, ZERO live Bond UTXOs, declared 433 == allowance, zero
//!          Bond inputs (one Normal input, the fee coin) — the n11 repair
//!   IP-D2  declared 430 < allowance 434, 400 Bond inputs — partial mismatch
//!   IP-D3  declared 60 == allowance, 100 owned Bond UTXOs, only 60 spent
//!   IP-D4  IP-D1 and IP-D3 constructions below the gate
//!   IP-D5  [AddBond(X,40), RequestWithdrawal(X, 60, 60 pre-block + the 40
//!          outpoints of that AddBond)] — the AUDIT-P1-006 chain
//!   IP-D6  IP-D5 below the gate
//!   IP-D7  IP-D1 driven through validation + apply + flush
//!   IP-D8  declared 60 == allowance, 60 own Bond inputs + 1 foreign Bond input
//!   IP-D9  IP-D1 shape as a canonical chain, live replay vs rebuild replay
//!
//! MATRIX (every enumerated cell has an assertion)
//!   O1,O2  ×PD1×IP-D1 → req_i180_001_post_ah_full_exit_drains_a_ledger_with_no_bond_utxos
//!   O1     ×PD2×IP-D2 → req_i180_001_post_ah_partial_under_spend_is_still_rejected
//!   O1     ×PD1×IP-D3 → req_i180_001_post_ah_full_exit_that_leaves_bond_utxos_behind_is_rejected
//!   O1     ×PD4×IP-D4 → req_i180_003_pre_ah_drain_shapes_keep_the_legacy_verdict
//!   O1     ×PD3×IP-D5 → req_i180_001_post_ah_same_block_created_inputs_are_rejected
//!   O1     ×PD4×IP-D6 → req_i180_003_pre_ah_same_block_created_inputs_keep_legacy
//!   O1,O3,O4,O5×PD1×IP-D7 → req_i180_001_post_ah_drain_shape_holds_validation_apply_parity
//!   O1     ×PD1×IP-D8 → req_i180_001_post_ah_full_exit_with_a_foreign_bond_rider_is_rejected
//!   O2,O5  ×PD5×IP-D9 → req_i180_001_rebuild_matches_live_for_the_drain_shape

use std::collections::HashSet;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::Transaction;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{PendingProducerUpdate, ProducerSet, ProducerStatus, UtxoSet};
use vdf::{VdfOutput, VdfProof};

use crate::inc_i_180_common::{
    add_bond_tx, make_node, run_block_case_unseeded, seed_bond_utxos, seed_bond_utxos_split,
    seed_normal_utxos, seed_owned_bond_utxos, verdict_in_mode, withdrawal_tx_chained,
    withdrawal_tx_with_inputs, N11_BONDS, POST_AH, PRE_AH,
};

/// Bond UTXOs the IP-D3 / IP-D5 producer owns in the pre-block view.
const OWNED_BONDS: u32 = 100;
/// Bond UTXOs the IP-D3 / IP-D5 withdrawal actually names as inputs.
const SPENT_BONDS: u32 = 60;
/// Bond UTXOs the IP-D5 in-block `AddBond` creates.
const FRESH_BONDS: u32 = 40;

const TAG_D1: u8 = 0x40;
const TAG_D2: u8 = 0x42;
const TAG_D3: u8 = 0x44;
const TAG_D3_UNSPENT: u8 = 0x45;
const TAG_D5_PRE: u8 = 0x46;
const TAG_D5_ADD: u8 = 0x47;
const TAG_D8: u8 = 0x48;

// ═══════════════════════════════════════════════════════════════════════════
// PD1 — the full-exit branch: DRAIN EVERYTHING
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O2 × PD1 × IP-D1 — **RED before the drain-everything fix.** The n11
/// repair: a ledger of 433 bonds with no Bond UTXOs left behind it declares its
/// whole allowance and lands on zero. Today `declared != bond_inputs` bails and
/// the producer is permanently unreconcilable (AUDIT-P1-002).
/// covers: validation_checks.rs, specs/protocol.md, docs/error-codes.md
#[tokio::test]
async fn req_i180_001_post_ah_full_exit_drains_a_ledger_with_no_bond_utxos() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    let tx = withdrawal_tx_with_inputs(&pk, N11_BONDS, 1, TAG_D1);
    seed_normal_utxos(&node, &tx, &pk).await;

    let verdict = verdict_in_mode(
        &node,
        &pk,
        N11_BONDS,
        0,
        vec![tx.clone()],
        POST_AH,
        ValidationMode::Light,
    )
    .await;
    assert!(
        verdict.is_ok(),
        "O1/PD1: declared 433 == allowance 433 with ZERO live Bond UTXOs is the \
         full-exit shape. The flush clamps bond_count to 0 (producer/info.rs \
         fallback) and auto-exit fires, so no weight can survive unbacked — the \
         obligation is to destroy every Bond UTXO owned, and there are none. \
         got {verdict:?}"
    );

    let o = run_block_case_unseeded(&node, &kp, N11_BONDS, 0, vec![tx], POST_AH).await;
    assert_eq!(
        (o.bond_count, o.status, o.weight),
        (0, ProducerStatus::Exited, 0),
        "O2/PD1: and the repair is COMPLETE — the ledger lands on zero and the \
         producer retires. An accept that left residual weight would just move \
         the n11 shape rather than close it. got {o:?}"
    );
}

/// O1 × PD2 × IP-D2 — **GREEN today, must STAY green.** Below the full-exit
/// boundary the shipped strict rule is unchanged: a partial withdrawal must
/// destroy exactly as many of its own Bond UTXOs as it declares.
#[tokio::test]
async fn req_i180_001_post_ah_partial_under_spend_is_still_rejected() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    let tx = withdrawal_tx_with_inputs(&pk, 430, 400, TAG_D2);
    seed_bond_utxos(&node, &tx, &pk).await;
    let verdict = verdict_in_mode(
        &node,
        &pk,
        N11_BONDS,
        1,
        vec![tx],
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    let err = verdict.expect_err(
        "O1/PD2: 430 < allowance 434, so this is a PARTIAL and the drain \
         exemption must not reach it. 430 declared against 400 Bond UTXOs \
         destroyed leaves 30 units of weight behind nothing",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]"),
        "O1/PD2: the partial branch keeps the shipped token — a fix that routed \
         every withdrawal through the drain check would emit INCOMPLETE_DRAIN \
         here and silently widen what a partial may do. got: {err}"
    );
}

/// O1 × PD1 × IP-D3 — **RED before the drain-everything fix.** The full-exit
/// exemption is not a licence to under-spend: declaring the whole allowance
/// obliges the transaction to destroy EVERY Bond UTXO the producer owns.
/// Without this row the exemption would let a producer zero its ledger while
/// keeping 40 spendable Bond UTXOs — value created from nothing.
/// covers: validation_checks.rs, specs/protocol.md, docs/error-codes.md
#[tokio::test]
async fn req_i180_001_post_ah_full_exit_that_leaves_bond_utxos_behind_is_rejected() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    let tx = withdrawal_tx_with_inputs(&pk, SPENT_BONDS, SPENT_BONDS, TAG_D3);
    seed_bond_utxos(&node, &tx, &pk).await;
    seed_owned_bond_utxos(&node, &pk, TAG_D3_UNSPENT, OWNED_BONDS - SPENT_BONDS).await;

    let verdict = verdict_in_mode(
        &node,
        &pk,
        SPENT_BONDS,
        0,
        vec![tx],
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    let err = verdict.expect_err(
        "O1/PD1: declared 60 == allowance 60 selects the full-exit branch, and \
         the flush will drive bond_count to 0. The 40 Bond UTXOs this tx does \
         not name stay spendable with no ledger behind them",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_INCOMPLETE_DRAIN]"),
        "O1/PD1: the full-exit branch owes its own code — bond_inputs 60 != \
         owned_live_bonds 100. got: {err}"
    );
}

/// O1 × PD1 × IP-D8 — **GREEN today, must STAY green.** R3 (exclusivity) is
/// evaluated BEFORE R2, so a full exit padded with a foreign Bond UTXO dies on
/// the mismatch token, not on the drain token. Ordering is the property: if R2
/// ran first this would report INCOMPLETE_DRAIN and the exclusivity rule would
/// become unobservable under the full-exit shape.
#[tokio::test]
async fn req_i180_001_post_ah_full_exit_with_a_foreign_bond_rider_is_rejected() {
    let (node, kp_b, _t) = make_node().await;
    let pk_b = *kp_b.public_key();
    let pk_a = *KeyPair::generate().public_key();

    let tx = withdrawal_tx_with_inputs(&pk_b, SPENT_BONDS, SPENT_BONDS + 1, TAG_D8);
    seed_bond_utxos_split(&node, &tx, &pk_b, SPENT_BONDS as usize, &pk_a).await;

    let verdict = verdict_in_mode(
        &node,
        &pk_b,
        SPENT_BONDS,
        0,
        vec![tx],
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    let err = verdict.expect_err(
        "O1/PD1: B drains its own 60 and A's Bond UTXO rides along. Every input \
         is spent by apply, so A loses a bond with no ledger effect at all",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]"),
        "O1/PD1: 61 Bond inputs, 60 of them B's — R3 fires first. got: {err}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PD3 — R4: an input created earlier in the SAME block is rejected outright
// ═══════════════════════════════════════════════════════════════════════════

/// Build the AUDIT-P1-006 block: an `AddBond` creating 40 Bond outputs, then a
/// withdrawal spending 60 pre-block Bond UTXOs PLUS those 40 fresh outpoints.
/// The 40 are invisible to the pre-block UTXO view, so both Bond counters read
/// 60 and the count equality passes while apply destroys all 100.
fn same_block_chain(node: &Node, pk: &PublicKey) -> Vec<Transaction> {
    let add = add_bond_tx(node, pk, FRESH_BONDS, TAG_D5_ADD);
    let wd = withdrawal_tx_chained(pk, SPENT_BONDS, SPENT_BONDS, TAG_D5_PRE, &add, FRESH_BONDS);
    vec![add, wd]
}

/// O1 × PD3 × IP-D5 — **RED before the R4 fix.** The gate resolves inputs
/// against the pre-block view only, and `validate_block_economics` holds no
/// `BlockBatch`, so same-block outputs cannot be counted — they must be
/// refused. Allowance is 60 + 40 = 100 and declared is 60, so every existing
/// rule passes and the block is admitted today.
/// covers: validation_checks.rs, specs/protocol.md, docs/error-codes.md
#[tokio::test]
async fn req_i180_001_post_ah_same_block_created_inputs_are_rejected() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    seed_owned_bond_utxos(&node, &pk, TAG_D5_PRE, SPENT_BONDS).await;
    let verdict = verdict_in_mode(
        &node,
        &pk,
        SPENT_BONDS,
        0,
        same_block_chain(&node, &pk),
        POST_AH,
        ValidationMode::Light,
    )
    .await;

    let err = verdict.expect_err(
        "O1/PD3: the queued AddBond still credits bond_count += 40 at the epoch \
         flush with no liveness check, while apply already spent those 40 fresh \
         Bond UTXOs inside this block. Net: 40 units of selection weight with \
         nothing behind them, from ONE key, for the price of fees",
    );
    assert!(
        err.contains("[ECON_WITHDRAWAL_SAME_BLOCK_INPUT]"),
        "O1/PD3: R4 must fire on the input's PROVENANCE, before any counting. \
         An INCOMPLETE_DRAIN or BOND_COUNT_MISMATCH here would mean the fix \
         tried to count the same-block outputs instead of refusing them, which \
         the validator's pre-block-only UTXO view cannot do correctly. got: {err}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PD4 — below the gate every rule is absent, bit-identically
// ═══════════════════════════════════════════════════════════════════════════

/// O1 × PD4 × IP-D4 — **GREEN today, must STAY green.** Both new rules ship
/// under the SAME activation height as the gate they live in, so historical
/// blocks of either shape must still replay as admitted.
#[tokio::test]
async fn req_i180_003_pre_ah_drain_shapes_keep_the_legacy_verdict() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();
    let drain = withdrawal_tx_with_inputs(&pk, N11_BONDS, 1, TAG_D1);
    seed_normal_utxos(&node, &drain, &pk).await;
    let verdict = verdict_in_mode(
        &node,
        &pk,
        N11_BONDS,
        0,
        vec![drain],
        PRE_AH,
        ValidationMode::Light,
    )
    .await;
    assert!(
        verdict.is_ok(),
        "O1/PD4: the IP-D1 construction below the gate — admitted, as it always \
         was. got {verdict:?}"
    );

    let (node2, kp2, _t2) = make_node().await;
    let pk2 = *kp2.public_key();
    let partial_drain = withdrawal_tx_with_inputs(&pk2, SPENT_BONDS, SPENT_BONDS, TAG_D3);
    seed_bond_utxos(&node2, &partial_drain, &pk2).await;
    seed_owned_bond_utxos(&node2, &pk2, TAG_D3_UNSPENT, OWNED_BONDS - SPENT_BONDS).await;
    let verdict2 = verdict_in_mode(
        &node2,
        &pk2,
        SPENT_BONDS,
        0,
        vec![partial_drain],
        PRE_AH,
        ValidationMode::Light,
    )
    .await;
    assert!(
        verdict2.is_ok(),
        "O1/PD4: the IP-D3 construction below the gate — no drain obligation \
         exists there, so leaving 40 Bond UTXOs behind is historical and legal. \
         got {verdict2:?}"
    );
}

/// O1 × PD4 × IP-D6 — **GREEN today, must STAY green.** R4 is gated too: a
/// canonical block that already chained an AddBond into a withdrawal must keep
/// replaying, or every node forks at that height.
#[tokio::test]
async fn req_i180_003_pre_ah_same_block_created_inputs_keep_legacy() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    seed_owned_bond_utxos(&node, &pk, TAG_D5_PRE, SPENT_BONDS).await;
    let verdict = verdict_in_mode(
        &node,
        &pk,
        SPENT_BONDS,
        0,
        same_block_chain(&node, &pk),
        PRE_AH,
        ValidationMode::Light,
    )
    .await;

    assert!(
        verdict.is_ok(),
        "O1/PD4: below the gate the whole block is skipped, so the same-block \
         chain replays exactly as it did. got {verdict:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PD1 — validation/apply parity for the drain shape
// ═══════════════════════════════════════════════════════════════════════════

/// O1,O3,O4,O5 × PD1 × IP-D7 — **RED before the drain-everything fix.** The
/// accept side is only safe if apply agrees: admitting the repair while apply
/// skipped the enqueue would spend the fee coin and leave 433 units of weight
/// standing — the same (admitted, not-enqueued) pair the gate exists to make
/// unreachable.
/// covers: validation_checks.rs, specs/protocol.md, docs/error-codes.md
#[tokio::test]
async fn req_i180_001_post_ah_drain_shape_holds_validation_apply_parity() {
    let (node, kp, _t) = make_node().await;
    let pk = *kp.public_key();

    let tx = withdrawal_tx_with_inputs(&pk, N11_BONDS, 1, TAG_D1);
    seed_normal_utxos(&node, &tx, &pk).await;
    let o = run_block_case_unseeded(&node, &kp, N11_BONDS, 0, vec![tx], POST_AH).await;

    assert!(
        o.validation_ok,
        "O1/PD1: the repair transaction must be admissible, otherwise the \
         producer has no in-band remedy at all. got {o:?}"
    );
    assert!(
        o.parity_holds(1),
        "O3/PD1: validation admitted the block, so apply must have queued the \
         RequestWithdrawal. got {o:?}"
    );
    assert_eq!(
        o.queued_withdrawals, 1,
        "O5/PD1: and the parity assertion above is non-vacuous — apply's own \
         `remaining` (433 + 0 - 0) covers the 433-bond request"
    );
    assert_eq!(
        o.withdrawal_pending, N11_BONDS,
        "O4/PD1: the in-epoch double-withdrawal guard is charged the full 433, \
         so a second request in the same epoch has zero allowance left"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PD5 — the rebuild replay must reach the same state for the drain shape
// ═══════════════════════════════════════════════════════════════════════════

const GENESIS_BLOCKS: u64 = 40;
const EPOCH_LEN: u64 = 4;
const REG_H: u64 = 41;
const WITHDRAW_H: u64 = 46;

fn chain_block(node: &Node, height: u64, producer: PublicKey, txs: Vec<Transaction>) -> Block {
    let slot = height as u32;
    let reward = node.params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(reward, pool_hash, height, slot);

    let mut all = vec![coinbase];
    all.extend(txs);

    let merkle_root = doli_core::block::compute_merkle_root(&all);
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (slot as u64 * node.params.slot_duration),
        slot,
        producer,
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, all)
}

fn live_replay(node: &Node, blocks: &[(u64, Block)]) -> ProducerSet {
    let mut ps = ProducerSet::new();
    let utxo = UtxoSet::new();
    for (height, block) in blocks {
        let mut dirty: HashSet<Hash> = HashSet::new();
        let mut regs: Vec<PublicKey> = Vec::new();
        for tx in &block.transactions {
            node.process_transaction_producer_effects(
                tx,
                *height,
                block.header.slot,
                &utxo,
                &mut ps,
                &mut dirty,
                &mut regs,
            );
        }
        if *height < EPOCH_LEN || height.is_multiple_of(EPOCH_LEN) {
            ps.apply_pending_updates_with_cap(0);
        }
        ps.process_unbonding(*height, doli_core::consensus::UNBONDING_PERIOD);
    }
    ps
}

fn state_of(ps: &ProducerSet, pk: &PublicKey) -> (u32, ProducerStatus, u64) {
    ps.get_by_pubkey(pk)
        .map(|i| (i.bond_count, i.status, i.selection_weight()))
        .unwrap_or((0, ProducerStatus::Exited, 0))
}

fn queued_withdrawals(ps: &ProducerSet, pk: &PublicKey) -> usize {
    ps.pending_updates_for(pk)
        .iter()
        .filter(|u| matches!(u, PendingProducerUpdate::RequestWithdrawal { .. }))
        .count()
}

/// O2,O5 × PD5 × IP-D9 — **GREEN today, must STAY green.** The drain rule is a
/// block-ADMISSION rule and the rebuild replays blocks that are already
/// canonical, so no drain check may leak into `rebuild_producer_set_from_blocks`.
/// If one did, a node that reorgs through the n11 repair block would compute a
/// different `ProducerSet` — and therefore a different `active_list` and
/// scheduler — than one that applied it live. That is the INC-I-078 fork shape.
#[tokio::test]
async fn req_i180_001_rebuild_matches_live_for_the_drain_shape() {
    let (node, node_kp, _t) = make_node().await;
    assert_eq!(
        node.config.network.genesis_blocks(),
        GENESIS_BLOCKS,
        "fixture: devnet genesis_blocks must match the pinned literal"
    );

    let target = KeyPair::generate();
    let tpk = *target.public_key();
    let unit = node.config.network.bond_unit();
    let producer = *node_kp.public_key();

    let mut blocks: Vec<(u64, Block)> = Vec::new();
    for height in 1..=WITHDRAW_H {
        let txs = if height == REG_H {
            vec![Transaction::new_registration(
                Vec::new(),
                tpk,
                unit * u64::from(N11_BONDS),
                u64::MAX,
                N11_BONDS,
            )]
        } else if height == WITHDRAW_H {
            let dest = crypto::hash::hash(b"inc-i-180-drain-destination");
            vec![Transaction::new_request_withdrawal(
                Vec::new(),
                tpk,
                N11_BONDS,
                dest,
                1,
            )]
        } else {
            Vec::new()
        };
        let block = chain_block(&node, height, producer, txs);
        node.block_store
            .put_block_canonical(&block, height)
            .expect("fixture: put_block_canonical");
        blocks.push((height, block));
    }
    node.block_store
        .ensure_blocks_present(1, WITHDRAW_H)
        .expect("fixture: the store must be DENSE over the replay range");

    let mut rebuilt = ProducerSet::new();
    node.rebuild_producer_set_from_blocks(&mut rebuilt, WITHDRAW_H)
        .expect("a dense store must rebuild");
    let live = live_replay(&node, &blocks);

    assert_eq!(
        queued_withdrawals(&rebuilt, &tpk),
        queued_withdrawals(&live, &tpk),
        "O5/PD5: the repair block declares the producer's ENTIRE holding and \
         carries zero Bond inputs. Both replay paths must enqueue it identically. \
         rebuilt={:?} live={:?}",
        rebuilt.pending_updates_for(&tpk),
        live.pending_updates_for(&tpk)
    );
    assert_eq!(
        queued_withdrawals(&live, &tpk),
        1,
        "O5/PD5: sanity — the LIVE reference must actually queue it, otherwise \
         the comparison above is empty-equals-empty"
    );

    let mut rebuilt_flushed = rebuilt;
    let mut live_flushed = live;
    rebuilt_flushed.apply_pending_updates_with_cap(0);
    live_flushed.apply_pending_updates_with_cap(0);
    assert_eq!(
        state_of(&rebuilt_flushed, &tpk),
        state_of(&live_flushed, &tpk),
        "O2/PD5: the tuple that feeds active_list must agree on both paths"
    );
    assert_eq!(
        state_of(&live_flushed, &tpk),
        (0, ProducerStatus::Exited, 0),
        "O2/PD5: and the agreed value is the repaired one — the drain lands on \
         zero and the auto-exit fires, which is what makes the remedy permanent"
    );
}
