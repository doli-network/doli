#!/usr/bin/env bash
# OUTPUT CONTRACT: scripts/gauntlet-gs019.sh — `_gs019_assert <token>` + `_gs019_inject`
#                  (INC-I-178, GS-019 attestation-aggregate-poisoning)
#   O1 return code   — 0 PASS · 1 FAIL · 2 SKIP. gauntlet.sh:684-689 treats 0 and 2
#                      IDENTICALLY.
#   O2 FAIL_REASONS  — caller-owned global, appended on rc 1 only.
#   O3 SKIP_REASONS  — caller-owned global, appended on rc 2 only. On this build GS-019 can
#                      ONLY ever return 2, so O3 is the entire user-visible output of the
#                      scenario: a token returning 2 without writing one reports as a green
#                      run of a scenario that never executed a single check.
#   O4 INFO_REASONS  — caller-owned global, appended on rc 0 only; unreachable today.
#   O5 SAFETY        — the stubbed curl/doli/sqlite3/pkill/launchctl log plus a keys-dir
#                      fingerprint. GS-019 is an INJECTION scenario that must inject
#                      NOTHING today: no attestation submission, no mutating RPC
#                      (sendTransaction, submitTransaction, broadcast, submitAttestation,
#                      sendAttestation, directAttestation, forceReorgTo), no signing `doli`
#                      subcommand, no pkill/launchctl, no write under ~/testnet/keys, and no
#                      INSERT into gauntlet_runs (gauntlet.sh:718-725 owns that table).
#   O6 REGISTRATION  — the gauntlet.sh source block, the `--gs019` flag arm, the inj_tag()
#                      arm, the assert() dispatch arm, and the GS-019 row in
#                      gauntlet-seed.sql. An opt-in scenario missing its inj_tag() line
#                      prints [obs] while injecting (fact sheet §8).
#   O7 OPT-IN SHAPE  — `--gs019` flag var AND `GAUNTLET_GS019_CONFIRM=1` checked INSIDE
#                      `_gs019_inject`, which writes "$WORK/gs019_injected"; `_gs019_assert`
#                      SKIPs on the missing marker. The plumbing must exist and be correct
#                      TODAY so the scenario works the day an attestation ingress exists.
#
# WHY EVERY TOKEN SKIPS (fact sheet §6, confirmed exhaustively):
#   No submitAttestation / directAttestation / sendAttestation exists anywhere in
#   crates/rpc/src/methods/. The only ingress is the libp2p gossipsub topic
#   /doli/attestations/1, which needs a Noise-encrypted transport, mesh admission, a payload
#   that deserializes as Attestation, a passing Ed25519 .verify(), AND ProducerSet
#   membership. `curl` cannot reach it, and M7 forbids new code under crates/ or bins/.
#   So GS-019 ships REGISTERED and permanently SKIPping with the reason
#   `no injection path; needs a submit RPC`. A token that returns 0 here would certify
#   poison rejection that was never attempted.
#
# LIBRARY ENV CONTRACT (this test IS the contract the developer implements against):
#   GS019 (flag, 0/1) · GAUNTLET_GS019_CONFIRM · GS019_PORTS · GS019_LOG_DIR · WORK
#
# INPUT PARTITIONS:
#   T1:  no flag, no confirm-var                       — every token O1=2, O3 set, O2/O4 empty
#   T2:  flag set, confirm-var unset                   — every token O1=2, O3 set, nothing injected
#   T3:  flag AND confirm-var set                      — every token O1=2 (never 0, never 1)
#   T4:  flag + confirm + injection marker present     — every token O1=2, reason unchanged
#   T5:  SKIP reason names the missing ingress         — O3 matches `no injection path` and `submit RPC`
#   T6:  every partition above                         — O5 clean
#   T7:  network=="mainnet"                            — O1=2, O5 clean (testnet-only guard)
#   T8:  unknown gs019-* token                         — O1=1
#   T9:  no live RPC                                   — O1=2, never O1=1
#   T10: `_gs019_inject` with neither flag nor confirm — writes no marker, O5 clean
#   T11: `_gs019_inject` with flag + confirm           — still injects nothing today, O5 clean
#   T12: library references GAUNTLET_GS019_CONFIRM and the $WORK marker   — O7
#   T13: gauntlet.sh sources scripts/gauntlet-gs019.sh                    — O6
#   T14: gauntlet.sh parses --gs019 into a flag var                       — O6/O7
#   T15: inj_tag() prints [inj] for GS-019 when the flag is set           — O6
#   T16: gauntlet.sh assert() routes all three tokens                     — O6
#   T17: gauntlet-seed.sql registers GS-019 active with all three tokens  — O6
#   T18: library parses under bash -n                                     — hygiene
# MATRIX: 7 outputs x 18 partitions (only cells a path reaches are asserted)
#   T1: O1 O2 O3 O4 | T2: O1 O3 O5 | T3: O1 O2 O3 | T4: O1 O3 | T5: O3 | T6: O5
#   T7: O1 O3 O5 | T8: O1 | T9: O1 O3 | T10: O5 O7 | T11: O5 O7 | T12: O7
#   T13: O6 | T14: O6 O7 | T15: O6 | T16: O6 | T17: O6 | T18: parse
#
# TDD RED tests for scripts/gauntlet-gs019.sh, which DOES NOT EXIST YET. `curl`, `doli`,
# `sqlite3`, `pkill` and `launchctl` are stubbed on PATH and HOME is redirected into the
# sandbox: no live testnet, no network, no chain, nothing injected anywhere.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
GS019_LIB="$PROJECT_ROOT/scripts/gauntlet-gs019.sh"
GAUNTLET="$PROJECT_ROOT/scripts/gauntlet.sh"
SEED_FILE="$PROJECT_ROOT/scripts/gauntlet-seed.sql"
TEST_DIR="/tmp/doli-gauntlet-gs019-test-$$"

