#!/usr/bin/env bash
# gauntlet-devnet-lib.sh — shared ephemeral-devnet harness for the gauntlet runners. A runner
# sources it after setting its globals; the file defines functions only.
# Inputs (set by the runner): DN_TAG (scratch prefix and SKIP tag), S (scratch from the runner's
#   own mktemp -d "$TMPROOT/$DN_TAG.XXXXXX"), N nodes, P genesis producers (nodes 0..P-1),
#   DN_BIN[i], DN_ENV (NAME=VALUE array for every node), DN_LOG[i] (default $S/n<i>/node.log),
#   DN_KEY[i] (producer wallet or empty; gen_wallets fills 0..P-1), DN_CLI, DN_CLI_ENV (NAME=VALUE
#   array for cli_as), DN_FORGE, DN_SIB_PK (sibling target, empty = any producer), DN_SIB_MIN
#   (lowest sibling height), DN_PHASE_MAX (s), DN_KEEP (1 = keep scratch), DN_SLOT (s),
#   DN_GEN_TS, DN_REWARD, DN_BOND_UNIT, DN_BONDS[i] (genesis bond count, default 1).
# Outputs: DN_P2P DN_RPC DN_MET [i] (check_ports), DN_PK[i] (gen_wallets), DN_PID[i] DN_PIDS
#   (start_node), DN_SAMPLER_PID, DN_TARGET_IDX DN_TARGET_PK DN_INJ_H DN_INJ_SLOT DN_OWN_HASH
#   DN_SIB_HASH DN_BOND (inject_sibling).
# SAFETY: signals only PIDs it started whose command line names $S, removes only $S, binds and
#   dials 127.0.0.1 only. Every RPC-derived value is checked (64-hex, digits, base64) before use.

skip() { echo "SKIP ${DN_TAG:-devnet}: $*"; exit 2; }

is_num() { case "${1:-}" in '' | *[!0-9]*) return 1 ;; esac; }
is_hex() { case "${1:-}" in '' | *[!0-9a-f]*) return 1 ;; esac; [ "${#1}" = 64 ]; }
is_b64() { case "${1:-}" in '' | *[!A-Za-z0-9+/=]*) return 1 ;; esac; }
env_ok() {
  local e
  for e in "$@"; do case "$e" in [A-Za-z_]*=*) ;; *) return 1 ;; esac; done
  return 0
}

all_equal() {
  local v
  [ "$#" -ge 2 ] || return 1
  for v in "$@"; do { [ -n "$v" ] && [ "$v" = "$1" ]; } || return 1; done
}

rpc() { curl -s -m 8 "http://127.0.0.1:$1" -H 'Content-Type: application/json' -d "$2" 2>/dev/null; }
call() { local p="${3:-}"; [ -n "$p" ] || p='{}'; rpc "$1" "{\"jsonrpc\":\"2.0\",\"method\":\"$2\",\"params\":$p,\"id\":1}"; }
port_busy() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
udp_busy() { lsof -nP -iUDP:"$1" >/dev/null 2>&1; }
alive() {
  local st
  st="$(ps -p "${1:-}" -o stat= 2>/dev/null | tr -d ' ')"
  [ -n "$st" ] && [ "${st#Z}" = "$st" ]
}
ours() { [ -n "${S:-}" ] && ps -p "$1" -o command= 2>/dev/null | grep -qF -- "$S"; }

