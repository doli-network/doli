#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/gauntlet-gs018.sh — `_gs018_assert <token>` (INC-I-178, GS-018)
#   O1 return code   — 0 PASS · 1 FAIL · 2 SKIP. gauntlet.sh:684-689 treats 0 and 2
#                      IDENTICALLY, so O1 alone can never separate "checked green" from
#                      "never checked".
#   O2 FAIL_REASONS  — caller-owned global, appended on rc 1 only.
#   O3 SKIP_REASONS  — caller-owned global, appended on rc 2 only. The ONLY signal that
#                      distinguishes a skip from a pass in the runner's output, so every
#                      SKIP partition below asserts it non-empty; a token returning 2
#                      without one is a silent hole in the fleet's coverage.
#   O4 INFO_REASONS  — caller-owned global, appended on rc 0 only. Exception, stated so
#                      the developer is not boxed in: gs018-active-producers-dual-sign can
#                      only ever return 2 on this build, so its measured denominator and
#                      new-build node count are accepted in EITHER O3 or O4.
#   O5 SAFETY        — the stubbed curl/sqlite3/pkill/launchctl log plus a keys-dir
#                      fingerprint. GS-018 is chain-read-only and state-neutral: no
#                      mutating RPC (sendTransaction, submitTransaction, broadcast,
#                      forceReorgTo), no write under ~/testnet/keys, no pkill/kill/
#                      launchctl, and no INSERT into gauntlet_runs — the runner owns that
#                      table (gauntlet.sh:718-725) and writes exactly one row per run.
#   O6 REGISTRATION  — the gauntlet.sh source block, the assert() dispatch arm and the
#                      assertions column of the GS-018 row in gauntlet-seed.sql. A green
#                      library the host runner never reaches is false assurance: the
#                      2026-09-01 run failed GS-015 with "unknown assertion token" while
#                      its unit suite was 100% green.
#
# PATHS (fact sheet docs/.workflow/m7-fact-sheet.md is authoritative for every fact here):
#   token gs018-presence-root-consistent  (REQ-BLS-013)
#     -> sample recent heights from >= 3 distinct answering nodes
#        -> fewer than 3 answered / none answered / no python3 => rc 2 + O3
#        -> same height, different presenceRoot across nodes    => rc 1 + O2 naming the height
#        -> pre-AH: aggregateBlsSig present on a sampled block  => the pre-AH branch no
#           longer applies (its presence IS the AH litmus, sheet §2), so never rc 1 for that
#        -> consistent                                          => rc 0 + O4
#   token gs018-active-producers-dual-sign  (REQ-BLS-006 AC-2)
#     -> ALWAYS rc 2 + O3 naming the missing observable. Sheet §4: the ingress VALID path
#        logs nothing at any level, no metric carries a producer label, no RPC exposes
#        parent_sig_pool, and getAttestationStats.hasBls is key REGISTRATION (already true
#        on the OLD build). "0 unverifiable-BLS warnings therefore 5/5" is a FALSE GREEN.
#     -> reports the denominator = count of getProducers rows with status=="active" (5 on
#        this fleet, NOT 7 rows, NOT 17 or 18 nodes) and the new-build node count as INFO.
#   token gs018-post-ah-aggregate-verifies  (REQ-BLS-013)
#     -> AH litmus (sheet §2): doli_attestation_verify_total > 0 on any node, OR
#        getAttestationStats.blocksWithBls > 0, OR aggregateBlsSig present on a block.
#        None of the three => rc 2 + O3 containing `pre-AH`. Never rc 1 pre-AH.
#        Any of the three => the token must actually evaluate, not skip as pre-AH.
#
# LIBRARY ENV CONTRACT (this test IS the contract the developer implements against):
#   GS018_PORTS · GS018_METRICS_PORTS · GS018_LOG_DIR · GS018_SAMPLE · WORK
#
# INPUT PARTITIONS:
#   S1:  healthy pre-AH, roots agree across nodes         — O1=0, O4 set, O2/O3 empty
#   S2:  presenceRoot diverges at one height across nodes — O1=1, O2 names the height
#   S3:  only 2 nodes answer                              — O1=2, O3 set, O2/O4 empty
#   S4:  no node answers at all                           — O1=2, O3 set, never O1=1
#   S5:  python3 absent from PATH                         — O1=2, O3 set, never O1=1
#   S6:  node logs unreadable                             — O1!=1
#   S7:  aggregateBlsSig present (AH crossed)             — O1!=1 on the pre-AH branch
#   S8:  attestationCount 5 vs 131, all else equal        — identical O1 and O2 (sheet §5:
#        it is a popcount of a HASH, meaningless as a headcount)
#   S9:  presence-root path is read-only                  — O5
#   S10: dual-sign, zero BLS warnings in every log        — O1=2 (never 0), O3 set
#   S11: dual-sign, every producer hasBls:true            — O1=2 (never 0), O2 empty
#   S12: dual-sign, warnings PRESENT in the logs          — O1=2 (never 1)
#   S13: dual-sign SKIP reason names the missing observable— O3 matches the §4 vocabulary
#   S14: 7 producer rows, 5 active + 2 exited             — reported denominator 5
#   S15: 3 active + 4 exited, same 18 nodes               — reported denominator 3, and the
#        text DIFFERS from S14 (an implementation dividing by node count reports the same)
#   S16: 0 vs 4 nodes carrying doli_attestation_verify_total — the reported build count differs
#   S17: dual-sign with no live RPC                       — O1=2, O3 set, never O1=1
#   S18: dual-sign path is read-only                      — O5
#   S19: pre-AH (verify_total 0, blocksWithBls 0, no agg) — O1=2, O3 contains `pre-AH`
#   S20: AH crossed via blocksWithBls > 0                 — evaluates; not a pre-AH skip
#   S21: AH crossed via doli_attestation_verify_total 3   — evaluates; not a pre-AH skip
#   S22: AH crossed via aggregateBlsSig on a block        — evaluates; not a pre-AH skip
#   S23: post-AH token with no live RPC                   — O1=2, O3 set, never O1=1
#   S24: post-AH path is read-only                        — O5
#   S25: network=="mainnet" on every node                 — every token O1=2, O3 set, O5 clean
#   S26: unknown gs018-* token                            — O1=1
#   S27: no confirm-var, no --gs018 flag (DEFAULT-run)    — O6
#   S28: gauntlet.sh sources scripts/gauntlet-gs018.sh    — O6
#   S29: gauntlet.sh assert() routes all three tokens     — O6
#   S30: gauntlet-seed.sql registers GS-018 active + tokens— O6
#   S31: library parses under bash -n                     — hygiene
# MATRIX: 6 outputs x 31 partitions (only cells a path reaches are asserted)
#   S1: O1 O2 O3 O4 | S2: O1 O2 | S3: O1 O2 O3 O4 | S4: O1 O3 | S5: O1 O3 | S6: O1
#   S7: O1 | S8: O1 O2 | S9: O5 | S10: O1 O3 O4 | S11: O1 O2 | S12: O1 | S13: O3
#   S14: O3 O4 | S15: O3 O4 | S16: O3 O4 | S17: O1 O3 | S18: O5 | S19: O1 O2 O3
#   S20: O1 O3 | S21: O1 O3 | S22: O1 O3 | S23: O1 O3 | S24: O5 | S25: O1 O3 O5
#   S26: O1 | S27: O6 | S28: O6 | S29: O6 | S30: O6 | S31: parse
#
# TDD RED tests for scripts/gauntlet-gs018.sh, which DOES NOT EXIST YET. `curl`, `sqlite3`,
# `pkill` and `launchctl` are stubbed on PATH and HOME is redirected into the sandbox, so
# every default the library derives from it resolves to fixtures: no live testnet, no
# network, no chain, and no node started, stopped or perturbed.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
GS018_LIB="$PROJECT_ROOT/scripts/gauntlet-gs018.sh"
GAUNTLET="$PROJECT_ROOT/scripts/gauntlet.sh"
SEED_FILE="$PROJECT_ROOT/scripts/gauntlet-seed.sql"
TEST_DIR="/tmp/doli-gauntlet-gs018-test-$$"

