//! INC-I-176 M2 — `valid_before` sentinel tripwire, PAYLOAD-SHAPE half.
//!
//! Split out of `inc_i_176_m2_sentinel_tripwire.rs` to respect the 800-line
//! test-file budget (CLAUDE.md rule 19). The assertions, their names and their
//! failure messages are byte-identical to the single-file version.
//!
//! This half pins the premise the M2 no-second-gate argument rests on: that a
//! v1 payload has no `valid_before` field and therefore maps to
//! `MAINTAINER_AUTH_VALID_BEFORE_UNSET`, so an M2.5 binary re-validating any
//! block above AH #22 recomputes a BIT-IDENTICAL message. It carries the
//! positive control for the pinned vector.
//!
//! Binding record: `specs/maintainer-authorization-architecture.md`, section
//! "M2 RESOLUTION — the `valid_before` sentinel".
//!
//! Requirements: REQ-176-021, REQ-176-022.

mod inc_i_176_m2_sentinel_tripwire_common;
use inc_i_176_m2_sentinel_tripwire_common::*;

// ===========================================================================
// G3 — the premise: MaintainerChangeData is still v1
// ===========================================================================

/// **REQ-176-021 — O3, O4, O5, O6 x {IP-F1, IP-F2, IP-F3}.** The payload still has
/// NO `valid_before` field.
///
/// # This failure is a MILESTONE, not a bug
///
/// The v1→sentinel mapping is only meaningful while "v1" is what the tree actually
/// declares. When M2.5 adds the field, this test is **supposed** to fire, so its
/// message is written as an implementation guide: the exact expression to write,
/// the exact expression never to write, and what each costs.
///
/// # Why the field list is checked EXACTLY, not just for `valid_before`
///
/// `MaintainerChangeData`'s bincode encoding is FROZEN: `from_bytes` is consumed
/// fatally and WITHOUT a height gate at
/// `crates/core/src/validation/tx_types.rs::validate_maintainer_change_data`, and a
/// real payload of this shape is already in testnet history (block 136_690).
/// Bincode writes fields positionally with no names, so a REORDER is as damaging as
/// an addition and considerably quieter — the old block would decode into different
/// values instead of failing. The ordered list catches both.
///
/// Two independent witnesses, because one source-text scanner is a single point of
/// failure: O4 reads the declaration, O5 reads the field set back through
/// `Serialize` — the impl that produces the wire bytes.
#[test]
fn req_176_021_tripwire_maintainer_change_data_is_still_v1() {
    let root = repo_root();
    let path = root.join(DATA_RS);
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "instrument is broken: cannot read {} ({e}). Do NOT silence this — the \
             v1→sentinel mapping argument depends on this file's contents. If the \
             payload moved, update DATA_RS and re-derive the argument.",
            path.display()
        )
    });

    // --- O6 / IP-F3 — POSITIVE CONTROL, taken FIRST and from the SAME source
    // string the property is computed from. The sibling struct in this same file
    // has four fields, one of them a bare `u64` (`activation_epoch`) — the exact
    // shape a future `valid_before` would take. If the extractor cannot see those,
    // an absent `valid_before` below proves nothing about the tree.
    let control_body = extract_struct_body(&src, CONTROL_STRUCT).unwrap_or_else(|| {
        panic!(
            "POSITIVE CONTROL FAILED: `pub struct {CONTROL_STRUCT}` was not found in \
             {DATA_RS}. The extractor cannot locate a struct it is known to contain, \
             so its result for MaintainerChangeData is a fact about the EXTRACTOR, \
             not about the tree. Fix the extractor — or, if the sibling genuinely \
             moved, choose a new control before trusting any result here."
        )
    });
    let control_fields = struct_field_names(&control_body);
    assert_eq!(
        control_fields, CONTROL_FIELDS,
        "POSITIVE CONTROL FAILED: the extractor read {CONTROL_STRUCT}'s fields as \
         {control_fields:?}, expected {CONTROL_FIELDS:?}. One of the expected fields \
         (`activation_epoch`) is a bare `u64`, i.e. exactly the declaration shape a \
         future `valid_before` would have. With the control dead, an empty \
         `valid_before` result below is a statement about the scanner. Fix the \
         scanner first."
    );

    // --- O3 / IP-F3 — extraction sanity for the struct actually under test.
    let body = extract_struct_body(&src, "MaintainerChangeData").unwrap_or_else(|| {
        panic!(
            "instrument is broken: `pub struct MaintainerChangeData` not found in \
             {DATA_RS}. A missing declaration must FAIL here rather than yield an \
             empty field list that looks green. If the type moved, update DATA_RS."
        )
    });

    // --- O4 / IP-F1, IP-F2 — the property.
    let fields = struct_field_names(&body);
    assert!(
        !fields.is_empty(),
        "instrument is broken: extracted MaintainerChangeData body yielded ZERO \
         fields. A mis-extraction would trivially satisfy the `valid_before` check \
         below. Body was:\n{body}"
    );

    assert!(
        !fields.iter().any(|f| f == "valid_before"),
        "INC-I-176 M2 / F9 TRIPWIRE (G3): `MaintainerChangeData` now declares a \
         `valid_before` field. Fields are {fields:?}.\n\n\
         This is the M2.5 wire change, and firing here is this test's PURPOSE — the \
         v1→sentinel mapping argument that let gate #22 serve both milestones only \
         holds while the payload is v1.\n\n{}",
        M25_GUIDE
    );

    assert_eq!(
        fields, V1_FIELDS,
        "INC-I-176 M2 / F9 TRIPWIRE (G3): `MaintainerChangeData`'s field list changed \
         to {fields:?}; the frozen v1 shape is {V1_FIELDS:?}. ORDER is part of this \
         assertion, not decoration: bincode writes fields POSITIONALLY with no names \
         on the wire, so a reorder makes the payload already in testnet history at \
         block 136_690 decode into DIFFERENT VALUES rather than fail loudly. \
         `from_bytes` is consumed fatally and WITHOUT a height gate in \
         `crates/core/src/validation/tx_types.rs`, so any shape change also stops \
         every node re-validating that block — in both deploy directions, and a \
         synchronized deploy does not repair it.\n\n{}",
        M25_GUIDE
    );

    // --- O5 / IP-F1, IP-F2 — SECOND WITNESS, independent of source text. Read the
    // field set back through `Serialize`, the same impl bincode drives for the wire.
    // A regression in the source scanner cannot retire G3 on its own.
    let value = serde_json::to_value(v1_payload())
        .expect("instrument is broken: MaintainerChangeData must serialize");
    let object = value.as_object().unwrap_or_else(|| {
        panic!(
            "instrument is broken: MaintainerChangeData serialized as {value:?}, not \
             a JSON object, so its field names cannot be read back."
        )
    });
    let mut serde_fields: Vec<&str> = object.keys().map(String::as_str).collect();
    serde_fields.sort_unstable();
    let mut expected_sorted: Vec<&str> = V1_FIELDS.to_vec();
    expected_sorted.sort_unstable();

    assert_eq!(
        serde_fields, expected_sorted,
        "INC-I-176 M2 / F9 TRIPWIRE (G3, second witness): the SERIALIZED field set of \
         MaintainerChangeData is {serde_fields:?}, expected {expected_sorted:?}. This \
         witness reads the type through the same `Serialize` impl that produces the \
         frozen wire bytes; it disagreeing with the source scan means one of the two \
         instruments is wrong — resolve that first.\n\n{}",
        M25_GUIDE
    );
}

