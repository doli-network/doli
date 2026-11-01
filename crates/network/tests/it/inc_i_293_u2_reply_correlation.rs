// covers: sync/fork_recovery.rs sync/manager/fork_walk.rs sync/manager/sync_engine/response.rs service/behaviour_events.rs service/command_handling.rs service/types.rs
//! INC-I-293 run 651 U-2 (operator ruling 6, lab observation (b), SEC-CR-649-003 /
//! SEC-ID-648-003 class): a by-hash fork walk in flight to peer P was cancelled
//! with `peer does not have block` 0.9 ms before P's full reply arrived.
//!
//! Structural cause at c041859e (each link read in the code):
//! - `GetBlockByHeight` and `GetBlockByHash` share ONE response type on the wire:
//!   `SyncResponse::Block(Option<Block>)` (protocols/sync.rs; node block_serve.rs
//!   answers both with `Block(None)` when it has no block).
//! - The swarm loop drops the libp2p `OutboundRequestId` on send
//!   (command_handling.rs `sync.send_request(..)` result unused) and on receive
//!   (behaviour_events.rs `Message::Response { response, .. }`);
//!   `NetworkEvent::SyncResponse` carries only `{ peer_id, response }`.
//! - `SyncManager::handle_response` feeds every `Block(_)` from a peer to
//!   `walk_intake` (sync_engine/response.rs), and `ForkRecoveryTracker::handle_block`
//!   takes a `None` from the walk peer as the walk's answer (keyed on peer +
//!   pending only; only `Some` is content-keyed by hash).
//! - Above the gate the node's orphan path sends `GetBlockByHeight{tip+1}` to the
//!   orphan's sender and then starts the walk on the SAME sender (block_handling.rs
//!   orphan chase, then `try_trigger_fork_recovery_from(Some(source_peer))`). A
//!   sender on a sibling branch that is not longer than ours has no block at
//!   tip+1: its empty by-height reply ends the walk.
//!
//! The libp2p request id is LOCAL state (it never crosses the wire), so the fix
//! needs no wire change: keep the outbound id -> request map in the swarm loop and
//! hand the answered request to the manager with the response.
//!
// OUTPUT CONTRACT: fn SyncManager::handle_response(peer, SyncResponse::Block(None|Some))
//   during an active walk (-> walk_intake -> ForkRecoveryTracker::handle_block),
//   fn SyncManager::next_request() (walk fetch)
//   O1: walk active: is_fork_recovery_active()
//   O2: handle_response return: the walk's answer is consumed (empty), never sent
//       to apply; a reply the walk did not ask for passes through
//   O3: next walk fetch: next_request() -> (peer, GetBlockByHash{hash})
// PATHS: E (an empty reply to an earlier GetBlockByHeight from the walk peer, no
//   evidence peer left) -> red_cancel; E' (same, one evidence peer Q) -> red_handover;
//   N (the walk's own GetBlockByHash answered None, evidence peer Q) -> control;
//   E below the gate -> pin
// INPUT PARTITIONS: local height above the INC-I-288 gate | below it (above the
//   INC-I-236 signature AH in both); handover list empty | [Q]; the reply answers
//   GetBlockByHeight{tip+1} | the walk's GetBlockByHash{R}
// MATRIX: E x O1 O2 O3 -> red_cancel · E' x O1 O2 O3 -> red_handover
//   · N x O1 O3 -> control · E(below) x O1 O2 -> pin

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader, Transaction};
use libp2p::PeerId;
use network::protocols::{SyncRequest, SyncResponse};
use network::sync::{SyncConfig, SyncManager};
use vdf::{VdfOutput, VdfProof};

/// INC-I-236 block-signature AH: every cell is above it (walk headers are signed).
const SIG_AH: u64 = 100;
/// INC-I-288 fork-choice gate (testnet 366_891, mainnet MAX in production).
const GATE: u64 = 10_000;
const ABOVE: u64 = 20_000;
const BELOW: u64 = 5_000;
const TS: u64 = 1_700_000_000;

fn peer(seed: u8) -> PeerId {
    libp2p::identity::Keypair::ed25519_from_bytes([seed; 32])
        .expect("fixed ed25519 seed")
        .public()
        .to_peer_id()
}

fn tx(tag: u32) -> Transaction {
    let mut t = Transaction::new_transfer(vec![], vec![]);
    t.extra_data = tag.to_le_bytes().to_vec();
    t
}

