//! INC-I-171 M6 — the observe-only vesting SHADOW below the activation height.
//!
//! REQ-VEST-011 (Must). INV-VEST-007 is the hard constraint: below
//! `inc_i_171_vesting_penalty_activation_height` every block verdict, state root
//! and builder selection stays byte-identical to the prior binary. The shadow is
//! allowed to move two counters and nothing else.
//!
//! OUTPUT CONTRACT — the M6 shadow arm of `Node::check_withdrawal_economics`,
//! reached through `Node::validate_block_economics(&self, block, height, mode)`:
//!   O1 return                      `Ok(())` | `Err(anyhow)` carrying `[CODE]`
//!   O2 `VESTING_SHADOW_EVALUATED`  +1 per RequestWithdrawal the shadow evaluated
//!   O3 `VESTING_WOULD_REJECT{code}` +1 per shadow `Err`, `code` taken from
//!                                  `ValidationError::error_code()`
//!   O4 rendered `/metrics` text    both series present in `REGISTRY.gather()`
//!   O5 `utxo_set`                  UNMUTATED
//!   O6 `producer_set`              UNMUTATED
//!   O7 `chain_state`               UNMUTATED
//!   (the `info!` line is O8, not asserted: no subscriber here, same facts as O2/O3)
//! PATHS: P-SHADOW (Full, `height < vesting_ah`, at/above the INC-I-180 gate) |
//!   P-SHADOW-BELOW-BOTH (Full, below that gate too) | P-ENFORCED (`height >=
//!   vesting_ah`: the live gate rejects, the shadow must not run) | P-LIGHT |
//!   P-REPLAY | P-RENDER (the exposition bytes an operator scrapes).
//! INPUT PARTITIONS: payout full raw value / at the Q1 bound; Bond `extra_data`
//!   well-formed / malformed; declared bond count == seeded / > seeded.
//! MATRIX: O1,O2,O3 x P-SHADOW (full value | at bound), x P-ENFORCED, x P-LIGHT,
//!   x P-REPLAY, x P-SHADOW-BELOW-BOTH; O2,O3,O4 x P-RENDER; O1,O5,O6,O7 x
//!   P-SHADOW over three txs; O3 label universe x source + rendered exposition.
//!   One test per cell, named `req_vest_011_*` in that order.
//!
//! Every counter assertion is a DELTA. `REGISTRY` and both counters are
//! process-global `lazy_static` and `cargo test` runs this binary's tests in
//! parallel THREADS of one process, so an absolute assertion is a flake. Each
//! delta-reading test holds `counter_lock()` for its whole body.
//!
//! INV-GOV-001 — the gate is pinned through the ENV OVERRIDE: the shipped window
//! `[u64::MAX, u64::MAX)` is empty at every reachable height. `NetworkParams` is a
//! per-network `OnceLock` frozen on the first `params()` call in the PROCESS, so
//! this must be its own test binary.

use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

use crypto::{Hash, KeyPair, PublicKey};
use doli_core::transaction::{Input, Output, Transaction};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader};
use doli_node::metrics::{
    register_metrics, REGISTRY, VESTING_SHADOW_EVALUATED, VESTING_WOULD_REJECT,
};
use doli_node::node::Node;
use storage::{Outpoint, ProducerSet, UtxoEntry};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

static INIT_ENV: Once = Once::new();
static REGISTER: Once = Once::new();

/// Called FIRST by every test, before any `Node` or `params()` touch. All tests
/// set identical values, so the `Once` race is benign.
fn init_vesting_env() {
    INIT_ENV.call_once(|| {
        std::env::set_var("DOLI_INC_I_171_VESTING_PENALTY_ACTIVATION_HEIGHT", "101");
        std::env::set_var("DOLI_INC_I_171_VESTING_PENALTY_DISABLE_HEIGHT", "201");
    });
}

/// Serializes every test that reads a counter DELTA. Tokio's mutex is used
/// rather than `std::sync::Mutex` because the guard is held across `.await`.
fn counter_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

// Bands, as in `inc_i_171_m4_vesting_bound.rs`. Every height is ODD on purpose:
// devnet runs `blocks_per_reward_epoch = 4` and an epoch-boundary height would
// drag the epoch-reward section into every ACCEPT assertion.
const H_BELOW: u64 = 99;
const H_ENFORCED: u64 = 101;
// Below the devnet INC-I-180 `withdrawal_holdings_gate_activation_height` (20,
// `crates/core/src/network_params/defaults.rs:738`). The tests that use it
// re-read the shipped value instead of trusting this comment.
const H_BELOW_BOTH: u64 = 19;

