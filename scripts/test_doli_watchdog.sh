#!/usr/bin/env bash
# covers: scripts/doli-watchdog.sh
# REQ-ARCH-009 — Decision: a failure means a cold-starting node is restarted mid-start (INC-I-249 loop) or a dead node is never restarted.
# REQ-ARCH-SEC-002 — Decision: a failure means a grace value executes code, crashes the watchdog, or systemctl gets argv outside the fixed lists.
#
# OUTPUT CONTRACT: main in scripts/doli-watchdog.sh
#   O1 systemctl argv log  — restart / show / is-enabled / is-active calls
#   O2 logger messages     — SKIP, RESTART, invalid-grace, start-time-unreadable
#   O3 side effects        — no command from the grace or start value runs (sentinel file)
#   O4 exit status         — not in the interface; not asserted
# PATHS: probe ok | probe dead -> read start -> age < grace | age >= grace | start unreadable
# INPUT PARTITIONS:
#   P1 probe ok                             -> O1 no show/restart, O2 silent (case 7)
#   P2 probe dead, age < grace              -> O1 no restart, O2 SKIP (cases 1, 4)
#   P3 probe dead, age = grace / > grace    -> O1 restart, O2 RESTART (cases 2, 3, 4)
#   P4 start empty | 0 | text | future      -> O1 restart, O2 unreadable, O3 clean (case 6)
#   P5 grace unset | valid | leading zero   -> 600 | value | base-10 value (cases 1-4)
#   P6 grace invalid / injection / overflow -> 600 + one invalid line, O3 clean (case 5)
#   P7 any run                              -> O1 argv only from the fixed lists (case 8)
# Usage: bash scripts/test_doli_watchdog.sh
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WATCHDOG="$SCRIPT_DIR/doli-watchdog.sh"
UNIT="doli-mainnet-n1"
PORT=8501
NOW_S=259200
ROOT="$(mktemp -d /tmp/test_doli_watchdog.XXXXXX)"
SENTINEL="$ROOT/sentinel"
trap 'rm -rf "$ROOT"' EXIT

P=0
F=0
ERRS=""
RUN_N=0
D=""
RC=0

fail() { ERRS="${ERRS:+$ERRS; }$*"; }
begin() { ERRS=""; }
finish() {
    if [[ -z "$ERRS" ]]; then
        echo "PASS $1"
        P=$((P + 1))
    else
        echo "FAIL $1: $ERRS"
        F=$((F + 1))
    fi
}

# start value whose integer seconds are <age> before NOW_S (sub-second part included)
us_ago() { echo $(((NOW_S - $1) * 1000000 + 250000)); }

make_shims() {
    mkdir -p "$1/bin"
    cat > "$1/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
d="${0%/*}/.."
printf '%s\n' "$*" >> "$d/systemctl.log"
case "${1:-}" in
    is-enabled|is-active) [[ "${!#}" == "doli-mainnet-n1" ]] ;;
    show) printf '%s\n' "$(cat "$d/start_value")" ;;
    *) exit 0 ;;
esac
EOF
    cat > "$1/bin/curl" <<'EOF'
#!/usr/bin/env bash
d="${0%/*}/.."
printf '%s\n' "$*" >> "$d/curl.log"
if [[ "$(cat "$d/curl_mode")" == "healthy" ]]; then
    printf '%s\n' '{"jsonrpc":"2.0","result":{"bestHeight":42},"id":1}'
    exit 0
fi
exit 7
EOF
    cat > "$1/bin/logger" <<'EOF'
