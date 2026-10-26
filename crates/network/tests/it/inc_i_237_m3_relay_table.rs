//! INC-I-237 M3 (R2) — the relay table after attestations, votes and producer
//! sets moved to the node verdict: blocks keep the ONE pre-verdict Accept.
//!
//! OUTPUT CONTRACT: fn relay_decision(topic, data, ctx) -> RelayDecision (+ acceptance())
//!   O1: none (M4 R6: no app dedup; the second decision is judged on its own)
//!   O3: RelayDecision variant and the MessageAcceptance the swarm reports
//!   O6: network source text (one relay authority: no AcceptTransitional,
//!       two report sites)
//! PATHS: P1 fresh decodable content per topic; P2 forged-signature variant
//!   first, genuine second (attestation, vote); P3 source scan
//! INPUT PARTITIONS: topic {all 10 RelayTopic} x payload {fresh block, tx batch,
//!   attestation bls / no-bls / signed, vote, producer set, digest, legacy
//!   producers, header, heartbeat, empty, garbage}; signature {valid, garbage}
//! MATRIX: O3 x P1 (every topic x every payload), O1+O3 x P2, O6 x P3.

use std::path::{Path, PathBuf};

use super::inc_i_237_m0_corpus_vectors as v;
use super::inc_i_237_m3_fake_node::{attestation, forge, vote_json};
use crypto::{Hash, KeyPair, PublicKey};
use doli_core::{Block, BlockHeader};
use libp2p::gossipsub::MessageAcceptance;
use network::gossip::staleness::StalenessCtx;
use network::gossip::{relay_decision, RelayDecision, RelayTopic};

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

fn decide(topic: RelayTopic, data: &[u8]) -> RelayDecision {
    let ctx = StalenessCtx {
        now_unix: v::NOW_UNIX,
        genesis_time: v::GENESIS_TIME,
        slot_duration: v::SLOT_DURATION,
        best_slot: WALL,
        best_height: 0,
        block_signature_activation_height: u64::MAX,
    };
    relay_decision(topic, data, &ctx)
}

/// Variant name without its payload; compiles whichever variants exist.
fn name(d: &RelayDecision) -> String {
    let s = format!("{d:?}");
    s.split('(').next().unwrap_or_default().to_string()
}

fn fresh_block() -> Vec<u8> {
    let header = BlockHeader {
        version: 2,
        prev_hash: Hash::ZERO,
        merkle_root: Hash::from_bytes([7; 32]),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: v::NOW_UNIX,
        slot: WALL,
        producer: PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![]).serialize()
}

fn payloads() -> Vec<(&'static str, Vec<u8>)> {
    let kp = KeyPair::from_seed([3; 32]);
    vec![
        ("fresh block", fresh_block()),
        ("tx batch", v::bytes(v::TX_BATCH_3)),
        ("attestation bls", v::bytes(v::ATTESTATION_BLS)),
        ("attestation no-bls", v::bytes(v::ATTESTATION_NO_BLS)),
        ("attestation signed", attestation(&kp, 1, WALL).to_bytes()),
        ("vote rpc", v::VOTE_RPC.to_vec()),
        ("vote signed", vote_json(&kp, v::NOW_UNIX)),
        ("producer set", v::bytes(v::PRODUCERS_PROTOBUF_SET)),
        ("producer digest", v::bytes(v::PRODUCERS_BLOOM_DIGEST)),
        ("legacy producers", v::bytes(v::PRODUCERS_LEGACY_BINCODE)),
        ("header", v::bytes(v::HEADER)),
        ("heartbeat", v::bytes(v::HEARTBEAT)),
        ("empty", Vec::new()),
        ("garbage", v::GARBAGE.to_vec()),
    ]
}

// REQ-GRV-003 — Decision: a failure means some non-block topic is still forwarded on the swarm's word before the node judged it (the INC-I-237 relay-before-verdict shape), or the block exception was lost.
#[test]
fn req_grv_003_relay_table_has_exactly_one_pre_verdict_accept_blocks() {
    let mut accepted = Vec::new();
    for topic in ALL {
        for (row, data) in payloads() {
            let d = decide(topic, &data);
            if matches!(d.acceptance(), Some(MessageAcceptance::Accept)) {
                accepted.push(format!("{topic:?} x {row} -> {}", name(&d)));
            }
        }
    }
    assert_eq!(
        accepted,
        vec!["Blocks x fresh block -> AcceptBlock".to_string()],
        "the only pre-verdict Accept must be a fresh block"
    );
}

