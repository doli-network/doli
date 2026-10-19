#!/usr/bin/env bash
# gauntlet-gs023.sh — GS-023 "archive-node-cold-restart" (INC-I-249).
# REQ-ARCH-010 — Decision: a FAIL means an archive node still cannot serve RPC during its cold-start archive check, so the INC-I-249 fix must not ship.
# Cold-starts a SCRATCH non-producer archive node from clones of the local testnet
# seed's newest checkpoint and block archive, probes its RPC the way
# scripts/doli-watchdog.sh does, then stops it with SIGTERM. Standalone, no args.
#
# OUTPUT CONTRACT: one `PASS|FAIL <id> <measured numbers>` line per assertion, then
# the METRIC line (last line), then the exit code.
#   gs023-rpc-30s          T_ready (T0 -> first getChainInfo answer) <= 30 s
#   gs023-rpc-10s          T_ready <= 10 s
#   gs023-second-call-1s   next call < 1000 ms AND an archive check is running
#                          (catch-up START markers > FINISH markers in node.log)
#   gs023-no-failed-probe  0 failed probes after the first answer
#   gs023-no-respawn       the scratch PID stays alive and unchanged all window
#   gs023-reaches-tip      node height >= seed height - 2 within GS023_TIP_MAX s of T0
#   gs023-clean-shutdown   SIGTERM -> process exits within 30 s
#   METRIC T_ready_s= second_call_ms= time_to_tip_s= shutdown_s=   (<n>|none)
#   exit: 0 all PASS · 1 any FAIL · 2 SKIP
#   setup failure after the clones (verified_height not stripped): `FAIL gs023-setup <why>`, exit 1, no METRIC.
# INPUT PARTITIONS: base binary (catch-up before RPC -> rpc-10s FAIL) · fixed binary
#   (background check -> all PASS) · precondition missing (`SKIP gs023: <reason>`, exit 2).
# Env: GS023_BINARY GS023_TESTNET_DIR GS023_CHECKPOINT GS023_ARCHIVE_SRC GS023_SEED_RPC
#   GS023_SEED_P2P GS023_P2P_PORT GS023_RPC_PORT GS023_METRICS_PORT GS023_PROBE_TIMEOUT
#   GS023_READY_WAIT GS023_TIP_MAX GS023_HOLD GS023_MIN_ARCHIVE GS023_KEEP (1 = keep scratch).
# SAFETY: never stops, signals or replaces a testnet node; never touches launchd;
#   the testnet dir is only read (APFS clone or copy, never hard links); only the PID
#   this script started is signalled; the scratch dir under $TMPDIR is deleted on exit.

set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GS023_BINARY="${GS023_BINARY:-$ROOT/target/release/doli-node}"
GS023_TESTNET_DIR="${GS023_TESTNET_DIR:-$HOME/testnet}"
GS023_ARCHIVE_SRC="${GS023_ARCHIVE_SRC:-$GS023_TESTNET_DIR/seed/blocks}"
GS023_SEED_RPC="${GS023_SEED_RPC:-http://127.0.0.1:8500}"
GS023_SEED_P2P="${GS023_SEED_P2P:-/ip4/127.0.0.1/tcp/30300}"
GS023_P2P_PORT="${GS023_P2P_PORT:-30390}"
GS023_RPC_PORT="${GS023_RPC_PORT:-8590}"
GS023_METRICS_PORT="${GS023_METRICS_PORT:-9090}"
_wd_timeout="$(sed -n 's/^TIMEOUT=\([0-9][0-9]*\).*/\1/p' "$ROOT/scripts/doli-watchdog.sh" 2>/dev/null | head -1)"
GS023_PROBE_TIMEOUT="${GS023_PROBE_TIMEOUT:-${_wd_timeout:-5}}"
GS023_READY_WAIT="${GS023_READY_WAIT:-600}"
GS023_TIP_MAX="${GS023_TIP_MAX:-300}"
GS023_HOLD="${GS023_HOLD:-60}"
GS023_MIN_ARCHIVE="${GS023_MIN_ARCHIVE:-100000}"
GS023_KEEP="${GS023_KEEP:-0}"

