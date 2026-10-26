#!/usr/bin/env bash
# Shared checks for scripts/test_gauntlet_gs028.sh and scripts/test_gauntlet_gs029.sh (sourced, never run).
# OUTPUT CONTRACT: N/A — fixture file (the contracts live in the two callers)
# INPUT PARTITIONS: N/A — fixture file
# The caller sets RUNNER, TAG (gs028|gs029), PREFIX (GS028|GS029), IDS and EXTRA (sourced helpers) first.
# shellcheck disable=SC2016,SC2034,SC2046,SC2086

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB="$ROOT/scripts/gauntlet-devnet-lib.sh"
PYH="$ROOT/scripts/gauntlet-devnet.py"
GATES="DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT"
LIB_OWNED="alive ours stop_one cleanup start_node start_cluster seed_gset gset_ready wait_height inject_sibling restart_node check_ports cli_as wait_tx_mined sampler_start sampler_stop write_chainspec gen_wallets"
RUNNER_MAX=480
T="$(mktemp -d "${TMPDIR:-/tmp}/t-devnet.XXXXXX")" || exit 1
trap 'rm -rf "$T"' EXIT
NP=0
NF=0
pass() { echo "PASS $1${2:+ ($2)}"; NP=$((NP + 1)); }
fail() { echo "FAIL $1: $2"; NF=$((NF + 1)); }
sorted() { tr ' ,' '\n\n' | sed '/^$/d' | sort -u | tr '\n' ' ' | sed 's/ $//'; }
code() { grep -vE '^[[:space:]]*#' "$1"; }
existing() { local f; for f in "$@"; do [ -f "$f" ] && printf '%s\n' "$f"; done; return 0; }
own_files() { existing "$RUNNER" $EXTRA; }
scope_files() { existing "$RUNNER" "$LIB" $EXTRA; }
code_of_files() { local f; for f in "$@"; do code "$f"; done; }
present() { [ -f "$RUNNER" ] && return 0; fail "$1" "${RUNNER#"$ROOT"/} absent"; return 1; }
count() { "$@" | sed '/^$/d' | wc -l | tr -d ' '; }

# --- predicates (each prints one line per violation) ---------------------------------
unsafe() {
  local c p
  c="$(code "$1")"
  p='(~|\$HOME|\$\{HOME\})/testnet|\.doli/(devnet|testnet|mainnet)'
  printf '%s\n' "$c" | grep -E '(^|[^[:alnum:]_])(pkill|killall|launchctl|systemctl|ssh|eval)([^[:alnum:]_]|$)|testnet\.sh' | sed 's/^/forbidden: /'
  printf '%s\n' "$c" | grep -E '(^|[;&|({[:space:]])ln[[:space:]]' | sed 's/^/link: /'
  printf '%s\n' "$c" | grep -E '(^|[^[:alnum:]_-])kill[[:space:]]' | sed -E 's/kill( -[A-Za-z0-9]+)? "\$PID"//g' |
    grep -E '(^|[^[:alnum:]_-])kill[[:space:]]' | sed 's/^/kill: /'
  printf '%s\n' "$c" | grep -E '(^|[^[:alnum:]_-])rm[[:space:]]+-[A-Za-z]*[rR]' |
    grep -vE '\*/"\$DN_TAG"\.\*\)[[:space:]]*rm -rf "\$S"' | sed 's/^/rm: /'
  printf '%s\n' "$c" | grep -E "$p" |
    grep -E "(^|[^[:alnum:]_-])(rm|cp|mv|mkdir|touch|tee|rsync|tar|install|dd)([^[:alnum:]_-]|$)|>>?[[:space:]]*\"?[^[:space:]]*($p)" |
    sed 's/^/path: /'
}

nonloopback() {
  code "$1" | grep -oE '([0-9]{1,3}\.){3}[0-9]{1,3}|https?://[^/:"'\''[:space:]]+|localhost' |
    grep -vxE '127\.0\.0\.1|http://127\.0\.0\.1' | sed 's/^/foreign-host: /'
}

# `$?` read right after a pipeline (same line before `;`, or the previous code line).
pipe_rc() {
  code "$1" | awk '
    function piped(s) { gsub(/\047[^\047]*\047/, "", s); gsub(/\|\|/, "", s); return s ~ /\|/ }
    /\$\?/ {
      cur = $0; pre = cur; sub(/[;&][^;&]*\$\?.*$/, "", pre)
      cand = (pre != cur) ? pre : prev
      if (piped(cand)) print "pipe-rc: " cand " -> " cur
    }
    !/^[[:space:]]*$/ { prev = $0 }'
}

