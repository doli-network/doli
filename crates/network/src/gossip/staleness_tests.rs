use super::super::encode_tx_batch;
use super::*;
use crate::gossip::{
    relay_decision, RelayDecision, RelayTopic, ATTESTATION_TOPIC, BLOCKS_TOPIC, PRODUCERS_TOPIC,
    TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::{Block, BlockHeader, Heartbeat};
use libp2p::gossipsub::MessageAcceptance;

// INC-I-237 M1: every classifier below is reached through the one relay table.
// M3 (R2): a first-sight, in-window attestation or vote is Deferred to the node
// verdict; like `tx_d`, Defer maps to Accept ("processed").
fn att_d(data: &[u8], c: &StalenessCtx) -> MessageAcceptance {
    deferred_or_ignored(relay_decision(RelayTopic::Attestations, data, c))
}

fn vote_d(data: &[u8], c: &StalenessCtx) -> MessageAcceptance {
    deferred_or_ignored(relay_decision(RelayTopic::Votes, data, c))
}

fn deferred_or_ignored(d: RelayDecision) -> MessageAcceptance {
    match d {
        RelayDecision::Defer => MessageAcceptance::Accept,
        RelayDecision::Ignore => MessageAcceptance::Ignore,
        _ => panic!("from M3 an attestation/vote relay decision is Defer or Ignore"),
    }
}

/// INC-I-237 M2 (D1): a first-sight full tx message is Deferred to the node
/// verdict, a 0xAA announce stays LocalOnly; both map to Accept ("processed").
fn tx_d(data: &[u8], c: &StalenessCtx) -> MessageAcceptance {
    match relay_decision(RelayTopic::Transactions, data, c) {
        RelayDecision::Defer | RelayDecision::LocalOnly => MessageAcceptance::Accept,
        RelayDecision::Ignore => MessageAcceptance::Ignore,
        _ => panic!("from M2 a tx relay decision is Defer, LocalOnly or Ignore"),
    }
}

// ── verdict helpers (MessageAcceptance has no PartialEq) ──────────────
fn is_accept(v: MessageAcceptance) -> bool {
    matches!(v, MessageAcceptance::Accept)
}
fn is_ignore(v: MessageAcceptance) -> bool {
    matches!(v, MessageAcceptance::Ignore)
}

// Standard clock fixture: genesis=1000, slot_duration=10, now=2000 → wall_slot=100.
const GEN: u64 = 1000;
const DUR: u64 = 10;
const NOW: u64 = 2000;
const WALL: u32 = 100;

fn ctx() -> StalenessCtx {
    StalenessCtx {
        now_unix: NOW,
        genesis_time: GEN,
        slot_duration: DUR,
        best_slot: WALL,
    }
}

fn make_vote_json(producer: &str, version: &str, vote: &str, timestamp: u64) -> Vec<u8> {
    format!(
            r#"{{"version":"{version}","vote":"{vote}","producer_id":"{producer}","timestamp":{timestamp},"signature":"00"}}"#
        )
        .into_bytes()
}

// Distinct n → distinct (amount,height) → distinct txid. Amount=BlockHeight=u64.
fn make_tx(n: u64) -> doli_core::Transaction {
    doli_core::Transaction::new_coinbase(100 + n, Hash::ZERO, n, 0)
}

// ── M0: registry completeness ─────────────────────────────────────────

// REQ-GRV-003 — Decision: a failure means a subscribed topic string no longer reaches its own table row, so its messages fall to the Unknown (Ignore) row and stop propagating.
#[test]
fn from_topic_str_round_trips_all_subscribed_topics() {
    let cases = [
        (BLOCKS_TOPIC, RelayTopic::Blocks),
        (PRODUCERS_TOPIC, RelayTopic::Producers),
        (ATTESTATION_TOPIC, RelayTopic::Attestations),
        (VOTES_TOPIC, RelayTopic::Votes),
        (TRANSACTIONS_TOPIC, RelayTopic::Transactions),
    ];
    for (topic_str, expected) in cases {
        assert_eq!(
            RelayTopic::from_topic_str(topic_str),
            expected,
            "{topic_str}"
        );
    }
    assert_eq!(
        RelayTopic::from_topic_str(&super::super::region_topic(3)),
        RelayTopic::RegionBlocks
    );
    assert_eq!(
        RelayTopic::from_topic_str("/doli/unknown/9"),
        RelayTopic::Unknown
    );
}

// ── M1: attestations ──────────────────────────────────────────────────

fn make_attestation(slot: u32) -> Vec<u8> {
    make_attestation_id(slot, 0, Hash::ZERO)
}

fn make_attestation_id(slot: u32, attester_byte: u8, block_hash: Hash) -> Vec<u8> {
    Attestation {
        block_hash,
        slot,
        height: 0,
        attester: PublicKey::from_bytes([attester_byte; 32]),
        attester_weight: 0,
        signature: Signature::from_bytes([0u8; 64]),
        bls_signature: Vec::new(),
    }
    .to_bytes()
}

#[test]
fn attestation_stale_ignored() {
    let data = make_attestation(WALL - ATTEST_STALE_SLOTS - 1);
    assert!(
        is_ignore(att_d(&data, &ctx())),
        "an attestation older than the staleness window must be Ignored"
    );
}

#[test]
fn attestation_fresh_accepted() {
    let data = make_attestation(WALL);
    assert!(
        is_accept(att_d(&data, &ctx())),
        "a current-slot attestation must be Accepted"
    );
}

#[test]
fn attestation_boundary_accepted() {
    let data = make_attestation(WALL - ATTEST_STALE_SLOTS);
    assert!(
        is_accept(att_d(&data, &ctx())),
        "attestation exactly at the threshold boundary must be Accepted (inclusive)"
    );
}

// REQ-GRV-SEC-004 / F-4 — Decision: a failure means undecodable attestation bytes are forwarded (fail-open Accept) or penalised (Reject).
#[test]
fn attestation_garbage_ignored_never_rejected() {
    let v = att_d(b"not-an-attestation", &ctx());
    assert!(
        is_ignore(v),
        "undecodable bytes: Ignore (not forwarded, never Reject)"
    );
}

#[test]
fn attestation_genesis_zero_fails_open() {
    let data = make_attestation(0);
    let mut c = ctx();
    c.genesis_time = 0;
    assert!(
        is_accept(att_d(&data, &c)),
        "genesis_time=0 must fail open to Accept (staleness disabled)"
    );
}

/// A genuinely-new attestation at age 90s (distinct identity, never seen) must
/// still forward — identity dedup must not suppress first sights.
#[test]
fn attestation_new_at_age_90s_accepted() {
    let data = make_attestation_id(WALL - 9, 0, Hash::from_bytes([9u8; 32]));
    assert!(
        is_accept(att_d(&data, &ctx())),
        "a genuinely-new late attestation (age 90s) must be Accepted"
    );
}

// ── M1 (D3): headers and heartbeats are unsubscribed — every input is Ignored ──

fn make_heartbeat(slot: u32) -> Vec<u8> {
    make_heartbeat_sig(slot, PublicKey::from_bytes([0u8; 32]), 0)
}

fn make_header_bytes(slot: u32) -> Vec<u8> {
    BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot,
        producer: PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof { pi: vec![] },
        missed_producers: vec![],
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    }
    .serialize()
}

