//! INC-I-176 **M2** — **REQ-176-022**, second half: the AUDIT-P0-011
//! domain-separation PAYOFF at the production site, the proof that the site stays
//! NON-FATAL, the persisted-trust-root channel, and the `ProtocolActivation`
//! scope fence.
//!
//! Companion to `bins/node/tests/inc_i_176_m2_gate_wiring.rs`, which carries the
//! below/at-gate accept-refuse matrix and the `>=` boundary proof. The two share
//! `inc_i_176_m2_common` — read that module's header for the harness hazard and
//! its solution BEFORE changing any height here. The split exists for the
//! 800-line test-file budget (CLAUDE.md rule 19).
//!
//! Design decisions (binding): `docs/.workflow/inc-i-176-M2-design-decision.md`.
//!
//! ---------------------------------------------------------------------------
//! TDD RED — EXPECTED, NOT A DEFECT
//! ---------------------------------------------------------------------------
//! Does **NOT compile** against the tree at `3f8bf185`
//! (`MAINTAINER_AUTH_VALID_BEFORE_UNSET` does not exist). Once it does, the
//! at-gate assertions are RUNTIME-red until `governance.rs` is rewired. The
//! BELOW-gate half of `audit_p0_011_…` asserts the DEFECT and is GREEN today —
//! deliberately, because it is frozen consensus history and it is the standing
//! proof that only the gate closes the collision.
//!
//! ---------------------------------------------------------------------------
//! WHY THE COLLISION IS REBUILT FROM THE FORMAT STRING
//! ---------------------------------------------------------------------------
//! The release-signing family is `format!("{}:{}", version, binary_sha256)`
//! (`crates/updater/src/verification.rs:33`). With `version = "add"` and
//! `binary_sha256 = target.to_hex()` those bytes are IDENTICAL to the legacy
//! maintainer-authorization message: a maintainer who signs what looks like a
//! release approval mints a permanent seat for an attacker key.
//!
//! `inc_i_176_m2_common::release_signing_message` rebuilds them from the FORMAT
//! STRING, not from `signing_message_legacy`, so the collision is DEMONSTRATED
//! rather than assumed. Borrowing the legacy constructor would make the claim
//! circular.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Function under test:
//!   G1 `Node::process_transaction_governance(&self, &Transaction, height: u64,
//!       &ProducerSet) -> Option<(u32, u64)>`
//!
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1 (return value)       `Option<(u32, u64)>` — `None` for both maintainer
//!                           arms (acceptance AND refusal), `Some` for an accepted
//!                           `ProtocolActivation`.
//!   O2 (receiver mutation)  `maintainer_state.set.members` — the acceptance
//!                           oracle.
//!   O4 (persistent store)   `<data_dir>/maintainer_state.bin` — the file the
//!                           updater reads as its binary-install trust root. An
//!                           in-memory-only mutation would leave it stale.
//!   O6 (process state)      no panic, no `Err`, no block rejection. This site is
//!                           reached from `apply_block`; an error there rejects
//!                           the BLOCK.
//!   mutable params          NONE — `&self`, `&Transaction`, `&ProducerSet`.
//!   side channels           `warn!`/`info!`. DECLARED UNASSERTED — no log capture
//!                           in this harness.
//!
//! CODE PATHS
//!   P-X-OPEN    below #22, release-signing bytes accepted (the live collision)
//!   P-X-CLOSED  at #22, the same bytes refused
//!   P-NONFATAL  a refusal warns and skips; the node stays usable
//!   P-PA        the `ProtocolActivation` arm — INVARIANT across #22
//!
//! INPUT PARTITIONS (heights DERIVED from the shipped testnet params at run
//! time, never re-typed as literals — see `gate_22`. Shipped #22 today: 15_087.)
//!   IP-X1 add signed over RELEASE-SIGNING bytes, h = #22 - 1 -> seat minted
//!   IP-X2 the same, h = #22                                  -> refused
//!   IP-X3 the bound message vs release-signing messages for five plausible
//!         `version` strings                                  -> disjoint, and
//!         disjoint BY LENGTH (32 bytes vs >= 66), so the separation is structural
//!         rather than an enumeration of one attack string
//!   IP-N1 legacy-signed add at h = #22, then a bound-signed add at the SAME
//!         height on the SAME node                            -> refused, then applied
//!   IP-D1 bound-signed add at h = #22, then read the file back
//!   IP-P1 ProtocolActivation, genuine 3-distinct quorum, h = #22 - 1 -> accepted
//!   IP-P2 the same at h = #22                                        -> accepted
//!
//! MATRIX
//!   O2 x {IP-X1, IP-X2, IP-N1} = 4 cells
//!   O1 x {IP-N1, IP-P1, IP-P2} = 4 cells
//!   O4 x {IP-D1}               = 2 cells (membership + threshold)
//!   O6 x {IP-N1}               = 1 cell
//!   (message-level) x {IP-X3}  = 10 cells
//!
//! ANTI-VACUITY PAIRING
//!   IP-X1 <-> IP-X2  only the height. Without IP-X1 passing, IP-X2's refusal
//!                    could just mean the harness never accepts anything.
//!   IP-N1 step 2     without it, step 1's refusal could mean the node was
//!                    poisoned rather than that the arm switched.
//!   IP-P1 <-> IP-P2  only the height; the expected outcome is UNCHANGED, which
//!                    is what makes it a scope fence rather than a test.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO (scope fence)
//! ---------------------------------------------------------------------------
//! 1. It adds NO reject condition and asserts nothing about
//!    `crates/core/src/validation/tx_types.rs` (`git diff HEAD` must stay EMPTY).
//! 2. It moves NO payload byte and touches no `HardForkSchedule` entry.
//! 3. It does NOT touch `crates/updater/`. The updater tripwire
//!    `the_collision_still_exists_and_only_m3_closes_it` tests the CLI-facing
//!    HELPER, which M2 deliberately leaves on the legacy arm — see
//!    `crates/core/tests/inc_i_176_m2_scope_fence.rs`. **Do not edit that file.**

