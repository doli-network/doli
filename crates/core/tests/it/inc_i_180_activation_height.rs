//! INC-I-180 M1 — the `withdrawal_holdings_gate_activation_height` contract
//! (brief F5). Requirement: REQ-I180-003 (Must).
//!
//! covers: network_params mod.rs, defaults.rs, env_loader.rs
//!
//! ---------------------------------------------------------------------------
//! TDD RED — EXPECTED, NOT A DEFECT
//! ---------------------------------------------------------------------------
//! This file does NOT compile against the current tree:
//! `NetworkParams::withdrawal_holdings_gate_activation_height` does not exist.
//! It is deliberately kept in its own module so its COMPILE failure cannot
//! suppress the BEHAVIOURAL evidence in
//! `bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs`, which names no
//! new symbol, compiles today, and fails on assertions.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/network_params/mod.rs
//! pub struct NetworkParams {
//!     ...
//!     /// INC-I-180. Gates the consensus rule "a RequestWithdrawal may not
//!     /// exceed the producer's bond holdings", enforced pre-mutation in
//!     /// `validate_block_economics` and mirrored at the apply-layer enqueue
//!     /// in `apply_block/tx_processing.rs`.
//!     ///
//!     /// INV-12 three-question verdict: Q1 YES (RequestWithdrawal is
//!     /// user-submittable), Q2 YES (it reaches `active_producers` through
//!     /// `selection_weight`), Q3 NO (a block carrying an over-allowance
//!     /// withdrawal flips ACCEPT -> REJECT) => ACTIVATION HEIGHT REQUIRED.
//!     ///
//!     /// CONSTANT GATE, never a `HardForkSchedule` entry: `current_fork_id`
//!     /// evaluates the schedule at `u64::MAX`, which would make the entry
//!     /// active in `fork_id` IMMEDIATELY and partition a rolling deploy.
//!     ///
//!     /// IMMUTABLE once crossed (INV-PARAMS-001 / INC-I-054).
//!     pub withdrawal_holdings_gate_activation_height: u64,
//! }
//! ```
//! Defaults (brief F5, binding): mainnet `u64::MAX`, testnet `230_000`,
//! devnet `20`. Env passthrough in `env_loader.rs` follows the INC-I-080
//! `addbond_cap_enforcement_activation_height` shape: mainnet locked to the
//! compiled default, non-mainnet overridable.
//!
//! WHY MAINNET IS `u64::MAX`. Pinning a real mainnet height is a separate
//! decision session (HC-6 / INC-I-075) that must re-measure the live tip and
//! clear the ~30 external auto-updating producers' window. A literal invented
//! today is either already crossed — and a crossed height is IMMUTABLE, so it
//! could never be corrected (INC-I-054) — or an arbitrary guess. `u64::MAX` is
//! the only value that is both fail-closed and freely re-pinnable later.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `NetworkParams::defaults(Network) -> NetworkParams`
//! ---------------------------------------------------------------------------
//! `defaults` is an associated pure function: no mutable parameters, no
//! receiver, no persistent store, no side channel, no blocking syscall.
//! EVERY observable output is a field of the returned struct.
//!   O1: `.withdrawal_holdings_gate_activation_height` — the new field.
//!   O2: every OTHER `*_activation_height` field — must be UNCHANGED.
//!       INV-PARAMS-001: a new feature gets its OWN height; no existing height
//!       is moved, reused or bundled. INC-I-054 is the incident that happened
//!       when one was.
//!   O3: the RELATION between O1 and O2 — the new gate must not COINCIDE with
//!       an existing one on a network where both are real heights. A shared
//!       literal is how "bundled onto an existing height" survives review.
//!
//! PATHS
//!   P1: `Network::Mainnet` → frozen (`u64::MAX`)
//!   P2: `Network::Testnet` → pinned near-future real height
//!   P3: `Network::Devnet`  → small, so devnet exercises the POST-AH arm
//!                            almost immediately while leaving a PRE-AH band
//!
//! INPUT PARTITIONS
//!   IP-A  the three `Network` variants, one per path (the enum is total)
//!   IP-B  the new field vs the five most recently added AHs (O3 dedication)
//!   IP-C  the six AHs a careless "reuse the nearest gate" edit would touch,
//!         read on all three networks (O2 regression guard)
//!
//! MATRIX
//!   O1×P1×IP-A → req_i180_003_mainnet_gate_is_pinned_above_the_measured_tip
//!   O1×P2×IP-A → req_i180_003_testnet_gate_is_pinned_near_future
//!   O1×P3×IP-A → req_i180_003_devnet_gate_leaves_a_pre_activation_band
//!   O3   ×IP-B → req_i180_003_the_gate_is_dedicated_and_not_bundled
//!   O2   ×IP-C → req_i180_003_no_existing_activation_height_was_moved

use doli_core::network_params::NetworkParams;
use doli_core::Network;

