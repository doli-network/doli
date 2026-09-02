//! INC-I-204 M5 — shared fixtures and the PRE-M5 LEGACY ORACLE.
//!
//! OUTPUT CONTRACT: N/A — fixture module. It asserts nothing; every observable it
//! produces is asserted by `tests_m5_fork_choice.rs` / `tests_m5_finality_authority.rs`.
//! INPUT PARTITIONS: N/A — fixture module.
//!
//! TDD RED, EXPECTED: this module does not compile against the tree at HEAD. It names
//! `ReorgHandler::with_activation_heights`, `ForkChoiceFinality`,
//! `record_fork_block_with_height` and the six-parameter `plan_reorg` /
//! `check_reorg_weighted`. That compile failure IS the red for the M5 suite; the
//! BEHAVIOURAL red is in `tests_m5_red_witness.rs`, which compiles and fails on HEAD.
//!
//! REQUIRED RE-EXPORT: `reorg/mod.rs` must `pub use fork_choice::{ForkChoiceFinality,
//! WeightVerdict};` so that `use super::*;` in the sibling test modules resolves them.
//!
//! WHY A LEGACY ORACLE. "Below the activation height the node behaves bit-identically
//! to today" is only testable against something that encodes TODAY. We cannot run the
//! old binary inside this process, so [`legacy_plan_reorg_admits`] and
//! [`legacy_weight_switch`] transcribe the pre-M5 rules from `mod.rs:336-363`,
//! `mod.rs:556-597` and `wedge_escape.rs:132` as independent implementations. They are
//! the mixed-fleet peer: a divergence between the oracle and the gated code IS trap T9.
//! They are deliberately written from the source lines, not by calling the code under
//! test, or the parity assertion would be a tautology.

use std::collections::HashMap;

use super::*;

/// The incident's finalized height. A REAL chain height, from `check_finality()`.
pub(super) const FINALIZED_H: u64 = 77_777;
/// Real chain height of the common ancestor in the incident cell — the finalized block.
pub(super) const ANCESTOR_REAL_H: u64 = FINALIZED_H;
/// Real chain height of both competing tips, one above the ancestor.
pub(super) const TIP_REAL_H: u64 = ANCESTOR_REAL_H + 1;
/// The incident's measured branch weight, on BOTH sides: `fork_w = 10390 <= our_w = 10390`.
pub(super) const INCIDENT_WEIGHT: u64 = 10_390;

/// `(our_weight, candidate_weight)` cells for the T9 parity table.
///
/// The first row is the incident. `(0, 0)` is included because
/// `should_reorg_by_weight_with_tiebreak` special-cases `new_weight > 0`, so a
/// zero-weight tie is the one cell where the THREE pre-M5 rules already differ from
/// each other — the parity assertions must survive it rather than route around it.
pub(super) const WEIGHT_VECTOR: &[(u64, u64)] = &[
    (INCIDENT_WEIGHT, INCIDENT_WEIGHT),
    (INCIDENT_WEIGHT, INCIDENT_WEIGHT + 1),
    (INCIDENT_WEIGHT, INCIDENT_WEIGHT - 1),
    (1, 1),
    (0, 0),
    (0, 1),
    (1, 0),
    (u64::MAX / 2, u64::MAX / 2),
    (5, 500),
    (500, 5),
];

/// A gate value that can never be reached by any chain height: the DORMANT window that
/// mainnet and testnet sit in for the whole of M5 (design brief S2).
pub(super) const GATE_DORMANT: u64 = u64::MAX;
/// A gate value every chain height clears: devnet's arm, and the post-activation world.
pub(super) const GATE_ACTIVE: u64 = 0;

/// Build `ForkChoiceFinality` without repeating the field names in thirty call sites.
pub(super) fn fin(
    finalized_height: Option<u64>,
    finalized_hash: Option<Hash>,
    local_tip_height: u64,
) -> ForkChoiceFinality {
    ForkChoiceFinality {
        finalized_height,
        finalized_hash,
        local_tip_height,
    }
}

