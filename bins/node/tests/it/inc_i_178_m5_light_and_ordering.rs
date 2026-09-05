//! INC-I-178 M5 — REQ-BLS-007 F2/C3 (the Light / divergent-universe skip),
//! C8/F11 (the DoS ordering witness), and REQ-BLS-002 pre-AH byte identity.
//!
//! OUTPUT CONTRACT
//!
//! F1: `Node::validate_block_for_apply(&self, &Block, height, ValidationMode)`
//!   O1 return value
//!   O2 mutable params — NONE; O3 receiver/self — NONE
//!   O4 `doli_attestation_verify_total`
//!   O5 `doli_attestation_verify_skipped_light_total`
//!   O6 `doli_attestation_verify_rejected_total{reason}`
//!
//!   PATHS:
//!     P-pre  `height < ah`  -> `Ok(())`/legacy verdict, NO counter touched at all
//!     P2a    `mode != Full` -> steps 3-4 skipped, `skipped_light` +1, step 1 STILL runs
//!     P2b    `snap_sync_height.is_some()` -> same skip in Full mode
//!     P-ord  the block fails `validate_block_with_mode` (VDF) FIRST -> the
//!            verifier is never reached and `verify_total` is UNCHANGED
//!
//!   INPUT PARTITIONS:
//!     I1 forged aggregate  x Light                 -> ACCEPT, skipped_light +1
//!     I2 forged aggregate  x Full + snap_sync      -> ACCEPT, skipped_light +1
//!     I3 wrong ROOT        x Light                 -> REJECT root_mismatch
//!     I4 wrong ROOT        x Full + snap_sync      -> REJECT root_mismatch
//!     I5 forged aggregate + INVALID VDF x Full     -> VDF error, verify_total +0
//!     I6 at `AH - 1`: stripped aggregate / empty aggregate / empty bitfield with
//!        garbage / legacy preimage — all behave as they do on the current tree
//!
//!   MATRIX: O1 per partition; O4/O5/O6 as deltas on I1-I5; O4/O5/O6 asserted
//!   ZERO across all of I6.
//!
//! WHY THE SKIP EXISTS (F2/C3): a node that finished snap sync after the AH has a
//! provably DIVERGENT `epoch_state.producer_list`, so the universe it would gather
//! keys against is not the universe the block was built over. Verifying there would
//! reject every honest gap block and deadlock every joining node — the skip is the
//! whole reason the milestone does not brick sync. Step 1 needs NO universe, so it
//! keeps running: a snap-syncing node still refuses a body that does not match its
//! own header.

use doli_core::validation::ValidationMode;
use doli_core::Block;

use crate::inc_i_178_m0_common::make_node;
use crate::inc_i_178_m0_common::{active_at, err_text, record_attesters, unix_now, N_SMALL};
use crate::inc_i_178_m5_common::{
    assemble_with, build_scheduled, expect_reject, full_mode_height, post_ah_fixture, recommit,
    seed_parent_pool, set_one_extra_bit, stamp_invalid_vdf, stamp_valid_vdf, universe_width,
    Counters, REASON_AGGREGATE_INVALID, REASON_ROOT_MISMATCH,
};

const N_SIGNERS: usize = 8;

// ===========================================================================
// I1-I4 — the Light / divergent-universe skip.
// ===========================================================================