TOKEN_POISON="gs019-poison-rejected"
TOKEN_LIVE="gs019-fleet-liveness-through-poison"
TOKEN_VICTIM="gs019-victim-attendance-preserved"
ALL_TOKENS="$TOKEN_POISON $TOKEN_LIVE $TOKEN_VICTIM"

RPC_PORTS="8500 8501 8502 8503 8504 8505 8506 8507 8508 8509 8510 8511 8512 8513 8514 8515 8516 8517"
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

# A missing gauntlet-gs019.sh leaves _gs019_assert undefined, RC keeps the 99 sentinel and
# the *_REASONS stay empty — vacuously satisfying "rc non-zero", "nothing injected" and
# "INFO empty". `ck` conjoins every behavioural cell with this, so a missing library reports
# as a clear FAIL rather than a bash error or a false green.
lib_ok() { [[ -f "$GS019_LIB" ]] && [[ "$FUNC_DEFINED" == "yes" ]]; }
inject_ok() { [[ -f "$GS019_LIB" ]] && [[ "$INJECT_DEFINED" == "yes" ]]; }
detail() { echo "rc=$RC func=$FUNC_DEFINED lib=$GS019_LIB fail=[$R_FAIL] skip=[$R_SKIP] info=[$R_INFO] log=$LOG_FILE"; }

ck() {
    local name="$1" cond="$2" det="${3:-}"
    if lib_ok && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "${det:-$(detail)}"; fi
}

ck_inject() {
    local name="$1" cond="$2" det="${3:-}"
    if inject_ok && eval "$cond"; then test_result "$name" "pass" ""
    else test_result "$name" "fail" "${det:-inject_defined=$INJECT_DEFINED lib=$GS019_LIB}"; fi
}

# --- stub writers ---

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
    */metrics*) [ -f "$R/$port.metrics" ] || exit 7; cat "$R/$port.metrics"; exit 0 ;;
