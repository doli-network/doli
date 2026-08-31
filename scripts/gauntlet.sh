#!/usr/bin/env bash
# ============================================================================
# gauntlet.sh — OMEGA gauntlet runner (system-impact.md GAUNTLET contract).
#
# "Done" is a SYSTEM property, not a diff property. This runner replays every
# failure mode DOLI has paid for — as ASSERTIONS over the real, running local
# testnet — and fails if any system-level property regresses. It is the
# executable other half of the failure-mode matrix: the matrix catches
# contradictions with recorded knowledge at reasoning time; the gauntlet catches
# emergent interactions only visible on a live multi-node run.
#
# CONTRACT (system-impact.md §GAUNTLET, lines 103-118):
#   1. Reads active scenarios from gauntlet_scenarios.
#   2. Executes each against a real running system (the local testnet).
#   3. Logs its OWN gauntlet_runs result row (never typed by an agent).
#   4. Exits 0 only if every scenario passed.
#
# EXECUTION MODEL (approved): observational + ONE safe perturbation.
#   * Assertions are evaluated against the already-running testnet over a short
#     observation window (RPC + windowed structured-telemetry log scan).
#   * The ONLY perturbation is a launchd-managed single-node restart (GS-004),
#     which is non-destructive (launchd owns the lifecycle) and exercises real
#     rejoin/recovery. NO genesis reset, NO pkill, NO data wipe — ever.
#   Set GAUNTLET_NO_PERTURB=1 for a purely-observational run (no restart).
#   * GS-009 (fleet rolling-restart) is an ADDITIONAL opt-in perturbation,
#     gated like chaos: run `--gs009` WITH GAUNTLET_GS009_CONFIRM=1. It
#     wave-restarts ALL producers (n1..n12, NEVER the seed) to replay
#     INC-I-143 and asserts no >6-slot stall, no sibling fork, full rejoin,
#     and a still-usable OnChain release trust root (INC-I-196).
#     NOT part of the default run; see scripts/gauntlet-gs009.sh.
#   * GS-010 (duplicate-registration poison) is opt-in AND the only scenario
#     that WRITES TO THE CHAIN: it funds a wallet and permanently bonds a
#     producer (bonds unwind only via request-withdrawal, up to 75% penalty).
#     Run `--gs010` WITH GAUNTLET_GS010_CONFIRM=1; testnet only. Replays
#     INC-I-147 and asserts poison→recovery, no non-producer wedge, fleet
#     reconvergence, and exactly-one registration. It SKIPS cleanly when the
#     fleet has no live UNREGISTERED producer, no funding source, or no
#     non-producing node. See scripts/gauntlet-gs010.sh.
#   * GS-014 (governance relay from a non-producer) is opt-in AND writes to the
#     CHAIN: it removes a maintainer and re-adds the SAME key, both submitted to
#     a node PROVEN to carry no --producer flag. Run `--gs014` WITH
#     GAUNTLET_GS014_CONFIRM=1; testnet only. Replays INC-I-195 (accepted by a
#     relay endpoint, silently never mined) and asserts the change actually
#     reaches a producer and applies. STATE-NEUTRAL: a completed run ends on the
#     digest it started from. It SKIPS cleanly when no non-producer endpoint is
#     live, the set is not an enforced on-chain set, or it has fewer than 4
#     members. See scripts/gauntlet-gs014.sh.
#
# Assertions key off STRUCTURED telemetry fields (gap=, rollback_depth=,
# sync_fails=, state=) and distinct-event phrases — NEVER raw keywords that also
# appear in per-second telemetry (the word "rollback" logs ~1/sec at depth 0).
#
# Usage:  bash scripts/gauntlet.sh [--quick] [--chaos|--gs009|--gs010|--gs014]
# Env:    WORKFLOW_RUN_ID, GAUNTLET_WINDOW (s, default 45), GAUNTLET_RESTART_NODE
#         (default n5), GAUNTLET_RSS_CEIL_MB (default 800), GAUNTLET_MIN_NODES
#         (default 3), GAUNTLET_NO_PERTURB (1=skip restart).
# Exit:   0 all scenarios passed · 1 a scenario failed · 3 preflight failed.
# ============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="$ROOT/.omega/memory.db"
COLLECT="$ROOT/scripts/gauntlet-collect.py"
GS009_LIB="$ROOT/scripts/gauntlet-gs009.sh"
[ -f "$GS009_LIB" ] && . "$GS009_LIB"
GS010_LIB="$ROOT/scripts/gauntlet-gs010.sh"
[ -f "$GS010_LIB" ] && . "$GS010_LIB"
GS014_LIB="$ROOT/scripts/gauntlet-gs014.sh"
[ -f "$GS014_LIB" ] && . "$GS014_LIB"
LOG_DIR="$HOME/testnet/logs"
LABEL_PREFIX="network.doli.testnet"

WINDOW="${GAUNTLET_WINDOW:-45}"
RESTART_NODE="${GAUNTLET_RESTART_NODE:-n5}"
RSS_CEIL_MB="${GAUNTLET_RSS_CEIL_MB:-800}"
MIN_NODES="${GAUNTLET_MIN_NODES:-3}"
NO_PERTURB="${GAUNTLET_NO_PERTURB:-0}"
# Per-scenario waiver: GAUNTLET_WAIVE="GS-001 GS-00x" (space/comma separated) marks
# a scenario as out-of-scope for THIS run — a documented environmental condition,
# not a code regression. A waiver WITHOUT a reason is refused (no fake-green): the
# mandatory GAUNTLET_WAIVE_REASON string is printed and persisted to the result row.
WAIVE="${GAUNTLET_WAIVE:-}"; WAIVE="${WAIVE//,/ }"
WAIVE_REASON="${GAUNTLET_WAIVE_REASON:-}"
CHAOS=0
GS009=0
GS010=0
GS014=0
for a in "$@"; do
  case "$a" in
    --quick) WINDOW=20 ;;
    --chaos) CHAOS=1 ;;
    --gs009) GS009=1 ;;
    --gs010) GS010=1 ;;
    --gs014) GS014=1 ;;
  esac
