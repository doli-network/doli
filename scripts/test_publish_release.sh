#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/publish-release.sh (INC-I-202 M2 + M2.5/REQ-202-007)
#   O1 exit code       — exit status of publish-release.sh                        — 0 only when verification passed
#   O2 gh mutation log — every `gh` invocation, checked for `release edit ... --draft=false --latest`
#   O3 operator message— combined stdout+stderr, checked for the refusal reason (0 count / missing file / usage)
#   O4 promoted notes  — content handed to `gh release edit ... --notes-file <path>` on promotion,
#                          checked for absence of the unsigned-draft banner and presence of the changelog
#   PATHS: publish-release.sh (argv check) -> `gh release download v<ver> --dir <tmp>`
#            -> `doli release verify --version v<ver> --dir <tmp>`
#            -> exit!=0 ? refuse (banner untouched) : strip DOLI-UNSIGNED-DRAFT-WARNING banner from the
#               draft's notes -> `gh release edit v<ver> --draft=false --latest --notes-file <stripped>`
# INPUT PARTITIONS:
#   S1: manifest present, `doli release verify` exits 1                 — O1!=0, O2 has NO promotion
#   S2: manifest present (3 entries), `doli release verify` exits 0     — O1=0, O2 has exactly one promotion
#   S3: downloaded SIGNATURES.json has "signatures": []                 — O1!=0, O2 no promotion, O3 names the 0 count
#   S4: `gh release download` produces no SIGNATURES.json               — O1!=0, O2 no promotion, O3 names the file
#   S5: no version argument                                             — O1!=0, O2 has NO gh mutation at all, O3 usage
#   S6: draft notes carry the CI banner, verify passes                  — O1=0, O2 one promotion, O4 banner-free + changelog intact
#   S7: draft notes carry the CI banner, verify FAILS                   — O1!=0, O2 no promotion, O4 no notes mutation at all
#   S8: (static, no sandbox) marker strings identical in CI + stripper  — drift guard, no O1-O4
#   S9: (static, no sandbox) CI writes the draft reminder to the step summary — drift guard, no O1-O4
# MATRIX: 4 outputs x 9 partitions (only cells the path reaches are asserted)
#   S1: O1 O2 | S2: O1 O2 | S3: O1 O2 O3 | S4: O1 O2 O3 | S5: O1 O2 O3 | S6: O1 O2 O4 | S7: O1 O2 O4 | S8/S9: static file content
#
# TDD RED tests for scripts/publish-release.sh. `doli` and `gh` are fully stubbed on
# PATH: no network, no live release, no promotion of anything real.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
PUBLISH_SCRIPT="$PROJECT_ROOT/scripts/publish-release.sh"
TEST_DIR="/tmp/doli-publish-release-test-$$"
VERSION="6.26.3"
TAG="v$VERSION"

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