/// REQ-BLS-007 (Must) — Decision: a failure in the SKIP half means every node that
/// snap-syncs across the activation height verifies an aggregate against a producer
/// list it provably does not share, rejects every honest gap block, and can never
/// join — the milestone would brick onboarding for the whole network. A failure in
/// the STILL-REJECTS half means the skip was implemented as "return Ok early", which
/// throws away the one check that needs no universe and lets a trusted-path block
/// carry a body its own header does not commit to.
#[tokio::test]
async fn req_bls_007_light_mode_skips_pairing_but_checks_root() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = crate::inc_i_178_m5_common::counter_lock().await;
    let mut f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let width = universe_width(&f.node, f.height).await;

    // A block whose aggregate is forged: the pairing WOULD reject it in Full.
    let mut forged = f.block.clone();
    set_one_extra_bit(&mut forged.attestation_bitfield, width).expect("a clear bit must exist");
    recommit(&mut forged);

    // A block whose ROOT is wrong: step 1 must answer in ANY mode.
    let mut bad_root = f.block.clone();
    bad_root.header.presence_root = crypto::hash::hash(b"m5-not-the-commitment");

    // Control: the forged block IS rejected in Full, or "Light accepted it" is
    // not evidence of a skip.
    let sanity = f.validate(&forged, ValidationMode::Full).await;
    assert!(
        sanity.is_err(),
        "control: Full must reject the forged aggregate, else the skip below \
         proves nothing"
    );

    // I1 — mode != Full.
    let before = Counters::now();
    let verdict = f.validate(&forged, ValidationMode::Light).await;
    let after = Counters::now();
    assert!(
        verdict.is_ok(),
        "Light must SKIP the pairing on a divergent-universe path; got {:?}",
        err_text(&verdict)
    );
    let d = after.delta(&before);
    assert_eq!(d.skipped_light, 1, "O5: the skip is counted, not silent");
    assert_eq!(d.total, 1, "O4: a skipped block still REACHED the verifier");
    assert_eq!(d.rejected_total(), 0, "O6: no reject series moved");

    // I3 — the root is still enforced in the same mode.
    let before = Counters::now();
    let verdict = f.validate(&bad_root, ValidationMode::Light).await;
    let after = Counters::now();
    expect_reject(
        &verdict,
        REASON_ROOT_MISMATCH,
        f.height,
        f.ah(),
        "wrong root in Light mode",
    );
    assert_eq!(
        after.delta(&before).rejected_for(REASON_ROOT_MISMATCH),
        1,
        "O6: step 1 runs before the skip, so it can still reject"
    );

    // I2 / I4 — the same two shapes with the snap-sync marker set, in FULL mode.
    // This is the F2 trigger: a node that finished snap sync after the AH.
    f.node.snap_sync_height = Some(f.height);

    let before = Counters::now();
    let verdict = f.validate(&forged, ValidationMode::Full).await;
    let after = Counters::now();
    assert!(
        verdict.is_ok(),
        "a snap-synced node must skip the pairing even in Full mode, or it can \
         never finish joining; got {:?}",
        err_text(&verdict)
    );
    assert_eq!(
        after.delta(&before).skipped_light,
        1,
        "O5: snap_sync_height.is_some() takes the same skip branch"
    );

    let verdict = f.validate(&bad_root, ValidationMode::Full).await;
    expect_reject(
        &verdict,
        REASON_ROOT_MISMATCH,
        f.height,
        f.ah(),
        "wrong root while snap-synced",
    );
}

// ===========================================================================
// I5 — C8/F11: the verifier runs AFTER the cheap checks.
// ===========================================================================

/// REQ-BLS-007 (Must) — Decision: a failure means an unauthenticated peer can make
/// every node on the network compute a BLS pairing by sending a block with a garbage
/// VDF and a 96-byte blob — the pairing is orders of magnitude more expensive than
/// the header checks, so ordering it first converts one cheap packet into a
/// network-wide CPU amplifier. `verify_total` staying UNCHANGED is the witness that
/// the verifier was never reached; a return value alone cannot prove that, because
/// the VDF error would be returned either way.
#[tokio::test]
async fn req_bls_007_verify_runs_after_vdf_total_unchanged() {
    let _guard = crate::inc_i_178_m5_common::counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let width = universe_width(&f.node, f.height).await;

    // A block that would reject at step 3 AND fails the VDF.
    let mut poison = f.block.clone();
    set_one_extra_bit(&mut poison.attestation_bitfield, width).expect("a clear bit must exist");
    recommit(&mut poison);

    // Control: with a VALID VDF this exact block reaches the verifier.
    let before = Counters::now();
    let control = f.validate_full(&poison).await;
    let after = Counters::now();
    expect_reject(
        &control,
        REASON_AGGREGATE_INVALID,
        f.height,
        f.ah(),
        "control: the poison block DOES reach the verifier when its VDF is valid",
    );
    assert_eq!(
        after.delta(&before).total,
        1,
        "control: verify_total moves when the VDF passes — so a later ZERO delta \
         means the ordering changed, not that the counter is dead"
    );

    // The witness: same block, deliberately wrong VDF output.
    stamp_invalid_vdf(&mut poison.header);
    let before = Counters::now();
    let verdict = f.validate_full(&poison).await;
    let after = Counters::now();

    assert_eq!(
        verdict.as_ref().err().map(|e| e.error_code()),
        Some("INVALID_VDF_PROOF"),
        "the CHEAP check must answer first; got {:?}",
        err_text(&verdict)
    );
    let d = after.delta(&before);
    assert_eq!(
        d.total, 0,
        "O4: verify_total UNCHANGED — the pairing was never reached by a block \
         that had already failed a cheap check"
    );
    assert_eq!(d.rejected_total(), 0, "O6: no reject series moved either");
    assert_eq!(d.skipped_light, 0, "O5: the skip branch was not reached");
}

