#!/usr/bin/env bash
# ============================================================================
# INC-I-217 M11 — doc/spec alignment assertions for BLS key rotation
# ============================================================================
# WHY THIS IS A TEST. Code is the source of truth, so no assertion here can
# make the feature more correct. What it CAN do is decide whether the feature
# is reachable. M1-M10 shipped `TxType::RotateBlsKey = 32`, a CLI, an RPC
# surface and a metric; none of that is discoverable by a producer who does
# not read Rust. These pages are the only surface an operator can find the
# feature through, and two of them currently assert the OPPOSITE of the code
# (`specs/protocol.md` still says no TxType rotates a BLS key;
# `docs/error-codes.md` still calls ERRTX-ROT006 "RESERVED"). A doc that
# contradicts shipped code is not stale prose, it is a wrong answer handed to
# an operator whose node earns zero. The assertions below are the falsifiers.
#
# REQ-ROT-015 (Should) — Decision: whether a producer can find rotation from the shipped docs at all; a failure means the feature exists only in Rust and mismatched producers keep earning zero.
# REQ-ROT-016 (Should) — Decision: whether the INC-I-162 analysis still tells its reader the BLS path is retired, routing them to Exit + re-Registration (forfeits `registered_at` seniority) instead of `rotate-bls`.
# REQ-ROT-SEC-010 (Must) — Decision: whether the frozen `u64::MAX` gate and the census-before-pinning precondition stay visible, so nobody pins the activation height from a doc that says the feature is merely "open".
#
# OUTPUT CONTRACT
#   O1 exit code   — 0 only when every assertion passes
#   O2 result line — one [PASS]/[FAIL] per assertion id, with a human reason
#   O3 offender    — for negative assertions, the concrete file:line at fault
#   O4 summary     — `RESULT: N/M passed`
#
# SCOPE. Reads files, `git` and `.omega/memory.db` only. No network, no node,
# no `cargo`, no writes. Runnable from any directory. Bash 3.2 compatible.
# `set -e` is deliberately OFF: a failing grep is an assertion result, not an
# abort.
# ============================================================================
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

PROTOCOL_SPEC="$PROJECT_ROOT/specs/protocol.md"
ATT_SPEC="$PROJECT_ROOT/specs/attestation-bls-architecture.md"
SPECS_INDEX="$PROJECT_ROOT/specs/SPECS.md"
CLI_DOC="$PROJECT_ROOT/docs/cli.md"
RPC_DOC="$PROJECT_ROOT/docs/rpc_reference.md"
ERR_DOC="$PROJECT_ROOT/docs/error-codes.md"
PRODUCER_DOC="$PROJECT_ROOT/docs/becoming_a_producer.md"
RECOVERY_DOC="$PROJECT_ROOT/docs/bls-key-recovery.md"
DOCS_INDEX="$PROJECT_ROOT/docs/DOCS.md"
I162_DOC="$PROJECT_ROOT/docs/bugfixes/inc-i-162-wallet-bls-derivation-analysis.md"
ERR_RS="$PROJECT_ROOT/crates/core/src/validation/error.rs"
GAUNTLET_SH="$PROJECT_ROOT/scripts/gauntlet.sh"
GAUNTLET_SEED="$PROJECT_ROOT/scripts/gauntlet-seed.sql"
SCRIPTS_README="$PROJECT_ROOT/scripts/README.md"
MEMORY_DB="$PROJECT_ROOT/.omega/memory.db"

RED='\033[0;31m'; GREEN='\033[0;32m'; BLUE='\033[0;34m'; NC='\033[0m'
TESTS_PASSED=0; TESTS_FAILED=0; TESTS_TOTAL=0

print_header() { echo; echo -e "${BLUE}========================================${NC}"; echo -e "${BLUE}  $1${NC}"; echo -e "${BLUE}========================================${NC}"; echo; }

test_result() {
    TESTS_TOTAL=$((TESTS_TOTAL + 1))
    if [ "$2" = "pass" ]; then
        TESTS_PASSED=$((TESTS_PASSED + 1)); echo -e "  ${GREEN}[PASS]${NC} $1"
    else
        TESTS_FAILED=$((TESTS_FAILED + 1)); echo -e "  ${RED}[FAIL]${NC} $1"
        [ -n "${3:-}" ] && echo -e "         ${RED}$3${NC}"
    fi
}

rel() { echo "${1#"$PROJECT_ROOT"/}"; }

# $file must contain the fixed string $needle.
assert_contains() {
    local label=$1 file=$2 needle=$3
    if [ ! -f "$file" ]; then test_result "$label" "fail" "missing file: $(rel "$file")"; return; fi
    if grep -qF -- "$needle" "$file"; then test_result "$label" "pass"
    else test_result "$label" "fail" "$(rel "$file") never says: $needle"; fi
}

