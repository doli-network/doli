//! INC-I-176 **M2** — the `MAINTAINER_AUTH_VALID_BEFORE_UNSET` sentinel contract.
//!
//! Requirements: **REQ-176-021** (Must). The activation-height half is in
//! `crates/core/tests/inc_i_176_m2_activation_height.rs`; the production-wiring
//! half (REQ-176-022) is in `bins/node/tests/inc_i_176_m2_gate_wiring.rs`.
//!
//! Design decision (binding): `docs/.workflow/inc-i-176-M2-design-decision.md`
//! DECISION 1 — "what M2 passes for `valid_before`". Option (a), a NAMED SENTINEL.
//!
//! ---------------------------------------------------------------------------
//! TDD RED — EXPECTED, NOT A DEFECT
//! ---------------------------------------------------------------------------
//! This file does **NOT compile** against the tree at `3f8bf185`:
//! `doli_core::maintainer::MAINTAINER_AUTH_VALID_BEFORE_UNSET` does not exist
//! yet. That compile failure IS the RED evidence.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/maintainer/authmsg.rs
//! /// The `valid_before` M2 feeds to `signing_message_at` at the production
//! /// governance site, because `MaintainerChangeData` carries no expiry field
//! /// until M2.5.
//! ///
//! /// `u64::MAX` = "never expires" = EXACTLY today's unbounded semantics, so M2
//! /// cannot WEAKEN the expiry axis while it strictly improves the domain-tag
//! /// (AUDIT-P0-011) and chain-identity (AUDIT-P1-016) axes. `0` would read as
//! /// "already expired" under M3's `height >= valid_before` rule and would kill
//! /// governance above the gate.
//! pub const MAINTAINER_AUTH_VALID_BEFORE_UNSET: u64 = u64::MAX;
//! ```
//! It must ALSO be added to the `pub use authmsg::{..}` re-export list in
//! `crates/core/src/maintainer/mod.rs:72-76`, so the node crate can name it.
//!
//! ---------------------------------------------------------------------------
//! WHY THIS FILE EXISTS — what the sentinel is FOR
//! ---------------------------------------------------------------------------
//! The sentinel is a TRANSITIONAL value. M2.5 gives `valid_before` a real payload
//! field, and at that point the verifier's rule becomes
//! `payload.valid_before().unwrap_or(MAINTAINER_AUTH_VALID_BEFORE_UNSET)`, keyed
//! on an explicit payload version discriminator that lives IN THE TRANSACTION
//! BYTES — not on height. Under that rule a v1 payload re-validated by an M2.5
//! binary produces a BIT-IDENTICAL message to the one an M2 binary produced, so
//! M2.5 needs no second activation height for the MESSAGE FORM.
//!
//! That whole argument depends on one measurable fact, and this file is where it
//! is measured: **a real, bounded `valid_before` produces a DIFFERENT message
//! from the sentinel.** If it did not, the M2.5 payload field would be
//! unobservable in the signature and the "introduce it behind the discriminator"
//! discipline would be pointless. `req_176_021_the_sentinel_is_distinguishable_
//! from_every_bounded_window` is that measurement.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions / values under test:
//!   S1 `MAINTAINER_AUTH_VALID_BEFORE_UNSET: u64` (a compile-time constant)
//!   S2 `signing_message(&[u8], bool, &PublicKey, u64) -> Vec<u8>` driven at the
//!      sentinel
//!   S3 `signing_message_at(.., height, activation_height) -> Vec<u8>` driven at
//!      the sentinel (the arm M2 wires)
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1 the numeric value of S1.
//!   O2 the LENGTH of S2's return — must be exactly 32 (a BLAKE3-256 digest).
//!   O3 the CONTENT of S2's return — the only channel that can show the expiry
//!      term is bound.
//!   O4 the branch S3 selects, observed as byte equality with O3 (at/above the
//!      gate) or with `signing_message_legacy` (below it).
//!   mutable params    : NONE — shared refs and `Copy` scalars only.
//!   receiver mutation : NONE — free functions, and S1 is a constant.
//!   persistent store  : NONE. No I/O on any path in this file.
//!   side channels     : NONE. DECLARED UNASSERTED — nothing is logged here.
//!
//! CODE PATHS
//!   P-SENT  the sentinel value is passed as `valid_before`.
//!   P-BOUND a finite window is passed as `valid_before`.
//!   (There is no branch on `valid_before` inside `signing_message` — it is a
//!    straight-line encoder. The "paths" here are input partitions of one path,
//!    and are labelled as such rather than dressed up as branches.)
//!
//! INPUT PARTITIONS
//!   IP-S  `valid_before = MAINTAINER_AUTH_VALID_BEFORE_UNSET`  (= u64::MAX)
//!   IP-B0 `valid_before = 0`            — "already expired"; worst scenario #1
//!   IP-B1 `valid_before = 1`            — off-by-one partner of IP-B0
//!   IP-B2 `valid_before = 17_280`       — the operational INC-I-175 window
//!                                          (≈ 2 days at SLOT_DURATION = 10s)
//!   IP-B3 `valid_before = u64::MAX - 1` — the adjacent numeric boundary;
//!                                          worst scenario #3
//!   IP-A0 / IP-A1  `is_add` false / true — both arms of the effect bit
//!
//! MATRIX
//!   O1 x IP-S                                = 1 cell
//!   O2 x {IP-S, IP-B0..IP-B3} x {IP-A0,IP-A1} = 10 cells
//!   O3 x (IP-S vs each of IP-B0..IP-B3) x {IP-A0,IP-A1} = 8 cells
//!   O4 x IP-S x {below-gate, at-gate}         = 2 cells
//!
//! ANTI-VACUITY / POSITIVE CONTROL (project rule)
//!   `req_176_021_positive_control_identical_inputs_produce_identical_messages`
//!   proves the instrument can COME OUT THE OTHER WAY. Every `assert_ne!` above
//!   would also pass against a `signing_message` that returned fresh randomness
//!   on every call; the determinism control is what excludes that hypothesis.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO
//! ---------------------------------------------------------------------------
//! 1. It asserts nothing about a payload expiry FIELD. There is none in M2, and
//!    `MaintainerChangeData`'s bincode encoding stays byte-frozen — that is
//!    `inc_i_176_m1a_wire_freeze` / `inc_i_176_m1a_wire_decode`'s contract.
//! 2. It asserts no EXPIRY ENFORCEMENT. Nothing in M2 compares a height to
//!    `valid_before`; the non-fatal expiry check is M3's.
//! 3. It touches no activation-height literal — that is the sibling file.

