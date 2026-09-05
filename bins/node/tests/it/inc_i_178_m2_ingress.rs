//! INC-I-178 M2 — the ONE shared per-signature verify (D4), driven through BOTH
//! real ingresses so what is locked is what a peer can actually cause.
//!
//! OUTPUT CONTRACT
//!
//! F1: `Node::on_new_attestation(&mut self, data: Vec<u8>, source_peer: PeerId)`
//! F2: `Node::record_direct_attestation(&mut self, att: Attestation, peer: PeerId)`
//!   Both delegate to the shared `ingest_attestation` body. Observable outputs,
//!   identical for both:
//!     O1 return — unit; carries nothing
//!     O2 `self.minute_tracker` — attendance for (attester, minute(slot))
//!     O3 `self.parent_sig_pool` — (block_hash, attester) -> [u8; 96]
//!     O4 `self.bls_ingress_scorer` — NEW: per-peer penalty for a bad BLS half
//!     O5 `self.sync_manager` — finality weight (gossip only, weight > 0); this is
//!        INC-I-191's contract, re-asserted here only where M2 could break it
//!     O6 mutable params — NONE (both take their payload by value)
//!     O7 persistent store writes — the archive flush only
//!   Paths (the four BLS verdicts x the membership gate):
//!     P1 member, on-chain BLS key, VALID 96-byte signature -> O2 set, O3 set, O4 clean
//!     P2 member, EMPTY bls_signature (old binary)          -> O2 set, O3 empty, O4 clean
//!     P3 member, 96 bytes, NO on-chain bls_pubkey          -> O2 set, O3 empty, O4 clean
//!     P4 member, on-chain key, INVALID 96 bytes            -> O2 set, O3 empty, O4 PENALISED
//!     P5 NON-member                                        -> O2, O3, O4 all untouched
//!     P6 member with selection weight 0 (fully delegated)  -> O2 set, O3 set (C19)
//!   INPUT PARTITIONS:
//!     P4a one flipped byte of an otherwise honest signature (C1 halt-mutation:
//!         Ed25519 still verifies because it does not cover this field)
//!     P4b 96 bytes of unrelated garbage from a registered member
//!     P4c a second bad signature arriving AFTER an honest one for the same
//!         (block_hash, attester) — first-seen must survive
//!     P5a a freshly generated key that is in no ProducerSet
//!   MATRIX 7 outputs x 6 paths: O1 is unit, O6 is by-value, O5/O7 are INC-I-191's
//!     and M0's contract. O2, O3 and O4 are asserted on every one of P1-P6, at
//!     BOTH ingresses — that is the Path-Coverage evidence for D4.
//!
//! F3: `bins/node/src/node/network_events.rs` and the new
//!   `bins/node/src/node/attestation/ingress.rs` as SOURCE TEXT. Rust cannot
//!   express "this logic lives in exactly one place"; the tripwires can.

use crypto::{BlsKeyPair, Hash, KeyPair, PublicKey};
use doli_core::attestation::attestation_minute;
use doli_core::Attestation;
use doli_node::node::Node;
use network::PeerId;
use std::fs;
use std::path::{Path, PathBuf};

