//! NAT traversal configuration
//!
//! This module provides configuration for NAT traversal using
//! libp2p's relay, dcutr (direct connection upgrade through relay),
//! and autonat protocols.

use libp2p::Multiaddr;

/// Configuration for NAT traversal
#[derive(Clone, Debug)]
pub struct NatConfig {
    /// Relay servers to use for hole punching
    pub relay_servers: Vec<Multiaddr>,
    /// Whether to enable automatic NAT detection
    pub enable_autonat: bool,
    /// Whether to enable direct connection upgrade through relay (DCUTR)
    pub enable_dcutr: bool,
    /// Whether this node can act as a relay for other nodes
    pub enable_relay_server: bool,
    /// Maximum number of relay reservations (if acting as relay server)
    pub max_relay_reservations: u32,
    /// Maximum circuits per peer (if acting as relay server)
    pub max_circuits_per_peer: u32,
}

impl Default for NatConfig {
    fn default() -> Self {
        Self {
            relay_servers: vec![],
            enable_autonat: true,
            enable_dcutr: true,
            enable_relay_server: false,
            max_relay_reservations: 128,
            max_circuits_per_peer: 4,
        }
    }
}

impl NatConfig {
    /// Create a config for a node behind NAT (client mode)
    pub fn client() -> Self {
        Self {
            enable_autonat: true,
            enable_dcutr: true,
            enable_relay_server: false,
            ..Default::default()
        }
    }

    /// Create a config for a relay server (public node)
    pub fn relay_server() -> Self {
        Self {
            enable_autonat: true,
            enable_dcutr: true,
            enable_relay_server: true,
            max_relay_reservations: 256,
            max_circuits_per_peer: 8,
            ..Default::default()
        }
    }

    /// Add a relay server address
    pub fn with_relay_server(mut self, addr: Multiaddr) -> Self {
        self.relay_servers.push(addr);
        self
    }

    /// Add multiple relay server addresses
    pub fn with_relay_servers(mut self, addrs: Vec<Multiaddr>) -> Self {
        self.relay_servers.extend(addrs);
        self
    }
}

/// NAT status detected by autonat
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NatStatus {
    /// NAT status is unknown (still probing)
    #[default]
    Unknown,
    /// Node is publicly reachable
    Public,
    /// Node is behind NAT
    Private,
}

impl std::fmt::Display for NatStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NatStatus::Unknown => write!(f, "unknown"),
            NatStatus::Public => write!(f, "public"),
            NatStatus::Private => write!(f, "private/NAT"),
        }
    }
}

/// Information about NAT traversal state
#[derive(Clone, Debug, Default)]
pub struct NatInfo {
    /// Current detected NAT status
    pub status: NatStatus,
    /// External address if detected
    pub external_address: Option<Multiaddr>,
    /// Number of active relay connections
    pub active_relay_connections: usize,
    /// Whether DCUTR is available
    pub dcutr_available: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = NatConfig::default();
        assert!(config.enable_autonat);
        assert!(config.enable_dcutr);
        assert!(!config.enable_relay_server);
    }

    #[test]
    fn test_client_config() {
        let config = NatConfig::client();
        assert!(config.enable_autonat);
        assert!(config.enable_dcutr);
        assert!(!config.enable_relay_server);
    }

    #[test]
    fn test_relay_server_config() {
        let config = NatConfig::relay_server();
        assert!(config.enable_autonat);
        assert!(config.enable_dcutr);
        assert!(config.enable_relay_server);
        assert!(config.max_relay_reservations > 0);
    }

    #[test]
    fn test_with_relay_servers() {
        let addr: Multiaddr = "/ip4/1.2.3.4/tcp/9000".parse().unwrap();
        let config = NatConfig::default().with_relay_server(addr.clone());
        assert_eq!(config.relay_servers.len(), 1);
        assert_eq!(config.relay_servers[0], addr);
    }
}
