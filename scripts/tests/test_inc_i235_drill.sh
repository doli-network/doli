#!/usr/bin/env bash
#
# INC-I-235 M3 — hermetic contract tests for the live-testnet fork-walk drill.
# Requirement: REQ-I235-014 (Could). No live network is touched: HOME and
# TESTNET_DIR point at a temp tree, and launchctl/curl/codesign/cp/kill/pkill/ssh
# are PATH shims (kill also as an exported bash function) that append to a log.
#
# OUTPUT CONTRACT:
#   O1 exit code   O2 stdout+stderr text   O3 shimmed tool/RPC call log
#   (the drill's only persistent side effects go through O3's shims)
# PATHS: PH help | PD dry-run plan | PR preflight refusal | PG gs022 unconfirmed
# INPUT PARTITIONS:
#   PH: --help                                   -> O1=0, O2 documents, O3 clean
#   PD: --target n3 (default / explicit 127.0.0.1:8500) -> O1=0, O2 plan, O3 clean
#   PR: target seed | unknown | empty | path/injection;
#       host non-loopback; port base 8400/8900/30200 -> O1!=0, O2 "refus", O3 clean
#   PG: GAUNTLET_GS022_CONFIRM unset            -> O1=0, O2 names var, O3 no drill
# MATRIX: 4 paths x 3 outputs, every cell asserted below.
#
# INTERFACE the developer must satisfy:
#   scripts/inc-i-235-drill.sh   (executable, bash)
#     --help                 exit 0; text names --target, --dry-run, the evidence
#                            captured: RECOVERY_START, RECOVERY_DONE, SnapSync
#                            (absence), and the RPC used (getChainInfo).
#     --target nN            one NON-seed node; must exist as "$TESTNET_DIR/nN"
#                            (TESTNET_DIR defaults to $HOME/testnet).
#                            "seed", unknown names and path-like values are
#                            refused: non-zero exit, message containing "refus".
#     --dry-run              print planned steps (naming the target, "dry-run",
#                            RECOVERY_DONE); exit 0; no launchctl/kill/pkill/
#                            codesign/cp and no mutating RPC.
#     env DRILL_RPC_HOST     default 127.0.0.1; any non-loopback value refused.
#     env DRILL_RPC_BASE     default 8500; 8400 / 8900 / 30200 (mainnet) refused.
#     Preflight guards (target, host, port) run BEFORE dry-run planning.
#   scripts/gauntlet-gs022.sh    (executable, bash -n clean, sourced library)
#     defines gs022_inject(); without GAUNTLET_GS022_CONFIRM=1 it prints a
#     message naming GAUNTLET_GS022_CONFIRM, returns 0, and never invokes
#     "$ROOT/scripts/inc-i-235-drill.sh".
#   scripts/gauntlet.sh          accepts --gs022, sources
#     "$ROOT/scripts/gauntlet-gs022.sh", calls gs022_inject.
#   Neither new script contains (outside full-line comments): pkill, kill,
#   rm -rf, chain-reset, --gs009, --gs010, --chaos, ssh.

set -u

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DRILL="$REPO_ROOT/scripts/inc-i-235-drill.sh"
GS022="$REPO_ROOT/scripts/gauntlet-gs022.sh"
GAUNTLET="$REPO_ROOT/scripts/gauntlet.sh"

unset DRILL_RPC_HOST DRILL_RPC_BASE GAUNTLET_GS022_CONFIRM

PASS_COUNT=0
FAIL_COUNT=0

pass() { printf "    PASS  %s\n" "$1"; PASS_COUNT=$((PASS_COUNT + 1)); }
fail() { printf "    FAIL  %s — %s\n" "$1" "$2"; FAIL_COUNT=$((FAIL_COUNT + 1)); }
check() { if [ "$2" = "0" ]; then pass "$1"; else fail "$1" "${3:-assertion false}"; fi; }
ok_if() { if "$@"; then echo 0; else echo 1; fi; }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/inc-i235-drill-test.XXXXXX")"
trap '/bin/rm -rf "$TMP"' EXIT

FAKE_HOME="$TMP/home"
FAKE_TESTNET="$FAKE_HOME/testnet"
SHIM="$TMP/shim"
CALL_LOG="$TMP/calls.log"
OUT="$TMP/out.txt"
/bin/mkdir -p "$SHIM" "$FAKE_TESTNET/logs" "$FAKE_TESTNET/seed" \
    "$FAKE_TESTNET/n1" "$FAKE_TESTNET/n2" "$FAKE_TESTNET/n3"
