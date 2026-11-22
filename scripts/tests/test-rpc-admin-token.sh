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
  "scripts/gauntlet-gs063.sh|reads the scratch node's own rpc-admin.token and passes it to ga_rpc (:75-76)"
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
    case "$mode" in empty) export DOLI_RPC_ADMIN_TOKEN="";; set) export DOLI_RPC_ADMIN_TOKEN="$TOK_ENV";; short) export DOLI_RPC_ADMIN_TOKEN="legacy-short";; esac
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

  # j. INC-I-308 P2-a — Decision: a FAIL means a short env token (ignored by the node) shadows the token file.
  out="$(run_helper short rpc_admin_token "$D_FILE")"
  [ "$out" = "$TOK_FILE"$'\n'"rc=0" ] && pass "j short env → file token" || fail "j short env → file token" "got: $(show "$out")"
fi

# f. python sibling module (optional)
if [ -f "$PYHELPER" ]; then
  pyrun() {
    (unset DOLI_RPC_ADMIN_TOKEN
     [ "$1" = set ] && export DOLI_RPC_ADMIN_TOKEN="$TOK_ENV"
     [ "$1" = empty ] && export DOLI_RPC_ADMIN_TOKEN=""
     [ "$1" = short ] && export DOLI_RPC_ADMIN_TOKEN="legacy-short"
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
  o="$(pyrun short "$D_FILE")"; [ "$o" = "$exp_f" ] && pass "j py short env → file token" || fail "j py short env → file token" "$o"
  # k. INC-I-308 P3-b — Decision: a FAIL means `--data-dir=/x` is missed, so no token is sent.
  # k2. send-back 1 — Decision: a FAIL means a listener whose uid does not own the token file is accepted (forged --data-dir).
  KD="$(mktemp -d)"; printf 'x\n' > "$KD/rpc-admin.token"
  pyport() { python3 -I -c 'import os, sys, types, importlib.util as u
s = u.spec_from_file_location("rpc_admin_token", sys.argv[1]); m = u.module_from_spec(s); s.loader.exec_module(m)
uid = os.stat(sys.argv[2] + "/rpc-admin.token").st_uid + int(sys.argv[3])
def run(argv, **kw):
    out = "4242\n" if argv[0] == "lsof" else ("%d\n" % uid if argv[2] == "uid=" else "doli-node --data-dir=%s run\n" % sys.argv[2])
    return types.SimpleNamespace(stdout=out)
m.subprocess.run = run
print(m.data_dir_for_port(1))' "$ROOT/$PYHELPER" "$KD" "$1" 2>&1; }
  o="$(pyport 0)"; [ "$o" = "$KD" ] && pass "k py data_dir_for_port --data-dir=/x" || fail "k py data_dir_for_port --data-dir=/x" "got: $o"
  o="$(pyport 1)"; [ "$o" = "None" ] && pass "k2 py listener uid != token file uid -> rejected" || fail "k2 py listener uid != token file uid -> rejected" "got: $o"
  shport() { (set +eu; source "$ROOT/scripts/lib/rpc-admin-token.sh"
    local u; u=$(( $(stat -c %u "$KD/rpc-admin.token" 2>/dev/null || stat -f %u "$KD/rpc-admin.token") + $1 ))
    lsof() { echo 4242; }
    ps() { case "$*" in *uid=*) echo " $u" ;; *) echo "doli-node --data-dir=$KD run" ;; esac; }
    rpc_admin_port_data_dir 1); }
  o="$(shport 0)"; [ "$o" = "$KD" ] && pass "k3 sh listener uid owns token file -> accepted" || fail "k3 sh listener uid owns token file -> accepted" "got: $o"
  o="$(shport 1)"; [ -z "$o" ] && pass "k4 sh listener uid != token file uid -> rejected" || fail "k4 sh listener uid != token file uid -> rejected" "got: $o"
  rm -rf "$KD"
else
  echo "SKIP f $PYHELPER absent"
fi

# l. INC-I-308 M13 — Decision: a FAIL means a data dir with spaces, or a node started without
#    --data-dir, yields no token path, so the admin call gets -32008.
SP="$TMP/home/Library/Application Support/doli/testnet"; mkdir -p "$SP"
if [ "$(uname -s)" = Darwin ]; then DEF="$SP"; else DEF="$TMP/home/.doli/testnet"; mkdir -p "$DEF"; fi
ENVD="$TMP/env dir"
L_CASES=(
  "space|doli-node --network testnet --data-dir $SP run --producer|$SP"
  "eq-space|doli-node --data-dir=$SP run|$SP"
  "flag-after|doli-node run --data-dir $SP --rpc-port 8500|$SP"
  "no-dir-default|doli-node --network testnet run --rpc-port 8500|$DEF"
  "no-dir-env|doli-node -n testnet run|$ENVD"
)
for c in "${L_CASES[@]}"; do
  IFS='|' read -r name cmd want <<< "$c"
  envset=""; [ "$name" = no-dir-env ] && envset="$ENVD"
  o="$( (export HOME="$TMP/home"; unset DOLI_DATA_DIR; [ -n "$envset" ] && export DOLI_DATA_DIR="$envset"
         source "$ROOT/$HELPER" >/dev/null 2>&1 || exit 97; rpc_admin_cmd_data_dir "$cmd") 2>/dev/null)"
  [ "$o" = "$want" ] && pass "l sh $name" || fail "l sh $name" "got: [$o] want: [$want]"
  if [ -f "$PYHELPER" ]; then
    o="$( (export HOME="$TMP/home"; unset DOLI_DATA_DIR; [ -n "$envset" ] && export DOLI_DATA_DIR="$envset"
           python3 -I -c 'import sys, importlib.util as u