use crypto::{KeyPair, PublicKey};
use doli_core::maintainer::{
    signing_message, signing_message_at, signing_message_legacy, MAINTAINER_AUTH_VALID_BEFORE_UNSET,
};

/// The published golden-vector genesis hash (`0x00..=0x1F`), reused from the
/// crate rather than re-typed: a third copy of the vector is a third thing that
/// can drift. M1a already binds the crate copy to the test copy.
use doli_core::maintainer::GOLDEN_AUTH_GENESIS_HASH;

/// The operational rotation window the INC-I-175 remediation names: 17_280 blocks
/// ≈ 2 days at `SLOT_DURATION = 10s`.
///
/// Chosen over a round number because its little-endian encoding
/// (`80 43 00 00 00 00 00 00`) is not a palindrome, so an LE↔BE swap inside the
/// preimage is detectable.
const BOUNDED_WINDOW: u64 = 17_280;

/// Deterministic real keypair — fixed seed, so every message in this file is
/// byte-stable across runs and a failure is reproducible from the source alone.
fn pk(seed: u8) -> PublicKey {
    *KeyPair::from_seed([seed; 32]).public_key()
}

// ===========================================================================
// O1 — the constant itself
// ===========================================================================

/// REQ-176-021 — O1 x IP-S: the sentinel is `u64::MAX`.
///
/// `u64::MAX` means "never expires", which is EXACTLY today's unbounded
/// semantics. That is the whole reason it was chosen: M2 must not be able to
/// WEAKEN the expiry axis while it strengthens the domain-tag and chain-identity
/// axes. A sentinel of `0` would read as "already expired" under M3's
/// `height >= valid_before` rule and would kill governance above the gate — the
/// exact liveness failure the design decision exists to avoid.
#[test]
fn req_176_021_sentinel_is_u64_max() {
    assert_eq!(
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        u64::MAX,
        "O1: MAINTAINER_AUTH_VALID_BEFORE_UNSET must be u64::MAX — 'never \
         expires', i.e. today's unbounded semantics preserved exactly. M2 \
         improves the domain-tag (AUDIT-P0-011) and chain-identity (AUDIT-P1-016) \
         axes; it must not regress the expiry axis (AUDIT-P1-004) in either \
         direction."
    );
    assert_ne!(
        MAINTAINER_AUTH_VALID_BEFORE_UNSET, 0,
        "O1: a sentinel of 0 reads as ALREADY EXPIRED under M3's \
         `height >= valid_before` rule and would make every maintainer change \
         above the gate unauthorizable — a governance lock-out, not a security fix"
    );
}

