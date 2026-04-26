//! Utility functions for the network service.
//!
//! Address filtering, keypair persistence, and multiaddr manipulation.

use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;

use super::types::NetworkError;

/// Strip the `/p2p/<peer_id>` suffix from a multiaddr, returning the transport-only address.
/// Used to store addresses without embedding peer IDs (which change after chain resets).
pub(super) fn strip_p2p_suffix(addr: &Multiaddr) -> Multiaddr {
    addr.iter()
        .filter(|p| !matches!(p, Protocol::P2p(_)))
        .collect()
}

/// Filters out non-routable addresses from Identify/Kademlia advertisements.
///
/// On mainnet (network_id=1): filters loopback, unspecified, link-local,
/// RFC 1918 private, and RFC 6598 CGNAT addresses.
///
/// On testnet/devnet: only filters unspecified (0.0.0.0) and link-local.
/// Loopback and private addresses are allowed so that nodes on localhost
/// or LAN can discover each other via DHT.
pub(super) fn is_routable_address(addr: &Multiaddr, network_id: u32) -> bool {
    let is_mainnet = network_id == 1;
    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => {
                // Always filter unspecified and link-local
                if ip.is_unspecified() || ip.is_link_local() {
                    return false;
                }
                // Mainnet only: filter loopback, private, CGNAT
                if is_mainnet && (ip.is_loopback() || ip.is_private() || is_shared_address(ip)) {
                    return false;
                }
            }
            Protocol::Ip6(ip) if ip.is_loopback() || ip.is_unspecified() => {
                return false;
            }
            _ => {}
        }
    }
    true
}

/// INC-I-050: Check if ALL addresses for a DHT peer are routable.
///
/// Used to filter Kademlia routing table updates. When a peer has ANY
/// non-routable address (loopback, private, CGNAT on mainnet), the peer
/// should be removed from the routing table to prevent DHT poisoning.
///
/// This closes the gap where `is_routable_address` was only applied to the
/// Identify path but not to Kademlia's internal FIND_NODE response path.
pub(super) fn all_addresses_routable<'a>(
    addrs: impl Iterator<Item = &'a Multiaddr>,
    network_id: u32,
) -> bool {
    let mut count = 0;
    for addr in addrs {
        count += 1;
        if !is_routable_address(addr, network_id) {
            return false;
        }
    }
    // A peer with zero addresses should not be kept
    count > 0
}

/// RFC 6598 shared address space (100.64.0.0/10) used by CGNAT.
fn is_shared_address(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0xC0) == 64
}

/// INC-I-048: Plan the startup dial order — bootstrap first, then capped cached peers.
///
/// Returns `(bootstrap_addrs, cached_addrs)` where bootstrap addrs must be dialed
/// first to guarantee they get pending connection slots before stale cached peers.
/// Cached addrs are filtered (no DNS) and capped to `pending_limit - bootstrap_count`.
pub(super) fn plan_startup_dials(
    bootstrap_nodes: &[String],
    cached_addrs: Vec<Multiaddr>,
    pending_limit: u32,
) -> (Vec<Multiaddr>, Vec<Multiaddr>) {
    // Parse and clean bootstrap addresses
    let bootstrap: Vec<Multiaddr> = bootstrap_nodes
        .iter()
        .filter_map(|s| s.parse::<Multiaddr>().ok())
        .map(|a| strip_p2p_suffix(&a))
        .collect();

    let bootstrap_count = bootstrap.len() as u32;

    // Cap cached dials to leave room for bootstrap slots
    let cached_limit = pending_limit.saturating_sub(bootstrap_count).max(1) as usize;

    // Filter DNS multiaddrs from cache (they belong in bootstrap, not cache)
    let cached: Vec<Multiaddr> = cached_addrs
        .into_iter()
        .filter(|a| {
            let s = a.to_string();
            !s.starts_with("/dns4/") && !s.starts_with("/dns6/")
        })
        .take(cached_limit)
        .map(|a| strip_p2p_suffix(&a))
        .collect();

    (bootstrap, cached)
}

/// Load keypair from file
pub(super) fn load_keypair(
    path: &std::path::Path,
) -> Result<libp2p::identity::Keypair, NetworkError> {
    let bytes = std::fs::read(path)
        .map_err(|e| NetworkError::Other(format!("Failed to read keypair: {}", e)))?;
    libp2p::identity::Keypair::from_protobuf_encoding(&bytes)
        .map_err(|e| NetworkError::Other(format!("Failed to decode keypair: {}", e)))
}

/// Save keypair to file
pub(super) fn save_keypair(
    path: &std::path::Path,
    keypair: &libp2p::identity::Keypair,
) -> Result<(), NetworkError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| NetworkError::Other(format!("Failed to create directory: {}", e)))?;
    }
    let bytes = keypair
        .to_protobuf_encoding()
        .map_err(|e| NetworkError::Other(format!("Failed to encode keypair: {}", e)))?;
    std::fs::write(path, bytes)
        .map_err(|e| NetworkError::Other(format!("Failed to write keypair: {}", e)))
}