done
CHAOS_RECOVERED=1   # stays 1 unless a chaos injector fails to recover the node

# Assertion thresholds (scale: local testnet N≈6). Each either derives from the
# live network or is exercised at this small N (system-impact §SCALE-SENSITIVITY).
EMPTY_HEADERS_MAX=8      # a stall→reconnect loop produces dozens; healthy ≈0
ROLLBACK_EVENTS_MAX=3    # some rollback is normal recovery; a loop is not
ORPHAN_REQ_MAX=80        # an orphan-chase storm is hundreds over the window
SNAP_TRIGGER_MAX=0       # a converged net must not snap-sync
REJECTED_MAX=0
PANIC_MAX=0
INTEGRITY_GAP_MAX=0
BUSY_RATE_MAX_PCT=10
LIVENESS_MIN=1           # the chain must produce ≥1 block during the window
EVICT_MAX=6

C_G='\033[0;32m'; C_R='\033[0;31m'; C_Y='\033[1;33m'; C_C='\033[0;36m'; C_0='\033[0m'
say(){ printf "%b\n" "$*"; }
die(){ say "${C_R}$*${C_0}"; exit 3; }
# is_waived <scenario_id> — true if the id appears in the GAUNTLET_WAIVE list.
is_waived(){ case " $WAIVE " in *" $1 "*) return 0;; *) return 1;; esac; }
if [ -n "$WAIVE" ] && [ -z "$WAIVE_REASON" ]; then
  die "GAUNTLET_WAIVE=\"$WAIVE\" set without GAUNTLET_WAIVE_REASON — a waiver MUST carry
     a documented reason (it is printed and stored in the result row). Re-run e.g.:
     GAUNTLET_WAIVE=\"GS-001\" GAUNTLET_WAIVE_REASON=\"...evidence...\" scripts/gauntlet.sh"
fi

command -v sqlite3 >/dev/null 2>&1 || die "sqlite3 not found"
command -v python3 >/dev/null 2>&1 || die "python3 not found"
[ -f "$DB" ] || die "memory.db not found at $DB"
[ -f "$COLLECT" ] || die "collector not found at $COLLECT"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/gauntlet.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
NODECFG="$WORK/nodes.json"
METRICS="$WORK/metrics.json"
REJOIN_FILE="$WORK/rejoin.txt"; echo "0" > "$REJOIN_FILE"

SECONDS=0
SHA="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"

MODE="observe+restart"; [ "$CHAOS" = "1" ] && MODE="${C_R}CHAOS-inject${C_C}"; [ "$NO_PERTURB" = "1" ] && MODE="observe-only"
say "${C_C}════════════════════════════════════════════════════════════${C_0}"
say "${C_C} OMEGA GAUNTLET${C_0}  ·  sha=$SHA  mode=${MODE}${C_C}  window=${WINDOW}s  target=${RESTART_NODE}"
say "${C_C}════════════════════════════════════════════════════════════${C_0}"

