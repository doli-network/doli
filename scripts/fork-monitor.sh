#!/usr/bin/env bash
# fork-monitor.sh — Detect chain forks, offline nodes, and lagging nodes across
# the DOLI fleet, with optional Telegram alerting on state transitions.
#
# Usage:
#   scripts/fork-monitor.sh                    # scan local devnet ports (28500-28550)
#   scripts/fork-monitor.sh --testnet          # scan testnet ports (8500-8512)
#   scripts/fork-monitor.sh --loop [SECS]      # continuous monitoring (default: 30s)
#
# Detected events:
#   fork     — two or more reachable nodes report different bestHash
#   offline  — a node that was previously reachable is no longer reachable
#   behind   — a node's bestHeight lags the fleet max by >= DOLI_BEHIND_THRESHOLD
#
# Telegram alerting (optional):
#   Set DOLI_TELEGRAM_BOT_TOKEN and DOLI_TELEGRAM_CHAT_ID to enable push alerts.
#   Alerts fire on STATE TRANSITIONS ONLY — one message when an issue starts,
#   one "recovered" message when it clears. No repeating reminders.
#
# State persistence:
#   State is persisted to $DOLI_MONITOR_STATE_DIR/fork-monitor-$MODE.json
#   (default: ~/.doli/monitor-state/). First run establishes a baseline from
#   the current poll, so starting the monitor on an already-forked network
#   will immediately emit entry alerts for every active issue.
#
# Env vars:
#   DOLI_TELEGRAM_BOT_TOKEN   Bot token — enables alerting
#   DOLI_TELEGRAM_CHAT_ID     Chat ID — enables alerting
#   DOLI_BEHIND_THRESHOLD     Blocks a node can lag before flagged (default: 10)
#   DOLI_MONITOR_STATE_DIR    State directory (default: ~/.doli/monitor-state)
#
# Exit codes: 0 = all healthy, 1 = issue detected, 2 = error
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TELEGRAM_ALERT="$SCRIPT_DIR/telegram-alert.sh"

# Defaults
MODE="devnet"
LOOP=false
LOOP_INTERVAL=30
BEHIND_THRESHOLD="${DOLI_BEHIND_THRESHOLD:-10}"
STATE_DIR="${DOLI_MONITOR_STATE_DIR:-$HOME/.doli/monitor-state}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --testnet) MODE="testnet"; shift ;;
    --devnet)  MODE="devnet";  shift ;;
    --loop)
      LOOP=true
      if [[ "${2:-}" =~ ^[0-9]+$ ]]; then
        LOOP_INTERVAL="$2"; shift
      fi
      shift ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \?//'; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; exit 2 ;;
  esac
done

STATE_FILE="$STATE_DIR/fork-monitor-$MODE.json"
mkdir -p "$STATE_DIR"

rpc_call() {
  local port="$1" method="$2"
  curl -sf --max-time 3 -X POST "http://127.0.0.1:${port}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":{},\"id\":1}" 2>/dev/null
}