esac
[ -f "$R/$port.up" ] || exit 7
method="$(printf '%s' "$data" | sed -n 's/.*"method"[[:space:]]*:[[:space:]]*"\([A-Za-z]*\)".*/\1/p')"
net="$(cat "$R/$port.network" 2>/dev/null || echo testnet)"
case "$method" in
    getChainInfo)
        printf '{"jsonrpc":"2.0","id":1,"result":{"bestHeight":%s,"bestHash":"050fd33e543d","bestSlot":%s,"network":"%s","version":"6.26.3"}}\n' "$BASE_H" "$BASE_H" "$net" ;;
    getNodeInfo)
        printf '{"jsonrpc":"2.0","id":1,"result":{"version":"6.26.3","network":"%s","peerId":"12D3KooWgs019","peerCount":17}}\n' "$net" ;;
    getAttestationStats)
        printf '{"jsonrpc":"2.0","id":1,"result":{"epoch":307,"epochStart":110520,"currentHeight":%s,"blocksInEpoch":316,"blocksWithAttestations":316,"blocksWithBls":0,"currentMinute":42,"producers":[]}}\n' "$BASE_H" ;;
    getProducers)
        printf '{"jsonrpc":"2.0","id":1,"result":{"producers":%s}}\n' "$(cat "$R/producers.json" 2>/dev/null || echo '[]')" ;;
    *)
        printf '{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}\n' ;;
esac
exit 0
CURL_STUB
    chmod +x "$1/curl"
}

# Any `doli` subcommand is recorded. GS-019 must never reach a signing or sending one.
write_doli_stub() {
    cat > "$1/doli" <<'DOLI_STUB'
#!/usr/bin/env bash
echo "doli $*" >> "${DOLI_LOG:?DOLI_LOG not set}"
case "$*" in
    *--version*) echo "doli 6.26.3 (c3d9e827)"; exit 0 ;;
esac
exit 0
DOLI_STUB
    chmod +x "$1/doli"
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
    WORK_DIR="$CASE_DIR/work"; BIN_DIR="$CASE_DIR/bin"; HOME_DIR="$CASE_DIR/home"
    KEYS_DIR="$HOME_DIR/testnet/keys"; LOGS_DIR="$HOME_DIR/testnet/logs"
    RPC_DIR="$CASE_DIR/rpc"
    CURL_LOG="$CASE_DIR/curl.log"; DOLI_LOG="$CASE_DIR/doli.log"
    MUTATE_LOG="$CASE_DIR/mutate.log"; LOG_FILE="$CASE_DIR/run.log"
    TEST_FLAG="0"; TEST_CONFIRM=""
    rm -rf "$CASE_DIR"
    mkdir -p "$WORK_DIR" "$BIN_DIR" "$KEYS_DIR" "$LOGS_DIR" "$RPC_DIR" "$HOME_DIR/testnet/bin"
    : > "$CURL_LOG"; : > "$DOLI_LOG"; : > "$MUTATE_LOG"; : > "$LOG_FILE"
    write_curl_stub "$BIN_DIR"; write_doli_stub "$BIN_DIR"; write_mutation_stubs "$BIN_DIR"
    echo '{"sentinel":"producer_5"}' > "$KEYS_DIR/producer_5.json"
    KEYS_FP="$(keys_fingerprint)"
    echo '[{"publicKey":"aa","status":"active","blsPubkey":"bb","bondAmount":1,"bondCount":1}]' > "$RPC_DIR/producers.json"
    for p in $RPC_PORTS; do rpc_up "$p"; done
    for p in seed n1 n2 n3 n4 n5; do echo "2026-09-04T10:00:00Z INFO Applied block h=$BASE_HEIGHT" > "$LOGS_DIR/$p.log"; done
}