const SLOT: u32 = 1_234;

// Tiers at SLOT=1234, devnet `vesting_quarter_slots = 60`: quarters =
// (1234 - creation_slot) / 60; 0=>75% 1=>50% 2=>25% 3+=>0%; penalty =
// amount * pct / 100 (truncating), net = amount - penalty.
const Q1: (u64, u32, u64) = (101, 1_224, 26);
const Q4: (u64, u32, u64) = (409, 1_034, 409);

/// Ledger holdings. Every fixture declares FEWER bonds than this, which puts
/// INC-I-180 on its partial-drain branch and keeps the exact-drain rule away.
const LEDGER_BONDS: u32 = 5;

const PAYOUT_EXCEEDS: &str = "[ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET]";

// The label universe of `doli_vesting_would_reject_total`: exactly the
// `error_code()` strings of the three `ValidationError` variants the vesting
// predicate can produce (`crates/core/src/validation/error.rs:566-568`).
const CODE_PAYOUT: &str = "ECON_WITHDRAWAL_PAYOUT_EXCEEDS_NET";
const CODE_MALFORMED: &str = "ECON_WITHDRAWAL_BOND_EXTRA_DATA_MALFORMED";
const CODE_QUARTER: &str = "ECON_VESTING_QUARTER_INVALID";
const CODES: [&str; 3] = [CODE_PAYOUT, CODE_MALFORMED, CODE_QUARTER];

const WOULD_REJECT_SERIES: &str = "doli_vesting_would_reject_total";
const EVALUATED_SERIES: &str = "doli_vesting_shadow_evaluated_total";

// ============================================================
// COUNTERS (delta-only)
// ============================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counters {
    evaluated: u64,
    payout_exceeds: u64,
    malformed: u64,
    quarter_invalid: u64,
}

fn would_reject(code: &str) -> u64 {
    VESTING_WOULD_REJECT.with_label_values(&[code]).get()
}

fn snapshot() -> Counters {
    Counters {
        evaluated: VESTING_SHADOW_EVALUATED.get(),
        payout_exceeds: would_reject(CODE_PAYOUT),
        malformed: would_reject(CODE_MALFORMED),
        quarter_invalid: would_reject(CODE_QUARTER),
    }
}

/// Saturating: an impossible decrement must not wrap into a passing number.
fn delta(b: Counters, a: Counters) -> Counters {
    Counters {
        evaluated: a.evaluated.saturating_sub(b.evaluated),
        payout_exceeds: a.payout_exceeds.saturating_sub(b.payout_exceeds),
        malformed: a.malformed.saturating_sub(b.malformed),
        quarter_invalid: a.quarter_invalid.saturating_sub(b.quarter_invalid),
    }
}

fn ensure_registered() {
    REGISTER.call_once(register_metrics);
}

/// The exposition text an operator's Prometheus scrapes: the same `TextEncoder`
/// over the same `REGISTRY.gather()` as `metrics_handler` (metrics.rs:1221-1231).
fn render_registry() -> String {
    use prometheus::Encoder;
    ensure_registered();
    let encoder = prometheus::TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&REGISTRY.gather(), &mut buffer)
        .expect("encode registry");
    String::from_utf8(buffer).expect("metrics are valid UTF-8")
}

/// Sample lines only: `# HELP` / `# TYPE` start with `#`, never `doli_vesting_`.
fn rendered_vesting_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with("doli_vesting_"))
        .map(|l| l.to_string())
        .collect()
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