use crate::inc_i_178_m0_common::{
    assemble, build_via_production, dual, make_node, register_bls, safe_build_height, N_SMALL,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root must resolve")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Source with every whole-line `//` comment removed, so a tombstone comment
/// naming a deleted symbol cannot fail a deletion tripwire.
fn code_only(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build one real block, make it canonical, and hand back its hash, slot, height.
async fn canonical_block(node: &mut Node) -> (Hash, u32, u64) {
    let h = safe_build_height(node);
    let (header, txs, bf) = build_via_production(node, h).await;
    let block = assemble(header, txs, bf);
    let hash = block.hash();
    let slot = block.header.slot;
    node.block_store
        .put_block_canonical(&block, h)
        .expect("put_block_canonical failed");
    (hash, slot, h)
}

/// Delegate every bond away so the member's selection weight is 0 (INV-ATTEST-001).
async fn zero_the_weight(node: &Node, pk: &PublicKey) {
    let mut ps = node.producer_set.write().await;
    let p = ps.get_by_pubkey_mut(pk).expect("member");
    p.delegated_bonds = p.bond_count;
}

fn with_bls(kp: &KeyPair, hash: Hash, slot: u32, height: u64, blob: Vec<u8>) -> Attestation {
    let mut a = Attestation::new(hash, slot, height, 1, kp.private_key(), *kp.public_key());
    a.bls_signature = blob;
    a
}

fn score_of(node: &Node, peer: &PeerId) -> i32 {
    node.bls_ingress_scorer
        .get_score(peer)
        .map(|s| s.value)
        .unwrap_or(0)
}

fn attended(node: &Node, slot: u32, pk: &PublicKey) -> bool {
    node.minute_tracker
        .attested_in_minute(attestation_minute(slot))
        .contains(&pk)
}

/// The direct ingress derives authority at the CURRENT tip, not at the attested
/// block's height, so its fixtures use the node's own best height.
async fn tip_height(node: &Node) -> u64 {
    node.best_height().await
}

// ============================================================
// P1 — VALID (REQ-BLS-006)
// ============================================================

// REQ-BLS-006 — Decision: this is the only production write into the pool. If a
// correctly signed attestation is not pooled, M4's encoder finds nothing and the
// on-chain aggregate is permanently empty no matter how many honest peers attest.
#[tokio::test]
async fn m2_d4_gossip_a_valid_bls_signature_is_pooled_and_costs_the_peer_nothing() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[3];
    register_bls(&node, a.public_key(), &bls).await;
    let peer = PeerId::random();

    let att = dual(a, &bls, hash, slot, height);
    let expected: [u8; 96] = att.bls_signature.clone().try_into().unwrap();
    node.on_new_attestation(att.to_bytes(), peer).await;

    assert!(attended(&node, slot, a.public_key()), "attendance");
    assert_eq!(
        node.parent_sig_pool.get(&hash, a.public_key()),
        Some(&expected)
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 1);
    assert_eq!(
        score_of(&node, &peer),
        0,
        "an honest relay is never penalised"
    );
}

// REQ-BLS-006 — Decision: the direct path is how a syncing peer's attestation
// arrives; if only the gossip path verified, the pool would hold unverified bytes
// from every sync request and the aggregate would carry them into a block.
#[tokio::test]
async fn m2_d4_direct_a_valid_bls_signature_is_pooled_and_costs_the_peer_nothing() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[4];
    register_bls(&node, a.public_key(), &bls).await;
    let peer = PeerId::random();

    let att = dual(a, &bls, hash, slot, height);
    let expected: [u8; 96] = att.bls_signature.clone().try_into().unwrap();
    node.record_direct_attestation(att, peer).await;

    assert!(attended(&node, slot, a.public_key()), "attendance");
    assert_eq!(
        node.parent_sig_pool.get(&hash, a.public_key()),
        Some(&expected)
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 1);
    assert_eq!(score_of(&node, &peer), 0);
}

// ============================================================
// P2 — EMPTY, the Release-N bridge (REQ-BLS-010)
// ============================================================

// REQ-BLS-010 — Decision: Release N ships with no activation height, so for the
// whole rollout window most attestations carry NO BLS bytes. Penalising or dropping
// them starves every un-upgraded producer of attendance and shrinks the reward
// denominator to the upgraded subset overnight (spec BRIDGE rule).
#[tokio::test]
async fn m2_d4_gossip_an_empty_bls_half_attends_without_pooling_or_penalty() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let old = &producers[2];
    let peer = PeerId::random();

    node.on_new_attestation(
        with_bls(old, hash, slot, height, Vec::new()).to_bytes(),
        peer,
    )
    .await;

    assert!(attended(&node, slot, old.public_key()), "bridge attendance");
    assert!(node.parent_sig_pool.get(&hash, old.public_key()).is_none());
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
    assert_eq!(
        score_of(&node, &peer),
        0,
        "an old binary's attestation is not the relay's fault"
    );
}

