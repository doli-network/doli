// covers: bodies.rs sync_engine/response.rs fork_recovery.rs fork_walk.rs block_lifecycle.rs types.rs
//! INC-I-236 Run A M2: sync Bodies and the fork-walk Block intake verify the block signature at h >= AH.
// REQ-AUTH-016 — Decision: a failure means a forged, unsigned or unrequested synced block above the AH reaches pending_blocks or the fork walk.
//
// OUTPUT CONTRACT (SyncManager::handle_response(Bodies | Block)):
//   O1 pipeline.pending_blocks: hash present / absent
//   O2 pipeline.body_downloader: has_block(hash), total_downloaded()
//   O3 pipeline.headers_needing_bodies: hash outstanding; next_request re-asks for it
//   O4 return Vec<Block> of handle_response
//   O5 get_blocks_to_apply(): ordered blocks released for apply
//   O6 fork walk: check_fork_recovery_connection(true) -> blocks + connection_point
// PATHS: Bodies {requested hash, hash outside the active request, no active request}
//   | Block(Some) during an active walk {consumed, refused}
// INPUT PARTITIONS: pi {producer, wrong key, empty, 64 garbage bytes}
//   x height {< AH, == AH, > AH} x request {outstanding, not requested, none}
// MATRIX: bodies forged/unsigned/garbage at or above AH -> O1,O2,O3,O4,O5; genuine resend -> O1,O3,O4,O5;
//   signed / below-AH controls -> O1,O2,O3,O4,O5; unrequested / no request -> O1,O2,O3,O4;
//   walk forged/unsigned/boundary -> O6; walk signed / below-AH -> O4,O6.
// HEIGHT: a walk block has no known height; the walk gates on local_height + 1 (as node header_auth).

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use libp2p::PeerId;
use vdf::{VdfOutput, VdfProof};

use super::{SyncConfig, SyncManager, SyncPipelineData};
use crate::protocols::{SyncRequest, SyncResponse};

const AH: u64 = 3;
const WALK_AH: u64 = 100;
const PEER_H: u64 = 1_000;
const GENESIS: Hash = Hash::ZERO;

