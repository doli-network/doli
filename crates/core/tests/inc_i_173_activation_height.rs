//! INC-I-173 M1 — CATEGORY C: the `inc_i_173_activation_height` contract
//! (spec F2) and the "nothing else moved" guard (REQ-173-007, core half).
//!
//! TDD RED. Does NOT compile against the current tree: neither
//! `NetworkParams::inc_i_173_activation_height` nor
//! `ValidationContext::inc_i_173_activation_height` exists yet. Kept in its own
//! file so its compile failure cannot hide the runtime evidence in the other
//! INC-I-173 test files.
//!
//! Spec: `specs/state-only-fee-gate-architecture.md` F2 + "Consensus
//!       Classification (INV-12)" + "Activation Plan".
//! Analysis: `docs/redesigns/state-only-fee-gate-redesign-analysis.md`.
//! Requirements: REQ-173-005 (Must), REQ-173-007 (Must, core half).
//!
//! ---------------------------------------------------------------------------
//! REQUIRED API
//! ---------------------------------------------------------------------------
//! ```ignore
//! // crates/core/src/network_params/mod.rs
//! pub struct NetworkParams {
//!     ...
//!     /// INC-I-173 (F2). Gates the replacement of the hand-maintained 3-type
//!     /// fee-exemption `matches!` at `validation/utxo.rs:222` with the single
//!     /// exhaustive `TxType::allows_empty_io()` authority, wrapped by
//!     /// `Transaction::is_zero_flow()`.
//!     ///
//!     /// INV-12: Q1 YES (AddMaintainer/RemoveMaintainer are user-submittable
//!     /// via RPC `submitMaintainerChange`), Q2 YES (SlashProducer is
//!     /// node-generated on equivocation), Q3 NO (a block containing a 0-fee
//!     /// AddMaintainer flips REJECT -> ACCEPT) => ACTIVATION HEIGHT REQUIRED.
//!     ///
//!     /// CONSTANT GATE, never a `HardForkSchedule` entry: `current_fork_id`
//!     /// evaluates the schedule at `u64::MAX`, which would make the entry
//!     /// active in `fork_id` IMMEDIATELY and partition a rolling deploy
//!     /// (CLAUDE.md "If You Touch" / INV-8).
//!     ///
//!     /// IMMUTABLE once crossed (INV-PARAMS-001 / INC-I-054).
//!     pub inc_i_173_activation_height: u64,
//! }
//! ```
//! Defaults: devnet `0`, testnet `133_000`, mainnet `u64::MAX`.
//!
//! WHY MAINNET IS `u64::MAX` IN M1. The spec is explicit that the mainnet VALUE
//! is decided at release (M4), after re-verifying the live tip, and must clear
//! the ~30 external auto-update producers' window. Shipping M1 with a mainnet
//! literal invented today would either be already-crossed (INC-I-054: a crossed
//! height is immutable, so it could never be corrected) or an arbitrary guess.
//! `u64::MAX` is the only value that is BOTH fail-closed AND freely re-pinnable
//! at M4, and it matches the shipped precedent of `oracle_activation_height`.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `NetworkParams::defaults(Network) -> NetworkParams`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O1: `.inc_i_173_activation_height` — the new field.
//!   O2: every OTHER `*_activation_height` field — must be UNCHANGED
//!       (REQ-173-005: "no existing activation height is moved or reused").
//!   mutable params / receiver mutation / persistent store: NONE — `defaults`
//!       is an associated pure function.
//!   side channels: NONE.
//! CODE PATHS: PN-mainnet / PN-testnet / PN-devnet (one arm each).
//! INPUT PARTITIONS
//!   IP-M mainnet -> u64::MAX (fail-closed until M4 pins it)
//!   IP-T testnet -> 133_000
//!   IP-D devnet  -> 0
//! MATRIX: (O1 x 3) + (O2 x 3) cells.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT — `ValidationContext::new(..)` /
//!                   `ValidationContext::with_inc_i_173_activation_height(h)`
//! ---------------------------------------------------------------------------
//! ENUMERATION OF OBSERVABLE OUTPUTS
//!   O3: `ctx.inc_i_173_activation_height` after `new(..)`  -> MUST be u64::MAX.
//!   O4: `ctx.inc_i_173_activation_height` after `with_..(h)` -> MUST be `h`.
//!   O5: every OTHER `ValidationContext` field after `with_..(h)` -> unchanged
//!       (the builder takes `mut self` and returns `Self`; a fat-fingered arm
//!       could clobber a neighbour).
//!   mutable params: `mut self` is MOVED into the builder — O4/O5 ARE the
//!       receiver-mutation enumeration.
//!   persistent store / side channels: NONE.
//! CODE PATHS: P-default (never call the builder) / P-set (call it).
//! INPUT PARTITIONS
//!   IP-C1 h = 0        (devnet shape: always above the gate)
//!   IP-C2 h = u64::MAX (mainnet M1 shape: never above the gate)
//!   IP-C3 h = 133_000  (testnet shape)
//!   IP-C4 builder called TWICE — last write wins (idempotence of the setter)
//! MATRIX: O3 x P-default + (O4,O5) x P-set x {IP-C1..IP-C4}.

