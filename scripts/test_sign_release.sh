#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/sign-release.sh (INC-I-202 M1)
#   O1 manifest entry count  — jq '.signatures | length' of the uploaded SIGNATURES.json  — expected 3
#   O2 exit code             — exit status of sign-release.sh                              — expected 0
#   O3 default key path      — path the script resolves with KEY_DIR unset                 — expected $HOME/.ssh/doli/maintainer-{1,2,3}.json
#   PATHS: sign-release.sh:43-44 (key defaults) -> :47-53 (pre-flight) -> :86 (stdout capture) -> :110 (jq -s assembly) -> :127 (count) -> :144 (upload)
# INPUT PARTITIONS:
#   P1: stdout=preamble+JSON, KEY_DIR override with valid keys — O1=3, distinct pubkeys, O2=0
#   P2: KEY_DIR unset, rotated maintainer-N.json present under $HOME/.ssh/doli — O3 resolves there, O2=0
#   P3: KEY_DIR unset, no keys anywhere — O2=1, error names $HOME/.ssh/doli/maintainer-1.json
#   P4: stdout=status-line-only (no JSON object), KEY_DIR override with valid keys — O2!=0, error names the offending key file
# MATRIX: 3 outputs x 4 partitions (only cells the path reaches are asserted)
#   P1: O1 O2
#   P2: O2 O3
#   P3: O2 O3
#   P4: O2
#
# TDD RED tests for scripts/sign-release.sh. doli/gh are fully stubbed:
# no network, no real signing, no private key material.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
SIGN_SCRIPT="$PROJECT_ROOT/scripts/sign-release.sh"
TEST_DIR="/tmp/doli-sign-release-test-$$"
VERSION="6.26.2"

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

# --- stub writers ---

write_doli_stub() {
    local bin_dir="$1"
    cat > "$bin_dir/doli" <<'DOLI_STUB'
#!/usr/bin/env bash
mode="${DOLI_STUB_MODE:-normal}"
key_path=""
version="v0.0.0"
prev=""
for arg in "$@"; do
    if [[ "$prev" == "--key" ]]; then
        key_path="$arg"
    elif [[ "$prev" == "--version" ]]; then
        version="$arg"
    fi
    prev="$arg"
done
echo "Fetching CHECKSUMS.txt for ${version}..."
if [[ "$mode" == "garbage" ]]; then
    exit 0
fi
keyname="$(basename "$key_path" .json)"
cat <<JSON
{
  "public_key": "pub_${keyname}_deadbeefdeadbeef",
  "signature": "sig_${keyname}_cafebabecafebabe"
}
JSON
exit 0
DOLI_STUB
    chmod +x "$bin_dir/doli"
}

write_gh_stub() {
    local bin_dir="$1"
    cat > "$bin_dir/gh" <<'GH_STUB'
#!/usr/bin/env bash
sub="$1 $2"
case "$sub" in
    "release view")
        if [[ "$*" == *"assets"* ]]; then
            echo "CHECKSUMS.txt"
        elif [[ "$*" == *"tagName"* ]]; then
            echo "$3"
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
        echo "dummy-checksums-content" > "$dir/CHECKSUMS.txt"
        exit 0
        ;;
    "release upload")
        manifest="$4"
        mkdir -p "${GH_CAPTURE_DIR:?GH_CAPTURE_DIR not set}"
        cp "$manifest" "$GH_CAPTURE_DIR/SIGNATURES.json"
        exit 0
        ;;
    "release delete-asset")
        exit 0
        ;;
    *)
        exit 0
        ;;
esac
GH_STUB
    chmod +x "$bin_dir/gh"
}

# --- sandbox builder: sets CASE_DIR/WORK_DIR/HOME_DIR/BIN_DIR/CAPTURE_DIR/LOG_FILE ---
new_sandbox() {
    local name="$1"
    CASE_DIR="$TEST_DIR/$name"
    WORK_DIR="$CASE_DIR/work"
    HOME_DIR="$CASE_DIR/home"
    BIN_DIR="$CASE_DIR/bin"
    CAPTURE_DIR="$CASE_DIR/capture"
    LOG_FILE="$CASE_DIR/run.log"
    rm -rf "$CASE_DIR"
    mkdir -p "$WORK_DIR" "$HOME_DIR" "$BIN_DIR" "$CAPTURE_DIR"
    write_doli_stub "$BIN_DIR"
    write_gh_stub "$BIN_DIR"
}

# Runs sign-release.sh under env -i in $WORK_DIR; sets RC and writes LOG_FILE.
# $1 = KEY_DIR override (empty string = unset), $2 = doli stub mode.
run_sign_release() {
    local key_dir="$1"
    local doli_mode="$2"
    local env_args=(
        "HOME=$HOME_DIR"
        "PATH=$BIN_DIR:/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin"
        "GH_CAPTURE_DIR=$CAPTURE_DIR"
        "DOLI_STUB_MODE=$doli_mode"
    )
    if [[ -n "$key_dir" ]]; then
        env_args+=("KEY_DIR=$key_dir")
    fi
    ( cd "$WORK_DIR" && env -i "${env_args[@]}" bash "$SIGN_SCRIPT" "$VERSION" ) > "$LOG_FILE" 2>&1
    RC=$?
}

write_override_keys() {
    local dir="$1"
    mkdir -p "$dir"
    for i in 1 2 3; do
        echo '{"stub":"maintainer-'"$i"'"}' > "$dir/maintainer-${i}.json"
    done
}

write_rotated_keys() {
    local dir="$HOME_DIR/.ssh/doli"
    mkdir -p "$dir"
    for i in 1 2 3; do
        echo '{"stub":"maintainer-'"$i"'"}' > "$dir/maintainer-${i}.json"
    done
}