keys_fingerprint() { (cd "$KEYS_DIR" 2>/dev/null && ls -1 . && cat ./* 2>/dev/null) | cksum; }
rpc_up()      { : > "$RPC_DIR/$1.up"; echo testnet > "$RPC_DIR/$1.network"; }
rpc_down()    { rm -f "$RPC_DIR/$1.up"; }
set_network() { echo "$2" > "$RPC_DIR/$1.network"; }
mark_injected() { : > "$WORK_DIR/gs019_injected"; }

# --- runners ---

_sandbox_env() {
    export PATH="$BIN_DIR:/usr/bin:/bin"
    export HOME="$HOME_DIR"
    export CURL_LOG DOLI_LOG MUTATE_LOG RPC_DIR
    export BASE_H="$BASE_HEIGHT"
    export GS019_PORTS="$RPC_PORTS" GS019_LOG_DIR="$LOGS_DIR"
    export WORK="$WORK_DIR"
    export GAUNTLET_WINDOW="${TEST_WINDOW:-45}"
    export GS019="${TEST_FLAG:-0}"
    if [ -n "${TEST_CONFIRM:-}" ]; then export GAUNTLET_GS019_CONFIRM="$TEST_CONFIRM"
    else unset GAUNTLET_GS019_CONFIRM; fi
}

run_assert() {
    local token="$1"
    (
        set +e
        unset NODECFG
        cd "$WORK_DIR" || exit 1
        _sandbox_env
        FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
        func="no"
        if [[ -f "$GS019_LIB" ]]; then
            # shellcheck disable=SC1090  # path is a test parameter, resolved at runtime
            . "$GS019_LIB" >/dev/null 2>&1
        fi
        declare -F _gs019_assert >/dev/null 2>&1 && func="yes"
        rc=99
        if [[ "$func" == "yes" ]]; then
            rc=0
            _gs019_assert "$token" < /dev/null > "$LOG_FILE" 2>&1 || rc=$?
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
}

run_inject() {
    (
        set +e
        unset NODECFG
        cd "$WORK_DIR" || exit 1
        _sandbox_env
        FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
        fn="no"
        if [[ -f "$GS019_LIB" ]]; then
            # shellcheck disable=SC1090  # path is a test parameter, resolved at runtime
            . "$GS019_LIB" >/dev/null 2>&1
        fi
        declare -F _gs019_inject >/dev/null 2>&1 && fn="yes"
        rc=99
        if [[ "$fn" == "yes" ]]; then
            rc=0
            _gs019_inject < /dev/null > "$LOG_FILE" 2>&1 || rc=$?
        fi
        printf '%s' "$fn" > "$CASE_DIR/injfn"; printf '%s' "$rc" > "$CASE_DIR/injrc"
    )
    INJECT_DEFINED="$(cat "$CASE_DIR/injfn" 2>/dev/null || echo no)"
    INJ_RC="$(cat "$CASE_DIR/injrc" 2>/dev/null || echo 99)"
}

# --- O5 predicates ---
mutating_rpc()  { grep -qEi 'sendTransaction|submitTransaction|broadcast|submitAttestation|sendAttestation|directAttestation|forceReorgTo' "$CURL_LOG" 2>/dev/null; }
signing_cli()   { grep -qEi 'send|submit|sign|add-bond|register|transfer|attest' "$DOLI_LOG" 2>/dev/null; }
touched_procs() { grep -qEi 'pkill|launchctl' "$MUTATE_LOG" 2>/dev/null; }
wrote_runs_row(){ grep -qiE 'insert[[:space:]]+into[[:space:]]+gauntlet_runs' "$MUTATE_LOG" 2>/dev/null; }
keys_touched()  { [[ "$(keys_fingerprint)" != "$KEYS_FP" ]]; }
o5_clean()      { ! mutating_rpc && ! signing_cli && ! touched_procs && ! wrote_runs_row && ! keys_touched; }
o5_detail()     { echo "curl=$(tr '\n' ' ' < "$CURL_LOG") doli=$(tr '\n' ' ' < "$DOLI_LOG") mutate=$(tr '\n' ' ' < "$MUTATE_LOG")"; }

print_header "gauntlet-gs019.sh RED tests (INC-I-178, GS-019)"
echo -e "${CYAN}Test directory: $TEST_DIR${NC}"

# T1 — REQ-BLS-013 (Must) — Decision: an opt-in scenario invoked on the default run must not evaluate anything; a rc 0 there would certify poison rejection nobody attempted.
for tok in $ALL_TOKENS; do
    new_sandbox "t1_default_${tok##*-}"; run_assert "$tok"
    ck "T1 default_run[$tok]: rc 2 (SKIP)" '[[ "$RC" -eq 2 ]]'
    ck "T1 default_run[$tok]: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'
    ck "T1 default_run[$tok]: FAIL_REASONS and INFO_REASONS empty" '[[ -z "$R_FAIL" && -z "$R_INFO" ]]'
    ck "T1 default_run[$tok]: nothing injected" 'o5_clean' "$(o5_detail)"
done

# T2 — REQ-BLS-013 (Must) — Decision: the flag alone is an operator's intent, not consent; injecting on it removes the confirm-var gate that keeps a destructive scenario off an unattended run.
for tok in $ALL_TOKENS; do
    new_sandbox "t2_flag_only_${tok##*-}"; TEST_FLAG=1; run_assert "$tok"
    ck "T2 flag_without_confirm[$tok]: rc 2 (SKIP)" '[[ "$RC" -eq 2 ]]'
    ck "T2 flag_without_confirm[$tok]: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'
    ck "T2 flag_without_confirm[$tok]: nothing injected" 'o5_clean' "$(o5_detail)"
done

# T3 — REQ-BLS-013 (Must) — Decision: fact sheet §6 proves curl cannot reach /doli/attestations/1, so a rc 0 with full consent is a claim of a rejected poison that was never sent.
for tok in $ALL_TOKENS; do
    new_sandbox "t3_full_consent_${tok##*-}"; TEST_FLAG=1; TEST_CONFIRM=1; run_assert "$tok"
    ck "T3 flag_and_confirm[$tok]: rc 2, never 0 and never 1" '[[ "$RC" -eq 2 ]]'
    ck "T3 flag_and_confirm[$tok]: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'
    ck "T3 flag_and_confirm[$tok]: FAIL_REASONS and INFO_REASONS empty" '[[ -z "$R_FAIL" && -z "$R_INFO" ]]'
    ck "T3 flag_and_confirm[$tok]: nothing injected" 'o5_clean' "$(o5_detail)"
done

# T4 — REQ-BLS-013 (Must) — Decision: the marker is written by _gs019_inject, so trusting it without an ingress lets a stale file from another scenario turn an unexecuted poison into a pass.
for tok in $ALL_TOKENS; do
    new_sandbox "t4_marker_${tok##*-}"; TEST_FLAG=1; TEST_CONFIRM=1; mark_injected; run_assert "$tok"
    ck "T4 marker_present[$tok]: rc 2 (no ingress exists to have injected through)" '[[ "$RC" -eq 2 ]]'
    ck "T4 marker_present[$tok]: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'
done

# T5 — REQ-BLS-013 (Must) — Decision: a SKIP whose reason does not name the missing ingress cannot convert into the work item that unblocks the scenario, so the gap is recorded and forgotten.
new_sandbox "t5_reason_text"; TEST_FLAG=1; TEST_CONFIRM=1; run_assert "$TOKEN_POISON"
ck "T5 skip_reason: names the absent injection path" \
   'printf "%s" "$R_SKIP" | grep -qi "no injection path"' "skip=[$R_SKIP]"
ck "T5 skip_reason: names the RPC that would unblock it" \
   'printf "%s" "$R_SKIP" | grep -qi "submit rpc"' "skip=[$R_SKIP]"

# T7 — REQ-BLS-013 (Must, safety) — Decision: GS-019 is an injection scenario; reaching mainnet at all is an unapproved live-network action on a chain carrying real value.
for tok in $ALL_TOKENS; do
    new_sandbox "t7_mainnet_${tok##*-}"; TEST_FLAG=1; TEST_CONFIRM=1
    for p in $RPC_PORTS; do set_network "$p" mainnet; done
    run_assert "$tok"
    ck "T7 mainnet_guard[$tok]: rc 2 (SKIP)" '[[ "$RC" -eq 2 ]]'
    ck "T7 mainnet_guard[$tok]: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'
    ck "T7 mainnet_guard[$tok]: nothing injected" 'o5_clean' "$(o5_detail)"
done

# T8 — REQ-BLS-013 (Must) — Decision: a token silently returning 0 means a typo in the seed CSV reads as a pass on every run forever.
new_sandbox "t8_unknown_token"; TEST_FLAG=1; TEST_CONFIRM=1; run_assert "gs019-bogus-token"
ck "T8 unknown_token: rc 1" '[[ "$RC" -eq 1 ]]'

# T9 — REQ-BLS-013 (Must) — Decision: one false FAIL is how a scenario earns a standing waiver and stops guarding anything, and an offline fleet is an environment condition.
new_sandbox "t9_no_rpc"; TEST_FLAG=1; TEST_CONFIRM=1
for p in $RPC_PORTS; do rpc_down "$p"; done
run_assert "$TOKEN_POISON"
ck "T9 no_rpc: rc 2, never 1" '[[ "$RC" -eq 2 ]]'
ck "T9 no_rpc: SKIP_REASONS non-empty" '[[ -n "$R_SKIP" ]]'

# T10 — REQ-BLS-013 (Must, safety) — Decision: an injector that writes its marker without the confirm-var makes every later assert believe a poison was delivered.
new_sandbox "t10_inject_no_consent"; run_inject
ck_inject "T10 inject_without_consent: writes no gs019_injected marker" '[[ ! -f "$WORK_DIR/gs019_injected" ]]'
ck_inject "T10 inject_without_consent: injects nothing" 'o5_clean' "$(o5_detail)"

# T11 — REQ-BLS-013 (Must, safety) — Decision: with full consent the injector must still reach no ingress today; anything it sends instead lands on a live testnet node as unreviewed traffic.
new_sandbox "t11_inject_consent"; TEST_FLAG=1; TEST_CONFIRM=1; run_inject
ck_inject "T11 inject_with_consent: sends no attestation and no mutating RPC" 'o5_clean' "$(o5_detail)"
ck_inject "T11 inject_with_consent: exits without a fatal error" '[[ "$INJ_RC" -ne 99 ]]' "inj_rc=$INJ_RC"

# T12 — REQ-BLS-013 (Must) — Decision: the confirm-var and marker plumbing must be correct the day an ingress ships; retrofitting a gate onto a scenario that already injects is how a destructive run escapes review.
if [ -f "$GS019_LIB" ] && grep -q 'GAUNTLET_GS019_CONFIRM' "$GS019_LIB" 2>/dev/null; then
    test_result "T12 opt_in_shape: library checks GAUNTLET_GS019_CONFIRM" "pass" ""
else
    test_result "T12 opt_in_shape: library checks GAUNTLET_GS019_CONFIRM" "fail" \
        "$( [ -f "$GS019_LIB" ] && echo "no confirm-var in $GS019_LIB" || echo "missing: $GS019_LIB" )"
fi
if [ -f "$GS019_LIB" ] && grep -q 'gs019_injected' "$GS019_LIB" 2>/dev/null; then
    test_result "T12 opt_in_shape: library uses the \$WORK/gs019_injected marker" "pass" ""
else
    test_result "T12 opt_in_shape: library uses the \$WORK/gs019_injected marker" "fail" \
        "$( [ -f "$GS019_LIB" ] && echo "no gs019_injected marker in $GS019_LIB" || echo "missing: $GS019_LIB" )"
fi
if [ -f "$GS019_LIB" ] && grep -q '_gs019_inject' "$GS019_LIB" 2>/dev/null; then
    test_result "T12 opt_in_shape: library defines _gs019_inject" "pass" ""
else
    test_result "T12 opt_in_shape: library defines _gs019_inject" "fail" \
        "$( [ -f "$GS019_LIB" ] && echo "no _gs019_inject in $GS019_LIB" || echo "missing: $GS019_LIB" )"
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

# T13 — REQ-BLS-013 (Must) — Decision: an unsourced library leaves _gs019_assert undefined and every token falls through to "unknown assertion token" while this unit suite stays green.
if grep -q 'gauntlet-gs019\.sh' "$GAUNTLET" 2>/dev/null && grep -q '\. "\$GS019_LIB"' "$GAUNTLET" 2>/dev/null; then
    test_result "T13 wiring: gauntlet.sh sources scripts/gauntlet-gs019.sh" "pass" ""
else
    test_result "T13 wiring: gauntlet.sh sources scripts/gauntlet-gs019.sh" "fail" \
        "no GS019_LIB source block in $GAUNTLET"
fi

# T14 — REQ-BLS-013 (Must) — Decision: without a --gs019 arm the operator has no way to arm the scenario, so the confirm-var alone would either never fire or fire unconditionally.
if grep -qE -- '--gs019\)' "$GAUNTLET" 2>/dev/null && grep -qE 'GS019=1' "$GAUNTLET" 2>/dev/null; then
    test_result "T14 opt_in_flag: gauntlet.sh parses --gs019 into GS019=1" "pass" ""
else
    test_result "T14 opt_in_flag: gauntlet.sh parses --gs019 into GS019=1" "fail" \
        "no --gs019 flag arm setting GS019=1 in $GAUNTLET"
fi

# T15 — REQ-BLS-013 (Must) — Decision: fact sheet §8 — an opt-in scenario that omits its inj_tag() line prints [obs] while injecting, so the run record misreports what was done to the fleet.
if awk '/^inj_tag\(\)/,/^}/' "$GAUNTLET" 2>/dev/null | grep -q 'GS-019'; then
    test_result "T15 inj_tag: inj_tag() prints [inj] for GS-019 when armed" "pass" ""
else
    test_result "T15 inj_tag: inj_tag() prints [inj] for GS-019 when armed" "fail" \
        "no GS-019 arm inside inj_tag() in $GAUNTLET"
fi

# T16 — REQ-BLS-013 (Must) — Decision: a seeded token landing on the unknown-token arm asserts nothing while still counting as a scenario in the run summary.
for tok in $ALL_TOKENS; do
    if dispatches_token "$tok" "_gs019_assert"; then
        test_result "T16 dispatch: gauntlet.sh assert() routes $tok" "pass" ""
    else
        test_result "T16 dispatch: gauntlet.sh assert() routes $tok" "fail" \
            "no case arm in $GAUNTLET dispatches $tok to _gs019_assert"
    fi
done

# T17 — REQ-BLS-013 (Must) — Decision: assert() is only ever called with tokens read from gauntlet_scenarios, so a scenario absent from the seed is dead code on every machine but this one.
SEED_019="$(seed_assertions_for GS-019)"
if [ -n "$SEED_019" ]; then
    test_result "T17 seed_registration: gauntlet-seed.sql registers GS-019 as active" "pass" ""
else
    test_result "T17 seed_registration: gauntlet-seed.sql registers GS-019 as active" "fail" \
        "no active GS-019 row parsed from $SEED_FILE"
fi
if [[ "$SEED_019" == *"$TOKEN_POISON"* ]] && [[ "$SEED_019" == *"$TOKEN_LIVE"* ]] && [[ "$SEED_019" == *"$TOKEN_VICTIM"* ]]; then
    test_result "T17 seed_registration: assertions column lists all three tokens" "pass" ""
else
    test_result "T17 seed_registration: assertions column lists all three tokens" "fail" \
        "assertions=[$SEED_019]"
fi

# T18 — REQ-BLS-013 (Must) — Decision: gauntlet.sh sources the library unconditionally, so a syntax error there takes the whole gauntlet down, not GS-019 alone.
if [ -f "$GS019_LIB" ] && bash -n "$GS019_LIB" 2>/dev/null; then
    test_result "T18 hygiene: scripts/gauntlet-gs019.sh parses under bash -n" "pass" ""
else
    test_result "T18 hygiene: scripts/gauntlet-gs019.sh parses under bash -n" "fail" \
        "$(bash -n "$GS019_LIB" 2>&1 | head -3 || echo "missing: $GS019_LIB")"
fi

print_header "TEST SUMMARY"
echo -e "  Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "  Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo -e "  Total Tests:  $TESTS_TOTAL"
echo

if [ "$TESTS_FAILED" -eq 0 ]; then EXIT_CODE=0; else EXIT_CODE=1; fi
exit $EXIT_CODE