copied_funcs() {
  local f
  for f in $LIB_OWNED; do
    code "$1" | grep -E "^[[:space:]]*(function[[:space:]]+)?${f}[[:space:]]*\(\)" | sed 's/^/copied: /'
  done
}

# Knobs PREFIX_* read by $1 (and the sourced helpers) that its leading comment header does not name.
undocumented() {
  local hdr n
  hdr="$(awk 'NR == 1 { next } /^#/ { print; next } { exit }' "$1")"
  for n in $(code_of_files "$1" $(existing $EXTRA) | grep -oE "${PREFIX}_[A-Z0-9_]+" | sort -u); do
    printf '%s\n' "$hdr" | grep -qw -- "$n" || echo "undocumented: $n"
  done
}

gate_values() {
  local g
  for g in $GATES; do
    code "$1" | grep -oE "(^|[^[:alnum:]_])$g=[^[:space:])]*" | sed -E "s/^.*$g=//; s/[\"']//g; s/^\$/<empty>/"
  done
}

gates() {
  local g v miss=""
  for g in $GATES; do code "$1" | grep -qE "(^|[^[:alnum:]_])$g=" || miss="$miss $g"; done
  [ -z "$miss" ] || echo "gate-unset:$miss"
  code "$1" | grep -qE '(^|[^[:alnum:]_])DOLI_INC_I_181_PENDING_ROOT_ACTIVATION_HEIGHT=0([^0-9]|$)' ||
    echo "gate-pending-root: never set to 0"
  v="$(gate_values "$1" | sort -u)"
  [ "$(printf '%s\n' "$v" | sed '/^$/d' | wc -l | tr -d ' ')" -le 1 ] ||
    echo "gate-values-differ: $(printf '%s' "$v" | tr '\n' ' ')"
}

over_budget() { [ "$(wc -l <"$1" | tr -d ' ')" -le "$2" ] || echo "over-budget: ${1#"$ROOT"/} $(wc -l <"$1" | tr -d ' ') > $2"; }

emitted_ids() {
  code_of_files "$@" | grep -oE "(^|[^[:alnum:]_-])res[[:space:]]+\"?$TAG-[a-z0-9-]+" | grep -oE "$TAG-[a-z0-9-]+" | sorted
}

# The `res <id> ...` call text, continuation lines included.
res_block() {
  local id="$1"
  shift
  code_of_files "$@" | awk -v id="$id" '
    on { print; if ($0 !~ /\\[[:space:]]*$/) on = 0; next }
    match($0, "(^|[^[:alnum:]_-])res[[:space:]]+\"?" id "([^a-z0-9-]|$)") { print; if ($0 ~ /\\[[:space:]]*$/) on = 1 }'
}

metric_text() { code_of_files "$@" | awk '/METRIC / { on = 1 } on { print; if ($0 !~ /\\[[:space:]]*$/) on = 0 }'; }

# --- cases ----------------------------------------------------------------------------
c_present() {
  present runner-present || return
  [ -x "$RUNNER" ] || { fail runner-present "not executable (a bare-name run exits 126)"; return; }
  bash -n "$RUNNER" 2>"$T/n.err" || { fail runner-present "bash -n: $(head -1 "$T/n.err")"; return; }
  pass runner-present
}

c_header() {
  local hdr id miss=""
  present output-contract || return
  head -n 40 "$RUNNER" | grep -q 'OUTPUT CONTRACT:' || { fail output-contract "no 'OUTPUT CONTRACT:' in the first 40 lines"; return; }
  hdr="$(awk 'NR == 1 { next } /^#/ { print; next } { exit }' "$RUNNER")"
  for id in $IDS; do printf '%s\n' "$hdr" | grep -qw -- "$id" || miss="$miss $id"; done
  [ -z "$miss" ] || { fail output-contract "header does not document:$miss"; return; }
  printf '%s\n' "$hdr" | grep -qE 'exit:.*0.*1.*2' || { fail output-contract "header has no 'exit: 0 ... 1 ... 2' line"; return; }
  pass output-contract "$(echo $IDS | wc -w | tr -d ' ') ids documented"
}

c_ids() {
  local got want
  present emitted-ids || return
  got="$(emitted_ids $(own_files))"
  want="$(echo "$IDS" | sorted)"
  [ "$got" = "$want" ] || { fail emitted-ids "runner emits {$got} want {$want}"; return; }
  pass emitted-ids
}

