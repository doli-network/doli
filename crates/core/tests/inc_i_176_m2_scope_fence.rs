//! INC-I-176 **M2** — regression LOCKS for the things M2 must leave alone.
//!
//! Requirements: **REQ-176-021**, **REQ-176-022** (both, negatively: this file
//! asserts what the milestone must NOT change).
//!
//! Design decisions (binding): `docs/.workflow/inc-i-176-M2-design-decision.md`,
//! DECISION 3 and the SCOPE FENCE section.
//!
//! ---------------------------------------------------------------------------
//! WHY A SEPARATE FILE, AND WHY IT IS SHORT
//! ---------------------------------------------------------------------------
//! Three suites must keep passing **unmodified** across M2. Two of them already
//! own their contracts completely and are NOT duplicated here — duplicating a
//! golden vector creates a second copy that can drift, which is the exact failure
//! mode `inc_i_176_m1a_common` was written to prevent:
//!
//! * `crates/core/tests/inc_i_176_m1a_wire_freeze.rs` and
//!   `crates/core/tests/inc_i_176_m1a_wire_decode.rs` — the byte-freeze of
//!   `MaintainerChangeData`, including the real on-chain `add_maintainer` payload
//!   mined at testnet block 136_690. M2 moves no payload byte, so both must stay
//!   green. **They own the golden hex; this file does not restate it.**
//! * `bins/node/tests/inc_i_174_*.rs` (5 suites: `maintainer_reorg`,
//!   `maintainer_rewind_guards`, `maintainer_undo_capture`, `maintainer_undo`,
//!   `snapshot_binding`) — the maintainer undo/rewind pipeline. M2 changes WHICH
//!   MESSAGE is verified, never WHAT is recorded or rewound, so all five must
//!   still pass **unedited**. They are node-level and cross-crate; there is no
//!   honest assertion this crate can make about them, so they are listed here as
//!   a reviewer checklist rather than faked into a local test.
//!
//! What this file DOES assert is the one lock that is genuinely M2-specific and
//! that no other file covers: **M2 must not flip
//! `MaintainerChangeData::signing_message` to the bound arm.**
//!
//! ---------------------------------------------------------------------------
//! THE TRIPWIRE THIS FILE PROTECTS
//! ---------------------------------------------------------------------------
//! `crates/updater/tests/inc_i_172_m2_release_sign_arg_validation.rs::the_collision_still_exists_and_only_m3_closes_it`
//! asserts that a signature produced by `sign_release_hash(signer, "add",
//! target_hex)` still verifies against
//! `MaintainerChangeData::new(target, vec![]).signing_message(true)`.
//!
//! **That file must NOT be edited by M2, and M2 does not oblige it to flip.** It
//! tests the CLI-facing HELPER, and Decision 3 deliberately leaves the helper on
//! the legacy arm: the in-repo signer at
//! `bins/node/src/commands/maintainer.rs:157-158,224-225` calls it, and below #22
//! legacy is exactly what the verifier requires — changing it in M2 would emit
//! signatures no live node accepts. The gate lives at the free function
//! `signing_message_at`, called directly in `governance.rs`.
//!
//! The tripwire's premise is therefore an M2 invariant, and it is asserted HERE
//! so that a developer who flips the helper gets a failure inside the INC-I-176
//! diff, naming the tripwire — instead of a mystery break in the `updater` crate
//! that looks like it wants "fixing" by editing the file it must not edit.
//!
//! ---------------------------------------------------------------------------
//! TDD STATUS
//! ---------------------------------------------------------------------------
//! **GREEN today, and must STAY green.** Unlike its three sibling M2 files this
//! one compiles and passes against the tree at `3f8bf185`: it names no new API.
//! That is the point — it is the "nothing moved" half of the milestone, and a
//! lock that only starts working after the change it guards would be no lock at
//! all.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   H1 `MaintainerChangeData::signing_message(&self, is_add: bool) -> Vec<u8>`
//!   H2 `MaintainerChangeData::{to_bytes, from_bytes}` (round-trip only; the
//!      BYTE contract belongs to `inc_i_176_m1a_wire_freeze.rs`)
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1 H1's return CONTENT — must remain the frozen
//!      `format!("{}:{}", "add"|"remove", target.to_hex())` bytes.
//!   O2 H1's return LENGTH — 68 for `add`, 71 for `remove`. Split out because a
//!      length change alone already proves the format moved.
//!   O3 H2 round-trip identity — the payload still decodes to itself.
//!   mutable params    : NONE — `&self` and a `Copy` scalar.
//!   receiver mutation : NONE — `signing_message` takes `&self`.
//!   persistent store  : NONE. No I/O on any path in this file.
//!   side channels     : NONE. DECLARED UNASSERTED — nothing is logged here.
//!
//! CODE PATHS
//!   P-ADD    `is_add = true`
//!   P-REMOVE `is_add = false`
//!
//! INPUT PARTITIONS
//!   IP-T0 a deterministic target key
//!   IP-T1 a second, different target key — so a constant function cannot pass
//!   IP-S0 no signatures / IP-S1 three signature entries — the helper must ignore
//!         them (they cannot sign themselves; canonical ordering is M2.5's)
//!
//! MATRIX
//!   (O1, O2) x {P-ADD, P-REMOVE} x {IP-T0, IP-T1} = 8 cells
//!   O1 x {IP-S0, IP-S1}                           = 2 cells
//!   O3 x {IP-S0, IP-S1}                           = 2 cells

