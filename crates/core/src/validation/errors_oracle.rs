//! Phase 2.1 oracle error message templates.
//!
//! Spec: `specs/oracle-structural-anchored-economics.md` §1.1, §1.8.
//!
//! Each constant is the full error-message template that the
//! corresponding emission site produces. The constants are
//! documentation + grep targets — they are NOT consumed via
//! `format!(CONST, ...)` (which Rust forbids), and they are NOT
//! parsed at runtime. M4 (validation + mempool) and M8 (sunset
//! check) duplicate the same literal in their `format!(...)` call
//! and substitute the `{field}` placeholders inline. The duplication
//! is intentional: tests in `tests_errors_oracle.rs` pin the
//! template shape, so any drift between caller and template fails
//! CI loudly.
//!
//! Convention matches the existing `[ERRTX-DEFI001]` (utxo.rs:989),
//! `[ERRTX-HTLC001]` (transaction.rs:411), and `[ERRTX-EC000]`
//! (transaction.rs:642) sites: a stable `[ERRTX-...]` prefix
//! embedded inside a human-readable string, surfaced through
//! `ValidationError::InvalidTransaction(String)`.
//!
//! Predecessor commit: `d80f127f`
//! (`feat(core): NetworkParams.oracle_activation_height field (Phase
//! 2.1 Oracle M1)`).
//! Future consumers:
//!   - M4: `crates/core/src/validation/transaction.rs` and
//!     `crates/mempool/src/` — emit `ERRTX_ORACLE_001` +
//!     `ERRTX_ORACLE_002`.
//!   - M8: sunset check (epoch boundary + pre-emptive validation
//!     rejection) — emits `ERRTX_ORACLE_003`.
//!
//! INC-I-075 three-question gate (this commit only):
//!   Q1: user-submittable tx triggers? NO (no caller).
//!   Q2: producer-action / attestation pattern triggers? NO (no caller).
//!   Q3: bit-identical to old behavior? YES (constants are dead
//!       code; `#[allow(dead_code)]` keeps clippy quiet until M4 lands).

#![allow(dead_code)]

/// Fired by M4 when a `PriceAttestation` tx (TxType=16) hits
/// validation or mempool admission before
/// `oracle_activation_height`.
///
/// Spec: `oracle-structural-anchored-economics.md` §1.1, validation
/// rule #1 ("Height gate: reject if `current_height <
/// oracle_activation_height`").
///
/// Placeholders: `{current_height}`, `{activation_height}`.
pub const ERRTX_ORACLE_001: &str = "[ERRTX-ORACLE001] oracle not activated: current_height={current_height} activation_height={activation_height}";

/// Fired by M4 when an attester submits a second `PriceAttestation`
/// tx within the same `(epoch_number, pair_id)` tuple.
///
/// Spec: `oracle-structural-anchored-economics.md` §1.1, validation
/// rule #5 ("At most ONE attestation per attester per epoch per
/// pair. Reject duplicates with `[ERRTX-ORACLE002]`").
///
/// Placeholders: `{attester}` (hex pubkey), `{epoch}`, `{pair_id}`
/// (hex).
pub const ERRTX_ORACLE_002: &str =
    "[ERRTX-ORACLE002] duplicate attestation: attester={attester} epoch={epoch} pair_id={pair_id}";

/// Fired by M8 (sunset check at epoch boundary + pre-emptive
/// rejection in validation) when `structural_share < 0.55`.
///
/// Spec: `oracle-structural-anchored-economics.md` §1.8 ("When
/// `structural_share < 0.55`: oracle stops accepting new
/// `PriceAttestation` TXs at validation time (return
/// `[ERRTX-ORACLE003]`)").
///
/// `structural_share` is encoded as basis points (5500 = 55.00%) at
/// the emission site so the message stays integer-only and matches
/// the threshold representation used by the sunset check.
///
/// Placeholder: `{share_bps}`. The threshold (5500) is emitted as a
/// literal so consumers can confirm which sunset boundary fired
/// without computing the ratio.
pub const ERRTX_ORACLE_003: &str = "[ERRTX-ORACLE003] oracle sunset triggered: structural_share_bps={share_bps} threshold_bps=5500";