# shellcheck disable=SC2329  # invoked indirectly via trap below
cleanup() {
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"

# A missing publish-release.sh makes bash exit 127 without running anything, which would
# satisfy every "exits non-zero" assertion vacuously. Each negative check is conjoined
# with this so no partition can pass while the script under test does not exist.
script_ran() {
    [[ -f "$PUBLISH_SCRIPT" ]] && [[ "$RC" -ne 127 ]]
}

# --- stub writers ---

write_gh_stub() {
    local bin_dir="$1"
    cat > "$bin_dir/gh" <<'GH_STUB'
#!/usr/bin/env bash
echo "gh $*" >> "${GH_LOG:?GH_LOG not set}"
mode="${GH_STUB_MODE:-normal}"
sub="$1 $2"
case "$sub" in
    "release view")
        body_requested=false
        raw_mode=false
        for arg in "$@"; do
            case "$arg" in
                --json) body_requested=true ;;
                -q|--jq) raw_mode=true ;;
            esac
        done
        if $body_requested; then
            body_content=""
            if [[ -n "${GH_VIEW_BODY_FILE:-}" && -f "${GH_VIEW_BODY_FILE:-}" ]]; then
                body_content="$(cat "$GH_VIEW_BODY_FILE")"
            fi
            if $raw_mode; then
                printf '%s' "$body_content"
            else
                jq -n --arg body "$body_content" '{body:$body}'
            fi
            exit 0
        fi
        echo "draft"
        exit 0
        ;;
    "release edit")
        # M2.5/REQ-202-007: record any --notes-file CONTENT so a promotion's actual
        # payload can be inspected after the caller's own tmpfile is gone.
        prev=""
        notes_file=""
        for arg in "$@"; do
            if [[ "$prev" == "--notes-file" ]]; then
                notes_file="$arg"
            fi
            prev="$arg"
        done
        if [[ -n "$notes_file" && -f "$notes_file" ]]; then
            {
                echo "notes-file-content-begin"
                cat "$notes_file"
                echo "notes-file-content-end"
            } >> "${GH_LOG:?GH_LOG not set}"
        fi
        exit 0
        ;;
    "release download")
        dir=""
        prev=""
        for arg in "$@"; do
            if [[ "$prev" == "--dir" ]]; then
                dir="$arg"
            fi
            prev="$arg"
        done
        [[ -n "$dir" ]] || exit 1
        mkdir -p "$dir"
        printf '%s\n' \
            "b6f0e7f3  doli-v6.26.3-linux-x86_64.tar.gz" \
            "1c2d3e4f  doli-v6.26.3-darwin-arm64.tar.gz" > "$dir/CHECKSUMS.txt"
        case "$mode" in
            no_manifest)
                ;;
            zero_entries)
                cat > "$dir/SIGNATURES.json" <<'JSON'
{
  "version": "6.26.3",
  "checksums_sha256": "7e0dd5f2a89306f1cd8f0e2a31e45a60b9f3f605400455e737d0fe8c4e3ce6cd",
  "signatures": []
}
JSON
                ;;
            *)
                cat > "$dir/SIGNATURES.json" <<'JSON'
{
  "version": "6.26.3",
  "checksums_sha256": "7e0dd5f2a89306f1cd8f0e2a31e45a60b9f3f605400455e737d0fe8c4e3ce6cd",
  "signatures": [
    { "public_key": "aa01", "signature": "5501" },
    { "public_key": "bb02", "signature": "5502" },
    { "public_key": "cc03", "signature": "5503" }
  ]
}
JSON
                ;;
        esac
        exit 0
        ;;
    *)
        exit 0
        ;;
esac
GH_STUB
    chmod +x "$bin_dir/gh"
}

# `doli release verify` stub. DOLI_STUB_MODE:
#   verify_real — mirror the real gate: refuse an absent or 0-entry manifest, else pass
#   verify_fail — always refuse (stands in for any broken link in the chain)
write_doli_stub() {
    local bin_dir="$1"
    cat > "$bin_dir/doli" <<'DOLI_STUB'
#!/usr/bin/env bash
echo "doli $*" >> "${DOLI_LOG:?DOLI_LOG not set}"
mode="${DOLI_STUB_MODE:-verify_real}"
dir=""
prev=""
for arg in "$@"; do
    if [[ "$prev" == "--dir" ]]; then
        dir="$arg"
    fi
    prev="$arg"
done
if [[ "$mode" == "verify_fail" ]]; then
    echo "Signature/artifact binding FAILED on \`checksums_sha256\`" >&2
    exit 1
fi
manifest="$dir/SIGNATURES.json"
if [[ ! -f "$manifest" ]]; then
    echo "error: $manifest: no such file" >&2
    exit 1
fi
count=$(grep -c '"public_key"' "$manifest" 2>/dev/null) || count=0
if [[ "$count" -lt 3 ]]; then
    echo "Insufficient signatures: ${count}/3" >&2
    exit 1
fi
echo "Verified: ${count} distinct maintainer signature(s)"
exit 0
DOLI_STUB
    chmod +x "$bin_dir/doli"
}

