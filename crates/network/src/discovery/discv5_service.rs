//! Discv5 UDP peer discovery service
//!
//! Runs a Discovery v5 UDP service alongside libp2p. Discovered peers are
//! fed to the libp2p swarm for TCP gossip connections.
//!
//! ## Architecture
//!
//! ```text
//! discv5 (UDP :p2p_port+1)  ──discovers──►  ENR with TCP port
//!                                              │
//!                                              ▼
//!                                     libp2p swarm.dial(multiaddr)
//!                                              │
//!                                              ▼
//!                                     gossipsub TCP connection
//! ```

use std::net::{Ipv4Addr, SocketAddr};

use discv5::enr::{CombinedKey, NodeId};
use discv5::socket::ListenConfig;
use discv5::{ConfigBuilder, Discv5, Enr, Event};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crypto::Hash;

/// Custom ENR key for DOLI network filtering.
/// Value: network_id (4 bytes big-endian) + genesis_hash[..4] (4 bytes) = 8 bytes.
/// Peers with mismatched "doli" field are ignored at discovery layer.
const DOLI_ENR_KEY: &str = "doli";

/// Configuration for the Discv5 discovery service
#[derive(Clone, Debug)]
pub struct Discv5Config {
    /// UDP listen port (default: p2p_port + 1)
    pub udp_port: u16,
    /// TCP port to advertise in ENR (so discovered peers know where to connect)
    pub tcp_port: u16,
    /// External IPv4 address to advertise (None = auto-detect via PONG responses)
    pub external_ip: Option<Ipv4Addr>,
    /// Network ID (1=mainnet, 2=testnet, 99=devnet)
    pub network_id: u32,
    /// Genesis hash (first 4 bytes used as fork digest in ENR)
    pub genesis_hash: Hash,
    /// Bootnode ENR strings to bootstrap from
    pub bootnode_enrs: Vec<String>,
}

/// Wrapper around discv5::Discv5 providing DOLI-specific functionality
pub struct Discv5Service {
    /// The underlying discv5 service
    discv5: Discv5,
    /// Network filter bytes: network_id(4) + genesis_hash[..4]
    network_filter: [u8; 8],
}

impl Discv5Service {
    /// Create and start a new Discv5 UDP discovery service.
    ///
    /// The ENR key is derived from the libp2p Ed25519 keypair.
    /// The service listens on `0.0.0.0:{udp_port}` for UDP packets.
    pub async fn new(
        libp2p_keypair: &libp2p::identity::Keypair,
        config: Discv5Config,
    ) -> Result<Self, String> {
        // Extract Ed25519 secret key bytes from libp2p keypair
        let enr_key = keypair_to_combined_key(libp2p_keypair)?;

        // Build network filter: network_id(4 BE) + genesis_hash[..4]
        let mut network_filter = [0u8; 8];
        network_filter[..4].copy_from_slice(&config.network_id.to_be_bytes());
        network_filter[4..8].copy_from_slice(&config.genesis_hash.as_bytes()[..4]);

        // Build ENR
        let mut enr_builder = discv5::enr::Builder::default();
        enr_builder.udp4(config.udp_port);
        enr_builder.tcp4(config.tcp_port);

        if let Some(ip) = config.external_ip {
            enr_builder.ip4(ip);
        }

        // Add DOLI network filter to ENR
        enr_builder.add_value(DOLI_ENR_KEY, &network_filter.as_slice());

        let local_enr = enr_builder
            .build(&enr_key)
            .map_err(|e| format!("Failed to build ENR: {:?}", e))?;

        info!(
            "[DISCV5] Local ENR: {} (node_id: {})",
            local_enr,
            local_enr.node_id()
        );
        info!(
            "[DISCV5] UDP port: {}, TCP port: {}",
            config.udp_port, config.tcp_port
        );

        // Configure discv5
        let listen_config = ListenConfig::Ipv4 {
            ip: Ipv4Addr::UNSPECIFIED,
            port: config.udp_port,
        };

        let discv5_config = ConfigBuilder::new(listen_config)
            .request_timeout(std::time::Duration::from_secs(2))
            .query_timeout(std::time::Duration::from_secs(30))
            .request_retries(1)
            .session_cache_capacity(500)
            .build();

        let mut discv5 = Discv5::new(local_enr, enr_key, discv5_config)
            .map_err(|e| format!("Failed to create Discv5: {}", e))?;

        // Add bootnodes
        let mut added = 0;
        for enr_str in &config.bootnode_enrs {
            match enr_str.parse::<Enr>() {
                Ok(enr) => {
                    if let Err(e) = discv5.add_enr(enr.clone()) {
                        warn!("[DISCV5] Failed to add bootnode ENR: {}", e);
                    } else {
                        debug!(
                            "[DISCV5] Added bootnode: {} ({})",
                            enr.node_id(),
                            enr.udp4_socket().map(|s| s.to_string()).unwrap_or_default()
                        );
                        added += 1;
                    }
                }
                Err(e) => {
                    warn!(
                        "[DISCV5] Failed to parse bootnode ENR '{}': {:?}",
                        enr_str, e
                    );
                }
            }
        }
        info!(
            "[DISCV5] Added {}/{} bootnodes",
            added,
            config.bootnode_enrs.len()
        );

        // Start the service
        discv5
            .start()
            .await
            .map_err(|e| format!("Failed to start Discv5: {:?}", e))?;

        info!("[DISCV5] Service started on UDP port {}", config.udp_port);

        Ok(Self {
            discv5,
            network_filter,
        })
    }

