#!/usr/bin/env bash
# node-heal.sh — Recover a poisoned/forked producer by rebuilding its data/
# directory from a healthy node.
#
# DESTRUCTIVE. Stops the target node, wipes its data/ directory, rsyncs the
# contents of a healthy node's data/ over, then restarts it. The target's
# node_key (libp2p identity) and wallet keys live OUTSIDE data/ and are
# preserved.
#
# Intended only for N# producer nodes, never seeds. The healthy source must
# be on the canonical chain — the script performs an RPC pre-check but the
# final judgement is on the operator.
#
# Usage:
#
#   # Testnet preset (local launchd, resolves everything):
#   scripts/node-heal.sh --testnet --target n5 --source n3 --yes
#
#   # Mainnet / generic (run on the target server; SSH'd in):
#   scripts/node-heal.sh \
#       --target-data  ~/.doli/mainnet/data \
#       --source-data  healthy-host:~/.doli/mainnet/data \
#       --stop-cmd     "sudo systemctl stop doli-n5" \
#       --start-cmd    "sudo systemctl start doli-n5" \
#       --target-rpc   127.0.0.1:8500 \
#       --source-rpc   127.0.0.1:8503 \
#       --yes
#
# Flags:
#   --testnet                    Testnet preset mode (requires --target, --source)
#   --target NAME                Testnet node name (n1..n12) — preset mode only
#   --source NAME                Testnet source node name — preset mode only
#   --target-data PATH           Target node's data directory (generic mode)
#   --source-data PATH|HOST:PATH Source data directory (local or remote for rsync)
#   --target-rpc HOST:PORT       Target RPC for pre/post health checks
#   --source-rpc HOST:PORT       Source RPC for pre-check (locally reachable)
#   --skip-source-rpc-check      Don't attempt source RPC check (cross-server setups)
#   --stop-cmd "CMD"             Command to stop the target node
#   --start-cmd "CMD"            Command to start the target node
#   --wipe-signed-slots          DANGEROUS: also wipe signed_slots.db (slashing protection).
#                                Default is to preserve it. Only use on fresh producers
#                                that have never signed a slot on any chain.
#   --yes                        Skip interactive confirmation
#   --dry-run                    Print plan without executing
#   -h, --help                   Show this help
#
# Safety notes (see .claude/skills/guardian/SKILL.md sections L1, L1.1, L6):
#   - signed_slots.db is PRESERVED by default — it's slashing protection, not chain
#     state, and losing it re-exposes the producer to double-sign slashing.
#   - utxo_store/ is NOT copied from the source — the node rebuilds it from
#     state_db on startup (self-heals since v6.7.9, per INC-I-027). Copying
#     utxo_store across binaries risks a silent-corruption rollback cascade.
#
# Exit codes: 0 = healed, 1 = aborted/error, 2 = bad args
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  sed -n '2,/^set -euo/p' "$0" | grep '^#' | sed 's/^# \?//'
  exit 2
}

# ---- arg parsing ----
PRESET=""
TARGET_NAME=""
SOURCE_NAME=""
TARGET_DATA=""
SOURCE_DATA=""
TARGET_RPC=""
SOURCE_RPC=""
STOP_CMD=""
START_CMD=""
SKIP_SOURCE_RPC=false
WIPE_SIGNED_SLOTS=false
YES=false
DRY_RUN=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --testnet)               PRESET="testnet"; shift ;;
    --target)                TARGET_NAME="$2"; shift 2 ;;
    --source)                SOURCE_NAME="$2"; shift 2 ;;
    --target-data)           TARGET_DATA="$2"; shift 2 ;;
    --source-data)           SOURCE_DATA="$2"; shift 2 ;;
    --target-rpc)            TARGET_RPC="$2"; shift 2 ;;
    --source-rpc)            SOURCE_RPC="$2"; shift 2 ;;
    --skip-source-rpc-check) SKIP_SOURCE_RPC=true; shift ;;
    --stop-cmd)              STOP_CMD="$2"; shift 2 ;;
    --start-cmd)             START_CMD="$2"; shift 2 ;;
    --wipe-signed-slots)     WIPE_SIGNED_SLOTS=true; shift ;;
    --yes)                   YES=true; shift ;;
    --dry-run)               DRY_RUN=true; shift ;;
    -h|--help)               usage ;;
    *) echo "Unknown arg: $1"; usage ;;
  esac
