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
#   # Single target (testnet preset):
#   scripts/node-heal.sh --testnet --target n5 --source n3 --yes
#
#   # Batch (testnet preset, sequential — recommended default):
#   scripts/node-heal.sh --testnet --source n13 --batch n14-n32 --yes
#
#   # Batch parallel (testnet only; refused on generic mode unless you
#   # pass --i-know-what-im-doing). See safety notes below.
#   scripts/node-heal.sh --testnet --source n13 --batch n14-n32 --parallel 4 --yes
#
#   # Mixed batch spec (ranges + singletons, comma-separated):
#   scripts/node-heal.sh --testnet --source n13 --batch n14-n16,n20,n25-n27 --yes
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
#   --testnet                    Testnet preset mode (requires --source + one of --target/--batch)
#   --target NAME                Single target node name (nNN) — preset mode only
#   --source NAME                Source node name — preset mode only
#   --batch SPEC                 Multiple targets: range (n14-n32), list (n14,n15,n20),
#                                or mixed (n14-n16,n20,n25-n27). Preset mode only (v1).
#                                Cannot combine with --target.
#   --parallel N                 Max concurrent heals in batch mode (default 1 = sequential).
#                                Parallel > 1 is testnet-preset-only unless --i-know-what-im-doing.
#                                See safety notes below before going parallel on mainnet.
#   --i-know-what-im-doing       Escape hatch: allow --parallel N>1 in generic (mainnet) mode.
#                                Only use if you have verified the source disk is on SSD and
#                                you understand the start-storm risk on your fleet's seed.
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
# Safety notes (see .claude/skills/guardian/reference/node-heal.md and hostile-recovery.md L1/L1.1/L6):
#   - signed_slots.db is PRESERVED by default — it's slashing protection, not chain
#     state, and losing it re-exposes the producer to double-sign slashing.
#   - utxo_store/ is NOT copied from the source — the node rebuilds it from
#     state_db on startup (self-heals since v6.7.9, per INC-I-027). Copying
#     utxo_store across binaries risks a silent-corruption rollback cascade.
#   - Parallel batch mode multiplies three failure modes: (1) source disk I/O
#     saturation during concurrent rsync, (2) chain liveness loss from N
#     producers down simultaneously, (3) start-storm on the seed's gossip mesh
#     (INC-I-014 territory). Default is --parallel 1 for a reason.
#
# Exit codes: 0 = all heals succeeded, 1 = one or more aborted/errored, 2 = bad args
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
BATCH_SPEC=""
PARALLEL=1
OVERRIDE=false
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
    --batch)                 BATCH_SPEC="$2"; shift 2 ;;
    --parallel)              PARALLEL="$2"; shift 2 ;;
    --i-know-what-im-doing)  OVERRIDE=true; shift ;;
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

# ---- validate flag combinations ----
if [[ -n "$TARGET_NAME" && -n "$BATCH_SPEC" ]]; then
  echo "ERROR: --target and --batch are mutually exclusive"
  exit 2
fi

if ! [[ "$PARALLEL" =~ ^[0-9]+$ ]] || [[ "$PARALLEL" -lt 1 ]]; then
  echo "ERROR: --parallel must be a positive integer (got: $PARALLEL)"
  exit 2
fi

if [[ "$PARALLEL" -gt 1 && -z "$BATCH_SPEC" ]]; then
  echo "ERROR: --parallel > 1 only makes sense with --batch"
  exit 2
fi

if [[ -n "$BATCH_SPEC" && "$PRESET" != "testnet" ]]; then
  echo "ERROR: --batch currently requires --testnet (generic batch not implemented)"
  exit 2
fi

if [[ "$PARALLEL" -gt 1 && "$PRESET" != "testnet" && "$OVERRIDE" != true ]]; then
  echo "ERROR: --parallel > 1 on non-testnet mode requires --i-know-what-im-doing"
  echo "       (parallel heals on mainnet risk liveness loss and source I/O saturation)"
  exit 2
fi

