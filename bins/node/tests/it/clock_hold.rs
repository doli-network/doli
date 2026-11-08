//! A peer block whose only fault is a timestamp ahead of the receiver's clock.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()>
//   O1 return (Ok on every path); O2 chain tip (best_hash, best_height);
//   O3 bls_ingress_scorer score of the sender; O4 sender refusal for that hash
//   (body_retry failed set, read through Node::body_refused_for_test)
// PATHS: P1 fails TimestampTooFuture only | P2 fails a body commitment (merkle)
//   | P3 same block, same sender, again once the node clock reaches its time
// MATRIX
//   P1 x O1,O2,O3 -> Ok, tip unchanged, score 0
//   P2 x O1,O2,O3 -> Ok, tip unchanged, score -100
//   P1 x O4 -> sender not refused; P1 then P3 x O1,O2 -> Ok, tip == block
// INPUT PARTITIONS:
//   node clock: block time - 2 s (ahead) | block time (current)
//   block: valid signed tip-extending | wrong merkle root, re-signed
//   sender: one peer for both copies | distinct peer per test
// covers: clock_hold, sites, body_retry, mod, init, validation_checks, periodic, block_handling

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crypto::{Hash, KeyPair};
use doli_core::chainspec::ChainSpec;
use doli_core::validation::{ValidationError, ValidationMode};
use doli_core::{Block, BlockHeader, Transaction};
use doli_node::node::Node;
use network::PeerId;
use tempfile::TempDir;
use vdf::{VdfOutput, VdfProof};

use super::inc_i_288_c1_common::leader;
use crate::block_sig::sign_header;

const SLOT_SECS: u64 = 10;
const BASE: u64 = 5;

struct Rig {
    node: Node,
    keys: Vec<KeyPair>,
    genesis: Hash,
    base_tip: Hash,
    _tmp: TempDir,
}

fn real_now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
}

/// Node clock = `secs`, set just after a second boundary so it holds for the test step.
async fn set_clock(node: &mut Node, secs: u64) {
    let to_next = Duration::from_nanos(1_000_000_000 - u64::from(real_now().subsec_nanos()));
    tokio::time::sleep(to_next + Duration::from_millis(5)).await;
    node.set_clock_offset_for_test(secs as i64 - real_now().as_secs() as i64);
}

fn make_block(r: &Rig, height: u64, slot: u32, prev: Hash, by: &KeyPair, merkle_ok: bool) -> Block {
    let p = &r.node.params;
    let pool = doli_core::consensus::reward_pool_pubkey_hash();
    let coinbase = Transaction::new_coinbase(p.block_reward(height), pool, height, 0);
    let merkle_root = if merkle_ok {
        doli_core::block::compute_merkle_root(std::slice::from_ref(&coinbase))
    } else {
        crypto::hash::hash(b"not the body")
    };
    let mut header = BlockHeader {
        version: 2,
        prev_hash: prev,
        merkle_root,
        presence_root: Hash::ZERO,
        genesis_hash: r.genesis,
        timestamp: p.genesis_time + u64::from(slot) * p.slot_duration,
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
    sign_header(&mut header, by);
    Block::new(header, vec![coinbase])
}

/// Devnet test node on a 10 s slot chainspec, base chain h=1..=BASE applied.
async fn rig() -> Rig {
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate()).collect();
    let tmp = TempDir::new().unwrap();
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys.clone())
        .await
        .expect("Node::new_for_test");
    let mut spec = ChainSpec::devnet();
    spec.genesis.timestamp = 0;
    spec.consensus.slot_duration = SLOT_SECS;
    let genesis = spec.genesis_hash();
    node.params.slot_duration = SLOT_SECS;
    node.params.apply_chainspec(&spec);
    node.config.chainspec = Some(spec);
    node.chain_state.write().await.genesis_hash = genesis;
    assert_eq!(node.params.slot_duration, SLOT_SECS, "fixture: 10 s slot");
    assert_eq!(node.params.genesis_time, 0, "fixture: genesis time 0");

    let mut r = Rig {
        node,
        keys,
        genesis,
        base_tip: Hash::ZERO,
        _tmp: tmp,
    };
    let mut prev = Hash::ZERO;
    for h in 1..=BASE {
        let by = r.keys[0].clone();
        let b = make_block(&r, h, h as u32, prev, &by, true);
        prev = b.hash();
        r.node
            .apply_block(b, ValidationMode::Light)
            .await
            .expect("fixture: base block");
    }
    r.base_tip = prev;
    assert_eq!(tip(&r).await, (prev, BASE), "fixture: tip = base top");
    r
}

