//! INC-I-193 M1 / TEST-193-05 — derive/rebuild parity for the attestor-refill gate.
//!
//! covers: `crates/core/src/epoch_state/mod.rs:242-246` (C5, the canonical retain)
//!         `bins/node/src/node/rewards.rs:1062-1083` (C7, the inline rebuild twin)
//!
//! These tests FAIL TO COMPILE until `EpochDerivationInput` gains
//! `inc_i_193_attestor_refill_activation_height` (C4). That is the intended TDD red.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS DEFENDS (FM-1, INV-193-01, behavioural learning #416)
//! ---------------------------------------------------------------------------
//! The tier retain exists TWICE. `derive_at_boundary` runs it at every live epoch
//! boundary; `rebuild_epoch_state_from_blocks` runs a hand-copied twin after a
//! rollback, a reorg or a restart. If only one of the two gets the gate — or if the
//! twin keys it on `target_height`/`current_h`/history availability instead of
//! `epoch_boundary_h` — a node that rebuilds schedules a DIFFERENT `active_list`
//! than its peers and forks. There is no shared pure function to test instead: the
//! twin is inline, so parity is the only available oracle.
//!
//! ---------------------------------------------------------------------------
//! REALISATION — why the AH lever is `node.config.network` and not an env var
//! ---------------------------------------------------------------------------
//! `Node` reads the gate through `self.config.network.params()` (C7), i.e.
//! `NetworkParams::load(net)` — a per-network `OnceLock` (`network_params/mod.rs:913`).
//! `DOLI_*` overrides are consumed by `env_loader::load_from_env` on FIRST touch and
//! then frozen for the process. Every test in `tests/it` shares ONE binary and runs in
//! parallel, so an env override here is order-dependent and leaks into unrelated tests
//! (the existing `set_var` precedents all live in single-test top-level binaries).
//! INC-I-178 sidestepped this with a node-local override field
//! (`node.inc_i_178_attestation_bls_activation_height`); INC-I-193's C7 deliberately
//! has no such field, so the only clean lever is the network selector. `Network::Devnet`
//! pins the gate at `u64::MAX`, so every boundary is below it — the legacy arm.
//! `Network::Testnet` pins it at `195_000`, so the first boundary at or above it — the
//! gated arm. Both arms therefore read the SHIPPED pins, never a test literal.
//!
//! No blocks are stored: `rebuild_epoch_state_from_blocks` takes its
//! `has_incomplete_history` path (`rewards.rs:592-602`, `:737-745`), which still runs the
//! tier stage (`rewards.rs:1050-1114` sits in an UNCONDITIONAL block opened at `:680`) and
//! preserves pre-seeded accumulators (`:942`). That is the smallest fixture that still
//! executes the real C7 code path.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT: fn Node::rebuild_epoch_state_from_blocks(&mut self, target_height: u64)
//!   O1: (none) — `target_height` is `u64` by value.
//!   O2: self.epoch_state.active_list — the quantity under test; MUST equal
//!       `EpochState::derive_at_boundary(prev, input).active_list` for the same boundary.
//!   O2: self.epoch_state.producer_list — the retain's denominator (`min_produced`) and the
//!       1/3 fallback both read its length, so parity here is a precondition, not decoration.
//!   O2: self.epoch_state.attestation_accum[0] / blocks_produced — the retain's inputs; the
//!       rebuild must PRESERVE the pre-seeded values, not overwrite them.
//!   O2: self.snap_sync_height — set to `target_height` on the incomplete-history path;
//!       asserted so a future change of that path cannot silently void the fixture.
//!   O3: return — `()`; carries nothing.
//!   O4: state_db (bond snapshot) — out of scope: the gate cannot reach it.
//!   O5/O6: none.
//! PATHS:
//!   P1: `epoch_boundary_h <  AH` — legacy retain on BOTH sites.
//!   P2: `epoch_boundary_h >= AH` — minutes-only retain on BOTH sites.
//! INPUT PARTITIONS:
//!   IP-A: producer attested >= 30 min AND produced >= min_produced — kept on both paths.
//!   IP-B: producer attested >= 30 min AND produced 0 — kept only on P2. The gate.
//!   The registry is 60 (> ACTIVE_PRODUCERS_CAP), 25 in IP-A and 35 in IP-B, so
//!   P1 -> 25 (>= 60/3, the fallback stays untaken) and P2 -> 50 (the cap).
//! MATRIX: 4 asserted outputs x 2 paths x 2 partitions.

use std::collections::{HashMap, HashSet};

use crypto::PublicKey;
use doli_core::consensus::{ACTIVE_PRODUCERS_CAP, MIN_ATTESTATION_MINUTES};
use doli_core::{EpochDerivationInput, EpochState, Network};
use doli_node::node::Node;

