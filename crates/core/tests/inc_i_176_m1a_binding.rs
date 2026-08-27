//! INC-I-176 **M1a** — the signed-message BINDING contract.
//!
//! What the message is bound TO: the chain (REQ-176-011), the concrete effect
//! (REQ-176-012), and the height at which each arm applies (REQ-176-021's
//! dispatcher). The ENCODING contract is in `inc_i_176_m1a_authmsg.rs`; the
//! WIRE-FREEZE contract in `inc_i_176_m1a_wire_freeze.rs`; the single-owner
//! contract in `inc_i_176_m1a_ownership.rs`. Split for the 800-line test-file
//! budget (CLAUDE.md rule 19), not by accident.
//!
//! ---------------------------------------------------------------------------
//! M1a = ZERO WIRE-FORMAT CHANGE
//! ---------------------------------------------------------------------------
//! `MaintainerChangeData` keeps `reason: Option<String>` exactly as at HEAD
//! (`3f8bf185`). The attempt-1 swap to a `valid_before: u64` payload field was an
//! ungated bincode break on frozen history (testnet block 136690 carries a real
//! `add_maintainer`, `62a3bfbd..bc81`) and is deferred to M2.5 behind its own
//! activation height.
//!
//! `valid_before` remains a `signing_message*` **PARAMETER**, fed by the CALLER,
//! not by a payload field. Every `valid_before` partition below therefore
//! exercises the MESSAGE constructor, never the payload. `signing_message_at`
//! having zero production callers is M1a's INTENDED state — M2 wires it.
//!
//! TDD RED. This file does NOT compile against the tree at `3f8bf185`:
//! `doli_core::maintainer::{signing_message, signing_message_preimage,
//! signing_message_legacy, signing_message_at}` do not exist. That failure IS the
//! RED evidence.
//!
//! Contract + full matrix: `docs/.workflow/inc-i-176-M1a-output-contract.md`.
//! Spec: `specs/maintainer-authorization-architecture.md` §141 ("Exact bytes signed"),
//! §172 ("Activation-height decision"), §312 (USER GATE, BINDING).
//! Required API: see the header of `inc_i_176_m1a_authmsg.rs`.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT (this file's share; full matrix in the .md)
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   A-O1 `signing_message_preimage` LENGTH — the genesis-width probe reads it.
//!   A-O2 `signing_message_preimage` CONTENT.
//!   B-O1 `signing_message` LENGTH (must be exactly 32).
//!   B-O2 `signing_message` DIGEST bytes — the only observable that can show a term is
//!        bound. Two inputs differing in ONE bound term must not share it.
//!   D-O1 `signing_message_at` SELECTED branch, observed as byte equality with the legacy
//!        message (below the gate) or with the new digest (at/above).
//!   D-O2 the `>=` BOUNDARY at `height == activation_height`, and the numeric extremes.
//!   mutable params   : NONE (shared refs / `Copy`).
//!   receiver mutation: NONE (free functions).
//!   persistent store : NONE. No I/O on any path in this file.
//!   side channels    : NONE. DECLARED UNASSERTED — nothing is logged on these paths.
//!
//! CODE PATHS
//!   P-NEW  `height >= activation_height` -> `signing_message_at` dispatches to the digest.
//!   P-LEG  `height <  activation_height` -> `signing_message_at` dispatches to legacy.
//!   (Both are the `set.rs:262-274` idiom; `activation_height` is a plain `u64` so
//!    `crates/core::maintainer` stays a LEAF module.)
//!
//! INPUT PARTITIONS
//!   IP-G1 genesis B != A, same width, ONE BIT apart  -> REQ-176-011 cross-network
//!   IP-G1b two unrelated genesis fills (0xAA / 0xBB) -> REQ-176-011 with identical key
//!         material on both sides (the AUDIT-P1-016 case the requirement names)
//!   IP-G2 genesis of 31 B / 33 B                     -> concatenation-ambiguity probe (#9)
//!   IP-G3 genesis empty (0 B)                        -> worst scenario #1
//!   IP-A0/IP-A1 `is_add` true / false                -> effect scope
//!   IP-T1 target Y != X                              -> transplant resistance
//!   IP-V0 `valid_before = 0`                         -> "already expired"; worst scenario #1
//!   IP-V1 `valid_before = 1`                         -> off-by-one partner of IP-V0
//!   IP-V2 `valid_before = 17_280`                    -> the operational window
//!   IP-V3 `valid_before = u64::MAX-1`, `u64::MAX`    -> worst scenario #3 (numeric boundary)
//!   IP-H0 `height` in {0, 1, AH-2, AH-1}             -> P-LEG
//!   IP-H1 `height = AH`                              -> P-NEW boundary; the `>=` proof
//!   IP-H2 `height` in {AH+1, AH+1_000_000}           -> P-NEW
//!   IP-H3 `height = 0, AH = 0`                       -> devnet gate at the origin
//!   IP-H4 `height = u64::MAX, AH = u64::MAX`         -> frozen-mainnet boundary, no overflow
//!   MATRIX: B-O2 x {IP-G1, IP-G1b, IP-A0/A1, IP-T1, IP-V0..IP-V3};
//!           (A-O1,A-O2,B-O2) x {IP-G2, IP-G3};
//!           (D-O1,D-O2) x {IP-H0..IP-H4} x {IP-A0, IP-A1}.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO (binding constraints, not revisited)
//! ---------------------------------------------------------------------------
//! 1. It adds NO reject condition to `crates/core/src/validation/tx_types.rs` (user gate 1).
//! 2. It does NOT test absolute single-use. REQ-176-010 is RELAXED to bounded-validity
//!    (user gate 2); the seen-set is DEFERRED, not built. Nothing here asserts that a
//!    replayed authorization is refused — M3 owns the non-fatal expiry check, and even
//!    there an IN-WINDOW replay still succeeds (the documented residual).
//! 3. It does NOT touch `crates/updater/`.
//! 4. It does NOT pin any real activation height. `signing_message_at` is driven with a
//!    SYNTHETIC `activation_height` because M1a must prove the GATE MECHANISM, not a
//!    literal; the field itself does not exist until M2.
//! 5. It asserts NOTHING about a payload expiry field. There is none in M1a, and adding
//!    one is M2.5's job — see `inc_i_176_m1a_wire_freeze.rs`.