/// First sight and re-delivery, on both unconsumed topics.
fn assert_unconsumed(data: &[u8], genesis_time: u64, row: &str) {
    for sight in ["first", "again"] {
        for topic in [RelayTopic::Heartbeats, RelayTopic::Headers] {
            let mut c = ctx();
            c.genesis_time = genesis_time;
            let d = relay_decision(topic, data, &c);
            assert!(
                matches!(d, RelayDecision::Ignore),
                "{topic:?} {row} ({sight} sight) must be Ignore"
            );
        }
    }
}

// REQ-GRV-015 — Decision: a failure means a stale heartbeat is relayed or processed on a topic with no consumer.
#[test]
fn heartbeat_stale_ignored() {
    assert_unconsumed(&make_heartbeat(WALL - 7), GEN, "stale heartbeat");
}

// REQ-GRV-015 — Decision: a failure means a fresh heartbeat is still relayed or processed although no node consumes heartbeats.
#[test]
fn heartbeat_fresh_ignored_unsubscribed() {
    assert_unconsumed(&make_heartbeat(WALL), GEN, "fresh heartbeat");
}

// REQ-GRV-015 — Decision: a failure means the old age-window boundary still admits a heartbeat.
#[test]
fn heartbeat_boundary_ignored_unsubscribed() {
    assert_unconsumed(&make_heartbeat(WALL - 6), GEN, "boundary heartbeat");
}

// REQ-GRV-015 / REQ-GRV-SEC-004 — Decision: a failure means undecodable heartbeat bytes fail open to Accept or are Rejected.
#[test]
fn heartbeat_garbage_ignored() {
    assert_unconsumed(b"not-a-heartbeat", GEN, "garbage heartbeat");
}