c_exit_contract() {
  local c last
  present exit-contract || return
  c="$(code "$RUNNER")"
  printf '%s\n' "$c" | grep -qE 'set -[a-z]*o[[:space:]]+pipefail' || { fail exit-contract "no set -o pipefail"; return; }
  printf '%s\n' "$c" | grep -qE "(^|[^[:alnum:]_])DN_TAG=[\"']?${TAG}[\"']?([^[:alnum:]_]|$)" ||
    { fail exit-contract "DN_TAG is not $TAG, so skip() would not print 'SKIP $TAG:'"; return; }
  code_of_files $(scope_files) | grep -qE '(^|[^[:alnum:]_])exit[[:space:]]+2([^0-9]|$)' || { fail exit-contract "no 'exit 2' SKIP path"; return; }
  printf '%s\n' "$c" | grep -qF 'control inconclusive' || { fail exit-contract "no 'control inconclusive' SKIP for an inconclusive control"; return; }
  last="$(printf '%s\n' "$c" | sed '/^[[:space:]]*$/d' | tail -1)"
  printf '%s' "$last" | grep -qE '^[[:space:]]*verdict;[[:space:]]*exit[[:space:]]+\$\?[[:space:]]*$' ||
    { fail exit-contract "last code line '$last' is not 'verdict; exit \$?'"; return; }
  pass exit-contract
}

c_metric() {
  local m f miss=""
  present metric-fields || return
  m="$(metric_text $(own_files))"
  [ -n "$m" ] || { fail metric-fields "no METRIC line"; return; }
  for f in "$@"; do printf '%s\n' "$m" | grep -qE "(^|[[:space:]\"])$f=" || miss="$miss $f="; done
  [ -z "$miss" ] || { fail metric-fields "METRIC lacks:$miss"; return; }
  pass metric-fields "$#"
}

c_res_kv() {
  local id="$1" b k miss=""
  shift
  present "kv-$id" || return
  b="$(res_block "$id" $(own_files))"
  [ -n "$b" ] || { fail "kv-$id" "no res $id call"; return; }
  for k in "$@"; do printf '%s\n' "$b" | grep -qE "(^|[[:space:]\"])$k=" || miss="$miss $k="; done
  [ -z "$miss" ] || { fail "kv-$id" "measured line lacks:$miss"; return; }
  pass "kv-$id"
}

c_sources_lib() {
  local c
  present sources-lib || return
  c="$(code "$RUNNER")"
  printf '%s\n' "$c" | grep -qE '^[[:space:]]*(\.|source)[[:space:]]+"?\$\{?ROOT\}?/scripts/gauntlet-devnet-lib\.sh"?' ||
    { fail sources-lib "does not source \$ROOT/scripts/gauntlet-devnet-lib.sh"; return; }
  printf '%s\n' "$c" | grep -qE '(\.|source)[[:space:]]+.*gauntlet-gs0[0-9]{2}\.sh' && { fail sources-lib "sources another runner"; return; }
  [ -z "$(code_of_files $(own_files) | grep -E 'GS0[0-9]{2}_LIB_ONLY')" ] || { fail sources-lib "uses a runner LIB_ONLY mode"; return; }
  pass sources-lib
}

c_env_documented() {
  local v
  present env-documented || return
  v="$(undocumented "$RUNNER")"
  [ -z "$v" ] || { fail env-documented "$(printf '%s' "$v" | tr '\n' ' ')"; return; }
  for v in "$@"; do code_of_files $(own_files) | grep -qE "(^|[^[:alnum:]_])$v([^[:alnum:]_]|$)" || { fail env-documented "never reads $v"; return; }; done
  pass env-documented "every ${PREFIX}_* read is in the header"
}

c_gates() {
  local v
  present gates-one-height || return
  v="$(gates "$RUNNER")"
  [ -z "$v" ] || { fail gates-one-height "$(printf '%s' "$v" | tr '\n' ' ')"; return; }
  pass gates-one-height "four gates share one value, pending-root 0"
}