# ---- preset: resolve SOURCE only ----
# (Targets are resolved per-iteration in heal_target so batch mode works cleanly.)
if [[ "$PRESET" == "testnet" ]]; then
  [[ -z "$SOURCE_NAME" ]] && { echo "--testnet requires --source"; exit 2; }
  [[ "$SOURCE_NAME" == "seed" ]] && { echo "refusing to use seed as source — producers only"; exit 2; }
  if [[ ! "$SOURCE_NAME" =~ ^n([0-9]+)$ ]]; then
    echo "source must match nN pattern (got: $SOURCE_NAME)"; exit 2
  fi
  SOURCE_IDX="${BASH_REMATCH[1]}"
  SOURCE_DATA="$HOME/testnet/$SOURCE_NAME/data"
  SOURCE_RPC="127.0.0.1:$((8500 + SOURCE_IDX))"

  # Single-target preset mode still needs a target
  if [[ -z "$BATCH_SPEC" ]]; then
    [[ -z "$TARGET_NAME" ]] && { echo "--testnet requires --target OR --batch"; exit 2; }
    [[ "$TARGET_NAME" == "seed" ]] && { echo "refusing to heal seed — producers only"; exit 2; }
    [[ "$TARGET_NAME" == "$SOURCE_NAME" ]] && { echo "target and source must differ"; exit 2; }
    if [[ ! "$TARGET_NAME" =~ ^n([0-9]+)$ ]]; then
      echo "target must match nN pattern (got: $TARGET_NAME)"; exit 2
    fi
  fi
fi

# ---- generic mode validation (single target) ----
if [[ "$PRESET" != "testnet" ]]; then
  for var in TARGET_DATA SOURCE_DATA STOP_CMD START_CMD; do
    if [[ -z "${!var}" ]]; then
      flag=$(echo "$var" | tr '[:upper:]_' '[:lower:]-')
      echo "Missing required: --$flag"
      exit 2
    fi
  done
fi

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

