//! Deterministic slot scheduler based on bond units.
//!
//! Each producer gets consecutive "tickets" equal to their bond units.
//! The primary producer for a slot is determined by: slot % total_tickets
//!
//! # Design
//!
//! This scheduler replaces the heartbeat/presence system with a simple,
//! deterministic round-robin selection based on bond count:
//!
//! - No network traffic for presence proofs
//! - Infinite scalability (no O(n) heartbeat broadcasts)
//! - Predictable producer selection for entire epochs
//!
//! # Fast Fallback with Block Verification
//!
//! This scheduler uses 1-second fallback windows (10 ranks for 10-second slots).
//! This aggressive fallback is safe because producers verify block existence
//! before producing:
//!
//! 1. Producer checks if a block already exists for the slot (early check)
//! 2. If no block, computes VDF (~700ms)
//! 3. Producer checks AGAIN after VDF (safety check)
//! 4. Only broadcasts if no block appeared during VDF computation
//!
//! This double-check prevents duplicate blocks while enabling fast failover.
//!
//! # Fallback Windows (1-second each)
//!
//! - 0-1s: rank 0 only (primary)
//! - 1-2s: rank 0-1
//! - 2-3s: rank 0-2
//! - ... (continues for each second)
//! - 9-10s: rank 0-9 (all 10 ranks)
//!
//! # Example
//!
//! With producers: Alice (3 bonds), Bob (2 bonds)
//! Total tickets = 5
//!
//! ```text
//! Slot 0: 0 % 5 = 0 -> ticket 0 (Alice, tickets 0-2)
//! Slot 1: 1 % 5 = 1 -> ticket 1 (Alice)
//! Slot 2: 2 % 5 = 2 -> ticket 2 (Alice)
//! Slot 3: 3 % 5 = 3 -> ticket 3 (Bob, tickets 3-4)
//! Slot 4: 4 % 5 = 4 -> ticket 4 (Bob)
//! Slot 5: 5 % 5 = 0 -> ticket 0 (Alice) [cycle repeats]
//! ```

use crypto::PublicKey;

use crate::types::Slot;

/// Maximum fallback producers (ranks 0-9, 10 total)
///
/// With 1-second fallback windows and block existence verification,
/// we can safely allow 10 ranks for 10-second slots. Each additional
/// second enables one more fallback rank.
pub const MAX_FALLBACK_RANK: usize = 9;

/// A producer scheduled for block production
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledProducer {
    /// Producer's public key
    pub pubkey: PublicKey,
    /// Number of bond units (tickets) this producer has
    pub bond_units: u32,
}

impl ScheduledProducer {
    /// Create a new scheduled producer
    pub fn new(pubkey: PublicKey, bond_units: u32) -> Self {
        Self { pubkey, bond_units }
    }

    /// Create a scheduled producer from total bond amount
    ///
    /// # Arguments
    /// * `pubkey` - Producer's public key
    /// * `bond_amount` - Total bond amount in base units
    /// * `bond_unit` - Bond unit size (use `NetworkParams::bond_unit()`)
    pub fn from_bond_amount(pubkey: PublicKey, bond_amount: u64, bond_unit: u64) -> Self {
        let bond_units = (bond_amount / bond_unit) as u32;
        Self { pubkey, bond_units }
    }
}

/// Deterministic scheduler for producer selection
///
/// Uses a ticket-based system where each producer gets consecutive
/// tickets equal to their bond units. The scheduler maintains a sorted
/// list of producers and uses binary search for efficient selection.
#[derive(Clone, Debug)]
pub struct DeterministicScheduler {
    /// Producers sorted by pubkey, with their bond units
    producers: Vec<ScheduledProducer>,
    /// Total bond units across all producers
    total_bonds: u64,
    /// Cumulative ticket boundaries for binary search
    /// ticket_boundaries[i] = sum of bond_units for producers[0..=i]
    ticket_boundaries: Vec<u64>,
}

