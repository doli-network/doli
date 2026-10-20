#!/usr/bin/env bash
# Fixtures, stubs and predicates for scripts/test_gauntlet_gs025.sh (sourced, never run).
# OUTPUT CONTRACT: N/A — fixture file (the contract lives in scripts/test_gauntlet_gs025.sh)
# INPUT PARTITIONS: N/A — fixture file
# shellcheck disable=SC2034  # fixture globals are read by test_gauntlet_gs025.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
LIB="$PROJECT_ROOT/scripts/gauntlet-gs025.sh"
GAUNTLET="$PROJECT_ROOT/scripts/gauntlet.sh"
SEED_SQL="$PROJECT_ROOT/scripts/gauntlet-seed.sql"
PUNCH="${GS025_PUNCH_BIN:-$PROJECT_ROOT/target/release/examples/integrity_hole_punch}"
PUNCH_BUILD="build: cargo build --release -p doli-node --example integrity_hole_punch"
_TROOT="${TMPDIR:-/tmp}"
TEST_DIR="$(mktemp -d "${_TROOT%/}/doli-gs025-test.XXXXXX")"
[ -n "$TEST_DIR" ] && [ -d "$TEST_DIR" ] || { echo "mktemp failed"; exit 2; }
ORIG_PATH="$PATH"
FIX="$TEST_DIR/fixtures"
STUB_DIR="$TEST_DIR/stubs"
MUTATE_LOG="$TEST_DIR/mutate.log"
GS025_FNS="gs025_scan_stats gs025_verdict_a gs025_verdict_b gs025_pick_checkpoint gs025_pick_hole gs025_metric"
TOKENS="gs025-a-three-scans gs025-a-no-missing gs025-a-no-repair gs025-b-repaired-once gs025-b-names-hole gs025-b-complete-after gs025-b-no-loop"
HOLD_PIDS=""

RED='\033[0;31m'; GREEN='\033[0;32m'; BLUE='\033[0;34m'; CYAN='\033[0;36m'; NC='\033[0m'
TESTS_PASSED=0; TESTS_FAILED=0; TESTS_TOTAL=0

print_header() {
    echo; echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}  $1${NC}"; echo -e "${BLUE}========================================${NC}"; echo
}

test_result() {
    local test_name=$1 result=$2 detail=$3
    TESTS_TOTAL=$((TESTS_TOTAL + 1))
    if [ "$result" = "pass" ]; then
        TESTS_PASSED=$((TESTS_PASSED + 1)); echo -e "  ${GREEN}[PASS]${NC} $test_name"
    else
        TESTS_FAILED=$((TESTS_FAILED + 1)); echo -e "  ${RED}[FAIL]${NC} $test_name"
        [ -n "$detail" ] && echo -e "         ${RED}$detail${NC}"
    fi
}

# shellcheck disable=SC2329  # invoked via the EXIT trap
cleanup() {
    local p
    for p in $HOLD_PIDS; do kill "$p" 2>/dev/null; done
    case "$TEST_DIR" in "${_TROOT%/}"/doli-gs025-test.?*) rm -rf "$TEST_DIR" ;; esac
}
trap cleanup EXIT

# --- stubs: any process the scenario must never start during a lib-only load ---
mkdir -p "$STUB_DIR" "$FIX"
: > "$MUTATE_LOG"
for n in curl pkill killall launchctl ssh nc lsof doli doli-node integrity_hole_punch taskpolicy; do
    printf '#!/usr/bin/env bash\necho "%s $*" >> "%s"\nexit 1\n' "$n" "$MUTATE_LOG" > "$STUB_DIR/$n"
