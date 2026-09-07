//! INC-I-171 M4 — the vesting payout bound at block validation.
//!
//! OUTPUT CONTRACT — `Node::validate_block_economics(&self, block, height, mode)`:
//!   O1 return       `Ok(())` | `Err(anyhow)` whose message carries a `[CODE]` prefix
//!   O2 `utxo_set`    must be UNMUTATED (the validator is read-only)
//!   O3 `producer_set` must be UNMUTATED
//!   O4 `chain_state`  must be UNMUTATED
//!   O5 `warn!` log   `[REPLAY_SKIP]` under `ValidationMode::Replay`
//! PATHS: P-BELOW (h < activation) | P-ENFORCED | P-DISABLED (h >= disable) |
//!   P-REPLAY | P-MALFORMED (Bond `extra_data.len() != 4`) | P-PRECEDENCE (an
//!   INC-I-180 rule already failing on the same tx).
//! INPUT PARTITIONS: tier Q1/Q2/Q3/Q4; payout at-bound / bound+1 / under-bound /
//!   full-raw-value; Bond-only inputs / mixed Bond + non-Bond.
//! MATRIX: O1 x each path x each partition (one test per cell below);
//!   O2-O4 x P-ENFORCED x full-value (`req_vest_005_rejection_leaves_state_byte_identical`);
//!   O1,O5 x P-REPLAY (`req_vest_005_replay_is_warn_only_where_full_and_light_reject`).
//!
//! INV-GOV-001 — this binary pins the gate through the ENV OVERRIDE rather than
//! deriving it from shipped params. The shipped window is `[u64::MAX, u64::MAX)`,
//! which is EMPTY at every reachable height, so no derived height can exercise
//! the enforcing side at all. The shipped-params side is already pinned by
//! `crates/core/tests/it/inc_i_171_m2_activation_height.rs::
//! req_vest_003_both_vesting_gates_are_frozen_on_every_network`, and the
//! params -> call-site binding by `req_vest_001_call_site_reads_both_gate_params`
//! below. `NetworkParams` is a per-network `OnceLock` frozen on the first
//! `params()` call in the PROCESS, so this must be its own test binary.

use std::path::{Path, PathBuf};
use std::sync::Once;

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::node::Node;
use storage::{Outpoint, ProducerSet, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

static INIT_ENV: Once = Once::new();

/// Called FIRST by every test, before any `Node` or `params()` touch. All tests
/// set identical values, so the `Once` race is benign.
fn init_vesting_env() {
    INIT_ENV.call_once(|| {
        std::env::set_var("DOLI_INC_I_171_VESTING_PENALTY_ACTIVATION_HEIGHT", "101");
        std::env::set_var("DOLI_INC_I_171_VESTING_PENALTY_DISABLE_HEIGHT", "201");
    });
}

// Bands. All three heights are ODD on purpose: devnet runs
// `blocks_per_reward_epoch = 4`, and an epoch-boundary height would drag the
// epoch-reward section (which runs AFTER the withdrawal pass) into every ACCEPT
// assertion. The devnet INC-I-180 gate is 20, so it is active in all three bands.
const H_BELOW: u64 = 99;
const H_ENFORCED: u64 = 101;
const H_DISABLED: u64 = 201;

const SLOT: u32 = 1_234;

// Tiers at SLOT=1234 with devnet `vesting_quarter_slots = 60`:
//   age = 1234 - creation_slot; quarters = age / 60; 0=>75% 1=>50% 2=>25% 3+=>0%
//   penalty = amount * pct / 100 (truncating), net = amount - penalty.
//   Q1 101 @75% -> penalty  75 -> net  26     Q2 203 @50% -> penalty 101 -> net 102
//   Q3 307 @25% -> penalty  76 -> net 231     Q4 409 @ 0% -> penalty   0 -> net 409
const Q1: (u64, u32, u64) = (101, 1_224, 26);
const Q2: (u64, u32, u64) = (203, 1_164, 102);
const Q3: (u64, u32, u64) = (307, 1_104, 231);
const Q4: (u64, u32, u64) = (409, 1_034, 409);

/// Ledger holdings. Every fixture declares FEWER bonds than this, which puts
/// INC-I-180 on its partial-drain branch (`declared == Bond inputs`) and keeps
/// the exact-drain rule out of the way.
const LEDGER_BONDS: u32 = 5;

const PAYOUT_EXCEEDS: &str = "[ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET]";
const EXTRA_DATA_MALFORMED: &str = "[ECON_WITHDRAWAL_BOND_EXTRA_DATA_MALFORMED]";
const COUNT_MISMATCH: &str = "[ECON_WITHDRAWAL_BOND_COUNT_MISMATCH]";

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

/// Harness integrity. Without this, "nothing rejected" reads the same whether
/// the gate is unimplemented or the env override never reached `NetworkParams`,
/// and the green phase would be a false pass.
fn assert_armed(node: &Node) {
    let params = node.config.network.params();
    assert_eq!(
        (
            params.inc_i_171_vesting_penalty_activation_height,
            params.inc_i_171_vesting_penalty_disable_height
        ),
        (H_ENFORCED, H_DISABLED),
        "the env override must arm the window BEFORE the first params() call in this process"
    );
}

fn addr(pk: &PublicKey) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, pk.as_bytes())
}

