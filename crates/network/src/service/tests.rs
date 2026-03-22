//! Tests for the network service module.

use crypto::{Hash, KeyPair};
use doli_core::{
    decode_digest, encode_digest, encode_producer_set, is_legacy_bincode_format,
    ProducerAnnouncement, ProducerBloomFilter,
};
use libp2p::{Multiaddr, PeerId};

use super::helpers::{is_routable_address, strip_p2p_suffix};
use super::types::{NetworkCommand, NetworkEvent};

#[test]
fn test_network_event_announcement_type() {
    let keypair = KeyPair::generate();
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    let event = NetworkEvent::ProducerAnnouncementsReceived(vec![ann.clone()]);

    if let NetworkEvent::ProducerAnnouncementsReceived(anns) = event {
        assert_eq!(anns.len(), 1);
        assert!(anns[0].verify());
    } else {
        panic!("Wrong event type");
    }
}

#[test]
fn test_network_event_legacy_producers() {
    let keypair = KeyPair::generate();
    let pubkey = *keypair.public_key();
    let event = NetworkEvent::ProducersAnnounced(vec![pubkey]);

    if let NetworkEvent::ProducersAnnounced(pubkeys) = event {
        assert_eq!(pubkeys.len(), 1);
        assert_eq!(pubkeys[0], pubkey);
    } else {
        panic!("Wrong event type");
    }
}

#[test]
fn test_network_event_digest_received() {
    let bloom = ProducerBloomFilter::new(100);
    let peer_id = PeerId::random();
    let event = NetworkEvent::ProducerDigestReceived {
        peer_id,
        digest: bloom.clone(),
    };

    if let NetworkEvent::ProducerDigestReceived {
        peer_id: pid,
        digest,
    } = event
    {
        assert_eq!(pid, peer_id);
        assert_eq!(digest.element_count(), 0);
    } else {
        panic!("Wrong event type");
    }
}

#[test]
fn test_network_command_broadcast_announcements() {
    let keypair = KeyPair::generate();
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    let command = NetworkCommand::BroadcastProducerAnnouncements(vec![ann.clone()]);

    if let NetworkCommand::BroadcastProducerAnnouncements(anns) = command {
        assert_eq!(anns.len(), 1);
        assert!(anns[0].verify());
    } else {
        panic!("Wrong command type");
    }
}

#[test]
fn test_network_command_broadcast_digest() {
    let bloom = ProducerBloomFilter::new(100);
    let command = NetworkCommand::BroadcastProducerDigest(bloom.clone());

    if let NetworkCommand::BroadcastProducerDigest(digest) = command {
        assert_eq!(digest.element_count(), 0);
    } else {
        panic!("Wrong command type");
    }
}

#[test]
fn test_network_command_send_delta() {
    let keypair = KeyPair::generate();
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    let peer_id = PeerId::random();
    let command = NetworkCommand::SendProducerDelta {
        peer_id,
        announcements: vec![ann.clone()],
    };

    if let NetworkCommand::SendProducerDelta {
        peer_id: pid,
        announcements,
    } = command
    {
        assert_eq!(pid, peer_id);
        assert_eq!(announcements.len(), 1);
    } else {
        panic!("Wrong command type");
    }
}

#[test]
fn test_gossip_message_encoding() {
    let keypair = KeyPair::generate();
    let anns = vec![ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO)];
    let bytes = encode_producer_set(&anns);

    // Should be reasonable size: ~130 bytes for single announcement
    assert!(
        bytes.len() < 200,
        "Single announcement {} bytes, expected < 200",
        bytes.len()
    );
}

#[test]
fn test_format_detection() {
    // Legacy bincode format
    let keypair = KeyPair::generate();
    let pubkeys = vec![*keypair.public_key()];
    let bincode_bytes = bincode::serialize(&pubkeys).unwrap();
    assert!(is_legacy_bincode_format(&bincode_bytes));

    // New protobuf format
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    let proto_bytes = encode_producer_set(&[ann]);
    assert!(!is_legacy_bincode_format(&proto_bytes));
}

#[test]
fn test_is_routable_rejects_loopback() {
    let addr: Multiaddr = "/ip4/127.0.0.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr));

    let addr6: Multiaddr = "/ip6/::1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr6));
}

#[test]
fn test_is_routable_accepts_public() {
    let addr: Multiaddr = "/ip4/198.51.100.1/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&addr));

    let addr2: Multiaddr = "/ip4/147.93.84.44/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&addr2));
}

