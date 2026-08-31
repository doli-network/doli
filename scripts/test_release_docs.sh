#!/usr/bin/env bash
# OUTPUT CONTRACT: release documentation static assertions (INC-I-202 M3, REQ-202-006)
#   O1 exit code           — 0 only when EVERY assertion below passes
#   O2 assertion line      — one [PASS]/[FAIL] line per assertion, naming the exact literal
#                             or file that is missing/offending (a failure must be actionable
#                             without re-reading the suite)
#   O3 offending file:line — for the negative assertions (S3, S4, S5, S6), the concrete
#                             `file:line: text` that violates the rule
#   O4 candidate-file set  — the file list actually scanned by S4/S5, printed verbatim so a
#                             future reader can see the scope the negative assertions cover
#   PATHS: test_release_docs.sh (no argv)
#            -> resolve PROJECT_ROOT from BASH_SOURCE (cwd-independent)
#            -> S1/S2 positive literal presence in the two release runbooks
#            -> S3 negative: pre-rotation key names absent from the two runbooks
#            -> S4/S5 negative: sweep the candidate set line by line
#            -> S6 cross-reference targets exist on disk
#            -> S7 SKILLS-INDEX release row mentions signing
#            -> any failure ? exit 1 : exit 0
# INPUT PARTITIONS (REQ-202-006, Must):
#   S1: .claude/skills/release/SKILL.md names the machinery         — O1 O2
#   S2: docs/releases.md names the machinery                        — O1 O2
#   S3: neither runbook references a pre-rotation key name          — O1 O2 O3
#   S4: no live runbook line pairs a pre-rotation key with signing  — O1 O2 O3 O4
#   S5: no live runbook claims CI publication ends the release      — O1 O2 O3 O4
#   S6: every .claude/skills/<n>/SKILL.md cross-ref resolves        — O1 O2 O3
#   S7: SKILLS-INDEX row `| release |` mentions signing             — O1 O2
# MATRIX: 4 outputs x 7 partitions (only cells the path reaches are asserted)
#   S1: O1 O2 | S2: O1 O2 | S3: O1 O2 O3 | S4: O1 O2 O3 O4
#   S5: O1 O2 O3 O4 | S6: O1 O2 O3 | S7: O1 O2
#
# TDD RED tests for REQ-202-006: the signing + promotion procedure must be impossible to miss
# when following the docs. This suite READS FILES ONLY — no network, no `gh`, no `doli`, no
# `cargo`, no `git`. It is deterministic and runnable from any working directory.
# Bash 3.2 compatible (macOS default): no associative arrays, no mapfile.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

SKILL_RELEASE="$PROJECT_ROOT/.claude/skills/release/SKILL.md"
RELEASES_DOC="$PROJECT_ROOT/docs/releases.md"
SKILLS_INDEX="$PROJECT_ROOT/.claude/skills/SKILLS-INDEX.md"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

TESTS_PASSED=0
TESTS_FAILED=0
TESTS_TOTAL=0

print_header() {
    echo
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}========================================${NC}"
    echo
}

test_result() {
    local test_name=$1
    local result=$2
    local detail=$3
    TESTS_TOTAL=$((TESTS_TOTAL + 1))
    if [ "$result" = "pass" ]; then
        TESTS_PASSED=$((TESTS_PASSED + 1))
        echo -e "  ${GREEN}[PASS]${NC} $test_name"
    else
        TESTS_FAILED=$((TESTS_FAILED + 1))
        echo -e "  ${RED}[FAIL]${NC} $test_name"
        if [ -n "$detail" ]; then
            echo -e "         ${RED}$detail${NC}"
        fi
    fi
}

# Path shown to the operator: repo-relative, never the machine-specific prefix.
rel() {
    echo "${1#"$PROJECT_ROOT"/}"
}

# Collapse multi-line grep output into one detail field (O3).
join_hits() {
    awk 'NF {printf "%s%s", sep, $0; sep=" ;; "} END {printf "\n"}'
}

# Positive assertion: $file contains the literal $needle.
assert_contains() {
    local label=$1 file=$2 needle=$3
    if [ ! -f "$file" ]; then
        test_result "$label" "fail" "file does not exist: $(rel "$file")"
        return
    fi
    if grep -qF -- "$needle" "$file"; then
        test_result "$label" "pass"
    else
        test_result "$label" "fail" "$(rel "$file") does not contain: $needle"
    fi
}

