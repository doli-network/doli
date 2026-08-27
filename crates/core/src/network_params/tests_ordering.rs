//! INC-I-176 **M2 review F4** — the `#22 >= #20` ordering on the **RUNTIME** path.
//!
//! Why this module exists
//! ----------------------
//! Six ordering tests already assert
//! `inc_i_176_auth_binding_activation_height (#22) >=
//! maintainer_derivation_activation_height (#20)` — and every one of them reads
//! `NetworkParams::defaults(network)`. `defaults()` **bypasses the env loader**, so
//! none of them observes the value a node actually runs on. On testnet and devnet
//! both heights pass through `env_loader::load_from_env`, where `DOLI_*` overrides
//! can move either one independently and invert the order that the field's own
//! rustdoc calls SECURITY-CRITICAL.
//!
//! The inversion is not cosmetic. In a band `[#22, #20)` the node verifies the
//! STRONGER, chain-bound INC-I-176 message with the WEAKER, pre-INC-I-172
//! ENTRY-COUNTING counter, in which three signature entries from ONE key clear a
//! 3-of-5 threshold (AUDIT-P0-010 re-armed underneath AUDIT-P1-016).
//!
//! These tests call `load_from_env` **directly**. They deliberately do NOT go
//! through `Network::params()` / `NetworkParams::load`, which memoize per network
//! in a `OnceLock` (`network_params/mod.rs`): the first load in a test binary wins
//! for the whole process, so a cached read cannot express more than one partition.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Units under test:
//!   U1 `env_loader::load_from_env(network)`, restricted to the two activation
//!      heights and the ordering relation BETWEEN them
//!   U2 `ordering::enforce_auth_binding_above_derivation(requested, compiled,
//!      effective_derivation)`, the pure decision function U1 delegates to
//!
//! OUTPUTS
//!   O1 (return value)  `params.inc_i_176_auth_binding_activation_height`   [U1]
//!   O2 (return value)  `params.maintainer_derivation_activation_height`    [U1]
//!   O3 (derived)       the relation `O1 >= O2` — the property itself       [U1]
//!   O4 (return value)  the substituted height                              [U2]
//!   O5 (mutable params)     — NONE; `load_from_env` takes `Network` by value
//!   O6 (receiver mutation)  — NONE; both are free functions
//!   O7 (persistent store)   — NONE. No file is written. Process ENV is global
//!      mutable state and IS restored by every test here, under `ENV_MUTEX`.
//!   O8 (log side effect)    — the refusal `error!`. NOT asserted: this tree has
//!      no log-capture harness, and the refusal is already observable through O1.
//!      Recorded so the omission is a decision, not an oversight.
//!   O9 (return value)  `ordering::blame_for(requested, compiled)` — WHICH `.env`
//!      variable the refusal names (AUDIT-P2-102). Added because O8 is
//!      unassertable: the decision the message is built from is exposed as a pure
//!      function so the diagnostic's correctness is testable without a log harness.
//!
//! PATHS
//!   PT-accept    `requested >= effective #20` -> the override is applied verbatim
//!   PT-refuse-lo `#22` overridden BELOW `#20` -> refused, substituted
//!   PT-refuse-hi `#20` overridden ABOVE the compiled `#22`, `#22` untouched ->
//!                refused, substituted (the leg an override-only-`#22` check misses)
//!   PT-locked    mainnet: neither override is read at all
//!
//! INPUT PARTITIONS
//!   IP-1  testnet, `#22` = 0                      (far below #20)  -> PT-refuse-lo
//!   IP-2  testnet, `#22` = #20 - 1                (boundary below) -> PT-refuse-lo
//!   IP-3  testnet, `#22` = #20 exactly            (the `>=` edge)  -> PT-accept
//!   IP-4  testnet, `#22` = #20 + 1                (boundary above) -> PT-accept
//!   IP-5  testnet, `#22` = a legal value that is NOT the compiled default
//!                                                                  -> PT-accept
//!                 (POSITIVE CONTROL — proves the variable is wired at all)
//!   IP-6  devnet, `#22` = 0 while `#20` = 0       (`0 >= 0`)       -> PT-accept
//!   IP-7  devnet, `#20` overridden to 1_000, `#22` unset (compiled 20)
//!                                                                  -> PT-refuse-hi
//!   IP-8  mainnet, BOTH variables set to 0                         -> PT-locked
//!   IP-9  no variable set, all three networks                      -> PT-accept
//!   IP-10 U2 directly: `compiled < effective_derivation` on the refusal path
//!                                    -> O4 == effective_derivation, not compiled
//!   IP-11 U3 `blame_for`, `requested != compiled`  -> AuthBindingOverride
//!   IP-12 U3 `blame_for`, `requested == compiled`  -> DerivationOverride
//!   IP-13 testnet, `#22` = `#20` exactly (127_200) -> PT-accept, and it is
//!         RETROACTIVE. The AUDIT-P2-105 residual, asserted as a KNOWN HOLE.
//!   IP-14 the shipped defaults vs the `#22 <= #21` upper half -> it is VIOLATED
//!         on testnet and devnet, which is why no upper-bound guard exists
//!
//! MATRIX
//!   O1 x {IP-1..IP-9, IP-13}  = 10 assertions (grouped into 8 tests)
//!   O2 x {IP-7, IP-8}   = 2 assertions
//!   O3 x {IP-1, IP-2, IP-7, IP-9}  = the invariant re-asserted on every path that
//!                                    could break it
//!   O4 x {IP-10, IP-12} = 2 assertions
//!   O9 x {IP-11, IP-12} = 2 assertions
//!   (IP-14 reads `defaults()`, not U1/U2 — it pins the PREMISE of the
//!   no-upper-bound decision, so the decision goes red if the premise dies.)
//!
//! ANTI-VACUITY
//!   IP-5 is a POSITIVE CONTROL: if `DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT`
//!   were misspelled or unwired, every "refused" assertion would pass for the wrong
//!   reason — the loader would simply be returning the compiled default. IP-5 fails
//!   in that world. IP-10 is the second control: it distinguishes "fell back to the
//!   compiled default" from "fail-closed to at least #20", which are the same value
//!   on every shipped network and differ only under IP-7.

