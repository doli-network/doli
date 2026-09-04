//! INC-I-178 M5 — shared harness for the post-AH aggregate VERIFIER (D7).
//!
//! OUTPUT CONTRACT: N/A — fixture file. It declares no `#[test]`; the
//! enumerations live with the functions under test in the sibling
//! `inc_i_178_m5_*` modules. INPUT PARTITIONS: N/A — fixture file.
//!
//! COUNTER READS ARE DELTA-BASED. The `it` binary runs every test in ONE
//! process against the process-global `lazy_static` REGISTRY, and cargo runs
//! those tests concurrently, so an absolute counter value is shared state and
//! asserting on it is a race. Every consumer snapshots with [`Counters::now`]
//! before the action and asserts on [`Counters::delta`]. A test that needs an
//! exclusive window over the counters holds [`counter_lock`] across
//! snapshot -> act -> read.
//!
//! WHY FULL MODE, AND WHAT IT COSTS. The verifier under test SKIPS its pairing
//! steps in any non-`Full` mode, so a Light-mode reject test would be vacuous.
//! Reaching `Full` against `Node::new_for_test` (which builds `Network::Devnet`,
//! and devnet has `vdf_enabled() == true`) needs three things the M0 harness does
//! not do, all three established by measurement, not by reading:
//!   1. a height ABOVE `ConsensusParams::bootstrap_blocks` (60 on devnet) — at
//!      `safe_build_height` (45) `validate_block_with_mode` skips the VDF even in
//!      Full mode, so a VDF-ordering witness taken there proves nothing.
//!      [`full_mode_height`] walks past it.
//!   2. `header.producer == epoch_producer_list[slot % len]` — Full mode runs
//!      `validate_producer_eligibility`, and `build_via_production` passes
//!      `node.producer_key` unconditionally. [`build_scheduled`] passes the
//!      round-robin-expected key for the slot it actually builds in.
//!   3. a real hash-chain VDF on the header — [`stamp_valid_vdf`].
//!
//! `inc_i_178_m5_verify::positive_control_*` is the instrument check for all three.

#![allow(dead_code)] // each consumer uses a subset

use std::sync::OnceLock;

use crypto::{bls_sign, BlsKeyPair, KeyPair, PublicKey};
use doli_core::attestation::bls_attest_msg;
use doli_core::tpop::heartbeat::hash_chain_vdf;
use doli_core::transaction::Transaction;
use doli_core::validation::{ValidationError, ValidationMode};
use doli_core::{presence_commitment, Block, BlockHeader};
use doli_node::metrics::{
    ATTESTATION_VERIFY_REJECTED, ATTESTATION_VERIFY_SKIPPED_LIGHT, ATTESTATION_VERIFY_TOTAL,
};
use doli_node::node::Node;
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

use crate::inc_i_178_m0_common::{
    make_node, register_bls, safe_build_height, wait_for_fresh_second,
};

/// The four `reason` labels `verify.rs` publishes. They mirror the `pub(crate)`
/// `REASON_*` constants, which an integration test cannot import; that is the
/// point — the label is a WIRE contract (it lands in a Prometheus series and in
/// an operator-visible error), so spelling it here binds the wire, not the code.
pub const REASON_ROOT_MISMATCH: &str = "root_mismatch";
pub const REASON_AGGREGATE_INVALID: &str = "aggregate_invalid";
pub const REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD: &str =
    "aggregate_nonempty_for_empty_bitfield";
pub const REASON_MISSING_BLS_KEY: &str = "missing_bls_key";

pub const ALL_REASONS: [&str; 4] = [
    REASON_ROOT_MISMATCH,
    REASON_AGGREGATE_INVALID,
    REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD,
    REASON_MISSING_BLS_KEY,
];

/// Stable `error_code()` string of the new variant.
pub const CODE_ATTESTATION_VERIFY_FAILED: &str = "ATTESTATION_VERIFY_FAILED";

pub type Built = (BlockHeader, Vec<Transaction>, Vec<u8>);

// ---------------------------------------------------------------------------
// Counter window
// ---------------------------------------------------------------------------

/// Serialises tests that need an exclusive window over the global counters.
/// `tokio::sync::Mutex`, not `std::sync::Mutex`: every caller holds the guard
/// across `.await`, which trips `clippy::await_holding_lock` on the std type;
/// tokio's guard is designed to be held across an await and never poisons, so
/// one panicking test cannot wedge every other test behind a poisoned lock.
pub async fn counter_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

/// One read of every `doli_attestation_verify_*` series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    pub total: u64,
    pub skipped_light: u64,
    pub rejected: [u64; 4],
}

