#!/usr/bin/env bash
# gauntlet-rpc-admin-lib.sh — shared helpers for the INC-I-308 RPC admin access rows
# (GS-056, GS-057, GS-058, GS-061). Sourced, never run.
#
# SAFETY: the helpers only READ from the running fleet (RPC POSTs of read-only methods, ps, lsof).
# A scratch node is a `--network testnet` node with its OWN data dir under $TMPDIR, its OWN ports,
# no bootstrap (`--no-dht` clears the default list, no `--bootstrap`), `--no-discv5`, no auto
# update, no snap sync, and the loopback bind shim (scripts/gauntlet-stranded/loopback_bind.c) so
# its P2P socket is on 127.0.0.1. The runner stops it by its own PID and checks that it is gone.
# Nothing under ~/testnet is written.
#
# Verdict protocol (same as the other standalone rows): METRIC lines, then
#   VERDICT: PASS exit 0 · VERDICT: FAIL exit 1 · VERDICT: NOT_MADE exit 3.

GA_ROOT="${GA_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
GA_BIN="${GA_BIN:-$GA_ROOT/target/release/doli-node}"
GA_WORK="${GA_WORK:-${TMPDIR:-/tmp}/gs-rpc-admin-$$}"
GA_UNAUTH=-32008
SCRATCH_PID=""
SCRATCH_DIR=""
SCRATCH_RPC=""

ga_metric(){ echo "METRIC $*"; }
ga_note(){ echo "  · $*"; }

# ga_rpc URL METHOD PARAMS_JSON [TOKEN] -> response body ("" on no connection)
ga_rpc(){
  local url="$1" m="$2" p="${3:-}" tok="${4:-}"
  [ -z "$p" ] && p='{}'
  local body="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$m\",\"params\":$p}"
  if [ -n "$tok" ]; then
    # Token goes through a header file, never argv.
    local hf; hf="$(mktemp "${GA_WORK}/hdr.XXXXXX")"
    printf 'Authorization: Bearer %s\n' "$tok" > "$hf"
    curl -s -m "${GA_RPC_TIMEOUT:-20}" -X POST -H 'Content-Type: application/json' -H "@$hf" -d "$body" "$url" 2>/dev/null
    rm -f "$hf"
  else
    curl -s -m "${GA_RPC_TIMEOUT:-20}" -X POST -H 'Content-Type: application/json' -d "$body" "$url" 2>/dev/null
  fi
}

# ga_code BODY -> "ok" (result present) | error code | "none" (no/invalid answer)
ga_code(){
  printf '%s' "$1" | python3 -I -c '
import json,sys
try: d=json.loads(sys.stdin.read())
except Exception: print("none"); sys.exit()
if isinstance(d,dict) and "error" in d and d["error"] is not None: print(d["error"].get("code"))
elif isinstance(d,dict) and "result" in d: print("ok")
else: print("none")'
}

# ga_admin_methods -> the ADMIN_METHODS names from crates/rpc/src/server.rs, one per line
ga_admin_methods(){
  awk '/pub const ADMIN_METHODS/{f=1;next} f&&/\];/{exit} f' "$GA_ROOT/crates/rpc/src/server.rs" \
    | grep -o '^[[:space:]]*"[A-Za-z]*"' | tr -d ' "'
}

ga_port_free(){ ! lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }

# ga_shim -> path of a built loopback_bind.dylib ("" when clang is missing)
ga_shim(){
  local out="$GA_WORK/loopback_bind.dylib"
  [ -f "$out" ] && { echo "$out"; return; }
  command -v clang >/dev/null 2>&1 || { echo ""; return; }
  clang -dynamiclib -o "$out" "$GA_ROOT/scripts/gauntlet-stranded/loopback_bind.c" >/dev/null 2>&1 && echo "$out" || echo ""
}

