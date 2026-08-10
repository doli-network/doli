//! INC-I-172 M2 — CATEGORY B: the `maintainer_derivation_activation_height`
//! contract.
//!
//! Does NOT compile against the current tree; the field does not exist yet.
//! Kept in its own file so a compile error cannot hide the Category A runtime
//! evidence in `inc_i_172_m2_maintainer_governance.rs`.
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/network_params/mod.rs
//! pub struct NetworkParams {
//!     ...
//!     /// INC-I-172 M2 (F2/F3/F4). ONE constant gate covering: the one-shot
//!     /// genesis seed (kills the per-block re-derivation reset button), the
//!     /// canonical (registered_at, pubkey_bytes) derivation, the
//!     /// distinct-signer k-of-n counter, and the fail-close removal of the
//!     /// ad-hoc producer-key fallback in ProtocolActivation verification.
//!     ///
//!     /// CONSTANT GATE, **never** a HardForkSchedule entry: `current_fork_id`
//!     /// evaluates the schedule at `u64::MAX`, which would make the entry
//!     /// active in fork_id IMMEDIATELY and partition a rolling deploy
//!     /// (CLAUDE.md "If You Touch" / INV-8).
//!     pub maintainer_derivation_activation_height: u64,
//! }
//! ```
//! Defaults: mainnet `172_000`, testnet `127_200`, devnet `0`.
//! Env override: `DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT`, honored on
//! testnet/devnet, **LOCKED on mainnet** — matching the AUDIT-CLI-001 pattern
//! already applied to every neighbouring `*_activation_height`
//! (`crates/core/src/network_params/env_loader.rs:202-320`).
//!
//! Requirements: REQ-172-002, REQ-172-005, REQ-172-010, REQ-172-012.
//! Spec: `specs/maintainer-trust-root-architecture.md` §F2 (consensus
//! classification: INV-12 Q1 YES, Q2 YES, Q3 NO => activation height REQUIRED).
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `NetworkParams::defaults(Network)` / `NetworkParams::load(Network)`
//! ---------------------------------------------------------------------------
//! OUTPUTS
//!   O1 `defaults(net).maintainer_derivation_activation_height`
//!   O2 `load(net).maintainer_derivation_activation_height` (env-resolved)
//!   O3 (mutable params / receiver mutation / persistent writes) — NONE;
//!      `defaults` is associated and pure, `load` writes only a process-local
//!      `OnceLock` cache
//! PATHS
//!   PN-mainnet / PN-testnet / PN-devnet
//!   PE-locked   — env var set, mainnet: the default wins
//!   PE-honored  — env var set, non-mainnet: the override wins
//! INPUT PARTITIONS
//!   IP-N1 mainnet default == 172_000, != 0, != u64::MAX, > live tip at pin time
//!   IP-N2 testnet default == 127_200
//!   IP-N3 devnet default == 0
//!   IP-N4 env var set to a sentinel, mainnet   -> PE-locked
//!   IP-N5 env var set to the same sentinel, devnet -> PE-honored (anti-vacuity:
//!         proves the variable NAME is actually wired, so IP-N4 cannot pass
//!         merely because nothing reads it)
//! MATRIX: O1 x {IP-N1(4 assertions), IP-N2, IP-N3} + O2 x {IP-N4, IP-N5} = 8 cells.

use doli_core::{Network, NetworkParams};

/// Mainnet best height observed when the gate was pinned (INC-I-172 M2, 2026-08).
/// The height MUST be in the future: moving an activation height forward after
/// the chain has crossed it retroactively deactivates live rules — that is
/// INC-I-054, and CLAUDE.md makes a crossed height IMMUTABLE.
const MAINNET_TIP_AT_PIN_TIME: u64 = 162_727;

const MAINNET_GATE: u64 = 172_000;
const TESTNET_GATE: u64 = 127_200;

const ENV_VAR: &str = "DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT";