mod inc_i_176_m2_common;

use crypto::KeyPair;
use doli_core::maintainer::{signing_message_legacy, ProtocolActivationData};
use doli_core::transaction::TxType;
use doli_core::{Network, Transaction};
use inc_i_176_m2_common::{
    bound_message, change_tx, make_node, members, quorum, release_signing_message, sig, submit,
    ACTIVATION_EPOCH, ACTIVATION_VERSION,
};
use storage::MaintainerState;

// ===========================================================================
// The gates, DERIVED — never re-typed as literals
//
// INC-I-203 M5 — Decision: the shipped testnet #22 was re-pinned 300_000 ->
// 15_087 (fresh testnet genesis, 2026-08-24). The file's old `BELOW_GATE`
// literal (299_999) is now ABOVE the shipped gate, so the AUDIT-P0-011 "below the gate"
// row was measuring the ABOVE-gate arm: the attack was correctly rejected where
// this test asserts frozen history must still accept it. The heights are now
// read through `Network::Testnet.params()` — the same accessor the production
// site uses (`governance.rs:47`) — so a re-pin moves the rows with the gate.
//
// The assertion SEMANTICS are unchanged: below #22 the AUDIT-P0-011 defect is
// PRESERVED (frozen consensus history); at/above #22 it is REJECTED.
//
// GATE COLLISION, RECORDED NOT HIDDEN. Testnet pins #20
// (`maintainer_derivation_activation_height`) at the SAME 15_087, so
// `below_gate()` is below BOTH and the two gates are INSEPARABLE on this
// network. #20 only selects signature-ENTRY counting over DISTINCT-SIGNER
// counting; every fixture here supplies three DISTINCT valid maintainer
// signers, for which both arms return the same verdict — proved by
// `inc_i_176_m2_gate_wiring::req_176_022_the_gate_20_collision_cannot_explain_the_below_gate_rows`.
// The `ProtocolActivation` scope fence additionally crosses #20's
// authority-source branch; see the note on that test.
// ===========================================================================

/// `inc_i_176_auth_binding_activation_height` (#22) for `Network::Testnet`.
fn gate_22() -> u64 {
    Network::Testnet
        .params()
        .inc_i_176_auth_binding_activation_height
}

/// One block BELOW #22 — the legacy arm, where AUDIT-P0-011 is frozen history.
fn below_gate() -> u64 {
    let g = gate_22();
    assert!(
        g >= 1,
        "harness: #22 is pinned at 0, so there is NO height below it. `#22 - 1` \
         would underflow and the below-gate row would collapse onto the gate, \
         leaving the AUDIT-P0-011 preservation half untested."
    );
    g - 1
}

/// EXACTLY #22 — the boundary. The comparison is `>=` (`set.rs:161`).
fn at_gate() -> u64 {
    gate_22()
}

// ===========================================================================
// AUDIT-P0-011 — THE DOMAIN-SEPARATION PAYOFF, ASSERTED AT THE PRODUCTION SITE
//
// The release-signing family is `format!("{}:{}", version, binary_sha256)`
// (`crates/updater/src/verification.rs:33`). With `version = "add"` and
// `binary_sha256 = target.to_hex()` those bytes are IDENTICAL to the legacy
// maintainer-authorization message: a maintainer who signs what looks like a
// release approval mints a permanent seat for an attacker key.
//
// The bytes are rebuilt here from the RELEASE-SIGNING format string, not from
// `signing_message_legacy`, so the collision is demonstrated rather than assumed.
// ===========================================================================

