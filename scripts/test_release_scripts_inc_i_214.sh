#!/usr/bin/env bash
# OUTPUT CONTRACT: the INC-I-214 release scripts — scripts/sign-release.sh,
# scripts/publish-release.sh, scripts/monitor-release-signed.sh (REQ-214-003/004/006).
# REQ-214-005 covers .claude/skills/release/SKILL.md and is verified by review, not here:
# a step table is prose, and no argv recorder can read it.
#   O1 exit code      — 0 promoted/healthy · non-zero refused; the only contract cron reads
#   O2 combined text  — stdout+stderr of the script, the operator's whole diagnosis
#   O3 gh argv        — every `gh` invocation, in call order (CALL_LOG lines `gh ...`)
#   O4 doli argv      — every `doli` invocation, in call order (CALL_LOG lines `doli ...`)
#                       O3 and O4 share ONE log: the ORDER of a download against a sign
#                       is the property under test, so they cannot live in two files
#   O5 fetched bytes  — absolute paths `gh release download` actually wrote (WROTE_LOG);
#                       the operand a signer must consume instead of re-fetching it
#   O6 release state  — the mutating subset of O3 (edit|upload|delete|create); INC-I-202
#                       says nothing leaves DRAFT before the signatures verify
#   PATHS:
#     sign-release.sh <v>
#       -> keys present? release exists? CHECKSUMS.txt in assets? -> NO => O1!=0
#       -> per key: doli release sign -> signer rc!=0 => O1!=0 + O2 must carry the signer's text
#       -> gh release download CHECKSUMS.txt -> assemble -> gh release upload (O6 upload only)
#     publish-release.sh <v>
#       -> gh release download SIGNATURES.json+CHECKSUMS.txt
#       -> manifest absent/malformed/sub-threshold => O1!=0, O2 REFUSING, O6 empty
#       -> doli release verify rc!=0             => O1!=0, O2 REFUSING, O6 empty
#       -> notes carry the CI banner ? gh release edit --notes-file ... : gh release edit ...
#     monitor-release-signed.sh
#       -> newest v* tag in REPO_DIR -> gh release view isDraft -> doli release verify
#       -> read-only in every path: O6 must stay empty
# INPUT PARTITIONS:
#   P1 sign: 3 keys sign clean, draft holds CHECKSUMS.txt   — O4 x3 with --checksums, O5 feeds it, O6 no edit
#   P2 sign: maintainer-2 signer exits non-zero on stderr   — O1!=0, O2 carries the signer's own text + the key
#   P3 publish: 3 signatures, notes carry the CI banner     — O3 edit --notes-file --draft=false --prerelease=false --latest
#   P4 publish: 3 signatures, notes empty                   — O3 edit --draft=false --prerelease=false --latest
#   P5 publish: manifest carries 2 signatures               — O1!=0, O2 REFUSING names 2 and 3, O6 empty
#   P6 publish: SIGNATURES.json absent from the release     — O1!=0, O2 REFUSING, O6 empty
#   P7 monitor: newest tag published and verifying          — O4 verify --version --dir --trust-root, O5 before it, O6 empty
# MATRIX: 6 outputs x 7 partitions (only cells the path reaches are asserted)
#   P1: O4 O5 O6 | P2: O1 O2 | P3: O3 O4 | P4: O3 O4
#   P5: O1 O2 O6 | P6: O1 O2 O6 | P7: O3 O4 O5 O6
#
# TDD RED tests for the M2 script changes, which HAVE NOT BEEN MADE YET.
# `gh` and `doli` are stubbed on PATH (and DOLI_CLI points at the same stub), the keys are
# throwaway JSON files, and the tag fixture is a REAL local git repo: no network, no live
# GitHub release, no real ~/.ssh/doli, no tag written into the real repo.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SIGN_SCRIPT="$PROJECT_ROOT/scripts/sign-release.sh"
PUBLISH_SCRIPT="$PROJECT_ROOT/scripts/publish-release.sh"
MONITOR_SCRIPT="$PROJECT_ROOT/scripts/monitor-release-signed.sh"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/doli-inc-i-214-test-XXXXXX")"
VERSION_BARE="6.28.0"
TAG="v${VERSION_BARE}"
THRESHOLD_EXPECTED="3"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

