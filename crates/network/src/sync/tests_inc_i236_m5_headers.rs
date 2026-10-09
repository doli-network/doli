// covers: headers.rs sync_engine/response.rs types.rs
//! INC-I-236 M5: header-first sync verifies the producer signature at h >= AH.
// REQ-AUTH-008 — Decision: a failure means header-first sync still accepts forged/unsigned headers above the AH (or wrongly rejects legacy ones below it).
//
// OUTPUT CONTRACT (HeaderDownloader::process_headers):
//   return valid_count | self.validated_headers (pending_count/take_headers)
//   | self.total_downloaded | self.expected_prev_hash | self.last_chain_break
// PATHS: all-valid | stop at bad signature (index i>0) | stop at index 0 | continuation batch
// INPUT PARTITIONS: sig {valid, empty, forged} x height {< AH, == AH, > AH, AH-1}
//   x AH {set, default u64::MAX} x batch {fresh, continuation}
// MATRIX: forged/empty above AH -> all 5 outputs asserted (forged mid-batch test);
//   index-0 stop -> return/pending/total/expected_prev asserted; all-valid -> return/pending/expected_prev;
//   continuation -> return/pending/total/expected_prev; chain_break asserted None on signature stop.

use std::time::Duration;

use crypto::{Hash, KeyPair};
use doli_core::BlockHeader;
use vdf::{VdfOutput, VdfProof};

use super::headers::HeaderDownloader;
use super::SyncConfig;

#[path = "../../../../bins/node/tests/support/block_sig.rs"]
mod block_sig;

const AH: u64 = 100;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn unsigned_header(prev_hash: Hash, slot: u32, kp: &KeyPair) -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: now(),
        slot,
        producer: *kp.public_key(),
        vdf_output: VdfOutput { value: vec![0; 32] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
}

#[derive(Clone, Copy)]
enum Sig {
    Valid,
    Empty,
    Forged,
}

/// Build a linked chain from `prev`; `sigs[i]` picks how header i is signed.
fn chain(prev: Hash, first_slot: u32, kp: &KeyPair, sigs: &[Sig]) -> Vec<BlockHeader> {
    let attacker = KeyPair::generate();
    let mut out = Vec::new();
    let mut prev_hash = prev;
    for (i, s) in sigs.iter().enumerate() {
        let mut h = unsigned_header(prev_hash, first_slot + i as u32, kp);
        match s {
            Sig::Valid => block_sig::sign_header(&mut h, kp),
            Sig::Empty => {}
            Sig::Forged => {
                // Signed by the attacker key while claiming `kp` as producer.
                let msg = h.signing_message();
                h.vdf_proof.pi =
                    crypto::signature::sign(msg.as_bytes(), attacker.private_key()).to_vec();
            }
        }
        prev_hash = h.hash();
        out.push(h);
    }
    out
}

fn downloader(ah: u64) -> HeaderDownloader {
    let mut d = HeaderDownloader::new(2000, Duration::from_secs(30));
    d.set_block_signature_activation_height(ah);
    d
}

// REQ-AUTH-008 (Should) — Acceptance: one forged signature above the AH stops the batch at it.
#[test]
fn inc_i_236_m5_forged_signature_above_ah_rejected_at_that_header() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([7u8; 32]);
    let hs = chain(
        tip,
        100,
        &kp,
        &[Sig::Valid, Sig::Valid, Sig::Forged, Sig::Valid, Sig::Valid],
    );
    let mut d = downloader(AH);
    let valid = d.process_headers(&hs, tip, AH - 1);
    assert_eq!(valid, 2, "batch must stop at the forged header (index 2)");
    assert_eq!(d.pending_count(), 2);
    assert_eq!(d.total_downloaded(), 2);
    assert_eq!(d.expected_prev_hash(), Some(hs[1].hash()));
    assert!(
        d.take_chain_break().is_none(),
        "bad signature is not a chain break"
    );
    let taken = d.take_headers(10);
    assert_eq!(taken.len(), 2);
    assert_eq!(taken[0].hash(), hs[0].hash());
    assert_eq!(taken[1].hash(), hs[1].hash());
}

// REQ-AUTH-008 (Should) — Acceptance: an empty signature above the AH stops the batch at it.
#[test]
fn inc_i_236_m5_empty_signature_above_ah_rejected_at_that_header() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([8u8; 32]);
    let hs = chain(tip, 200, &kp, &[Sig::Valid, Sig::Empty, Sig::Valid]);
    let mut d = downloader(AH);
    let valid = d.process_headers(&hs, tip, AH + 50);
    assert_eq!(valid, 1);
    assert_eq!(d.pending_count(), 1);
    assert_eq!(d.expected_prev_hash(), Some(hs[0].hash()));
}

// REQ-AUTH-008 (Should) — Acceptance: a bad first header yields zero and leaves continuation state untouched.
#[test]
fn inc_i_236_m5_bad_first_header_yields_zero_and_no_state_change() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([9u8; 32]);
    let hs = chain(tip, 300, &kp, &[Sig::Forged, Sig::Valid]);
    let mut d = downloader(AH);
    assert_eq!(d.process_headers(&hs, tip, AH), 0);
    assert_eq!(d.pending_count(), 0);
    assert_eq!(d.total_downloaded(), 0);
    assert_eq!(d.expected_prev_hash(), None);
}

// REQ-AUTH-008 (Should) — Acceptance: a fully signed chain above the AH is fully accepted.
#[test]
fn inc_i_236_m5_fully_signed_chain_above_ah_accepted() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([1u8; 32]);
    let hs = chain(tip, 100, &kp, &[Sig::Valid; 6]);
    let mut d = downloader(AH);
    assert_eq!(d.process_headers(&hs, tip, AH - 1), 6);
    assert_eq!(d.pending_count(), 6);
    assert_eq!(d.expected_prev_hash(), Some(hs[5].hash()));
}