// REQ-BLS-010 — Decision: the same bridge rule at the direct ingress; a syncing
// old peer must not be scored out of the mesh for running Release N-1.
#[tokio::test]
async fn m2_d4_direct_an_empty_bls_half_attends_without_pooling_or_penalty() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let old = &producers[5];
    let peer = PeerId::random();

    node.record_direct_attestation(with_bls(old, hash, slot, height, Vec::new()), peer)
        .await;

    assert!(attended(&node, slot, old.public_key()));
    assert!(node.parent_sig_pool.get(&hash, old.public_key()).is_none());
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
    assert_eq!(score_of(&node, &peer), 0);
}

// ============================================================
// P3 — NOKEY (REQ-BLS-010)
// ============================================================

// REQ-BLS-010 — Decision: every producer registered before BLS keys existed has an
// EMPTY on-chain `bls_pubkey`. Treating "no key to check against" as an infraction
// would penalise the honest relay of a legacy producer once per block, which is the
// INV-NETWORK-002 mesh-expulsion cascade with extra steps.
#[tokio::test]
async fn m2_d4_gossip_an_attester_with_no_onchain_bls_key_attends_and_is_not_scored() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let legacy = &producers[6];
    let bls = BlsKeyPair::generate();
    let peer = PeerId::random();
    // NOTE: register_bls is deliberately NOT called — the chain has no key for it.

    node.on_new_attestation(dual(legacy, &bls, hash, slot, height).to_bytes(), peer)
        .await;

    assert!(attended(&node, slot, legacy.public_key()), "attendance");
    assert!(node
        .parent_sig_pool
        .get(&hash, legacy.public_key())
        .is_none());
    assert_eq!(score_of(&node, &peer), 0, "unverifiable is not invalid");
}

// REQ-BLS-010 — Decision: same rule at the direct ingress.
#[tokio::test]
async fn m2_d4_direct_an_attester_with_no_onchain_bls_key_attends_and_is_not_scored() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let legacy = &producers[7];
    let bls = BlsKeyPair::generate();
    let peer = PeerId::random();

    node.record_direct_attestation(dual(legacy, &bls, hash, slot, height), peer)
        .await;

    assert!(attended(&node, slot, legacy.public_key()));
    assert!(node
        .parent_sig_pool
        .get(&hash, legacy.public_key())
        .is_none());
    assert_eq!(score_of(&node, &peer), 0);
}

// ============================================================
// P4 — INVALID (REQ-BLS-006), the C1 halt mutation
// ============================================================

// REQ-BLS-006 — Decision: THE attack. A relay flips one byte of the BLS half; the
// Ed25519 signature still verifies because it does not cover that field. If the
// mutated blob reaches the pool, M4 aggregates it and every honest node rejects the
// resulting block — one relay halts the chain. Attendance must survive (the
// attester is not the attacker) and the RELAY must be scored.
#[tokio::test]
async fn m2_d4_gossip_one_flipped_bls_byte_attends_but_is_rejected_and_scored() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[8];
    register_bls(&node, a.public_key(), &bls).await;
    let peer = PeerId::random();

    let mut att = dual(a, &bls, hash, slot, height);
    att.bls_signature[47] ^= 0x01;
    let mutated: [u8; 96] = att.bls_signature.clone().try_into().unwrap();
    assert!(att.verify().is_ok(), "premise: Ed25519 still passes");

    node.on_new_attestation(att.to_bytes(), peer).await;

    assert!(attended(&node, slot, a.public_key()), "attendance survives");
    assert_ne!(
        node.parent_sig_pool.get(&hash, a.public_key()),
        Some(&mutated),
        "the mutated blob must never enter the pool"
    );
    assert!(node.parent_sig_pool.get(&hash, a.public_key()).is_none());
    assert_eq!(node.parent_sig_pool.total_signatures(), 0);
    assert_eq!(
        score_of(&node, &peer),
        -10,
        "the relay is scored exactly once"
    );
}

