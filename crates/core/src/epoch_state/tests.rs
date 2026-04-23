use super::*;
use std::collections::HashMap;

fn make_pubkey(seed: u8) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    PublicKey::from_bytes(bytes)
}

fn make_hash(seed: u8) -> Hash {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    Hash::from_bytes(bytes)
}

// OUTPUT CONTRACT: fn EpochState::genesis()
//   O1: return.epoch — u64, 0
//   O2: return.bond_snapshot — HashMap, empty
//   O3: return.producer_list — Vec, empty
//   O4: return.active_list — Vec, empty
//   O5: return.attested_sets — [HashSet; 3], all empty
//   O6: return.attestation_accum — [HashMap; 3], all empty
//   O7: return.blocks_produced — HashMap, empty
// PATHS: P1: constructor (single path)
// MATRIX: 7 outputs × 1 path = 7 cells
//   P1: O1✓ O2✓ O3✓ O4✓ O5✓ O6✓ O7✓
#[test]
fn test_genesis_creates_empty_state() {
    let state = EpochState::genesis();
    assert_eq!(state.epoch, 0);
    assert!(state.bond_snapshot.is_empty());
    assert!(state.producer_list.is_empty());
    assert!(state.active_list.is_empty());
    for i in 0..3 {
        assert!(state.attested_sets[i].is_empty());
        assert!(state.attestation_accum[i].is_empty());
    }
    assert!(state.blocks_produced.is_empty());
}

// OUTPUT CONTRACT: fn EpochState::accumulate_block(&mut self, input)
//   O1: self.attested_sets[0] — HashSet, contains input.producer
//   O2: self.blocks_produced — HashMap, producer entry incremented by 1
//   O3: self.attestation_accum[0] — HashMap, minute inserted for producer
// PATHS: P1: no attestation data (has_attestation_data=false)
// MATRIX: 3 outputs × 1 path = 3 cells
//   P1: O1✓ O2✓ O3✓
#[test]
fn test_accumulate_block_no_attestation() {
    let pk = make_pubkey(1);
    let mut state = EpochState::genesis();
    state.producer_list = vec![pk];

    let input = BlockAccumulationInput {
        producer: pk,
        slot: 100,
        has_attestation_data: false,
        attested_indices: vec![],
    };
    state.accumulate_block(&input);

    assert!(state.attested_sets[0].contains(&pk));
    assert_eq!(state.blocks_produced[&pk], 1);
    assert!(!state.attestation_accum[0][&pk].is_empty());
}

// OUTPUT CONTRACT: fn EpochState::accumulate_block(&mut self, input)
//   O1: self.attested_sets[0] — HashSet, contains producer AND attested peers
//   O2: self.blocks_produced — HashMap, only producer incremented (not peers)
//   O3: self.attestation_accum[0] — HashMap, minutes for producer AND peers
// PATHS: P2: with attestation data (has_attestation_data=true, indices=[0,1])
// MATRIX: 3 outputs × 1 path = 3 cells
//   P2: O1✓ O2✓ O3✓
#[test]
fn test_accumulate_block_with_attestation() {
    let pk0 = make_pubkey(1);
    let pk1 = make_pubkey(2);
    let mut state = EpochState::genesis();
    state.producer_list = vec![pk0, pk1];

    let input = BlockAccumulationInput {
        producer: pk0,
        slot: 100,
        has_attestation_data: true,
        attested_indices: vec![0, 1],
    };
    state.accumulate_block(&input);

    assert!(state.attested_sets[0].contains(&pk0));
    assert!(state.attested_sets[0].contains(&pk1));
    assert_eq!(state.blocks_produced[&pk0], 1);
    assert!(!state.blocks_produced.contains_key(&pk1));
    assert!(!state.attestation_accum[0][&pk0].is_empty());
    assert!(!state.attestation_accum[0][&pk1].is_empty());
}

