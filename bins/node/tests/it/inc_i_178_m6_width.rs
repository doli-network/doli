//! INC-I-178 M6 — R5 / REQ-BLS-014: post-AH the body bitfield must be EXACTLY
//! `ceil(universe_len / 8)` bytes; below the gate the tolerant rule stays.
//!
//! Requirements: **REQ-BLS-014** (Should), **REQ-BLS-005** (Must, the pre-AH identity half).
//! TDD RED, EXPECTED: `doli_node::node::attestation::width` does not exist at the M6
//! branch point, so this module does not compile against HEAD.
//!
//! WHAT IS TOLERANT TODAY, MEASURED NOT ASSUMED (`crates/core/src/attestation/bitfield.rs:99`,
//! `validate_attestation_bitfield_vec`). The guard checks two things and neither is a
//! width equality:
//!   - every byte BEYOND `ceil(n/8)` must be zero — so a bitfield one byte too LONG
//!     whose extra byte is `0x00` is ACCEPTED today;
//!   - the stray-bit mask on the last expected byte is guarded by
//!     `expected_bytes <= bitfield.len()` — so a bitfield one byte too SHORT skips the
//!     check entirely and is ACCEPTED today.
//!
//! Both M6 shapes are therefore live ACCEPTs on the current binary. The post-AH tests
//! below are a real behaviour delta, not a restatement of the existing guard.
//!
//! WHY A LENGTHS-ONLY PREDICATE. `bitfield_width_accepted_at` sees `bitfield_len` and
//! `universe_len`, never the bytes, so it CANNOT reproduce the two content checks above.
//! The only shape that keeps the pre-AH path byte-identical is: the new predicate is
//! inert below the gate and is consulted IN ADDITION TO (never instead of)
//! `validate_attestation_bitfield_vec`.
//!
//! OUTPUT CONTRACT — ENUMERATION OF OBSERVABLE OUTPUTS.
//!
//!   F1: `width::bitfield_width_accepted_at(usize, usize, u64, &NetworkParams) -> bool` (PURE)
//!       O1 return value.
//!       O2 mutable params / receiver / store writes / statics / channels — NONE.
//!          Structural: every argument is a `usize`, a `u64` or a shared borrow.
//!       PATHS: P-pre (`height == AH - 1`), P-post (`height == AH`).
//!       INPUT PARTITIONS: I-exact (`len == ceil(u/8)`), I-short (`len == ceil(u/8) - 1`),
//!          I-long (`len == ceil(u/8) + 1`), I-empty (`len == 0`).
//!
//!   F2: `Node::validate_block_for_apply(&Block, u64, ValidationMode) -> Result<(), ValidationError>`
//!       O3 return value — the verdict a gossiped block gets.
//!       O4 process-global `doli_attestation_verify_*` counters (M5). Claimed
//!          negatively on the reject paths: a width reject must fire BEFORE the
//!          aggregate verifier, so `verify_total` must not move.
//!       O5 receiver — `&self`; no store write. Asserted via the producer list length.
//!       PATHS: P-post x {I-exact, I-short, I-long}; P-pre x {I-short, I-long}.
//!
//!   MATRIX: O1 on both paths x all four partitions; O3 on P-post x 3 and P-pre x 2;
//!     O4 on the two post-AH rejects; O2/O5 structural.
//!
//! COUNTER HAZARD (M5 lesson, INC-I-178 M5 close-out): every test here that drives
//! `validate_block_for_apply` at a height >= the activation height holds
//! `inc_i_178_m5_common::counter_lock()` for its whole body. The `it` binary runs tests
//! concurrently in ONE process against a global `lazy_static` registry.

use doli_core::network_params::NetworkParams;
use doli_core::validation::{ValidationError, ValidationMode};
use doli_core::{validate_attestation_bitfield_vec, Network};
use doli_node::node::attestation::width::bitfield_width_accepted_at;

use crate::inc_i_178_m0_common::{err_text, N_SMALL};
use crate::inc_i_178_m5_common::{counter_lock, post_ah_fixture, Counters, Fixture};

const N_SIGNERS: usize = 8;

// ---------------------------------------------------------------------------
// Gate-derived fixtures (the `inc_i_178_m4_gate.rs` idiom, verbatim)
// ---------------------------------------------------------------------------

/// A params value carrying an arbitrary probe gate. The shipped default is `u64::MAX`
/// on every network, so a test that wanted to exercise the post-AH arm at the shipped
/// height could never run.
fn gated(ah: u64) -> NetworkParams {
    let mut p = NetworkParams::defaults(Network::Devnet);
    p.inc_i_178_attestation_bls_activation_height = ah;
    p
}