/// Harness integrity: without it, "the shadow never fired" reads the same whether
/// M6 is unimplemented or the env override never reached `NetworkParams`.
fn assert_armed(node: &Node) {
    let params = node.config.network.params();
    assert_eq!(
        (
            params.inc_i_171_vesting_penalty_activation_height,
            params.inc_i_171_vesting_penalty_disable_height
        ),
        (H_ENFORCED, 201),
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

/// Seed one pre-block UTXO per entry, Bonds first, then Normal amounts.
/// `malformed_at` truncates that Bond's `extra_data` to 3 bytes — a direct
/// `UtxoSet` insert bypasses `[ERRTX007]`, the snap-sync hole INV-VEST-013 names.
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
    let dest = crypto::hash::hash(b"inc-i-171-m6-withdrawal-destination");
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

/// The three states CLAUDE.md names, hashed by the production canonical encoder.
async fn state_root(node: &Node) -> Hash {
    storage::compute_state_root(
        &*node.chain_state.read().await,
        &*node.utxo_set.read().await,
        &*node.producer_set.read().await,
    )
    .expect("compute_state_root")
}

async fn utxo_fingerprint(node: &Node) -> Hash {
    crypto::hash::hash(&node.utxo_set.read().await.serialize_canonical())
}

async fn producer_fingerprint(node: &Node) -> Hash {
    crypto::hash::hash(&node.producer_set.read().await.serialize_canonical())
}

// ============================================================
// SOURCE SCAN (cardinality tripwire)
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

/// Production text only: everything from the first `#[cfg(test)]` onward is a
/// test module and cannot construct a production label.
fn production_text(text: &str) -> &str {
    match text.find("#[cfg(test)]") {
        Some(i) => &text[..i],
        None => text,
    }
}

/// Every `VESTING_WOULD_REJECT.with_label_values(&[ .. ])` argument in non-test
/// `bins/node/src/`, as `(file, argument text)`. Read at RUNTIME, never
/// `include_str!`: a missing file must fail one assertion, not the whole build.
fn label_arg_sites() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    files.sort();
    let mut sites = Vec::new();
    for path in files {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let text = production_text(&raw);
        for (i, _) in text.match_indices("VESTING_WOULD_REJECT") {
            // rustfmt may break the call over several lines; a 200-char window
            // covers the longest formatting of one call.
            let window: String = text[i..].chars().take(200).collect();
            let Some(open) = window.find("with_label_values(&[") else {
                continue;
            };
            let rest = &window[open + "with_label_values(&[".len()..];
            let Some(close) = rest.find(']') else {
                continue;
            };
            sites.push((path.clone(), rest[..close].trim().to_string()));
        }
    }
    sites
}

// ============================================================
// MUST — REQ-VEST-011
// ============================================================

// REQ-VEST-011 (Must) — Decision: a failure here means precondition T3 for pinning
// the INC-I-171 height can never be measured — either the shadow is silent (the
// operator reads "zero would-rejects" from a rule that never ran) or the shadow
// changed a pre-activation verdict, which is an INV-VEST-007 consensus break.
#[tokio::test]
async fn req_vest_011_full_value_q1_below_activation_is_accepted_and_counted() {
    init_vesting_env();
    let _lock = counter_lock().lock().await;
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    assert_armed(&node);

    // 3 Q1 bonds: raw total 3*101 = 303, vested bound 3*26 = 78. The payout takes
    // the RAW total — the INC-I-171 loss itself.
    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xA1).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    let before = snapshot();
    let outcome = verdict(&node, &pk, vec![tx], H_BELOW).await;
    let moved = delta(before, snapshot());

    assert_eq!(
        outcome,
        Ok(()),
        "INV-VEST-007: below the activation height the shadow must not change the verdict"
    );
    assert_eq!(
        moved.evaluated, 1,
        "REQ-VEST-011: the shadow must evaluate exactly one RequestWithdrawal"
    );
    assert_eq!(
        moved.payout_exceeds, 1,
        "REQ-VEST-011: a 303-unit payout against a 78-unit bound is one would-reject on \
         {CODE_PAYOUT}"
    );
    assert_eq!(
        (moved.malformed, moved.quarter_invalid),
        (0, 0),
        "REQ-VEST-011: no other label may move for a well-formed over-payout"
    );
}

// REQ-VEST-011 (Must) — Decision: a failure here means the counter cannot separate
// "no honest producer would be rejected" from "the shadow never ran", so T3 would be
// read off a series that is zero for the wrong reason.
#[tokio::test]
async fn req_vest_011_honest_withdrawal_below_activation_counts_evaluated_only() {
    init_vesting_env();
    let _lock = counter_lock().lock().await;
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    assert_armed(&node);

    // One Q1 bond paid out at exactly the vested bound: what the shipped CLI sends.
    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1)], &[], None, 0xB1).await;
    let tx = withdrawal(inputs, &pk, 1, Q1.2);

    let before = snapshot();
    let outcome = verdict(&node, &pk, vec![tx], H_BELOW).await;
    let moved = delta(before, snapshot());

    assert_eq!(
        outcome,
        Ok(()),
        "REQ-VEST-011: an honest payout must be accepted"
    );
    assert_eq!(
        moved.evaluated, 1,
        "REQ-VEST-011: the shadow must count every evaluation, not only the failures"
    );
    assert_eq!(
        (moved.payout_exceeds, moved.malformed, moved.quarter_invalid),
        (0, 0, 0),
        "REQ-VEST-011: an honest CLI payout must produce ZERO would-rejects — this series \
         IS the T3 precondition"
    );
}

