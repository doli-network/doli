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
//!   O1×P1×IP-A → req_i180_003_mainnet_gate_is_frozen_and_not_pinned_in_m1
//!   O1×P2×IP-A → req_i180_003_testnet_gate_is_pinned_near_future
//!   O1×P3×IP-A → req_i180_003_devnet_gate_leaves_a_pre_activation_band
//!   O3   ×IP-B → req_i180_003_the_gate_is_dedicated_and_not_bundled
//!   O2   ×IP-C → req_i180_003_no_existing_activation_height_was_moved

use doli_core::network_params::NetworkParams;
use doli_core::Network;

/// Brief F5: measured against a local testnet tip of 215_847 on 2026-08-20,
/// leaving roughly seven days of headroom.
const TESTNET_GATE: u64 = 230_000;

/// Brief F5. Small enough that devnet reaches the post-activation arm at once,
/// large enough to leave a pre-activation band for replay-parity tests.
const DEVNET_GATE: u64 = 20;

// ───────────────────────────── O1 — the new field ─────────────────────────

/// O1 × P1 × IP-A
#[test]
fn req_i180_003_mainnet_gate_is_frozen_and_not_pinned_in_m1() {
    let p = NetworkParams::defaults(Network::Mainnet);
    assert_eq!(
        p.withdrawal_holdings_gate_activation_height,
        u64::MAX,
        "O1: mainnet stays frozen in M1. Pinning is a later decision session; \
         a crossed height is immutable and could never be corrected"
    );
}

/// O1 × P2 × IP-A
#[test]
fn req_i180_003_testnet_gate_is_pinned_near_future() {
    let p = NetworkParams::defaults(Network::Testnet);
    let h = p.withdrawal_holdings_gate_activation_height;

    assert_eq!(h, TESTNET_GATE, "O1: the testnet gate is pinned at 230_000");
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
        h > 215_847,
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

/// O3 × IP-B — read on TESTNET, where every compared gate is a real height.
/// (On devnet several gates legitimately share `0` or `20`, so the comparison
/// carries no information there.)
#[test]
fn req_i180_003_the_gate_is_dedicated_and_not_bundled() {
    let p = NetworkParams::defaults(Network::Testnet);
    let h = p.withdrawal_holdings_gate_activation_height;

    assert_ne!(
        h, p.inc_i_176_auth_binding_activation_height,
        "O3: gate #22 (maintainer auth binding) is a different feature"
    );
    assert_ne!(
        h, p.inc_i_173_activation_height,
        "O3: the state-only fee gate is a different feature"
    );
    assert_ne!(
        h, p.maintainer_derivation_activation_height,
        "O3: gate #20 is a different feature"
    );
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
    assert_eq!(
        t.maintainer_derivation_activation_height, 127_200,
        "O2 testnet"
    );
    assert_eq!(d.maintainer_derivation_activation_height, 0, "O2 devnet");

    assert_eq!(m.inc_i_173_activation_height, u64::MAX, "O2 mainnet");
    assert_eq!(t.inc_i_173_activation_height, 136_431, "O2 testnet");
    assert_eq!(d.inc_i_173_activation_height, 0, "O2 devnet");

    assert_eq!(
        m.inc_i_176_auth_binding_activation_height,
        u64::MAX,
        "O2 mainnet"
    );
    assert_eq!(
        t.inc_i_176_auth_binding_activation_height, 300_000,
        "O2 testnet"
    );
    assert_eq!(d.inc_i_176_auth_binding_activation_height, 20, "O2 devnet");
}
