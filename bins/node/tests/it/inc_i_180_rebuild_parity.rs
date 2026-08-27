//! INC-I-180 M1 QA-1 — the reorg/rollback replay must queue what live apply
//! queued. Requirements: REQ-I180-001 (Must), REQ-I180-003 (Must).
//!
//! covers: bins/node/src/node/rewards.rs (rebuild_producer_set_from_blocks),
//!         bins/node/src/node/apply_block/tx_processing.rs (live withdrawal arm)
//!
//! ---------------------------------------------------------------------------
//! THE DEFECT THIS FILE REPRODUCES (QA run 525, PROBE F)
//! ---------------------------------------------------------------------------
//! `rebuild_producer_set_from_blocks` is the reorg/rollback replay, reachable
//! from `rollback.rs:152`, `rollback.rs:275`, `block_handling.rs:752` and
//! `block_handling.rs:928`. Its `RequestWithdrawal` arm computed
//! `bond_count - withdrawal_pending_count` with NO pending-AddBond term and no
//! height gate, while the live arm gained one post-activation. Before INC-I-180
//! the two formulas were IDENTICAL; the M1 fix broke that parity, so on the same
//! canonical chain the rebuild queued 0 withdrawals where live queued 1 —
//! divergent `ProducerSet` ⇒ divergent `active_list` ⇒ divergent scheduler ⇒
//! FORK. The rule the drift violates is written into the same function at
//! `rewards.rs` (INC-I-078): mirror the live-apply gate in the rebuild path.
//!
//! WHICH RULES BELONG HERE. INC-I-180 QA-1 adds three rules. Two of them —
//! `Exit` charging the allowance, and binding the declared count to the Bond
//! UTXOs destroyed — are BLOCK-ADMISSION rules: they decide whether a block may
//! enter the chain, and they spend nothing. The rebuild replays blocks that are
//! ALREADY canonical, so re-deciding admission there would be meaningless (and
//! the rebuild's own `Exit` arm already bumps `withdrawal_pending_count` exactly
//! as live does, so it is in parity on that count without any change). Only the
//! third rule — the pending-AddBond term in the ENQUEUE condition — is an
//! APPLY/queue rule, and only that one is mirrored here.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT:
//! ---------------------------------------------------------------------------
//! Function under test:
//!   `Node::rebuild_producer_set_from_blocks(&self, &mut ProducerSet, u64) -> Result<()>`
//! Return is `Result<()>`, so the interesting outputs are receiver-parameter
//! mutations, compared against the LIVE reference built by driving
//! `Node::process_transaction_producer_effects` over the same blocks with the
//! same epoch-flush rule (`state_update.rs`: every block in epoch 0, then at
//! boundaries).
//!
//!   R1: `pending_updates_for(target)` — the queued `RequestWithdrawal` count.
//!       This is where PROBE F measured 0-vs-1.
//!   R2: `withdrawal_pending_count(target)` after the replay.
//!   R3: post-flush `(bond_count, status, selection_weight())` — the quantity
//!       that feeds `active_list` and therefore the scheduler.
//!   R4: `Result` — a dense store must return Ok.
//!   NOT outputs: the block store is read-only here; no side channel.
//!
//! PATHS
//!   PR1: replay of a withdrawal covered ONLY by an in-flight AddBond
//!   PR2: replay of a withdrawal that exceeds holdings with NOTHING in flight
//!
//! INPUT PARTITIONS:
//!   IP-R1  Registration(433) @41, AddBond(1) @45, RequestWithdrawal(434) @46.
//!          The AddBond is queued and unflushed at h=46 (next boundary is 48),
//!          so the request is covered ONLY by the in-flight term.
//!   IP-R2  Registration(433) @41, RequestWithdrawal(434) @46, no AddBond.
//!          `in_flight` is 0 on BOTH sides of the gate, so the historical skip
//!          must survive the fix unchanged.
//!
//! MATRIX (every enumerated cell has an assertion)
//!   R1,R2,R3,R4 × PR1 × IP-R1 → req_i180_001_rebuild_matches_live_with_addbond_in_flight
//!   R1,R3,R4    × PR2 × IP-R2 → req_i180_003_rebuild_keeps_the_legacy_skip_with_nothing_in_flight
//!
//! ---------------------------------------------------------------------------
//! M2 / S5 EXTENSION — AUDIT-P1-004 (the rebuild arm omits the auto-revoke)
//! ---------------------------------------------------------------------------
//! `rewards.rs` contains ZERO occurrences of `delegated_bonds` and its
//! `RequestWithdrawal` arm has no `RevokeDelegation` branch, while the live arm
//! (`apply_block/tx_processing.rs:382-437`) reads `info.delegated_bonds` at :382
//! and queues `PendingProducerUpdate::RevokeDelegation` at :415 (INC-I-058).
//! For `bond_count=10, delegated_bonds=5, RequestWithdrawal(p,10)`: live queues
//! `[RevokeDelegation, RequestWithdrawal]`, rebuild queues only the withdrawal.
//! `set_persistence.rs:82-100` puts `received_delegations` and the whole
//! `ProducerInfo` IN the state root, so after a reorg through such a block the
//! rebuilt and the live node differ in the state root. INC-I-054 shape.
//! (`rewards.rs:1460-1472` is the separate `TxType::RevokeDelegation` arm from
//! the INC-I-078 mirror — NOT the auto-revoke.)
//!
//! Added outputs for that extension:
//!   R5: `delegated_bonds` on the delegator and `received_delegations` on the
//!       delegate, post-flush.
//!   R6: `ProducerSet::serialize_canonical()` — the exact bytes the state root
//!       is computed over.
//! Added path:
//!   PR3: replay of a FULL EXIT whose allowance is blocked by an active
//!        delegation.
//! Added partition:
//!   IP-R3D Registration(A,10) @41, Registration(B,1) @42,
//!          DelegateBond(A→B,5) @43, epoch flush @44,
//!          RequestWithdrawal(A,10) @46.
//!   R1,R5,R6 × PR3 × IP-R3D
//!        → audit_p1_004_rebuild_matches_live_for_a_delegated_full_exit
//!
//! NO PRE-ACTIVATION ROW EXISTS, and that is a property of devnet, not an
//! omission: the devnet gate is pinned to h=20 while the earliest height at
//! which any producer can exist in a rebuilt set is `genesis_blocks + 1` = 41
//! (the replay SKIPS every Registration while `height <= genesis_blocks`). The
//! pre-activation partition of this arm is therefore UNREACHABLE on devnet. The
//! fixture asserts that premise explicitly, so if the devnet gate is ever raised
//! above 41 this file fails and demands the row. IP-R2 covers the invariance
//! that IS reachable: with nothing in flight the new term is 0 and the verdict
//! is identical on both sides of the gate.

