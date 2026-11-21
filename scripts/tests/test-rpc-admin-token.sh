#!/usr/bin/env bash
# INC-I-308 M3 — admin RPC token helper + caller census.
# REQ-RPC-ADMIN-TOKEN — Decision: a FAIL means an operator script calls an admin method without a token path, so it gets -32008 once every address needs the token.
# OUTPUT CONTRACT: rpc_admin_token → stdout (token|empty), return code (0|1); rpc_admin_curl_args → stdout (2 lines|empty), return code (0);
#   no persistent writes. Every (output × partition) cell below is asserted in b–e (bash) and f (python).
# INPUT PARTITIONS: env set + file present (b); env unset + file (c); env empty + file (c); env unset/empty + no file (d);
#   empty token file (d); env set + no file (e); file with trailing newline (c).
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 2
HELPER="scripts/lib/rpc-admin-token.sh"
PYHELPER="scripts/lib/rpc_admin_token.py"
ADMIN='pauseProduction|resumeProduction|createCheckpoint|pruneBlocks|backfillFromPeer|enterRecoveryMode|exitRecoveryMode|bridgeFromArchive|getUtxoDiff|getStateSnapshot|getStateRootDebug|verifyChainIntegrity|repairArchiveFromPeer|forceReorgTo'
REF='rpc-admin-token\.sh|rpc_admin_curl_args|rpc_admin_token|rpc_admin_token\.py|auth_headers|Authorization'

EXCEPTIONS=(
  "scripts/test_gauntlet_gs024.sh|offline test: method name is a grep pattern over a mock command trace (:488,:495)"
  "scripts/test_gauntlet_gs029.sh|offline test: method name is a token asserted present in probe script text (:119)"
  "scripts/test_gauntlet_stateroot_settle.py|unit test: mock RPC dispatch compares method names (:64)"
  "scripts/test_gauntlet_stranded_gsn01.py|unit test: mock RPC responses keyed by method name (:334,:337)"
  "scripts/test_gauntlet_stranded_replay_side.py|unit test: asserts recorded mock events (:182,:183,:205)"
  "scripts/fork-lab/gs035.py|routes through gauntlet-gs033/tn.rpc, which adds rpc_admin_token.headers_for (:329)"
  "scripts/gauntlet-gs033/probes.py|routes through tn.rpc, which adds rpc_admin_token.headers_for (:180)"
  "scripts/gauntlet-stranded/bank.py|routes through nodes.rpcp, which adds rpc_admin_token.headers_for (:54)"
  "scripts/gauntlet-stranded/fastpath.py|routes through nodes.rpcp, which adds rpc_admin_token.headers_for (:249,:291)"
  "scripts/gauntlet-stranded/gsn01.py|routes through nodes.rpcp / tn.rpc, which add rpc_admin_token.headers_for (:141,:369)"
  "scripts/gauntlet-stranded/gsn02_strand.py|routes through nodes.rpcp / tn.rpc, which add rpc_admin_token.headers_for (:53,:54)"
  "scripts/gauntlet-stranded/fork.py|method name is a result dict key; the call is nodes.integrity -> nodes.rpcp (:46,:48)"
  "scripts/gauntlet-stranded/strand.py|method name is a result dict key; the call is nodes.integrity -> nodes.rpcp (:247,:295)"
  "scripts/gauntlet-gs016-runner/replica.py|token=True -> lib.rpc sends Authorization (lib.py:35); node gets DOLI_RPC_ADMIN_TOKEN (:123)"
  "scripts/gauntlet-gs024.sh|routes through gs024_rpc/gs024_irpc in gauntlet-gs024-lib.sh, which add rpc_admin_header_for (:148,:175,:176)"
  "scripts/gauntlet-gs029-probes.sh|routes through call()/rpc() in gauntlet-devnet-lib.sh, which add rpc_admin_header_for (:121,:192,:208)"
)

FAILS=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1${2:+ — $2}"; FAILS=$((FAILS + 1)); }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/rpc-admin-token-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
TOK_FILE="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
TOK_ENV="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
D_FILE="$TMP/with-token"; D_NONE="$TMP/no-token"
mkdir -p "$D_FILE" "$D_NONE"
printf '%s\n' "$TOK_FILE" > "$D_FILE/rpc-admin.token"
chmod 600 "$D_FILE/rpc-admin.token"