# ---- Python state machine + alerting ----------------------------------------
# Reads NODE_DATA lines (name|status|height|hash) from env, diffs against
# previous state file, emits stdout display, and invokes TELEGRAM_ALERT for
# every transition. Python handles all the stateful logic to keep bash sane.
# Heredoc uses a QUOTED delimiter ('PYEOF') so nothing inside is expanded,
# letting us write Python naturally with quotes, backslashes, and f-strings.
PY_STATE_MACHINE=$(cat <<'PYEOF'
import sys, json, os, subprocess, time

STATE_FILE        = os.environ["STATE_FILE"]
MODE              = os.environ["MODE"]
BEHIND_THRESHOLD  = int(os.environ["BEHIND_THRESHOLD"])
TELEGRAM_ALERT    = os.environ["TELEGRAM_ALERT"]
TIMESTAMP         = os.environ["TIMESTAMP"]
NODE_DATA         = os.environ.get("NODE_DATA", "")

# ANSI color constants for stdout display
RED    = "\033[0;31m"
GREEN  = "\033[0;32m"
YELLOW = "\033[0;33m"
BOLD   = "\033[1m"
NC     = "\033[0m"


def send_alert(severity, title, body):
    try:
        subprocess.run(
            [TELEGRAM_ALERT, "--severity", severity, "--title", title, "--body", body],
            check=False, timeout=15,
        )
    except Exception as e:
        print(f"fork-monitor: telegram dispatch failed: {e}", file=sys.stderr)


def short_hash(h):
    if not h:
        return "-"
    if len(h) > 18:
        return h[:12] + "..." + h[-6:]
    return h


# ---- parse input ----
nodes = {}
for line in NODE_DATA.splitlines():
    line = line.strip()
    if not line:
        continue
    parts = line.split("|")
    if len(parts) < 4:
        continue
    name, status, height, h = parts[0], parts[1], parts[2], parts[3]
    nodes[name] = {
        "online": status == "ONLINE",
        "height": int(height) if height else None,
        "hash":   h if h else None,
    }

online = {k: v for k, v in nodes.items() if v["online"]}
heights = [v["height"] for v in online.values() if v["height"] is not None]
max_height = max(heights) if heights else 0

# ---- compute current fork groups ----
groups = {}  # short_hash -> {"height", "nodes"}
for name, info in online.items():
    sh = short_hash(info["hash"])
    if sh not in groups:
        groups[sh] = {"height": info["height"], "nodes": []}
    groups[sh]["nodes"].append(name)

if len(groups) <= 1:
    current_fork = "ok"
else:
    current_fork = "fork:" + ",".join(sorted(groups.keys()))

# ---- load previous state ----
try:
    with open(STATE_FILE) as f:
        prev = json.load(f)
except (FileNotFoundError, json.JSONDecodeError):
    prev = {"fork": "ok", "offline": [], "behind": [], "ever_seen_online": []}

prev_fork        = prev.get("fork", "ok")
prev_offline     = set(prev.get("offline", []))
prev_behind      = set(prev.get("behind", []))
ever_seen_online = set(prev.get("ever_seen_online", []))

# Update the "has ever been reachable" set so we do not alert on ports that
# were never populated (relevant for devnet iterating 28500-28550).
for name, info in nodes.items():
    if info["online"]:
        ever_seen_online.add(name)

# ---- compute current offline/behind sets ----
# Only count a node as "offline" if we have actually seen it online before.
current_offline = {n for n, v in nodes.items() if not v["online"]} & ever_seen_online

# "Behind" only applies to online nodes that lag the fleet max.
current_behind = {
    n for n, v in online.items()
    if v["height"] is not None and (max_height - v["height"]) >= BEHIND_THRESHOLD
}

# ---- emit stdout display ----
if current_fork == "ok" and not current_offline and not current_behind:
    if groups:
        sole_hash = list(groups.keys())[0]
        sole_height = list(groups.values())[0]["height"]
    else:
        sole_hash = "-"
        sole_height = 0
    print(f"{GREEN}[{TIMESTAMP}] OK{NC} — {len(online)} nodes, height={sole_height}, hash={sole_hash}")
    exit_code = 0
else:
    print()
    print(f"{RED}{BOLD}[{TIMESTAMP}] DEGRADED{NC} — {len(online)} reachable, max_height={max_height}")
    if current_fork != "ok":
        print(f"  {RED}FORK{NC}: {len(groups)} chain tips")
        for i, (sh, g) in enumerate(sorted(groups.items()), 1):
            g_height = g["height"]
            g_nodes = ", ".join(sorted(g["nodes"]))
            print(f"    Group {i}: hash={sh} height={g_height}")
            print(f"      Nodes: {g_nodes}")
    if current_offline:
        off_list = ", ".join(sorted(current_offline))
        print(f"  {RED}OFFLINE{NC}: {off_list}")
    if current_behind:
        parts = []
        for n in sorted(current_behind):
            h_val = nodes[n]["height"]
            lag = max_height - (h_val if h_val is not None else 0)
            parts.append(f"{n}@{h_val}(-{lag})")
        print(f"  {YELLOW}BEHIND{NC}: {', '.join(parts)}")
    print()
    exit_code = 1

# ---- diff vs previous state and emit transitions ----
transitions = []  # list of (severity, title, body)


def format_fork_body():
    lines = [f"Network: {MODE}", f"Groups: {len(groups)}", ""]
    for i, (sh, g) in enumerate(sorted(groups.items()), 1):
        g_height = g["height"]
        g_nodes = ", ".join(sorted(g["nodes"]))
        lines.append(f"Group {i}: hash={sh} height={g_height}")
        lines.append(f"  Nodes: {g_nodes}")
    return "\n".join(lines)


# Fork transitions
if prev_fork == "ok" and current_fork != "ok":
    transitions.append(("critical", f"FORK DETECTED on {MODE}", format_fork_body()))
elif prev_fork != "ok" and current_fork == "ok":
    if groups:
        sole_hash = list(groups.keys())[0]
        sole_height = list(groups.values())[0]["height"]
    else:
        sole_hash = "unknown"
        sole_height = "?"
    body = (f"Network: {MODE}\n"
            f"All {len(online)} reachable nodes now agree.\n"
            f"height={sole_height} hash={sole_hash}")
    transitions.append(("recovery", f"FORK RECOVERED on {MODE}", body))
elif prev_fork != "ok" and current_fork != "ok" and prev_fork != current_fork:
    body = "Fork topology changed.\n\n" + format_fork_body()
    transitions.append(("critical", f"FORK STATE CHANGED on {MODE}", body))

# Offline transitions (per-node)
new_offline       = current_offline - prev_offline
recovered_offline = prev_offline    - current_offline
if new_offline:
    nodes_str = ", ".join(sorted(new_offline))
    body = f"Network: {MODE}\nNodes that went offline: {nodes_str}"
    transitions.append(("warning", f"NODE OFFLINE on {MODE}", body))
if recovered_offline:
    nodes_str = ", ".join(sorted(recovered_offline))
    body = f"Network: {MODE}\nNodes back online: {nodes_str}"
    transitions.append(("recovery", f"NODE RECOVERED on {MODE}", body))

# Behind transitions (per-node) — skip offline nodes (would be redundant)
current_behind_only = current_behind - current_offline
new_behind       = current_behind_only - prev_behind
recovered_behind = (prev_behind - current_behind_only) - current_offline
if new_behind:
    details = []
    for n in sorted(new_behind):
        h_val = nodes[n].get("height")
        lag = max_height - (h_val if h_val is not None else 0)
        details.append(f"{n}: h={h_val} lag={lag}")
    details_str = "\n".join(details)
    body = (f"Network: {MODE}\n"
            f"max_height: {max_height}\n"
            f"threshold: {BEHIND_THRESHOLD} blocks\n\n"
            f"{details_str}")
    transitions.append(("warning", f"NODE BEHIND on {MODE}", body))
if recovered_behind:
    nodes_str = ", ".join(sorted(recovered_behind))
    body = f"Network: {MODE}\nNodes caught up: {nodes_str}"
    transitions.append(("recovery", f"NODE CAUGHT UP on {MODE}", body))

# ---- persist new state (atomic via temp + rename) ----
new_state = {
    "ts":               int(time.time()),
    "mode":             MODE,
    "fork":             current_fork,
    "offline":          sorted(current_offline),
    "behind":           sorted(current_behind_only),
    "ever_seen_online": sorted(ever_seen_online),
    "max_height":       max_height,
    "groups":           {
        sh: {"height": g["height"], "nodes": sorted(g["nodes"])}
        for sh, g in groups.items()
    },
}
tmp = STATE_FILE + ".tmp"
with open(tmp, "w") as f:
    json.dump(new_state, f, indent=2)
os.replace(tmp, STATE_FILE)

# ---- fire alerts (last, so state is persisted even if dispatch hangs) ----
for sev, title, body in transitions:
    send_alert(sev, title, body)

sys.exit(exit_code)
PYEOF
)