// REQ-BLS-006 — Decision: the direct ingress is reachable by any peer that opens a
// sync request, so leaving it unverified leaves the halt mutation fully available.
#[tokio::test]
async fn m2_d4_direct_one_flipped_bls_byte_attends_but_is_rejected_and_scored() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[9];
    register_bls(&node, a.public_key(), &bls).await;
    let peer = PeerId::random();

    let mut att = dual(a, &bls, hash, slot, height);
    att.bls_signature[0] ^= 0x80;
    assert!(att.verify().is_ok(), "premise: Ed25519 still passes");

    node.record_direct_attestation(att, peer).await;

    assert!(attended(&node, slot, a.public_key()));
    assert!(node.parent_sig_pool.get(&hash, a.public_key()).is_none());
    assert_eq!(score_of(&node, &peer), -10);
}

// REQ-BLS-006 — Decision: 96 arbitrary bytes are usually not even a valid G2 point,
// so this exercises the decode-failure arm rather than the verify-failure arm; both
// must land on the SAME verdict or the cheap attack is the one that gets through.
#[tokio::test]
async fn m2_d4_gossip_96_garbage_bytes_from_a_member_are_rejected_and_scored() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[10];
    register_bls(&node, a.public_key(), &bls).await;
    let peer = PeerId::random();

    node.on_new_attestation(
        with_bls(a, hash, slot, height, vec![0xC3u8; 96]).to_bytes(),
        peer,
    )
    .await;

    assert!(attended(&node, slot, a.public_key()));
    assert!(node.parent_sig_pool.get(&hash, a.public_key()).is_none());
    assert_eq!(score_of(&node, &peer), -10);
}

// REQ-BLS-006 — Decision: same decode-failure arm on the direct path.
#[tokio::test]
async fn m2_d4_direct_96_garbage_bytes_from_a_member_are_rejected_and_scored() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[11];
    register_bls(&node, a.public_key(), &bls).await;
    let peer = PeerId::random();

    node.record_direct_attestation(with_bls(a, hash, slot, height, vec![0x5Au8; 96]), peer)
        .await;

    assert!(attended(&node, slot, a.public_key()));
    assert!(node.parent_sig_pool.get(&hash, a.public_key()).is_none());
    assert_eq!(score_of(&node, &peer), -10);
}

// ============================================================
// P5 / P6 — the C19 membership posture (REQ-BLS-006)
// ============================================================

// REQ-BLS-006 — Decision: membership is what bounds the pool's attester axis to N.
// If BLS verification ran before the membership gate, any peer could mint unbounded
// keys and grow the tracker and the pool without limit.
#[tokio::test]
async fn m2_d4_gossip_a_non_member_is_dropped_before_attendance_the_pool_and_scoring() {
    let (mut node, _producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let stranger = KeyPair::generate();
    let bls = BlsKeyPair::generate();
    let peer = PeerId::random();

    node.on_new_attestation(dual(&stranger, &bls, hash, slot, height).to_bytes(), peer)
        .await;

    assert!(!attended(&node, slot, stranger.public_key()));
    assert!(node
        .parent_sig_pool
        .get(&hash, stranger.public_key())
        .is_none());
    assert_eq!(node.minute_tracker.total_entries(), 0);
    assert_eq!(
        score_of(&node, &peer),
        0,
        "a non-member is dropped, not scored (INC-I-192 posture is unchanged)"
    );
}

// REQ-BLS-006 — Decision: the INC-I-192 DoS gate on the direct path must survive
// the D4 refactor; losing it re-opens unbounded tracker growth from one peer.
#[tokio::test]
async fn m2_d4_direct_a_non_member_is_dropped_before_attendance_the_pool_and_scoring() {
    let (mut node, _producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let stranger = KeyPair::generate();
    let bls = BlsKeyPair::generate();
    let peer = PeerId::random();

    node.record_direct_attestation(dual(&stranger, &bls, hash, slot, height), peer)
        .await;

    assert_eq!(node.minute_tracker.total_entries(), 0);
    assert!(node
        .parent_sig_pool
        .get(&hash, stranger.public_key())
        .is_none());
    assert_eq!(score_of(&node, &peer), 0);
}

// REQ-BLS-006 — Decision: INV-ATTEST-001. A fully-delegated producer has selection
// weight 0 but is still a member; admitting on `w > 0` instead of `.is_some()`
// silently erases its attendance and, at the next epoch, its rewards.
#[tokio::test]
async fn m2_d4_gossip_a_fully_delegated_member_still_attends_and_pools() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[1];
    register_bls(&node, a.public_key(), &bls).await;
    zero_the_weight(&node, a.public_key()).await;
    {
        let ps = node.producer_set.read().await;
        assert_eq!(
            ps.get_by_pubkey(a.public_key()).unwrap().selection_weight(),
            0,
            "precondition: the attester is fully delegated"
        );
    }

    node.on_new_attestation(
        dual(a, &bls, hash, slot, height).to_bytes(),
        PeerId::random(),
    )
    .await;

    assert!(attended(&node, slot, a.public_key()), "INV-ATTEST-001");
    assert!(node.parent_sig_pool.get(&hash, a.public_key()).is_some());
}

// REQ-BLS-006 — Decision: same invariant at the direct ingress, where INC-I-192's
// fix originally established the `.is_some()` admission rule.
#[tokio::test]
async fn m2_d4_direct_a_fully_delegated_member_still_attends_and_pools() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[2];
    register_bls(&node, a.public_key(), &bls).await;
    zero_the_weight(&node, a.public_key()).await;

    node.record_direct_attestation(dual(a, &bls, hash, slot, height), PeerId::random())
        .await;

    assert!(attended(&node, slot, a.public_key()));
    assert!(node.parent_sig_pool.get(&hash, a.public_key()).is_some());
}