: > "$FAKE_TESTNET/logs/n3.log"

for tool in launchctl codesign cp kill pkill ssh; do
    printf '#!/bin/sh\necho "%s $*" >> "$CALL_LOG"\nexit 0\n' "$tool" > "$SHIM/$tool"
    /bin/chmod +x "$SHIM/$tool"
done
cat > "$SHIM/curl" <<'EOF'
#!/bin/sh
echo "curl $*" >> "$CALL_LOG"
echo '{"jsonrpc":"2.0","id":1,"result":{"network":"testnet","bestHeight":100,"bestHash":"aa","bestSlot":100}}'
exit 0
EOF
/bin/chmod +x "$SHIM/curl"

export CALL_LOG

# run_drill ARGS... — runs the drill hermetically; sets RC, output in $OUT.
run_drill() {
    : > "$CALL_LOG"
    (
        export PATH="$SHIM:$PATH" HOME="$FAKE_HOME" TESTNET_DIR="$FAKE_TESTNET"
        kill() { echo "kill $*" >> "$CALL_LOG"; }
        export -f kill
        bash "$DRILL" "$@"
    ) > "$OUT" 2>&1 < /dev/null
    RC=$?
}

out_has() { grep -qiE -- "$1" "$OUT"; }
log_has() { grep -qE -- "$1" "$CALL_LOG"; }
calls() { tr '\n' '|' < "$CALL_LOG"; }
snippet() { echo "rc=$RC out=$(head -c 200 "$OUT")"; }

MUTATING_TOOLS='^(launchctl|kill|pkill|codesign|cp|ssh) '
MUTATING_RPC='pauseProduction|resumeProduction|forceReorgTo|backfillFromPeer|createCheckpoint|rollback|sendTransaction|submitTransaction'

# no_mutation NAME — asserts the last run invoked no mutating tool or RPC.
no_mutation() {
    local m=0
    log_has "$MUTATING_TOOLS" && m=1
    log_has "$MUTATING_RPC" && m=1
    check "$1" "$m" "mutating call recorded: $(calls)"
}

# refused NAME — asserts the last run exited non-zero with a refusal message.
refused() {
    local r=0
    [ "$RC" != 0 ] || r=1
    out_has 'refus' || r=1
    check "$1" "$r" "$(snippet)"
}

# code_only FILE — strips full-line comments.
code_only() { sed -e 's/^[[:space:]]*#.*$//' "$1"; }

FORBIDDEN='(^|[^[:alnum:]_-])p?kill([^[:alnum:]_-]|$)|rm[[:space:]]+-[a-zA-Z]*r[a-zA-Z]*f|rm[[:space:]]+-[a-zA-Z]*f[a-zA-Z]*r|chain-reset|--gs009|--gs010|--chaos|(^|[^[:alnum:]_-])ssh([^[:alnum:]_-]|$)'

# forbidden_scan NAME FILE — asserts FILE code has no destructive tokens.
forbidden_scan() {
    local name="$1" file="$2" hits
    if [ ! -f "$file" ]; then fail "$name" "$file missing"; return; fi
    hits="$(code_only "$file" | grep -nE -- "$FORBIDDEN")"
    if [ -z "$hits" ]; then pass "$name"; else fail "$name" "forbidden: $hits"; fi
}

echo "INC-I-235 M3 drill contract tests (REQ-I235-014)"

# --- Existence / syntax ---------------------------------------------------

# TEST-I235-M3-01 REQ-I235-014 — Decision: a missing/non-executable drill means the live evidence cannot be produced at all.
check "TEST-I235-M3-01 drill exists and is executable" "$(ok_if test -x "$DRILL")" "$DRILL"

# TEST-I235-M3-02 REQ-I235-014 — Decision: a missing gs022 library means the gauntlet silently skips the scenario.
check "TEST-I235-M3-02 gs022 library exists and is executable" "$(ok_if test -x "$GS022")" "$GS022"

# TEST-I235-M3-03 REQ-I235-014 — Decision: a syntax error would abort the drill mid-run on a live node.
if [ -f "$DRILL" ]; then bash -n "$DRILL" 2>"$OUT"; r=$?; else r=1; fi
check "TEST-I235-M3-03 drill passes bash -n" "$r" "$(cat "$OUT" 2>/dev/null)"