// ===========================================================================
// O2, O3 — the message the sentinel produces
// ===========================================================================

/// REQ-176-021 — O2 x IP-S: the sentinel yields a well-defined 32-byte digest.
///
/// Well-defined is not a given: the sentinel is the numeric extreme of the
/// `valid_before` term (worst scenario #3, numeric boundary), and it is written
/// into the preimage as 8 little-endian bytes. If anything in the encoder
/// saturated, truncated or short-circuited on `u64::MAX`, this is where it shows.
#[test]
fn req_176_021_sentinel_message_is_a_well_defined_32_byte_digest() {
    for is_add in [true, false] {
        let target = pk(0xA1);
        let msg = signing_message(
            &GOLDEN_AUTH_GENESIS_HASH,
            is_add,
            &target,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        );
        assert_eq!(
            msg.len(),
            32,
            "O2: the sentinel message must be a BLAKE3-256 digest — exactly 32 \
             bytes — for is_add={}",
            is_add
        );
        assert_ne!(
            msg,
            vec![0u8; 32],
            "O2: an all-zero digest is the signature of an encoder that never ran"
        );
    }
}

/// REQ-176-021 — O3: **the sentinel is distinguishable from every bounded
/// window.** This is what the whole M2.5 argument rests on.
///
/// If a real `valid_before` produced the SAME message as the sentinel, then the
/// M2.5 payload field would be invisible in the signature: an authorization
/// signed for a 2-day window would be interchangeable with one signed for
/// "forever", and the bounded-validity relaxation would be vacuous. Because they
/// DIFFER, M2.5's real values necessarily change the message and therefore MUST
/// be introduced behind the explicit payload version discriminator, exactly as
/// the design decision requires.
///
/// IP-B0..IP-B3 include both numeric extremes: `0` (worst scenario #1) and
/// `u64::MAX - 1`, the value adjacent to the sentinel — the hardest case, one
/// single bit away.
#[test]
fn req_176_021_the_sentinel_is_distinguishable_from_every_bounded_window() {
    let target = pk(0xA2);

    for is_add in [true, false] {
        let sentinel_msg = signing_message(
            &GOLDEN_AUTH_GENESIS_HASH,
            is_add,
            &target,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        );

        for window in [0u64, 1, BOUNDED_WINDOW, u64::MAX - 1] {
            let bounded_msg = signing_message(&GOLDEN_AUTH_GENESIS_HASH, is_add, &target, window);
            assert_ne!(
                sentinel_msg, bounded_msg,
                "O3: valid_before={} produced the SAME signed message as the \
                 sentinel (u64::MAX), for is_add={}. The expiry term is then NOT \
                 bound, so M2.5's real values would be unobservable in the \
                 signature and an authorization for a bounded window would be \
                 interchangeable with one that never expires.",
                window, is_add
            );
        }
    }
}

/// REQ-176-021 — O3: the sentinel message is not the legacy message.
///
/// If the sentinel arm degenerated into `signing_message_legacy`, the gate would
/// be a no-op: nothing would change at #22, AUDIT-P0-011 would stay open above it,
/// and every test that "passes" below AND above the gate would be passing
/// vacuously.
#[test]
fn req_176_021_the_sentinel_message_is_not_the_legacy_message() {
    let target = pk(0xA3);
    for is_add in [true, false] {
        assert_ne!(
            signing_message(
                &GOLDEN_AUTH_GENESIS_HASH,
                is_add,
                &target,
                MAINTAINER_AUTH_VALID_BEFORE_UNSET
            ),
            signing_message_legacy(is_add, &target),
            "O3: the sentinel arm must not degenerate into the chain-blind, \
             domain-tag-less legacy message for is_add={} — if it did, gate #22 \
             would change nothing",
            is_add
        );
    }
}

