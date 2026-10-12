//! INC-I-237 M5 decision C — on one link, vote and producer gossip cannot
//! spend the attestation budget. Live A1, A2 -> B -> C line of real
//! `NetworkService`s (127.0.0.1); B and C run the M3 fake node (Judge: every
//! vote and producer set Accepted, attestations judged R2).
//!
//! OUTPUT CONTRACT: swarm intake at C (`handle_gossip_message` ->
//!   `within_peer_budget` / `defer`) for messages forwarded by B; contract §C1
//!   O1-O5: none   O6: NewAttestation / NewVote / producer events at B and C
//! PATHS: P1 B forwards a vote-variant flood above C's request budget for B
//!   while B publishes genuine attestations -> every attestation at C; P2 the
//!   same with a producer-announcement flood
//! INPUT PARTITIONS: flood {hex-case vote variants, distinct one-key producer
//!   sets}; 2 attacker links x 50/s x 16 s; honest 10 attestations, 1/s
//! MATRIX: O6 at C (honest delivery) x P1, P2; O6 at B (load forwarded to C
//!   above C's budget) x P1, P2 for non-vacuity; topology asserted.

use std::collections::HashSet;
use std::time::Duration;

use crypto::KeyPair;
use network::NetworkCommand;

use super::inc_i_237_m2_common as h;
use super::inc_i_237_m3_fake_node::{
    announcement, attestation, keys, now_secs, spawn_fake3, vote_json, Fake3, Kind, Mode,
};

const LINKS: usize = 2;
const RATE_PER_LINK: u64 = 50;
const FLOOD_SECS: u64 = 16;
const HONEST: u32 = 10;
const HONEST_START_SECS: u64 = 5;
/// C's request budget for B over the flood (burst 200 + 20/s).
const C_BUDGET: usize = 200 + 20 * FLOOD_SECS as usize;

#[derive(Clone, Copy)]
enum Flood {
    Votes,
    Producers,
}

/// Variant `i` of a vote: hex letters of its signature upper-cased by the bits of `i`.
fn vote_variant(base: &[u8], i: u64) -> Vec<u8> {
    let key = b"\"signature\":\"";
    let start = base.windows(key.len()).position(|w| w == key).unwrap() + key.len();
    let mut out = base.to_vec();
    let letters: Vec<usize> = (start..out.len())
        .filter(|&k| (b'a'..=b'f').contains(&out[k]))
        .collect();
    assert!(letters.len() >= 24, "setup: too few hex letters to vary");
    for (bit, &k) in letters.iter().take(32).enumerate() {
        if (i >> bit) & 1 == 1 {
            out[k] = out[k].to_ascii_uppercase();
        }
    }
    out
}

fn flood_msg(flood: Flood, link: usize, k: u64, base: &[u8]) -> NetworkCommand {
    match flood {
        Flood::Votes => NetworkCommand::BroadcastVote(vote_variant(base, k)),
        Flood::Producers => {
            let mut seed = [0u8; 32];
            seed[0] = 0xD0 + link as u8;
            seed[1..9].copy_from_slice(&k.to_le_bytes());
            NetworkCommand::BroadcastProducerAnnouncements(vec![announcement(&KeyPair::from_seed(
                seed,
            ))])
        }
    }
}

