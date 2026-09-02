#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/gauntlet-gs015.sh — `_gs015_assert <token>` (INC-I-202, GS-015)
#   O1 return code   — 0 PASS · 1 FAIL · 2 SKIP (gauntlet.sh:639 treats 0 and 2 alike,
#                       so O1 alone can never separate "checked green" from "not checked")
#   O2 FAIL_REASONS  — caller-owned global, appended on rc 1 only
#   O3 SKIP_REASONS  — caller-owned global, appended on rc 2 only; the ONLY signal that
#                       distinguishes a skip from a pass in the runner's output
#   O4 INFO_REASONS  — caller-owned global, appended on rc 0 only
#   O5 gh/doli log   — every gh/doli invocation, checked for release edit|upload|delete|create
#                       (GS-015 is observational: this must be empty in every partition)
#   O6 registration  — the assertions column the runner would read for GS-015; the tokens
#                       above are only ever dispatched from scripts/gauntlet-seed.sql
#   PATHS:
#     token gs015-newest-release-published-and-signed
#       -> preflight: gh on PATH? · gh auth status? · $GS015_MONITOR readable? · doli resolvable?
#          -> any NO  => rc 2 + O3
#       -> run $GS015_MONITOR (REPO_DIR=$GS015_REPO_DIR); rc 0 => rc 0 + O4 ; rc!=0 => rc 1 + O2
#     token gs015-workflow-drafts-releases
#       -> $GS015_WORKFLOW readable? NO => rc 2 + O3
#       -> contains `draft: true` ? YES => rc 0 + O4 : rc 1 + O2
#     unknown token -> non-zero
# INPUT PARTITIONS:
#   S1: newest tag published + verifying          — O1=0, O4 names tag, O3/O2 empty
#   S2: newest tag release is a DRAFT             — O1=1, O2 names tag + draft, O4 empty
#   S3: published, `doli release verify` rc!=0    — O1=1, O2 non-empty, O4 empty
#   S4: `gh` absent from PATH                     — O1=2, O3 names gh, O2/O4 empty
#   S5: `gh auth status` rc!=0 (unauth/offline)   — O1=2, O3 says unauthenticated/offline, O2/O4 empty
#   S6: S1 sandbox                                — O5 has no gh/doli mutation
#   S7: release.yml fixture with `draft: true`    — O1=0
#   S8: release.yml fixture with `draft: false`   — O1=1, O2 names release.yml + draft
#   S9: $GS015_WORKFLOW does not exist            — O1=2, O3 non-empty
#   S10: unknown token                            — O1!=0 (never a silent pass)
#   S11: scripts/gauntlet-seed.sql registration   — O6 names GS-015 active + both tokens
#   S12: `jq` absent from PATH                    — O1=2, O3 names jq, O2/O4 empty
#   S13: DOLI_CLI set to a non-executable path    — O1=2, O3 names the path, O2 empty
#   S14: `git` absent from PATH                   — O1=2, O3 names git, O2 empty
#   S15: repo has no v* tag                       — O1=2, O3 names the repo dir, O2 empty
#   S16: `draft: true` present but OUTSIDE the release step — O1=1, O2 names release.yml
# MATRIX: 6 outputs x 16 partitions (only cells the path reaches are asserted)
#   S1: O1 O3 O4 | S2: O1 O2 O4 | S3: O1 O2 O4 | S4: O1 O2 O3 O4 | S5: O1 O2 O3 O4
#   S6: O5 | S7: O1 | S8: O1 O2 | S9: O1 O3 | S10: O1 | S11: O1 O2 O6
#   S12: O1 O2 O3 O4 | S13: O1 O2 O3 | S14: O1 O2 O3 | S15: O1 O2 O3 | S16: O1 O2
#
# TDD RED tests for scripts/gauntlet-gs015.sh, which DOES NOT EXIST YET.
# `gh` and `doli` are stubbed on PATH and the tag fixture is a REAL local git repo:
# no network, no live GitHub release, no tag written into the real repo.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
GS015_LIB="$PROJECT_ROOT/scripts/gauntlet-gs015.sh"
MONITOR_SCRIPT="$PROJECT_ROOT/scripts/monitor-release-signed.sh"
SEED_FILE="$PROJECT_ROOT/scripts/gauntlet-seed.sql"
TEST_DIR="/tmp/doli-gauntlet-gs015-test-$$"
TAG="v6.26.3"
TOKEN_RELEASE="gs015-newest-release-published-and-signed"
TOKEN_WORKFLOW="gs015-workflow-drafts-releases"

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

