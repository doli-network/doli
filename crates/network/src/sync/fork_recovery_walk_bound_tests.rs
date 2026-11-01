// covers: fork_recovery.rs manager/fork_walk.rs manager/sync_engine/response.rs manager/sync_engine/dispatch.rs
//! INC-I-293 run 651 U-1 (operator ruling 3, AUDIT-P2-649-003): the by-hash walk
//! bound is 64 x the era block size limit, read from the authority block
//! validation uses: `doli_core::consensus::max_block_size(height)`
//! (`consensus/constants.rs:56`, era = height / BLOCKS_PER_ERA: 2 MB in Era 0,
//! doubling per era, 32 MB cap from Era 4), checked by `validation/block.rs:167`
//! and `:340` against `Block::size()`. At c041859e the bound is the fixed
//! `WALK_HELD_BYTES_MAX` = 32 MiB (`fork_recovery.rs:36`): 17 full Era-0 blocks
//! do not fit, the walk is cancelled, and the cancel blames the peer
//! (`fork_walk.rs:76-86`) without handing the walk to the next evidence peer.
//! The ruling: an overflow marks the peer (never asked again in this walk) and
//! hands the same walk to the next evidence peer, as a `None` answer does, and
//! does not blame it.
//!
//! "64 full blocks" counts every block the walk holds, the seed included (the
//! tracker counts the seed: `fork_recovery.rs:257-259`): 64 blocks of exactly
//! `max_block_size(h)` bytes fit, the 65th trips.
//!
//! OUTPUT CONTRACT (SyncManager::handle_response(Block) during an active walk
//!   -> walk_intake -> admit_bytes / ForkRecoveryTracker::handle_block;
//!   next_request walk fetch):
//!   O1 held set: ForkRecoveryTracker.active.blocks (count, bytes = Block::size)
//!   O2 walk active: is_fork_recovery_active()
//!   O3 next fetch after an overflow: next_request() -> (peer, GetBlockByHash)
//!   O4 blame: take_refused_body_suppliers() (the node's peer-score input)
//!   O5 completion: check_fork_recovery_connection(true) -> blocks, connection_point
//!   O6 escalation flag: take_exceeded_max_depth() (node -> genesis resync)
//! PATHS: answer held | answer over the bound (mark, hand over, no blame) |
//!   completion.
//! INPUT PARTITIONS: block size {= max_block_size(h) Era 0 (2 MB), = the Era-0
//!   limit at an Era-1 height (half the Era-1 limit)} x held count {64, 65, 66}
//!   x height {Era 0 above GATE, Era 1}; evidence peer {P_e2 set, none}.
//! MATRIX: sixty_four -> O1,O2,O5,O6 | overflow -> O1,O2,O3,O4,O6
//!   | tracks (Era 0) -> O1,O2 | tracks (Era 1) -> O1,O2,O5.
//! GATE: every cell walks above the INC-I-288 gate; the below-gate pin is
//!   `s2_pin_below_gate_walk_keeps_unbounded_hold` (walk_bytes_tests), unchanged.

use std::sync::Mutex;

use crypto::{Hash, KeyPair};
use doli_core::consensus::{max_block_size, BLOCKS_PER_ERA};
use doli_core::{Block, BlockHeader, Transaction};
use libp2p::PeerId;
use vdf::{VdfOutput, VdfProof};

use super::super::{SyncConfig, SyncManager};
use crate::protocols::{SyncRequest, SyncResponse};

/// INC-I-236 block-signature AH: every cell walks above it (headers are signed).
const SIG_AH: u64 = 100;
/// INC-I-288 fork-choice gate (testnet 366_891, mainnet MAX in production).
const GATE: u64 = 10_000;
/// The ruling: one walk holds at most this many full era blocks.
const FULL_BLOCKS: usize = 64;
/// An Era-0 local height well above GATE.
const ERA0_H: u64 = GATE + 50;
/// An Era-1 local height; the deepest walked block stays in Era 1.
const ERA1_H: u64 = BLOCKS_PER_ERA + 1_000;
const TS: u64 = 1_700_000_000;

/// The cells hold up to 66 x 2 MB each: run them one at a time.
static BIG: Mutex<()> = Mutex::new(());

