// OUTPUT CONTRACT: fn relay_decision(RelayTopic::Producers, &[u8], &StalenessCtx) -> RelayDecision;
//   fn on_producer_digest(...) driven through Node::handle_network_event(
//   NetworkEvent::ProducerDigestReceived { peer_id, digest, handle }) at B on a live
//   A -> B(node, producer x in its GSet) -> C line.
//   O1: none (bytes borrowed)
//   O2: B's producer_gset: unchanged by any digest; B stays live
//   O3: RelayDecision variant (Ignore / LocalOnly / Defer)
//   O4: none   O5: none
//   O6: SendProducerDelta, observed on the wire: A receives a producer set holding x
// PATHS: P1 honest digest from a peer lacking x -> LocalOnly, B answers with x;
//   P2 digest with size_bits > 0 and bad parameters -> Ignore, never reaches B's handler,
//      GSet unchanged, then P1 still answered;
//   P3 honest producer-set bytes -> Defer (unchanged)
// INPUT PARTITIONS: size_bits > 8*len {empty filter, 1-byte filter}; bloom_k {0, 17, u32::MAX};
//   honest digest {1, 5 producers}; honest producer set {1, 3 announcements}
// MATRIX: (O3, O2, O6) x P2 vectors + (O3, O6) x P1 + O3 x P3.
// covers: bloom.rs proto.rs relay_decision.rs network_events.rs

use std::time::Duration;

use crypto::{Hash, KeyPair};
use doli_core::{
    decode_digest, encode_digest, encode_producer_set, ProducerAnnouncement, ProducerGSet,
};
use network::gossip::staleness::StalenessCtx;
use network::gossip::{relay_decision, RelayDecision, RelayTopic};
use network::{GossipHandle, NetworkCommand, NetworkEvent};

use super::inc_i_237_m3_node_common as c;
use super::inc_i_237_m3_node_common::Ev;

fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// ProducerSetDigest wire bytes; zero / empty fields are omitted like prost does.
fn digest_bytes(filter: &[u8], k: u32, size_bits: u32) -> Vec<u8> {
    let mut out = Vec::new();
    if !filter.is_empty() {
        out.push(0x0a);
        varint(filter.len() as u64, &mut out);
        out.extend_from_slice(filter);
    }
    for (tag, v) in [(0x10u8, k), (0x20, size_bits)] {
        if v != 0 {
            out.push(tag);
            varint(u64::from(v), &mut out);
        }
    }
    out
}

/// (name, bytes, deliver-if-LocalOnly)
fn malformed() -> Vec<(&'static str, Vec<u8>, bool)> {
    vec![
        (
            "size_bits 65536, empty filter",
            digest_bytes(&[], 1, 65_536),
            true,
        ),
        (
            "size_bits 65536, 1-byte filter",
            digest_bytes(&[0xFF], 1, 65_536),
            true,
        ),
        ("bloom_k 17", digest_bytes(&[0u8; 8], 17, 64), true),
        ("bloom_k 0", digest_bytes(&[0u8; 8], 0, 64), true),
        (
            "bloom_k u32::MAX",
            digest_bytes(&[0u8; 8], u32::MAX, 64),
            false,
        ),
    ]
}

fn ctx() -> StalenessCtx {
    StalenessCtx {
        now_unix: c::now_secs(),
        genesis_time: 0,
        slot_duration: 10,
        best_slot: 0,
        best_height: 0,
        block_signature_activation_height: u64::MAX,
    }
}

