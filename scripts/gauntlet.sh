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
#
# Assertions key off STRUCTURED telemetry fields (gap=, rollback_depth=,
# sync_fails=, state=) and distinct-event phrases — NEVER raw keywords that also
# appear in per-second telemetry (the word "rollback" logs ~1/sec at depth 0).
#
# Usage:  bash scripts/gauntlet.sh [--quick]
# Env:    WORKFLOW_RUN_ID, GAUNTLET_WINDOW (s, default 45), GAUNTLET_RESTART_NODE
#         (default n5), GAUNTLET_RSS_CEIL_MB (default 800), GAUNTLET_MIN_NODES
#         (default 3), GAUNTLET_NO_PERTURB (1=skip restart).
# Exit:   0 all scenarios passed · 1 a scenario failed · 3 preflight failed.
# ============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="$ROOT/.omega/memory.db"
COLLECT="$ROOT/scripts/gauntlet-collect.py"
LOG_DIR="$HOME/testnet/logs"
LABEL_PREFIX="network.doli.testnet"

WINDOW="${GAUNTLET_WINDOW:-45}"
RESTART_NODE="${GAUNTLET_RESTART_NODE:-n5}"
RSS_CEIL_MB="${GAUNTLET_RSS_CEIL_MB:-800}"
MIN_NODES="${GAUNTLET_MIN_NODES:-3}"
NO_PERTURB="${GAUNTLET_NO_PERTURB:-0}"
[ "${1:-}" = "--quick" ] && WINDOW=20

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

say "${C_C}════════════════════════════════════════════════════════════${C_0}"
say "${C_C} OMEGA GAUNTLET${C_0}  ·  sha=$SHA  window=${WINDOW}s  restart=${RESTART_NODE}"
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

say "\n${C_C}▸ capturing baseline (log offsets + heights)${C_0}"
build_nodecfg
BASE_MAX=$(python3 -c "import json;d=json.load(open('$NODECFG'));print(max(n['baseline_height'] for n in d['nodes']))")
say "  baseline max height = $BASE_MAX"

# ── GS-004 perturbation: safe launchd single-node restart ───────────────────
if [ "$NO_PERTURB" = "1" ]; then
  say "\n${C_Y}▸ perturbation skipped (GAUNTLET_NO_PERTURB=1) — observational only${C_0}"
  echo "0" > "$REJOIN_FILE"
else
  say "\n${C_C}▸ perturbation: launchd restart of ${RESTART_NODE} (non-destructive)${C_0}"
  RP="$(port_of "$RESTART_NODE")"
  if echo "$LIVE" | grep -qw "$RESTART_NODE"; then
    "$ROOT/scripts/testnet.sh" restart "$RESTART_NODE" >/dev/null 2>&1 || say "  ${C_Y}restart command returned nonzero (continuing)${C_0}"
    # measure rejoin: time until node RPC responds AND height within 1 of net tip
    t0=$(date +%s); rejoin=-1
    while [ $(( $(date +%s) - t0 )) -le 90 ]; do
      h="$(height_of "$RP")"
      if [ -n "$h" ] && [ "$h" != "-1" ]; then
        # net tip = current max across other live nodes
        tip=0
        for name in $LIVE; do
          [ "$name" = "$RESTART_NODE" ] && continue
          nh="$(height_of "$(port_of "$name")")"; [ "${nh:-0}" -gt "$tip" ] && tip="$nh"
        done
        if [ "$h" -ge $(( tip - 1 )) ] && [ "$h" -gt 0 ]; then rejoin=$(( $(date +%s) - t0 )); break; fi
      fi
      sleep 3
    done
    echo "$rejoin" > "$REJOIN_FILE"
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

# ── assertion engine ────────────────────────────────────────────────────────
# jget <python-expr on M> — M is the parsed metrics dict.
jget(){ python3 -c "import json;M=json.load(open('$METRICS'));print($1)" 2>/dev/null; }
FAIL_REASONS=""   # accumulates within a single scenario