c_harness_safety() {
  local v f
  present harness-safety || return
  v="$(for f in $(scope_files); do unsafe "$f"; nonloopback "$f"; pipe_rc "$f"; done)"
  [ -z "$v" ] || { fail harness-safety "$(printf '%s' "$v" | head -3 | tr '\n' ' ')"; return; }
  v="$(copied_funcs "$RUNNER"; for f in $EXTRA; do [ -f "$f" ] && copied_funcs "$f"; done)"
  [ -z "$v" ] || { fail harness-safety "library function redefined (copy-paste): $(printf '%s' "$v" | head -2 | tr '\n' ' ')"; return; }
  grep -qE "mktemp[[:space:]]+-d[[:space:]]+\"?\\$\\{?(TMPROOT|TMPDIR)[^\"]*/$TAG\\.X{6,}" "$RUNNER" ||
    { fail harness-safety "scratch is not mktemp -d \"\$TMPROOT/$TAG.XXXXXX\""; return; }
  code "$RUNNER" | grep -qF 'TMPDIR' || { fail harness-safety "scratch root does not come from TMPDIR"; return; }
  code_of_files $(scope_files) | grep -qE 'kill[[:space:]]+(-[A-Za-z0-9]+[[:space:]]+)?"\$PID"' ||
    { fail harness-safety "never signals only its own \$PID"; return; }
  pass harness-safety "own PID, own scratch, loopback only, no eval, no rc read through a pipe"
}

c_order() {
  local cp sc dc
  present start-order || return
  cp="$(grep -nE '^[[:space:]]*[^#]*(^|[^[:alnum:]_])check_ports([^[:alnum:]_]|$)' "$RUNNER" | head -1 | cut -d: -f1)"
  sc="$(grep -nE '^[[:space:]]*[^#]*(^|[^[:alnum:]_])start_cluster([^[:alnum:]_(]|$)' "$RUNNER" | head -1 | cut -d: -f1)"
  dc="$(grep -nE '^[^#]*CONTROL[[:space:]]+expect_fail=' "$RUNNER" | head -1 | cut -d: -f1)"
  { [ -n "$cp" ] && [ -n "$sc" ] && [ "$cp" -lt "$sc" ]; } || { fail start-order "check_ports (line ${cp:-none}) must precede start_cluster (line ${sc:-none})"; return; }
  { [ -n "$dc" ] && [ "$dc" -lt "$sc" ]; } || { fail start-order "CONTROL declaration (line ${dc:-none}) must precede start_cluster (line $sc)"; return; }
  pass start-order "ports checked and control declared before the cluster starts"
}

c_control() {
  local line exp pre
  present control-declared || return
  line="$(code "$RUNNER" | grep -m1 -E 'CONTROL[[:space:]]+expect_fail=')"
  [ -n "$line" ] || { fail control-declared "no 'CONTROL expect_fail=<ids> preconditions=<ids>' declaration"; return; }
  exp="$(printf '%s' "$line" | sed -E 's/.*expect_fail=//; s/preconditions=.*//' | grep -oE "$TAG-[a-z0-9-]+" | sorted)"
  pre="$(printf '%s' "$line" | grep -oE 'preconditions=[^"[:space:]]+' | sed 's/^preconditions=//')"
  [ "$exp" = "$(echo "$1" | sorted)" ] || { fail control-declared "expect_fail {$exp} want {$(echo "$1" | sorted)}"; return; }
  [ -n "$pre" ] || { fail control-declared "empty preconditions="; return; }
  [ -z "$2" ] || [ "$(printf '%s' "$pre" | grep -oE "$TAG-[a-z0-9-]+" | sorted)" = "$(echo "$2" | sorted)" ] ||
    { fail control-declared "preconditions {$pre} want {$(echo "$2" | sorted)}"; return; }
  pass control-declared
}

c_tokens() {
  local name="$1" t miss=""
  shift
  present "$name" || return
  for t in "$@"; do code_of_files $(scope_files) $(existing "$PYH") | grep -qE -- "$t" || miss="$miss '$t'"; done
  [ -z "$miss" ] || { fail "$name" "missing:$miss"; return; }
  pass "$name"
}

# Each helper subcommand is called with an argument (`<sub> "$...` or `<sub> $...`).
c_helper_calls() {
  local s miss="" c
  present helper-calls || return
  c="$(code_of_files $(scope_files))"
  printf '%s\n' "$c" | grep -qF 'gauntlet-devnet.py' || { fail helper-calls "never invokes the devnet python helper"; return; }
  for s in "$@"; do printf '%s\n' "$c" | grep -qE "[[:space:]]${s}[[:space:]]+\"?[\$]" || miss="$miss $s"; done
  [ -z "$miss" ] || { fail helper-calls "helper subcommands never called:$miss"; return; }
  pass helper-calls "$*"
}

