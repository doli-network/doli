// covers: headers.rs
//! INC-I-236 Run A: the header-stage signature check after a sync restart.
//!
//! Production path driven here (public SyncManager API only):
//!   1. h=0, three peers that disagree on the tip -> start_sync takes the
//!      "no majority" snap fallback, which calls `header_downloader.clear()`.
//!   2. Header batch accepted -> `pending_headers` filled.
//!   3. Restart -> preserve_valid_sync_data -> `resume_from(last)` -> the
//!      no-majority fallback clears the downloader again (pending kept).
//!   4. Block 1 applied (h=1, so the next restart does not take snap).
//!   5. Restart -> preserve_valid_sync_data -> `resume_from(last)` ->
//!      header-first. The continuation height must match `last + 1`.
//
// OUTPUT CONTRACT (restart path + handle_response(Headers) on the continuation):
//   O1 pipeline.pending_headers: len delta (= headers accepted) and back() hash
//   O2 pipeline.headers_needing_bodies: len delta (one per accepted header)
//   O3 header_downloader.expected_prev_hash(): continuation point
//   O4 return of handle_response(Headers): always empty (headers carry no blocks)
// PATHS: P1 restart via preserve + earlier downloader clear (stale height)
//   | P2 downloader re-created by a reset path, then restart (no preserve)
// MATRIX: P1 x {unsigned AH-1, unsigned AH, wrong-key AH, signed AH} -> O1,O2,O3,O4;
//   P2 x {unsigned AH-1 then unsigned AH} -> O1,O2,O3.
//
// INPUT PARTITIONS:
//   true height of continuation header {AH-1, AH}
//   signature {none, wrong key, producer key}
//   downloader history {cleared before resume (P1), re-created (P2)}
//   batch shape {single header, two headers crossing the AH}

use crypto::{Hash, KeyPair};
use doli_core::BlockHeader;
use libp2p::PeerId;
use vdf::{VdfOutput, VdfProof};

use super::{SyncConfig, SyncManager, SyncState};
use crate::protocols::SyncResponse;

const AH: u64 = 10;
const PEER_H: u64 = 1_000;
const GENESIS: Hash = Hash::ZERO;

#[derive(Clone, Copy)]
enum Sig {
    None,
    Producer,
    WrongKey,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Linked headers from `prev`; header i has slot `first_slot + i`.
fn link(prev: Hash, first_slot: u32, kp: &KeyPair, sigs: &[Sig]) -> Vec<BlockHeader> {
    let attacker = KeyPair::generate();
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
        let signer = match s {
            Sig::None => None,
            Sig::Producer => Some(kp),
            Sig::WrongKey => Some(&attacker),
        };
        if let Some(k) = signer {
            let msg = h.signing_message();
            h.vdf_proof.pi = crypto::signature::sign(msg.as_bytes(), k.private_key()).to_vec();
        }
        prev_hash = h.hash();
        out.push(h);
    }
    out
}

/// Sync restart as production does it: back to Idle, then a peer status update.
fn restart(m: &mut SyncManager, peer: PeerId) {
    m.set_state(SyncState::Idle, "test_restart");
    m.update_peer(peer, PEER_H, Hash::from_bytes([0xA0; 32]), PEER_H as u32);
    assert!(m.state().is_syncing(), "restart must start a new sync");
}

/// h=0 manager with three peers that disagree on the tip (no snap majority).
fn split_peer_manager() -> (SyncManager, PeerId) {
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
    assert!(m.state().is_syncing(), "setup: 3rd peer must start a sync");
    (m, peers[0])
}

/// Steps 1-5 of the module doc with `n` unsigned headers at heights 1..=n (n < AH).
fn restarted_with_preserved(n: u64) -> (SyncManager, PeerId, KeyPair, Vec<BlockHeader>) {
    assert!(n < AH);
    let (mut m, p) = split_peer_manager();
    let kp = KeyPair::generate();
    let chain = link(GENESIS, 1, &kp, &vec![Sig::None; n as usize]);
    let _ = m.handle_response(p, SyncResponse::Headers(chain.clone()));
    assert_eq!(
        m.pipeline.pending_headers.len(),
        n as usize,
        "setup: batch 1"
    );

    restart(&mut m, p);
    assert_eq!(
        m.pipeline.pending_headers.len(),
        n as usize,
        "setup: preserved"
    );

    m.block_applied(chain[0].hash(), 1, chain[0].slot);
    restart(&mut m, p);

    let last = chain.last().unwrap().hash();
    assert_eq!(m.pipeline.pending_headers.len(), n as usize - 1);
    assert_eq!(
        m.pipeline.header_downloader.expected_prev_hash(),
        Some(last),
        "setup: downloader must resume from the last preserved header"
    );
    (m, p, kp, chain)
}

/// Feed one continuation batch; returns (accepted, bodies queued).
fn feed(m: &mut SyncManager, p: PeerId, batch: &[BlockHeader]) -> (usize, usize) {
    let h0 = m.pipeline.pending_headers.len();
    let b0 = m.pipeline.headers_needing_bodies.len();
    let blocks = m.handle_response(p, SyncResponse::Headers(batch.to_vec()));
    assert!(blocks.is_empty(), "a headers response returns no blocks");
    (
        m.pipeline.pending_headers.len() - h0,
        m.pipeline.headers_needing_bodies.len() - b0,
    )
}

