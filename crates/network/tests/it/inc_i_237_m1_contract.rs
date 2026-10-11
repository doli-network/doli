//! INC-I-237 M1 — structural contract: D3 (no header/heartbeat topic, command
//! or publish) and D4 (the swarm->node sink cannot await, by type and by source).
//!
//! OUTPUT CONTRACT: fn subscribe_to_topics(&mut Gossipsub) -> Result<(), GossipError>
//!   O1: the gossipsub behaviour's subscription set (observed via topics())
//!   O3: Result Ok
//! Type shape: enum NetworkCommand (exhaustive match, no wildcard).
//! Source shape: service/{event_sink, swarm_loop, behaviour_events, swarm_events,
//!   backpressure}.rs non-test sections (the part above the first #[cfg(test)]),
//!   bins/node/src (publish call sites).
//! PATHS: P1 production subscribe; P2 every NetworkCommand variant; P3 each
//!   forbidden token per file; P4 each required sync method present.
//! INPUT PARTITIONS: subscription set {5 consumed topics present, headers absent,
//!   heartbeats absent}; NetworkCommand {17 kept variants}; tokens {async fn,
//!   async block, .await, Sender accessor, Sender return, pub Sender field,
//!   awaitable reserve, raw sender use, flush_one/has_backlog leftovers}.
//! MATRIX: O1 x P1 (exact set); shape x P2-P4 — all asserted.

use std::collections::BTreeSet;

use network::gossip::{
    compute_dynamic_mesh, new_gossipsub, subscribe_to_topics, ATTESTATION_TOPIC, BLOCKS_TOPIC,
    PRODUCERS_TOPIC, TRANSACTIONS_TOPIC, VOTES_TOPIC,
};
use network::{Keypair, NetworkCommand};

fn source(rel: &str) -> String {
    let path = format!("{}/src/service/{rel}", env!("CARGO_MANIFEST_DIR"));
    let all = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    match all.find("#[cfg(test)]") {
        Some(i) => all[..i].to_string(),
        None => all,
    }
}

fn code_lines(src: &str) -> Vec<(usize, &str)> {
    src.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.starts_with("//"))
        .collect()
}

fn offending(src: &str, token: &str) -> Vec<String> {
    code_lines(src)
        .into_iter()
        .filter(|(_, l)| l.contains(token))
        .map(|(n, l)| format!("{n}: {l}"))
        .collect()
}

fn command_name(c: &NetworkCommand) -> &'static str {
    match c {
        NetworkCommand::BroadcastBlock(_) => "BroadcastBlock",
        NetworkCommand::BroadcastTransaction(_) => "BroadcastTransaction",
        NetworkCommand::RequestStatus { .. } => "RequestStatus",
        NetworkCommand::RequestSync { .. } => "RequestSync",
        NetworkCommand::SendStatusResponse { .. } => "SendStatusResponse",
        NetworkCommand::SendSyncResponse { .. } => "SendSyncResponse",
        NetworkCommand::Connect(_) => "Connect",
        NetworkCommand::Disconnect(_) => "Disconnect",
        NetworkCommand::Bootstrap => "Bootstrap",
        NetworkCommand::BroadcastProducers(_) => "BroadcastProducers",
        NetworkCommand::BroadcastProducerAnnouncements(_) => "BroadcastProducerAnnouncements",
        NetworkCommand::BroadcastProducerDigest(_) => "BroadcastProducerDigest",
        NetworkCommand::SendProducerDelta { .. } => "SendProducerDelta",
        NetworkCommand::BroadcastVote(_) => "BroadcastVote",
        NetworkCommand::BroadcastAttestation(_) => "BroadcastAttestation",
        NetworkCommand::RequestTxFetch { .. } => "RequestTxFetch",
        NetworkCommand::SendTxFetchResponse { .. } => "SendTxFetchResponse",
        NetworkCommand::GossipVerdict { .. } => "GossipVerdict",
    }
}