TESTS_PASSED=0
TESTS_FAILED=0
TESTS_TOTAL=0
CUR_CASE=""
CASE_STATUS_a="PASS"; CASE_STATUS_b="PASS"; CASE_STATUS_c="PASS"
CASE_STATUS_d="PASS"; CASE_STATUS_e="PASS"

# Stub-mode knobs, declared once so `set -u` never trips on an unset export.
GH_NO_RELEASE=0
GH_IS_DRAFT=false
GH_NOTES=""
GH_TAG="$TAG"
GH_ASSET_SRC_DIR=""
STUB_SIGN_FAIL_KEY=""
STUB_VERIFY_RC=0
BANNER_BEGIN='<!-- DOLI-UNSIGNED-DRAFT-WARNING:BEGIN -->'
BANNER_END='<!-- DOLI-UNSIGNED-DRAFT-WARNING:END -->'

print_header() {
    echo
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}  $1${NC}"
    echo -e "${BLUE}========================================${NC}"
    echo
}

case_begin() {
    CUR_CASE="$1"
    echo
    echo -e "${CYAN}--- CASE $1 --- $2${NC}"
}

test_result() {
    local test_name=$1 result=$2 detail=$3
    TESTS_TOTAL=$((TESTS_TOTAL + 1))
    if [ "$result" = "pass" ]; then
        TESTS_PASSED=$((TESTS_PASSED + 1))
        echo -e "  ${GREEN}[PASS]${NC} $test_name"
    else
        TESTS_FAILED=$((TESTS_FAILED + 1))
        [ -n "$CUR_CASE" ] && eval "CASE_STATUS_${CUR_CASE}=FAIL"
        echo -e "  ${RED}[FAIL]${NC} $test_name"
        [ -n "$detail" ] && echo -e "         ${RED}$detail${NC}"
    fi
}

# shellcheck disable=SC2329  # invoked indirectly via trap below
cleanup() {
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT

# --- stub writers ---

# `gh` stub. Records argv to the shared CALL_LOG, then serves release state from the
# per-case fixture directory GH_ASSET_SRC_DIR: a file present there is an asset on the
# release, a file absent is an asset the release does not carry. `release download`
# copies the bytes out and records every path it wrote to WROTE_LOG (O5).
write_gh_stub() {
    cat > "$1/gh" <<'GH_STUB'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "${CALL_LOG:?CALL_LOG not set}"

json=""; dir=""; out=""; patterns=""; prev=""
for a in "$@"; do
    case "$prev" in
        --json)      json="$a" ;;
        --dir)       dir="$a" ;;
        -O|--output) out="$a" ;;
        --pattern|-p) patterns="$patterns $a" ;;
    esac
    case "$a" in
        --json=*)    json="${a#--json=}" ;;
        --dir=*)     dir="${a#--dir=}" ;;
        --output=*)  out="${a#--output=}" ;;
        --pattern=*) patterns="$patterns ${a#--pattern=}" ;;
    esac
    prev="$a"
done

serve_asset() {
    local name="$1" dest="$2"
    [ -f "${GH_ASSET_SRC_DIR:-/nonexistent}/$name" ] || return 1
    cp "${GH_ASSET_SRC_DIR}/$name" "$dest" || return 1
    printf '%s\n' "$dest" >> "${WROTE_LOG:?WROTE_LOG not set}"
    return 0
}

