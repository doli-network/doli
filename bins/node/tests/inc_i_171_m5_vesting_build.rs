//! INC-I-171 M5 — the vesting payout bound at the BLOCK BUILDER, and the
//! three-site parity harness (REQ-VEST-008, INV-VEST-007/008/011/013).
//!
//! OUTPUT CONTRACT — `Node::build_block_content(&mut self, Hash, u32, u64, u32,
//! PublicKey) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>`:
//!   O1 return        `Ok(Some(..))`; refusing a tx is a SKIP, never `Err`/`None`
//!   O2 selected list which mempool txs appear in `Vec<Transaction>`, in order
//!   O3 receiver      `node.mempool` membership UNCHANGED by a build
//!   O4 `node.utxo_set` / `node.producer_set` UNMUTATED by a build
//! PATHS: P-BELOW | P-ENFORCED | P-DISABLED | P-SLOT0 (admission skipped, the
//!   builder is the only gate) | P-MALFORMED (Bond `extra_data.len() != 4`).
//! INPUT PARTITIONS: tier Q1/Q2/Q3/Q4; payout at-bound / bound+1 / full raw value.
//! MATRIX: O1,O2 x each path x each partition (one test per cell below);
//!   O3 x P-ENFORCED (the parity harness asserts it per row).
//!
//! TARGET API — the developer replaces the ONE `admit` shim below; everything
//! else compiles against the CURRENT tree, so the M5 probe prints a value on the
//! RED tree as well as the GREEN one. Contract:
//! `docs/.workflow/inc-i-171-m5-test-plan.md`.
//!
//! INV-GOV-001 — the shipped window is `[u64::MAX, u64::MAX)`, empty at every
//! reachable height, so the gate is armed through the ENV OVERRIDE.
//! `NetworkParams` is a per-network `OnceLock` frozen on the first `params()`
//! call in the PROCESS, so this must be its own test binary.
//!
//! HARNESS NOTE — the builder's slot is the WALL CLOCK (`assembly.rs:377`
//! aborts the build if it drifts), so bond ages are centred inside their
//! quarter and every fixture is derived from the observed slot.