mod inc_i_176_m1a_common;

use doli_core::maintainer::{
    signing_message, signing_message_at, signing_message_legacy, signing_message_preimage,
    MaintainerChangeData, MaintainerSignature,
};
use inc_i_176_m1a_common::{pk, sig_entry, AUTH_DOMAIN, GOLDEN_GENESIS, GOLDEN_VALID_BEFORE};

// ===========================================================================
// REQ-176-011 — NETWORK SCOPE
// ===========================================================================

/// REQ-176-011 / B-O2 — an authorization built against genesis A does not verify
/// against genesis B. IP-G1: the two chains are ONE BIT apart, the weakest
/// possible difference.
#[test]
fn req_176_011_authorization_is_bound_to_one_genesis_hash() {
    let target = pk(0x66);
    let genesis_a = GOLDEN_GENESIS;
    let mut genesis_b = GOLDEN_GENESIS;
    genesis_b[31] ^= 0x01;

    let on_a = signing_message(&genesis_a, true, &target, GOLDEN_VALID_BEFORE);
    let on_b = signing_message(&genesis_b, true, &target, GOLDEN_VALID_BEFORE);

    assert_ne!(
        on_a, on_b,
        "REQ-176-011: two chains one BIT apart must not share an authorization message. If \
         they do, a signature harvested on testnet authorizes the same maintainer change on \
         mainnet."
    );
    assert_eq!(on_a.len(), 32, "B-O1");
    assert_eq!(on_b.len(), 32, "B-O1");
}

/// REQ-176-011 — byte-identical bootstrap key arrays across mainnet and testnet
/// are NO LONGER sufficient to make an authorization portable. IP-G1b.
///
/// The requirement names this case explicitly (AUDIT-P1-016: the mainnet and
/// testnet `BOOTSTRAP_MAINTAINER_KEYS_*` arrays have been byte-identical, so
/// membership alone never distinguished the networks). Asserted as a NAMED test,
/// not as a comment — that is the acceptance criterion verbatim.
#[test]
fn req_176_011_identical_key_material_no_longer_makes_an_authorization_portable() {
    let target = pk(0x77);
    let quorum: Vec<MaintainerSignature> = (1u8..=3).map(sig_entry).collect();

    let mainnet_genesis = [0xAAu8; 32];
    let testnet_genesis = [0xBBu8; 32];

    let data_mainnet = MaintainerChangeData::new(target, quorum.clone());
    let data_testnet = MaintainerChangeData::new(target, quorum);
    assert_eq!(
        data_mainnet, data_testnet,
        "fixture / POSITIVE CONTROL: the two payloads are IDENTICAL — identical key material, \
         identical target, identical window. Only the chain differs. If this fails, the test \
         proves nothing about the genesis term."
    );

    let msg_mainnet = signing_message(&mainnet_genesis, true, &target, GOLDEN_VALID_BEFORE);
    let msg_testnet = signing_message(&testnet_genesis, true, &target, GOLDEN_VALID_BEFORE);

    assert_ne!(
        msg_mainnet, msg_testnet,
        "REQ-176-011: identical bootstrap key arrays must no longer make an authorization \
         portable between networks. The genesis hash is the ONLY term that distinguishes \
         them, so if these match, cross-network replay is still open."
    );
    assert_ne!(
        msg_mainnet,
        signing_message_legacy(true, &target),
        "REQ-176-011: and the new message must not degenerate into the chain-blind legacy one"
    );
}

