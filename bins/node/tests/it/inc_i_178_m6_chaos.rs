//! INC-I-178 M6 — C11 / REQ-BLS-010 (Must): half the fleet stops BLS-signing
//! post-AH and block production continues, observably.
//!
//! C11 is the one open constraint that is a LIVENESS constraint: "producer without a
//! valid aggregate still produces; fallback rate-observed". The first half is a chaos
//! test, the second half is a metric — a fallback nobody can see is a fallback nobody
//! notices going to 100 %.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1: `Node::build_block_content(..) -> Result<Option<(BlockHeader, Vec<Transaction>, Vec<u8>)>>`
//!       O1 `Some`/`None` — a `None` at a slot IS the liveness failure
//!       O2 the body bitfield
//!       O3 `node.last_built_aggregate`
//!       O4 `header.presence_root`
//!       O5 the process-global `doli_attestation_bitfield_fill_ratio` gauge
//!       O6 `node.parent_sig_pool` — read, never consumed; asserted negatively
//!
//!   F2: `Node::validate_block_for_apply(&Block, u64, Full) -> Result<(), ValidationError>`
//!       O7 return value — every built block must be ACCEPTED by the same fleet
//!
//!   PATHS: P-full (a pool holding every producer's signature for the parent),
//!     P-empty (an empty pool), P-pre-AH-empty (below the gate with no attesters —
//!     the zero-width universe that a ratio can divide by).
//!   INPUT PARTITIONS: one epoch of slots, alternating P-full and P-empty.
//!   MATRIX: O1/O7 on every slot; O2/O3/O4 per path; O5 on all three paths; O6 once.
//!
//! COUNTER HAZARD (M5 lesson). The gauge and the `doli_attestation_verify_*` counters are
//! process-global and the `it` binary runs concurrently, so every test here holds
//! `counter_lock()` across snapshot -> act -> read.

use crypto::{bls_sign, PublicKey};
use doli_core::attestation::bls_attest_msg;
use doli_core::presence_commitment;
use doli_node::node::attestation::commit::is_canonical_empty_attendance_at;

use crate::inc_i_178_m0_common::{err_text, make_node, N_SMALL};
use crate::inc_i_178_m5_common::{
    assemble_with, build_scheduled, counter_lock, full_mode_height, seed_parent_pool,
    stamp_valid_vdf,
};
use crate::inc_i_204_m0_common::{encode_registry, exported_value};

const FAMILY_FILL_RATIO: &str = "doli_attestation_bitfield_fill_ratio";

/// A producer's pooled signature for the parent under test, kept so the pool can be
/// emptied and refilled without re-registering keys.
type Pooled = Vec<(PublicKey, [u8; 96])>;

/// A node whose gate is OPEN at `height`, plus every producer's parent signature held
/// aside so each slot can choose to have a pool or not.
struct Fleet {
    node: doli_node::node::Node,
    height: u64,
    parent: crypto::Hash,
    pooled: Pooled,
    _tmp: tempfile::TempDir,
}

async fn fleet(n: usize) -> Fleet {
    let (mut node, producers, _tmp) = make_node(n).await;
    let height = full_mode_height(&node);
    node.inc_i_178_attestation_bls_activation_height = height;

    let parent = node.chain_state.read().await.best_hash;
    let signers = seed_parent_pool(&mut node, &producers, n).await;
    let pooled: Pooled = signers
        .iter()
        .map(|s| {
            let sig = bls_sign(&bls_attest_msg(&parent), s.bls.secret_key())
                .expect("BLS signing must succeed");
            (s.pk, *sig.as_bytes())
        })
        .collect();

    Fleet {
        node,
        height,
        parent,
        pooled,
        _tmp,
    }
}

impl Fleet {
    /// `true` = this slot's producer holds every parent signature; `false` = it holds
    /// none, the half of the fleet that stopped BLS-signing.
    fn set_pool(&mut self, signing: bool) {
        self.node.parent_sig_pool.clear();
        if signing {
            for (pk, sig) in &self.pooled {
                self.node.parent_sig_pool.insert(self.parent, *pk, *sig);
            }
        }
    }

