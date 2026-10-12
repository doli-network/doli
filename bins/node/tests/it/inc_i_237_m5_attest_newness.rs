//! INC-I-237 M5 (AUDIT-P1-506) — attestation relay newness keyed on
//! (attester, slot): a verified member key earns at most one forward per slot,
//! plus one for the genuine BLS half the first time it pools.
//!
//! OUTPUT CONTRACT: Node::on_new_attestation(&mut self, data, source_peer) -> Verdict
//!   O1: none (data by value)   O2: self.parent_sig_pool, self.minute_tracker
//!   (unchanged from V), self.attest_seen (new)   O3: Verdict   O4/O5: none
//!   O6: attestation gossip at C on a fan-in A1, A2, H -> B(node) -> C line
//! PATHS: P1 first (attester, slot) -> Accept; P2 genuine BLS newly pooled ->
//!   Accept; P3 seen slot, nothing pooled -> Ignore; P4 seen slot, BLS already
//!   pooled -> Ignore; P5 unknown block, first -> Accept; P6 unknown block,
//!   junk hash on a seen slot -> Ignore; P7 mutant flood + honest member on
//!   another link -> family <= 2 at C, honest all at C
//! INPUT PARTITIONS: bls {wrong 96 B, genuine}; height {h, h+1, h+2}; block
//!   {known, unknown, junk hash}; links {2 attackers x 40/s x 30 s, honest 1/s}
//! MATRIX: O3 x P1-P6, O2 x P1-P4 (pool + attendance), O6 x P7 — asserted below.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crypto::{BlsKeyPair, Hash, KeyPair};
use doli_core::attestation::attestation_minute;
use doli_core::{Attestation, Network};
use doli_node::node::Node;
use network::service::Verdict;
use network::{NetworkCommand, PeerId};
use storage::ProducerInfo;

use super::inc_i_178_m0_common::{
    assemble, build_via_production, dual, make_node, register_bls, safe_build_height, N_SMALL,
};
use super::inc_i_237_m2_node_common as c2;
use super::inc_i_237_m3_node_common as c;
use super::inc_i_237_m3_node_common::Ev;
use super::inc_i_237_m6_common::set_wall;

async fn known_block(node: &mut Node) -> (Hash, u32, u64) {
    let h = safe_build_height(node);
    let (header, txs, bf) = build_via_production(node, h).await;
    let block = assemble(header, txs, bf);
    node.block_store
        .put_block_canonical(&block, h)
        .expect("put_block_canonical");
    (block.hash(), block.header.slot, h)
}

fn variant(a: &Attestation, height: u64, bls: Vec<u8>) -> Attestation {
    let mut m = a.clone();
    m.height = height;
    m.bls_signature = bls;
    m
}

// REQ-GRV-SEC-002 (AUDIT-P1-506) — Decision: a failure means one verified member attestation still mints unlimited relay Accepts (height / weight / BLS / junk-hash variants), or the newness rule hides the genuine BLS half that pools after a mutant spent the slot.
#[tokio::test]
async fn m5_audit_p1_506_second_variant_per_attester_slot_is_ignored() {
    let (mut node, producers, _t) = make_node(N_SMALL).await;
    let (hash, slot, height) = known_block(&mut node).await;
    let bls = BlsKeyPair::generate();
    let a = &producers[3];
    register_bls(&node, a.public_key(), &bls).await;
    let peer = PeerId::random();
    let genuine = dual(a, &bls, hash, slot, height);
    let blob: [u8; 96] = genuine.bls_signature.clone().try_into().unwrap();
    let unknown_slot = slot + 1;
    let first_unknown = Attestation::new(
        Hash::from_bytes([0xE7; 32]),
        unknown_slot,
        height,
        1,
        a.private_key(),
        *a.public_key(),
    );
    let junk_hash = Attestation::new(
        Hash::from_bytes([0xE8; 32]),
        unknown_slot,
        height,
        1,
        a.private_key(),
        *a.public_key(),
    );
    let rows = [
        (
            "wrong-BLS mutant first",
            variant(&genuine, height, vec![0xA5; 96]),
        ),
        ("genuine (BLS newly pooled)", genuine.clone()),
        (
            "mutant h+1, wrong BLS",
            variant(&genuine, height + 1, vec![0xA6; 96]),
        ),
        (
            "re-bytes h+2, genuine BLS",
            variant(&genuine, height + 2, blob.to_vec()),
        ),
        ("unknown block, first", first_unknown),
        ("unknown slot, junk hash", junk_hash),
    ];
    let mut got = Vec::new();
    for (i, (row, att)) in rows.iter().enumerate() {
        got.push((*row, node.on_new_attestation(att.to_bytes(), peer).await));
        if i == 1 {
            assert_eq!(
                node.parent_sig_pool.get(&hash, a.public_key()),
                Some(&blob),
                "the genuine BLS half must pool after a mutant spent the slot"
            );
            assert!(
                node.minute_tracker
                    .attested_in_minute(attestation_minute(slot))
                    .contains(&a.public_key()),
                "attendance is recorded exactly as at V"
            );
        }
    }
    assert_eq!(
        node.parent_sig_pool.get(&hash, a.public_key()),
        Some(&blob),
        "a later variant displaced the pooled genuine half"
    );
    assert_eq!(node.parent_sig_pool.total_signatures(), 1);
    let want = [
        Verdict::Accept,
        Verdict::Accept,
        Verdict::Ignore,
        Verdict::Ignore,
        Verdict::Accept,
        Verdict::Ignore,
    ];
    let expected: Vec<_> = rows.iter().map(|r| r.0).zip(want).collect();
    assert_eq!(got, expected, "AUDIT-P1-506: per-(attester, slot) verdicts");
}