/// REQ-176-011 / A-O1, A-O2, B-O2 — genesis width variation cannot alias.
/// IP-G2 and IP-G3.
///
/// `genesis_hash` is the one variable-length input (`&[u8]`, the leaf-module
/// idiom copied verbatim from `maintainer_set_digest`). Worst scenario #9:
/// correctly-formatted input carrying inconsistent data. Every field AFTER it is
/// fixed-width, so two inputs of different genesis length always produce
/// different-length preimages — but that must be asserted, not assumed.
#[test]
fn req_176_011_genesis_of_a_different_width_cannot_alias() {
    let target = pk(0x88);
    let base = GOLDEN_GENESIS;
    let widths: [&[u8]; 4] = [&[], &base[..31], &base[..], &[0u8; 33]];

    let mut seen: Vec<(usize, Vec<u8>, Vec<u8>)> = Vec::new();
    for g in widths {
        let pre = signing_message_preimage(g, true, &target, GOLDEN_VALID_BEFORE);
        assert_eq!(
            pre.len(),
            AUTH_DOMAIN.len() + g.len() + 1 + 32 + 8,
            "A-O1: the preimage length must be domain + genesis + 1 + 32 + 8 for a {}-byte \
             genesis",
            g.len()
        );
        seen.push((
            g.len(),
            pre,
            signing_message(g, true, &target, GOLDEN_VALID_BEFORE),
        ));
    }

    for i in 0..seen.len() {
        for j in (i + 1)..seen.len() {
            assert_ne!(
                seen[i].1, seen[j].1,
                "A-O2: a {}-byte genesis and a {}-byte genesis produced the SAME preimage",
                seen[i].0, seen[j].0
            );
            assert_ne!(
                seen[i].2, seen[j].2,
                "B-O2: a {}-byte genesis and a {}-byte genesis produced the SAME digest",
                seen[i].0, seen[j].0
            );
        }
    }
}

// ===========================================================================
// REQ-176-012 — EFFECT SCOPE
// ===========================================================================

/// REQ-176-012 / B-O2 — a signature authorizing one concrete change cannot
/// authorize a different one: `add(X) != remove(X)` and `add(X) != add(Y)`.
/// IP-A0/IP-A1, IP-T1.
#[test]
fn req_176_012_action_and_target_are_both_inside_the_signed_bytes() {
    let x = pk(0x91);
    let y = pk(0x92);
    assert_ne!(x, y, "fixture: the two targets must actually differ");

    let add_x = signing_message(&GOLDEN_GENESIS, true, &x, GOLDEN_VALID_BEFORE);
    let remove_x = signing_message(&GOLDEN_GENESIS, false, &x, GOLDEN_VALID_BEFORE);
    let add_y = signing_message(&GOLDEN_GENESIS, true, &y, GOLDEN_VALID_BEFORE);

    assert_ne!(
        add_x, remove_x,
        "REQ-176-012: an `add` authorization must not be usable as a `remove` for the same \
         target — the effect bit is signed"
    );
    assert_ne!(
        add_x, add_y,
        "REQ-176-012: an authorization for target X must not be transplantable to target Y"
    );
    assert_ne!(remove_x, add_y);
}

