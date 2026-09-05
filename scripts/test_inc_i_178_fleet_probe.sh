#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/inc-i-178-fleet-probe.sh (INC-I-178 M7 outcome-metric probe)
#   P1 exit code     — ALWAYS 0. The probe is run BEFORE and AFTER the deploy, and the
#                      "before" run legitimately counts 0 nodes. A non-zero exit on an
#                      all-zero or all-unreachable fleet turns the before-measurement into
#                      a failure and destroys the before/after pair the milestone needs.
#   P2 final line    — the last non-empty stdout line ends in a BARE integer, so the caller
#                      can take it with `awk '{print $NF}'` with no parsing.
#   P3 the count     — the number of the 18 nodes whose http://127.0.0.1:<9000+i>/metrics
#                      body contains the exact series name `doli_attestation_verify_total`.
#                      Fact sheet §3: versions are byte-identical between the old and the
#                      INC-I-178 build (both report 6.26.3), so the metrics surface is the
#                      ONLY capability marker, and REGISTRATION is the signal — a node
#                      exposing the series at value 0 IS on the new build and MUST count.
#   P4 SAFETY        — the stubbed curl/sqlite3/pkill/launchctl log plus a keys-dir
#                      fingerprint: no mutating RPC, no pkill/launchctl, no write under
#                      ~/testnet/keys, no INSERT into any .omega/memory.db table.
#   P5 hygiene       — the file exists, is executable, and parses under bash -n.
#
# SCAN SET (fact sheet §1): 18 launchd services — seed plus n1..n17 — metrics 9000 + 9000+i.
# The probe reads /metrics only; it never touches the RPC ports and never reads the chain.
#
# LIBRARY ENV CONTRACT (this test IS the contract the developer implements against):
#   PROBE_METRICS_PORTS — space-separated override of the 9000..9017 scan set
#
# INPUT PARTITIONS:
#   F1:  0 of 18 carry the series (the measured pre-deploy fleet) — P1=0, count 0
#   F2:  18 of 18 carry it (the post-deploy target)               — P1=0, count 18
#   F3:  4 of 18 carry it (a partial rolling restart)             — P1=0, count 4
#   F4:  a node exposing ONLY doli_attestation_verify_rejected_total — not counted
#        (a substring match on `doli_attestation_verify` over-counts)
#   F5:  a node exposing doli_attestation_verify_total at VALUE 0  — COUNTED
#        (a value-based predicate reports 0/18 on a fully deployed fleet)
#   F6:  6 metrics ports refuse the connection (curl exit 7)       — P1=0, they do not count
#   F7:  every metrics port refuses                                — P1=0, count 0
#   F8:  a port answers with garbage/HTML                          — P1=0, not counted
#   F9:  the scan covers all 18 metrics ports, 9000 through 9017   — one request each
#   F10: the probe never requests an RPC port (8500-8517)          — P4
#   F11: the final line's last field is purely digits              — P2
#   F12: two consecutive runs report the same count                — read-only/idempotent
#   F13: probe is read-only                                        — P4
#   F14: file exists, is executable, parses under bash -n          — P5
# MATRIX: 5 outputs x 14 partitions (only cells a path reaches are asserted)
#   F1: P1 P2 P3 | F2: P1 P3 | F3: P1 P3 | F4: P3 | F5: P1 P3 | F6: P1 P3 | F7: P1 P2 P3
#   F8: P1 P3 | F9: P4 | F10: P4 | F11: P2 | F12: P3 | F13: P4 | F14: P5
#
# TDD RED tests for scripts/inc-i-178-fleet-probe.sh, which DOES NOT EXIST YET. `curl`,
# `sqlite3`, `pkill` and `launchctl` are stubbed on PATH and HOME is redirected into the
# sandbox: no live testnet, no network, no chain, no node touched.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
PROBE="$PROJECT_ROOT/scripts/inc-i-178-fleet-probe.sh"
TEST_DIR="/tmp/doli-fleet-probe-test-$$"

