//! Runtime enforcement of the ONE activation-height ordering that is
//! SECURITY-CRITICAL: `#22 >= #20`.
//!
//! INC-I-176 M2 review **F4**. `NetworkParams::defaults()` satisfies
//! `inc_i_176_auth_binding_activation_height (#22) >=
//! maintainer_derivation_activation_height (#20)` on all three networks, and six
//! ordering tests assert exactly that. But `defaults()` is not the value the node
//! runs on: on testnet and devnet both heights pass through
//! [`super::env_loader::load_from_env`], where `DOLI_*` overrides can move either
//! one independently. Nothing checked the ORDER after the overrides were applied.
//!
//! # Why the order is security-critical (and one-sided)
//!
//! `#20` decides WHICH COUNTER verifies a maintainer authorization; `#22` decides
//! WHICH BYTES are verified. Below `#20` the historical **entry-counting** counter
//! is in force, where three signature entries from ONE key clear a 3-of-5
//! threshold (AUDIT-P0-010). Below `#22` the legacy, unbound
//! `format!("{}:{}", action, target_hex)` message is in force.
//!
//! With `#22 < #20` there is a band `[#22, #20)` in which the STRONGER,
//! chain-bound message is verified by the WEAKER, pre-INC-I-172 counter —
//! AUDIT-P1-016's binding live while AUDIT-P0-010's defect is re-armed underneath
//! it. `network_params/mod.rs` states this as unconditional on every network,
//! devnet included.
//!
//! # Why NO upper bound is enforced here — and what that leaves open
//!
//! Audit **AUDIT-P2-105**. The recorded upper half of REV-176-M1a-001 is
//! `#22 <= #21` (`inc_i_173_activation_height`). It is **not runtime-enforceable**,
//! and the reason is a measured property of the shipped defaults, not a preference:
//! the audited testnet default already VIOLATES it (`300_000 > 136_431`, an
//! accepted exception — `#21` is crossed, so no satisfying value exists above the
//! tip) and so does the audited devnet default (`20 > 0`, the user-decided
//! exemption). A runtime guard on `#22 <= #21` would therefore refuse the shipped,
//! audited configuration on two of the three networks, on every boot. Asserted, not
//! asserted-by-prose, in `tests_ordering.rs::f16_*`.
//!
//! **RESIDUAL, recorded rather than silently accepted.** The hazard AUDIT-P2-105
//! actually names is not the `<= #21` bound at all: it is RETROACTIVITY. On testnet
//! `DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT=127_200` satisfies `#22 >= #20`
//! and is accepted here, yet it sits below already-mined governance history and
//! would re-bind the real `add_maintainer` at block `136_690` to bytes no archived
//! signature covers. "Is this height already crossed?" is a function of the LIVE
//! CHAIN TIP, and this module is a leaf reached from `NetworkParams::load` before
//! any block store exists — the tip is not available at this call site, so the
//! property cannot be evaluated here at all. It is checkable only (a) by a
//! tip-aware startup check, recorded as an M4/rollout item in
//! `docs/.workflow/milestone-progress.md`, or (b) by the dated staleness tripwire
//! in `crates/core/tests/inc_i_176_m2_activation_height.rs`. Enforcing a static
//! stand-in ("never below the compiled default") was rejected: it would refuse the
//! legitimate downward re-pins that are legal while a height is uncrossed, and it
//! would break the devnet `#22 = 0` case the ordering rule deliberately admits.
//!
//! # Fail-closed, never fatal
//!
//! A refusal here must not become a new fatal startup path — a node that will not
//! boot because of an `.env` typo is a worse outcome than a node that boots with
//! the audited compiled value and says so loudly. So the violating value is
//! DISCARDED and a safe one substituted, at `error!`.

use tracing::error;