use doli_core::chainspec::ChainSpec;
use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationContext;
use doli_core::{Network, NetworkParams};

/// Devnet: always on. Fresh genesis every run, no history to reinterpret.
const DEVNET_GATE: u64 = 0;

/// Testnet: pinned NEAR-FUTURE so M2 (REQ-173-008) can exercise the maintainer
/// flow above the gate this cycle, with enough lead for the whole local fleet
/// to cross together. NOT `0` — `0` would reinterpret already-validated testnet
/// history under the new predicate.
///
/// RE-PINNED to `136_431` by INC-I-173 M2 (commit `7f917e7a`): the original
/// `133_000` was overtaken by the live testnet tip before the fleet crossed it,
/// so the gate was moved forward while still un-crossed and therefore not yet
/// consensus history. That commit changed `network_params/defaults.rs` alone
/// and left this constant behind, so this file asserted `133_000` against a
/// code value of `136_431` and had been red ever since. Code is the source of
/// truth (CLAUDE.md); the constant is corrected here to match
/// `crates/core/src/network_params/defaults.rs:492`.
///
/// Moving a crossed-AND-enforced height is forbidden (INV-PARAMS-001 /
/// INC-I-054). This re-pin predates crossing, so it is legal — and it is now
/// history: the testnet gate at `136_431` HAS been crossed (tip 146_711 at
/// 2026-08-12), so this value must never be moved again.
const TESTNET_GATE: u64 = 136_431;

/// Mainnet: NOT PINNED IN M1. See the module header.
const MAINNET_GATE: u64 = u64::MAX;

// ===========================================================================
// REQ-173-005 (Must) — the field exists on all 3 networks with pinned values
// ===========================================================================

/// REQ-173-005 (Must) — O1 x IP-D.
#[test]
fn req_173_005_devnet_gate_is_zero() {
    let p = NetworkParams::defaults(Network::Devnet);
    assert_eq!(
        p.inc_i_173_activation_height, DEVNET_GATE,
        "O1: devnet is always above the gate — matching every other INC gate's \
         devnet arm (inc_i_092, inc_i_096, inc_i_147, maintainer_derivation all 0)"
    );
}

/// REQ-173-005 (Must) — O1 x IP-T.
#[test]
fn req_173_005_testnet_gate_is_pinned_near_future_and_is_not_a_no_op() {
    let p = NetworkParams::defaults(Network::Testnet);
    let h = p.inc_i_173_activation_height;

    assert_eq!(h, TESTNET_GATE, "O1: the testnet gate is pinned at 136_431");
    assert_ne!(
        h, 0,
        "O1: a testnet gate of 0 would reinterpret already-validated testnet \
         history under the new predicate (INV-PARAMS-001 / INC-I-054)"
    );
    assert_ne!(
        h,
        u64::MAX,
        "O1: a testnet gate of u64::MAX makes the fix unreachable and M2 \
         (REQ-173-008) impossible to run"
    );
    // The testnet gate must be strictly ABOVE the testnet
    // maintainer_derivation gate (127_200, pinned 2026-08-10 at tip 126_801):
    // INC-I-173 depends on INC-I-172's derivation already being live, otherwise
    // the newly-mineable maintainer txs land on a trust root that has not been
    // seeded yet.
    assert!(
        h > p.maintainer_derivation_activation_height,
        "O1: the INC-I-173 testnet gate ({}) must be strictly above the INC-I-172 \
         derivation gate ({}) — maintainer txs must not become mineable before the \
         trust root they mutate is derived",
        h,
        p.maintainer_derivation_activation_height
    );
}

