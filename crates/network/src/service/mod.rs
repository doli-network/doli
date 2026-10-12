//! Network service implementation
//!
//! Manages the libp2p swarm and provides high-level network operations.
//! ## Module structure
//! - `types` — Public enums: `NetworkEvent`, `NetworkCommand`, `NetworkError`
//! - `helpers` — Address filtering, keypair persistence, multiaddr manipulation
//! - `swarm_loop` — Main tokio::select! event loop driving the swarm
//! - `swarm_events` — Swarm-level event handling (connections, listen addrs)
//! - `behaviour_events` — Behaviour-level event handling (gossip, identify, status, sync)
//! - `command_api` — `NetworkService` accessors and command senders
//! - `command_handling` — Network command dispatch (publish, dial, request-response)
//! - `backpressure` — Gossip block load-shedding metrics (INC-I-114)

pub(super) mod backpressure;
mod behaviour_events;
mod command_api;
mod command_handling;
mod event_lanes;
mod event_sink;
mod event_size;
mod gossip_handle;
mod gossip_intake;
mod helpers;
mod swarm_events;
mod swarm_loop;
mod types;
mod verdict;

#[cfg(test)]
mod tests;

pub use event_sink::{LaneBudgets, LaneSnapshot, LaneStats, TopicCounts};
pub use gossip_handle::GossipHandle;
pub use types::{NetworkCommand, NetworkError, NetworkEvent};
pub use verdict::{send_gossip_verdict, Verdict, DROPPED_VERDICTS, LATE_VERDICTS};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;
use std::time::Duration;

use libp2p::{autonat, dcutr, relay, Multiaddr, PeerId, Swarm};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

use crate::behaviour::DoliBehaviour;
use crate::config::NetworkConfig;
use crate::discovery::new_kademlia;
use crate::gossip::{new_gossipsub, subscribe_to_topics};
use crate::peer::PeerInfo;
use crate::peer_cache::PeerCache;
use crate::transport::build_transport;
use crate::watchdog::MemoryWatchdog;

use backpressure::GossipShedMetrics;
use event_lanes::EventLanes;
use helpers::{load_keypair, plan_startup_dials, save_keypair};
use swarm_loop::run_swarm;

/// Network service handle
pub struct NetworkService {
    /// Configuration
    #[allow(dead_code)]
    config: NetworkConfig,
    /// Connected peers
    peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    /// Node side of the two event lanes; the swarm task owns the only senders.
    lanes: EventLanes,
    lane_stats: Arc<LaneStats>,
    lane_budgets: LaneBudgets,
    /// Command sender
    command_tx: mpsc::Sender<NetworkCommand>,
    /// Local peer ID
    local_peer_id: PeerId,
    /// INC-I-049: best slot, exempts candidate-next blocks from rate limiting.
    best_slot: Arc<AtomicU32>,
    /// INC-I-114: Load-shedding metrics for gossip block events.
    gossip_shed_metrics: Arc<GossipShedMetrics>,
    memory_shed_flag: Arc<AtomicBool>,
}

