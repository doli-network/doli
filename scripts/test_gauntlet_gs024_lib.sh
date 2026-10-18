#!/usr/bin/env bash
# Fixtures, stubs and predicates for scripts/test_gauntlet_gs024.sh (sourced, never run).
# OUTPUT CONTRACT: N/A — fixture file (the contract lives in scripts/test_gauntlet_gs024.sh)
# INPUT PARTITIONS: N/A — fixture file
# shellcheck disable=SC2034  # fixture globals are read by test_gauntlet_gs024.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
LIB="$PROJECT_ROOT/scripts/gauntlet-gs024.sh"
HLIB="$PROJECT_ROOT/scripts/gauntlet-gs024-lib.sh"
SCAN="$PROJECT_ROOT/scripts/gauntlet-gs024-scan.py"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/doli-gs024-test.XXXXXX")"
[ -n "$TEST_DIR" ] && [ -d "$TEST_DIR" ] || { echo "mktemp failed"; exit 2; }
ORIG_PATH="$PATH"
SEED_PORT=18500
ALT_PORT=28500
TIP=50000
TS=1792323139
TOKENS="gs024-no-snap gs024-no-missing-height gs024-commitment-matches-seed gs024-reaches-tip gs024-archive-files-present gs024-clean-exit gs024-gap-held"

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

# shellcheck disable=SC2329  # invoked indirectly via trap below
cleanup() {
    local p
    for p in ${DUMMY_PID:-} ${SRV_PIDS:-}; do kill "$p" 2>/dev/null; done
    rm -rf "$TEST_DIR"
}
trap cleanup EXIT
/bin/sleep 900 &
DUMMY_PID=$!; disown "$DUMMY_PID" 2>/dev/null
SRV_PIDS=""

# --- stubs ---

write_stubs() {
    local b="$1" n
    cat > "$b/curl" <<'STUB'
#!/usr/bin/env bash
echo "curl $*" >> "${CURL_LOG:?}"
all="$*"; url=""
for a in "$@"; do
    case "$a" in http://*|https://*) url="$a" ;; @-) all="$all $(cat)" ;; esac
done
hp="${url#*://}"; hp="${hp%%/*}"; port="${hp##*:}"
F="${FIX_DIR:?}/$port"
{ [ -n "$url" ] && [ -d "$F" ] && [ ! -f "$F/down" ]; } || exit 7
k=$(( $(cat "$F/ncalls" 2>/dev/null || echo 0) + 1 )); echo "$k" > "$F/ncalls"
case " $(cat "$F/failat" 2>/dev/null) " in *" $k "*) exit 28 ;; esac
n="$(cat "$F/failn" 2>/dev/null)"; [ "${n:-0}" -gt 0 ] && { echo "$((n - 1))" > "$F/failn"; exit 28; }
n="$(cat "$F/emptyn" 2>/dev/null)"; [ "${n:-0}" -gt 0 ] && { echo "$((n - 1))" > "$F/emptyn"; exit 0; }
method="$(printf '%s' "$all" | sed -n 's/.*"method"[[:space:]]*:[[:space:]]*"\([A-Za-z]*\)".*/\1/p')"
case "$method" in
    getChainInfo)
        t="$(cat "$F/tip")"
        printf '{"jsonrpc":"2.0","result":{"network":"testnet","version":"6.44.0","bestHash":"%064x","bestHeight":%s,"bestSlot":%s,"genesisHash":"%064x"},"id":1}\n' "$t" "$t" "$((t + 7))" 1 ;;
    getBlockByHeight)
        h="$(printf '%s' "$all" | sed -n 's/.*"params"[^0-9]*\([0-9][0-9]*\).*/\1/p')"
        if [ -n "$h" ] && [ -f "$F/hash.$h" ]; then
            printf '{"jsonrpc":"2.0","result":{"hash":"%s","prevHash":"%064x","height":%s,"slot":%s,"timestamp":1792323139,"producer":"%064x","txCount":1},"id":1}\n' "$(cat "$F/hash.$h")" 7 "$h" "$((h + 7))" 9
        else
            printf '{"jsonrpc":"2.0","error":{"code":-32000,"message":"Block not found"},"id":1}\n'
        fi ;;
    verifyChainIntegrity)
        printf '{"jsonrpc":"2.0","result":{"complete":true,"missingCount":0,"missing":[],"chainCommitment":"%064x"},"id":1}\n' 5 ;;
    *) printf '{"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":1}\n' ;;