async fn install_ledger(node: &Node, pk: &PublicKey) {
    let mut ps = ProducerSet::new();
    ps.register_genesis_producer(*pk, LEDGER_BONDS, node.config.network.bond_unit())
        .expect("register_genesis_producer");
    *node.producer_set.write().await = ps;
}

fn utxo_entry(output: Output) -> UtxoEntry {
    UtxoEntry {
        output,
        height: 1,
        is_coinbase: false,
        is_epoch_reward: false,
    }
}

/// Seed one pre-block UTXO per entry and return the matching inputs: the
/// `(amount, creation_slot)` Bonds first, then the non-Bond Normal amounts.
/// `malformed_at` truncates that Bond's `extra_data` to 3 bytes — a direct
/// `UtxoSet` insert bypasses `[ERRTX007]`, which is exactly the snap-sync
/// ingress hole INV-VEST-013 names.
async fn seed_inputs(
    node: &Node,
    pk: &PublicKey,
    bonds: &[(u64, u32)],
    non_bonds: &[u64],
    malformed_at: Option<usize>,
    tag: u8,
) -> Vec<Input> {
    let pkh = addr(pk);
    let h = Hash::from_bytes([tag; 32]);
    let mut inputs = Vec::new();
    let mut utxo = node.utxo_set.write().await;
    for (i, (amount, creation_slot)) in bonds.iter().enumerate() {
        let mut output = Output::bond(*amount, pkh, u64::MAX, *creation_slot);
        if malformed_at == Some(i) {
            output.extra_data = vec![0u8; 3];
        }
        let index = inputs.len() as u32;
        utxo.insert(Outpoint::new(h, index), utxo_entry(output))
            .expect("seed Bond UTXO");
        inputs.push(Input::new(h, index));
    }
    for amount in non_bonds {
        let index = inputs.len() as u32;
        utxo.insert(
            Outpoint::new(h, index),
            utxo_entry(Output::normal(*amount, pkh)),
        )
        .expect("seed Normal UTXO");
        inputs.push(Input::new(h, index));
    }
    inputs
}

fn withdrawal(inputs: Vec<Input>, pk: &PublicKey, declared: u32, payout: u64) -> Transaction {
    let dest = crypto::hash::hash(b"inc-i-171-m4-withdrawal-destination");
    Transaction::new_request_withdrawal(inputs, *pk, declared, dest, payout)
}