use crypto::{KeyPair, PublicKey, Signature};
use doli_core::maintainer::{
    signing_message, signing_message_legacy, MaintainerChangeData, MaintainerSignature,
    GOLDEN_AUTH_GENESIS_HASH,
};

fn pk(seed: u8) -> PublicKey {
    *KeyPair::from_seed([seed; 32]).public_key()
}

/// The release-signing preimage, reproduced verbatim from
/// `crates/updater/src/verification.rs:33` (`format!("{}:{}", version,
/// binary_sha256)`). Rebuilt from the FORMAT STRING rather than borrowed from
/// `signing_message_legacy`, so the collision below is demonstrated, not assumed.
fn release_signing_message(version: &str, binary_sha256: &str) -> Vec<u8> {
    format!("{}:{}", version, binary_sha256).into_bytes()
}

// ===========================================================================
// O1, O2 — the CLI-facing helper stays on the LEGACY arm
// ===========================================================================

/// SCOPE FENCE 3 / Decision 3 — `MaintainerChangeData::signing_message` must
/// still delegate to `signing_message_legacy`, for BOTH actions.
///
/// M2 wires the gate at the free function `signing_message_at`, called directly
/// in `bins/node/src/node/apply_block/governance.rs`. It does NOT route the
/// helper through the gate, because the helper has no height to gate on and its
/// caller — the in-repo signer — must keep emitting the bytes the live fleet
/// accepts below #22.
#[test]
fn m2_leaves_the_maintainer_change_data_helper_on_the_legacy_arm() {
    for seed in [0xC1u8, 0xC2] {
        let target = pk(seed);
        for is_add in [true, false] {
            let via_helper = MaintainerChangeData::new(target, vec![]).signing_message(is_add);
            assert_eq!(
                via_helper,
                signing_message_legacy(is_add, &target),
                "SCOPE FENCE: MaintainerChangeData::signing_message({}) must still \
                 delegate to signing_message_legacy. M2 gates at signing_message_at \
                 in governance.rs and leaves this helper alone (Decision 3).",
                is_add
            );

            let action = if is_add { "add" } else { "remove" };
            assert_eq!(
                via_helper,
                format!("{}:{}", action, target.to_hex()).into_bytes(),
                "O1: the frozen format is `{{action}}:{{target_hex}}`, verbatim"
            );
            assert_eq!(
                via_helper.len(),
                if is_add { 68 } else { 71 },
                "O2: 68 bytes for `add` (4 + 64 hex), 71 for `remove` (7 + 64). A \
                 length change alone already proves the format moved."
            );
        }
    }
}

/// ANTI-VACUITY — the helper is not a constant function.
///
/// Every assertion above would also hold for a helper that returned the same
/// bytes for every input, which would make the format checks meaningless.
#[test]
fn m2_helper_anti_vacuity_different_inputs_give_different_bytes() {
    let a = pk(0xC1);
    let b = pk(0xC2);
    assert_ne!(a, b, "fixture: the two targets must actually differ");

    let add_a = MaintainerChangeData::new(a, vec![]).signing_message(true);
    let add_b = MaintainerChangeData::new(b, vec![]).signing_message(true);
    let remove_a = MaintainerChangeData::new(a, vec![]).signing_message(false);

    assert_ne!(add_a, add_b, "ANTI-VACUITY: the target must move the bytes");
    assert_ne!(
        add_a, remove_a,
        "ANTI-VACUITY: the action must move the bytes"
    );
}