use std::sync::Mutex;

use crate::Network;

use super::env_loader::load_from_env;
use super::ordering::{blame_for, enforce_auth_binding_above_derivation, InversionBlame};
use super::NetworkParams;

const VAR_22: &str = "DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT";
const VAR_20: &str = "DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT";

/// Env vars are process-global; these tests mutate them. Serialize.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// Set the listed vars, run `f`, restore the previous values whatever happens.
///
/// The restore runs BEFORE any assertion in the caller, so a failing test cannot
/// leak process state into the rest of the binary.
fn with_env<T>(vars: &[(&str, &str)], f: impl FnOnce() -> T) -> T {
    let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    let saved: Vec<(String, Option<String>)> = vars
        .iter()
        .map(|(k, _)| ((*k).to_string(), std::env::var(k).ok()))
        .collect();
    for (k, v) in vars {
        std::env::set_var(k, v);
    }

    let out = f();

    for (k, v) in saved {
        match v {
            Some(v) => std::env::set_var(&k, v),
            None => std::env::remove_var(&k),
        }
    }
    out
}

fn compiled(network: Network) -> (u64, u64) {
    let d = NetworkParams::defaults(network);
    (
        d.maintainer_derivation_activation_height,
        d.inc_i_176_auth_binding_activation_height,
    )
}

/// IP-5 — POSITIVE CONTROL, asserted first in the file's reading order because
/// every refusal test below is meaningless without it.
///
/// A legal, ordering-respecting override must actually reach `NetworkParams`. If
/// it does not, the variable is unwired and "the loader refused it" is
/// indistinguishable from "the loader never read it".
#[test]
fn f4_positive_control_a_legal_override_is_honored_on_the_runtime_path() {
    let (gate20, gate22) = compiled(Network::Testnet);
    // Between #20 and the compiled #22, and equal to neither, so no branch can
    // pass by coincidence.
    let legal = gate20 + 1_000;
    assert!(
        legal > gate20 && legal != gate22,
        "test setup is degenerate: {legal} must be above #20 ({gate20}) and \
         different from the compiled #22 ({gate22})"
    );

    let got = with_env(&[(VAR_22, &legal.to_string())], || {
        load_from_env(Network::Testnet).inc_i_176_auth_binding_activation_height
    });

    assert_eq!(
        got, legal,
        "POSITIVE CONTROL FAILED: {VAR_22} was not honored on testnet. With the \
         variable unwired, the refusal tests below prove nothing — they would pass \
         by returning the compiled default for the wrong reason."
    );
}