/// `(pre, post)` read back OUT of the params the code under test is handed. Nothing
/// downstream may spell a height literal (rule `18779b1e`, INV-GOV-001).
fn sides(p: &NetworkParams) -> (u64, u64) {
    let ah = p.inc_i_178_attestation_bls_activation_height;
    assert!(ah > 0, "the probe gate must leave room for a pre-AH height");
    (ah - 1, ah)
}

/// The one width the post-AH rule accepts.
fn exact(universe_len: usize) -> usize {
    universe_len.div_ceil(8)
}

/// Universe sizes that exercise both a byte-aligned and a ragged last byte.
const UNIVERSES: [usize; 4] = [8, 12, 45, 51];

// ---------------------------------------------------------------------------
// Block shaping
// ---------------------------------------------------------------------------

/// The width the validator itself computes for this fixture, read through the shipped
/// universe function rather than counted by the test.
async fn width_of(f: &Fixture) -> usize {
    crate::inc_i_178_m5_common::universe_width(&f.node, f.height).await
}

/// Truncate the honest bitfield by one byte and re-commit the header to the body it now
/// carries, so the block reaches the width guard rather than dying on the commitment.
fn one_byte_short(f: &Fixture) -> doli_core::Block {
    let mut b = f.block.clone();
    assert!(
        b.attestation_bitfield.len() > 1,
        "the fixture bitfield must be wider than one byte for the SHORT shape to exist"
    );
    b.attestation_bitfield.pop();
    crate::inc_i_178_m5_common::recommit(&mut b);
    b
}

/// Append ONE ZERO byte. A zero pad is the shape the current guard accepts: it carries
/// no extra set bit, so nothing but an exact-width rule can reject it.
fn one_byte_long(f: &Fixture) -> doli_core::Block {
    let mut b = f.block.clone();
    b.attestation_bitfield.push(0u8);
    crate::inc_i_178_m5_common::recommit(&mut b);
    b
}

/// Bind a reject to the pre-existing width/stray-bit guard rather than to the M5
/// aggregate verifier. Matching only "is an Err" would pass for either, and the whole
/// point of R5 is that the WIDTH decision rejects first (C8: cheap checks stay early).
fn expect_width_reject(verdict: &Result<(), ValidationError>, ctx: &str) {
    match verdict {
        Err(ValidationError::InvalidTransaction(msg)) => assert!(
            msg.contains("attestation_bitfield"),
            "{ctx}: the reject must come from the body-bitfield guard; got {msg:?}"
        ),
        other => panic!(
            "{ctx}: expected the InvalidTransaction body-bitfield reject; got {:?}",
            other.as_ref().err()
        ),
    }
}

// ===========================================================================
// F1 x P-post — the exact-width rule itself.
// ===========================================================================

/// REQ-BLS-014 (Should) — Decision: a failure means the post-AH committee contract is not
/// a contract at all. If the exact width is REJECTED, every honestly built block dies at
/// the gate and the chain halts on the activation block; if it is merely not REQUIRED, the
/// aggregate keeps being verified against a key list whose length the block does not pin,
/// which is the `≤` denominator R5 exists to close.
#[test]
fn req_bls_014_m6_post_ah_the_exact_width_is_the_only_accepted_width() {
    let p = gated(4_242);
    let (_, post) = sides(&p);

    for u in UNIVERSES {
        let w = exact(u);
        assert!(
            bitfield_width_accepted_at(w, u, post, &p),
            "universe={u}: ceil(u/8)={w} bytes is the honest width and must be accepted"
        );
        assert!(
            !bitfield_width_accepted_at(w - 1, u, post, &p),
            "universe={u}: {} bytes is one byte SHORT and must be rejected post-AH",
            w - 1
        );
        assert!(
            !bitfield_width_accepted_at(w + 1, u, post, &p),
            "universe={u}: {} bytes is one byte LONG and must be rejected post-AH",
            w + 1
        );
        assert!(
            !bitfield_width_accepted_at(0, u, post, &p),
            "universe={u}: a zero-length bitfield is not the exact width. The canonical \
             EMPTY block never reaches this predicate — `validation_checks.rs` guards the \
             whole body block behind `!attestation_bitfield.is_empty()` — so rejecting it \
             here cannot cost liveness (REQ-BLS-010)"
        );
    }
}

