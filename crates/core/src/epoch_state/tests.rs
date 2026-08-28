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
// INPUT PARTITIONS:
//   P1a: no inputs — constructor is deterministic, single partition
// MATRIX: 7 outputs × 1 partition = 7 cells
//   P1a: O1✓ O2✓ O3✓ O4✓ O5✓ O6✓ O7✓
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
// INPUT PARTITIONS:
//   P1a: single producer, no attestation — only producer tracked
// MATRIX: 3 outputs × 1 partition = 3 cells
//   P1a: O1✓ O2✓ O3✓
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
// INPUT PARTITIONS:
//   P2a: 2 producers, attestation indices [0,1] — both tracked in attested/accum, only producer in blocks_produced
// MATRIX: 3 outputs × 1 partition = 3 cells
//   P2a: O1✓ O2✓ O3✓
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
// INPUT PARTITIONS:
//   P3a: 5 consecutive blocks from same producer — counter increments to 5
// MATRIX: 1 output × 1 partition = 1 cell
//   P3a: O2✓
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
// INPUT PARTITIONS:
//   P1a: epoch=1, 2 active producers — epoch<=1 bypass, all included
// MATRIX: 8 outputs × 1 partition = 8 cells
//   P1a: O1✓ O2✓ O3✓ O4✓ O5✓ O6✓ O7✓ O8✓
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
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
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
// INPUT PARTITIONS:
//   P2a: 3 active, 2 attested — 2/3 >= 2/3 floor, filter applies, 2 retained
// MATRIX: 1 output × 1 partition = 1 cell
//   P2a: O3✓
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
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
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
// INPUT PARTITIONS:
//   P4a: 3 active, 1 attested — 1/3 < 2/3 floor, all included
// MATRIX: 1 output × 1 partition = 1 cell
//   P4a: O3✓
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
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // 1/3 < 2/3 floor -> all included
    assert_eq!(new_state.producer_list.len(), 3);
}

// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, ALL active (empty accum fallback)
// PATHS: P3: epoch>1, empty attested_sets (post-snap/cold start)
// INPUT PARTITIONS:
//   P3a: 2 active, 0 attested — empty accum, all included
// MATRIX: 1 output × 1 partition = 1 cell
//   P3a: O3✓
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
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Empty accum + height >= skip -> all active
    assert_eq!(new_state.producer_list.len(), 2);
}

// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O5: return.attested_sets — [HashSet;3], [0]=empty, [1]=prev[0], [2]=prev[1]
//   O7: return.attestation_accum — [HashMap;3], [0]=empty, [1]=prev[0], [2]=prev[1]
// PATHS: P5: rotation with all 3 slots populated in prev
// INPUT PARTITIONS:
//   P5a: 3 producers across 3 prev slots — rotation shifts [0]->[1]->[2], prev[2] discarded
// MATRIX: 2 outputs × 1 partition = 2 cells (7 sub-assertions)
//   P5a: O5([0]empty✓ [1]=prev[0]✓ [2]=prev[1]✓ prev[2]discarded✓) O7([0]empty✓ [1]=prev[0]✓ [2]=prev[1]✓)
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
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
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
// INPUT PARTITIONS:
//   P1a: genesis (all-empty) state — round-trip identity
// MATRIX: 1 output × 1 partition = 1 cell
//   P1a: O3(epoch✓ producer_list✓)
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
// INPUT PARTITIONS:
//   P2a: populated state with 1 producer — all 7 fields round-trip
// MATRIX: 1 output × 1 partition = 1 cell (7 sub-assertions)
//   P2a: O3(epoch✓ bond_snapshot✓ producer_list✓ active_list✓ attested_sets✓ attestation_accum✓ blocks_produced✓)
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
//   O3: return — Hash, deterministic (same input -> same output)
// PATHS: P1: same state called twice
// INPUT PARTITIONS:
//   P1a: genesis state — identity under repeated hashing
// MATRIX: 1 output × 1 partition = 1 cell
//   P1a: O3✓
#[test]
fn test_hash_deterministic() {
    let state = EpochState::genesis();
    assert_eq!(state.hash(), state.hash());
}