/// IP-1 + IP-2 / PT-refuse-lo. O1, O3.
///
/// The finding itself: an override that drives #22 below #20 must NOT be applied.
#[test]
fn f4_an_override_that_drives_22_below_20_is_refused_on_the_runtime_path() {
    let (gate20, gate22) = compiled(Network::Testnet);

    for raw in ["0", &(gate20 - 1).to_string()] {
        let params = with_env(&[(VAR_22, raw)], || load_from_env(Network::Testnet));
        let got = params.inc_i_176_auth_binding_activation_height;

        assert_ne!(
            got,
            raw.parse::<u64>().unwrap(),
            "M2 review F4: {VAR_22}={raw} inverts the SECURITY-CRITICAL #22 >= #20 \
             ordering on testnet (#20 = {gate20}) and MUST be refused. Applying it \
             opens a band in which the chain-bound INC-I-176 message is verified by \
             the pre-INC-I-172 entry-counting counter."
        );
        assert_eq!(
            got, gate22,
            "a refused override must fall back to the compiled default ({gate22}), \
             not to some third value"
        );
        // O3 — the property, restated on the value the node would run on.
        assert!(
            got >= params.maintainer_derivation_activation_height,
            "#22 ({got}) must be >= #20 ({}) after the loader runs",
            params.maintainer_derivation_activation_height
        );
    }
}

/// IP-3 + IP-4 / PT-accept. O1.
///
/// The rule is `>=`, matching `MaintainerSet::verify_multisig_at` and
/// `signing_message_at`. `#22 == #20` is legal; one below it is not.
#[test]
fn f4_the_runtime_boundary_is_gte_not_gt() {
    let (gate20, _) = compiled(Network::Testnet);

    let at_edge = with_env(&[(VAR_22, &gate20.to_string())], || {
        load_from_env(Network::Testnet).inc_i_176_auth_binding_activation_height
    });
    assert_eq!(
        at_edge, gate20,
        "#22 == #20 satisfies the ordering and must be applied verbatim; refusing \
         it would make the runtime rule stricter than the documented one"
    );

    let above = with_env(&[(VAR_22, &(gate20 + 1).to_string())], || {
        load_from_env(Network::Testnet).inc_i_176_auth_binding_activation_height
    });
    assert_eq!(above, gate20 + 1, "#22 == #20 + 1 must be applied verbatim");
}

/// IP-6 / PT-accept. O1.
///
/// Precision: the enforced rule is EXACTLY `#22 >= #20` and nothing more. On
/// devnet `#20` is `0`, so `#22 = 0` satisfies it and is applied. The devnet
/// hazard recorded elsewhere is the OTHER, exempted half (`#22 <= #21`), and this
/// guard deliberately does not police it.
#[test]
fn f4_devnet_zero_is_accepted_because_devnet_20_is_zero() {
    let (gate20, _) = compiled(Network::Devnet);
    assert_eq!(gate20, 0, "premise: devnet #20 is 0");

    let got = with_env(&[(VAR_22, "0")], || {
        load_from_env(Network::Devnet).inc_i_176_auth_binding_activation_height
    });
    assert_eq!(
        got, 0,
        "0 >= 0 holds, so the override is ordering-respecting and must be applied. \
         This guard enforces the #22 >= #20 half only."
    );
}

/// IP-7 / PT-refuse-hi. O1, O2, O3.
///
/// The leg a `#22`-only check misses: the operator never touches `#22` at all and
/// raises `#20` above it. Same inversion, different variable. The substituted
/// value must be at least `#20` — falling back to the compiled `#22` (20) would
/// leave the node inside the forbidden band.
#[test]
fn f4_raising_20_above_the_compiled_22_is_also_refused() {
    let (_, gate22) = compiled(Network::Devnet);
    let raised = gate22 + 980;

    let params = with_env(&[(VAR_20, &raised.to_string())], || {
        load_from_env(Network::Devnet)
    });

    assert_eq!(
        params.maintainer_derivation_activation_height, raised,
        "premise: #20 IS overridable on devnet — if this fails the test proves \
         nothing about the ordering"
    );
    assert!(
        params.inc_i_176_auth_binding_activation_height
            >= params.maintainer_derivation_activation_height,
        "M2 review F4: raising #20 to {raised} above the compiled #22 ({gate22}) \
         inverts the ordering just as surely as lowering #22 does. Got #22 = {}, \
         #20 = {}.",
        params.inc_i_176_auth_binding_activation_height,
        params.maintainer_derivation_activation_height
    );
    assert_eq!(
        params.inc_i_176_auth_binding_activation_height, raised,
        "the substitution must be max(compiled #22, effective #20) — fail-CLOSED. \
         Returning the compiled #22 ({gate22}) would still sit inside the forbidden \
         band."
    );
}