/// IP-N1. O1 x PN-mainnet.
#[test]
fn mainnet_gate_is_pinned_in_the_future_and_is_not_a_no_op() {
    let p = NetworkParams::defaults(Network::Mainnet);
    let h = p.maintainer_derivation_activation_height;

    assert_eq!(h, MAINNET_GATE, "O1: the mainnet gate is pinned at 172_000");
    assert_ne!(
        h, 0,
        "O1: 0 would apply the new derivation retroactively from genesis and \
         rewrite consensus history — the CLAUDE.md #0 rule (forward-only \
         activation, never from block 0)"
    );
    assert_ne!(
        h,
        u64::MAX,
        "O1: u64::MAX would ship the fix permanently disabled"
    );
    assert!(
        h > MAINNET_TIP_AT_PIN_TIME,
        "O1: the gate MUST still be in the future at pin time (tip was \
         {MAINNET_TIP_AT_PIN_TIME}). A height the chain has already crossed is \
         IMMUTABLE consensus history; pinning below the tip repeats INC-I-054."
    );
}

/// IP-N2. O1 x PN-testnet.
#[test]
fn testnet_gate_is_pinned_at_the_agreed_height() {
    assert_eq!(
        NetworkParams::defaults(Network::Testnet).maintainer_derivation_activation_height,
        TESTNET_GATE,
        "O1: the testnet gate is pinned at 127_200 so the fix can be rehearsed \
         before mainnet crosses 172_000"
    );
}

/// IP-N3. O1 x PN-devnet. Devnet activates from genesis: it is reset freely and
/// every other feature gate there is 0.
#[test]
fn devnet_gate_is_active_from_genesis() {
    assert_eq!(
        NetworkParams::defaults(Network::Devnet).maintainer_derivation_activation_height,
        0,
        "O1: devnet activates from block 0, matching every neighbouring gate"
    );
}

/// The three networks must not accidentally share one height — a shared value
/// is the signature of a copy-paste that would make the testnet rehearsal
/// meaningless.
#[test]
fn the_three_networks_carry_distinct_gates() {
    let m = NetworkParams::defaults(Network::Mainnet).maintainer_derivation_activation_height;
    let t = NetworkParams::defaults(Network::Testnet).maintainer_derivation_activation_height;
    let d = NetworkParams::defaults(Network::Devnet).maintainer_derivation_activation_height;

    assert_ne!(m, t, "mainnet and testnet gates must differ");
    assert_ne!(t, d, "testnet and devnet gates must differ");
    assert_ne!(m, d, "mainnet and devnet gates must differ");
}

/// IP-N4 + IP-N5. O2 x {PE-locked, PE-honored}.
///
/// Both partitions live in ONE test on purpose: `NetworkParams::load` caches per
/// network in a `OnceLock`, so the env var must be set before the FIRST load of
/// either network in this process. Nothing else in this test binary calls
/// `load`, which keeps that guarantee local and inspectable.
#[test]
fn mainnet_ignores_the_env_override_while_non_mainnet_honors_it() {
    // A sentinel that is neither network's default, so neither branch can pass
    // by coincidence.
    const SENTINEL: &str = "7";
    let original = std::env::var(ENV_VAR);
    std::env::set_var(ENV_VAR, SENTINEL);

    let mainnet = NetworkParams::load(Network::Mainnet).maintainer_derivation_activation_height;
    let devnet = NetworkParams::load(Network::Devnet).maintainer_derivation_activation_height;

    // Restore BEFORE asserting so a failure cannot leak process state into the
    // rest of the binary.
    match original {
        Ok(v) => std::env::set_var(ENV_VAR, v),
        Err(_) => std::env::remove_var(ENV_VAR),
    }

    // IP-N5 (anti-vacuity, asserted FIRST): if the variable name is not wired at
    // all, this fails and tells us the mainnet lock below is meaningless.
    assert_eq!(
        devnet, 7,
        "IP-N5 anti-vacuity: {ENV_VAR} must be honored on devnet. If this fails, \
         the variable is not wired and the mainnet-lock assertion below proves \
         nothing."
    );

    // IP-N4 — the lock.
    assert_eq!(
        mainnet, MAINNET_GATE,
        "AUDIT-CLI-001 / IP-N4: mainnet activation heights are LOCKED. An .env \
         override here would let a single operator move the maintainer trust-root \
         gate and fork itself off the network, or re-open the producer-key \
         fallback that F4 closes."
    );
}