fn block_with(node: &Node, height: u64, producer: PublicKey, txs: Vec<Transaction>) -> Block {
    let reward = node.params.block_reward(height);
    let pool_hash = doli_core::consensus::reward_pool_pubkey_hash();
    let mut all = vec![Transaction::new_coinbase(reward, pool_hash, height, SLOT)];
    all.extend(txs);
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: doli_core::block::compute_merkle_root(&all),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (SLOT as u64 * node.params.slot_duration),
        slot: SLOT,
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

async fn verdict_in_mode(
    node: &Node,
    pk: &PublicKey,
    txs: Vec<Transaction>,
    height: u64,
    mode: ValidationMode,
) -> Result<(), String> {
    install_ledger(node, pk).await;
    let block = block_with(node, height, *pk, txs);
    node.validate_block_economics(&block, height, mode)
        .await
        .map_err(|e| e.to_string())
}

async fn verdict(
    node: &Node,
    pk: &PublicKey,
    txs: Vec<Transaction>,
    height: u64,
) -> Result<(), String> {
    verdict_in_mode(node, pk, txs, height, ValidationMode::Full).await
}

/// The three states CLAUDE.md names, hashed by the production canonical
/// encoder. Cheaper than diffing the sets and strictly more faithful: a
/// mutation a summary tuple would round off still moves this hash.
async fn state_root(node: &Node) -> Hash {
    storage::compute_state_root(
        &*node.chain_state.read().await,
        &*node.utxo_set.read().await,
        &*node.producer_set.read().await,
    )
    .expect("compute_state_root")
}

// ============================================================
// SOURCE SCAN (tripwires)
// ============================================================

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Read at RUNTIME, never `include_str!`: a missing file must fail one
/// assertion, not the compile of the whole binary.
/// Returns (the file defining `validate_block_economics`, the files calling the predicate).
fn call_site_scan() -> (Option<PathBuf>, Vec<(PathBuf, String)>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/node");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    files.sort();
    let (mut host, mut sites) = (None, Vec::new());
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.contains("pub async fn validate_block_economics") {
            host = Some(path.clone());
        }
        if text.contains("check_withdrawal_payout_bound") {
            sites.push((path, text));
        }
    }
    (host, sites)
}

// ============================================================
// MUST — REQ-VEST-001
// ============================================================

// REQ-VEST-001 (Must) — Decision: a failure here means a bonded producer can still
// convert an unvested Q1 bond into full-value spendable coin at consensus, which is
// the INC-I-171 loss itself and would say the gate never reached the block path.
#[tokio::test]
async fn req_vest_001_m4_full_value_q1_withdrawal_is_rejected_at_activation() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    assert_armed(&node);

    // 3 Q1 bonds: raw total 3*101 = 303, bound 3*26 = 78. The payout takes the RAW total.
    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xA1).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    let enforced = verdict(&node, &pk, vec![tx.clone()], H_ENFORCED).await;
    println!(
        "M4-PROBE-VERDICT: {}",
        match &enforced {
            Ok(()) => "ACCEPTED".to_string(),
            Err(e) if e.contains(PAYOUT_EXCEEDS) =>
                "REJECTED(ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET)".to_string(),
            Err(e) => format!("REJECTED(other): {e}"),
        }
    );
    let msg = enforced.expect_err(
        "REQ-VEST-001: a 303-unit payout against a 78-unit vested bound must be rejected \
         at the activation height",
    );
    assert!(
        msg.contains(PAYOUT_EXCEEDS),
        "REQ-VEST-001: expected {PAYOUT_EXCEEDS}, got: {msg}"
    );

    assert_eq!(
        verdict(&node, &pk, vec![tx], H_BELOW).await,
        Ok(()),
        "INV-VEST-007: one height below activation the very same block must still be accepted"
    );
}

