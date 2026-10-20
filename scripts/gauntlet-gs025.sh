#!/usr/bin/env bash
# gauntlet-gs025.sh — GS-025 "archive-integrity-no-phantom-loop" (INC-I-253).
# REQ-INTEG-008 — Decision: a FAIL means the periodic integrity scan still reports a phantom
# missing block, or repairs a real hole more than once, so the INC-I-253 fix must not ship.
# Starts a SCRATCH non-producer archive node from clones of the local testnet seed's OLDEST
# checkpoint and block archive, bootstrapped to the seed, then stops it with SIGTERM.
#   Part A: the store as cloned (it holds height key 0); >= 3 periodic scans run in catch-up.
#   Part B: one block is punched out of a fresh clone with the node stopped
#           (integrity_hole_punch); the node must repair it once and then scan complete.
# Standalone, no args.
#
# OUTPUT CONTRACT: one `PASS|FAIL <id> <measured numbers>` line per assertion, an evidence
# block per part, then the METRIC line (last line), then the exit code.
#   gs025-a-three-scans     Part A: >= 3 `[INTEGRITY] Periodic scan: chain complete` lines
#   gs025-a-no-missing      Part A: 0 `[INTEGRITY] Periodic scan: N missing` lines
#   gs025-a-no-repair       Part A: 0 archive repair passes (full `Backfilling` or `Targeted`)
#   gs025-b-repaired-once   Part B: 1 targeted pass, 0 full passes, 1 block imported
#   gs025-b-names-hole      Part B: an auto-repair line lists HOLE as a whole list element
#   gs025-b-complete-after  Part B: a `chain complete` scan after the first repair pass
#   gs025-b-no-loop         Part B: <= 1 repair pass, 0 back-offs, 0 missing scans after it
#   gs025-b-block-served    Part B, only after an import: getBlockByHeight HOLE == punched hash
#   METRIC partA_scans= partA_repair_passes= partA_heights_read= partB_repair_passes=
#          partB_imported= partB_heights_read= hole=          (<n>|none)
#   exit: 0 all PASS · 1 any FAIL · 2 SKIP (`SKIP gs025: <reason>`)
#   Part B setup failure: `FAIL gs025-b-setup <why>`, the METRIC line, exit 1.
# INPUT PARTITIONS: ccc2ca37 binary (phantom key-0 scan -> Part A FAIL) · fixed binary
#   (all PASS) · precondition missing (SKIP, exit 2).
# Env: GS025_BINARY GS025_PUNCH_BIN GS025_PARTS (AB | A) GS025_TESTNET_DIR GS025_SEED_RPC
#   GS025_SEED_P2P GS025_P2P_PORT GS025_RPC_PORT GS025_METRICS_PORT GS025_PART_MAX (s per part)
#   GS025_KEEP (1 = keep scratch) GS025_LIB_ONLY (1 = define functions only).
# SAFETY: never stops, signals or replaces a testnet node; the testnet dir is only read
#   (APFS clone or copy, never hard links); the punch runs only on the scratch clone; only
#   the PID this script started is signalled; the scratch dir under $TMPDIR is deleted on exit.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GS025_BINARY="${GS025_BINARY:-$ROOT/target/release/doli-node}"
GS025_PUNCH_BIN="${GS025_PUNCH_BIN:-$ROOT/target/release/examples/integrity_hole_punch}"
GS025_PARTS="${GS025_PARTS:-AB}"
GS025_TESTNET_DIR="${GS025_TESTNET_DIR:-$HOME/testnet}"
GS025_SEED_RPC="${GS025_SEED_RPC:-http://127.0.0.1:8500}"
GS025_SEED_P2P="${GS025_SEED_P2P:-/ip4/127.0.0.1/tcp/30300}"
GS025_P2P_PORT="${GS025_P2P_PORT:-30392}"
GS025_RPC_PORT="${GS025_RPC_PORT:-8592}"
GS025_METRICS_PORT="${GS025_METRICS_PORT:-9092}"
GS025_PART_MAX="${GS025_PART_MAX:-1500}"
GS025_KEEP="${GS025_KEEP:-0}"