// REQ-VEST-011 (Must) — Decision: a failure here means the shadow double-counts what
// the live gate already rejected, so the would-reject series keeps climbing after the
// height is pinned and no operator can tell a dormant-window signal from an enforced one.
#[tokio::test]
async fn req_vest_011_shadow_does_not_run_at_or_after_activation() {
    init_vesting_env();
    let _lock = counter_lock().lock().await;
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    assert_armed(&node);

    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xC1).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    let before = snapshot();
    let outcome = verdict(&node, &pk, vec![tx], H_ENFORCED).await;
    let moved = delta(before, snapshot());

    let msg = outcome.expect_err("REQ-VEST-011: at the activation height the LIVE gate rejects");
    assert!(
        msg.contains(PAYOUT_EXCEEDS),
        "REQ-VEST-011: expected {PAYOUT_EXCEEDS} from the live gate, got: {msg}"
    );
    assert_eq!(
        moved,
        Counters::default(),
        "REQ-VEST-011: the shadow runs only BELOW the activation height; at or above it \
         neither series may move"
    );
}

// REQ-VEST-011 (Must) — Decision: a failure here means the shadow reached Replay,
// where the pre-block UTXO view is legitimately degraded (INC-I-064), and a reindex
// would manufacture would-rejects that no live node ever saw — poisoning T3 with
// noise from a path that cannot reject.
#[tokio::test]
async fn req_vest_011_light_and_replay_do_not_run_the_shadow() {
    init_vesting_env();
    let _lock = counter_lock().lock().await;
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    assert_armed(&node);

    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xD1).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    for mode in [ValidationMode::Light, ValidationMode::Replay] {
        let before = snapshot();
        let outcome = verdict_in_mode(&node, &pk, vec![tx.clone()], H_BELOW, mode).await;
        let moved = delta(before, snapshot());

        assert_eq!(
            outcome,
            Ok(()),
            "INV-VEST-007 {mode:?}: below the activation height the verdict is unchanged"
        );
        assert_eq!(
            moved,
            Counters::default(),
            "REQ-VEST-011 {mode:?}: the shadow is Full-only; neither series may move"
        );
    }
}

// REQ-VEST-011 (Must) — Decision: a failure here is the INC-I-187 failure repeating —
// a metric registered and never written, or written and never exported. T3 is read off
// the SCRAPED bytes, so a series absent from the exposition text is no evidence at all.
#[tokio::test]
async fn req_vest_011_metrics_render_carries_both_shadow_series() {
    init_vesting_env();
    let _lock = counter_lock().lock().await;
    ensure_registered();
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    assert_armed(&node);

    // ONE INC-I-171 reproduction block: a full-raw-value Q1 RequestWithdrawal
    // validated BELOW the activation height.
    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xE1).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    let before = snapshot();
    let outcome = verdict(&node, &pk, vec![tx], H_BELOW).await;
    let moved = delta(before, snapshot());
    let rendered = render_registry();

    for line in rendered_vesting_lines(&rendered) {
        println!("M6-PROBE-AFTER {line}");
    }

    assert_eq!(
        outcome,
        Ok(()),
        "INV-VEST-007: the reproduction block must still be accepted below the height"
    );
    assert!(
        rendered.contains(WOULD_REJECT_SERIES),
        "REQ-VEST-011: {WOULD_REJECT_SERIES} must be registered AND exported by the same \
         TextEncoder the /metrics handler uses"
    );
    assert!(
        rendered.contains(EVALUATED_SERIES),
        "REQ-VEST-011: {EVALUATED_SERIES} must be registered AND exported by the same \
         TextEncoder the /metrics handler uses"
    );
    assert_eq!(
        (moved.evaluated, moved.payout_exceeds),
        (1, 1),
        "REQ-VEST-011: one reproduction block is one evaluation and one would-reject"
    );
}