use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::vesting::penalized_bond_net;
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use mempool::{Mempool, MempoolPolicy};
use storage::{Outpoint, ProducerSet, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

mod inc_i_171_m5_support;
use inc_i_171_m5_support::{AGE_Q1, AGE_Q2, AGE_Q3, AGE_Q4, ROWS};

static INIT_ENV: Once = Once::new();

fn init_vesting_env() {
    INIT_ENV.call_once(|| {
        std::env::set_var("DOLI_INC_I_171_VESTING_PENALTY_ACTIVATION_HEIGHT", "101");
        std::env::set_var("DOLI_INC_I_171_VESTING_PENALTY_DISABLE_HEIGHT", "201");
    });
}

// All three heights are ODD: devnet runs `blocks_per_reward_epoch = 4`, and an
// epoch-boundary height would drag the reward section into every assertion.
const H_BELOW: u64 = 99;
const H_ENFORCED: u64 = 101;
const H_DISABLED: u64 = 201;

/// Devnet `vesting_quarter_slots`, re-read from params in `assert_armed`.
const QUARTER: u32 = 60;

/// Flushed bonds the ledger credits the producer with. Every fixture declares
/// fewer, which keeps INC-I-180 on its partial-drain branch.
const LEDGER_BONDS: u32 = 5;

// ============================================================
// TARGET API — the one shim the developer retargets
// ============================================================

/// TARGET: `mempool.add_transaction(tx, utxo, height, slot)`.
async fn admit(node: &Node, tx: Transaction, height: u64, slot: u32) -> Result<(), String> {
    let utxo = node.utxo_set.read().await;
    let mut pool = node.mempool.write().await;
    pool.add_transaction(tx, &utxo, height, slot)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ============================================================
// FIXTURE
// ============================================================

async fn make_node() -> (Node, KeyPair, TempDir) {
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    (node, kp, temp)
}

/// SITE (b) on a pool built exactly as the node's own (`init.rs:1218`), so the
/// residency admit stays the node pool's FIRST touch (watermark 0, INV-VEST-011).
async fn scratch_admit(node: &Node, tx: Transaction, height: u64, slot: u32) -> Result<(), String> {
    let utxo = node.utxo_set.read().await;
    let mut pool = Mempool::new(
        MempoolPolicy::testnet(),
        node.params.clone(),
        node.config.network,
    );
    pool.add_transaction(tx, &utxo, height, slot)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn assert_armed(node: &Node) {
    let params = node.config.network.params();
    assert_eq!(
        (
            params.inc_i_171_vesting_penalty_activation_height,
            params.inc_i_171_vesting_penalty_disable_height,
            params.vesting_quarter_slots,
        ),
        (H_ENFORCED, H_DISABLED, QUARTER as u64),
        "the env override must arm the window BEFORE the first params() call in this process"
    );
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

fn now_slot(node: &Node) -> u32 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    node.params.timestamp_to_slot(now)
}

fn unit(node: &Node) -> u64 {
    node.config.network.bond_unit()
}

/// Funds the fee on the tiers whose penalty is zero, so every fixture clears
/// `Transaction::minimum_fee()`.
fn fee_input(node: &Node) -> u64 {
    unit(node) / 100
}

async fn install_ledger(node: &Node, pk: &PublicKey) {
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*pk, LEDGER_BONDS, node.config.network.bond_unit())
        .expect("register_genesis_producer");
    *node.producer_set.write().await = ps;
}

fn entry(output: Output) -> UtxoEntry {
    UtxoEntry {
        output,
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

/// One Bond UTXO per `bonds` entry plus one Normal of `fee_input()`. A direct
/// `UtxoSet` insert bypasses `[ERRTX007]`, which is the snap-sync ingress hole
/// INV-VEST-013 names.
async fn seed(
    node: &Node,
    kp: &KeyPair,
    bonds: &[(u64, u32)],
    malformed_at: Option<usize>,
    tag: u8,
) -> Vec<Input> {
    let pk = *kp.public_key();
    let h = Hash::from_bytes([tag; 32]);
    let fee = fee_input(node);
    let mut inputs = Vec::new();
    let mut utxo = node.utxo_set.write().await;
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
        entry(Output::normal(fee, addr(&pk))),
    )
    .expect("fixture: seed the fee input");
    let mut inp = Input::new(h, index);
    inp.public_key = Some(pk);
    inputs.push(inp);
    inputs
}

fn signed_withdrawal(kp: &KeyPair, inputs: Vec<Input>, declared: u32, payout: u64) -> Transaction {
    let pk = *kp.public_key();
    let dest = crypto::hash::hash(b"inc-i-171-m5-build-destination");
    let mut tx = Transaction::new_request_withdrawal(inputs, pk, declared, dest, payout);
    for i in 0..tx.inputs.len() {
        let signing_hash = tx.signing_message_for_input(i);
        tx.inputs[i].signature = crypto::signature::sign_hash(&signing_hash, kp.private_key());
    }
    tx
}

fn bound_at(node: &Node, bonds: &[(u64, u32)], slot: u32) -> u64 {
    bonds
        .iter()
        .fold(0u64, |acc, (amount, creation)| {
            acc.saturating_add(penalized_bond_net(*amount, *creation, slot, QUARTER))
        })
        .saturating_add(fee_input(node))
}

/// Build one block. A devnet slot-boundary crossing during assembly is a TIMING
/// abort (`Ok(None)`), not a verdict, and is retried. `Err` is never retried —
/// refusing a transaction must be a SKIP.
async fn build_at(node: &mut Node, kp: &KeyPair, height: u64) -> Block {
    let our_pubkey = *kp.public_key();
    for _ in 0..16 {
        let slot = now_slot(node);
        let built = node
            .build_block_content(Hash::ZERO, slot.saturating_sub(1), height, slot, our_pubkey)
            .await
            .expect(
                "O1 / INV-PROD-002: build_block_content returned Err. Refusing an over-paying \
                 withdrawal must be a SKIP (continue) in the selection loop, never a build \
                 failure and never a lost slot.",
            );
        if let Some((header, txs, _bitfield)) = built {
            return Block::new(header, txs);
        }
    }
    panic!("fixture: 16 consecutive slot-boundary aborts while building at h={height}");
}

/// The ordered hashes of the MEMPOOL-sourced transactions of a built block.
/// Coinbase and epoch-reward transactions are not selection decisions.
async fn selected(node: &Node, block: &Block) -> Vec<Hash> {
    let pool = node.mempool.read().await;
    block
        .transactions
        .iter()
        .map(|t| t.hash())
        .filter(|h| pool.contains(h))
        .collect()
}

/// The M4 block-validation verdict for the SAME transaction at the SAME slot.
async fn validation_verdict(
    node: &Node,
    kp: &KeyPair,
    tx: &Transaction,
    height: u64,
    slot: u32,
) -> Result<(), String> {
    let pk = *kp.public_key();
    let reward = node.params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let mut all = vec![Transaction::new_coinbase(reward, pool_hash, height, slot)];
    all.push(tx.clone());
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: doli_core::block::compute_merkle_root(&all),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (slot as u64 * node.params.slot_duration),
        slot,
        producer: pk,
        vdf_output: VdfOutput {
            value: vec![0u8; 32],
        },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    let block = Block::new(header, all);
    node.validate_block_economics(&block, height, ValidationMode::Full)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Accept,
    Reject(String),
}

fn verdict_of(result: Result<(), String>) -> Verdict {
    match result {
        Ok(()) => Verdict::Accept,
        Err(msg) => Verdict::Reject(msg),
    }
}

fn is_accept(v: &Verdict) -> bool {
    matches!(v, Verdict::Accept)
}

// ============================================================
// MUST — REQ-VEST-008: one verdict at all three sites
// ============================================================

// REQ-VEST-008 (Should) / INV-VEST-008 — Decision: reveals two of the three
// sites disagreeing on the same transaction. Builder weaker than the gate means
// a producer deterministically assembles blocks the whole fleet rejects and
// rolls back — a lost slot per occurrence. Gate weaker than the builder means
// the rule does not bind at all. Admission out of step with either censors an
// honest exit or floods every mempool with unspendable withdrawals.
#[tokio::test]
async fn req_vest_008_all_three_sites_reach_the_same_verdict_at_the_activation_height() {
    init_vesting_env();
    let mut disagreements: Vec<String> = Vec::new();

    for row in ROWS.iter() {
        let (mut node, kp, _temp) = make_node().await;
        assert_armed(&node);
        install_ledger(&node, kp.public_key()).await;

        let observed = now_slot(&node);
        let amount = unit(&node);
        let creation = observed - row.age;
        let bonds = [(amount, creation)];
        let bound = bound_at(&node, &bonds, observed);
        let payout = if row.raw_value {
            amount
        } else if row.over >= 0 {
            bound + row.over as u64
        } else {
            bound - fee_input(&node)
        };

        let malformed_at = if row.malformed { Some(0) } else { None };
        let inputs = seed(&node, &kp, &bonds, malformed_at, row.tag).await;
        let tx = signed_withdrawal(&kp, inputs, 1, if row.malformed { 1 } else { payout });
        let tx_hash = tx.hash();

        // SITE (b): admission at the real slot, on a scratch pool.
        let admission = verdict_of(scratch_admit(&node, tx.clone(), H_ENFORCED, observed).await);
        // Residency, INDEPENDENT of that verdict: this is the node pool's first
        // touch, so its watermark is still 0 and the check is skipped there.
        admit(&node, tx.clone(), H_ENFORCED, 0)
            .await
            .expect("fixture: the watermark-0 admission skip must make the transaction resident");
        assert!(
            node.mempool.read().await.contains(&tx_hash),
            "fixture [{}]: the transaction must be resident before the build",
            row.label
        );

        // SITE (c): the builder.
        let block = build_at(&mut node, &kp, H_ENFORCED).await;
        let included = block.transactions.iter().any(|t| t.hash() == tx_hash);
        assert!(
            node.mempool.read().await.contains(&tx_hash),
            "O3 [{}]: build_block_content must not mutate mempool membership",
            row.label
        );

        // SITE (a): block validation, at the slot the builder actually used.
        let validation =
            verdict_of(validation_verdict(&node, &kp, &tx, H_ENFORCED, block.header.slot).await);

        let three = [
            ("validation", is_accept(&validation)),
            ("admission", is_accept(&admission)),
            ("builder", included),
        ];
        if three.iter().any(|(_, v)| *v != row.expect_accept) {
            disagreements.push(format!(
                "[{}] expected all three to {} — validation={:?} admission={:?} builder={}",
                row.label,
                if row.expect_accept {
                    "ACCEPT"
                } else {
                    "REJECT"
                },
                validation,
                admission,
                if included { "INCLUDED" } else { "EXCLUDED" }
            ));
            continue;
        }
        if !row.expect_accept {
            for (site, verdict) in [("validation", &validation), ("admission", &admission)] {
                let Verdict::Reject(msg) = verdict else {
                    continue;
                };
                if !msg.contains(row.expect_code) {
                    disagreements.push(format!(
                        "[{}] {site} rejected with the wrong code: expected {}, got {msg}",
                        row.label, row.expect_code
                    ));
                }
            }
        }
    }

    assert!(
        disagreements.is_empty(),
        "INV-VEST-008: mempool admission, the block builder and block validation must reach ONE \
         verdict for every row.\n{}",
        disagreements.join("\n")
    );
}

// REQ-VEST-008 (Should) — Decision: the milestone's outcome metric. Reveals
// whether the builder packs an unvested full-value withdrawal into a block its
// own Light self-apply then rejects, costing the producer the slot.
#[tokio::test]
async fn req_vest_008_m5_full_value_q1_withdrawal_is_excluded_from_the_built_block() {
    init_vesting_env();
    let (mut node, kp, _temp) = make_node().await;
    assert_armed(&node);
    install_ledger(&node, kp.public_key()).await;

    let observed = now_slot(&node);
    let amount = unit(&node);
    let creation = observed - AGE_Q1;
    let inputs = seed(&node, &kp, &[(amount, creation)], None, 0xB1).await;
    let tx = signed_withdrawal(&kp, inputs, 1, amount);
    let tx_hash = tx.hash();
    // Slot 0: admission SKIPS, so the builder is the only gate left.
    admit(&node, tx, H_ENFORCED, 0)
        .await
        .expect("fixture: the slot-0 admission skip must admit the transaction");

    let block = build_at(&mut node, &kp, H_ENFORCED).await;
    let included = block.transactions.iter().any(|t| t.hash() == tx_hash);
    println!(
        "M5-PROBE-VERDICT-BUILD: {}",
        if included { "INCLUDED" } else { "EXCLUDED" }
    );

    assert!(
        !included,
        "REQ-VEST-008: a {amount} payout against a {} bound must be SKIPPED by the builder",
        bound_at(&node, &[(amount, creation)], block.header.slot)
    );
    assert!(
        !block.transactions.is_empty(),
        "O1: the build must still produce a block — refusal is a skip, not a build failure"
    );
}

// REQ-VEST-008 (Should) / INV-VEST-011 — Decision: reveals the builder trusting
// admission. Admission is skipped at slot 0 and its rule is a strict subset of
// the gate's in every other band, so a builder that only re-checks what
// admission already checked is not a safety gate at all.
#[tokio::test]
async fn req_vest_011_the_builder_excludes_even_when_admission_skipped_at_slot_zero() {
    init_vesting_env();
    let (mut node, kp, _temp) = make_node().await;
    assert_armed(&node);
    install_ledger(&node, kp.public_key()).await;

    let observed = now_slot(&node);
    let amount = unit(&node);
    let creation = observed - AGE_Q1;
    let inputs = seed(&node, &kp, &[(amount, creation)], None, 0xB2).await;
    let over_payout = signed_withdrawal(
        &kp,
        inputs,
        1,
        bound_at(&node, &[(amount, creation)], observed) + 1,
    );
    let hash = over_payout.hash();
    admit(&node, over_payout, H_ENFORCED, 0)
        .await
        .expect("fixture: admission at slot 0 must skip the vesting check and admit");
    assert!(
        node.mempool.read().await.contains(&hash),
        "fixture: the slot-0 skip must leave the transaction resident"
    );

    let block = build_at(&mut node, &kp, H_ENFORCED).await;
    assert!(
        !block.transactions.iter().any(|t| t.hash() == hash),
        "INV-VEST-011: the builder is the safety gate — a resident over-paying withdrawal that \
         admission never evaluated must still be skipped"
    );
}

// REQ-VEST-008 (Should) / INV-VEST-013 — Decision: reveals the builder reusing a
// fail-OPEN creation-slot default and packing a snap-synced malformed Bond as
// though it were fully vested.
#[tokio::test]
async fn req_vest_013_the_builder_excludes_a_malformed_bond_withdrawal() {
    init_vesting_env();
    let (mut node, kp, _temp) = make_node().await;
    assert_armed(&node);
    install_ledger(&node, kp.public_key()).await;

    let observed = now_slot(&node);
    let creation = observed - AGE_Q1;
    let inputs = seed(&node, &kp, &[(unit(&node), creation)], Some(0), 0xB3).await;
    // A payout of 1 is below ANY plausible bound: only a fail-CLOSED rule skips it.
    let tx = signed_withdrawal(&kp, inputs, 1, 1);
    let hash = tx.hash();
    admit(&node, tx, H_ENFORCED, 0)
        .await
        .expect("fixture: admission at slot 0 must skip and admit");

    let block = build_at(&mut node, &kp, H_ENFORCED).await;
    assert!(
        !block.transactions.iter().any(|t| t.hash() == hash),
        "INV-VEST-013: an undecodable Bond creation slot must make the builder skip the \
         withdrawal, never treat it as fully vested"
    );
}

// REQ-VEST-008 (Should) — Decision: reveals over-rejection at the builder. A
// builder that skips honest exits censors every producer trying to leave, which
// is a worse outcome than the unenforced rule it replaces.
#[tokio::test]
async fn req_vest_008_the_builder_includes_an_honest_withdrawal_at_every_tier() {
    init_vesting_env();
    for (tag, age) in [
        (0xC1u8, AGE_Q1),
        (0xC2, AGE_Q2),
        (0xC3, AGE_Q3),
        (0xC4, AGE_Q4),
    ] {
        let (mut node, kp, _temp) = make_node().await;
        assert_armed(&node);
        install_ledger(&node, kp.public_key()).await;

        let observed = now_slot(&node);
        let amount = unit(&node);
        let creation = observed - age;
        let bonds = [(amount, creation)];
        let payout = bound_at(&node, &bonds, observed) - fee_input(&node);
        let inputs = seed(&node, &kp, &bonds, None, tag).await;
        let tx = signed_withdrawal(&kp, inputs, 1, payout);
        let hash = tx.hash();
        admit(&node, tx, H_ENFORCED, observed)
            .await
            .expect("REQ-VEST-008: an honest withdrawal must be admitted");

        let block = build_at(&mut node, &kp, H_ENFORCED).await;
        assert!(
            block.transactions.iter().any(|t| t.hash() == hash),
            "REQ-VEST-008 (age {age}): an honest payout of {payout} must be INCLUDED"
        );
    }
}

// ============================================================
// MUST — INV-VEST-007: nothing changes outside the window
// ============================================================

// REQ-VEST-008 (Should) / INV-VEST-007 — Decision: reveals the builder's
// selection changing at a height where the rule is dormant. Below the
// activation height every node still accepts a full-value withdrawal, so a
// builder that skips it there silently censors on the pre-activation chain
// while producing a block-content difference no activation height covers.
#[tokio::test]
async fn req_vest_007_outside_the_window_the_selected_list_is_identical() {
    init_vesting_env();
    let (mut node, kp, _temp) = make_node().await;
    assert_armed(&node);
    install_ledger(&node, kp.public_key()).await;

    let observed = now_slot(&node);
    let amount = unit(&node);
    let creation = observed - AGE_Q1;
    let inputs = seed(&node, &kp, &[(amount, creation)], None, 0xD1).await;
    let tx = signed_withdrawal(&kp, inputs, 1, amount);
    let hash = tx.hash();
    admit(&node, tx, H_BELOW, observed)
        .await
        .expect("INV-VEST-007: below the activation height admission must be unchanged");

    let below = build_at(&mut node, &kp, H_BELOW).await;
    let below_list = selected(&node, &below).await;
    let disabled = build_at(&mut node, &kp, H_DISABLED).await;
    let disabled_list = selected(&node, &disabled).await;

    assert!(
        below_list.contains(&hash),
        "INV-VEST-007: below the activation height the full-value withdrawal must still be \
         selected; selected list was {below_list:?}"
    );
    assert_eq!(
        below_list, disabled_list,
        "INV-VEST-007: the window is [{H_ENFORCED}, {H_DISABLED}); the selected list at \
         h={H_BELOW} and at h={H_DISABLED} must be identical"
    );
}

// ============================================================
// MUST — INV-VEST-008 backstop: one predicate, three sites
// ============================================================

// REQ-VEST-008 (Should) / INV-VEST-008 — Decision: reveals a fourth copy of the
// bound expression appearing at one of the three sites. Behavioural parity
// tests only sample the input space; a hand-copied predicate drifts on the row
// nobody wrote — the INC-I-180 `allowance_with` lesson and the Full Bitfield
// Decode pillar are the same failure twice.
#[test]
fn req_vest_008_all_three_sites_reference_one_shared_predicate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let sites = [
        root.join("bins/node/src/node/validation_checks/withdrawal_economics.rs"),
        root.join("crates/mempool/src/pool.rs"),
        root.join("bins/node/src/node/production/withdrawal_holdings.rs"),
    ];
    let shared = ["check_withdrawal_payout_bound", "vesting_bound_verdict"];

    let mut missing: Vec<PathBuf> = Vec::new();
    for path in sites.iter() {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if !shared.iter().any(|symbol| text.contains(symbol)) {
            missing.push(path.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "INV-VEST-008: block validation, mempool admission and the block builder must each \
         reference one of {shared:?}. These do not: {missing:?}"
    );

    // Non-vacuous: the shared predicate must not be re-derived. No site may
    // compute a penalty percentage of its own.
    for path in sites.iter() {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            !text.contains("withdrawal_penalty_rate"),
            "INV-VEST-008: {path:?} re-derives the vesting ladder instead of calling the shared \
             predicate"
        );
    }
}

// REQ-VEST-008 (Should) — Decision: reveals the wrapper reading a wall clock or
// its own height instead of the slot and gate it is handed, which is how two
// honest nodes come to disagree on the same transaction.
#[test]
fn req_vest_008_the_shared_wrapper_reads_no_wall_clock() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let path = root.join("crates/mempool/src/vesting_bound.rs");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("REQ-VEST-008: the shared wrapper {path:?} must exist ({e})");
    });
    for banned in ["SystemTime", "Instant", "now("] {
        assert!(
            !text.contains(banned),
            "REQ-VEST-008: {path:?} must not read a wall clock; found `{banned}`"
        );
    }
    for required in [
        "check_withdrawal_payout_bound",
        "WithdrawalBondExtraDataMalformed",
    ] {
        assert!(
            text.contains(required),
            "REQ-VEST-008: {path:?} must reference `{required}`"
        );
    }
}