dn_py() { printf '%s/gauntlet-devnet.py' "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; }
devnet_py() { python3 -I "$(dn_py)" "$@"; }

# Prints cargo|rustc when a build of this checkout runs (names its root, not a worktree under it).
build_running() {
  local r
  r="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  ps -axww -o args= 2>/dev/null | awk -v r="$r/" -v w="$r/.claude/worktrees/" '
    { b = $1; sub(/.*\//, "", b) }
    (b == "cargo" || b == "rustc") && index($0, r) && !index($0, w) { print b; exit }'
}

# result.<key> from a JSON-RPC body on stdin.
jfield() {
  python3 -I -c '
import json, sys
try:
    d = json.load(sys.stdin)
    print((d.get("result", d) if isinstance(d, dict) else d)[sys.argv[1]])
except Exception:
    pass' "$1" 2>/dev/null
}

# One line per element of the JSON list <key> on stdin (list elements joined by spaces).
jlines() {
  python3 -I -c '
import json, sys
try:
    v = json.load(sys.stdin).get(sys.argv[1])
except Exception:
    v = None
for e in v if isinstance(v, list) else []:
    print(" ".join(str(x) for x in e) if isinstance(e, list) else e)' "$1" 2>/dev/null
}

best_height() { local h; h="$(call "$1" getChainInfo | jfield bestHeight)"; is_num "$h" && echo "$h"; }
block_json() { is_num "$2" && call "$1" getBlockByHeight "{\"height\":$2}"; }
block_hash() { local h; h="$(block_json "$1" "$2" | jfield hash)"; is_hex "$h" && echo "$h"; }
block_raw() {
  local r
  is_num "$2" || return 1
  r="$(call "$1" getBlockRaw "{\"height\":$2}" | jfield block)"
  is_b64 "$r" && echo "$r"
}
producer_json() { is_hex "$2" && call "$1" getProducer "{\"publicKey\":\"$2\"}"; }

stop_one() {
  local PID="${1:-}" n=0
  [ -n "$PID" ] || return 0
  if alive "$PID" && ours "$PID"; then
    kill -TERM "$PID" 2>/dev/null
    while alive "$PID" && [ "$n" -lt 150 ]; do sleep 0.2; n=$((n + 1)); done
    alive "$PID" && ours "$PID" && kill -KILL "$PID" 2>/dev/null
  fi
  wait "$PID" 2>/dev/null
}

# shellcheck disable=SC2329  # invoked via the runner's EXIT trap
cleanup() {
  local PID
  for PID in ${DN_SAMPLER_PID:-} ${DN_PIDS:-}; do stop_one "$PID"; done
  { [ -n "${S:-}" ] && [ -n "${DN_TAG:-}" ]; } || return 0
  [ "${DN_KEEP:-0}" = 1 ] && echo "kept $S" && return 0
  case "$S" in */"$DN_TAG".*) rm -rf "$S" ;; esac
}

node_log() { printf '%s' "${DN_LOG[$1]:-$S/n$1/node.log}"; }

start_node() {
  local idx="$1" dd="$S/n$1/data" bin="${DN_BIN[$1]:-}"
  [ -x "$bin" ] || skip "node binary for node $idx missing or not executable: $bin"
  env_ok ${DN_ENV[@]+"${DN_ENV[@]}"} || skip "DN_ENV holds an entry that is not NAME=VALUE"
  # Nested data dir: node_key lives in its parent, so each node gets its own peer id.
  mkdir -p "$dd" "$S/n$idx/blk"
  : >"$dd/.env"
  local args=(--network devnet --data-dir "$dd" --log-level info run
    --chainspec "$S/chainspec.json"
    --p2p-port "${DN_P2P[$idx]}" --rpc-port "${DN_RPC[$idx]}" --metrics-port "${DN_MET[$idx]}"
    --no-auto-update --force-start --yes)
  [ -n "${DN_KEY[$idx]:-}" ] && args+=(--producer --producer-key "${DN_KEY[$idx]}")
  [ "$idx" -gt 0 ] && args+=(--bootstrap "/ip4/127.0.0.1/tcp/${DN_P2P[0]}")
  env ${DN_ENV[@]+"${DN_ENV[@]}"} "$bin" "${args[@]}" >>"$(node_log "$idx")" 2>&1 &
  DN_PID[idx]=$!
  DN_PIDS="${DN_PIDS:-} ${DN_PID[$idx]}"
}

# start_cluster DEADLINE: node 0, wait for its RPC (until epoch second DEADLINE), then nodes 1..N-1.
start_cluster() {
  local i until_ts="${1:-}"
  is_num "$until_ts" || skip "start_cluster needs a deadline in epoch seconds"
  start_node 0
  until [ -n "$(best_height "${DN_RPC[0]}")" ] || [ "$(date +%s)" -ge "$until_ts" ]; do
    alive "${DN_PID[0]}" || skip "node 0 exited before RPC came up (see $(node_log 0))"
    sleep 1
  done
  for ((i = 1; i < N; i++)); do start_node "$i"; sleep 1; done
}

# restart_node I [BIN] [LOG]: stop node I (own PID only), then start it on the same data dir.
restart_node() {
  is_num "${1:-}" || return 2
  stop_one "${DN_PID[$1]:-}"
  [ -z "${2:-}" ] || DN_BIN[$1]="$2"
  [ -z "${3:-}" ] || DN_LOG[$1]="$3"
  start_node "$1"
}

# check_ports P2P_BASE RPC_BASE MET_BASE: node i gets base + i on each; SKIP if any port is busy.
check_ports() {
  local i p
  { is_num "${1:-}" && is_num "${2:-}" && is_num "${3:-}"; } || skip "port bases must be digits"
  command -v lsof >/dev/null 2>&1 || skip "lsof unavailable"
  for ((i = 0; i < N; i++)); do
    DN_P2P[i]=$(($1 + i))
    DN_RPC[i]=$(($2 + i))
    DN_MET[i]=$(($3 + i))
    for p in "${DN_P2P[$i]}" "${DN_RPC[$i]}" "${DN_MET[$i]}"; do port_busy "$p" && skip "TCP port $p is busy"; done
    udp_busy $((DN_P2P[i] + 1)) && skip "UDP discovery port $((DN_P2P[i] + 1)) is busy"
  done
  return 0
}

# gen_wallets [COUNT]: wallets $S/w<i>.json (default P) -> DN_PK[i]; DN_KEY[i] for producers.
gen_wallets() {
  local i k="${1:-$P}"
  is_num "$k" || skip "gen_wallets count must be digits"
  for ((i = 0; i < k; i++)); do
    "$DN_CLI" -w "$S/w$i.json" new -n "${DN_TAG}n$i" >/dev/null 2>&1 || skip "wallet generation failed for producer $i"
    DN_PK[i]="$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["addresses"][0]["public_key"])' "$S/w$i.json" 2>/dev/null)"
    [ -n "${DN_PK[$i]}" ] || skip "could not read pubkey for producer $i"
    [ "$i" -ge "$P" ] || [ -n "${DN_KEY[$i]:-}" ] || DN_KEY[i]="$S/w$i.json"
  done
  return 0
}

# write_chainspec NAME MESSAGE: $S/chainspec.json from DN_GEN_TS DN_SLOT DN_REWARD DN_BOND_UNIT DN_PK DN_BONDS.
write_chainspec() {
  local i
  local -a specs
  for ((i = 0; i < P; i++)); do specs[i]="${DN_PK[$i]}:${DN_BONDS[$i]:-1}"; done
  devnet_py chainspec "$S/chainspec.json" "$DN_GEN_TS" "$DN_SLOT" "$DN_REWARD" "$DN_BOND_UNIT" "$1" "$2" \
    ${specs[@]+"${specs[@]}"} >/dev/null
}

heights() { local i; for ((i = 0; i < N; i++)); do best_height "${DN_RPC[$i]}"; done; }
min_height() {
  local lo="" h i
  for ((i = 0; i < N; i++)); do
    h="$(best_height "${DN_RPC[$i]}")"
    is_num "$h" || return 1
    { [ -z "$lo" ] || [ "$h" -lt "$lo" ]; } && lo="$h"
  done
  echo "$lo"
}
# Block hash at height $1 on every node; "-" where a node returns none.
hashes_at() { local i h; for ((i = 0; i < N; i++)); do h="$(block_hash "${DN_RPC[$i]}" "$1")"; printf '%s ' "${h:--}"; done; }
all_alive() { local i; for ((i = 0; i < N; i++)); do alive "${DN_PID[$i]:-}" || return 1; done; }

# Nodes whose latest schedule line shows a G-Set of all P producers.
gset_ready() {
  local i k=0 last
  for ((i = 0; i < N; i++)); do
    last="$(grep -E 'Producer schedule (view|DIVERGENCE)' "$(node_log "$i")" | tail -1)"
    case "$last" in *"(count=$P, source=gset)"* | *"] (count=$P) vs known="*) k=$((k + 1)) ;; esac
  done
  echo "$k"
}

# Stop the cluster once every producer persisted its own announcement, then write the union of
# the producer_gset.bin files to every node.
seed_gset() {
  local i t0 PID
  for ((i = 0; i < P; i++)); do
    t0="$(date +%s)"
    until [ -s "$S/n$i/data/producer_gset.bin" ]; do [ $(($(date +%s) - t0)) -ge 30 ] && return 1; sleep 1; done
  done
  for PID in ${DN_PIDS:-}; do stop_one "$PID"; done
  DN_PIDS=""
  devnet_py gset-merge "$S" "$N" "$P" >/dev/null
}

# One poll of every node into $S/n<i>/samples.txt (S supply, M mempool slashes, T tip); a no-op
# while the background sampler runs.
sample() { [ -n "${DN_SAMPLER_PID:-}" ] || devnet_py sample "$S" 0 "${DN_RPC[@]}" >/dev/null 2>&1; }
sampler_start() {
  [ -z "${DN_SAMPLER_PID:-}" ] || return 0
  python3 -I "$(dn_py)" sample "$S" 2 "${DN_RPC[@]}" >>"$S/sampler.log" 2>&1 &
  DN_SAMPLER_PID=$!
}
sampler_stop() { [ -z "${DN_SAMPLER_PID:-}" ] || stop_one "$DN_SAMPLER_PID"; DN_SAMPLER_PID=""; }

# Most frequent sampled supply of node $1 at height $2.
supply_at() {
  awk -v h="$2" '$1 == "S" && $2 == h { c[$3]++ }
    END { b = 0; for (v in c) if (c[v] > b) { b = c[v]; m = v }; if (b) print m }' "$S/n$1/samples.txt" 2>/dev/null
}
# Highest sampled value under key $2 (S = height, M = mempool slashes) on node $1.
max_of() { awk -v k="$2" '$1 == k && $2 > m { m = $2 } END { print m + 0 }' "$S/n$1/samples.txt" 2>/dev/null; }

# wait_height H MAX: poll until every node is at height >= H; rc 1 on timeout or a dead node.
wait_height() {
  local mh t0
  t0="$(date +%s)"
  while [ $(($(date +%s) - t0)) -lt "$2" ]; do
    sample
    mh="$(min_height)" && [ "$mh" -ge "$1" ] && return 0
    all_alive || return 1
    sleep 2
  done
  return 1
}

# wait_tx_mined TXID MAX: print the one height every node reports for TXID; rc 1 timeout, 2 bad input.
wait_tx_mined() {
  local tx="${1:-}" max="${2:-}" t0 i h hs
  { is_hex "$tx" && is_num "$max"; } || return 2
  t0="$(date +%s)"
  while [ $(($(date +%s) - t0)) -lt "$max" ]; do
    sample
    hs=""
    for ((i = 0; i < N; i++)); do
      h="$(call "${DN_RPC[$i]}" getTransaction "{\"hash\":\"$tx\"}" | jfield blockHeight)"
      is_num "$h" || { hs=""; break; }
      hs="$hs $h"
    done
    # shellcheck disable=SC2086
    if [ -n "$hs" ] && all_equal $hs; then echo "${hs##* }"; return 0; fi
    sleep 2
  done
  return 1
}

# cli_as W NODE ARGS...: the CLI with wallet $S/w<W>.json, the DN_CLI_ENV gates and node NODE's RPC.
cli_as() {
  local w="${1:-}" n="${2:-}"
  { is_num "$w" && is_num "$n" && [ -n "${DN_RPC[$n]:-}" ]; } || return 2
  shift 2
  env_ok ${DN_CLI_ENV[@]+"${DN_CLI_ENV[@]}"} || return 2
  env ${DN_CLI_ENV[@]+"${DN_CLI_ENV[@]}"} "$DN_CLI" -w "$S/w$w.json" --rpc "http://127.0.0.1:${DN_RPC[$n]}" -n devnet "$@"
}

# First block >= DN_SIB_MIN by DN_SIB_PK (empty = any producer node) that every node holds:
# publish its signed sibling, at most 3 forge attempts within DN_PHASE_MAX s.
# shellcheck disable=SC2046
inject_sibling() {
  local t0 tip blk prod i idx tried=0 attempts=0 n raw out dial="" rc own slot bond sib
  { is_num "${DN_SIB_MIN:-}" && is_num "${DN_PHASE_MAX:-}"; } || return 2
  for ((i = 0; i < N; i++)); do dial="$dial,/ip4/127.0.0.1/tcp/${DN_P2P[$i]}"; done
  t0="$(date +%s)"
  while [ $(($(date +%s) - t0)) -lt "$DN_PHASE_MAX" ] && [ "$attempts" -lt 3 ]; do
    sample
    tip="$(best_height "${DN_RPC[0]}")"
    is_num "$tip" || tip=0
    { [ "$tip" -lt "$DN_SIB_MIN" ] || [ "$tip" -le "$tried" ]; } && { sleep 1; continue; }
    tried="$tip"
    blk="$(block_json "${DN_RPC[0]}" "$tip")"
    prod="$(printf '%s' "$blk" | jfield producer)"
    own="$(printf '%s' "$blk" | jfield hash)"
    slot="$(printf '%s' "$blk" | jfield slot)"
    { is_hex "$prod" && is_hex "$own" && is_num "$slot"; } || continue
    [ -z "${DN_SIB_PK:-}" ] || [ "$prod" = "$DN_SIB_PK" ] || continue
    idx=-1
    for ((i = 0; i < P; i++)); do [ "${DN_PK[$i]}" = "$prod" ] && idx="$i"; done
    { [ "$idx" -ge 0 ] && [ -n "${DN_KEY[$idx]:-}" ]; } || continue
    n=0
    until all_equal "$own" $(hashes_at "$tip") || [ "$n" -ge $((DN_SLOT * 2)) ]; do sleep 0.5; n=$((n + 1)); done
    all_equal "$own" $(hashes_at "$tip") || continue
    bond="$(producer_json "${DN_RPC[0]}" "$prod" | jfield bondAmount)"
    is_num "$bond" || bond=""
    raw="$(block_raw "${DN_RPC[0]}" $((tip - 1)))"
    attempts=$((attempts + 1))
    out="$("$DN_FORGE" --kind sibling --tip-b64 "$raw" --height "$tip" --slot "$slot" \
      --producer "$prod" --key-file "${DN_KEY[$idx]}" --chainspec "$S/chainspec.json" \
      --differ-from "$own" --genesis-time "$DN_GEN_TS" --slot-duration "$DN_SLOT" \
      --dial "${dial#,}" 2>>"$S/forge.log")"
    rc=$?
    sib="$(printf '%s\n' "$out" | sed -n 's/^INJECTED .*hash=\([0-9a-f]*\).*/\1/p')"
    # shellcheck disable=SC2034
    if [ "$rc" = 0 ] && is_hex "$sib"; then
      DN_TARGET_IDX="$idx" DN_TARGET_PK="$prod" DN_INJ_H="$tip" DN_INJ_SLOT="$slot"
      DN_OWN_HASH="$own" DN_SIB_HASH="$sib" DN_BOND="$bond"
      return 0
    fi
    echo "$DN_TAG: forge attempt $attempts at h=$tip rc=$rc (see forge.log)"
  done
  return 1
}
