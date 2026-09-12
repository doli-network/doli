//! INC-I-217 M9 — REQ-ROT-012: an applied BLS key rotation must be countable by
//! Prometheus and readable in the node log.
//!
//! INC-I-187 is why the counter assertions are DELTAS through a real `Node` and not a
//! registration check: 28 of 57 shipped `doli_*` series were registered and never
//! written, so "the series exists" proves nothing an operator can use.
//!
//! OUTPUT CONTRACT — `Node::apply_block(block, ValidationMode::Light)` at the epoch
//! boundary that flushes the queue (`state_update.rs`, `is_epoch_0 || is_boundary`)
//!   O1 `BLS_ROTATIONS_APPLIED`   +1 per `RotateBlsKey` the flush drained
//!   O2 rendered REGISTRY text    `doli_producer_bls_rotation_total` present, value moved
//!   O3 `producer_set` queue      the `RotateBlsKey` entry: present before, drained after
//!   O4 `producer_set` bls_pubkey OLD before the boundary, NEW after
//!   O5 `[BLS_ROTATE] applied`    one line carrying `producer=`, `old=`, `new=`, `h=`
//!   O6 utxo_set / chain_state / state_db — NOT this milestone's outputs; the M8 suite
//!      (`tests/it/bls_rotation_convergence.rs`) pins both apply paths, the rollback and
//!      the persisted `ProducerSet`, and `producer/wire_golden_tests.rs` pins the
//!      `serialize_canonical` byte layout. M9 adds no hashed state, so neither is re-asserted.
//! PATHS
//!   PA mid-epoch, the rotation queued, no boundary crossed
//!   PB the boundary block that flushes exactly one rotation
//!   PC a boundary block that flushes only NON-rotation updates
//!   PD a boundary block that flushes one rotation among three queued updates
//! INPUT PARTITIONS: queue composition at the flush — {rotation} / {non-rotation x2} /
//!   {rotation + non-rotation x2}; observation before vs after the boundary.
//! MATRIX
//!   PA: O1 (delta 0) ✓  O3 ✓  O4 ✓
//!   PB: O1 (delta 1) ✓  O2 ✓  O3 ✓  O4 ✓  O5 ✓
//!   PC: O1 (delta 0) ✓
//!   PD: O1 (delta 1, NOT 3) ✓
//!
//! Every counter read is a DELTA taken under `counter_lock()`: `REGISTRY` and the counter
//! are process-global `lazy_static`s and this binary runs its tests in parallel threads.
//! `NetworkParams` is a per-network `OnceLock` frozen by the first `params()` call in the
//! process and the rotation gate ships `u64::MAX` on all three networks, so the env
//! override must win the race — hence this file is its own test binary.

use std::io::Write;
use std::sync::{Mutex, Once, OnceLock};

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::genesis::genesis_hash;
use doli_core::network_params::NetworkParams;
use doli_core::transaction::{rotation_auth_digest, Input, Output, RotateBlsData};
use doli_core::validation::ValidationMode;
use doli_core::{Block, BlockHeader, Network, Transaction, TxType};
use doli_node::metrics::{register_metrics, BLS_ROTATIONS_APPLIED, REGISTRY};
use doli_node::node::Node;
use storage::{Outpoint, PendingProducerUpdate, ProducerSet, UtxoEntry, UtxoSet};
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

const NET: Network = Network::Devnet;
const EPOCH_LEN: u64 = 4;
const GENESIS_BLOCKS: u64 = 40;

/// Devnet override. `> EPOCH_LEN` so no observation lands in epoch 0, where the flush
/// runs every block and a "boundary" partition would not exist.
const ROT_AH: u64 = 8;
const ROT_H: u64 = 9;
const BOUNDARY: u64 = 12;
const NEXT_BOUNDARY: u64 = 16;

const FUND: u64 = 100_000;
const CHANGE: u64 = 99_000;

const SERIES: &str = "doli_producer_bls_rotation_total";
const APPLIED_TOKEN: &str = "[BLS_ROTATE] applied";

// ============================================================
// Process boot: the gate override + the INFO tee
// ============================================================

