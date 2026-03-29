//! Peer management
//!
//! This module provides peer information tracking and diversity management
//! to protect against eclipse attacks.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use crypto::Hash;
use libp2p::{Multiaddr, PeerId};

/// Peer information
#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// Peer ID (libp2p PeerId as string)
    pub id: String,
    /// Remote address
    pub address: String,
    /// Protocol version
    pub version: u32,
    /// Best known height
    pub best_height: u64,
    /// Best known hash
    pub best_hash: Hash,
    /// Connection time
    pub connected_at: Instant,
    /// Last message time
    pub last_seen: Instant,
    /// Bytes received
    pub bytes_received: u64,
    /// Bytes sent
    pub bytes_sent: u64,
    /// Latency (from ping/pong)
    pub latency: Option<Duration>,
    /// Whether this peer identified itself as a producer via StatusResponse.
    /// Used by connection eviction to protect producer peers from being
    /// dropped when at max_peers capacity (SCALE-T2-004).
    pub is_producer: bool,
}

impl PeerInfo {
    /// Create a new peer info
    pub fn new(id: String, address: String) -> Self {
        let now = Instant::now();
        Self {
            id,
            address,
            version: 0,
            best_height: 0,
            best_hash: Hash::ZERO,
            connected_at: now,
            last_seen: now,
            bytes_received: 0,
            bytes_sent: 0,
            latency: None,
            is_producer: false,
        }
    }

    /// Update last seen time
    pub fn touch(&mut self) {
        self.last_seen = Instant::now();
    }

    /// Check if peer is stale (no messages for a while)
    pub fn is_stale(&self, timeout: Duration) -> bool {
        self.last_seen.elapsed() > timeout
    }

    /// Get connection duration
    pub fn connection_duration(&self) -> Duration {
        self.connected_at.elapsed()
    }
}

/// Peer state
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerState {
    /// Connecting
    Connecting,
    /// Connected, performing handshake
    Handshaking,
    /// Fully connected and synced
    Active,
    /// Disconnecting
    Disconnecting,
    /// Disconnected
    Disconnected,
}

/// IP prefix for diversity tracking (e.g., /24 for IPv4, /48 for IPv6)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IpPrefix {
    /// The prefix bytes
    bytes: [u8; 16],
    /// Length of the prefix in bits
    prefix_len: u8,
}

impl IpPrefix {
    /// Create an IP prefix from an IPv4 address (/24 prefix)
    pub fn from_ipv4(ip: Ipv4Addr) -> Self {
        let octets = ip.octets();
        let mut bytes = [0u8; 16];
        // Store first 3 octets (24 bits) as the prefix
        bytes[0..3].copy_from_slice(&octets[0..3]);
        Self {
            bytes,
            prefix_len: 24,
        }
    }

    /// Create an IP prefix from an IPv6 address (/48 prefix)
    pub fn from_ipv6(ip: Ipv6Addr) -> Self {
        let octets = ip.octets();
        let mut bytes = [0u8; 16];
        // Store first 6 octets (48 bits) as the prefix
        bytes[0..6].copy_from_slice(&octets[0..6]);
        Self {
            bytes,
            prefix_len: 48,
        }
    }

    /// Create an IP prefix from an IP address
    pub fn from_ip(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(v4) => Self::from_ipv4(v4),
            IpAddr::V6(v6) => Self::from_ipv6(v6),
        }
    }
}

/// Configuration for peer diversity
#[derive(Clone, Debug)]
pub struct PeerDiversityConfig {
    /// Maximum peers from the same /24 (IPv4) or /48 (IPv6) prefix
    pub max_per_prefix: usize,
    /// Maximum peers from the same ASN (if ASN tracking is enabled)
    pub max_per_asn: usize,
    /// Whether to enforce diversity limits
    pub enabled: bool,
}

impl Default for PeerDiversityConfig {
    fn default() -> Self {
        Self {
            max_per_prefix: 3,
            max_per_asn: 5,
            enabled: true,
        }
    }
}