# A missing gauntlet-gs015.sh means _gs015_assert is never called, so RC keeps the 99
# sentinel and the three *_REASONS stay empty — which would satisfy "rc non-zero",
# "INFO_REASONS empty" and "no mutation" vacuously. Every assertion below is conjoined
# with this so no partition can go green while the lib does not exist.
lib_ok() {
    [[ -f "$GS015_LIB" ]] && [[ "$FUNC_DEFINED" == "yes" ]]
}

# --- stub writers ---

# `gh` stub. GH_AUTH_MODE: ok (default) | fail — `gh auth status` exit status, which is
# how an unauthenticated or offline host presents itself.
# GH_STUB_MODE (release view): published (default) | draft | no_release.
write_gh_stub() {
    local bin_dir="$1"
    cat > "$bin_dir/gh" <<'GH_STUB'
#!/usr/bin/env bash
echo "gh $*" >> "${GH_LOG:?GH_LOG not set}"
case "$1 $2" in
    "auth status")
        [[ "${GH_AUTH_MODE:-ok}" == "fail" ]] && { echo "not logged in / offline" >&2; exit 1; }
        echo "Logged in to github.com"
        exit 0
        ;;
    "release view")
        case "${GH_STUB_MODE:-published}" in
            no_release) exit 1 ;;
            draft)      echo '{"isDraft":true}'; exit 0 ;;
            *)          echo '{"isDraft":false}'; exit 0 ;;
        esac
        ;;
    *)
        exit 0
        ;;
esac
GH_STUB
    chmod +x "$bin_dir/gh"
}

# `doli release verify` stub. DOLI_STUB_MODE:
#   verify_ok  — exits 0 (threshold met)
#   verify_low — exits 1 with the INC-I-202 sub-threshold shape
write_doli_stub() {
    local bin_dir="$1"
    cat > "$bin_dir/doli" <<'DOLI_STUB'
#!/usr/bin/env bash
echo "doli $*" >> "${DOLI_LOG:?DOLI_LOG not set}"
if [[ "${DOLI_STUB_MODE:-verify_ok}" == "verify_low" ]]; then
    echo "Insufficient signatures: 1/3" >&2
    exit 1
fi
echo "Verified: 3 distinct maintainer signature(s)"
exit 0
DOLI_STUB
    chmod +x "$bin_dir/doli"
}

# --- fixtures ---

# Sets CASE_DIR/WORK_DIR/BIN_DIR/REPO_DIR/WF_FILE/GH_LOG/DOLI_LOG/LOG_FILE.
# with_gh=0 leaves `gh` off the sandbox PATH (S4). jq/git come from /usr/bin, never
# from /opt/homebrew/bin, so "gh absent" stays absent.
new_sandbox() {
    local name="$1" with_gh="${2:-1}"
    CASE_DIR="$TEST_DIR/$name"
    WORK_DIR="$CASE_DIR/work"
    BIN_DIR="$CASE_DIR/bin"
    REPO_DIR="$CASE_DIR/repo"
    WF_FILE="$CASE_DIR/release.yml"
    GH_LOG="$CASE_DIR/gh.log"
    DOLI_LOG="$CASE_DIR/doli.log"
    LOG_FILE="$CASE_DIR/run.log"
    rm -rf "$CASE_DIR"
    mkdir -p "$WORK_DIR" "$BIN_DIR"
    : > "$GH_LOG"
    : > "$DOLI_LOG"
    : > "$LOG_FILE"
    write_doli_stub "$BIN_DIR"
    [ "$with_gh" = "1" ] && write_gh_stub "$BIN_DIR"
    build_git_repo "$REPO_DIR" "$TAG"
    write_workflow "$WF_FILE" "true"
}

# Real local git repo with the given tags on one empty commit (git needs no network).
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

