//! INC-I-237 M0 / F-10 / REQ-GRV-012 — gossip wire-format corpus, retargeted in
//! M1 to the one relay-decision table.
//!
//! Every old wire format is pinned to its M1 relay decision. Rows the spec
//! changes (D2/D3: undecodable non-block -> Ignore, legacy bincode producer list
//! -> LocalOnly, headers / heartbeats / tier1 / region / unknown -> Ignore) carry
//! their new verdict; every other row keeps its v6.43.3 verdict.
//!
//! OUTPUT CONTRACT: fn decode_tx_gossip / decode_digest / decode_producer_set /
//!   BlockHeader::deserialize / Heartbeat::deserialize / Attestation::from_bytes
//!   and fn relay_decision(topic, data, ctx) -> RelayDecision (+ acceptance())
//!   O1: ctx.seen — SeenCache identity record (observed via the re-delivery verdict)
//!   O2: none (no receiver)
//!   O3: decoded value per vector; reported MessageAcceptance per (topic, vector)
//!   O4: none   O5: none   O6: local NetworkEvent kind, emitted only when the
//!       decision processes locally (AcceptBlock | LocalOnly | AcceptTransitional)
//! PATHS: P1 decodes + fresh, P2 empty bytes, P3 garbage bytes, P4 re-delivery,
//!   P5 wrong-topic bytes (header on blocks), P6 unknown topic
//! INPUT PARTITIONS: one row per format era (single era since v6.24.0 except
//!   TxType::RotateBlsKey) x {txs, producers, votes, heartbeats, attestations,
//!   headers, blocks, tier1, region, unknown}
//! MATRIX: 4 outputs (O1 via P4, O3 decode, O3 verdict, O6) x 38 rows — every
//!   row asserts first verdict, re-delivery verdict and local event.

use super::inc_i_237_m0_corpus_vectors as v;
use crypto::PublicKey;
use doli_core::{Attestation, Block, BlockHeader, Heartbeat, Transaction, TxType};
use libp2p::gossipsub::MessageAcceptance;
use network::gossip::staleness::{
    SeenCache, StalenessCtx, SEEN_CACHE_CAPACITY, SEEN_CACHE_TTL_SECS,
};
use network::gossip::{
    decode_tx_gossip, encode_tx_announce, encode_tx_batch, relay_decision, RelayDecision,
    RelayTopic, TxGossipMessage, ATTESTATION_TOPIC, BLOCKS_TOPIC, PRODUCERS_TOPIC,
    TIER1_BLOCKS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};

const REGION_BLOCKS_TOPIC: &str = "/doli/r1/blocks/1";
const HEADERS_TOPIC: &str = "/doli/headers/1";
const HEARTBEATS_TOPIC: &str = "/doli/heartbeats/1";
const UNKNOWN_TOPIC: &str = "/doli/unknown/1";

fn label(a: &MessageAcceptance) -> &'static str {
    match a {
        MessageAcceptance::Accept => "Accept",
        MessageAcceptance::Ignore => "Ignore",
        MessageAcceptance::Reject => "Reject",
    }
}

/// What the swarm reports to gossipsub, and whether it processes the message locally.
fn swarm_report(topic: &str, data: &[u8], seen: &mut SeenCache) -> (&'static str, bool) {
    let mut ctx = StalenessCtx {
        now_unix: v::NOW_UNIX,
        genesis_time: v::GENESIS_TIME,
        slot_duration: v::SLOT_DURATION,
        best_slot: 1_000,
        seen,
    };
    let d = relay_decision(RelayTopic::from_topic_str(topic), data, &mut ctx);
    let local = matches!(
        d,
        RelayDecision::AcceptBlock(_)
            | RelayDecision::LocalOnly
            | RelayDecision::AcceptTransitional
    );
    (label(&d.acceptance()), local)
}

/// The NetworkEvent kind the swarm emits for a locally processed message, rate limits aside.
fn local_event(topic: &str, data: &[u8]) -> String {
    if topic == BLOCKS_TOPIC {
        return match Block::deserialize(data) {
            Some(_) => "NewBlock".into(),
            None => "none".into(),
        };
    }
    match topic {
        TRANSACTIONS_TOPIC => match decode_tx_gossip(data) {
            Some(TxGossipMessage::FullBatch(txs)) => format!("NewTransaction x{}", txs.len()),
            Some(TxGossipMessage::Announce(h)) => format!("TxAnnouncement x{}", h.len()),
            None => "none".into(),
        },
        PRODUCERS_TOPIC => {
            if let Ok(bloom) = doli_core::decode_digest(data) {
                if bloom.size_bits() > 0 {
                    return "ProducerDigestReceived".into();
                }
            }
            if doli_core::is_legacy_bincode_format(data) {
                match bincode::deserialize::<Vec<PublicKey>>(data) {
                    Ok(pks) => format!("ProducersAnnounced x{}", pks.len()),
                    Err(_) => "none".into(),
                }
            } else {
                match doli_core::decode_producer_set(data) {
                    Ok(anns) => format!("ProducerAnnouncementsReceived x{}", anns.len()),
                    Err(_) => "none".into(),
                }
            }
        }
        VOTES_TOPIC => "NewVote".into(),
        t if t == ATTESTATION_TOPIC => "NewAttestation".into(),
        _ => "none".into(),
    }
}