impl DeterministicScheduler {
    /// Create a new scheduler from active producers.
    ///
    /// Producers are sorted by public key for deterministic ordering.
    /// Each producer receives consecutive tickets equal to their bond units.
    pub fn new(mut producers: Vec<ScheduledProducer>) -> Self {
        // Sort by pubkey for deterministic ordering
        producers.sort_by(|a, b| a.pubkey.as_bytes().cmp(b.pubkey.as_bytes()));

        // Filter out producers with 0 bonds
        producers.retain(|p| p.bond_units > 0);

        // Calculate ticket boundaries for binary search
        let mut ticket_boundaries = Vec::with_capacity(producers.len());
        let mut cumulative: u64 = 0;

        for producer in &producers {
            cumulative += producer.bond_units as u64;
            ticket_boundaries.push(cumulative);
        }

        let total_bonds = cumulative;

        Self {
            producers,
            total_bonds,
            ticket_boundaries,
        }
    }

    /// Create an empty scheduler (for testing or when no producers exist)
    pub fn empty() -> Self {
        Self {
            producers: Vec::new(),
            total_bonds: 0,
            ticket_boundaries: Vec::new(),
        }
    }

    /// Get the number of active producers
    pub fn producer_count(&self) -> usize {
        self.producers.len()
    }

    /// Get total bond units in the scheduler
    pub fn total_bonds(&self) -> u64 {
        self.total_bonds
    }

    /// Check if scheduler has any producers
    pub fn is_empty(&self) -> bool {
        self.producers.is_empty()
    }

    /// Get all scheduled producers
    pub fn producers(&self) -> &[ScheduledProducer] {
        &self.producers
    }

    /// Select the producer for a given slot at a specific rank.
    ///
    /// - Rank 0: Primary producer (ticket at `slot % total_bonds`)
    /// - Rank 1-9: Fallback producers with evenly distributed offsets
    ///
    /// Each rank gets an offset of `total_bonds * rank / 10`, ensuring
    /// fallback producers are spread across the ticket space.
    ///
    /// Returns None if:
    /// - No producers registered
    /// - Rank > MAX_FALLBACK_RANK
    pub fn select_producer(&self, slot: Slot, rank: usize) -> Option<&PublicKey> {
        if self.producers.is_empty() || rank > MAX_FALLBACK_RANK {
            return None;
        }

        // Calculate ticket offset based on rank
        // Evenly distributed: rank 0 = 0%, rank 1 = 10%, rank 2 = 20%, etc.
        let offset = (self.total_bonds * rank as u64) / 10;

        // Calculate ticket number for this slot
        let ticket = ((slot as u64) + offset) % self.total_bonds;

        // Binary search for the producer owning this ticket
        self.producer_at_ticket(ticket)
    }

    /// Get the public key of the producer owning a specific ticket.
    fn producer_at_ticket(&self, ticket: u64) -> Option<&PublicKey> {
        if self.producers.is_empty() {
            return None;
        }

        // Binary search in ticket_boundaries
        // We're looking for the first boundary > ticket
        let idx = self
            .ticket_boundaries
            .partition_point(|&boundary| boundary <= ticket);

        self.producers.get(idx).map(|p| &p.pubkey)
    }

    /// Get eligible producers based on elapsed time in slot.
    ///
    /// Returns producers in order of priority:
    /// - 0-3s: [rank 0]
    /// - 3-6s: [rank 0, rank 1]
    /// - 6-10s: [rank 0, rank 1, rank 2]
    pub fn eligible_producers(&self, slot: Slot, elapsed_secs: u64) -> Vec<&PublicKey> {
        let max_rank = allowed_producer_rank(elapsed_secs);
        let mut eligible = Vec::with_capacity(max_rank + 1);

        for rank in 0..=max_rank {
            if let Some(pubkey) = self.select_producer(slot, rank) {
                // Avoid duplicates (can happen with small producer sets)
                if !eligible.contains(&pubkey) {
                    eligible.push(pubkey);
                }
            }
        }

        eligible
    }