static CAPTURED: Mutex<String> = Mutex::new(String::new());

#[derive(Clone, Copy)]
struct Tee;

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut c) = CAPTURED.lock() {
            c.push_str(&String::from_utf8_lossy(buf));
        }
        let mut out = std::io::stdout();
        out.write_all(buf)?;
        out.flush()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stdout().flush()
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for Tee {
    type Writer = Tee;
    fn make_writer(&self) -> Tee {
        Tee
    }
}

static BOOT: Once = Once::new();
static REGISTER: Once = Once::new();

/// Every test agrees on the same value, so the `Once` race is benign. Must run before
/// the first `Network::params()` touch: that call is `OnceLock`-cached for the process.
/// The subscriber is INFO because the applied line is `tracing::info!`.
fn boot() {
    BOOT.call_once(|| {
        std::env::set_var(
            "DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT",
            ROT_AH.to_string(),
        );
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_writer(Tee)
            .with_ansi(false)
            .try_init();
    });
}

fn assert_gate_armed() {
    let live = NET.params().bls_key_rotation_activation_height;
    assert_eq!(
        live, ROT_AH,
        "DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT is not honoured on devnet (resolved {live}). \
         Without it every rotation below is rejected and the counter cannot move for a \
         reason that has nothing to do with M9."
    );
}

fn captured() -> String {
    CAPTURED.lock().map(|c| c.clone()).unwrap_or_default()
}

/// Serialises every test that reads a counter delta or the shared capture buffer.
/// Tokio's mutex, because the guard is held across `.await`.
fn counter_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

// ============================================================
// Counter + exposition
// ============================================================

fn ensure_registered() {
    REGISTER.call_once(register_metrics);
}

fn counter() -> u64 {
    ensure_registered();
    BLS_ROTATIONS_APPLIED.get()
}

/// The exposition text an operator's Prometheus scrapes: the same `TextEncoder` over the
/// same `REGISTRY.gather()` as the node's `/metrics` handler.
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

/// The sample value of `SERIES` in the rendered text. `# HELP` / `# TYPE` lines start
/// with `#`, so a prefix match on the bare series name selects the sample only.
fn rendered_value(text: &str) -> Option<u64> {
    text.lines()
        .find(|l| l.starts_with(SERIES))
        .and_then(|l| l.rsplit_once(' '))
        .and_then(|(_, v)| v.trim().parse::<f64>().ok())
        .map(|v| v as u64)
}

// ============================================================
// Builders (shapes shared with tests/it/bls_rotation_convergence.rs)
// ============================================================

fn new_bls(seed: u8) -> BlsKeyPair {
    BlsKeyPair::from_seed(&[seed; 32]).expect("a 32-byte BLS seed is valid")
}

fn address_of(kp: &KeyPair) -> Hash {
    crypto::hash::hash_with_domain(crypto::ADDRESS_DOMAIN, kp.public_key().as_bytes())
}

/// A fully valid `RotateBlsKey`: the payload signature covers `rotation_auth_digest`,
/// the PoP uses the rotation DST, and the input is signed as an ordinary spend.
fn rotation_tx(producer: &KeyPair, bls: &BlsKeyPair, outpoint: Outpoint, to: Hash) -> Transaction {
    let new_bls_pubkey = *bls.public_key().as_bytes();
    let gh = genesis_hash(NET);
    let digest = rotation_auth_digest(
        gh.as_bytes(),
        &new_bls_pubkey,
        outpoint.tx_hash.as_bytes(),
        outpoint.index,
    );
    let payload = RotateBlsData {
        producer: *producer.public_key().as_bytes(),
        new_bls_pubkey,
        bls_pop: *crypto::sign_rotation_pop(
            bls.secret_key(),
            bls.public_key(),
            gh.as_bytes(),
            producer.public_key().as_bytes(),
        )
        .expect("rotation PoP signing cannot fail")
        .as_bytes(),
        signature: *crypto::signature::sign(&digest, producer.private_key()).as_bytes(),
    };
    let mut input = Input::new(outpoint.tx_hash, outpoint.index);
    input.public_key = Some(*producer.public_key());
    let mut tx = Transaction {
        version: 1,
        tx_type: TxType::RotateBlsKey,
        inputs: vec![input],
        outputs: vec![Output::normal(CHANGE, to)],
        extra_data: payload.encode().to_vec(),
    };
    let sighash = tx.signing_message_for_input(0);
    tx.inputs[0].signature = crypto::signature::sign_hash(&sighash, producer.private_key());
    tx
}