#[derive(Clone, Copy)]
enum Sig {
    None,
    Producer,
    WrongKey,
    Garbage,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn pi(h: &BlockHeader, kp: &KeyPair, s: Sig) -> Vec<u8> {
    let sign = |k: &KeyPair| {
        crypto::signature::sign(h.signing_message().as_bytes(), k.private_key()).to_vec()
    };
    match s {
        Sig::None => Vec::new(),
        Sig::Producer => sign(kp),
        Sig::WrongKey => sign(&KeyPair::generate()),
        Sig::Garbage => (0..64u8).map(|i| i.wrapping_mul(37) ^ 0x5a).collect(),
    }
}

/// Linked headers from `prev`; header i has slot `first_slot + i`, producer `kp`.
fn link(prev: Hash, first_slot: u32, kp: &KeyPair, sigs: &[Sig]) -> Vec<BlockHeader> {
    let mut out = Vec::new();
    let mut prev_hash = prev;
    for (i, s) in sigs.iter().enumerate() {
        let mut h = BlockHeader {
            version: 1,
            prev_hash,
            merkle_root: Hash::ZERO,
            presence_root: Hash::ZERO,
            genesis_hash: Hash::ZERO,
            timestamp: now(),
            slot: first_slot + i as u32,
            producer: *kp.public_key(),
            vdf_output: VdfOutput { value: vec![0; 32] },
            vdf_proof: VdfProof::empty(),
            missed_producers: Vec::new(),
            data_root: Hash::ZERO,
            fork_id: Hash::ZERO,
        };
        h.vdf_proof.pi = pi(&h, kp, *s);
        prev_hash = h.hash();
        out.push(h);
    }
    out
}

/// Same block hash, different pi.
fn variant(b: &Block, kp: &KeyPair, s: Sig) -> Block {
    let mut v = b.clone();
    v.header.vdf_proof.pi = pi(&b.header, kp, s);
    assert_eq!(v.hash(), b.hash(), "setup: pi is outside the block hash");
    v
}

fn hashes(bs: &[Block]) -> Vec<Hash> {
    bs.iter().map(|b| b.hash()).collect()
}

struct BodyStage {
    m: SyncManager,
    peer: PeerId,
    others: Vec<PeerId>,
    kp: KeyPair,
    blocks: Vec<Block>,
}

/// h=0 header-first sync: headers 1..=n accepted, one GetBodies outstanding to `peer`.
fn body_stage(sigs: &[Sig]) -> BodyStage {
    let config = SyncConfig {
        inc_i_236_block_signature_activation_height: AH,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, GENESIS);
    let peers: Vec<PeerId> = (0..3).map(|_| PeerId::random()).collect();
    for (i, p) in peers.iter().enumerate() {
        m.add_peer(
            *p,
            PEER_H,
            Hash::from_bytes([0xA0 + i as u8; 32]),
            PEER_H as u32,
        );
    }
    assert!(m.state().is_syncing(), "setup: sync started");
    let kp = KeyPair::generate();
    let headers = link(GENESIS, 1, &kp, sigs);
    let _ = m.handle_response(peers[0], SyncResponse::Headers(headers.clone()));
    assert_eq!(
        m.pipeline.pending_headers.len(),
        headers.len(),
        "setup: header stage accepts the chain"
    );
    let _ = m.handle_response(peers[0], SyncResponse::Headers(vec![]));
    let (peer, req) = m.next_request().expect("setup: body request");
    let SyncRequest::GetBodies { hashes: asked } = req else {
        panic!("setup: expected GetBodies, got {req:?}");
    };
    assert_eq!(asked, headers.iter().map(|h| h.hash()).collect::<Vec<_>>());
    let blocks = headers.into_iter().map(|h| Block::new(h, vec![])).collect();
    let others = peers.into_iter().filter(|p| *p != peer).collect();
    BodyStage {
        m,
        peer,
        others,
        kp,
        blocks,
    }
}

/// Respond with every requested body, body `idx` replaced by its `s` pi-variant.
fn respond_with_variant(s: &mut BodyStage, idx: usize, sig: Sig) -> Vec<Block> {
    let mut resp = s.blocks.clone();
    resp[idx] = variant(&s.blocks[idx], &s.kp, sig);
    s.m.handle_response(s.peer, SyncResponse::Bodies(resp))
}

fn assert_body_refused(s: &mut BodyStage, idx: usize, out: &[Block]) {
    let h = s.blocks[idx].hash();
    assert!(
        !s.m.pipeline.pending_blocks.contains_key(&h),
        "O1: bad body at height {} stored in pending_blocks",
        idx + 1
    );
    assert!(out.iter().all(|b| b.hash() != h), "O4: bad body handed on");
    assert!(
        !s.m.pipeline.body_downloader.has_block(&h),
        "O2: bad body kept as downloaded"
    );
    assert_eq!(
        s.m.pipeline.body_downloader.total_downloaded(),
        s.blocks.len() - 1,
        "O2: bad body counted downloaded"
    );
    assert!(
        s.m.pipeline.headers_needing_bodies.contains(&h),
        "O3: a bad body closed the request"
    );
    for (i, b) in s.blocks.iter().enumerate() {
        if i != idx {
            assert!(s.m.pipeline.pending_blocks.contains_key(&b.hash()));
        }
    }
    let applied = s.m.get_blocks_to_apply();
    assert_eq!(hashes(&applied), hashes(&s.blocks[..idx]), "O5");
}

fn assert_all_accepted(s: &mut BodyStage, out: &[Block]) {
    assert_eq!(hashes(out), hashes(&s.blocks), "O4");
    for b in &s.blocks {
        assert!(s.m.pipeline.pending_blocks.contains_key(&b.hash()), "O1");
        assert!(s.m.pipeline.body_downloader.has_block(&b.hash()), "O2");
    }
    assert_eq!(
        s.m.pipeline.body_downloader.total_downloaded(),
        s.blocks.len()
    );
    assert!(s.m.pipeline.headers_needing_bodies.is_empty(), "O3");
    assert!(matches!(
        s.m.pipeline_data,
        SyncPipelineData::Processing { .. }
    ));
    let applied = s.m.get_blocks_to_apply();
    assert_eq!(hashes(&applied), hashes(&s.blocks), "O5");
}

const ABOVE: [Sig; 4] = [Sig::None, Sig::None, Sig::Producer, Sig::Producer];

// REQ-AUTH-016 (Must) — Decision: a failure means a forged body above the AH is stored for apply.
#[test]
fn req_auth_016_p4_forged_body_not_pending() {
    let mut s = body_stage(&ABOVE);
    let out = respond_with_variant(&mut s, 3, Sig::WrongKey);
    assert_body_refused(&mut s, 3, &out);
}

// REQ-AUTH-016 (Must) — Decision: a failure means an unsigned body at the AH is stored for apply.
#[test]
fn req_auth_016_p4_unsigned_body_at_ah_not_pending() {
    let mut s = body_stage(&ABOVE);
    let out = respond_with_variant(&mut s, 2, Sig::None);
    assert_body_refused(&mut s, 2, &out);
}

// REQ-AUTH-016 (Must) — Decision: a failure means a 64-byte garbage pi above the AH is stored for apply.
#[test]
fn req_auth_016_p4_garbage_pi_body_above_ah_not_pending() {
    let mut s = body_stage(&ABOVE);
    let out = respond_with_variant(&mut s, 3, Sig::Garbage);
    assert_body_refused(&mut s, 3, &out);
}

// REQ-AUTH-016 (Must) — Decision: a failure means one forged response blocks or replaces the genuine body.
#[test]
fn req_auth_016_p4_genuine_resend_after_forged_accepted() {
    let mut s = body_stage(&ABOVE);
    let _ = respond_with_variant(&mut s, 2, Sig::WrongKey);
    let (p2, req) =
        s.m.next_request()
            .expect("O3: the refused hash must be re-requested");
    let SyncRequest::GetBodies { hashes: again } = req else {
        panic!("O3: expected GetBodies, got {req:?}");
    };
    assert_eq!(again, vec![s.blocks[2].hash()], "O3");
    let out =
        s.m.handle_response(p2, SyncResponse::Bodies(vec![s.blocks[2].clone()]));
    assert_eq!(hashes(&out), vec![s.blocks[2].hash()], "O4");
    assert!(s.m.pipeline.headers_needing_bodies.is_empty(), "O3");
    let applied = s.m.get_blocks_to_apply();
    assert_eq!(hashes(&applied), hashes(&s.blocks), "O5");
    assert_eq!(
        applied[2].header.vdf_proof.pi, s.blocks[2].header.vdf_proof.pi,
        "O1: the genuine signature is the one applied"
    );
}

// REQ-AUTH-016 (Must) — Decision: a failure means the fix rejects correctly signed bodies above the AH.
#[test]
fn req_auth_016_p4_signed_bodies_above_ah_accepted() {
    let mut s = body_stage(&ABOVE);
    let out =
        s.m.handle_response(s.peer, SyncResponse::Bodies(s.blocks.clone()));
    assert_all_accepted(&mut s, &out);
}

// REQ-AUTH-016 (Must) — Decision: a failure means the fix rejects legal unsigned bodies below the AH.
#[test]
fn req_auth_016_p4_unsigned_bodies_below_ah_accepted() {
    let mut s = body_stage(&[Sig::None, Sig::None]);
    let out =
        s.m.handle_response(s.peer, SyncResponse::Bodies(s.blocks.clone()));
    assert_all_accepted(&mut s, &out);
}

// REQ-AUTH-016 (Must) — Decision: a failure means a peer can inject a body for a hash it was never asked for.
#[test]
fn req_auth_016_p4_unrequested_body_not_pending() {
    let mut s = body_stage(&ABOVE);
    let tip = s.blocks[3].hash();
    let stray = Block::new(link(tip, 9, &s.kp, &[Sig::Producer])[0].clone(), vec![]);
    let mut resp = s.blocks.clone();
    resp.push(stray.clone());
    let out = s.m.handle_response(s.peer, SyncResponse::Bodies(resp));
    let h = stray.hash();
    assert!(!s.m.pipeline.pending_blocks.contains_key(&h), "O1");
    assert!(!s.m.pipeline.body_downloader.has_block(&h), "O2");
    assert_eq!(s.m.pipeline.body_downloader.total_downloaded(), 4, "O2");
    assert!(out.iter().all(|b| b.hash() != h), "O4");
    assert!(s.m.pipeline.headers_needing_bodies.is_empty(), "O3");
    for b in &s.blocks {
        assert!(s.m.pipeline.pending_blocks.contains_key(&b.hash()), "O1");
    }
}

// REQ-AUTH-016 (Must) — Decision: a failure means a peer with no outstanding request can inject bodies.
#[test]
fn req_auth_016_p4_body_without_active_request_not_pending() {
    let mut s = body_stage(&ABOVE);
    let other = s.others[0];
    let out =
        s.m.handle_response(other, SyncResponse::Bodies(s.blocks.clone()));
    assert!(out.is_empty(), "O4: unrequested bodies handed on");
    for b in &s.blocks {
        assert!(!s.m.pipeline.pending_blocks.contains_key(&b.hash()), "O1");
    }
    assert_eq!(s.m.pipeline.body_downloader.total_downloaded(), 0, "O2");
    assert_eq!(s.m.pipeline.headers_needing_bodies.len(), 4, "O3");
}

struct Walk {
    m: SyncManager,
    peer: PeerId,
    parent: Block,
    orphan: Block,
}

/// Local tip at `local_h`; a walk started from a signed orphan; parent fetch outstanding.
fn walk(local_h: u64, parent_sig: Sig) -> Walk {
    let config = SyncConfig {
        inc_i_236_block_signature_activation_height: WALK_AH,
        ..SyncConfig::default()
    };
    let mut m = SyncManager::new(config, GENESIS);
    m.update_local_tip(local_h, Hash::from_bytes([0x11; 32]), local_h as u32);
    let peer = PeerId::random();
    let kp = KeyPair::generate();
    let ph = link(Hash::from_bytes([0x22; 32]), 500, &kp, &[parent_sig]);
    let oh = link(ph[0].hash(), 501, &kp, &[Sig::Producer]);
    let parent = Block::new(ph[0].clone(), vec![]);
    let orphan = Block::new(oh[0].clone(), vec![]);
    assert!(
        m.start_fork_recovery(orphan.clone(), peer),
        "setup: walk starts"
    );
    let (to, req) = m.next_request().expect("setup: walk fetch");
    assert_eq!(to, peer);
    assert!(matches!(req, SyncRequest::GetBlockByHash { hash } if hash == parent.hash()));
    Walk {
        m,
        peer,
        parent,
        orphan,
    }
}

fn assert_walk_refused(w: &mut Walk) {
    let _ =
        w.m.handle_response(w.peer, SyncResponse::Block(Some(w.parent.clone())));
    let done = w.m.check_fork_recovery_connection(true);
    let walked = done.as_ref().map(|c| hashes(&c.blocks)).unwrap_or_default();
    assert!(
        !walked.contains(&w.parent.hash()),
        "O6: refused parent entered the fork walk"
    );
    if let Some(c) = done {
        assert_eq!(c.connection_point, w.parent.hash(), "O6: walk advanced");
        assert_eq!(hashes(&c.blocks), vec![w.orphan.hash()], "O6");
    }
}

fn assert_walk_advanced(w: &mut Walk) {
    let out =
        w.m.handle_response(w.peer, SyncResponse::Block(Some(w.parent.clone())));
    assert!(out.is_empty(), "O4: walk parent must be consumed");
    let done =
        w.m.check_fork_recovery_connection(true)
            .expect("O6: walk active");
    assert_eq!(
        hashes(&done.blocks),
        vec![w.parent.hash(), w.orphan.hash()],
        "O6"
    );
    assert_eq!(done.connection_point, w.parent.header.prev_hash, "O6");
}

// REQ-AUTH-016 (Must) — Decision: a failure means a forged parent above the AH enters the fork walk.
#[test]
fn req_auth_016_p8_fork_walk_forged_parent_not_cached() {
    let mut w = walk(WALK_AH + 50, Sig::WrongKey);
    assert_walk_refused(&mut w);
}

// REQ-AUTH-016 (Must) — Decision: a failure means an unsigned parent above the AH enters the fork walk.
#[test]
fn req_auth_016_p8_fork_walk_unsigned_parent_not_cached() {
    let mut w = walk(WALK_AH + 50, Sig::None);
    assert_walk_refused(&mut w);
}

// REQ-AUTH-016 (Must) — Decision: a failure means the walk gate is off at prospective height == AH.
#[test]
fn req_auth_016_p8_fork_walk_boundary_garbage_parent_not_cached() {
    let mut w = walk(WALK_AH - 1, Sig::Garbage);
    assert_walk_refused(&mut w);
}

// REQ-AUTH-016 (Must) — Decision: a failure means the fix stops the walk on a correctly signed parent.
#[test]
fn req_auth_016_p8_fork_walk_signed_parent_advances() {
    let mut w = walk(WALK_AH + 50, Sig::Producer);
    assert_walk_advanced(&mut w);
}

// REQ-AUTH-016 (Must) — Decision: a failure means the fix stops a legal pre-AH walk on an unsigned parent.
#[test]
fn req_auth_016_p8_fork_walk_unsigned_parent_below_ah_advances() {
    let mut w = walk(WALK_AH - 2, Sig::None);
    assert_walk_advanced(&mut w);
}
