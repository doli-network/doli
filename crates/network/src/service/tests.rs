//! Tests for the network service module.

use crypto::{Hash, KeyPair};
use doli_core::{
    encode_producer_set, is_legacy_bincode_format, ProducerAnnouncement, ProducerBloomFilter,
};
use libp2p::{Multiaddr, PeerId};

use super::helpers::{
    all_addresses_routable, is_routable_address, plan_startup_dials,
    purge_non_routable_dht_addresses, strip_p2p_suffix,
};
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
    assert!(!is_routable_address(&addr, 1));

    let addr6: Multiaddr = "/ip6/::1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr6, 1));
}

#[test]
fn test_is_routable_accepts_public() {
    let addr: Multiaddr = "/ip4/198.51.100.1/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&addr, 1));

    let addr2: Multiaddr = "/ip4/147.93.84.44/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&addr2, 1));
}

#[test]
fn test_is_routable_rejects_unspecified() {
    let addr: Multiaddr = "/ip4/0.0.0.0/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr, 1));

    let addr6: Multiaddr = "/ip6/::/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr6, 1));
}

#[test]
fn test_is_routable_rejects_link_local() {
    let addr: Multiaddr = "/ip4/169.254.1.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr, 1));
}

#[test]
fn test_is_routable_rejects_rfc1918_private() {
    // 10.0.0.0/8
    let addr: Multiaddr = "/ip4/10.0.0.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr, 1));
    let addr2: Multiaddr = "/ip4/10.10.10.189/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr2, 1));

    // 172.16.0.0/12
    let addr3: Multiaddr = "/ip4/172.16.0.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr3, 1));
    let addr4: Multiaddr = "/ip4/172.31.255.255/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr4, 1));
    // 172.32.x should pass (outside /12 range)
    let addr5: Multiaddr = "/ip4/172.32.0.1/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&addr5, 1));

    // 192.168.0.0/16
    let addr6: Multiaddr = "/ip4/192.168.1.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr6, 1));
    let addr7: Multiaddr = "/ip4/192.168.134.128/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr7, 1));

    // Docker bridge IPs
    let addr8: Multiaddr = "/ip4/172.17.0.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr8, 1));
    let addr9: Multiaddr = "/ip4/172.18.0.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr9, 1));
}

#[test]
fn test_is_routable_rejects_cgnat_shared() {
    // RFC 6598 shared address space (100.64.0.0/10)
    let addr: Multiaddr = "/ip4/100.64.0.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr, 1));
    let addr2: Multiaddr = "/ip4/100.127.255.255/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&addr2, 1));

    // 100.128.x should pass (outside /10 range)
    let addr3: Multiaddr = "/ip4/100.128.0.1/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&addr3, 1));
}

#[test]
fn test_is_routable_testnet_allows_loopback_and_private() {
    // Testnet (network_id=2): loopback and private are allowed for localhost discovery
    let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&loopback, 2));

    let private: Multiaddr = "/ip4/192.168.1.1/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&private, 2));

    let ten: Multiaddr = "/ip4/10.0.0.1/tcp/30300".parse().unwrap();
    assert!(is_routable_address(&ten, 2));

    // Unspecified and link-local are still rejected on testnet
    let unspec: Multiaddr = "/ip4/0.0.0.0/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&unspec, 2));

    let link_local: Multiaddr = "/ip4/169.254.1.1/tcp/30300".parse().unwrap();
    assert!(!is_routable_address(&link_local, 2));
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

// ── INC-I-048: Bootstrap dial priority tests ──────────────────────────

/// Reproduces the bug: with the OLD ordering (cached first, bootstrap second),
/// 17 cached peers would fill all 5 pending slots, starving the bootstrap dial.
/// The fix ensures bootstrap comes first and cached dials are capped.
#[test]
fn test_inc_i048_bootstrap_dials_before_cached_peers() {
    let bootstrap = vec!["/ip4/127.0.0.1/tcp/30300".to_string()];
    // 17 cached peers — the exact scenario from N3's logs
    let cached: Vec<Multiaddr> = (1..=17)
        .map(|i| {
            format!("/ip4/192.168.1.{}/tcp/{}", i, 30300 + i)
                .parse()
                .unwrap()
        })
        .collect();
    let pending_limit = 5;

    let (boot_addrs, cached_addrs) = plan_startup_dials(&bootstrap, cached, pending_limit);

    // Bootstrap MUST be in the first batch (gets dialed first)
    assert_eq!(boot_addrs.len(), 1);
    assert_eq!(boot_addrs[0].to_string(), "/ip4/127.0.0.1/tcp/30300");

    // Cached peers MUST be capped to pending_limit - bootstrap_count = 4
    assert_eq!(cached_addrs.len(), 4);

    // Total dials (bootstrap + cached) must not exceed pending_limit
    assert!(boot_addrs.len() + cached_addrs.len() <= pending_limit as usize);
}

