#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/monitor-release-signed.sh (INC-I-202 M2.5, REQ-202-008)
#   O1 exit code        — 0 only when the newest v* tag has a PUBLISHED, verified release
#   O2 gh/doli mutation — every gh/doli invocation, checked for ANY release edit|upload|delete
#                          (must be absent in every partition — the monitor never writes)
#   O3 operator message — combined stdout+stderr, checked for the tag name + the fix command
#   O4 tag selection     — which tag name reaches gh/doli (VERSION order, not lexicographic)
#   PATHS: monitor-release-signed.sh (no argv)
#            -> newest `v*` tag in REPO_DIR (git, version-sorted)
#            -> `gh release view <tag> --json isDraft`
#            -> draft or absent ? refuse (name tag + fix) : `doli release verify --version <tag>`
#            -> exit!=0 ? refuse (name tag) : exit 0
# INPUT PARTITIONS (REQ-202-008, Must):
#   S1: newest tag PUBLISHED, `doli release verify` exits 0        — O1=0, O2 empty (read-only)
#   S2: newest tag release is a DRAFT                               — O1!=0, O3 names tag + script
#   S3: newest tag PUBLISHED, `doli release verify` exits 1 (1/3)   — O1!=0, O3 names tag
#   S4: newest tag has NO release at all                            — O1!=0, O3 names tag, O2 empty
#   S5: tags v6.26.2/v6.26.9/v6.26.10 — VERSION order picks .10      — O4 names v6.26.10 never .9
# MATRIX: 4 outputs x 5 partitions (only cells the path reaches are asserted)
#   S1: O1 O2 | S2: O1 O3 | S3: O1 O3 | S4: O1 O2 O3 | S5: O1 O4
#
# TDD RED tests for scripts/monitor-release-signed.sh, which DOES NOT EXIST YET.
# `gh` and `doli` are fully stubbed on PATH: no network, no live release touched.
# The tag fixture is a REAL local git repo (git itself needs no network for tag/commit).

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
MONITOR_SCRIPT="$PROJECT_ROOT/scripts/monitor-release-signed.sh"
TEST_DIR="/tmp/doli-monitor-release-test-$$"

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

# A missing monitor-release-signed.sh makes bash exit 127 without running anything, which
# would satisfy every "exits non-zero" / "no mutation" assertion vacuously. Every assertion
# below is conjoined with this so no partition can pass while the script does not exist.
script_ran() {
    [[ -f "$MONITOR_SCRIPT" ]] && [[ "$RC" -ne 127 ]]
}

# --- stub writers ---

# `gh release view <tag> --json isDraft` stub. GH_STUB_MODE:
#   no_release — `gh release view` fails (stands in for "no release for this tag")
#   draft      — reports isDraft=true
#   published  — reports isDraft=false (default)
write_gh_stub() {
    local bin_dir="$1"
    cat > "$bin_dir/gh" <<'GH_STUB'
#!/usr/bin/env bash
echo "gh $*" >> "${GH_LOG:?GH_LOG not set}"
mode="${GH_STUB_MODE:-published}"
sub="$1 $2"
case "$sub" in
    "release view")
        case "$mode" in
            no_release)
                exit 1
                ;;
            draft)
                echo '{"isDraft":true}'
                exit 0
                ;;
            *)
                echo '{"isDraft":false}'
                exit 0
                ;;
        esac
        ;;
    *)
        exit 0
        ;;
esac
GH_STUB
    chmod +x "$bin_dir/gh"
}

# `doli release verify --version <tag>` stub. DOLI_STUB_MODE:
#   verify_ok  — exits 0 (3/3 signatures)
#   verify_low — exits 1, prints the sub-threshold count (stands in for the INC-I-202 shape)
write_doli_stub() {
    local bin_dir="$1"
    cat > "$bin_dir/doli" <<'DOLI_STUB'
#!/usr/bin/env bash
echo "doli $*" >> "${DOLI_LOG:?DOLI_LOG not set}"
mode="${DOLI_STUB_MODE:-verify_ok}"
if [[ "$mode" == "verify_low" ]]; then
    echo "Insufficient signatures: 1/3" >&2
    exit 1
fi
echo "Verified: 3 distinct maintainer signature(s)"
exit 0
DOLI_STUB
    chmod +x "$bin_dir/doli"
}