# Positive assertion: $file matches the case-insensitive ERE $pattern.
assert_matches_i() {
    local label=$1 file=$2 pattern=$3
    if [ ! -f "$file" ]; then
        test_result "$label" "fail" "file does not exist: $(rel "$file")"
        return
    fi
    if grep -qiE -- "$pattern" "$file"; then
        test_result "$label" "pass"
    else
        test_result "$label" "fail" "$(rel "$file") never matches (case-insensitive): $pattern"
    fi
}

# Negative assertion: $file has ZERO lines matching the ERE $pattern; offenders go to O3.
assert_no_match() {
    local label=$1 file=$2 pattern=$3
    local hits
    if [ ! -f "$file" ]; then
        test_result "$label" "fail" "file does not exist: $(rel "$file")"
        return
    fi
    hits=$(grep -nE -- "$pattern" "$file" 2>/dev/null | join_hits)
    if [ -z "$(echo "$hits" | tr -d '[:space:]')" ]; then
        test_result "$label" "pass"
    else
        test_result "$label" "fail" "$(rel "$file"): $hits"
    fi
}

# O4 — the LIVE runbook surface swept by S4 and S5.
# Top-level docs/*.md only (NOT recursive) + every skill SKILL.md + the two READMEs.
# Historical incident/audit archives under docs/{bugfixes,reviews,qa,audits,postmortems,
# redesigns,legacy,.workflow,announcements} are records of what WAS true and are deliberately
# out of scope: they are never followed as a procedure.
candidate_files() {
    ls "$PROJECT_ROOT"/docs/*.md 2>/dev/null
    ls "$PROJECT_ROOT"/.claude/skills/*/SKILL.md 2>/dev/null
    [ -f "$PROJECT_ROOT/scripts/README.md" ] && echo "$PROJECT_ROOT/scripts/README.md"
    [ -f "$PROJECT_ROOT/README.md" ] && echo "$PROJECT_ROOT/README.md"
    return 0
}

# Superseded by .claude/skills/cli/SKILL.md; kept only as an archive of the old CLI surface.
is_excluded() {
    case "$1" in
        */.claude/skills/cli/SKILL-legacy.md) return 0 ;;
    esac
    return 1
}

# INC-I-175 leaked / pre-rotation maintainer key names. The private halves of these are
# committed in this repo, so a runbook must never point a signer at them.
PRE_ROTATION_RE='\.doli/mainnet/keys|producer_[0-9]|producer_\{1'
# Narrower form for the per-line pairing in S4.
PRE_ROTATION_KEY_RE='\.doli/mainnet/keys/producer_|producer_[0-9]\.json|producer_\{1'
# Release-signing context. Producer-WALLET examples (balance/send) use the same key names
# legitimately; they are a defect only when they land on a signing line.
SIGNING_CONTEXT_RE='release sign|sign-release|SIGNATURES|--version v|maintainer sign'
# Claims that CI publication is the end of the release. It is not: CI creates a DRAFT.
CI_PUBLISHES_RE='GitHub Actions (automatically )?(publishes|will publish)|[Pp]ublished (to GitHub )?automatically|release is (now )?(public|live|published) (once|when|as soon as) (CI|GitHub Actions|the workflow)'

print_header "REQ-202-006 — RELEASE DOCUMENTATION STATIC ASSERTIONS"
echo -e "  ${CYAN}project root:${NC} $PROJECT_ROOT"
echo -e "  ${CYAN}mode:${NC} read-only (no network, no gh, no doli, no cargo, no git)"

# ============================================================
# O4 — candidate-file set actually scanned by S4/S5.
# ============================================================
print_header "O4 — CANDIDATE FILE SET (S4/S5 SCAN SCOPE)"
CANDIDATE_COUNT=0
while IFS= read -r cf; do
    [ -n "$cf" ] || continue
    if is_excluded "$cf"; then
        echo -e "  ${CYAN}[skip]${NC} $(rel "$cf")  (superseded archive)"
        continue
    fi
    CANDIDATE_COUNT=$((CANDIDATE_COUNT + 1))
    echo "  $(rel "$cf")"
done <<< "$(candidate_files)"
echo
echo -e "  ${CYAN}scanned:${NC} $CANDIDATE_COUNT files"