# $file must contain a line matching the ERE $re.
assert_regex() {
    local label=$1 file=$2 re=$3
    if [ ! -f "$file" ]; then test_result "$label" "fail" "missing file: $(rel "$file")"; return; fi
    if grep -qE -- "$re" "$file"; then test_result "$label" "pass"
    else test_result "$label" "fail" "$(rel "$file") has no line matching /$re/"; fi
}

# No line of $file may match the ERE $re; the offender is printed (O3).
assert_absent_regex() {
    local label=$1 file=$2 re=$3 hit
    if [ ! -f "$file" ]; then test_result "$label" "fail" "missing file: $(rel "$file")"; return; fi
    hit=$(grep -nE -- "$re" "$file" | head -2 | awk 'NF {printf "%s%s", sep, $0; sep=" ;; "}')
    if [ -z "$hit" ]; then test_result "$label" "pass"
    else test_result "$label" "fail" "$(rel "$file"):$hit"; fi
}

# ============================================================
print_header "1-2  specs/protocol.md — the RotateBlsKey section"
assert_contains "DOC-ROT-01a protocol_spec_has_rotate_section"     "$PROTOCOL_SPEC" 'RotateBlsKey (type 32)'
assert_regex    "DOC-ROT-01b protocol_spec_states_240_byte_payload" "$PROTOCOL_SPEC" '240[ -]byte'
assert_contains "DOC-ROT-01c1 protocol_spec_names_auth_domain_tag"  "$PROTOCOL_SPEC" 'DOLI-ROTATE-BLS-V1'
assert_contains "DOC-ROT-01c2 protocol_spec_names_pop_domain_tag"   "$PROTOCOL_SPEC" 'DOLI-ROTATE-POP-V1'
assert_contains "DOC-ROT-01d protocol_spec_names_activation_gate"   "$PROTOCOL_SPEC" 'bls_key_rotation_activation_height'
assert_absent_regex "DOC-ROT-02a protocol_spec_drops_no_rotation_claim" "$PROTOCOL_SPEC" 'No .?TxType.? among the 24 rotates a BLS key'
assert_absent_regex "DOC-ROT-02b protocol_spec_txtype_count_not_24"     "$PROTOCOL_SPEC" 'TxType.*\b24\b|\b24\b.*TxType'

# ============================================================
print_header "3-5  docs/ — CLI, RPC and the metric"
assert_contains "DOC-ROT-03a cli_doc_documents_rotate_bls" "$CLI_DOC" 'producer rotate-bls'
assert_contains "DOC-ROT-03b cli_doc_documents_import_bls" "$CLI_DOC" 'doli import-bls'
assert_contains "DOC-ROT-04a rpc_doc_documents_new_bls_pubkey"     "$RPC_DOC" 'newBlsPubkey'
assert_contains "DOC-ROT-04b rpc_doc_documents_effective_at_height" "$RPC_DOC" 'effectiveAtHeight'
assert_contains "DOC-ROT-04c rpc_doc_documents_update_type_string"  "$RPC_DOC" 'rotate_bls_key'

METRIC='doli_producer_bls_rotation_total'
METRIC_HITS=$(grep -rln "$METRIC" "$PROJECT_ROOT/docs" 2>/dev/null | grep -v '/\.workflow/' | head -3 | awk 'NF {printf "%s%s", sep, $0; sep=" ;; "}')
if [ -n "$METRIC_HITS" ]; then
    test_result "DOC-ROT-05 metric_documented_for_operators" "pass"
else
    test_result "DOC-ROT-05 metric_documented_for_operators" "fail" \
        "$METRIC appears in no operator doc (docs/.workflow/ excluded); the live metric table is docs/architecture.md, fallback docs/rpc_reference.md"
fi

# ============================================================
print_header "6    docs/error-codes.md — cross-checked against error.rs"
if [ ! -f "$ERR_DOC" ] || [ ! -f "$ERR_RS" ]; then
    test_result "DOC-ROT-06a error_code_table_matches_code" "fail" "missing $(rel "$ERR_DOC") or $(rel "$ERR_RS")"