/// IP-X1, IP-X2 / O2 — AUDIT-P0-011 is LIVE below the gate and CLOSED above it.
///
/// **RED today** on the above-gate half. The below-gate half asserts the DEFECT on
/// purpose: it is frozen consensus history, and it is the standing proof that only
/// the gate closes the collision.
#[tokio::test]
async fn audit_p0_011_release_signing_bytes_mint_a_seat_below_the_gate_and_not_above() {
    let target = *KeyPair::generate().public_key();

    // The collision itself, stated as bytes.
    let attack_bytes = release_signing_message("add", &target.to_hex());
    assert_eq!(
        attack_bytes,
        signing_message_legacy(true, &target),
        "AUDIT-P0-011: a `release sign --version add --hash <target-hex>` \
         invocation produces BYTE-IDENTICAL bytes to the legacy maintainer \
         authorization. If this ever stops holding, the two families have been \
         separated by some other means and this test must be re-derived."
    );

    // BELOW the gate — the collision is live. This asserts the defect.
    {
        let (node, maintainers, _t) = make_node(4).await;
        let signers = quorum(&maintainers);
        let tx = change_tx(true, &target, &attack_bytes, &signers);
        submit(&node, &tx, below_gate()).await;
        assert!(
            members(&node).await.contains(&target),
            "AUDIT-P0-011, below #22: three maintainers who each believed they \
             were approving a RELEASE have seated a permanent maintainer. This is \
             frozen consensus history and must be preserved — it is why the fix \
             needs an activation height instead of being applied retroactively."
        );
    }

    // AT the gate — the collision is closed.
    {
        let (node, maintainers, _t) = make_node(4).await;
        let signers = quorum(&maintainers);
        let tx = change_tx(true, &target, &attack_bytes, &signers);
        submit(&node, &tx, at_gate()).await;
        assert!(
            !members(&node).await.contains(&target),
            "AUDIT-P0-011, at #22: release-signing bytes must NO LONGER seat a \
             maintainer. This is the payoff of the whole milestone."
        );
    }

    // And the structural reason it can never collide again: the bound message is
    // a 32-byte digest, while EVERY release-signing message over a 64-character
    // sha256 is at least 1 + 1 + 64 = 66 bytes. No `format!("{}:{}", version,
    // binary_sha256)` invocation with a well-formed hash can produce 32 bytes, so
    // the two families are now disjoint by LENGTH alone — not merely by content
    // for the one attack string tested above.
    let (node, _m, _t) = make_node(4).await;
    let bound = bound_message(&node, true, &target);
    assert_eq!(bound.len(), 32, "the bound message is a BLAKE3-256 digest");
    for version in ["add", "remove", "6.24.1", "v6.24.1", "0.2.0"] {
        let release = release_signing_message(version, &target.to_hex());
        assert_ne!(
            bound, release,
            "AUDIT-P0-011: the above-gate message must not equal the \
             release-signing message for version={:?}",
            version
        );
        assert!(
            release.len() > bound.len(),
            "AUDIT-P0-011 (structural): every release-signing message over a \
             64-char hash is longer than 32 bytes, so no such invocation can ever \
             alias the bound digest — the separation is by construction, not by \
             enumeration"
        );
    }
}

// ===========================================================================
// O6 — NO NEW FATAL REJECT PATH
// ===========================================================================

/// IP-W2 / O1, O2, O6 — a failed verification above the gate WARNS AND SKIPS.
///
/// The site is NON-FATAL by construction: `process_transaction_governance`
/// returns `Option`, never `Result`, and it is called from `apply_block` where an
/// error would reject the BLOCK. M2 must not change that — a maintainer change
/// whose signatures do not verify is skipped, and the block that carried it is
/// still applied.
///
/// The strongest available statement of "not poisoned" is that the very next
/// CORRECTLY-signed change, at the SAME height, on the SAME node, still applies.
#[tokio::test]
async fn req_176_022_a_failed_verification_above_the_gate_is_non_fatal() {
    let (node, maintainers, _t) = make_node(4).await;
    let target = *KeyPair::generate().public_key();
    let signers = quorum(&maintainers);

    // 1. A legacy-signed change above the gate — must not verify.
    let bad = change_tx(
        true,
        &target,
        &signing_message_legacy(true, &target),
        &signers,
    );
    let returned = submit(&node, &bad, at_gate()).await;
    assert_eq!(
        returned, None,
        "O1 / O6: the maintainer arms return None on BOTH acceptance and refusal. \
         A refusal must not become an Err, a panic or a block rejection — this \
         site is reached from apply_block."
    );
    assert_eq!(
        members(&node).await.len(),
        4,
        "O2: nothing was applied, and nothing was corrupted either"
    );

    // 2. The SAME node, the SAME height, correctly signed — still works.
    let msg = bound_message(&node, true, &target);
    let good = change_tx(true, &target, &msg, &signers);
    assert_eq!(submit(&node, &good, at_gate()).await, None, "O1");
    assert!(
        members(&node).await.contains(&target),
        "O6: a refused change must leave the node fully functional — if the first \
         submission had poisoned the lock, the state or the file, this would fail"
    );
}

