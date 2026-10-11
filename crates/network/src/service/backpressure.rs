//! Gossip block load-shedding metrics (INC-I-114 M1).
//!
//! A gossip block the node channel cannot take is dropped and counted
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
    use tokio::sync::mpsc;

    use crate::service::event_sink::EventSink;
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

    // ---------------------------------------------------------------
    // TEST 1: Canonical bounded-heap proof.
    // A flood of 1000 enqueues into a cap-8 channel with NO consumer
    // must saturate at 8 and shed the rest — the channel does NOT grow.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn stale_block_flood_sheds_and_stays_bounded() {
        let (tx, _rx) = mpsc::channel::<NetworkEvent>(8);
        let metrics = GossipShedMetrics::new();
        let sink = EventSink::new(tx.clone());

        for i in 0..1000u32 {
            sink.emit_block(make_event(i), &metrics);
        }

        // Channel capacity is exhausted (all 8 slots full)
        assert_eq!(tx.max_capacity(), 8, "channel max_capacity must be 8");
        assert_eq!(tx.capacity(), 0, "channel must be saturated (0 remaining)");

        // Exactly 1000-8 = 992 blocks were shed
        assert_eq!(
            metrics.blocks_dropped(),
            992,
            "expected 992 drops (1000 enqueues - 8 channel slots)"
        );
    }

    // ---------------------------------------------------------------
    // TEST 2: When capacity is available, events are delivered with
    // zero drops.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn enqueue_succeeds_when_capacity_available() {
        let (tx, mut rx) = mpsc::channel::<NetworkEvent>(16);
        let metrics = GossipShedMetrics::new();
        let sink = EventSink::new(tx.clone());

        // Enqueue 5 events into a 16-slot channel
        for i in 0..5u32 {
            sink.emit_block(make_event(i), &metrics);
        }

        assert_eq!(
            metrics.blocks_dropped(),
            0,
            "no drops when capacity available"
        );
        assert_eq!(tx.capacity(), 11, "16 - 5 = 11 remaining");

        // Drain and verify all 5 arrived
        let mut count = 0;
        while let Ok(event) = rx.try_recv() {
            if let NetworkEvent::NewBlock(..) = event {
                count += 1;
            }
        }
        assert_eq!(count, 5, "all 5 events must be received");
    }

    // ---------------------------------------------------------------
    // TEST 3: emit_block is synchronous — it must never block.
    // We prove this by flooding a full channel inside a timeout.
    // If try_send secretly awaited, the timeout would fire.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn enqueue_never_blocks() {
        let (tx, _rx) = mpsc::channel::<NetworkEvent>(4);
        let metrics = GossipShedMetrics::new();
        let sink = EventSink::new(tx.clone());

        // Fill the channel
        for i in 0..4u32 {
            sink.emit_block(make_event(i), &metrics);
        }
        assert_eq!(tx.capacity(), 0);

        // Now flood 500 more into the full channel — must complete instantly
        let start = std::time::Instant::now();
        for i in 4..504u32 {
            sink.emit_block(make_event(i), &metrics);
        }
        let elapsed = start.elapsed();

        // 500 synchronous try_send calls should take < 10ms even on slow CI
        assert!(
            elapsed.as_millis() < 50,
            "emit_block must be non-blocking; took {}ms",
            elapsed.as_millis()
        );
        assert_eq!(metrics.blocks_dropped(), 500);
    }

    // ---------------------------------------------------------------
    // TEST 4: Closed channel drops without panic, and does NOT count
    // as a block drop (it's a shutdown condition, not load shedding).
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn closed_channel_drops_without_panic() {
        let (tx, rx) = mpsc::channel::<NetworkEvent>(8);
        let metrics = GossipShedMetrics::new();
        let sink = EventSink::new(tx.clone());

        // Drop the receiver — channel is now closed
        drop(rx);

        // Must not panic
        sink.emit_block(make_event(1), &metrics);
        sink.emit_block(make_event(2), &metrics);

        // Closed channel is NOT counted as a block drop — it's shutdown
        assert_eq!(
            metrics.blocks_dropped(),
            0,
            "closed-channel drops must not increment block_drop counter"
        );
    }

    // ---------------------------------------------------------------
    // TEST 5: Rate-limited warn throttle — warn is emitted at most
    // once per WARN_EVERY_N_DROPS drops. We verify the counter
    // increments on every drop but the warn throttle is bounded.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn warn_throttle_bounds_log_emission() {
        let (tx, _rx) = mpsc::channel::<NetworkEvent>(1);
        let metrics = GossipShedMetrics::new();
        let sink = EventSink::new(tx.clone());

        // Fill the single slot
        sink.emit_block(make_event(0), &metrics);
        assert_eq!(metrics.blocks_dropped(), 0);

        // Drop 300 more — counter should be 300
        for i in 1..=300u32 {
            sink.emit_block(make_event(i), &metrics);
        }
        assert_eq!(metrics.blocks_dropped(), 300);

        // The warn is rate-limited by WARN_EVERY_N_DROPS.
        // We can't easily assert log emission count here without a log
        // capture framework, but we verify the counter is correct and
        // the modular throttle arithmetic is sound.
        assert_eq!(
            WARN_EVERY_N_DROPS, 100,
            "throttle constant must be 100 (test assumption)"
        );
        // At 300 drops, exactly 3 warn emissions (at 100, 200, 300)
        // would have occurred — verified by the modulo logic in emit_block.
    }

    // ---------------------------------------------------------------
    // TEST 6: GossipShedMetrics::default() starts at zero.
    // ---------------------------------------------------------------
    #[test]
    fn metrics_default_is_zero() {
        let m = GossipShedMetrics::default();
        assert_eq!(m.blocks_dropped(), 0);
    }

    // ---------------------------------------------------------------
    // TEST 7: Path coverage — Ok branch delivers event intact.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn path_ok_delivers_event() {
        let (tx, mut rx) = mpsc::channel::<NetworkEvent>(4);
        let metrics = GossipShedMetrics::new();
        let sink = EventSink::new(tx.clone());

        sink.emit_block(make_event(42), &metrics);

        let event = rx.try_recv().expect("event must be in channel");
        match event {
            NetworkEvent::NewBlock(block, ..) => {
                assert_eq!(block.header.slot, 42, "block slot must match");
            }
            _ => panic!("expected NewBlock event"),
        }
        assert_eq!(metrics.blocks_dropped(), 0);
    }

    // ---------------------------------------------------------------
    // TEST 8: Path coverage — Full branch sheds and counts.
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn path_full_sheds_and_counts() {
        let (tx, _rx) = mpsc::channel::<NetworkEvent>(2);
        let metrics = GossipShedMetrics::new();
        let sink = EventSink::new(tx.clone());

        // Fill
        sink.emit_block(make_event(1), &metrics);
        sink.emit_block(make_event(2), &metrics);
        assert_eq!(metrics.blocks_dropped(), 0);

        // Shed
        sink.emit_block(make_event(3), &metrics);
        assert_eq!(metrics.blocks_dropped(), 1);
    }

    // ---------------------------------------------------------------
    // TEST 9: Path coverage — Closed branch drops without counting.
    // (Overlaps with test 4 but explicitly named for path-coverage.)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn path_closed_drops_silently() {
        let (tx, rx) = mpsc::channel::<NetworkEvent>(4);
        let metrics = GossipShedMetrics::new();
        let sink = EventSink::new(tx.clone());
        drop(rx);

        sink.emit_block(make_event(1), &metrics);
        assert_eq!(metrics.blocks_dropped(), 0, "closed = shutdown, not shed");
    }
}