/// Two distinct hashes as `(lower, higher)` in byte order, so a test can place the
/// candidate on either side of the tie-break without depending on which label happens
/// to hash lower.
pub(super) fn ordered_pair(a: &[u8], b: &[u8]) -> (Hash, Hash) {
    let ha = crypto::hash::hash(a);
    let hb = crypto::hash::hash(b);
    if ha.as_bytes() < hb.as_bytes() {
        (ha, hb)
    } else {
        (hb, ha)
    }
}

/// A block on a competing branch. Only `prev_hash` and the resulting hash matter.
pub(super) fn fork_block(prev_hash: Hash, slot: u32) -> Block {
    let header = doli_core::BlockHeader {
        version: 1,
        prev_hash,
        merkle_root: Hash::ZERO,
        presence_root: Hash::ZERO,
        genesis_hash: Hash::ZERO,
        timestamp: 0,
        slot,
        producer: crypto::PublicKey::from_bytes([0u8; 32]),
        vdf_output: vdf::VdfOutput { value: vec![] },
        vdf_proof: vdf::VdfProof::empty(),
        missed_producers: Vec::new(),
        data_root: Hash::ZERO,
        fork_id: Hash::ZERO,
    };
    Block::new(header, vec![])
}

/// A fork block whose HASH sits on a chosen side of `target`, found by varying the slot.
///
/// The equal-weight tie-break compares block hashes, so a test of the tie needs to
/// choose which side of it the candidate falls on. Roughly two iterations expected.
pub(super) fn fork_block_ordered(prev_hash: Hash, target: Hash, want_below: bool) -> Block {
    for slot in 0u32..100_000 {
        let b = fork_block(prev_hash, slot);
        if (b.hash().as_bytes() < target.as_bytes()) == want_below {
            return b;
        }
    }
    panic!("no fork block hash on the requested side of {target} in 100k slots");
}

/// Two competing tips off one shared ancestor, both at real height [`TIP_REAL_H`].
pub(super) struct TwoBranch {
    pub handler: ReorgHandler,
    pub ancestor: Hash,
    pub our_tip: Hash,
    pub cand_tip: Hash,
    pub heights: HashMap<Hash, u64>,
    pub parents: HashMap<Hash, Hash>,
}

/// Build [`TwoBranch`] with exact branch weights and a chosen hash ordering.
///
/// `inc_i_147` is pinned to `0`, matching `ReorgHandler::new()` on HEAD, so the ONLY
/// gate under test is the M5 one.
pub(super) fn two_branch(
    our_w: u64,
    cand_w: u64,
    cand_hash_lower: bool,
    m5_gate: u64,
) -> TwoBranch {
    let (lower, higher) = ordered_pair(b"m5_branch_alpha", b"m5_branch_beta");
    let (cand_tip, our_tip) = if cand_hash_lower {
        (lower, higher)
    } else {
        (higher, lower)
    };
    let ancestor = crypto::hash::hash(b"m5_two_branch_fork_point");

    let mut handler = ReorgHandler::with_activation_heights(0, m5_gate);
    handler.record_block_with_height(ancestor, Hash::ZERO, 0, ANCESTOR_REAL_H);
    handler.record_block_with_height(our_tip, ancestor, our_w, TIP_REAL_H);
    handler.record_fork_block_with_height(cand_tip, ancestor, cand_w, TIP_REAL_H);

    let heights = HashMap::from([
        (ancestor, ANCESTOR_REAL_H),
        (our_tip, TIP_REAL_H),
        (cand_tip, TIP_REAL_H),
    ]);
    let parents = HashMap::from([(our_tip, ancestor), (cand_tip, ancestor)]);

    TwoBranch {
        handler,
        ancestor,
        our_tip,
        cand_tip,
        heights,
        parents,
    }
}