c_rpc_values() {
  local c
  present rpc-values || return
  c="$(code_of_files $(scope_files))"
  printf '%s\n' "$c" | grep -qF -- 'error_code' && { fail rpc-values "matches error_code (always INVALID_TRANSACTION)"; return; }
  printf '%s\n' "$c" | grep -qE '\[0-9a-f\]\{64\}|\*\[!0-9a-f\]\*' || { fail rpc-values "no 64-hex validation of RPC-derived hashes"; return; }
  printf '%s\n' "$c" | grep -qF '*[!0-9]*' || { fail rpc-values "no digits validation of RPC-derived numbers"; return; }
  pass rpc-values
}

c_cli_as() {
  local body
  [ -f "$LIB" ] || { fail cli-explicit-rpc "library absent"; return; }
  body="$(awk '/^cli_as[[:space:]]*\(\)/ { on = 1; print; if ($0 ~ /\}[[:space:]]*$/) exit; next } on { print } on && /^}/ { exit }' "$LIB")"
  [ -n "$body" ] || { fail cli-explicit-rpc "the library defines no cli_as()"; return; }
  printf '%s\n' "$body" | grep -q -- '--rpc' || { fail cli-explicit-rpc "cli_as does not pass --rpc (DOLI_RPC_URL would leak in)"; return; }
  printf '%s\n' "$body" | grep -qF 'http://127.0.0.1' || { fail cli-explicit-rpc "cli_as --rpc is not http://127.0.0.1"; return; }
  printf '%s\n' "$body" | grep -qF 'DN_CLI_ENV' || { fail cli-explicit-rpc "cli_as does not export DN_CLI_ENV (the gate env)"; return; }
  pass cli-explicit-rpc
}

c_size() {
  local v f lim
  v=""
  for f in "$@"; do
    lim="${f##*:}"
    f="${f%:*}"
    [ -f "$f" ] || { v="$v missing:${f#"$ROOT"/}"; continue; }
    v="$v$(over_budget "$f" "$lim")"
  done
  [ -z "$v" ] || { fail size-budget "$v"; return; }
  pass size-budget
}

# Copy the runner, plant one violation class at a time; each checker must flag exactly its plants.
c_planted() {
  local p base n
  present planted-controls || return
  p="$T/planted.sh"
  plant() { cp "$RUNNER" "$p"; printf '%s\n' "$@" >>"$p"; }
  delta() { base="$(count "$1" "$RUNNER")"; n="$(count "$1" "$p")"; echo $((n - base)); }
  plant 'pkill -f doli-node' 'kill -TERM "$OTHER_PID"' 'rm -rf "$HOME/x"' 'eval "$reply"'
  [ "$(delta unsafe)" = 4 ] || { fail planted-controls "safety checker flagged $(delta unsafe) of 4"; return; }
  plant 'curl -s http://10.0.0.5:8500' 'args+=(--bootstrap /ip4/192.168.1.9/tcp/1)'
  [ "$(delta nonloopback)" = 2 ] || { fail planted-controls "loopback checker flagged $(delta nonloopback) of 2"; return; }
  plant 'python3 -I "$PYH" removed "$d" | tail -1' 'rc=$?' 'out="$(sort "$f" | head -1)"; rc=$?'
  [ "$(delta pipe_rc)" = 2 ] || { fail planted-controls "pipe-rc checker flagged $(delta pipe_rc) of 2"; return; }
  plant 'start_node() { :; }'
  [ "$(delta copied_funcs)" = 1 ] || { fail planted-controls "copy checker flagged $(delta copied_funcs) of 1"; return; }
  plant "X=\"\${${PREFIX}_PLANTED_KNOB:-}\""
  [ "$(delta undocumented)" = 1 ] || { fail planted-controls "env checker flagged $(delta undocumented) of 1"; return; }
  plant 'DN_ENV+=(DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT=99999)'
  [ "$(count gates "$p")" -ge 1 ] || { fail planted-controls "gate checker missed a second gate value"; return; }
  seq 1 $((RUNNER_MAX + 1)) >"$T/long.sh"
  [ -n "$(over_budget "$T/long.sh" "$RUNNER_MAX")" ] || { fail planted-controls "size checker missed $((RUNNER_MAX + 1)) lines"; return; }
  seq 1 "$RUNNER_MAX" >"$T/long.sh"
  [ -z "$(over_budget "$T/long.sh" "$RUNNER_MAX")" ] || { fail planted-controls "size checker flagged exactly $RUNNER_MAX lines"; return; }
  pass planted-controls "safety 4/4, loopback 2/2, pipe-rc 2/2, copy 1/1, env 1/1, gates, size"
}