async fn producer_line() -> c::Line {
    let mut l = c::line().await;
    let gh = l.b.chain_state.read().await.genesis_hash;
    let nid = l.b.config.network.id();
    l.warm(c::is_producers, move |i| {
        let kp = KeyPair::from_seed([i as u8 + 1; 32]);
        NetworkCommand::BroadcastProducerAnnouncements(vec![ProducerAnnouncement::new(
            &kp, nid, 1, gh,
        )])
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    l
}

fn local_gset(n: u8) -> ProducerGSet {
    let mut g = ProducerGSet::new(1, Hash::ZERO);
    for i in 0..n {
        let kp = KeyPair::from_seed([0xC0 + i; 32]);
        g.merge_one(ProducerAnnouncement::new(&kp, 1, 1, Hash::ZERO))
            .expect("setup: valid announcement");
    }
    g
}

// REQ-VOTE-003 — Decision: a failure means a digest with bad bloom parameters reaches the node's delta handler, or an honest digest after one gets no delta reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn producer_digest_with_bad_parameters_is_ignored_and_node_keeps_answering() {
    let mut l = producer_line().await;
    let (x, k) = (KeyPair::generate(), KeyPair::generate());
    let ax = c::announcement(&l.b, &x, 1).await;
    l.b.producer_gset
        .write()
        .await
        .merge_one(ax)
        .expect("B knows x");
    let before = l.b.producer_gset.read().await.sorted_producers();

    for (name, bytes, deliver) in malformed() {
        let decision = relay_decision(RelayTopic::Producers, &bytes, &ctx());
        if let (RelayDecision::LocalOnly, Ok(digest), true) =
            (&decision, decode_digest(&bytes), deliver)
        {
            let ev = NetworkEvent::ProducerDigestReceived {
                peer_id: l.a_peer,
                digest,
                handle: GossipHandle::now(),
            };
            let _ = l.b.handle_network_event(ev).await;
        }
        assert!(
            matches!(decision, RelayDecision::Ignore),
            "{name}: relay decision {decision:?}, want Ignore"
        );
    }
    assert_eq!(
        l.b.producer_gset.read().await.sorted_producers(),
        before,
        "a digest changed B's GSet"
    );

    let gh = l.b.chain_state.read().await.genesis_hash;
    let mut theirs = ProducerGSet::new(l.b.config.network.id(), gh);
    theirs
        .merge_one(c::announcement(&l.b, &k, 1).await)
        .expect("digest content");
    let honest = encode_digest(&theirs.to_bloom_filter());
    let decision = relay_decision(RelayTopic::Producers, &honest, &ctx());
    assert!(
        matches!(decision, RelayDecision::LocalOnly),
        "honest digest: {decision:?}"
    );
    let x_pk = *x.public_key();
    let has_x = |e: &Ev| matches!(e, Ev::Producers(p) if p.contains(&x_pk));
    let answered_before = c::count_where(&l.a_log, has_x);
    l.send(NetworkCommand::BroadcastProducerDigest(
        theirs.to_bloom_filter(),
    ))
    .await;
    c::pump(&mut l.b, Duration::from_millis(3_000)).await;
    assert!(
        c::count_where(&l.a_log, has_x) > answered_before,
        "B must answer the honest digest with the delta it lacks"
    );
}

// REQ-VOTE-003 — Decision: a failure means the digest bound changed the relay decision of honest digests or honest producer sets.
#[test]
fn honest_producer_gossip_keeps_its_relay_decision() {
    for n in [1u8, 5] {
        let bytes = encode_digest(&local_gset(n).to_bloom_filter());
        assert!(decode_digest(&bytes).is_ok(), "{n}-producer digest decodes");
        let decision = relay_decision(RelayTopic::Producers, &bytes, &ctx());
        assert!(
            matches!(decision, RelayDecision::LocalOnly),
            "{n}-producer digest: {decision:?}"
        );
    }
    for n in [1u8, 3] {
        let anns = local_gset(n).export_all();
        assert_eq!(anns.len(), n as usize, "setup: {n} announcements");
        let decision = relay_decision(RelayTopic::Producers, &encode_producer_set(&anns), &ctx());
        assert!(
            matches!(decision, RelayDecision::Defer),
            "{n}-announcement producer set: {decision:?}"
        );
    }
}