SHUTDOWN_MAX=30
TMPROOT="${TMPDIR:-/tmp}"
TMPROOT="${TMPROOT%/}"
S=""
PID=""
NO_DHT=0
HOLE=""
PUNCHED=""
SERVED_LINE=""
STATS_A=""
STATS_B=""
ALL=0

# --- log parsing (pure: no process, no scratch dir) ------------------------------

_gs025_filtered() {
  [ -f "$1" ] || return 0
  grep -E '\[(INTEGRITY|ARCHIVER)\]' "$1" 2>/dev/null | sed $'s/\x1b\\[[0-9;]*m//g'
}

_gs025_kv() { printf '%s\n' "$2" | tr ' ' '\n' | sed -n "s/^$1=//p" | head -n 1; }

_gs025_res() {
  if [ "$2" = 1 ]; then
    echo "PASS $1 $3"
    return 0
  fi
  echo "FAIL $1 $3"
  return 1
}

# gs025_scan_stats LOG — one line of ten counters; an absent LOG counts as all 0.
gs025_scan_stats() {
  _gs025_filtered "$1" | awk '
    function after(s, k,   i) { i = index(s, k); return i ? substr(s, i + length(k)) + 0 : 0 }
    /\[INTEGRITY\] Periodic scan: chain complete/ { c++ }
    /\[INTEGRITY\] Periodic scan: [0-9]+ missing blocks/ { m++ }
    /\[INTEGRITY\] Auto-repair successful/ { s++ }
    /\[INTEGRITY\] .*repair backed off/ { bo++ }
    /\[ARCHIVER\] Backfilling from archive: [0-9]+ blocks available/ { f++; hr += after($0, "Backfilling from archive: ") }
    /\[ARCHIVER\] Backfill complete: [0-9]+ blocks imported/ { im += after($0, "Backfill complete: ") }
    /\[ARCHIVER\] Targeted backfill: [0-9]+ heights/ {
      t++; hr += after($0, "Targeted backfill: "); im += after($0, "heights, ")
    }
    END {
      printf "scans=%d complete=%d missing_scans=%d full_backfills=%d targeted=%d repair_passes=%d imported=%d heights_read=%d backoffs=%d successes=%d\n",
        c + m + s, c, m, f, t, f + t, im, hr, bo, s
    }'
}

# _gs025_order LOG HOLE — ordering facts relative to the first archive repair pass.
_gs025_order() {
  _gs025_filtered "$1" | awk -v h="${2:-}" '
    function listed(s,   r, n, a, k) {
      if (h == "" || !match(s, /missing blocks in [0-9]+\.\.=[0-9]+: \[[0-9, ]*\]/)) return 0
      r = substr(s, RSTART, RLENGTH); sub(/^.*: \[/, "", r); sub(/\]$/, "", r)
      n = split(r, a, /, */)
      for (k = 1; k <= n; k++) if (a[k] == h) return 1
      return 0
    }
    /\[ARCHIVER\] (Targeted backfill: [0-9]+ heights|Backfilling from archive:)/ { if (!first) first = NR }
    first && /\[INTEGRITY\] Periodic scan: chain complete/ { ca++ }
    first && /\[INTEGRITY\] Periodic scan: [0-9]+ missing blocks/ { ma++ }
    /auto-repairing from archive/ && listed($0) { nm = 1 }
    END { printf "first=%d complete_after=%d missing_after=%d named=%d\n", first, ca, ma, nm }'
}

# gs025_verdict_a LOG — Part A assertions; rc 0 iff all PASS.
gs025_verdict_a() {
  local st c m p ok rc=0
  st="$(gs025_scan_stats "$1")"
  c="$(_gs025_kv complete "$st")"
  m="$(_gs025_kv missing_scans "$st")"
  p="$(_gs025_kv repair_passes "$st")"
  ok=0; [ "$c" -ge 3 ] && ok=1
  _gs025_res gs025-a-three-scans "$ok" "complete=$c scans=$(_gs025_kv scans "$st") min=3" || rc=1
  ok=0; [ "$m" -eq 0 ] && ok=1
  _gs025_res gs025-a-no-missing "$ok" "missing_scans=$m" || rc=1
  ok=0; [ "$p" -eq 0 ] && ok=1
  _gs025_res gs025-a-no-repair "$ok" "repair_passes=$p full_backfills=$(_gs025_kv full_backfills "$st") targeted=$(_gs025_kv targeted "$st") heights_read=$(_gs025_kv heights_read "$st")" || rc=1
  return "$rc"
}