// REQ-VEST-001 (Must) — Decision: reveals an off-by-one or a net-first rounding
// order in the node bound, either of which silently rejects honest CLI withdrawals
// or silently admits one unit of unvested value per bond.
#[tokio::test]
async fn req_vest_001_bound_is_exact_at_every_tier_and_rejects_one_unit_over() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();

    for (tag, label, (amount, creation_slot, net)) in [
        (0xB1u8, "Q1 101@75%", Q1),
        (0xB2, "Q2 203@50%", Q2),
        (0xB3, "Q3 307@25%", Q3),
        (0xB4, "Q4 409@0%", Q4),
    ] {
        let inputs = seed_inputs(&node, &pk, &[(amount, creation_slot)], &[], None, tag).await;
        let at_bound = withdrawal(inputs.clone(), &pk, 1, net);
        let one_over = withdrawal(inputs, &pk, 1, net + 1);

        for height in [H_BELOW, H_ENFORCED] {
            assert_eq!(
                verdict(&node, &pk, vec![at_bound.clone()], height).await,
                Ok(()),
                "REQ-VEST-009 {label}: honest CLI payout {net} must be accepted at height {height}"
            );
        }
        assert_eq!(
            verdict(&node, &pk, vec![one_over.clone()], H_BELOW).await,
            Ok(()),
            "INV-VEST-007 {label}: below activation payout {} must still be accepted",
            net + 1
        );
        let msg = verdict(&node, &pk, vec![one_over], H_ENFORCED)
            .await
            .expect_err("REQ-VEST-001: one unit over the bound must be rejected");
        assert!(
            msg.contains(PAYOUT_EXCEEDS),
            "REQ-VEST-001 {label}: payout {} against bound {net}, expected {PAYOUT_EXCEEDS}, \
             got: {msg}",
            net + 1
        );
    }
}

// REQ-VEST-001 (Must) — Decision: a failure means the rule forces an EXACT payout,
// so a producer choosing to burn more than the penalty would have its honest block
// rejected.
#[tokio::test]
async fn req_vest_001_over_burn_below_the_bound_is_accepted() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();

    // One Q1 bond: bound 26. Paying out 10 burns 16 more than the rule demands.
    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1)], &[], None, 0xC1).await;
    let tx = withdrawal(inputs, &pk, 1, 10);

    assert_eq!(
        verdict(&node, &pk, vec![tx], H_ENFORCED).await,
        Ok(()),
        "REQ-VEST-001: the bound is `payout <= net`, so over-burning must be accepted"
    );
}

// REQ-VEST-001 (Must) — Decision: reveals the bound failing OPEN on an undecodable
// Bond slot (INV-VEST-013), which would treat a snap-synced malformed Bond as fully
// vested and hand back 100% of it.
#[tokio::test]
async fn req_vest_001_malformed_bond_extra_data_fails_closed() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();

    // Two Bonds; the second carries 3 bytes of `extra_data`. The payout of 1 is
    // below ANY plausible bound, so only a fail-CLOSED rule can reject this block.
    let inputs = seed_inputs(
        &node,
        &pk,
        &[(Q1.0, Q1.1), (Q4.0, Q4.1)],
        &[],
        Some(1),
        0xD1,
    )
    .await;
    let tx = withdrawal(inputs, &pk, 2, 1);

    let msg = verdict(&node, &pk, vec![tx.clone()], H_ENFORCED)
        .await
        .expect_err("INV-VEST-013: an undecodable Bond creation slot must invalidate the block");
    assert!(
        msg.contains(EXTRA_DATA_MALFORMED),
        "REQ-VEST-001: expected {EXTRA_DATA_MALFORMED}, got: {msg}"
    );

    assert_eq!(
        verdict(&node, &pk, vec![tx], H_BELOW).await,
        Ok(()),
        "INV-VEST-007: below activation a malformed Bond must not change the verdict"
    );
}