# Expand a batch spec like "n14-n32" or "n14,n15,n20" or "n14-n16,n20,n25-n27"
# into a newline-separated list of node names.
expand_batch() {
  local spec="$1"
  local -a out=()
  local part start end i
  IFS=',' read -ra parts <<< "$spec"
  for part in "${parts[@]}"; do
    part="${part// /}"  # strip whitespace
    [[ -z "$part" ]] && continue
    if [[ "$part" =~ ^n([0-9]+)-n([0-9]+)$ ]]; then
      start="${BASH_REMATCH[1]}"
      end="${BASH_REMATCH[2]}"
      if [[ "$start" -gt "$end" ]]; then
        echo "bad range (start>end): $part" >&2
        return 1
      fi
      for i in $(seq "$start" "$end"); do
        out+=("n$i")
      done
    elif [[ "$part" =~ ^n[0-9]+$ ]]; then
      out+=("$part")
    else
      echo "bad batch spec part: $part" >&2
      return 1
    fi
  done
  if [[ ${#out[@]} -eq 0 ]]; then
    echo "empty batch spec" >&2
    return 1
  fi
  printf "%s\n" "${out[@]}"
}

# ---- pre-check: source health (ONCE, regardless of single or batch) ----
echo -e "${BOLD}=== NODE HEAL ===${NC}"
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

# ---- resolve batch list ----
declare -a TARGETS=()
if [[ -n "$BATCH_SPEC" ]]; then
  # Portable: bash 3.2 on macOS has no mapfile/readarray.
  batch_out=$(expand_batch "$BATCH_SPEC") || exit 2
  while IFS= read -r line; do
    [[ -n "$line" ]] && TARGETS+=("$line")
  done <<< "$batch_out"
  # Refuse source in target list
  for t in "${TARGETS[@]}"; do
    if [[ "$t" == "$SOURCE_NAME" ]]; then
      echo -e "${RED}ERROR: source ($SOURCE_NAME) is in the target list — would wipe the source${NC}"
      exit 1
    fi
    if [[ "$t" == "seed" ]]; then
      echo -e "${RED}ERROR: seed in target list — producers only${NC}"
      exit 1
    fi
  done
else
  TARGETS=("$TARGET_NAME")
fi

# ---- rsync excludes (shared across all heals) ----
EXCLUDES=(
  --exclude='signed_slots.db'
  --exclude='signed_slots.db/'
  --exclude='utxo_store'
  --exclude='utxo_store/'
  --exclude='producer.lock'
  --exclude='pending_update.json'
)

# --progress is portable across GNU rsync and macOS openrsync.
# Older --info=progress2 is GNU-rsync only and breaks on stock macOS.
if is_remote_source "$SOURCE_DATA"; then
  RSYNC_OPTS=(-az --progress -e ssh)
else
  RSYNC_OPTS=(-a --progress)
fi

# ---- per-target heal function ----
# Usage: heal_target <name> [<prefix>]
#   name   = node name (e.g. n14), or "single" for generic mode
#   prefix = optional log prefix for batch mode (e.g. "[n14] ")
# Returns 0 on success, non-zero on failure. Each worker prints its own step labels.
heal_target() {
  local name="$1"
  local prefix="${2:-}"

  # Per-target variables: local to this function so parallel workers don't clobber each other.
  local target_data target_rpc stop_cmd start_cmd
  if [[ "$PRESET" == "testnet" ]]; then
    if [[ ! "$name" =~ ^n([0-9]+)$ ]]; then
      echo -e "${prefix}${RED}bad target name in heal_target: $name${NC}"
      return 1
    fi
    local idx="${BASH_REMATCH[1]}"
    target_data="$HOME/testnet/$name/data"
    target_rpc="127.0.0.1:$((8500 + idx))"
    stop_cmd="$SCRIPT_DIR/testnet.sh stop $name"
    start_cmd="$SCRIPT_DIR/testnet.sh start $name"
  else
    target_data="$TARGET_DATA"
    target_rpc="$TARGET_RPC"
    stop_cmd="$STOP_CMD"
    start_cmd="$START_CMD"
  fi

  # Sanity: source and target can't resolve to the same path
  if ! is_remote_source "$SOURCE_DATA"; then
    local src_real tgt_real
    src_real=$(realpath "$SOURCE_DATA" 2>/dev/null || echo "$SOURCE_DATA")
    tgt_real=$(realpath "$target_data" 2>/dev/null || echo "$target_data")
    if [[ "$src_real" == "$tgt_real" ]]; then
      echo -e "${prefix}${RED}ERROR: source and target resolve to the same path${NC}"
      return 1
    fi
  fi

  # Step 1/5: stop
  echo -e "${prefix}${BOLD}Step 1/5:${NC} Stop target node"
  run "$stop_cmd"
  if [[ -n "$target_rpc" && "$DRY_RUN" != true ]]; then
    echo -n "${prefix}  Waiting for target to release RocksDB locks"
    for _ in $(seq 1 20); do
      if ! rpc_call "$target_rpc" "getChainInfo" >/dev/null 2>&1; then
        echo " — stopped"
        break
      fi
      echo -n "."
      sleep 1
    done
    sleep 2
  fi

  # Step 2/5: wipe (preserving signed_slots.db unless --wipe-signed-slots)
  echo -e "${prefix}${BOLD}Step 2/5:${NC} Wipe target data"
  if [[ "$DRY_RUN" == true ]]; then
    if [[ "$WIPE_SIGNED_SLOTS" == true ]]; then
      echo -e "${prefix}  ${YELLOW}[dry-run]${NC} find $target_data -mindepth 1 -delete"
    else
      echo -e "${prefix}  ${YELLOW}[dry-run]${NC} find $target_data -mindepth 1 ! -name signed_slots.db ! -path '*/signed_slots.db/*' -delete"
    fi
  else
    mkdir -p "$target_data"
    if [[ "$WIPE_SIGNED_SLOTS" == true ]]; then
      echo -e "${prefix}  ${RED}WIPING signed_slots.db${NC} (operator override)"
      find "$target_data" -mindepth 1 -delete
    else
      find "$target_data" -mindepth 1 \
        ! -name signed_slots.db \
        ! -path "$target_data/signed_slots.db/*" \
        -delete 2>/dev/null || true
    fi
    echo -e "${prefix}  ${GREEN}OK${NC} — $target_data wiped"
  fi

  # Step 3/5: rsync
  echo -e "${prefix}${BOLD}Step 3/5:${NC} Rsync from source"
  local rsync_src="${SOURCE_DATA%/}/"
  if [[ "$DRY_RUN" == true ]]; then
    echo -e "${prefix}  ${YELLOW}[dry-run]${NC} rsync ${RSYNC_OPTS[*]} ${EXCLUDES[*]} $rsync_src ${target_data%/}/"
  else
    if ! rsync "${RSYNC_OPTS[@]}" "${EXCLUDES[@]}" "$rsync_src" "${target_data%/}/" >/dev/null 2>&1; then
      echo -e "${prefix}  ${RED}RSYNC FAILED${NC}"
      return 1
    fi
    echo -e "${prefix}  ${GREEN}OK${NC} — rsync complete"
  fi

  # Step 4/5: verify preserved files
  echo -e "${prefix}${BOLD}Step 4/5:${NC} Verify preserved files"
  if [[ "$DRY_RUN" != true ]]; then
    if [[ "$WIPE_SIGNED_SLOTS" != true ]]; then
      if [[ -d "$target_data/signed_slots.db" ]]; then
        echo -e "${prefix}  ${GREEN}OK${NC} signed_slots.db preserved"
      else
        echo -e "${prefix}  ${YELLOW}NOTE${NC} no signed_slots.db on target (fresh producer)"
      fi
    fi
    if [[ -d "$target_data/utxo_store" ]]; then
      echo -e "${prefix}  ${YELLOW}WARN${NC} utxo_store/ present — should have been excluded"
    else
      echo -e "${prefix}  ${GREEN}OK${NC} utxo_store/ absent — node will self-heal from state_db"
    fi
  fi

  # Step 5/5: start
  echo -e "${prefix}${BOLD}Step 5/5:${NC} Start target node"
  run "$start_cmd"

  # Post-check
  if [[ -n "$target_rpc" && "$DRY_RUN" != true ]]; then
    echo -n "${prefix}  Waiting for target RPC to come back"
    local tgt_info new_h new_hash
    for _ in $(seq 1 30); do
      if tgt_info=$(rpc_call "$target_rpc" "getChainInfo"); then
        new_h=$(echo "$tgt_info" | json_field bestHeight)
        new_hash=$(echo "$tgt_info" | json_field bestHash)
        if [[ -n "$new_h" ]]; then
          echo " — up"
          echo -e "${prefix}  ${GREEN}Online${NC}: height=$new_h hash=$(short_hash "$new_hash")"
          return 0
        fi
      fi
      echo -n "."
      sleep 1
    done
    echo
    echo -e "${prefix}  ${RED}RPC did not come back within 30s${NC}"
    return 1
  fi

  return 0
}

# ---- confirmation ----
echo ""
if [[ ${#TARGETS[@]} -eq 1 ]]; then
  # Single target — show current state vs source state
  TARGET_PRE_H="?"
  TARGET_PRE_HASH="?"
  if [[ "$PRESET" == "testnet" ]]; then
    tgt_rpc="127.0.0.1:$((8500 + ${TARGETS[0]#n}))"
    if tgt_info=$(rpc_call "$tgt_rpc" "getChainInfo"); then
      TARGET_PRE_H=$(echo "$tgt_info" | json_field bestHeight)
      TARGET_PRE_HASH=$(echo "$tgt_info" | json_field bestHash)
    fi
  elif [[ -n "$TARGET_RPC" ]]; then
    if tgt_info=$(rpc_call "$TARGET_RPC" "getChainInfo"); then
      TARGET_PRE_H=$(echo "$tgt_info" | json_field bestHeight)
      TARGET_PRE_HASH=$(echo "$tgt_info" | json_field bestHash)
    fi
  fi
  echo -e "${BOLD}About to DESTROY target data.${NC}"
  echo "  Target              : ${TARGETS[0]}"
  echo "  Current target state: height=${TARGET_PRE_H} hash=$(short_hash "${TARGET_PRE_HASH:-?}")"
  echo "  Will be replaced w/ : height=${SOURCE_HEIGHT} hash=$(short_hash "${SOURCE_HASH:-?}")"
else
  # Batch mode
  MODE_LABEL="sequential"
  [[ "$PARALLEL" -gt 1 ]] && MODE_LABEL="parallel (max=$PARALLEL)"
  echo -e "${BOLD}About to DESTROY target data on ${#TARGETS[@]} nodes.${NC}"
  echo "  Source   : ${SOURCE_NAME:-<generic>} (height=${SOURCE_HEIGHT} hash=$(short_hash "${SOURCE_HASH:-?}"))"
  echo "  Targets  : ${TARGETS[*]}"
  echo "  Mode     : $MODE_LABEL"
  if [[ "$PARALLEL" -gt 1 ]]; then
    echo -e "  ${YELLOW}WARNING${NC}: parallel mode may cause start-storm on the seed and"
    echo -e "           ${YELLOW}       ${NC} temporarily remove ${#TARGETS[@]} producers from the schedule."
  fi
fi
echo ""

if [[ "$YES" != true ]]; then
  read -rp "Proceed with heal? Type 'yes' to continue: " confirm
  [[ "$confirm" != "yes" ]] && { echo "Aborted."; exit 1; }
fi

# ---- execute ----
declare -a SUCCEEDED=()
declare -a FAILED=()

if [[ "$PARALLEL" -le 1 ]]; then
  # Sequential — stop on first failure
  for t in "${TARGETS[@]}"; do
    echo ""
    if [[ ${#TARGETS[@]} -gt 1 ]]; then
      echo -e "${BOLD}========== $t ==========${NC}"
    fi
    if heal_target "$t"; then
      SUCCEEDED+=("$t")
    else
      FAILED+=("$t")
      if [[ ${#TARGETS[@]} -gt 1 ]]; then
        echo -e "${RED}!!! heal failed for $t — stopping batch${NC}"
        # Compute unprocessed tail
        local_remaining=()
        started=false
        for u in "${TARGETS[@]}"; do
          if $started; then
            local_remaining+=("$u")
          elif [[ "$u" == "$t" ]]; then
            started=true
          fi
        done
      fi
      break
    fi
  done
else
  # Parallel — cap concurrency, let all launched workers finish
  declare -a PIDS=()
  declare -a PID_NAMES=()
  STOP_LAUNCHING=false

  for t in "${TARGETS[@]}"; do
    if $STOP_LAUNCHING; then
      FAILED+=("$t (skipped)")
      continue
    fi
    # Wait for a free slot
    while [[ $(jobs -rp | wc -l | tr -d ' ') -ge $PARALLEL ]]; do
      sleep 0.2
    done
    echo ""
    echo -e "${BOLD}========== launching $t ==========${NC}"
    heal_target "$t" "[$t] " &
    PIDS+=($!)
    PID_NAMES+=("$t")
  done

  # Wait for all launched workers
  for i in "${!PIDS[@]}"; do
    if wait "${PIDS[$i]}"; then
      SUCCEEDED+=("${PID_NAMES[$i]}")
    else
      FAILED+=("${PID_NAMES[$i]}")
    fi
  done
fi

# ---- summary ----
echo ""
echo -e "${BOLD}=== SUMMARY ===${NC}"
echo -e "  ${GREEN}Succeeded (${#SUCCEEDED[@]})${NC}: ${SUCCEEDED[*]:-<none>}"
if [[ ${#FAILED[@]} -gt 0 ]]; then
  echo -e "  ${RED}Failed (${#FAILED[@]})${NC}: ${FAILED[*]}"
fi
echo ""

if [[ ${#FAILED[@]} -gt 0 ]]; then
  echo -e "${RED}${BOLD}Heal completed with errors.${NC}"
  exit 1
fi

echo -e "${GREEN}${BOLD}Heal complete.${NC}"
echo ""
echo "Next steps:"
echo "  - Monitor via: scripts/fork-monitor.sh${PRESET:+ --testnet}"
echo "  - If any target keeps diverging, check node_key and wallet keys are intact"
echo "  - Note: the INC-I-027 self-heal log line ([UTXO] ... rebuilding from state_db)"
echo "    is emitted at INFO level; with RUST_LOG=warn it won't appear. The real"
echo "    verification is fleet convergence via fork-monitor.sh."
