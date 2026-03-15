//! Tests for the network service module.

use crypto::{Hash, KeyPair};
use doli_core::{
    encode_producer_set, is_legacy_bincode_format, ProducerAnnouncement, ProducerBloomFilter,
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