# Minimal release.yml around the softprops/action-gh-release step whose `draft:` value
# is the INC-I-202 M2 gate (real file: .github/workflows/release.yml:592).
write_workflow() {
    local path="$1" draft="$2"
    cat > "$path" <<WF
name: Release
jobs:
  create-release:
    steps:
      - name: Create Release
        uses: softprops/action-gh-release@v2
        with:
          body_path: RELEASE_NOTES.md
          files: release/*
          draft: $draft
          prerelease: false
WF
}

# --- runner ---
# Sources the lib in a subshell (PATH/env mutation cannot leak between partitions),
# resets the caller-owned globals, calls the entry point and ships rc + the three
# *_REASONS back out through files.
run_assert() {
    local token="$1" monitor="${2:-$MONITOR_SCRIPT}" workflow="${3:-$WF_FILE}"
    (
        set +e
        unset REPO_DIR REPO DOLI_CLI
        cd "$WORK_DIR" || exit 1
        # TEST_PATH lets a partition present a host where one tool is genuinely absent;
        # TEST_DOLI_CLI presents an operator profile that exports a stale DOLI_CLI.
        export PATH="${TEST_PATH:-$BIN_DIR:/usr/bin:/bin}"
        [ -n "${TEST_DOLI_CLI:-}" ] && export DOLI_CLI="$TEST_DOLI_CLI"
        export GH_LOG DOLI_LOG
        export GS015_REPO_DIR="$CASE_DIR/repo"
        export GS015_MONITOR="$monitor"
        export GS015_WORKFLOW="$workflow"
        FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
        func="no"
        if [[ -f "$GS015_LIB" ]]; then
            # shellcheck disable=SC1090  # path is a test parameter, resolved at runtime
            . "$GS015_LIB" >/dev/null 2>&1
        fi
        declare -F _gs015_assert >/dev/null 2>&1 && func="yes"
        rc=99
        if [[ "$func" == "yes" ]]; then
            rc=0
            _gs015_assert "$token" > "$LOG_FILE" 2>&1 || rc=$?
        fi
        printf '%s' "$func" > "$CASE_DIR/func"
        printf '%s' "$rc"   > "$CASE_DIR/rc"
        printf '%s' "$FAIL_REASONS" > "$CASE_DIR/fail"
        printf '%s' "$SKIP_REASONS" > "$CASE_DIR/skip"
        printf '%s' "$INFO_REASONS" > "$CASE_DIR/info"
    )
    FUNC_DEFINED="$(cat "$CASE_DIR/func" 2>/dev/null || echo no)"
    RC="$(cat "$CASE_DIR/rc" 2>/dev/null || echo 99)"
    R_FAIL="$(cat "$CASE_DIR/fail" 2>/dev/null || true)"
    R_SKIP="$(cat "$CASE_DIR/skip" 2>/dev/null || true)"
    R_INFO="$(cat "$CASE_DIR/info" 2>/dev/null || true)"
}

mutated() {
    grep -qEi 'release (edit|upload|delete|create)' "$GH_LOG" 2>/dev/null && return 0
    grep -qEi 'release (sign|publish|upload|edit|delete)' "$DOLI_LOG" 2>/dev/null
}

detail() {
    echo "rc=$RC func=$FUNC_DEFINED lib=$GS015_LIB fail=[$R_FAIL] skip=[$R_SKIP] info=[$R_INFO] log=$LOG_FILE"
}

print_header "gauntlet-gs015.sh RED tests (INC-I-202, GS-015)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# ============================================================
# S1 — REQ-202-GS015 (Must) — Decision: a scenario that cannot report a healthy,
# published+verified newest release is a permanent red in the gauntlet, and a
# permanently red scenario gets waived and stops guarding anything.
# ============================================================
new_sandbox "s1_published_verified"
GH_STUB_MODE=published DOLI_STUB_MODE=verify_ok run_assert "$TOKEN_RELEASE"

if lib_ok && [[ "$RC" -eq 0 ]]; then
    test_result "S1 published_and_verified: rc 0 (PASS)" "pass"
else
    test_result "S1 published_and_verified: rc 0 (PASS)" "fail" "$(detail)"
fi

if lib_ok && [[ -n "$R_INFO" ]] && [[ "$R_INFO" == *"$TAG"* ]]; then
    test_result "S1 published_and_verified: INFO_REASONS names the tag" "pass"
else
    test_result "S1 published_and_verified: INFO_REASONS names the tag" "fail" "$(detail)"
fi

# A pass that also wrote a skip reason would print as PASS + yellow skip: line —
# indistinguishable, to an operator, from a scenario that never ran.
if lib_ok && [[ -z "$R_SKIP" ]] && [[ -z "$R_FAIL" ]]; then
    test_result "S1 published_and_verified: SKIP_REASONS and FAIL_REASONS empty" "pass"
else
    test_result "S1 published_and_verified: SKIP_REASONS and FAIL_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S6 — REQ-202-GS015 (Must) — Decision: GS-015 is declared observational; if it can
# reach a mutating gh/doli subcommand it is a release-publishing tool running
# unattended inside the gauntlet, which no confirm-var guards.
# ============================================================
if lib_ok && ! mutated; then
    test_result "S6 read_only: no gh/doli release mutation in the PASS path" "pass"
else
    test_result "S6 read_only: no gh/doli release mutation in the PASS path" "fail" \
        "gh log=$GH_LOG doli log=$DOLI_LOG"
fi

# ============================================================
# S2 — REQ-202-GS015 (Must) — Decision: a draft newest release is the exact INC-I-202
# root cause (v6.26.2 unreachable, `doli upgrade` refused 0/3). If GS-015 reads that
# as green, the gauntlet certifies the failure it was built to catch.
# ============================================================
new_sandbox "s2_newest_is_draft"
GH_STUB_MODE=draft DOLI_STUB_MODE=verify_ok run_assert "$TOKEN_RELEASE"

if lib_ok && [[ "$RC" -eq 1 ]]; then
    test_result "S2 draft_release: rc 1 (FAIL)" "pass"
else
    test_result "S2 draft_release: rc 1 (FAIL)" "fail" "$(detail)"
fi

if lib_ok && [[ "$R_FAIL" == *"$TAG"* ]] && [[ "$(printf '%s' "$R_FAIL" | tr '[:upper:]' '[:lower:]')" == *"draft"* ]]; then
    test_result "S2 draft_release: FAIL_REASONS names the tag and the monitor's diagnosis" "pass"
else
    test_result "S2 draft_release: FAIL_REASONS names the tag and the monitor's diagnosis" "fail" "$(detail)"
fi

if lib_ok && [[ -z "$R_INFO" ]]; then
    test_result "S2 draft_release: INFO_REASONS empty" "pass"
else
    test_result "S2 draft_release: INFO_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S3 — REQ-202-GS015 (Must) — Decision: "published but sub-threshold" is the second
# half of INC-I-202 (empty `"signatures": []`). A release reachable by installers and
# rejected by every fail-closed install gate must not read as green.
# ============================================================
new_sandbox "s3_sub_threshold"
GH_STUB_MODE=published DOLI_STUB_MODE=verify_low run_assert "$TOKEN_RELEASE"

if lib_ok && [[ "$RC" -eq 1 ]]; then
    test_result "S3 sub_threshold_signatures: rc 1 (FAIL)" "pass"
else
    test_result "S3 sub_threshold_signatures: rc 1 (FAIL)" "fail" "$(detail)"
fi

if lib_ok && [[ -n "$R_FAIL" ]] && [[ -z "$R_INFO" ]]; then
    test_result "S3 sub_threshold_signatures: FAIL_REASONS non-empty, INFO_REASONS empty" "pass"
else
    test_result "S3 sub_threshold_signatures: FAIL_REASONS non-empty, INFO_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S4 — REQ-202-GS015 (Must) — Decision: without a preflight, a host with no `gh`
# makes `gh release view` fail, monitor:50 reports "no GitHub release found", and the
# gauntlet reports a FAIL against a release that is in fact fine. One false FAIL is
# how a scenario earns a standing waiver.
# ============================================================
new_sandbox "s4_gh_missing" 0
run_assert "$TOKEN_RELEASE"

if lib_ok && [[ "$RC" -eq 2 ]]; then
    test_result "S4 gh_absent: rc 2 (SKIP, not FAIL)" "pass"
else
    test_result "S4 gh_absent: rc 2 (SKIP, not FAIL)" "fail" "$(detail)"
fi

if lib_ok && [[ "$R_SKIP" == *"gh"* ]]; then
    test_result "S4 gh_absent: SKIP_REASONS names gh" "pass"
else
    test_result "S4 gh_absent: SKIP_REASONS names gh" "fail" "$(detail)"
fi

# rc 2 alone prints as PASS (gauntlet.sh:639). Only a non-empty SKIP_REASONS with an
# empty INFO_REASONS tells the operator the check did not happen.
if lib_ok && [[ -z "$R_INFO" ]] && [[ -z "$R_FAIL" ]]; then
    test_result "S4 gh_absent: INFO_REASONS and FAIL_REASONS empty (skip is not a pass)" "pass"
else
    test_result "S4 gh_absent: INFO_REASONS and FAIL_REASONS empty (skip is not a pass)" "fail" "$(detail)"
fi

# ============================================================
# S5 — REQ-202-GS015 (Must) — Decision: unauthenticated or offline `gh` is the common
# case on a developer laptop. Reporting it as a release defect makes every local
# gauntlet run red for a reason that has nothing to do with the release.
# ============================================================
new_sandbox "s5_gh_unauthenticated"
GH_AUTH_MODE=fail GH_STUB_MODE=published DOLI_STUB_MODE=verify_ok run_assert "$TOKEN_RELEASE"

if lib_ok && [[ "$RC" -eq 2 ]]; then
    test_result "S5 gh_unauthenticated: rc 2 (SKIP, not FAIL)" "pass"
else
    test_result "S5 gh_unauthenticated: rc 2 (SKIP, not FAIL)" "fail" "$(detail)"
fi

R_SKIP_LC="$(printf '%s' "$R_SKIP" | tr '[:upper:]' '[:lower:]')"
if lib_ok && { [[ "$R_SKIP_LC" == *"auth"* ]] || [[ "$R_SKIP_LC" == *"offline"* ]]; }; then
    test_result "S5 gh_unauthenticated: SKIP_REASONS says unauthenticated/offline" "pass"
else
    test_result "S5 gh_unauthenticated: SKIP_REASONS says unauthenticated/offline" "fail" "$(detail)"
fi

if lib_ok && [[ -z "$R_INFO" ]] && [[ -z "$R_FAIL" ]]; then
    test_result "S5 gh_unauthenticated: INFO_REASONS and FAIL_REASONS empty" "pass"
else
    test_result "S5 gh_unauthenticated: INFO_REASONS and FAIL_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S7 — REQ-202-GS015 (Must) — Decision: the M2 draft gate is the only thing that keeps
# an unsigned CI artifact unreachable. The gauntlet must be able to report it intact,
# or the guard has no standing check at all.
# ============================================================
new_sandbox "s7_workflow_drafts"
write_workflow "$WF_FILE" "true"
run_assert "$TOKEN_WORKFLOW"

if lib_ok && [[ "$RC" -eq 0 ]]; then
    test_result "S7 workflow_draft_true: rc 0 (PASS)" "pass"
else
    test_result "S7 workflow_draft_true: rc 0 (PASS)" "fail" "$(detail)"
fi

# ============================================================
# S8 — REQ-202-GS015 (Must) — Decision: a revert of `draft: true` republishes unsigned
# releases directly to every installer, silently. Nothing else in the repo notices a
# one-word change on release.yml:592.
# ============================================================
new_sandbox "s8_workflow_reverted"
write_workflow "$WF_FILE" "false"
run_assert "$TOKEN_WORKFLOW"

if lib_ok && [[ "$RC" -eq 1 ]]; then
    test_result "S8 workflow_draft_false: rc 1 (FAIL)" "pass"
else
    test_result "S8 workflow_draft_false: rc 1 (FAIL)" "fail" "$(detail)"
fi

R_FAIL_LC="$(printf '%s' "$R_FAIL" | tr '[:upper:]' '[:lower:]')"
if lib_ok && [[ "$R_FAIL_LC" == *"release.yml"* ]] && [[ "$R_FAIL_LC" == *"draft"* ]]; then
    test_result "S8 workflow_draft_false: FAIL_REASONS names release.yml and draft" "pass"
else
    test_result "S8 workflow_draft_false: FAIL_REASONS names release.yml and draft" "fail" "$(detail)"
fi

# ============================================================
# S9 — REQ-202-GS015 (Must) — Decision: an unreadable workflow file means the check did
# not run. Reading "no `draft: false` found" out of a missing file is a green built on
# nothing — the vacuous-pass shape this whole token exists to detect.
# ============================================================
new_sandbox "s9_workflow_missing"
run_assert "$TOKEN_WORKFLOW" "$MONITOR_SCRIPT" "$CASE_DIR/does-not-exist.yml"

if lib_ok && [[ "$RC" -eq 2 ]]; then
    test_result "S9 workflow_missing: rc 2 (SKIP)" "pass"
else
    test_result "S9 workflow_missing: rc 2 (SKIP)" "fail" "$(detail)"
fi

if lib_ok && [[ -n "$R_SKIP" ]] && [[ -z "$R_INFO" ]]; then
    test_result "S9 workflow_missing: SKIP_REASONS non-empty, INFO_REASONS empty" "pass"
else
    test_result "S9 workflow_missing: SKIP_REASONS non-empty, INFO_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S10 — REQ-202-GS015 (Should) — Decision: a typo in the assertions column of
# gauntlet_scenarios would otherwise make the scenario report PASS while asserting
# nothing at all.
# ============================================================
new_sandbox "s10_unknown_token"
run_assert "gs015-not-a-real-token"

if lib_ok && [[ "$RC" -ne 0 ]]; then
    test_result "S10 unknown_token: rc non-zero (never a silent pass)" "pass"
else
    test_result "S10 unknown_token: rc non-zero (never a silent pass)" "fail" "$(detail)"
fi

# --- helpers for the registration and missing-tool partitions ---

# The runner dispatches only tokens read from gauntlet_scenarios (gauntlet.sh:658) and
# .omega/ is gitignored, so scripts/gauntlet-seed.sql is the only version-controlled
# registration. Applied to a throwaway DB — never to .omega/memory.db — so the row is
# parsed exactly as the runner reads it.
seed_gs015_assertions() {
    local db="$TEST_DIR/seed-check.db"
    rm -f "$db"
    if command -v sqlite3 >/dev/null 2>&1; then
        sqlite3 "$db" "CREATE TABLE gauntlet_scenarios (scenario_id TEXT PRIMARY KEY, name TEXT, description TEXT, incident_ids TEXT, assertions TEXT, scale_params TEXT, runner TEXT, status TEXT);" >/dev/null 2>&1
        sqlite3 "$db" < "$SEED_FILE" >/dev/null 2>&1
        sqlite3 "$db" "SELECT assertions FROM gauntlet_scenarios WHERE scenario_id='GS-015' AND status='active';" 2>/dev/null
    else
        grep -A8 "'GS-015'," "$SEED_FILE" | grep -oE "'gs015-[a-z0-9,-]+'" | tr -d "'" | head -1
    fi
}

# A PATH mirror of /bin and /usr/bin with one tool left out: `command -v <tool>` has to
# genuinely fail, and a shim that exits 127 is still on PATH.
mirror_path_without() {
    local dir="$1/nobin-$2" omit="$2" f b
    rm -rf "$dir"; mkdir -p "$dir"
    for f in /bin/* /usr/bin/*; do
        b="${f##*/}"
        [ "$b" = "$omit" ] || ln -s "$f" "$dir/$b" 2>/dev/null
    done
    printf '%s' "$dir"
}

