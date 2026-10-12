//! INC-I-237 M1 (D2) — the one relay-decision table `relay_decision`.
//!
//! OUTPUT CONTRACT: fn relay_decision(topic: RelayTopic, data: &[u8],
//!   ctx: &StalenessCtx) -> RelayDecision; fn RelayDecision::acceptance(&self);
//!   fn RelayTopic::from_topic_str(&str) -> RelayTopic
//!   O1: none (M4 R6: the vendor duplicate cache, not the app, drops re-deliveries)
//!   O3: RelayDecision variant (+ the decoded block inside AcceptBlock);
//!       MessageAcceptance reported to gossipsub; RelayTopic
//! PATHS: blocks {undecodable, stale, boundary, fresh, clock unset, equivocating
//!   pair}; txs {legacy, 0xBA, 0xAA, re-delivery, undecodable}; attestations /
//!   votes {fresh, re-delivery, stale, boundary, undecodable, clock unset};
//!   producers {set, digest, legacy bincode, all-stale, undecodable, clock unset};
//!   headers / heartbeats / tier1 / region / unknown {any bytes}
//! INPUT PARTITIONS: every RelayTopic x {valid current format, undecodable,
//!   adversarial (injection strings, ANSI, 1 MiB)}; topic strings {exact const,
//!   region_topic(n), look-alike}
//! MATRIX: O3 x every path above (re-delivery = first-sight verdict); acceptance() x variant.

use super::inc_i_237_m0_corpus_vectors as v;
use crypto::{Hash, PublicKey, Signature};
use doli_core::{Attestation, Block, BlockHeader};
use libp2p::gossipsub::MessageAcceptance;
use network::gossip::staleness::{StalenessCtx, ATTEST_STALE_SLOTS, VOTE_MAX_AGE_SECS};
use network::gossip::{
    region_topic, relay_decision, RelayDecision, RelayTopic, ATTESTATION_TOPIC, BLOCKS_TOPIC,
    PRODUCERS_TOPIC, PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS, STALE_BLOCK_SLOT_THRESHOLD,
    TIER1_BLOCKS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

const WALL: u32 = 1_000;

const ALL: [RelayTopic; 10] = [
    RelayTopic::Blocks,
    RelayTopic::Transactions,
    RelayTopic::Producers,
    RelayTopic::Votes,
    RelayTopic::Attestations,
    RelayTopic::Headers,
    RelayTopic::Heartbeats,
    RelayTopic::Tier1Blocks,
    RelayTopic::RegionBlocks,
    RelayTopic::Unknown,
];

fn variant_name(t: RelayTopic) -> &'static str {
    match t {
        RelayTopic::Blocks => "Blocks",
        RelayTopic::Transactions => "Transactions",
        RelayTopic::Producers => "Producers",
        RelayTopic::Votes => "Votes",
        RelayTopic::Attestations => "Attestations",
        RelayTopic::Headers => "Headers",
        RelayTopic::Heartbeats => "Heartbeats",
        RelayTopic::Tier1Blocks => "Tier1Blocks",
        RelayTopic::RegionBlocks => "RegionBlocks",
        RelayTopic::Unknown => "Unknown",
    }
}

fn decide_at(topic: RelayTopic, data: &[u8], genesis_time: u64, now_unix: u64) -> RelayDecision {
    let ctx = StalenessCtx {
        now_unix,
        genesis_time,
        slot_duration: v::SLOT_DURATION,
        best_slot: WALL,
    };
    relay_decision(topic, data, &ctx)
}

fn decide(topic: RelayTopic, data: &[u8]) -> RelayDecision {
    decide_at(topic, data, v::GENESIS_TIME, v::NOW_UNIX)
}

/// INC-I-237 M3: by Debug name, so the test compiles with or without a variant.
fn name(d: &RelayDecision) -> &'static str {
    match format!("{d:?}").split('(').next().unwrap_or_default() {
        "Reject" => "Reject",
        "Ignore" => "Ignore",
        "AcceptBlock" => "AcceptBlock",
        "LocalOnly" => "LocalOnly",
        "Defer" => "Defer",
        _ => "other",
    }
}

