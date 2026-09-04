//! INC-I-178 M5 — REQ-BLS-002 / REQ-BLS-010: the carried BLS aggregate is
//! actually VERIFIED above the activation height, and inability to aggregate
//! never blocks a block.
//!
//! OUTPUT CONTRACT
//!
//! F1: `Node::validate_block_for_apply(&self, &Block, height, ValidationMode)`
//!     — the ONE call site of the new verifier; no separable entry point exists.
//!   O1 return value — `Ok(())`, or `AttestationVerifyFailed { reason, height,
//!      activation_height }` whose `error_code()` is `ATTESTATION_VERIFY_FAILED`
//!   O2 mutable params — NONE (`&Block` is shared); asserted negatively
//!   O3 receiver/self — NONE (`&self`); asserted via the epoch list and the
//!      producer set surviving a reject unchanged
//!   O4 process-global metric registry — `doli_attestation_verify_total` +1 on
//!      every post-AH block that REACHES the verifier, and
//!      `doli_attestation_verify_rejected_total{reason}` +1 on a reject.
//!      Counted here as a DELTA (see `inc_i_178_m5_common`); the exclusive-window
//!      assertions live in `inc_i_178_m5_observability`.
//!
//!   PATHS (the decision order `verify.rs` implements, post-AH):
//!     P1 root check, UNCONDITIONAL          -> `root_mismatch`
//!     P2 Light / divergent-universe skip     -> `inc_i_178_m5_light_and_ordering`
//!     P3 any bit set: 96-byte aggregate + `bls_verify_aggregate` over exactly
//!        the set-bit keys                    -> `aggregate_invalid`
//!        P3b a set bit whose producer has no usable on-chain key
//!                                            -> `missing_bls_key`
//!     P4 no bit set: the aggregate MUST be empty
//!                                            -> `aggregate_nonempty_for_empty_bitfield`
//!
//!   INPUT PARTITIONS (crossed with the paths above):
//!     I1 honest body, untouched                       -> accept   (P1,P3)
//!     I2 honest body + ONE forged extra bit, recommitted -> P3
//!     I3 honest bitfield, aggregate STRIPPED, original root -> P1
//!     I4 honest bitfield, ONE aggregate byte flipped, recommitted -> P3
//!     I5 non-empty bitfield, EMPTY aggregate, recommitted -> P3 (length arm)
//!     I6 zero-length bitfield, garbage aggregate, original root -> P1
//!     I7 zero-length bitfield, garbage aggregate, recommitted -> P4
//!     I8 all-ZERO-BYTES bitfield, garbage aggregate, recommitted -> P4
//!     I9 canonical empty body (empty pool)            -> accept   (P1,P4)
//!     I10 legacy `BLAKE3(bitfield)` preimage at h >= AH -> P1
//!     I11 honest body, one SET-bit producer's key cleared -> P3b
//!
//!   MATRIX: F1 O1 claimed per partition below; O2/O3 asserted on I2 and I11;
//!   O4 asserted as a delta on I1, I9 and every reject partition.
//!
//! SPECS NOTE (developer must resolve, see docs/.workflow/test-plan-M5.md):
//! `inc_i_178_m4_preimage.rs` currently binds the string `"presence_root
//! mismatch"` for the post-AH non-empty-bitfield root reject, which today is
//! emitted by `validation_checks.rs:421`. I3/I10 below assert that the SAME
//! shape now returns `AttestationVerifyFailed{root_mismatch}`. Both cannot hold.

use crypto::Hash;
use doli_core::presence_commitment;
use doli_core::validation::ValidationMode;

use crate::inc_i_178_m0_common::{err_text, N_SMALL};
use crate::inc_i_178_m5_common::{
    clear_bls, counter_lock, expect_reject, post_ah_fixture, recommit, set_one_extra_bit,
    universe_width, Counters, REASON_AGGREGATE_INVALID,
    REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD, REASON_MISSING_BLS_KEY, REASON_ROOT_MISMATCH,
};

const N_SIGNERS: usize = 8;