impl Counters {
    pub fn now() -> Self {
        let mut rejected = [0u64; 4];
        for (i, reason) in ALL_REASONS.iter().enumerate() {
            rejected[i] = ATTESTATION_VERIFY_REJECTED
                .with_label_values(&[*reason])
                .get();
        }
        Self {
            total: ATTESTATION_VERIFY_TOTAL.get(),
            skipped_light: ATTESTATION_VERIFY_SKIPPED_LIGHT.get(),
            rejected,
        }
    }

    /// `self` MINUS `before`. Counters are monotonic, so a negative delta is a
    /// registry reset and is a bug in the test harness, not in the node.
    pub fn delta(&self, before: &Counters) -> Counters {
        Counters {
            total: self.total - before.total,
            skipped_light: self.skipped_light - before.skipped_light,
            rejected: std::array::from_fn(|i| self.rejected[i] - before.rejected[i]),
        }
    }

    pub fn rejected_for(&self, reason: &str) -> u64 {
        let i = ALL_REASONS
            .iter()
            .position(|r| *r == reason)
            .expect("unknown reason label");
        self.rejected[i]
    }

    pub fn rejected_total(&self) -> u64 {
        self.rejected.iter().sum()
    }
}

// ---------------------------------------------------------------------------
// Verdict binding
// ---------------------------------------------------------------------------