fn block_at(
    node: &Node,
    height: u64,
    slot: u32,
    prev: Hash,
    by: &KeyPair,
    txs: Vec<Transaction>,
) -> Block {
    let coinbase = Transaction::new_coinbase(
        node.params.block_reward(height),
        doli_core::consensus::reward_pool_pubkey_hash(),
        height,
        slot,
    );
    let mut all = vec![coinbase];
    all.extend(txs);
    let header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(&all),
        presence_root: Hash::ZERO,
        genesis_hash: doli_core::chainspec::ChainSpec::devnet().genesis_hash(),
        timestamp: node.params.genesis_time + (u64::from(slot) * node.params.slot_duration),
        slot,
        producer: *by.public_key(),
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

async fn make_node() -> (Node, KeyPair, TempDir) {
    boot();
    let temp = TempDir::new().expect("tempdir");
    let kp = KeyPair::generate();
    let node = Node::new_for_test(temp.path().to_path_buf(), vec![kp.clone()])
        .await
        .expect("Node::new_for_test");
    assert_eq!(node.config.network.blocks_per_reward_epoch(), EPOCH_LEN);
    assert_eq!(node.config.network.genesis_blocks(), GENESIS_BLOCKS);
    (node, kp, temp)
}

async fn bls_of(node: &Node, pk: &PublicKey) -> Vec<u8> {
    node.producer_set
        .read()
        .await
        .get_by_pubkey(pk)
        .expect("the rotation target must be in the ProducerSet")
        .bls_pubkey
        .clone()
}

fn rotations_queued(ps: &ProducerSet, pk: &PublicKey) -> usize {
    ps.pending_updates_for(pk)
        .iter()
        .filter(|u| matches!(u, PendingProducerUpdate::RotateBlsKey { .. }))
        .count()
}

/// Production UTXO backend, a funded outpoint, and the chain extended to `ROT_H - 1`.
async fn applied_fixture() -> (Node, KeyPair, BlsKeyPair, Outpoint, TempDir) {
    let (mut node, kp, temp) = make_node().await;
    assert_gate_armed();
    {
        let mut utxo = node.utxo_set.write().await;
        *utxo = UtxoSet::from_state_db(node.state_db.clone());
    }
    let outpoint = Outpoint::new(crypto::hash::hash(b"inc-i-217-m9-rotation-funding"), 0);
    node.utxo_set
        .write()
        .await
        .insert(
            outpoint,
            UtxoEntry {
                output: Output::normal(FUND, address_of(&kp)),
                height: 0,
                is_coinbase: false,
                is_epoch_reward: false,
            },
        )
        .expect("fixture: funding insert");
    let mut prev = Hash::ZERO;
    for h in 1..ROT_H {
        let block = block_at(&node, h, h as u32, prev, &kp, Vec::new());
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("fixture: apply_block failed at h={h}: {e}"));
    }
    (node, kp, new_bls(0x91), outpoint, temp)
}

async fn apply_rotation(node: &mut Node, kp: &KeyPair, bls: &BlsKeyPair, op: Outpoint) {
    let prev = node.chain_state.read().await.best_hash;
    let tx = rotation_tx(kp, bls, op, address_of(kp));
    let block = block_at(node, ROT_H, ROT_H as u32, prev, kp, vec![tx]);
    node.apply_block(block, ValidationMode::Light)
        .await
        .unwrap_or_else(|e| {
            panic!(
                "the rotation block at h={ROT_H} was rejected: {e}. Above \
                 DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT={ROT_AH} a well-formed rotation must \
                 be a VALID block — if this is [ERRTX-ROT002] the env override is missing."
            )
        });
}