TOKEN_ROOT="gs018-presence-root-consistent"
TOKEN_DUAL="gs018-active-producers-dual-sign"
TOKEN_POSTAH="gs018-post-ah-aggregate-verifies"

RPC_PORTS="8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517"
MET_PORTS="9000 9001 9002 9003 9004 9005 9006 9007 9008 9009 9010 9011 9012 9013 9014 9015 9016 9017"
BASE_HEIGHT=110836

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
cleanup() { rm -rf "$TEST_DIR"; }
trap cleanup EXIT
rm -rf "$TEST_DIR"; mkdir -p "$TEST_DIR"

# A missing gauntlet-gs018.sh means _gs018_assert is never called, RC keeps the 99 sentinel
# and the *_REASONS stay empty — vacuously satisfying "rc non-zero", "INFO empty" and "no
# mutating RPC". `ck` conjoins every behavioural cell with this, so nothing goes green
# without a real library, and a missing library reports as a clear FAIL, not a bash error.
lib_ok() { [[ -f "$GS018_LIB" ]] && [[ "$FUNC_DEFINED" == "yes" ]]; }
detail() { echo "rc=$RC func=$FUNC_DEFINED lib=$GS018_LIB fail=[$R_FAIL] skip=[$R_SKIP] info=[$R_INFO] log=$LOG_FILE"; }

ck() {
    local name="$1" cond="$2" det="${3:-}"
    if lib_ok && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "${det:-$(detail)}"; fi
}

# --- stub writers ---

# `curl` stub. GET *.../metrics serves $RPC_DIR/<port>.metrics; a JSON-RPC POST is answered
# from per-port fixture files. A port with no <port>.up (RPC) or no <port>.metrics exits 7,
# curl's "couldn't connect" — how a node that is simply not running presents itself.
write_curl_stub() {
    cat > "$1/curl" <<'CURL_STUB'
#!/usr/bin/env bash
echo "curl $*" >> "${CURL_LOG:?CURL_LOG not set}"
url=""; data=""; prev=""
for a in "$@"; do
    case "$a" in http://*) url="$a" ;; esac
    if [ "$prev" = "-d" ] || [ "$prev" = "--data" ] || [ "$prev" = "--data-binary" ]; then data="$a"; fi
    prev="$a"
done
hostport="${url#http://}"; hostport="${hostport%%/*}"; port="${hostport##*:}"
R="${RPC_DIR:?RPC_DIR not set}"
case "$url" in
    */metrics*)
        [ -f "$R/$port.metrics" ] || exit 7
        cat "$R/$port.metrics"; exit 0 ;;