    /// Build through the REAL builder and validate through the ONE funnel every gossiped
    /// block takes.
    async fn build_and_validate(&mut self) -> (Vec<u8>, Vec<u8>, crypto::Hash) {
        let height = self.height;
        let (mut header, txs, bitfield) = build_scheduled(&mut self.node, height).await;
        let aggregate = std::mem::take(&mut self.node.last_built_aggregate);
        stamp_valid_vdf(&mut header, &self.node);
        let root = header.presence_root;
        let block = assemble_with(header, txs, bitfield.clone(), aggregate.clone());

        let verdict = self
            .node
            .validate_block_for_apply(&block, height, doli_core::validation::ValidationMode::Full)
            .await;
        assert!(
            verdict.is_ok(),
            "every slot must produce a block the fleet ACCEPTS; got {:?}",
            err_text(&verdict)
        );
        (bitfield, aggregate, root)
    }
}

// ===========================================================================
// P-full / P-empty — C11 liveness.
// ===========================================================================

/// REQ-BLS-010 (Must) / C11 — Decision: a failure means the activation height is a
/// scheduled chain halt. Post-AH a producer's bitfield comes from a pool of gossiped BLS
/// signatures it does not control; a producer that receives none must still emit a block
/// its peers accept, or the first partial-adoption window after the gate stalls every slot
/// owned by an unlucky producer. That is the death-spiral shape of v6.17.1, and it is
/// reachable on the very first block above the gate.
#[tokio::test]
async fn req_bls_010_m6_chaos_half_the_fleet_stops_bls_signing_and_production_continues() {
    let _guard = counter_lock().await;
    let mut f = fleet(N_SMALL).await;
    // "One epoch of slots", read from the shipped params — never a literal.
    let slots = f.node.config.network.blocks_per_reward_epoch();
    assert!(slots >= 2, "the alternation needs both halves");
    let ah = f.node.inc_i_178_attestation_bls_activation_height;
    let height = f.height;

    let mut saw_full = false;
    let mut saw_empty = false;
    for k in 0..slots {
        let signing = k % 2 == 0;
        f.set_pool(signing);
        let (bitfield, aggregate, root) = f.build_and_validate().await;

        if signing {
            saw_full = true;
            assert!(
                bitfield.iter().any(|b| *b != 0),
                "slot {k}: a producer holding every parent signature must set bits"
            );
            assert_eq!(
                aggregate.len(),
                96,
                "slot {k}: the set bits must be covered by ONE aggregate"
            );
            assert_eq!(
                root,
                presence_commitment(&bitfield, &aggregate),
                "slot {k}: the root must bind the pair it carries (D6/C9)"
            );
        } else {
            saw_empty = true;
            assert!(
                bitfield.is_empty(),
                "slot {k}: an empty pool must yield an EMPTY bitfield, not a fabricated one"
            );
            assert!(
                aggregate.is_empty(),
                "slot {k}: there is nothing to aggregate, and bls_aggregate rejects an \
                 empty set"
            );
            assert!(
                is_canonical_empty_attendance_at(ah, height, &root, &bitfield),
                "slot {k}: the zero-attester block must carry the CANONICAL empty \
                 commitment, not Hash::ZERO — the rewards, RPC and schedule empty \
                 detectors all key on it (M4)"
            );
        }
    }
    assert!(
        saw_full && saw_empty,
        "the chaos loop must exercise BOTH halves of the fleet"
    );

    // O6: reading the pool must not consume it — a producer that emptied its own pool by
    // building would only ever attest once per parent.
    f.set_pool(true);
    let before = f.node.parent_sig_pool.total_signatures();
    let _ = f.build_and_validate().await;
    assert_eq!(
        f.node.parent_sig_pool.total_signatures(),
        before,
        "O6: building must READ the parent pool, never drain it (the pool is cleared at \
         the epoch boundary only)"
    );
}

