// covers: manager/fork_walk.rs (walk_intake) fork_recovery.rs doli_core attestation/commitment.rs
//! INC-I-293 U-1 (run 651), AUDIT-P2-649-002, operator ruling 4: a walked body
//! whose attestation aggregate is padded is refused at walk intake.
//! Today `walk_intake` checks each walked body with `body_matches_header(ah, None)`:
//! at an unknown height `presence_root_matches` accepts `pre() || post()`, and the
//! pre arm is `bitfield.is_empty() || root == BLAKE3(bitfield)` — it never reads the
//! aggregate. So the body of an honest block nobody attested (empty bitfield, root =
//! `presence_commitment(&[], &[])`) passes intake with any aggregate bytes appended:
//! the padding only inflates the held bytes toward `WALK_HELD_BYTES_MAX`, and an
//! honest walk is cancelled at the bound. The block hash is the header hash, so the
//! padded body is fetched and delivered under the honest hash.
//! (An attested honest body — non-empty bitfield, 96-byte aggregate — is already
//! refused when padded: neither arm matches. The bound hand-over is lane A's.)
//!
// OUTPUT CONTRACT: fn SyncManager::handle_response(peer, SyncResponse::Block(Some(b)))
//   during an active by-hash walk -> walk_intake -> ForkRecoveryTracker::handle_block
//   O1: held set ForkRecoveryTracker.active.blocks (count; no padded aggregate held)
//   O2: walk position (does not move past a refused body): fork_recovery_current_parent()
//       after an accepted hop; after a refused hop the fetch stays pending until its
//       timeout failover and current_parent() is None, so read active.next_parent
//   O3: peer blame take_refused_body_suppliers() -> (walk peer, slot of the refused body)
//   O4: handle_response return: a walk answer is consumed (empty), never sent to apply
//   O5: completion check_fork_recovery_connection(true) -> bodies handed to fork choice
// PATHS: above both gates honest body (empty | attested) -> held | above both gates
//   padded aggregate -> refused + blamed | below the INC-I-288 gate padded -> held (pin)
// INPUT PARTITIONS: bitfield {empty, [0b1]} x aggregate {canonical (0 | 96 B),
//   canonical + PAD bytes} x local_height+1 {> GATE, BLS_AH < . < GATE}
// MATRIX: padded x O1 O2 O3 O4 -> u1c_padded · honest x O1 O2 O3 O4 O5 -> u1c_honest
//   · below-gate padded x O1 O2 O3 O4 O5 -> u1c_below_gate

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader, Transaction};
use libp2p::PeerId;
use vdf::{VdfOutput, VdfProof};

use super::super::{SyncConfig, SyncManager};
use crate::protocols::{SyncRequest, SyncResponse};

/// INC-I-236 block-signature AH: every cell walks above it (headers are signed).
const SIG_AH: u64 = 100;
/// INC-I-178 attestation-BLS AH: every cell walks above it (aggregates committed).
const BLS_AH: u64 = 200;
/// INC-I-288 fork-choice gate (testnet 366_891, mainnet MAX in production).
const GATE: u64 = 10_000;
/// Bytes appended to the canonical aggregate. Far under the walk byte bound, so
/// only the per-hop check can refuse the body.
const PAD: usize = 64 * 1024;
const TS: u64 = 1_700_000_000;

/// Canonical post-INC-I-178 attestation bodies an honest producer builds.
#[derive(Clone, Copy)]
enum Att {
    /// Nobody attested: empty bitfield, empty aggregate.
    Empty,
    /// Bit 0 set, a 96-byte BLS aggregate.
    Attested,
}

struct Rig {
    m: SyncManager,
    peer: PeerId,
    seed: Block,
    /// parents[0] = the seed's parent (first fetch) .. parents[last] = deepest.
    parents: Vec<Block>,
    root: Hash,
}

fn walk_peer() -> PeerId {
    libp2p::identity::Keypair::ed25519_from_bytes([0x2au8; 32])
        .expect("fixed ed25519 seed")
        .public()
        .to_peer_id()
}

fn tx(tag: u32) -> Transaction {
    let mut t = Transaction::new_transfer(vec![], vec![]);
    t.extra_data = tag.to_le_bytes().to_vec();
    t
}

