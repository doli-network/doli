//! Test seam: a `NetworkService` with its command channel and no swarm.
// OUTPUT CONTRACT: N/A — fixture file
// INPUT PARTITIONS: N/A — fixture file

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use libp2p::PeerId;
use tokio::sync::{mpsc, RwLock};

use super::backpressure::GossipShedMetrics;
use super::gossip_intake::LocalTip;
use super::{event_sink, LaneBudgets, NetworkCommand, NetworkService, Retention};
use crate::config::NetworkConfig;

impl NetworkService {
    /// Same channels as `new`; the caller reads commands from the receiver.
    #[doc(hidden)]
    pub fn for_test() -> (NetworkService, mpsc::Receiver<NetworkCommand>) {
        let config = NetworkConfig::default();
        let command_buf = (config.max_peers * 4).clamp(2048, 32768);
        let lane_budgets = LaneBudgets::for_max_peers(config.max_peers);
        let (event_sink, lanes) = event_sink::event_lanes(lane_budgets);
        let lane_stats = event_sink.stats();
        let (command_tx, command_rx) = mpsc::channel(command_buf);
        let service = Self {
            config,
            peers: Arc::new(RwLock::new(HashMap::new())),
            lanes,
            lane_stats,
            lane_budgets,
            retention: Arc::new(Retention::default()),
            command_tx,
            local_peer_id: PeerId::random(),
            local_tip: Arc::new(LocalTip::default()),
            gossip_shed_metrics: Arc::new(GossipShedMetrics::new()),
            memory_shed_flag: Arc::new(AtomicBool::new(false)),
        };
        (service, command_rx)
    }
}