/// Brief F5 originally pinned this at 230_000, measured against a local testnet
/// tip of 215_847 on 2026-08-20.
///
/// RE-PINNED 2026-08-24 → 15_087. The local testnet took a FRESH GENESIS on
/// 2026-08-22: the chain restarted at height 0, which stranded every gate
/// pinned for the OLD chain (127_200 / 136_431 / 230_000 / 300_000) weeks in
/// the future and left four finished features dormant. All four
/// INC-I-172/173/176/180 testnet gates were re-pinned together to this single
/// height, measured against a live tip of ~15_006.
///
/// The chain has since crossed it (tip > 16_000), so 15_087 is now consensus
/// history on THIS testnet chain. Per INC-I-054 it must never be raised again:
/// moving a crossed height forward deactivates live consensus rules.
const TESTNET_GATE: u64 = 15_087;

/// The live testnet tip measured when `TESTNET_GATE` was re-pinned. The gate had
/// to sit ABOVE this, or it would have been crossed the moment the binary landed.
const TESTNET_TIP_AT_PIN: u64 = 15_006;

/// Brief F5. Small enough that devnet reaches the post-activation arm at once,
/// large enough to leave a pre-activation band for replay-parity tests.
const DEVNET_GATE: u64 = 20;

// ───────────────────────────── O1 — the new field ─────────────────────────

/// The mainnet pin, chosen 2026-08-25 against a MEASURED live tip of 292_388 —
/// 8_632 blocks of lead time, about 24 h at the 10 s slot. External producers
/// are upgraded MANUALLY for this release, so that window is the operator-chased
/// adoption budget: a node still on an older binary at this height forks.
const MAINNET_GATE: u64 = 301_020;

/// The live mainnet tip measured when `MAINNET_GATE` was chosen.
const MAINNET_TIP_AT_PIN: u64 = 292_388;

/// O1 × P1 × IP-A — mainnet is now PINNED (was `u64::MAX` through M1-M3).
#[test]
fn req_i180_003_mainnet_gate_is_pinned_above_the_measured_tip() {
    let p = NetworkParams::defaults(Network::Mainnet);
    let h = p.withdrawal_holdings_gate_activation_height;

    assert_eq!(h, MAINNET_GATE, "O1: mainnet is pinned at 301_020");
    assert!(
        h > MAINNET_TIP_AT_PIN,
        "O1: the pin must sit ABOVE the tip measured when it was chosen, or the \
         rule activates retroactively on blocks already in the chain"
    );
    assert_ne!(
        h, 0,
        "O1: 0 would re-validate every historical block under the new rule — \
         the INC-I-054 shape"
    );
    assert_ne!(
        h,
        u64::MAX,
        "O1: u64::MAX leaves mainnet unprotected — the state this pin exists to \
         leave behind"
    );
}

/// O1 × P2 × IP-A
#[test]
fn req_i180_003_testnet_gate_is_pinned_near_future() {
    let p = NetworkParams::defaults(Network::Testnet);
    let h = p.withdrawal_holdings_gate_activation_height;

    assert_eq!(h, TESTNET_GATE, "O1: the testnet gate is pinned at 15_087");
    assert_ne!(
        h, 0,
        "O1: 0 would activate retroactively from genesis and re-validate every \
         historical block under the new rule — the INC-I-054 shape"
    );
    assert_ne!(
        h,
        u64::MAX,
        "O1: u64::MAX on testnet would make the gate untestable, so M1 could \
         never be verified on a live network before mainnet pinning"
    );
    assert!(
        h > TESTNET_TIP_AT_PIN,
        "O1: the gate must sit ABOVE the tip measured when it was chosen, or it \
         is already crossed the moment the binary lands"
    );
}

/// O1 × P3 × IP-A — devnet must leave a PRE-activation band. The behavioural
/// suite `bins/node/tests/it/inc_i_180_withdrawal_holdings_gate.rs` drives the
/// gate by (network, height) at heights 5 and 1_000_007; a devnet gate of 0
/// would silently turn its four replay-parity rows into post-activation rows.
#[test]
fn req_i180_003_devnet_gate_leaves_a_pre_activation_band() {
    let p = NetworkParams::defaults(Network::Devnet);
    let h = p.withdrawal_holdings_gate_activation_height;

    assert_eq!(h, DEVNET_GATE, "O1: the devnet gate is 20");
    assert!(
        h > 5,
        "O1: heights 0..=5 must be BELOW the devnet gate — that band is where \
         the pre-activation bit-identity rows live"
    );
    assert!(
        h <= 1_000_007,
        "O1: height 1_000_007 must be at or above the devnet gate — that is the \
         post-activation row of the behavioural suite"
    );
}

// ──────────────────────── O3 — dedicated, not bundled ─────────────────────