s = u.spec_from_file_location("rpc_admin_token", sys.argv[1]); m = u.module_from_spec(s); s.loader.exec_module(m)
print(m.data_dir_from_command(sys.argv[2]))' "$ROOT/$PYHELPER" "$cmd") 2>&1)"
    [ "$o" = "$want" ] && pass "l py $name" || fail "l py $name" "got: [$o] want: [$want]"
  fi
done

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

# i. rpc_admin_header_for: local token file only for a loopback URL host
# REQ-RPC-ADMIN-LOOPBACK — Decision: a FAIL means the helper sends the local node's admin token to a remote or look-alike host.
hdr() {
  ( unset DOLI_RPC_ADMIN_TOKEN
    [ "$1" = set ] && export DOLI_RPC_ADMIN_TOKEN="$TOK_ENV"
    [ "$1" = short ] && export DOLI_RPC_ADMIN_TOKEN="legacy-short"
    source "$ROOT/$HELPER" >/dev/null 2>&1 || exit 97
    rpc_admin_port_data_dir() { printf '%s\n' "$D_FILE"; }
    rpc_admin_header_for pauseProduction "$2" ) 2>/dev/null
}
for u in "http://203.0.113.9:8501" "10.0.0.5:8501" "http://localhost@203.0.113.9:8501" "http://127.0.0.1.evil.example:8501" "http://8545"; do
  o="$(hdr unset "$u")"; [ -z "$o" ] && pass "i remote $u, env unset → no header" || fail "i remote $u, env unset → no header" "got: $(show "$o")"
  o="$(hdr set "$u")"; [ "$o" = "Authorization: Bearer $TOK_ENV" ] && pass "i remote $u, env set → env token" || fail "i remote $u, env set → env token" "got: $(show "$o")"
done
for u in "http://127.0.0.1:8501" "localhost:8501" "http://[::1]:8501" "8545"; do
  o="$(hdr unset "$u")"; [ "$o" = "Authorization: Bearer $TOK_FILE" ] && pass "i loopback $u → file token" || fail "i loopback $u → file token" "got: $(show "$o")"
done
o="$(hdr short 8545)"; [ "$o" = "Authorization: Bearer $TOK_FILE" ] && pass "j loopback 8545, short env → file token" || fail "j loopback 8545, short env → file token" "got: $(show "$o")"
o="$(hdr short http://203.0.113.9:8501)"; [ -z "$o" ] && pass "j remote, short env → no header" || fail "j remote, short env → no header" "got: $(show "$o")"

echo "---"
[ "$FAILS" = 0 ] && { echo "ALL PASS"; exit 0; }
echo "$FAILS check(s) failed"; exit 1