else
    DOC_CODES=$(grep -oE '^\| .?ERRTX-ROT[0-9]{3}' "$ERR_DOC" | grep -oE 'ROT[0-9]{3}' | sort -u)
    RS_CODES=$(grep -oE '\[ERRTX-ROT[0-9]{3}\]' "$ERR_RS" | grep -oE 'ROT[0-9]{3}' | sort -u)
    ONLY_DOC=$(comm -23 <(echo "$DOC_CODES") <(echo "$RS_CODES") | tr '\n' ' ')
    ONLY_RS=$(comm -13 <(echo "$DOC_CODES") <(echo "$RS_CODES") | tr '\n' ' ')
    if [ -z "$DOC_CODES" ]; then
        test_result "DOC-ROT-06a error_code_table_matches_code" "fail" "$(rel "$ERR_DOC") has no ERRTX-ROT table rows at all"
    elif [ -z "${ONLY_DOC// /}" ] && [ -z "${ONLY_RS// /}" ]; then
        test_result "DOC-ROT-06a error_code_table_matches_code" "pass"
    else
        test_result "DOC-ROT-06a error_code_table_matches_code" "fail" \
            "doc-only (never emitted): [${ONLY_DOC}] | code-only (undocumented): [${ONLY_RS}]"
    fi
fi
assert_absent_regex "DOC-ROT-06b rot006_no_longer_reserved_for_m7" "$ERR_DOC" 'ROT006.*RESERVED|RESERVED for M7'

# ============================================================
print_header "7-8  operator recovery path"
assert_contains "DOC-ROT-07a producer_doc_names_log_symptom" "$PRODUCER_DOC" 'own BLS half does not verify'
assert_contains "DOC-ROT-07b producer_doc_names_import_remedy" "$PRODUCER_DOC" 'doli import-bls'
assert_contains "DOC-ROT-07c producer_doc_names_rotate_remedy" "$PRODUCER_DOC" 'doli producer rotate-bls'

if [ -f "$RECOVERY_DOC" ]; then
    RECOVERY_LINES=$(wc -l < "$RECOVERY_DOC" | tr -d ' ')
else
    RECOVERY_LINES=0
fi
if [ "$RECOVERY_LINES" -ge 100 ] && [ "$RECOVERY_LINES" -le 500 ]; then
    test_result "DOC-ROT-08a recovery_page_is_real_not_stub" "pass"
else
    test_result "DOC-ROT-08a recovery_page_is_real_not_stub" "fail" \
        "$(rel "$RECOVERY_DOC") is $RECOVERY_LINES lines; want 100..500 (13 = the architect stub)"
fi
assert_contains "DOC-ROT-08b recovery_page_links_producer_guide" "$RECOVERY_DOC" 'becoming_a_producer'
assert_contains "DOC-ROT-08c producer_guide_links_recovery_page" "$PRODUCER_DOC" 'bls-key-recovery'

# ============================================================
print_header "9-10 superseded analysis + attestation-BLS P4"
if [ ! -f "$I162_DOC" ]; then
    test_result "DOC-ROT-09a i162_top_points_at_inc_i_217" "fail" "missing file: $(rel "$I162_DOC")"
    test_result "DOC-ROT-09b i162_section6_points_at_inc_i_217" "fail" "missing file: $(rel "$I162_DOC")"
else
    if head -15 "$I162_DOC" | grep -q 'INC-I-217' && head -15 "$I162_DOC" | grep -qE '20[0-9]{2}-[0-9]{2}-[0-9]{2}'; then
        test_result "DOC-ROT-09a i162_top_points_at_inc_i_217" "pass"
    else
        test_result "DOC-ROT-09a i162_top_points_at_inc_i_217" "fail" \
            "first 15 lines lack a dated INC-I-217 resolution pointer"
    fi
    S6_LINE=$(grep -nE '^#{1,4} +(§)?6[\. ]' "$I162_DOC" | head -1 | cut -d: -f1)
    if [ -n "${S6_LINE:-}" ] && sed -n "${S6_LINE},$((S6_LINE + 12))p" "$I162_DOC" | grep -q 'INC-I-217'; then
        test_result "DOC-ROT-09b i162_section6_points_at_inc_i_217" "pass"
    else
        test_result "DOC-ROT-09b i162_section6_points_at_inc_i_217" "fail" \
            "no INC-I-217 pointer within 12 lines of the section-6 heading (heading line: ${S6_LINE:-not found})"
    fi
fi

P4_ROW=$(grep -n '^| P4 |' "$ATT_SPEC" 2>/dev/null | head -1)
if [ -z "$P4_ROW" ]; then
    test_result "DOC-ROT-10 att_spec_p4_states_shipped_and_frozen" "fail" "no '| P4 |' row in $(rel "$ATT_SPEC")"
elif echo "$P4_ROW" | grep -q 'bls_key_rotation_activation_height' && echo "$P4_ROW" | grep -q 'u64::MAX'; then
    test_result "DOC-ROT-10 att_spec_p4_states_shipped_and_frozen" "pass"
else
    test_result "DOC-ROT-10 att_spec_p4_states_shipped_and_frozen" "fail" \
        "P4 row names neither the gate field nor u64::MAX: $(echo "$P4_ROW" | cut -c1-150)"
fi

