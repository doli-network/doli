// covers: manager/sync_engine/response.rs (handle_headers_response) manager/recovery.rs (classify Rules 1/1b)
//! INC-I-294 M-2: a header proof is minted only by an authenticated first header that
//! answers a tracked GetHeadersByHeight(tip) request, and counts only for the tip hash
//! it was recorded against.
//
// OUTPUT CONTRACT: SyncManager::handle_response(Headers) -> O2 coordinator evidence
//   (HeaderDivergence), O2 header_downloader (accepted headers). Observed through
//   SyncManager::classify_and_dispatch(0).0 -> O3 RecoveryAction, with two fresh orphan
//   reports, tip below the mainnet fork-choice gate.
// PARTITIONS
//   request: Rh tracked GetHeadersByHeight(tip) | Rx tracked by height, tip moved after send
//            | Rr tracked, cleared by a rollback | Rk tracked GetHeaders(tip hash)
//   header:  Hv valid signed, prev != tip | Hs bad signature | H0 version 0 | He prev == tip
//   tip:     T= unchanged | Tr same height, other hash | Td rolled back one | Ta advanced one
// MATRIX
//   Rh Hv T=  -> ShallowRollback{1} (true depth-1 fork heals)            [control]
//   Rh He T=  -> HeaderFirstSync, header accepted                         [control]
//   Rh Hs|H0 T= -> HeaderFirstSync (no proof)
//   Rh Hv Tr  -> HeaderFirstSync (proof is for the old tip hash)
//   Rr Hv Td  -> HeaderFirstSync (untracked reply mints nothing)
//   Rx Hv Ta  -> HeaderFirstSync (first header is not at tip+1)
//   Rk Hv T=  -> HeaderFirstSync (hash-anchored reply fixes no height)

use crypto::{Hash, KeyPair};
use doli_core::BlockHeader;
use libp2p::PeerId;

use super::recovery::RecoveryAction;
use super::tests_inc_i294_header_proof::{
    mgr_awaiting_headers, send_tracked_request, signed_header, BLIND, HFS, TIP_BELOW,
};
use crate::protocols::SyncResponse;
use crate::sync::manager::SyncManager;

/// Key `signed_header` uses as producer and signer.
const PRODUCER_SEED: [u8; 32] = [0x6e; 32];
const OTHER_SEED: [u8; 32] = [0x5a; 32];

fn other_parent() -> Hash {
    crypto::hash::hash(b"proof-auth other parent")
}

fn resign(mut h: BlockHeader, seed: [u8; 32]) -> BlockHeader {
    let kp = KeyPair::from_seed(seed);
    h.vdf_proof.pi =
        crypto::signature::sign(h.signing_message().as_bytes(), kp.private_key()).to_vec();
    h
}

/// Tip `local`, one peer at `local + gap`, real tracked header request already sent.
fn tracked(local: u64, gap: u64, by_height: bool) -> (SyncManager, PeerId) {
    let (mut m, peer) = mgr_awaiting_headers(local, gap);
    send_tracked_request(&mut m, peer, by_height);
    (m, peer)
}

fn classify_with_orphans(m: &mut SyncManager) -> RecoveryAction {
    m.report_orphan_gossip(11, 1);
    m.report_orphan_gossip(11, 1);
    m.classify_and_dispatch(0).0
}

// INC-I-294 M-2 (Must) — Decision: a failure means a first header that fails validate_header or the block signature check still mints a rollback proof.
#[test]
fn proof_auth_invalid_first_header_mints_no_proof() {
    let slot = TIP_BELOW as u32 + 1;
    let bad_sig = resign(signed_header(other_parent(), slot), OTHER_SEED);
    assert!(
        doli_core::validation::verify_block_signature(&bad_sig).is_err(),
        "setup: signature must be invalid"
    );
    let mut v0 = signed_header(other_parent(), slot);
    v0.version = 0;
    let v0 = resign(v0, PRODUCER_SEED);

    for (case, h) in [("bad signature", bad_sig), ("version 0", v0)] {
        let (mut m, peer) = tracked(TIP_BELOW, 1, true);
        m.handle_response(peer, SyncResponse::Headers(vec![h]));
        let got = classify_with_orphans(&mut m);
        assert_eq!(got, HFS, "{case}: an invalid first header is no proof");
    }
}

