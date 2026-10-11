use super::super::{encode_tx_announce, encode_tx_batch};
use super::*;
use crate::gossip::{
    relay_decision, RelayDecision, RelayTopic, ATTESTATION_TOPIC, BLOCKS_TOPIC, PRODUCERS_TOPIC,
    TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use crypto::{Hash, KeyPair, PublicKey, Signature};
use doli_core::{Block, BlockHeader, Heartbeat};
use libp2p::gossipsub::MessageAcceptance;

// INC-I-237 M1: every classifier below is reached through the one relay table.
fn att_d(data: &[u8], c: &mut StalenessCtx<'_>) -> MessageAcceptance {
    relay_decision(RelayTopic::Attestations, data, c)
        .acceptance()
        .expect("attestations are swarm-judged until M3")
}

fn vote_d(data: &[u8], c: &mut StalenessCtx<'_>) -> MessageAcceptance {
    relay_decision(RelayTopic::Votes, data, c)
        .acceptance()
        .expect("votes are swarm-judged until M3")
}

/// INC-I-237 M2 (D1): a first-sight full tx message is Deferred to the node
/// verdict, a 0xAA announce stays LocalOnly; both map to Accept ("processed").
fn tx_d(data: &[u8], c: &mut StalenessCtx<'_>) -> MessageAcceptance {
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

fn make_seen() -> SeenCache {
    SeenCache::new(SEEN_CACHE_TTL_SECS, SEEN_CACHE_CAPACITY)
}

fn ctx(seen: &mut SeenCache) -> StalenessCtx<'_> {
    StalenessCtx {
        now_unix: NOW,
        genesis_time: GEN,
        slot_duration: DUR,
        best_slot: WALL,
        seen,
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

// ── M0: generalized SeenCache ─────────────────────────────────────────

#[test]
fn seen_cache_dedups_within_ttl_and_reaccepts_after() {
    let mut c = SeenCache::new(100, 4);
    let k = [7u8; 32];
    assert!(!c.check_and_insert(k, 1000), "first sight is new");
    assert!(
        c.check_and_insert(k, 1050),
        "re-sight within TTL is a duplicate"
    );
    assert!(
        !c.check_and_insert(k, 1200),
        "re-sight after TTL is fresh again (rebroadcast preserved)"
    );
}

#[test]
fn seen_cache_distinct_keys_all_accepted() {
    let mut c = make_seen();
    for i in 0..5u8 {
        assert!(
            !c.check_and_insert([i; 32], NOW),
            "each distinct identity key must be treated as new"
        );
    }
}

#[test]
fn seen_cache_evicts_oldest_past_capacity() {
    // Capacity 2: inserting a 3rd key drops the oldest, so the oldest is
    // treated as new again on re-sight (bounded, drop-oldest).
    let mut c = SeenCache::new(1000, 2);
    assert!(!c.check_and_insert([1; 32], 100));
    assert!(!c.check_and_insert([2; 32], 100));
    assert!(!c.check_and_insert([3; 32], 100)); // evicts key 1
    assert!(
        !c.check_and_insert([1; 32], 100),
        "key evicted past capacity must be seen as new again"
    );
    assert!(
        c.check_and_insert([3; 32], 100),
        "a recently-inserted, non-evicted key is still a duplicate"
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
    let mut seen = make_seen();
    assert!(
        is_ignore(att_d(&data, &mut ctx(&mut seen))),
        "an attestation older than the staleness window must be Ignored"
    );
}

#[test]
fn attestation_fresh_accepted() {
    let data = make_attestation(WALL);
    let mut seen = make_seen();
    assert!(
        is_accept(att_d(&data, &mut ctx(&mut seen))),
        "a current-slot attestation must be Accepted"
    );
}

#[test]
fn attestation_boundary_accepted() {
    let data = make_attestation(WALL - ATTEST_STALE_SLOTS);
    let mut seen = make_seen();
    assert!(
        is_accept(att_d(&data, &mut ctx(&mut seen))),
        "attestation exactly at the threshold boundary must be Accepted (inclusive)"
    );
}

// REQ-GRV-SEC-004 / F-4 — Decision: a failure means undecodable attestation bytes are forwarded (fail-open Accept) or penalised (Reject).
#[test]
fn attestation_garbage_ignored_never_rejected() {
    let mut seen = make_seen();
    let v = att_d(b"not-an-attestation", &mut ctx(&mut seen));
    assert!(
        is_ignore(v),
        "undecodable bytes: Ignore (not forwarded, never Reject)"
    );
}

#[test]
fn attestation_genesis_zero_fails_open() {
    let data = make_attestation(0);
    let mut seen = make_seen();
    let mut c = ctx(&mut seen);
    c.genesis_time = 0;
    assert!(
        is_accept(att_d(&data, &mut c)),
        "genesis_time=0 must fail open to Accept (staleness disabled)"
    );
}

/// STORM-CLOSURE (M1). FAILS against the age-only M1 (both sights Accept at
/// age 90s, inside the 120s window); PASSES with identity dedup.
#[test]
fn attestation_redelivery_within_ttl_ignored() {
    let data = make_attestation(WALL - 9); // age 90s — inside the age window
    let mut seen = make_seen();
    assert!(
        is_accept(att_d(&data, &mut ctx(&mut seen))),
        "first sight of a new attestation must forward (Accept)"
    );
    assert!(
        is_ignore(att_d(&data, &mut ctx(&mut seen))),
        "a re-delivered identical (attester, block_hash) attestation within TTL must Ignore"
    );
}

/// A genuinely-new attestation at age 90s (distinct identity, never seen) must
/// still forward — identity dedup must not suppress first sights.
#[test]
fn attestation_new_at_age_90s_accepted() {
    let data = make_attestation_id(WALL - 9, 0, Hash::from_bytes([9u8; 32]));
    let mut seen = make_seen();
    assert!(
        is_accept(att_d(&data, &mut ctx(&mut seen))),
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
    let mut seen = make_seen();
    for sight in ["first", "again"] {
        for topic in [RelayTopic::Heartbeats, RelayTopic::Headers] {
            let mut c = ctx(&mut seen);
            c.genesis_time = genesis_time;
            let d = relay_decision(topic, data, &mut c);
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
    let mut seen = make_seen();
    let v = relay_decision(RelayTopic::Headers, &data, &mut ctx(&mut seen));
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
    let mut seen = make_seen();
    assert!(is_accept(vote_d(&data, &mut ctx(&mut seen))));
}

/// STORM-CLOSURE (M4). FAILS against the Accept stub (both sights Accept);
/// PASSES with identity dedup on (producer_id, version, vote).
#[test]
fn vote_redelivery_within_ttl_ignored() {
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", NOW);
    let mut seen = make_seen();
    assert!(
        is_accept(vote_d(&data, &mut ctx(&mut seen))),
        "first sight of a new vote must forward (Accept)"
    );
    assert!(
        is_ignore(vote_d(&data, &mut ctx(&mut seen))),
        "a re-delivered identical (producer,version,vote) within TTL must Ignore"
    );
}

#[test]
fn vote_new_identity_accepted() {
    let mut seen = make_seen();
    let a = make_vote_json("prod-a", "v6.24.0", "Veto", NOW);
    let b = make_vote_json("prod-b", "v6.24.0", "Veto", NOW);
    assert!(is_accept(vote_d(&a, &mut ctx(&mut seen))));
    assert!(
        is_accept(vote_d(&b, &mut ctx(&mut seen))),
        "a distinct producer_id is a new identity → Accept"
    );
}

/// STORM-CLOSURE-adjacent: SECONDARY age bound drops truly-ancient votes.
/// FAILS against the Accept stub.
#[test]
fn vote_stale_ignored() {
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", 1);
    let mut seen = make_seen();
    let mut c = ctx(&mut seen);
    c.now_unix = VOTE_MAX_AGE_SECS + 1000;
    assert!(
        is_ignore(vote_d(&data, &mut c)),
        "a vote older than VOTE_MAX_AGE_SECS must be Ignored"
    );
}

#[test]
fn vote_age_boundary_accepted() {
    let ts = 5000u64;
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", ts);
    let mut seen = make_seen();
    let mut c = ctx(&mut seen);
    c.now_unix = ts + VOTE_MAX_AGE_SECS; // exactly at boundary, strict > → Accept
    assert!(
        is_accept(vote_d(&data, &mut c)),
        "a vote exactly at the age boundary must be Accepted (inclusive)"
    );
}

#[test]
fn vote_timestamp_zero_fails_open() {
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", 0);
    let mut seen = make_seen();
    assert!(
        is_accept(vote_d(&data, &mut ctx(&mut seen))),
        "timestamp==0 must fail open to Accept (cannot judge age)"
    );
}

/// F2 regression: a re-delivered timestamp==0 vote is deduped (recorded on the
/// fail-open first Accept), closing the re-forward residual.
#[test]
fn vote_timestamp_zero_redelivery_ignored() {
    let data = make_vote_json("prod-a", "v6.24.0", "Veto", 0);
    let mut seen = make_seen();
    assert!(
        is_accept(vote_d(&data, &mut ctx(&mut seen))),
        "first sight of a timestamp==0 vote fails open to Accept"
    );
    assert!(
        is_ignore(vote_d(&data, &mut ctx(&mut seen))),
        "a re-delivered identical timestamp==0 vote must Ignore (F2 residual closed)"
    );
}

/// F3 drift guard: the canonical VoteMessage JSON wire shape (camelCase
/// `producerId` alias + `Approve`/`Veto` variant names, per updater/src/vote.rs
/// serialized by rpc governance) must decode and classify as a fresh Accept. A
/// future wire/field/variant rename that breaks the local VoteIdentity mirror
/// trips this test. (Full cross-crate parity guard is deferred.)
#[test]
fn vote_canonical_wire_shape_decodes() {
    let mut seen = make_seen();
    let camel = format!(
            r#"{{"version":"v6.24.0","vote":"Approve","producerId":"prod-a","timestamp":{NOW},"signature":"00"}}"#
        )
        .into_bytes();
    assert!(
        is_accept(vote_d(&camel, &mut ctx(&mut seen))),
        "canonical camelCase producerId + Approve variant must decode → Accept"
    );
}

// REQ-GRV-SEC-004 / F-4 — Decision: a failure means undecodable vote JSON is forwarded (fail-open Accept) or penalised (Reject).
#[test]
fn vote_bad_json_ignored_never_rejected() {
    let mut seen = make_seen();
    assert!(
        is_ignore(vote_d(b"not-json", &mut ctx(&mut seen))),
        "first sight"
    );
    assert!(
        is_ignore(vote_d(b"not-json", &mut ctx(&mut seen))),
        "re-delivery"
    );
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
    let mut seen = make_seen();
    let a = encode_tx_batch(&[make_tx(1)]);
    let b = encode_tx_batch(&[make_tx(2)]);
    assert!(is_accept(tx_d(&a, &mut ctx(&mut seen))));
    assert!(
        is_accept(tx_d(&b, &mut ctx(&mut seen))),
        "a distinct txid is a new identity → Accept"
    );
}

/// STORM-CLOSURE (M5). FAILS against the Accept stub; PASSES with identity
/// dedup on tx.hash().
#[test]
fn transaction_redelivery_within_ttl_ignored() {
    let data = encode_tx_batch(&[make_tx(7)]);
    let mut seen = make_seen();
    assert!(
        is_accept(tx_d(&data, &mut ctx(&mut seen))),
        "first sight of a new txid must forward (Accept)"
    );
    assert!(
        is_ignore(tx_d(&data, &mut ctx(&mut seen))),
        "a re-delivered identical txid within TTL must Ignore"
    );
}

/// STORM-CLOSURE (M5, Announce). FAILS against the Accept stub.
#[test]
fn transaction_announce_redelivery_ignored() {
    let h = make_tx(9).hash();
    let data = encode_tx_announce(&[h]);
    let mut seen = make_seen();
    assert!(
        is_accept(tx_d(&data, &mut ctx(&mut seen))),
        "first sight of an announced hash must forward"
    );
    assert!(
        is_ignore(tx_d(&data, &mut ctx(&mut seen))),
        "a re-announced identical hash within TTL must Ignore"
    );
}

/// Legitimate rebroadcast preserved: a txid re-seen AFTER TTL expiry re-Accepts.
#[test]
fn transaction_reaccepted_after_ttl_expiry() {
    let data = encode_tx_batch(&[make_tx(11)]);
    let mut seen = make_seen();
    {
        let mut c = ctx(&mut seen);
        assert!(is_accept(tx_d(&data, &mut c)));
    }
    {
        let mut c = ctx(&mut seen);
        c.now_unix = NOW + TX_SEEN_TTL_SECS + 1;
        assert!(
            is_accept(tx_d(&data, &mut c)),
            "a txid re-seen after TTL expiry must re-Accept (legit rebroadcast preserved)"
        );
    }
}

/// A batch containing at least one NEW txid must forward (never drop a first
/// sight); a batch whose every txid was already seen must Ignore. The
/// re-included, already-seen txid does NOT suppress the batch.
#[test]
fn transaction_batch_with_one_new_tx_accepted() {
    let mut seen = make_seen();
    let seed_batch = encode_tx_batch(&[make_tx(20)]);
    assert!(is_accept(tx_d(&seed_batch, &mut ctx(&mut seen))));
    let mixed = encode_tx_batch(&[make_tx(20), make_tx(21)]);
    assert!(
        is_accept(tx_d(&mixed, &mut ctx(&mut seen))),
        "a batch with at least one new txid must forward (first sight preserved)"
    );
    let again = encode_tx_batch(&[make_tx(20), make_tx(21)]);
    assert!(
        is_ignore(tx_d(&again, &mut ctx(&mut seen))),
        "a batch whose every txid was already seen must Ignore"
    );
}

// REQ-GRV-SEC-004 / F-4 — Decision: a failure means empty or undecodable tx bytes are processed or penalised instead of Ignored.
#[test]
fn transaction_garbage_ignored_never_rejected() {
    let mut seen = make_seen();
    assert!(is_ignore(tx_d(b"", &mut ctx(&mut seen))), "empty");
    assert!(is_ignore(tx_d(b"\xde\xad", &mut ctx(&mut seen))), "garbage");
}

#[test]
#[allow(clippy::assertions_on_constants)] // compile-time invariant guard
fn tx_seen_ttl_satisfies_f1_rebroadcast_invariant() {
    // 120s ≤ TX_SEEN_TTL < mempool rebroadcast interval. DOLI has NO mempool
    // rebroadcast loop (interval = ∞), so the shared 180s TTL satisfies it.
    assert!(TX_SEEN_TTL_SECS >= 120);
    assert_eq!(TX_SEEN_TTL_SECS, SEEN_CACHE_TTL_SECS);
}

// ── INC-I-142 M6 SEC-LOGIC-001/002/003: suppression-resistance ─────────
//
// A forged message carrying a VICTIM's exact SEMANTIC identity fields but
// DIFFERENT raw bytes (e.g. a flipped signature byte — signatures are NOT
// checked at the gossip gate and are EXCLUDED from tx.hash()) MUST NOT be able
// to pre-seed the SeenCache key of the genuine message. With the pre-fix
// SEMANTIC identity keys these tests FAIL (the forgery shares the victim's key
// → genuine is Ignored/suppressed for the TTL). With the raw-bytes identity key
// `blake3(topic || raw_data)` they PASS (different bytes → different key).

// `attester` MUST be a valid Ed25519 curve point (PublicKey deserialize validates
// it, keys.rs:90) or `Attestation::from_bytes` fails open and the semantic-key
// collision the test targets never triggers — hence a generated victim key.
fn make_attestation_sig(slot: u32, attester: PublicKey, block_hash: Hash, sig_byte: u8) -> Vec<u8> {
    Attestation {
        block_hash,
        slot,
        height: 0,
        attester,
        attester_weight: 0,
        signature: Signature::from_bytes([sig_byte; 64]),
        bls_signature: Vec::new(),
    }
    .to_bytes()
}

#[test]
fn attestation_forged_variant_does_not_suppress_genuine() {
    let mut seen = make_seen();
    let victim = *KeyPair::generate().public_key();
    let bh = Hash::from_bytes([3u8; 32]);
    let forged = make_attestation_sig(WALL, victim, bh, 0xAA); // victim identity, garbage sig
    let genuine = make_attestation_sig(WALL, victim, bh, 0x00); // same (attester,block_hash), real sig
    assert!(
        is_accept(att_d(&forged, &mut ctx(&mut seen))),
        "forged first-sight is Accepted + recorded"
    );
    assert!(
        is_accept(att_d(&genuine, &mut ctx(&mut seen))),
        "genuine attestation with same (attester,block_hash) but different bytes must STILL be \
             Accepted - a forgery must not pre-seed its identity key (SEC-LOGIC-001)"
    );
}

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

fn make_vote_json_sig(
    producer: &str,
    version: &str,
    vote: &str,
    timestamp: u64,
    sig: &str,
) -> Vec<u8> {
    format!(
            r#"{{"version":"{version}","vote":"{vote}","producer_id":"{producer}","timestamp":{timestamp},"signature":"{sig}"}}"#
        )
        .into_bytes()
}

#[test]
fn vote_forged_variant_does_not_suppress_genuine() {
    let mut seen = make_seen();
    // Same (producer_id, version, vote) - the pre-fix vote identity - different sig field.
    let forged = make_vote_json_sig("prod-a", "v6.24.0", "Veto", NOW, "ff");
    let genuine = make_vote_json_sig("prod-a", "v6.24.0", "Veto", NOW, "00");
    assert!(is_accept(vote_d(&forged, &mut ctx(&mut seen))));
    assert!(
        is_accept(vote_d(&genuine, &mut ctx(&mut seen))),
        "genuine vote with same (producer_id,version,vote) but different bytes must STILL be \
             Accepted (SEC-LOGIC-002 governance suppression)"
    );
}

// Same txid (signatures are EXCLUDED from tx.hash(), core.rs:491) but different
// serialized bytes - the exact SegWit-style gap the pre-fix per-txid key exposed.
fn make_tx_sig(n: u64, sig_byte: u8) -> doli_core::Transaction {
    let mut tx = doli_core::Transaction::new_transfer(
        vec![doli_core::Input::new(Hash::from_bytes([n as u8; 32]), 0)],
        vec![doli_core::Output::normal(100 + n, Hash::ZERO)],
    );
    tx.inputs[0].signature = Signature::from_bytes([sig_byte; 64]);
    tx
}

#[test]
fn transaction_forged_variant_does_not_suppress_genuine() {
    let mut seen = make_seen();
    let forged_tx = make_tx_sig(7, 0xAA);
    let genuine_tx = make_tx_sig(7, 0x00);
    assert_eq!(
        forged_tx.hash(),
        genuine_tx.hash(),
        "precondition: signatures are excluded from txid -> same hash (else test is vacuous)"
    );
    let forged = encode_tx_batch(&[forged_tx]);
    let genuine = encode_tx_batch(&[genuine_tx]);
    assert!(is_accept(tx_d(&forged, &mut ctx(&mut seen))));
    assert!(
        is_accept(tx_d(&genuine, &mut ctx(&mut seen))),
        "genuine tx batch with the same txid but different bytes must STILL be Accepted - a \
             forged same-txid variant must not pre-seed its key (SEC-LOGIC-001 tx censorship)"
    );
}

// BLS-vs-non-BLS no-collision (SEC-CONSENSUS-003): two DIFFERENT valid
// attestations sharing (attester, block_hash) - one with a bls_signature, one
// without - must not collide (the pre-fix key omitted bls_signature). Raw-bytes
// keying distinguishes them for free.
#[test]
fn attestation_bls_and_non_bls_do_not_collide() {
    let mut seen = make_seen();
    let attester = *KeyPair::generate().public_key();
    let bh = Hash::from_bytes([4u8; 32]);
    let non_bls = make_attestation_sig(WALL, attester, bh, 0x00);
    let bls = Attestation {
        block_hash: bh,
        slot: WALL,
        height: 0,
        attester,
        attester_weight: 0,
        signature: Signature::from_bytes([0u8; 64]),
        bls_signature: vec![1u8; 48],
    }
    .to_bytes();
    assert!(is_accept(att_d(&non_bls, &mut ctx(&mut seen))));
    assert!(
        is_accept(att_d(&bls, &mut ctx(&mut seen))),
        "a BLS-aggregate attestation for the same (attester,block_hash) must not be suppressed \
             by a prior non-BLS one (SEC-CONSENSUS-003)"
    );
}
