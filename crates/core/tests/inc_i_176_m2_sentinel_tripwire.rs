//! INC-I-176 **M2** reviewer finding **F9** — tripwires for the **v1 -> sentinel
//! mapping**, the one load-bearing claim that lets gate `#22` serve M2 *and* M2.5
//! with a single activation height.
//!
//! Requirements: **REQ-176-021** (Must, the sentinel + its own forward-only gate)
//! and **REQ-176-022** (Must, no version bump / no genesis reset / no second
//! gate). Spec: `specs/maintainer-authorization-architecture.md`, section
//! **"M2 RESOLUTION — the `valid_before` sentinel. ACCEPTED 2026-08-13. BINDING
//! ON M2.5."** (`:887-973`) — in particular "BINDING OBLIGATIONS ON M2.5" `:936`.
//!
//! # THIS FILE ENCODES A BINDING OBLIGATION ON M2.5
//!
//! Said plainly, because prose has already proven insufficient here: the rule below
//! is not advice and not a note-to-self. It is a contract M2 signed on M2.5's behalf
//! to avoid taking a second activation height, and this file is the instrument that
//! holds M2.5 to it.
//!
//! # The claim under test
//!
//! M2 wired the bound message constructor into production behind
//! `inc_i_176_auth_binding_activation_height` (`#22`) while [`MaintainerChangeData`]
//! still has **no** `valid_before` field, so M2 feeds
//! [`MAINTAINER_AUTH_VALID_BEFORE_UNSET`] instead. The obvious objection: when M2.5
//! gives the payload a real expiry field, does the message change a SECOND time, and
//! does that need a second gate? The accepted answer is **no**, resting on exactly
//! one mapping:
//!
//! ```text
//! valid_before = payload.valid_before().unwrap_or(MAINTAINER_AUTH_VALID_BEFORE_UNSET)
//! ```
//!
//! keyed on M2.5's EXPLICIT payload version discriminator, which lives **in the
//! transaction bytes** — not on height. Every `AddMaintainer` / `RemoveMaintainer`
//! payload written into any block above `#22` before M2.5 ships is a **v1**
//! (frozen-shape) payload; v1 has no `valid_before` field, so it maps to the
//! sentinel; so an M2.5 binary re-validating such a block recomputes a
//! **bit-identical** 32-byte message. Historical re-validation is unchanged,
//! therefore **no second activation height is needed for the message form**. (M2.5
//! still needs a gate for EMISSION of the v2 wire shape — an old binary hard-rejects
//! an unknown shape at the pre-existing, height-ungated fatal decode in
//! `crates/core/src/validation/tx_types.rs` — and it needed that regardless of M2.)
//!
//! # Why this file exists
//!
//! That obligation was **prose-only**. The structurally identical risk in
//! `derivation.rs` — a premise that is a *fact about the tree* rather than an
//! invariant — got an executable tripwire in this same milestone
//! (`crates/core/tests/inc_i_176_m2_derivation_tripwire.rs`). This one did not. F9 is
//! that asymmetry: a comment cannot fail a build, and the moment M2.5 lands nothing
//! in the tree would have objected to `unwrap_or_default()`. Mirrors that file in
//! style, rigor and structure, including its POSITIVE CONTROL discipline.
//!
//! Scope note: a green run proves the M2 mapping ARGUMENT is still standing, NOT that
//! M2.5 was implemented correctly — when the payload grows the field,
//! `tripwire_maintainer_change_data_is_still_v1` FAILS **by design**, and its message
//! is the implementation guide, not a nuisance to silence.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Two units, in one file because neither half is meaningful alone: a VALUE unit
//! ([`MAINTAINER_AUTH_VALID_BEFORE_UNSET`] and the message functions driven at
//! it) and a STRUCTURAL unit (the field list of [`MaintainerChangeData`], which
//! is the PREMISE the value half rests on).
//!
//! Properties under test:
//!   G1: the numeric identity of the sentinel constant
//!   G2: the sentinel is not the type's `Default`, i.e. `unwrap_or_default()` is
//!       NOT a legal spelling of the mapping
//!   G3: [`MaintainerChangeData`] has no `valid_before` field (the v1 premise)
//!   G4: for a v1 payload, the message the production path builds at and above
//!       `#22` is bit-identical to
//!       `signing_message(genesis, is_add, target, MAINTAINER_AUTH_VALID_BEFORE_UNSET)`,
//!       and equals a PINNED byte string
//!
//! OUTPUTS
//!   O1 (value)      the `u64` value of the sentinel. Subject for G1 and G2.
//!   O2 (derived)    `signing_message(..)` at the sentinel vs at `u64::default()` —
//!                   the channel that makes G2 observable, not merely numeric.
//!   O3 (derived)    brace-matched body of `pub struct MaintainerChangeData`.
//!   O4 (derived)    the ordered field-name list from O3. Subject for G3.
//!   O5 (derived)    top-level key set of `serde_json::to_value(&payload)` — a
//!                   SECOND, source-text-independent witness for G3, read through the
//!                   same `Serialize` impl bincode drives for the wire.
//!   O6 (instrument) the same extractor over the SIBLING struct
//!                   `ProtocolActivationData` in the same file. POSITIVE CONTROL: it
//!                   must FIND fields — including a bare `u64` — when they exist.
//!   O7 (derived)    `signing_message_preimage(..)` at the sentinel, as hex.
//!   O8 (derived)    `signing_message_at(.., height, #22)` for `height >= #22`, as
//!                   bytes and as hex. Subject for G4.
//!   O9 (instrument) the extractor over a SYNTHETIC source that DOES declare
//!                   `valid_before`. POSITIVE CONTROL for the token G3 needs.
//!   mutable params    — NONE. Shared refs and `Copy` scalars only.
//!   receiver mutation — NONE. Free functions; O1 is a compile-time constant.
//!   persistent store  — NONE. Every path in this file only READS files.
//!   side channels     — NONE. DECLARED UNASSERTED; nothing here logs.
//!
//! PATHS
//!   PT-clean  — the mapping premise holds; the tripwires stay green
//!   PT-m25    — M2.5 landed (the payload grew the field). MUST fail, with the guide
//!   PT-broken — the instrument stopped working (struct renamed/moved, extraction
//!               empty, dead control). MUST fail, never pass vacuously
//!
//! INPUT PARTITIONS
//!   IP-V1  `valid_before = SENTINEL` (= `u64::MAX`) — what v1 maps to  [PT-clean]
//!   IP-V0  `valid_before = u64::default()` (= 0) — what `unwrap_or_default()`
//!          yields; "already expired" under M3's `height >= valid_before` [PT-m25]
//!   IP-VN  `valid_before = u64::MAX - 1` — adjacent boundary, one bit from the
//!          sentinel; anti-vacuity for the pinned vector
//!   IP-A0/A1  `is_add` false/true — both production arms (`governance.rs:97` add,
//!          `:157` remove)
//!   IP-H0/H1/H2  `height` = `#22` (AT the gate; comparison is `>=`), `#22 + 1`,
//!          `u64::MAX` (the far end of "any block above `#22`")
//!   IP-F1  the payload declares exactly `{target, signatures, reason}` [PT-clean]
//!   IP-F2  it declares `valid_before` (or any 4th field)               [PT-m25]
//!   IP-F3  extractor degeneracy: struct not found, empty list, dead ctl [PT-broken]
//!
//! MATRIX
//!   O1 x {IP-V1} = 1 (G1);  O1 x {IP-V0} = 1 (G2, numeric)
//!   O2 x {IP-V1 vs IP-V0} x {IP-A0,A1} = 2 (G2, observable)
//!   O3 x {IP-F3} = 1 (extraction sanity);  O6 x {IP-F3} = 1 (positive control)
//!   O4 x {IP-F1, IP-F2} = 2 (G3);  O5 x {IP-F1, IP-F2} = 2 (G3, second witness)
//!   O7 x {IP-V1} x {IP-A0,A1} = 2 (G4, preimage pin)
//!   O8 x {IP-V1} x {IP-A0,A1} x {IP-H0,H1,H2} = 6 cells + 2 digest pins
//!   O9 x {IP-F2} = 1 (positive control)
//!
//! ANTI-VACUITY
//!   O6 mirrors the derivation tripwire's O4 and is the strong instrument: the
//!   SIBLING struct lives in the SAME file, is read by the SAME extractor, and one of
//!   its four fields (`activation_epoch`) is a bare `u64` — exactly the SHAPE a future
//!   `valid_before` would take. If the extractor still sees those, an absent
//!   `valid_before` is a fact about the TREE, not the extractor. O9 sharpens that to
//!   the literal token; O5 bypasses source text entirely, so a scanner regression
//!   cannot retire G3 alone; and the positive-control test excludes the hypothesis
//!   that `signing_message` ignores `valid_before`, which would make every G4
//!   equality true for the wrong reason.
//!
//! WHAT THIS FILE DOES NOT DO. (1) It pins no activation-height VALUE — mainnet
//! `u64::MAX`, testnet `300_000`, devnet `20` are the sibling file's contract
//! (`crates/core/tests/inc_i_176_m2_activation_height.rs`); every height here is a
//! local fixture. (2) It asserts no EXPIRY ENFORCEMENT — the `height >= valid_before`
//! rule is M3's. (3) It does not re-test the sentinel's value semantics against
//! bounded windows — that is `crates/core/tests/inc_i_176_m2_sentinel.rs`. This file
//! is about the MAPPING and its premise.