/// How the common ancestor is present in `ReorgHandler.block_weights` — the axis that
/// separates a chain-derived height from a per-process one.
#[derive(Clone, Copy, Debug)]
pub(super) enum AncestorRecord {
    /// Recorded through `record_block_with_height`: stored height == real height.
    Real,
    /// Recorded through the FORK path, so the stored height is the per-process counter.
    /// The value is REQUESTED by seeding the trunk at `value - 1`.
    Synthetic(u64),
    /// Absent from `block_weights` entirely — LRU-evicted, or pruned by a rollback
    /// (INC-I-081 Bug 2). Only reachable through `plan_reorg`, which has a `get_parent`
    /// closure; `check_reorg_weighted` needs the parent in `recent_blocks`.
    Evicted,
}

/// genesis -> trunk -> ancestor -> {our_tip | cand_tip}, with the ancestor's STORED
/// height controlled independently of its REAL height.
pub(super) struct TrunkFork {
    pub handler: ReorgHandler,
    pub trunk: Hash,
    pub ancestor: Hash,
    pub our_tip: Hash,
    pub cand_tip: Hash,
    pub heights: HashMap<Hash, u64>,
    pub parents: HashMap<Hash, Hash>,
}

impl TrunkFork {
    pub fn get_height(&self) -> impl Fn(&Hash) -> Option<u64> + '_ {
        |h| self.heights.get(h).copied()
    }
    pub fn get_parent(&self) -> impl Fn(&Hash) -> Option<Hash> + '_ {
        |h| self.parents.get(h).copied()
    }
    /// The stored height, or `None` when the ancestor is not tracked.
    pub fn stored_ancestor_height(&self) -> Option<u64> {
        self.handler
            .get_block_weight(&self.ancestor)
            .map(|w| w.height)
    }
}

/// Build [`TrunkFork`]. `ancestor_real_h` is what `get_height` reports; `record` decides
/// what `block_weights` holds. Both tips weigh [`INCIDENT_WEIGHT`], so a tie is the
/// default and the weight half never silently decides a finality test.
///
/// CAVEAT, deliberate: `record_block_with_height` is itself gated by
/// `inc_i_147_activation_height`, so with `inc_i_147_gate = u64::MAX` even
/// [`AncestorRecord::Real`] stores a DERIVED height. The requested value is honoured
/// only at `inc_i_147_gate = 0`. Never assume the stored height — read it back with
/// [`TrunkFork::stored_ancestor_height`]. The parity tests feed that read-back value to
/// the oracle, which is what makes them a comparison and not an assumption.
pub(super) fn trunk_fork(
    ancestor_real_h: u64,
    record: AncestorRecord,
    inc_i_147_gate: u64,
    m5_gate: u64,
) -> TrunkFork {
    let trunk = crypto::hash::hash(b"m5_trunk_below_the_fork_point");
    let ancestor = crypto::hash::hash(b"m5_common_ancestor");
    let (cand_tip, our_tip) = ordered_pair(b"m5_tf_candidate", b"m5_tf_ours");

    let mut handler = ReorgHandler::with_activation_heights(inc_i_147_gate, m5_gate);

    match record {
        AncestorRecord::Real => {
            handler.record_block_with_height(trunk, Hash::ZERO, 0, ancestor_real_h - 1);
            handler.record_block_with_height(ancestor, trunk, 0, ancestor_real_h);
        }
        AncestorRecord::Synthetic(s) => {
            // Seed the TRUNK at s-1 so the fork path derives exactly `s` for the
            // ancestor while its parent link stays correct — the ancestry walk must
            // remain intact or the test would be measuring two things at once.
            handler.record_block_with_height(trunk, Hash::ZERO, 0, s.saturating_sub(1));
            handler.record_fork_block(ancestor, trunk, 0);
        }
        AncestorRecord::Evicted => {
            handler.record_block_with_height(trunk, Hash::ZERO, 0, ancestor_real_h - 1);
        }
    }
    handler.record_block_with_height(our_tip, ancestor, INCIDENT_WEIGHT, ancestor_real_h + 1);
    handler.record_fork_block_with_height(cand_tip, ancestor, INCIDENT_WEIGHT, ancestor_real_h + 1);

    let heights = HashMap::from([
        (trunk, ancestor_real_h - 1),
        (ancestor, ancestor_real_h),
        (our_tip, ancestor_real_h + 1),
        (cand_tip, ancestor_real_h + 1),
    ]);
    let parents = HashMap::from([(ancestor, trunk), (our_tip, ancestor), (cand_tip, ancestor)]);

    TrunkFork {
        handler,
        trunk,
        ancestor,
        our_tip,
        cand_tip,
        heights,
        parents,
    }
}