// REQ-AUTH-008 — Decision: a failure means a restart drops a legal pre-AH unsigned header.
// Acceptance: below the AH nothing changes (unsigned header at true height AH-1 accepted).
#[test]
fn inc_i_236_run_a_resume_unsigned_at_ah_minus_one_accepted_after_restart() {
    let (mut m, p, kp, chain) = restarted_with_preserved(AH - 2);
    let next = link(
        chain.last().unwrap().hash(),
        AH as u32 - 1,
        &kp,
        &[Sig::None],
    );
    assert_eq!(feed(&mut m, p, &next), (1, 1), "true height AH-1 is pre-AH");
    assert_eq!(
        m.pipeline.pending_headers.back().unwrap().hash(),
        next[0].hash()
    );
    assert_eq!(
        m.pipeline.header_downloader.expected_prev_hash(),
        Some(next[0].hash())
    );
}

// REQ-AUTH-008 — Decision: a failure means an unsigned header at the AH passes the header stage after a restart.
// Acceptance: batch [unsigned AH-1, unsigned AH] stops at the AH header.
#[test]
fn inc_i_236_run_a_resume_unsigned_at_ah_rejected_after_restart() {
    let (mut m, p, kp, chain) = restarted_with_preserved(AH - 2);
    let next = link(
        chain.last().unwrap().hash(),
        AH as u32 - 1,
        &kp,
        &[Sig::None, Sig::None],
    );
    assert_eq!(
        feed(&mut m, p, &next),
        (1, 1),
        "unsigned header at true height AH accepted after a restart: \
         the resumed downloader used a stale continuation height"
    );
    assert_eq!(
        m.pipeline.pending_headers.back().unwrap().hash(),
        next[0].hash()
    );
    assert_eq!(
        m.pipeline.header_downloader.expected_prev_hash(),
        Some(next[0].hash())
    );
}

// REQ-AUTH-008 — Decision: a failure means a wrong-key header at the AH passes the header stage after a restart.
// Acceptance: a header at true height AH signed by a key other than its producer is rejected.
#[test]
fn inc_i_236_run_a_resume_wrong_key_at_ah_rejected_after_restart() {
    let (mut m, p, kp, chain) = restarted_with_preserved(AH - 1);
    let last = chain.last().unwrap().hash();
    let forged = link(last, AH as u32, &kp, &[Sig::WrongKey]);
    assert_eq!(
        feed(&mut m, p, &forged),
        (0, 0),
        "wrong-key header at true height AH accepted after a restart: \
         the resumed downloader used a stale continuation height"
    );
    assert_eq!(m.pipeline.pending_headers.back().unwrap().hash(), last);
    assert_eq!(
        m.pipeline.header_downloader.expected_prev_hash(),
        Some(last)
    );
}

// REQ-AUTH-008 — Decision: a failure means the fix rejects correctly signed headers at the AH after a restart.
// Acceptance: control for the two tests above; same position, producer signature, accepted.
#[test]
fn inc_i_236_run_a_resume_signed_at_ah_accepted_after_restart() {
    let (mut m, p, kp, chain) = restarted_with_preserved(AH - 1);
    let signed = link(
        chain.last().unwrap().hash(),
        AH as u32,
        &kp,
        &[Sig::Producer],
    );
    assert_eq!(feed(&mut m, p, &signed), (1, 1));
    assert_eq!(
        m.pipeline.header_downloader.expected_prev_hash(),
        Some(signed[0].hash())
    );
}

// REQ-AUTH-008 — Decision: a failure means a re-created downloader loses the AH or the height.
// Acceptance: after a reset path re-creates the downloader, AH-1 unsigned passes and AH unsigned stops.
#[test]
fn inc_i_236_run_a_recreated_downloader_enforces_ah() {
    let (mut m, p) = split_peer_manager();
    let kp = KeyPair::generate();
    let chain = link(GENESIS, 1, &kp, &vec![Sig::None; AH as usize - 2]);
    let _ = m.handle_response(p, SyncResponse::Headers(chain.clone()));
    let last = chain.last().unwrap();
    m.block_applied(last.hash(), AH - 2, last.slot);

    // Reset path: pending headers dropped, downloader re-created from SyncConfig.
    m.reset_sync_after_successful_reorg();
    assert!(m.pipeline.pending_headers.is_empty());
    assert_eq!(m.pipeline.header_downloader.expected_prev_hash(), None);
    restart(&mut m, p);

    let next = link(last.hash(), AH as u32 - 1, &kp, &[Sig::None, Sig::None]);
    assert_eq!(feed(&mut m, p, &next), (1, 1), "AH-1 accepted, AH rejected");
    assert_eq!(
        m.pipeline.pending_headers.back().unwrap().hash(),
        next[0].hash()
    );
    assert_eq!(
        m.pipeline.header_downloader.expected_prev_hash(),
        Some(next[0].hash())
    );
}