# --- sandbox builder: sets CASE_DIR/WORK_DIR/HOME_DIR/BIN_DIR/GH_LOG/DOLI_LOG/LOG_FILE ---
new_sandbox() {
    local name="$1"
    CASE_DIR="$TEST_DIR/$name"
    WORK_DIR="$CASE_DIR/work"
    HOME_DIR="$CASE_DIR/home"
    BIN_DIR="$CASE_DIR/bin"
    GH_LOG="$CASE_DIR/gh.log"
    DOLI_LOG="$CASE_DIR/doli.log"
    LOG_FILE="$CASE_DIR/run.log"
    rm -rf "$CASE_DIR"
    mkdir -p "$WORK_DIR" "$HOME_DIR" "$BIN_DIR"
    : > "$GH_LOG"
    : > "$DOLI_LOG"
    write_gh_stub "$BIN_DIR"
    write_doli_stub "$BIN_DIR"
}

# Runs publish-release.sh under env -i in $WORK_DIR; sets RC and writes LOG_FILE.
# $1 = gh stub mode, $2 = doli stub mode, $3.. = argv for the script.
# If $NOTES_BODY_FILE is set (M2.5 cases), it is exported as GH_VIEW_BODY_FILE so the
# `gh release view --json body` stub answers with fixture-controlled draft notes.
run_publish_release() {
    local gh_mode="$1"
    local doli_mode="$2"
    shift 2
    local extra_env=()
    if [[ -n "${NOTES_BODY_FILE:-}" ]]; then
        extra_env+=("GH_VIEW_BODY_FILE=$NOTES_BODY_FILE")
    fi
    ( cd "$WORK_DIR" && env -i \
        "HOME=$HOME_DIR" \
        "PATH=$BIN_DIR:/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin" \
        "GH_LOG=$GH_LOG" \
        "DOLI_LOG=$DOLI_LOG" \
        "GH_STUB_MODE=$gh_mode" \
        "DOLI_STUB_MODE=$doli_mode" \
        "${extra_env[@]}" \
        bash "$PUBLISH_SCRIPT" "$@" ) > "$LOG_FILE" 2>&1
    RC=$?
}

# Extracts the content the gh stub recorded from a --notes-file argument, if any.
notes_file_content() {
    sed -n '/^notes-file-content-begin$/,/^notes-file-content-end$/p' "$GH_LOG" | sed '1d;$d'
}

# `grep -c` prints 0 AND exits 1 on no match, so the count must be captured before the
# fallback, never with `|| echo 0` (that yields two lines and breaks the arithmetic test).
promotion_count() {
    local n
    n=$(grep -cE 'release edit .*--draft=false' "$GH_LOG" 2>/dev/null) || n=0
    echo "$n"
}

print_header "publish-release.sh RED tests (INC-I-202 M2)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# ============================================================
# S1 — REQ-202-004 (Must) — Decision: a promotion after a failed verify is the
# whole defect; it would publish an unverifiable release as public Latest.
# ============================================================
new_sandbox "s1_verify_fails"
run_publish_release "normal" "verify_fail" "$VERSION"

if script_ran && [[ "$RC" -ne 0 ]]; then
    test_result "S1 failed_verification_does_not_promote: exit code non-zero" "pass"
else
    test_result "S1 failed_verification_does_not_promote: exit code non-zero" "fail" \
        "exit=$RC script=$PUBLISH_SCRIPT log=$LOG_FILE"
fi

if script_ran && [[ "$(promotion_count)" -eq 0 ]] && ! grep -q -- "--draft=false" "$GH_LOG"; then
    test_result "S1 failed_verification_does_not_promote: no gh release edit --draft=false" "pass"
else
    test_result "S1 failed_verification_does_not_promote: no gh release edit --draft=false" "fail" \
        "gh log=$GH_LOG run log=$LOG_FILE"
fi

# ============================================================
# S2 — REQ-202-005 (Must) — Decision: a gate that never promotes is a gate that
# blocks every genuine release, so the locally callable verifier is unusable.
# ============================================================
new_sandbox "s2_verify_passes"
run_publish_release "normal" "verify_real" "$VERSION"

if script_ran && [[ "$RC" -eq 0 ]]; then
    test_result "S2 verified_release_is_promoted: exit code 0" "pass"
else
    test_result "S2 verified_release_is_promoted: exit code 0" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && [[ "$(promotion_count)" -eq 1 ]] \
    && grep -qE "release edit $TAG .*--draft=false" "$GH_LOG" \
    && grep -q -- "--latest" "$GH_LOG"; then
    test_result "S2 verified_release_is_promoted: exactly one gh release edit $TAG --draft=false --latest" "pass"