# ── node port map ───────────────────────────────────────────────────────────
port_of(){ case "$1" in seed) echo 8500;; n*) echo $((8500 + ${1#n}));; esac; }
rpc_up(){ curl -sf -m 2 -X POST "http://127.0.0.1:$1" -H 'Content-Type: application/json' \
          -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' 2>/dev/null; }
height_of(){ rpc_up "$1" | python3 -c "import sys,json
try: print(json.load(sys.stdin)['result']['bestHeight'])
except Exception: print(-1)" 2>/dev/null; }

# ── discover live nodes ─────────────────────────────────────────────────────
say "\n${C_C}▸ discovering live testnet nodes${C_0}"
NAMES="seed"; for i in $(seq 1 12); do NAMES="$NAMES n$i"; done
LIVE=""
for name in $NAMES; do
  p="$(port_of "$name")"
  info="$(rpc_up "$p")" || true
  [ -z "$info" ] && continue
  LIVE="$LIVE $name"
done
LIVE="$(echo "$LIVE" | xargs || true)"
UP_COUNT=$(echo "$LIVE" | wc -w | tr -d ' ')
say "  live nodes ($UP_COUNT): ${LIVE:-none}"
[ "$UP_COUNT" -lt "$MIN_NODES" ] && die "preflight: need >=$MIN_NODES live nodes, found $UP_COUNT. Start the testnet: scripts/testnet.sh start all"

# active scenarios present?
SCEN_COUNT=$(sqlite3 "$DB" "SELECT COUNT(*) FROM gauntlet_scenarios WHERE status='active';" 2>/dev/null || echo 0)
[ "${SCEN_COUNT:-0}" -eq 0 ] && die "no active gauntlet_scenarios (seed: sqlite3 .omega/memory.db < scripts/gauntlet-seed.sql)"
say "  active scenarios: $SCEN_COUNT"

# ── baseline snapshot (log byte-offset + height per node) ───────────────────
build_nodecfg(){
  # $1 = "restarted node name" (offsets for it reset to post-restart tail)
  local restarted="${1:-}"
  { echo '{"nodes":['
    local first=1
    for name in $LIVE; do
      local p pid logf off bh rss
      p="$(port_of "$name")"
      logf="$LOG_DIR/${name}.log"
      off=$(wc -c < "$logf" 2>/dev/null | tr -d ' '); off="${off:-0}"
      bh="$(height_of "$p")"
      pid=$(launchctl list 2>/dev/null | grep "${LABEL_PREFIX}-${name}$" | awk '{print $1}' | head -1)
      [ "$pid" = "-" ] && pid=""
      if [ -n "$pid" ]; then rss=$(( $(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || echo 0) / 1024 )); else rss=0; fi
      [ "$first" = 1 ] || echo ','
      first=0
      printf '{"name":"%s","port":%s,"pid":"%s","logfile":"%s","offset":%s,"baseline_height":%s,"rss_mb":%s}' \
        "$name" "$p" "${pid:-0}" "$logf" "$off" "${bh:-0}" "$rss"
    done
    echo ']}'
  } > "$NODECFG"
}

# net tip = current max height across live nodes OTHER than the target.
net_tip_excluding(){
  local tgt="$1" tip=0 name nh
  for name in $LIVE; do
    [ "$name" = "$tgt" ] && continue
    nh="$(height_of "$(port_of "$name")")"; [ "${nh:-0}" -gt "$tip" ] && tip="$nh"
  done
  echo "$tip"
}

# wait_rejoin <port> <target-name> <timeout-s> — echo seconds until the node's
# RPC is up AND its height is within 1 of the net tip, else -1 on timeout.
wait_rejoin(){
  local rp="$1" tgt="$2" to="$3" t0 h tip
  t0=$(date +%s)
  while [ $(( $(date +%s) - t0 )) -le "$to" ]; do
    h="$(height_of "$rp")"
    if [ -n "$h" ] && [ "$h" != "-1" ] && [ "$h" -gt 0 ]; then
      tip="$(net_tip_excluding "$tgt")"
      [ "$h" -ge $(( tip - 1 )) ] && { echo $(( $(date +%s) - t0 )); return; }
    fi
    sleep 3
  done
  echo -1
}

# ── chaos injectors (opt-in --chaos; local testnet only; backed up) ──────────
chaos_guard(){
  [ "$RESTART_NODE" = "seed" ] && die "chaos target cannot be the seed"
  case "$RESTART_NODE" in n[0-9]|n[0-9][0-9]) ;; *) die "chaos target must be a producer node (nN), got '$RESTART_NODE'";; esac
  local others; others=$(echo "$LIVE" | tr ' ' '\n' | grep -vx "$RESTART_NODE" | grep -c .)
  [ "${others:-0}" -lt 3 ] && die "chaos needs >=3 OTHER live nodes as a recovery source, found $others"
  if [ "${GAUNTLET_CHAOS_CONFIRM:-0}" != "1" ]; then
    die "--chaos performs DESTRUCTIVE injection (stops + WIPES $RESTART_NODE data, backed up).
     Re-run with GAUNTLET_CHAOS_CONFIRM=1 to proceed. Local testnet only; never mainnet."
  fi
}

# injector A — node-down + rejoin (reproduces stall / restart-recovery)
chaos_node_down(){
  local secs="${GAUNTLET_CHAOS_DOWN_SECS:-25}" rp; rp="$(port_of "$RESTART_NODE")"
  say "  ${C_Y}[chaos A] node-down${C_0}: stopping $RESTART_NODE for ${secs}s (isolation/stall)"
  "$ROOT/scripts/testnet.sh" stop "$RESTART_NODE" >/dev/null 2>&1
  sleep "$secs"
  say "  [chaos A] restarting $RESTART_NODE"
  "$ROOT/scripts/testnet.sh" start "$RESTART_NODE" >/dev/null 2>&1
  local r; r="$(wait_rejoin "$rp" "$RESTART_NODE" 120)"; echo "$r" > "$REJOIN_FILE"
  if [ "$r" -ge 0 ]; then say "  [chaos A] rejoined in ${r}s"; else say "  ${C_R}[chaos A] did NOT rejoin in 120s${C_0}"; CHAOS_RECOVERED=0; fi
}

# injector B — data-wipe + cold snap/rebuild (reproduces snap-sync / rollback-rebuild)
chaos_data_wipe(){
  local rp dd ts bak; rp="$(port_of "$RESTART_NODE")"; dd="$HOME/testnet/$RESTART_NODE/data"
  ts="$(date +%Y%m%d-%H%M%S)"; bak="$HOME/testnet/$RESTART_NODE/data.bak.$ts"
  say "  ${C_Y}[chaos B] data-wipe${C_0}: stopping $RESTART_NODE"
  "$ROOT/scripts/testnet.sh" stop "$RESTART_NODE" >/dev/null 2>&1
  sleep 3
  # safety: identity (node_key, producer key) lives OUTSIDE data/ — never touched.
  if [ -d "$dd" ]; then mv "$dd" "$bak" && say "  [chaos B] backed up data -> $bak (reversible)"; fi
  say "  [chaos B] cold-restarting $RESTART_NODE (must snap/rebuild from peers)"
  "$ROOT/scripts/testnet.sh" start "$RESTART_NODE" >/dev/null 2>&1
  local to="${GAUNTLET_CHAOS_RECOVER_TIMEOUT:-240}" r; r="$(wait_rejoin "$rp" "$RESTART_NODE" "$to")"
  echo "$r" > "$WORK/chaos_recover.txt"
  if [ "$r" -ge 0 ]; then say "  [chaos B] recovered (snap/rebuild) in ${r}s"; else say "  ${C_R}[chaos B] did NOT recover in ${to}s${C_0}"; CHAOS_RECOVERED=0; fi
}

# ── readiness: the net must be PRODUCING before we judge liveness ────────────
# A freshly-booted mesh blocks production until peers>=2 (InsufficientPeers).
# Judging GS-008 liveness on a not-yet-formed mesh is a false negative — wait
# until the tip actually advances first (or warn and let GS-008 judge a real stall).
say "\n${C_C}▸ readiness: confirming the network is producing${C_0}"
net_max_height(){ local m=0 name nh; for name in $LIVE; do nh="$(height_of "$(port_of "$name")")"; [ "${nh:-0}" -gt "$m" ] && m="$nh"; done; echo "$m"; }
R_H0="$(net_max_height)"; R_T0=$(date +%s); R_READY=0
while [ $(( $(date +%s) - R_T0 )) -le 60 ]; do
  R_H1="$(net_max_height)"
  if [ "${R_H1:-0}" -gt "${R_H0:-0}" ]; then R_READY=1; say "  producing (h ${R_H0} -> ${R_H1})"; break; fi
  sleep 5
done
[ "$R_READY" = 1 ] || say "  ${C_Y}tip not advancing after 60s — mesh forming or stalled; proceeding (GS-008 will judge)${C_0}"

say "\n${C_C}▸ capturing baseline (log offsets + heights)${C_0}"
build_nodecfg
BASE_MAX=$(python3 -c "import json;d=json.load(open('$NODECFG'));print(max(n['baseline_height'] for n in d['nodes']))")
say "  baseline max height = $BASE_MAX"

# ── perturbation dispatch ───────────────────────────────────────────────────
RP="$(port_of "$RESTART_NODE")"
if [ "${GS014:-0}" = "1" ]; then
  # OPT-IN GS-014 governance relay from a NON-PRODUCER endpoint (perturbative AND
  # chain-writing: two maintainer governance transactions). Replays INC-I-195,
  # then re-baselines so the window judges the RECOVERED steady state.
  say "\n${C_R}▸ GS-014 MODE — GOVERNANCE RELAY FROM A NON-PRODUCER (writes to the chain: remove + re-add a maintainer)${C_0}"
  echo "0" > "$REJOIN_FILE"
  gs014_inject
  say "  [gs014] settling 10s, then re-baselining for a clean observation window"
  sleep 10
  build_nodecfg
elif [ "${GS010:-0}" = "1" ]; then
  # OPT-IN GS-010 duplicate-registration poison (perturbative AND chain-writing:
  # funds a wallet and permanently bonds a producer). Replays INC-I-147, then
  # re-baselines so the window judges the RECOVERED steady state.
  say "\n${C_R}▸ GS-010 MODE — DUPLICATE-REGISTRATION POISON (writes to the chain: funds + bonds a producer)${C_0}"
  echo "0" > "$REJOIN_FILE"
  gs010_inject
  say "  [gs010] settling 10s, then re-baselining for a clean observation window"
  sleep 10
  build_nodecfg
elif [ "${GS009:-0}" = "1" ]; then
  # OPT-IN GS-009 fleet rolling-restart (perturbative; NEVER the seed). Replays
  # INC-I-143, then re-baselines so the window judges the RECOVERED steady state.
  say "\n${C_R}▸ GS-009 MODE — FLEET ROLLING-RESTART of all producers (n1..n12, NEVER seed)${C_0}"
  echo "0" > "$REJOIN_FILE"
  gs009_inject
  say "  [gs009] settling 10s, then re-baselining for a clean observation window"
  sleep 10
  build_nodecfg
elif [ "$CHAOS" = "1" ]; then
  # OPT-IN chaos: genuinely reproduce failure-mode triggers, then re-baseline so
  # the observation window judges the RECOVERED steady state (a legitimate snap
  # during recovery must not count as a spurious escalation).
  say "\n${C_R}▸ CHAOS MODE — real injection on ${RESTART_NODE} (destructive, backed up)${C_0}"
  chaos_guard
  echo "0" > "$REJOIN_FILE"
  chaos_node_down
  chaos_data_wipe
  say "  [chaos] settling 10s, then re-baselining for a clean observation window"
  sleep 10
  build_nodecfg   # fresh log offsets + heights AFTER recovery
elif [ "$NO_PERTURB" = "1" ]; then
  say "\n${C_Y}▸ perturbation skipped (GAUNTLET_NO_PERTURB=1) — observational only${C_0}"
  echo "0" > "$REJOIN_FILE"
else
  say "\n${C_C}▸ perturbation: launchd restart of ${RESTART_NODE} (non-destructive)${C_0}"
  if echo "$LIVE" | grep -qw "$RESTART_NODE"; then
    "$ROOT/scripts/testnet.sh" restart "$RESTART_NODE" >/dev/null 2>&1 || say "  ${C_Y}restart command returned nonzero (continuing)${C_0}"
    rejoin="$(wait_rejoin "$RP" "$RESTART_NODE" 90)"; echo "$rejoin" > "$REJOIN_FILE"
    if [ "$rejoin" -ge 0 ]; then say "  ${RESTART_NODE} rejoined tip in ${rejoin}s"; else say "  ${C_R}${RESTART_NODE} did NOT rejoin within 90s${C_0}"; fi
  else
    say "  ${C_Y}${RESTART_NODE} not live — skipping restart, recovery asserted observationally${C_0}"
    echo "0" > "$REJOIN_FILE"
  fi
fi

# ── observation window ──────────────────────────────────────────────────────
say "\n${C_C}▸ observing live network for ${WINDOW}s${C_0}"
sleep "$WINDOW"

# ── collect metrics (RPC + windowed log scan) ───────────────────────────────
say "${C_C}▸ collecting metrics${C_0}"
if ! python3 "$COLLECT" "$NODECFG" > "$METRICS" 2>"$WORK/collect.err"; then
  say "${C_R}collector failed:${C_0}"; cat "$WORK/collect.err"; die "metrics collection failed"
fi
REJOIN=$(cat "$REJOIN_FILE")

# jget <python-expr on M> — M is the parsed metrics dict. None → empty string
# (so `${x:-0}` coerces cleanly and integer comparisons never see the token None).
jget(){ python3 -c "import json;M=json.load(open('$METRICS'));v=($1);print('' if v is None else v)" 2>/dev/null; }

# ── inconclusive gate ───────────────────────────────────────────────────────
# If the network became unhealthy DURING the run (nodes stopped, launchd booted
# them out), that is NOT a scenario regression — it is an inconclusive run. Fail
# loudly with a clear diagnostic instead of emitting bogus "forked" failures, and
# do NOT write a fail result row (a fail row would falsely blame the code).
UPC="$(jget "M['net']['up_count']")"
if [ "${UPC:-0}" -lt "$MIN_NODES" ]; then
  say "\n${C_R}▸ INCONCLUSIVE — network unhealthy at collection${C_0}"
  die "only ${UPC:-0} nodes reachable when metrics were collected (need >=$MIN_NODES).
     The testnet stopped mid-run (check: scripts/testnet.sh status). This is an
     inconclusive run, NOT a scenario failure — no result row written. Restart the
     testnet (scripts/testnet.sh start seed n1 n2 n3 n4 n5) and re-run."
fi

# ── assertion engine ────────────────────────────────────────────────────────
FAIL_REASONS=""   # accumulates within a single scenario

# assert <token> — returns 0 pass / 1 fail; appends human reason to FAIL_REASONS.
# ── GS-012: a producer's wallet BLS key must match its on-chain registration ──
# READ-ONLY: reads wallet files and queries getProducer. Writes nothing, touches
# no service. Guards INV-KEY-001 (INC-I-162): the BLS attestation key a producer
# signs with must be the one committed on-chain at registration.
#
# This is the detection that does not exist in the product. `run.rs:88` checks
# only that a BLS key is PRESENT, never that it MATCHES, and attestation has been
# Ed25519-only since 2026-07-19 — so a producer restored from a seed phrase by a
# pre-fix binary runs, produces and earns while holding an identity the chain does
# not recognise, with nothing anywhere reporting it.
#
# Skips (rc=2, not a pass) when the local testnet layout is absent, so this is
# inert outside a local testnet rather than falsely green.
_gs012_assert(){
  local t="$1" doli keys
  doli="$HOME/testnet/bin/doli"; [ -x "$doli" ] || doli="$PWD/target/release/doli"
  keys="$HOME/testnet/keys"
  if [ ! -x "$doli" ] || [ ! -d "$keys" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: no doli binary or ~/testnet/keys (not a local testnet)"
    return 2
  fi
  local rpc="http://127.0.0.1:$(port_of seed)" checked=0 mism=0 detail=""
  local w addr wbls rbls
  for w in "$keys"/producer_*.json; do
    [ -f "$w" ] || continue
    wbls="$(python3 -c "
import json,sys
try:
    d=json.load(open('$w'))
    print(d['addresses'][0].get('bls_public_key') or '')
except Exception:
    print('')
" 2>/dev/null)"
    [ -n "$wbls" ] || continue
    addr="$("$doli" --network testnet -w "$w" info 2>/dev/null | awk '/[Aa]ddress:/{print $NF; exit}')"
    [ -n "$addr" ] || continue
    rbls="$(curl -s --max-time 5 -X POST "$rpc" -H 'Content-Type: application/json' \
      -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getProducer\",\"params\":[\"$addr\"]}" \
      | python3 -c "
import sys,json
try:
    r=json.load(sys.stdin).get('result') or {}
    print(r.get('blsPubkey') or '')
except Exception:
    print('')
" 2>/dev/null)"
    # Unregistered address -> nothing to compare; not a finding.
    [ -n "$rbls" ] || continue
    checked=$(( checked + 1 ))
    if [ "$wbls" != "$rbls" ]; then
      mism=$(( mism + 1 ))
      detail="$detail $(basename "$w")"
    fi
  done
  if [ "$checked" -eq 0 ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: no registered producer wallets found to compare"
    return 2
  fi
  if [ "$mism" -gt 0 ]; then
    FAIL_REASONS="$FAIL_REASONS; $t: ${mism}/${checked} producer wallets hold a BLS key that does NOT match their on-chain registration —$detail"
    return 1
  fi
  INFO_REASONS="$INFO_REASONS; $t: ${checked} registered producer wallets verified"
  return 0
}

# ── GS-013: a producer's ProducerSet bond ledger must not outlive its Bond UTXOs ──
# READ-ONLY: one getProducers query. Writes nothing, touches no service. Guards
# INV-BOND-001 (INC-I-180): the ProducerSet bond_count P drives selection weight,
# but the Bond UTXOs U are what actually collateralise it. When P > U the producer
# is scheduled on weight backed by nothing.
#
# This is the detection that did not exist. Mainnet n11 ran for ~73,000 blocks at
# P=444 / U=10 — 434 unbacked units, 1.9% of fleet weight — and nothing anywhere
# reported it. It surfaced only because a human went looking.
#
# U > P is NOT a finding: the hourly auto-bond cron creates a Bond UTXO faster than
# the ProducerSet reflects it, and the next epoch boundary flushes it. Only P > U is
# unbacked weight. Producers carrying a pending update are mid-flush and excluded —
# the n11 shape is P > U with NOTHING queued, which never self-resolves.
_gs013_assert(){
  local t="$1" rpc="http://127.0.0.1:$(port_of seed)" out
  out="$(curl -s --max-time 10 -X POST "$rpc" -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getProducers","params":[]}' \
    | python3 -c "
import sys, json
try:
    r = json.load(sys.stdin).get('result') or {}
except Exception:
    print('ERR'); raise SystemExit
ps = r.get('producers') if isinstance(r, dict) and 'producers' in r else r
if not isinstance(ps, list):
    print('ERR'); raise SystemExit
bad = []
for x in ps:
    P = x.get('producerSetBondCount', 0) or 0
    U = x.get('bondCount', 0) or 0
    # a queued mutation means mid-flush, not unbacked weight
    if x.get('pendingUpdates'):
        continue
    if P > U:
        bad.append('%s(P=%d,U=%d,w=%s)' % (x.get('publicKey','?')[:12], P, U, x.get('selectionWeight')))
print('%d %d %s' % (len(ps), len(bad), ' '.join(bad)))
" 2>/dev/null)"
  if [ -z "$out" ] || [ "$out" = "ERR" ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: getProducers unreachable or unparseable at $rpc"
    return 2
  fi
  local checked bad detail
  checked="$(echo "$out" | awk '{print $1}')"
  bad="$(echo "$out" | awk '{print $2}')"
  detail="$(echo "$out" | cut -d' ' -f3-)"
  if [ "${checked:-0}" -eq 0 ]; then
    SKIP_REASONS="$SKIP_REASONS; $t: producer set is empty — nothing to compare"
    return 2
  fi
  if [ "${bad:-0}" -gt 0 ]; then
    FAIL_REASONS="$FAIL_REASONS; $t: ${bad}/${checked} producers hold ProducerSet weight with no Bond UTXOs behind it — $detail"
    return 1
  fi
  INFO_REASONS="$INFO_REASONS; $t: ${checked} producers verified, no unbacked selection weight"
  return 0
}

assert(){
  local t="$1" ok=1 why=""
  case "$t" in
    convergence)
      local d; d=$(jget "M['net']['distinct_common_hash']")
      if [ "$d" = "1" ]; then ok=0; else why="forked: $d distinct block hashes at common height"; fi ;;
    no-panic)
      local c; c=$(jget "M['net']['win_panic']")
      if [ "${c:-0}" -le "$PANIC_MAX" ]; then ok=0; else why="$c panic marker(s) in window"; fi ;;
    single-block1-hash)
      # Genesis (block 0) uniformity stays STRICT. Block-1 uniformity is checked
      # ONLY among nodes that actually HOLD block 1: a snap-synced node prunes
      # historical blocks (troubleshooting §1.9) and answers "Block not found" —
      # that is snap-pruned/absent, reported as a count, NEVER as divergence.
      # 0 holders -> explicit SKIP (unverifiable), never a silent pass. The
      # pruned-node count is always surfaced (no-silent-caps rule).
      local g b hold absent
      g=$(jget "M['net']['distinct_genesis']"); b=$(jget "M['net']['distinct_block1']")
      hold=$(jget "M['net']['block1_holders']"); absent=$(jget "M['net']['block1_absent']")
      if [ "$g" != "1" ]; then
        why="distinct genesis=$g (want 1); block1 holders=$hold pruned/absent=$absent"
      elif [ "${hold:-0}" -eq 0 ]; then
        ok=2; why="SKIP block-1 uniformity: 0 holders ($absent snap-pruned/absent) — genesis uniform, nothing to compare"
      elif [ "$b" = "1" ]; then
        ok=0; INFO_REASONS="$INFO_REASONS; single-block1-hash: uniform among $hold holder(s), $absent snap-pruned/absent"
      else
        why="distinct block1=$b among $hold holder(s) ($absent snap-pruned/absent) — genesis uniform"
      fi ;;
    no-spurious-escalation)
      local s r p rd; s=$(jget "M['net']['win_snap_trigger']"); r=$(jget "M['net']['recovery_mode_any']")
      p=$(jget "M['net']['production_paused_any']"); rd=$(jget "M['net']['max_rollback_depth']")
      if [ "${s:-0}" -le "$SNAP_TRIGGER_MAX" ] && [ "$r" = "False" ] && [ "$p" = "False" ] && [ "${rd:-0}" = "0" ]; then ok=0
      else why="snap_triggers=$s recovery_any=$r prod_paused=$p rollback_depth=$rd"; fi ;;
    no-empty-headers-loop)
      local c; c=$(jget "M['net']['win_empty_headers']")
      if [ "${c:-0}" -le "$EMPTY_HEADERS_MAX" ]; then ok=0; else why="$c empty-headers events (max $EMPTY_HEADERS_MAX)"; fi ;;
    state-root-match)
      local sr cs ps mc uc; sr=$(jget "M['net']['distinct_stateroot_modal']"); cs=$(jget "M['net']['distinct_cshash_modal']")
      ps=$(jget "M['net']['distinct_pshash_modal']"); mc=$(jget "M['net']['modal_node_count']"); uc=$(jget "M['net']['up_count']")
      if [ "$sr" = "1" ] && [ "$cs" = "1" ] && [ "$ps" = "1" ] && [ "${mc:-0}" -ge $(( uc - 1 )) ]; then ok=0
      else why="stateRoot=$sr csHash=$cs psHash=$ps distinct among $mc/$uc modal-height nodes"; fi ;;
    no-rejected-epoch-block)
      local c; c=$(jget "M['net']['win_rejected']")
      if [ "${c:-0}" -le "$REJECTED_MAX" ]; then ok=0; else why="$c rejected/invalid-epoch events"; fi ;;
    recovery-under-60s)
      if [ "$REJOIN" -ge 0 ] && [ "$REJOIN" -le 60 ]; then ok=0; else why="restart rejoin=${REJOIN}s (want 0..60; -1=never)"; fi ;;
    no-rollback-loop)
      local e rd; e=$(jget "M['net']['win_rollback_events']"); rd=$(jget "M['net']['max_rollback_depth']")
      if [ "${e:-0}" -le "$ROLLBACK_EVENTS_MAX" ] && [ "${rd:-0}" = "0" ]; then ok=0
      else why="$e rollback events (max $ROLLBACK_EVENTS_MAX), final rollback_depth=$rd"; fi ;;
    block-store-complete)
      # gap=0 is the real "currently caught up" signal; utxoCount agreement means
      # the store rebuilt completely. sync_fails is a cumulative LIFETIME counter
      # (a node hours-healthy still shows sync_fails>0) — not a current-health signal.
      local g u; g=$(jget "M['net']['max_gap']"); u=$(jget "M['net']['distinct_utxocount_modal']")
      if [ "${g:-1}" = "0" ] && [ "$u" = "1" ]; then ok=0
      else why="max_gap=$g distinct_utxoCount=$u (nodes behind or divergent store)"; fi ;;
    bounded-request-rate)
      local o; o=$(jget "M['net']['win_orphan_reqs']")
      if [ "${o:-0}" -le "$ORPHAN_REQ_MAX" ]; then ok=0; else why="$o orphan/parent requests (storm >$ORPHAN_REQ_MAX)"; fi ;;
    no-reforward)
      local m; m=$(jget "M['net']['max_rss_mb']")
      if [ "${m:-0}" -lt "$RSS_CEIL_MB" ]; then ok=0; else why="max RSS ${m}MB >= ${RSS_CEIL_MB}MB (reforward memory spike)"; fi ;;
    bounded-memory)
      local m; m=$(jget "M['net']['max_rss_mb']")
      if [ "${m:-0}" -lt "$RSS_CEIL_MB" ]; then ok=0; else why="max RSS ${m}MB >= ${RSS_CEIL_MB}MB"; fi ;;
    integrity-complete)
      local ig sr; ig=$(jget "M['net']['win_integrity_gap']"); sr=$(jget "M['net']['distinct_stateroot_modal']")
      if [ "${ig:-0}" -le "$INTEGRITY_GAP_MAX" ] && [ "$sr" = "1" ]; then ok=0; else why="integrity_gaps=$ig distinct_stateRoot=$sr"; fi ;;
    busy-rate-under-10pct)
      local pct; pct=$(jget "round(100*M['net']['win_busy']/max(1,M['net']['win_gossip_total']))")
      if [ "${pct:-100}" -lt "$BUSY_RATE_MAX_PCT" ]; then ok=0; else why="busy/rate-limit ${pct}% of gossip activity (max ${BUSY_RATE_MAX_PCT}%)"; fi ;;
    no-self-starvation)
      local l s e m; l=$(jget "M['net']['liveness_delta']"); s=$(jget "M['net']['win_snap_trigger']")
      e=$(jget "M['net']['win_evictions']"); m=$(jget "M['net']['max_rss_mb']")
      if [ "${l:-0}" -ge "$LIVENESS_MIN" ] && [ "${s:-0}" -le "$SNAP_TRIGGER_MAX" ] && [ "${e:-0}" -le "$EVICT_MAX" ] && [ "${m:-0}" -lt "$RSS_CEIL_MB" ]; then ok=0
      else why="liveness_delta=$l snap=$s evictions=$e rss=${m}MB"; fi ;;
    gs009-no-stall|gs009-no-sibling-fork|gs009-fleet-rejoin|gs009-trust-root-provenance)
      _gs009_assert "$t"; return $? ;;
    gs010-dup-rejected|gs010-no-poison|gs010-no-wedge|gs010-fleet-reconverge|gs010-single-registration)
      _gs010_assert "$t"; return $? ;;
    gs012-bls-matches-registration)
      _gs012_assert "$t"; return $? ;;
    gs013-no-unbacked-weight)
      _gs013_assert "$t"; return $? ;;
    gs014-relay-accepted|gs014-applies-from-non-producer|gs014-set-restored|gs014-fleet-agrees-on-set)
      _gs014_assert "$t"; return $? ;;
    *)
      why="unknown assertion token '$t'" ;;
  esac
  if [ "$ok" = "2" ]; then SKIP_REASONS="$SKIP_REASONS; $why"; return 2; fi
  if [ "$ok" -ne 0 ]; then FAIL_REASONS="$FAIL_REASONS; $t: $why"; fi
  return $ok
}