case "$1 ${2:-}" in
    "auth status")
        echo "Logged in to github.com"
        exit 0
        ;;
    "release view")
        [ "${GH_NO_RELEASE:-0}" = "1" ] && exit 1
        case "$json" in
            *tagName*) printf '%s\n' "${GH_TAG:-v0.0.0}" ;;
            *assets*)  [ -d "${GH_ASSET_SRC_DIR:-/nonexistent}" ] && ls -1 "$GH_ASSET_SRC_DIR" ;;
            *isDraft*) printf '{"isDraft":%s}\n' "${GH_IS_DRAFT:-false}" ;;
            *body*)    printf '%s\n' "${GH_NOTES:-}" ;;
            *)         printf '{}\n' ;;
        esac
        exit 0
        ;;
    "release download")
        d="${dir:-$PWD}"
        mkdir -p "$d"
        want="$patterns"
        [ -n "$want" ] || want="CHECKSUMS.txt SIGNATURES.json"
        rc=0
        for p in $want; do
            case "$p" in
                *CHECKSUMS.txt*)   serve_asset CHECKSUMS.txt   "${out:-$d/CHECKSUMS.txt}"   || rc=1 ;;
                *SIGNATURES.json*) serve_asset SIGNATURES.json "${out:-$d/SIGNATURES.json}" || rc=1 ;;
            esac
        done
        exit $rc
        ;;
    *)
        exit 0
        ;;
esac
GH_STUB
    chmod +x "$1/gh"
}

# `doli` stub. Records argv with a fixed `doli` prefix (never $0, which is a DOLI_CLI
# path) so the recorded shape is stable. STUB_SIGN_FAIL_KEY names the maintainer key
# basename whose signer fails, writing its OWN distinctive text to stderr.
write_doli_stub() {
    cat > "$1/doli" <<'DOLI_STUB'
#!/usr/bin/env bash
printf 'doli %s\n' "$*" >> "${CALL_LOG:?CALL_LOG not set}"

sub=""; keyfile=""; prev=""
for a in "$@"; do
    if [ "$prev" = "release" ] && [ -z "$sub" ]; then sub="$a"; fi
    if [ "$prev" = "--key" ]; then keyfile="$a"; fi
    prev="$a"
done

case "$sub" in
    sign)
        if [ -n "${STUB_SIGN_FAIL_KEY:-}" ] && [ "$(basename "${keyfile:-}")" = "$STUB_SIGN_FAIL_KEY" ]; then
            echo "STUB_SIGNER_BOOM: key rejected" >&2
            exit 3
        fi
        cat <<'SIGJSON'
{
  "maintainer": "stub-maintainer",
  "pubkey": "aa00",
  "signature": "bb11"
}
SIGJSON
        exit 0
        ;;
    verify)
        echo "Verified: 3 distinct maintainer signature(s)"
        exit "${STUB_VERIFY_RC:-0}"
        ;;
    *)
        exit 0
        ;;
esac
DOLI_STUB
    chmod +x "$1/doli"
}

# --- fixtures ---

# Sets CASE_DIR/WORK_DIR/BIN_DIR/KEY_DIR/ASSETS/REPO_DIR/CALL_LOG/WROTE_LOG/OUT_FILE.
new_sandbox() {
    CASE_DIR="$TEST_DIR/$1"
    WORK_DIR="$CASE_DIR/work"
    BIN_DIR="$CASE_DIR/bin"
    KEY_DIR="$CASE_DIR/keys"
    ASSETS="$CASE_DIR/assets"
    REPO_DIR="$CASE_DIR/repo"
    CALL_LOG="$CASE_DIR/calls.log"
    WROTE_LOG="$CASE_DIR/wrote.log"
    OUT_FILE="$CASE_DIR/out.log"
    rm -rf "$CASE_DIR"
    mkdir -p "$WORK_DIR" "$BIN_DIR" "$KEY_DIR" "$ASSETS"
    : > "$CALL_LOG"; : > "$WROTE_LOG"; : > "$OUT_FILE"
    write_gh_stub "$BIN_DIR"
    write_doli_stub "$BIN_DIR"
    local i
    for i in 1 2 3; do
        printf '{"name":"maintainer-%s","secret":"00"}\n' "$i" > "$KEY_DIR/maintainer-$i.json"
    done
    printf 'abc123  doli-node-linux-x86_64\ndef456  doli-linux-x86_64\n' > "$ASSETS/CHECKSUMS.txt"
    GH_ASSET_SRC_DIR="$ASSETS"
    GH_NO_RELEASE=0; GH_IS_DRAFT=false; GH_NOTES=""; GH_TAG="$TAG"
    STUB_SIGN_FAIL_KEY=""; STUB_VERIFY_RC=0
    build_git_repo "$REPO_DIR" "$TAG"
}