/// A walk block the intake accepts: self-signed header, merkle root over its body,
/// presence root valid for an empty bitfield.
fn signed_block(prev: Hash, slot: u32, kp: &KeyPair) -> Block {
    let txs = vec![tx(slot)];
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

struct Rig {
    m: SyncManager,
    /// The walk peer P: the orphan's sender, on a sibling branch at our height.
    p: PeerId,
    /// R: the seed's parent, the walk's first by-hash fetch.
    r: Block,
    /// R's parent (unknown to us): the walk's second fetch once R is held.
    root: Hash,
    local_h: u64,
}

/// Local tip at `local_h`; P reports the same height on a sibling branch
/// root <- R <- S; P gossiped S (an orphan), so the node sent P the orphan chase
/// `GetBlockByHeight{local_h + 1}` and then started the walk on P.
fn rig(local_h: u64, handover: Vec<PeerId>) -> Rig {
    let config = SyncConfig {
        inc_i_236_block_signature_activation_height: SIG_AH,
        inc_i_288_fork_choice_activation_height: GATE,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, Hash::ZERO);
    let local_hash = Hash::from_bytes([0x11; 32]);
    m.update_local_tip(local_h, local_hash, local_h as u32);
    let p = peer(0x29);
    let branch_tip = Hash::from_bytes([0x33; 32]);
    m.add_peer(p, local_h, branch_tip, local_h as u32);
    for q in &handover {
        m.add_peer(*q, local_h, branch_tip, local_h as u32);
    }
    let kp = KeyPair::from_seed([0x5e; 32]);
    let root = Hash::from_bytes([0x22; 32]);
    let r = signed_block(root, local_h as u32 - 1, &kp);
    let s = signed_block(r.hash(), local_h as u32, &kp);
    assert!(
        r.body_matches_header(GATE, None),
        "setup: R matches its header"
    );
    assert!(m.start_fork_recovery(s, p), "setup: the walk starts on P");
    m.set_fork_walk_handover(handover);
    assert_eq!(
        walk_fetch(&mut m),
        Some((p, r.hash())),
        "setup: the walk asks P for R by hash"
    );
    Rig {
        m,
        p,
        r,
        root,
        local_h,
    }
}

/// O3: the next request as (peer, hash) when it is a by-hash fetch.
fn walk_fetch(m: &mut SyncManager) -> Option<(PeerId, Hash)> {
    let (to, req) = m.next_request()?;
    let SyncRequest::GetBlockByHash { hash } = req else {
        panic!("O3: expected a by-hash walk fetch, got {req:?} to {to}");
    };
    Some((to, hash))
}

/// The node's orphan chase to P, sent BEFORE the walk fetch (block_handling.rs).
fn orphan_chase(r: &Rig) -> SyncRequest {
    SyncRequest::GetBlockByHeight {
        height: r.local_h + 1,
    }
}

/// Deliver P's reply to `answers`. At c041859e the reply reaches the manager as
/// `(peer, response)` only: `answers` has no slot on the path. The fix threads the
/// answered request (local outbound-request state) into the manager; update ONLY
/// this helper's body to pass it, never the assertions.
fn deliver(
    m: &mut SyncManager,
    from: PeerId,
    answers: &SyncRequest,
    resp: SyncResponse,
) -> Vec<Block> {
    let _ = answers;
    m.handle_response(from, resp)
}

#[test]
fn u2_empty_height_reply_does_not_cancel_the_by_hash_walk() {
    let mut r = rig(ABOVE, Vec::new());
    let chase = orphan_chase(&r);
    let out = deliver(&mut r.m, r.p, &chase, SyncResponse::Block(None));
    assert!(out.is_empty(), "O2: an empty reply applies nothing");
    assert!(
        r.m.is_fork_recovery_active(),
        "U-2: P's empty reply to the earlier GetBlockByHeight{{{}}} ended the by-hash walk \
         for R {:.16} (taken as P's answer to GetBlockByHash)",
        r.local_h + 1,
        r.r.hash()
    );
    let ask = SyncRequest::GetBlockByHash { hash: r.r.hash() };
    let out = deliver(&mut r.m, r.p, &ask, SyncResponse::Block(Some(r.r.clone())));
    assert!(
        out.is_empty(),
        "O2: P's answer R is the walk's (consumed), not sent to apply"
    );
    assert_eq!(
        walk_fetch(&mut r.m),
        Some((r.p, r.root)),
        "O3: with R held the walk asks P for R's parent"
    );
}

#[test]
fn u2_empty_height_reply_does_not_move_the_walk_off_its_peer() {
    let q = peer(0x31);
    let mut r = rig(ABOVE, vec![q]);
    let chase = orphan_chase(&r);
    let _ = deliver(&mut r.m, r.p, &chase, SyncResponse::Block(None));
    assert!(r.m.is_fork_recovery_active(), "O1: the walk is active");
    let ask = SyncRequest::GetBlockByHash { hash: r.r.hash() };
    let out = deliver(&mut r.m, r.p, &ask, SyncResponse::Block(Some(r.r.clone())));
    assert!(
        out.is_empty(),
        "U-2: P's empty reply to the earlier GetBlockByHeight{{{}}} moved the walk to \
         evidence peer {}; P's answer R {:.16} then passed through as a plain block",
        r.local_h + 1,
        q,
        r.r.hash()
    );
    assert_eq!(
        walk_fetch(&mut r.m),
        Some((r.p, r.root)),
        "O3: with R held the walk asks P for R's parent"
    );
}

#[test]
fn u2_control_the_walks_own_none_still_hands_over() {
    let q = peer(0x31);
    let mut r = rig(ABOVE, vec![q]);
    let ask = SyncRequest::GetBlockByHash { hash: r.r.hash() };
    let _ = deliver(&mut r.m, r.p, &ask, SyncResponse::Block(None));
    assert!(
        r.m.is_fork_recovery_active(),
        "O1: the walk survives its own None"
    );
    assert_eq!(
        walk_fetch(&mut r.m),
        Some((q, r.r.hash())),
        "O3: P's None to the walk's own fetch moves the same walk to evidence peer Q"
    );
}

#[test]
fn u2_pin_below_gate_an_empty_reply_still_ends_the_walk() {
    let mut r = rig(BELOW, Vec::new());
    let chase = orphan_chase(&r);
    let _ = deliver(&mut r.m, r.p, &chase, SyncResponse::Block(None));
    assert!(
        !r.m.is_fork_recovery_active(),
        "pin: below the gate an empty Block reply from the walk peer ends the walk (old rule)"
    );
    let ask = SyncRequest::GetBlockByHash { hash: r.r.hash() };
    let out = deliver(&mut r.m, r.p, &ask, SyncResponse::Block(Some(r.r.clone())));
    assert_eq!(
        out.iter().map(Block::hash).collect::<Vec<_>>(),
        vec![r.r.hash()],
        "pin: below the gate P's later answer passes through to block handling"
    );
}