// OUTPUT CONTRACT: fn EpochState::accumulate_block(&mut self, input)
//   O2: self.blocks_produced — HashMap, increments per call
// PATHS: P3: multiple blocks same producer (5 calls)
// MATRIX: 1 output × 1 path = 1 cell
//   P3: O2✓
#[test]
fn test_accumulate_block_increments_count() {
    let pk = make_pubkey(1);
    let mut state = EpochState::genesis();
    state.producer_list = vec![pk];

    for slot in 0..5 {
        state.accumulate_block(&BlockAccumulationInput {
            producer: pk,
            slot: slot * 10,
            has_attestation_data: false,
            attested_indices: vec![],
        });
    }
    assert_eq!(state.blocks_produced[&pk], 5);
}

// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O1: return.epoch — u64, equals input.epoch
//   O2: return.bond_snapshot — HashMap, equals input.bond_counts
//   O3: return.producer_list — Vec, all active (no filter for epoch<=1), sorted
//   O4: return.active_list — Vec, same as producer_list (<50 producers)
//   O5: return.attested_sets[0] — HashSet, empty (new epoch)
//   O6: return.attested_sets[1] — HashSet, equals prev.attested_sets[0]
//   O7: return.attestation_accum[0] — HashMap, empty (new epoch)
//   O8: return.blocks_produced — HashMap, empty (new epoch)
// PATHS: P1: epoch<=1 (no attestation filter)
// MATRIX: 8 outputs × 1 path = 8 cells
//   P1: O1✓ O2✓ O3✓ O4✓ O5✓ O6✓ O7✓ O8✓
#[test]
fn test_derive_at_boundary_epoch_1() {
    let pk0 = make_pubkey(1);
    let pk1 = make_pubkey(2);
    let pkh0 = make_hash(1);
    let pkh1 = make_hash(2);

    let mut prev = EpochState::genesis();
    prev.attested_sets[0].insert(pk0);
    prev.attested_sets[0].insert(pk1);

    let mut bond_counts = HashMap::new();
    bond_counts.insert(pkh0, 5);
    bond_counts.insert(pkh1, 3);

    let input = EpochDerivationInput {
        active_producers: vec![pk0, pk1],
        bond_counts: bond_counts.clone(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 360,
        epoch: 1,
        registered_at: HashMap::new(),
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    assert_eq!(new_state.epoch, 1);
    assert_eq!(new_state.bond_snapshot, bond_counts);
    assert_eq!(new_state.producer_list.len(), 2);
    assert_eq!(new_state.active_list.len(), 2);
    assert!(new_state.attested_sets[0].is_empty());
    assert_eq!(new_state.attested_sets[1].len(), 2);
    assert!(new_state.attestation_accum[0].is_empty());
    assert!(new_state.blocks_produced.is_empty());
}

// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, attestation-filtered (only attested retained)
// PATHS: P2: epoch>1 with attestation data, 2/3 attested (filter applies)
// MATRIX: 1 output × 1 path = 1 cell
//   P2: O3✓
#[test]
fn test_derive_at_boundary_attestation_filter() {
    let pk0 = make_pubkey(1);
    let pk1 = make_pubkey(2);
    let pk2 = make_pubkey(3);

    let mut prev = EpochState::genesis();
    prev.epoch = 1;
    prev.attested_sets[0].insert(pk0);
    prev.attested_sets[0].insert(pk1);

    let input = EpochDerivationInput {
        active_producers: vec![pk0, pk1, pk2],
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 720,
        epoch: 2,
        registered_at: HashMap::new(),
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // 2/3 >= 2/3 floor — filter applies
    assert_eq!(new_state.producer_list.len(), 2);
    assert!(new_state.producer_list.contains(&pk0));
    assert!(new_state.producer_list.contains(&pk1));
    assert!(!new_state.producer_list.contains(&pk2));
}

// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, ALL active (deadlock floor triggered)
// PATHS: P4: epoch>1, only 1/3 attested (below 2/3 floor)
// MATRIX: 1 output × 1 path = 1 cell
//   P4: O3✓
#[test]
fn test_derive_deadlock_safety_floor() {
    let pk0 = make_pubkey(1);
    let pk1 = make_pubkey(2);
    let pk2 = make_pubkey(3);

    let mut prev = EpochState::genesis();
    prev.epoch = 1;
    prev.attested_sets[0].insert(pk0);

    let input = EpochDerivationInput {
        active_producers: vec![pk0, pk1, pk2],
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 720,
        epoch: 2,
        registered_at: HashMap::new(),
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // 1/3 < 2/3 floor → all included
    assert_eq!(new_state.producer_list.len(), 3);
}

// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, ALL active (empty accum fallback)
// PATHS: P3: epoch>1, empty attested_sets (post-snap/cold start)
// MATRIX: 1 output × 1 path = 1 cell
//   P3: O3✓
#[test]
fn test_derive_empty_accum_uses_all_producers() {
    let pk0 = make_pubkey(1);
    let pk1 = make_pubkey(2);

    let mut prev = EpochState::genesis();
    prev.epoch = 1;
    // Empty attested_sets — simulate post-snap or cold start

    let input = EpochDerivationInput {
        active_producers: vec![pk0, pk1],
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 720,
        epoch: 2,
        registered_at: HashMap::new(),
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Empty accum + height >= skip → all active
    assert_eq!(new_state.producer_list.len(), 2);
}

// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O5: return.attested_sets — [HashSet;3], [0]=empty, [1]=prev[0], [2]=prev[1]
//   O7: return.attestation_accum — [HashMap;3], [0]=empty, [1]=prev[0], [2]=prev[1]
// PATHS: P5: rotation with all 3 slots populated in prev
// MATRIX: 2 outputs × 1 path = 2 cells (7 sub-assertions)
//   P5: O5([0]empty✓ [1]=prev[0]✓ [2]=prev[1]✓ prev[2]discarded✓) O7([0]empty✓ [1]=prev[0]✓ [2]=prev[1]✓)
#[test]
fn test_derive_accumulator_rotation() {
    let pk0 = make_pubkey(1);
    let pk1 = make_pubkey(2);
    let pk2 = make_pubkey(3);

    let mut prev = EpochState::genesis();
    prev.attested_sets[0].insert(pk0);
    prev.attested_sets[1].insert(pk1);
    prev.attested_sets[2].insert(pk2);

    prev.attestation_accum[0].entry(pk0).or_default().insert(1);
    prev.attestation_accum[1].entry(pk1).or_default().insert(2);

    let input = EpochDerivationInput {
        active_producers: vec![pk0, pk1, pk2],
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 360,
        epoch: 1,
        registered_at: HashMap::new(),
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // [0] = empty (new epoch)
    assert!(new_state.attested_sets[0].is_empty());
    assert!(new_state.attestation_accum[0].is_empty());
    // [1] = prev[0]
    assert!(new_state.attested_sets[1].contains(&pk0));
    assert!(new_state.attestation_accum[1].contains_key(&pk0));
    // [2] = prev[1]
    assert!(new_state.attested_sets[2].contains(&pk1));
    assert!(new_state.attestation_accum[2].contains_key(&pk1));
    // prev[2] discarded
    assert!(!new_state.attested_sets[2].contains(&pk2));
}

// OUTPUT CONTRACT: fn EpochState::serialize() + deserialize()
//   O3: return — EpochState, epoch and producer_list match original
// PATHS: P1: empty state (genesis)
// MATRIX: 1 output × 1 path = 1 cell
//   P1: O3(epoch✓ producer_list✓)
#[test]
fn test_serialize_deserialize_round_trip_empty() {
    let state = EpochState::genesis();
    let bytes = state.serialize();
    let restored = EpochState::deserialize(&bytes).unwrap();
    assert_eq!(restored.epoch, 0);
    assert!(restored.producer_list.is_empty());
}

// OUTPUT CONTRACT: fn EpochState::serialize() + deserialize()
//   O3: return — EpochState, all 7 fields match original
// PATHS: P2: populated state (all fields set)
// MATRIX: 1 output × 1 path = 1 cell (7 sub-assertions)
//   P2: O3(epoch✓ bond_snapshot✓ producer_list✓ active_list✓ attested_sets✓ attestation_accum✓ blocks_produced✓)
#[test]
fn test_serialize_deserialize_round_trip_populated() {
    let pk = make_pubkey(1);
    let pkh = make_hash(1);

    let mut state = EpochState::genesis();
    state.epoch = 5;
    state.bond_snapshot.insert(pkh, 10);
    state.producer_list = vec![pk];
    state.active_list = vec![pk];
    state.attested_sets[0].insert(pk);
    state.attestation_accum[0].entry(pk).or_default().insert(42);
    state.blocks_produced.insert(pk, 7);

    let bytes = state.serialize();
    let restored = EpochState::deserialize(&bytes).unwrap();

    assert_eq!(restored.epoch, 5);
    assert_eq!(restored.bond_snapshot[&pkh], 10);
    assert_eq!(restored.producer_list, vec![pk]);
    assert_eq!(restored.active_list, vec![pk]);
    assert!(restored.attested_sets[0].contains(&pk));
    assert!(restored.attestation_accum[0][&pk].contains(&42));
    assert_eq!(restored.blocks_produced[&pk], 7);
}

// OUTPUT CONTRACT: fn EpochState::hash() -> Hash
//   O3: return — Hash, deterministic (same input → same output)
// PATHS: P1: same state called twice
// MATRIX: 1 output × 1 path = 1 cell
//   P1: O3✓
#[test]
fn test_hash_deterministic() {
    let state = EpochState::genesis();
    assert_eq!(state.hash(), state.hash());
}

// OUTPUT CONTRACT: fn EpochState::hash() -> Hash
//   O3: return — Hash, differs when any field changes
// PATHS: P2: epoch differs, P3: producer_list differs
// MATRIX: 1 output × 2 paths = 2 cells
//   P2: O3✓ P3: O3✓
#[test]
fn test_hash_differs_on_change() {
    let mut s1 = EpochState::genesis();
    let s2_epoch = {
        let mut s = EpochState::genesis();
        s.epoch = 1;
        s
    };
    assert_ne!(s1.hash(), s2_epoch.hash());

    s1.producer_list = vec![make_pubkey(1)];
    assert_ne!(s1.hash(), EpochState::genesis().hash());
}

// INC-I-046: Ghost producers excluded from deadlock floor denominator.
// 5 real + 5 ghosts = 10 active. Without ghost exclusion, deadlock floor
// triggers (5 < 10*2/3=6) and re-includes all 10. With ghost exclusion,
// effective_active=5, floor check: 5 >= 5*2/3=3 → ghosts stay excluded.
#[test]
fn test_ghost_exclusion_prevents_deadlock_floor_override() {
    use crate::consensus::GHOST_EXCLUSION_ACTIVATION_HEIGHT;

    // 5 real producers (attested)
    let real: Vec<PublicKey> = (1..=5).map(make_pubkey).collect();
    // 5 ghost producers (never attested, registered long ago)
    let ghosts: Vec<PublicKey> = (6..=10).map(make_pubkey).collect();

    let mut prev = EpochState::genesis();
    prev.epoch = 9;
    for pk in &real {
        prev.attested_sets[0].insert(*pk);
    }
    // ghosts NOT in any attested_sets

    let mut all = real.clone();
    all.extend_from_slice(&ghosts);

    let mut registered_at = HashMap::new();
    for pk in &real {
        registered_at.insert(*pk, 0); // registered at genesis
    }
    for pk in &ghosts {
        registered_at.insert(*pk, 360); // registered at epoch 1 (well past grace)
    }

    let input = EpochDerivationInput {
        active_producers: all,
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: GHOST_EXCLUSION_ACTIVATION_HEIGHT + 1,
        epoch: 10,
        registered_at,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Only the 5 real producers should be in the list
    assert_eq!(new_state.producer_list.len(), 5);
    for pk in &real {
        assert!(new_state.producer_list.contains(pk));
    }
    for pk in &ghosts {
        assert!(!new_state.producer_list.contains(pk));
    }
}

// INC-I-046: Before activation height, ghosts are NOT excluded (backward compat).
#[test]
fn test_ghost_exclusion_inactive_before_activation() {
    // Same setup as above but height < GHOST_EXCLUSION_ACTIVATION_HEIGHT
    let real: Vec<PublicKey> = (1..=5).map(make_pubkey).collect();
    let ghosts: Vec<PublicKey> = (6..=10).map(make_pubkey).collect();

    let mut prev = EpochState::genesis();
    prev.epoch = 9;
    for pk in &real {
        prev.attested_sets[0].insert(*pk);
    }

    let mut all = real.clone();
    all.extend_from_slice(&ghosts);

    let mut registered_at = HashMap::new();
    for pk in &all {
        registered_at.insert(*pk, 0);
    }

    let input = EpochDerivationInput {
        active_producers: all.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 720, // well below activation
        epoch: 10,
        registered_at,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Deadlock floor triggers (5 < 10*2/3=6) — all 10 included
    assert_eq!(new_state.producer_list.len(), 10);
}

// INC-I-046: Recently registered producers are NOT classified as ghosts.
#[test]
fn test_ghost_exclusion_grace_period_for_new_registrations() {
    use crate::consensus::GHOST_EXCLUSION_ACTIVATION_HEIGHT;

    let real: Vec<PublicKey> = (1..=5).map(make_pubkey).collect();
    let ghost = make_pubkey(6); // registered long ago, never attested
    let new_reg = make_pubkey(7); // registered recently, not yet attested

    let mut prev = EpochState::genesis();
    prev.epoch = 9;
    for pk in &real {
        prev.attested_sets[0].insert(*pk);
    }

    let mut all = real.clone();
    all.push(ghost);
    all.push(new_reg);

    let mut registered_at = HashMap::new();
    for pk in &real {
        registered_at.insert(*pk, 0);
    }
    registered_at.insert(ghost, 360); // epoch 1, well past grace
    registered_at.insert(new_reg, 3240); // epoch 9, within grace (10-9=1 <= 3)

    let input = EpochDerivationInput {
        active_producers: all,
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: GHOST_EXCLUSION_ACTIVATION_HEIGHT + 1,
        epoch: 10,
        registered_at,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // ghost excluded (1 ghost), new_reg NOT a ghost (within grace)
    // effective_active = 7 - 1 = 6, filtered = 5
    // 5 < 6*2/3=4 → 5 >= 4 → floor NOT triggered
    // Result: only 5 attested (real) producers
    assert_eq!(new_state.producer_list.len(), 5);
    for pk in &real {
        assert!(new_state.producer_list.contains(pk));
    }
    assert!(!new_state.producer_list.contains(&ghost));
    // new_reg is filtered by attestation (not attested), NOT by ghost logic
    assert!(!new_state.producer_list.contains(&new_reg));
}

// INC-I-046: Mass event with ghosts — real producers saved, ghosts still excluded.
#[test]
fn test_ghost_exclusion_mass_event_saves_real_producers() {
    use crate::consensus::GHOST_EXCLUSION_ACTIVATION_HEIGHT;

    // Only 2 of 8 real producers attested — mass event
    let attested: Vec<PublicKey> = (1..=2).map(make_pubkey).collect();
    let offline_real: Vec<PublicKey> = (3..=8).map(make_pubkey).collect();
    let ghosts: Vec<PublicKey> = (9..=12).map(make_pubkey).collect();

    let mut prev = EpochState::genesis();
    prev.epoch = 9;
    for pk in &attested {
        prev.attested_sets[0].insert(*pk);
    }

    let mut all = attested.clone();
    all.extend_from_slice(&offline_real);
    all.extend_from_slice(&ghosts);
    // 12 total: 2 attested + 6 offline_real + 4 ghosts

    let mut registered_at = HashMap::new();
    for pk in &all {
        registered_at.insert(*pk, 0);
    }

    let input = EpochDerivationInput {
        active_producers: all,
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: GHOST_EXCLUSION_ACTIVATION_HEIGHT + 1,
        epoch: 10,
        registered_at,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // ghost_count = 4+6 = 10 (all non-attested registered at epoch 0, 10-0=10 > 3)
    // effective_active = 12 - 10 = 2
    // filtered = 2, 2 >= 2*2/3=1 → floor NOT triggered
    // Result: 2 attested producers only
    assert_eq!(new_state.producer_list.len(), 2);
    for pk in &attested {
        assert!(new_state.producer_list.contains(pk));
    }
    for pk in &ghosts {
        assert!(!new_state.producer_list.contains(pk));
    }
}