/// REQ-BLS-014 (Should) — Decision: a failure means the accepted width is not derived from
/// the SHARED universe the encoder and `post_commit` index against. A width computed from a
/// second denominator re-opens the exact `≤`-vs-`==` disagreement M4 collapsed, one layer
/// down, where nothing else in the milestone would notice.
#[test]
fn req_bls_014_m6_post_ah_the_accepted_width_tracks_the_universe_length() {
    let p = gated(9_001);
    let (_, post) = sides(&p);

    // The accepted width must move with the universe, byte for byte.
    for u in 1..=130usize {
        let accepted: Vec<usize> = (0..20)
            .filter(|len| bitfield_width_accepted_at(*len, u, post, &p))
            .collect();
        assert_eq!(
            accepted,
            vec![exact(u)],
            "universe={u}: exactly one width may be accepted post-AH"
        );
    }
}

// ===========================================================================
// F1 x P-pre — the identity guarantee.
// ===========================================================================

/// REQ-BLS-005 (Must) — Decision: a failure means R5 tightened consensus for blocks BELOW
/// its own activation height. Every historical block whose bitfield is not exactly the
/// current width would be re-judged on replay and on rebuild-from-genesis, which is the
/// INC-I-054 shape: a validator running under its own gate is a live consensus dependency
/// even when nobody thinks it is armed.
#[test]
fn req_bls_014_m6_below_the_gate_the_width_predicate_is_inert() {
    let p = gated(4_242);
    let (pre, _) = sides(&p);

    for u in UNIVERSES {
        let w = exact(u);
        for len in [0usize, w - 1, w, w + 1, w + 4] {
            assert!(
                bitfield_width_accepted_at(len, u, pre, &p),
                "universe={u} len={len}: below the gate the width predicate must accept \
                 everything and leave the verdict to validate_attestation_bitfield_vec"
            );
        }
    }
}

/// REQ-BLS-005 (Must) — Decision: a failure means the shapes M6 newly rejects were NOT
/// accepted before, so the milestone's own premise is wrong and its post-AH tests are
/// re-asserting a guard that already existed. This test reads the CURRENT verdict out of
/// the shipped `validate_attestation_bitfield_vec` and pins it; it never asserts the new
/// rule.
#[test]
fn req_bls_014_m6_the_shapes_m6_rejects_are_accepted_by_todays_guard() {
    for u in UNIVERSES {
        let w = exact(u);
        let full: Vec<usize> = (0..u).collect();
        let honest = doli_core::encode_attestation_bitfield_vec(&full, u);
        assert_eq!(honest.len(), w, "universe={u}: fixture width");

        let mut short = honest.clone();
        short.pop();
        assert!(
            validate_attestation_bitfield_vec(&short, u),
            "universe={u}: today a one-byte-SHORT bitfield is ACCEPTED — that is the \
             tolerance R5 closes, and if this ever fails the M6 post-AH reject tests are \
             testing nothing new"
        );

        let mut long = honest.clone();
        long.push(0u8);
        assert!(
            validate_attestation_bitfield_vec(&long, u),
            "universe={u}: today a one-byte-LONG bitfield with a ZERO pad is ACCEPTED"
        );
    }
}

// ===========================================================================
// F2 — the same three shapes through the ONE funnel every gossiped block takes.
// ===========================================================================

/// REQ-BLS-014 (Should) — Decision: a failure means the exact-width rule is not reachable
/// from the live apply path. A pure predicate that production never consults is the most
/// expensive way a consensus rule can pass its own tests (the REQ-BLS-007 shape, one
/// milestone later).
#[tokio::test]
async fn req_bls_014_m6_post_ah_an_honest_block_keeps_its_exact_width_accept() {
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    let w = width_of(&f).await;
    assert_eq!(
        f.block.attestation_bitfield.len(),
        exact(w),
        "anti-vacuity: the REAL builder must already emit the exact width, or R5 rejects \
         every honest block on the activation block"
    );

    let verdict = f.validate_full(&f.block).await;
    assert!(
        verdict.is_ok(),
        "an honest post-AH block must still ACCEPT; got {:?}",
        err_text(&verdict)
    );
    assert_eq!(
        f.node.epoch_state.producer_list.len(),
        N_SMALL,
        "O5: validation must not mutate node state"
    );
}