const RATE_PER_LINK: u64 = 40;
const FLOOD_SECS: u64 = 30;
const FAMILY_SLOT: u32 = 700;
const HONEST_BASE: u32 = 800;

fn decoded(e: &Ev) -> Option<Attestation> {
    match e {
        Ev::Attestation(d) => Attestation::from_bytes(d),
        _ => None,
    }
}

async fn register_member(node: &Node, kp: &KeyPair) {
    let bond = Network::Devnet.bond_unit();
    let info = ProducerInfo::new(*kp.public_key(), 1_000_000, bond, (Hash::ZERO, 0), 0, bond);
    node.producer_set
        .write()
        .await
        .register(info, 1_000_000)
        .expect("register second member");
}

// REQ-GRV-SEC-002 / REQ-GRV-010 (AUDIT-P1-506) — Decision: a failure means attacker-minted Accepts of one member attestation drain C's per-peer bucket for the honest forwarder B, so a second member's genuine attestations are Ignored at C (attendance erased downstream).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn m5_audit_p1_506_mutant_flood_does_not_suppress_a_genuine_attester() {
    let (mut b, member, _t) = c::member_node().await;
    set_wall(&mut b, FAMILY_SLOT);
    let honest = KeyPair::generate();
    register_member(&b, &honest).await;
    let b_port = c2::reserve_port();
    b.network = Some(c2::start_net(b_port, vec![]).await);
    let mut attackers = Vec::new();
    for _ in 0..2 {
        let a = c2::start_net(c2::reserve_port(), vec![c2::addr(b_port)]).await;
        attackers.push(a.command_sender());
        c::observe(a);
    }
    let h = c2::start_net(c2::reserve_port(), vec![c2::addr(b_port)]).await;
    let h_cmd = h.command_sender();
    c::observe(h);
    let c_log = c::observe(c2::start_net(c2::reserve_port(), vec![c2::addr(b_port)]).await);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut i = 0;
    while c::count_where(&c_log, c::is_attestation) == 0 {
        assert!(Instant::now() < deadline, "setup: H -> B -> C never formed");
        let w = c::attestation(&honest, 0x51, 50 + i).to_bytes();
        h_cmd
            .send(NetworkCommand::BroadcastAttestation(w))
            .await
            .unwrap();
        i += 1;
        c::pump(&mut b, Duration::from_millis(500)).await;
    }

    let genuine = c::attestation(&member, 0x61, FAMILY_SLOT);
    let stop = Arc::new(AtomicBool::new(false));
    for (j, cmd) in attackers.into_iter().enumerate() {
        let (g, stop) = (genuine.clone(), stop.clone());
        tokio::spawn(async move {
            for k in 0..RATE_PER_LINK * FLOOD_SECS {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let n = (j as u64) << 32 | k;
                let mut bls = vec![0xB0; 96];
                bls[..8].copy_from_slice(&n.to_le_bytes());
                let m = variant(&g, 1_000 + n, bls).to_bytes();
                let _ = cmd.send(NetworkCommand::BroadcastAttestation(m)).await;
                tokio::time::sleep(Duration::from_millis(1_000 / RATE_PER_LINK)).await;
            }
        });
    }
    let honest_sent: Vec<Attestation> = (0..20)
        .map(|s| c::attestation(&honest, 0x70, HONEST_BASE + s))
        .collect();
    let hs = honest_sent.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(FLOOD_SECS - 20)).await;
        for a in hs {
            let _ = h_cmd
                .send(NetworkCommand::BroadcastAttestation(a.to_bytes()))
                .await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    c::pump(&mut b, Duration::from_secs(FLOOD_SECS)).await;
    stop.store(true, Ordering::Relaxed);
    c::pump(&mut b, Duration::from_secs(3)).await;

    let at_c: Vec<Attestation> = c_log.lock().unwrap().iter().filter_map(decoded).collect();
    let family = at_c
        .iter()
        .filter(|a| a.attester == *member.public_key() && a.slot == FAMILY_SLOT)
        .count();
    let per_honest: Vec<usize> = honest_sent
        .iter()
        .map(|h| c::count(&c_log, &Ev::Attestation(h.to_bytes())))
        .collect();
    let delivered = per_honest.iter().filter(|n| **n >= 1).count();
    println!(
        "AUDIT-P1-506 line: family at C = {family}, honest delivered {delivered}/20 {per_honest:?}"
    );
    assert_eq!(
        delivered,
        honest_sent.len(),
        "AUDIT-P1-506: the mutant flood suppressed a genuine attester at C ({delivered}/20 delivered)"
    );
    assert!(
        per_honest.iter().all(|n| *n == 1),
        "each honest attestation crosses B once: {per_honest:?}"
    );
    assert!(
        family <= 2,
        "AUDIT-P1-506: {family} variants of one (attester, slot) crossed B (bound 2)"
    );
}