// ===========================================================================
// I1 — the instrument check. Nothing below means anything if this fails.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure here says the M5 harness cannot reach
/// `ValidationMode::Full` on an HONEST block, which would make every reject test in
/// this file vacuous — a Light-mode "reject" proves nothing because Light skips the
/// pairing steps under test. This test is the instrument, not the subject.
#[tokio::test]
async fn positive_control_honest_post_ah_block_passes_full_unmodified() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    assert!(
        !f.block.attestation_bitfield.is_empty(),
        "anti-vacuity: the post-AH builder must have produced a real bitfield \
         from the seeded parent pool, or P3 is never reached"
    );
    assert_eq!(
        f.block.aggregate_bls_signature.len(),
        96,
        "anti-vacuity: one compressed BLS aggregate"
    );
    assert!(
        f.height >= f.ah(),
        "the fixture must sit AT or ABOVE the gate"
    );
    assert!(
        !f.node.params.is_bootstrap(f.height),
        "the fixture height must be past bootstrap_blocks, or Full mode skips the \
         VDF and the ordering witness in this milestone is vacuous"
    );

    let before = Counters::now();
    let verdict = f.validate_full(&f.block).await;
    let after = Counters::now();

    assert!(
        verdict.is_ok(),
        "an honestly built post-AH block must pass Full validation unmodified; got {:?}",
        err_text(&verdict)
    );
    assert_eq!(
        after.delta(&before).total,
        1,
        "O4: an accepted post-AH block still REACHES the verifier, so total moves"
    );
    assert_eq!(
        after.delta(&before).rejected_total(),
        0,
        "O4: an accepted block writes no reject series"
    );
}

// ===========================================================================
// I2 — P3: a forged extra bit.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure means a producer can credit any peer it
/// likes by flipping a bit and re-hashing the header, which is the exact gap this
/// milestone exists to close — the bitfield would remain an unverified assertion and
/// reward attribution plus 3-epoch liveness exclusion stay forgeable post-AH.
#[tokio::test]
async fn req_bls_002_forged_extra_bit_rejects() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let width = universe_width(&f.node, f.height).await;

    let mut forged = f.block.clone();
    let idx = set_one_extra_bit(&mut forged.attestation_bitfield, width)
        .expect("the fixture must leave at least one non-attesting producer");
    assert!(
        idx < width,
        "the forged bit must land inside the legal width, or the pre-existing \
         stray-bit guard answers instead of the aggregate verifier"
    );
    // Step 1 must PASS, or this test would only re-prove the root check.
    recommit(&mut forged);

    let before = Counters::now();
    let verdict = f.validate_full(&forged).await;
    let after = Counters::now();

    expect_reject(
        &verdict,
        REASON_AGGREGATE_INVALID,
        f.height,
        f.ah(),
        "forged extra bit",
    );
    let d = after.delta(&before);
    assert_eq!(d.total, 1, "O4: the block reached the verifier");
    assert_eq!(
        d.rejected_for(REASON_AGGREGATE_INVALID),
        1,
        "O4: the reject is attributed to the aggregate, not to the root"
    );
    assert_eq!(
        d.rejected_for(REASON_ROOT_MISMATCH),
        0,
        "O4: step 1 passed, so no root series may move"
    );

    // O2/O3: a reject mutates neither the block nor the node.
    assert_eq!(
        forged.header.presence_root,
        presence_commitment(
            &forged.attestation_bitfield,
            &forged.aggregate_bls_signature
        ),
        "O2: the block the caller handed in is unchanged"
    );
    assert_eq!(
        f.node.epoch_state.producer_list.len(),
        N_SMALL,
        "O3: no epoch-list rotation during validation"
    );
}

// ===========================================================================
// I3 — P1: the aggregate is stripped and the root is left alone.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure means a relaying peer can drop the
/// aggregate in flight and the block still applies, so the aggregate is optional
/// cargo rather than a consensus commitment; every node downstream would then accept
/// an unproven bitfield and the verifier would be dead weight on the honest path only.
#[tokio::test]
async fn req_bls_002_stripped_aggregate_rejects() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    let mut stripped = f.block.clone();
    let original_root = stripped.header.presence_root;
    stripped.aggregate_bls_signature.clear();
    assert_eq!(
        stripped.header.presence_root, original_root,
        "the stripping attack does NOT re-commit — that is what makes it cheap"
    );

    let verdict = f.validate_full(&stripped).await;
    expect_reject(
        &verdict,
        REASON_ROOT_MISMATCH,
        f.height,
        f.ah(),
        "stripped aggregate, original root",
    );
}

// ===========================================================================
// I4 — P3: one byte of the aggregate flipped, root re-committed.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure means the aggregate is only bound by a
/// hash and never by a pairing, so any 96 bytes that the producer also hashes into
/// the root are accepted. That is a root check wearing a verifier's name.
#[tokio::test]
async fn req_bls_002_mutated_aggregate_byte_rejects() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    let mut mutated = f.block.clone();
    mutated.aggregate_bls_signature[0] ^= 0x01;
    assert_ne!(
        mutated.aggregate_bls_signature, f.block.aggregate_bls_signature,
        "anti-vacuity: the mutation must actually change the bytes"
    );
    recommit(&mut mutated);

    let before = Counters::now();
    let verdict = f.validate_full(&mutated).await;
    let after = Counters::now();

    expect_reject(
        &verdict,
        REASON_AGGREGATE_INVALID,
        f.height,
        f.ah(),
        "one aggregate byte flipped",
    );
    assert_eq!(
        after.delta(&before).rejected_for(REASON_AGGREGATE_INVALID),
        1,
        "O4: attributed to the pairing"
    );
}