// ===========================================================================
// G4 — the bit-identity property, with the bytes pinned
// ===========================================================================

/// **REQ-176-021 / REQ-176-022 — O7, O8 x IP-V1 x {IP-A0, IP-A1} x {IP-H0..IP-H2}.**
/// For a v1 payload, the production message at and above `#22` IS the sentinel
/// message — and these are its exact bytes.
///
/// This is the property the whole M2 architecture argument reduces to. Stated as
/// the spec states it: *"an M2.5 binary re-validating those blocks recomputes a
/// bit-identical message; historical re-validation is unchanged ⇒ no second gate
/// for the message form."*
///
/// Three assertions, in increasing strength:
///
/// 1. The production call path — the `unwrap_or(MAINTAINER_AUTH_VALID_BEFORE_UNSET)`
///    mapping feeding `signing_message_at`, exactly as
///    `bins/node/src/node/apply_block/governance.rs:97` and `:157` do it — produces
///    the same bytes as `signing_message(genesis, is_add, target, SENTINEL)`
///    directly. That is the equality the argument names.
/// 2. It does so at EVERY height at or above the gate, including `u64::MAX`: "any
///    block above `#22`" is an unbounded claim and is tested as one.
/// 3. The bytes equal a PIN derived outside this repository. Without it, (1) and (2)
///    would still hold if `signing_message` moved on BOTH sides of the comparison at
///    once. The pin is what makes such a change DETECTABLE rather than merely
///    self-consistent.
///
/// The preimage is pinned as well as the digest because a preimage is READABLE: an
/// out-of-repo signer can be diffed against it by eye, whereas two differing 32-byte
/// digests tell nobody WHICH field moved.
#[test]
fn req_176_021_tripwire_v1_payload_message_is_bit_identical_to_the_sentinel_message() {
    let payload = v1_payload();
    let gate = 300_000u64; // local fixture; pins no network's activation height

    // The premise, restated as an assertion so a green run cannot rest on a
    // silently-changed helper.
    assert!(
        v1_payload_valid_before(&payload).is_none(),
        "INC-I-176 M2 / F9 TRIPWIRE (G4 premise): `v1_payload_valid_before` no \
         longer returns None. The v1→sentinel mapping is only meaningful while a v1 \
         payload HAS no expiry field; see \
         `req_176_021_tripwire_maintainer_change_data_is_still_v1`.\n\n{}",
        M25_GUIDE
    );

    for (is_add, pinned_preimage, pinned_digest) in [
        (
            true,
            PINNED_SENTINEL_PREIMAGE_ADD_HEX,
            PINNED_SENTINEL_DIGEST_ADD_HEX,
        ),
        (
            false,
            PINNED_SENTINEL_PREIMAGE_REMOVE_HEX,
            PINNED_SENTINEL_DIGEST_REMOVE_HEX,
        ),
    ] {
        // O7 — the readable half of the pin.
        let preimage = signing_message_preimage(
            &GOLDEN_AUTH_GENESIS_HASH,
            is_add,
            &payload.target,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        );
        assert_eq!(
            hex::encode(&preimage),
            pinned_preimage,
            "INC-I-176 M2 / F9 TRIPWIRE (G4, preimage): the sentinel preimage for \
             is_add={is_add} moved. Read the pin in five groups — domain 25 | genesis \
             32 | action 1 | target 32 | expiry 8 — and the group that differs names \
             the defect: a reordered field, a changed width, a big-endian expiry, an \
             ASCII action byte, a dropped or altered domain tag. Every authorization \
             ever signed above gate #22 was signed over bytes of THIS shape; moving \
             them re-writes consensus history.\n\n{}",
            M25_GUIDE
        );

        // O8 — the message the production path builds, at three heights at or above
        // the gate. `u64::MAX` is included because "any block above #22" is an
        // unbounded claim.
        let direct = signing_message(
            &GOLDEN_AUTH_GENESIS_HASH,
            is_add,
            &payload.target,
            MAINTAINER_AUTH_VALID_BEFORE_UNSET,
        );

        for height in [gate, gate + 1, u64::MAX] {
            let produced = production_message_for_v1_payload(
                &GOLDEN_AUTH_GENESIS_HASH,
                is_add,
                &payload,
                height,
                gate,
            );

            assert_eq!(
                produced, direct,
                "INC-I-176 M2 / F9 TRIPWIRE (G4): at height={height} (gate={gate}, \
                 is_add={is_add}) the production message for a v1 payload is NOT \
                 `signing_message(genesis, is_add, target, SENTINEL)`.\n\n\
                 That equality IS the reason gate #22 needs no partner gate at M2.5. \
                 Broken, an M2.5 binary re-validating a v1 payload above #22 computes \
                 a DIFFERENT message from the one the M2 binary accepted: a \
                 previously-valid authorization is rejected and the two binaries hold \
                 different maintainer trust roots — fragmentation arriving silently, \
                 because this verifier only logs. If the message form must change it \
                 needs its OWN height (#23), pinned while UNCROSSED (INV-PARAMS-001 / \
                 INC-I-054). Do not relax this assertion.\n\n{}",
                M25_GUIDE
            );

            assert_eq!(
                hex::encode(&produced),
                pinned_digest,
                "INC-I-176 M2 / F9 TRIPWIRE (G4, pinned bytes): at height={height} \
                 (is_add={is_add}) the v1-payload message no longer equals the \
                 externally-derived pin. The equality above can stay true through a \
                 refactor that moves BOTH sides at once; this pin cannot. It was \
                 computed OUTSIDE this repository and cross-checked against the \
                 crate's published GOLDEN_AUTH_DIGEST_HEX before use. Do NOT \
                 regenerate it from `authmsg.rs` output — that destroys the only \
                 detector for a field reorder, a width change or a re-encoding.\n\n{}",
                M25_GUIDE
            );
        }

        assert_eq!(
            direct.len(),
            32,
            "INC-I-176 M2 / F9 TRIPWIRE (G4): the sentinel message must be a \
             BLAKE3-256 digest — exactly 32 bytes — for is_add={is_add}"
        );
    }
}