done

# ---- preset expansion ----
if [[ "$PRESET" == "testnet" ]]; then
  [[ -z "$TARGET_NAME" || -z "$SOURCE_NAME" ]] && { echo "--testnet requires --target and --source"; exit 2; }
  [[ "$TARGET_NAME" == "$SOURCE_NAME" ]] && { echo "target and source must differ"; exit 2; }
  [[ "$TARGET_NAME" == "seed" || "$SOURCE_NAME" == "seed" ]] && { echo "refusing to heal/use seed nodes — producers only"; exit 2; }

  # Resolve ports: nN → 8500+N
  if [[ ! "$TARGET_NAME" =~ ^n([0-9]+)$ ]]; then
    echo "target must match nN pattern (got: $TARGET_NAME)"; exit 2
  fi
  TARGET_IDX="${BASH_REMATCH[1]}"
  if [[ ! "$SOURCE_NAME" =~ ^n([0-9]+)$ ]]; then
    echo "source must match nN pattern (got: $SOURCE_NAME)"; exit 2
  fi
  SOURCE_IDX="${BASH_REMATCH[1]}"

  TARGET_DATA="$HOME/testnet/$TARGET_NAME/data"
  SOURCE_DATA="$HOME/testnet/$SOURCE_NAME/data"
  TARGET_RPC="127.0.0.1:$((8500 + TARGET_IDX))"
  SOURCE_RPC="127.0.0.1:$((8500 + SOURCE_IDX))"
  STOP_CMD="$SCRIPT_DIR/testnet.sh stop $TARGET_NAME"
  START_CMD="$SCRIPT_DIR/testnet.sh start $TARGET_NAME"
fi

# ---- validate required ----
for var in TARGET_DATA SOURCE_DATA STOP_CMD START_CMD; do
  if [[ -z "${!var}" ]]; then
    flag=$(echo "$var" | tr '[:upper:]_' '[:lower:]-')
    echo "Missing required: --$flag"
    exit 2
  fi
done

# ---- helpers ----
rpc_call() {
  local endpoint="$1" method="$2"
  curl -sf --max-time 5 -X POST "http://${endpoint}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":{},\"id\":1}" 2>/dev/null
}

json_field() {
  python3 -c "import sys,json; r=json.load(sys.stdin).get('result',{}); print(r.get('$1',''))" 2>/dev/null
}

# Detect remote rsync source: "host:path" where host has no leading "/"
is_remote_source() {
  [[ "$1" =~ ^[^/][^/:]*: ]]
}