// ===========================================================================
// I5 — P3 length arm: a real bitfield with no aggregate at all.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure means a producer can claim N attesters
/// while carrying zero bytes of proof, simply by committing the root over the empty
/// aggregate. That is the cheapest possible forgery and it must not survive step 3.
#[tokio::test]
async fn req_bls_002_valid_bitfield_empty_aggregate_rejects() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    let mut empty_agg = f.block.clone();
    empty_agg.aggregate_bls_signature = Vec::new();
    recommit(&mut empty_agg);
    assert!(
        !empty_agg.attestation_bitfield.is_empty()
            && empty_agg.attestation_bitfield.iter().any(|b| *b != 0),
        "anti-vacuity: bits ARE set, so P4 must not be the branch that answers"
    );

    let verdict = f.validate_full(&empty_agg).await;
    expect_reject(
        &verdict,
        REASON_AGGREGATE_INVALID,
        f.height,
        f.ah(),
        "set bits with a zero-length aggregate",
    );
}

// ===========================================================================
// I6 / I7 / I8 — the empty-bitfield pair. I7 is the branch that proves P4 exists.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure on the ORIGINAL-root half means the
/// post-AH root check is still gated behind `!bitfield.is_empty()` and an empty body
/// is unbound; a failure on the RE-COMMITTED half means step 4 does not exist, and an
/// empty bitfield becomes a free channel for arbitrary bytes that every node must
/// hash, store and relay forever.
#[tokio::test]
async fn req_bls_002_empty_bitfield_garbage_aggregate_rejects() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let garbage = vec![0x5Au8; 96];

    // I6 — original root: step 1 answers, because the root commits to the body
    // that WAS built, not to this one.
    let mut original_root = f.block.clone();
    original_root.attestation_bitfield = Vec::new();
    original_root.aggregate_bls_signature = garbage.clone();
    let verdict = f.validate_full(&original_root).await;
    expect_reject(
        &verdict,
        REASON_ROOT_MISMATCH,
        f.height,
        f.ah(),
        "zero-length bitfield, garbage aggregate, original root",
    );

    // I7 — re-committed: step 1 passes, so only step 4 can answer.
    let before = Counters::now();
    let mut recommitted = original_root.clone();
    recommit(&mut recommitted);
    let verdict = f.validate_full(&recommitted).await;
    let after = Counters::now();
    expect_reject(
        &verdict,
        REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD,
        f.height,
        f.ah(),
        "zero-length bitfield, garbage aggregate, re-committed root",
    );
    assert_eq!(
        after.delta(&before).rejected_for(REASON_ROOT_MISMATCH),
        0,
        "O4: step 1 passed on the re-committed body"
    );

    // I8 — "no bit set" must mean BITS, not LENGTH. An all-zero-BYTES bitfield
    // has the honest width and still carries no claim.
    let mut zero_bytes = f.block.clone();
    zero_bytes.attestation_bitfield = vec![0u8; f.block.attestation_bitfield.len()];
    zero_bytes.aggregate_bls_signature = garbage;
    recommit(&mut zero_bytes);
    let verdict = f.validate_full(&zero_bytes).await;
    expect_reject(
        &verdict,
        REASON_AGGREGATE_NONEMPTY_FOR_EMPTY_BITFIELD,
        f.height,
        f.ah(),
        "all-zero-BYTES bitfield, garbage aggregate, re-committed root",
    );
}

// ===========================================================================
// I9 — REQ-BLS-010 liveness.
// ===========================================================================