async fn extend_plain(node: &mut Node, kp: &KeyPair, from: u64, to: u64) {
    let mut prev = node.chain_state.read().await.best_hash;
    for h in from..=to {
        let block = block_at(node, h, h as u32, prev, kp, Vec::new());
        prev = block.hash();
        node.apply_block(block, ValidationMode::Light)
            .await
            .unwrap_or_else(|e| panic!("apply_block failed at h={h}: {e}"));
    }
}

/// A queued update that is NOT a rotation, for a key that is not in the set: the flush
/// drains it and mutates nothing, so it isolates "the flush ran" from "a rotation applied".
fn non_rotation_update() -> PendingProducerUpdate {
    PendingProducerUpdate::Exit {
        pubkey: *KeyPair::generate().public_key(),
        height: ROT_H,
    }
}

/// The hex field values of the last `[BLS_ROTATE] applied` line, as `(old, new)`.
/// The shipped line renders a full 64-char hash per field (the `{:.8}` precision is not
/// honoured by that Display impl), so the assertions below compare 8-char PREFIXES.
fn applied_old_and_new(log: &str) -> Option<(String, String)> {
    let line = log.lines().rfind(|l| l.contains(APPLIED_TOKEN))?;
    let field = |name: &str| -> Option<String> {
        let rest = line.split_once(name)?.1;
        let end = rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    };
    Some((field("old=")?, field("new=")?))
}

// ============================================================
// Preconditions
// ============================================================

// REQ-ROT-012 — Decision: a shipped default that drifted off `u64::MAX` would arm the
// rotation gate on a live network without a decision session, and would make every
// assertion below pass for a reason that is not the code under test.
#[test]
fn precondition_the_three_shipped_defaults_are_still_frozen() {
    // INC-I-217 pins (2026-09-12): mainnet 450_789, testnet 176_200; this harness runs on
    // Devnet, whose default must stay u64::MAX so the gate is armed by ENV only.
    for (network, expected) in [
        (Network::Devnet, u64::MAX),
        (Network::Testnet, 176_200),
        (Network::Mainnet, 450_789),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).bls_key_rotation_activation_height,
            expected,
            "{network:?} default must be exactly {expected} — devnet stays frozen, the M8 harness \
             arms the devnet gate by ENV, never by pin"
        );
    }
}

// ============================================================
// PA — before the boundary
// ============================================================

// REQ-ROT-012 (Must) — Decision: a failure here means the counter moves when the rotation
// is only QUEUED, so an operator watching the series believes the new key is live while
// the node is still signing attestations with the old one.
#[tokio::test]
async fn req_rot_012_nothing_is_counted_before_the_boundary_flush() {
    let _lock = counter_lock().lock().await;
    let (mut node, kp, bls, op, _t) = applied_fixture().await;
    let pk = *kp.public_key();
    let old_key = bls_of(&node, &pk).await;
    let new_key = bls.public_key().as_bytes().to_vec();

    let before = counter();
    apply_rotation(&mut node, &kp, &bls, op).await;
    extend_plain(&mut node, &kp, ROT_H + 1, BOUNDARY - 1).await;
    let moved = counter() - before;

    assert_eq!(
        rotations_queued(&*node.producer_set.read().await, &pk),
        1,
        "O3 [PA]: the queue must carry the rotation until the boundary"
    );
    assert_eq!(
        node.producer_set
            .read()
            .await
            .pending_updates_for(&pk)
            .iter()
            .find_map(|u| match u {
                PendingProducerUpdate::RotateBlsKey { new_bls_pubkey, .. } =>
                    Some(new_bls_pubkey.clone()),
                _ => None,
            }),
        Some(new_key.clone()),
        "O3 [PA]: the queued entry must carry the 48-byte key the tx published"
    );
    assert_eq!(
        bls_of(&node, &pk).await,
        old_key,
        "O4 [PA]: mid-epoch the installed key must still be the OLD one"
    );
    assert_eq!(
        moved, 0,
        "O1 [PA]: a queued rotation is not an applied rotation"
    );
}