    /// Get the rank of a producer for a given slot.
    ///
    /// Returns:
    /// - Some(0): Primary producer
    /// - Some(1): Secondary fallback
    /// - Some(2): Tertiary fallback
    /// - None: Not in top 3 eligible producers
    pub fn producer_rank(&self, slot: Slot, pubkey: &PublicKey) -> Option<usize> {
        for rank in 0..=MAX_FALLBACK_RANK {
            if let Some(selected) = self.select_producer(slot, rank) {
                if selected == pubkey {
                    return Some(rank);
                }
            }
        }
        None
    }

    /// Check if a producer is eligible at a given time.
    pub fn is_producer_eligible(&self, slot: Slot, pubkey: &PublicKey, elapsed_secs: u64) -> bool {
        if let Some(rank) = self.producer_rank(slot, pubkey) {
            let max_rank = allowed_producer_rank(elapsed_secs);
            rank <= max_rank
        } else {
            false
        }
    }

    /// Calculate slots until this producer's next primary slot.
    ///
    /// Returns None if producer is not in the scheduler.
    pub fn slots_until_next(&self, current_slot: Slot, pubkey: &PublicKey) -> Option<u64> {
        // Find producer's ticket range
        let producer_idx = self.producers.iter().position(|p| &p.pubkey == pubkey)?;

        // Calculate first ticket owned by this producer
        let first_ticket = if producer_idx == 0 {
            0
        } else {
            self.ticket_boundaries[producer_idx - 1]
        };

        // Current slot's ticket
        let current_ticket = (current_slot as u64) % self.total_bonds;

        // Calculate slots until producer's first ticket comes up
        if current_ticket <= first_ticket {
            Some(first_ticket - current_ticket)
        } else {
            // Wrap around
            Some(self.total_bonds - current_ticket + first_ticket)
        }
    }

    /// Get statistics about producer distribution
    pub fn stats(&self) -> SchedulerStats {
        if self.producers.is_empty() {
            return SchedulerStats::default();
        }

        let bond_counts: Vec<u32> = self.producers.iter().map(|p| p.bond_units).collect();
        let min_bonds = *bond_counts.iter().min().unwrap();
        let max_bonds = *bond_counts.iter().max().unwrap();
        let avg_bonds = self.total_bonds as f64 / self.producers.len() as f64;

        SchedulerStats {
            producer_count: self.producers.len(),
            total_bonds: self.total_bonds,
            min_bonds,
            max_bonds,
            avg_bonds,
        }
    }
}

/// Statistics about the scheduler's producer distribution
#[derive(Clone, Debug, Default)]
pub struct SchedulerStats {
    pub producer_count: usize,
    pub total_bonds: u64,
    pub min_bonds: u32,
    pub max_bonds: u32,
    pub avg_bonds: f64,
}

/// Determine the maximum allowed producer rank based on elapsed time.
///
/// # Fast Fallback Windows (1-second each)
///
/// With block existence verification before production, we can use
/// aggressive 1-second windows:
///
/// - 0-1s: rank 0 only (primary)
/// - 1-2s: rank 0-1
/// - 2-3s: rank 0-2
/// - 3-4s: rank 0-3
/// - 4-5s: rank 0-4
/// - 5-6s: rank 0-5
/// - 6-7s: rank 0-6
/// - 7-8s: rank 0-7
/// - 8-9s: rank 0-8
/// - 9-10s: rank 0-9 (all 10 ranks)
///
/// The safety comes from producers checking if a block already exists
/// for the slot BEFORE and AFTER VDF computation.
pub fn allowed_producer_rank(elapsed_secs: u64) -> usize {
    // Each second unlocks one more rank, capped at MAX_FALLBACK_RANK
    std::cmp::min(elapsed_secs as usize, MAX_FALLBACK_RANK)
}