# A manifest with exactly $1 signature entries — the INC-I-202 threshold input.
write_manifest() {
    local n="$1" i sigs=""
    for ((i = 0; i < n; i++)); do
        sigs="$sigs{\"maintainer\":\"m$i\",\"pubkey\":\"aa0$i\",\"signature\":\"bb1$i\"},"
    done
    printf '{"version":"%s","checksums_sha256":"cc22","signatures":[%s]}\n' \
        "$VERSION_BARE" "${sigs%,}" > "$ASSETS/SIGNATURES.json"
}

# Real local git repo with the given tags on one empty commit (git needs no network,
# and the real repo is never tagged by this harness).
build_git_repo() {
    local repo_dir="$1"
    shift
    rm -rf "$repo_dir"
    mkdir -p "$repo_dir"
    (
        cd "$repo_dir" || exit 1
        git init -q
        git -c user.email=t@t -c user.name=t commit --allow-empty -q -m x
        for tag in "$@"; do git tag "$tag"; done
    )
}

# --- runner ---
# Runs a script in a subshell whose PATH sees only the stubs plus /usr/bin and /bin, from
# a cwd with no ./target/release/doli, so nothing resolves to a real gh, doli or key.
run_script() {
    local script="$1"
    shift
    (
        cd "$WORK_DIR" || exit 1
        export PATH="$BIN_DIR:/usr/bin:/bin"
        export CALL_LOG WROTE_LOG GH_ASSET_SRC_DIR
        export GH_NO_RELEASE GH_IS_DRAFT GH_NOTES GH_TAG
        export STUB_SIGN_FAIL_KEY STUB_VERIFY_RC
        export DOLI_CLI="$BIN_DIR/doli"
        export KEY_DIR REPO_DIR
        export REPO="doli-network/doli"
        export THRESHOLD="$THRESHOLD_EXPECTED"
        bash "$script" "$@"
    ) > "$OUT_FILE" 2>&1
    RC=$?
}

# --- assertion helpers ---

# First 1-based line number in CALL_LOG matching the extended regex, or empty.
line_of() {
    grep -nE "$1" "$CALL_LOG" 2>/dev/null | head -1 | cut -d: -f1
}

# Value of a flag inside one recorded argv line: both `--flag value` and `--flag=value`.
flag_value() {
    awk -v f="$2" '{
        for (i = 1; i <= NF; i++) {
            if ($i == f && i < NF) { print $(i + 1); exit }
            if (index($i, f "=") == 1) { print substr($i, length(f) + 2); exit }
        }
    }' <<<"$1"
}

gh_lines()   { grep -E '^gh ' "$CALL_LOG" 2>/dev/null; }
doli_lines() { grep -E '^doli ' "$CALL_LOG" 2>/dev/null; }
mutating()   { grep -qE '^gh release (edit|upload|delete|delete-asset|create)' "$CALL_LOG" 2>/dev/null; }
out_text()   { cat "$OUT_FILE" 2>/dev/null; }
detail()     { echo "rc=$RC calls=$CALL_LOG out=$OUT_FILE"; }

print_header "INC-I-214 release-script RED tests (REQ-214-003/004/006)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# ============================================================
# CASE a — REQ-214-001 script half (Must) — Decision: whether the signer is handed the
# authenticated bytes the caller already downloaded. If sign-release.sh keeps letting the
# CLI fetch CHECKSUMS.txt from the public URL, a DRAFT 404s and NOTHING can be signed
# before publication — which forces the operator to publish first and inverts INC-I-202.
# The ORDER is the whole assertion: a download after the loop cannot feed the loop.
# ============================================================
case_begin a "sign-release.sh feeds the downloaded CHECKSUMS.txt to every signer"
new_sandbox "a_sign_happy"
run_script "$SIGN_SCRIPT" "$VERSION_BARE"