// ============================================================
// PB — the boundary flush
// ============================================================

// REQ-ROT-012 (Must) — Decision: a failure here means the only on-chain record of a key
// rotation is a queue entry that vanishes at the boundary, so nobody can answer "did this
// producer's key actually change, and at which height" without replaying blocks.
#[tokio::test]
async fn req_rot_012_counter_delta_is_one_across_the_boundary_flush() {
    let _lock = counter_lock().lock().await;
    let (mut node, kp, bls, op, _t) = applied_fixture().await;
    let pk = *kp.public_key();
    let new_key = bls.public_key().as_bytes().to_vec();

    apply_rotation(&mut node, &kp, &bls, op).await;
    extend_plain(&mut node, &kp, ROT_H + 1, BOUNDARY - 1).await;

    let before = counter();
    extend_plain(&mut node, &kp, BOUNDARY, BOUNDARY).await;
    let moved = counter() - before;

    assert_eq!(
        bls_of(&node, &pk).await,
        new_key,
        "O4 [PB]: the boundary block at h={BOUNDARY} must install the new key"
    );
    assert_eq!(
        rotations_queued(&*node.producer_set.read().await, &pk),
        0,
        "O3 [PB]: the flush must drain the queue"
    );
    assert_eq!(
        moved, 1,
        "O1 [PB]: one rotation flushed is exactly one increment of {SERIES}"
    );
}

// REQ-ROT-012 (Must) — Decision: a failure here means the counter exists in the binary but
// never reaches an operator's Prometheus, which is the INC-I-187 failure mode exactly: 28
// of 57 doli_* series were registered and never written, so an alert on this series would
// stay silent through every rotation on the fleet.
#[tokio::test]
async fn req_rot_012_counter_series_is_scraped_and_moves() {
    let _lock = counter_lock().lock().await;
    let (mut node, kp, bls, op, _t) = applied_fixture().await;

    apply_rotation(&mut node, &kp, &bls, op).await;
    extend_plain(&mut node, &kp, ROT_H + 1, BOUNDARY - 1).await;

    let rendered_before = render_registry();
    let value_before = rendered_value(&rendered_before);
    extend_plain(&mut node, &kp, BOUNDARY, BOUNDARY).await;
    let rendered_after = render_registry();
    let value_after = rendered_value(&rendered_after);

    assert!(
        rendered_after.contains(SERIES),
        "O2 [PB]: {SERIES} must be registered in REGISTRY AND exported by the same \
         TextEncoder the /metrics handler uses"
    );
    assert_eq!(
        value_after,
        value_before.map(|v| v + 1),
        "O2 [PB]: the SCRAPED sample must move by 1, not just the in-process counter"
    );
    println!(
        "ROT-METRIC-MOVES {SERIES} {} -> {}",
        value_before.expect("the series must be present before the flush"),
        value_after.expect("the series must be present after the flush")
    );
}

// REQ-ROT-012 (Must) — Decision: a failure here means an operator reading the log cannot
// tell WHICH key was replaced, so a rotation onto a wrong or stale key is indistinguishable
// from the intended one — the post-incident question "was the old key really retired" has
// no answer in the record.
#[tokio::test]
async fn req_rot_012_applied_log_carries_old_and_new() {
    let _lock = counter_lock().lock().await;
    let (mut node, kp, bls, op, _t) = applied_fixture().await;

    apply_rotation(&mut node, &kp, &bls, op).await;
    extend_plain(&mut node, &kp, ROT_H + 1, BOUNDARY).await;

    let log = captured();
    let line = log
        .lines()
        .rfind(|l| l.contains(APPLIED_TOKEN))
        .unwrap_or_else(|| {
            panic!("O5 [PB]: the boundary flush must emit an `{APPLIED_TOKEN}` line at INFO")
        })
        .to_string();
    assert!(
        line.contains("producer=") && line.contains("h="),
        "O5 [PB]: the line must name the producer and the height: {line}"
    );
    let (old, new) = applied_old_and_new(&log)
        .unwrap_or_else(|| panic!("O5 [PB]: the line must carry BOTH `old=` and `new=`: {line}"));
    assert!(
        old.len() >= 8 && new.len() >= 8,
        "O5 [PB]: `old=` and `new=` must each carry at least an 8-hex prefix: {line}"
    );
    let (old8, new8) = (&old[..8], &new[..8]);
    assert_ne!(
        old8, new8,
        "O5 [PB]: a rotation that logs the same prefix for old and new records nothing: {line}"
    );
    println!("ROT-LOG-APPLIED-OLD old={old8} new={new8}");
}