// REQ-VEST-011 (Must) — Decision: a failure here means the observe-only shadow wrote
// to state, so a node running M6 and a node running the prior binary hold different
// UTXO/producer sets below the activation height — a silent state-root fork at the
// next epoch boundary, the exact harm INV-VEST-007 forbids.
#[tokio::test]
async fn req_vest_011_shadow_leaves_state_byte_identical_below_activation() {
    init_vesting_env();
    let _lock = counter_lock().lock().await;
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    assert_armed(&node);

    // The three shapes M4's `req_vest_009_below_activation_and_at_disable_the_verdict_is
    // _unchanged` and `req_vest_001_malformed_bond_extra_data_fails_closed` pin as
    // ACCEPTED below the activation height.
    let one = seed_inputs(&node, &pk, &[(Q1.0, Q1.1)], &[], None, 0xF1).await;
    let three = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0xF2).await;
    let pair = [(Q1.0, Q1.1), (Q4.0, Q4.1)];
    let bad = seed_inputs(&node, &pk, &pair, &[], Some(1), 0xF3).await;
    let honest = withdrawal(one, &pk, 1, Q1.2);
    let full_value = withdrawal(three, &pk, 3, Q1.0 * 3);
    let malformed = withdrawal(bad, &pk, 2, 1);
    install_ledger(&node, &pk).await;

    let root_before = state_root(&node).await;
    let utxo_before = utxo_fingerprint(&node).await;
    let producers_before = producer_fingerprint(&node).await;
    let counters_before = snapshot();

    for (label, tx) in [
        ("honest", honest),
        ("full-value", full_value),
        ("malformed", malformed),
    ] {
        let block = block_with(&node, H_BELOW, pk, vec![tx]);
        let outcome = node
            .validate_block_economics(&block, H_BELOW, ValidationMode::Full)
            .await
            .map_err(|e| e.to_string());
        assert_eq!(
            outcome,
            Ok(()),
            "INV-VEST-007 {label}: the pre-M6 verdict below the activation height is Ok"
        );
        assert_eq!(
            state_root(&node).await,
            root_before,
            "INV-VEST-007 {label}: ChainState + UtxoSet + ProducerSet must be byte-identical \
             after the shadow ran"
        );
        assert_eq!(
            utxo_fingerprint(&node).await,
            utxo_before,
            "INV-VEST-007 {label}: the shadow must not touch the UTXO set"
        );
        assert_eq!(
            producer_fingerprint(&node).await,
            producers_before,
            "INV-VEST-007 {label}: the shadow must not touch the producer set"
        );
    }

    let moved = delta(counters_before, snapshot());
    assert_eq!(
        (moved.evaluated, moved.payout_exceeds, moved.malformed),
        (3, 1, 1),
        "REQ-VEST-011: three withdrawals evaluated; the full-value one is a payout \
         would-reject and the malformed one fails CLOSED on {CODE_MALFORMED}"
    );
    assert_eq!(
        moved.quarter_invalid, 0,
        "REQ-VEST-011: devnet ships vesting_quarter_slots = 60, so {CODE_QUARTER} is \
         unreachable here"
    );
}

// REQ-VEST-011 (Must) — Decision: a failure here means either the shadow is blind on
// every chain below the INC-I-180 holdings gate (T3 measures nothing on a fresh
// network), or entering the withdrawal pass early woke the INC-I-180 rules below their
// own height — an activation-height violation of somebody else's gate (INC-I-054).
#[tokio::test]
async fn req_vest_011_shadow_runs_below_the_inc_i_180_withdrawal_gate() {
    init_vesting_env();
    let _lock = counter_lock().lock().await;
    let (node, kp, _temp) = make_node().await;
    let pk = *kp.public_key();
    assert_armed(&node);

    let holdings_gate = node
        .config
        .network
        .params()
        .withdrawal_holdings_gate_activation_height;
    assert!(
        H_BELOW_BOTH < holdings_gate,
        "harness: H_BELOW_BOTH={H_BELOW_BOTH} must sit below the shipped devnet \
         withdrawal_holdings_gate_activation_height={holdings_gate}"
    );

    // 1) The shadow still observes below BOTH gates.
    let inputs = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 3], &[], None, 0x11).await;
    let tx = withdrawal(inputs, &pk, 3, Q1.0 * 3);

    let before = snapshot();
    let outcome = verdict(&node, &pk, vec![tx], H_BELOW_BOTH).await;
    let moved = delta(before, snapshot());

    assert_eq!(
        outcome,
        Ok(()),
        "INV-VEST-007: below both gates the verdict must still be Ok"
    );
    assert_eq!(
        (moved.evaluated, moved.payout_exceeds),
        (1, 1),
        "REQ-VEST-011: the early-return must let the shadow through below the INC-I-180 gate"
    );

    // 2) Entering the pass for the shadow must NOT wake the INC-I-180 rules: three
    // bonds declared against two seeded is a count mismatch that only becomes an
    // error at or above the holdings gate.
    let short = seed_inputs(&node, &pk, &[(Q1.0, Q1.1); 2], &[], None, 0x12).await;
    let count_mismatch = withdrawal(short, &pk, 3, Q1.0 * 3);
    assert_eq!(
        verdict(&node, &pk, vec![count_mismatch], H_BELOW_BOTH).await,
        Ok(()),
        "INV-VEST-007: below the INC-I-180 gate a declared/seeded count mismatch must \
         still be ACCEPTED — the shadow may not drag the holdings rules below their height"
    );
}