# gs025_verdict_b LOG HOLE — Part B assertions; rc 0 iff all PASS.
gs025_verdict_b() {
  local st o t f im p bo first ca ma nm ok rc=0
  st="$(gs025_scan_stats "$1")"
  o="$(_gs025_order "$1" "${2:-}")"
  t="$(_gs025_kv targeted "$st")"
  f="$(_gs025_kv full_backfills "$st")"
  im="$(_gs025_kv imported "$st")"
  p="$(_gs025_kv repair_passes "$st")"
  bo="$(_gs025_kv backoffs "$st")"
  first="$(_gs025_kv first "$o")"
  ca="$(_gs025_kv complete_after "$o")"
  ma="$(_gs025_kv missing_after "$o")"
  nm="$(_gs025_kv named "$o")"
  ok=0; [ "$t" -eq 1 ] && [ "$f" -eq 0 ] && [ "$im" -eq 1 ] && ok=1
  _gs025_res gs025-b-repaired-once "$ok" "targeted=$t full_backfills=$f imported=$im" || rc=1
  ok=0; [ "$nm" -eq 1 ] && ok=1
  _gs025_res gs025-b-names-hole "$ok" "hole=${2:-none} named=$nm" || rc=1
  ok=0; [ "$first" -gt 0 ] && [ "$ca" -ge 1 ] && ok=1
  _gs025_res gs025-b-complete-after "$ok" "complete_scans_after_repair=$ca repaired=$([ "$first" -gt 0 ] && echo yes || echo no)" || rc=1
  ok=0; [ "$p" -le 1 ] && [ "$bo" -eq 0 ] && [ "$ma" -eq 0 ] && ok=1
  _gs025_res gs025-b-no-loop "$ok" "repair_passes=$p backoffs=$bo missing_scans_after_repair=$ma" || rc=1
  return "$rc"
}

_gs025_ckpt_list() {
  local d n h t
  [ -d "$1" ] || return 0
  for d in "$1"/h*-*; do
    [ -d "$d" ] || continue
    n="${d##*/}"
    h="${n#h}"
    t="${h#*-}"
    h="${h%%-*}"
    case "$h" in '' | *[!0-9]*) continue ;; esac
    case "$t" in '' | *[!0-9]*) continue ;; esac
    printf '%s %s %s\n' "$((10#$h))" "$((10#$t))" "$d"
  done | sort -n -k1,1 -k2,2
}

# gs025_pick_checkpoint DIR — the oldest h<height>-<time> subdir (numeric order).
gs025_pick_checkpoint() {
  local l
  l="$(_gs025_ckpt_list "$1" | head -n 1)"
  [ -n "$l" ] || return 1
  printf '%s\n' "${l#* * }"
}

_gs025_has() {
  local f
  printf -v f '%s/%010d' "$1" "$2"
  [ -f "$f.block" ] && [ -f "$f.blake3" ]
}