# ============================================================
# S1 — REQ-202-006 (Must) — Decision: a failure reveals that the release skill, the file an
# agent loads to CUT a release, never names the signing/promotion machinery — so the agent
# stops at the tag push and ships a DRAFT no node can download.
# ============================================================
print_header "S1 — RELEASE SKILL NAMES THE MACHINERY"
assert_contains "S1a release_skill_names_sign_release_script" "$SKILL_RELEASE" "sign-release.sh"
assert_contains "S1b release_skill_names_verify_command" "$SKILL_RELEASE" "doli release verify"
assert_contains "S1c release_skill_names_publish_release_script" "$SKILL_RELEASE" "publish-release.sh"
# shellcheck disable=SC2088  # literal string searched for in the doc, not a path to open
assert_contains "S1d release_skill_names_rotated_key_dir" "$SKILL_RELEASE" "~/.ssh/doli/maintainer-"
assert_matches_i "S1e release_skill_states_the_draft_state" "$SKILL_RELEASE" "draft"

# ============================================================
# S2 — REQ-202-006 (Must) — Decision: a failure reveals that the human-facing release runbook
# omits a step of the signing chain, so a human following it end to end still leaves the
# release unsigned, unverified, unpromoted, or unmonitored.
# ============================================================
print_header "S2 — RELEASES.MD NAMES THE MACHINERY"
assert_contains "S2a releases_doc_names_sign_release_script" "$RELEASES_DOC" "sign-release.sh"
assert_contains "S2b releases_doc_names_verify_command" "$RELEASES_DOC" "doli release verify"
assert_contains "S2c releases_doc_names_publish_release_script" "$RELEASES_DOC" "publish-release.sh"
assert_contains "S2d releases_doc_names_monitor_script" "$RELEASES_DOC" "monitor-release-signed.sh"
# shellcheck disable=SC2088  # literal string searched for in the doc, not a path to open
assert_contains "S2e releases_doc_names_rotated_key_dir" "$RELEASES_DOC" "~/.ssh/doli/maintainer-"

# ============================================================
# S3 — REQ-202-006 (Must) — Decision: a failure reveals that a release runbook still points a
# signer at an INC-I-175 pre-rotation key whose private half is committed in this repo, which
# would sign a release with a publicly known key and hand anyone a valid update manifest.
# ============================================================
print_header "S3 — RUNBOOKS CARRY NO PRE-ROTATION KEY REFERENCE"
assert_no_match "S3a release_skill_has_no_pre_rotation_key_reference" "$SKILL_RELEASE" "$PRE_ROTATION_RE"
assert_no_match "S3b releases_doc_has_no_pre_rotation_key_reference" "$RELEASES_DOC" "$PRE_ROTATION_RE"

# ============================================================
# S4 — REQ-202-006 (Must) — Decision: a failure reveals a LIVE runbook (not an archive) whose
# own copy-pasteable signing command carries a pre-rotation key name, so an operator following
# any doc in the live surface signs with a leaked key.
# ============================================================
print_header "S4 — NO LIVE RUNBOOK PAIRS A PRE-ROTATION KEY WITH RELEASE SIGNING"
S4_HITS=""
while IFS= read -r cf; do
    [ -n "$cf" ] || continue
    is_excluded "$cf" && continue
    [ -f "$cf" ] || continue
    line_hits=$(grep -nE -- "$PRE_ROTATION_KEY_RE" "$cf" 2>/dev/null | grep -E -- "$SIGNING_CONTEXT_RE")
    [ -n "$line_hits" ] || continue
    while IFS= read -r lh; do
        [ -n "$lh" ] || continue
        S4_HITS="$S4_HITS$(rel "$cf"):$lh
"
    done <<< "$line_hits"
done <<< "$(candidate_files)"

if [ -z "$(echo "$S4_HITS" | tr -d '[:space:]')" ]; then
    test_result "S4 no_live_runbook_signs_with_a_pre_rotation_key" "pass"
else
    test_result "S4 no_live_runbook_signs_with_a_pre_rotation_key" "fail" \
        "$(echo "$S4_HITS" | join_hits)"
fi

# ============================================================
# S5 — REQ-202-006 (Must) — Decision: a failure reveals a live doc that tells the reader the
# release is done when CI finishes. CI creates a DRAFT: believing that claim ends the release
# with nothing reachable by `doli upgrade` and no signal that anything is missing.
# ============================================================
print_header "S5 — NO LIVE RUNBOOK CLAIMS CI PUBLICATION ENDS THE RELEASE"
S5_HITS=""
while IFS= read -r cf; do
    [ -n "$cf" ] || continue
    is_excluded "$cf" && continue
    [ -f "$cf" ] || continue
    line_hits=$(grep -nE -- "$CI_PUBLISHES_RE" "$cf" 2>/dev/null)
    [ -n "$line_hits" ] || continue
    while IFS= read -r lh; do
        [ -n "$lh" ] || continue
        S5_HITS="$S5_HITS$(rel "$cf"):$lh