/// DNS multiaddrs in the cache must be filtered out — they consume pending
/// slots with 75s DNS timeouts on localhost.
#[test]
fn test_inc_i048_dns_filtered_from_cached_peers() {
    let bootstrap = vec!["/ip4/127.0.0.1/tcp/30300".to_string()];
    let cached: Vec<Multiaddr> = vec![
        "/ip4/192.168.1.1/tcp/30301".parse().unwrap(),
        "/dns4/bootstrap1.testnet.doli.network/tcp/40300"
            .parse()
            .unwrap(),
        "/dns4/bootstrap2.testnet.doli.network/tcp/40300"
            .parse()
            .unwrap(),
        "/dns6/seeds.testnet.doli.network/tcp/40300"
            .parse()
            .unwrap(),
        "/ip4/192.168.1.2/tcp/30302".parse().unwrap(),
    ];
    let pending_limit = 5;

    let (_, cached_addrs) = plan_startup_dials(&bootstrap, cached, pending_limit);

    // Only IP4 addresses should remain — all dns4/dns6 filtered
    assert_eq!(cached_addrs.len(), 2);
    for addr in &cached_addrs {
        let s = addr.to_string();
        assert!(!s.starts_with("/dns4/"), "DNS4 should be filtered: {}", s);
        assert!(!s.starts_with("/dns6/"), "DNS6 should be filtered: {}", s);
    }
}

/// With multiple bootstrap nodes, cached dial cap adjusts correctly.
#[test]
fn test_inc_i048_multiple_bootstrap_nodes_reduce_cached_cap() {
    let bootstrap = vec![
        "/ip4/127.0.0.1/tcp/30300".to_string(),
        "/ip4/10.0.0.1/tcp/30300".to_string(),
        "/ip4/10.0.0.2/tcp/30300".to_string(),
    ];
    let cached: Vec<Multiaddr> = (1..=10)
        .map(|i| {
            format!("/ip4/192.168.1.{}/tcp/{}", i, 30300 + i)
                .parse()
                .unwrap()
        })
        .collect();
    let pending_limit = 5;

    let (boot_addrs, cached_addrs) = plan_startup_dials(&bootstrap, cached, pending_limit);

    assert_eq!(boot_addrs.len(), 3);
    // 5 - 3 = 2 cached slots
    assert_eq!(cached_addrs.len(), 2);
    assert!(boot_addrs.len() + cached_addrs.len() <= pending_limit as usize);
}

/// Edge case: more bootstrap nodes than pending_limit — cached gets at least 1.
#[test]
fn test_inc_i048_bootstrap_exceeds_pending_limit() {
    let bootstrap: Vec<String> = (1..=7)
        .map(|i| format!("/ip4/10.0.0.{}/tcp/30300", i))
        .collect();
    let cached: Vec<Multiaddr> = (1..=5)
        .map(|i| {
            format!("/ip4/192.168.1.{}/tcp/{}", i, 30300 + i)
                .parse()
                .unwrap()
        })
        .collect();
    let pending_limit = 5;

    let (boot_addrs, cached_addrs) = plan_startup_dials(&bootstrap, cached, pending_limit);

    assert_eq!(boot_addrs.len(), 7);
    // saturating_sub(7).max(1) = 1 — always allow at least 1 cached dial
    assert_eq!(cached_addrs.len(), 1);
}

/// Empty cache — only bootstrap dials happen.
#[test]
fn test_inc_i048_empty_cache() {
    let bootstrap = vec!["/ip4/127.0.0.1/tcp/30300".to_string()];
    let cached: Vec<Multiaddr> = vec![];
    let pending_limit = 5;

    let (boot_addrs, cached_addrs) = plan_startup_dials(&bootstrap, cached, pending_limit);

    assert_eq!(boot_addrs.len(), 1);
    assert_eq!(cached_addrs.len(), 0);
}

/// P2P suffixes are stripped from both bootstrap and cached addrs.
#[test]
fn test_inc_i048_p2p_suffixes_stripped() {
    // Generate a real peer ID for the test
    let kp = libp2p::identity::Keypair::generate_ed25519();
    let pid = PeerId::from(kp.public());
    let bootstrap = vec![format!("/ip4/127.0.0.1/tcp/30300/p2p/{}", pid)];
    let cached_addr: Multiaddr = format!("/ip4/192.168.1.1/tcp/30301/p2p/{}", pid)
        .parse()
        .unwrap();
    let cached: Vec<Multiaddr> = vec![cached_addr];
    let pending_limit = 5;

    let (boot_addrs, cached_addrs) = plan_startup_dials(&bootstrap, cached, pending_limit);

    assert_eq!(boot_addrs[0].to_string(), "/ip4/127.0.0.1/tcp/30300");
    assert_eq!(cached_addrs[0].to_string(), "/ip4/192.168.1.1/tcp/30301");
}