# assert <token> — returns 0 pass / 1 fail; appends human reason to FAIL_REASONS.
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
      local g b; g=$(jget "M['net']['distinct_genesis']"); b=$(jget "M['net']['distinct_block1']")
      if [ "$g" = "1" ] && [ "$b" = "1" ]; then ok=0; else why="distinct genesis=$g block1=$b (want 1/1)"; fi ;;
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
    *)
      why="unknown assertion token '$t'" ;;
  esac
  if [ "$ok" -ne 0 ]; then FAIL_REASONS="$FAIL_REASONS; $t: $why"; fi
  return $ok
}

# ── run scenarios ───────────────────────────────────────────────────────────
say "\n${C_C}▸ evaluating scenarios${C_0}"
TOTAL=0; PASSED=0; FAILURES_JSON="["; FJ_FIRST=1
while IFS='|' read -r sid sname sassert; do
  [ -z "$sid" ] && continue
  TOTAL=$(( TOTAL + 1 ))
  FAIL_REASONS=""
  s_ok=1
  # split CSV assertion tokens
  OLDIFS="$IFS"; IFS=','; for tok in $sassert; do IFS="$OLDIFS"; assert "$tok" || s_ok=0; IFS=','; done; IFS="$OLDIFS"
  if [ "$s_ok" = "1" ]; then
    PASSED=$(( PASSED + 1 ))
    printf "  ${C_G}PASS${C_0} %-32s %s\n" "$sid" "$sname"
  else
    printf "  ${C_R}FAIL${C_0} %-32s %s\n" "$sid" "$sname"
    printf "       ${C_R}%s${C_0}\n" "${FAIL_REASONS# ; }"
    local_reasons="$(printf '%s' "${FAIL_REASONS# ; }" | python3 -c "import sys,json;print(json.dumps(sys.stdin.read()))")"
    [ "$FJ_FIRST" = 1 ] || FAILURES_JSON="$FAILURES_JSON,"
    FJ_FIRST=0
    FAILURES_JSON="$FAILURES_JSON{\"scenario\":\"$sid\",\"reasons\":$local_reasons}"
  fi
done <<EOF
$(sqlite3 -separator '|' "$DB" "SELECT scenario_id,name,assertions FROM gauntlet_scenarios WHERE status='active' ORDER BY scenario_id;")
EOF
FAILURES_JSON="$FAILURES_JSON]"

# ── log OWN result row (system-impact §GAUNTLET.3 — ground truth) ───────────
DUR=$SECONDS
if [ "$PASSED" -eq "$TOTAL" ]; then STATUS="pass"; else STATUS="fail"; fi
WFID_SQL="NULL"; [ -n "${WORKFLOW_RUN_ID:-}" ] && WFID_SQL="$WORKFLOW_RUN_ID"
FJ_ESCAPED="$(printf '%s' "$FAILURES_JSON" | sed "s/'/''/g")"
sqlite3 "$DB" "INSERT INTO gauntlet_runs (run_id, status, scenarios_run, scenarios_passed, failures, duration_seconds, git_sha) VALUES ($WFID_SQL, '$STATUS', $TOTAL, $PASSED, '$FJ_ESCAPED', $DUR, '$SHA');" 2>/dev/null \
  && say "\n${C_C}▸ result row written to gauntlet_runs${C_0}" \
  || say "\n${C_Y}▸ could not write gauntlet_runs row (DB error)${C_0}"

# ── summary ─────────────────────────────────────────────────────────────────
say "${C_C}════════════════════════════════════════════════════════════${C_0}"
if [ "$STATUS" = "pass" ]; then
  say "${C_G} GAUNTLET PASS${C_0}  ${PASSED}/${TOTAL} scenarios · ${DUR}s · sha=$SHA"
  say "${C_C}════════════════════════════════════════════════════════════${C_0}"
  exit 0
else
  say "${C_R} GAUNTLET FAIL${C_0}  ${PASSED}/${TOTAL} scenarios · ${DUR}s · sha=$SHA"
  say "${C_C}════════════════════════════════════════════════════════════${C_0}"
  exit 1
fi
