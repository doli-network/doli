//! A peer block whose only fault is a timestamp ahead of the receiver's clock.
//!
// OUTPUT CONTRACT: fn Node::handle_new_block(&mut self, Block, PeerId) -> Result<()>
//   and fn Node::run_periodic_tasks(&mut self) -> Result<()> (the hold release)
//   O1 return (Ok on every path); O2 chain tip (best_hash, best_height);
//   O3 bls_ingress_scorer score of the sender; O4 sender refusal for that hash
//   (body_retry failed set, read through Node::body_refused_for_test);
//   O5 hold map: Node::clock_hold_len, Node::clock_held(&hash)
// PATHS: P1 fails TimestampTooFuture only | P2 fails a body commitment (merkle)
//   | P3 same block, same sender, again once the node clock reaches its time
//   | P4 held, then ticks after its time | P5 ahead, timestamp one slot past its slot
//   | P6 33 in-band ahead blocks | P7 same block from two peers
//   | P8 two blocks at one slot | P9 ahead and beyond the band (wall slot + 2)
//   | P10 in-band slot, timestamp far past its slot | P11 held block, then 32 and 33 P10 blocks
// MATRIX
//   P1 x O1,O2,O3 -> Ok, tip unchanged, score 0
//   P2 x O1,O2,O3 -> Ok, tip unchanged, score -100
//   P1 x O4 -> sender not refused; P1 then P3 x O1,O2 -> Ok, tip == block
//   P4 x O2,O3 -> tip == block within 2 ticks, score 0
//   P5 x O2,O3 -> tip unchanged, score -100
//   P6 x O5 -> len 32, first gone, last held | P7 x O5 -> len 1
//   P8 x O5 -> len 2, both held | P9 x O2,O3,O5 -> tip unchanged, score -100, len 0
//   P10 x O3,O5 -> not held, len 0, score -100
//   P11 x O5 -> valid block held, len 1 after 32 and 33; P11 x O2 -> tip == block within 2 ticks
// INPUT PARTITIONS:
//   node clock: block time - 2 s (ahead) | -1 s (due) | block time | -12 s (out of band)
//   block: valid signed tip-extending | wrong merkle root, re-signed
//     | timestamp one slot past its slot, signed | same-slot variant (header vdf bytes)
//     | timestamp 5000 slots past its slot, signed by the leader or a fresh key
//   sender: one peer for both copies | distinct peer per test | two peers, one block
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

async fn tick(r: &mut Rig) {
    r.node
        .run_periodic_tasks()
        .await
        .expect("run_periodic_tasks returns Ok");
}

/// Node clock 2 s behind the block time: only the future-time rule fails.
async fn ahead_by_two(r: &mut Rig, b: &Block) {
    ahead_by(r, b, 2).await;
}