print_header "sign-release.sh RED tests (INC-I-202 M1)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# ============================================================
# REQ-202-002 (Must) — Decision: proves the jq -s assembly step survives a
# CLI stdout preamble instead of dying with a bare parse error before any
# signature is collected. [P1]
# ============================================================
new_sandbox "case1_preamble"
write_override_keys "$CASE_DIR/keys"
run_sign_release "$CASE_DIR/keys" "normal"

if [[ "$RC" -eq 0 ]]; then
    test_result "manifest_has_three_entries_when_cli_prints_preamble: exit code 0" "pass"
else
    test_result "manifest_has_three_entries_when_cli_prints_preamble: exit code 0" "fail" "exit=$RC log=$LOG_FILE"
fi

MANIFEST="$CAPTURE_DIR/SIGNATURES.json"
if [[ -f "$MANIFEST" ]]; then
    test_result "manifest_has_three_entries_when_cli_prints_preamble: manifest uploaded" "pass"
    SIG_COUNT=$(jq '.signatures | length' "$MANIFEST" 2>/dev/null || echo "0")
    if [[ "$SIG_COUNT" == "3" ]]; then
        test_result "manifest_has_three_entries_when_cli_prints_preamble: 3 signature entries" "pass"
    else
        test_result "manifest_has_three_entries_when_cli_prints_preamble: 3 signature entries" "fail" "got $SIG_COUNT entries, log=$LOG_FILE"
    fi
    DISTINCT_KEYS=$(jq -r '[.signatures[].public_key] | unique | length' "$MANIFEST" 2>/dev/null || echo "0")
    if [[ "$DISTINCT_KEYS" == "3" ]]; then
        test_result "manifest_has_three_entries_when_cli_prints_preamble: 3 distinct public keys" "pass"
    else
        test_result "manifest_has_three_entries_when_cli_prints_preamble: 3 distinct public keys" "fail" "got $DISTINCT_KEYS distinct, log=$LOG_FILE"
    fi
else
    test_result "manifest_has_three_entries_when_cli_prints_preamble: manifest uploaded" "fail" "no manifest captured, log=$LOG_FILE"
fi

# ============================================================
# REQ-202-001 (Must) — Decision: proves the default KEY_DIR (with KEY_DIR
# unset) resolves to the live rotated wallets, not the dead pre-rotation
# producer_N.json names that no longer exist post-INC-I-175. [P2]
# ============================================================
new_sandbox "case2_default_rotated"
write_rotated_keys
run_sign_release "" "normal"

if [[ "$RC" -eq 0 ]]; then
    test_result "default_key_path_resolves_to_rotated_maintainer_wallets: exit code 0" "pass"
else
    test_result "default_key_path_resolves_to_rotated_maintainer_wallets: exit code 0" "fail" "exit=$RC log=$LOG_FILE"
fi

if grep -q "maintainer-1.json" "$LOG_FILE" && grep -q "maintainer-2.json" "$LOG_FILE" \
    && grep -q "maintainer-3.json" "$LOG_FILE"; then
    test_result "default_key_path_resolves_to_rotated_maintainer_wallets: log names maintainer-{1,2,3}.json" "pass"
else
    test_result "default_key_path_resolves_to_rotated_maintainer_wallets: log names maintainer-{1,2,3}.json" "fail" "log=$LOG_FILE"
fi

if grep -q "producer_1.json" "$LOG_FILE"; then
    test_result "default_key_path_resolves_to_rotated_maintainer_wallets: log does NOT mention producer_1.json" "fail" "log=$LOG_FILE"
else
    test_result "default_key_path_resolves_to_rotated_maintainer_wallets: log does NOT mention producer_1.json" "pass"
fi

# ============================================================
# REQ-202-001 (Must) — Decision: proves a maintainer with no local keys at
# all is pointed at the rotated location, not the dead ~/.doli/mainnet/keys
# path a post-rotation operator would waste time chasing. [P3]
# ============================================================
new_sandbox "case3_missing_keys"
run_sign_release "" "normal"

if [[ "$RC" -eq 1 ]]; then
    test_result "missing_default_keys_error_names_rotated_location: exit code 1" "pass"
else
    test_result "missing_default_keys_error_names_rotated_location: exit code 1" "fail" "exit=$RC log=$LOG_FILE"
fi

if grep -q ".ssh/doli" "$LOG_FILE" && grep -q "maintainer-1.json" "$LOG_FILE"; then
    test_result "missing_default_keys_error_names_rotated_location: message names .ssh/doli/maintainer-1.json" "pass"
else
    test_result "missing_default_keys_error_names_rotated_location: message names .ssh/doli/maintainer-1.json" "fail" "log=$LOG_FILE"
fi

# ============================================================
# REQ-202-003 (Should) — Decision: proves a non-JSON signer response fails
# with a message naming the specific offending key, not a bare jq parse
# error a maintainer cannot act on without re-running each key by hand. [P4]
# ============================================================
new_sandbox "case4_garbage_output"
write_override_keys "$CASE_DIR/keys"
run_sign_release "$CASE_DIR/keys" "garbage"

if [[ "$RC" -ne 0 ]]; then
    test_result "non_json_signer_output_fails_loudly_naming_the_key: exit code non-zero" "pass"
else
    test_result "non_json_signer_output_fails_loudly_naming_the_key: exit code non-zero" "fail" "exit=$RC log=$LOG_FILE"
fi

if grep -qiE "error.*maintainer-1\.json|maintainer-1\.json.*error" "$LOG_FILE"; then
    test_result "non_json_signer_output_fails_loudly_naming_the_key: error message names the offending key" "pass"
else
    test_result "non_json_signer_output_fails_loudly_naming_the_key: error message names the offending key" "fail" "log=$LOG_FILE"
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