struct Row {
    name: &'static str,
    topic: &'static str,
    data: Vec<u8>,
    first: &'static str,
    again: &'static str,
    local: &'static str,
}

fn row(
    name: &'static str,
    topic: &'static str,
    data: Vec<u8>,
    first: &'static str,
    again: &'static str,
    local: &'static str,
) -> Row {
    Row {
        name,
        topic,
        data,
        first,
        again,
        local,
    }
}

/// The frozen table: (vector, topic) -> first verdict, re-delivery verdict, local event.
fn corpus() -> Vec<Row> {
    let empty = Vec::new;
    let junk = || v::GARBAGE.to_vec();
    vec![
        // txs: classify Accept -> reported Ignore (6.43.3 b); local dispatch still runs.
        row(
            "tx legacy transfer",
            TRANSACTIONS_TOPIC,
            v::bytes(v::TX_LEGACY_TRANSFER),
            "Ignore",
            "Ignore",
            "NewTransaction x1",
        ),
        row(
            "tx legacy rotate-bls era",
            TRANSACTIONS_TOPIC,
            v::bytes(v::TX_LEGACY_ROTATE_BLS_ERA),
            "Ignore",
            "Ignore",
            "NewTransaction x1",
        ),
        row(
            "tx legacy minimum",
            TRANSACTIONS_TOPIC,
            v::bytes(v::TX_LEGACY_MIN),
            "Ignore",
            "Ignore",
            "NewTransaction x1",
        ),
        row(
            "tx 0xBA batch 1",
            TRANSACTIONS_TOPIC,
            v::bytes(v::TX_BATCH_1),
            "Ignore",
            "Ignore",
            "NewTransaction x1",
        ),
        row(
            "tx 0xBA batch 3",
            TRANSACTIONS_TOPIC,
            v::bytes(v::TX_BATCH_3),
            "Ignore",
            "Ignore",
            "NewTransaction x3",
        ),
        row(
            "tx 0xAA announce 2",
            TRANSACTIONS_TOPIC,
            v::bytes(v::TX_ANNOUNCE_2),
            "Ignore",
            "Ignore",
            "TxAnnouncement x2",
        ),
        row(
            "tx empty",
            TRANSACTIONS_TOPIC,
            empty(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "tx garbage",
            TRANSACTIONS_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        // producers: no identity dedup -> re-delivery verdict equals first sight.
        row(
            "producers legacy bincode",
            PRODUCERS_TOPIC,
            v::bytes(v::PRODUCERS_LEGACY_BINCODE),
            "Ignore",
            "Ignore",
            "ProducersAnnounced x2",
        ),
        row(
            "producers protobuf set",
            PRODUCERS_TOPIC,
            v::bytes(v::PRODUCERS_PROTOBUF_SET),
            "Accept",
            "Accept",
            "ProducerAnnouncementsReceived x1",
        ),
        row(
            "producers bloom digest",
            PRODUCERS_TOPIC,
            v::bytes(v::PRODUCERS_BLOOM_DIGEST),
            "Accept",
            "Accept",
            "ProducerDigestReceived",
        ),
        row(
            "producers empty",
            PRODUCERS_TOPIC,
            empty(),
            "Accept",
            "Accept",
            "ProducerAnnouncementsReceived x0",
        ),
        row(
            "producers garbage",
            PRODUCERS_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        // votes/heartbeats/attestations/headers: raw-bytes identity dedup, fail-open decode.
        row(
            "vote rpc json",
            VOTES_TOPIC,
            v::VOTE_RPC.to_vec(),
            "Accept",
            "Ignore",
            "NewVote",
        ),
        row(
            "vote empty",
            VOTES_TOPIC,
            empty(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "vote garbage",
            VOTES_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "heartbeat v1",
            HEARTBEATS_TOPIC,
            v::bytes(v::HEARTBEAT),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "heartbeat empty",
            HEARTBEATS_TOPIC,
            empty(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "heartbeat garbage",
            HEARTBEATS_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "attestation bls",
            ATTESTATION_TOPIC,
            v::bytes(v::ATTESTATION_BLS),
            "Accept",
            "Ignore",
            "NewAttestation",
        ),
        row(
            "attestation no-bls",
            ATTESTATION_TOPIC,
            v::bytes(v::ATTESTATION_NO_BLS),
            "Accept",
            "Ignore",
            "NewAttestation",
        ),
        row(
            "attestation empty",
            ATTESTATION_TOPIC,
            empty(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "attestation garbage",
            ATTESTATION_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "header",
            HEADERS_TOPIC,
            v::bytes(v::HEADER),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "header empty",
            HEADERS_TOPIC,
            empty(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "header garbage",
            HEADERS_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        // block-body topics: undecodable -> Reject (P4 penalty), no dedup.
        row(
            "blocks empty",
            BLOCKS_TOPIC,
            empty(),
            "Reject",
            "Reject",
            "none",
        ),
        row(
            "blocks garbage",
            BLOCKS_TOPIC,
            junk(),
            "Reject",
            "Reject",
            "none",
        ),
        row(
            "blocks header bytes",
            BLOCKS_TOPIC,
            v::bytes(v::HEADER),
            "Reject",
            "Reject",
            "none",
        ),
        row(
            "tier1 empty",
            TIER1_BLOCKS_TOPIC,
            empty(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "tier1 garbage",
            TIER1_BLOCKS_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "region empty",
            REGION_BLOCKS_TOPIC,
            empty(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "region garbage",
            REGION_BLOCKS_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        // unknown topic fails open.
        row(
            "unknown topic garbage",
            UNKNOWN_TOPIC,
            junk(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "unknown topic empty",
            UNKNOWN_TOPIC,
            empty(),
            "Ignore",
            "Ignore",
            "none",
        ),
        row(
            "unknown topic tx batch",
            UNKNOWN_TOPIC,
            v::bytes(v::TX_BATCH_3),
            "Ignore",
            "Ignore",
            "none",
        ),
    ]
}

// REQ-GRV-012 / F-10 / REQ-GRV-SEC-004 — Decision: a failure means the relay table moved the verdict or the local event for a wire format an old peer still sends (Reject = peer penalty, Ignore = silent drop) beyond the rows M1 deliberately changes.
#[test]
fn req_grv_012_f10_corpus_relay_decisions_and_local_events() {
    let rows = corpus();
    assert!(rows.len() >= 36, "corpus must not shrink");
    let mut bad = Vec::new();
    for r in &rows {
        let mut seen = SeenCache::new(SEEN_CACHE_TTL_SECS, SEEN_CACHE_CAPACITY);
        let (first, processed) = swarm_report(r.topic, &r.data, &mut seen);
        let (again, _) = swarm_report(r.topic, &r.data, &mut seen);
        let local = if processed {
            local_event(r.topic, &r.data)
        } else {
            "none".to_string()
        };
        if (first, again, local.as_str()) != (r.first, r.again, r.local) {
            bad.push(format!(
                "{} on {}: got ({first}, {again}, {local}) want ({}, {}, {})",
                r.name, r.topic, r.first, r.again, r.local
            ));
        }
    }
    assert!(bad.is_empty(), "F-10 corpus drift:\n{}", bad.join("\n"));
}

// REQ-GRV-012 — Decision: a failure means a format era no longer decodes to the same transactions, so an old node's txs would be lost or altered at a new node.
#[test]
fn req_grv_012_tx_vectors_decode_to_the_frozen_transactions() {
    let one = |hex: &str| -> Transaction {
        match decode_tx_gossip(&v::bytes(hex)) {
            Some(TxGossipMessage::FullBatch(mut txs)) if txs.len() == 1 => txs.remove(0),
            _ => panic!("expected a 1-tx FullBatch"),
        }
    };
    let t1 = one(v::TX_LEGACY_TRANSFER);
    assert_eq!(t1.hash().to_hex(), v::TX1_HASH);
    assert_eq!((t1.inputs.len(), t1.outputs.len()), (1, 1));
    assert_eq!(t1.outputs[0].amount, 1_001);
    assert_eq!(
        t1.serialize(),
        v::bytes(v::TX_LEGACY_TRANSFER),
        "re-encode drift"
    );

    let rot = one(v::TX_LEGACY_ROTATE_BLS_ERA);
    assert_eq!(rot.tx_type, TxType::RotateBlsKey);
    let min = one(v::TX_LEGACY_MIN);
    assert_eq!(min.tx_type, TxType::Transfer);
    assert!(min.inputs.is_empty() && min.outputs.is_empty() && min.extra_data.is_empty());
    assert_eq!(
        v::bytes(v::TX_LEGACY_MIN).len(),
        32,
        "minimum legacy tx is 32 bytes"
    );

    assert_eq!(one(v::TX_BATCH_1).hash().to_hex(), v::TX1_HASH);
    let batch = match decode_tx_gossip(&v::bytes(v::TX_BATCH_3)) {
        Some(TxGossipMessage::FullBatch(txs)) => txs,
        _ => panic!("0xBA batch 3 must decode"),
    };
    let amounts: Vec<u64> = batch.iter().map(|t| t.outputs[0].amount).collect();
    assert_eq!(amounts, vec![1_001, 1_002, 1_003]);
    assert_eq!(batch[1].hash().to_hex(), v::TX2_HASH);
    assert_eq!(
        encode_tx_batch(&batch),
        v::bytes(v::TX_BATCH_3),
        "batch re-encode drift"
    );

    match decode_tx_gossip(&v::bytes(v::TX_ANNOUNCE_2)) {
        Some(TxGossipMessage::Announce(h)) => {
            let hex: Vec<String> = h.iter().map(|x| x.to_hex()).collect();
            assert_eq!(hex, vec![v::TX1_HASH.to_string(), v::TX2_HASH.to_string()]);
            assert_eq!(encode_tx_announce(&h), v::bytes(v::TX_ANNOUNCE_2));
        }
        _ => panic!("0xAA announce must decode"),
    }
    assert!(decode_tx_gossip(&[]).is_none());
    assert!(decode_tx_gossip(v::GARBAGE).is_none());
}

// REQ-GRV-012 — Decision: a failure means one of the three producer formats decodes differently, so GSet convergence with old peers (INC-I-002 decode order) breaks.
#[test]
fn req_grv_012_producer_vectors_decode_in_todays_order() {
    let pk = |hex: &str| PublicKey::from_bytes(v::bytes(hex).try_into().unwrap());
    let legacy = v::bytes(v::PRODUCERS_LEGACY_BINCODE);
    assert!(doli_core::decode_digest(&legacy).map_or(true, |b| b.size_bits() == 0));
    assert!(doli_core::is_legacy_bincode_format(&legacy));
    let pks: Vec<PublicKey> = bincode::deserialize(&legacy).unwrap();
    assert_eq!(pks, vec![pk(v::PK9), pk(v::PK10)]);

    let set = v::bytes(v::PRODUCERS_PROTOBUF_SET);
    assert!(!doli_core::is_legacy_bincode_format(&set));
    let anns = doli_core::decode_producer_set(&set).unwrap();
    assert_eq!(anns.len(), 1);
    assert_eq!(anns[0].pubkey, pk(v::PK9));
    assert_eq!((anns[0].network_id, anns[0].sequence), (1, 5));
    assert_eq!(anns[0].timestamp, 1_790_000_000);
    assert!(
        anns[0].verify(),
        "frozen announcement signature must still verify"
    );
    assert_eq!(
        doli_core::encode_producer_set(&anns),
        set,
        "protobuf re-encode drift"
    );

    let digest = doli_core::decode_digest(&v::bytes(v::PRODUCERS_BLOOM_DIGEST)).unwrap();
    assert_eq!((digest.size_bits(), digest.element_count()), (64, 2));
    assert!(digest.probably_contains(&pk(v::PK9)) && digest.probably_contains(&pk(v::PK10)));
}

// REQ-GRV-012 — Decision: a failure means header/heartbeat/attestation/vote bytes from an old peer no longer decode to the same fields, so the staleness age gate or the node handler sees a different message.
#[test]
fn req_grv_012_header_heartbeat_attestation_vote_vectors_decode() {
    let pk9 = PublicKey::from_bytes(v::bytes(v::PK9).try_into().unwrap());
    let h = BlockHeader::deserialize(&v::bytes(v::HEADER)).expect("header decodes");
    assert_eq!((h.slot, h.timestamp, h.version), (1_000, 1_790_000_100, 2));
    assert_eq!(h.hash().to_hex(), v::HEADER_HASH);
    assert_eq!(h.serialize(), v::bytes(v::HEADER));

    let hb = Heartbeat::deserialize(&v::bytes(v::HEARTBEAT)).expect("heartbeat decodes");
    assert_eq!((hb.version, hb.slot, hb.producer), (1, 1_000, pk9));
    assert!(hb.witnesses.is_empty());

    let a = Attestation::from_bytes(&v::bytes(v::ATTESTATION_BLS)).expect("attestation decodes");
    assert_eq!(
        (a.slot, a.height, a.attester_weight, a.attester),
        (1_000, 900, 3, pk9)
    );
    assert_eq!(a.bls_signature, vec![0xCC; 96]);
    let a0 = Attestation::from_bytes(&v::bytes(v::ATTESTATION_NO_BLS)).expect("no-bls decodes");
    assert!(a0.bls_signature.is_empty());
    assert_eq!(a0.to_bytes(), v::bytes(v::ATTESTATION_NO_BLS));

    let vote: serde_json::Value = serde_json::from_slice(v::VOTE_RPC).unwrap();
    assert_eq!(vote["timestamp"], 1_790_000_000u64);
    assert_eq!(vote["producerId"], v::PK9);
    assert_eq!(vote["vote"], "approve");
}