// REQ-GRV-015 / F-2 — Decision: a failure means the node still joins the headers or heartbeats mesh, so it relays consumer-less traffic and forged headers can drain the global block bucket of direct peers (C-3).
#[test]
fn req_grv_015_production_subscribes_exactly_the_five_consumed_topics() {
    let mut g = new_gossipsub(&Keypair::generate_ed25519(), &compute_dynamic_mesh(8))
        .expect("production gossipsub config");
    subscribe_to_topics(&mut g).expect("subscribe");
    let got: BTreeSet<String> = g.topics().map(|t| t.as_str().to_string()).collect();
    let want: BTreeSet<String> = [
        BLOCKS_TOPIC,
        TRANSACTIONS_TOPIC,
        PRODUCERS_TOPIC,
        VOTES_TOPIC,
        ATTESTATION_TOPIC,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(got, want);
    assert!(!got.contains("/doli/headers/1") && !got.contains("/doli/heartbeats/1"));
}

// REQ-GRV-015 / REQ-GRV-020 — Decision: a failure means a header or heartbeat publish command came back, so the node again publishes a header ahead of every block on a topic nobody consumes.
#[test]
fn req_grv_015_network_command_has_no_header_or_heartbeat_publish() {
    let c = NetworkCommand::Bootstrap;
    assert_eq!(command_name(&c), "Bootstrap");
    let bins = format!("{}/../../bins/node/src", env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(&bins)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{dir:?}: {e}")) {
            let p = entry.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let s = std::fs::read_to_string(&p).unwrap();
                for (n, l) in code_lines(&s) {
                    if l.contains("broadcast_header(") || l.contains("broadcast_heartbeat(") {
                        hits.push(format!("{}:{n}: {l}", p.display()));
                    }
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "header/heartbeat publish call sites:\n{}",
        hits.join("\n")
    );
}

// REQ-GRV-006 / F-6 — Decision: a failure means the swarm->node sink regained an awaitable path (async method, Sender accessor or public sender field), so the swarm can park on the node again (the 8-freeze cycle C1).
#[test]
fn req_grv_006_event_sink_has_no_async_and_no_sender_accessor() {
    let src = source("event_sink.rs");
    for token in ["async ", "async{", ".await", "fn sender", ".reserve()"] {
        let hits = offending(&src, token);
        assert!(
            hits.is_empty(),
            "event_sink.rs contains `{token}`:\n{}",
            hits.join("\n")
        );
    }
    for (n, l) in code_lines(&src) {
        if l.contains("fn ") {
            if let Some((_, ret)) = l.split_once("->") {
                assert!(
                    !ret.contains("Sender"),
                    "event_sink.rs:{n} returns a Sender: {l}"
                );
            }
        }
        let public = l.starts_with("pub ") || l.starts_with("pub(");
        assert!(
            !(public && l.contains("Sender<") && !l.contains("fn ")),
            "event_sink.rs:{n} exposes a Sender field: {l}"
        );
    }
    for required in ["fn emit(", "fn emit_block(", "fn try_flush("] {
        assert!(src.contains(required), "event_sink.rs lacks `{required}`");
    }
}

// REQ-GRV-006 / F-6 — Decision: a failure means a swarm-side caller still reaches the raw event Sender or awaits a reserve, so the await-cycle is again possible by convention only.
#[test]
fn req_grv_006_swarm_side_never_touches_the_raw_event_sender() {
    let swarm_loop = source("swarm_loop.rs");
    for token in [".reserve()", ".sender()", "flush_one", "has_backlog"] {
        let hits = offending(&swarm_loop, token);
        assert!(
            hits.is_empty(),
            "swarm_loop.rs contains `{token}`:\n{}",
            hits.join("\n")
        );
    }
    assert!(
        swarm_loop.contains("try_flush()"),
        "swarm_loop.rs never drains the backlog"
    );
    for file in ["behaviour_events.rs", "swarm_events.rs"] {
        let hits = offending(&source(file), ".sender()");
        assert!(
            hits.is_empty(),
            "{file} uses the raw sender:\n{}",
            hits.join("\n")
        );
    }
    let bp = source("backpressure.rs");
    for token in ["mpsc::Sender", "Sender<"] {
        let hits = offending(&bp, token);
        assert!(
            hits.is_empty(),
            "backpressure.rs takes a raw sender:\n{}",
            hits.join("\n")
        );
    }
}