# run_helper <unset|empty|set> <fn> <data_dir> — prints the function stdout then "rc=<n>"
run_helper() {
  local mode="$1" fn="$2" dir="$3"
  (
    unset DOLI_RPC_ADMIN_TOKEN
    case "$mode" in empty) export DOLI_RPC_ADMIN_TOKEN="";; set) export DOLI_RPC_ADMIN_TOKEN="$TOK_ENV";; esac
    # shellcheck disable=SC1090
    source "$ROOT/$HELPER" >/dev/null 2>&1 || exit 97
    "$fn" "$dir"
    echo "rc=$?"
  ) 2>/dev/null
}
show() { printf '%s' "$1" | tr '\n' '|'; }

# a. helper exists and parses
if [ -f "$HELPER" ] && bash -n "$HELPER" 2>/dev/null; then
  pass "a helper exists and parses"; HAVE=1
else
  fail "a helper exists and parses" "$HELPER missing or bash -n failed"; HAVE=0
fi

if [ "$HAVE" = 1 ]; then
  # b. env wins over file
  out="$(run_helper set rpc_admin_token "$D_FILE")"
  [ "$out" = "$TOK_ENV"$'\n'"rc=0" ] && pass "b env wins over file" || fail "b env wins over file" "got: $(show "$out")"

  # c. file used when env unset / empty; trailing newline trimmed
  out="$(run_helper unset rpc_admin_token "$D_FILE")"
  [ "$out" = "$TOK_FILE"$'\n'"rc=0" ] && pass "c file used when env unset" || fail "c file used when env unset" "got: $(show "$out")"
  out="$(run_helper empty rpc_admin_token "$D_FILE")"
  [ "$out" = "$TOK_FILE"$'\n'"rc=0" ] && pass "c file used when env empty" || fail "c file used when env empty" "got: $(show "$out")"
  tok="$( (unset DOLI_RPC_ADMIN_TOKEN; source "$ROOT/$HELPER" >/dev/null 2>&1; printf '[%s]' "$(rpc_admin_token "$D_FILE")") 2>/dev/null)"
  [ "$tok" = "[$TOK_FILE]" ] && pass "c trailing newline trimmed" || fail "c trailing newline trimmed" "got: $tok"

  # d. none → rc 1, nothing printed; curl args empty with rc 0
  out="$(run_helper unset rpc_admin_token "$D_NONE")"
  [ "$out" = "rc=1" ] && pass "d no token → rc=1, empty" || fail "d no token → rc=1, empty" "got: $(show "$out")"
  out="$(run_helper empty rpc_admin_token "$D_NONE")"
  [ "$out" = "rc=1" ] && pass "d empty env + no file → rc=1" || fail "d empty env + no file → rc=1" "got: $(show "$out")"
  out="$(run_helper unset rpc_admin_curl_args "$D_NONE")"
  [ "$out" = "rc=0" ] && pass "d no token → curl args empty, rc=0" || fail "d no token → curl args empty, rc=0" "got: $(show "$out")"
  : > "$D_NONE/rpc-admin.token"
  out="$(run_helper unset rpc_admin_token "$D_NONE")"
  [ "$out" = "rc=1" ] && pass "d empty token file → rc=1" || fail "d empty token file → rc=1" "got: $(show "$out")"
  rm -f "$D_NONE/rpc-admin.token"

  # e. curl args shape, consumed line-by-line as mapfile would
  arr_out="$( (
    unset DOLI_RPC_ADMIN_TOKEN
    source "$ROOT/$HELPER" >/dev/null 2>&1
    args=()
    while IFS= read -r l; do args+=("$l"); done < <(rpc_admin_curl_args "$D_FILE")
    echo "${#args[@]}"; printf '%s\n' "${args[@]}"
  ) 2>/dev/null)"
  [ "$arr_out" = "2"$'\n'"-H"$'\n'"Authorization: Bearer $TOK_FILE" ] && pass "e curl args = -H / Authorization: Bearer <tok>" \
    || fail "e curl args = -H / Authorization: Bearer <tok>" "got: $(show "$arr_out")"
  out="$(run_helper set rpc_admin_curl_args "$D_NONE")"
  [ "$out" = "-H"$'\n'"Authorization: Bearer $TOK_ENV"$'\n'"rc=0" ] && pass "e curl args from env without file" \
    || fail "e curl args from env without file" "got: $(show "$out")"
