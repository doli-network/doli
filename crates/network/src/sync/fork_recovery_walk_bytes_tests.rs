// covers: fork_recovery.rs manager/fork_walk.rs manager/sync_engine/response.rs manager/sync_engine/dispatch.rs
//! INC-I-293 S-2 (run 649), AUDIT-P2-648-002 / SEC-IN-648-002, operator ruling 3:
//! a by-hash walk must bound the body bytes it holds, or check each body's
//! commitment per hop. Today `walk_intake` checks only the header self-signature
//! and the tracker holds up to 1000 bodies, each bounded only by the 16 MiB codec
//! frame; body/header matching runs only at completion (node `fork_walk.rs`).
//!
//! OUTPUT CONTRACT (SyncManager::handle_response(Block) during an active walk
//!   -> walk_intake -> ForkRecoveryTracker::handle_block; next_request walk fetch):
//!   O1 held set: ForkRecoveryTracker.active.blocks (count, bytes = Block::size)
//!   O2 walk active: is_fork_recovery_active()
//!   O3 handle_response return: a walk answer is consumed (empty), never sent to apply
//!   O4 next fetch: next_request() -> (walk peer, GetBlockByHash{next parent})
//!   O5 completion: check_fork_recovery_connection(true) -> blocks, connection_point
//!   O6 escalation flag: take_exceeded_max_depth() (node -> genesis resync)
//!   O7 peer blame: no crate-level observable today -> not asserted (reported)
//! PATHS: answer held | refused by size (cancel, held set dropped) | refused by
//!   commitment (consumed, not held) | completion.
//! INPUT PARTITIONS: body {64 KiB honest, 1 MiB self-consistent (merkle + presence
//!   match the attacker's own header), small junk not matching its header}
//!   x height {local_height+1 > GATE, SIG_AH < local_height+1 < GATE}.
//! MATRIX: b1 -> O1,O2,O3,O4,O5,O6 | b2 -> O1,O3,O4,O5 | control -> O1,O2,O3,O4,O5,O6
//!   | pin -> O1,O2,O3,O4,O5.
//! ROUTES: b1 = primary route (byte bound). b2 = the alternative route (per-hop
//!   commitment). b2 alone does NOT close SEC-IN-648-002: the attacker signs its own
//!   headers, so its 1 MiB bodies match them (b1 bodies pass `body_matches_header`).
//! GATE: the crate walk path (walk_intake, handle_block) has no INC-I-288 gate
//!   today; only the INC-I-236 signature check is gated. The pin fixes today's
//!   below-gate hold; an ungated bound turns the pin red and must be justified.

use std::collections::HashMap;

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader, Transaction};
use libp2p::PeerId;
use vdf::{VdfOutput, VdfProof};

use super::super::{SyncConfig, SyncManager};
use crate::protocols::{SyncRequest, SyncResponse};

/// INC-I-236 block-signature AH: every cell walks above it (headers are signed).
const SIG_AH: u64 = 100;
/// INC-I-288 fork-choice gate (testnet 366_891, mainnet MAX in production).
const GATE: u64 = 10_000;
/// Proposed bound B on the body bytes one walk holds (developer may rename).
const WALK_HELD_BYTES_MAX: usize = 32 * 1024 * 1024;
const MIB: usize = 1024 * 1024;
const TS: u64 = 1_700_000_000;

struct Rig {
    m: SyncManager,
    peer: PeerId,
    seed: Block,
    /// parents[0] = the seed's parent (first fetch) .. parents[last] = deepest.
    parents: Vec<Block>,
    sizes: HashMap<Hash, usize>,
    root: Hash,
}

fn walk_peer() -> PeerId {
    libp2p::identity::Keypair::ed25519_from_bytes([0x29u8; 32])
        .expect("fixed ed25519 seed")
        .public()
        .to_peer_id()
}

/// One transaction carrying `bytes` of payload, tagged so every body differs.
fn tx(tag: u32, bytes: usize) -> Transaction {
    let mut t = Transaction::new_transfer(vec![], vec![]);
    let mut extra = vec![(tag % 251) as u8; bytes.max(4)];
    extra[..4].copy_from_slice(&tag.to_le_bytes());
    t.extra_data = extra;
    t
}

/// A header the walk accepts today: self-signed by `kp`, merkle root over `txs`,
/// presence root valid under both the pre- and post-BLS arm for an empty bitfield.
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

