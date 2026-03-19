use super::*;

#[test]
fn test_topic_constants() {
    assert_eq!(BLOCKS_TOPIC, "/doli/blocks/1");
    assert_eq!(TRANSACTIONS_TOPIC, "/doli/txs/1");
    assert_eq!(PRODUCERS_TOPIC, "/doli/producers/1");
    assert_eq!(VOTES_TOPIC, "/doli/votes/1");
    assert_eq!(HEARTBEATS_TOPIC, "/doli/heartbeats/1");
    assert_eq!(TIER1_BLOCKS_TOPIC, "/doli/t1/blocks/1");
    assert_eq!(HEADERS_TOPIC, "/doli/headers/1");
    assert_eq!(ATTESTATION_TOPIC, "/doli/attestations/1");
}

#[test]
fn test_region_topic_format() {
    assert_eq!(region_topic(0), "/doli/r0/blocks/1");
    assert_eq!(region_topic(1), "/doli/r1/blocks/1");
    assert_eq!(region_topic(42), "/doli/r42/blocks/1");
}


#[test]
fn test_mesh_config_default() {
    let config = MeshConfig {
        mesh_n: 6,
        mesh_n_low: 4,
        mesh_n_high: 12,
        gossip_lazy: 6,
    };
    assert!(config.mesh_n >= config.mesh_n_low);
    assert!(config.mesh_n <= config.mesh_n_high);
}

#[test]
fn test_gossip_error_display() {
    let e = GossipError::Config("bad config".into());
    assert!(e.to_string().contains("bad config"));
    let e = GossipError::Subscribe("topic failed".into());
    assert!(e.to_string().contains("topic failed"));
}

#[test]
fn test_tx_batch_roundtrip() {
    let tx1 = doli_core::Transaction::new_coinbase(100, crypto::Hash::ZERO, 0);
    let tx2 = doli_core::Transaction::new_coinbase(200, crypto::Hash::ZERO, 1);

    let encoded = encode_tx_batch(&[tx1.clone(), tx2.clone()]);
    assert_eq!(encoded[0], TX_MSG_BATCH);

    let decoded = decode_tx_message(&encoded).expect("decode should succeed");
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].hash(), tx1.hash());
    assert_eq!(decoded[1].hash(), tx2.hash());
}

#[test]
fn test_tx_single_legacy_decode() {
    let tx = doli_core::Transaction::new_coinbase(500, crypto::Hash::ZERO, 42);
    let raw = tx.serialize();

    let decoded = decode_tx_message(&raw).expect("legacy decode should succeed");
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].hash(), tx.hash());
}

#[test]
fn test_tx_batch_empty_returns_none() {
    assert!(decode_tx_message(&[]).is_none());
    // Batch prefix with count=0
    let mut data = vec![TX_MSG_BATCH];
    data.extend_from_slice(&0u32.to_le_bytes());
    assert!(decode_tx_message(&data).is_none());
}

#[test]
fn test_topics_for_tier0_is_legacy() {
    let topics = topics_for_tier(0, None);
    assert!(topics.contains(&BLOCKS_TOPIC.to_string()));
    assert!(topics.contains(&TRANSACTIONS_TOPIC.to_string()));
    assert!(topics.contains(&HEARTBEATS_TOPIC.to_string()));
    assert!(topics.contains(&HEADERS_TOPIC.to_string()));
    assert!(topics.contains(&PRODUCERS_TOPIC.to_string()));
    assert!(topics.contains(&VOTES_TOPIC.to_string()));
    // Tier 0 should NOT have tier1-specific topics
    assert!(!topics.contains(&TIER1_BLOCKS_TOPIC.to_string()));
    // But all tiers receive attestations for finality tracking
    assert!(topics.contains(&ATTESTATION_TOPIC.to_string()));
}

#[test]
fn test_topics_for_tier1_includes_tier1_blocks() {
    let topics = topics_for_tier(1, None);
    assert!(topics.contains(&TIER1_BLOCKS_TOPIC.to_string()));
    assert!(topics.contains(&ATTESTATION_TOPIC.to_string()));
    assert!(topics.contains(&BLOCKS_TOPIC.to_string()));
    assert_eq!(topics.len(), 8); // producers, votes, blocks, txs, heartbeats, t1_blocks, attestations, headers
}

#[test]
fn test_topics_for_tier3_headers_only() {
    let topics = topics_for_tier(3, None);
    assert!(topics.contains(&HEADERS_TOPIC.to_string()));
    assert!(topics.contains(&PRODUCERS_TOPIC.to_string()));
    assert!(topics.contains(&VOTES_TOPIC.to_string()));
    assert!(topics.contains(&ATTESTATION_TOPIC.to_string()));
    assert!(!topics.contains(&BLOCKS_TOPIC.to_string()));
    assert!(!topics.contains(&TRANSACTIONS_TOPIC.to_string()));
    assert!(!topics.contains(&HEARTBEATS_TOPIC.to_string()));
    assert_eq!(topics.len(), 4);
}

