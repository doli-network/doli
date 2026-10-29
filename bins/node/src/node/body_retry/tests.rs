use super::*;

fn h(b: &[u8]) -> Hash {
    crypto::hash::hash(b)
}

// OUTPUT CONTRACT: BodyRetry::refuses, take_attempt, answered, supplier_of.
// INPUT PARTITIONS: failed peer | other peer asked (gate live / timed out / no gate) | asked peer | bound.
#[test]
fn refuses_failed_and_unasked_peers_until_the_ask_times_out() {
    let (x, a, b) = (h(b"x"), PeerId::random(), PeerId::random());
    let t0 = Instant::now();
    let mut r = BodyRetry::default();
    assert!(!r.refuses(&x, &a, t0));
    r.fail(x, a, Fault::Failed);
    assert!(r.refuses(&x, &a, t0));
    r.ask(x, b, t0, true);
    let c = PeerId::random();
    assert!(r.refuses(&x, &c, t0));
    assert!(!r.refuses(&x, &b, t0));
    assert!(!r.refuses(&x, &c, t0 + ASK_TIMEOUT));
    assert!(!r.answered(&x, &c));
    assert!(r.answered(&x, &b));
    assert!(!r.refuses(&x, &c, t0));
    r.ask(x, b, t0, false);
    assert!(!r.refuses(&x, &c, t0) && r.asking(&x, t0));
}

#[test]
fn attempts_are_bounded_per_key_and_suppliers_match_by_content() {
    let mut r = BodyRetry::default();
    let k = RetryKey::Height(9);
    assert_eq!(
        (1..=4).map(|_| r.take_attempt(k)).collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(3), None]
    );
    assert_eq!(r.take_attempt(RetryKey::Hash(h(b"y"))), Some(1));
    let (x, p) = (h(b"x"), PeerId::random());
    r.note_supplier(x, h(b"k1"), p, 7);
    assert_eq!(r.supplier_of(&x, &h(b"k1")), Some((p, 7)));
    assert_eq!(r.supplier_of(&x, &h(b"k2")), None);
}

#[test]
fn pick_prefers_peers_at_the_height_and_skips_excluded() {
    let (lo, hi, bad) = (PeerId::random(), PeerId::random(), PeerId::random());
    let peers = vec![(lo, 5), (hi, 12), (bad, 20)];
    assert_eq!(pick_peer(peers.clone(), Some(10), |p| *p == bad), Some(hi));
    assert_eq!(pick_peer(peers.clone(), Some(30), |p| *p == bad), Some(hi));
    assert_eq!(pick_peer(vec![(bad, 20)], Some(10), |p| *p == bad), None);
}