// =========================================================================
// INC-I-050: Kademlia DHT address filter tests
// =========================================================================

// OUTPUT CONTRACT: fn all_addresses_routable(addrs, network_id)
// O1: return value (bool) — true if ALL addrs are routable for network_id, false otherwise
// O2: return value (bool) — false when addrs is empty (no addresses = not routable)
// PATHS: mainnet+loopback, mainnet+public, mainnet+mixed, testnet+loopback, empty
// MATRIX: O1×mainnet_loopback=false, O1×mainnet_public=true, O1×mainnet_mixed=false,
//         O1×testnet_loopback=true, O2×empty=false

#[test]
fn test_all_addresses_routable_rejects_loopback_on_mainnet() {
    // INC-I-050: Kademlia FIND_NODE responses propagate addresses without filtering.
    // A peer with 127.0.0.1 in its DHT entry must be rejected on mainnet.
    let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/30300".parse().unwrap();
    assert!(
        !all_addresses_routable([&loopback].into_iter(), 1),
        "Loopback address must be rejected on mainnet (network_id=1)"
    );
}

#[test]
fn test_all_addresses_routable_accepts_public_on_mainnet() {
    let public: Multiaddr = "/ip4/198.51.100.1/tcp/30300".parse().unwrap();
    assert!(
        all_addresses_routable([&public].into_iter(), 1),
        "Public address must be accepted on mainnet"
    );
}

#[test]
fn test_all_addresses_routable_rejects_mixed_on_mainnet() {
    // If a peer has both public and loopback addresses, the whole entry is tainted
    let public: Multiaddr = "/ip4/198.51.100.1/tcp/30300".parse().unwrap();
    let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/30300".parse().unwrap();
    assert!(
        !all_addresses_routable([&public, &loopback].into_iter(), 1),
        "Mixed routable+loopback must be rejected on mainnet"
    );
}

#[test]
fn test_all_addresses_routable_allows_loopback_on_testnet() {
    // Testnet allows loopback for localhost discovery
    let loopback: Multiaddr = "/ip4/127.0.0.1/tcp/30300".parse().unwrap();
    assert!(
        all_addresses_routable([&loopback].into_iter(), 2),
        "Loopback must be allowed on testnet (network_id=2)"
    );
}

#[test]
fn test_all_addresses_routable_rejects_empty() {
    // A peer with zero addresses should not be kept in the routing table
    let empty: Vec<&Multiaddr> = vec![];
    assert!(
        !all_addresses_routable(empty.into_iter(), 1),
        "Empty address list must be rejected"
    );
}

// =========================================================================
// INC-I-050 v2: Periodic DHT routing table purge tests
// =========================================================================
//
// The RoutingUpdated filter (f023eefd) only catches NEW peers entering the
// k-bucket. It misses non-routable addresses injected into EXISTING peers
// via libp2p's internal connection_updated() path (co-located nodes connect
// via 127.0.0.1, address enters routing table, propagates via FIND_NODE).
//
// The periodic purge scans the entire routing table and removes non-routable
// addresses, draining the network of poisoned entries over bootstrap cycles.

// OUTPUT CONTRACT: fn purge_non_routable_dht_addresses(kad, network_id) -> usize
// O1: return value (usize) — count of addresses removed
// O2: side effect — non-routable addresses removed from Kademlia routing table
// O3: side effect — peers with NO remaining routable addresses fully removed
// PATHS: peer_with_mixed_addrs, peer_with_only_loopback, clean_peer, empty_table, testnet
// MATRIX:
//   O1×mixed=1, O2×mixed=loopback_gone+public_stays, O3×mixed=peer_stays
//   O1×only_loopback=1, O2×only_loopback=addr_gone, O3×only_loopback=peer_removed
//   O1×clean=0, O2×clean=unchanged, O3×clean=peer_stays
//   O1×empty=0
//   O1×testnet=0 (loopback is routable on testnet)

use crate::discovery::kademlia::new_kademlia;