/// Tracks peer diversity to protect against eclipse attacks
///
/// An eclipse attack occurs when all of a node's peers are controlled
/// by an attacker. Diversity tracking ensures peers are distributed
/// across different networks.
#[derive(Debug)]
pub struct PeerDiversity {
    /// Configuration
    config: PeerDiversityConfig,
    /// Peers by IP prefix
    by_prefix: HashMap<IpPrefix, HashSet<PeerId>>,
    /// Peers by ASN (autonomous system number)
    by_asn: HashMap<u32, HashSet<PeerId>>,
    /// Peer to IP mapping
    peer_ips: HashMap<PeerId, IpAddr>,
    /// Peer to ASN mapping (if known)
    peer_asns: HashMap<PeerId, u32>,
}

impl PeerDiversity {
    /// Create a new peer diversity tracker
    pub fn new(config: PeerDiversityConfig) -> Self {
        Self {
            config,
            by_prefix: HashMap::new(),
            by_asn: HashMap::new(),
            peer_ips: HashMap::new(),
            peer_asns: HashMap::new(),
        }
    }

    /// Extract IP address from a multiaddr
    fn extract_ip(addr: &Multiaddr) -> Option<IpAddr> {
        for protocol in addr.iter() {
            match protocol {
                libp2p::multiaddr::Protocol::Ip4(ip) => return Some(IpAddr::V4(ip)),
                libp2p::multiaddr::Protocol::Ip6(ip) => return Some(IpAddr::V6(ip)),
                _ => continue,
            }
        }
        None
    }

    /// Check if a new connection to this address would maintain diversity
    pub fn can_connect(&self, addr: &Multiaddr) -> bool {
        if !self.config.enabled {
            return true;
        }

        let Some(ip) = Self::extract_ip(addr) else {
            // Can't determine IP, allow connection
            return true;
        };

        let prefix = IpPrefix::from_ip(ip);

        // Check prefix limit
        if let Some(peers) = self.by_prefix.get(&prefix) {
            if peers.len() >= self.config.max_per_prefix {
                return false;
            }
        }

        true
    }

    /// Check if connecting to a specific peer at this address would maintain diversity
    pub fn can_connect_peer(&self, peer: &PeerId, addr: &Multiaddr) -> bool {
        if !self.config.enabled {
            return true;
        }

        // If we already have this peer, always allow
        if self.peer_ips.contains_key(peer) {
            return true;
        }

        self.can_connect(addr)
    }

    /// Record a new connection
    pub fn record_connection(&mut self, peer: PeerId, addr: &Multiaddr) {
        let Some(ip) = Self::extract_ip(addr) else {
            return;
        };

        let prefix = IpPrefix::from_ip(ip);

        // Add to prefix tracking
        self.by_prefix.entry(prefix).or_default().insert(peer);

        // Store peer IP mapping
        self.peer_ips.insert(peer, ip);
    }

    /// Record a peer's ASN (call when ASN is discovered)
    pub fn record_asn(&mut self, peer: &PeerId, asn: u32) {
        self.peer_asns.insert(*peer, asn);
        self.by_asn.entry(asn).or_default().insert(*peer);
    }

    /// Remove a disconnected peer
    pub fn remove_peer(&mut self, peer: &PeerId) {
        // Remove from prefix tracking
        if let Some(ip) = self.peer_ips.remove(peer) {
            let prefix = IpPrefix::from_ip(ip);
            if let Some(peers) = self.by_prefix.get_mut(&prefix) {
                peers.remove(peer);
                if peers.is_empty() {
                    self.by_prefix.remove(&prefix);
                }
            }
        }

        // Remove from ASN tracking
        if let Some(asn) = self.peer_asns.remove(peer) {
            if let Some(peers) = self.by_asn.get_mut(&asn) {
                peers.remove(peer);
                if peers.is_empty() {
                    self.by_asn.remove(&asn);
                }
            }
        }
    }

    /// Get diversity statistics
    pub fn stats(&self) -> DiversityStats {
        let unique_prefixes = self.by_prefix.len();
        let unique_asns = self.by_asn.len();
        let total_peers = self.peer_ips.len();

        let max_peers_per_prefix = self.by_prefix.values().map(|s| s.len()).max().unwrap_or(0);

        let max_peers_per_asn = self.by_asn.values().map(|s| s.len()).max().unwrap_or(0);

        DiversityStats {
            total_peers,
            unique_prefixes,
            unique_asns,
            max_peers_per_prefix,
            max_peers_per_asn,
        }
    }