mod inc_i_176_m2_sentinel_tripwire_common;
use inc_i_176_m2_sentinel_tripwire_common::*;

// ===========================================================================
// G1 / G2 — the sentinel constant
// ===========================================================================

/// **REQ-176-021 — O1 x IP-V1.** The sentinel is still `u64::MAX`.
///
/// The narrowest and most consequential assertion in the file. The sentinel is not a
/// tuning parameter: it is HASHED INTO every authorization message ever produced
/// above gate `#22`, so changing it silently changes all of their meanings and would
/// compile, pass clippy, and split the network at the next replay. `u64::MAX` also
/// means *never expires* — EXACTLY today's unbounded semantics — so M2 cannot WEAKEN
/// the expiry axis (AUDIT-P1-004) while it strengthens the domain-tag (AUDIT-P0-011)
/// and chain-identity (AUDIT-P1-016) axes.
#[test]
fn req_176_021_tripwire_the_sentinel_constant_is_still_u64_max() {
    assert_eq!(
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        u64::MAX,
        "INC-I-176 M2 / F9 TRIPWIRE (G1): MAINTAINER_AUTH_VALID_BEFORE_UNSET is no \
         longer u64::MAX. It is hashed into EVERY maintainer-authorization message \
         produced at or above `inc_i_176_auth_binding_activation_height` (#22), so \
         changing it re-writes the meaning of all of them retroactively: signatures \
         the fleet accepted are rejected on replay, and nodes that disagree about the \
         value hold different maintainer trust roots. It is consensus history, not a \
         tunable — and it is ALSO the value a v1 payload maps to under the M2.5 rule, \
         so a change here breaks the no-second-gate argument at the root.\n\n{}",
        M25_GUIDE
    );
}