// REQ-VEST-011 (Must) — Decision: a failure here means one withdrawal flood can mint
// unbounded label values, and the series that T3 is read from becomes an unscrapable
// cardinality explosion on every node in the fleet.
#[test]
fn req_vest_011_would_reject_label_cardinality_is_bounded() {
    let sites = label_arg_sites();
    assert!(
        !sites.is_empty(),
        "REQ-VEST-011: no {WOULD_REJECT_SERIES} write site exists in non-test \
         bins/node/src — a registered-but-never-written metric is the INC-I-187 failure"
    );

    let metrics_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/metrics.rs");
    let metrics_text = std::fs::read_to_string(&metrics_path).expect("read src/metrics.rs");
    assert!(
        sites.iter().any(|(p, _)| p != &metrics_path),
        "REQ-VEST-011: every write site is in {metrics_path:?} (registration/zero-init \
         only); the hot path must actually increment the counter"
    );

    for (path, arg) in &sites {
        for banned in ["format!", "to_string", "hex::", "{", "+", "height", "hash"] {
            assert!(
                !arg.contains(banned),
                "REQ-VEST-011: {path:?} builds a label from `{banned}` in `{arg}` — a \
                 per-tx/per-producer/per-height label is an unbounded series"
            );
        }
        let literal = CODES.iter().any(|c| arg == &format!("\"{c}\""));
        let from_error = arg.ends_with(".error_code()")
            && arg
                .trim_end_matches(".error_code()")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        let bound_identifier = !arg.is_empty()
            && arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && arg.starts_with(|c: char| c.is_ascii_lowercase() || c == '_');
        assert!(
            literal || from_error || bound_identifier,
            "REQ-VEST-011: {path:?} passes `{arg}` as a label; only one of {CODES:?}, \
             `<err>.error_code()` or a variable bound to that fixed list is allowed"
        );
        if bound_identifier && !from_error && !literal {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let declared = CODES.iter().all(|c| production_text(&text).contains(c));
            assert!(
                declared,
                "REQ-VEST-011: {path:?} passes the variable `{arg}` as a label without \
                 declaring the fixed vocabulary {CODES:?} in the same file"
            );
        }
    }

    // INC-I-154/INC-I-187: a label value that only appears after it first fires is
    // invisible on a healthy node, so all three are zero-initialised at registration.
    let register_body = metrics_text
        .find("pub fn register_metrics")
        .map(|i| &metrics_text[i..])
        .unwrap_or("");
    let zero_initialised = register_body
        .match_indices("VESTING_WOULD_REJECT")
        .any(|(i, _)| {
            register_body[i..]
                .chars()
                .take(320)
                .collect::<String>()
                .contains("inc_by(0)")
        });
    assert!(
        zero_initialised,
        "REQ-VEST-011: register_metrics must zero-initialise {WOULD_REJECT_SERIES} the way \
         PRE_ACTIVATION_BRANCH does, or the family publishes nothing until it first fires"
    );
    for code in CODES {
        assert!(
            metrics_text.contains(code),
            "REQ-VEST-011: {metrics_path:?} must name the fixed label value `{code}`"
        );
    }

    // The rendered universe, not just the source: whatever this process produced,
    // every published `code` label must come from the fixed vocabulary.
    for line in rendered_vesting_lines(&render_registry()) {
        if !line.starts_with(WOULD_REJECT_SERIES) {
            continue;
        }
        assert!(
            CODES.iter().any(|c| line.contains(c)),
            "REQ-VEST-011: exported series `{line}` carries a label outside {CODES:?}"
        );
    }
}