#[test]
fn test_reconfigure_tier_unsubscribes() {
    let keypair = libp2p::identity::Keypair::generate_ed25519();
    let mesh = MeshConfig { mesh_n: 6, mesh_n_low: 4, mesh_n_high: 12, gossip_lazy: 6 };
    let mut gs = new_gossipsub(&keypair, &mesh).unwrap();

    // Start with Tier 0 (legacy) subscriptions
    subscribe_to_topics_for_tier(&mut gs, 0, None).unwrap();
    let initial_count = gs.topics().count();
    assert_eq!(initial_count, 7); // blocks, txs, heartbeats, headers, attestations, producers, votes

    // Reconfigure to Tier 3 (header-only)
    // BLOCKS_TOPIC and TRANSACTIONS_TOPIC are protected — never unsubscribed
    reconfigure_topics_for_tier(&mut gs, 3, None).unwrap();
    let final_count = gs.topics().count();
    // headers + attestations + producers + votes + blocks(protected) + txs(protected) = 6
    assert_eq!(final_count, 6);
}

#[test]
fn test_protected_topics_never_unsubscribed() {
    let keypair = libp2p::identity::Keypair::generate_ed25519();
    let mesh = MeshConfig { mesh_n: 6, mesh_n_low: 4, mesh_n_high: 12, gossip_lazy: 6 };
    let mut gs = new_gossipsub(&keypair, &mesh).unwrap();

    // Subscribe to all Tier 0 topics
    subscribe_to_topics_for_tier(&mut gs, 0, None).unwrap();

    // Even reconfiguring to Tier 3 must keep blocks and txs
    reconfigure_topics_for_tier(&mut gs, 3, None).unwrap();

    let subscribed: Vec<String> = gs.topics().map(|t| t.to_string()).collect();
    assert!(
        subscribed.contains(&BLOCKS_TOPIC.to_string()),
        "BLOCKS_TOPIC must never be unsubscribed"
    );
    assert!(
        subscribed.contains(&TRANSACTIONS_TOPIC.to_string()),
        "TRANSACTIONS_TOPIC must never be unsubscribed"
    );
}

#[test]
fn test_dynamic_mesh_fallback_for_zero_producers() {
    let m = compute_dynamic_mesh(0);
    assert_eq!(m.mesh_n, 6);
    assert_eq!(m.mesh_n_low, 4);
    assert_eq!(m.mesh_n_high, 12);
    assert_eq!(m.gossip_lazy, 6);
}

#[test]
fn test_dynamic_mesh_fallback_for_one_producer() {
    let m = compute_dynamic_mesh(1);
    assert_eq!(m.mesh_n, 6);
}

#[test]
fn test_dynamic_mesh_small_network() {
    // Small networks (≤20): full mesh (total_peers - 1)
    let m = compute_dynamic_mesh(3);
    assert_eq!(m.mesh_n, 6); // min 6
    assert_eq!(m.mesh_n_low, 4);

    let m = compute_dynamic_mesh(5);
    assert_eq!(m.mesh_n, 6); // min 6

    let m = compute_dynamic_mesh(10);
    assert_eq!(m.mesh_n, 9);

    let m = compute_dynamic_mesh(15);
    assert_eq!(m.mesh_n, 14);

    let m = compute_dynamic_mesh(20);
    assert_eq!(m.mesh_n, 19);
}

#[test]
fn test_dynamic_mesh_large_network_sqrt_scaling() {
    // Large networks (>20): sqrt(N) * 1.5
    // 50 peers: sqrt(50)*1.5 = 10.6 → 11
    let m = compute_dynamic_mesh(50);
    assert_eq!(m.mesh_n, 11);

    // 106 peers: sqrt(106)*1.5 = 15.4 → 16
    let m = compute_dynamic_mesh(106);
    assert_eq!(m.mesh_n, 16);

    // 200 peers: sqrt(200)*1.5 = 21.2 → 22
    let m = compute_dynamic_mesh(200);
    assert_eq!(m.mesh_n, 22);

    // 1000 peers: sqrt(1000)*1.5 = 47.4 → 48
    let m = compute_dynamic_mesh(1000);
    assert_eq!(m.mesh_n, 48);

    // 2000 peers: capped at 50
    let m = compute_dynamic_mesh(2000);
    assert_eq!(m.mesh_n, 50);
}

#[test]
fn test_dynamic_mesh_invariants() {
    for n in 0..=2000 {
        let m = compute_dynamic_mesh(n);
        assert!(m.mesh_n_low >= 1, "mesh_n_low must be >= 1 for n={}", n);
        assert!(m.mesh_n_low <= m.mesh_n, "mesh_n_low <= mesh_n for n={}", n);
        assert!(
            m.mesh_n <= m.mesh_n_high,
            "mesh_n <= mesh_n_high for n={}",
            n
        );
        assert!(m.gossip_lazy >= 6, "gossip_lazy >= 6 for n={}", n);
    }
}

#[test]
fn test_dynamic_mesh_gossipsub_creation() {
    let keypair = libp2p::identity::Keypair::generate_ed25519();
    for n in [0, 1, 3, 5, 10, 15, 21, 50, 100] {
        let mesh = compute_dynamic_mesh(n);
        let gs = new_gossipsub(&keypair, &mesh);
        assert!(gs.is_ok(), "gossipsub creation must succeed for n={}", n);
    }
}