/// REQ-173-005 (Must) — O1 x IP-M. Fail-closed until M4.
#[test]
fn req_173_005_mainnet_gate_is_not_pinned_in_m1() {
    let p = NetworkParams::defaults(Network::Mainnet);
    assert_eq!(
        p.inc_i_173_activation_height, MAINNET_GATE,
        "O1: M1 ships mainnet fail-closed at u64::MAX. The real value is decided \
         at M4 after re-verifying the live tip and adding the external \
         auto-update window (~8_680 blocks ~ 24.1h). Pinning a guess now would \
         make it IMMUTABLE the moment the chain crossed it (INC-I-054)."
    );
    assert_ne!(
        p.inc_i_173_activation_height, 0,
        "O1: a mainnet gate of 0 activates a consensus rule change retroactively \
         over the entire chain — that is the INC-I-054 failure mode"
    );
}

/// REQ-173-005 (Must) — the gate is a NEW, DEDICATED field, not a reuse.
///
/// INV-PARAMS-001 / INC-I-054: bundling a new rule onto an existing height is
/// how INC-I-054 deactivated live security features. The new height must be
/// distinguishable from every neighbouring gate on at least one network.
#[test]
fn req_173_005_the_gate_is_dedicated_and_not_bundled_onto_an_existing_height() {
    let m = NetworkParams::defaults(Network::Mainnet);
    let t = NetworkParams::defaults(Network::Testnet);

    assert_ne!(
        t.inc_i_173_activation_height, t.maintainer_derivation_activation_height,
        "the INC-I-173 gate must NOT be bundled onto maintainer_derivation \
         (INC-I-172's height, testnet 127_200)"
    );
    assert_ne!(
        m.inc_i_173_activation_height, m.inc_i_147_activation_height,
        "the INC-I-173 gate must NOT be bundled onto inc_i_147"
    );
    assert_ne!(
        t.inc_i_173_activation_height, t.inc_i_096_activation_height,
        "the INC-I-173 gate must NOT be bundled onto inc_i_096"
    );
    assert_ne!(
        t.inc_i_173_activation_height, t.inc_i_092_activation_height,
        "the INC-I-173 gate must NOT be bundled onto inc_i_092"
    );
}

/// REQ-173-005 (Must) — O2: NO EXISTING ACTIVATION HEIGHT MOVED.
///
/// These literals are read from `crates/core/src/network_params/defaults.rs`
/// as of the INC-I-173 M1 branch point. A crossed mainnet height is consensus
/// history and is IMMUTABLE (INV-PARAMS-001). If adding one field perturbed a
/// neighbour, this fires.
#[test]
fn req_173_005_no_existing_activation_height_was_moved() {
    let m = NetworkParams::defaults(Network::Mainnet);
    assert_eq!(
        m.maintainer_derivation_activation_height, 172_000,
        "mainnet maintainer_derivation must stay 172_000 (INC-I-172, b5f68bba)"
    );
    // HARNESS FIX (developer, INC-I-173 M1): the mainnet and testnet inc_i_147
    // literals were TRANSPOSED when this test was written. Verified twice
    // against the branch point `b5f68bba`: `defaults.rs:251` (mainnet block,
    // the same block whose maintainer_derivation is 172_000) is 129_500 and
    // `defaults.rs:430` (testnet block) is 80_700; the field doc at
    // `network_params/mod.rs` states the same split independently. Only the two
    // literals were swapped — both values are still pinned, one per network, so
    // the assertion retains its full strength. Making the CODE match the
    // original literals would have required moving a mainnet activation height,
    // which is exactly what this test exists to forbid (INC-I-054).
    assert_eq!(
        m.inc_i_147_activation_height, 129_500,
        "mainnet inc_i_147 must stay 129_500"
    );
    assert_eq!(
        m.oracle_activation_height,
        u64::MAX,
        "mainnet oracle stays frozen pre-activation (HC-6 / INC-I-075)"
    );
    // HARNESS FIX (developer, INC-I-173 M1): mainnet `defi_activation_height` is
    // the literal `0`, not `u64::MAX`. Verified at the branch point `b5f68bba`
    // (`defaults.rs:164`) and unchanged by INC-I-173 (`git diff` on defaults.rs
    // shows only `inc_i_173_activation_height` additions). `u64::MAX` is what
    // CLAUDE.md claims ("Oracle + DeFi gates are u64::MAX") — that claim is
    // CODE-vs-DOC DRIFT already flagged as Follow-up 4 of
    // `specs/state-only-fee-gate-architecture.md`, and code is the source of
    // truth. Pinning the TRUE baseline keeps this test's purpose intact: it
    // still fires if INC-I-173 perturbs the neighbouring gate.
    assert_eq!(
        m.defi_activation_height, 0,
        "mainnet defi must stay 0 (see Follow-up 4: it contradicts CLAUDE.md)"
    );

    let t = NetworkParams::defaults(Network::Testnet);
    assert_eq!(
        t.maintainer_derivation_activation_height, 127_200,
        "testnet maintainer_derivation must stay 127_200"
    );
    // HARNESS FIX (developer, INC-I-173 M1): see the mainnet half above — this
    // is the other side of the same transposition.
    assert_eq!(
        t.inc_i_147_activation_height, 80_700,
        "testnet inc_i_147 must stay 80_700"
    );

    let d = NetworkParams::defaults(Network::Devnet);
    assert_eq!(d.maintainer_derivation_activation_height, 0);
    assert_eq!(d.inc_i_147_activation_height, 0);
}