#[test]
fn test_inc_i050v2_purge_removes_loopback_from_mixed_peer() {
    // INC-I-050 v2: A peer on the same server has BOTH public and 127.0.0.1
    // addresses in the DHT (injected via connection_updated). The purge must
    // remove the loopback but keep the peer with its public address.
    let local_peer = PeerId::random();
    let mut kad = new_kademlia(local_peer);

    let remote_peer = PeerId::random();
    let public_addr: Multiaddr = "/ip4/198.51.100.1/tcp/30300".parse().unwrap();
    let loopback_addr: Multiaddr = "/ip4/127.0.0.1/tcp/30300".parse().unwrap();

    // Simulate connection_updated + Identify adding both addresses
    kad.add_address(&remote_peer, public_addr.clone());
    kad.add_address(&remote_peer, loopback_addr.clone());

    // Act: periodic purge on mainnet (network_id=1)
    let removed = purge_non_routable_dht_addresses(&mut kad, 1);

    // Assert: loopback removed, peer still present with public address
    assert!(removed > 0, "Should have removed non-routable address(es)");

    let mut peer_found = false;
    let mut has_loopback = false;
    let mut has_public = false;
    for bucket in kad.kbuckets() {
        for entry in bucket.iter() {
            if entry.node.key.preimage() == &remote_peer {
                peer_found = true;
                for addr in entry.node.value.iter() {
                    let s = addr.to_string();
                    if s.contains("127.0.0.1") {
                        has_loopback = true;
                    }
                    if s.contains("198.51.100.1") {
                        has_public = true;
                    }
                }
            }
        }
    }
    assert!(
        peer_found,
        "Peer should still be in DHT (has public address)"
    );
    assert!(has_public, "Public address should remain");
    assert!(!has_loopback, "Loopback address should have been purged");
}

#[test]
fn test_inc_i050v2_purge_removes_peer_with_only_loopback() {
    // A peer with ONLY a loopback address (the original poisoned peer) should
    // be fully removed from the routing table after purge.
    let local_peer = PeerId::random();
    let mut kad = new_kademlia(local_peer);

    let poisoned_peer = PeerId::random();
    let loopback_addr: Multiaddr = "/ip4/127.0.0.1/tcp/30300".parse().unwrap();
    kad.add_address(&poisoned_peer, loopback_addr);

    let removed = purge_non_routable_dht_addresses(&mut kad, 1);

    assert!(removed > 0, "Should have removed the loopback address");

    // Peer should be gone entirely (no routable addresses left)
    let mut peer_found = false;
    for bucket in kad.kbuckets() {
        for entry in bucket.iter() {
            if entry.node.key.preimage() == &poisoned_peer {
                peer_found = true;
            }
        }
    }
    assert!(
        !peer_found,
        "Peer with only loopback should be fully removed"
    );
}

#[test]
fn test_inc_i050v2_purge_skips_clean_peers() {
    // A peer with only public addresses should not be touched.
    let local_peer = PeerId::random();
    let mut kad = new_kademlia(local_peer);

    let clean_peer = PeerId::random();
    let public_addr: Multiaddr = "/ip4/198.51.100.1/tcp/30300".parse().unwrap();
    kad.add_address(&clean_peer, public_addr);

    let removed = purge_non_routable_dht_addresses(&mut kad, 1);

    assert_eq!(removed, 0, "Clean peer should not be touched");

    let mut peer_found = false;
    for bucket in kad.kbuckets() {
        for entry in bucket.iter() {
            if entry.node.key.preimage() == &clean_peer {
                peer_found = true;
            }
        }
    }
    assert!(peer_found, "Clean peer should remain in DHT");
}

#[test]
fn test_inc_i050v2_purge_allows_loopback_on_testnet() {
    // On testnet (network_id=2), loopback is routable — purge should not remove it.
    let local_peer = PeerId::random();
    let mut kad = new_kademlia(local_peer);

    let peer = PeerId::random();
    let loopback_addr: Multiaddr = "/ip4/127.0.0.1/tcp/30300".parse().unwrap();
    kad.add_address(&peer, loopback_addr);

    let removed = purge_non_routable_dht_addresses(&mut kad, 2);

    assert_eq!(removed, 0, "Loopback should be allowed on testnet");
}

#[test]
fn test_inc_i050v2_purge_removes_private_on_mainnet() {
    // RFC 1918 private addresses should also be purged on mainnet.
    let local_peer = PeerId::random();
    let mut kad = new_kademlia(local_peer);

    let peer = PeerId::random();
    let private_addr: Multiaddr = "/ip4/192.168.1.1/tcp/30300".parse().unwrap();
    kad.add_address(&peer, private_addr);

    let removed = purge_non_routable_dht_addresses(&mut kad, 1);

    assert!(removed > 0, "Private address should be purged on mainnet");
}