READY_30=30
READY_10=10
SECOND_MAX_MS=1000
TIP_SLACK=2
SHUTDOWN_MAX=30
START_RE='Archiver: catching up to height|\[ARCHIVER\] Catch-up: scanning for gaps|\[ARCHIVER\] Catch-up started'
FINISH_RE='Archiver: caught up|Archiver: already up to date|Archiver catch-up error|\[ARCHIVER\] Catch-up: filled|\[ARCHIVER\] Catch-up: archive complete|\[ARCHIVER\] Catch-up error|\[ARCHIVER\] Catch-up finished'
REQ='{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
LOCAL_RPC="http://127.0.0.1:$GS023_RPC_PORT"
TMPROOT="${TMPDIR:-/tmp}"
TMPROOT="${TMPROOT%/}"
S=""
PID=""
EVIDENCE_DONE=0

skip() {
  echo "SKIP gs023: $*"
  exit 2
}

if command -v perl >/dev/null 2>&1; then
  now_ms() { perl -MTime::HiRes=time -e 'printf "%d\n", time()*1000'; }
elif command -v python3 >/dev/null 2>&1; then
  now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
else
  skip "neither perl nor python3 for millisecond timing"
fi

secs() { awk -v a="$1" -v b="$2" 'BEGIN { printf "%.1f", (b - a) / 1000 }'; }
le() { awk -v x="$1" -v y="$2" 'BEGIN { exit !(x <= y) }'; }
lt() { awk -v x="$1" -v y="$2" 'BEGIN { exit !(x < y) }'; }
rpc() { curl -s -m "$GS023_PROBE_TIMEOUT" "$1" -H 'Content-Type: application/json' -d "$REQ" 2>/dev/null; }
ok_body() { case "$1" in *'"result"'*) return 0 ;; esac; return 1; }
height_of() { printf '%s' "$1" | grep -o '"bestHeight":[0-9]*' | head -1 | cut -d: -f2; }
port_busy() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
udp_busy() { lsof -nP -iUDP:"$1" >/dev/null 2>&1; }
count() {
  local n
  n="$(grep -cE "$1" "$S/node.log" 2>/dev/null)"
  echo "${n:-0}"
}
alive() {
  local st
  st="$(ps -p "$1" -o stat= 2>/dev/null | tr -d ' ')"
  [ -n "$st" ] && [ "${st#Z}" = "$st" ]
}
ours() { ps -p "$1" -o command= 2>/dev/null | grep -qF -- "$S"; }
# Prints cargo|rustc when a build of THIS checkout runs (its args name ROOT/, not a worktree under it).
build_running() {
  ps -axww -o args= 2>/dev/null | awk -v r="$ROOT/" -v w="$ROOT/.claude/worktrees/" '
    { b = $1; sub(/.*\//, "", b) }
    (b == "cargo" || b == "rustc") && index($0, r) && !index($0, w) { print b; exit }'
}

evidence() {
  [ "$EVIDENCE_DONE" = 1 ] && return 0
  EVIDENCE_DONE=1
  [ -n "$S" ] && [ -f "$S/node.log" ] || return 0
  echo "--- node.log archiver evidence (first 15 lines)"
  sed $'s/\x1b\\[[0-9;]*m//g' "$S/node.log" | grep -E '\[ARCHIVER\]|Archiver[: ]' |
    grep -v 'Archived block' | head -15
}

# shellcheck disable=SC2329
cleanup() {
  if [ -n "$PID" ] && [ -n "$S" ] && alive "$PID" && ours "$PID"; then
    kill -TERM "$PID" 2>/dev/null
    for _ in $(seq 1 150); do
      alive "$PID" || break
      sleep 0.2
    done
    alive "$PID" && ours "$PID" && kill -KILL "$PID" 2>/dev/null
  fi
  [ -n "$PID" ] && wait "$PID" 2>/dev/null
  evidence
  [ -n "$S" ] || return 0
  if [ "$GS023_KEEP" = 1 ]; then
    echo "kept $S"
    return 0
  fi
  case "$S" in "$TMPROOT"/gs023.?*) rm -rf "$S" ;; esac
}
trap cleanup EXIT
trap 'exit 1' INT TERM

clone() { cp -c -R "$1" "$2" 2>/dev/null || { rm -rf "$2" && cp -R "$1" "$2"; }; }

setup_fail() {
  echo "FAIL gs023-setup $*"
  exit 1
}

# Removes only `verified_height` from manifest $1 (exact ints, atomic rewrite); non-zero + message on failure.
gs023_strip_verified_height() {
  python3 -I - "$1" <<'PY'
import contextlib, json, os, sys, tempfile
path = sys.argv[1]
try:
    with open(path, encoding="utf-8") as f:
        m = json.load(f)
    if not isinstance(m, dict):
        raise ValueError("not a JSON object")
    if "verified_height" in m:
        del m["verified_height"]
        fd, tmp = tempfile.mkstemp(prefix=".manifest.", dir=os.path.dirname(os.path.abspath(path)))
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                json.dump(m, f, indent=2)
            os.replace(tmp, path)
        except BaseException:
            with contextlib.suppress(OSError):
                os.unlink(tmp)
            raise
    with open(path, encoding="utf-8") as f:
        if "verified_height" in json.load(f):
            raise ValueError("verified_height still present after the rewrite")
except Exception as e:
    sys.exit(f"gs023: cannot strip verified_height from {path}: {e}")
PY
}

# --- preconditions --------------------------------------------------------------
[ -n "$(build_running)" ] && skip "a build is running (cargo/rustc) in this checkout $ROOT"
[ -f "$GS023_BINARY" ] && [ -x "$GS023_BINARY" ] || skip "binary $GS023_BINARY missing or not executable"
for p in "$GS023_P2P_PORT" "$GS023_RPC_PORT" "$GS023_METRICS_PORT"; do
  port_busy "$p" && skip "TCP port $p is busy"
done
udp_busy $((GS023_P2P_PORT + 1)) && skip "UDP discovery port $((GS023_P2P_PORT + 1)) is busy"
ok_body "$(rpc "$GS023_SEED_RPC")" || skip "seed RPC $GS023_SEED_RPC does not answer getChainInfo"
if [ -z "${GS023_CHECKPOINT:-}" ]; then
  besth=-1
  GS023_CHECKPOINT=""
  for d in "$GS023_TESTNET_DIR"/seed/data/checkpoints/h*-*; do
    [ -d "$d" ] || continue
    n="${d##*/}"
    n="${n#h}"
    n="${n%%-*}"
    case "$n" in '' | *[!0-9]*) continue ;; esac
    [ "$n" -gt "$besth" ] && { besth="$n"; GS023_CHECKPOINT="$d"; }
  done