struct Rig {
    m: SyncManager,
    /// The walk peer.
    e1: PeerId,
    /// The next evidence peer.
    e2: PeerId,
    /// A taller peer: the walk's F11 alternate.
    tall: PeerId,
    seed_hash: Hash,
    /// parents[0] = the seed's parent (first fetch) .. parents[last] = deepest.
    parents: Vec<Option<Block>>,
    hashes: Vec<Hash>,
    root: Hash,
    block_size: usize,
}

fn fixed_peer(b: u8) -> PeerId {
    libp2p::identity::Keypair::ed25519_from_bytes([b; 32])
        .expect("fixed ed25519 seed")
        .public()
        .to_peer_id()
}

fn tx(tag: u32, bytes: usize) -> Transaction {
    let mut t = Transaction::new_transfer(vec![], vec![]);
    let mut extra = vec![(tag % 251) as u8; bytes.max(4)];
    extra[..4].copy_from_slice(&tag.to_le_bytes());
    t.extra_data = extra;
    t
}

/// Self-signed by `kp`, merkle root over `txs`, presence root valid under both
/// presence arms for an empty bitfield (the walk_bytes fixture).
fn signed_block(prev: Hash, slot: u32, kp: &KeyPair, txs: Vec<Transaction>) -> Block {
    let mut h = BlockHeader {
        version: 1,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(&txs),
        presence_root: doli_core::presence_commitment(&[], &[]),
        genesis_hash: Hash::ZERO,
        timestamp: TS + slot as u64,
        slot,
        producer: *kp.public_key(),
        vdf_output: VdfOutput { value: vec![0; 32] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    h.vdf_proof.pi =
        crypto::signature::sign(h.signing_message().as_bytes(), kp.private_key()).to_vec();
    Block::new(h, txs)
}

/// Payload bytes that make a one-transaction block exactly `size` bytes.
fn payload_for(size: usize, kp: &KeyPair) -> usize {
    let probe = signed_block(Hash::ZERO, 1_000, kp, vec![tx(0, 4096)]);
    let overhead = probe.size() - 4096;
    size - overhead
}

/// Local tip at `local_h`; a branch of `total` blocks (the seed + `total - 1`
/// parents) of exactly `block_size` bytes each; the walk started from the seed
/// on P_e1, P_e2 set as the next evidence peer when `handover`.
fn rig(local_h: u64, total: usize, block_size: usize, handover: bool) -> Rig {
    let config = SyncConfig {
        inc_i_236_block_signature_activation_height: SIG_AH,
        inc_i_288_fork_choice_activation_height: GATE,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, Hash::ZERO);
    let local_hash = Hash::from_bytes([0x11; 32]);
    m.update_local_tip(local_h, local_hash, local_h as u32);
    let (e1, e2, tall) = (fixed_peer(0x29), fixed_peer(0x2a), fixed_peer(0x2b));
    let off_chain = Hash::from_bytes([0x33; 32]);
    m.add_peer(e1, local_h, off_chain, local_h as u32);
    m.add_peer(e2, local_h, off_chain, local_h as u32);
    m.add_peer(tall, local_h + 5, local_hash, local_h as u32 + 5);
    let kp = KeyPair::from_seed([0x5e; 32]);
    let payload = payload_for(block_size, &kp);
    let root = Hash::from_bytes([0x22; 32]);
    let mut chain = Vec::with_capacity(total);
    let mut prev = root;
    for i in 0..total {
        let b = signed_block(prev, 1_000 + i as u32, &kp, vec![tx(i as u32, payload)]);
        prev = b.hash();
        chain.push(b);
    }
    for b in [&chain[0], &chain[total - 1]] {
        assert_eq!(b.size(), block_size, "setup: block is exactly the size");
        assert!(b.body_matches_header(GATE, None), "setup: self-consistent");
    }
    let seed = chain.pop().expect("seed");
    chain.reverse();
    let seed_hash = seed.hash();
    assert!(
        m.start_fork_recovery(seed, e1),
        "setup: walk starts on P_e1"
    );
    if handover {
        m.set_fork_walk_handover(vec![e2]);
    }
    Rig {
        m,
        e1,
        e2,
        tall,
        seed_hash,
        hashes: chain.iter().map(Block::hash).collect(),
        parents: chain.into_iter().map(Some).collect(),
        root,
        block_size,
    }
}

fn held(m: &SyncManager) -> usize {
    (m.fork.fork_recovery.active.as_ref()).map_or(0, |r| r.blocks.len())
}

fn name(r: &Rig, p: PeerId) -> &'static str {
    match p {
        p if p == r.e1 => "P_e1",
        p if p == r.e2 => "P_e2",
        p if p == r.tall => "P_tall",
        _ => "other",
    }
}

/// O3: the next request as (peer name, hash); a request that is not a by-hash
/// fetch is named "not a walk fetch" with a zero hash.
fn next_fetch(r: &mut Rig) -> Option<(&'static str, Hash)> {
    let (to, req) = r.m.next_request()?;
    let SyncRequest::GetBlockByHash { hash } = req else {
        return Some(("not a walk fetch", Hash::ZERO));
    };
    Some((name(r, to), hash))
}

/// How the walk from P_e1 went: the hop (1-based) whose answer was not held, if any.
struct Run {
    tripped_at: Option<usize>,
    /// (held blocks, held bytes) just before the trip, or at the end.
    held: (usize, usize),
}

/// P_e1 answers hops 1..=`hops` in order while the walk asks it; stops at the
/// first answer that is not held.
fn walk_from_e1(r: &mut Rig, hops: usize) -> Run {
    for k in 0..hops {
        let before = held(&r.m);
        let asked = next_fetch(r);
        assert_eq!(
            asked,
            Some(("P_e1", r.hashes[k])),
            "setup: hop {} is asked of the walk peer P_e1 ({} blocks held)",
            k + 1,
            before
        );
        let b = r.parents[k].clone().expect("each parent is answered once");
        let out = r.m.handle_response(r.e1, SyncResponse::Block(Some(b)));
        assert!(out.is_empty(), "a walk answer is never passed to apply");
        if held(&r.m) == before + 1 {
            r.parents[k] = None;
        } else {
            return Run {
                tripped_at: Some(k + 1),
                held: (before, before * r.block_size),
            };
        }
    }
    let n = held(&r.m);
    Run {
        tripped_at: None,
        held: (n, n * r.block_size),
    }
}

// U1B-1 INC-I-293 AUDIT-P2-649-003 (Must) — Decision: a failure means an honest heavy branch of full blocks (64 at the era limit) still cannot be walked because the bound is a fixed byte count below 64 x the validation limit, so a minority on a full-block fork never heals by the walk.
#[test]
fn u1b_sixty_four_full_blocks_heal_uncancelled() {
    let _one = BIG.lock().unwrap_or_else(|e| e.into_inner());
    let limit = max_block_size(ERA0_H + 1);
    let mut r = rig(ERA0_H, FULL_BLOCKS, limit, true);
    let run = walk_from_e1(&mut r, FULL_BLOCKS - 1);
    assert!(
        run.tripped_at.is_none(),
        "the walk was cancelled at hop {:?} with reason \"held body bytes over the walk \
         bound\" (the only cancel of a held answer under 1000 hops; walk active {}, {} \
         full blocks = {} B held before it); 64 full blocks must fit: 64 x \
         max_block_size({}) = {} B",
        run.tripped_at,
        r.m.is_fork_recovery_active(),
        run.held.0,
        run.held.1,
        ERA0_H + 1,
        FULL_BLOCKS * limit
    );
    assert_eq!(held(&r.m), FULL_BLOCKS, "O1: the seed + 63 parents held");
    let done =
        r.m.check_fork_recovery_connection(true)
            .expect("O5: the 64-block walk completes and reaches fork choice");
    assert_eq!(done.connection_point, r.root, "O5: connection point");
    assert_eq!(done.blocks.len(), FULL_BLOCKS, "O5: every block handed on");
    assert_eq!(
        done.blocks.last().map(Block::hash),
        Some(r.seed_hash),
        "O5: seed last"
    );
    assert!(
        !r.m.fork.fork_recovery.take_exceeded_max_depth(),
        "O6: no genesis-resync escalation"
    );
}

// U1B-2 INC-I-293 AUDIT-P2-649-003 (Must) — Decision: a failure means a walk peer whose branch overflows the bound is still blamed (scored as a bad body supplier for honest full blocks) and the walk is cancelled instead of being handed to the next evidence peer, so one heavy honest branch costs every serving peer its score and the walk slot.
#[test]
fn u1b_overflow_marks_peer_hands_over_without_blame() {
    let _one = BIG.lock().unwrap_or_else(|e| e.into_inner());
    let limit = max_block_size(ERA0_H + 1);
    let mut r = rig(ERA0_H, FULL_BLOCKS + 2, limit, true);
    let run = walk_from_e1(&mut r, FULL_BLOCKS + 1);
    let Some(hop) = run.tripped_at else {
        panic!(
            "control: 66 full blocks never tripped the bound ({} held)",
            run.held.0
        );
    };
    assert!(
        !r.m.fork.fork_recovery.take_exceeded_max_depth(),
        "control O6: a bound trip must not raise the genesis-resync escalation"
    );
    let trip_hash = r.hashes[hop - 1];
    let blamed: Vec<u32> = (r.m.take_refused_body_suppliers().into_iter())
        .filter(|(p, _)| *p == r.e1)
        .map(|(_, slot)| slot)
        .collect();
    let active = r.m.is_fork_recovery_active();
    let next = next_fetch(&mut r);
    assert!(
        blamed.is_empty() && active && next == Some(("P_e2", trip_hash)),
        "after the bound trip at hop {hop} ({} full blocks held): P_e1 blamed at slots \
         {blamed:?}, walk active {active}, next walk fetch {next:?}; expected P_e1 marked \
         and not blamed, and the same walk asking the next evidence peer P_e2 for the same \
         hash {trip_hash}",
        run.held.0
    );
    let again = r.parents[hop - 1].clone().expect("the trip block is kept");
    let out = r.m.handle_response(r.e2, SyncResponse::Block(Some(again)));
    assert!(out.is_empty(), "a walk answer is never passed to apply");
    let after = next_fetch(&mut r);
    assert!(
        !matches!(after, Some(("P_e1", _))),
        "P_e1 overflowed the bound but the walk asked it again after P_e2 overflowed \
         too: {after:?}"
    );
}

// U1B-3 INC-I-293 AUDIT-P2-649-003 (Must) — Decision: a failure means the walk bound is not exactly 64 x max_block_size at the walked era (too small: honest full branches cancel; too large: a peer holds more than the ruling allows), so the bound and block validation disagree on what a full block is.
#[test]
fn u1b_bound_tracks_the_validation_limit() {
    let _one = BIG.lock().unwrap_or_else(|e| e.into_inner());
    let limit = max_block_size(ERA0_H + 1);
    let mut r = rig(ERA0_H, FULL_BLOCKS + 1, limit, false);
    let run = walk_from_e1(&mut r, FULL_BLOCKS);
    assert_eq!(
        run.tripped_at,
        Some(FULL_BLOCKS),
        "the bound is not 64 x max_block_size({}) = {} B: with blocks of exactly the \
         limit the walk must hold 64 (the seed + 63 parents) and trip on the 65th (hop \
         64); it held {} blocks ({} B) and tripped at hop {:?}",
        ERA0_H + 1,
        FULL_BLOCKS * limit,
        run.held.0,
        run.held.1,
        run.tripped_at
    );
}

// U1B-4 INC-I-293 AUDIT-P2-649-003 (Must, era) — Decision: a failure means the bound does not follow the era of max_block_size (e.g. a fixed Era-0 or cap constant), so after the first era doubling honest full branches no longer fit, or the bound is far above the ruling.
#[test]
fn u1b_bound_tracks_the_validation_limit_in_era_1() {
    let _one = BIG.lock().unwrap_or_else(|e| e.into_inner());
    let era0 = max_block_size(0);
    let era1 = max_block_size(ERA1_H + 1);
    assert_eq!(era1, 2 * era0, "setup: Era 1 doubles the Era-0 limit");
    let total = FULL_BLOCKS + 1;
    let mut r = rig(ERA1_H, total, era0, true);
    let run = walk_from_e1(&mut r, total - 1);
    assert!(
        run.tripped_at.is_none(),
        "at an Era-1 height (h={}) the bound must be 64 x max_block_size = {} B, but the \
         walk tripped at hop {:?} holding {} blocks of the Era-0 limit ({} B); the 65 \
         blocks of this branch are above 64 x the Era-0 limit = {} B and must still fit",
        ERA1_H + 1,
        FULL_BLOCKS * era1,
        run.tripped_at,
        run.held.0,
        run.held.1,
        FULL_BLOCKS * era0
    );
    let done = r.m.check_fork_recovery_connection(true);
    assert_eq!(
        done.map(|d| d.blocks.len()),
        Some(total),
        "O5: the 65-block Era-1 walk completes"
    );
}