use std::collections::HashSet;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::Transaction;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{PendingProducerUpdate, ProducerSet, ProducerStatus, UtxoSet};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

use crate::inc_i_180_common::{make_node, N11_BONDS};

const GENESIS_BLOCKS: u64 = 40;
const EPOCH_LEN: u64 = 4;
const REG_H: u64 = 41;
const ADDBOND_H: u64 = 45;
const WITHDRAW_H: u64 = 46;

/// A chain block whose slot tracks its height, so no two fixture blocks hash
/// the same and the canonical height index stays one-to-one.
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

/// The canonical chain both paths replay: coinbase-only up to the genesis
/// boundary, then a post-genesis registration, an AddBond and a withdrawal.
async fn seed_chain(with_addbond: bool) -> (Node, KeyPair, Vec<(u64, Block)>, TempDir) {
    let (node, node_kp, temp) = make_node().await;
    let params = node.config.network.params();
    assert_eq!(
        node.config.network.genesis_blocks(),
        GENESIS_BLOCKS,
        "fixture: devnet genesis_blocks must match the pinned literal"
    );
    assert_eq!(
        node.config.network.blocks_per_reward_epoch(),
        EPOCH_LEN,
        "fixture: devnet blocks_per_reward_epoch must match the pinned literal"
    );
    assert!(
        params.withdrawal_holdings_gate_activation_height <= GENESIS_BLOCKS + 1,
        "fixture premise: the devnet withdrawal gate ({}) must sit at or below \
         the first height at which a rebuilt producer can exist ({}). If it is \
         raised above that, the pre-activation partition of the rebuild arm \
         becomes REACHABLE on devnet and this file owes it a row.",
        params.withdrawal_holdings_gate_activation_height,
        GENESIS_BLOCKS + 1
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
        } else if height == ADDBOND_H && with_addbond {
            vec![Transaction::new_add_bond(
                Vec::new(),
                tpk,
                1,
                unit,
                u64::MAX,
            )]
        } else if height == WITHDRAW_H {
            let dest = crypto::hash::hash(b"inc-i-180-rebuild-destination");
            vec![Transaction::new_request_withdrawal(
                Vec::new(),
                tpk,
                N11_BONDS + 1,
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

    (node, target, blocks, temp)
}

/// The LIVE reference: `process_transaction_producer_effects` over the same
/// blocks, with the epoch-flush rule `state_update.rs` applies (every block in
/// epoch 0, then at boundaries) and the same per-block unbonding sweep.
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

fn queued_withdrawals(ps: &ProducerSet, pk: &PublicKey) -> usize {
    ps.pending_updates_for(pk)
        .iter()
        .filter(|u| matches!(u, PendingProducerUpdate::RequestWithdrawal { .. }))
        .count()
}

fn state_of(ps: &ProducerSet, pk: &PublicKey) -> (u32, ProducerStatus, u64) {
    ps.get_by_pubkey(pk)
        .map(|i| (i.bond_count, i.status, i.selection_weight()))
        .unwrap_or((0, ProducerStatus::Exited, 0))
}

// ═══════════════════════════════════════════════════════════════════════════
// PR1 / IP-R1 — the withdrawal covered only by an in-flight AddBond
// ═══════════════════════════════════════════════════════════════════════════

/// R1,R2,R3,R4 × PR1 × IP-R1 — **RED before the QA-1 fix** (PROBE F replay).
#[tokio::test]
async fn req_i180_001_rebuild_matches_live_with_addbond_in_flight() {
    let (node, target, blocks, _t) = seed_chain(true).await;
    let tpk = *target.public_key();

    let mut rebuilt = ProducerSet::new();
    node.rebuild_producer_set_from_blocks(&mut rebuilt, WITHDRAW_H)
        .expect("R4: a dense store must rebuild");
    let live = live_replay(&node, &blocks);

    assert_eq!(
        queued_withdrawals(&rebuilt, &tpk),
        queued_withdrawals(&live, &tpk),
        "R1: the rebuild's enqueue condition must accept exactly what live \
         apply accepted. The AddBond at h={ADDBOND_H} is queued and unflushed at \
         h={WITHDRAW_H}, so the 434-bond request is covered ONLY by the \
         pending-AddBond term. A rebuild without that term queues 0 where live \
         queues 1, and a node that reorgs through this range ends up with a \
         different ProducerSet — and therefore a different active_list and \
         scheduler — than one that applied the same blocks live (INC-I-078, \
         compare INC-I-054). rebuilt={:?} live={:?}",
        rebuilt.pending_updates_for(&tpk),
        live.pending_updates_for(&tpk)
    );
    assert_eq!(
        queued_withdrawals(&live, &tpk),
        1,
        "R1: sanity — the LIVE reference must actually queue the withdrawal, \
         otherwise the comparison above is empty-equals-empty"
    );
    assert_eq!(
        rebuilt
            .get_by_pubkey(&tpk)
            .map(|i| i.withdrawal_pending_count),
        live.get_by_pubkey(&tpk).map(|i| i.withdrawal_pending_count),
        "R2: the in-epoch double-withdrawal guard must be charged identically"
    );

    let mut rebuilt_flushed = rebuilt;
    let mut live_flushed = live;
    rebuilt_flushed.apply_pending_updates_with_cap(0);
    live_flushed.apply_pending_updates_with_cap(0);
    assert_eq!(
        state_of(&rebuilt_flushed, &tpk),
        state_of(&live_flushed, &tpk),
        "R3: after the next epoch boundary the two paths must agree on \
         (bond_count, status, selection_weight). This tuple feeds active_list; \
         any drift here IS the fork"
    );
    assert_eq!(
        state_of(&live_flushed, &tpk),
        (0, ProducerStatus::Exited, 0),
        "R3: and the agreed value is the correct one — the retirement lands"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PR2 / IP-R2 — nothing in flight: the historical skip must survive
// ═══════════════════════════════════════════════════════════════════════════

/// R1,R3,R4 × PR2 × IP-R2 — **GREEN today, must STAY green.** With no queued
/// AddBond the new term is 0, so the rebuild's verdict must be identical on
/// both sides of the activation height, and identical to live apply.
#[tokio::test]
async fn req_i180_003_rebuild_keeps_the_legacy_skip_with_nothing_in_flight() {
    let (node, target, blocks, _t) = seed_chain(false).await;
    let tpk = *target.public_key();

    let mut rebuilt = ProducerSet::new();
    node.rebuild_producer_set_from_blocks(&mut rebuilt, WITHDRAW_H)
        .expect("R4: a dense store must rebuild");
    let live = live_replay(&node, &blocks);

    assert_eq!(
        queued_withdrawals(&rebuilt, &tpk),
        0,
        "R1: 434 > 433 with nothing in flight — the request is skipped, exactly \
         as before the fix. Post-activation such a block cannot be canonical at \
         all (validate_block_economics rejects it), so keeping the conservative \
         skip costs nothing and keeps the replay bit-identical below the gate"
    );
    assert_eq!(
        queued_withdrawals(&live, &tpk),
        0,
        "R1: and live apply skips it too — the two paths agree"
    );

    let mut rebuilt_flushed = rebuilt;
    let mut live_flushed = live;
    rebuilt_flushed.apply_pending_updates_with_cap(0);
    live_flushed.apply_pending_updates_with_cap(0);
    assert_eq!(
        state_of(&rebuilt_flushed, &tpk),
        state_of(&live_flushed, &tpk),
        "R3: identical producer state on both paths"
    );
    assert_eq!(
        state_of(&live_flushed, &tpk),
        (N11_BONDS, ProducerStatus::Active, N11_BONDS as u64),
        "R3: the legacy outcome, unchanged — the producer keeps its 433 bonds"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PR3 / IP-R3D — M2 / S5 · AUDIT-P1-004 · the delegated full exit
// ═══════════════════════════════════════════════════════════════════════════

const DELEGATE_REG_H: u64 = 42;
const DELEGATE_H: u64 = 43;
const A_BONDS: u32 = 10;
const DELEGATED: u32 = 5;

/// The canonical chain for the delegated full exit. A registers 10 bonds, B
/// registers 1, A delegates 5 to B, the epoch boundary at h=44 flushes all
/// three, and A then requests a withdrawal of ALL 10 at h=46.
async fn seed_delegation_chain() -> (Node, KeyPair, KeyPair, Vec<(u64, Block)>, TempDir) {
    let (node, node_kp, temp) = make_node().await;
    let unit = node.config.network.bond_unit();
    let producer = *node_kp.public_key();

    let a = KeyPair::generate();
    let b = KeyPair::generate();
    let (apk, bpk) = (*a.public_key(), *b.public_key());

    let mut delegate_data = doli_core::transaction::DelegateBondData::new(apk, bpk, DELEGATED);
    delegate_data.signature =
        crypto::signature::sign_hash(&delegate_data.signing_message(), a.private_key());

    let mut blocks: Vec<(u64, Block)> = Vec::new();
    for height in 1..=WITHDRAW_H {
        let txs = if height == REG_H {
            vec![Transaction::new_registration(
                Vec::new(),
                apk,
                unit * u64::from(A_BONDS),
                u64::MAX,
                A_BONDS,
            )]
        } else if height == DELEGATE_REG_H {
            vec![Transaction::new_registration(
                Vec::new(),
                bpk,
                unit,
                u64::MAX,
                1,
            )]
        } else if height == DELEGATE_H {
            vec![Transaction::new_delegate_bond(delegate_data.clone())]
        } else if height == WITHDRAW_H {
            let dest = crypto::hash::hash(b"inc-i-180-delegated-exit-destination");
            vec![Transaction::new_request_withdrawal(
                Vec::new(),
                apk,
                A_BONDS,
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

    (node, a, b, blocks, temp)
}

fn revoke_count(ps: &ProducerSet, pk: &PublicKey) -> usize {
    ps.pending_updates_for(pk)
        .iter()
        .filter(|u| matches!(u, PendingProducerUpdate::RevokeDelegation { .. }))
        .count()
}

/// R1,R5,R6 × PR3 × IP-R3D — **RED today (AUDIT-P1-004).**
///
/// A reorg through a delegated full exit leaves the rebuilt node with a
/// delegation the live node revoked. `received_delegations` and the whole
/// `ProducerInfo` are inside `serialize_canonical()`, so the two nodes then
/// disagree on the producer-set contribution to the state root.
#[tokio::test]
async fn audit_p1_004_rebuild_matches_live_for_a_delegated_full_exit() {
    let (node, a, b, blocks, _t) = seed_delegation_chain().await;
    let (apk, bpk) = (*a.public_key(), *b.public_key());

    let mut rebuilt = ProducerSet::new();
    node.rebuild_producer_set_from_blocks(&mut rebuilt, WITHDRAW_H)
        .expect("a dense store must rebuild");
    let live = live_replay(&node, &blocks);

    // Harness: the delegation must actually be in effect at the withdrawal, or
    // the auto-revoke branch is never reached and the test proves nothing.
    assert_eq!(
        live.get_by_pubkey(&apk).map(|i| i.delegated_bonds),
        Some(DELEGATED),
        "harness: A must hold {DELEGATED} delegated bonds when the withdrawal is applied"
    );
    assert_eq!(
        rebuilt.get_by_pubkey(&apk).map(|i| i.delegated_bonds),
        Some(DELEGATED),
        "harness: the rebuild must reach the same delegation state before the withdrawal"
    );

    // R1 — the queued-update sets themselves.
    assert_eq!(
        revoke_count(&rebuilt, &apk),
        revoke_count(&live, &apk),
        "AUDIT-P1-004: live apply queues RevokeDelegation for a full exit blocked \
         by an active delegation (tx_processing.rs:415, INC-I-058) and the reorg \
         rebuild does not — rewards.rs has zero occurrences of `delegated_bonds`. \
         rebuilt={:?} live={:?}",
        rebuilt.pending_updates_for(&apk),
        live.pending_updates_for(&apk)
    );
    assert_eq!(
        revoke_count(&live, &apk),
        1,
        "R1: sanity — the LIVE reference must actually queue the auto-revoke, \
         otherwise the comparison above is empty-equals-empty"
    );
    assert_eq!(
        queued_withdrawals(&rebuilt, &apk),
        queued_withdrawals(&live, &apk),
        "R1: both paths must queue the withdrawal itself"
    );

    let mut rebuilt_flushed = rebuilt;
    let mut live_flushed = live;
    rebuilt_flushed.apply_pending_updates_with_cap(0);
    live_flushed.apply_pending_updates_with_cap(0);

    // R5 — the delegation fields the state root carries.
    assert_eq!(
        rebuilt_flushed
            .get_by_pubkey(&apk)
            .map(|i| i.delegated_bonds),
        live_flushed.get_by_pubkey(&apk).map(|i| i.delegated_bonds),
        "R5: the delegator's `delegated_bonds` must agree after the flush"
    );
    assert_eq!(
        rebuilt_flushed
            .get_by_pubkey(&bpk)
            .map(|i| i.received_delegations.clone()),
        live_flushed
            .get_by_pubkey(&bpk)
            .map(|i| i.received_delegations.clone()),
        "R5: the delegate's `received_delegations` must agree after the flush — \
         this field feeds selection_weight and therefore the scheduler"
    );

    // R6 — the exact bytes the state root is computed over.
    assert_eq!(
        rebuilt_flushed.serialize_canonical(),
        live_flushed.serialize_canonical(),
        "R6: `serialize_canonical()` (set_persistence.rs:78-113) is the producer-set \
         contribution to the state root and it carries the full ProducerInfo. A node \
         that reorgs through this block ends up with a different state root than one \
         that applied it live — INC-I-054 shape."
    );
}