/// O3 × IP-B — read on TESTNET. (On devnet several gates legitimately share `0`
/// or `20`, and on mainnet INC-I-173 and #22 are both `u64::MAX`, so the
/// comparison carries no information on either.)
///
/// SCOPE NARROWED 2026-08-24. The genesis reset re-pinned INC-I-172/173/176/180
/// to one shared testnet height (see `TESTNET_GATE`), so pairwise distinctness
/// among those four is no longer observable on testnet either — the same blind
/// spot devnet and mainnet already have. Asserting it would only assert that the
/// re-pin had not happened.
///
/// What REMAINS observable is the anti-pattern this test exists to catch: the
/// withdrawal gate bundled onto a height belonging to an unrelated rule that is
/// ALREADY CROSSED (`addbond_cap` and `delegation_auth` are both `0` on
/// testnet). Bundling there would retroactively change the withdrawal rule
/// across every historical block — the INC-I-054 shape. Those two stay strict.
#[test]
fn req_i180_003_the_gate_is_dedicated_and_not_bundled() {
    let p = NetworkParams::defaults(Network::Testnet);
    let h = p.withdrawal_holdings_gate_activation_height;

    // The genesis-reset cohort: these may collide with `h`, but ONLY at the one
    // documented post-reset height. Any OTHER shared value is a real bundling
    // bug and still fails here.
    for (feature, sibling) in [
        (
            "gate #22 (maintainer auth binding)",
            p.inc_i_176_auth_binding_activation_height,
        ),
        ("the state-only fee gate", p.inc_i_173_activation_height),
        ("gate #20", p.maintainer_derivation_activation_height),
    ] {
        assert!(
            sibling != h || h == TESTNET_GATE,
            "O3: {} is a different feature. It may share the withdrawal gate \
             ONLY at the documented post-genesis-reset height {}, got {}",
            feature,
            TESTNET_GATE,
            sibling
        );
    }

    assert_ne!(
        h, p.addbond_cap_enforcement_activation_height,
        "O3: the INC-I-080 AddBond cap is the SIBLING rule, not the same rule. \
         Bundling the withdrawal gate onto it would retroactively change the \
         withdrawal rule at a height the AddBond cap already crossed"
    );
    assert_ne!(
        h, p.delegation_auth_activation_height,
        "O3: delegation auth is a different feature"
    );
}

// ─────────────────────── O2 — nothing else moved ──────────────────────────

/// O2 × IP-C — the regression guard. These are the heights a careless "just
/// reuse the nearest gate" edit would touch. Read on all three networks.
///
/// If one of these literals is ever changed on purpose, this test must be
/// updated in the SAME commit and the change justified against INV-PARAMS-001
/// — that friction is the point.
#[test]
fn req_i180_003_no_existing_activation_height_was_moved() {
    let m = NetworkParams::defaults(Network::Mainnet);
    let t = NetworkParams::defaults(Network::Testnet);
    let d = NetworkParams::defaults(Network::Devnet);

    assert_eq!(m.addbond_cap_enforcement_activation_height, 0, "O2 mainnet");
    assert_eq!(t.addbond_cap_enforcement_activation_height, 0, "O2 testnet");
    assert_eq!(
        d.addbond_cap_enforcement_activation_height,
        u64::MAX,
        "O2 devnet"
    );

    assert_eq!(m.delegation_auth_activation_height, 0, "O2 mainnet");
    assert_eq!(t.delegation_auth_activation_height, 0, "O2 testnet");
    assert_eq!(d.delegation_auth_activation_height, u64::MAX, "O2 devnet");

    assert_eq!(m.security_audit_activation_height, 0, "O2 mainnet");
    assert_eq!(t.security_audit_activation_height, 0, "O2 testnet");
    assert_eq!(d.security_audit_activation_height, 0, "O2 devnet");

    assert_eq!(
        m.maintainer_derivation_activation_height, 172_000,
        "O2 mainnet"
    );
    // Deliberately a literal, NOT `TESTNET_GATE`: this guard must stay
    // INDEPENDENT of the withdrawal gate. If it tracked `TESTNET_GATE`, a future
    // re-pin of that constant would silently drag gate #20 along — which is the
    // exact "just reuse the nearest gate" edit this test exists to block.
    assert_eq!(
        t.maintainer_derivation_activation_height, 15_087,
        "O2 testnet — re-pinned 127_200 → 15_087 by the 2026-08-22 genesis reset"
    );
    assert_eq!(d.maintainer_derivation_activation_height, 0, "O2 devnet");

    assert_eq!(m.inc_i_173_activation_height, u64::MAX, "O2 mainnet");
    assert_eq!(
        t.inc_i_173_activation_height, 15_087,
        "O2 testnet — re-pinned 136_431 → 15_087 by the 2026-08-22 genesis reset"
    );
    assert_eq!(d.inc_i_173_activation_height, 0, "O2 devnet");

    assert_eq!(
        m.inc_i_176_auth_binding_activation_height,
        u64::MAX,
        "O2 mainnet"
    );
    assert_eq!(
        t.inc_i_176_auth_binding_activation_height, 15_087,
        "O2 testnet — re-pinned 300_000 → 15_087 by the 2026-08-22 genesis reset"
    );
    assert_eq!(d.inc_i_176_auth_binding_activation_height, 20, "O2 devnet");
}