// ============================================================
// PC / PD — non-vacuity: the counter counts ROTATIONS
// ============================================================

// REQ-ROT-012 (Must) — Decision: a failure here means the counter is incremented per
// boundary flush rather than per rotation, so the series climbs on every epoch of every
// node in the fleet and an alert built on it is pure noise.
#[tokio::test]
async fn req_rot_012_counter_does_not_move_on_a_boundary_without_a_rotation() {
    let _lock = counter_lock().lock().await;
    let (mut node, kp, _bls, _op, _t) = applied_fixture().await;

    extend_plain(&mut node, &kp, ROT_H, BOUNDARY - 1).await;
    {
        let mut producers = node.producer_set.write().await;
        producers.queue_update(non_rotation_update());
        producers.queue_update(non_rotation_update());
        assert!(
            producers.has_pending_updates(),
            "the boundary must actually FLUSH, or this test is vacuous"
        );
    }

    let before = counter();
    extend_plain(&mut node, &kp, BOUNDARY, BOUNDARY).await;
    let moved = counter() - before;

    assert_eq!(
        node.producer_set.read().await.pending_update_count(),
        0,
        "the queue must have been flushed at h={BOUNDARY} — otherwise a delta of 0 proves nothing"
    );
    assert_eq!(
        moved, 0,
        "O1 [PC]: a flush that drained two non-rotation updates must not move {SERIES}"
    );
}

// REQ-ROT-012 (Must) — Decision: a failure here means the counter is incremented by the
// total pending-update count, so a busy epoch reports several rotations where one happened
// and the number an operator reads has no relationship to keys that changed.
#[tokio::test]
async fn req_rot_012_counter_counts_rotations_not_flushed_updates() {
    let _lock = counter_lock().lock().await;
    let (mut node, kp, bls, op, _t) = applied_fixture().await;

    apply_rotation(&mut node, &kp, &bls, op).await;
    extend_plain(&mut node, &kp, ROT_H + 1, BOUNDARY - 1).await;
    {
        let mut producers = node.producer_set.write().await;
        producers.queue_update(non_rotation_update());
        producers.queue_update(non_rotation_update());
        assert_eq!(
            producers.pending_update_count(),
            3,
            "the flush must see three updates of which exactly one is a rotation"
        );
    }

    let before = counter();
    extend_plain(&mut node, &kp, BOUNDARY, BOUNDARY).await;
    let moved = counter() - before;

    assert_eq!(
        moved, 1,
        "O1 [PD]: three updates flushed, one rotation among them, is one increment"
    );
}

// REQ-ROT-012 (Must) — Decision: a failure here means the counter keeps climbing after the
// rotation is installed, so the series double-counts one key change across later epochs and
// the fleet-wide total overstates how many producers actually rotated.
#[tokio::test]
async fn req_rot_012_counter_does_not_move_again_at_the_next_boundary() {
    let _lock = counter_lock().lock().await;
    let (mut node, kp, bls, op, _t) = applied_fixture().await;

    apply_rotation(&mut node, &kp, &bls, op).await;
    extend_plain(&mut node, &kp, ROT_H + 1, BOUNDARY).await;

    let before = counter();
    extend_plain(&mut node, &kp, BOUNDARY + 1, NEXT_BOUNDARY).await;
    let moved = counter() - before;

    assert_eq!(
        moved, 0,
        "O1 [PB→PC]: the rotation was already installed at h={BOUNDARY}; crossing \
         h={NEXT_BOUNDARY} must not count it a second time"
    );
}