/// SCOPE FENCE 3 — the helper must NOT have gained chain binding.
///
/// The failure this guards against is a well-meaning developer routing the helper
/// through `signing_message` with some genesis hash "for consistency". That would
/// make the CLI signer emit bytes no node accepts below #22 and would silently
/// break the updater tripwire from the other direction.
#[test]
fn m2_helper_is_not_the_bound_message() {
    let target = pk(0xC3);
    for is_add in [true, false] {
        let via_helper = MaintainerChangeData::new(target, vec![]).signing_message(is_add);
        assert_ne!(
            via_helper,
            signing_message(&GOLDEN_AUTH_GENESIS_HASH, is_add, &target, u64::MAX),
            "SCOPE FENCE: the helper must NOT return the chain-bound message. If \
             it does, the in-repo signer emits bytes the live fleet rejects below \
             #22 (Decision 3)."
        );
        assert_ne!(
            via_helper.len(),
            32,
            "SCOPE FENCE: the helper must not return a 32-byte digest — the legacy \
             format is a 68/71-byte ASCII string"
        );
    }
}

/// SCOPE FENCE 3 — **the updater tripwire's premise, asserted inside the M2 diff.**
///
/// `crates/updater/tests/inc_i_172_m2_release_sign_arg_validation.rs::the_collision_still_exists_and_only_m3_closes_it`
/// verifies a `sign_release_hash(signer, "add", target_hex)` signature against
/// `MaintainerChangeData::new(target, vec![]).signing_message(true)`. That only
/// works while the two byte strings are identical.
///
/// **DO NOT EDIT THAT FILE.** M2 does not oblige it to flip: it exercises the
/// HELPER, which stays legacy. The gate closes the collision at the NODE apply
/// site, and that closure is proven by
/// `bins/node/tests/inc_i_176_m2_gate_wiring.rs::audit_p0_011_release_signing_bytes_mint_a_seat_below_the_gate_and_not_above`.
///
/// If THIS test fails, the helper was flipped and the correct response is to
/// revert the helper — not to edit the tripwire.
#[test]
fn m2_the_updater_tripwire_premise_still_holds() {
    let target = pk(0xC4);

    assert_eq!(
        release_signing_message("add", &target.to_hex()),
        MaintainerChangeData::new(target, vec![]).signing_message(true),
        "TRIPWIRE PREMISE: `release sign --version add --hash <target-hex>` still \
         produces bytes identical to the legacy maintainer authorization. This is \
         what `the_collision_still_exists_and_only_m3_closes_it` measures, and M2 \
         must leave it true. Do NOT edit that file — if this fails, revert the \
         change to MaintainerChangeData::signing_message instead."
    );
    assert_eq!(
        release_signing_message("remove", &target.to_hex()),
        MaintainerChangeData::new(target, vec![]).signing_message(false),
        "TRIPWIRE PREMISE: the same holds for the remove action"
    );
}

// ===========================================================================
// O3 — the payload still round-trips (the BYTE contract lives elsewhere)
// ===========================================================================

/// SCOPE FENCE 2 — M2 moves no payload byte.
///
/// A light round-trip only. The authoritative byte-level freeze — golden hex,
/// encoded lengths, and the real on-chain payload from testnet block 136_690 —
/// belongs to `inc_i_176_m1a_wire_freeze.rs` and `inc_i_176_m1a_wire_decode.rs`,
/// and is NOT restated here: a second copy of a golden vector is a second thing
/// that can drift, which defeats the purpose of having one.
///
/// What this adds on top of those files is the M2-specific statement that the
/// helper's signature-carrying and signature-free shapes both survive M2 — i.e.
/// that no `valid_before` field crept into the payload while the message gained
/// one as a PARAMETER.
#[test]
fn m2_payload_round_trips_unchanged_with_and_without_signatures() {
    let target = pk(0xC5);
    let entries: Vec<MaintainerSignature> = (1u8..=3)
        .map(|s| MaintainerSignature::new(pk(s), Signature::default()))
        .collect();

    for signatures in [Vec::new(), entries] {
        let data = MaintainerChangeData::new(target, signatures);
        let encoded = data.to_bytes();
        assert_eq!(
            MaintainerChangeData::from_bytes(&encoded),
            Some(data.clone()),
            "O3 / SCOPE FENCE 2: the payload must still decode to itself. The \
             authoritative byte freeze is inc_i_176_m1a_wire_freeze.rs; this is \
             the M2 statement that nothing was added to the shape."
        );

        // The signed bytes must NOT depend on the signature vector — signatures
        // cannot sign themselves, and canonical ordering is M2.5's, not M2's.
        assert_eq!(
            data.signing_message(true),
            MaintainerChangeData::new(target, vec![]).signing_message(true),
            "O1: the helper's output must be independent of the signature vector, \
             exactly as it was before M2"
        );
    }
}