/// An honest signed block above INC-I-178: the presence root commits to the
/// bitfield and the aggregate, exactly as `pooled_commitment` builds it.
fn honest_block(prev: Hash, slot: u32, kp: &KeyPair, att: Att) -> Block {
    let (bitfield, aggregate) = match att {
        Att::Empty => (Vec::new(), Vec::new()),
        Att::Attested => (vec![0b1u8], vec![0xA5u8; crypto::BLS_SIGNATURE_SIZE]),
    };
    let txs = vec![tx(slot)];
    let mut h = BlockHeader {
        version: 1,
        prev_hash: prev,
        merkle_root: doli_core::block::compute_merkle_root(&txs),
        presence_root: doli_core::presence_commitment(&bitfield, &aggregate),
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
    let mut b = Block::new(h, txs);
    b.attestation_bitfield = bitfield;
    b.aggregate_bls_signature = aggregate;
    b
}

/// The honest body with `PAD` bytes appended to its aggregate: same header (same
/// hash), same transactions, same bitfield.
fn padded(honest: &Block) -> Block {
    let mut b = honest.clone();
    b.aggregate_bls_signature
        .extend(std::iter::repeat_n(0x5Au8, PAD));
    b
}

/// Local tip at `local_h`; a signed branch whose bodies follow `shapes` (seed
/// last); the walk started from its seed on the walk peer.
fn rig(local_h: u64, shapes: &[Att]) -> Rig {
    let config = SyncConfig {
        inc_i_236_block_signature_activation_height: SIG_AH,
        inc_i_178_attestation_bls_activation_height: BLS_AH,
        inc_i_288_fork_choice_activation_height: GATE,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, Hash::ZERO);
    let local_hash = Hash::from_bytes([0x11; 32]);
    m.update_local_tip(local_h, local_hash, local_h as u32);
    let peer = walk_peer();
    m.add_peer(peer, local_h, local_hash, local_h as u32);
    let kp = KeyPair::from_seed([0x6e; 32]);
    let root = Hash::from_bytes([0x33; 32]);
    let mut chain = Vec::with_capacity(shapes.len());
    let mut prev = root;
    for (i, att) in shapes.iter().enumerate() {
        let b = honest_block(prev, 2_000 + i as u32, &kp, *att);
        assert!(
            b.body_matches_header(BLS_AH, Some(BLS_AH)),
            "setup: honest body {} matches its header under the post arm",
            i
        );
        prev = b.hash();
        chain.push(b);
    }
    let seed = chain.pop().expect("seed");
    chain.reverse();
    assert!(
        m.start_fork_recovery(seed.clone(), peer),
        "setup: walk starts"
    );
    Rig {
        m,
        peer,
        seed,
        parents: chain,
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

/// Aggregate bytes in held bodies beyond the canonical encoding for their bitfield.
fn held_padding(m: &SyncManager) -> usize {
    held(m)
        .iter()
        .map(|b| {
            let canonical = if b.attestation_bitfield.is_empty() {
                0
            } else {
                crypto::BLS_SIGNATURE_SIZE
            };
            b.aggregate_bls_signature.len().saturating_sub(canonical)
        })
        .sum()
}

/// The walk asks the walk peer for `want`.
fn ask(r: &mut Rig, want: Hash) {
    let (to, req) = r.m.next_request().expect("O2: the walk issues its fetch");
    assert_eq!(to, r.peer, "the walk fetch goes to the walk peer");
    assert!(
        matches!(req, SyncRequest::GetBlockByHash { hash } if hash == want),
        "O2: the walk asks for the next parent"
    );
}

/// O4: the walk peer answers; a walk answer never reaches the apply path.
fn answer(r: &mut Rig, b: Block) {
    let out = r.m.handle_response(r.peer, SyncResponse::Block(Some(b)));
    assert!(
        out.is_empty(),
        "O4: a walk answer is consumed, never passed to apply"
    );
}

/// Walk `n` honest hops (parents[0..n]).
fn honest_hops(r: &mut Rig, n: usize) {
    for k in 0..n {
        let b = r.parents[k].clone();
        ask(r, b.hash());
        answer(r, b);
    }
}

// U1C-PAD INC-I-293 AUDIT-P2-649-002 (Must) — Decision: a failure means a relayer can still pad the aggregate of an honest unattested body so the walk holds bytes the header never committed, and honest walks are cancelled at the byte bound (ruling 4 unmet).
#[test]
fn u1c_padded_aggregate_refused_at_intake() {
    let mut r = rig(
        GATE + 50,
        &[Att::Attested, Att::Empty, Att::Attested, Att::Empty],
    );
    honest_hops(&mut r, 1);
    assert_eq!(held(&r.m).len(), 2, "setup: seed + one honest hop held");
    assert!(
        r.m.take_refused_body_suppliers().is_empty(),
        "setup: no blame on honest hops"
    );
    let honest = r.parents[1].clone();
    let bad = padded(&honest);
    assert_eq!(
        bad.hash(),
        honest.hash(),
        "setup: the aggregate is outside the hash"
    );
    assert!(
        bad.verify_merkle_root(),
        "setup: same transactions, merkle root still matches"
    );
    assert!(
        !bad.body_matches_header(BLS_AH, Some(BLS_AH)),
        "setup: the post arm (known height above INC-I-178) refuses the padded aggregate"
    );
    ask(&mut r, honest.hash());
    answer(&mut r, bad);
    let padding = held_padding(&r.m);
    assert_eq!(
        padding,
        0,
        "intake accepted a body whose aggregate carries {} padding bytes \
         (held {} bodies; the walk must refuse it at the hop, as a body that does not \
         match its header)",
        PAD,
        held(&r.m).len()
    );
    assert_eq!(held(&r.m).len(), 2, "O1: the refused body is not held");
    assert_eq!(
        r.m.fork
            .fork_recovery
            .active
            .as_ref()
            .map(|a| a.next_parent),
        Some(honest.hash()),
        "O2: the walk did not advance past the refused body"
    );
    let blamed = r.m.take_refused_body_suppliers();
    assert!(
        blamed.contains(&(r.peer, honest.header.slot)),
        "O3: the walk peer is blamed for the padded body as for a body mismatch: {:?}",
        blamed
    );
}

// U1C-OK INC-I-293 AUDIT-P2-649-002 (Must, control) — Decision: a failure means the padding check also refuses canonical aggregates (empty or 96-byte attested bodies), so honest walks above both gates stop healing onto the heavier branch.
#[test]
fn u1c_honest_aggregate_accepted() {
    let shapes = [
        Att::Empty,
        Att::Attested,
        Att::Attested,
        Att::Empty,
        Att::Attested,
    ];
    let mut r = rig(GATE + 50, &shapes);
    let hops = r.parents.len();
    for k in 0..hops {
        let b = r.parents[k].clone();
        ask(&mut r, b.hash());
        answer(&mut r, b);
        assert_eq!(held(&r.m).len(), k + 2, "O1: honest hop {} held", k + 1);
        assert!(
            r.m.take_refused_body_suppliers().is_empty(),
            "O3: honest hop {} not blamed",
            k + 1
        );
    }
    let done =
        r.m.check_fork_recovery_connection(true)
            .expect("O5: honest walk completes");
    assert_eq!(done.connection_point, r.root, "O5: connection point");
    let mut want: Vec<Block> = r.parents.iter().rev().cloned().collect();
    want.push(r.seed.clone());
    assert_eq!(
        done.blocks, want,
        "O5: every honest body handed on intact, aggregates included, seed last"
    );
}

// U1C-PIN INC-I-293 AUDIT-P2-649-002 (Must, below-gate pin) — Decision: a failure means the intake refusal reaches below the INC-I-288 gate, so mainnet walk intake changes before the pin; gate it on local_height + 1 >= the INC-I-288 height.
#[test]
fn u1c_below_gate_intake_unchanged() {
    let mut r = rig(
        GATE - 50,
        &[Att::Attested, Att::Empty, Att::Attested, Att::Empty],
    );
    honest_hops(&mut r, 1);
    let honest = r.parents[1].clone();
    let bad = padded(&honest);
    ask(&mut r, honest.hash());
    answer(&mut r, bad.clone());
    assert_eq!(
        held(&r.m).len(),
        3,
        "pin O1: below the gate the padded body is held"
    );
    assert_eq!(
        held_padding(&r.m),
        PAD,
        "pin O1: its padding is held as delivered"
    );
    assert_eq!(
        r.m.fork_recovery_current_parent(),
        Some(honest.header.prev_hash),
        "pin O2: below the gate the walk advances past it"
    );
    assert!(
        r.m.take_refused_body_suppliers().is_empty(),
        "pin O3: below the gate nobody is blamed"
    );
    let rest = r.parents.len();
    for k in 2..rest {
        let b = r.parents[k].clone();
        ask(&mut r, b.hash());
        answer(&mut r, b);
    }
    let done =
        r.m.check_fork_recovery_connection(true)
            .expect("pin O5: below-gate walk completes");
    assert_eq!(done.blocks.len(), rest + 1, "pin O5: every body handed on");
    assert!(
        done.blocks.contains(&bad),
        "pin O5: the padded body reaches fork choice below the gate"
    );
}