    /// Get the event stream receiver for polling discovery events.
    pub async fn event_stream(&self) -> Result<mpsc::Receiver<Event>, String> {
        self.discv5
            .event_stream()
            .await
            .map_err(|e| format!("Failed to get event stream: {:?}", e))
    }

    /// Trigger a random-walk query to discover new peers.
    /// Results arrive via the event stream as `Event::Discovered`.
    pub fn find_random_peers(&self) -> impl std::future::Future<Output = ()> + 'static {
        let fut = self.discv5.find_node(NodeId::random());
        async move {
            match fut.await {
                Ok(found) => {
                    debug!("[DISCV5] Random walk found {} peers", found.len());
                }
                Err(e) => {
                    debug!("[DISCV5] Random walk error: {:?}", e);
                }
            }
        }
    }

    /// Check if a discovered ENR belongs to the same DOLI network.
    /// Compares the "doli" custom field (network_id + genesis prefix).
    pub fn is_same_network(&self, enr: &Enr) -> bool {
        match enr.get_raw_rlp(DOLI_ENR_KEY) {
            Some(raw_rlp) => {
                // The value is RLP-encoded bytes. For a byte string of length 8,
                // RLP prepends a length byte (0x88) followed by the 8 bytes.
                // Try to decode or do a suffix match.
                if raw_rlp.len() >= 8 {
                    // Try direct comparison of last 8 bytes (handles RLP prefix)
                    let data = &raw_rlp[raw_rlp.len() - 8..];
                    data == self.network_filter
                } else {
                    false
                }
            }
            None => {
                // No DOLI field — might be a non-DOLI node or old version.
                // Accept for now (Identify/Status protocol will filter later).
                debug!(
                    "[DISCV5] ENR {} has no 'doli' field, accepting",
                    enr.node_id()
                );
                true
            }
        }
    }

    /// Extract a libp2p-compatible TCP multiaddr from a discovered ENR.
    /// Returns `/ip4/{ip}/tcp/{port}` if the ENR has both IP and TCP port.
    pub fn enr_to_multiaddr(enr: &Enr) -> Option<libp2p::Multiaddr> {
        let ip = enr.ip4()?;
        let tcp_port = enr.tcp4()?;

        let multiaddr_str = format!("/ip4/{}/tcp/{}", ip, tcp_port);
        multiaddr_str.parse().ok()
    }

    /// Get the local ENR string (for sharing with other nodes as bootnode).
    pub fn local_enr(&self) -> Enr {
        self.discv5.local_enr()
    }

    /// Get the local ENR as a base64 string.
    pub fn local_enr_base64(&self) -> String {
        self.discv5.local_enr().to_base64()
    }

    /// Number of connected discv5 sessions.
    pub fn connected_peers(&self) -> usize {
        self.discv5.connected_peers()
    }

    /// Shutdown the discv5 service.
    pub fn shutdown(&mut self) {
        self.discv5.shutdown();
        info!("[DISCV5] Service shut down");
    }
}

/// Convert a libp2p Ed25519 keypair to a discv5 CombinedKey.
///
/// libp2p stores Ed25519 keys in protobuf format. We extract the raw 32-byte
/// secret key and create a CombinedKey::Ed25519 from it.
fn keypair_to_combined_key(keypair: &libp2p::identity::Keypair) -> Result<CombinedKey, String> {
    // Try to get Ed25519 keypair from libp2p
    let ed25519_kp = keypair
        .clone()
        .try_into_ed25519()
        .map_err(|_| "Keypair is not Ed25519")?;

    // Extract the 32-byte secret key
    let mut secret_bytes = ed25519_kp.secret().as_ref().to_vec();

    let combined = CombinedKey::ed25519_from_bytes(&mut secret_bytes)
        .map_err(|e| format!("Failed to create CombinedKey: {:?}", e))?;

    // Zero out secret bytes
    secret_bytes.fill(0);

    Ok(combined)
}

/// Parse a socket address string like "1.2.3.4:9001" into an Ipv4Addr.
pub fn parse_ipv4(addr_str: &str) -> Option<Ipv4Addr> {
    addr_str.parse::<SocketAddr>().ok().and_then(|sa| match sa {
        SocketAddr::V4(v4) => Some(*v4.ip()),
        _ => None,
    })
}