/// IP-8 / PT-locked. O1, O2.
///
/// Mainnet reads neither variable. The guard must not become a back door that
/// makes a mainnet height env-reachable.
#[test]
fn f4_mainnet_ignores_both_overrides() {
    let (gate20, gate22) = compiled(Network::Mainnet);

    let params = with_env(&[(VAR_22, "0"), (VAR_20, "0")], || {
        load_from_env(Network::Mainnet)
    });

    assert_eq!(
        params.inc_i_176_auth_binding_activation_height, gate22,
        "mainnet #22 is LOCKED: an .env override must never move the gate that \
         decides WHICH BYTES authorize a maintainer change"
    );
    assert_eq!(
        params.maintainer_derivation_activation_height, gate20,
        "mainnet #20 is LOCKED"
    );
}

/// IP-9 / PT-accept. O3.
///
/// The runtime-path counterpart of the six `defaults()` ordering assertions: with
/// no variable set, every network still satisfies the ordering AFTER the loader
/// has run.
#[test]
fn f4_the_shipped_defaults_satisfy_the_ordering_on_the_runtime_path() {
    for network in [Network::Mainnet, Network::Testnet, Network::Devnet] {
        let params = with_env(&[], || {
            // Clear both so an ambient value from the developer's shell cannot
            // make this pass or fail for reasons unrelated to the tree.
            std::env::remove_var(VAR_22);
            std::env::remove_var(VAR_20);
            load_from_env(network)
        });
        assert!(
            params.inc_i_176_auth_binding_activation_height
                >= params.maintainer_derivation_activation_height,
            "{network:?}: #22 ({}) must be >= #20 ({}) on the RUNTIME path, not only \
             in defaults()",
            params.inc_i_176_auth_binding_activation_height,
            params.maintainer_derivation_activation_height
        );
    }
}

// ===========================================================================
// AUDIT-P2-102 (F13) — the refusal must name the variable the operator SET
// ===========================================================================

/// AUDIT-P2-102. O9 (the blame channel, previously absent).
///
/// The old refusal blamed `DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT` on both
/// legs, including the leg where that variable was never set and the operator had
/// only raised `#20`. The log is not capturable in this tree (O8), so the DECISION
/// is exposed as a pure function and asserted here instead of asserting the string.
///
/// The discriminator is `requested != compiled`: at the call site in
/// `env_loader::load_from_env`, an `.env` override is the only thing that can make
/// those two differ.
#[test]
fn f13_the_refusal_blames_the_variable_the_operator_actually_set() {
    assert_eq!(
        blame_for(0, 300),
        InversionBlame::AuthBindingOverride,
        "the operator moved #22 itself (requested 0 != compiled 300), so the \
         diagnostic must name DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT"
    );
    assert_eq!(
        blame_for(20, 20),
        InversionBlame::DerivationOverride,
        "#22 is UNTOUCHED at its compiled default (requested == compiled == 20), so \
         the inversion can only have come from raising #20. Naming #22 here sends \
         the reader to a line of .env they never wrote — AUDIT-P2-102."
    );
}

/// AUDIT-P2-102, the other half: the substituted value must not be presented as
/// something the operator asked for.
///
/// This pins the FACT the second message is obliged to state — that on the
/// derivation leg the returned height is neither the operator's value nor the
/// compiled default. If this equality ever became true, the message's claim ("a
/// value you never supplied") would be false and must be rewritten with it.
#[test]
fn f13_on_the_derivation_leg_the_result_is_neither_input() {
    let compiled_22 = 20;
    let raised_20 = 1_000;

    let got = enforce_auth_binding_above_derivation(compiled_22, compiled_22, raised_20);

    assert_eq!(got, raised_20);
    assert_ne!(
        got, compiled_22,
        "AUDIT-P2-102: the compiled #22 is inside the forbidden band, so it is NOT \
         what the node runs. The refusal message must say this out loud rather than \
         silently rewriting a value the operator never supplied."
    );
    assert_eq!(
        blame_for(compiled_22, compiled_22),
        InversionBlame::DerivationOverride
    );
}

// ===========================================================================
// AUDIT-P2-105 (F16) — why ONLY the lower bound is runtime-enforceable
// ===========================================================================