fi

# f. python sibling module (optional)
if [ -f "$PYHELPER" ]; then
  pyrun() {
    (unset DOLI_RPC_ADMIN_TOKEN
     [ "$1" = set ] && export DOLI_RPC_ADMIN_TOKEN="$TOK_ENV"
     [ "$1" = empty ] && export DOLI_RPC_ADMIN_TOKEN=""
     python3 -I -c 'import sys, importlib.util as u
s = u.spec_from_file_location("rpc_admin_token", sys.argv[1]); m = u.module_from_spec(s); s.loader.exec_module(m)
print(repr(m.admin_token(sys.argv[2])), repr(m.auth_headers(sys.argv[2])))' "$ROOT/$PYHELPER" "$2") 2>&1
  }
  exp_f="'$TOK_FILE' {'Authorization': 'Bearer $TOK_FILE'}"
  exp_e="'$TOK_ENV' {'Authorization': 'Bearer $TOK_ENV'}"
  o="$(pyrun set "$D_FILE")";   [ "$o" = "$exp_e" ] && pass "f py env wins" || fail "f py env wins" "$o"
  o="$(pyrun unset "$D_FILE")"; [ "$o" = "$exp_f" ] && pass "f py file when env unset" || fail "f py file when env unset" "$o"
  o="$(pyrun empty "$D_FILE")"; [ "$o" = "$exp_f" ] && pass "f py file when env empty" || fail "f py file when env empty" "$o"
  o="$(pyrun unset "$D_NONE")"; [ "$o" = "None {}" ] && pass "f py none" || fail "f py none" "$o"
else
  echo "SKIP f $PYHELPER absent"
fi

# g. bash -n on admin-calling shell scripts
gbad=0
while IFS= read -r f; do
  bash -n "$f" 2>/dev/null || { echo "  syntax error: $f"; gbad=$((gbad + 1)); }
done < <(grep -rlE "$ADMIN" scripts testnetlinux/scripts --include='*.sh' 2>/dev/null | sort)
[ "$gbad" = 0 ] && pass "g bash -n on admin-calling scripts" || fail "g bash -n on admin-calling scripts" "$gbad file(s)"

# h. census: admin method in an RPC call (quoted literal, or bare argument of a shell function) → must reference the helper
QUOTED="[\"'](${ADMIN})\\\\?[\"']|(${ADMIN})\\\\\""
BARE="[A-Za-z_][A-Za-z0-9_]*[[:space:]]+([^[:space:]]+[[:space:]]+)?(${ADMIN})([[:space:]]|\$|[\"')])"
is_exception() {
  local e
  for e in "${EXCEPTIONS[@]}"; do [ "${e%%|*}" = "$1" ] && return 0; done
  return 1
}
offenders=0; callers=0
while IFS= read -r f; do
  body="$(grep -vE '^[[:space:]]*#' "$f")"
  hit=0
  printf '%s\n' "$body" | grep -qE "$QUOTED" && hit=1
  case "$f" in *.sh) [ "$hit" = 0 ] && printf '%s\n' "$body" | grep -qE "$BARE" && hit=1;; esac
  [ "$hit" = 1 ] || continue
  is_exception "$f" && continue
  callers=$((callers + 1))
  grep -qE "$REF" "$f" || { echo "  census offender: $f"; offenders=$((offenders + 1)); }
done < <(find scripts -type f \( -name '*.sh' -o -name '*.py' \) -not -path 'scripts/tests/*' -not -path 'scripts/lib/*' | sort)
for e in "${EXCEPTIONS[@]}"; do [ -f "${e%%|*}" ] || echo "  stale exception (file gone): ${e%%|*}"; done
[ "$callers" -gt 0 ] || fail "h census non-vacuous" "no admin callers detected"
[ "$offenders" = 0 ] && pass "h census: $callers admin callers reference the token helper" \
  || fail "h census" "$offenders of $callers admin callers lack a token path"

echo "---"
[ "$FAILS" = 0 ] && { echo "ALL PASS"; exit 0; }
echo "$FAILS check(s) failed"; exit 1
