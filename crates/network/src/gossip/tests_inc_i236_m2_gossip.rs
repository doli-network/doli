// covers: validation.rs relay_decision.rs staleness.rs service/gossip_intake.rs service/behaviour_events.rs service/command_api.rs service/mod.rs config.rs
//! INC-I-236 Run A M2: the blocks gossip topic verifies the block signature before the swarm Accepts.
// REQ-AUTH-023 — Decision: a failure means a forged or unsigned block above the AH is relayed fleet-wide with score credit and emitted as NewBlock.
// REQ-AUTH-021 — Decision: a failure means a forged pi-variant seen first suppresses or replaces the genuine signed block.
// NEEDS-API: StalenessCtx { best_height: u64, block_signature_activation_height: u64 } (best height beside best_slot, AH from NetworkConfig); relay_decision(Blocks) Rejects when best_height + 1 >= AH and doli_core::validation::verify_block_signature fails.
//
// OUTPUT CONTRACT (relay_decision(RelayTopic::Blocks, bytes, &StalenessCtx)):
//   O1 RelayDecision variant: AcceptBlock (only arm that emits NetworkEvent::NewBlock) | Reject | Ignore
//   O2 acceptance(): the MessageAcceptance the swarm reports (Accept relays + credits the sender)
//   O3 AcceptBlock payload: the decoded block, pi byte-equal to the sent block
// PATHS: staleness on (genesis_time set) | staleness off (genesis_time == 0); rule on | rule dormant (u64::MAX)
// INPUT PARTITIONS: pi {producer, wrong key, empty, 64 garbage bytes}
//   x prospective height best_height + 1 {< AH, == AH, > AH} x order {forged variant first, genuine second}
// MATRIX: forged/unsigned/garbage above AH -> O1,O2; boundary == AH -> O1,O2; staleness off -> O1,O2;
//   signed / below-AH / dormant controls -> O1,O2,O3; f76 sequence -> O1,O2,O3 per message.

use crypto::{Hash, KeyPair};
use doli_core::{Block, BlockHeader};
use libp2p::gossipsub::MessageAcceptance;
use vdf::{VdfOutput, VdfProof};

use super::staleness::StalenessCtx;
use super::{relay_decision, RelayDecision, RelayTopic};

const AH: u64 = 500;
const SLOT_SECS: u64 = 10;
const WALL: u32 = 1_000;
const GENESIS_TIME: u64 = 1_000_000;
const NOW: u64 = GENESIS_TIME + WALL as u64 * SLOT_SECS;