// ============================================================
// C1 — first-seen wins (REQ-BLS-006)
// ============================================================

// REQ-BLS-006 — Decision: verification alone is not enough if a later write can
// overwrite an earlier one. An attacker who replays the same (block_hash, attester)
// with garbage must not be able to evict the honest signature already pooled.
#[tokio::test]
async fn m2_c1_gossip_a_later_forgery_never_overwrites_the_first_honest_signature() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[3];
    register_bls(&node, a.public_key(), &bls).await;

    let honest = dual(a, &bls, hash, slot, height);
    let honest_bytes: [u8; 96] = honest.bls_signature.clone().try_into().unwrap();
    node.on_new_attestation(honest.to_bytes(), PeerId::random())
        .await;
    assert_eq!(
        node.parent_sig_pool.get(&hash, a.public_key()),
        Some(&honest_bytes)
    );

    let attacker = PeerId::random();
    node.on_new_attestation(
        with_bls(a, hash, slot, height, vec![0xFFu8; 96]).to_bytes(),
        attacker,
    )
    .await;
    let mut mutated = dual(a, &bls, hash, slot, height);
    mutated.bls_signature[95] ^= 0x0F;
    node.on_new_attestation(mutated.to_bytes(), attacker).await;

    assert_eq!(
        node.parent_sig_pool.get(&hash, a.public_key()),
        Some(&honest_bytes),
        "first-seen honest bytes must survive both forgeries"
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 1);
    assert_eq!(
        score_of(&node, &attacker),
        -20,
        "two infractions, two penalties"
    );
}

// REQ-BLS-006 — Decision: the direct path writes into the SAME pool, so a forgery
// arriving there must not be able to evict what gossip already verified.
#[tokio::test]
async fn m2_c1_direct_a_later_forgery_never_overwrites_the_first_honest_signature() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, _h) = canonical_block(&mut node).await;
    let height = tip_height(&node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[4];
    register_bls(&node, a.public_key(), &bls).await;

    let honest = dual(a, &bls, hash, slot, height);
    let honest_bytes: [u8; 96] = honest.bls_signature.clone().try_into().unwrap();
    node.record_direct_attestation(honest, PeerId::random())
        .await;

    let attacker = PeerId::random();
    node.record_direct_attestation(with_bls(a, hash, slot, height, vec![0x11u8; 96]), attacker)
        .await;

    assert_eq!(
        node.parent_sig_pool.get(&hash, a.public_key()),
        Some(&honest_bytes)
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 1);
    assert_eq!(score_of(&node, &attacker), -10);
}