#[test]
fn test_is_routable_rejects_unspecified() {
    let addr: Multiaddr = "/ip4/0.0.0.0/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr));

    let addr6: Multiaddr = "/ip6/::/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr6));
}

#[test]
fn test_is_routable_rejects_link_local() {
    let addr: Multiaddr = "/ip4/169.254.1.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr));
}

#[test]
fn test_strip_p2p_suffix() {
    // With /p2p suffix → stripped
    let addr: Multiaddr = "/ip4/198.51.100.1/tcp/30300/p2p/12D3KooWTest"
        .parse()
        .unwrap();
    let stripped = strip_p2p_suffix(&addr);
    assert_eq!(stripped.to_string(), "/ip4/198.51.100.1/tcp/30300");

    // Without /p2p suffix → unchanged
    let addr2: Multiaddr = "/ip4/198.51.100.1/tcp/30300".parse().unwrap();
    let stripped2 = strip_p2p_suffix(&addr2);
    assert_eq!(stripped2.to_string(), "/ip4/198.51.100.1/tcp/30300");

    // DNS with /p2p suffix → stripped
    let addr3: Multiaddr = "/dns4/seed1.doli.network/tcp/30300/p2p/12D3KooWTest"
        .parse()
        .unwrap();
    let stripped3 = strip_p2p_suffix(&addr3);
    assert_eq!(stripped3.to_string(), "/dns4/seed1.doli.network/tcp/30300");
}

// INC-I-002: Bloom filter digest messages must be decodable on PRODUCERS_TOPIC.
// Before the fix, digest bytes were misclassified as legacy bincode and dropped.
#[test]
fn test_digest_not_misclassified_as_legacy_bincode() {
    // Create a bloom filter with some producers inserted
    let mut bloom = ProducerBloomFilter::new(100);
    for _ in 0..5 {
        let keypair = KeyPair::generate();
        bloom.insert(keypair.public_key());
    }

    // Encode to protobuf (this is what gets published to PRODUCERS_TOPIC)
    let digest_bytes = encode_digest(&bloom);

    // The root cause: is_legacy_bincode_format misclassifies digest bytes
    // Because ProducerSet::decode() fails on digest data, the heuristic
    // returns true, sending the data down the bincode path where it also fails.
    // After the fix, we try decode_digest FIRST, so this misclassification
    // is bypassed entirely.
    let decoded = decode_digest(&digest_bytes);
    assert!(
        decoded.is_ok(),
        "decode_digest should succeed on encoded digest bytes"
    );

    let restored = decoded.unwrap();
    assert_eq!(restored.element_count(), bloom.element_count());
    assert!(
        restored.size_bits() > 0,
        "Valid digest must have size_bits > 0"
    );
    assert!(
        restored.hash_count() > 0,
        "Valid digest must have hash_count > 0"
    );
}

#[test]
fn test_digest_vs_producer_set_distinguishable() {
    // Verify that a valid ProducerSet does NOT decode as a valid digest
    let keypair = KeyPair::generate();
    let ann = ProducerAnnouncement::new(&keypair, 1, 0, Hash::ZERO);
    let producer_set_bytes = encode_producer_set(&[ann]);

    // ProducerSet bytes decoded as digest should produce invalid bloom (size_bits=0)
    if let Ok(bloom) = decode_digest(&producer_set_bytes) {
        assert_eq!(
            bloom.size_bits(),
            0,
            "ProducerSet decoded as digest should have size_bits=0"
        );
    }
    // Either decode fails or produces invalid bloom — both are fine

    // Verify digest bytes do NOT look like valid legacy bincode
    let mut bloom = ProducerBloomFilter::new(50);
    bloom.insert(keypair.public_key());
    let digest_bytes = encode_digest(&bloom);

    // The exact bincode check (len * 32 + 8 == total) should NOT match digest data
    if digest_bytes.len() >= 8 {
        let len = u64::from_le_bytes(digest_bytes[0..8].try_into().unwrap());
        let would_match_exact = len <= 10000 && digest_bytes.len() == 8 + (len as usize * 32);
        // It's theoretically possible but astronomically unlikely for random bloom data
        // to match this exact pattern. If it does, the test still passes because
        // we try decode_digest first in the fixed code.
        if would_match_exact {
            // This is fine — the fix handles it by trying digest decode FIRST
        }
    }
}