#[derive(Clone, Copy)]
enum Sig {
    None,
    Producer,
    WrongKey,
    Garbage,
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

/// A fresh (wall-clock slot) block by `kp`, pi chosen by `s`.
fn block(kp: &KeyPair, s: Sig) -> Block {
    let mut h = BlockHeader {
        version: 1,
        prev_hash: Hash::from_bytes([0x33; 32]),
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: NOW,
        slot: WALL,
        producer: *kp.public_key(),
        vdf_output: VdfOutput { value: vec![0; 32] },
        vdf_proof: VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    h.vdf_proof.pi = pi(&h, kp, s);
    Block::new(h, vec![])
}

fn ctx(best_height: u64, ah: u64, genesis_time: u64) -> StalenessCtx {
    StalenessCtx {
        now_unix: NOW,
        genesis_time,
        slot_duration: SLOT_SECS,
        best_slot: WALL - 1,
        best_height,
        block_signature_activation_height: ah,
    }
}

fn decide(b: &Block, c: &StalenessCtx) -> RelayDecision {
    relay_decision(RelayTopic::Blocks, &b.serialize(), c)
}

fn assert_rejected(d: &RelayDecision, what: &str) {
    assert!(
        matches!(d.acceptance(), Some(MessageAcceptance::Reject)),
        "O2: {what} must be reported Reject, got {d:?}"
    );
    assert!(
        !matches!(d, RelayDecision::AcceptBlock(_)),
        "O1: {what} must not reach NewBlock"
    );
}

fn assert_accepted(d: RelayDecision, sent: &Block) {
    assert!(
        matches!(d.acceptance(), Some(MessageAcceptance::Accept)),
        "O2: {d:?}"
    );
    let RelayDecision::AcceptBlock(got) = d else {
        panic!("O1: expected AcceptBlock");
    };
    assert_eq!(got.hash(), sent.hash(), "O3");
    assert_eq!(got.header.vdf_proof.pi, sent.header.vdf_proof.pi, "O3");
}

// REQ-AUTH-023 (Must) — Decision: a failure means a forged block above the AH is Accept-relayed by the swarm.
#[test]
fn req_auth_023_p1_gossip_forged_block_rejected() {
    let kp = KeyPair::generate();
    let d = decide(&block(&kp, Sig::WrongKey), &ctx(AH + 10, AH, GENESIS_TIME));
    assert_rejected(&d, "wrong-key block above the AH");
}

// REQ-AUTH-023 (Must) — Decision: a failure means an unsigned block above the AH is Accept-relayed by the swarm.
#[test]
fn req_auth_023_p1_gossip_unsigned_block_above_ah_rejected() {
    let kp = KeyPair::generate();
    let d = decide(&block(&kp, Sig::None), &ctx(AH + 10, AH, GENESIS_TIME));
    assert_rejected(&d, "unsigned block above the AH");
}

// REQ-AUTH-023 (Must) — Decision: a failure means a 64-byte garbage pi above the AH is Accept-relayed.
#[test]
fn req_auth_023_p1_gossip_garbage_pi_rejected() {
    let kp = KeyPair::generate();
    let d = decide(&block(&kp, Sig::Garbage), &ctx(AH + 10, AH, GENESIS_TIME));
    assert_rejected(&d, "garbage-pi block above the AH");
}

// REQ-AUTH-023 (Must) — Decision: a failure means the gossip gate is off at prospective height == AH.
#[test]
fn req_auth_023_p1_gossip_unsigned_at_boundary_rejected() {
    let kp = KeyPair::generate();
    let d = decide(&block(&kp, Sig::None), &ctx(AH - 1, AH, GENESIS_TIME));
    assert_rejected(&d, "unsigned block at prospective height == AH");
}

// REQ-AUTH-023 (Must) — Decision: a failure means the staleness fail-open path also skips the signature check.
#[test]
fn req_auth_023_p1_gossip_forged_rejected_with_staleness_disabled() {
    let kp = KeyPair::generate();
    let d = decide(&block(&kp, Sig::WrongKey), &ctx(AH + 10, AH, 0));
    assert_rejected(&d, "wrong-key block with genesis_time == 0");
}

// REQ-AUTH-023 (Must) — Decision: a failure means the fix stops relaying correctly signed blocks above the AH.
#[test]
fn req_auth_023_p1_gossip_signed_block_accepted() {
    let kp = KeyPair::generate();
    let b = block(&kp, Sig::Producer);
    assert_accepted(decide(&b, &ctx(AH + 10, AH, GENESIS_TIME)), &b);
}

// REQ-AUTH-023 (Must) — Decision: a failure means the fix stops relaying legal unsigned blocks below the AH.
#[test]
fn req_auth_023_p1_gossip_unsigned_below_ah_accepted() {
    let kp = KeyPair::generate();
    let b = block(&kp, Sig::None);
    assert_accepted(decide(&b, &ctx(AH - 2, AH, GENESIS_TIME)), &b);
}

// REQ-AUTH-023 (Must) — Decision: a failure means the gate is live before activation (dormant default broken).
#[test]
fn req_auth_023_p1_gossip_dormant_ah_accepts_forged() {
    let kp = KeyPair::generate();
    let b = block(&kp, Sig::WrongKey);
    assert_accepted(decide(&b, &ctx(1_000_000, u64::MAX, GENESIS_TIME)), &b);
}

// REQ-AUTH-021 (Must) — Decision: a failure means a forged pi-variant seen first gets relayed or blocks the genuine block.
#[test]
fn req_auth_021_f76_forged_pi_variant_first_genuine_accepted() {
    let kp = KeyPair::generate();
    let genuine = block(&kp, Sig::Producer);
    let mut forged = genuine.clone();
    forged.header.vdf_proof.pi = pi(&genuine.header, &kp, Sig::WrongKey);
    assert_eq!(forged.hash(), genuine.hash(), "setup: same block hash");
    assert_ne!(
        forged.serialize(),
        genuine.serialize(),
        "setup: distinct bytes"
    );
    let c = ctx(AH + 10, AH, GENESIS_TIME);
    assert_rejected(&decide(&forged, &c), "forged pi-variant (first)");
    assert_accepted(decide(&genuine, &c), &genuine);
}