# TEST-I235-M3-04 REQ-I235-014 — Decision: a syntax error in a sourced lib aborts the whole gauntlet run.
if [ -f "$GS022" ]; then bash -n "$GS022" 2>"$OUT"; r=$?; else r=1; fi
check "TEST-I235-M3-04 gs022 passes bash -n" "$r" "$(cat "$OUT" 2>/dev/null)"

# --- Help -----------------------------------------------------------------

# TEST-I235-M3-05 REQ-I235-014 — Decision: an operator must be able to read the contract without side effects.
run_drill --help
check "TEST-I235-M3-05 --help exits 0" "$(ok_if test "$RC" = 0)" "$(snippet)"
no_mutation "TEST-I235-M3-05b --help performs no mutation"

# TEST-I235-M3-06 REQ-I235-014 — Decision: undocumented flags/evidence means operators cannot tell what proof the drill gathers.
r=0; miss=""
for tok in '--target' '--dry-run' 'RECOVERY_START' 'RECOVERY_DONE' 'snap ?sync' 'getChainInfo'; do
    out_has "$tok" || { r=1; miss="$miss $tok"; }
done
check "TEST-I235-M3-06 --help documents flags and captured evidence" "$r" "missing:$miss"

# --- Dry run --------------------------------------------------------------

# TEST-I235-M3-07 REQ-I235-014 — Decision: a dry-run that fails or prints no plan cannot be reviewed before a live run.
run_drill --dry-run --target n3
r=0; [ "$RC" = 0 ] || r=1
for tok in 'n3' 'dry-run' 'RECOVERY_DONE'; do out_has "$tok" || r=1; done
check "TEST-I235-M3-07 --dry-run --target n3 exits 0 and prints the plan" "$r" "$(snippet)"

# TEST-I235-M3-08 REQ-I235-014 — Decision: a dry-run that restarts/kills a node is a live perturbation in disguise.
check "TEST-I235-M3-08 dry-run invokes no launchctl/kill/pkill" \
    "$(if log_has '^(launchctl|kill|pkill) '; then echo 1; else echo 0; fi)" "$(calls)"

# TEST-I235-M3-09 REQ-I235-014 — Decision: a dry-run that sends a mutating RPC changes live chain/production state.
no_mutation "TEST-I235-M3-09 dry-run sends no mutating RPC and copies/signs nothing"

# --- Target refusal -------------------------------------------------------

# TEST-I235-M3-10 REQ-I235-014 — Decision: forking the seed would take down sync for every other testnet node.
run_drill --dry-run --target seed
refused "TEST-I235-M3-10 --target seed refused with message"
no_mutation "TEST-I235-M3-10b seed refusal performs no mutation"

# TEST-I235-M3-11 REQ-I235-014 — Decision: an unknown or empty target would act on a wrong/default node.
for t in n99 bogus ""; do
    run_drill --dry-run --target "$t"
    refused "TEST-I235-M3-11 --target '$t' refused with message"
done

# TEST-I235-M3-12 REQ-I235-014 — Decision: a path-like target escapes TESTNET_DIR and could address an arbitrary data dir.
for t in "../n3" "n3/../seed" "n3;id" '$(whoami)'; do
    run_drill --dry-run --target "$t"
    refused "TEST-I235-M3-12 --target '$t' (path/injection) refused"
done
no_mutation "TEST-I235-M3-12b injection refusals perform no mutation"

# --- Network guard --------------------------------------------------------

# TEST-I235-M3-13 REQ-I235-014 — Decision: a non-loopback RPC host may be a mainnet machine.
for h in 10.0.0.5 example.com 0.0.0.0; do
    DRILL_RPC_HOST="$h" run_drill --dry-run --target n3
    refused "TEST-I235-M3-13 DRILL_RPC_HOST=$h refused"
done
no_mutation "TEST-I235-M3-13b host refusals perform no mutation"

# TEST-I235-M3-14 REQ-I235-014 — Decision: a mainnet port base on loopback would drill a co-located mainnet node.
for p in 8400 8900 30200; do
    DRILL_RPC_BASE="$p" run_drill --dry-run --target n3
    refused "TEST-I235-M3-14 DRILL_RPC_BASE=$p refused"
done

# TEST-I235-M3-15 REQ-I235-014 — Decision: guards that reject the loopback testnet config would make the drill unusable.
DRILL_RPC_HOST=127.0.0.1 DRILL_RPC_BASE=8500 run_drill --dry-run --target n3
check "TEST-I235-M3-15 explicit 127.0.0.1:8500 accepted in dry-run" "$(ok_if test "$RC" = 0)" "$(snippet)"

