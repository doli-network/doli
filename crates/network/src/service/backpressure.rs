//! Gossip block load-shedding metrics (INC-I-114 M1).
//!
//! A gossip block the consensus lane cannot take is dropped and counted
//! (`EventSink::emit_block`), never awaited, so the swarm task is not
//! suspended and libp2p's inbound queue stays bounded.

use std::sync::atomic::{AtomicU64, Ordering};

/// Interval (in drop count) between warn-level log emissions.
/// During a flood, logging every drop would itself become a resource problem.
/// We emit one warn per WARN_EVERY_N_DROPS drops.
pub const WARN_EVERY_N_DROPS: u64 = 100;

/// Atomic counters for gossip block load-shedding observability.
///
/// Shared via `Arc` between the swarm loop (writer) and the node/RPC layer
/// (reader, via `NetworkService::gossip_shed_metrics()`).
#[derive(Debug, Default)]
pub struct GossipShedMetrics {
    blocks_dropped: AtomicU64,
}

impl GossipShedMetrics {
    /// Create a new zero-initialized metrics instance.
    pub fn new() -> Self {
        Self {
            blocks_dropped: AtomicU64::new(0),
        }
    }

    /// Record a single block drop (channel full).
    pub fn record_block_drop(&self) {
        self.blocks_dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Total number of blocks dropped since service start.
    pub fn blocks_dropped(&self) -> u64 {
        self.blocks_dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doli_core::{Block, BlockHeader};
    use libp2p::PeerId;

    use crate::service::event_lanes::EventLanes;
    use crate::service::event_sink::{event_bytes, event_lanes, EventSink, LaneBudgets};
    use crate::service::types::NetworkEvent;
    use crate::service::GossipHandle;

    /// Build a minimal valid block with a given slot for testing.
    fn make_block(slot: u32) -> Block {
        let header = BlockHeader {
            version: 2,
            prev_hash: crypto::Hash::ZERO,
            merkle_root: crypto::Hash::ZERO,
            presence_root: crypto::Hash::ZERO,
            genesis_hash: crypto::Hash::ZERO,
            timestamp: 0,
            slot,
            producer: crypto::PublicKey::from_bytes([0u8; 32]),
            vdf_output: vdf::VdfOutput { value: vec![] },
            vdf_proof: vdf::VdfProof { pi: vec![] },
            missed_producers: vec![],
            data_root: crypto::Hash::ZERO,
            fork_id: crypto::Hash::ZERO,
        };
        Block::new(header, vec![])
    }

    fn make_event(slot: u32) -> NetworkEvent {
        NetworkEvent::NewBlock(make_block(slot), PeerId::random(), GossipHandle::now())
    }

    /// A sink whose consensus lane holds `cap` events (production byte budgets).
    fn lanes_with_cap(cap: usize) -> (EventSink, EventLanes) {
        event_lanes(LaneBudgets {
            consensus_cap: cap,
            ..LaneBudgets::for_max_peers(50)
        })
    }

    fn drain(lanes: &mut EventLanes) -> usize {
        std::iter::from_fn(|| lanes.try_recv()).count()
    }

    // TEST 1: a 1000-block flood into a cap-8 lane with no consumer
    // saturates at 8 and sheds the rest — the lane does NOT grow.
    #[tokio::test]
    async fn stale_block_flood_sheds_and_stays_bounded() {
        let (sink, mut lanes) = lanes_with_cap(8);
        let metrics = GossipShedMetrics::new();

        for i in 0..1000u32 {
            sink.emit_block(make_event(i), &metrics);
        }

        assert_eq!(
            metrics.blocks_dropped(),
            992,
            "expected 992 drops (1000 enqueues - 8 lane slots)"
        );
        assert_eq!(sink.stats().snapshot().consensus_shed, 992);
        assert_eq!(drain(&mut lanes), 8, "lane must hold exactly its 8 slots");
    }

    // TEST 2: with room in the lane, events are delivered with zero drops.
    #[tokio::test]
    async fn enqueue_succeeds_when_capacity_available() {
        let (sink, mut lanes) = lanes_with_cap(16);
        let metrics = GossipShedMetrics::new();

        for i in 0..5u32 {
            sink.emit_block(make_event(i), &metrics);
        }

        assert_eq!(
            metrics.blocks_dropped(),
            0,
            "no drops when capacity available"
        );
        let per_block = event_bytes(&make_event(0));
        assert_eq!(
            sink.stats().snapshot().consensus_bytes,
            5 * per_block,
            "5 of 16 slots in flight"
        );

        let mut count = 0;
        while let Some(event) = lanes.try_recv() {
            if let NetworkEvent::NewBlock(..) = event {
                count += 1;
            }
        }
        assert_eq!(count, 5, "all 5 events must be received");
    }

    // TEST 3: emit_block is synchronous — flooding a full lane completes
    // inside a tight time bound.
    #[tokio::test]
    async fn enqueue_never_blocks() {
        let (sink, _lanes) = lanes_with_cap(4);
        let metrics = GossipShedMetrics::new();

        for i in 0..4u32 {
            sink.emit_block(make_event(i), &metrics);
        }
        assert_eq!(metrics.blocks_dropped(), 0, "the 4 slots are filled");

        let start = std::time::Instant::now();
        for i in 4..504u32 {
            sink.emit_block(make_event(i), &metrics);
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 50,
            "emit_block must be non-blocking; took {}ms",
            elapsed.as_millis()
        );
        assert_eq!(metrics.blocks_dropped(), 500);
    }

    // TEST 4: a closed lane drops without panic, and does NOT count as a
    // block drop (shutdown, not load shedding).
    #[tokio::test]
    async fn closed_channel_drops_without_panic() {
        let (sink, lanes) = lanes_with_cap(8);
        let metrics = GossipShedMetrics::new();

        drop(lanes);

        sink.emit_block(make_event(1), &metrics);
        sink.emit_block(make_event(2), &metrics);

        assert_eq!(
            metrics.blocks_dropped(),
            0,
            "closed-lane drops must not increment block_drop counter"
        );
    }

    // TEST 5: the drop counter increments on every drop; the warn is
    // throttled to one per WARN_EVERY_N_DROPS drops.
    #[tokio::test]
    async fn warn_throttle_bounds_log_emission() {
        let (sink, _lanes) = lanes_with_cap(1);
        let metrics = GossipShedMetrics::new();

        sink.emit_block(make_event(0), &metrics);
        assert_eq!(metrics.blocks_dropped(), 0);

        for i in 1..=300u32 {
            sink.emit_block(make_event(i), &metrics);
        }
        assert_eq!(metrics.blocks_dropped(), 300);
        assert_eq!(
            WARN_EVERY_N_DROPS, 100,
            "throttle constant must be 100 (test assumption)"
        );
    }

    // TEST 6: GossipShedMetrics::default() starts at zero.
    #[test]
    fn metrics_default_is_zero() {
        let m = GossipShedMetrics::default();
        assert_eq!(m.blocks_dropped(), 0);
    }

    // TEST 7: Path coverage — Ok branch delivers the event intact.
    #[tokio::test]
    async fn path_ok_delivers_event() {
        let (sink, mut lanes) = lanes_with_cap(4);
        let metrics = GossipShedMetrics::new();

        sink.emit_block(make_event(42), &metrics);

        let event = lanes.try_recv().expect("event must be in the lane");
        match event {
            NetworkEvent::NewBlock(block, ..) => {
                assert_eq!(block.header.slot, 42, "block slot must match");
            }
            _ => panic!("expected NewBlock event"),
        }
        assert_eq!(metrics.blocks_dropped(), 0);
    }

    // TEST 8: Path coverage — Full branch sheds and counts.
    #[tokio::test]
    async fn path_full_sheds_and_counts() {
        let (sink, _lanes) = lanes_with_cap(2);
        let metrics = GossipShedMetrics::new();

        sink.emit_block(make_event(1), &metrics);
        sink.emit_block(make_event(2), &metrics);
        assert_eq!(metrics.blocks_dropped(), 0);

        sink.emit_block(make_event(3), &metrics);
        assert_eq!(metrics.blocks_dropped(), 1);
    }

    // TEST 9: Path coverage — Closed branch drops without counting.
    #[tokio::test]
    async fn path_closed_drops_silently() {
        let (sink, lanes) = lanes_with_cap(4);
        let metrics = GossipShedMetrics::new();
        drop(lanes);

        sink.emit_block(make_event(1), &metrics);
        assert_eq!(metrics.blocks_dropped(), 0, "closed = shutdown, not shed");
    }
}