done
chmod +x "$STUB_DIR"/*

new_case() {
    CASE_DIR="$TEST_DIR/cases/$1"; CWD_DIR="$CASE_DIR/cwd"
    mkdir -p "$CWD_DIR" "$CASE_DIR/home" "$CASE_DIR/tmp"
}

_env() {
    export HOME="$CASE_DIR/home" TMPDIR="$CASE_DIR/tmp" PATH="$STUB_DIR:$ORIG_PATH"
    export GS025_BINARY="$STUB_DIR/doli-node" GS025_PUNCH_BIN="$STUB_DIR/integrity_hole_punch"
    export GS025_TESTNET_DIR="$CASE_DIR/home/testnet" GS025_SEED_RPC="http://127.0.0.1:9"
}

bounded() {
    rm -f "$CASE_DIR/timedout"
    "$@" &
    local p=$! n=0
    while kill -0 "$p" 2>/dev/null && [ "$n" -lt 300 ]; do sleep 0.1; n=$((n + 1)); done
    if kill -0 "$p" 2>/dev/null; then kill -TERM "$p" 2>/dev/null; : > "$CASE_DIR/timedout"; fi
    wait "$p" 2>/dev/null
}

_call_body() {
    local fn="$1"; shift
    cd "$CWD_DIR" || exit 1
    _env
    local defined=no rc=99
    # shellcheck disable=SC1090  # path is a test parameter
    if [[ -f "$LIB" ]]; then export GS025_LIB_ONLY=1; . "$LIB" >/dev/null 2>&1 </dev/null; fi
    declare -F "$fn" >/dev/null 2>&1 && defined=yes
    if [[ "$defined" == yes ]]; then
        rc=0
        ( "$fn" "$@" ) </dev/null > "$CASE_DIR/out" 2> "$CASE_DIR/err" || rc=$?
    fi
    printf '%s' "$defined" > "$CASE_DIR/defined"; printf '%s' "$rc" > "$CASE_DIR/rc"
}

_collect() {
    DEFINED="$(cat "$CASE_DIR/defined" 2>/dev/null || echo no)"
    RC="$(cat "$CASE_DIR/rc" 2>/dev/null || echo 99)"
    OUT="$(cat "$CASE_DIR/out" 2>/dev/null)"; ERR="$(cat "$CASE_DIR/err" 2>/dev/null)"
    rm -f "$CASE_DIR/defined" "$CASE_DIR/rc" "$CASE_DIR/out" "$CASE_DIR/err"
}

# call NAME FN ARGS... — fresh case dir, lib sourced lib-only, FN run bounded.
call() { new_case "$1"; shift; bounded _call_body "$@"; _collect; }

_load_body() {
    cd "$CWD_DIR" || exit 1
    _env
    export GS025_LIB_ONLY=1
    local n=0 f
    # shellcheck disable=SC1090  # path is a test parameter
    if [[ -f "$LIB" ]]; then . "$LIB" > "$CASE_DIR/out" 2> "$CASE_DIR/err" < /dev/null; fi
    for f in $GS025_FNS; do declare -F "$f" >/dev/null 2>&1 && n=$((n + 1)); done
    printf '%s' "$n" > "$CASE_DIR/nfn"
}

# --- predicates ---

lib_ok() { [[ -f "$LIB" && "$DEFINED" == "yes" ]]; }
det() {
    echo "rc=$RC defined=$DEFINED lib=$LIB out=[$(printf '%s' "$OUT" | head -5 | tr '\n' '|')] err=[$(printf '%s' "$ERR" | head -3 | tr '\n' '|')]"
}
ck() {
    local name="$1" cond="$2" d="${3:-}"
    if lib_ok && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "${d:-$(det)}"; fi
}
sck() {
    local name="$1" cond="$2" d="${3:-}"
    if [[ -f "$LIB" ]] && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "$( [[ -f "$LIB" ]] && echo "${d:-condition false}" || echo "missing: $LIB" )"; fi
}
tck() {
    local name="$1" cond="$2" d="${3:-}"
    if eval "$cond"; then test_result "$name" "pass" ""; else test_result "$name" "fail" "${d:-condition false}"; fi
}
not_timed_out() { [[ ! -f "$CASE_DIR/timedout" ]]; }

# stats_is "k=v ..." — OUT is ONE line holding exactly these pairs (any order).
stats_is() {
    [[ "$(printf '%s\n' "$OUT" | grep -c .)" -eq 1 ]] || return 1
    [[ "$(printf '%s\n' "$OUT" | tr ' ' '\n' | grep . | sort)" == "$(printf '%s\n' "$1" | tr ' ' '\n' | grep . | sort)" ]]
}
# vline ID — the PASS|FAIL line for ID (exactly one), else empty.
vline() {
    local l; l="$(printf '%s\n' "$OUT" | grep -E "^(PASS|FAIL) $1( |\$)")"
    [[ "$(printf '%s\n' "$l" | grep -c .)" -eq 1 ]] && printf '%s' "$l"
}
is_pass() { [[ "$(vline "$1")" == PASS* ]]; }
is_fail() { [[ "$(vline "$1")" == FAIL* ]]; }

# --- static helpers on scripts/gauntlet-gs025.sh ---

code_lines() { grep -v '^[[:space:]]*#' "$LIB" 2>/dev/null; }

bad_kills() {
    local re='^kill([[:space:]]+-[A-Za-z0-9]+|[[:space:]]+-s[[:space:]]+[A-Za-z0-9]+)*[[:space:]]+"?\$\{?PID\}?"?$' k
    code_lines | sed -E 's/[0-9]*>&?[^[:space:];&|)]*//g' \
        | grep -oE "(^|[^A-Za-z0-9_./-])kill([[:space:]][^;&|)<>'\`]*|$)" \
        | sed -E 's/^[^k]//; s/[[:space:]]+$//' \
        | while IFS= read -r k; do [[ "$k" =~ $re ]] || echo "$k"; done
}

# Recursive rm allowed only as a case arm `<pattern naming gs025.>) rm -r… "$VAR"` (subject == VAR when on the line).
bad_rms() {
    local fre='^(-[A-Za-z-]*)[[:space:]]+(.*)$' arm='([^()]*gs025\.[^()]*)\)[[:space:]]*rm[[:space:]]+(-[A-Za-z-]+[[:space:]]+)+"\$\{?([A-Za-z_][A-Za-z0-9_]*)\}?"[[:space:]]*(;;|$)'
    local subj='case[[:space:]]+"\$\{?([A-Za-z_][A-Za-z0-9_]*)\}?"[[:space:]]+in' line rest flags var
    code_lines | grep -E '(^|[^A-Za-z0-9_./-])rm[[:space:]]+-' | while IFS= read -r line; do
        rest="$(printf '%s' "$line" | sed -E 's/^(.*[^A-Za-z0-9_./-])?rm[[:space:]]+//')"; flags=""
        while [[ "$rest" =~ $fre ]]; do flags="$flags${BASH_REMATCH[1]}"; rest="${BASH_REMATCH[2]}"; done
        [[ "$flags" == *[rR]* || "$flags" == *recursive* ]] || continue
        if [[ "$line" =~ $arm ]]; then
            var="${BASH_REMATCH[3]}"
            [[ "${BASH_REMATCH[1]}" =~ testnet|HOME ]] && { echo "$line"; continue; }
            if [[ "$line" =~ $subj ]] && [[ "${BASH_REMATCH[1]}" != "$var" ]]; then echo "$line"; fi
        else
            echo "$line"
        fi
    done
}

TN='testnet|TESTNET'
tn_norm() { code_lines | sed -E 's/--network[[:space:]=]+testnet//g; s/GS025_PROTECTED_ROOTS="[^"]*"//g'; }
testnet_writes() {
    tn_norm | grep -E ">>?[[:space:]]*\"?[^[:space:]\";|&>]*($TN)"
    tn_norm | grep -E "$TN" | grep -E "(^|[;&|(){]|then|do|else)[[:space:]]*(rm|mv|touch|mkdir|ln|chmod|chown|tee|truncate|sqlite3)[[:space:]]"
    tn_norm | grep -E "$TN" | awk '{ gsub(/\|\||&&|;|\|/, "\n"); print }' \
        | grep -E '(^|[^A-Za-z0-9_./-])(cp|ditto|rsync|clone)[[:space:]]' \
        | sed -E 's/[0-9]*>&?[^[:space:]]*//g; s/[[:space:]{}()]+$//' | awk '$NF ~ /testnet|TESTNET/'
    tn_norm | grep -E -- "--(data-dir|archive-to)[[:space:]=]+\"?[^[:space:]\"]*($TN|CHECKPOINT|ARCHIVE_SRC)"
    tn_norm | grep -E 'integrity_hole_punch|PUNCH' | grep -E "$TN|CHECKPOINT|ARCHIVE_SRC"
}

# ga_assert TOKEN — gauntlet.sh's real assert() on TOKEN: "rc|SKIP_REASONS|FAIL_REASONS".
ga_assert() {
    ( SKIP_REASONS=""; FAIL_REASONS=""
      eval "$(awk '/^assert\(\)\{/{f=1} f{print} f&&/^}/{exit}' "$GAUNTLET")" 2>/dev/null || exit 99
      assert "$1"; rc=$?
      printf '%s|%s|%s\n' "$rc" "$SKIP_REASONS" "$FAIL_REASONS" )
}
seed_tokens() {
    awk "/VALUES \\('GS-025'/{f=1} f{print} f&&/^ON CONFLICT/{exit}" "$SEED_SQL" 2>/dev/null \
        | grep -oE 'gs025-[a-z0-9-]+' | sort -u
}

# --- hole-punch helper ---

pck() {
    local name="$1" cond="$2" d="${3:-}"
    if [[ ! -x "$PUNCH" ]]; then test_result "$name" "fail" "helper $PUNCH missing — $PUNCH_BUILD"
    elif eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "${d:-rc=$PRC out=[$(printf '%s' "$POUT" | tr '\n' '|')]}"; fi
}
# punch HOME_DIR ROOTS DIR HEIGHT — run the helper bounded (20 s); sets PRC, POUT. Never under the real ~/testnet.
punch() {
    PRC=99; POUT=""
    [[ -x "$PUNCH" ]] || return 0
    case "$3" in "$HOME"/testnet*) echo "test bug: real testnet path $3"; exit 2 ;; esac
    local o="$TEST_DIR/punch.out" n=0 p
    if [[ -n "$2" ]]; then env HOME="$1" GS025_PROTECTED_ROOTS="$2" "$PUNCH" "$3" "$4" > "$o" 2>&1 &
    else env -u GS025_PROTECTED_ROOTS HOME="$1" "$PUNCH" "$3" "$4" > "$o" 2>&1 & fi
    p=$!
    while kill -0 "$p" 2>/dev/null && [ "$n" -lt 200 ]; do sleep 0.1; n=$((n + 1)); done
    kill -0 "$p" 2>/dev/null && kill -TERM "$p" 2>/dev/null
    wait "$p" 2>/dev/null; PRC=$?; POUT="$(cat "$o" 2>/dev/null)"
}
refused() { [[ "$PRC" -eq 1 ]] && printf '%s\n' "$POUT" | grep -qE '^REFUSE '; }
# snap DIR — listing + checksums; "ABSENT" when DIR does not exist.
snap() { [[ -e "$1" ]] || { echo ABSENT; return; }; ( cd "$1" && find . | sort && find . -type f -exec cksum {} \; | sort ); }
# fake_db DIR — looks like a RocksDB dir from outside (CURRENT names a manifest).
fake_db() { mkdir -p "$1"; printf 'MANIFEST-000005\n' > "$1/CURRENT"; : > "$1/LOCK"; }
db_created() { find "$1" \( -name 'MANIFEST-*' -o -name 'OPTIONS-*' -o -name '*.sst' -o -name '[0-9]*.log' \) | grep -q .; }

# --- canned logs (real tracing shape: ANSI-coloured timestamp, level, target) ---

E=$'\033'
_ln() { printf '%s[2m2000-01-01T00:%02d:14.712253Z%s[0m %s[%sm %s%s[0m %s[2m%s%s[0m%s[2m:%s[0m %s\n' \
    "$E" "$1" "$E" "$E" "$2" "$3" "$E" "$E" "$4" "$E" "$E" "$E" "$5"; }
info() { _ln "$1" 32 INFO doli_node::node::periodic "$2"; }
warn() { _ln "$1" 33 WARN doli_node::node::periodic "$2"; }
arch() { _ln "$1" 32 INFO storage::archiver "$2"; }

phantom_scan() {
    info "$2" "[INTEGRITY] 1 missing blocks in 1..=$1 — auto-repairing from archive \"/tmp/x/archive\""
    arch "$2" "[ARCHIVER] Backfilling from archive: $(($1 - 2)) blocks available"
    arch "$2" "[ARCHIVER] Backfill complete: 0 blocks imported, $(($1 - 2)) already present, 0 files missing"
    info "$2" "[INTEGRITY] Auto-repair imported 0 blocks from archive"
    warn "$2" "[INTEGRITY] After auto-repair: still 1 missing blocks in 1..=$1 (archive may be incomplete)"
    warn "$2" "[INTEGRITY] Periodic scan: 1 missing blocks in 1..=$1"
    arch "$2" "Archived block $(($1 + 1))"
}
complete_scan() { info "$2" "[INTEGRITY] Periodic scan: chain complete (1..=$1), commitment=${3:-0123456789abcdef}"; }
hole_found() { info 40 "[INTEGRITY] 1 missing blocks in 1..=${2:-281500}: [$1] — auto-repairing from archive \"/tmp/gs025.x/archive\""; }
hole_repair() {
    hole_found 140000
    arch 40 "[ARCHIVER] Targeted backfill: 1 heights, 1 imported, 0 present, 0 files missing, 0 rejected"
    info 40 "[INTEGRITY] Auto-repair imported 1 blocks from archive"
    info 40 "[INTEGRITY] Auto-repair successful: 1 blocks restored, chain complete (1..=281500), commitment=0123456789abcdef"
    arch 41 "Archived block 281501"
}
backoff_line() { info 45 "[INTEGRITY] 1 missing blocks in 1..=$1: [140000] — repair backed off: same missing set as the last failed repair, retry in 9 scans"; }

write_fixtures() {
    { phantom_scan 281500 30; phantom_scan 281600 34; phantom_scan 281700 38; } > "$FIX/phantom.log"
    { arch 29 "[ARCHIVER] Catch-up: scanning for gaps"; complete_scan 281500 30; arch 31 "Archived block 281601"
      complete_scan 281600 34; arch 35 "Archived block 281701"; complete_scan 281700 38; } > "$FIX/fixed.log"
    { complete_scan 281500 30; complete_scan 281600 34; } > "$FIX/two_scans.log"
    { cat "$FIX/fixed.log"; warn 39 "[INTEGRITY] Periodic scan: 1 missing blocks in 1..=281800: [281799]"; } > "$FIX/fixed_missing.log"
    { cat "$FIX/fixed.log"; hole_found 281750 281800
      arch 39 "[ARCHIVER] Targeted backfill: 1 heights, 1 imported, 0 present, 0 files missing, 0 rejected"; } > "$FIX/fixed_repair.log"
    { hole_repair; complete_scan 281600 42 fedcba9876543210; } > "$FIX/hole_ok.log"
    { complete_scan 281400 39; hole_repair; } > "$FIX/hole_before.log"
    { hole_repair; arch 41 "[ARCHIVER] Targeted backfill: 1 heights, 0 imported, 1 present, 0 files missing, 0 rejected"
      complete_scan 281600 42; } > "$FIX/hole_twice.log"
    { hole_repair; warn 42 "[INTEGRITY] Periodic scan: 1 missing blocks in 1..=281600: [140000]"
      complete_scan 281700 44; } > "$FIX/hole_loop.log"
    { hole_repair; backoff_line 281600; complete_scan 281700 46; } > "$FIX/hole_backoff.log"
    { warn 39 "[INTEGRITY] Periodic scan: 1 missing blocks in 1..=281450: [140000]"; hole_repair
      complete_scan 281600 42; } > "$FIX/hole_pre_missing.log"
    { info 40 "[INTEGRITY] 3 missing blocks in 1..=281500: [139998, 140000, 140002] — auto-repairing from archive \"/tmp/gs025.x/archive\""
      arch 40 "[ARCHIVER] Targeted backfill: 3 heights, 3 imported, 0 present, 0 files missing, 0 rejected"
      complete_scan 281600 42; } > "$FIX/hole_list.log"
    { arch 20 "[ARCHIVER] Catch-up: scanning for gaps"
      info 20 "[BRIDGE] Backfill complete: 5 blocks imported from archive"
      info 21 "[INTEGRITY] Scan at 300 skipped: previous scan still running"
      info 22 "[INTEGRITY] 2 missing blocks in 1..=300: [7, 9] — auto-repairing from archive \"/a\""
      arch 22 "[ARCHIVER] Backfilling from archive: 298 blocks available"
      arch 22 "[ARCHIVER] Backfill complete: 2 blocks imported, 296 already present, 0 files missing"
      info 22 "[INTEGRITY] Auto-repair imported 2 blocks from archive"
      info 22 "[INTEGRITY] Auto-repair successful: 2 blocks restored, chain complete (1..=300), commitment=0123456789abcdef"
      info 23 "[INTEGRITY] 3 missing blocks in 1..=400: [310, 320, 330] — auto-repairing from archive \"/a\""
      arch 23 "[ARCHIVER] Targeted backfill: 3 heights, 1 imported, 0 present, 2 files missing, 0 rejected"
      info 23 "[INTEGRITY] Auto-repair imported 1 blocks from archive"
      warn 23 "[INTEGRITY] After auto-repair: still 2 missing blocks in 1..=400: [320, 330] (archive may be incomplete)"
      warn 23 "[INTEGRITY] Periodic scan: 2 missing blocks in 1..=400: [320, 330]"
      backoff_line 500
      warn 24 "[INTEGRITY] Periodic scan: 2 missing blocks in 1..=500: [320, 330]"
      complete_scan 600 25; arch 25 "Archived block 601"; } > "$FIX/mixed.log"
}

# mk_ckpts DIR NAME... — checkpoint-like subdirs.
mk_ckpts() { local d="$1" n; shift; mkdir -p "$d"; for n in "$@"; do mkdir -p "$d/$n/blocks"; done; }
# mk_arch DIR block|blake3|both H... — 10-digit zero-padded archive files.
mk_arch() {
    local d="$1" kind="$2" h f; shift 2; mkdir -p "$d"
    for h in "$@"; do
        f="$(printf '%010d' "$h")"
        [[ "$kind" == blake3 ]] || : > "$d/$f.block"
        [[ "$kind" == block ]] || : > "$d/$f.blake3"
    done
}
