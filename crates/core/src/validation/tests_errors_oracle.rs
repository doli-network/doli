//! Tests for the Phase 2.1 oracle error message templates.
//!
//! Spec: specs/oracle-structural-anchored-economics.md §1.1, §1.8
//!
//! Three-question gate (INC-I-075) for THIS commit (ME1):
//!   Q1: Can any user-submittable tx trigger this code path? NO
//!       (no production caller exists until M4 + M8 land).
//!   Q2: Can any producer-action or attestation pattern trigger it? NO
//!       (same as Q1 — no caller).
//!   Q3: Bit-identical to old behavior for ALL reachable inputs? YES
//!       (no production code path reads or writes these constants in
//!       this commit; they are reachable only from tests).
//!   VERDICT: activation height NOT required for this commit. When M4
//!            and M8 land, the existing `oracle_activation_height`
//!            (committed in d80f127f) gates the production paths that
//!            will emit these messages.
//!
//! These tests guard the SHAPE of each constant so that downstream
//! emission sites (M4, M8) and agentic consumers grepping for the
//! `[ERRTX-ORACLE00X]` prefix can rely on a stable contract:
//!   - each constant carries the correct `[ERRTX-ORACLE00X]` prefix,
//!   - each constant names every field that the corresponding spec
//!     section requires the message to carry,
//!   - the three prefixes are pairwise distinct,
//!   - the three constants are not the empty string.
//!
//! These tests MUST fail to compile / fail to assert before
//! `errors_oracle.rs` lands, and pass once it does. They follow the
//! same inline-string convention used by `[ERRTX-HTLC001]`,
//! `[ERRTX-EC000]`, and `[ERRTX-DEFI001]` already present in the
//! codebase — the constants are documentation + grep targets, and
//! callers at M4/M8 will substitute the `{field}` placeholders via
//! their own `format!` at the emission site.

use super::errors_oracle::{ERRTX_ORACLE_001, ERRTX_ORACLE_002, ERRTX_ORACLE_003};

// OUTPUT CONTRACT: each `ERRTX_ORACLE_00X: &str` constant
//   O1: nonempty
//   O2: starts with the spec-defined `[ERRTX-ORACLE00X]` prefix
//   O3: contains every field-placeholder token that the spec section
//       requires the message to carry (`{current_height}` /
//       `{activation_height}` / `{attester}` / `{epoch}` / `{pair_id}` /
//       `{share_bps}`)
// PATHS:
//   P1: ERRTX_ORACLE_001 (HeightGate template)
//   P2: ERRTX_ORACLE_002 (DuplicateAttestation template)
//   P3: ERRTX_ORACLE_003 (SunsetTriggered template)
// INPUT PARTITIONS:
//   Each constant is a compile-time `&'static str` — no runtime input,
//   so there is exactly ONE partition per path (the constant itself).
// MATRIX: 3 outputs × 3 paths × 1 partition = 9 cells
//   P1×part-1: O1✓ O2✓ O3✓
//   P2×part-1: O1✓ O2✓ O3✓
//   P3×part-1: O1✓ O2✓ O3✓
#[test]
fn test_errtx_oracle_001_template_carries_prefix_and_fields() {
    assert!(!ERRTX_ORACLE_001.is_empty());
    assert!(
        ERRTX_ORACLE_001.starts_with("[ERRTX-ORACLE001]"),
        "missing ERRTX-ORACLE001 prefix: {ERRTX_ORACLE_001}"
    );
    assert!(
        ERRTX_ORACLE_001.contains("{current_height}"),
        "missing {{current_height}} placeholder: {ERRTX_ORACLE_001}"
    );
    assert!(
        ERRTX_ORACLE_001.contains("{activation_height}"),
        "missing {{activation_height}} placeholder: {ERRTX_ORACLE_001}"
    );
}

#[test]
fn test_errtx_oracle_002_template_carries_prefix_and_fields() {
    assert!(!ERRTX_ORACLE_002.is_empty());
    assert!(
        ERRTX_ORACLE_002.starts_with("[ERRTX-ORACLE002]"),
        "missing ERRTX-ORACLE002 prefix: {ERRTX_ORACLE_002}"
    );
    assert!(
        ERRTX_ORACLE_002.contains("{attester}"),
        "missing {{attester}} placeholder: {ERRTX_ORACLE_002}"
    );
    assert!(
        ERRTX_ORACLE_002.contains("{epoch}"),
        "missing {{epoch}} placeholder: {ERRTX_ORACLE_002}"
    );
    assert!(
        ERRTX_ORACLE_002.contains("{pair_id}"),
        "missing {{pair_id}} placeholder: {ERRTX_ORACLE_002}"
    );
}

#[test]
fn test_errtx_oracle_003_template_carries_prefix_and_fields() {
    assert!(!ERRTX_ORACLE_003.is_empty());
    assert!(
        ERRTX_ORACLE_003.starts_with("[ERRTX-ORACLE003]"),
        "missing ERRTX-ORACLE003 prefix: {ERRTX_ORACLE_003}"
    );
    assert!(
        ERRTX_ORACLE_003.contains("{share_bps}"),
        "missing {{share_bps}} placeholder: {ERRTX_ORACLE_003}"
    );
    assert!(
        ERRTX_ORACLE_003.contains("threshold_bps=5500"),
        "missing literal sunset threshold (basis points): {ERRTX_ORACLE_003}"
    );
}

// OUTPUT CONTRACT: pairwise-uniqueness invariant across the three constants
//   O1: each constant begins with a distinct `[ERRTX-ORACLE00X]` token
//       (no two constants share a prefix, no two constants are equal)
// PATHS:
//   P1: full pairwise comparison across the 3 constants
// INPUT PARTITIONS:
//   Each constant is its own compile-time partition.
// MATRIX: 1 output × 1 path × 1 partition = 1 cell (asserts the
//   pairwise-distinct property over all 3 constants in a single sweep)
#[test]
fn test_errtx_oracle_prefixes_are_pairwise_unique() {
    let prefixes = [
        ("ERRTX_ORACLE_001", "[ERRTX-ORACLE001]", ERRTX_ORACLE_001),
        ("ERRTX_ORACLE_002", "[ERRTX-ORACLE002]", ERRTX_ORACLE_002),
        ("ERRTX_ORACLE_003", "[ERRTX-ORACLE003]", ERRTX_ORACLE_003),
    ];
    for (name_i, prefix_i, _) in prefixes {
        for (name_j, _, value_j) in prefixes {
            if name_i == name_j {
                continue;
            }
            assert!(
                !value_j.starts_with(prefix_i),
                "prefix {prefix_i} from {name_i} collides with {name_j}: {value_j}"
            );
        }
    }
    assert_ne!(ERRTX_ORACLE_001, ERRTX_ORACLE_002);
    assert_ne!(ERRTX_ORACLE_002, ERRTX_ORACLE_003);
    assert_ne!(ERRTX_ORACLE_001, ERRTX_ORACLE_003);
}