// REQ-AUTH-008 (Should) — Acceptance: legacy unsigned headers below the AH stay accepted.
#[test]
fn inc_i_236_m5_unsigned_chain_below_ah_accepted() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([2u8; 32]);
    let hs = chain(tip, 1, &kp, &[Sig::Empty; 5]);
    let mut d = downloader(AH);
    assert_eq!(d.process_headers(&hs, tip, 0), 5);
    assert_eq!(d.pending_count(), 5);
}

// REQ-AUTH-008 (Should) — Acceptance: the default (u64::MAX) keeps the rule off.
#[test]
fn inc_i_236_m5_default_downloader_rule_off() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([3u8; 32]);
    let hs = chain(tip, 500, &kp, &[Sig::Forged, Sig::Empty]);
    let mut d = HeaderDownloader::new(2000, Duration::from_secs(30));
    assert_eq!(d.process_headers(&hs, tip, 1_000_000), 2);
}

// REQ-AUTH-008 (Should) — Acceptance: header exactly at the AH needs a signature.
#[test]
fn inc_i_236_m5_boundary_header_at_ah_rejected() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([4u8; 32]);
    let hs = chain(tip, 99, &kp, &[Sig::Empty, Sig::Empty]);
    let mut d = downloader(AH);
    // heights: AH-1 (accepted), AH (rejected)
    assert_eq!(d.process_headers(&hs, tip, AH - 2), 1);
    assert_eq!(d.pending_count(), 1);
    assert_eq!(d.expected_prev_hash(), Some(hs[0].hash()));
}

// REQ-AUTH-008 (Should) — Acceptance: header at AH-1 with empty pi is accepted on its own.
#[test]
fn inc_i_236_m5_boundary_header_at_ah_minus_one_accepted() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([5u8; 32]);
    let hs = chain(tip, 99, &kp, &[Sig::Empty]);
    let mut d = downloader(AH);
    assert_eq!(d.process_headers(&hs, tip, AH - 2), 1);

    let mut d2 = downloader(AH);
    assert_eq!(
        d2.process_headers(&hs, tip, AH - 1),
        0,
        "same header at AH is rejected"
    );
}

// REQ-AUTH-008 (Should) — Acceptance: continuation batches track height across calls.
#[test]
fn inc_i_236_m5_continuation_tracks_height_across_batches() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([6u8; 32]);
    let local_height = AH - 6; // first batch: AH-5, AH-4, AH-3
    let first = chain(tip, 10, &kp, &[Sig::Empty; 3]);
    let mut d = downloader(AH);
    assert_eq!(d.process_headers(&first, tip, local_height), 3);

    // second batch: AH-2, AH-1 (unsigned OK), AH (unsigned -> reject), AH+1
    let last = first[2].hash();
    let second = chain(
        last,
        13,
        &kp,
        &[Sig::Empty, Sig::Empty, Sig::Empty, Sig::Valid],
    );
    let valid = d.process_headers(&second, tip, local_height);
    assert_eq!(valid, 2, "header at AH in the 2nd batch must be rejected");
    assert_eq!(d.pending_count(), 5);
    assert_eq!(d.total_downloaded(), 5);
    assert_eq!(d.expected_prev_hash(), Some(second[1].hash()));
}

// REQ-AUTH-008 (Should) — Acceptance: a signed resend after a rejection continues at the right height.
#[test]
fn inc_i_236_m5_continuation_after_rejection_keeps_height() {
    let kp = KeyPair::generate();
    let tip = Hash::from_bytes([10u8; 32]);
    let first = chain(tip, 50, &kp, &[Sig::Empty, Sig::Empty]); // AH-2, AH-1
    let mut d = downloader(AH);
    assert_eq!(d.process_headers(&first, tip, AH - 3), 2);

    let bad = chain(first[1].hash(), 52, &kp, &[Sig::Empty]); // at AH
    assert_eq!(d.process_headers(&bad, tip, AH - 3), 0);

    let mut good = bad[0].clone();
    block_sig::sign_header(&mut good, &kp);
    let next = chain(good.hash(), 53, &kp, &[Sig::Empty]); // AH+1 unsigned
    assert_eq!(d.process_headers(&[good, next[0].clone()], tip, AH - 3), 1);
    assert_eq!(d.pending_count(), 3);
}

// REQ-AUTH-008 (Should) — Acceptance: SyncConfig ships the rule dormant.
#[test]
fn inc_i_236_m5_sync_config_default_is_u64_max() {
    assert_eq!(
        SyncConfig::default().inc_i_236_block_signature_activation_height,
        u64::MAX
    );
}

// REQ-AUTH-008 (Should) — Acceptance: probe — forged headers above the AH accepted out of 3.
#[test]
fn inc_i_236_m5_probe_forged_headers_accepted() {
    let kp = KeyPair::generate();
    let mut accepted = 0usize;
    for k in 0..3u64 {
        let tip = Hash::from_bytes([0x40 + k as u8; 32]);
        let hs = chain(tip, 700 + k as u32, &kp, &[Sig::Forged]);
        let mut d = downloader(AH);
        accepted += d.process_headers(&hs, tip, AH + k);
    }
    println!("FORGED_HEADERS_ACCEPTED={accepted}");
    eprintln!("FORGED_HEADERS_ACCEPTED={accepted}");
    assert_eq!(accepted, 0);
}