check_integrity() {
  local timestamp
  timestamp=$(date '+%Y-%m-%d %H:%M:%S')

  # Determine port range
  local start_port end_port
  if [[ "$MODE" == "testnet" ]]; then
    start_port=8500; end_port=8512
  else
    start_port=28500; end_port=28550
  fi

  # Collect per-node status: "name|ONLINE|height|hash" or "name|OFFLINE||"
  local node_data=""
  for ((port=start_port; port<=end_port; port++)); do
    local name
    if [[ $port -eq $start_port ]]; then name="seed"; else name="n$((port - start_port))"; fi

    local result height hash
    if result=$(rpc_call "$port" "getChainInfo" 2>/dev/null) && [[ -n "$result" && "$result" != "null" ]]; then
      height=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('bestHeight',''))" 2>/dev/null || echo "")
      hash=$(echo "$result" | python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('bestHash',''))" 2>/dev/null || echo "")
      if [[ -n "$height" && -n "$hash" ]]; then
        node_data+="${name}|ONLINE|${height}|${hash}"$'\n'
        continue
      fi
    fi
    node_data+="${name}|OFFLINE||"$'\n'
  done

  # Run the Python state machine
  NODE_DATA="$node_data" \
  STATE_FILE="$STATE_FILE" \
  MODE="$MODE" \
  BEHIND_THRESHOLD="$BEHIND_THRESHOLD" \
  TELEGRAM_ALERT="$TELEGRAM_ALERT" \
  TIMESTAMP="$timestamp" \
  python3 -c "$PY_STATE_MACHINE"
}

# Main
if [[ "$LOOP" == true ]]; then
  TELEGRAM_STATUS="disabled (set DOLI_TELEGRAM_BOT_TOKEN)"
  if [[ -n "${DOLI_TELEGRAM_BOT_TOKEN:-}" && -n "${DOLI_TELEGRAM_CHAT_ID:-}" ]]; then
    TELEGRAM_STATUS="enabled"
  fi
  echo -e "${BOLD}Fork monitor running (every ${LOOP_INTERVAL}s). Ctrl+C to stop.${NC}"
  echo "  Mode:       $MODE"
  echo "  State file: $STATE_FILE"
  echo "  Telegram:   $TELEGRAM_STATUS"
  echo ""
  while true; do
    check_integrity || true
    sleep "$LOOP_INTERVAL"
  done
else
  check_integrity
fi