// OUTPUT CONTRACT: fn EpochState::hash() -> Hash
//   O3: return — Hash, differs when any field changes
// PATHS: P2: epoch differs, P3: producer_list differs
// INPUT PARTITIONS:
//   P2a: epoch 0 vs epoch 1 — hash differs
//   P3a: empty producer_list vs [pk1] — hash differs
// MATRIX: 1 output × 2 partitions = 2 cells
//   P2a: O3✓ P3a: O3✓
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
// effective_active=5, floor check: 5 >= 5*2/3=3 -> ghosts stay excluded.
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, only real (non-ghost) producers
// PATHS: P6: ghost_exclusion active, ghosts excluded from floor denominator
// INPUT PARTITIONS:
//   P6a: 5 real attested + 5 ghosts (registered epoch 1, now epoch 10) — ghosts excluded, floor passes
// MATRIX: 1 output × 1 partition = 1 cell
//   P6a: O3(len=5✓ contains_real✓ excludes_ghosts✓)
#[test]
fn test_ghost_exclusion_prevents_deadlock_floor_override() {
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
        height: 10_680,
        epoch: 10,
        registered_at,
        ghost_exclusion_activation_height: 0,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
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
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, ALL active (ghost exclusion OFF, floor fires)
// PATHS: P7: ghost_exclusion_activation_height=u64::MAX (OFF), floor triggers
// INPUT PARTITIONS:
//   P7a: 5 attested + 5 non-attested, ghost exclusion OFF — floor 5 < 10*2/3=6 fires, all 10 included
// MATRIX: 1 output × 1 partition = 1 cell
//   P7a: O3(len=10✓)
#[test]
fn test_ghost_exclusion_inactive_before_activation() {
    // Same setup as above but ghost_exclusion_activation_height = u64::MAX (OFF)
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
        ghost_exclusion_activation_height: u64::MAX, // ghost exclusion OFF
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Deadlock floor triggers (5 < 10*2/3=6) — all 10 included
    assert_eq!(new_state.producer_list.len(), 10);
}

// INC-I-046: Recently registered producers are NOT classified as ghosts.
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, real attested only (ghost excluded, new_reg filtered by attestation not ghost)
// PATHS: P8: ghost exclusion active, mix of attested/ghost/new-registration
// INPUT PARTITIONS:
//   P8a: 5 attested + 1 ghost (epoch 1) + 1 new_reg (epoch 9, within grace) — ghost excluded, new_reg not ghost but filtered by attestation
// MATRIX: 1 output × 1 partition = 1 cell
//   P8a: O3(len=5✓ contains_real✓ excludes_ghost✓ excludes_new_reg✓)
#[test]
fn test_ghost_exclusion_grace_period_for_new_registrations() {
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
        height: 10_680,
        epoch: 10,
        registered_at,
        ghost_exclusion_activation_height: 0,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // ghost excluded (1 ghost), new_reg NOT a ghost (within grace)
    // effective_active = 7 - 1 = 6, filtered = 5
    // 5 < 6*2/3=4 -> 5 >= 4 -> floor NOT triggered
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
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, only attested producers (mass event, ghosts excluded)
// PATHS: P9: ghost exclusion active, mass event (2/12 attested), ghosts + offline_real both non-attested
// INPUT PARTITIONS:
//   P9a: 2 attested + 6 offline_real + 4 ghosts — all non-attested registered at epoch 0 are ghosts, effective_active=2, 2 >= 2*2/3=1, only 2 retained
// MATRIX: 1 output × 1 partition = 1 cell
//   P9a: O3(len=2✓ contains_attested✓ excludes_ghosts✓)
#[test]
fn test_ghost_exclusion_mass_event_saves_real_producers() {
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
        height: 10_680,
        epoch: 10,
        registered_at,
        ghost_exclusion_activation_height: 0,
        epoch_prune_activation_height: u64::MAX,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // ghost_count = 4+6 = 10 (all non-attested registered at epoch 0, 10-0=10 > 3)
    // effective_active = 12 - 10 = 2
    // filtered = 2, 2 >= 2*2/3=1 -> floor NOT triggered
    // Result: 2 attested producers only
    assert_eq!(new_state.producer_list.len(), 2);
    for pk in &attested {
        assert!(new_state.producer_list.contains(pk));
    }
    for pk in &ghosts {
        assert!(!new_state.producer_list.contains(pk));
    }
}

// ============================================================================
// INC-I-116 M1: Epoch-boundary liveness prune tests
// ============================================================================

// Requirement: REQ-PRUNE-002 (Must) — FILTER-08 pre-activation bit-identity
// Acceptance: Pre-activation height: derive_at_boundary() produces identical
//   output to current implementation
//
// REGRESSION TEST: This MUST PASS with current code (pre-M1) AND after M1.
// It locks the current proportional-floor behavior when epoch_prune_activation_height
// is u64::MAX (disabled).
//
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, ALL 57 active (proportional floor fires)
// PATHS: P10: epoch>1, epoch_prune_activation_height=u64::MAX, 12/57 attested
// INPUT PARTITIONS:
//   P10a: 57 active, 12 attested, prune disabled — proportional floor 12 < 57*2/3=38 fires, all 57 included
// MATRIX: 1 output × 1 partition = 1 cell
//   P10a: O3(len=57✓ contains_all_active✓)
#[test]
fn test_pre_activation_floor_is_identical_to_current_behavior() {
    // 57 active producers, only 12 attested — reproduces the INC-I-116 scenario
    let all_producers: Vec<PublicKey> = (1..=57).map(make_pubkey).collect();
    let attested_set: Vec<PublicKey> = (1..=12).map(make_pubkey).collect();

    let mut prev = EpochState::genesis();
    prev.epoch = 4;
    for pk in &attested_set {
        prev.attested_sets[0].insert(*pk);
    }

    let mut registered_at = HashMap::new();
    for pk in &all_producers {
        registered_at.insert(*pk, 0); // all registered at genesis
    }

    let input = EpochDerivationInput {
        active_producers: all_producers.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 500,
        epoch: 5,
        registered_at,
        ghost_exclusion_activation_height: u64::MAX, // ghost exclusion OFF for this test
        epoch_prune_activation_height: u64::MAX,     // prune DISABLED (pre-activation)
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Pre-activation: proportional floor fires (12 < 57*2/3 = 38) and
    // re-includes ALL 57 producers. This IS the current (buggy) behavior
    // we are locking as a regression baseline.
    assert_eq!(
        new_state.producer_list.len(),
        57,
        "Pre-activation must reproduce the proportional floor override: \
         all 57 producers included despite only 12 attesting"
    );
    for pk in &all_producers {
        assert!(
            new_state.producer_list.contains(pk),
            "Pre-activation: every active producer must be in the list"
        );
    }
}

// Requirement: REQ-PRUNE-001 (Must), REQ-PRUNE-002 (Must)
// Acceptance: Given producer B (0 blocks, 0 attestations in epoch N),
//   when epoch N+1 boundary is reached, then B is NOT in producer_list
//
// NEW TEST: WILL FAIL until developer implements the post-activation prune.
//
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, ONLY the 12 attested producers (pruned)
// PATHS: P11: epoch>1, epoch_prune_activation_height=0, 12/57 attested
// INPUT PARTITIONS:
//   P11a: 57 active, 12 attested, prune active — absolute floor 12 >= MIN_PRODUCERS_FLOOR(3), only 12 retained
// MATRIX: 1 output × 1 partition = 1 cell
//   P11a: O3(len=12✓ contains_only_attested✓ excludes_absent✓)
#[test]
fn test_post_activation_prunes_absent_producers() {
    // Same setup as the regression test but with prune ACTIVE
    let all_producers: Vec<PublicKey> = (1..=57).map(make_pubkey).collect();
    let attested_set: Vec<PublicKey> = (1..=12).map(make_pubkey).collect();
    let absent_set: Vec<PublicKey> = (13..=57).map(make_pubkey).collect();

    let mut prev = EpochState::genesis();
    prev.epoch = 4;
    for pk in &attested_set {
        prev.attested_sets[0].insert(*pk);
    }

    let mut registered_at = HashMap::new();
    for pk in &all_producers {
        registered_at.insert(*pk, 0);
    }

    let input = EpochDerivationInput {
        active_producers: all_producers,
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 500,
        epoch: 5,
        registered_at,
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: 0, // prune ACTIVE from genesis
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Post-activation: the absolute floor (MIN_PRODUCERS_FLOOR=3) replaces the
    // proportional floor. 12 attested >= 3, so only the 12 attested producers
    // remain in the schedule. The 45 absent producers are pruned.
    assert_eq!(
        new_state.producer_list.len(),
        12,
        "Post-activation must prune absent producers: only 12 attested should remain"
    );
    for pk in &attested_set {
        assert!(
            new_state.producer_list.contains(pk),
            "Attested producer must remain in the list"
        );
    }
    for pk in &absent_set {
        assert!(
            !new_state.producer_list.contains(pk),
            "Absent producer must be pruned from the list"
        );
    }
}

// Requirement: REQ-PRUNE-005 (Must)
// Acceptance: If pruning would leave fewer than MIN_PRODUCERS_FLOOR (3)
//   producers, prune is capped to retain at least that many
//
// NEW TEST: WILL FAIL until developer implements the absolute floor fallback.
//
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, at least MIN_PRODUCERS_FLOOR(3) producers
// PATHS: P12: epoch>1, epoch_prune_activation_height=0, 2/57 attested (below floor)
// INPUT PARTITIONS:
//   P12a: 57 active, 2 attested, prune active — 2 < MIN_PRODUCERS_FLOOR(3), fallback fires
// MATRIX: 1 output × 1 partition = 1 cell
//   P12a: O3(len>=3✓ contains_attested✓)
#[test]
fn test_post_activation_absolute_floor_fires() {
    let all_producers: Vec<PublicKey> = (1..=57).map(make_pubkey).collect();
    // Only 2 attested — below MIN_PRODUCERS_FLOOR of 3
    let attested_set: Vec<PublicKey> = (1..=2).map(make_pubkey).collect();

    let mut prev = EpochState::genesis();
    prev.epoch = 4;
    for pk in &attested_set {
        prev.attested_sets[0].insert(*pk);
    }

    let mut registered_at = HashMap::new();
    for pk in &all_producers {
        registered_at.insert(*pk, 0);
    }

    let input = EpochDerivationInput {
        active_producers: all_producers.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 500,
        epoch: 5,
        registered_at,
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: 0, // prune ACTIVE
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Post-activation: only 2 attested, which is below MIN_PRODUCERS_FLOOR=3.
    // The absolute floor fallback fires — producer_list must have >= 3 producers.
    // The fallback should include all non-ghost active producers (same as current
    // fallback behavior).
    assert!(
        new_state.producer_list.len() >= 3,
        "Absolute floor must ensure at least MIN_PRODUCERS_FLOOR=3 producers; got {}",
        new_state.producer_list.len()
    );
    // The 2 attested producers must still be in the list
    for pk in &attested_set {
        assert!(
            new_state.producer_list.contains(pk),
            "Attested producers must be in the fallback list"
        );
    }
}

// Requirement: REQ-PRUNE-003 (Must)
// Acceptance: Given producer P was pruned at epoch N boundary, and P attests
//   during epoch N, then P is included in producer_list at epoch N+1 boundary.
//   Re-inclusion does not require any on-chain transaction from P.
//
// NEW TEST: WILL FAIL until developer implements the post-activation prune.
//
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec
//     epoch N+1: pruned producer absent
//     epoch N+2: pruned producer re-included after attesting
// PATHS: P13: two sequential derive_at_boundary calls, producer absent then present
// INPUT PARTITIONS:
//   P13a: epoch N+1 (producer absent) -> epoch N+2 (producer re-attests) — prune then re-include
// MATRIX: 1 output × 2 sequential states = 2 cells
//   P13a-epoch-N+1: O3(excludes_pruned✓)
//   P13a-epoch-N+2: O3(includes_re-attested✓)
#[test]
fn test_pruned_producer_reappears_on_attestation() {
    // Setup: 10 active producers
    let all_producers: Vec<PublicKey> = (1..=10).map(make_pubkey).collect();
    let always_attested: Vec<PublicKey> = (1..=8).map(make_pubkey).collect();
    let pruned_producer = make_pubkey(9);
    let other_absent = make_pubkey(10);

    // --- Epoch N boundary: pruned_producer and other_absent are NOT attested ---
    let mut prev_n = EpochState::genesis();
    prev_n.epoch = 4;
    for pk in &always_attested {
        prev_n.attested_sets[0].insert(*pk);
    }
    // pruned_producer and other_absent NOT in attested_sets

    let mut registered_at = HashMap::new();
    for pk in &all_producers {
        registered_at.insert(*pk, 0);
    }

    let input_n1 = EpochDerivationInput {
        active_producers: all_producers.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 1800,
        epoch: 5,
        registered_at: registered_at.clone(),
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: 0, // prune ACTIVE
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let state_n1 = EpochState::derive_at_boundary(&prev_n, &input_n1);

    // At epoch N+1: pruned_producer should be absent (only 8 attested >= floor 3)
    assert_eq!(state_n1.producer_list.len(), 8);
    assert!(
        !state_n1.producer_list.contains(&pruned_producer),
        "pruned_producer must be absent at epoch N+1"
    );
    assert!(
        !state_n1.producer_list.contains(&other_absent),
        "other_absent must also be absent at epoch N+1"
    );

    // --- Epoch N+1: pruned_producer re-attests during this epoch ---
    let mut prev_n1 = state_n1;
    // Simulate: pruned_producer attested during epoch N+1
    prev_n1.attested_sets[0].insert(pruned_producer);
    // The 8 always_attested continue attesting
    for pk in &always_attested {
        prev_n1.attested_sets[0].insert(*pk);
    }
    // other_absent still does NOT attest

    let input_n2 = EpochDerivationInput {
        active_producers: all_producers.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 2160,
        epoch: 6,
        registered_at,
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: 0,
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let state_n2 = EpochState::derive_at_boundary(&prev_n1, &input_n2);

    // At epoch N+2: pruned_producer re-attested, so they should be back
    assert!(
        state_n2.producer_list.contains(&pruned_producer),
        "pruned_producer must be re-included after attesting in epoch N+1"
    );
    // other_absent still absent (but might be in attested_union via lookback
    // from prev_n1.attested_sets[1] which is prev_n.attested_sets[0] — only
    // the 8 always_attested). Check they are NOT in the list.
    assert!(
        !state_n2.producer_list.contains(&other_absent),
        "other_absent must remain pruned (never re-attested)"
    );
    assert_eq!(
        state_n2.producer_list.len(),
        9,
        "8 always_attested + 1 re-attested = 9"
    );
}

// Requirement: REQ-PRUNE-005 (Must)
// Acceptance: Degenerate case — 0 attested producers, fallback must fire
//
// NEW TEST: WILL FAIL until developer implements the post-activation absolute floor.
//
// OUTPUT CONTRACT: fn EpochState::derive_at_boundary(prev, input) -> EpochState
//   O3: return.producer_list — Vec, NOT empty (fallback fires when 0 attested)
// PATHS: P14: epoch>1, epoch_prune_activation_height=0, 0/10 attested
// INPUT PARTITIONS:
//   P14a: 10 active, 0 attested, prune active — 0 < MIN_PRODUCERS_FLOOR(3), fallback fires
// MATRIX: 1 output × 1 partition = 1 cell
//   P14a: O3(not_empty✓ len>=3✓)
#[test]
fn test_post_activation_zero_attested_uses_fallback() {
    let all_producers: Vec<PublicKey> = (1..=10).map(make_pubkey).collect();
    // NO producers attested

    let mut prev = EpochState::genesis();
    prev.epoch = 4;
    // attested_sets all empty — nobody attested

    let mut registered_at = HashMap::new();
    for pk in &all_producers {
        registered_at.insert(*pk, 0);
    }

    let input = EpochDerivationInput {
        active_producers: all_producers.clone(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: 360,
        snap_attestation_skip_height: u64::MAX,
        height: 500,
        epoch: 5,
        registered_at,
        ghost_exclusion_activation_height: u64::MAX,
        epoch_prune_activation_height: 0, // prune ACTIVE
        inc_i_190_floor_bound_activation_height: u64::MAX,
    };

    let new_state = EpochState::derive_at_boundary(&prev, &input);

    // Post-activation: 0 attested is below MIN_PRODUCERS_FLOOR=3.
    // The fallback fires — the list must NOT be empty. The chain must
    // always have producers to schedule.
    assert!(
        !new_state.producer_list.is_empty(),
        "Zero attested must trigger fallback — producer_list cannot be empty"
    );
    assert!(
        new_state.producer_list.len() >= 3,
        "Fallback must include at least MIN_PRODUCERS_FLOOR=3 producers; got {}",
        new_state.producer_list.len()
    );
}