esac
STUB
    cat > "$b/lsof" <<'STUB'
#!/usr/bin/env bash
echo "lsof $*" >> "${LSOF_LOG:?}"
port="$(printf '%s' "$*" | sed -n 's/.*:\([0-9][0-9]*\).*/\1/p')"
{ [ -n "$port" ] && [ -f "${FIX_DIR:?}/busy.$port" ]; } || exit 1
echo "COMMAND    PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME"
echo "doli-node 4242 user   12u  IPv4 0x1      0t0  TCP 127.0.0.1:$port (LISTEN)"
STUB
    cat > "$b/pgrep" <<'STUB'
#!/usr/bin/env bash
echo "pgrep $*" >> "${PGREP_LOG:?}"
name=""; for a in "$@"; do case "$a" in -*) ;; *) name="$a" ;; esac; done
{ [ -n "$name" ] && [ -f "${FIX_DIR:?}/proc.$name" ]; } || exit 1
echo 4242
STUB
    cat > "$b/ps" <<'STUB'
#!/usr/bin/env bash
case " $* " in *" -p "*) exec /bin/ps "$@" ;; esac
echo "ps $*" >> "${PGREP_LOG:?}"
cat "${FIX_DIR:?}/ps.txt" 2>/dev/null
exit 0
STUB
    cat > "$b/pystub" <<'STUB'
#!/usr/bin/env bash
echo "pyscan $*" >> "${CURL_LOG:?}"
n=$#; m=$((n - 1)); hi="${!n}"; lo="${!m}"
if [ -f "${FIX_DIR:?}/scan.out" ]; then cat "$FIX_DIR/scan.out"
else echo "scan $lo..$hi checked=$((hi - lo + 1)) missing=0 ranges=none mismatched=0 mismatch_ranges=none errors=0 seconds=0"; fi
exit "$(cat "$FIX_DIR/scan.rc" 2>/dev/null || echo 0)"
STUB
    cat > "$b/tpcatch" <<'STUB'