esac
[ -f "$R/$port.up" ] || exit 7
method="$(printf '%s' "$data" | sed -n 's/.*"method"[[:space:]]*:[[:space:]]*"\([A-Za-z]*\)".*/\1/p')"
case "$method" in
    getChainInfo)
        net="$(cat "$R/$port.network" 2>/dev/null || echo testnet)"
        h="$(cat "$R/$port.height" 2>/dev/null || echo 110836)"
        printf '{"jsonrpc":"2.0","id":1,"result":{"bestHeight":%s,"bestHash":"050fd33e543d","bestSlot":%s,"network":"%s","version":"6.26.3"}}\n' "$h" "$h" "$net" ;;
    getNodeInfo)
        net="$(cat "$R/$port.network" 2>/dev/null || echo testnet)"
        printf '{"jsonrpc":"2.0","id":1,"result":{"version":"6.26.3","network":"%s","peerId":"12D3KooWgs018","peerCount":17}}\n' "$net" ;;
    getAttestationStats)
        body="$(cat "$R/$port.attstats.json" 2>/dev/null)"
        [ -n "$body" ] || body='{}'
        printf '{"jsonrpc":"2.0","id":1,"result":%s}\n' "$body" ;;
    getProducers)
        body="$(cat "$R/producers.json" 2>/dev/null)"
        printf '{"jsonrpc":"2.0","id":1,"result":{"producers":%s}}\n' "${body:-[]}" ;;
    getBlockByHeight|getBlockByHash)
        h="$(printf '%s' "$data" | sed -n 's/.*"height"[[:space:]]*:[[:space:]]*\([0-9]*\).*/\1/p')"
        [ -n "$h" ] || h=0
        if [ -f "$R/$port.block.$h.json" ]; then
            printf '{"jsonrpc":"2.0","id":1,"result":%s}\n' "$(cat "$R/$port.block.$h.json")"; exit 0
        fi
        pre="$(cat "$R/$port.rootprefix" 2>/dev/null || echo aaaa)"
        cnt="$(cat "$R/$port.attcount" 2>/dev/null || echo 131)"
        root="$(printf '%s%060d' "$pre" "$h")"
        agg=""
        [ -f "$R/$port.agg" ] && agg=",\"aggregateBlsSig\":\"$(cat "$R/$port.agg")\""
        printf '{"jsonrpc":"2.0","id":1,"result":{"hash":"%s","prevHash":"%s","height":%s,"slot":%s,"timestamp":1757000000,"producer":"3047e96b","merkleRoot":"%s","txCount":0,"transactions":[],"size":512,"presenceRoot":"%s","attestationCount":%s%s}}\n' \
            "$root" "$root" "$h" "$h" "$root" "$root" "$cnt" "$agg" ;;
    getBlockRaw)
        h="$(printf '%s' "$data" | sed -n 's/.*"height"[[:space:]]*:[[:space:]]*\([0-9]*\).*/\1/p')"
        if [ -f "$R/$port.rawfail" ]; then
            printf '{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}\n'
        else
            printf '{"jsonrpc":"2.0","id":1,"result":{"block":"AAECAwQ=","blake3":"%064d","height":%s}}\n' "${h:-0}" "${h:-0}"
        fi ;;
    *)
        printf '{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}\n' ;;
esac
exit 0
CURL_STUB
    chmod +x "$1/curl"
}

# Every process- or state-mutating tool the library must never reach. sqlite3 logs its argv
# AND its stdin, so `sqlite3 db < file` is caught as well as an inline statement.
write_mutation_stubs() {
    cat > "$1/sqlite3" <<'SQL_STUB'
#!/usr/bin/env bash
{ echo "sqlite3 $*"; cat; } >> "${MUTATE_LOG:?MUTATE_LOG not set}"
exit 0
SQL_STUB
    cat > "$1/pkill" <<'PK_STUB'
#!/usr/bin/env bash
echo "pkill $*" >> "${MUTATE_LOG:?MUTATE_LOG not set}"
exit 0
PK_STUB
    cat > "$1/launchctl" <<'LC_STUB'
#!/usr/bin/env bash
echo "launchctl $*" >> "${MUTATE_LOG:?MUTATE_LOG not set}"
exit 0
LC_STUB
    chmod +x "$1/sqlite3" "$1/pkill" "$1/launchctl"
}

# Shadows /usr/bin/python3 with a 127 exit — the sandbox PATH puts BIN_DIR first.
write_nopython_stub() {
    printf '#!/usr/bin/env bash\nexit 127\n' > "$1/python3"; chmod +x "$1/python3"
}

# --- fixtures ---

new_sandbox() {
    local name="$1" p i=0
    CASE_DIR="$TEST_DIR/$name"
    WORK_DIR="$CASE_DIR/work"; BIN_DIR="$CASE_DIR/bin"; HOME_DIR="$CASE_DIR/home"
    KEYS_DIR="$HOME_DIR/testnet/keys"; LOGS_DIR="$HOME_DIR/testnet/logs"
    RPC_DIR="$CASE_DIR/rpc"
    CURL_LOG="$CASE_DIR/curl.log"; MUTATE_LOG="$CASE_DIR/mutate.log"; LOG_FILE="$CASE_DIR/run.log"
    rm -rf "$CASE_DIR"
    mkdir -p "$WORK_DIR" "$BIN_DIR" "$KEYS_DIR" "$LOGS_DIR" "$RPC_DIR" "$HOME_DIR/testnet/bin"
    : > "$CURL_LOG"; : > "$MUTATE_LOG"; : > "$LOG_FILE"
    write_curl_stub "$BIN_DIR"; write_mutation_stubs "$BIN_DIR"
    echo '{"sentinel":"producer_5"}' > "$KEYS_DIR/producer_5.json"
    KEYS_FP="$(keys_fingerprint)"
    write_producers 5 2
    for p in $RPC_PORTS; do rpc_up "$p"; att_stats "$p" 0; done
    for p in $MET_PORTS; do metrics_old "$p"; done
    write_logs
    i=0
}