/// Determine the maximum allowed producer rank based on elapsed time (milliseconds).
///
/// Same as `allowed_producer_rank` but with millisecond precision.
/// Each 1000ms unlocks one more fallback rank.
pub fn allowed_producer_rank_ms(elapsed_ms: u64) -> usize {
    let elapsed_secs = elapsed_ms / 1000;
    std::cmp::min(elapsed_secs as usize, MAX_FALLBACK_RANK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pubkey(seed: u8) -> PublicKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        PublicKey::from_bytes(bytes)
    }

    #[test]
    fn test_empty_scheduler() {
        let scheduler = DeterministicScheduler::empty();
        assert!(scheduler.is_empty());
        assert_eq!(scheduler.producer_count(), 0);
        assert_eq!(scheduler.total_bonds(), 0);
        assert!(scheduler.select_producer(0, 0).is_none());
    }

    #[test]
    fn test_single_producer() {
        let pubkey = make_pubkey(1);
        let scheduler =
            DeterministicScheduler::new(vec![ScheduledProducer::new(pubkey.clone(), 5)]);

        assert_eq!(scheduler.producer_count(), 1);
        assert_eq!(scheduler.total_bonds(), 5);

        // Single producer gets all slots
        for slot in 0..10 {
            assert_eq!(scheduler.select_producer(slot, 0), Some(&pubkey));
        }
    }

    #[test]
    fn test_consecutive_tickets() {
        // Alice: 3 tickets, Bob: 2 tickets (sorted by pubkey)
        let alice = make_pubkey(1);
        let bob = make_pubkey(2);

        let scheduler = DeterministicScheduler::new(vec![
            ScheduledProducer::new(alice.clone(), 3),
            ScheduledProducer::new(bob.clone(), 2),
        ]);

        assert_eq!(scheduler.total_bonds(), 5);

        // Verify consecutive ticket assignment
        // Alice gets tickets 0, 1, 2
        // Bob gets tickets 3, 4
        assert_eq!(scheduler.select_producer(0, 0), Some(&alice));
        assert_eq!(scheduler.select_producer(1, 0), Some(&alice));
        assert_eq!(scheduler.select_producer(2, 0), Some(&alice));
        assert_eq!(scheduler.select_producer(3, 0), Some(&bob));
        assert_eq!(scheduler.select_producer(4, 0), Some(&bob));
        // Cycle repeats
        assert_eq!(scheduler.select_producer(5, 0), Some(&alice));
    }

    #[test]
    fn test_fallback_offsets() {
        let alice = make_pubkey(1);
        let bob = make_pubkey(2);
        let charlie = make_pubkey(3);

        let scheduler = DeterministicScheduler::new(vec![
            ScheduledProducer::new(alice.clone(), 10),
            ScheduledProducer::new(bob.clone(), 10),
            ScheduledProducer::new(charlie.clone(), 10),
        ]);

        // Total = 30 tickets
        // Each rank offset = total_bonds * rank / 10
        // Rank 0: offset = 0
        // Rank 1: offset = 3
        // Rank 2: offset = 6
        // Rank 3: offset = 9
        // etc.

        // Slot 0:
        // - Rank 0: ticket 0 -> Alice (tickets 0-9)
        // - Rank 1: ticket 3 -> Alice (still in 0-9)
        // - Rank 2: ticket 6 -> Alice (still in 0-9)
        // - Rank 3: ticket 9 -> Alice (still in 0-9)
        // - Rank 4: ticket 12 -> Bob (tickets 10-19)
        assert_eq!(scheduler.select_producer(0, 0), Some(&alice));
        assert_eq!(scheduler.select_producer(0, 1), Some(&alice));
        assert_eq!(scheduler.select_producer(0, 2), Some(&alice));
        assert_eq!(scheduler.select_producer(0, 3), Some(&alice));
        assert_eq!(scheduler.select_producer(0, 4), Some(&bob));
    }

    #[test]
    fn test_producer_rank() {
        let alice = make_pubkey(1);
        let bob = make_pubkey(2);

        let scheduler = DeterministicScheduler::new(vec![
            ScheduledProducer::new(alice.clone(), 3),
            ScheduledProducer::new(bob.clone(), 3),
        ]);

        // At slot 0, Alice is rank 0
        assert_eq!(scheduler.producer_rank(0, &alice), Some(0));

        // At slot 3, Bob is rank 0 and Alice is fallback
        assert_eq!(scheduler.producer_rank(3, &bob), Some(0));
    }

    #[test]
    fn test_eligible_producers() {
        let alice = make_pubkey(1);
        let bob = make_pubkey(2);
        let charlie = make_pubkey(3);

        let scheduler = DeterministicScheduler::new(vec![
            ScheduledProducer::new(alice.clone(), 10),
            ScheduledProducer::new(bob.clone(), 10),
            ScheduledProducer::new(charlie.clone(), 10),
        ]);

        // At 0 seconds, only primary is eligible (rank 0)
        let eligible = scheduler.eligible_producers(0, 0);
        assert_eq!(eligible.len(), 1);

        // At 1 second, ranks 0-1 eligible
        let eligible = scheduler.eligible_producers(0, 1);
        assert!(eligible.len() >= 1);

        // At 5 seconds, ranks 0-5 eligible
        let eligible = scheduler.eligible_producers(0, 5);
        assert!(eligible.len() >= 1);

        // At 9 seconds, all 10 ranks eligible
        let eligible = scheduler.eligible_producers(0, 9);
        assert!(eligible.len() >= 1);
    }

    #[test]
    fn test_is_producer_eligible() {
        let alice = make_pubkey(1);
        let bob = make_pubkey(2);

        let scheduler = DeterministicScheduler::new(vec![
            ScheduledProducer::new(alice.clone(), 10),
            ScheduledProducer::new(bob.clone(), 10),
        ]);

        // Total = 20 bonds
        // Slot 0:
        //   Rank 0: ticket 0 -> Alice (tickets 0-9)
        //   Rank 1: ticket 2 (20/10) -> Alice
        //   Rank 2: ticket 4 (20*2/10) -> Alice
        //   ...
        //   Rank 5: ticket 10 (20*5/10) -> Bob (tickets 10-19)

        // Alice is primary at slot 0
        assert!(scheduler.is_producer_eligible(0, &alice, 0));
        assert!(scheduler.is_producer_eligible(0, &alice, 5));

        // Bob is rank 5 at slot 0 (ticket 10), not eligible at 0 seconds
        assert!(!scheduler.is_producer_eligible(0, &bob, 0));
        // Not eligible at 4 seconds (rank 5 needs 5+ seconds)
        assert!(!scheduler.is_producer_eligible(0, &bob, 4));
        // Eligible at 5 seconds (rank 5 window starts)
        assert!(scheduler.is_producer_eligible(0, &bob, 5));
    }

    #[test]
    fn test_slots_until_next() {
        let alice = make_pubkey(1);
        let bob = make_pubkey(2);

        let scheduler = DeterministicScheduler::new(vec![
            ScheduledProducer::new(alice.clone(), 3),
            ScheduledProducer::new(bob.clone(), 2),
        ]);

        // At slot 0, Alice's next primary is immediately (slot 0)
        assert_eq!(scheduler.slots_until_next(0, &alice), Some(0));

        // At slot 0, Bob's next primary is slot 3
        assert_eq!(scheduler.slots_until_next(0, &bob), Some(3));

        // At slot 4, Alice's next primary is slot 5
        assert_eq!(scheduler.slots_until_next(4, &alice), Some(1));
    }

    #[test]
    fn test_allowed_producer_rank() {
        // 1-second windows: each second unlocks one more rank

        // 0-1s: rank 0
        assert_eq!(allowed_producer_rank(0), 0);

        // 1-2s: rank 1
        assert_eq!(allowed_producer_rank(1), 1);

        // 2-3s: rank 2
        assert_eq!(allowed_producer_rank(2), 2);

        // 5-6s: rank 5
        assert_eq!(allowed_producer_rank(5), 5);

        // 9-10s: rank 9 (max)
        assert_eq!(allowed_producer_rank(9), 9);

        // Beyond 10s: still capped at rank 9
        assert_eq!(allowed_producer_rank(10), 9);
        assert_eq!(allowed_producer_rank(100), 9);
    }

    #[test]
    fn test_allowed_producer_rank_ms() {
        // 0-999ms: rank 0
        assert_eq!(allowed_producer_rank_ms(0), 0);
        assert_eq!(allowed_producer_rank_ms(999), 0);

        // 1000-1999ms: rank 1
        assert_eq!(allowed_producer_rank_ms(1000), 1);
        assert_eq!(allowed_producer_rank_ms(1999), 1);

        // 5000-5999ms: rank 5
        assert_eq!(allowed_producer_rank_ms(5000), 5);
        assert_eq!(allowed_producer_rank_ms(5999), 5);

        // 9000-9999ms: rank 9 (max)
        assert_eq!(allowed_producer_rank_ms(9000), 9);
        assert_eq!(allowed_producer_rank_ms(9999), 9);

        // Beyond 10s: capped at rank 9
        assert_eq!(allowed_producer_rank_ms(10000), 9);
    }

    #[test]
    fn test_from_bond_amount() {
        let pubkey = make_pubkey(1);
        // Use mainnet bond_unit: 100 DOLI = 10_000_000_000 base units
        let bond_unit = 10_000_000_000u64;

        // 100 DOLI = 1 bond unit
        let producer =
            ScheduledProducer::from_bond_amount(pubkey.clone(), 10_000_000_000, bond_unit);
        assert_eq!(producer.bond_units, 1);

        // 1000 DOLI = 10 bond units
        let producer =
            ScheduledProducer::from_bond_amount(pubkey.clone(), 100_000_000_000, bond_unit);
        assert_eq!(producer.bond_units, 10);

        // Partial bonds round down
        let producer =
            ScheduledProducer::from_bond_amount(pubkey.clone(), 15_000_000_000, bond_unit);
        assert_eq!(producer.bond_units, 1); // 150 DOLI = 1 bond (rounds down)
    }

    #[test]
    fn test_scheduler_stats() {
        let scheduler = DeterministicScheduler::new(vec![
            ScheduledProducer::new(make_pubkey(1), 3),
            ScheduledProducer::new(make_pubkey(2), 5),
            ScheduledProducer::new(make_pubkey(3), 2),
        ]);

        let stats = scheduler.stats();
        assert_eq!(stats.producer_count, 3);
        assert_eq!(stats.total_bonds, 10);
        assert_eq!(stats.min_bonds, 2);
        assert_eq!(stats.max_bonds, 5);
    }

    #[test]
    fn test_deterministic_ordering() {
        // Order of input shouldn't matter - sorted by pubkey
        let alice = make_pubkey(1);
        let bob = make_pubkey(2);

        let scheduler1 = DeterministicScheduler::new(vec![
            ScheduledProducer::new(alice.clone(), 3),
            ScheduledProducer::new(bob.clone(), 2),
        ]);

        let scheduler2 = DeterministicScheduler::new(vec![
            ScheduledProducer::new(bob.clone(), 2),
            ScheduledProducer::new(alice.clone(), 3),
        ]);

        // Both should select same producer for each slot
        for slot in 0..10 {
            assert_eq!(
                scheduler1.select_producer(slot, 0),
                scheduler2.select_producer(slot, 0)
            );
        }
    }

    #[test]
    fn test_zero_bond_filtered() {
        let alice = make_pubkey(1);
        let bob = make_pubkey(2);

        let scheduler = DeterministicScheduler::new(vec![
            ScheduledProducer::new(alice.clone(), 0), // Should be filtered
            ScheduledProducer::new(bob.clone(), 5),
        ]);

        assert_eq!(scheduler.producer_count(), 1);
        assert_eq!(scheduler.select_producer(0, 0), Some(&bob));
    }
}