else
    test_result "S2 verified_release_is_promoted: exactly one gh release edit $TAG --draft=false --latest" "fail" \
        "gh log=$GH_LOG run log=$LOG_FILE"
fi

if script_ran && grep -q "release verify" "$DOLI_LOG"; then
    test_result "S2 verified_release_is_promoted: verification actually ran before promotion" "pass"
else
    test_result "S2 verified_release_is_promoted: verification actually ran before promotion" "fail" \
        "doli log=$DOLI_LOG run log=$LOG_FILE"
fi

# ============================================================
# S3 — REQ-202-004 (Must) — Decision: the 0-entry manifest is the shape CI has
# been publishing; a silent pass here reproduces the mainnet upgrade brick.
# ============================================================
new_sandbox "s3_zero_entries"
run_publish_release "zero_entries" "verify_real" "$VERSION"

if script_ran && [[ "$RC" -ne 0 ]]; then
    test_result "S3 zero_entry_manifest_is_refused: exit code non-zero" "pass"
else
    test_result "S3 zero_entry_manifest_is_refused: exit code non-zero" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && [[ "$(promotion_count)" -eq 0 ]]; then
    test_result "S3 zero_entry_manifest_is_refused: no promotion" "pass"
else
    test_result "S3 zero_entry_manifest_is_refused: no promotion" "fail" \
        "gh log=$GH_LOG run log=$LOG_FILE"
fi

if script_ran && grep -qE "0/3|0 of 3|0 signature" "$LOG_FILE"; then
    test_result "S3 zero_entry_manifest_is_refused: message names the 0 count" "pass"
else
    test_result "S3 zero_entry_manifest_is_refused: message names the 0 count" "fail" \
        "run log=$LOG_FILE"
fi

# ============================================================
# S4 — REQ-202-004 (Must) — Decision: an absent manifest treated as "nothing to
# check" promotes an entirely unsigned release.
# ============================================================
new_sandbox "s4_missing_manifest"
run_publish_release "no_manifest" "verify_real" "$VERSION"

if script_ran && [[ "$RC" -ne 0 ]]; then
    test_result "S4 missing_manifest_is_refused: exit code non-zero" "pass"
else
    test_result "S4 missing_manifest_is_refused: exit code non-zero" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && [[ "$(promotion_count)" -eq 0 ]]; then
    test_result "S4 missing_manifest_is_refused: no promotion" "pass"
else
    test_result "S4 missing_manifest_is_refused: no promotion" "fail" \
        "gh log=$GH_LOG run log=$LOG_FILE"
fi

if script_ran && grep -q "SIGNATURES.json" "$LOG_FILE"; then
    test_result "S4 missing_manifest_is_refused: message names SIGNATURES.json" "pass"
else
    test_result "S4 missing_manifest_is_refused: message names SIGNATURES.json" "fail" \
        "run log=$LOG_FILE"
fi

# ============================================================
# S5 — REQ-202-005 (Must) — Decision: a script that mutates a release before it
# knows which release it was asked about can promote the wrong tag.
# ============================================================
new_sandbox "s5_no_argument"
run_publish_release "normal" "verify_real"

if script_ran && [[ "$RC" -ne 0 ]]; then
    test_result "S5 missing_version_argument_is_a_usage_error: exit code non-zero" "pass"
else
    test_result "S5 missing_version_argument_is_a_usage_error: exit code non-zero" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && ! grep -qE "release (edit|upload|delete)" "$GH_LOG"; then
    test_result "S5 missing_version_argument_is_a_usage_error: no gh mutation at all" "pass"
else
    test_result "S5 missing_version_argument_is_a_usage_error: no gh mutation at all" "fail" \
        "gh log=$GH_LOG run log=$LOG_FILE"
fi

if script_ran && grep -qiE "usage|version.*required|missing.*version" "$LOG_FILE"; then
    test_result "S5 missing_version_argument_is_a_usage_error: message states the usage" "pass"
else
    test_result "S5 missing_version_argument_is_a_usage_error: message states the usage" "fail" \
        "run log=$LOG_FILE"
fi