#!/usr/bin/env bash
d="${0%/*}/.."
if [[ $# -eq 3 && "$1" == "-t" && "$2" == "doli-watchdog" ]]; then
    printf '%s\n' "$3" >> "$d/logger.log"
else
    printf 'BADARGV %s\n' "$*" >> "$d/logger.log"
fi
EOF
    chmod +x "$1/bin/systemctl" "$1/bin/curl" "$1/bin/logger"
}

# run_wd <start_value> <dead|healthy> <unset|set> [grace]
run_wd() {
    local start="$1" curl_mode="$2" grace_mode="$3" grace="${4:-}"
    RUN_N=$((RUN_N + 1))
    D="$ROOT/run$RUN_N"
    make_shims "$D"
    : > "$D/systemctl.log"
    : > "$D/logger.log"
    : > "$D/curl.log"
    printf '%s' "$start" > "$D/start_value"
    printf '%s' "$curl_mode" > "$D/curl_mode"
    rm -f "$SENTINEL"
    # shellcheck disable=SC2016  # expanded by the child bash
    local prog='__now=$2; source "$1"; monotonic_now_s() { echo "$__now"; }; main'
    if [[ "$grace_mode" == "set" ]]; then
        PATH="$D/bin:$PATH" DOLI_WATCHDOG_COLD_START_GRACE="$grace" \
            bash -c "$prog" _ "$WATCHDOG" "$NOW_S" > "$D/out" 2>&1
    else
        env -u DOLI_WATCHDOG_COLD_START_GRACE PATH="$D/bin:$PATH" \
            bash -c "$prog" _ "$WATCHDOG" "$NOW_S" > "$D/out" 2>&1
    fi
    RC=$?
}

cnt_x() { grep -cxF -- "$2" "$D/$1"; }
cnt_f() { grep -cF -- "$2" "$D/$1"; }
cnt_e() { grep -cE -- "$2" "$D/$1"; }

# want <label> <what> <got> <-eq|-ge> <expected>
want() {
    local ok=0 w="$5"
    if [[ "$4" == "-ge" ]]; then
        w=">=$5"
        [[ "$3" -ge "$5" ]] && ok=1
    else
        [[ "$3" -eq "$5" ]] && ok=1
    fi
    [[ "$ok" -eq 1 ]] || fail "$1: $2 got $3, want $w (rc=$RC)"
}

restarts() { cnt_e systemctl.log '^restart( |$)'; }
restart_exact() { cnt_x systemctl.log "restart $UNIT"; }
skip_line() { echo "SKIP $UNIT — started $1s ago < grace $2s"; }
restart_line() { echo "RESTART $UNIT — RPC port $PORT unresponsive"; }
invalid_lines() { cnt_f logger.log "invalid DOLI_WATCHDOG_COLD_START_GRACE"; }
unreadable_lines() { grep -F -- "start time unreadable" "$D/logger.log" | grep -cF -- "$UNIT"; }

# proof the watchdog ran, and every logger call used `-t doli-watchdog <msg>`
ran() {
    if [[ "$(cnt_x systemctl.log "is-enabled --quiet $UNIT")" -lt 1 ]]; then
        fail "$1: watchdog did not run (rc=$RC: $(tail -n 1 "$D/out"))"
    fi
    want "$1" "logger calls with bad argv" "$(cnt_f logger.log BADARGV)" -eq 0
}

# expect_skip <label> <age> <grace>
expect_skip() {
    want "$1" "restart calls (restart issued)" "$(restarts)" -eq 0
    want "$1" "exact SKIP line" "$(cnt_x logger.log "$(skip_line "$2" "$3")")" -eq 1
    want "$1" "RESTART lines" "$(cnt_e logger.log '^RESTART ')" -eq 0
}

# expect_restart <label>
expect_restart() {
    want "$1" "exact 'restart $UNIT' calls" "$(restart_exact)" -eq 1
    want "$1" "restart calls" "$(restarts)" -eq 1
    want "$1" "SKIP lines" "$(cnt_e logger.log '^SKIP ')" -eq 0
}

# case 1 — REQ-ARCH-009 — Decision: a failure means a node 300 s into a cold start is restarted (the INC-I-249 loop).
begin
run_wd "$(us_ago 300)" dead unset
expect_skip age300 300 600
ran age300
want age300 "invalid-grace lines" "$(invalid_lines)" -eq 0
finish young_start_dead_rpc_no_restart

# case 2 — REQ-ARCH-009 — Decision: a failure means a node dead past the grace is no longer restarted.
begin
run_wd "$(us_ago 700)" dead unset
ran age700
expect_restart age700
want age700 "exact RESTART line" "$(cnt_x logger.log "$(restart_line)")" -eq 1
run_wd "$(us_ago 600)" dead unset
ran age600
expect_restart "age600(boundary)"
want "age600(boundary)" "exact RESTART line" "$(cnt_x logger.log "$(restart_line)")" -eq 1
finish old_start_dead_rpc_restarts

# case 3 — REQ-ARCH-009 — Decision: a failure means grace=0 cannot disable the grace for an operator.
begin
run_wd "$(us_ago 1)" dead set 0
ran grace0
expect_restart grace0
want grace0 "invalid-grace lines" "$(invalid_lines)" -eq 0
finish grace_zero_restarts_immediately

# case 4 — REQ-ARCH-009 — Decision: a failure means the unit's grace override is ignored or a leading zero crashes the watchdog.
begin
run_wd "$(us_ago 300)" dead set 200
ran grace200
expect_restart grace200
want grace200 "invalid-grace lines" "$(invalid_lines)" -eq 0
run_wd "$(us_ago 700)" dead set 900
ran grace900
expect_skip grace900 700 900
want grace900 "invalid-grace lines" "$(invalid_lines)" -eq 0
run_wd "$(us_ago 700)" dead set 0900
ran grace0900
expect_skip grace0900 700 900
want grace0900 "invalid-grace lines" "$(invalid_lines)" -eq 0
finish grace_override_honored

# case 5 — REQ-ARCH-SEC-002 — Decision: a failure means a hostile or malformed grace runs code, crashes the run, or skips the default.
begin
payloads=(
    "10; touch $SENTINEL"
    "\$(touch $SENTINEL)"
    "\`touch $SENTINEL\`"
    "BASH_VERSINFO[\$(touch $SENTINEL)]"
    "-5"
    "abc"
    ""
    "1.5"
    "99999999999999999999"
)
for p in "${payloads[@]}"; do
    lbl="grace=$(printf '%q' "${p//$SENTINEL/S}")"
    run_wd "$(us_ago 300)" dead set "$p"
    ran "$lbl"
    expect_skip "$lbl" 300 600
    want "$lbl" "invalid-grace lines" "$(invalid_lines)" -eq 1
    want "$lbl" "invalid-grace lines naming 600" \
        "$(grep -F -- "invalid DOLI_WATCHDOG_COLD_START_GRACE" "$D/logger.log" | grep -cF 600)" -eq 1
    [[ -e "$SENTINEL" ]] && fail "$lbl: sentinel exists (grace value executed a command)"
done
finish invalid_grace_uses_default

# case 6 — REQ-ARCH-009 — Decision: a failure means an unreadable start time silently skips a dead node or runs code from it.
begin
starts=("" "0" "garbage" "BASH_VERSINFO[\$(touch $SENTINEL)]" "$(((NOW_S + 100) * 1000000))")
for s in "${starts[@]}"; do
    lbl="start=$(printf '%q' "${s//$SENTINEL/S}")"
    run_wd "$s" dead unset
    ran "$lbl"
    expect_restart "$lbl"
    want "$lbl" "'start time unreadable' lines naming $UNIT" "$(unreadable_lines)" -ge 1
    [[ -e "$SENTINEL" ]] && fail "$lbl: sentinel exists (start value executed a command)"
done
finish unreadable_start_restarts

# case 7 — REQ-ARCH-009 — Decision: a failure means a healthy node is restarted, logged, or has its start time queried.
begin
run_wd "$(us_ago 300)" healthy unset
ran healthy
want healthy "is-active $UNIT calls" "$(cnt_x systemctl.log "is-active --quiet $UNIT")" -eq 1
want healthy "curl probes of port $PORT" "$(cnt_f curl.log "127.0.0.1:$PORT")" -ge 1
want healthy "restart calls" "$(restarts)" -eq 0
want healthy "show calls" "$(cnt_e systemctl.log '^show( |$)')" -eq 0
want healthy "SKIP/RESTART lines" "$(cnt_e logger.log '^(SKIP|RESTART) ')" -eq 0
want healthy "unreadable lines" "$(cnt_f logger.log "start time unreadable")" -eq 0
finish healthy_rpc_no_action

# case 8 — REQ-ARCH-SEC-002 — Decision: a failure means systemctl received a unit or argument outside the script's fixed lists.
begin
ALLOWED="$ROOT/allowed"
{
    for n in $(seq 1 12); do
        echo "is-enabled --quiet doli-mainnet-n$n"
        echo "is-enabled --quiet doli-testnet-nt$n"
    done
    for u in doli-mainnet-seed doli-testnet-seed doli-mainnet-santiago doli-mainnet-ivan; do
        echo "is-enabled --quiet $u"
    done
    echo "is-active --quiet $UNIT"
    echo "show -p ActiveEnterTimestampMonotonic --value $UNIT"
    echo "restart $UNIT"
} > "$ALLOWED"
D="$ROOT/run2"
want case2 "exact 'restart $UNIT' calls" "$(restart_exact)" -eq 1
for log in "$ROOT"/run*/systemctl.log; do
    run="$(basename "$(dirname "$log")")"
    bad="$(grep -vxF -f "$ALLOWED" "$log" | head -n 3 | tr '\n' '|')"
    [[ -n "$bad" ]] && fail "$run: argv outside fixed lists: $bad"
done
finish systemctl_argv_exact

echo "$P passed, $F failed"
[[ "$F" -eq 0 ]] || exit 1