/// **REQ-176-021 — O1 x IP-V0, O2 x {IP-V1 vs IP-V0}.** The sentinel is NOT the
/// type's `Default`, and the difference is observable in the signed bytes.
///
/// This prevents one specific wrong turn, worth naming exactly: writing the M2.5
/// mapping as `unwrap_or_default()` instead of `unwrap_or(SENTINEL)`. The two
/// spellings look interchangeable and are both idiomatic Rust. They are not.
///
/// `u64::default()` is `0`, and under M3's `height >= valid_before` rule `0` reads as
/// **already expired** — for every authorization, at every height. The wrong spelling
/// therefore costs BOTH: **history** (a v1 payload above `#22` re-validates against a
/// different message, so a previously-accepted authorization is rejected) and
/// **liveness** (no maintainer change above the gate can ever be authorized again — a
/// lock-out, strictly worse than the audit finding the milestone set out to close).
///
/// The numeric assertion alone would be trivially true; the message-level assertion
/// gives it teeth, showing the two spellings produce different signed BYTES and not
/// merely different numbers.
#[test]
fn req_176_021_tripwire_the_sentinel_is_not_the_unwrap_or_default_value() {
    assert_eq!(
        u64::default(),
        0,
        "fixture: this test's whole point is that `unwrap_or_default()` yields 0. If \
         u64::default() stopped being 0, re-derive the argument first."
    );

    assert_ne!(
        MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        u64::default(),
        "INC-I-176 M2 / F9 TRIPWIRE (G2): the sentinel now EQUALS u64::default(), \
         so `unwrap_or_default()` and `unwrap_or(MAINTAINER_AUTH_VALID_BEFORE_UNSET)` \
         became indistinguishable — and the sentinel's value is 0, which M3's \
         `height >= valid_before` rule reads as ALREADY EXPIRED. Every maintainer \
         change above gate #22 would be unauthorizable.\n\n{}",
        M25_GUIDE
    );

    // O2 — the same statement, made observable in the bytes that are actually
    // signed. Without this the test above is just arithmetic.
    let target = golden_target();
    for is_add in [true, false] {
        let sentinel_msg = signing_message(
            &GOLDEN_AUTH_GENESIS_HASH,
            is_add,
            &target,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        );
        let default_msg =
            signing_message(&GOLDEN_AUTH_GENESIS_HASH, is_add, &target, u64::default());
        assert_ne!(
            sentinel_msg, default_msg,
            "INC-I-176 M2 / F9 TRIPWIRE (G2, observable): for is_add={is_add} the \
             sentinel and u64::default() produce the SAME signed message. Either \
             the expiry term stopped being bound into the preimage — in which case \
             M2.5's real `valid_before` values would be invisible in the signature \
             and the whole bounded-validity design is vacuous — or the sentinel \
             changed. Both are stop conditions.\n\n{}",
            M25_GUIDE
        );
    }
}