/// The tip-extending block at the first slot after BASE led by a key that is not the node's.
fn next_block(r: &Rig, merkle_ok: bool) -> Block {
    let own = *r
        .node
        .producer_key
        .as_ref()
        .expect("fixture: node has a producer key")
        .public_key();
    let slot = (BASE as u32 + 1..BASE as u32 + 30)
        .find(|s| *leader(&r.keys, *s).public_key() != own)
        .expect("fixture: a slot led by another key");
    let by = leader(&r.keys, slot).clone();
    make_block(r, BASE + 1, slot, r.base_tip, &by, merkle_ok)
}

async fn tip(r: &Rig) -> (Hash, u64) {
    let cs = r.node.chain_state.read().await;
    (cs.best_hash, cs.best_height)
}

fn score(r: &Rig, peer: &PeerId) -> i32 {
    r.node
        .bls_ingress_scorer
        .get_score(peer)
        .map_or(0, |s| s.value)
}

async fn deliver(r: &mut Rig, b: &Block, from: PeerId) {
    r.node
        .handle_new_block(b.clone(), from)
        .await
        .expect("handle_new_block returns Ok");
}

/// Node clock 2 s behind the block time: only the future-time rule fails.
async fn ahead_by_two(r: &mut Rig, b: &Block) {
    let t = b.header.timestamp;
    set_clock(&mut r.node, t - 2).await;
    let v = r
        .node
        .validate_block_for_apply(b, BASE + 1, ValidationMode::Full)
        .await;
    assert_eq!(
        v,
        Err(ValidationError::TimestampTooFuture(t)),
        "PRECONDITION: at clock T-2 the block fails the future-time rule only"
    );
}

// REQ-CLOCK-004 — Decision: a failure means a sender is still scored as an invalid-block source for a block that is only ahead of our clock.
#[tokio::test]
async fn block_ahead_of_clock_costs_sender_no_score() {
    let mut r = rig().await;
    let b = next_block(&r, true);
    ahead_by_two(&mut r, &b).await;
    let p = PeerId::random();

    deliver(&mut r, &b, p).await;

    assert_eq!(
        tip(&r).await,
        (r.base_tip, BASE),
        "a block ahead of the clock is not applied"
    );
    let score = score(&r, &p);
    println!("CLOCK_PROBE sender_score={}", score);
    assert_eq!(
        score, 0,
        "the sender of a block ahead of the clock keeps score 0"
    );
}

// REQ-CLOCK-001 — Decision: a failure means the not-yet class swallowed a real body fault, so junk senders escape today's -100.
#[tokio::test]
async fn invalid_block_is_still_condemned() {
    let mut r = rig().await;
    let b = next_block(&r, false);
    set_clock(&mut r.node, b.header.timestamp).await;
    let v = r
        .node
        .validate_block_for_apply(&b, BASE + 1, ValidationMode::Full)
        .await;
    assert!(
        matches!(v, Err(ValidationError::InvalidMerkleRoot { .. })),
        "PRECONDITION: at a current clock the block fails on its merkle root; got {v:?}"
    );
    let q = PeerId::random();

    deliver(&mut r, &b, q).await;

    assert_eq!(
        tip(&r).await,
        (r.base_tip, BASE),
        "an invalid block is not applied"
    );
    assert_eq!(
        score(&r, &q),
        -100,
        "an invalid-block sender is still recorded at -100"
    );
}

// REQ-CLOCK-001 — Decision: a failure means a block ahead of the clock still leaves its sender refused, so the sender's copy never applies once its time comes.
#[tokio::test]
async fn ahead_block_from_same_sender_applies_once_its_time_comes() {
    let mut r = rig().await;
    let b = next_block(&r, true);
    ahead_by_two(&mut r, &b).await;
    let p = PeerId::random();

    deliver(&mut r, &b, p).await;
    assert_eq!(
        tip(&r).await,
        (r.base_tip, BASE),
        "PRECONDITION: not applied while ahead of the clock"
    );
    assert!(
        !r.node.body_refused_for_test(&b.hash(), &p),
        "a block ahead of the clock does not put its sender on the refused list for that hash"
    );

    set_clock(&mut r.node, b.header.timestamp).await;
    let v = r
        .node
        .validate_block_for_apply(&b, BASE + 1, ValidationMode::Full)
        .await;
    assert_eq!(v, Ok(()), "PRECONDITION: at its time the block is valid");

    deliver(&mut r, &b, p).await;

    assert_eq!(
        tip(&r).await,
        (b.hash(), BASE + 1),
        "the same sender's copy applies once the clock reaches the block time"
    );
}