# release.yml where the release step publishes non-draft and an unrelated job carries a
# stray `draft: true` — the shape a file-global grep reads as green.
write_workflow_stray_draft() {
    cat > "$1" <<'WF'
name: Release
jobs:
  create-release:
    steps:
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          body_path: RELEASE_NOTES.md
          files: release/*
          draft: false
          prerelease: false
  mirror-nightly:
    steps:
      - name: Mirror build to the nightly channel
        uses: some/other-release-action@v1
        with:
          draft: true
WF
}

# ============================================================
# S11 — REQ-202-GS015 (Must) — Decision: assert() is only ever called with tokens read
# from gauntlet_scenarios (gauntlet.sh:658), and .omega/ is gitignored, so a scenario
# absent from the seed does not exist on any other machine. A green suite over a library
# the runner never reaches is the false-assurance shape GS-015 exists to catch.
# ============================================================
SEED_ASSERTIONS="$(seed_gs015_assertions)"

if [ -n "$SEED_ASSERTIONS" ]; then
    test_result "S11 seed_registration: gauntlet-seed.sql registers GS-015 as active" "pass"
else
    test_result "S11 seed_registration: gauntlet-seed.sql registers GS-015 as active" "fail" \
        "no active GS-015 row parsed from $SEED_FILE"
fi

if [[ "$SEED_ASSERTIONS" == *"$TOKEN_RELEASE"* ]] && [[ "$SEED_ASSERTIONS" == *"$TOKEN_WORKFLOW"* ]]; then
    test_result "S11 seed_registration: registered assertions list both GS-015 tokens" "pass"
else
    test_result "S11 seed_registration: registered assertions list both GS-015 tokens" "fail" \
        "assertions=[$SEED_ASSERTIONS]"
fi

# Registration and dispatch must agree: a token in the seed that lands on the unknown-token
# arm asserts nothing while still counting as a scenario.
new_sandbox "s11_seed_dispatch"
DISPATCH_OK=1
DISPATCH_DETAIL=""
[ -n "$SEED_ASSERTIONS" ] || { DISPATCH_OK=0; DISPATCH_DETAIL="no tokens registered"; }
S11_OLDIFS="$IFS"; IFS=','
for tok in $SEED_ASSERTIONS; do
    IFS="$S11_OLDIFS"
    run_assert "$tok"
    if ! lib_ok || [[ "$R_FAIL" == *"unknown"* ]]; then
        DISPATCH_OK=0
        DISPATCH_DETAIL="$DISPATCH_DETAIL [$tok -> rc=$RC fail=$R_FAIL]"
    fi
    IFS=','
done
IFS="$S11_OLDIFS"

if [ "$DISPATCH_OK" = "1" ]; then
    test_result "S11 seed_registration: every registered token reaches a real assertion arm" "pass"
else
    test_result "S11 seed_registration: every registered token reaches a real assertion arm" "fail" \
        "$DISPATCH_DETAIL"
fi

# ============================================================
# S12 — REQ-202-GS015 (Must) — Decision: monitor:56 is fail-closed — a jq that is not
# there exits 127, IS_DRAFT falls back to "true", and the monitor reports "release is
# still a DRAFT" about a release that is published. That is a false FAIL whose text
# names the INC-I-202 root cause, on a host that simply has no jq.
# ============================================================
new_sandbox "s12_jq_missing"
TEST_PATH="$BIN_DIR:$(mirror_path_without "$CASE_DIR" jq)"
run_assert "$TOKEN_RELEASE"
unset TEST_PATH

if lib_ok && [[ "$RC" -eq 2 ]]; then
    test_result "S12 jq_absent: rc 2 (SKIP, not FAIL)" "pass"
else
    test_result "S12 jq_absent: rc 2 (SKIP, not FAIL)" "fail" "$(detail)"
fi

if lib_ok && [[ "$R_SKIP" == *"jq"* ]]; then
    test_result "S12 jq_absent: SKIP_REASONS names jq" "pass"
else
    test_result "S12 jq_absent: SKIP_REASONS names jq" "fail" "$(detail)"
fi

if lib_ok && [[ -z "$R_FAIL" ]] && [[ -z "$R_INFO" ]]; then
    test_result "S12 jq_absent: FAIL_REASONS and INFO_REASONS empty" "pass"
else
    test_result "S12 jq_absent: FAIL_REASONS and INFO_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S13 — REQ-202-GS015 (Must) — Decision: a stale DOLI_CLI in a shell profile makes the
# monitor run a path that does not exist; the shell returns 127 and the monitor blames
# the signatures. The operator is then sent to re-sign a correctly signed release —
# the environment is broken, not the release.
# ============================================================
new_sandbox "s13_doli_cli_stale"
TEST_DOLI_CLI="$CASE_DIR/stale/doli"
run_assert "$TOKEN_RELEASE"
unset TEST_DOLI_CLI

if lib_ok && [[ "$RC" -eq 2 ]]; then
    test_result "S13 doli_cli_not_executable: rc 2 (SKIP, not FAIL)" "pass"
else
    test_result "S13 doli_cli_not_executable: rc 2 (SKIP, not FAIL)" "fail" "$(detail)"
fi

if lib_ok && [[ "$R_SKIP" == *"$CASE_DIR/stale/doli"* ]] && [[ -z "$R_FAIL" ]]; then
    test_result "S13 doli_cli_not_executable: SKIP_REASONS names the path, FAIL_REASONS empty" "pass"
else
    test_result "S13 doli_cli_not_executable: SKIP_REASONS names the path, FAIL_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S14 — REQ-202-GS015 (Must) — Decision: the monitor resolves the newest tag with git.
# Without git it finds no tag and reports UNHEALTHY, which reads as a release defect on
# a host that cannot look at the tags at all.
# ============================================================
new_sandbox "s14_git_missing"
TEST_PATH="$BIN_DIR:$(mirror_path_without "$CASE_DIR" git)"
run_assert "$TOKEN_RELEASE"
unset TEST_PATH

if lib_ok && [[ "$RC" -eq 2 ]]; then
    test_result "S14 git_absent: rc 2 (SKIP, not FAIL)" "pass"
else
    test_result "S14 git_absent: rc 2 (SKIP, not FAIL)" "fail" "$(detail)"
fi

if lib_ok && [[ "$R_SKIP" == *"git"* ]] && [[ -z "$R_FAIL" ]]; then
    test_result "S14 git_absent: SKIP_REASONS names git, FAIL_REASONS empty" "pass"
else
    test_result "S14 git_absent: SKIP_REASONS names git, FAIL_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S15 — REQ-202-GS015 (Must) — Decision: actions/checkout fetches no tags at the default
# fetch-depth, so any CI or tarball checkout has none. "No tags here" is an environment
# fact; "tags exist but the newest has no release" stays a FAIL.
# ============================================================
new_sandbox "s15_no_tags"
build_git_repo "$REPO_DIR"
run_assert "$TOKEN_RELEASE"

if lib_ok && [[ "$RC" -eq 2 ]]; then
    test_result "S15 no_v_tag: rc 2 (SKIP, not FAIL)" "pass"
else
    test_result "S15 no_v_tag: rc 2 (SKIP, not FAIL)" "fail" "$(detail)"
fi

if lib_ok && [[ "$R_SKIP" == *"$REPO_DIR"* ]] && [[ -z "$R_FAIL" ]]; then
    test_result "S15 no_v_tag: SKIP_REASONS names the repo dir, FAIL_REASONS empty" "pass"
else
    test_result "S15 no_v_tag: SKIP_REASONS names the repo dir, FAIL_REASONS empty" "fail" "$(detail)"
fi

# ============================================================
# S16 — REQ-202-GS015 (Must) — Decision: a file-global grep lets a second release path
# (nightly, RC, mirror) carry the `draft: true` that satisfies the gate while the real
# release step publishes straight to Latest. The gate must be read where it acts.
# ============================================================
new_sandbox "s16_stray_draft_outside_step"
write_workflow_stray_draft "$WF_FILE"
run_assert "$TOKEN_WORKFLOW"

if lib_ok && [[ "$RC" -eq 1 ]]; then
    test_result "S16 stray_draft_outside_release_step: rc 1 (FAIL)" "pass"
else
    test_result "S16 stray_draft_outside_release_step: rc 1 (FAIL)" "fail" "$(detail)"
fi

S16_FAIL_LC="$(printf '%s' "$R_FAIL" | tr '[:upper:]' '[:lower:]')"
if lib_ok && [[ "$S16_FAIL_LC" == *"draft"* ]] && [[ -z "$R_INFO" ]]; then
    test_result "S16 stray_draft_outside_release_step: FAIL_REASONS names the draft gate" "pass"
else
    test_result "S16 stray_draft_outside_release_step: FAIL_REASONS names the draft gate" "fail" "$(detail)"
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