# ============================================================
# Fixed, byte-exact banner markers (M2.5/REQ-202-007). Both release.yml (writer) and
# publish-release.sh (stripper) MUST use these literal strings — see S8.
BEGIN_MARKER='<!-- DOLI-UNSIGNED-DRAFT-WARNING:BEGIN -->'
END_MARKER='<!-- DOLI-UNSIGNED-DRAFT-WARNING:END -->'
WARNING_LINE='This release has not yet been verified against the maintainer trust root.'

# ============================================================
# S6 — REQ-202-007 (Must) — Decision: a published release that still carries the
# "UNSIGNED DRAFT" banner falsely tells every downstream reader (nodes, `doli upgrade`,
# operators) to distrust a release that was, in fact, just verified 3/3.
# ============================================================
new_sandbox "s6_strip_banner"
cat > "$CASE_DIR/original_body.txt" <<BODY_EOF
$BEGIN_MARKER
> **UNSIGNED DRAFT.** $WARNING_LINE Do not use it until \`scripts/publish-release.sh\` promotes it.
$END_MARKER
## What's Changed
* Real changelog entry one
* Real changelog entry two
BODY_EOF
NOTES_BODY_FILE="$CASE_DIR/original_body.txt"
run_publish_release "normal" "verify_real" "$VERSION"
NOTES_BODY_FILE=""

if script_ran && [[ "$RC" -eq 0 ]]; then
    test_result "S6 promotion_strips_the_warning_banner: exit code 0" "pass"
else
    test_result "S6 promotion_strips_the_warning_banner: exit code 0" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && [[ "$(promotion_count)" -eq 1 ]]; then
    test_result "S6 promotion_strips_the_warning_banner: exactly one promotion" "pass"
else
    test_result "S6 promotion_strips_the_warning_banner: exactly one promotion" "fail" \
        "gh log=$GH_LOG"
fi

NOTES_CONTENT="$(notes_file_content)"
if script_ran && [[ -n "$NOTES_CONTENT" ]] \
    && ! grep -qF -- "$BEGIN_MARKER" <<<"$NOTES_CONTENT" \
    && ! grep -qF -- "$END_MARKER" <<<"$NOTES_CONTENT" \
    && ! grep -qF -- "$WARNING_LINE" <<<"$NOTES_CONTENT"; then
    test_result "S6 promotion_strips_the_warning_banner: promoted notes carry neither marker nor warning text" "pass"
else
    test_result "S6 promotion_strips_the_warning_banner: promoted notes carry neither marker nor warning text" "fail" \
        "notes-file content: [$NOTES_CONTENT] gh log=$GH_LOG"
fi

if script_ran && [[ -n "$NOTES_CONTENT" ]] && grep -qF -- "## What's Changed" <<<"$NOTES_CONTENT"; then
    test_result "S6 promotion_strips_the_warning_banner: promoted notes still carry the changelog" "pass"
else
    test_result "S6 promotion_strips_the_warning_banner: promoted notes still carry the changelog" "fail" \
        "notes-file content: [$NOTES_CONTENT]"
fi

# ============================================================
# S7 — REQ-202-007 (Must) — Decision: banner-stripping logic must never run ahead of
# the verification gate; if it did, a failed-verify draft could still get its notes
# silently rewritten even though the release stays unpublished and unsigned.
# ============================================================
new_sandbox "s7_verify_fails_leaves_notes_alone"
cat > "$CASE_DIR/original_body.txt" <<BODY_EOF
$BEGIN_MARKER
> **UNSIGNED DRAFT.** $WARNING_LINE Do not use it until \`scripts/publish-release.sh\` promotes it.
$END_MARKER
## What's Changed
* Real changelog entry one
BODY_EOF
NOTES_BODY_FILE="$CASE_DIR/original_body.txt"
run_publish_release "normal" "verify_fail" "$VERSION"
NOTES_BODY_FILE=""

if script_ran && [[ "$RC" -ne 0 ]]; then
    test_result "S7 failed_verification_leaves_the_banner_alone: exit code non-zero" "pass"
else
    test_result "S7 failed_verification_leaves_the_banner_alone: exit code non-zero" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && [[ "$(promotion_count)" -eq 0 ]]; then
    test_result "S7 failed_verification_leaves_the_banner_alone: no promotion" "pass"
else
    test_result "S7 failed_verification_leaves_the_banner_alone: no promotion" "fail" \
        "gh log=$GH_LOG"
fi

if script_ran && [[ -z "$(notes_file_content)" ]] && ! grep -q -- "--notes-file" "$GH_LOG"; then
    test_result "S7 failed_verification_leaves_the_banner_alone: no notes mutation at all" "pass"
else
    test_result "S7 failed_verification_leaves_the_banner_alone: no notes mutation at all" "fail" \
        "gh log=$GH_LOG"
fi

# ============================================================
# S8 — REQ-202-007 (Must) — Decision: CI writing marker A while the stripper deletes
# marker B is silent breakage — a published release keeps its banner and nobody notices
# until an operator reads it by hand. Static check, no sandbox, no script execution.
# ============================================================
WORKFLOW_FILE="$PROJECT_ROOT/.github/workflows/release.yml"

if [[ -f "$WORKFLOW_FILE" ]] \
    && grep -qF -- "$BEGIN_MARKER" "$WORKFLOW_FILE" \
    && grep -qF -- "$END_MARKER" "$WORKFLOW_FILE"; then
    test_result "S8 banner_markers_are_identical_in_ci_and_in_the_stripper: markers present in release.yml" "pass"
else
    test_result "S8 banner_markers_are_identical_in_ci_and_in_the_stripper: markers present in release.yml" "fail" \
        "file=$WORKFLOW_FILE"
fi

if [[ -f "$PUBLISH_SCRIPT" ]] \
    && grep -qF -- "$BEGIN_MARKER" "$PUBLISH_SCRIPT" \
    && grep -qF -- "$END_MARKER" "$PUBLISH_SCRIPT"; then
    test_result "S8 banner_markers_are_identical_in_ci_and_in_the_stripper: markers present in publish-release.sh" "pass"
else
    test_result "S8 banner_markers_are_identical_in_ci_and_in_the_stripper: markers present in publish-release.sh" "fail" \
        "file=$PUBLISH_SCRIPT"
fi

# ============================================================
# S9 — REQ-202-007 (Must) — Decision: without a step-summary reminder, the ONLY place
# a maintainer sees "this is a draft" is inside the release notes themselves; the CI
# run summary is what a maintainer glances at right after the workflow finishes.
# Static check, no sandbox, no script execution. The reminder text must sit NEAR the
# actual GITHUB_STEP_SUMMARY write — an unrelated stray comment elsewhere in the file
# that happens to say both words does not satisfy the requirement.
# ============================================================
step_summary_window() {
    local line
    line="$(grep -n "GITHUB_STEP_SUMMARY" "$WORKFLOW_FILE" 2>/dev/null | head -1 | cut -d: -f1)"
    [[ -n "$line" ]] || return 1
    local start=$(( line - 30 ))
    (( start < 1 )) && start=1
    local end=$(( line + 30 ))
    sed -n "${start},${end}p" "$WORKFLOW_FILE"
}

if [[ -f "$WORKFLOW_FILE" ]] && grep -q "GITHUB_STEP_SUMMARY" "$WORKFLOW_FILE"; then
    test_result "S9 ci_writes_the_draft_reminder_to_the_step_summary: writes to GITHUB_STEP_SUMMARY" "pass"
else
    test_result "S9 ci_writes_the_draft_reminder_to_the_step_summary: writes to GITHUB_STEP_SUMMARY" "fail" \
        "file=$WORKFLOW_FILE"
fi

SUMMARY_WINDOW="$(step_summary_window || true)"
if [[ -n "$SUMMARY_WINDOW" ]] \
    && grep -q "publish-release.sh" <<<"$SUMMARY_WINDOW" \
    && grep -q "DRAFT" <<<"$SUMMARY_WINDOW"; then
    test_result "S9 ci_writes_the_draft_reminder_to_the_step_summary: reminder near the write names publish-release.sh and DRAFT" "pass"
else
    test_result "S9 ci_writes_the_draft_reminder_to_the_step_summary: reminder near the write names publish-release.sh and DRAFT" "fail" \
        "file=$WORKFLOW_FILE"
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