keys_fingerprint() { (cd "$KEYS_DIR" 2>/dev/null && ls -1 . && cat ./* 2>/dev/null) | cksum; }

rpc_up()   { : > "$RPC_DIR/$1.up"; echo "$BASE_HEIGHT" > "$RPC_DIR/$1.height"; echo testnet > "$RPC_DIR/$1.network"; echo aaaa > "$RPC_DIR/$1.rootprefix"; }
rpc_down() { rm -f "$RPC_DIR/$1.up"; }
set_network() { echo "$2" > "$RPC_DIR/$1.network"; }
set_rootprefix() { echo "$2" > "$RPC_DIR/$1.rootprefix"; }
set_attcount() { echo "$2" > "$RPC_DIR/$1.attcount"; }
set_aggregate() { echo "${2:-b7c1aa02}" > "$RPC_DIR/$1.agg"; }

# 7 chain-registered producer rows: $1 active + $2 exited. All carry a 96-hex-char blsPubkey
# on the OLD build already, so hasBls can never be dual-sign evidence (fact sheet §4).
write_producers() {
    local n=0 out="[" first=1 i
    for ((i=0; i<$1; i++)); do
        [ "$first" = 1 ] || out="$out,"; first=0
        out="$out{\"publicKey\":\"$(printf 'a%063d' $n)\",\"status\":\"active\",\"blsPubkey\":\"$(printf 'b%095d' $n)\",\"bondAmount\":100000000000,\"bondCount\":1}"
        n=$((n+1))
    done
    for ((i=0; i<$2; i++)); do
        [ "$first" = 1 ] || out="$out,"; first=0
        out="$out{\"publicKey\":\"$(printf 'c%063d' $n)\",\"status\":\"exited\",\"blsPubkey\":\"$(printf 'd%095d' $n)\",\"bondAmount\":0,\"bondCount\":0}"
        n=$((n+1))
    done
    echo "$out]" > "$RPC_DIR/producers.json"
}

# getAttestationStats. $2 = blocksWithBls. $3 = hasBls literal (default true — the OLD-build
# reality: every producer already has a registered BLS key.)
att_stats() {
    local port="$1" bls="$2" has="${3:-true}" i rows="" first=1
    for i in 0 1 2 3 4; do
        [ "$first" = 1 ] || rows="$rows,"; first=0
        rows="$rows{\"publicKey\":\"$(printf 'a%063d' $i)\",\"attestedMinutes\":58,\"totalMinutes\":60,\"threshold\":54,\"qualified\":true,\"hasBls\":$has}"
    done
    cat > "$RPC_DIR/$port.attstats.json" <<ATT
{"epoch":307,"epochStart":110520,"currentHeight":$BASE_HEIGHT,"blocksInEpoch":316,"blocksWithAttestations":316,"blocksWithBls":$bls,"currentMinute":42,"producers":[$rows]}
ATT
}

metrics_old() {
    cat > "$RPC_DIR/$1.metrics" <<'MET'
# HELP doli_attestation_misses_total Attestation misses
doli_attestation_misses_total 12
doli_attestation_missing_current 0
doli_chain_height 110836
MET
}

# The INC-I-178 build marker (fact sheet §3): registration is the signal, not the value, so
# the default emits verify_total at 0. $2 overrides the value for the AH litmus partitions.
metrics_new() {
    metrics_old "$1"
    cat >> "$RPC_DIR/$1.metrics" <<MET
doli_attestation_verify_total ${2:-0}
doli_attestation_verify_rejected_total{reason="root_mismatch"} 0
doli_attestation_verify_skipped_light_total 0
doli_attestation_bitfield_fill_ratio 0.83
MET
}

metrics_down() { rm -f "$RPC_DIR/$1.metrics"; }

write_logs() {
    local n
    for n in seed n1 n2 n3 n4 n5 n6 n7 n8 n9 n10 n11 n12 n13 n14 n15 n16 n17; do
        {
            echo "2026-09-04T10:00:00Z INFO Applied block h=110834 hash=050fd33e"
            echo "2026-09-04T10:00:10Z INFO Applied block h=110835 hash=1a2b3c4d"
        } > "$LOGS_DIR/$n.log"
    done
}

# The ONLY per-producer BLS log line that exists (ingress.rs:95-103). Its ABSENCE proves
# nothing (sheet §4) and its PRESENCE is still not a dual-sign measurement.
append_bls_warn() {
    echo "2026-09-04T10:01:00Z WARN [ATTEST_INGEST] unverifiable BLS half from $(printf 'a%063d' 0) relayed by 12D3KooWpeer (score 3)" >> "$LOGS_DIR/$1.log"
}

# --- runner ---
run_assert() {
    local token="$1"
    (
        set +e
        unset NODECFG GS018_AH_HEIGHT
        cd "$WORK_DIR" || exit 1
        export PATH="$BIN_DIR:/usr/bin:/bin"
        export HOME="$HOME_DIR"
        export CURL_LOG MUTATE_LOG RPC_DIR
        export GS018_PORTS="$RPC_PORTS" GS018_METRICS_PORTS="$MET_PORTS"
        export GS018_LOG_DIR="$LOGS_DIR" GS018_SAMPLE="${TEST_SAMPLE:-5}"
        export WORK="$WORK_DIR"
        export GAUNTLET_WINDOW="${TEST_WINDOW:-45}"
        FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
        func="no"
        if [[ -f "$GS018_LIB" ]]; then
            # shellcheck disable=SC1090  # path is a test parameter, resolved at runtime
            . "$GS018_LIB" >/dev/null 2>&1
        fi
        declare -F _gs018_assert >/dev/null 2>&1 && func="yes"
        rc=99
        if [[ "$func" == "yes" ]]; then
            rc=0
            _gs018_assert "$token" < /dev/null > "$LOG_FILE" 2>&1 || rc=$?
        fi
        printf '%s' "$func" > "$CASE_DIR/func"; printf '%s' "$rc" > "$CASE_DIR/rc"
        printf '%s' "$FAIL_REASONS" > "$CASE_DIR/fail"
        printf '%s' "$SKIP_REASONS" > "$CASE_DIR/skip"
        printf '%s' "$INFO_REASONS" > "$CASE_DIR/info"
    )
    FUNC_DEFINED="$(cat "$CASE_DIR/func" 2>/dev/null || echo no)"
    RC="$(cat "$CASE_DIR/rc" 2>/dev/null || echo 99)"
    R_FAIL="$(cat "$CASE_DIR/fail" 2>/dev/null || true)"
    R_SKIP="$(cat "$CASE_DIR/skip" 2>/dev/null || true)"
    R_INFO="$(cat "$CASE_DIR/info" 2>/dev/null || true)"
    R_ALL="$R_SKIP $R_INFO"
}

# --- O5 predicates ---
mutating_rpc()   { grep -qEi 'sendTransaction|submitTransaction|broadcast|forceReorgTo|submitAttestation' "$CURL_LOG" 2>/dev/null; }
touched_procs()  { grep -qEi 'pkill|launchctl' "$MUTATE_LOG" 2>/dev/null; }
wrote_runs_row() { grep -qiE 'insert[[:space:]]+into[[:space:]]+gauntlet_runs' "$MUTATE_LOG" 2>/dev/null; }
keys_touched()   { [[ "$(keys_fingerprint)" != "$KEYS_FP" ]]; }
o5_clean()       { ! mutating_rpc && ! touched_procs && ! wrote_runs_row && ! keys_touched; }
o5_detail()      { echo "curl=$CURL_LOG mutate=$(tr '\n' ' ' < "$MUTATE_LOG") keys_fp=$(keys_fingerprint) expect=$KEYS_FP"; }
has_word()       { printf '%s' "$2" | grep -qE "(^|[^0-9A-Za-z])$1([^0-9A-Za-z]|$)"; }

print_header "gauntlet-gs018.sh RED tests (INC-I-178, GS-018)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# ── gs018-presence-root-consistent ──────────────────────────────────────────

# S1 — REQ-BLS-013 (Must) — Decision: a scenario that cannot report a healthy pre-AH fleet green is permanently red, earns a standing waiver, and stops guarding bitfield integrity at all.
new_sandbox "s1_roots_agree"; run_assert "$TOKEN_ROOT"
ck "S1 roots_agree: rc 0 (PASS)" '[[ "$RC" -eq 0 ]]'
ck "S1 roots_agree: INFO_REASONS records the sample" '[[ -n "$R_INFO" ]]'
ck "S1 roots_agree: FAIL_REASONS and SKIP_REASONS empty" '[[ -z "$R_FAIL" && -z "$R_SKIP" ]]'

# S9 — REQ-BLS-013 (Must, safety) — Decision: GS-018 is a DEFAULT-run scenario, so any mutation it performs fires unattended on every gauntlet invocation against the live testnet.
ck "S9 root_path: no mutating RPC, no pkill/launchctl, no keys write, no gauntlet_runs INSERT" 'o5_clean' "$(o5_detail)"

# S2 — REQ-BLS-013 (Must) — Decision: two nodes disagreeing on presenceRoot at one height is a real consensus divergence; reporting it green is the exact blindness GS-018 exists to remove.
new_sandbox "s2_root_divergence"; set_rootprefix 8503 bbbb; run_assert "$TOKEN_ROOT"
ck "S2 root_divergence: rc 1 (FAIL)" '[[ "$RC" -eq 1 ]]'
ck "S2 root_divergence: FAIL_REASONS non-empty" '[[ -n "$R_FAIL" ]]'
ck "S2 root_divergence: FAIL_REASONS names a sampled height" 'printf "%s" "$R_FAIL" | grep -qE "[0-9]{5,}"'

# S3 — REQ-BLS-013 (Must) — Decision: agreement measured over fewer than 3 nodes is not cross-node agreement; calling it a pass certifies a property never checked.
new_sandbox "s3_two_nodes"
for p in $RPC_PORTS; do rpc_down "$p"; done
rpc_up 8500; rpc_up 8501; att_stats 8500 0; att_stats 8501 0
run_assert "$TOKEN_ROOT"
ck "S3 two_nodes: rc 2 (SKIP)" '[[ "$RC" -eq 2 ]]'
ck "S3 two_nodes: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'
ck "S3 two_nodes: FAIL_REASONS and INFO_REASONS empty" '[[ -z "$R_FAIL" && -z "$R_INFO" ]]'

# S4 — REQ-BLS-013 (Must) — Decision: an unreachable fleet is an environment condition; one false FAIL is how a scenario earns a standing waiver and stops guarding anything.
new_sandbox "s4_no_rpc"; for p in $RPC_PORTS; do rpc_down "$p"; done
run_assert "$TOKEN_ROOT"
ck "S4 no_rpc: rc 2, never 1" '[[ "$RC" -eq 2 ]]'
ck "S4 no_rpc: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'

# S5 — REQ-BLS-013 (Must) — Decision: no jq on the runner means python3 does all JSON parsing; a missing interpreter must degrade to SKIP, not to a fabricated verdict.
new_sandbox "s5_no_python"; write_nopython_stub "$BIN_DIR"; run_assert "$TOKEN_ROOT"
ck "S5 no_python: rc 2, never 1" '[[ "$RC" -eq 2 ]]'
ck "S5 no_python: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'

# S6 — REQ-BLS-013 (Must) — Decision: n13-n17 run at log level warn (fact sheet §1), so an empty or missing log is normal and must never be read as a defect.
new_sandbox "s6_no_logs"; rm -rf "$LOGS_DIR"; run_assert "$TOKEN_ROOT"
ck "S6 no_logs: rc != 1" '[[ "$RC" -ne 1 ]]'

# S7 — REQ-BLS-013 (Must) — Decision: aggregateBlsSig appearing IS the AH-crossed litmus (sheet §2); failing on it would turn a successful activation into a red gauntlet.
new_sandbox "s7_aggregate_present"; for p in $RPC_PORTS; do set_aggregate "$p"; done
run_assert "$TOKEN_ROOT"
ck "S7 aggregate_present: rc != 1 (the pre-AH branch no longer applies)" '[[ "$RC" -ne 1 ]]'
ck "S7 aggregate_present: a rc-2 outcome still writes SKIP_REASONS" '[[ "$RC" -ne 2 || -n "$R_SKIP" ]]'

# S8 — REQ-BLS-013 (Must) — Decision: attestationCount is a popcount of the presence_root HASH (sheet §5); any implementation reading it as a headcount produces a verdict driven by hash entropy.
new_sandbox "s8a_attcount_5"; for p in $RPC_PORTS; do set_attcount "$p" 5; done
run_assert "$TOKEN_ROOT"; RC_A="$RC"; FAIL_A="$R_FAIL"
new_sandbox "s8b_attcount_131"; for p in $RPC_PORTS; do set_attcount "$p" 131; done
run_assert "$TOKEN_ROOT"
ck "S8 attcount_trap: rc identical for attestationCount 5 and 131" '[[ "$RC_A" == "$RC" ]]' "rc(5)=$RC_A rc(131)=$RC"
ck "S8 attcount_trap: FAIL_REASONS identical for both" '[[ "$FAIL_A" == "$R_FAIL" ]]' "fail(5)=[$FAIL_A] fail(131)=[$R_FAIL]"
ck "S8 attcount_trap: neither fixture FAILs" '[[ "$RC_A" -ne 1 && "$RC" -ne 1 ]]'

# ── gs018-active-producers-dual-sign (REQ-BLS-006 AC-2) ─────────────────────

# S10 — REQ-BLS-006 (Must) — Decision: "0 unverifiable-BLS warnings therefore 5/5 dual-signing" is the exact false green the fact sheet §4 refuses; a rc 0 here certifies an unmeasured claim.
new_sandbox "s10_no_warnings"; run_assert "$TOKEN_DUAL"
ck "S10 dual_sign_no_warnings: rc 2 (SKIP), never 0" '[[ "$RC" -eq 2 ]]'
ck "S10 dual_sign_no_warnings: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'
ck "S10 dual_sign_no_warnings: FAIL_REASONS empty" '[[ -z "$R_FAIL" ]]'

# S13 — REQ-BLS-006 (Must) — Decision: a SKIP whose reason does not name the missing observable cannot convert into a work item, so the gap is recorded and then forgotten.
ck "S13 dual_sign: SKIP reason names the missing observable" \
   'printf "%s" "$R_SKIP" | grep -qiE "not observable|unobservable|no per-producer|per-producer|parent_sig_pool|no positive|logs nothing|no signal"' \
   "skip=[$R_SKIP]"

# S11 — REQ-BLS-006 (Must) — Decision: hasBls is BLS pubkey REGISTRATION, already true for all 7 producers on the OLD build (sheet §4); reading it as emission passes before a single line of M7 code ships.
new_sandbox "s11_hasbls_true"; for p in $RPC_PORTS; do att_stats "$p" 0 true; done
run_assert "$TOKEN_DUAL"
ck "S11 hasbls_trap: rc 2, never a claimed 5/5 pass" '[[ "$RC" -eq 2 ]]'
ck "S11 hasbls_trap: FAIL_REASONS empty" '[[ -z "$R_FAIL" ]]'
ck "S11 hasbls_trap: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'

# S12 — REQ-BLS-006 (Must) — Decision: the warning fires on a relayed half from any peer; failing on it makes an unmeasurable property look measured and red, which gets it waived.
new_sandbox "s12_warnings_present"; append_bls_warn n2; append_bls_warn n5
run_assert "$TOKEN_DUAL"
ck "S12 warnings_present: rc 2, never 1" '[[ "$RC" -eq 2 ]]'
ck "S12 warnings_present: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'

# S14 — REQ-BLS-006 (Must) — Decision: the AC-2 denominator is chain-registered ACTIVE producers (5), not 7 rows and not 17/18 nodes; a wrong denominator makes every future ratio wrong.
new_sandbox "s14_denominator_5"; write_producers 5 2; run_assert "$TOKEN_DUAL"
ALL_5="$R_ALL"
ck "S14 denominator: reason text reports 5" 'has_word 5 "$R_ALL"' "skip+info=[$R_ALL]"
ck "S14 denominator: reason text mentions active producers" 'printf "%s" "$R_ALL" | grep -qi "active"' "skip+info=[$R_ALL]"

# S15 — REQ-BLS-006 (Must) — Decision: an implementation dividing by the node count reports the SAME number for both fixtures; only a differential catches it, since 5 alone also matches a hardcoded 5.
new_sandbox "s15_denominator_3"; write_producers 3 4; run_assert "$TOKEN_DUAL"
ck "S15 denominator: 3 active + 4 exited reports 3" 'has_word 3 "$R_ALL"' "skip+info=[$R_ALL]"
ck "S15 denominator: text DIFFERS from the 5-active fixture" '[[ "$ALL_5" != "$R_ALL" ]]' "5-active=[$ALL_5] 3-active=[$R_ALL]"
new_sandbox "s15b_denominator_node_independent"; write_producers 5 2
for p in $RPC_PORTS; do rpc_down "$p"; done
for p in 8500 8501 8502; do rpc_up "$p"; att_stats "$p" 0; done
run_assert "$TOKEN_DUAL"
ck "S15 denominator: 5 active over only 3 answering nodes still reports 5" \
   'has_word 5 "$R_ALL"' "skip+info=[$R_ALL]"

# S16 — REQ-BLS-006 (Must) — Decision: versions are byte-identical across builds (sheet §3), so the metrics surface is the only build marker; a probe that cannot count it cannot say whether the deploy landed.
new_sandbox "s16a_build_0"; run_assert "$TOKEN_DUAL"; ALL_B0="$R_ALL"
new_sandbox "s16b_build_4"
for p in 9000 9001 9002 9003; do metrics_new "$p"; done
run_assert "$TOKEN_DUAL"
ck "S16 build_count: reason text changes when 4 nodes carry doli_attestation_verify_total" \
   '[[ "$ALL_B0" != "$R_ALL" ]]' "0-build=[$ALL_B0] 4-build=[$R_ALL]"
ck "S16 build_count: 4-node fixture reports 4" 'has_word 4 "$R_ALL"' "skip+info=[$R_ALL]"

# S17 — REQ-BLS-006 (Must) — Decision: an offline fleet is an environment condition, and one false FAIL is how a scenario earns a standing waiver.
new_sandbox "s17_dual_no_rpc"; for p in $RPC_PORTS; do rpc_down "$p"; done
for p in $MET_PORTS; do metrics_down "$p"; done
run_assert "$TOKEN_DUAL"
ck "S17 dual_no_rpc: rc 2, never 1" '[[ "$RC" -eq 2 ]]'
ck "S17 dual_no_rpc: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'

# S18 — REQ-BLS-006 (Must, safety) — Decision: this token runs on every default gauntlet invocation against the live testnet.
new_sandbox "s18_dual_readonly"; run_assert "$TOKEN_DUAL"
ck "S18 dual_path: no mutating RPC, no pkill/launchctl, no keys write, no gauntlet_runs INSERT" 'o5_clean' "$(o5_detail)"

# ── gs018-post-ah-aggregate-verifies ────────────────────────────────────────

# S19 — REQ-BLS-013 (Must) — Decision: the AH is u64::MAX on every network (sheet §2), so a post-AH assertion that FAILs pre-AH is red on every run from the day it ships.
new_sandbox "s19_pre_ah"; run_assert "$TOKEN_POSTAH"
ck "S19 pre_ah: rc 2 (SKIP)" '[[ "$RC" -eq 2 ]]'
ck "S19 pre_ah: SKIP_REASONS contains pre-AH" 'printf "%s" "$R_SKIP" | grep -qi "pre-ah"' "skip=[$R_SKIP]"
ck "S19 pre_ah: FAIL_REASONS empty" '[[ -z "$R_FAIL" ]]'

# S20 — REQ-BLS-013 (Must) — Decision: if the token still reports pre-AH after the AH is crossed, the aggregate is never verified on the live fleet and the scenario guards nothing post-activation.
new_sandbox "s20_ah_blockswithbls"; for p in $RPC_PORTS; do att_stats "$p" 12; done
run_assert "$TOKEN_POSTAH"
ck "S20 ah_via_blocksWithBls: does not skip as pre-AH" \
   '[[ "$RC" -ne 2 ]] || ! printf "%s" "$R_SKIP" | grep -qi "pre-ah"' "rc=$RC skip=[$R_SKIP]"

# S21 — REQ-BLS-013 (Must) — Decision: verify_total is the first counter to move at activation; missing it delays post-AH coverage until someone notices manually.
new_sandbox "s21_ah_metrics"; for p in $MET_PORTS; do metrics_new "$p" 3; done
run_assert "$TOKEN_POSTAH"
ck "S21 ah_via_verify_total: does not skip as pre-AH" \
   '[[ "$RC" -ne 2 ]] || ! printf "%s" "$R_SKIP" | grep -qi "pre-ah"' "rc=$RC skip=[$R_SKIP]"

# S22 — REQ-BLS-013 (Must) — Decision: a non-empty aggregateBlsSig on a block is structurally impossible pre-AH, so it is proof of activation the token must act on.
new_sandbox "s22_ah_aggregate"; for p in $RPC_PORTS; do set_aggregate "$p"; done
run_assert "$TOKEN_POSTAH"
ck "S22 ah_via_aggregate: does not skip as pre-AH" \
   '[[ "$RC" -ne 2 ]] || ! printf "%s" "$R_SKIP" | grep -qi "pre-ah"' "rc=$RC skip=[$R_SKIP]"

# S23 — REQ-BLS-013 (Must) — Decision: an unreachable fleet cannot disprove aggregate verification, so a FAIL there is a false accusation.
new_sandbox "s23_postah_no_rpc"; for p in $RPC_PORTS; do rpc_down "$p"; done
for p in $MET_PORTS; do metrics_down "$p"; done
run_assert "$TOKEN_POSTAH"
ck "S23 postah_no_rpc: rc 2, never 1" '[[ "$RC" -eq 2 ]]'
ck "S23 postah_no_rpc: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'

# S24 — REQ-BLS-013 (Must, safety) — Decision: same default-run exposure as every other GS-018 token.
new_sandbox "s24_postah_readonly"; run_assert "$TOKEN_POSTAH"
ck "S24 postah_path: no mutating RPC, no pkill/launchctl, no keys write, no gauntlet_runs INSERT" 'o5_clean' "$(o5_detail)"

# ── cross-cutting ───────────────────────────────────────────────────────────

# S25 — REQ-BLS-013 (Must, safety) — Decision: GS-018 is testnet-only; reading mainnet is a live-network probe nobody approved, and asserting on it risks a red gauntlet driving a mainnet action.
for tok in "$TOKEN_ROOT" "$TOKEN_DUAL" "$TOKEN_POSTAH"; do
    new_sandbox "s25_mainnet_${tok##*-}"
    for p in $RPC_PORTS; do set_network "$p" mainnet; done
    run_assert "$tok"
    ck "S25 mainnet_guard[$tok]: rc 2 (SKIP)" '[[ "$RC" -eq 2 ]]'
    ck "S25 mainnet_guard[$tok]: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'
    ck "S25 mainnet_guard[$tok]: nothing mutated" 'o5_clean' "$(o5_detail)"
done

# S26 — REQ-BLS-013 (Must) — Decision: a token silently returning 0 means a typo in the seed CSV reads as a pass on every run forever.
new_sandbox "s26_unknown_token"; run_assert "gs018-bogus-token"
ck "S26 unknown_token: rc 1" '[[ "$RC" -eq 1 ]]'

# S27 — REQ-BLS-013 (Must) — Decision: GS-018 is DEFAULT-run and observational; a confirm-var or an opt-in flag would make it skip on every unattended invocation, which is exactly the coverage it was created to provide.
if [ -f "$GS018_LIB" ] && ! grep -q 'GAUNTLET_GS018_CONFIRM' "$GS018_LIB" 2>/dev/null; then
    test_result "S27 default_run: library requires no GAUNTLET_GS018_CONFIRM" "pass" ""
else
    test_result "S27 default_run: library requires no GAUNTLET_GS018_CONFIRM" "fail" \
        "$( [ -f "$GS018_LIB" ] && echo "GS-018 is observational and part of the default run; found a confirm-var in $GS018_LIB" || echo "missing: $GS018_LIB" )"
fi
if [ -f "$GAUNTLET" ] && ! grep -qE -- '--gs018\)' "$GAUNTLET" 2>/dev/null; then
    test_result "S27 default_run: gauntlet.sh carries no --gs018 opt-in flag" "pass" ""
else
    test_result "S27 default_run: gauntlet.sh carries no --gs018 opt-in flag" "fail" \
        "an opt-in flag would make the default run skip GS-018"
fi

dispatches_token() {
    awk -v tok="$1" -v fn="$2" '
        $0 ~ /^[[:space:]]*[a-z0-9|_-]+\)[[:space:]]*$/ && index($0, tok) { armed = 1; next }
        armed { if (index($0, fn)) ok = 1; armed = 0 }
        END { exit ok ? 0 : 1 }
    ' "$GAUNTLET"
}

seed_assertions_for() {
    local sid="$1" db="$TEST_DIR/seed-check-$sid.db"
    rm -f "$db"
    if command -v sqlite3 >/dev/null 2>&1; then
        sqlite3 "$db" "CREATE TABLE gauntlet_scenarios (scenario_id TEXT PRIMARY KEY, name TEXT, description TEXT, incident_ids TEXT, assertions TEXT, scale_params TEXT, runner TEXT, status TEXT);" >/dev/null 2>&1
        sqlite3 "$db" < "$SEED_FILE" >/dev/null 2>&1
        sqlite3 "$db" "SELECT assertions FROM gauntlet_scenarios WHERE scenario_id='$sid' AND status='active';" 2>/dev/null
    else
        grep -A8 "'$sid'," "$SEED_FILE" | grep -oE "'gs[0-9]{3}-[a-z0-9,-]+'" | tr -d "'" | head -1
    fi
}

# S28 — REQ-BLS-013 (Must) — Decision: an unsourced library leaves _gs018_assert undefined and every token falls through to "unknown assertion token" while this unit suite stays green.
if grep -q 'gauntlet-gs018\.sh' "$GAUNTLET" 2>/dev/null && grep -q '\. "\$GS018_LIB"' "$GAUNTLET" 2>/dev/null; then
    test_result "S28 wiring: gauntlet.sh sources scripts/gauntlet-gs018.sh" "pass" ""
else
    test_result "S28 wiring: gauntlet.sh sources scripts/gauntlet-gs018.sh" "fail" \
        "no GS018_LIB source block in $GAUNTLET"
fi

# S29 — REQ-BLS-013 (Must) — Decision: a seeded token landing on the unknown-token arm asserts nothing while still counting as a scenario in the run summary — the 2026-09-01 GS-015 shape.
for tok in "$TOKEN_ROOT" "$TOKEN_DUAL" "$TOKEN_POSTAH"; do
    if dispatches_token "$tok" "_gs018_assert"; then
        test_result "S29 dispatch: gauntlet.sh assert() routes $tok" "pass" ""
    else
        test_result "S29 dispatch: gauntlet.sh assert() routes $tok" "fail" \
            "no case arm in $GAUNTLET dispatches $tok to _gs018_assert"
    fi
done

# S30 — REQ-BLS-013 (Must) — Decision: assert() is only ever called with tokens read from gauntlet_scenarios, so a scenario absent from the seed is dead code on every machine but this one.
SEED_018="$(seed_assertions_for GS-018)"
if [ -n "$SEED_018" ]; then
    test_result "S30 seed_registration: gauntlet-seed.sql registers GS-018 as active" "pass" ""
else
    test_result "S30 seed_registration: gauntlet-seed.sql registers GS-018 as active" "fail" \
        "no active GS-018 row parsed from $SEED_FILE"
fi
if [[ "$SEED_018" == *"$TOKEN_ROOT"* ]] && [[ "$SEED_018" == *"$TOKEN_DUAL"* ]] && [[ "$SEED_018" == *"$TOKEN_POSTAH"* ]]; then
    test_result "S30 seed_registration: assertions column lists all three tokens" "pass" ""
else
    test_result "S30 seed_registration: assertions column lists all three tokens" "fail" \
        "assertions=[$SEED_018]"
fi

# S31 — REQ-BLS-013 (Must) — Decision: gauntlet.sh sources the library unconditionally, so a syntax error there takes the whole gauntlet down, not GS-018 alone.
if [ -f "$GS018_LIB" ] && bash -n "$GS018_LIB" 2>/dev/null; then
    test_result "S31 hygiene: scripts/gauntlet-gs018.sh parses under bash -n" "pass" ""
else
    test_result "S31 hygiene: scripts/gauntlet-gs018.sh parses under bash -n" "fail" \
        "$(bash -n "$GS018_LIB" 2>&1 | head -3 || echo "missing: $GS018_LIB")"
fi

print_header "TEST SUMMARY"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo -e "  Total Tests:  $TESTS_TOTAL"
echo

if [ "$TESTS_FAILED" -eq 0 ]; then EXIT_CODE=0; else EXIT_CODE=1; fi
exit $EXIT_CODE