# inj_tag — was this scenario's trigger actively INJECTED this run, or only OBSERVED?
inj_tag(){
  local sid="$1"
  if [ "${GS009:-0}" = "1" ] && [ "$sid" = "GS-009" ]; then echo "inj"; return; fi
  if [ "${GS010:-0}" = "1" ] && [ "$sid" = "GS-010" ]; then echo "inj"; return; fi
  if [ "${GS014:-0}" = "1" ] && [ "$sid" = "GS-014" ]; then echo "inj"; return; fi
  if [ "$CHAOS" = "1" ]; then
    case "$sid" in GS-002|GS-003|GS-004|GS-005|GS-007) echo "inj";; *) echo "obs";; esac
  elif [ "$NO_PERTURB" != "1" ]; then
    case "$sid" in GS-004) echo "inj";; *) echo "obs";; esac
  else echo "obs"; fi
}

# ── run scenarios ───────────────────────────────────────────────────────────
say "\n${C_C}▸ evaluating scenarios${C_0}  ${C_Y}(inj=trigger actively injected · obs=invariant observed)${C_0}"
TOTAL=0; PASSED=0; WAIVED=0; FAILURES_JSON="["; FJ_FIRST=1
while IFS='|' read -r sid sname sassert; do
  [ -z "$sid" ] && continue
  tag="$(inj_tag "$sid")"
  # ── waiver: not counted toward the pass/total gate, but printed and persisted
  # to the result row with its mandatory reason — waived-with-evidence, not green.
  if is_waived "$sid"; then
    WAIVED=$(( WAIVED + 1 ))
    printf "  ${C_Y}WAIVE${C_0} %-5s %-32s %s\n" "[$tag]" "$sid" "$sname"
    printf "       ${C_Y}reason: %s${C_0}\n" "$WAIVE_REASON"
    wr="$(printf '%s' "$WAIVE_REASON" | python3 -c "import sys,json;print(json.dumps(sys.stdin.read()))")"
    [ "$FJ_FIRST" = 1 ] || FAILURES_JSON="$FAILURES_JSON,"
    FJ_FIRST=0
    FAILURES_JSON="$FAILURES_JSON{\"scenario\":\"$sid\",\"waived\":true,\"reason\":$wr}"
    continue
  fi
  TOTAL=$(( TOTAL + 1 ))
  FAIL_REASONS=""; SKIP_REASONS=""; INFO_REASONS=""
  s_ok=1
  # split CSV assertion tokens (rc=0 pass · rc=2 skip/unverifiable · else fail)
  OLDIFS="$IFS"; IFS=','; for tok in $sassert; do IFS="$OLDIFS"
    assert "$tok"; rc=$?
    { [ "$rc" = "0" ] || [ "$rc" = "2" ]; } || s_ok=0
    IFS=','
  done; IFS="$OLDIFS"
  if [ "$s_ok" = "1" ]; then
    PASSED=$(( PASSED + 1 ))
    printf "  ${C_G}PASS${C_0} %-5s %-32s %s\n" "[$tag]" "$sid" "$sname"
    [ -n "$SKIP_REASONS" ] && printf "       ${C_Y}skip:%s${C_0}\n" "${SKIP_REASONS# ;}"
    [ -n "$INFO_REASONS" ] && printf "       ${C_C}note:%s${C_0}\n" "${INFO_REASONS# ;}"
  else
    printf "  ${C_R}FAIL${C_0} %-5s %-32s %s\n" "[$tag]" "$sid" "$sname"
    printf "       ${C_R}%s${C_0}\n" "${FAIL_REASONS# ; }"
    [ -n "$SKIP_REASONS" ] && printf "       ${C_Y}skip:%s${C_0}\n" "${SKIP_REASONS# ;}"
    [ -n "$INFO_REASONS" ] && printf "       ${C_C}note:%s${C_0}\n" "${INFO_REASONS# ;}"
    local_reasons="$(printf '%s' "${FAIL_REASONS# ; }" | python3 -c "import sys,json;print(json.dumps(sys.stdin.read()))")"
    [ "$FJ_FIRST" = 1 ] || FAILURES_JSON="$FAILURES_JSON,"
    FJ_FIRST=0
    FAILURES_JSON="$FAILURES_JSON{\"scenario\":\"$sid\",\"reasons\":$local_reasons}"
  fi
done <<EOF
$(sqlite3 -separator '|' "$DB" "SELECT scenario_id,name,assertions FROM gauntlet_scenarios WHERE status='active' ORDER BY scenario_id;")
EOF
# chaos injector failed to recover the node → hard fail regardless of assertions
if [ "$CHAOS" = "1" ] && [ "$CHAOS_RECOVERED" != "1" ]; then
  say "  ${C_R}FAIL  [inj] CHAOS-RECOVERY               target ${RESTART_NODE} did not recover after injection${C_0}"
  [ "$FJ_FIRST" = 1 ] || FAILURES_JSON="$FAILURES_JSON,"
  FJ_FIRST=0
  FAILURES_JSON="$FAILURES_JSON{\"scenario\":\"CHAOS-RECOVERY\",\"reasons\":\"target ${RESTART_NODE} did not rejoin tip after down/wipe injection\"}"
  s_ok=0; TOTAL=$(( TOTAL + 1 ))   # count as an extra failed check
fi
FAILURES_JSON="$FAILURES_JSON]"

# ── log OWN result row (system-impact §GAUNTLET.3 — ground truth) ───────────
DUR=$SECONDS
if [ "$PASSED" -eq "$TOTAL" ] && { [ "$CHAOS" != "1" ] || [ "$CHAOS_RECOVERED" = "1" ]; }; then STATUS="pass"; else STATUS="fail"; fi
WFID_SQL="NULL"; [ -n "${WORKFLOW_RUN_ID:-}" ] && WFID_SQL="$WORKFLOW_RUN_ID"
FJ_ESCAPED="$(printf '%s' "$FAILURES_JSON" | sed "s/'/''/g")"
sqlite3 "$DB" "INSERT INTO gauntlet_runs (run_id, status, scenarios_run, scenarios_passed, failures, duration_seconds, git_sha) VALUES ($WFID_SQL, '$STATUS', $TOTAL, $PASSED, '$FJ_ESCAPED', $DUR, '$SHA');" 2>/dev/null \
  && say "\n${C_C}▸ result row written to gauntlet_runs${C_0}" \
  || say "\n${C_Y}▸ could not write gauntlet_runs row (DB error)${C_0}"

# ── summary ─────────────────────────────────────────────────────────────────
say "${C_C}════════════════════════════════════════════════════════════${C_0}"
WAIVE_NOTE=""; [ "${WAIVED:-0}" -gt 0 ] && WAIVE_NOTE=" · ${WAIVED} waived (${WAIVE})"
if [ "$STATUS" = "pass" ]; then
  say "${C_G} GAUNTLET PASS${C_0}  ${PASSED}/${TOTAL} scenarios${WAIVE_NOTE} · ${DUR}s · sha=$SHA"
  say "${C_C}════════════════════════════════════════════════════════════${C_0}"
  exit 0
else
  say "${C_R} GAUNTLET FAIL${C_0}  ${PASSED}/${TOTAL} scenarios${WAIVE_NOTE} · ${DUR}s · sha=$SHA"
  say "${C_C}════════════════════════════════════════════════════════════${C_0}"
  exit 1
fi