SIGN_COUNT="$(doli_lines | grep -cE 'release sign')"
if [ "$SIGN_COUNT" -eq 3 ]; then
    test_result "a1 exactly 3 'doli release sign' invocations recorded" "pass" ""
else
    test_result "a1 exactly 3 'doli release sign' invocations recorded" "fail" \
        "count=$SIGN_COUNT $(detail)"
fi

# Every signer must receive a --checksums path that the gh download actually wrote.
# Asserting against WROTE_LOG (not a hardcoded name) keeps the test blind to how the
# developer spells the download, while still refusing a path nobody fetched.
A_MISSING=""
while IFS= read -r line; do
    [ -n "$line" ] || continue
    cpath="$(flag_value "$line" --checksums)"
    if [ -z "$cpath" ]; then
        A_MISSING="$A_MISSING [no --checksums]"
    elif ! grep -qxF "$cpath" "$WROTE_LOG" 2>/dev/null; then
        A_MISSING="$A_MISSING [$cpath not downloaded]"
    fi
done <<<"$(doli_lines | grep -E 'release sign')"

if [ "$SIGN_COUNT" -eq 3 ] && [ -z "$A_MISSING" ]; then
    test_result "a2 each signer gets --checksums <a path gh release download wrote>" "pass" ""
else
    test_result "a2 each signer gets --checksums <a path gh release download wrote>" "fail" \
        "$A_MISSING wrote=$(tr '\n' ' ' < "$WROTE_LOG") $(detail)"
fi

A_DL_LINE="$(line_of '^gh release download')"
A_SIGN_LINE="$(line_of '^doli .*release sign')"
if [ -n "$A_DL_LINE" ] && [ -n "$A_SIGN_LINE" ] && [ "$A_DL_LINE" -lt "$A_SIGN_LINE" ]; then
    test_result "a3 'gh release download' is recorded BEFORE the first signer call" "pass" ""
else
    test_result "a3 'gh release download' is recorded BEFORE the first signer call" "fail" \
        "download_line=${A_DL_LINE:-none} first_sign_line=${A_SIGN_LINE:-none} $(detail)"
fi

# INC-I-202: signing must never make anything visible. Upload onto the draft is expected;
# an edit, or any --draft flag, would be the signer quietly publishing.
if ! grep -qE '^gh release edit' "$CALL_LOG" 2>/dev/null; then
    test_result "a4 no 'gh release edit' — signing never promotes (INC-I-202)" "pass" ""
else
    test_result "a4 no 'gh release edit' — signing never promotes (INC-I-202)" "fail" \
        "$(gh_lines | grep -E 'release edit' | tr '\n' ' ')"
fi

if ! gh_lines | grep -q -- '--draft'; then
    test_result "a5 no --draft flag anywhere in the recorded gh argv" "pass" ""
else
    test_result "a5 no --draft flag anywhere in the recorded gh argv" "fail" \
        "$(gh_lines | grep -- '--draft' | tr '\n' ' ')"
fi

# ============================================================
# CASE b — REQ-214-004 (Must) — Decision: whether an operator can tell WHY a signature
# failed. `2>/dev/null` on the signer turns "this key is wrong / this file is unreadable /
# this version is malformed" into one generic "no valid JSON object", and under `set -e`
# the assignment aborts before even that prints. The operator then re-runs a correct
# command repeatedly, blind.
# ============================================================
case_begin b "sign-release.sh surfaces the signer's own stderr and names the key"
new_sandbox "b_signer_stderr"
STUB_SIGN_FAIL_KEY="maintainer-2.json"
run_script "$SIGN_SCRIPT" "$VERSION_BARE"

if grep -qF 'STUB_SIGNER_BOOM: key rejected' "$OUT_FILE" 2>/dev/null; then
    test_result "b1 the signer's own stderr text reaches the operator" "pass" ""