/// Local tip at `local_h`; a `hops`-deep signed branch of `body_bytes` bodies;
/// the walk started from its tip (the seed) on the walk peer.
fn rig(local_h: u64, hops: usize, body_bytes: usize) -> Rig {
    let config = SyncConfig {
        inc_i_236_block_signature_activation_height: SIG_AH,
        inc_i_288_fork_choice_activation_height: GATE,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, Hash::ZERO);
    let local_hash = Hash::from_bytes([0x11; 32]);
    m.update_local_tip(local_h, local_hash, local_h as u32);
    let peer = walk_peer();
    m.add_peer(peer, local_h, local_hash, local_h as u32);
    let kp = KeyPair::from_seed([0x5e; 32]);
    let root = Hash::from_bytes([0x22; 32]);
    let mut chain = Vec::with_capacity(hops + 1);
    let mut prev = root;
    for i in 0..=hops {
        let b = signed_block(prev, 1_000 + i as u32, &kp, vec![tx(i as u32, body_bytes)]);
        prev = b.hash();
        chain.push(b);
    }
    let seed = chain.pop().expect("seed");
    chain.reverse();
    let mut sizes = HashMap::new();
    for b in chain.iter().chain(std::iter::once(&seed)) {
        assert!(
            b.body_matches_header(GATE, None),
            "setup: body matches its own header"
        );
        sizes.insert(b.hash(), b.size());
    }
    assert!(
        m.start_fork_recovery(seed.clone(), peer),
        "setup: walk starts"
    );
    Rig {
        m,
        peer,
        seed,
        parents: chain,
        sizes,
        root,
    }
}

fn held(m: &SyncManager) -> &[Block] {
    m.fork
        .fork_recovery
        .active
        .as_ref()
        .map(|r| r.blocks.as_slice())
        .unwrap_or(&[])
}

fn held_bytes(r: &Rig) -> usize {
    held(&r.m)
        .iter()
        .map(|b| r.sizes.get(&b.hash()).copied().unwrap_or_else(|| b.size()))
        .sum()
}

/// O4: the walk asks the walk peer for `want`; false when no fetch is issued.
fn ask(r: &mut Rig, want: Hash) -> bool {
    let Some((to, req)) = r.m.next_request() else {
        return false;
    };
    assert_eq!(to, r.peer, "O4: walk fetch goes to the walk peer");
    assert!(
        matches!(req, SyncRequest::GetBlockByHash { hash } if hash == want),
        "O4: walk asks for the next parent"
    );
    true
}

/// O3: the walk peer answers; a walk answer never reaches the apply path.
fn answer(r: &mut Rig, b: Block) {
    let out = r.m.handle_response(r.peer, SyncResponse::Block(Some(b)));
    assert!(
        out.is_empty(),
        "O3: a walk answer is consumed, never passed to apply"
    );
}

// S2B1 INC-I-293 AUDIT-P2-648-002 (Must) — Decision: a failure means one unstaked walk peer can still make the node hold far more than WALK_HELD_BYTES_MAX of self-consistent bodies (≈ 16 GiB at 1000 × 16 MiB), so ruling 3 is not met and the gate cannot be pinned on mainnet.
#[test]
fn s2_b1_walk_holds_at_most_walk_held_bytes_max_above_gate() {
    let mut r = rig(GATE + 50, 64, MIB);
    for k in 0..r.parents.len() {
        let want = r.parents[k].hash();
        if !r.m.is_fork_recovery_active() || !ask(&mut r, want) {
            break;
        }
        let b = r.parents[k].clone();
        answer(&mut r, b);
        let (n, bytes) = (held(&r.m).len(), held_bytes(&r));
        assert!(
            bytes <= WALK_HELD_BYTES_MAX,
            "WALK_HELD_BYTES_MAX = {} B: the walk holds {} bytes in {} bodies after hop {} \
             of 64 self-consistent 1 MiB answers from one walk peer",
            WALK_HELD_BYTES_MAX,
            bytes,
            n,
            k + 1
        );
    }
    assert!(
        !r.m.is_fork_recovery_active(),
        "O2: the answer that would pass WALK_HELD_BYTES_MAX must cancel the walk"
    );
    assert!(held(&r.m).is_empty(), "O1: the held set is dropped");
    assert!(
        !r.m.fork.fork_recovery.take_exceeded_max_depth(),
        "O6: a byte cancel must not raise the genesis-resync escalation"
    );
    assert!(
        r.m.check_fork_recovery_connection(true).is_none(),
        "O5: a cancelled walk hands nothing to fork choice"
    );
}