# --- Static safety --------------------------------------------------------

# TEST-I235-M3-16 REQ-I235-014 — Decision: a destructive token in the drill can wipe data or split the chain (never-pkill rule).
forbidden_scan "TEST-I235-M3-16 drill source has no destructive tokens" "$DRILL"

# TEST-I235-M3-17 REQ-I235-014 — Decision: a destructive token in gs022 escalates an opt-in scenario into chaos.
forbidden_scan "TEST-I235-M3-17 gs022 source has no destructive tokens" "$GS022"

# TEST-I235-M3-18 REQ-I235-014 — Decision: the scanner must be non-vacuous or M3-16/17 prove nothing.
PLANT="$TMP/planted.sh"
printf '#!/bin/bash\n# pkill and ssh in a comment are fine\npkill -f doli-node\nrm -rf "$X"\nssh some-host true\nskill_level=1\n' > "$PLANT"
hits="$(code_only "$PLANT" | grep -cE -- "$FORBIDDEN")"
check "TEST-I235-M3-18 scanner catches 3 planted tokens, ignores comments and lookalikes" \
    "$(ok_if test "$hits" = 3)" "hits=$hits"

# --- Gauntlet wiring ------------------------------------------------------

# TEST-I235-M3-19 REQ-I235-014 — Decision: without the flag arm the scenario can never be selected.
check "TEST-I235-M3-19 gauntlet.sh accepts --gs022" \
    "$(ok_if grep -qE -- '--gs022\)[[:space:]]*GS022=1' "$GAUNTLET")"

# TEST-I235-M3-20 REQ-I235-014 — Decision: an unsourced library leaves gs022_inject undefined at run time.
r=0
grep -qE 'GS022_LIB="\$ROOT/scripts/gauntlet-gs022\.sh"' "$GAUNTLET" || r=1
grep -qE '(\.|source)[[:space:]]+"\$GS022_LIB"' "$GAUNTLET" || r=1
code_only "$GAUNTLET" | grep -qE '(^|[^_[:alnum:]])gs022_inject([^_[:alnum:]]|$)' || r=1
check "TEST-I235-M3-20 gauntlet.sh sources gauntlet-gs022.sh and calls gs022_inject" "$r"

# TEST-I235-M3-21 REQ-I235-014 — Decision: a missing confirm guard lets a plain --gs022 perturb the live testnet.
r=0
if [ -f "$GS022" ]; then
    code_only "$GS022" | grep -qE 'GAUNTLET_GS022_CONFIRM:-0\}"?[[:space:]]*!=[[:space:]]*"?1' || r=1
    code_only "$GS022" | grep -q 'inc-i-235-drill.sh' || r=1
else
    r=1
fi
check "TEST-I235-M3-21 gs022 has the confirm-var guard and invokes the drill" "$r"

# TEST-I235-M3-22 REQ-I235-014 — Decision: a guard that is present but not on the call path still runs the drill.
FAKE_ROOT="$TMP/root"
/bin/mkdir -p "$FAKE_ROOT/scripts" "$TMP/work"
printf '#!/bin/sh\necho "drill $*" >> "$CALL_LOG"\nexit 0\n' > "$FAKE_ROOT/scripts/inc-i-235-drill.sh"
/bin/chmod +x "$FAKE_ROOT/scripts/inc-i-235-drill.sh"
: > "$CALL_LOG"
(
    export PATH="$SHIM:$PATH" HOME="$FAKE_HOME" TESTNET_DIR="$FAKE_TESTNET"
    unset GAUNTLET_GS022_CONFIRM
    ROOT="$FAKE_ROOT"; LIVE="n3"; WORK="$TMP/work"
    C_G=''; C_R=''; C_Y=''; C_C=''; C_0=''
    say() { printf "%b\n" "$*"; }
    kill() { echo "kill $*" >> "$CALL_LOG"; }
    [ -f "$GS022" ] || exit 90
    # shellcheck source=/dev/null
    . "$GS022"
    declare -F gs022_inject >/dev/null || exit 91
    gs022_inject
) > "$OUT" 2>&1 < /dev/null
RC=$?
r=0
[ "$RC" = 0 ] || r=1
log_has '^drill ' && r=1
out_has 'GAUNTLET_GS022_CONFIRM' || r=1
check "TEST-I235-M3-22 gs022_inject without confirm skips the drill" "$r" "$(snippet) calls=$(calls)"
no_mutation "TEST-I235-M3-22b unconfirmed gs022 performs no mutation"