// REQ-GRV-015 — Decision: a failure means an unset clock re-opens the heartbeat fail-open Accept.
#[test]
fn heartbeat_genesis_zero_ignored() {
    assert_unconsumed(&make_heartbeat(0), 0, "heartbeat, genesis_time 0");
}

// REQ-GRV-015 — Decision: a failure means a heartbeat re-delivery is relayed (INC-I-142 storm shape on an unconsumed topic).
#[test]
fn heartbeat_redelivery_within_ttl_ignored() {
    assert_unconsumed(&make_heartbeat(WALL - 1), GEN, "heartbeat re-delivery");
}

// REQ-GRV-015 — Decision: a failure means a heartbeat with a new identity is still relayed.
#[test]
fn heartbeat_new_identity_ignored() {
    assert_unconsumed(
        &make_heartbeat_sig(WALL, PublicKey::from_bytes([7u8; 32]), 0),
        GEN,
        "new heartbeat identity",
    );
}

// REQ-GRV-015 — Decision: a failure means a fresh header is still relayed or processed although no node consumes headers.
#[test]
fn header_fresh_ignored_unsubscribed() {
    assert_unconsumed(&make_header_bytes(WALL), GEN, "fresh header");
}

// REQ-GRV-015 — Decision: a failure means a stale header is relayed or processed.
#[test]
fn header_stale_ignored() {
    assert_unconsumed(&make_header_bytes(WALL - 7), GEN, "stale header");
}

// REQ-GRV-015 / F-2 — Decision: a failure means a header re-delivery is relayed and can drain a peer's block budget (C-3).
#[test]
fn header_redelivery_ignored() {
    assert_unconsumed(&make_header_bytes(WALL - 1), GEN, "header re-delivery");
}

// INV-NETWORK-002 / REQ-GRV-SEC-004 — Decision: a failure means header bytes on the headers topic are Rejected (P4 penalty, mesh-expulsion cascade P0-001) instead of Ignored.
#[test]
fn header_bytes_never_rejected_p0_001() {
    let data = make_header_bytes(WALL);
    assert!(
        Block::deserialize(&data).is_none(),
        "precondition: header bytes MUST fail Block::deserialize (else the test is vacuous)"
    );
    let v = relay_decision(RelayTopic::Headers, &data, &ctx());
    assert!(
        matches!(v, RelayDecision::Ignore),
        "header bytes on the headers topic must be Ignored, never Rejected (P0-001)"
    );
}

// REQ-GRV-015 / REQ-GRV-SEC-004 — Decision: a failure means undecodable header bytes fail open to Accept or are Rejected.
#[test]
fn header_garbage_ignored() {
    assert_unconsumed(b"not-a-header", GEN, "garbage header");
}

// REQ-GRV-015 — Decision: a failure means an unset clock re-opens the header fail-open Accept.
#[test]
fn header_genesis_zero_ignored() {
    assert_unconsumed(&make_header_bytes(0), 0, "header, genesis_time 0");
}

// ── M4: votes ─────────────────────────────────────────────────────────
#[test]
fn vote_fresh_accepted() {
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", NOW);
    assert!(is_accept(vote_d(&data, &ctx())));
}

#[test]
fn vote_new_identity_accepted() {
    let a = make_vote_json("prod-a", "v6.24.0", "Veto", NOW);
    let b = make_vote_json("prod-b", "v6.24.0", "Veto", NOW);
    assert!(is_accept(vote_d(&a, &ctx())));
    assert!(
        is_accept(vote_d(&b, &ctx())),
        "a distinct producer_id is a new identity → Accept"
    );
}

/// STORM-CLOSURE-adjacent: SECONDARY age bound drops truly-ancient votes.
/// FAILS against the Accept stub.
#[test]
fn vote_stale_ignored() {
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", 1);
    let mut c = ctx();
    c.now_unix = VOTE_MAX_AGE_SECS + 1000;
    assert!(
        is_ignore(vote_d(&data, &c)),
        "a vote older than VOTE_MAX_AGE_SECS must be Ignored"
    );
}

#[test]
fn vote_age_boundary_accepted() {
    let ts = 5000u64;
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", ts);
    let mut c = ctx();
    c.now_unix = ts + VOTE_MAX_AGE_SECS; // exactly at boundary, strict > → Accept
    assert!(
        is_accept(vote_d(&data, &c)),
        "a vote exactly at the age boundary must be Accepted (inclusive)"
    );
}