fi
[ -n "$GS023_CHECKPOINT" ] && [ -d "$GS023_CHECKPOINT" ] || skip "no checkpoint under $GS023_TESTNET_DIR/seed/data/checkpoints"
n_arch="$(find "$GS023_ARCHIVE_SRC" -maxdepth 1 -name '*.block' 2>/dev/null | wc -l | tr -d ' ')"
[ "${n_arch:-0}" -ge "$GS023_MIN_ARCHIVE" ] || skip "archive $GS023_ARCHIVE_SRC has ${n_arch:-0} .block files (< $GS023_MIN_ARCHIVE)"

# --- scratch node ---------------------------------------------------------------
S="$(mktemp -d "$TMPROOT/gs023.XXXXXX")" || skip "mktemp failed"
clone "$GS023_CHECKPOINT" "$S/data" || skip "cannot clone checkpoint $GS023_CHECKPOINT"
clone "$GS023_ARCHIVE_SRC" "$S/archive" || skip "cannot clone archive $GS023_ARCHIVE_SRC"
gs023_strip_verified_height "$S/archive/manifest.json" || setup_fail "verified_height not stripped from the archive clone: no background full check to measure"
echo "gs023: binary=$("$GS023_BINARY" --version 2>/dev/null) checkpoint=${GS023_CHECKPOINT##*/} archive_blocks=$n_arch scratch=$S"

args=(--network testnet --data-dir "$S/data" --log-level info run
  --p2p-port "$GS023_P2P_PORT" --rpc-port "$GS023_RPC_PORT" --metrics-port "$GS023_METRICS_PORT"
  --archive-to "$S/archive" --bootstrap "$GS023_SEED_P2P" --no-auto-update --yes --no-snap-sync)
"$GS023_BINARY" run --help 2>/dev/null | grep -q -- '--no-dht' && args+=(--no-dht)

T0="$(now_ms)"
"$GS023_BINARY" "${args[@]}" >"$S/node.log" 2>&1 &
PID=$!

# --- phase A: wait for the first answer -----------------------------------------
T_FIRST=""
DEAD=0
while :; do
  alive "$PID" || { DEAD=1; break; }
  [ $(($(now_ms) - T0)) -gt $((GS023_READY_WAIT * 1000)) ] && break
  if ok_body "$(rpc "$LOCAL_RPC")"; then
    T_FIRST="$(now_ms)"
    break
  fi
  sleep 1
done