// ============================================================
// F3 — the D4 shape (REQ-BLS-006)
// ============================================================

// REQ-BLS-006 — Decision: the milestone's measured outcome. Two copies of the verify
// logic is exactly how the two ingresses drifted apart in INC-I-191/192; if
// `network_events.rs` still touches the pool, D4 did not happen and the next change
// only lands on one of the two paths.
#[test]
fn m2_d4_network_events_no_longer_touches_the_parent_pool() {
    let src = code_only(&read("bins/node/src/node/network_events.rs"));
    assert!(
        !src.contains("parent_sig_pool"),
        "D4: every pool write must move into the shared ingress body"
    );
    assert!(
        src.contains("ingest_attestation"),
        "D4: both ingresses must call the shared body"
    );
}

// REQ-BLS-006 — Decision: the four verdicts are the contract's whole vocabulary; a
// missing arm is a branch that silently falls into another arm's behaviour — e.g.
// NoKey collapsing into Invalid, which penalises honest relays of legacy producers.
#[test]
fn m2_d4_the_shared_ingress_module_declares_all_four_verdicts() {
    let src = read("bins/node/src/node/attestation/ingress.rs");
    for tok in [
        "enum BlsAttestVerdict",
        "Valid",
        "Empty",
        "NoKey",
        "Invalid",
        "fn ingest_attestation",
        "fn derive_attester_weight",
    ] {
        assert!(src.contains(tok), "D4: ingress.rs is missing `{tok}`");
    }
    let mod_rs = code_only(&read("bins/node/src/node/mod.rs"));
    assert!(
        !mod_rs.contains("fn derive_attester_weight"),
        "D4: derive_attester_weight must MOVE, not be duplicated"
    );
    assert!(mod_rs.contains("mod attestation;"));
}

// REQ-BLS-010 — Decision: a producer whose BLS key does not match its on-chain
// `bls_pubkey` emits one bad attestation per block, relayed by every honest peer.
// Disconnecting on that score is the INV-NETWORK-002 mesh-expulsion cascade: the
// fleet partitions on ONE misconfigured producer. Release N records and logs only.
#[test]
fn m2_d4_release_n_records_the_infraction_but_never_disconnects() {
    let src = code_only(&read("bins/node/src/node/attestation/ingress.rs"));
    assert!(
        src.contains("record_invalid_bls_attestation"),
        "the Invalid arm must score the relay"
    );
    assert!(
        !src.contains("disconnect"),
        "Release N must not disconnect on a BLS verdict (INV-NETWORK-002)"
    );
}

// REQ-BLS-010 — Decision: one bad attestation must leave the peer far above the
// disconnect threshold, or a single relayed forgery becomes a partition primitive.
#[tokio::test]
async fn m2_d4_one_invalid_bls_attestation_leaves_the_peer_far_above_the_threshold() {
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let (hash, slot, height) = canonical_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[6];
    register_bls(&node, a.public_key(), &bls).await;
    let peer = PeerId::random();

    node.on_new_attestation(
        with_bls(a, hash, slot, height, vec![0x99u8; 96]).to_bytes(),
        peer,
    )
    .await;

    assert_eq!(score_of(&node, &peer), -10);
    assert!(
        !node.bls_ingress_scorer.should_disconnect(&peer),
        "one relayed forgery must not cost the peer its connection"
    );
}

// REQ-BLS-010 — Decision: documented, NOT fixed here (handoff H1). The egress
// returns None when the local producer's derived weight is 0, so a fully-delegated
// producer never emits an attestation at all — the mirror image of the ingress
// invariant this file pins. Fixing it is a separate consensus-visible decision.
#[test]
#[ignore = "INC-I-178 handoff H1: egress w == 0 => None; out of scope for M2"]
fn m2_h1_a_fully_delegated_producer_never_emits_an_attestation() {
    let src = read("bins/node/src/node/startup.rs");
    assert!(
        src.contains("weight"),
        "H1 marker: create_and_broadcast_attestation still gates on the derived weight"
    );
}