// REQ-GRV-003 — Decision: a failure means a fresh attestation, vote or producer set is not handed to the node verdict (Ignored or processed LocalOnly), so valid consensus gossip stops at one hop.
#[test]
fn req_grv_003_fresh_attestation_vote_and_producer_set_defer() {
    let kp = KeyPair::from_seed([4; 32]);
    for (topic, row, data) in [
        (
            RelayTopic::Attestations,
            "attestation",
            attestation(&kp, 2, WALL).to_bytes(),
        ),
        (RelayTopic::Votes, "vote", vote_json(&kp, v::NOW_UNIX)),
        (
            RelayTopic::Producers,
            "producer set",
            v::bytes(v::PRODUCERS_PROTOBUF_SET),
        ),
    ] {
        let d = decide(topic, &data);
        assert_eq!(
            (name(&d), d.acceptance().is_none()),
            ("Defer".into(), true),
            "{row}"
        );
    }
}

// REQ-GRV-SEC-002 — Decision: a failure means a forged-signature variant carrying a victim's (attester, slot) or (producer, timestamp) changes the swarm's decision for the genuine message, so an attacker can pre-empt it.
#[test]
fn req_grv_sec_002_forged_variant_first_leaves_the_genuine_deferred() {
    let kp = KeyPair::from_seed([5; 32]);
    let genuine = attestation(&kp, 9, WALL);
    let forged = forge(&genuine);
    assert_ne!(
        forged.to_bytes(),
        genuine.to_bytes(),
        "fixture: distinct bytes"
    );
    let mut vote_forged = vote_json(&kp, v::NOW_UNIX);
    let at = vote_forged.len() - 4;
    vote_forged[at] = if vote_forged[at] == b'0' { b'1' } else { b'0' };
    let cases = [
        (
            RelayTopic::Attestations,
            forged.to_bytes(),
            genuine.to_bytes(),
        ),
        (RelayTopic::Votes, vote_forged, vote_json(&kp, v::NOW_UNIX)),
    ];
    for (topic, forged, genuine) in cases {
        let first = decide(topic, &forged);
        let second = decide(topic, &genuine);
        assert_eq!(
            name(&first),
            "Defer",
            "{topic:?}: forged variant goes to the node"
        );
        assert_eq!(
            name(&second),
            "Defer",
            "{topic:?}: genuine after a forged variant"
        );
        assert!(
            second.acceptance().is_none(),
            "{topic:?}: no swarm report for the genuine"
        );
    }
}

fn network_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Non-test `.rs` files under `dir`, with comment lines removed.
fn non_test_sources(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read src").flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if p.is_dir() {
                if name != "tests" {
                    stack.push(p);
                }
            } else if name.ends_with(".rs") && !name.contains("test") {
                let code: String = std::fs::read_to_string(&p)
                    .unwrap()
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .map(|l| format!("{l}\n"))
                    .collect();
                out.push((p.display().to_string(), code));
            }
        }
    }
    out
}

// REQ-GRV-014 — Decision: a failure means a second relay authority survived M3 (the transitional swarm Accept, or a third report site), so forwarding is again decided in more than the block row and the node verdict.
#[test]
fn req_grv_014_one_relay_authority_in_network_source() {
    let sources = non_test_sources(&network_src());
    assert!(sources.len() > 20, "scan found the network sources");
    let transitional: Vec<_> = sources
        .iter()
        .filter(|(_, c)| c.contains("AcceptTransitional"))
        .map(|(p, _)| p.clone())
        .collect();
    assert!(
        transitional.is_empty(),
        "AcceptTransitional still in {transitional:?}"
    );
    let mut sites: Vec<String> = sources
        .iter()
        .filter(|(_, c)| c.contains("report_message_validation_result("))
        .map(|(p, c)| {
            let n = c.matches("report_message_validation_result(").count();
            let file = Path::new(p)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            format!("{file} x{n}")
        })
        .collect();
    sites.sort();
    assert_eq!(
        sites,
        vec![
            "command_handling.rs x1".to_string(),
            "gossip_intake.rs x1".to_string()
        ],
        "report sites: the swarm intake and the node-verdict arm only"
    );
}
