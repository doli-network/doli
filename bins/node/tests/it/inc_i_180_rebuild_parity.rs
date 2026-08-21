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