MET_PORTS="9000 9001 9002 9003 9004 9005 9006 9007 9008 9009 9010 9011 9012 9013 9014 9015 9016 9017"
RPC_PORTS="8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517"

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

# A missing probe leaves EXIT at the 99 sentinel and COUNT at the empty string, which would
# vacuously satisfy "no mutating RPC" and "count is not 18". Every behavioural cell conjoins
# probe_ok, so a missing probe reports as a clear FAIL rather than a bash error.
probe_ok() { [[ -f "$PROBE" ]] && [[ "$RAN" == "yes" ]]; }
detail() { echo "exit=$EXIT count=[$COUNT] last=[$LAST_LINE] probe=$PROBE out=$OUT_FILE"; }

ck() {
    local name="$1" cond="$2" det="${3:-}"
    if probe_ok && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "${det:-$(detail)}"; fi
}

# --- stub writers ---

write_curl_stub() {
    cat > "$1/curl" <<'CURL_STUB'
#!/usr/bin/env bash
echo "curl $*" >> "${CURL_LOG:?CURL_LOG not set}"
url=""
for a in "$@"; do case "$a" in http://*) url="$a" ;; esac; done
hostport="${url#http://}"; hostport="${hostport%%/*}"; port="${hostport##*:}"
R="${RPC_DIR:?RPC_DIR not set}"
[ -f "$R/$port.metrics" ] || exit 7
cat "$R/$port.metrics"
exit 0
CURL_STUB
    chmod +x "$1/curl"
}

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

# --- fixtures ---

new_sandbox() {
    local name="$1" p
    CASE_DIR="$TEST_DIR/$name"
    BIN_DIR="$CASE_DIR/bin"; HOME_DIR="$CASE_DIR/home"
    KEYS_DIR="$HOME_DIR/testnet/keys"; RPC_DIR="$CASE_DIR/rpc"
    CURL_LOG="$CASE_DIR/curl.log"; MUTATE_LOG="$CASE_DIR/mutate.log"
    OUT_FILE="$CASE_DIR/stdout.txt"; ERR_FILE="$CASE_DIR/stderr.txt"
    rm -rf "$CASE_DIR"
    mkdir -p "$BIN_DIR" "$KEYS_DIR" "$RPC_DIR" "$HOME_DIR/testnet/logs" "$HOME_DIR/testnet/bin"
    : > "$CURL_LOG"; : > "$MUTATE_LOG"
    write_curl_stub "$BIN_DIR"; write_mutation_stubs "$BIN_DIR"
    echo '{"sentinel":"producer_5"}' > "$KEYS_DIR/producer_5.json"
    KEYS_FP="$(keys_fingerprint)"
    for p in $MET_PORTS; do metrics_old "$p"; done
}