// ===========================================================================
// I6 — pre-AH byte identity at `AH - 1`.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure means the milestone changed consensus
/// BELOW its own activation height, which re-runs INC-I-054: every node that has
/// already applied those blocks now disagrees about them, and the disagreement
/// surfaces at the next epoch boundary rather than at the block that caused it. The
/// counters staying at ZERO is the second half of the claim — a verifier that runs
/// pre-AH and merely happens to return `Ok` is still a live consensus dependency.
#[tokio::test]
async fn req_bls_002_same_shapes_at_ah_minus_one_accept_as_today() {
    // exclusive window: asserts an exact zero delta on the global counters.
    let _guard = crate::inc_i_178_m5_common::counter_lock().await;
    let (mut node, producers, _tmp) = make_node(N_SMALL).await;
    let height = full_mode_height(&node);
    // The gate opens one block ABOVE the height under test, so `height` is pre-AH.
    node.inc_i_178_attestation_bls_activation_height = height + 1;
    assert!(
        height < node.inc_i_178_attestation_bls_activation_height,
        "the fixture must sit strictly BELOW the gate"
    );

    // Pre-AH the builder reads the MINUTE tracker, not the parent pool.
    let all: Vec<crypto::PublicKey> = producers.iter().map(|k| *k.public_key()).collect();
    let slot = node.params.timestamp_to_slot(unix_now());
    record_attesters(&mut node, slot, &all);
    // Publish the BLS keys anyway, so nothing below is excused by a missing key.
    let _ = seed_parent_pool(&mut node, &producers, 0).await;

    let (mut header, txs, bf) = build_scheduled(&mut node, height).await;
    stamp_valid_vdf(&mut header, &node);
    assert!(
        !bf.is_empty(),
        "the pre-AH fixture needs a non-empty legacy bitfield"
    );
    assert_eq!(
        header.presence_root,
        crypto::hash::hash(&bf),
        "below the gate the root IS the legacy BLAKE3(bitfield) preimage"
    );
    let active = active_at(&node, height).await;
    assert!(!active.is_empty(), "fixture sanity");

    let honest = assemble_with(header, txs, bf.clone(), Vec::new());
    let before = Counters::now();

    // The pre-AH baseline: the legacy block validates.
    let verdict = node
        .validate_block_for_apply(&honest, height, ValidationMode::Full)
        .await;
    assert!(
        verdict.is_ok(),
        "pre-AH the legacy block must validate exactly as it does on the current \
         tree; got {:?}",
        err_text(&verdict)
    );

    // Shape of I3 — an aggregate present or absent is IGNORED below the gate,
    // because the legacy preimage covers the bitfield only.
    let mut stripped: Block = honest.clone();
    stripped.aggregate_bls_signature = Vec::new();
    assert!(
        node.validate_block_for_apply(&stripped, height, ValidationMode::Full)
            .await
            .is_ok(),
        "pre-AH: an empty aggregate is the normal case"
    );

    // Shape of I5/I7 — garbage in the aggregate field is UNBOUND pre-AH. That is
    // the very gap the AH exists to close, and it must stay open below it.
    let mut garbage: Block = honest.clone();
    garbage.aggregate_bls_signature = vec![0x5Au8; 96];
    assert!(
        node.validate_block_for_apply(&garbage, height, ValidationMode::Full)
            .await
            .is_ok(),
        "pre-AH: the legacy preimage does not commit to the aggregate, so garbage \
         there is accepted — changing this below the gate would be a silent fork"
    );

    // Shape of I6 — an empty bitfield short-circuits the legacy check entirely.
    let mut empty_bf: Block = honest.clone();
    empty_bf.attestation_bitfield = Vec::new();
    empty_bf.aggregate_bls_signature = vec![0x5Au8; 96];
    assert!(
        node.validate_block_for_apply(&empty_bf, height, ValidationMode::Full)
            .await
            .is_ok(),
        "pre-AH: `if !bitfield.is_empty()` is the escape hatch, and it must still \
         escape below the gate"
    );

    // Shape of I10 — the POST-AH commitment is what fails below the gate.
    let mut post_root: Block = honest.clone();
    post_root.header.presence_root =
        doli_core::presence_commitment(&post_root.attestation_bitfield, &[]);
    let verdict = node
        .validate_block_for_apply(&post_root, height, ValidationMode::Full)
        .await;
    assert!(
        verdict.is_err(),
        "pre-AH the NEW preimage must not validate, or the gate has no direction"
    );

    // The second half of the claim: nothing below the gate touches the verifier.
    let after = Counters::now();
    let d = after.delta(&before);
    assert_eq!(
        d.total, 0,
        "O4: `height < ah` must return before any counter is touched — a verifier \
         that runs pre-AH is a live consensus dependency even when it returns Ok"
    );
    assert_eq!(d.rejected_total(), 0, "O6: no reject series moved pre-AH");
    assert_eq!(d.skipped_light, 0, "O5: no skip series moved pre-AH");
}