# ga_scratch_start TAG RPC P2P METRICS [extra env assignments...]
ga_scratch_start(){
  local tag="$1" rpc="$2" p2p="$3" met="$4"; shift 4
  local port
  for port in "$rpc" "$p2p" "$met"; do
    ga_port_free "$port" || { echo "port $port busy"; return 3; }
  done
  [ -x "$GA_BIN" ] || { echo "no binary $GA_BIN"; return 3; }
  SCRATCH_DIR="$GA_WORK/$tag"; mkdir -p "$SCRATCH_DIR/data"
  local shim; shim="$(ga_shim)"
  ( cd "$SCRATCH_DIR" && exec env -u DOLI_RPC_ADMIN_TOKEN -u DOLI_RPC_TRUSTED_PROXIES "$@" \
      ${shim:+DYLD_INSERT_LIBRARIES=$shim} "$GA_BIN" --network testnet --data-dir "$SCRATCH_DIR/data" \
      --log-level info run --p2p-port "$p2p" --rpc-port "$rpc" --rpc-bind 127.0.0.1 \
      --metrics-port "$met" --no-dht --no-discv5 --no-auto-update --no-snap-sync --yes \
      > "$SCRATCH_DIR/node.log" 2>&1 ) &
  SCRATCH_PID=$!
  SCRATCH_RPC="http://127.0.0.1:$rpc"
  echo "SCRATCH_STARTED pid=$SCRATCH_PID tag=$tag rpc=$rpc p2p=$p2p metrics=$met shim=${shim:-none} dir=$SCRATCH_DIR"
  local i
  for i in $(seq 1 90); do
    kill -0 "$SCRATCH_PID" 2>/dev/null || { echo "scratch node exited early"; tail -5 "$SCRATCH_DIR/node.log"; return 3; }
    [ "$(ga_code "$(ga_rpc "$SCRATCH_RPC" getChainInfo '{}')")" = "ok" ] && return 0
    sleep 1
  done
  echo "scratch RPC never answered"; return 3
}

# ga_scratch_stop -> stops SCRATCH_PID, proves it is gone, deletes its data dir (keeps node.log)
ga_scratch_stop(){
  [ -n "$SCRATCH_PID" ] || return 0
  kill -TERM "$SCRATCH_PID" 2>/dev/null
  local i; for i in $(seq 1 30); do kill -0 "$SCRATCH_PID" 2>/dev/null || break; sleep 1; done
  kill -0 "$SCRATCH_PID" 2>/dev/null && kill -KILL "$SCRATCH_PID" 2>/dev/null && sleep 1
  if kill -0 "$SCRATCH_PID" 2>/dev/null; then echo "SCRATCH_STOPPED pid=$SCRATCH_PID gone=no"
  else echo "SCRATCH_STOPPED pid=$SCRATCH_PID gone=yes"; fi
  [ -n "$SCRATCH_DIR" ] && rm -rf "$SCRATCH_DIR/data"
  SCRATCH_PID=""
}

# ga_fleet_pids -> PIDs of running doli-node processes (read-only)
ga_fleet_pids(){ pgrep -f 'doli-node.* run' 2>/dev/null; }

# ga_argv PID -> full argv of PID
ga_argv(){ ps -o command= -p "$1" 2>/dev/null; }

# ga_flag ARGV FLAG -> value after FLAG ("" when absent)
ga_flag(){ printf '%s\n' "$1" | tr ' ' '\n' | awk -v f="$2" 'p{print;exit} $0==f{p=1}'; }

# ga_listen_addrs PID PORT -> local listen addresses of PID on PORT (e.g. 127.0.0.1:8501)
ga_listen_addrs(){ lsof -nP -a -p "$1" -iTCP:"$2" -sTCP:LISTEN 2>/dev/null | awk 'NR>1{print $9}'; }

ga_is_loopback(){ case "$1" in 127.*|\[::1\]*|localhost*) return 0 ;; *) return 1 ;; esac; }

# ga_env_has_token PID -> "env" when DOLI_RPC_ADMIN_TOKEN is in the process env (value never printed)
ga_env_has_token(){ ps eww -o command= -p "$1" 2>/dev/null | grep -q 'DOLI_RPC_ADMIN_TOKEN=' && echo env || echo ""; }