keys_fingerprint() { (cd "$KEYS_DIR" 2>/dev/null && ls -1 . && cat ./* 2>/dev/null) | cksum; }

# The OLD build's entire doli_attestation_* surface, measured live on n2:9002 (fact sheet §3).
metrics_old() {
    cat > "$RPC_DIR/$1.metrics" <<'MET'
# HELP doli_attestation_misses_total Attestation misses
# TYPE doli_attestation_misses_total counter
doli_attestation_misses_total 12
doli_attestation_missing_current 0
doli_chain_height 110836
MET
}

# The INC-I-178 build. $2 overrides the value; the DEFAULT is 0 because registration, not
# value, is the capability signal pre-AH.
metrics_new() {
    metrics_old "$1"
    cat >> "$RPC_DIR/$1.metrics" <<MET
# TYPE doli_attestation_verify_total counter
doli_attestation_verify_total ${2:-0}
doli_attestation_verify_rejected_total{reason="root_mismatch"} 0
doli_attestation_verify_skipped_light_total 0
doli_attestation_bitfield_fill_ratio 0.83
MET
}

# The substring trap: `_rejected_total` contains `doli_attestation_verify` but is NOT the
# series the capability predicate names.
metrics_rejected_only() {
    metrics_old "$1"
    cat >> "$RPC_DIR/$1.metrics" <<'MET'
doli_attestation_verify_rejected_total{reason="missing_bls_key"} 0
MET
}

metrics_garbage() { printf '<html><body>502 Bad Gateway</body></html>\n' > "$RPC_DIR/$1.metrics"; }
metrics_down()    { rm -f "$RPC_DIR/$1.metrics"; }

# --- runner ---
run_probe() {
    (
        set +e
        cd "$CASE_DIR" || exit 1
        export PATH="$BIN_DIR:/usr/bin:/bin"
        export HOME="$HOME_DIR"
        export CURL_LOG MUTATE_LOG RPC_DIR
        export PROBE_METRICS_PORTS="$MET_PORTS"
        rc=99
        if [[ -x "$PROBE" ]]; then
            rc=0
            bash "$PROBE" < /dev/null > "$OUT_FILE" 2> "$ERR_FILE" || rc=$?
        elif [[ -f "$PROBE" ]]; then
            rc=0
            bash "$PROBE" < /dev/null > "$OUT_FILE" 2> "$ERR_FILE" || rc=$?
        fi
        printf '%s' "$rc" > "$CASE_DIR/rc"
    )
    if [[ -f "$PROBE" ]]; then RAN="yes"; else RAN="no"; fi
    EXIT="$(cat "$CASE_DIR/rc" 2>/dev/null || echo 99)"
    LAST_LINE="$(grep -v '^[[:space:]]*$' "$OUT_FILE" 2>/dev/null | tail -1)"
    COUNT="$(printf '%s' "$LAST_LINE" | awk '{print $NF}')"
}

# --- P4 predicates ---
mutating_rpc()   { grep -qEi 'sendTransaction|submitTransaction|broadcast|forceReorgTo' "$CURL_LOG" 2>/dev/null; }
touched_rpc_port() { grep -qE '127\.0\.0\.1:85[0-1][0-9]' "$CURL_LOG" 2>/dev/null; }
touched_procs()  { grep -qEi 'pkill|launchctl' "$MUTATE_LOG" 2>/dev/null; }
wrote_db()       { grep -qiE 'insert[[:space:]]+into' "$MUTATE_LOG" 2>/dev/null; }
keys_touched()   { [[ "$(keys_fingerprint)" != "$KEYS_FP" ]]; }
p4_clean()       { ! mutating_rpc && ! touched_procs && ! wrote_db && ! keys_touched; }
p4_detail()      { echo "curl=$(tr '\n' ' ' < "$CURL_LOG") mutate=$(tr '\n' ' ' < "$MUTATE_LOG")"; }
is_int()         { [[ "$1" =~ ^[0-9]+$ ]]; }
scanned_port()   { grep -qE "127\.0\.0\.1:$1(/|[^0-9]|\$)" "$CURL_LOG" 2>/dev/null; }

print_header "inc-i-178-fleet-probe.sh RED tests (INC-I-178 M7 outcome metric)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# F1 — REQ-BLS-006 (Must) — Decision: the pre-deploy run is the BEFORE half of the milestone's outcome metric; if a zero count exits non-zero, the deploy has no baseline to be measured against.
new_sandbox "f1_zero_of_18"; run_probe
ck "F1 pre_deploy_fleet: exit 0 with a zero count" '[[ "$EXIT" -eq 0 ]]'
ck "F1 pre_deploy_fleet: final line ends in a bare integer" 'is_int "$COUNT"' "last=[$LAST_LINE]"
ck "F1 pre_deploy_fleet: count is 0" '[[ "$COUNT" == "0" ]]' "last=[$LAST_LINE]"

# F13 — REQ-BLS-006 (Must, safety) — Decision: this probe runs against the live 18-node testnet immediately before a rolling restart; any write it performs lands on a fleet mid-deploy.
ck "F13 probe_readonly: no mutating RPC, no pkill/launchctl, no keys write, no DB INSERT" 'p4_clean' "$(p4_detail)"

# F10 — REQ-BLS-006 (Should) — Decision: the capability marker lives on /metrics; querying RPC instead reads the chain and, on a node mid-restart, blocks the probe behind an archiver scan.
ck "F10 probe_scope: never requests an RPC port (8500-8517)" '! touched_rpc_port' "$(p4_detail)"

# F9 — REQ-BLS-006 (Must) — Decision: the fleet is 18 nodes (fact sheet §1); a probe stopping at n12, as NODECFG does, silently reports at most 13 and understates the deploy.
new_sandbox "f9_scan_set"; run_probe
ck "F9 scan_set: requests metrics port 9000 (seed)" 'scanned_port 9000' "$(p4_detail)"
ck "F9 scan_set: requests metrics port 9012" 'scanned_port 9012' "$(p4_detail)"
ck "F9 scan_set: requests metrics port 9017 (n17)" 'scanned_port 9017' "$(p4_detail)"
ck "F9 scan_set: requests all 18 metrics ports" \
   '[[ "$(grep -oE "127\.0\.0\.1:90[0-1][0-9]" "$CURL_LOG" | sort -u | wc -l | tr -d " ")" -eq 18 ]]' \
   "$(p4_detail)"

# F2 — REQ-BLS-006 (Must) — Decision: 18 is the AFTER value that proves the deploy landed; a probe that cannot reach it leaves the milestone with no observable outcome.
new_sandbox "f2_all_18"; for p in $MET_PORTS; do metrics_new "$p"; done
run_probe
ck "F2 post_deploy_fleet: exit 0" '[[ "$EXIT" -eq 0 ]]'
ck "F2 post_deploy_fleet: count is 18" '[[ "$COUNT" == "18" ]]' "last=[$LAST_LINE]"

# F5 — REQ-BLS-006 (Must) — Decision: fact sheet §3 — pre-AH the counter reads 0 on every new-build node, so a value-based predicate reports 0/18 on a fully successful deploy.
new_sandbox "f5_value_zero"; for p in $MET_PORTS; do metrics_new "$p" 0; done
run_probe
ck "F5 registration_not_value: a series at value 0 still counts" '[[ "$COUNT" == "18" ]]' "last=[$LAST_LINE]"
ck "F5 registration_not_value: exit 0" '[[ "$EXIT" -eq 0 ]]'

# F3 — REQ-BLS-006 (Must) — Decision: the rolling restart lands node by node, so a probe that cannot report a partial count cannot tell a stalled deploy from a finished one.
new_sandbox "f3_four_of_18"; for p in 9000 9003 9009 9017; do metrics_new "$p"; done
run_probe
ck "F3 partial_deploy: exit 0" '[[ "$EXIT" -eq 0 ]]'
ck "F3 partial_deploy: count is 4" '[[ "$COUNT" == "4" ]]' "last=[$LAST_LINE]"

# F4 — REQ-BLS-006 (Must) — Decision: doli_attestation_verify_rejected_total contains the string doli_attestation_verify, so a substring predicate counts an old node as new and reports a deploy that never happened.
new_sandbox "f4_rejected_only"
for p in $MET_PORTS; do metrics_rejected_only "$p"; done
metrics_new 9001
run_probe
ck "F4 substring_trap: only the node exposing verify_total counts" '[[ "$COUNT" == "1" ]]' "last=[$LAST_LINE]"

# F6 — REQ-BLS-006 (Must) — Decision: during a rolling restart several nodes are down by design; a probe that aborts on the first refused connection cannot be run during the deploy it measures.
new_sandbox "f6_some_down"
for p in $MET_PORTS; do metrics_new "$p"; done
for p in 9002 9004 9006 9008 9010 9012; do metrics_down "$p"; done
run_probe
ck "F6 partial_fleet_down: exit 0" '[[ "$EXIT" -eq 0 ]]'
ck "F6 partial_fleet_down: count is 12" '[[ "$COUNT" == "12" ]]' "last=[$LAST_LINE]"

# F7 — REQ-BLS-006 (Must) — Decision: a stopped fleet is an environment condition, and a non-zero exit there makes the probe unusable as the deploy's before/after instrument.
new_sandbox "f7_all_down"; for p in $MET_PORTS; do metrics_down "$p"; done
run_probe
ck "F7 fleet_down: exit 0" '[[ "$EXIT" -eq 0 ]]'
ck "F7 fleet_down: final line ends in a bare integer" 'is_int "$COUNT"' "last=[$LAST_LINE]"
ck "F7 fleet_down: count is 0" '[[ "$COUNT" == "0" ]]' "last=[$LAST_LINE]"

# F8 — REQ-BLS-006 (Should) — Decision: a node answering HTML mid-restart is not a node on the new build; counting it inflates the AFTER value and fakes a completed deploy.
new_sandbox "f8_garbage"
for p in $MET_PORTS; do metrics_new "$p"; done
for p in 9005 9011; do metrics_garbage "$p"; done
run_probe
ck "F8 garbage_response: exit 0" '[[ "$EXIT" -eq 0 ]]'
ck "F8 garbage_response: count is 16" '[[ "$COUNT" == "16" ]]' "last=[$LAST_LINE]"

# F11 — REQ-BLS-006 (Must) — Decision: the caller takes the count with awk '{print $NF}'; a trailing unit, colon or percent sign silently yields a non-numeric before/after pair.
new_sandbox "f11_final_line"; for p in 9000 9001 9002; do metrics_new "$p"; done
run_probe
ck "F11 final_line: last field is purely digits" 'is_int "$COUNT"' "last=[$LAST_LINE]"
ck "F11 final_line: last field equals the count 3" '[[ "$COUNT" == "3" ]]' "last=[$LAST_LINE]"
ck "F11 final_line: stdout is non-empty" '[[ -n "$LAST_LINE" ]]'

# F12 — REQ-BLS-009 (Should) — Decision: the before/after pair is only meaningful if the probe is repeatable; a probe whose count drifts between back-to-back runs cannot bound anything.
new_sandbox "f12_idempotent"; for p in 9000 9001 9002 9003 9004; do metrics_new "$p"; done
run_probe; COUNT_A="$COUNT"; EXIT_A="$EXIT"
run_probe
ck "F12 idempotent: same count across two consecutive runs" '[[ "$COUNT_A" == "$COUNT" ]]' "first=$COUNT_A second=$COUNT"
ck "F12 idempotent: both runs exit 0" '[[ "$EXIT_A" -eq 0 && "$EXIT" -eq 0 ]]'
ck "F12 idempotent: count is 5" '[[ "$COUNT" == "5" ]]' "last=[$LAST_LINE]"
ck "F12 idempotent: nothing mutated across both runs" 'p4_clean' "$(p4_detail)"

# F14 — REQ-BLS-006 (Must) — Decision: the deploy runbook invokes the probe directly; a non-executable or unparseable file fails at the moment the baseline must be captured.
if [ -f "$PROBE" ]; then
    test_result "F14 hygiene: scripts/inc-i-178-fleet-probe.sh exists" "pass" ""
else
    test_result "F14 hygiene: scripts/inc-i-178-fleet-probe.sh exists" "fail" "missing: $PROBE"
fi
if [ -x "$PROBE" ]; then
    test_result "F14 hygiene: probe is executable" "pass" ""
else
    test_result "F14 hygiene: probe is executable" "fail" "not executable: $PROBE"
fi
if [ -f "$PROBE" ] && bash -n "$PROBE" 2>/dev/null; then
    test_result "F14 hygiene: probe parses under bash -n" "pass" ""
else
    test_result "F14 hygiene: probe parses under bash -n" "fail" \
        "$(bash -n "$PROBE" 2>&1 | head -3 || echo "missing: $PROBE")"
fi

print_header "TEST SUMMARY"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo -e "  Total Tests:  $TESTS_TOTAL"
echo

if [ "$TESTS_FAILED" -eq 0 ]; then EXIT_CODE=0; else EXIT_CODE=1; fi
exit $EXIT_CODE