use crate::inc_i_178_m0_common::make_node;

/// Above `ACTIVE_PRODUCERS_CAP`, so the tier stage is reached on both sites.
const REGISTRY: usize = 60;
/// IP-A count. `min_produced` is 1 on both networks (blocks_per_epoch / 60 -> 0 -> max(1)),
/// so 25 pass the legacy clause and the other 35 do not.
const WITH_BLOCKS: usize = 25;

/// The first epoch boundary at or above `ah`, in `bpe`-sized epochs.
fn first_boundary_at_or_above(ah: u64, bpe: u64) -> u64 {
    ah.div_ceil(bpe) * bpe
}

/// Active producers at `boundary`, read through the SAME accessor and the SAME gates
/// `rebuild_epoch_state_from_blocks` uses (`rewards.rs:682-703`). Returns the pubkeys and
/// their `registered_at`, so the reference derivation cannot drift from the node's view.
async fn scheduling_view(node: &Node, boundary: u64) -> (Vec<PublicKey>, HashMap<PublicKey, u64>) {
    let params = node.config.network.params();
    let producers = node.producer_set.read().await;
    let infos = producers.active_producers_for_scheduling_at_height(
        boundary,
        params.inc_i_068_weight_filter_activation_height,
        params.security_audit_activation_height,
    );
    let keys: Vec<PublicKey> = infos.iter().map(|p| p.public_key).collect();
    let reg: HashMap<PublicKey, u64> = infos
        .iter()
        .map(|p| (p.public_key, p.registered_at))
        .collect();
    (keys, reg)
}

/// Seed the just-completed epoch: everybody reaches `MIN_ATTESTATION_MINUTES`, only the
/// last `WITH_BLOCKS` by pubkey order produced anything. Picking the TAIL (not a prefix)
/// makes the legacy and minutes-only results differ in membership, not just in length.
fn seed_just_completed_epoch(state: &mut EpochState, universe: &[PublicKey]) {
    let mut ordered = universe.to_vec();
    ordered.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    let producers_of_blocks: HashSet<PublicKey> = ordered[ordered.len() - WITH_BLOCKS..]
        .iter()
        .copied()
        .collect();

    state.attested_sets = [HashSet::new(), HashSet::new(), HashSet::new()];
    state.attestation_accum = [HashMap::new(), HashMap::new(), HashMap::new()];
    state.blocks_produced = HashMap::new();
    for pk in &ordered {
        for slot in 0..3 {
            state.attested_sets[slot].insert(*pk);
        }
        state.attestation_accum[0].insert(
            *pk,
            (0..MIN_ATTESTATION_MINUTES as u32).collect::<HashSet<u32>>(),
        );
        if producers_of_blocks.contains(pk) {
            state.blocks_produced.insert(*pk, 10);
        }
    }
}

/// The canonical derivation (C5), fed the node's own params and producer view.
/// `ah_override` replaces ONLY the INC-I-193 gate — used to build the opposite-arm control
/// that proves a parity assertion is not vacuous.
fn derive_reference(
    node: &Node,
    prev: &EpochState,
    active: &[PublicKey],
    reg: &HashMap<PublicKey, u64>,
    boundary: u64,
    ah_override: Option<u64>,
) -> EpochState {
    let params = node.config.network.params();
    let bpe = node.config.network.blocks_per_reward_epoch();
    let input = EpochDerivationInput {
        active_producers: active.to_vec(),
        bond_counts: HashMap::new(),
        blocks_per_epoch: bpe,
        snap_attestation_skip_height: params.snap_attestation_skip_height,
        height: boundary,
        epoch: boundary / bpe,
        registered_at: reg.clone(),
        ghost_exclusion_activation_height: params.ghost_exclusion_activation_height,
        epoch_prune_activation_height: params.epoch_prune_activation_height,
        inc_i_190_floor_bound_activation_height: params.inc_i_190_floor_bound_activation_height,
        inc_i_193_attestor_refill_activation_height: ah_override
            .unwrap_or(params.inc_i_193_attestor_refill_activation_height),
    };
    EpochState::derive_at_boundary(prev, &input)
}

/// One arm's three lists: what the rebuild twin produced, what the canonical derivation
/// produced, and what the canonical derivation produces on the OPPOSITE arm of the gate.
struct ArmResult {
    rebuilt: Vec<PublicKey>,
    derived: Vec<PublicKey>,
    opposite_arm: Vec<PublicKey>,
}