/// REQ-BLS-010 (Must) — Decision: a failure means the network HALTS the moment the
/// gate opens. A producer that has received no parent signatures — a fresh node, a
/// partitioned node, the first block after the AH — can build nothing else, so if the
/// canonical empty body does not validate, no such producer can ever produce again.
#[tokio::test]
async fn req_bls_010_empty_bitfield_empty_aggregate_canonical_root_accepts() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, 0).await;

    assert!(
        f.block.attestation_bitfield.iter().all(|b| *b == 0),
        "anti-vacuity: an empty pool must yield a bitfield with no bit set"
    );
    assert!(
        f.block.aggregate_bls_signature.is_empty(),
        "anti-vacuity: an empty pool must yield an EMPTY aggregate"
    );
    assert_eq!(
        f.block.header.presence_root,
        presence_commitment(
            &f.block.attestation_bitfield,
            &f.block.aggregate_bls_signature
        ),
        "the builder must commit the canonical empty body post-AH"
    );

    let before = Counters::now();
    let verdict = f.validate_full(&f.block).await;
    let after = Counters::now();

    assert!(
        verdict.is_ok(),
        "the canonical empty commitment must ACCEPT above the gate; got {:?}",
        err_text(&verdict)
    );
    let d = after.delta(&before);
    assert_eq!(
        d.total, 1,
        "O4: the empty-body block still reaches the verifier and is counted"
    );
    assert_eq!(d.rejected_total(), 0, "O4: no reject series moves");
}

// ===========================================================================
// I10 — P1: the retired preimage above the gate.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure means a node still running the pre-AH
/// preimage keeps producing acceptable blocks after the gate opens, so the two rules
/// coexist on one chain and the fork the activation height exists to prevent happens
/// anyway — silently, at the first block a legacy producer wins.
#[tokio::test]
async fn req_bls_002_legacy_preimage_above_ah_rejects() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    let mut legacy = f.block.clone();
    let legacy_root: Hash = crypto::hash::hash(&legacy.attestation_bitfield);
    assert_ne!(
        legacy_root, legacy.header.presence_root,
        "anti-vacuity: the two preimages must actually differ for this body"
    );
    legacy.header.presence_root = legacy_root;

    let verdict = f.validate_full(&legacy).await;
    expect_reject(
        &verdict,
        REASON_ROOT_MISMATCH,
        f.height,
        f.ah(),
        "pre-AH BLAKE3(bitfield) preimage validated above the gate",
    );
}

// ===========================================================================
// I11 — P3b: a set bit whose producer published no BLS key.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure means the key gather silently DROPS or
/// substitutes a bit it cannot resolve. Dropping shrinks the key set and the pairing
/// then fails for an honest block whose attester simply has not registered yet;
/// substituting makes the aggregate verify against the wrong universe. Either way the
/// bit no longer means what the reward decoder reads it to mean, and the failure is
/// silent — which is why it needs its own labelled reject.
#[tokio::test]
async fn req_bls_002_set_bit_with_empty_bls_pubkey_rejects() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    let victim = f.signers[0].pk;
    clear_bls(&f.node, &victim).await;

    let before = Counters::now();
    let verdict = f.validate_full(&f.block).await;
    let after = Counters::now();

    expect_reject(
        &verdict,
        REASON_MISSING_BLS_KEY,
        f.height,
        f.ah(),
        "a set bit whose producer's on-chain bls_pubkey is empty",
    );
    let d = after.delta(&before);
    assert_eq!(d.total, 1, "O4: the block reached the verifier");
    assert_eq!(
        d.rejected_for(REASON_MISSING_BLS_KEY),
        1,
        "O4: the unresolvable key gets its OWN label, so an operator can tell a \
         registration gap apart from a forgery"
    );
    assert_eq!(
        d.rejected_for(REASON_AGGREGATE_INVALID),
        0,
        "O4: a missing key must NOT be reported as a bad aggregate"
    );

    // O3: the reject left the producer set as the test set it.
    let ps = f.node.producer_set.read().await;
    assert!(
        ps.get_by_pubkey(&victim)
            .expect("victim is a member")
            .bls_pubkey
            .is_empty(),
        "O3: validation does not repair on-chain state"
    );
}

// ===========================================================================
// Mode reachability guard.
// ===========================================================================

/// REQ-BLS-002 (Must) — Decision: a failure means `ValidationMode::Full` is NOT the
/// mode that exercises the pairing in this harness, so every assertion above is
/// measuring something other than the branch it names. It fails loudly rather than
/// letting the suite go green for the wrong reason.
#[tokio::test]
async fn req_bls_002_full_mode_is_the_mode_that_verifies() {
    // exclusive window: this test writes the global attestation-verify counters.
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let mut forged = f.block.clone();
    let width = universe_width(&f.node, f.height).await;
    set_one_extra_bit(&mut forged.attestation_bitfield, width).expect("a clear bit must exist");
    recommit(&mut forged);

    let before = Counters::now();
    let full = f.validate(&forged, ValidationMode::Full).await;
    let after = Counters::now();

    assert!(full.is_err(), "Full must reject the forged bit");
    assert_eq!(
        after.delta(&before).total,
        1,
        "Full is the mode that reaches the pairing"
    );
}