/// INC-I-237 M2: `None` = no swarm report (Defer waits for the node verdict).
fn reported(d: &RelayDecision) -> &'static str {
    match d.acceptance() {
        Some(MessageAcceptance::Accept) => "Accept",
        Some(MessageAcceptance::Ignore) => "Ignore",
        Some(MessageAcceptance::Reject) => "Reject",
        None => "none",
    }
}

fn block(slot: u32, merkle_tag: u8) -> Block {
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::from_bytes([merkle_tag; 32]),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: v::NOW_UNIX,
        slot,
        producer: PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

fn attestation(slot: u32) -> Vec<u8> {
    Attestation {
        block_hash: Hash::ZERO,
        slot,
        height: 0,
        attester: PublicKey::from_bytes([0u8; 32]),
        attester_weight: 0,
        signature: Signature::from_bytes([0u8; 64]),
        bls_signature: Vec::new(),
    }
    .to_bytes()
}

fn vote(timestamp: u64) -> Vec<u8> {
    format!(
        "{{\"version\":\"6.43.3\",\"vote\":\"approve\",\"producerId\":\"{}\",\"timestamp\":{timestamp},\"signature\":\"bb\"}}",
        v::PK9
    )
    .into_bytes()
}

fn assert_row(topic: RelayTopic, data: &[u8], want: &str, row: &str) {
    let d = decide(topic, data);
    assert_eq!(name(&d), want, "{row}");
}

// REQ-GRV-009 — Decision: a failure means relay_decision stopped being a plain sync fn, so an await entered the block forward path.
#[test]
fn req_grv_009_relay_decision_is_a_sync_fn() {
    let f: fn(RelayTopic, &[u8], &StalenessCtx) -> RelayDecision = relay_decision;
    let ctx = StalenessCtx {
        now_unix: v::NOW_UNIX,
        genesis_time: v::GENESIS_TIME,
        slot_duration: v::SLOT_DURATION,
        best_slot: WALL,
    };
    assert_eq!(name(&f(RelayTopic::Blocks, &[], &ctx)), "Reject");
}

// REQ-GRV-003 — Decision: a failure means a topic string maps to the wrong table row, so a look-alike topic could borrow the block pre-verdict exception or a real topic loses its rule.
#[test]
fn req_grv_003_from_topic_str_maps_every_topic_and_nothing_else() {
    let exact = [
        (BLOCKS_TOPIC, RelayTopic::Blocks),
        (TRANSACTIONS_TOPIC, RelayTopic::Transactions),
        (PRODUCERS_TOPIC, RelayTopic::Producers),
        (VOTES_TOPIC, RelayTopic::Votes),
        (ATTESTATION_TOPIC, RelayTopic::Attestations),
        ("/doli/headers/1", RelayTopic::Headers),
        ("/doli/heartbeats/1", RelayTopic::Heartbeats),
        (TIER1_BLOCKS_TOPIC, RelayTopic::Tier1Blocks),
    ];
    for (s, t) in exact {
        assert_eq!(RelayTopic::from_topic_str(s), t, "{s}");
    }
    for n in [0u32, 1, 7, u32::MAX] {
        let s = region_topic(n);
        assert_eq!(
            RelayTopic::from_topic_str(&s),
            RelayTopic::RegionBlocks,
            "{s}"
        );
    }
    for s in [
        "",
        "/doli/unknown/1",
        "/doli/blocks/2",
        "/doli/blocks/1 ",
        " /doli/blocks/1",
        "/DOLI/BLOCKS/1",
        "/doli/blocks/1\0",
        "/doli/txs/1/../blocks/1",
    ] {
        assert_eq!(
            RelayTopic::from_topic_str(s),
            RelayTopic::Unknown,
            "{s:?} must not borrow a real topic's rule"
        );
    }
    let mut names: Vec<_> = ALL.iter().map(|t| variant_name(*t)).collect();
    names.dedup();
    assert_eq!(names.len(), 10);
}

// REQ-GRV-009 / REQ-GRV-SEC-004 — Decision: a failure means the block exception moved: undecodable block bytes no longer Reject, a stale block is forwarded, or a fresh block is not Accepted (or decoded twice).
#[test]
fn req_grv_009_block_rows_reject_ignore_accept_block() {
    for (row, data) in [
        ("empty", Vec::new()),
        ("garbage", v::GARBAGE.to_vec()),
        ("header bytes", v::bytes(v::HEADER)),
    ] {
        let d = decide(RelayTopic::Blocks, &data);
        assert_eq!((name(&d), reported(&d)), ("Reject", "Reject"), "{row}");
    }
    let cutoff = WALL - STALE_BLOCK_SLOT_THRESHOLD;
    let stale = block(cutoff - 1, 1);
    let d = decide(RelayTopic::Blocks, &stale.serialize());
    assert_eq!(
        (name(&d), reported(&d)),
        ("Ignore", "Ignore"),
        "stale block"
    );
    for (row, b) in [
        ("boundary", block(cutoff, 2)),
        ("current", block(WALL, 3)),
        ("future", block(WALL + 50, 4)),
    ] {
        match decide(RelayTopic::Blocks, &b.serialize()) {
            RelayDecision::AcceptBlock(got) => {
                assert_eq!(got.hash(), b.hash(), "{row}: decoded block");
                assert_eq!(got.header.slot, b.header.slot, "{row}");
            }
            other => panic!("{row}: want AcceptBlock, got {}", name(&other)),
        }
        let d = decide(RelayTopic::Blocks, &b.serialize());
        assert_eq!(
            reported(&d),
            "Accept",
            "{row}: re-delivery is not app-deduped"
        );
    }
    let old = block(1, 5);
    let d = decide_at(RelayTopic::Blocks, &old.serialize(), 0, v::NOW_UNIX);
    assert_eq!(
        name(&d),
        "AcceptBlock",
        "genesis_time 0 fails open for blocks"
    );
}

// F-16 / REQ-GRV-SEC-002 — Decision: a failure means a (slot, producer) dedup entered the block row, so a forged variant suppresses the genuine block.
#[test]
fn f_16_equivocating_block_pair_both_accepted() {
    let a = block(WALL, 0xA1);
    let b = block(WALL, 0xB2);
    assert_ne!(a.hash(), b.hash());
    assert_eq!(
        name(&decide(RelayTopic::Blocks, &a.serialize())),
        "AcceptBlock"
    );
    assert_eq!(
        name(&decide(RelayTopic::Blocks, &b.serialize())),
        "AcceptBlock"
    );
}

// REQ-GRV-003 / D1 — Decision: a failure means the swarm still decides tx relay (the 6.43.3 b LocalOnly bridge survived M2, or a tx message is reported before the node verdict).
#[test]
fn req_grv_003_tx_rows_defer_to_the_node_verdict() {
    for (row, data, want) in [
        (
            "legacy single tx",
            v::bytes(v::TX_LEGACY_TRANSFER),
            ("Defer", "none"),
        ),
        (
            "legacy minimum tx",
            v::bytes(v::TX_LEGACY_MIN),
            ("Defer", "none"),
        ),
        ("0xBA batch 3", v::bytes(v::TX_BATCH_3), ("Defer", "none")),
        (
            "0xAA announce",
            v::bytes(v::TX_ANNOUNCE_2),
            ("LocalOnly", "Ignore"),
        ),
    ] {
        let d = decide(RelayTopic::Transactions, &data);
        assert_eq!((name(&d), reported(&d)), want, "{row}");
        assert_row(
            RelayTopic::Transactions,
            &data,
            want.0,
            &format!("{row} again (R6: vendor dedup)"),
        );
    }
    for (row, data) in [("empty", Vec::new()), ("garbage", v::GARBAGE.to_vec())] {
        assert_row(RelayTopic::Transactions, &data, "Ignore", row);
    }
    let one = v::bytes(v::TX_BATCH_1);
    let three = v::bytes(v::TX_BATCH_3);
    assert_row(RelayTopic::Transactions, &one, "Defer", "batch 1");
    assert_row(
        RelayTopic::Transactions,
        &three,
        "Defer",
        "superset batch: raw-bytes key",
    );
}

// REQ-GRV-003 / F-4 — Decision: a failure means a fresh attestation is not Deferred to the node verdict (M3), a stale one reaches the node, or undecodable bytes fail open.
#[test]
fn req_grv_003_attestation_rows() {
    for (row, data) in [
        ("bls", v::bytes(v::ATTESTATION_BLS)),
        ("no bls", v::bytes(v::ATTESTATION_NO_BLS)),
        ("boundary", attestation(WALL - ATTEST_STALE_SLOTS)),
    ] {
        let d = decide(RelayTopic::Attestations, &data);
        assert_eq!((name(&d), reported(&d)), ("Defer", "none"), "{row}");
        assert_row(
            RelayTopic::Attestations,
            &data,
            "Defer",
            &format!("{row} again (R6: vendor dedup)"),
        );
    }
    let stale = attestation(WALL - ATTEST_STALE_SLOTS - 1);
    assert_row(RelayTopic::Attestations, &stale, "Ignore", "stale");
    for (row, data) in [("empty", Vec::new()), ("garbage", v::GARBAGE.to_vec())] {
        assert_row(RelayTopic::Attestations, &data, "Ignore", row);
    }
    let d = decide_at(RelayTopic::Attestations, &attestation(1), 0, v::NOW_UNIX);
    assert_eq!(name(&d), "Defer", "clock unset: decodable goes to the node");
    let d = decide_at(RelayTopic::Attestations, v::GARBAGE, 0, v::NOW_UNIX);
    assert_eq!(
        name(&d),
        "Ignore",
        "clock unset: undecodable is still Ignore"
    );
}

// REQ-GRV-003 / F-4 — Decision: a failure means a fresh vote is not Deferred to the node verdict (M3) or a non-vote JSON reaches the node or is forwarded.
#[test]
fn req_grv_003_vote_rows() {
    let boundary = vote(v::NOW_UNIX - VOTE_MAX_AGE_SECS);
    for (row, data) in [
        ("rpc json", v::VOTE_RPC.to_vec()),
        ("age boundary", boundary),
        ("timestamp 0", vote(0)),
    ] {
        let d = decide(RelayTopic::Votes, &data);
        assert_eq!((name(&d), reported(&d)), ("Defer", "none"), "{row}");
        assert_row(
            RelayTopic::Votes,
            &data,
            "Defer",
            &format!("{row} again (R6: vendor dedup)"),
        );
    }
    let stale = vote(v::NOW_UNIX - VOTE_MAX_AGE_SECS - 1);
    assert_row(RelayTopic::Votes, &stale, "Ignore", "stale");
    for (row, data) in [
        ("empty", Vec::new()),
        ("garbage", v::GARBAGE.to_vec()),
        ("json without timestamp", b"{\"vote\":\"approve\"}".to_vec()),
        ("json array", b"[1,2,3]".to_vec()),
    ] {
        assert_row(RelayTopic::Votes, &data, "Ignore", row);
    }
}

// REQ-GRV-003 / F-10 — Decision: a failure means a producer set is not Deferred to the GSet verdict (M3), a digest is forwarded (X5) or not processed, the legacy bincode list is forwarded, or undecodable bytes fail open.
#[test]
fn req_grv_003_producer_rows() {
    let set = v::bytes(v::PRODUCERS_PROTOBUF_SET);
    for (row, data, want) in [
        ("protobuf set", set.clone(), ("Defer", "none")),
        (
            "bloom digest",
            v::bytes(v::PRODUCERS_BLOOM_DIGEST),
            ("LocalOnly", "Ignore"),
        ),
    ] {
        let d = decide(RelayTopic::Producers, &data);
        assert_eq!((name(&d), reported(&d)), want, "{row}");
        assert_row(
            RelayTopic::Producers,
            &data,
            want.0,
            &format!("{row} again"),
        );
    }
    let legacy = v::bytes(v::PRODUCERS_LEGACY_BINCODE);
    let d = decide(RelayTopic::Producers, &legacy);
    assert_eq!(
        (name(&d), reported(&d)),
        ("LocalOnly", "Ignore"),
        "legacy bincode"
    );
    let ann_ts = 1_790_000_000u64;
    let late = ann_ts + PRODUCER_ANNOUNCEMENT_MAX_AGE_SECS + 1;
    let d = decide_at(RelayTopic::Producers, &set, v::GENESIS_TIME, late);
    assert_eq!(name(&d), "Ignore", "every announcement stale");
    let d = decide_at(RelayTopic::Producers, &set, v::GENESIS_TIME, late - 1);
    assert_eq!(name(&d), "Defer", "age boundary is fresh");
    assert_row(RelayTopic::Producers, v::GARBAGE, "Ignore", "garbage");
    let d = decide_at(RelayTopic::Producers, &set, v::GENESIS_TIME, 0);
    assert_eq!(name(&d), "Defer", "clock unset: decodable goes to the node");
    let d = decide_at(RelayTopic::Producers, v::GARBAGE, v::GENESIS_TIME, 0);
    assert_eq!(
        name(&d),
        "Ignore",
        "clock unset: undecodable is still Ignore"
    );
}

// REQ-GRV-015 / REQ-GRV-003 — Decision: a failure means a topic with no local consumer (headers, heartbeats, tier1, region, unknown incl. the old None -> Accept) can still be forwarded or processed.
#[test]
fn req_grv_015_unconsumed_topics_are_always_ignored() {
    let payloads = [
        ("header", v::bytes(v::HEADER)),
        ("heartbeat", v::bytes(v::HEARTBEAT)),
        ("fresh block", block(WALL, 9).serialize()),
        ("tx batch", v::bytes(v::TX_BATCH_3)),
        ("attestation", v::bytes(v::ATTESTATION_BLS)),
        ("empty", Vec::new()),
        ("garbage", v::GARBAGE.to_vec()),
    ];
    for topic in [
        RelayTopic::Headers,
        RelayTopic::Heartbeats,
        RelayTopic::Tier1Blocks,
        RelayTopic::RegionBlocks,
        RelayTopic::Unknown,
    ] {
        for (row, data) in &payloads {
            for genesis in [v::GENESIS_TIME, 0] {
                let d = decide_at(topic, data, genesis, v::NOW_UNIX);
                assert_eq!(
                    (name(&d), reported(&d)),
                    ("Ignore", "Ignore"),
                    "{} x {row} (genesis {genesis})",
                    variant_name(topic)
                );
            }
        }
    }
}

// REQ-GRV-SEC-004 / F-4 — Decision: a failure means a non-block topic can Reject (peer-score penalty) for bytes an old or hostile peer sends, which graylists honest mixed-version peers.
#[test]
fn req_grv_sec_004_no_non_block_topic_ever_rejects() {
    let nested: Vec<u8> = std::iter::repeat_n(b'[', 10_000)
        .chain(std::iter::repeat_n(b']', 10_000))
        .collect();
    let payloads: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("garbage", v::GARBAGE.to_vec()),
        ("0xAA only", vec![0xAA]),
        ("0xBA only", vec![0xBA]),
        ("0xBA huge count", vec![0xBA, 0xFF, 0xFF, 0xFF, 0xFF]),
        ("header bytes", v::bytes(v::HEADER)),
        ("sql", b"'; DROP TABLE x; --".to_vec()),
        ("shell", b"$(whoami); rm -rf /".to_vec()),
        ("path", b"../../etc/passwd".to_vec()),
        ("format", b"%s%s%s%s{0}${7*7}".to_vec()),
        ("ansi", b"\x1b[2J\r\n\x00".to_vec()),
        ("nested json", nested),
        ("1 MiB zeros", vec![0u8; 1 << 20]),
        ("1 MiB 0xFF", vec![0xFF; 1 << 20]),
    ];
    for topic in ALL {
        if topic == RelayTopic::Blocks {
            continue;
        }
        for (row, data) in &payloads {
            let d = decide(topic, data);
            assert_ne!(name(&d), "Reject", "{} x {row}", variant_name(topic));
            assert_ne!(reported(&d), "Reject", "{} x {row}", variant_name(topic));
            assert_ne!(name(&d), "AcceptBlock", "{} x {row}", variant_name(topic));
        }
    }
}