    /// Check if current peer set has good diversity
    pub fn has_good_diversity(&self) -> bool {
        let stats = self.stats();

        // At least 3 unique prefixes if we have enough peers
        if stats.total_peers >= 6 && stats.unique_prefixes < 3 {
            return false;
        }

        // No single prefix should have more than 50% of peers
        if stats.total_peers > 0 && stats.max_peers_per_prefix > stats.total_peers / 2 {
            return false;
        }

        true
    }
}

/// Statistics about peer diversity
#[derive(Clone, Debug)]
pub struct DiversityStats {
    /// Total number of tracked peers
    pub total_peers: usize,
    /// Number of unique IP prefixes
    pub unique_prefixes: usize,
    /// Number of unique ASNs (if tracking)
    pub unique_asns: usize,
    /// Maximum peers from a single prefix
    pub max_peers_per_prefix: usize,
    /// Maximum peers from a single ASN
    pub max_peers_per_asn: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_peer_id() -> PeerId {
        PeerId::random()
    }

    #[test]
    fn test_ip_prefix_ipv4() {
        let ip1 = Ipv4Addr::new(192, 168, 1, 100);
        let ip2 = Ipv4Addr::new(192, 168, 1, 200);
        let ip3 = Ipv4Addr::new(192, 168, 2, 100);

        let prefix1 = IpPrefix::from_ipv4(ip1);
        let prefix2 = IpPrefix::from_ipv4(ip2);
        let prefix3 = IpPrefix::from_ipv4(ip3);

        // Same /24 should have same prefix
        assert_eq!(prefix1, prefix2);
        // Different /24 should have different prefix
        assert_ne!(prefix1, prefix3);
    }

    #[test]
    fn test_peer_diversity_limits() {
        let config = PeerDiversityConfig {
            max_per_prefix: 2,
            ..Default::default()
        };
        let mut diversity = PeerDiversity::new(config);

        let addr1: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();
        let addr2: Multiaddr = "/ip4/192.168.1.2/tcp/9000".parse().unwrap();
        let addr3: Multiaddr = "/ip4/192.168.1.3/tcp/9000".parse().unwrap();
        let addr4: Multiaddr = "/ip4/10.0.0.1/tcp/9000".parse().unwrap();

        let peer1 = test_peer_id();
        let peer2 = test_peer_id();
        let _peer3 = test_peer_id();
        let peer4 = test_peer_id();

        // First two from same /24 should be allowed
        assert!(diversity.can_connect(&addr1));
        diversity.record_connection(peer1, &addr1);

        assert!(diversity.can_connect(&addr2));
        diversity.record_connection(peer2, &addr2);

        // Third from same /24 should be rejected (over limit)
        assert!(!diversity.can_connect(&addr3));

        // Different /24 should be allowed
        assert!(diversity.can_connect(&addr4));
        diversity.record_connection(peer4, &addr4);

        // After removing peer, should allow again
        diversity.remove_peer(&peer1);
        assert!(diversity.can_connect(&addr3));
    }

    #[test]
    fn test_diversity_disabled() {
        let config = PeerDiversityConfig {
            enabled: false,
            ..Default::default()
        };
        let diversity = PeerDiversity::new(config);

        // Should always allow when disabled
        let addr: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();
        assert!(diversity.can_connect(&addr));
    }

    #[test]
    fn test_diversity_stats() {
        let config = PeerDiversityConfig::default();
        let mut diversity = PeerDiversity::new(config);

        let peer1 = test_peer_id();
        let peer2 = test_peer_id();
        let peer3 = test_peer_id();

        let addr1: Multiaddr = "/ip4/192.168.1.1/tcp/9000".parse().unwrap();
        let addr2: Multiaddr = "/ip4/192.168.1.2/tcp/9000".parse().unwrap();
        let addr3: Multiaddr = "/ip4/10.0.0.1/tcp/9000".parse().unwrap();

        diversity.record_connection(peer1, &addr1);
        diversity.record_connection(peer2, &addr2);
        diversity.record_connection(peer3, &addr3);

        let stats = diversity.stats();
        assert_eq!(stats.total_peers, 3);
        assert_eq!(stats.unique_prefixes, 2);
        assert_eq!(stats.max_peers_per_prefix, 2);
    }
}