# gs025_pick_hole ARCH LO HI — archived height nearest (LO+HI)/2, up first; never LO, HI or <= 1.
gs025_pick_hole() {
  local a="$1" lo="$2" hi="$3" mid d=0 c
  case "$lo" in '' | *[!0-9]*) return 1 ;; esac
  case "$hi" in '' | *[!0-9]*) return 1 ;; esac
  [ -d "$a" ] || return 1
  lo=$((10#$lo))
  hi=$((10#$hi))
  mid=$(((lo + hi) / 2))
  while [ $((mid + d)) -lt "$hi" ] || [ $((mid - d)) -gt "$lo" ]; do
    for c in $((mid + d)) $((mid - d)); do
      if [ "$c" -gt "$lo" ] && [ "$c" -lt "$hi" ] && [ "$c" -gt 1 ] && _gs025_has "$a" "$c"; then
        echo "$c"
        return 0
      fi
    done
    d=$((d + 1))
  done
  return 1
}

_gs025_mv() {
  local v
  v="$(_gs025_kv "$1" "$2")"
  echo "${v:-none}"
}

# gs025_metric STATS_A STATS_B HOLE — the METRIC line; a part with no stats prints none.
gs025_metric() {
  printf 'METRIC partA_scans=%s partA_repair_passes=%s partA_heights_read=%s partB_repair_passes=%s partB_imported=%s partB_heights_read=%s hole=%s\n' \
    "$(_gs025_mv scans "$1")" "$(_gs025_mv repair_passes "$1")" "$(_gs025_mv heights_read "$1")" \
    "$(_gs025_mv repair_passes "$2")" "$(_gs025_mv imported "$2")" "$(_gs025_mv heights_read "$2")" "${3:-none}"
}

# --- scratch node control (main only) --------------------------------------------

skip() { echo "SKIP gs025: $*"; exit 2; }

rpc() {
  curl -s -m 10 "$1" -H 'Content-Type: application/json' -d "$2" 2>/dev/null
}
ok_body() { case "$1" in *'"result"'*) return 0 ;; esac; return 1; }
port_busy() { lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }
udp_busy() { lsof -nP -iUDP:"$1" >/dev/null 2>&1; }
alive() {
  local st
  st="$(ps -p "$1" -o stat= 2>/dev/null | tr -d ' ')"
  [ -n "$st" ] && [ "${st#Z}" = "$st" ]
}
ours() { [ -n "$S" ] && ps -p "$1" -o command= 2>/dev/null | grep -qF -- "$S"; }
# Prints cargo|rustc when a build of THIS checkout runs (its args name ROOT/, not a worktree under it).
build_running() {
  ps -axww -o args= 2>/dev/null | awk -v r="$ROOT/" -v w="$ROOT/.claude/worktrees/" '
    { b = $1; sub(/.*\//, "", b) }
    (b == "cargo" || b == "rustc") && index($0, r) && !index($0, w) { print b; exit }'
}

_gs025_rmtree() {
  local t="$1"
  case "$t" in "$TMPROOT"/gs025.?*) rm -rf "$t" ;; esac
}

clone() {
  local dst="$2"
  cp -c -R "$1" "$dst" 2>/dev/null && return 0
  _gs025_rmtree "$dst"
  cp -R "$1" "$dst"
}

_gs025_nfiles() { find "$1" -type f 2>/dev/null | wc -l | tr -d ' '; }

_gs025_stop() {
  local n=0
  [ -n "$PID" ] || return 0
  if alive "$PID" && ours "$PID"; then
    kill -TERM "$PID" 2>/dev/null
    while alive "$PID" && [ "$n" -lt $((SHUTDOWN_MAX * 5)) ]; do
      sleep 0.2
      n=$((n + 1))
    done
    alive "$PID" && ours "$PID" && kill -KILL "$PID" 2>/dev/null
  fi
  wait "$PID" 2>/dev/null
  PID=""
}

# shellcheck disable=SC2329  # invoked via the EXIT trap
_gs025_cleanup() {
  _gs025_stop
  [ -n "$S" ] || return 0
  if [ "$GS025_KEEP" = 1 ]; then
    echo "kept $S"
    return 0
  fi
  _gs025_rmtree "$S"
}

_gs025_evidence() {
  [ -f "$1" ] || return 0
  echo "--- $2 node.log evidence (first 12 [INTEGRITY] / [ARCHIVER] repair lines)"
  _gs025_filtered "$1" | grep -E '\[INTEGRITY\]|\[ARCHIVER\] (Backfill|Targeted)' | head -n 12
}

_gs025_preflight() {
  case "$GS025_PARTS" in AB | A) ;; *) skip "GS025_PARTS must be AB or A (got $GS025_PARTS)" ;; esac
  [ -n "$(build_running)" ] && skip "a build is running (cargo/rustc) in this checkout $ROOT"
  [ -f "$GS025_BINARY" ] && [ -x "$GS025_BINARY" ] || skip "binary $GS025_BINARY missing or not executable"
  if [ "$GS025_PARTS" = AB ] && [ ! -x "$GS025_PUNCH_BIN" ]; then
    skip "helper $GS025_PUNCH_BIN missing; build it with: cargo build --release -p doli-node --example integrity_hole_punch"
  fi
  for p in "$GS025_P2P_PORT" "$GS025_RPC_PORT" "$GS025_METRICS_PORT"; do
    port_busy "$p" && skip "TCP port $p is busy"
  done
  udp_busy $((GS025_P2P_PORT + 1)) && skip "UDP discovery port $((GS025_P2P_PORT + 1)) is busy"
  ok_body "$(rpc "$GS025_SEED_RPC" '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}')" ||
    skip "seed RPC $GS025_SEED_RPC does not answer getChainInfo"
  gs025_pick_checkpoint "$GS025_TESTNET_DIR/seed/data/checkpoints" >/dev/null ||
    skip "no checkpoint under $GS025_TESTNET_DIR/seed/data/checkpoints"
  [ -d "$GS025_TESTNET_DIR/seed/blocks" ] || skip "no seed block archive at $GS025_TESTNET_DIR/seed/blocks"
}

# Sets CKPT and CK_H to the oldest checkpoint whose copy matches its source file count.
_gs025_clone_pristine() {
  local h t ck ns nd
  while read -r h t ck; do
    [ -n "$ck" ] || continue
    _gs025_rmtree "$S/pristine"
    clone "$ck" "$S/pristine" 2>/dev/null || continue
    [ -d "$ck" ] || continue
    ns="$(_gs025_nfiles "$ck")"
    nd="$(_gs025_nfiles "$S/pristine")"
    if [ "$ns" = "$nd" ] && [ "${nd:-0}" -gt 0 ]; then
      CKPT="$ck"
      CK_H="$h"
      return 0
    fi
    echo "gs025: checkpoint ${ck##*/} changed during the copy (source=$ns files, copy=$nd); trying the next one (time $t)"
  done <<<"$(_gs025_ckpt_list "$GS025_TESTNET_DIR/seed/data/checkpoints")"
  return 1
}

# _gs025_run_part DIR STOP_FN — starts the node on DIR/data, polls DIR/node.log until STOP_FN,
# GS025_PART_MAX or death, then stops it. Sets END_WHY and ELAPSED.
_gs025_run_part() {
  local p="$1" t0
  local args=(--network testnet --data-dir "$p/data" --log-level info run
    --p2p-port "$GS025_P2P_PORT" --rpc-port "$GS025_RPC_PORT" --metrics-port "$GS025_METRICS_PORT"
    --archive-to "$S/archive" --bootstrap "$GS025_SEED_P2P" --no-auto-update --yes --no-snap-sync)
  [ "$NO_DHT" = 1 ] && args+=(--no-dht)
  t0="$(date +%s)"
  "$GS025_BINARY" "${args[@]}" >"$p/node.log" 2>&1 &
  PID=$!
  END_WHY=max
  while :; do
    sleep 2
    alive "$PID" || { END_WHY=died; break; }
    "$2" "$p/node.log" && { END_WHY=stop_condition; break; }
    [ $(($(date +%s) - t0)) -ge "$GS025_PART_MAX" ] && break
  done
  ELAPSED=$(($(date +%s) - t0))
}

_gs025_stop_a() { [ "$(_gs025_kv scans "$(gs025_scan_stats "$1")")" -ge 3 ]; }

_gs025_stop_b() {
  local st o
  st="$(gs025_scan_stats "$1")"
  o="$(_gs025_order "$1" "$HOLE")"
  if [ "$(_gs025_kv successes "$st")" -ge 1 ] && [ "$(_gs025_kv complete_after "$o")" -ge 1 ]; then
    return 0
  fi
  [ "$(_gs025_kv scans "$st")" -ge 4 ]
}

_gs025_served() {
  local body served ok=0
  [ "$(_gs025_kv imported "$(gs025_scan_stats "$1")")" -ge 1 ] || return 0
  alive "$PID" || return 0
  body="$(rpc "http://127.0.0.1:$GS025_RPC_PORT" "{\"jsonrpc\":\"2.0\",\"method\":\"getBlockByHeight\",\"params\":{\"height\":$HOLE},\"id\":1}")"
  served="$(printf '%s' "$body" | grep -oE '"hash"[[:space:]]*:[[:space:]]*"[0-9a-f]*"' | head -n 1 | sed -E 's/.*"([0-9a-f]*)"$/\1/')"
  [ -n "$served" ] && [ "$served" = "$PUNCHED" ] && ok=1
  SERVED_LINE="$(_gs025_res gs025-b-block-served "$ok" "hash=${served:-none} punched=$PUNCHED height=$HOLE")"
}

_gs025_part_a() {
  mkdir -p "$S/a"
  clone "$S/pristine" "$S/a/data" || skip "cannot copy the pristine store into $S/a/data"
  _gs025_run_part "$S/a" _gs025_stop_a
  _gs025_stop
  echo "gs025: part A ended=$END_WHY elapsed_s=$ELAPSED"
  _gs025_evidence "$S/a/node.log" "part A"
  STATS_A="$(gs025_scan_stats "$S/a/node.log")"
  echo "gs025: part A $STATS_A"
  gs025_verdict_a "$S/a/node.log" || ALL=1
}

_gs025_setup_fail() {
  echo "FAIL gs025-b-setup $*"
  gs025_metric "$STATS_A" "" "$HOLE"
  exit 1
}

_gs025_part_b() {
  local out
  mkdir -p "$S/b"
  clone "$S/pristine" "$S/b/data" || _gs025_setup_fail "cannot copy the pristine store into $S/b/data"
  HOLE="$(gs025_pick_hole "$S/archive" 1 "$CK_H")" || _gs025_setup_fail "no archived height strictly inside 1..$CK_H"
  out="$(GS025_PROTECTED_ROOTS="$GS025_TESTNET_DIR" "$GS025_PUNCH_BIN" "$S/b/data/blocks" "$HOLE" 2>&1)"
  PUNCHED="$(printf '%s\n' "$out" | sed -n -E "s/^PUNCHED height=$HOLE hash=([0-9a-f]+)$/\\1/p" | head -n 1)"
  [ -n "$PUNCHED" ] || _gs025_setup_fail "helper did not punch height $HOLE: $(printf '%s' "$out" | tr '\n' ' ')"
  echo "gs025: part B punched height=$HOLE hash=$PUNCHED"
  _gs025_run_part "$S/b" _gs025_stop_b
  _gs025_served "$S/b/node.log"
  _gs025_stop
  echo "gs025: part B ended=$END_WHY elapsed_s=$ELAPSED"
  _gs025_evidence "$S/b/node.log" "part B"
  STATS_B="$(gs025_scan_stats "$S/b/node.log")"
  echo "gs025: part B $STATS_B"
  gs025_verdict_b "$S/b/node.log" "$HOLE" || ALL=1
  if [ -n "$SERVED_LINE" ]; then
    echo "$SERVED_LINE"
    case "$SERVED_LINE" in PASS*) ;; *) ALL=1 ;; esac
  fi
}

gs025_main() {
  set -u
  trap _gs025_cleanup EXIT
  trap 'exit 1' INT TERM
  _gs025_preflight
  S="$(mktemp -d "$TMPROOT/gs025.XXXXXX")" || skip "mktemp failed"
  CKPT=""
  CK_H=""
  _gs025_clone_pristine || skip "no checkpoint could be copied intact (each changed or vanished during the copy)"
  clone "$GS025_TESTNET_DIR/seed/blocks" "$S/archive" || skip "cannot copy the seed block archive"
  "$GS025_BINARY" run --help 2>/dev/null | grep -q -- '--no-dht' && NO_DHT=1
  echo "gs025: binary=$("$GS025_BINARY" --version 2>/dev/null) checkpoint=${CKPT##*/} parts=$GS025_PARTS scratch=$S"
  _gs025_part_a
  [ "$GS025_PARTS" = AB ] && _gs025_part_b
  gs025_metric "$STATS_A" "$STATS_B" "$HOLE"
  exit "$ALL"
}

[ "${GS025_LIB_ONLY:-0}" = 1 ] || gs025_main