/// Which `.env` variable the operator ACTUALLY set to produce an inversion.
///
/// Audit **AUDIT-P2-102**: the refusal used to blame
/// `DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT` unconditionally, including on
/// the leg where the operator never set that variable at all and merely raised
/// `#20`. A diagnostic that names a variable the reader did not set sends them to
/// the wrong line of their `.env`.
///
/// The blame is derived from the values the guard already holds: at this call site
/// an `.env` override is the ONLY way `requested` can differ from `compiled`, so
/// `requested != compiled` is exactly "the operator moved `#22`".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InversionBlame {
    /// `#22` itself was overridden, to a value below the effective `#20`.
    AuthBindingOverride,
    /// `#22` was left at its compiled default; `#20` was raised above it. The
    /// substituted value is one the operator never supplied, and the log must say
    /// so.
    DerivationOverride,
}

/// Attribute an inversion to the variable that produced it. Pure; see
/// [`InversionBlame`].
pub(super) fn blame_for(requested: u64, compiled: u64) -> InversionBlame {
    if requested == compiled {
        InversionBlame::DerivationOverride
    } else {
        InversionBlame::AuthBindingOverride
    }
}

/// Enforce `#22 >= #20` on the value that will actually be used at runtime.
///
/// * `requested` — `#22` after the `DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT`
///   override has been applied.
/// * `compiled` — the compiled default for `#22` on this network.
/// * `effective_derivation` — `#20` **after** its own override
///   (`DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT`) has been applied. It must be
///   the post-override value: overriding `#20` UPWARD inverts the same ordering
///   without touching `#22` at all.
///
/// Mainnet never reaches this function — both heights are `is_mainnet`-locked to
/// their compiled values in `load_from_env`, and locked values are ordered by
/// construction.
///
/// # Path coverage
///
/// * `requested >= effective_derivation` → returned unchanged. The only path the
///   shipped defaults of all three networks take (mainnet `u64::MAX >= 172_000`,
///   testnet `300_000 >= 127_200`, devnet `20 >= 0`), and the only path any
///   ordering-respecting override takes.
/// * `requested < effective_derivation` → REFUSED at `error!`, and
///   `max(compiled, effective_derivation)` is used instead. The `max` is what makes
///   the substitution fail-CLOSED rather than merely "back to the default": when
///   the inversion was produced by raising `#20` above the compiled `#22`, the
///   compiled `#22` is itself in the forbidden band, so returning it would not
///   restore the ordering. Raising `#22` only ever keeps the weaker message form
///   in force for longer, which is the safe direction.
///   The refusal branches once more, on [`blame_for`], and the two messages differ
///   in WHICH variable they name and in whether they announce that a value the
///   operator never supplied is being replaced (AUDIT-P2-102). Both legs return the
///   same substituted height — the split is diagnostic only.
pub(super) fn enforce_auth_binding_above_derivation(
    requested: u64,
    compiled: u64,
    effective_derivation: u64,
) -> u64 {
    if requested >= effective_derivation {
        return requested;
    }

    let substituted = compiled.max(effective_derivation);
    let band = "That opens a band in which the chain-bound INC-I-176 authorization message is \
                verified by the pre-INC-I-172 ENTRY-COUNTING multisig counter, where three \
                signature entries from ONE key clear a 3-of-5 threshold (AUDIT-P0-010 re-armed \
                under AUDIT-P1-016). The #22 >= #20 ordering is UNCONDITIONAL on every network.";

    match blame_for(requested, compiled) {
        InversionBlame::AuthBindingOverride => error!(
            "REFUSED activation-height override: you set \
             DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT = {} (#22), which is BELOW the \
             effective maintainer_derivation_activation_height (#20) = {}. {} Using {} instead \
             — this node did NOT apply the value you asked for. Fix \
             DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT in the data directory's .env.",
            requested, effective_derivation, band, substituted
        ),
        InversionBlame::DerivationOverride => error!(
            "REFUSED activation-height configuration: you set \
             DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT = {} (#20). You did NOT set \
             DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT, so #22 is its COMPILED default {} \
             — which your #20 now sits above. {} This node is therefore running #22 = {}, a \
             value you never supplied: the compiled default was NOT used, because it is itself \
             inside the forbidden band. Either lower \
             DOLI_MAINTAINER_DERIVATION_ACTIVATION_HEIGHT back to at most {}, or set \
             DOLI_INC_I_176_AUTH_BINDING_ACTIVATION_HEIGHT explicitly to the #22 you intend.",
            effective_derivation, compiled, band, substituted, compiled
        ),
    }

    substituted
}
