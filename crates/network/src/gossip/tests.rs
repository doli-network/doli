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
fn test_mesh_config_invariants() {
    let config = MeshConfig {
        mesh_n: 12,
        mesh_n_low: 8,
        mesh_n_high: 24,
        gossip_lazy: 12,
    };
    assert!(config.mesh_n >= config.mesh_n_low);
    assert!(config.mesh_n <= config.mesh_n_high);
    assert!(config.gossip_lazy >= config.mesh_n);
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
fn test_gossipsub_creation_with_universal_mesh() {
    let keypair = libp2p::identity::Keypair::generate_ed25519();
    let mesh = MeshConfig {
        mesh_n: 12,
        mesh_n_low: 8,
        mesh_n_high: 24,
        gossip_lazy: 12,
    };
    let gs = new_gossipsub(&keypair, &mesh);
    assert!(gs.is_ok(), "gossipsub creation must succeed");
}