/// Run one arm end to end.
async fn parity_arm(network: Network, boundary_of: fn(u64, u64) -> u64) -> ArmResult {
    let (mut node, _producers, _tmp) = make_node(REGISTRY).await;
    node.config.network = network;

    let params = node.config.network.params();
    let bpe = node.config.network.blocks_per_reward_epoch();
    let boundary = boundary_of(params.inc_i_193_attestor_refill_activation_height, bpe);
    assert!(
        boundary / bpe > 1,
        "precondition: the tier retain only runs for epoch > 1 (boundary={}, bpe={})",
        boundary,
        bpe
    );

    let (active, reg) = scheduling_view(&node, boundary).await;
    assert_eq!(
        active.len(),
        REGISTRY,
        "precondition: all {} genesis producers must be schedulable at h={}",
        REGISTRY,
        boundary
    );
    assert!(
        active.len() > ACTIVE_PRODUCERS_CAP,
        "precondition: the tier stage only runs above the cap"
    );

    seed_just_completed_epoch(&mut node.epoch_state, &active);
    let prev = node.epoch_state.clone();

    let derived = derive_reference(&node, &prev, &active, &reg, boundary, None);
    // The opposite arm of the gate at the SAME boundary: below the AH -> force minutes-only,
    // at/above it -> force legacy. Any parity assertion is vacuous unless these differ.
    let opposite_ah = if boundary >= params.inc_i_193_attestor_refill_activation_height {
        boundary + 1
    } else {
        boundary
    };
    let opposite_arm =
        derive_reference(&node, &prev, &active, &reg, boundary, Some(opposite_ah)).active_list;

    node.rebuild_epoch_state_from_blocks(boundary).await;

    assert_eq!(
        node.snap_sync_height,
        Some(boundary),
        "O2: the fixture stores no blocks, so the rebuild must take its \
         incomplete-history path — if this moved, the tier stage may no longer be reached"
    );
    assert_eq!(
        node.epoch_state.attestation_accum[0].len(),
        REGISTRY,
        "O2: the rebuild must PRESERVE the pre-seeded minute accumulator (rewards.rs:942)"
    );
    assert_eq!(
        node.epoch_state.blocks_produced.len(),
        WITH_BLOCKS,
        "O2: the rebuild must PRESERVE the pre-seeded production counter"
    );

    let mut rebuilt_list = node.epoch_state.producer_list.clone();
    let mut derived_list = derived.producer_list.clone();
    rebuilt_list.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    derived_list.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(
        rebuilt_list, derived_list,
        "O2: the two sites must feed the retain the SAME producer_list, or an active_list \
         match would prove nothing about the gate"
    );

    ArmResult {
        rebuilt: node.epoch_state.active_list.clone(),
        derived: derived.active_list,
        opposite_arm,
    }
}

// TEST-193-05 — Decision: a failure means the rebuild twin drifted from the canonical
// derivation below the AH, so a node that rolls back re-derives a pre-AH schedule its
// peers never had — the FM-1 fork.
#[tokio::test]
async fn test_193_05_parity_below_the_activation_height() {
    // Devnet pins the gate at u64::MAX, so any boundary is strictly below it. Epoch 3.
    let arm = parity_arm(Network::Devnet, |_ah, bpe| bpe * 3).await;

    assert_eq!(
        arm.rebuilt.len(),
        WITH_BLOCKS,
        "O2/P1/IP-A: below the AH only the {} producers that met BOTH clauses stay listed",
        WITH_BLOCKS
    );
    assert_eq!(
        arm.rebuilt, arm.derived,
        "O2/P1: rebuild and derive must produce a bit-identical active_list (INV-193-01)"
    );
    assert_ne!(
        arm.derived, arm.opposite_arm,
        "non-vacuity: the minutes-only arm must disagree at this boundary, or the parity \
         assertion above holds for inputs the gate cannot separate"
    );
}

// TEST-193-05 — Decision: a failure means C5 landed without C7 (or C7 keyed the gate on a
// height the caller supplies rather than the epoch boundary), so a rebuilding node keeps
// the shrink-only schedule after the fleet has moved on — the FM-1 fork, post-AH.
#[tokio::test]
async fn test_193_05_parity_at_and_above_the_activation_height() {
    // Testnet pins the gate at 195_000; the first boundary at/above it is h = 195_012.
    let arm = parity_arm(Network::Testnet, first_boundary_at_or_above).await;

    assert_eq!(
        arm.rebuilt.len(),
        ACTIVE_PRODUCERS_CAP,
        "O2/P2/IP-B: at/above the AH the zero-block attesters are admitted and the list \
         refills to the cap — a length of {} means the rebuild twin is still on the legacy arm",
        WITH_BLOCKS
    );
    assert_eq!(
        arm.rebuilt, arm.derived,
        "O2/P2: rebuild and derive must produce a bit-identical active_list (INV-193-01)"
    );
    assert_ne!(
        arm.derived, arm.opposite_arm,
        "non-vacuity: the legacy arm must disagree at this boundary, or the parity assertion \
         above holds for inputs the gate cannot separate"
    );
}