else
    test_result "b1 the signer's own stderr text reaches the operator" "fail" \
        "captured=[$(out_text | tail -3 | tr '\n' '|')] $(detail)"
fi

# The per-key banner is printed BEFORE the call, so naming must be proved on an error
# line: keys 1 and 2 are both announced, only key 2 failed.
if grep -iE '(error|fail|refus)' "$OUT_FILE" 2>/dev/null | grep -qE 'maintainer[-_]2'; then
    test_result "b2 an error line names the maintainer key that failed" "pass" ""
else
    test_result "b2 an error line names the maintainer key that failed" "fail" \
        "no error-context line names maintainer-2 $(detail)"
fi

if [ "$RC" -ne 0 ]; then
    test_result "b3 exit non-zero — a failed signer never yields a manifest" "pass" ""
else
    test_result "b3 exit non-zero — a failed signer never yields a manifest" "fail" "$(detail)"
fi

if ! grep -qE '^gh release upload' "$CALL_LOG" 2>/dev/null; then
    test_result "b4 no SIGNATURES.json uploaded after a signer failure" "pass" ""
else
    test_result "b4 no SIGNATURES.json uploaded after a signer failure" "fail" "$(detail)"
fi

# ============================================================
# CASE c — REQ-214-003 + REQ-214-002 (Must) — Decision: whether promotion actually lands.
# A draft created with prerelease:true is promoted with --draft=false alone into a
# published NON-Latest prerelease (or a 422), which is exactly the stuck v6.28.0 state
# that produced the manual workaround. And verify against a stale local on-chain root
# reports 0/3 on a correctly signed release, so the operator is told to re-sign bytes
# that are already fine. Both `gh release edit` branches must carry the same flags —
# a fix applied to one branch only breaks whichever release notes take the other path.
# ============================================================
case_begin c "publish-release.sh promotes with prerelease cleared and verifies via bootstrap"

assert_publish_promotion() {
    local label="$1"
    local edit_line dl_line verify_line dl_dir v_dir missing=""

    edit_line="$(gh_lines | grep -E '^gh release edit' | head -1)"
    if [ -z "$edit_line" ]; then
        test_result "c$label promotion: 'gh release edit' recorded" "fail" "$(detail)"
    else
        test_result "c$label promotion: 'gh release edit' recorded" "pass" ""
    fi

    grep -q -- '--draft=false'      <<<"$edit_line" || missing="$missing --draft=false"
    grep -q -- '--prerelease=false' <<<"$edit_line" || missing="$missing --prerelease=false"
    grep -q -- '--latest'           <<<"$edit_line" || missing="$missing --latest"
    if [ -n "$edit_line" ] && [ -z "$missing" ]; then
        test_result "c$label promotion: edit argv carries --draft=false --prerelease=false --latest" "pass" ""
    else
        test_result "c$label promotion: edit argv carries --draft=false --prerelease=false --latest" "fail" \
            "missing:$missing argv=[$edit_line]"
    fi

    dl_line="$(gh_lines | grep -E '^gh release download' | head -1)"
    verify_line="$(doli_lines | grep -E 'release verify' | head -1)"
    dl_dir="$(flag_value "$dl_line" --dir)"
    v_dir="$(flag_value "$verify_line" --dir)"

    if [ -n "$verify_line" ] && grep -q -- "--version $TAG" <<<"$verify_line"; then
        test_result "c$label verify: argv carries --version $TAG" "pass" ""
    else
        test_result "c$label verify: argv carries --version $TAG" "fail" "argv=[$verify_line]"
    fi

    if [ -n "$dl_dir" ] && [ "$v_dir" = "$dl_dir" ]; then
        test_result "c$label verify: --dir is the directory gh downloaded into" "pass" ""
    else
        test_result "c$label verify: --dir is the directory gh downloaded into" "fail" \
            "download_dir=[$dl_dir] verify_dir=[$v_dir]"
    fi

    if grep -q -- '--trust-root bootstrap' <<<"$verify_line"; then
        test_result "c$label verify: argv carries --trust-root bootstrap" "pass" ""
    else
        test_result "c$label verify: argv carries --trust-root bootstrap" "fail" "argv=[$verify_line]"
    fi
}