// ===========================================================================
// O5 — C11's "fallback rate-observed" half.
// ===========================================================================

/// REQ-BLS-010 (Must) / C11 — Decision: a failure means the fallback is invisible. Every
/// producer emitting the canonical EMPTY commitment is a perfectly healthy-looking chain —
/// blocks land, validation passes, no error is logged — while attestation coverage is
/// zero and every producer is silently accruing an unqualified epoch. Without this series
/// the first symptom is the reward distribution at the NEXT epoch boundary, six minutes of
/// evidence too late. This is the INC-I-187 shape, so the test reads the value back out of
/// the RENDERED exposition, not off the handle.
#[tokio::test]
async fn req_bls_010_m6_chaos_the_fill_ratio_is_registered_and_written() {
    let _guard = counter_lock().await;

    assert!(
        encode_registry().contains(FAMILY_FILL_RATIO),
        "{FAMILY_FILL_RATIO} is not published by register_metrics(); an alert on \
         attestation coverage would have nothing to evaluate"
    );

    let mut f = fleet(N_SMALL).await;

    f.set_pool(true);
    let (bitfield, _, _) = f.build_and_validate().await;
    let full = exported_value(FAMILY_FILL_RATIO, &[])
        .expect("the gauge must render a series after a real build");
    assert!(
        full > 0.0,
        "registered-but-never-written is the INC-I-187 failure: a full pool built a \
         {}-byte bitfield and the exported ratio is still {full}",
        bitfield.len()
    );
    assert!(
        full <= 1.0,
        "the ratio is set bits over universe width and cannot exceed 1.0; got {full}"
    );

    f.set_pool(false);
    let _ = f.build_and_validate().await;
    let empty =
        exported_value(FAMILY_FILL_RATIO, &[]).expect("the gauge must still render a series");
    assert!(
        empty < full,
        "the degraded half must move the series DOWN ({full} -> {empty}); a gauge that \
         does not fall when coverage collapses is not measuring coverage"
    );
    assert_eq!(
        empty, 0.0,
        "a zero-attester block has zero coverage; got {empty}"
    );
}

/// REQ-BLS-010 (Must) — Decision: a failure means the fill-ratio computation divides by a
/// zero universe. Below the gate `assembly.rs` passes an EMPTY universe whenever no
/// producer attested this minute, so the degenerate input is on the ordinary pre-AH path,
/// not a corner case — and `0/0` in an f64 gauge exports as `NaN`, which Prometheus stores
/// and every downstream `rate()`/`avg()` propagates.
#[tokio::test]
async fn req_bls_010_m6_chaos_the_fill_ratio_survives_a_zero_width_universe() {
    let _guard = counter_lock().await;
    let mut f = fleet(N_SMALL).await;

    // Below the gate AND with nothing in the minute tracker: `assembly.rs` builds an
    // empty universe, which is the only input that can produce 0/0.
    f.node.inc_i_178_attestation_bls_activation_height = f.height + 1;
    f.set_pool(false);
    f.node.minute_tracker = doli_core::attestation::MinuteAttestationTracker::new();

    let (bitfield, aggregate) = {
        let height = f.height;
        let (_header, _txs, bitfield) = build_scheduled(&mut f.node, height).await;
        let aggregate = std::mem::take(&mut f.node.last_built_aggregate);
        (bitfield, aggregate)
    };
    assert!(
        bitfield.is_empty() && aggregate.is_empty(),
        "fixture: this build must take the zero-attester pre-AH arm"
    );

    let ratio = exported_value(FAMILY_FILL_RATIO, &[])
        .expect("the gauge must render a series even for a zero-width universe");
    assert!(
        (0.0..=1.0).contains(&ratio),
        "the exported ratio must stay a real number in [0, 1]; got {ratio} (a NaN here \
         poisons every aggregation built on the series)"
    );
}