// ===========================================================================
// POSITIVE CONTROL — the instruments can come out the other way
// ===========================================================================

/// **POSITIVE CONTROL.** Every instrument in this file must be able to FAIL.
///
/// Mirrors the derivation tripwire's control discipline — a tripwire that cannot
/// produce a negative result is decoration — and excludes three hypotheses:
///
/// * **O9, the field extractor.** A green `valid_before` result in
///   `req_176_021_tripwire_maintainer_change_data_is_still_v1` means nothing if the
///   extractor cannot see that token at all. Fed a synthetic declaration that DOES
///   contain it, in the awkward-but-legal `pub(crate)` form, it must report it.
/// * **the pinned vector.** Every G4 equality would also hold against a
///   `signing_message` that IGNORED `valid_before` — the sentinel would simply never
///   be observable. A value one bit from the sentinel must therefore MISS the pin.
/// * **determinism.** The equalities need the mirror-image control: identical inputs
///   give identical bytes, and a different input still moves them.
#[test]
fn req_176_021_positive_control_the_pinned_vector_can_come_out_the_other_way() {
    // --- O9: the extractor CAN see `valid_before`, including behind a
    // non-trivial visibility token.
    let synthetic = "\
/// A synthetic v2-shaped payload. Never compiled; only ever read as text.\n\
#[derive(Clone, Debug)]\n\
pub struct SyntheticV2Payload {\n\
    /// valid_before mentioned in a doc comment must NOT count as a field.\n\
    pub target: PublicKey,\n\
    pub(crate) valid_before: u64,\n\
    pub reason: Option<String>,\n\
}\n";
    let synthetic_body = extract_struct_body(synthetic, "SyntheticV2Payload").expect(
        "POSITIVE CONTROL FAILED: the extractor could not locate a struct in a \
                 synthetic source it is given verbatim",
    );
    let synthetic_fields = struct_field_names(&synthetic_body);
    assert_eq!(
        synthetic_fields,
        vec!["target", "valid_before", "reason"],
        "POSITIVE CONTROL FAILED: fed a declaration that DOES contain \
         `pub(crate) valid_before: u64`, the extractor reported {synthetic_fields:?}. \
         It must (a) SEE the field, (b) see it through a `pub(crate)` visibility \
         token, and (c) NOT be fooled by the word appearing in a doc comment. With \
         this control dead, `tripwire_maintainer_change_data_is_still_v1` passing is \
         a fact about the extractor and not about the payload."
    );

    // --- the pinned vector: a value ONE BIT from the sentinel must miss the pin. If
    // `signing_message` ignored `valid_before`, every G4 equality would still hold
    // and this is the only assertion that would notice.
    //
    // NEAR_MISS is the LITERAL `u64::MAX - 1`, never `SENTINEL - 1`, and that is a
    // lesson from this file's own mutation campaign: written in terms of the
    // constant, mutating the sentinel to `0` turns it into a compile-time overflow,
    // so the whole test TARGET fails to build and not one tripwire above ever prints
    // its guide. A tripwire silenced by the very mutation it exists to catch is
    // worthless. Pinned independently, it works under any sentinel value; the
    // sentinel's required value is asserted by
    // `req_176_021_tripwire_the_sentinel_constant_is_still_u64_max`.
    const NEAR_MISS: u64 = u64::MAX - 1;

    let target = golden_target();
    let near_miss = signing_message(&GOLDEN_AUTH_GENESIS_HASH, true, &target, NEAR_MISS);
    assert_ne!(
        hex::encode(&near_miss),
        PINNED_SENTINEL_DIGEST_ADD_HEX,
        "POSITIVE CONTROL FAILED: valid_before = u64::MAX - 1 produced the PINNED \
         SENTINEL digest. The expiry term is then not bound into the message at all, \
         which makes every equality in \
         `req_176_021_tripwire_v1_payload_message_is_bit_identical_to_the_sentinel_message` \
         true for the wrong reason — and makes M2.5's real `valid_before` values \
         invisible in the signature."
    );

    // --- determinism, both directions. Excludes "the message is random" (which
    // would break the equalities for reasons unrelated to the mapping) and
    // "the message is constant" (which would satisfy the near-miss check above).
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
        "POSITIVE CONTROL FAILED: two calls with identical inputs produced different \
         messages. Every equality in this file would then be failing for a reason \
         that has nothing to do with the v1→sentinel mapping."
    );

    let other = PublicKey::from_bytes([0xAA; 32]);
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
        "POSITIVE CONTROL FAILED (other direction): a constant function would satisfy \
         the determinism check above; it must still fail this one. The target is one \
         of the two terms that decide the EFFECT of an authorization and it must move \
         the message."
    );
}