// REQ-VEST-001 (Must) — Decision: reveals the bound dropping or double-counting the
// non-Bond input value, which would reject honest change-carrying withdrawals or let
// a bond ride in unpenalized behind a coin input.
#[tokio::test]
async fn req_vest_001_mixed_bond_and_non_bond_inputs_sum_into_one_bound() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();

    // Q1 101 -> net 26, Q3 307 -> net 231, plus a 1_000 Normal input carried whole.
    // bound = 26 + 231 + 1_000 = 1_257.
    const BOUND: u64 = 1_257;
    let inputs = seed_inputs(
        &node,
        &pk,
        &[(Q1.0, Q1.1), (Q3.0, Q3.1)],
        &[1_000],
        None,
        0xE1,
    )
    .await;
    let at_bound = withdrawal(inputs.clone(), &pk, 2, BOUND);
    let one_over = withdrawal(inputs, &pk, 2, BOUND + 1);

    assert_eq!(
        verdict(&node, &pk, vec![at_bound], H_ENFORCED).await,
        Ok(()),
        "REQ-VEST-001: bound must be sum(penalized bond nets) + non-Bond input value = {BOUND}"
    );
    let msg = verdict(&node, &pk, vec![one_over], H_ENFORCED)
        .await
        .expect_err("REQ-VEST-001: one unit over the mixed bound must be rejected");
    assert!(
        msg.contains(PAYOUT_EXCEEDS),
        "REQ-VEST-001: expected {PAYOUT_EXCEEDS}, got: {msg}"
    );
}

// REQ-VEST-001 (Must) — Decision: reveals the predicate being placed above the
// INC-I-180 count rules, which would change the error a node reports for blocks that
// already violate the shipped gate and split peers on the scoring reason.
#[tokio::test]
async fn req_vest_001_inc_i_180_count_error_still_wins_over_the_vesting_bound() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();

    // Two Bond UTXOs seeded, THREE declared: INC-I-180's partial-drain rule already
    // fails. The 303 payout also exceeds the 52 vested bound, so both rules apply.
    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 2], &[], None, 0xF1).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    let msg = verdict(&node, &pk, vec![tx], H_ENFORCED)
        .await
        .expect_err("both the INC-I-180 count rule and the vesting bound reject this block");
    assert!(
        msg.contains(COUNT_MISMATCH),
        "REQ-VEST-001: the shipped INC-I-180 rule must report first, expected \
         {COUNT_MISMATCH}, got: {msg}"
    );
    assert!(
        !msg.contains(PAYOUT_EXCEEDS),
        "REQ-VEST-001: the vesting predicate must run AFTER the INC-I-180 counts, got: {msg}"
    );
}

// REQ-VEST-001 (Must) — Decision: reveals the call site reading a borrowed or
// hard-coded height instead of its own two `NetworkParams` fields, which is how
// INC-I-054 deactivated live features by moving somebody else's gate.
#[test]
fn req_vest_001_call_site_reads_both_gate_params() {
    let (host, sites) = call_site_scan();
    assert_eq!(
        sites.len(),
        1,
        "REQ-VEST-001: exactly ONE file under bins/node/src/node must call \
         `check_withdrawal_payout_bound`; found {:?}",
        sites.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
    let (path, text) = &sites[0];
    assert_ne!(
        Some(path),
        host.as_ref(),
        "REQ-VEST-001: the withdrawal pass must be extracted to a SIBLING module, not left \
         in the file that defines `validate_block_economics` ({path:?})"
    );
    for param in [
        "inc_i_171_vesting_penalty_activation_height",
        "inc_i_171_vesting_penalty_disable_height",
    ] {
        assert!(
            text.contains(param),
            "REQ-VEST-001: {path:?} must read `{param}` from NetworkParams"
        );
    }
}

// ============================================================
// MUST — REQ-VEST-005
// ============================================================

// REQ-VEST-005 (Must) — Decision: reveals the rejection happening AFTER a write,
// which would leave a node that rejected a block holding state a node that never saw
// it does not have — a silent state-root fork at the next epoch boundary.
#[tokio::test]
async fn req_vest_005_rejection_leaves_state_byte_identical() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();

    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xA5).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);
    install_ledger(&node, &pk).await;

    // `apply_block/mod.rs:113` calls this validator BEFORE any state mutation, so
    // the validator's own read-only-ness is the whole of the pre-mutation claim.
    let before = state_root(&node).await;
    let block = block_with(&node, H_ENFORCED, pk, vec![tx]);
    let outcome = node
        .validate_block_economics(&block, H_ENFORCED, ValidationMode::Full)
        .await;
    let after = state_root(&node).await;

    let msg = outcome
        .expect_err("REQ-VEST-005: the full-value Q1 withdrawal must be rejected here")
        .to_string();
    assert!(
        msg.contains(PAYOUT_EXCEEDS),
        "REQ-VEST-005: expected {PAYOUT_EXCEEDS}, got: {msg}"
    );
    assert_eq!(
        before, after,
        "REQ-VEST-005: ChainState + UtxoSet + ProducerSet must be byte-identical after a \
         rejected block"
    );
}