# c1 — notes carry the CI banner, so the --notes-file branch runs.
new_sandbox "c1_publish_banner"
write_manifest 3
GH_NOTES="$BANNER_BEGIN
This build is unsigned and must not be installed.
$BANNER_END
## Changelog
- something"
run_script "$PUBLISH_SCRIPT" "$VERSION_BARE"

if [ "$RC" -eq 0 ]; then
    test_result "c1 banner branch: exit 0 (a verified release promotes)" "pass" ""
else
    test_result "c1 banner branch: exit 0 (a verified release promotes)" "fail" "$(detail)"
fi

if gh_lines | grep -E '^gh release edit' | grep -q -- '--notes-file'; then
    test_result "c1 banner branch: the --notes-file branch was the one exercised" "pass" ""
else
    test_result "c1 banner branch: the --notes-file branch was the one exercised" "fail" "$(detail)"
fi
assert_publish_promotion 1

# c2 — empty notes, so the plain branch runs. Same flags required.
new_sandbox "c2_publish_plain"
write_manifest 3
GH_NOTES=""
run_script "$PUBLISH_SCRIPT" "$VERSION_BARE"

if gh_lines | grep -E '^gh release edit' | grep -qv -- '--notes-file'; then
    test_result "c2 plain branch: the no-notes branch was the one exercised" "pass" ""
else
    test_result "c2 plain branch: the no-notes branch was the one exercised" "fail" "$(detail)"
fi
assert_publish_promotion 2

# ============================================================
# CASE d — REQ-214-003 refusal criteria (Must) — Decision: whether the INC-I-202 guard
# survives the M2 edits. This case is GREEN today and must stay green: adding
# --prerelease=false is one flag on the line that a sub-threshold manifest must never
# reach. A refactor that moves the edit above the count check would publish an unsigned
# release to every installer, silently.
# ============================================================
case_begin d "publish-release.sh still refuses a sub-threshold or absent manifest"

new_sandbox "d1_sub_threshold"
write_manifest 2
run_script "$PUBLISH_SCRIPT" "$VERSION_BARE"

if [ "$RC" -ne 0 ]; then
    test_result "d1 sub-threshold: exit non-zero" "pass" ""
else
    test_result "d1 sub-threshold: exit non-zero" "fail" "$(detail)"
fi

if grep -q 'REFUSING' "$OUT_FILE" 2>/dev/null &&
   grep -qE "2/${THRESHOLD_EXPECTED}|2 of ${THRESHOLD_EXPECTED}" "$OUT_FILE" 2>/dev/null; then
    test_result "d1 sub-threshold: REFUSING names the count 2 and the threshold 3" "pass" ""
else
    test_result "d1 sub-threshold: REFUSING names the count 2 and the threshold 3" "fail" \
        "out=[$(out_text | tail -2 | tr '\n' '|')]"
fi

if ! mutating; then
    test_result "d1 sub-threshold: no mutating gh release call recorded" "pass" ""
else
    test_result "d1 sub-threshold: no mutating gh release call recorded" "fail" \
        "$(gh_lines | tr '\n' ' ')"
fi

new_sandbox "d2_manifest_absent"
rm -f "$ASSETS/SIGNATURES.json"
run_script "$PUBLISH_SCRIPT" "$VERSION_BARE"

if [ "$RC" -ne 0 ] && grep -q 'REFUSING' "$OUT_FILE" 2>/dev/null; then
    test_result "d2 manifest absent: exit non-zero and REFUSING" "pass" ""
else
    test_result "d2 manifest absent: exit non-zero and REFUSING" "fail" "$(detail)"
fi

if ! mutating; then
    test_result "d2 manifest absent: no mutating gh release call recorded" "pass" ""
else
    test_result "d2 manifest absent: no mutating gh release call recorded" "fail" \
        "$(gh_lines | tr '\n' ' ')"