#!/usr/bin/env bash
echo "taskpolicy $*" >> "${CURL_LOG:?}"
echo "$(date +%s) $1" >> "${FIX_DIR:?}/tp.time"
[ "$1" = -B ] && cat "$FIX_DIR/catchup.tip" > "$FIX_DIR/8590/tip"
exit 0
STUB
    for n in pkill killall launchctl sqlite3 ssh nc doli taskpolicy python3; do
        printf '#!/usr/bin/env bash\necho "%s $*" >> "${MUTATE_LOG:?}"\nexit 0\n' "$n" > "$b/$n"
    done
    printf '#!/usr/bin/env bash\necho "taskpolicy $*" >> "${CURL_LOG:?}"\nexit 0\n' > "$b/tpstub"
    chmod +x "$b"/*
}

# --- fixtures ---

new_sandbox() {
    CASE_DIR="$TEST_DIR/$1"
    BIN_DIR="$CASE_DIR/stubs"; FIX_DIR="$CASE_DIR/fix"; HOME_DIR="$CASE_DIR/home"
    POOL_DIR="$CASE_DIR/pool"; CWD_DIR="$CASE_DIR/cwd"; NODE_BIN="$CASE_DIR/nodebin/doli-node"
    CURL_LOG="$CASE_DIR/curl.log"; LSOF_LOG="$CASE_DIR/lsof.log"; PGREP_LOG="$CASE_DIR/pgrep.log"
    MUTATE_LOG="$CASE_DIR/mutate.log"; BIN_LOG="$CASE_DIR/bin.log"
    T_MIN=""; T_MAX=""; T_ALLOW=""; T_P2P=""; T_RPC=""; T_MET=""; T_SEED_URL=""; STDIN_FILE=""
    T_THR=""; T_TRIES=""; T_RTO=""; T_GRACE=""; T_TIMEOUT=""; T_WIN=""; T_FULL=""; T_PY=""; T_TP=""
    mkdir -p "$BIN_DIR" "$FIX_DIR/$SEED_PORT" "$HOME_DIR/testnet" "$POOL_DIR" "$CWD_DIR" \
        "$CASE_DIR/nodebin" "$CASE_DIR/seedarch" "$CASE_DIR/seedckpt"
    : > "$CURL_LOG"; : > "$LSOF_LOG"; : > "$PGREP_LOG"; : > "$MUTATE_LOG"; : > "$BIN_LOG"
    echo "$TIP" > "$FIX_DIR/$SEED_PORT/tip"
    write_stubs "$BIN_DIR"
    printf '#!/usr/bin/env bash\necho "bin $*" >> "${BIN_LOG:?}"\nexit 0\n' > "$NODE_BIN"
    chmod +x "$NODE_BIN"
}

hx() { printf '%064x' "$1"; }
base_path() { echo "$POOL_DIR/ckpt-h$1-$TS"; }

mk_base() {
    local h="$1" hash="${2:-$(hx "$1")}" d
    d="$(base_path "$h")"
    mkdir -p "$d/blocks" "$d/state_db"; echo sst > "$d/state_db/000001.sst"
    printf '{\n  "hash": "%s",\n  "healthy": true,\n  "height": %s,\n  "isolated": false,\n  "peer_count": 6,\n  "timestamp": %s\n}\n' \
        "$hash" "$h" "$TS" > "$d/health.json"
}

mk_health_at() {
    mkdir -p "$1"
    printf '{\n  "hash": "%s",\n  "healthy": true,\n  "height": %s,\n  "timestamp": %s\n}\n' "$(hx "$2")" "$2" "$TS" > "$1/health.json"
}

seed_has() { mkdir -p "$FIX_DIR/${3:-$SEED_PORT}"; echo "${2:-$(hx "$1")}" > "$FIX_DIR/${3:-$SEED_PORT}/hash.$1"; }
canon() { local h; for h in "$@"; do mk_base "$h"; seed_has "$h"; done; }

# --- runners ---

_env() {
    export PATH="$BIN_DIR:$ORIG_PATH" HOME="$HOME_DIR"
    export FIX_DIR CURL_LOG LSOF_LOG PGREP_LOG MUTATE_LOG BIN_LOG
    unset GS024_MIN_GAP GS024_MAX_GAP GS024_P2P_PORT GS024_RPC_PORT GS024_METRICS_PORT GS024_ALLOW_BUILD GS024_LIB_ONLY \
        GS024_THROTTLE GS024_RPC_TRIES GS024_RPC_TIMEOUT GS024_STOP_GRACE GS024_TIMEOUT GS024_THROTTLE_WINDOW GS024_FULL_SCAN
    export GS024_TASKPOLICY="$BIN_DIR/${T_TP:-tpstub}" GS024_PYTHON="${T_PY:-$BIN_DIR/pystub}"
    [ -n "$T_THR" ] && export GS024_THROTTLE="$T_THR"
    [ -n "$T_TRIES" ] && export GS024_RPC_TRIES="$T_TRIES"
    [ -n "$T_RTO" ] && export GS024_RPC_TIMEOUT="$T_RTO"
    [ -n "$T_GRACE" ] && export GS024_STOP_GRACE="$T_GRACE"
    [ -n "$T_TIMEOUT" ] && export GS024_TIMEOUT="$T_TIMEOUT"
    [ -n "$T_WIN" ] && export GS024_THROTTLE_WINDOW="$T_WIN"
    [ -n "$T_FULL" ] && export GS024_FULL_SCAN="$T_FULL"
    export GS024_POOL="$POOL_DIR" GS024_SEED_RPC="${T_SEED_URL:-http://127.0.0.1:$SEED_PORT}"
    export GS024_SEED_ARCHIVE="$CASE_DIR/seedarch" GS024_SEED_CKPT_DIR="$CASE_DIR/seedckpt" GS024_BIN="$NODE_BIN"
    [ -n "$T_MIN" ] && export GS024_MIN_GAP="$T_MIN"
    [ -n "$T_MAX" ] && export GS024_MAX_GAP="$T_MAX"
    [ -n "$T_ALLOW" ] && export GS024_ALLOW_BUILD="$T_ALLOW"
    [ -n "$T_P2P" ] && export GS024_P2P_PORT="$T_P2P"
    [ -n "$T_RPC" ] && export GS024_RPC_PORT="$T_RPC"
    [ -n "$T_MET" ] && export GS024_METRICS_PORT="$T_MET"
    return 0
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
    local -a args=("$@")
    cd "$CWD_DIR" || exit 1
    _env
    local defined=no rc=99
    # shellcheck disable=SC1090  # path is a test parameter
    if [[ -f "$LIB" ]]; then export GS024_LIB_ONLY=1; . "$LIB" >/dev/null 2>&1 </dev/null; fi
    declare -F "$fn" >/dev/null 2>&1 && defined=yes
    if [[ "$defined" == yes ]]; then
        rc=0
        ( "$fn" ${args[@]+"${args[@]}"} ) < "${STDIN_FILE:-/dev/null}" > "$CASE_DIR/out" 2> "$CASE_DIR/err" || rc=$?
    fi
    printf '%s' "$defined" > "$CASE_DIR/defined"; printf '%s' "$rc" > "$CASE_DIR/rc"
}

_collect() {
    DEFINED="$(cat "$CASE_DIR/defined" 2>/dev/null || echo no)"
    RC="$(cat "$CASE_DIR/rc" 2>/dev/null || echo 99)"
    OUT="$(cat "$CASE_DIR/out" 2>/dev/null)"; ERR="$(cat "$CASE_DIR/err" 2>/dev/null)"
    rm -f "$CASE_DIR/defined" "$CASE_DIR/rc" "$CASE_DIR/out" "$CASE_DIR/err"
}

call_fn() { bounded _call_body "$@"; _collect; }

_main_body() {
    cd "$CWD_DIR" || exit 1
    _env
    local rc=0
    bash "$LIB" < /dev/null > "$CASE_DIR/out" 2> "$CASE_DIR/err" || rc=$?
    printf '%s' "$rc" > "$CASE_DIR/rc"
}

run_main() {
    bounded _main_body; _collect
    if [[ -f "$LIB" ]]; then DEFINED=yes; else DEFINED=no; fi
}

_load_body() {
    cd "$CWD_DIR" || exit 1
    _env
    export GS024_LIB_ONLY=1
    local n=0 f
    # shellcheck disable=SC1090  # path is a test parameter
    if [[ -f "$LIB" ]]; then . "$LIB" > "$CASE_DIR/out" 2> "$CASE_DIR/err" < /dev/null; fi
    for f in ${LOAD_FNS:-gs024_rpc gs024_pick_base gs024_preflight gs024_ranges gs024_prune_pool gs024_main}; do
        declare -F "$f" >/dev/null 2>&1 && n=$((n + 1))
    done
    printf '%s' "$n" > "$CASE_DIR/nfn"
}

# --- predicates ---

lib_ok() { [[ -f "$LIB" ]] && [[ "$DEFINED" == "yes" ]]; }
det() {
    echo "rc=$RC defined=$DEFINED lib=$LIB out=[$(printf '%s' "$OUT" | head -3 | tr '\n' '|')] err=[$(printf '%s' "$ERR" | head -3 | tr '\n' '|')]"
}
ck() {
    local name="$1" cond="$2" d="${3:-}"
    if lib_ok && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "${d:-$(det)}"; fi
}
nr_lines()   { printf '%s\n%s\n' "$OUT" "$ERR" | grep -E '^NOT READY: '; }
not_ready()  { nr_lines >/dev/null; }
reason_has() { nr_lines | grep -qi -- "$1"; }
picked()     { [[ "$RC" -eq 0 && "${OUT%/}" == "$(base_path "$1")" ]] && ! not_ready; }
launched()   { grep -qE '(^| )run( |$)|--p2p-port|--archive-to|--data-dir' "$BIN_LOG" 2>/dev/null; }
procs_touched() { grep -qE '^(pkill|killall|launchctl) ' "$MUTATE_LOG" 2>/dev/null; }
not_timed_out() { [[ ! -f "$CASE_DIR/timedout" ]]; }

# Runner + its sourced helper library, comment lines dropped.
code_lines() { cat "$LIB" "$HLIB" 2>/dev/null | grep -v '^[[:space:]]*#'; }

bad_kills() {
    local re='^kill([[:space:]]+-[A-Za-z0-9]+|[[:space:]]+-s[[:space:]]+[A-Za-z0-9]+)*[[:space:]]+"?\$\{?PID\}?"?$' k
    code_lines | sed -E 's/[0-9]*>&?[^[:space:];&|)]*//g' \
        | grep -oE "(^|[^A-Za-z0-9_./-])kill([[:space:]][^;&|)<>'\`]*|$)" \
        | sed -E 's/^[^k]//; s/[[:space:]]+$//' \
        | while IFS= read -r k; do [[ "$k" =~ $re ]] || echo "$k"; done
}

bad_rms() {
    local fre='^(-[A-Za-z-]*)[[:space:]]+(.*)$' line rest flags tgt
    code_lines | grep -E '(^|[^A-Za-z0-9_./-])rm[[:space:]]+-' | while IFS= read -r line; do
        rest="$(printf '%s' "$line" | sed -E 's/^(.*[^A-Za-z0-9_./-])?rm[[:space:]]+//')"
        flags=""
        while [[ "$rest" =~ $fre ]]; do flags="$flags${BASH_REMATCH[1]}"; rest="${BASH_REMATCH[2]}"; done
        [[ "$flags" == *[rR]* || "$flags" == *recursive* ]] || continue
        tgt="$(printf '%s' "$rest" | sed -E 's/[;&|)].*$//; s/[0-9]*>.*$//')"
        if printf '%s' "$tgt" | grep -qE '(^|[[:space:]])\$|\*|GS024_SEED_ARCHIVE|GS024_SEED_CKPT_DIR|testnet/seed|testnet/n[0-9]|keys|node_key' \
           || printf '%s' "$tgt" | grep -qE '(^|[[:space:]])"?(/|~|\$\{?HOME\}?|\$\{?GS024_POOL\}?)"?/?([[:space:]]|$)'; then
            echo "$line"
        fi
    done
}

sck() {
    local name="$1" cond="$2" d="${3:-}"
    if [[ -f "$LIB" && -f "$HLIB" ]] && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "$( [[ -f "$LIB" && -f "$HLIB" ]] && echo "${d:-condition false}" || echo "missing: $LIB or $HLIB" )"; fi
}

# --- throttle / irpc / full-run helpers ---

fast_sleep() { printf '#!/usr/bin/env bash\nexit 0\n' > "$BIN_DIR/sleep"; chmod +x "$BIN_DIR/sleep"; }
tp_calls() { grep -E '^taskpolicy ' "$CURL_LOG" 2>/dev/null; }
no_tp()    { [[ -z "$(tp_calls)" ]] && ! grep -q '^taskpolicy ' "$MUTATE_LOG"; }
ncurl()    { grep -cE "^curl .*127\.0\.0\.1:$1([^0-9]|\$)" "$CURL_LOG"; }
lno()      { grep -nE -- "$1" "$CURL_LOG" | head -n 1 | cut -d: -f1; }
before()   { local a b; a="$(lno "$1")"; b="$(lno "$2")"; [[ -n "$a" && -n "$b" && "$a" -lt "$b" ]]; }
result_has() { printf '%s\n' "$OUT" | grep -qE "^GS024 RESULT: .* $1( |\$)"; }
result_line() { printf '%s\n' "$OUT" | grep -m 1 'GS024 RESULT'; }
out_has()  { printf '%s\n' "$OUT" | grep -qE -- "$1"; }

mt_bad() {
    local l t
    grep -E "^curl .*127\.0\.0\.1:$1([^0-9]|\$)" "$CURL_LOG" | while IFS= read -r l; do
        t="$(printf '%s' "$l" | sed -nE 's/.* (-m|--max-time)[ =]?([0-9]+)( .*)?$/\2/p')"
        if [ -z "$t" ] || { [ "$2" = ne ] && [ "$t" -ne "$3" ]; } || { [ "$2" = lt ] && [ "$t" -lt "$3" ]; }; then echo "$l"; fi
    done
}

reap() {
    local f p
    for f in "$CASE_DIR/main.pid" "$FIX_DIR/node.pid" "$CASE_DIR/hold.pid"; do
        p="$(cat "$f" 2>/dev/null)"
        [ -n "$p" ] && ps -p "$p" -o command= 2>/dev/null | grep -qF -e "$CASE_DIR" -e "$LIB" && kill -KILL "$p" 2>/dev/null
    done
    return 0
}

run_sandbox() {
    new_sandbox "$1"; T_MIN=10; fast_sleep; canon 49980
    local h=49981
    while [ "$h" -le 50000 ]; do seed_has "$h" "" 8590; h=$((h + 1)); done
    echo "$2" > "$FIX_DIR/8590/tip"; echo "49981 50000" > "$FIX_DIR/arch.range"; echo 50000 > "$FIX_DIR/catchup.tip"
    printf '{\n  "genesis_hash": "%s",\n  "latest_height": 50000\n}\n' "$(hx 1)" > "$CASE_DIR/seedarch/manifest.json"
    : > "$CASE_DIR/seedarch/0000049000.block"; : > "$CASE_DIR/seedarch/0000049990.block"
    printf '#!/usr/bin/env bash\necho "Filesystem 1024-blocks Used Available Capacity Mounted"\necho "/dev/x 999999999 1 999999999 1%% /"\n' > "$BIN_DIR/df"
    cat > "$NODE_BIN" <<'STUB'
#!/usr/bin/env bash
case " $* " in *" --version "*) echo "doli-node 6.44.0 (gs024 stub)"; exit 0 ;; esac
echo "bin $*" >> "${BIN_LOG:?}"
echo "$$" > "${FIX_DIR:?}/node.pid"
a=""; p=""
for x in "$@"; do [ "$p" = "--archive-to" ] && a="$x"; p="$x"; done
if [ -n "$a" ] && read -r lo hi < "$FIX_DIR/arch.range"; then
    h="$lo"
    while [ "$h" -le "$hi" ]; do printf -v f '%s/%010d.block' "$a" "$h"; : > "$f"; h=$((h + 1)); done
fi
[ -f "$FIX_DIR/snapline" ] && cat "$FIX_DIR/snapline"
trap 'echo "node TERM" >> "${CURL_LOG:?}"; exit 0' TERM
n=0
while [ "$n" -lt 900 ]; do /bin/sleep 0.1; n=$((n + 1)); done
STUB
    chmod +x "$BIN_DIR/df" "$NODE_BIN"
}

_run_body() {
    cd "$CWD_DIR" || exit 1
    _env
    local rc=0
    bash "$LIB" < /dev/null > "$CASE_DIR/out" 2> "$CASE_DIR/err" &
    echo "$!" > "$CASE_DIR/main.pid"
    wait "$!" || rc=$?
    printf '%s' "$rc" > "$CASE_DIR/rc"
}

run_full() {
    bounded _run_body; _collect; reap
    if [[ -f "$LIB" ]]; then DEFINED=yes; else DEFINED=no; fi
    NPID="$(cat "$FIX_DIR/node.pid" 2>/dev/null)"
}
mt_det() { echo "rc=$RC npid=$NPID tp=[$(tp_calls | tr '\n' '|')] result=[$(result_line)] timedout=$([ -f "$CASE_DIR/timedout" ] && echo yes || echo no) err=[$(printf '%s' "$ERR" | head -2 | tr '\n' '|')]"; }
fail_line() { printf '%s\n' "$OUT" | grep -m 1 -E "(PASS|FAIL) $1"; }

# --- scanner fixture: JSON-RPC getBlockByHeight server on an ephemeral port ---

# srv_start NAME HI MISSING_CSV ALTERED_CSV — sets SRV_URL; ALTERED heights answer another hash.
srv_start() {
    local pf="$TEST_DIR/srv-$1.port" n=0
    SRV_URL=""
    python3 -I -c '
import sys, json, http.server
pf, hi = sys.argv[1], int(sys.argv[2])
miss = {int(x) for x in sys.argv[3].split(",") if x}
alt = {int(x) for x in sys.argv[4].split(",") if x}
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        p = req.get("params"); h = p.get("height") if isinstance(p, dict) else None
        if req.get("method") == "getBlockByHeight" and isinstance(h, int) and 1 <= h <= hi and h not in miss:
            out = {"jsonrpc": "2.0", "id": req.get("id"), "result": {"hash": "%064x" % (h + (7 if h in alt else 0)), "height": h}}
        else:
            out = {"jsonrpc": "2.0", "id": req.get("id"), "error": {"code": -32000, "message": "Block not found"}}
        b = json.dumps(out).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
open(pf + ".tmp", "w").write(str(s.server_address[1])); __import__("os").rename(pf + ".tmp", pf)
s.serve_forever()
' "$pf" "$2" "$3" "$4" >/dev/null 2>&1 &
    SRV_PIDS="$SRV_PIDS $!"; disown "$!" 2>/dev/null
    while [ ! -s "$pf" ] && [ "$n" -lt 100 ]; do sleep 0.1; n=$((n + 1)); done
    [ -s "$pf" ] && SRV_URL="http://127.0.0.1:$(cat "$pf")"
}

# scan_run ARGS... — runs the scanner; sets OUT, RC, DEFINED (yes when the scanner exists).
scan_run() {
    local f="$TEST_DIR/scan.out"
    rm -f "$f.timedout"
    RC=99; OUT=""; DEFINED=no
    [[ -f "$SCAN" ]] || return 0
    DEFINED=yes
    python3 -I "$SCAN" "$@" > "$f" 2>&1 & local p=$! n=0
    while kill -0 "$p" 2>/dev/null && [ "$n" -lt 300 ]; do sleep 0.1; n=$((n + 1)); done
    kill -0 "$p" 2>/dev/null && { kill -KILL "$p" 2>/dev/null; : > "$f.timedout"; }
    wait "$p"; RC=$?
    OUT="$(cat "$f")"
}
sc_ok() { [[ "$DEFINED" == yes ]] && [[ ! -f "$TEST_DIR/scan.out.timedout" ]]; }
sc_ck() {
    if sc_ok && eval "$2"; then test_result "$1" "pass" ""
    else test_result "$1" "fail" "rc=$RC scanner=$SCAN exists=$DEFINED out=[$OUT]"; fi
}