// ===========================================================================
// REQ-173-005 (Must) — ValidationContext plumbing (spec F2)
// ===========================================================================

fn fresh_ctx(height: u64) -> ValidationContext {
    ValidationContext::new(ConsensusParams::mainnet(), Network::Mainnet, 0, height)
}

/// REQ-173-005 (Must) — O3 x P-default: FAIL-CLOSED.
///
/// The default MUST be `u64::MAX`, exactly like `sig_verification_height`,
/// `inc_i_092_activation_height`, `inc_i_096_activation_height` and every other
/// gate in `ValidationContext::new` (`validation/types.rs:266-282`). A default
/// of `0` would make every context that forgets the builder call silently
/// ABOVE the gate — the exact fork shape F2 warns about.
#[test]
fn req_173_005_validation_context_defaults_the_gate_to_u64_max() {
    let ctx = fresh_ctx(1);
    assert_eq!(
        ctx.inc_i_173_activation_height,
        u64::MAX,
        "O3: ValidationContext::new must default inc_i_173_activation_height to \
         u64::MAX (fail-closed)"
    );
}

/// REQ-173-005 (Must) — O4 x P-set x IP-C1..IP-C3.
#[test]
fn req_173_005_the_builder_sets_the_gate() {
    for h in [DEVNET_GATE, TESTNET_GATE, MAINNET_GATE, 200_000] {
        let ctx = fresh_ctx(1).with_inc_i_173_activation_height(h);
        assert_eq!(
            ctx.inc_i_173_activation_height, h,
            "O4: with_inc_i_173_activation_height({}) must set the field",
            h
        );
    }
}

/// REQ-173-005 (Must) — O4 x IP-C4: last write wins, the setter is a plain
/// assignment and not an accumulate/min/max.
#[test]
fn req_173_005_the_builder_is_a_plain_assignment_last_write_wins() {
    let ctx = fresh_ctx(1)
        .with_inc_i_173_activation_height(u64::MAX)
        .with_inc_i_173_activation_height(0);
    assert_eq!(
        ctx.inc_i_173_activation_height, 0,
        "O4: the setter must assign, not fold"
    );
}

