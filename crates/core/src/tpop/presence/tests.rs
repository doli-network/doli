//! Tests for the presence system.

use super::*;
use crypto::{Hash, PublicKey};

#[test]
fn test_presence_score_calculation() {
    // New producer with no history
    assert_eq!(calculate_presence_score(0, 0, 0, 0), 0);

    // Producer with perfect short history
    assert_eq!(calculate_presence_score(100, 100, 0, 0), 100); // 100 consecutive

    // Producer with perfect long history
    // consecutive: 10000, history: (10000/1000)*100 = 1000
    assert_eq!(calculate_presence_score(10_000, 10_000, 0, 0), 11_000);

    // Producer with 50% reliability
    // consecutive: 0, history: (1000/1000)*50 = 50
    assert_eq!(calculate_presence_score(0, 1000, 500, 0), 50);

    // Producer with age bonus (era 4 = log2(4)+1 = 3, *5 = 15)
    assert_eq!(calculate_presence_score(0, 0, 0, 4), 15);

    // Score capping
    assert!(
        calculate_presence_score(MAX_PRESENCE_SCORE * 2, MAX_PRESENCE_SCORE * 2, 0, 100)
            <= MAX_PRESENCE_SCORE
    );
}

#[test]
fn test_eligibility_windows() {
    // Rank 0 can produce immediately
    assert!(can_produce_at_time(0, 0));
    assert!(can_produce_at_time(0, 30));

    // Rank 1-2 must wait until 15s
    assert!(!can_produce_at_time(1, 0));
    assert!(!can_produce_at_time(2, 14));
    assert!(can_produce_at_time(1, 15));
    assert!(can_produce_at_time(2, 15));

    // Rank 3-9 must wait until 30s
    assert!(!can_produce_at_time(5, 29));
    assert!(can_produce_at_time(5, 30));
    assert!(can_produce_at_time(9, 30));

    // Rank 10-99 must wait until 45s
    assert!(!can_produce_at_time(50, 44));
    assert!(can_produce_at_time(50, 45));
    assert!(can_produce_at_time(99, 45));

    // Rank 100+ cannot produce
    assert!(!can_produce_at_time(100, 59));
}

#[test]
fn test_producer_eligibility_offset() {
    assert_eq!(producer_eligibility_offset(0), 0);
    assert_eq!(producer_eligibility_offset(2), 15);
    assert_eq!(producer_eligibility_offset(9), 30);
    assert_eq!(producer_eligibility_offset(99), 45);
    assert_eq!(producer_eligibility_offset(100), u64::MAX);
}

#[test]
fn test_vdf_link_input_determinism() {
    let output1 = vec![1, 2, 3];
    let output2 = vec![1, 2, 3];
    let slot = 42;
    let producer = PublicKey::from_bytes([7u8; 32]);

    let input1 = VdfLink::compute_input(&output1, slot, &producer);
    let input2 = VdfLink::compute_input(&output2, slot, &producer);

    assert_eq!(input1, input2);

    // Different inputs produce different hashes
    let input3 = VdfLink::compute_input(&output1, slot + 1, &producer);
    assert_ne!(input1, input3);
}

#[test]
fn test_epoch_presence_records() {
    let mut records = EpochPresenceRecords::new();
    let producer = PublicKey::from_bytes([1u8; 32]);

    assert_eq!(records.slots_with_valid_proof(&producer), 0);

    records.record_presence(&producer, 10);
    records.record_presence(&producer, 11);
    records.record_presence(&producer, 12);

    assert_eq!(records.slots_with_valid_proof(&producer), 3);

    let other = PublicKey::from_bytes([2u8; 32]);
    assert_eq!(records.slots_with_valid_proof(&other), 0);
}

#[test]
fn test_rank_producers_includes_bond_count() {
    // Test that rank_producers_by_presence includes bond_count in output
    let producer1 = PublicKey::from_bytes([1u8; 32]);
    let producer2 = PublicKey::from_bytes([2u8; 32]);

    let checkpoint = PresenceCheckpoint::new(
        0,
        0,
        Hash::default(),
        vec![
            ProducerPresenceState::with_bonds(producer1, 0, 5), // 5 bonds
            ProducerPresenceState::with_bonds(producer2, 0, 10), // 10 bonds
        ],
    );

    let rankings = rank_producers_by_presence(&checkpoint);

    assert_eq!(rankings.len(), 2);
    // Both have same score (0), so order might vary
    // Just verify bond counts are included correctly
    let prod1_entry = rankings
        .iter()
        .find(|(pk, _, _, _)| pk == &producer1)
        .unwrap();
    let prod2_entry = rankings
        .iter()
        .find(|(pk, _, _, _)| pk == &producer2)
        .unwrap();

    assert_eq!(prod1_entry.2, 5); // bond_count
    assert_eq!(prod2_entry.2, 10); // bond_count
}