/// O4 — the PERSISTED trust root follows the in-memory root above the gate.
///
/// `maintainer_state.bin` is what the updater reads as its binary-install trust
/// root. An in-memory-only mutation would leave the on-disk root stale, and the
/// updater would keep trusting the pre-change set for every install.
#[tokio::test]
async fn req_176_022_the_persisted_trust_root_follows_the_bound_arm() {
    let (node, maintainers, _t) = make_node(4).await;
    let data_dir = node.config.data_dir.clone();
    let target = *KeyPair::generate().public_key();
    let signers = quorum(&maintainers);

    let msg = bound_message(&node, true, &target);
    let tx = change_tx(true, &target, &msg, &signers);
    submit(&node, &tx, at_gate()).await;

    let persisted = MaintainerState::load(&data_dir).expect("O4: the state file must be readable");
    assert!(
        persisted.set.is_maintainer(&target),
        "O4: the change accepted above #22 must reach <data_dir>/maintainer_state.bin — \
         that file is the updater's install trust root, not a cache"
    );
    assert_eq!(persisted.set.threshold, 3, "O4: threshold persisted too");
}

// ===========================================================================
// SCOPE FENCE — the `ProtocolActivation` arm is UNTOUCHED but NOT CLEARED
// ===========================================================================

/// IP-P1, IP-P2 / O1 — `ProtocolActivation` acceptance is INVARIANT across #22.
///
/// `ProtocolActivationData::signing_message()` (`data.rs:144`, `"activate:{v}:{e}"`)
/// is out of M2's scope. It is gated on #20, not #22, and it must behave
/// identically on both sides of #22. If a developer "helpfully" routed the third
/// arm through `signing_message_at` as well, the above-gate row fails.
///
/// **This test asserts a fence, NOT a clean bill of health** (audit AUDIT-P1-101).
/// The arm it fences off is the one governance family left with no genesis binding
/// and no expiry, ungated at every height on every network, and it is
/// byte-collidable with the release family `{version}:{sha}`
/// (`crates/updater/src/verification.rs:93`) under the SAME 5-key root, raw Ed25519
/// on both sides — the same collision class M2 closes for the maintainer arms.
/// Closing it is a new consensus surface: it needs its own milestone and its own
/// activation height. Do NOT wire #22 into it.
#[tokio::test]
async fn scope_fence_protocol_activation_is_unaffected_by_gate_22() {
    // INC-I-203 M5 — Decision: `below_gate()` is now also below #20, where this
    // arm picks its authority set from `is_fully_bootstrapped()` rather than
    // fail-closed. `make_node(5)` seats INITIAL_MAINTAINER_COUNT = 5 members, so
    // `is_fully_bootstrapped()` holds and BOTH sides resolve the SAME on-chain
    // set — the fence still varies only #22.
    for height in [below_gate(), at_gate()] {
        let (node, maintainers, _t) = make_node(5).await;

        let data = ProtocolActivationData::new(
            ACTIVATION_VERSION,
            ACTIVATION_EPOCH,
            String::new(),
            vec![],
        );
        let msg = data.signing_message();
        let signed = ProtocolActivationData::new(
            ACTIVATION_VERSION,
            ACTIVATION_EPOCH,
            "INC-I-176 M2 scope fence".to_string(),
            quorum(&maintainers)
                .iter()
                .map(|kp| sig(kp, &msg))
                .collect(),
        );
        let tx = Transaction {
            version: 1,
            tx_type: TxType::ProtocolActivation,
            inputs: vec![],
            outputs: vec![],
            extra_data: signed.to_bytes(),
        };

        assert_eq!(
            submit(&node, &tx, height).await,
            Some((ACTIVATION_VERSION, ACTIVATION_EPOCH)),
            "SCOPE FENCE: ProtocolActivation is a different signing family \
             (`activate:{{v}}:{{e}}`) and #22 must not touch it — UNTOUCHED, not \
             CLEARED (AUDIT-P1-101): it keeps the same collision class, ungated at \
             every height, and needs its own milestone and activation height. It \
             stayed accepted at height {} — if it did not, the ProtocolActivation \
             arm was rewired and M2 has exceeded its scope.",
            height
        );
    }
}