# TEST-I235-M3-23 REQ-I235-014 — Decision: an early-exit `grep -q` on window() SIGPIPEs tail under pipefail, so the drill reports by=none / no-snap although the line is in the window.
r=0
code_only "$DRILL" | grep -qE 'window "[^"]*" \| grep -q' && r=1
check "TEST-I235-M3-23a no detection pipes window() into an early-exit grep -q" "$r"
SIG="$TMP/sigpipe"; /bin/mkdir -p "$SIG/logs"
FILL="$(printf 'filler %.0s' $(seq 1 12))"
for pat in '[SNAP_SYNC] starting snap sync' '[ROLLBACK] Initiating rollback to h=1'; do
    { echo "$pat"; yes "$FILL" | head -c 6000000; } > "$SIG/logs/n3.log"
    key='SNAP_SYNC'; case "$pat" in *ROLLBACK*) key='ROLLBACK.*Initiating';; esac
    cond="$(code_only "$DRILL" | sed -n "s/.*if \(.*$key.*\); then.*/\1/p" | head -n 1)"
    fns="$(code_only "$DRILL" | sed -n '/^window() {/,/^}/p;/^window_has() {/,/^}/p')"
    rc=1
    if [ -n "$cond" ] && [ -n "$fns" ]; then
        ( set -uo pipefail; LOG_DIR="$SIG/logs"; TARGET=n3; offset_of() { echo 0; }
          eval "$fns"; eval "$cond" ) >/dev/null 2>&1; rc=$?
    fi
    check "TEST-I235-M3-23b early '${pat%% *}' match in a 6 MB window is detected (pipefail)" "$rc" "cond=[$cond] rc=$rc"
done

# --- M3b: gap>=50 isolation (Rule 1b closed, the seed2 shape) ----------------

# TEST-I235-M3B-01 REQ-I235-014 — Decision: without an isolation step the drill never
# reaches gap >= 50, Rule 1/1b always settle the fork first and the walk is never exercised.
run_drill --help
r=0; for tok in '--isolate-gap' '--isolate-exec' 'testnet.sh stop'; do out_has "$tok" || r=1; done
check "TEST-I235-M3B-01 --help documents --isolate-gap, --isolate-exec and the stop" "$r" "$(snippet)"

# TEST-I235-M3B-02 REQ-I235-014 — Decision: an isolation plan must be reviewable before it stops a node.
run_drill --dry-run --target n3 --isolate-gap 60
r=0; for tok in 'isolate' 'gap >= 60' 'testnet.sh stop n3' 'testnet.sh start n3'; do out_has "$tok" || r=1; done
[ "$RC" = 0 ] || r=1
check "TEST-I235-M3B-02 dry-run with --isolate-gap 60 prints the stop/start plan" "$r" "$(snippet)"
no_mutation "TEST-I235-M3B-02b dry-run with --isolate-gap stops nothing"

# TEST-I235-M3B-03 REQ-I235-014 — Decision: a non-numeric gap would loop forever or stop the node for nothing.
for bad in abc 0 -5; do
    run_drill --dry-run --target n3 --isolate-gap "$bad"
    r=0; [ "$RC" != 0 ] || r=1
    check "TEST-I235-M3B-03 --isolate-gap '$bad' rejected" "$r" "$(snippet)"
    no_mutation "TEST-I235-M3B-03b --isolate-gap '$bad' performs no mutation"
done

# TEST-I235-M3B-04 REQ-I235-014 — Decision: an exit path that leaves the target stopped
# drops a producer from the fleet; restore() must start it again.
r=0
code_only "$DRILL" | sed -n '/^restore() {/,/^}/p' | grep -qE 'TESTNET_SH"? start' || r=1
check "TEST-I235-M3B-04 restore() restarts a target the drill stopped" "$r"

# TEST-I235-M3B-05 REQ-I235-014 — Decision: the walk evidence must be in the window report.
r=0
for tok in 'RECOVERY_SKIP' 'Walked block' 'BLOCKED'; do code_only "$DRILL" | grep -qF -- "$tok" || r=1; done
check "TEST-I235-M3B-05 evidence regex covers RECOVERY_SKIP, ineligible walk and production blocks" "$r"

echo
echo "passed=$PASS_COUNT failed=$FAIL_COUNT"
[ "$FAIL_COUNT" = 0 ]