// S2B2 INC-I-293 AUDIT-P2-648-002 INV-FORK-005 (Must, alternative route) — Decision: a failure means a body that does not match its own header is still held until the walk completes, so junk bodies cost memory for the whole walk instead of being refused at the hop that brought them.
#[test]
fn s2_b2_walk_refuses_body_not_matching_its_header_at_intake() {
    let mut r = rig(GATE + 50, 4, 2 * 1024);
    for k in 0..2 {
        let want = r.parents[k].hash();
        assert!(ask(&mut r, want), "setup: honest hop {} fetched", k + 1);
        let b = r.parents[k].clone();
        answer(&mut r, b);
    }
    assert_eq!(held(&r.m).len(), 3, "setup: seed + two honest hops held");
    let honest = r.parents[2].clone();
    let mut junk = honest.clone();
    junk.transactions = vec![tx(0xBAD, 2 * 1024)];
    assert_eq!(
        junk.hash(),
        honest.hash(),
        "setup: the body is outside the hash"
    );
    assert!(
        !junk.verify_merkle_root(),
        "setup: junk body does not match header"
    );
    assert!(ask(&mut r, honest.hash()), "setup: hop 3 fetched");
    answer(&mut r, junk);
    let bad: Vec<Hash> = held(&r.m)
        .iter()
        .filter(|b| !b.verify_merkle_root())
        .map(|b| b.hash())
        .collect();
    assert!(
        bad.is_empty(),
        "INV-FORK-005: after hop 3 the walk holds {} of {} bodies that do not match \
         their header; the body must be refused at intake, not at completion",
        bad.len(),
        held(&r.m).len()
    );
    assert_ne!(
        r.m.fork_recovery_current_parent(),
        Some(honest.header.prev_hash),
        "O4: the walk advanced past a refused body"
    );
    if let Some(done) = r.m.check_fork_recovery_connection(true) {
        assert!(
            done.blocks.iter().all(|b| b.verify_merkle_root()),
            "O5: a junk body reached fork choice"
        );
    }
}

// S2C INC-I-293 AUDIT-P2-648-002 (Must, control) — Decision: a failure means the bound or the per-hop check also stops an honest 50-hop walk of normal bodies, so a minority node could no longer heal onto the heavier branch (the INC-I-293 failure shape).
#[test]
fn s2_control_honest_50_hop_walk_completes_above_gate() {
    let mut r = rig(GATE + 50, 50, 64 * 1024);
    for k in 0..r.parents.len() {
        assert!(
            r.m.is_fork_recovery_active(),
            "O2: honest walk alive at hop {}",
            k + 1
        );
        let want = r.parents[k].hash();
        assert!(ask(&mut r, want), "O4: honest hop {} fetched", k + 1);
        let b = r.parents[k].clone();
        answer(&mut r, b);
        assert_eq!(held(&r.m).len(), k + 2, "O1: honest hop {} held", k + 1);
    }
    let done =
        r.m.check_fork_recovery_connection(true)
            .expect("O5: honest walk completes");
    assert_eq!(done.connection_point, r.root, "O5: connection point");
    let mut want: Vec<Hash> = r.parents.iter().rev().map(|b| b.hash()).collect();
    want.push(r.seed.hash());
    let got: Vec<Hash> = done.blocks.iter().map(|b| b.hash()).collect();
    assert_eq!(got, want, "O5: forward order, seed last");
    assert!(
        done.blocks.iter().all(|b| b.verify_merkle_root()),
        "O5: honest bodies intact"
    );
    assert!(
        !r.m.fork.fork_recovery.take_exceeded_max_depth(),
        "O6: no escalation on an honest walk"
    );
}

// S2P INC-I-293 AUDIT-P2-648-002 (Must, below-gate pin) — Decision: a failure means the byte bound reaches below the INC-I-288 gate, so mainnet walk behaviour changes before the pin; gate it or justify an ungated bound and restate this pin.
#[test]
fn s2_pin_below_gate_walk_keeps_unbounded_hold() {
    let mut r = rig(GATE - 50, 64, MIB);
    for k in 0..r.parents.len() {
        let want = r.parents[k].hash();
        assert!(
            ask(&mut r, want),
            "pin O4: below-gate hop {} fetched",
            k + 1
        );
        let b = r.parents[k].clone();
        answer(&mut r, b);
    }
    assert!(
        r.m.is_fork_recovery_active(),
        "pin O2: below the gate the walk is not cut by size"
    );
    assert_eq!(held(&r.m).len(), 65, "pin O1: seed + 64 bodies held");
    let bytes = held_bytes(&r);
    assert!(
        bytes > WALK_HELD_BYTES_MAX,
        "pin O1: below the gate the walk holds all {} bytes",
        bytes
    );
    let done =
        r.m.check_fork_recovery_connection(true)
            .expect("pin O5: below-gate walk completes");
    assert_eq!(done.blocks.len(), 65, "pin O5: every body handed on");
}