fn bare(slot: u32) -> Block {
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash: h(b"inc-i-281-mb-parent"),
        merkle_root: doli_core::block::compute_merkle_root(&[]),
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: slot as u64 * 10,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

async fn node() -> (Node, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let keys = vec![KeyPair::generate()];
    let mut node = Node::new_for_test(tmp.path().to_path_buf(), keys)
        .await
        .expect("Node::new_for_test");
    node.inc_i_178_attestation_bls_activation_height = u64::MAX;
    (node, tmp)
}

fn invalid_blocks(node: &Node, p: &PeerId) -> usize {
    node.bls_ingress_scorer.get_score(p).map_or(0, |s| {
        s.infractions
            .iter()
            .filter(|(_, i)| matches!(i, network::Infraction::InvalidBlock { .. }))
            .count()
    })
}

// OUTPUT CONTRACT: cache_walked -> cached copy, supplier record, body refusal count.
// INPUT PARTITIONS: failed supplier, slot held | failed supplier, empty slot | other supplier.
#[tokio::test]
async fn cache_walked_replaces_a_different_copy_unless_its_supplier_failed_it() {
    let (mut node, _tmp) = node().await;
    let b = bare(5);
    let mut v = b.clone();
    v.aggregate_bls_signature = vec![1];
    let (bad, good) = (PeerId::random(), PeerId::random());
    node.fork_block_cache
        .write()
        .await
        .insert(b.hash(), v.clone());
    node.body_retry.fail(b.hash(), bad, Fault::Failed);
    let walk = |p: PeerId| network::sync::CompletedRecovery {
        blocks: vec![b.clone()],
        connection_point: h(b"inc-i-281-mb-unknown"),
        suppliers: vec![Some(p)],
    };
    node.cache_walked(&walk(bad)).await;
    assert_eq!(node.fork_block_cache.read().await.get(&b.hash()), Some(&v));
    let refusals = node.refused_reorg_blocks.body_refusals();
    node.fork_block_cache.write().await.clear();
    node.cache_walked(&walk(bad)).await;
    assert!(node.fork_block_cache.read().await.is_empty());
    assert_eq!(node.refused_reorg_blocks.body_refusals(), refusals + 1);
    node.fork_block_cache
        .write()
        .await
        .insert(b.hash(), v.clone());
    node.cache_walked(&walk(good)).await;
    assert_eq!(node.fork_block_cache.read().await.get(&b.hash()), Some(&b));
    let key = content_key(&b);
    assert_eq!(
        node.body_retry.supplier_of(&b.hash(), &key),
        Some((good, 5))
    );
}

// OUTPUT CONTRACT: screen_peer_copy -> bool, scorer infractions, refetch ask.
// INPUT PARTITIONS: another peer asked | asked peer | junk body, an ask open.
#[tokio::test]
async fn screen_refuses_unasked_peers_and_fails_junk_with_no_peer_to_ask() {
    let (mut node, _tmp) = node().await;
    let b = bare(6);
    let (asked, other) = (PeerId::random(), PeerId::random());
    node.body_retry.ask(b.hash(), asked, Instant::now(), true);
    node.body_retry.intake = Some((b.hash(), other));
    assert!(!node.screen_peer_copy(&b, None).await);
    assert_eq!(invalid_blocks(&node, &other), 0);
    node.body_retry.intake = Some((b.hash(), asked));
    assert!(node.screen_peer_copy(&b, None).await);
    let mut junk = b.clone();
    junk.attestation_bitfield = vec![1];
    assert!(!node.screen_peer_copy(&junk, None).await);
    assert_eq!(invalid_blocks(&node, &asked), 1);
    assert!(node.body_retry.never_ask(&b.hash(), &asked));
    assert!(node
        .body_retry
        .asked
        .iter()
        .all(|e| e.0 != b.hash() || e.1 == asked));
}

// OUTPUT CONTRACT: screen_peer_copy on junk -> scorer record, never-ask, no copy refusal,
//   refetch (no gate) only when no copy is held and no ask is open.
// INPUT PARTITIONS: copy held | nothing held, no ask | the supplier's later honest copy | ask open.
#[tokio::test]
async fn junk_is_recorded_never_asked_and_does_not_refuse_later_copies() {
    let (mut node, _tmp) = node().await;
    let b = bare(7);
    let (relay, other) = (PeerId::random(), PeerId::random());
    {
        let mut sm = node.sync_manager.write().await;
        sm.add_peer(relay, 10, b.hash(), 7);
        sm.add_peer(other, 10, b.hash(), 7);
    }
    let mut junk = b.clone();
    junk.attestation_bitfield = vec![1];
    node.fork_block_cache
        .write()
        .await
        .insert(b.hash(), b.clone());
    node.body_retry.intake = Some((b.hash(), relay));
    assert!(!node.screen_peer_copy(&junk, None).await);
    assert!(node.body_retry.asked.is_empty());
    node.fork_block_cache.write().await.clear();
    assert!(!node.screen_peer_copy(&junk, None).await);
    assert_eq!(invalid_blocks(&node, &relay), 2);
    assert!(node.body_retry.never_ask(&b.hash(), &relay));
    assert!(!node.body_retry.has_failed(&b.hash(), &relay));
    let asks: Vec<_> = node
        .body_retry
        .asked
        .iter()
        .map(|e| (e.0, e.1, e.3))
        .collect();
    assert_eq!(asks, vec![(b.hash(), other, false)]);
    assert!(node.screen_peer_copy(&b, None).await);
    assert!(!node.screen_peer_copy(&junk, None).await);
    assert_eq!(node.body_retry.asked.len(), 1);
}