/// AUDIT-P2-105. The reasoning behind the missing upper bound, asserted rather
/// than argued.
///
/// The recorded upper half of REV-176-M1a-001 is `#22 <= #21`. It cannot be a
/// runtime guard because a SHIPPED, AUDITED default still violates it — devnet,
/// as a user-decided exemption. A guard would fire on every devnet boot and
/// refuse the audited configuration.
///
/// NARROWED 2026-08-25. Testnet used to violate it too (`#22 = 300_000` above a
/// crossed `#21 = 136_431`, with no satisfying value above the tip). The genesis
/// reset re-pinned both to `15_087`, and mainnet was pinned to `317_861` at the
/// 6.25.0 release, so those two now SATISFY the bound at equality. Devnet is the
/// only remaining blocker. This test goes red the moment devnet aligns too — and
/// at that moment the upper bound BECOMES enforceable and should be added.
#[test]
fn f16_the_upper_bound_is_not_runtime_enforceable_because_the_shipped_defaults_break_it() {
    // Testnet and mainnet now satisfy the bound; only devnet keeps it unenforceable.
    for (net, p) in [
        (Network::Testnet, NetworkParams::defaults(Network::Testnet)),
        (Network::Mainnet, NetworkParams::defaults(Network::Mainnet)),
    ] {
        assert!(
            p.inc_i_176_auth_binding_activation_height <= p.inc_i_173_activation_height,
            "{:?} #22 ({}) must now sit at or below #21 ({}) — the re-pin removed \
             this network from the exception list",
            net,
            p.inc_i_176_auth_binding_activation_height,
            p.inc_i_173_activation_height
        );
    }

    let d = NetworkParams::defaults(Network::Devnet);
    assert!(
        d.inc_i_176_auth_binding_activation_height > d.inc_i_173_activation_height,
        "premise: devnet #22 ({}) is ABOVE #21 ({}), the user-decided exemption. \
         Same conclusion.",
        d.inc_i_176_auth_binding_activation_height,
        d.inc_i_173_activation_height
    );
}

/// AUDIT-P2-105 — the RESIDUAL, made visible instead of silent.
///
/// The hazard the finding actually names is RETROACTIVITY, not the `<= #21` bound:
/// testnet `#22 = 127_200` satisfies `#22 >= #20` and IS accepted here, yet it sits
/// below already-mined governance history (the real `add_maintainer` at block
/// `136_690`) and would re-bind it to bytes no archived signature covers.
///
/// This test asserts that the guard does NOT catch it — a known, recorded hole, not
/// a claim of safety. "Already crossed?" is a function of the LIVE CHAIN TIP, and
/// this module is reached from `NetworkParams::load` before any block store exists,
/// so the property is not evaluable at this call site at all. The real check is a
/// tip-aware startup check (M4/rollout item) plus the dated staleness tripwire in
/// `crates/core/tests/inc_i_176_m2_activation_height.rs`.
#[test]
fn f16_a_retroactive_but_ordering_respecting_override_is_accepted_known_residual() {
    let (gate20, _) = compiled(Network::Testnet);
    let retroactive = gate20; // 127_200 — legal for the ordering, below mined history

    let got = with_env(&[(VAR_22, &retroactive.to_string())], || {
        load_from_env(Network::Testnet).inc_i_176_auth_binding_activation_height
    });

    assert_eq!(
        got, retroactive,
        "AUDIT-P2-105, RECORDED RESIDUAL: this guard enforces the ordering only. A \
         value that respects `#22 >= #20` but sits below already-mined governance \
         history is ACCEPTED here, because the live tip is not available at this \
         call site. If this ever starts failing, a retroactivity check was added — \
         update the residual in ordering.rs and in milestone-progress.md."
    );
}

/// IP-10. O4. Unit test of U2 in isolation.
///
/// Distinguishes "fall back to the compiled default" from "fail closed to at
/// least #20". They coincide on every shipped network and diverge only when #20
/// was itself raised, which is exactly the case a naive fallback gets wrong.
#[test]
fn f4_the_substitution_is_fail_closed_not_merely_the_compiled_default() {
    // Ordering respected: passthrough, both at the edge and above it.
    assert_eq!(enforce_auth_binding_above_derivation(100, 50, 100), 100);
    assert_eq!(enforce_auth_binding_above_derivation(101, 50, 100), 101);

    // Violated, compiled default is itself legal: fall back to it.
    assert_eq!(enforce_auth_binding_above_derivation(0, 300, 100), 300);

    // Violated AND the compiled default is also inside the band: clamp UP to #20.
    assert_eq!(
        enforce_auth_binding_above_derivation(0, 20, 1_000),
        1_000,
        "a compiled default below the effective #20 must not be handed back — it \
         is inside the forbidden band"
    );
}