FAILS=0
SECOND_MS=""
DURING=no
STARTS=0
FINISHES=0
TIP_T=""
NODE_H=""
SEED_H=""
if [ -n "$T_FIRST" ]; then
  STARTS="$(count "$START_RE")"
  FINISHES="$(count "$FINISH_RE")"
  [ "$STARTS" -gt "$FINISHES" ] && DURING=yes
  out="$(curl -s -m "$GS023_PROBE_TIMEOUT" "$LOCAL_RPC" -H 'Content-Type: application/json' \
    -d "$REQ" -w '\n%{time_total}' 2>/dev/null)"
  if ok_body "$out"; then
    SECOND_MS="$(printf '%s' "$out" | tail -1 | awk '{ printf "%.1f", $1 * 1000 }')"
  else
    FAILS=1
  fi

  # --- phase B: hold window -----------------------------------------------------
  while :; do
    t="$(now_ms)"
    alive "$PID" || { DEAD=1; break; }
    [ $((t - T0)) -ge $((GS023_TIP_MAX * 1000)) ] && break
    [ -n "$TIP_T" ] && [ $((t - T_FIRST)) -ge $((GS023_HOLD * 1000)) ] && break
    h=""
    body="$(rpc "$LOCAL_RPC")"
    if ok_body "$body"; then
      h="$(height_of "$body")"
      NODE_H="$h"
    else
      FAILS=$((FAILS + 1))
    fi
    sb="$(rpc "$GS023_SEED_RPC")"
    ok_body "$sb" && SEED_H="$(height_of "$sb")"
    if [ -z "$TIP_T" ] && [ -n "$h" ] && [ -n "$SEED_H" ] && [ "$h" -ge $((SEED_H - TIP_SLACK)) ]; then
      TIP_T="$(now_ms)"
    fi
    sleep 1
  done
fi

# --- shutdown -------------------------------------------------------------------
SHUT=""
if alive "$PID" && ours "$PID"; then
  ts="$(now_ms)"
  kill -TERM "$PID" 2>/dev/null
  while alive "$PID"; do
    [ $(($(now_ms) - ts)) -ge $((SHUTDOWN_MAX * 1000)) ] && break
    sleep 0.2
  done
  te="$(now_ms)"
  alive "$PID" || SHUT="$(secs "$ts" "$te")"
fi
alive "$PID" || wait "$PID" 2>/dev/null
evidence

# --- verdict --------------------------------------------------------------------
ALL=0
res() {
  if [ "$2" = 1 ]; then echo "PASS $1 $3"; else
    echo "FAIL $1 $3"
    ALL=1
  fi
}
TR=none
[ -n "$T_FIRST" ] && TR="$(secs "$T0" "$T_FIRST")"
TT=none
[ -n "$TIP_T" ] && TT="$(secs "$T0" "$TIP_T")"
got() { [ "$TR" != none ]; }
pre=""
got || pre="T_ready=none "
died=no
[ "$DEAD" = 1 ] && died=yes

ok=0; got && le "$TR" "$READY_30" && ok=1
res gs023-rpc-30s "$ok" "T_ready=${TR} limit=${READY_30}s exited_early=${died}"
ok=0; got && le "$TR" "$READY_10" && ok=1
res gs023-rpc-10s "$ok" "T_ready=${TR} limit=${READY_10}s"
ok=0; got && [ -n "$SECOND_MS" ] && lt "$SECOND_MS" "$SECOND_MAX_MS" && [ "$DURING" = yes ] && ok=1
res gs023-second-call-1s "$ok" "${pre}latency_ms=${SECOND_MS:-none} during_check=${DURING} starts=${STARTS} finishes=${FINISHES}"
ok=0; got && [ "$FAILS" -eq 0 ] && ok=1
res gs023-no-failed-probe "$ok" "${pre}failed_probes=${FAILS}"
ok=0; got && [ "$DEAD" = 0 ] && ok=1
res gs023-no-respawn "$ok" "${pre}pid=${PID} exited_in_window=${died}"
ok=0; got && [ "$TT" != none ] && le "$TT" "$GS023_TIP_MAX" && ok=1
res gs023-reaches-tip "$ok" "${pre}time_to_tip_s=${TT} node_h=${NODE_H:-none} seed_h=${SEED_H:-none} limit=${GS023_TIP_MAX}s"
ok=0; [ -n "$SHUT" ] && le "$SHUT" "$SHUTDOWN_MAX" && ok=1
res gs023-clean-shutdown "$ok" "shutdown_s=${SHUT:-none} limit=${SHUTDOWN_MAX}s"
echo "METRIC T_ready_s=${TR} second_call_ms=${SECOND_MS:-none} time_to_tip_s=${TT} shutdown_s=${SHUT:-none}"
exit "$ALL"