/// REQ-173-005 (Must) — O5: the builder must not clobber a neighbouring gate.
///
/// The `.with_*` chain at both consensus call sites is ten calls long and every
/// one of them is a copy-paste of its neighbour. A body that assigned the wrong
/// field would still compile and would silently move an unrelated, possibly
/// already-crossed, consensus gate.
#[test]
fn req_173_005_the_builder_touches_only_its_own_field() {
    let sentinel = 424_242_u64;
    let base = fresh_ctx(1)
        .with_sig_verification_height(11)
        .with_security_audit_activation_height(22)
        .with_defi_activation_height(33)
        .with_amm_activation_height(44)
        .with_oracle_activation_height(55)
        .with_inc_i_092_activation_height(66)
        .with_inc_i_096_activation_height(77)
        .with_inc_i_026_scheduler_activation_height(88);

    let after = base.clone().with_inc_i_173_activation_height(sentinel);

    assert_eq!(after.inc_i_173_activation_height, sentinel, "O4");
    assert_eq!(after.sig_verification_height, 11, "O5: neighbour untouched");
    assert_eq!(
        after.security_audit_activation_height, 22,
        "O5: neighbour untouched"
    );
    assert_eq!(after.defi_activation_height, 33, "O5: neighbour untouched");
    assert_eq!(after.amm_activation_height, 44, "O5: neighbour untouched");
    assert_eq!(
        after.oracle_activation_height, 55,
        "O5: neighbour untouched"
    );
    assert_eq!(
        after.inc_i_092_activation_height, 66,
        "O5: neighbour untouched"
    );
    assert_eq!(
        after.inc_i_096_activation_height, 77,
        "O5: neighbour untouched"
    );
    assert_eq!(
        after.inc_i_026_scheduler_activation_height, 88,
        "O5: neighbour untouched"
    );
    assert_eq!(
        after.current_height, base.current_height,
        "O5: height intact"
    );
    assert_eq!(after.network, base.network, "O5: network intact");
}

// ===========================================================================
// REQ-173-007 (Must, core half) — NO GENESIS RESET
// The version-constant half lives in bins/node/tests/inc_i_173_state_only_fee_gate.rs
// (CURRENT_PROTOCOL_VERSION and friends are in the `network` crate, which
// doli-core does not and must not depend on).
// ===========================================================================

/// REQ-173-007 (Must) — the mainnet genesis hash is UNCHANGED.
///
/// A different genesis hash means a different chain identity: the binary would
/// be incompatible with the live network and every peer handshake would fail.
/// INC-I-173 touches no chainspec field, so this must hold byte-for-byte.
/// Golden value duplicated from `chainspec.rs::test_mainnet_genesis_hash_hardcoded`
/// ON PURPOSE — an INC-I-173 reviewer must see the assertion in the INC-I-173
/// diff, not have to go find it.
#[test]
fn req_173_007_mainnet_genesis_hash_is_unchanged() {
    assert_eq!(
        ChainSpec::mainnet().genesis_hash().to_hex(),
        "a91f6ba2aacc45c31fc1f26e4f0d4907edcc3c58a5ec464a65377cc1c8737338",
        "REQ-173-007: INC-I-173 must not change the mainnet genesis hash. \
         No genesis reset, no chainspec edit."
    );
}

/// REQ-173-007 (Must) — testnet and devnet genesis identities are unchanged
/// relative to each other and to mainnet.
///
/// Pinning the testnet/devnet hex here would couple this test to whichever
/// local genesis the operator last reset. What INC-I-173 must guarantee is the
/// weaker, stable claim: the three chains keep DISTINCT identities and the
/// mainnet hash is not silently reused.
#[test]
fn req_173_007_the_three_networks_keep_distinct_genesis_identities() {
    let m = ChainSpec::mainnet().genesis_hash();
    let t = ChainSpec::testnet().genesis_hash();
    let d = ChainSpec::devnet().genesis_hash();
    assert_ne!(
        m, t,
        "mainnet and testnet must not share a genesis identity"
    );
    assert_ne!(m, d, "mainnet and devnet must not share a genesis identity");
    assert_ne!(t, d, "testnet and devnet must not share a genesis identity");
}

/// REQ-173-007 (Must) — `ConsensusParams` still carries the same genesis hash
/// the chainspec computes. INC-I-173 adds a `NetworkParams` field; if that
/// perturbed params construction, the fork-id / handshake path would break
/// before any fee-gate test noticed.
#[test]
fn req_173_007_consensus_params_genesis_hash_still_matches_the_chainspec() {
    assert_eq!(
        ConsensusParams::mainnet().genesis_hash,
        ChainSpec::mainnet().genesis_hash(),
        "REQ-173-007: ConsensusParams::mainnet().genesis_hash must equal \
         ChainSpec::mainnet().genesis_hash()"
    );
    assert_eq!(
        ConsensusParams::testnet().genesis_hash,
        ChainSpec::testnet().genesis_hash()
    );
    assert_eq!(
        ConsensusParams::devnet().genesis_hash,
        ChainSpec::devnet().genesis_hash()
    );
}