fi

# ============================================================
# CASE e — REQ-214-006 (Should) — Decision: whether the monitor's verdict means anything.
# It calls `release verify --version` with no --dir, so the CLI re-fetches from the public
# API and resolves whatever local trust root this host holds. On the dev Mac that root is
# a stale pre-rotation snapshot, so the monitor reports UNHEALTHY on a correctly signed
# release. GS-015 dispatches this script — a monitor with a standing false UNHEALTHY earns
# a waiver, and then nothing watches the release at all.
# ============================================================
case_begin e "monitor-release-signed.sh verifies with the same inputs as publish-release.sh"
new_sandbox "e_monitor_published"
write_manifest 3
GH_IS_DRAFT=false
run_script "$MONITOR_SCRIPT"

E_VERIFY_LINE="$(doli_lines | grep -E 'release verify' | head -1)"
E_DL_LINE="$(gh_lines | grep -E '^gh release download' | head -1)"
E_DL_DIR="$(flag_value "$E_DL_LINE" --dir)"
E_V_DIR="$(flag_value "$E_VERIFY_LINE" --dir)"
E_DL_NO="$(line_of '^gh release download')"
E_V_NO="$(line_of '^doli .*release verify')"

if [ -n "$E_VERIFY_LINE" ] && grep -q -- "--version $TAG" <<<"$E_VERIFY_LINE"; then
    test_result "e1 verify argv carries --version $TAG" "pass" ""
else
    test_result "e1 verify argv carries --version $TAG" "fail" "argv=[$E_VERIFY_LINE] $(detail)"
fi

if [ -n "$E_DL_LINE" ] &&
   grep -q 'SIGNATURES.json' <<<"$E_DL_LINE" &&
   grep -q 'CHECKSUMS.txt' <<<"$E_DL_LINE"; then
    test_result "e2 downloads SIGNATURES.json and CHECKSUMS.txt" "pass" ""
else
    test_result "e2 downloads SIGNATURES.json and CHECKSUMS.txt" "fail" \
        "argv=[${E_DL_LINE:-none}] $(detail)"
fi

if [ -n "$E_DL_NO" ] && [ -n "$E_V_NO" ] && [ "$E_DL_NO" -lt "$E_V_NO" ]; then
    test_result "e3 the download is recorded BEFORE the verify" "pass" ""
else
    test_result "e3 the download is recorded BEFORE the verify" "fail" \
        "download_line=${E_DL_NO:-none} verify_line=${E_V_NO:-none}"
fi

if [ -n "$E_DL_DIR" ] && [ "$E_V_DIR" = "$E_DL_DIR" ]; then
    test_result "e4 verify --dir is the directory those assets landed in" "pass" ""
else
    test_result "e4 verify --dir is the directory those assets landed in" "fail" \
        "download_dir=[$E_DL_DIR] verify_dir=[$E_V_DIR]"
fi

if grep -q -- '--trust-root bootstrap' <<<"$E_VERIFY_LINE"; then
    test_result "e5 verify argv carries --trust-root bootstrap (no stale-root false UNHEALTHY)" "pass" ""
else
    test_result "e5 verify argv carries --trust-root bootstrap (no stale-root false UNHEALTHY)" "fail" \
        "argv=[$E_VERIFY_LINE]"
fi

if ! mutating; then
    test_result "e6 read-only: no gh release edit/upload/delete/create recorded" "pass" ""
else
    test_result "e6 read-only: no gh release edit/upload/delete/create recorded" "fail" \
        "$(gh_lines | tr '\n' ' ')"
fi

# ============================================================
print_header "TEST SUMMARY"
CASE_LINE=""
for c in a b c d e; do
    eval "v=\$CASE_STATUS_$c"
    CASE_LINE="$CASE_LINE$c=$v "
done
echo -e "  Cases:        $CASE_LINE"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo -e "  Total Tests:  $TESTS_TOTAL"
echo

if [ "$TESTS_FAILED" -eq 0 ]; then
    exit 0
fi
exit 1