// ===========================================================================
// O4 — the sentinel through the dispatcher M2 wires
// ===========================================================================

/// REQ-176-021 — O4 x IP-S: the sentinel travels through `signing_message_at`
/// unchanged, on both sides of a gate.
///
/// M2's production call is
/// `signing_message_at(genesis, is_add, target, MAINTAINER_AUTH_VALID_BEFORE_UNSET,
///  height, activation_height)`. This is the unit-level statement of what
/// `bins/node/tests/inc_i_176_m2_gate_wiring.rs` then proves at the production
/// site: below the gate the `valid_before` argument is IGNORED (legacy bytes carry
/// no expiry at all), and at/above it the sentinel is what gets hashed.
#[test]
fn req_176_021_the_sentinel_flows_through_the_height_dispatcher() {
    let target = pk(0xA4);
    let ah = 300_000u64;

    for is_add in [true, false] {
        let legacy = signing_message_legacy(is_add, &target);
        let bound = signing_message(
            &GOLDEN_AUTH_GENESIS_HASH,
            is_add,
            &target,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        );

        assert_eq!(
            signing_message_at(
                &GOLDEN_AUTH_GENESIS_HASH,
                is_add,
                &target,
                MAINTAINER_AUTH_VALID_BEFORE_UNSET,
                ah - 1,
                ah
            ),
            legacy,
            "O4: one block BELOW the gate the dispatcher must return EXACTLY the \
             legacy bytes — the sentinel is not observable there, because the \
             legacy format has no expiry term at all"
        );
        assert_eq!(
            signing_message_at(
                &GOLDEN_AUTH_GENESIS_HASH,
                is_add,
                &target,
                MAINTAINER_AUTH_VALID_BEFORE_UNSET,
                ah,
                ah
            ),
            bound,
            "O4: AT the gate (the comparison is `>=`, matching set.rs:269) the \
             dispatcher must return EXACTLY the sentinel-bound digest"
        );
    }
}

// ===========================================================================
// POSITIVE CONTROL — the instrument can come out the other way
// ===========================================================================

/// **POSITIVE CONTROL.** Identical inputs must produce identical messages.
///
/// Every `assert_ne!` in this file would also pass against a `signing_message`
/// that returned fresh random bytes on each call, or that mixed in a timestamp,
/// an address or an allocation-order-dependent value. Those hypotheses have to be
/// excluded before an inequality can mean "the term is bound".
///
/// This control also covers the mirror-image bug: a `signing_message` that always
/// differs would make the below-gate byte-identity assertions in
/// `bins/node/tests/inc_i_176_m2_gate_wiring.rs` fail for a reason that has
/// nothing to do with the gate.
#[test]
fn req_176_021_positive_control_identical_inputs_produce_identical_messages() {
    let target = pk(0xA5);

    let a = signing_message(
        &GOLDEN_AUTH_GENESIS_HASH,
        true,
        &target,
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
    );
    let b = signing_message(
        &GOLDEN_AUTH_GENESIS_HASH,
        true,
        &target,
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
    );
    assert_eq!(
        a, b,
        "POSITIVE CONTROL: two calls with the SAME four inputs must produce the \
         SAME message. If this fails, every assert_ne! in this file is passing for \
         the wrong reason and proves nothing about term binding."
    );

    // And the same control for the legacy arm, which the wiring test compares
    // byte-for-byte below the gate.
    assert_eq!(
        signing_message_legacy(true, &target),
        signing_message_legacy(true, &target),
        "POSITIVE CONTROL: the legacy message must also be deterministic"
    );

    // A different target must still move the digest — the control must not be
    // satisfiable by a constant function either.
    let other = pk(0xA6);
    assert_ne!(
        target, other,
        "fixture: the two targets must actually differ"
    );
    assert_ne!(
        a,
        signing_message(
            &GOLDEN_AUTH_GENESIS_HASH,
            true,
            &other,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET
        ),
        "POSITIVE CONTROL (other direction): a constant function would satisfy the \
         determinism check above; it must still fail this one"
    );
}