/// REQ-176-012 / B-O2 — `valid_before` is inside the signed bytes.
/// IP-V0..IP-V3.
///
/// It is a PARAMETER of the message constructor in M1a, not a payload field. The
/// property still has to hold NOW: M2.5 will add the field, and if the term were
/// not already bound at that point the field would arrive rewritable in flight by
/// any relayer, making the whole bounded-validity relaxation (user gate 2)
/// vacuous. Extremes included: worst scenario #1 (zero) and #3 (numeric
/// boundary).
#[test]
fn req_176_012_valid_before_is_inside_the_signed_bytes() {
    let target = pk(0x93);
    let mut digests: Vec<(u64, Vec<u8>)> = Vec::new();
    for w in [0u64, 1, GOLDEN_VALID_BEFORE, u64::MAX - 1, u64::MAX] {
        digests.push((w, signing_message(&GOLDEN_GENESIS, true, &target, w)));
    }
    for i in 0..digests.len() {
        for j in (i + 1)..digests.len() {
            assert_ne!(
                digests[i].1, digests[j].1,
                "REQ-176-012: valid_before={} and valid_before={} produced the SAME signed \
                 message. The expiry term is not bound, so it can be rewritten in flight.",
                digests[i].0, digests[j].0
            );
        }
    }
}

// ===========================================================================
// GATE DISPATCH — `signing_message_at`
//
// Mirrors `MaintainerSet::verify_multisig_at` (set.rs:262-274). At M1 no
// production caller passes anything but the legacy arm; M2 wires the real
// activation height. The dispatcher is tested now so M2 only has to supply a
// number.
// ===========================================================================

/// REQ-176-021 / D-O1, D-O2 — the dispatcher selects EXACTLY the legacy bytes
/// below the gate and EXACTLY the new digest at and above it. IP-H0/H1/H2.
///
/// The boundary `height == activation_height` is included: the comparison must be
/// `>=`, matching `set.rs:269`. A `>` here would shift the whole activation by one
/// block relative to every other maintainer gate.
#[test]
fn req_176_021_gate_dispatch_selects_legacy_below_and_new_at_or_above() {
    let target = pk(0x9c);
    let ah = 200_000u64;

    for is_add in [true, false] {
        let legacy = signing_message_legacy(is_add, &target);
        let new = signing_message(&GOLDEN_GENESIS, is_add, &target, GOLDEN_VALID_BEFORE);
        assert_ne!(legacy, new, "fixture: the two arms must be distinguishable");

        for height in [0u64, 1, ah - 2, ah - 1] {
            assert_eq!(
                signing_message_at(
                    &GOLDEN_GENESIS,
                    is_add,
                    &target,
                    GOLDEN_VALID_BEFORE,
                    height,
                    ah
                ),
                legacy,
                "D-O1: at height {} (< AH {}) the dispatcher must return EXACTLY the legacy \
                 bytes. Anything else re-writes frozen consensus history.",
                height,
                ah
            );
        }

        for height in [ah, ah + 1, ah + 1_000_000] {
            assert_eq!(
                signing_message_at(
                    &GOLDEN_GENESIS,
                    is_add,
                    &target,
                    GOLDEN_VALID_BEFORE,
                    height,
                    ah
                ),
                new,
                "D-O1/D-O2: at height {} (>= AH {}) the dispatcher must return EXACTLY the \
                 new digest. The comparison is `>=`, as at set.rs:269.",
                height,
                ah
            );
        }
    }
}

/// REQ-176-021 / D-O2 — the dispatcher at both numeric extremes. IP-H3, IP-H4.
///
/// `activation_height = 0` is the devnet default (every height is at or above the
/// gate); `activation_height = u64::MAX` is the frozen mainnet default, where only
/// `height == u64::MAX` reaches it. Neither may overflow or invert.
#[test]
fn req_176_021_gate_dispatch_at_numeric_extremes() {
    let target = pk(0x9d);
    let legacy = signing_message_legacy(true, &target);
    let new = signing_message(&GOLDEN_GENESIS, true, &target, GOLDEN_VALID_BEFORE);

    let at = |height: u64, ah: u64| {
        signing_message_at(
            &GOLDEN_GENESIS,
            true,
            &target,
            GOLDEN_VALID_BEFORE,
            height,
            ah,
        )
    };

    assert_eq!(
        at(0, 0),
        new,
        "D-O2: devnet pins the gate at 0, so height 0 is AT the gate and takes the new arm"
    );
    assert_eq!(at(u64::MAX, 0), new, "D-O2: far above a zero gate");
    assert_eq!(
        at(u64::MAX - 1, u64::MAX),
        legacy,
        "D-O2: one block below a u64::MAX gate is still the legacy arm"
    );
    assert_eq!(
        at(u64::MAX, u64::MAX),
        new,
        "D-O2: a frozen (u64::MAX) gate is unreachable in practice but must not invert"
    );
}