#[test]
fn test_deterministic_round_robin_selection() {
    // Test deterministic round-robin rotation (NOT lottery)
    //
    // With Alice:1, Bob:5, Carol:4 bonds (total 10):
    // - Alice gets slot 0 (1 turn per cycle)
    // - Bob gets slots 1-5 (5 turns per cycle)
    // - Carol gets slots 6-9 (4 turns per cycle)
    //
    // This is DETERMINISTIC, not probabilistic!

    // Create producers sorted by pubkey bytes (this is how production sorts)
    let alice = PublicKey::from_bytes([0x01; 32]); // Lowest pubkey
    let bob = PublicKey::from_bytes([0x02; 32]); // Middle pubkey
    let carol = PublicKey::from_bytes([0x03; 32]); // Highest pubkey

    // Eligible producers: (pubkey, bonds) - will be sorted by pubkey
    let mut eligible: Vec<(&PublicKey, u32)> = vec![
        (&alice, 1), // 1 bond = 1 ticket
        (&bob, 5),   // 5 bonds = 5 tickets
        (&carol, 4), // 4 bonds = 4 tickets
    ];
    eligible.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let total_tickets: u64 = eligible.iter().map(|(_, bonds)| *bonds as u64).sum();
    assert_eq!(total_tickets, 10);

    // Verify exact assignments for a full cycle
    // Sorted order: Alice (0x01), Bob (0x02), Carol (0x03)
    // Tickets: [Alice, Bob, Bob, Bob, Bob, Bob, Carol, Carol, Carol, Carol]
    // Indices:    0      1    2    3    4    5     6      7      8      9

    fn select_for_slot(eligible: &[(&PublicKey, u32)], slot: u64, total_tickets: u64) -> PublicKey {
        let ticket_index = slot % total_tickets;
        let mut cumulative: u64 = 0;
        for (pubkey, bonds) in eligible {
            cumulative += *bonds as u64;
            if ticket_index < cumulative {
                return *(*pubkey);
            }
        }
        *eligible.last().unwrap().0
    }

    // Test first cycle (slots 0-9)
    assert_eq!(select_for_slot(&eligible, 0, total_tickets), alice); // ticket 0 -> Alice
    assert_eq!(select_for_slot(&eligible, 1, total_tickets), bob); // ticket 1 -> Bob
    assert_eq!(select_for_slot(&eligible, 2, total_tickets), bob); // ticket 2 -> Bob
    assert_eq!(select_for_slot(&eligible, 3, total_tickets), bob); // ticket 3 -> Bob
    assert_eq!(select_for_slot(&eligible, 4, total_tickets), bob); // ticket 4 -> Bob
    assert_eq!(select_for_slot(&eligible, 5, total_tickets), bob); // ticket 5 -> Bob
    assert_eq!(select_for_slot(&eligible, 6, total_tickets), carol); // ticket 6 -> Carol
    assert_eq!(select_for_slot(&eligible, 7, total_tickets), carol); // ticket 7 -> Carol
    assert_eq!(select_for_slot(&eligible, 8, total_tickets), carol); // ticket 8 -> Carol
    assert_eq!(select_for_slot(&eligible, 9, total_tickets), carol); // ticket 9 -> Carol

    // Test cycle repeats (slot 10 = same as slot 0)
    assert_eq!(select_for_slot(&eligible, 10, total_tickets), alice);
    assert_eq!(select_for_slot(&eligible, 11, total_tickets), bob);
    assert_eq!(select_for_slot(&eligible, 16, total_tickets), carol);

    // Count over 100 slots - must be EXACTLY proportional
    let mut alice_count = 0u32;
    let mut bob_count = 0u32;
    let mut carol_count = 0u32;

    for slot in 0..100 {
        let selected = select_for_slot(&eligible, slot, total_tickets);
        if selected == alice {
            alice_count += 1;
        } else if selected == bob {
            bob_count += 1;
        } else if selected == carol {
            carol_count += 1;
        }
    }

    // EXACT counts, not approximate!
    assert_eq!(
        alice_count, 10,
        "Alice (1 bond) should produce exactly 10 of 100 blocks"
    );
    assert_eq!(
        bob_count, 50,
        "Bob (5 bonds) should produce exactly 50 of 100 blocks"
    );
    assert_eq!(
        carol_count, 40,
        "Carol (4 bonds) should produce exactly 40 of 100 blocks"
    );

    // ROI verification: all producers have same ROI percentage
    // Alice: 10 blocks / 1 bond = 10 blocks per bond
    // Bob: 50 blocks / 5 bonds = 10 blocks per bond
    // Carol: 40 blocks / 4 bonds = 10 blocks per bond
    assert_eq!(alice_count as f64 / 1.0, 10.0);
    assert_eq!(bob_count as f64 / 5.0, 10.0);
    assert_eq!(carol_count as f64 / 4.0, 10.0);
}

// =============================================================================
// PROPERTY-BASED TESTS
// =============================================================================

mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Presence score is always within bounds
        #[test]
        fn prop_score_bounded(
            consecutive in 0u64..1_000_000,
            total in 0u64..10_000_000,
            missed in 0u64..10_000_000,
            age in 0u32..100,
        ) {
            let score = calculate_presence_score(consecutive, total, missed, age);
            prop_assert!(score <= MAX_PRESENCE_SCORE);
        }

        /// Score is monotonically increasing with consecutive presence
        #[test]
        fn prop_score_increases_with_consecutive(
            base in 0u64..10_000,
            delta in 1u64..1000,
        ) {
            let score1 = calculate_presence_score(base, base, 0, 0);
            let score2 = calculate_presence_score(base + delta, base + delta, 0, 0);
            prop_assert!(score2 >= score1);
        }

        /// Score decreases with missed slots (given same total)
        #[test]
        fn prop_score_decreases_with_missed(
            total in 1000u64..100_000,
            missed in 1u64..999,
        ) {
            let score1 = calculate_presence_score(0, total, 0, 0);
            let score2 = calculate_presence_score(0, total, missed, 0);
            prop_assert!(score2 <= score1);
        }

        /// Eligibility windows are properly ordered
        #[test]
        fn prop_windows_ordered(rank in 0usize..200) {
            let offset = producer_eligibility_offset(rank);
            if offset < u64::MAX {
                // If eligible at offset, should be eligible at any later time
                prop_assert!(can_produce_at_time(rank, offset));
                prop_assert!(can_produce_at_time(rank, offset + 10));
            }
        }
    }
}