# ============================================================
print_header "11   GS-021 gauntlet registration"
assert_regex    "DOC-ROT-11a gauntlet_runner_dispatches_gs021" "$GAUNTLET_SH" '_gs021_assert'
assert_regex    "DOC-ROT-11b gauntlet_seed_carries_gs021"      "$GAUNTLET_SEED" "'GS-021'"
assert_contains "DOC-ROT-11c scripts_readme_has_gs021_row"     "$SCRIPTS_README" 'GS-021'

if [ ! -f "$MEMORY_DB" ]; then
    test_result "DOC-ROT-11d gs021_active_in_scenario_registry" "fail" "missing $(rel "$MEMORY_DB")"
else
    GS021_ACTIVE=$(sqlite3 "$MEMORY_DB" "SELECT count(*) FROM gauntlet_scenarios WHERE scenario_id='GS-021' AND status='active';" 2>/dev/null || echo 0)
    if [ "${GS021_ACTIVE:-0}" = "1" ]; then
        test_result "DOC-ROT-11d gs021_active_in_scenario_registry" "pass"
    else
        test_result "DOC-ROT-11d gs021_active_in_scenario_registry" "fail" \
            "gauntlet_scenarios has ${GS021_ACTIVE:-0} active GS-021 row(s), want 1"
    fi
fi

gs021_code() {
    awk '/^_gs021[A-Za-z0-9_]*\(\)/ {inf=1} inf {print} inf && /^\}/ {inf=0}' "$GAUNTLET_SH" 2>/dev/null
    for f in "$PROJECT_ROOT/scripts/"gauntlet-gs021*.sh; do [ -f "$f" ] && cat "$f"; done
    return 0
}
GS021_BODY=$(gs021_code | grep -vE '^[[:space:]]*#' | grep -vE '^[[:space:]]*$')
GS021_WRITES=$(echo "$GS021_BODY" | grep -nE 'submit|sendTransaction|pkill|rm -rf' | head -2 | awk 'NF {printf "%s%s", sep, $0; sep=" ;; "}')
if [ -z "${GS021_BODY// /}" ]; then
    test_result "DOC-ROT-11e gs021_is_observational_only" "fail" "no GS-021 code path exists yet (nothing to prove non-destructive)"
elif [ -z "$GS021_WRITES" ]; then
    test_result "DOC-ROT-11e gs021_is_observational_only" "pass"
else
    test_result "DOC-ROT-11e gs021_is_observational_only" "fail" "chain-writing token in the GS-021 path: $GS021_WRITES"
fi

# ============================================================
print_header "12-13 indexes, and the no-Rust fence"
DOCS_ROW=$(grep -n 'bls-key-recovery\.md' "$DOCS_INDEX" 2>/dev/null | head -1)
if [ -z "$DOCS_ROW" ]; then
    test_result "DOC-ROT-12a docs_index_row_for_recovery_page" "fail" "no bls-key-recovery.md row in $(rel "$DOCS_INDEX")"
elif echo "$DOCS_ROW" | grep -qi 'stub'; then
    test_result "DOC-ROT-12a docs_index_row_for_recovery_page" "fail" "row still calls the page a stub: $(echo "$DOCS_ROW" | cut -c1-140)"
else
    test_result "DOC-ROT-12a docs_index_row_for_recovery_page" "pass"
fi
assert_contains "DOC-ROT-12b1 specs_index_row_requirements"  "$SPECS_INDEX" 'bls-key-rotation-requirements.md'
assert_contains "DOC-ROT-12b2 specs_index_row_architecture"  "$SPECS_INDEX" 'bls-key-rotation-architecture.md'
assert_absent_regex "DOC-ROT-12c specs_index_discriminant_not_23" "$SPECS_INDEX" 'RotateBlsKey.*discriminant 23|discriminant 23.*RotateBlsKey'

RS_TOUCHED=$( { git -C "$PROJECT_ROOT" diff --name-only HEAD 2>/dev/null; git -C "$PROJECT_ROOT" ls-files --others --exclude-standard 2>/dev/null; } | grep -E '\.rs$' | sort -u | head -5 | awk 'NF {printf "%s%s", sep, $0; sep=" ;; "}')
if [ -z "$RS_TOUCHED" ]; then
    test_result "DOC-ROT-13 no_rust_file_touched_by_m11" "pass"
else
    test_result "DOC-ROT-13 no_rust_file_touched_by_m11" "fail" "M11 is docs-only; Rust paths dirty: $RS_TOUCHED"
fi

# ============================================================
print_header "TEST SUMMARY"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo "  RESULT: $TESTS_PASSED/$TESTS_TOTAL passed"
echo

[ "$TESTS_FAILED" -eq 0 ] && exit 0
exit 1