# --- sandbox builder: sets CASE_DIR/WORK_DIR/HOME_DIR/BIN_DIR/REPO_DIR/GH_LOG/DOLI_LOG/LOG_FILE ---
new_sandbox() {
    local name="$1"
    CASE_DIR="$TEST_DIR/$name"
    WORK_DIR="$CASE_DIR/work"
    HOME_DIR="$CASE_DIR/home"
    BIN_DIR="$CASE_DIR/bin"
    REPO_DIR="$CASE_DIR/repo"
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

# Builds a REAL local git repo with the given tags on one empty commit. Runs in the
# harness's own (non-sandboxed) environment — the fixture itself is never under env -i.
build_git_repo() {
    local repo_dir="$1"
    shift
    rm -rf "$repo_dir"
    mkdir -p "$repo_dir"
    (
        cd "$repo_dir" || exit 1
        git init -q
        git -c user.email=t@t -c user.name=t commit --allow-empty -q -m x
        for tag in "$@"; do
            git tag "$tag"
        done
    )
}

# Runs monitor-release-signed.sh under env -i in $WORK_DIR; sets RC and writes LOG_FILE.
# No positional arguments per the contract — argv is always empty.
run_monitor() {
    local gh_mode="$1"
    local doli_mode="$2"
    ( cd "$WORK_DIR" && env -i \
        "HOME=$HOME_DIR" \
        "GIT_CONFIG_GLOBAL=/dev/null" \
        "PATH=$BIN_DIR:/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin" \
        "GH_LOG=$GH_LOG" \
        "DOLI_LOG=$DOLI_LOG" \
        "GH_STUB_MODE=$gh_mode" \
        "DOLI_STUB_MODE=$doli_mode" \
        "DOLI_CLI=$BIN_DIR/doli" \
        "REPO_DIR=$REPO_DIR" \
        bash "$MONITOR_SCRIPT" ) > "$LOG_FILE" 2>&1
    RC=$?
}

gh_mutated() {
    grep -qE 'release (edit|upload|delete)' "$GH_LOG" 2>/dev/null
}

print_header "monitor-release-signed.sh RED tests (INC-I-202 M2.5)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# ============================================================
# S1 — REQ-202-008 (Must) — Decision: a monitor that cannot confirm a healthy,
# published+verified newest release would page an operator for nothing every time —
# false alarms are how a monitor gets ignored right before the next real incident.
# ============================================================
new_sandbox "s1_published_verified"
build_git_repo "$REPO_DIR" "v1.0.0"
run_monitor "published" "verify_ok"

if script_ran && [[ "$RC" -eq 0 ]]; then
    test_result "S1 published_and_verified_is_healthy: exit code 0" "pass"
else
    test_result "S1 published_and_verified_is_healthy: exit code 0" "fail" \
        "exit=$RC script=$MONITOR_SCRIPT log=$LOG_FILE"
fi

if script_ran && ! gh_mutated; then
    test_result "S1 published_and_verified_is_healthy: monitor is read-only (no gh mutation)" "pass"
else
    test_result "S1 published_and_verified_is_healthy: monitor is read-only (no gh mutation)" "fail" \
        "gh log=$GH_LOG run log=$LOG_FILE"
fi

# ============================================================
# S2 — REQ-202-008 (Must) — Decision: if a draft newest release reads as healthy, an
# upgrade artifact reachable by nobody goes unnoticed — the exact INC-I-202 root cause.
# ============================================================
new_sandbox "s2_newest_is_draft"
build_git_repo "$REPO_DIR" "v1.0.0"
run_monitor "draft" "verify_ok"

if script_ran && [[ "$RC" -ne 0 ]]; then
    test_result "S2 draft_newest_release_is_unhealthy: exit code non-zero" "pass"
else
    test_result "S2 draft_newest_release_is_unhealthy: exit code non-zero" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && grep -q "v1.0.0" "$LOG_FILE" && grep -q "publish-release.sh" "$LOG_FILE"; then
    test_result "S2 draft_newest_release_is_unhealthy: message names the tag and the fix script" "pass"
else
    test_result "S2 draft_newest_release_is_unhealthy: message names the tag and the fix script" "fail" \
        "run log=$LOG_FILE"
fi

# ============================================================
# S3 — REQ-202-008 (Must) — Decision: if a sub-threshold-signature newest release reads
# as healthy, the monitor is blind to the precise defect class that caused INC-I-202.
# ============================================================
new_sandbox "s3_below_threshold"
build_git_repo "$REPO_DIR" "v1.0.0"
run_monitor "published" "verify_low"

if script_ran && [[ "$RC" -ne 0 ]]; then
    test_result "S3 sub_threshold_signatures_is_unhealthy: exit code non-zero" "pass"
else
    test_result "S3 sub_threshold_signatures_is_unhealthy: exit code non-zero" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && grep -q "v1.0.0" "$LOG_FILE"; then
    test_result "S3 sub_threshold_signatures_is_unhealthy: message names the tag" "pass"
else
    test_result "S3 sub_threshold_signatures_is_unhealthy: message names the tag" "fail" \
        "run log=$LOG_FILE"
fi

# ============================================================
# S4 — REQ-202-008 (Must) — Decision: an absent release for the newest tag must refuse
# loudly, not be swallowed as "nothing to check" nor trigger any write.
# ============================================================
new_sandbox "s4_no_release_at_all"
build_git_repo "$REPO_DIR" "v1.0.0"
run_monitor "no_release" "verify_ok"

if script_ran && [[ "$RC" -ne 0 ]]; then
    test_result "S4 missing_release_is_unhealthy: exit code non-zero" "pass"
else
    test_result "S4 missing_release_is_unhealthy: exit code non-zero" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran && grep -q "v1.0.0" "$LOG_FILE"; then
    test_result "S4 missing_release_is_unhealthy: message names the tag" "pass"
else
    test_result "S4 missing_release_is_unhealthy: message names the tag" "fail" \
        "run log=$LOG_FILE"
fi

if script_ran && ! gh_mutated; then
    test_result "S4 missing_release_is_unhealthy: no gh/doli mutation" "pass"
else
    test_result "S4 missing_release_is_unhealthy: no gh/doli mutation" "fail" \
        "gh log=$GH_LOG run log=$LOG_FILE"
fi

# ============================================================
# S5 — REQ-202-008 (Must) — Decision: lexicographic sort would pin the monitor to a
# STALE tag (v6.26.9 sorts after v6.26.10 as text) forever once a two-digit patch
# exists, permanently blinding it to the real newest release.
# ============================================================
new_sandbox "s5_version_ordering"
build_git_repo "$REPO_DIR" "v6.26.2" "v6.26.9" "v6.26.10"
run_monitor "published" "verify_ok"

if script_ran && [[ "$RC" -eq 0 ]]; then
    test_result "S5 tag_selection_is_version_ordered: exit code 0 on the true newest tag" "pass"
else
    test_result "S5 tag_selection_is_version_ordered: exit code 0 on the true newest tag" "fail" \
        "exit=$RC log=$LOG_FILE"
fi

if script_ran \
    && { grep -qF "v6.26.10" "$GH_LOG" || grep -qF "v6.26.10" "$DOLI_LOG"; } \
    && ! grep -qF "v6.26.9" "$GH_LOG" \
    && ! grep -qF "v6.26.9" "$DOLI_LOG"; then
    test_result "S5 tag_selection_is_version_ordered: gh/doli asked about v6.26.10, never v6.26.9" "pass"
else
    test_result "S5 tag_selection_is_version_ordered: gh/doli asked about v6.26.10, never v6.26.9" "fail" \
        "gh log=$GH_LOG doli log=$DOLI_LOG"
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