/// Publish from `f` until `probe` holds (one more message per try).
async fn warm(
    f: &Fake3,
    what: &str,
    make: impl Fn(u32) -> NetworkCommand,
    probe: impl Fn() -> bool,
) {
    for i in 0..60u32 {
        if probe() {
            return;
        }
        f.cmd.send(make(i)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("setup: {what} never formed");
}

/// (honest attestations delivered at C, flood events at B, flood events at C).
async fn run(flood: Flood) -> (usize, usize, usize) {
    let member = KeyPair::from_seed([0x5C; 32]);
    let members: HashSet<_> = [*member.public_key()].into();
    let fake = |svc| spawn_fake3(svc, Mode::Judge, members.clone(), Duration::ZERO);
    let b_port = h::reserve_port();
    let b = fake(h::start_node(b_port, vec![]).await);
    let mut attackers = Vec::new();
    for _ in 0..LINKS {
        attackers.push(fake(
            h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await,
        ));
    }
    let c = fake(h::start_node(h::reserve_port(), vec![h::addr(b_port)]).await);
    let kind = match flood {
        Flood::Votes => Kind::Vote,
        Flood::Producers => Kind::Producers,
    };
    let t0 = now_secs();
    let m = member.clone();
    warm(
        &b,
        "B -> C attestations",
        |i| NetworkCommand::BroadcastAttestation(attestation(&m, 0x21, 10 + i).to_bytes()),
        || c.count_kind(Kind::Attestation) > 0,
    )
    .await;
    let bases: Vec<Vec<u8>> = (0..LINKS)
        .map(|j| {
            vote_json(
                &KeyPair::from_seed([0xE0 + j as u8; 32]),
                t0 - 10 - j as u64,
            )
        })
        .collect();
    for (j, a) in attackers.iter().enumerate() {
        let base = &bases[j];
        let warm_k = |i: u32| (1u64 << 20) + u64::from(i);
        warm(
            a,
            "attacker -> B -> C",
            |i| flood_msg(flood, j, warm_k(i), base),
            || {
                (0..60).any(|i| match flood_msg(flood, j, warm_k(i), base) {
                    NetworkCommand::BroadcastVote(v) => c.count(Kind::Vote, &v) > 0,
                    NetworkCommand::BroadcastProducerAnnouncements(anns) => {
                        c.count(Kind::Producers, &keys(&anns)) > 0
                    }
                    _ => false,
                })
            },
        )
        .await;
    }
    assert_eq!(
        c.peer_count().await,
        1,
        "topology: C must have one peer (B)"
    );
    let (b_before, c_before) = (b.count_kind(kind), c.count_kind(kind));
    for (j, a) in attackers.iter().enumerate() {
        let (cmd, base) = (a.cmd.clone(), bases[j].clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(1_000 / RATE_PER_LINK));
            for k in 0..RATE_PER_LINK * FLOOD_SECS {
                tick.tick().await;
                let _ = cmd.send(flood_msg(flood, j, k, &base)).await;
            }
        });
    }
    let honest: Vec<Vec<u8>> = (0..HONEST)
        .map(|s| attestation(&member, 0x70, 800 + s).to_bytes())
        .collect();
    tokio::time::sleep(Duration::from_secs(HONEST_START_SECS)).await;
    for a in &honest {
        b.cmd
            .send(NetworkCommand::BroadcastAttestation(a.clone()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let rest = FLOOD_SECS - HONEST_START_SECS - u64::from(HONEST);
    tokio::time::sleep(Duration::from_secs(rest + 3)).await;
    let delivered = honest
        .iter()
        .filter(|a| c.count(Kind::Attestation, a) >= 1)
        .count();
    let (b_flood, c_flood) = (b.count_kind(kind) - b_before, c.count_kind(kind) - c_before);
    println!(
        "decision C line: honest at C {delivered}/{HONEST}; flood events at B {b_flood}, at C {c_flood} (C budget {C_BUDGET})"
    );
    (delivered, b_flood, c_flood)
}

fn check(flood: &str, (delivered, b_flood, c_flood): (usize, usize, usize)) {
    assert!(
        b_flood > C_BUDGET * 3 / 2,
        "non-vacuity: B forwarded {b_flood} {flood} events, not above C's request budget {C_BUDGET}"
    );
    assert!(c_flood > 0, "non-vacuity: the {flood} flood reached C");
    assert_eq!(
        delivered, HONEST as usize,
        "decision C: a {flood} flood on B's link suppressed B's genuine attestations at C ({delivered}/{HONEST})"
    );
}

// REQ-GRV-SEC-002 / REQ-GRV-010 (decision C, AUDIT-P1-507 load) — Decision: a failure means hex-case vote variants forwarded by an honest B spend C's per-peer budget for B, so B's genuine attestations are Ignored at C (attendance erased downstream, soak #21).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_decision_c_vote_variant_flood_does_not_suppress_attestations_at_c() {
    check("vote", run(Flood::Votes).await);
}

// REQ-GRV-SEC-003 / REQ-GRV-010 (decision C) — Decision: a failure means a producer-announcement flood forwarded by an honest B spends C's per-peer budget for B, so B's genuine attestations are Ignored at C.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_decision_c_producer_flood_does_not_suppress_attestations_at_c() {
    check("producer", run(Flood::Producers).await);
}