/// REQ-BLS-014 (Should) — Decision: a failure means a relay can strip the tail byte off a
/// gossiped bitfield and have the block still accepted. Every bit that byte carried is
/// silently dropped, so the attesters it credited lose attendance for that minute on the
/// nodes that saw the truncated copy — attendance divergence between peers over the same
/// block hash is the INC-I-034 reward-set split.
#[tokio::test]
async fn req_bls_014_m6_post_ah_a_one_byte_short_bitfield_is_rejected() {
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let short = one_byte_short(&f);

    let w = width_of(&f).await;
    assert_eq!(
        short.attestation_bitfield.len() + 1,
        exact(w),
        "fixture: the shape under test is exactly one byte short"
    );

    let before = Counters::now();
    let verdict = f.validate_full(&short).await;
    let after = Counters::now();

    expect_width_reject(&verdict, "post-AH one byte SHORT");
    assert_eq!(
        after.delta(&before).total,
        0,
        "O4: the width decision must reject BEFORE the aggregate pairing (C8) — a block \
         that cannot be the right shape must not cost a pairing to refuse"
    );
}

/// REQ-BLS-014 (Should) — Decision: a failure means a producer can pad the bitfield past
/// the universe and still be accepted. The pad is free bytes in every block forever, and
/// worse, it makes `bitfield.len()` stop being a witness of the committee size — the
/// decoders bound their scan by their OWN denominator, so two nodes with different
/// denominators read different attendance out of the same accepted block.
#[tokio::test]
async fn req_bls_014_m6_post_ah_a_one_byte_long_bitfield_is_rejected() {
    let _guard = counter_lock().await;
    let f = post_ah_fixture(N_SMALL, N_SIGNERS).await;
    let long = one_byte_long(&f);

    let w = width_of(&f).await;
    assert_eq!(
        long.attestation_bitfield.len(),
        exact(w) + 1,
        "fixture: the shape under test is exactly one byte long"
    );
    assert_eq!(
        long.attestation_bitfield.last().copied(),
        Some(0u8),
        "fixture: the pad byte must be ZERO, or the existing guard rejects it and this \
         test proves nothing new"
    );

    let before = Counters::now();
    let verdict = f.validate_full(&long).await;
    let after = Counters::now();

    expect_width_reject(&verdict, "post-AH one byte LONG");
    assert_eq!(
        after.delta(&before).total,
        0,
        "O4: the width decision must reject BEFORE the aggregate pairing (C8)"
    );
}

/// REQ-BLS-005 (Must) — Decision: THIS is the pre-AH identity half of R5. A failure means
/// the tightened rule reaches one block below its own activation height, which retroactively
/// invalidates stored history: a node replaying its own chain would refuse a block its peers
/// accepted, and rebuild-from-genesis would stop at that height. The assertion binds the
/// CURRENT verdict, deliberately: it must not change when M6 lands.
#[tokio::test]
async fn req_bls_014_m6_at_ah_minus_one_both_shapes_behave_exactly_as_today() {
    let _guard = counter_lock().await;
    let mut f = post_ah_fixture(N_SMALL, N_SIGNERS).await;

    // Move the gate one block ABOVE the fixture height: the same block, now pre-AH.
    let pre_ah_gate = f.height + 1;
    f.node.inc_i_178_attestation_bls_activation_height = pre_ah_gate;
    assert_eq!(
        f.height,
        pre_ah_gate - 1,
        "the fixture height must be exactly AH-1, read back out of the field the code reads"
    );

    let w = crate::inc_i_178_m5_common::universe_width(&f.node, f.height).await;
    for (label, mut block) in [
        ("one byte SHORT", one_byte_short(&f)),
        ("one byte LONG", one_byte_long(&f)),
    ] {
        // Pre-AH the commitment is BLAKE3(bitfield); `recommit` wrote the post-AH
        // preimage, so re-stamp the legacy root or the block dies on step 1 instead of
        // reaching the width guard.
        block.header.presence_root = crypto::hash::hash(&block.attestation_bitfield);

        let today = validate_attestation_bitfield_vec(&block.attestation_bitfield, w);
        assert!(
            today,
            "{label}: fixture premise — today's guard accepts this shape at width {w}"
        );

        let before = Counters::now();
        let verdict = f
            .node
            .validate_block_for_apply(&block, f.height, ValidationMode::Light)
            .await;
        let after = Counters::now();

        assert!(
            verdict.is_ok(),
            "{label}: below the gate this shape is ACCEPTED today and must stay accepted. \
             Got {:?}",
            err_text(&verdict)
        );
        assert_eq!(
            after.delta(&before).total,
            0,
            "{label}: the M5 verifier is inert below the gate, so no counter may move"
        );
    }
}