/// The PRE-M5 weight/tie rule of the wedge escape, transcribed from
/// `bins/node/src/node/wedge_escape.rs:119-132`: `fork_weight <= our_weight` gives up.
///
/// `true` = switch to the candidate.
// The `!(<=)` shape is the transcription: `wedge_escape.rs:132` gives up on `<=`, and
// the oracle's job is to look like the source line it copies, not to be minimal.
#[allow(clippy::nonminimal_bool)]
pub(super) fn legacy_weight_switch(our_w: u64, cand_w: u64) -> bool {
    !(cand_w <= our_w)
}

/// The PRE-M5 weight/tie rule of the gossip door, transcribed from
/// `reorg/mod.rs:336-363`: strictly lighter is refused; on an exact tie the LOWER block
/// hash wins (`>= current_tip` is refused).
///
/// `true` = switch to the candidate.
pub(super) fn legacy_gossip_switch(our_w: u64, cand_w: u64, cand: &Hash, ours: &Hash) -> bool {
    if cand_w < our_w {
        return false;
    }
    if cand_w == our_w {
        return cand.as_bytes() < ours.as_bytes();
    }
    true
}

/// The PRE-M5 finality ordering of `plan_reorg`, transcribed from `mod.rs:556-597`.
///
/// `true` = the plan is admitted (the finality guard did not refuse it).
pub(super) fn legacy_plan_reorg_admits(
    mirror: Option<u64>,
    synthetic: Option<u64>,
    real: Option<u64>,
    inc_i_147_gate: u64,
) -> bool {
    let Some(finality_height) = mirror else {
        return true;
    };
    let post_activation = real.is_some_and(|h| h >= inc_i_147_gate);
    let ancestor_height = if post_activation {
        real.expect("post_activation implies real.is_some()")
    } else {
        match synthetic {
            Some(h) => h,
            None => match real {
                Some(h) => h,
                // [ANCESTOR_UNKNOWN] — declines the reorg.
                None => return false,
            },
        }
    };
    ancestor_height >= finality_height
}

/// The PRE-M5 rule of `should_reorg_by_weight_with_tiebreak`, transcribed from
/// `reorg/mod.rs:284-294`.
///
/// NOT the same rule as [`legacy_gossip_switch`]: this one requires `new_weight > 0`
/// before it will tie-break, and `check_reorg_weighted` does not. At a ZERO-weight tie
/// the two pre-M5 rules therefore already disagree with each other, which is why they
/// are transcribed separately rather than shared.
///
/// This method has ZERO production callers (measured workspace-wide, run 542). It is
/// pinned so that "M5 did not silently re-point it at the new authority below the gate"
/// is checkable, and so that its deletion is a visible decision rather than a drift.
pub(super) fn legacy_tiebreak_method_switch(
    our_w: u64,
    cand_w: u64,
    cand: &Hash,
    ours: &Hash,
) -> bool {
    if cand_w > our_w {
        return true;
    }
    if cand_w == our_w && cand_w > 0 {
        return cand.as_bytes() < ours.as_bytes();
    }
    false
}