// INC-I-294 M-2 (Must) — Decision: a failure means a proof recorded against one tip hash still rolls back a different tip at the same height.
#[test]
fn proof_auth_proof_for_another_tip_hash_does_not_count() {
    let (mut m, peer) = tracked(TIP_BELOW, 1, true);
    let sibling = signed_header(other_parent(), TIP_BELOW as u32 + 1);
    m.handle_response(peer, SyncResponse::Headers(vec![sibling]));
    let replaced = crypto::hash::hash(b"proof-auth replaced tip");
    m.update_local_tip(TIP_BELOW, replaced, TIP_BELOW as u32);
    let got = classify_with_orphans(&mut m);
    assert_eq!(
        got, HFS,
        "proof for the old tip hash is no proof for the new one"
    );
}

// INC-I-294 M-2 (Must) — Decision: a failure means a reply whose tracking a rollback cleared still mints a proof against the rolled-back tip.
#[test]
fn proof_auth_stale_reply_after_rollback_mints_no_proof() {
    let (mut m, peer) = tracked(TIP_BELOW, 1, true);
    let old_tip = m.local_hash;
    let parent = crypto::hash::hash(b"proof-auth parent");
    m.update_local_tip(TIP_BELOW - 1, parent, (TIP_BELOW - 1) as u32);
    m.reset_sync_for_rollback();
    let late = signed_header(old_tip, TIP_BELOW as u32 + 1);
    m.handle_response(peer, SyncResponse::Headers(vec![late]));
    let got = classify_with_orphans(&mut m);
    assert_eq!(got, HFS, "an untracked late reply is no proof");
}

// INC-I-294 M-2 (Must) — Decision: a failure means a reply to a height request sent before our tip advanced is read as a header at tip+1 and mints a proof.
#[test]
fn proof_auth_first_header_not_at_tip_plus_one_mints_no_proof() {
    let (mut m, peer) = tracked(TIP_BELOW - 1, 2, true);
    let old_tip = m.local_hash;
    let new_tip = crypto::hash::hash(b"proof-auth new tip");
    m.update_local_tip(TIP_BELOW, new_tip, TIP_BELOW as u32);
    let at_our_height = signed_header(old_tip, TIP_BELOW as u32);
    assert_ne!(at_our_height.hash(), new_tip, "setup: a sibling of our tip");
    m.handle_response(peer, SyncResponse::Headers(vec![at_our_height]));
    let got = classify_with_orphans(&mut m);
    assert_eq!(
        got, HFS,
        "reply to an older height request: header is at our tip height"
    );
}

// INC-I-294 M-2 (Must) — Decision: a failure means a reply to a hash-anchored request, which fixes no header height, mints a proof.
#[test]
fn proof_auth_hash_anchored_reply_mints_no_proof() {
    let (mut m, peer) = tracked(TIP_BELOW, 1, false);
    let sibling = signed_header(other_parent(), TIP_BELOW as u32 + 1);
    m.handle_response(peer, SyncResponse::Headers(vec![sibling]));
    let got = classify_with_orphans(&mut m);
    assert_eq!(
        got, HFS,
        "hash-anchored reply: header height is not established"
    );
}

// INC-I-294 M-2 (Must) — Decision: a failure means the authentication checks also block the valid sibling proof a true depth-1 fork needs.
#[test]
fn proof_auth_control_valid_sibling_at_tip_plus_one_still_heals() {
    let (mut m, peer) = tracked(TIP_BELOW, 1, true);
    let sibling = signed_header(other_parent(), TIP_BELOW as u32 + 1);
    m.handle_response(peer, SyncResponse::Headers(vec![sibling]));
    let got = classify_with_orphans(&mut m);
    assert_eq!(
        got, BLIND,
        "valid sibling at tip+1 for a tracked request is proof"
    );
}

// INC-I-294 M-2 (Must) — Decision: a failure means a header that extends our tip is no longer accepted or is read as a fork.
#[test]
fn proof_auth_control_extending_header_is_accepted_not_proof() {
    let (mut m, peer) = tracked(TIP_BELOW, 1, true);
    let next = signed_header(m.local_hash, TIP_BELOW as u32 + 1);
    m.handle_response(peer, SyncResponse::Headers(vec![next]));
    assert_eq!(
        m.pipeline.header_downloader.total_downloaded(),
        1,
        "extending header is accepted"
    );
    let got = classify_with_orphans(&mut m);
    assert_eq!(got, HFS, "extending header is no proof");
}