/// Node clock `secs` behind the block time; the block fails the future-time rule.
async fn ahead_by(r: &mut Rig, b: &Block, secs: u64) {
    let t = b.header.timestamp;
    set_clock(&mut r.node, t - secs).await;
    let v = r
        .node
        .validate_block_for_apply(b, BASE + 1, ValidationMode::Full)
        .await;
    assert_eq!(
        v,
        Err(ValidationError::TimestampTooFuture(t)),
        "PRECONDITION: at clock T-{secs} the block fails the future-time rule"
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

/// `next_block` with a timestamp one slot later than its slot, re-signed by its producer.
fn slot_mismatch_block(r: &Rig) -> Block {
    let mut b = next_block(r, true);
    let by = leader(&r.keys, b.header.slot).clone();
    b.header.timestamp += SLOT_SECS;
    sign_header(&mut b.header, &by);
    b
}

// REQ-CLOCK-003 — Decision: a failure means a block that was only ahead of our clock is never applied by the node itself once its time comes, so nodes with slower clocks stay on another tip.
#[tokio::test]
async fn held_block_applies_within_two_ticks_of_its_time() {
    let mut r = rig().await;
    let b = next_block(&r, true);
    ahead_by_two(&mut r, &b).await;
    let p = PeerId::random();
    deliver(&mut r, &b, p).await;

    tick(&mut r).await;
    tick(&mut r).await;
    assert_eq!(
        tip(&r).await,
        (r.base_tip, BASE),
        "CONTROL: two ticks while the block is still ahead do not move the tip"
    );

    set_clock(&mut r.node, b.header.timestamp - 1).await;
    let mut n: i32 = -1;
    for i in 1..=2 {
        tick(&mut r).await;
        if tip(&r).await.0 == b.hash() {
            n = i;
            break;
        }
    }

    println!("CLOCK_PROBE applied_after_ticks={}", n);
    assert_eq!(
        tip(&r).await,
        (b.hash(), BASE + 1),
        "a held block applies within 2 ticks once the clock reaches its time"
    );
    assert_eq!(
        score(&r, &p),
        0,
        "the sender of the held block keeps score 0"
    );
}

// REQ-CLOCK-002 — Decision: a failure means a block whose timestamp is outside its slot is applied, or its sender escapes today's penalty.
#[tokio::test]
async fn held_block_invalid_at_its_time_is_rejected() {
    let mut r = rig().await;
    let b = slot_mismatch_block(&r);
    set_clock(&mut r.node, b.header.timestamp).await;
    let v = r
        .node
        .validate_block_for_apply(&b, BASE + 1, ValidationMode::Full)
        .await;
    assert!(
        matches!(v, Err(ValidationError::InvalidSlot { .. })),
        "PRECONDITION: at its time the block fails the slot-from-time rule, not the future-time rule; got {v:?}"
    );
    ahead_by_two(&mut r, &b).await;
    let q = PeerId::random();
    deliver(&mut r, &b, q).await;

    set_clock(&mut r.node, b.header.timestamp - 1).await;
    tick(&mut r).await;
    tick(&mut r).await;

    assert_eq!(
        tip(&r).await,
        (r.base_tip, BASE),
        "a held block that is invalid at its time is not applied"
    );
    assert_eq!(
        score(&r, &q),
        -100,
        "the sender of a held block that is invalid at its time gets today's -100"
    );
}

/// `next_block`'s slot, height and parent, produced and signed by a fresh key.
fn block_by_fresh_key(r: &Rig) -> Block {
    let slot = next_block(r, true).header.slot;
    make_block(r, BASE + 1, slot, r.base_tip, &KeyPair::generate(), true)
}

// REQ-CLOCK-SEC-003 — Decision: a failure means the hold grows past 32 entries or drops the newest instead of the oldest, so a flood of ahead blocks costs unbounded memory or evicts the live block.
#[tokio::test]
async fn hold_drops_oldest_past_cap() {
    let mut r = rig().await;
    let blocks: Vec<Block> = (0..33).map(|_| block_by_fresh_key(&r)).collect();
    let hashes: std::collections::HashSet<Hash> = blocks.iter().map(Block::hash).collect();
    assert_eq!(hashes.len(), 33, "PRECONDITION: 33 distinct hashes");
    ahead_by(&mut r, &blocks[0], 5).await;

    for b in &blocks {
        deliver(&mut r, b, PeerId::random()).await;
    }

    assert_eq!(
        r.node.clock_hold_len(),
        32,
        "the hold keeps at most 32 entries"
    );
    assert!(
        !r.node.clock_held(&blocks[0].hash()),
        "the first inserted entry is dropped past the cap"
    );
    assert!(
        r.node.clock_held(&blocks[32].hash()),
        "the last inserted entry is held"
    );
}

// REQ-CLOCK-002 — Decision: a failure means one block delivered by several peers takes several hold entries, so honest gossip fan-out fills the cap.
#[tokio::test]
async fn same_block_twice_holds_one_entry() {
    let mut r = rig().await;
    let b = next_block(&r, true);
    ahead_by_two(&mut r, &b).await;

    deliver(&mut r, &b, PeerId::random()).await;
    deliver(&mut r, &b, PeerId::random()).await;

    assert_eq!(
        r.node.clock_hold_len(),
        1,
        "one block from two peers is one entry"
    );
    assert!(
        r.node.clock_held(&b.hash()),
        "the block is held by its hash"
    );
}

/// Same producer, same slot: the equivocation detector records a proof but does not stop intake.
// REQ-CLOCK-002 — Decision: a failure means a second block at one slot overwrites the first, so the hold picks a side instead of leaving it to fork choice.
#[tokio::test]
async fn two_blocks_one_slot_hold_two_entries() {
    let mut r = rig().await;
    let b1 = next_block(&r, true);
    let mut b2 = b1.clone();
    b2.header.vdf_output.value[0] = 1;
    sign_header(&mut b2.header, leader(&r.keys, b1.header.slot));
    assert_ne!(b1.hash(), b2.hash(), "PRECONDITION: two hashes at one slot");
    ahead_by_two(&mut r, &b1).await;

    deliver(&mut r, &b1, PeerId::random()).await;
    deliver(&mut r, &b2, PeerId::random()).await;

    assert_eq!(
        r.node.clock_hold_len(),
        2,
        "two blocks at one slot are two entries"
    );
    assert!(r.node.clock_held(&b1.hash()), "the first block is held");
    assert!(r.node.clock_held(&b2.hash()), "the second block is held");
}

// REQ-CLOCK-SEC-002 — Decision: a failure means blocks far ahead of the wall slot are held, so the hold accepts slots the gossip band refuses.
#[tokio::test]
async fn block_beyond_band_is_not_held() {
    let mut r = rig().await;
    let b = next_block(&r, true);
    ahead_by(&mut r, &b, SLOT_SECS + 2).await;
    assert_eq!(
        u64::from(b.header.slot),
        r.node.wall_now_secs() / SLOT_SECS + 2,
        "PRECONDITION: the block is two slots past the node wall slot"
    );
    let p = PeerId::random();

    deliver(&mut r, &b, p).await;

    assert_eq!(
        tip(&r).await,
        (r.base_tip, BASE),
        "the block is not applied"
    );
    assert_eq!(r.node.clock_hold_len(), 0, "nothing is held");
    assert!(!r.node.clock_held(&b.hash()), "the block is not held");
    assert_eq!(
        score(&r, &p),
        -100,
        "a block the hold refuses keeps the previous handling"
    );
}

/// `next_block`'s slot and parent with a timestamp 5000 slots later, signed by `by`.
fn far_time_block(r: &Rig, by: &KeyPair) -> Block {
    let mut b = next_block(r, true);
    b.header.timestamp += 5000 * SLOT_SECS;
    b.header.producer = *by.public_key();
    sign_header(&mut b.header, by);
    b
}

/// Node clock 2 s before the slot time of `b`; `b` fails the future-time rule.
async fn clock_before_slot(r: &mut Rig, b: &Block) {
    let slot_time = u64::from(b.header.slot) * SLOT_SECS;
    set_clock(&mut r.node, slot_time - 2).await;
    let v = r
        .node
        .validate_block_for_apply(b, BASE + 1, ValidationMode::Full)
        .await;
    assert_eq!(
        v,
        Err(ValidationError::TimestampTooFuture(b.header.timestamp)),
        "PRECONDITION: the block fails the future-time rule"
    );
}

// REQ-CLOCK-SEC-004 — Decision: a failure means a block whose timestamp is outside its own slot is held, so it never leaves the hold and takes a place under the cap.
#[tokio::test]
async fn block_with_time_outside_its_slot_is_not_held() {
    let mut r = rig().await;
    let by = leader(&r.keys, next_block(&r, true).header.slot).clone();
    let b = far_time_block(&r, &by);
    clock_before_slot(&mut r, &b).await;
    assert!(
        u64::from(b.header.slot) <= r.node.wall_now_secs() / SLOT_SECS + 1,
        "PRECONDITION: the slot is in the band"
    );
    let p = PeerId::random();

    deliver(&mut r, &b, p).await;

    assert!(!r.node.clock_held(&b.hash()), "the block is not held");
    assert_eq!(r.node.clock_hold_len(), 0, "nothing is held");
    assert_eq!(
        score(&r, &p),
        -100,
        "the sender gets the invalid-block score"
    );
}

// REQ-CLOCK-SEC-004 — Decision: a failure means blocks whose timestamp is outside their slot push a valid held block out of the hold, so it never applies at its time.
#[tokio::test]
async fn held_block_stays_after_blocks_with_time_outside_their_slot() {
    let mut r = rig().await;
    let b = next_block(&r, true);
    ahead_by_two(&mut r, &b).await;
    deliver(&mut r, &b, PeerId::random()).await;
    assert!(
        r.node.clock_held(&b.hash()),
        "PRECONDITION: the block is held"
    );

    let others: Vec<Block> = (0..33)
        .map(|_| far_time_block(&r, &KeyPair::generate()))
        .collect();
    for (i, o) in others.iter().enumerate() {
        deliver(&mut r, o, PeerId::random()).await;
        if i == 31 || i == 32 {
            assert!(
                r.node.clock_held(&b.hash()),
                "the valid block is held after {} other blocks",
                i + 1
            );
            assert_eq!(r.node.clock_hold_len(), 1, "only the valid block is held");
        }
    }

    set_clock(&mut r.node, b.header.timestamp - 1).await;
    for _ in 0..2 {
        tick(&mut r).await;
        if tip(&r).await.0 == b.hash() {
            break;
        }
    }
    assert_eq!(
        tip(&r).await,
        (b.hash(), BASE + 1),
        "the held block applies within 2 ticks of its time"
    );
}