"
    done <<< "$line_hits"
done <<< "$(candidate_files)"

if [ -z "$(echo "$S5_HITS" | tr -d '[:space:]')" ]; then
    test_result "S5a no_live_runbook_claims_ci_publishes_the_release" "pass"
else
    test_result "S5a no_live_runbook_claims_ci_publishes_the_release" "fail" \
        "$(echo "$S5_HITS" | join_hits)"
fi

assert_contains "S5b releases_doc_states_the_draft_state_explicitly" "$RELEASES_DOC" "DRAFT"

# The promotion step must be marked as blocking in at least two places in the checklist,
# not mentioned once as an aside.
if [ -f "$RELEASES_DOC" ]; then
    BLOCKING_COUNT=$(grep -cF "BLOCKING" "$RELEASES_DOC")
    if [ "$BLOCKING_COUNT" -ge 2 ]; then
        test_result "S5c releases_doc_marks_signing_steps_blocking_at_least_twice" "pass"
    else
        test_result "S5c releases_doc_marks_signing_steps_blocking_at_least_twice" "fail" \
            "$(rel "$RELEASES_DOC") has $BLOCKING_COUNT 'BLOCKING' markers, need >= 2"
    fi
else
    test_result "S5c releases_doc_marks_signing_steps_blocking_at_least_twice" "fail" \
        "file does not exist: $(rel "$RELEASES_DOC")"
fi

# ============================================================
# S6 — REQ-202-006 (Must) — Decision: a failure reveals a runbook that hands the reader off to
# a skill file that does not exist, so the procedure dead-ends at the exact point where it
# delegates the remaining steps.
# ============================================================
print_header "S6 — RELEASE RUNBOOK CROSS-REFERENCES RESOLVE ON DISK"
assert_skill_refs_resolve() {
    local label=$1 file=$2
    local refs ref dangling ln
    if [ ! -f "$file" ]; then
        test_result "$label" "fail" "file does not exist: $(rel "$file")"
        return
    fi
    refs=$(grep -oE '\.claude/skills/[A-Za-z0-9_.-]+/SKILL\.md' "$file" 2>/dev/null | sort -u)
    dangling=""
    for ref in $refs; do
        [ -f "$PROJECT_ROOT/$ref" ] && continue
        ln=$(grep -nF -- "$ref" "$file" | head -1 | cut -d: -f1)
        dangling="$dangling$(rel "$file"):$ln: dangling -> $ref
"
    done
    if [ -z "$(echo "$dangling" | tr -d '[:space:]')" ]; then
        test_result "$label" "pass"
    else
        test_result "$label" "fail" "$(echo "$dangling" | join_hits)"
    fi
}
assert_skill_refs_resolve "S6a release_skill_skill_references_all_exist" "$SKILL_RELEASE"
assert_skill_refs_resolve "S6b releases_doc_skill_references_all_exist" "$RELEASES_DOC"

# ============================================================
# S7 — REQ-202-006 (Must) — Decision: a failure reveals that the grep-first skill index
# describes the release skill without the word "sign", so an agent searching the index for
# release signing never finds the skill that owns it.
# ============================================================
print_header "S7 — SKILLS-INDEX RELEASE ROW MENTIONS SIGNING"
if [ -f "$SKILLS_INDEX" ]; then
    RELEASE_ROW=$(grep -E '^\| *release *\|' "$SKILLS_INDEX" | head -1)
    if [ -z "$RELEASE_ROW" ]; then
        test_result "S7 skills_index_release_row_mentions_signing" "fail" \
            "$(rel "$SKILLS_INDEX") has no manifest row starting '| release |'"
    elif echo "$RELEASE_ROW" | grep -qiE "sign"; then
        test_result "S7 skills_index_release_row_mentions_signing" "pass"
    else
        test_result "S7 skills_index_release_row_mentions_signing" "fail" \
            "row never says 'sign': $RELEASE_ROW"
    fi
else
    test_result "S7 skills_index_release_row_mentions_signing" "fail" \
        "file does not exist: $(rel "$SKILLS_INDEX")"
fi

# ============================================================
print_header "TEST SUMMARY"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo -e "  Total Tests:  $TESTS_TOTAL"
echo

if [ "$TESTS_FAILED" -eq 0 ]; then
    EXIT_CODE=0
else
    EXIT_CODE=1
fi

exit $EXIT_CODE