/// Bind a reject to the NEW variant, its reason, and both heights it reports.
///
/// Matching the variant (not the Display text) is deliberate: a reject that
/// merely "contains the word" would also pass for the legacy
/// `InvalidTransaction("presence_root mismatch")` string that the pre-AH path
/// emits, and the whole milestone is the claim that post-AH rejects come from
/// the aggregate verifier.
pub fn expect_reject(
    verdict: &Result<(), ValidationError>,
    reason: &str,
    height: u64,
    activation_height: u64,
    ctx: &str,
) {
    match verdict {
        Err(
            e @ ValidationError::AttestationVerifyFailed {
                reason: got_reason,
                height: got_h,
                activation_height: got_ah,
            },
        ) => {
            assert_eq!(got_reason, reason, "{ctx}: wrong reject reason");
            assert_eq!(*got_h, height, "{ctx}: reject reports the wrong height");
            assert_eq!(
                *got_ah, activation_height,
                "{ctx}: reject reports the wrong activation height"
            );
            assert_eq!(
                e.error_code(),
                CODE_ATTESTATION_VERIFY_FAILED,
                "{ctx}: the stable error code is the operator-facing contract"
            );
        }
        other => panic!("{ctx}: expected AttestationVerifyFailed({reason}); got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Block construction
// ---------------------------------------------------------------------------

/// Recompute the hash-chain VDF the Full-mode validator re-derives.
pub fn stamp_valid_vdf(header: &mut BlockHeader, node: &Node) {
    let iters = node.config.network.heartbeat_vdf_iterations();
    header.vdf_output.value = hash_chain_vdf(&header.vdf_input(), iters).to_vec();
}

/// A well-formed 32-byte output that is NOT the chain for this input.
pub fn stamp_invalid_vdf(header: &mut BlockHeader) {
    header.vdf_output.value = vec![0xABu8; 32];
}

/// A height at which `ValidationMode::Full` actually verifies the VDF.
///
/// `safe_build_height` alone is not enough: it lands below
/// `ConsensusParams::bootstrap_blocks`, where `validate_block_with_mode` skips
/// `validate_vdf`. Everything is derived from the shipped params — never a
/// literal height (INV-GOV-001, commit `18779b1e`).
pub fn full_mode_height(node: &Node) -> u64 {
    let bpe = node.config.network.blocks_per_reward_epoch();
    let mut h = safe_build_height(node);
    while node.params.is_bootstrap(h)
        || doli_core::consensus::reward_epoch::is_epoch_start_with(h, bpe)
        || node.config.network.is_in_genesis(h)
    {
        h += 1;
    }
    h
}

/// Build through the REAL builder with the producer the round-robin scheduler
/// expects for the slot the build lands in, so `validate_producer_eligibility`
/// (Full mode only) accepts the result.
pub async fn build_scheduled(node: &mut Node, height: u64) -> Built {
    let prev_hash = node.chain_state.read().await.best_hash;
    for _ in 0..8 {
        let now = wait_for_fresh_second();
        let slot = node.params.timestamp_to_slot(now);
        let list = &node.epoch_state.producer_list;
        assert!(!list.is_empty(), "the fixture needs an epoch producer list");
        let expected = list[(slot as usize) % list.len()];
        let built = node
            .build_block_content(prev_hash, slot - 1, height, slot, expected)
            .await
            .expect("build_block_content returned Err");
        if let Some(parts) = built {
            return parts;
        }
    }
    panic!("build_block_content returned None on every attempt (slot boundary)");
}

pub fn assemble_with(
    header: BlockHeader,
    txs: Vec<Transaction>,
    bf: Vec<u8>,
    agg: Vec<u8>,
) -> Block {
    Block {
        header,
        transactions: txs,
        aggregate_bls_signature: agg,
        attestation_bitfield: bf,
    }
}

/// Re-commit the header to the body it now carries, so step 1 passes and the
/// test reaches the step it is actually about.
pub fn recommit(block: &mut Block) {
    block.header.presence_root =
        presence_commitment(&block.attestation_bitfield, &block.aggregate_bls_signature);
}

/// One universe member whose BLS key is published ON-CHAIN.
pub struct Signer {
    pub pk: PublicKey,
    pub bls: BlsKeyPair,
}

/// Publish EVERY producer's BLS key on-chain, then drop real parent signatures
/// for the first `n` of them into the node's pool — which is what the post-AH
/// builder reads. Above the gate the minute tracker is NOT consulted, so
/// `record_attesters` has no effect there.
///
/// Registering all and signing only `n` separates two rejects that would
/// otherwise be confounded: a forged EXTRA bit must land on a producer that HAS
/// a key (so the verifier reaches the pairing and answers `aggregate_invalid`),
/// while `missing_bls_key` is reached only by explicitly clearing a key.
pub async fn seed_parent_pool(node: &mut Node, producers: &[KeyPair], n: usize) -> Vec<Signer> {
    let prev_hash = node.chain_state.read().await.best_hash;
    let mut signers = Vec::with_capacity(n);
    for (i, kp) in producers.iter().enumerate() {
        let bls = BlsKeyPair::generate();
        register_bls(node, kp.public_key(), &bls).await;
        if i < n {
            let sig = bls_sign(&bls_attest_msg(&prev_hash), bls.secret_key())
                .expect("BLS signing must succeed");
            node.parent_sig_pool
                .insert(prev_hash, *kp.public_key(), *sig.as_bytes());
            signers.push(Signer {
                pk: *kp.public_key(),
                bls,
            });
        }
    }
    signers
}

/// A node whose gate is OPEN at the returned height, plus an honest post-AH
/// block that passes `Full` unmodified.
pub struct Fixture {
    pub node: Node,
    pub producers: Vec<KeyPair>,
    pub height: u64,
    pub signers: Vec<Signer>,
    pub block: Block,
    _tmp: TempDir,
}

/// `n_signers == 0` yields the canonical-empty (liveness) shape.
pub async fn post_ah_fixture(n_producers: usize, n_signers: usize) -> Fixture {
    let (mut node, producers, _tmp) = make_node(n_producers).await;
    let height = full_mode_height(&node);
    node.inc_i_178_attestation_bls_activation_height = height;

    let signers = seed_parent_pool(&mut node, &producers, n_signers).await;
    let (mut header, txs, bf) = build_scheduled(&mut node, height).await;
    let agg = std::mem::take(&mut node.last_built_aggregate);
    stamp_valid_vdf(&mut header, &node);
    let block = assemble_with(header, txs, bf, agg);

    Fixture {
        node,
        producers,
        height,
        signers,
        block,
        _tmp,
    }
}

impl Fixture {
    pub fn ah(&self) -> u64 {
        self.node.inc_i_178_attestation_bls_activation_height
    }

    pub async fn validate(
        &self,
        block: &Block,
        mode: ValidationMode,
    ) -> Result<(), ValidationError> {
        self.node
            .validate_block_for_apply(block, self.height, mode)
            .await
    }

    pub async fn validate_full(&self, block: &Block) -> Result<(), ValidationError> {
        self.validate(block, ValidationMode::Full).await
    }
}

/// Flip the lowest CLEAR bit of `bitfield` and return its index, or `None` when
/// every bit in the allocated width is already set.
pub fn set_one_extra_bit(bitfield: &mut [u8], width: usize) -> Option<usize> {
    for i in 0..width {
        let (byte, bit) = (i / 8, i % 8);
        if bitfield[byte] & (1 << bit) == 0 {
            bitfield[byte] |= 1 << bit;
            return Some(i);
        }
    }
    None
}

/// Clear the producer's on-chain BLS key, the exact inverse of `register_bls`.
pub async fn clear_bls(node: &Node, pk: &PublicKey) {
    let mut ps = node.producer_set.write().await;
    ps.get_by_pubkey_mut(pk)
        .expect("target must be a ProducerSet member")
        .bls_pubkey = Vec::new();
}

/// The stray-bit denominator the validator itself uses, so a forged bit lands
/// INSIDE the legal width and is judged by the aggregate verifier rather than
/// by the pre-existing "bits set beyond producer_count" guard.
pub async fn universe_width(node: &Node, height: u64) -> usize {
    let active: Vec<PublicKey> = {
        let ps = node.producer_set.read().await;
        ps.active_producers_at_height(height)
            .iter()
            .map(|p| p.public_key)
            .collect()
    };
    doli_node::node::attestation::commit::stray_bit_universe_width_at(
        node.inc_i_178_attestation_bls_activation_height,
        height,
        &node.epoch_state.producer_list,
        &active,
    )
}