#[test]
fn vote_timestamp_zero_fails_open() {
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", 0);
    assert!(
        is_accept(vote_d(&data, &ctx())),
        "timestamp==0 must fail open to Accept (cannot judge age)"
    );
}

/// F3 drift guard: the canonical VoteMessage JSON wire shape (camelCase
/// `producerId` alias + `Approve`/`Veto` variant names, per updater/src/vote.rs
/// serialized by rpc governance) must decode and classify as a fresh Accept. A
/// future wire/field/variant rename that breaks the local VoteIdentity mirror
/// trips this test. (Full cross-crate parity guard is deferred.)
#[test]
fn vote_canonical_wire_shape_decodes() {
    let camel = format!(
            r#"{{"version":"v6.24.0","vote":"Approve","producerId":"prod-a","timestamp":{NOW},"signature":"00"}}"#
        )
        .into_bytes();
    assert!(
        is_accept(vote_d(&camel, &ctx())),
        "canonical camelCase producerId + Approve variant must decode → Accept"
    );
}

// REQ-GRV-SEC-004 / F-4 — Decision: a failure means undecodable vote JSON is forwarded (fail-open Accept) or penalised (Reject).
#[test]
fn vote_bad_json_ignored_never_rejected() {
    assert!(is_ignore(vote_d(b"not-json", &ctx())), "first sight");
    assert!(is_ignore(vote_d(b"not-json", &ctx())), "re-delivery");
}

#[test]
#[allow(clippy::assertions_on_constants)] // compile-time invariant guard
fn vote_max_age_exceeds_governance_window() {
    // Must safely exceed the largest veto window (7-day target) so a vote
    // still inside its veto window is never dropped as stale.
    assert!(VOTE_MAX_AGE_SECS >= 7 * 24 * 60 * 60);
}

// ── M5: transactions ──────────────────────────────────────────────────
#[test]
fn transaction_distinct_txids_accepted() {
    let a = encode_tx_batch(&[make_tx(1)]);
    let b = encode_tx_batch(&[make_tx(2)]);
    assert!(is_accept(tx_d(&a, &ctx())));
    assert!(
        is_accept(tx_d(&b, &ctx())),
        "a distinct txid is a new identity → Accept"
    );
}

/// A batch containing at least one NEW txid must forward (never drop a first
/// sight). The re-included, already-seen txid does NOT suppress the batch.
#[test]
fn transaction_batch_with_one_new_tx_accepted() {
    let seed_batch = encode_tx_batch(&[make_tx(20)]);
    assert!(is_accept(tx_d(&seed_batch, &ctx())));
    let mixed = encode_tx_batch(&[make_tx(20), make_tx(21)]);
    assert!(
        is_accept(tx_d(&mixed, &ctx())),
        "a batch with at least one new txid must forward (first sight preserved)"
    );
    let again = encode_tx_batch(&[make_tx(20), make_tx(21)]);
    assert!(
        is_accept(tx_d(&again, &ctx())),
        "R6: the vendor cache drops re-deliveries; one that reaches the app is judged as first sight"
    );
}

// REQ-GRV-SEC-004 / F-4 — Decision: a failure means empty or undecodable tx bytes are processed or penalised instead of Ignored.
#[test]
fn transaction_garbage_ignored_never_rejected() {
    assert!(is_ignore(tx_d(b"", &ctx())), "empty");
    assert!(is_ignore(tx_d(b"\xde\xad", &ctx())), "garbage");
}

// ── INC-I-142 M6: heartbeat suppression-resistance (unsubscribed topic) ──

fn make_heartbeat_sig(slot: u32, producer: PublicKey, sig_byte: u8) -> Vec<u8> {
    Heartbeat {
        version: 1,
        producer,
        slot,
        prev_block_hash: Hash::ZERO,
        vdf_output: [0u8; 32],
        signature: Signature::from_bytes([sig_byte; 64]),
        witnesses: Vec::new(),
    }
    .serialize()
}

// REQ-GRV-015 / REQ-GRV-SEC-002 — Decision: a failure means a forged or genuine heartbeat variant is relayed on the unsubscribed topic.
#[test]
fn heartbeat_forged_variant_does_not_suppress_genuine() {
    let victim = *KeyPair::generate().public_key();
    assert_unconsumed(
        &make_heartbeat_sig(WALL, victim, 0xAA),
        GEN,
        "forged heartbeat",
    );
    assert_unconsumed(
        &make_heartbeat_sig(WALL, victim, 0x00),
        GEN,
        "genuine heartbeat",
    );
}