// ============================================================
// SHOULD — the builder never loses a slot
// ============================================================

// REQ-VEST-008 (Should) — Decision: reveals a refusal implemented as a bail
// rather than a `continue`. INV-PROD-002: a skipped transaction must cost the
// producer nothing; a build error costs it the whole slot, which is a strictly
// worse outcome than packing the bad transaction.
#[tokio::test]
async fn req_vest_008_a_refused_withdrawal_never_fails_the_build() {
    init_vesting_env();
    let (mut node, kp, _temp) = make_node().await;
    assert_armed(&node);
    install_ledger(&node, kp.public_key()).await;

    let observed = now_slot(&node);
    let amount = unit(&node);
    let creation = observed - AGE_Q1;
    let inputs = seed(&node, &kp, &[(amount, creation)], None, 0xE1).await;
    let tx = signed_withdrawal(&kp, inputs, 1, amount);
    admit(&node, tx, H_ENFORCED, 0)
        .await
        .expect("fixture: slot-0 admission must admit");

    let block = build_at(&mut node, &kp, H_ENFORCED).await;
    // The coinbase is the input-less transaction the builder inserts at index 0.
    let coinbase = block
        .transactions
        .first()
        .expect("O1: the build must still produce a non-empty block");
    assert!(
        coinbase.inputs.is_empty() && !coinbase.outputs.is_empty(),
        "O1: the block must still carry its coinbase after the withdrawal was skipped"
    );
}