# Short hash formatter for display
short_hash() {
  local h="$1"
  if [[ ${#h} -gt 18 ]]; then
    echo "${h:0:12}...${h: -6}"
  else
    echo "$h"
  fi
}

run() {
  if [[ "$DRY_RUN" == true ]]; then
    echo -e "  ${YELLOW}[dry-run]${NC} $*"
  else
    eval "$@"
  fi
}

# ---- pre-check: source health ----
echo -e "${BOLD}=== NODE HEAL ===${NC}"
echo ""
echo "  Target data : $TARGET_DATA"
echo "  Source data : $SOURCE_DATA"
echo "  Target RPC  : ${TARGET_RPC:-<none>}"
echo "  Source RPC  : ${SOURCE_RPC:-<none>}"
echo "  Stop cmd    : $STOP_CMD"
echo "  Start cmd   : $START_CMD"
echo ""

SOURCE_HEIGHT="?"
SOURCE_HASH="?"
if [[ -n "$SOURCE_RPC" && "$SKIP_SOURCE_RPC" != true ]]; then
  echo -n "Checking source RPC ($SOURCE_RPC)... "
  src_info=$(rpc_call "$SOURCE_RPC" "getChainInfo") || {
    echo -e "${RED}UNREACHABLE${NC}"
    echo "Cannot confirm source is healthy. Use --skip-source-rpc-check to override."
    exit 1
  }
  SOURCE_HEIGHT=$(echo "$src_info" | json_field bestHeight)
  SOURCE_HASH=$(echo "$src_info"   | json_field bestHash)
  [[ -z "$SOURCE_HEIGHT" || -z "$SOURCE_HASH" ]] && { echo -e "${RED}EMPTY RESPONSE${NC}"; exit 1; }
  echo -e "${GREEN}OK${NC} — height=$SOURCE_HEIGHT hash=$(short_hash "$SOURCE_HASH")"
elif [[ "$SKIP_SOURCE_RPC" == true ]]; then
  echo -e "${YELLOW}Source RPC check SKIPPED${NC} (operator asserts source is healthy)"
else
  echo -e "${YELLOW}Source RPC check SKIPPED${NC} (no --source-rpc provided)"
fi

TARGET_HEIGHT="?"
TARGET_HASH="?"
if [[ -n "$TARGET_RPC" ]]; then
  if tgt_info=$(rpc_call "$TARGET_RPC" "getChainInfo"); then
    TARGET_HEIGHT=$(echo "$tgt_info" | json_field bestHeight)
    TARGET_HASH=$(echo "$tgt_info"   | json_field bestHash)
  fi
fi

echo ""
echo -e "${BOLD}About to DESTROY target data.${NC}"
echo "  Current target state : height=${TARGET_HEIGHT} hash=$(short_hash "${TARGET_HASH:-?}")"
echo "  Will be replaced with: height=${SOURCE_HEIGHT} hash=$(short_hash "${SOURCE_HASH:-?}")"
echo ""

# Source must not equal target path (would wipe source while rsync from it)
if ! is_remote_source "$SOURCE_DATA" && [[ "$(realpath "$SOURCE_DATA" 2>/dev/null || echo "$SOURCE_DATA")" == "$(realpath "$TARGET_DATA" 2>/dev/null || echo "$TARGET_DATA")" ]]; then
  echo -e "${RED}ERROR: source and target resolve to the same path${NC}"
  exit 1
fi

# Verify local source exists and looks like a DOLI data dir
if ! is_remote_source "$SOURCE_DATA"; then
  if [[ ! -d "$SOURCE_DATA" ]]; then
    echo -e "${RED}ERROR: source data directory does not exist: $SOURCE_DATA${NC}"
    exit 1
  fi
  if [[ ! -d "$SOURCE_DATA/state_db" || ! -d "$SOURCE_DATA/blocks" ]]; then
    echo -e "${RED}ERROR: source does not look like a DOLI data/ dir (missing state_db/ or blocks/)${NC}"
    exit 1
  fi
fi

# Verify target data path exists so we're not creating state in the wrong place
if [[ ! -d "$TARGET_DATA" ]]; then
  echo -e "${YELLOW}WARN: target data directory does not exist yet: $TARGET_DATA${NC}"
  echo "Creating parent directory; verify this is the right node before confirming."
fi

if [[ "$YES" != true ]]; then
  read -rp "Proceed with heal? Type 'yes' to continue: " confirm
  [[ "$confirm" != "yes" ]] && { echo "Aborted."; exit 1; }
fi

# ---- execute ----
echo ""
echo -e "${BOLD}Step 1/5:${NC} Stop target node"
run "$STOP_CMD"

# Wait for target RPC to stop responding (up to 20s)
if [[ -n "$TARGET_RPC" && "$DRY_RUN" != true ]]; then
  echo -n "  Waiting for target to release RocksDB locks"
  for _ in $(seq 1 20); do
    if ! rpc_call "$TARGET_RPC" "getChainInfo" >/dev/null 2>&1; then
      echo " — stopped"
      break
    fi
    echo -n "."
    sleep 1
  done
  # Also give filesystem a moment to flush
  sleep 2
fi

echo ""
echo -e "${BOLD}Step 2/5:${NC} Wipe target data (preserving signed_slots.db unless --wipe-signed-slots)"
if [[ "$DRY_RUN" == true ]]; then
  if [[ "$WIPE_SIGNED_SLOTS" == true ]]; then
    echo -e "  ${YELLOW}[dry-run]${NC} find $TARGET_DATA -mindepth 1 -delete  (wipes signed_slots.db too)"
  else
    echo -e "  ${YELLOW}[dry-run]${NC} find $TARGET_DATA -mindepth 1 ! -name signed_slots.db ! -path '*/signed_slots.db/*' -delete"
  fi
else
  mkdir -p "$TARGET_DATA"
  if [[ "$WIPE_SIGNED_SLOTS" == true ]]; then
    echo -e "  ${RED}WIPING signed_slots.db${NC} (operator override)"
    find "$TARGET_DATA" -mindepth 1 -delete
  else
    # Preserve signed_slots.db (slashing protection) per L6
    find "$TARGET_DATA" -mindepth 1 \
      ! -name signed_slots.db \
      ! -path "$TARGET_DATA/signed_slots.db/*" \
      -delete 2>/dev/null || true
  fi
  echo -e "  ${GREEN}OK${NC} — $TARGET_DATA wiped"
fi

echo ""
echo -e "${BOLD}Step 3/5:${NC} Rsync from source"
# Trailing slashes matter: source/ (contents) → target/ (into)
# Exclude signed_slots.db (preserve target's slashing protection — L6)
# Exclude utxo_store/ (regenerated by node on startup, avoids L1.1 silent corruption)
# Exclude producer.lock and pending_update.json (stale runtime state)
EXCLUDES=(
  --exclude='signed_slots.db'
  --exclude='signed_slots.db/'
  --exclude='utxo_store'
  --exclude='utxo_store/'
  --exclude='producer.lock'
  --exclude='pending_update.json'
)
if [[ "$WIPE_SIGNED_SLOTS" == true ]]; then
  # Operator wants the slashing DB reset too — still don't copy source's (wrong keys)
  :
fi

if is_remote_source "$SOURCE_DATA"; then
  RSYNC_OPTS=(-az --info=progress2 -e ssh)
else
  RSYNC_OPTS=(-a --info=progress2)
fi
RSYNC_SRC="${SOURCE_DATA%/}/"
run rsync "${RSYNC_OPTS[@]}" "${EXCLUDES[@]}" "$RSYNC_SRC" "${TARGET_DATA%/}/"

echo ""
echo -e "${BOLD}Step 4/5:${NC} Verify preserved files"
if [[ "$DRY_RUN" != true ]]; then
  if [[ "$WIPE_SIGNED_SLOTS" != true ]]; then
    if [[ -d "$TARGET_DATA/signed_slots.db" ]]; then
      echo -e "  ${GREEN}OK${NC} signed_slots.db preserved"
    else
      echo -e "  ${YELLOW}NOTE${NC} no signed_slots.db on target (fresh producer — will be created on first sign)"
    fi
  fi
  # utxo_store should NOT exist after rsync (we excluded it); node will rebuild
  if [[ -d "$TARGET_DATA/utxo_store" ]]; then
    echo -e "  ${YELLOW}WARN${NC} utxo_store/ present — should have been excluded from rsync"
  else
    echo -e "  ${GREEN}OK${NC} utxo_store/ absent — node will rebuild from state_db on startup"
  fi
fi

echo ""
echo -e "${BOLD}Step 5/5:${NC} Start target node"
run "$START_CMD"

# ---- post-check ----
if [[ -n "$TARGET_RPC" && "$DRY_RUN" != true ]]; then
  echo ""
  echo -n "  Waiting for target RPC to come back"
  for _ in $(seq 1 30); do
    if tgt_info=$(rpc_call "$TARGET_RPC" "getChainInfo"); then
      new_h=$(echo "$tgt_info" | json_field bestHeight)
      new_hash=$(echo "$tgt_info" | json_field bestHash)
      if [[ -n "$new_h" ]]; then
        echo " — up"
        echo -e "  ${GREEN}Target online${NC}: height=$new_h hash=$(short_hash "$new_hash")"
        break
      fi
    fi
    echo -n "."
    sleep 1
  done
fi

echo ""
echo -e "${GREEN}${BOLD}Heal complete.${NC}"
echo ""
echo "Next steps:"
echo "  - Monitor target via: scripts/fork-monitor.sh${PRESET:+ --testnet}"
echo "  - Tail logs: scripts/testnet.sh logs ${TARGET_NAME:-<node>}"
echo "  - If target keeps diverging, check node_key and wallet keys are intact"