impl NetworkService {
    /// Create a new network service
    pub async fn new(config: NetworkConfig) -> Result<Self, NetworkError> {
        // SCALE-T2-003: command buffer scales with max_peers; event lanes per LaneBudgets.
        let command_buf = (config.max_peers * 4).clamp(2048, 32768);
        let lane_budgets = LaneBudgets::for_max_peers(config.max_peers);
        let (event_sink, lanes) = event_sink::event_lanes(lane_budgets);
        let lane_stats = event_sink.stats();
        let (command_tx, command_rx) = mpsc::channel(command_buf);

        // INC-I-114: Gossip block load-shedding metrics (shared with swarm task)
        let gossip_shed_metrics = Arc::new(GossipShedMetrics::new());

        let peers = Arc::new(RwLock::new(HashMap::new()));

        // Generate or load keypair
        let keypair = if let Some(ref key_path) = config.node_key_path {
            if key_path.exists() {
                load_keypair(key_path)?
            } else {
                let kp = libp2p::identity::Keypair::generate_ed25519();
                save_keypair(key_path, &kp)?;
                kp
            }
        } else {
            libp2p::identity::Keypair::generate_ed25519()
        };

        let local_peer_id = PeerId::from(keypair.public());
        info!("Local peer ID: {}", local_peer_id);

        // Create relay client (returns transport + behaviour pair)
        let (relay_transport, relay_client) = relay::client::new(local_peer_id);

        // Build transport WITH relay support for NAT traversal
        let transport = build_transport(&keypair, Some(relay_transport))
            .map_err(|e| NetworkError::Other(format!("Transport error: {}", e)))?;

        // Build gossipsub with mesh params from NetworkParams.
        // All topics are subscribed at startup. Topic subscriptions are
        // reconfigured at epoch boundaries when the node's tier is computed.
        let mesh = crate::gossip::MeshConfig {
            mesh_n: config.mesh_n,
            mesh_n_low: config.mesh_n_low,
            mesh_n_high: config.mesh_n_high,
            gossip_lazy: config.gossip_lazy,
        };
        crate::gossip::assert_production_hardening(&mesh, &lane_budgets)
            .map_err(|e| NetworkError::Other(format!("GossipSub error: {}", e)))?;
        let mut gossipsub = new_gossipsub(&keypair, &mesh)
            .map_err(|e| NetworkError::Other(format!("GossipSub error: {}", e)))?;
        subscribe_to_topics(&mut gossipsub)
            .map_err(|e| NetworkError::Other(format!("Subscribe error: {}", e)))?;

        // Build kademlia
        let kademlia = new_kademlia(local_peer_id);

        // INC-I-014: Connection limits tuned for large localhost networks.
        //
        // max_established_per_peer=1: The old value of 2 meant each peer consumed
        // 2 connections (1MB Yamux each). At 156 nodes this caused 100 connections
        // per node and 54GB RAM. With 1 conn/peer, the same conn_limit serves
        // 2x more unique peers. The simultaneous-dial race (libp2p#752) resolves
        // itself — libp2p closes the duplicate, the surviving conn works fine.
        //
        // INC-I-014: Connection limits for large networks.
        // Key insight: per-direction limits (in/out) are checked independently —
        // 60 in + 60 out = 120 total connections, causing RAM explosion.
        // Fix: cap TOTAL connections with with_max_established().
        //
        // max_peers*2 total gives fast Kademlia discovery (tested: great bootstrap
        // up to 70+ nodes) while bounding RAM. At max_peers=25: 50 total connections
        // × 256KB Yamux = 12.5MB/node. Eviction keeps steady-state at ~25 conns.
        // INC-I-014: Connection limit tightened from max_peers*2 to max_peers+10.
        // The old 2x headroom allowed up to max_peers "ghost" connections that completed
        // the Noise+Yamux handshake (~456KB each) but weren't in the peer table. With
        // 213 nodes and constant churn, the macOS allocator never returned these freed
        // pages → RSS grew monotonically. +10 gives headroom for bootstrap-only connections
        // and connection races without doubling the buffer cost.
        let total_conn_limit = std::env::var("DOLI_CONN_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or((config.max_peers + 10) as u32);
        // INC-I-014: Pending connection limits. with_max_established() only caps
        // COMPLETED connections. During handshake (TCP+Noise+Yamux), each pending
        // connection allocates ~1MB with NO cap. At 156 nodes: pending_out=37/node
        // = 37MB invisible RAM per node. with_max_pending_* checks BEFORE the
        // handshake starts (handle_pending_inbound/outbound_connection), preventing
        // Noise+Yamux buffer allocation entirely.
        // INC-I-014: Pending limit slashed from max_peers (25) to 5.
        // Each pending connection allocates ~456KB for Noise+Yamux handshake BEFORE
        // ConnectionEstablished. With max_peers=25 pending, 206 nodes × 25 pending
        // = 5,150 simultaneous handshakes × 456KB = 2.3GB in handshake buffers that
        // churn constantly (macOS allocator doesn't return pages → 6.6GB/min leak).
        // At 5 pending, the handshake buffer cost drops to ~460MB system-wide.
        let pending_limit = std::env::var("DOLI_PENDING_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5u32);
        let limits = libp2p::connection_limits::ConnectionLimits::default()
            .with_max_established_per_peer(Some(1))
            .with_max_established(Some(total_conn_limit))
            .with_max_established_incoming(Some(total_conn_limit))
            .with_max_established_outgoing(Some(total_conn_limit))
            .with_max_pending_incoming(Some(pending_limit))
            .with_max_pending_outgoing(Some(pending_limit));
        let connection_limits = libp2p::connection_limits::Behaviour::new(limits);

        // Build relay server — all nodes instantiate it, but reservation limits
        // control whether it actively serves relay circuits.
        let relay_server_config = if config.nat_config.enable_relay_server {
            relay::Config {
                max_reservations: config.nat_config.max_relay_reservations as usize,
                max_circuits_per_peer: config.nat_config.max_circuits_per_peer as usize,
                ..Default::default()
            }
        } else {
            // Disabled: zero reservations means no peers can reserve
            relay::Config {
                max_reservations: 0,
                max_circuits_per_peer: 0,
                ..Default::default()
            }
        };
        let relay_server = relay::Behaviour::new(local_peer_id, relay_server_config);

        // DCUtR — hole punching via relay-coordinated upgrade
        let dcutr = dcutr::Behaviour::new(local_peer_id);

        // AutoNAT — detects whether we're behind NAT
        let autonat_config = autonat::Config {
            boot_delay: std::time::Duration::from_secs(15),
            refresh_interval: std::time::Duration::from_secs(60),
            ..Default::default()
        };
        let autonat = autonat::Behaviour::new(local_peer_id, autonat_config);

        // Build behaviour
        let behaviour = DoliBehaviour::new(
            gossipsub,
            kademlia,
            keypair.public(),
            connection_limits,
            relay_client,
            relay_server,
            dcutr,
            autonat,
        );

        // INC-I-014: Network-aware idle timeout. 24h for mainnet (stable long-lived
        // connections). 1h for testnet (moderate churn during stress tests). 5min for
        // devnet (rapid iteration). Override with DOLI_IDLE_TIMEOUT_SECS for tuning.
        let default_idle_secs = match config.network_id {
            1 => 86400, // mainnet: 24h
            2 => 300, // testnet: 5min (INC-I-014: 1h was too long, stale connections held buffers)
            _ => 300, // devnet: 5min
        };
        let idle_timeout_secs = std::env::var("DOLI_IDLE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default_idle_secs);

        // Build swarm
        let mut swarm = Swarm::new(
            transport,
            behaviour,
            local_peer_id,
            libp2p::swarm::Config::with_tokio_executor()
                .with_idle_connection_timeout(Duration::from_secs(idle_timeout_secs)),
        );

        // Listen on configured address
        let listen_addr: Multiaddr = format!(
            "/ip4/{}/tcp/{}",
            config.listen_addr.ip(),
            config.listen_addr.port()
        )
        .parse()
        .map_err(|e| NetworkError::Other(format!("Invalid listen address: {}", e)))?;

        swarm
            .listen_on(listen_addr.clone())
            .map_err(|_| NetworkError::BindError)?;

        // Register explicit external address so Identify advertises it instead of
        // non-routable local addresses (e.g., 127.0.0.1 on multi-node hosts).
        if let Some(ref ext_addr) = config.external_address {
            swarm.add_external_address(ext_addr.clone());
            info!("Registered external address: {}", ext_addr);
        }

        let yamux_window = std::env::var("DOLI_YAMUX_WINDOW")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(262_144);
        info!("Network service listening on {}", listen_addr);
        let eviction_grace_secs = std::env::var("DOLI_EVICTION_GRACE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);
        info!(
            "[MEM-BOOT] max_peers={} conn_limit={} pending_limit={} yamux_window={}KB idle_timeout={}s eviction_grace={}s event_buf={} cmd_buf={}",
            config.max_peers,
            total_conn_limit,
            pending_limit,
            yamux_window / 1024,
            idle_timeout_secs,
            eviction_grace_secs,
            lane_budgets.consensus_cap,
            command_buf
        );

        // INC-I-048: Dial bootstrap nodes FIRST, directly on swarm.
        // Previously, cached peers were dialed first (filling all pending slots),
        // and bootstrap nodes were dialed via async command channel (arriving too late).
        // With max_pending_outgoing=5, stale cached peers with 75s timeouts permanently
        // starved the bootstrap dial, leaving nodes like N3/N10 at 0 peers forever.
        let cached_addrs = config
            .peer_cache_path
            .as_ref()
            .and_then(|p| PeerCache::load(p))
            .map(|c| c.addresses())
            .unwrap_or_default();

        let (bootstrap_addrs, cached_addrs) =
            plan_startup_dials(&config.bootstrap_nodes, cached_addrs, pending_limit);

        for addr in &bootstrap_addrs {
            if let Err(e) = swarm.dial(addr.clone()) {
                warn!("Failed to dial bootstrap {}: {}", addr, e);
            }
        }
        if !bootstrap_addrs.is_empty() {
            info!(
                "Dialed {} bootstrap nodes (priority)",
                bootstrap_addrs.len()
            );
        }

        if !cached_addrs.is_empty() {
            info!(
                "Dialing {} cached peers (limit {})",
                cached_addrs.len(),
                pending_limit
                    .saturating_sub(bootstrap_addrs.len() as u32)
                    .max(1),
            );
            for addr in &cached_addrs {
                let _ = swarm.dial(addr.clone());
            }
        }

        // Start Discv5 UDP discovery service (if enabled)
        let (discv5_events, discv5_svc) = if config.enable_discv5 {
            let tcp_port = config.listen_addr.port();
            let udp_port = config.discv5_port.unwrap_or(tcp_port + 1);

            let discv5_config = crate::discovery::discv5_service::Discv5Config {
                udp_port,
                tcp_port,
                external_ip: config.external_address.as_ref().and_then(|addr| {
                    addr.iter().find_map(|p| match p {
                        libp2p::multiaddr::Protocol::Ip4(ip) if !ip.is_unspecified() => Some(ip),
                        _ => None,
                    })
                }),
                network_id: config.network_id,
                genesis_hash: config.genesis_hash,
                bootnode_enrs: config.bootnode_enrs.clone(),
            };

            match crate::discovery::discv5_service::Discv5Service::new(&keypair, discv5_config)
                .await
            {
                Ok(svc) => {
                    match svc.event_stream().await {
                        Ok(rx) => {
                            let svc = std::sync::Arc::new(svc);
                            info!("[DISCV5] ENR: {}", svc.local_enr_base64());
                            (Some(rx), Some(svc))
                        }
                        Err(e) => {
                            warn!("[DISCV5] Failed to get event stream: {} — continuing without discv5", e);
                            (None, None)
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "[DISCV5] Failed to start: {} — continuing without discv5",
                        e
                    );
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        // INC-I-049: Shared best slot for rate limiter height+1 exemption
        let best_slot = Arc::new(AtomicU32::new(0));

        // Spawn the swarm event loop
        let peers_clone = peers.clone();
        let config_clone = config.clone();
        let peer_cache_path = config.peer_cache_path.clone();
        let best_slot_clone = best_slot.clone();
        let shed_metrics_clone = gossip_shed_metrics.clone();
        let watchdog_threshold = config.memory_watchdog_threshold_bytes;
        let watchdog =
            (watchdog_threshold > 0).then(|| MemoryWatchdog::with_real_sampler(watchdog_threshold));
        let memory_shed_flag = watchdog
            .as_ref()
            .map_or_else(|| Arc::new(AtomicBool::new(false)), |wd| wd.shed_flag());
        let shed_flag_clone = memory_shed_flag.clone();

        tokio::spawn(async move {
            run_swarm(
                swarm,
                command_rx,
                event_sink,
                peers_clone,
                config_clone,
                peer_cache_path,
                discv5_events,
                discv5_svc,
                best_slot_clone,
                shed_metrics_clone,
                watchdog,
                shed_flag_clone,
            )
            .await;
        });

        Ok(Self {
            config,
            peers,
            lanes,
            lane_stats,
            lane_budgets,
            command_tx,
            local_peer_id,
            best_slot,
            gossip_shed_metrics,
            memory_shed_flag,
        })
    }
}
