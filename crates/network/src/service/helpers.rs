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

/// Filters out non-globally-routable addresses that should not be advertised
/// to remote peers via Identify/Kademlia:
/// - Loopback (127.x)
/// - Unspecified (0.0.0.0)
/// - Link-local (169.254.x)
/// - RFC 1918 private (10.x, 172.16-31.x, 192.168.x)
/// - RFC 6598 shared/CGNAT (100.64-127.x)
pub(super) fn is_routable_address(addr: &Multiaddr) -> bool {
    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => {
                if ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_link_local()
                    || ip.is_private()
                    || is_shared_address(ip)
                {
                    return false;
                }
            }
            Protocol::Ip6(ip) => {
                if ip.is_loopback() || ip.is_unspecified() {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

/// RFC 6598 shared address space (100.64.0.0/10) used by CGNAT.
fn is_shared_address(ip: std::net::Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0xC0) == 64
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