// REQ-VEST-005 (Must) — Decision: reveals the predicate running strict in Replay,
// which would make a node reject its OWN historical chain on restart and wedge it
// below the activation height forever.
#[tokio::test]
async fn req_vest_005_replay_is_warn_only_where_full_and_light_reject() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();

    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xA6).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    for mode in [ValidationMode::Full, ValidationMode::Light] {
        let msg = verdict_in_mode(&node, &pk, vec![tx.clone()], H_ENFORCED, mode)
            .await
            .expect_err("REQ-VEST-005: Full and Light must both reject");
        assert!(
            msg.contains(PAYOUT_EXCEEDS),
            "REQ-VEST-005 {mode:?}: expected {PAYOUT_EXCEEDS}, got: {msg}"
        );
    }
    assert_eq!(
        verdict_in_mode(&node, &pk, vec![tx], H_ENFORCED, ValidationMode::Replay).await,
        Ok(()),
        "REQ-VEST-005: Replay reads a degraded pre-block UTXO view and must warn, not reject"
    );
}

// ============================================================
// MUST — REQ-VEST-007
// ============================================================

// REQ-VEST-007 (Must) — Decision: reveals a wall-clock read in the bound, which makes
// two honest nodes disagree on the same block purely by when they validated it — a
// non-deterministic consensus rule, the worst class of fork.
#[test]
fn req_vest_007_bound_module_reads_no_wall_clock() {
    let (_, sites) = call_site_scan();
    assert_eq!(
        sites.len(),
        1,
        "REQ-VEST-007: exactly ONE file under bins/node/src/node must call \
         `check_withdrawal_payout_bound`; found {:?}",
        sites.iter().map(|(p, _)| p).collect::<Vec<_>>()
    );
    let (path, text) = &sites[0];
    for banned in ["SystemTime", "Instant", "now("] {
        assert!(
            !text.contains(banned),
            "REQ-VEST-007: {path:?} must not read a wall clock; found `{banned}`"
        );
    }
    assert!(
        text.contains("header.slot"),
        "REQ-VEST-007: {path:?} must age bonds from the BLOCK's own `block.header.slot`"
    );
}

// ============================================================
// SHOULD — REQ-VEST-009
// ============================================================

// REQ-VEST-009 (Should) — Decision: reveals the gate changing verdicts OUTSIDE its own
// window, which breaks replay of the pre-activation chain (INV-VEST-007) or keeps
// enforcing after an emergency disable height has been crossed.
#[tokio::test]
async fn req_vest_009_below_activation_and_at_disable_the_verdict_is_unchanged() {
    init_vesting_env();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();

    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xB9).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    for height in [H_BELOW, H_DISABLED] {
        assert_eq!(
            verdict(&node, &pk, vec![tx.clone()], height).await,
            Ok(()),
            "REQ-VEST-009: the window is [101, 201); at height {height} the full-value \
             withdrawal must be accepted"
        );
    }
}
