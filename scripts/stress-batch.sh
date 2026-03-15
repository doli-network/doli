#!/usr/bin/env bash
# stress-batch.sh — Launch/stop a batch of 50 nodes
#
# Usage:
#   scripts/stress-batch.sh start <batch>    Start batch 1-10 (50 nodes each)
#   scripts/stress-batch.sh stop <batch>     Stop batch
#   scripts/stress-batch.sh stop-all         Stop all batches
#   scripts/stress-batch.sh status           Show all batch status
#
# Each batch runs 50 doli-node processes. Batches map to:
#   Batch 1:  n13-n62    Batch 6:  n263-n312
#   Batch 2:  n63-n112   Batch 7:  n313-n362
#   Batch 3:  n113-n162  Batch 8:  n363-n412
#   Batch 4:  n163-n212  Batch 9:  n413-n462
#   Batch 5:  n213-n262  Batch 10: n463-n512
#
# Tier assignment:
#   Batch 1-2:   Tier 1 (bootstrap to seed)
#   Batch 3-6:   Tier 2 (bootstrap to random Tier 1 node)
#   Batch 7-10:  Tier 3 (bootstrap to random Tier 2 node)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
NODE_BIN="$PROJECT_ROOT/target/release/doli-node"
MAINNET_DIR="$HOME/mainnet"
KEYS_DIR="$MAINNET_DIR/keys"

# Port bases (seed=30300/8500/9000, N1-N12 use +1 to +12)
# Stress nodes start at +13
BASE_P2P=30300
BASE_RPC=8500
BASE_METRICS=9000

SEED_P2P=30300

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

batch_range() {
  local batch=$1
  local start=$(( (batch - 1) * 50 + 13 ))
  local end=$(( start + 49 ))
  echo "$start $end"
}

bootstrap_for_batch() {
  local batch=$1
  if [[ $batch -le 2 ]]; then
    # Tier 1: bootstrap to SEED (port 30300) — survives producer deaths
    echo "/ip4/127.0.0.1/tcp/${SEED_P2P}"
  elif [[ $batch -le 6 ]]; then
    # Tier 2: bootstrap to a Tier 1 node (from batch 1, pick a random one)
    local t1_node=$(( RANDOM % 50 + 13 ))
    local t1_p2p=$(( BASE_P2P + t1_node ))
    echo "/ip4/127.0.0.1/tcp/${t1_p2p}"
  else
    # Tier 3: bootstrap to a Tier 2 node (from batch 3-4)
    local t2_node=$(( RANDOM % 100 + 113 ))
    local t2_p2p=$(( BASE_P2P + t2_node ))
    echo "/ip4/127.0.0.1/tcp/${t2_p2p}"
  fi
}

do_start() {
  local batch=$1
  local range=($(batch_range $batch))
  local start=${range[0]} end=${range[1]}
  local batch_dir="$MAINNET_DIR/nodes${batch}"
  local pid_file="$batch_dir/pids"
  local log_dir="$MAINNET_DIR/logs/nodes${batch}"
  local bootstrap=$(bootstrap_for_batch $batch)

  mkdir -p "$log_dir"

  if [[ -f "$pid_file" ]]; then
    local running=0
    while read pid; do
      kill -0 "$pid" 2>/dev/null && running=$((running + 1))
    done < "$pid_file"
    if [[ $running -gt 0 ]]; then
      echo -e "${YELLOW}Batch $batch already has $running running nodes. Stop first.${NC}"
      return 1
    fi
  fi

  local tier_label="Tier 1"
  [[ $batch -gt 2 && $batch -le 6 ]] && tier_label="Tier 2"
  [[ $batch -gt 6 ]] && tier_label="Tier 3"

  echo -e "${CYAN}Starting batch $batch: n${start}-n${end} ($tier_label, bootstrap=${bootstrap})${NC}"

  > "$pid_file"  # truncate
  local launched=0

  for ((i=start; i<=end; i++)); do
    local p2p=$((BASE_P2P + i))
    local rpc=$((BASE_RPC + i))
    local metrics=$((BASE_METRICS + i))
    local data_dir="$batch_dir/n${i}/data"
    local key_file="$KEYS_DIR/producer_${i}.json"

    mkdir -p "$data_dir"

    # Launch with 1MB rotating log (keeps .log and .log.1)
    local node_log="$log_dir/n${i}.log"
    (
      $NODE_BIN \
        --network mainnet \
        --data-dir "$data_dir" \
        run \
        --producer \
        --producer-key "$key_file" \
        --p2p-port "$p2p" \
        --rpc-port "$rpc" \
        --metrics-port "$metrics" \
        --bootstrap "$bootstrap" \
        --yes \
        --force-start 2>&1 |
      while IFS= read -r line; do
        echo "$line" >> "$node_log"
        # Rotate at 1MB
        if [[ -f "$node_log" ]] && (( $(stat -f%z "$node_log" 2>/dev/null || echo 0) > 1048576 )); then
          mv "$node_log" "${node_log}.1"
        fi
      done
    ) &

    echo $! >> "$pid_file"
    launched=$((launched + 1))
  done

  echo -e "  ${GREEN}Launched $launched nodes${NC} (PIDs in $pid_file)"
  echo -e "  Logs: $log_dir/n{${start}-${end}}.log"
}

do_stop() {
  local batch=$1
  local pid_file="$MAINNET_DIR/nodes${batch}/pids"

  if [[ ! -f "$pid_file" ]]; then
    echo "Batch $batch: no pid file"
    return
  fi

  local killed=0
  while read pid; do
    if kill "$pid" 2>/dev/null; then
      killed=$((killed + 1))
    fi
  done < "$pid_file"

  # Wait briefly then force-kill stragglers
  sleep 2
  while read pid; do
    kill -9 "$pid" 2>/dev/null || true
  done < "$pid_file"

  rm -f "$pid_file"
  echo -e "  Batch $batch: ${GREEN}stopped $killed nodes${NC}"
}

do_stop_all() {
  echo "Stopping all stress batches..."
  for batch in $(seq 1 10); do
    do_stop $batch
  done
}

do_status() {
  printf "%-8s %-10s %-8s %-8s %-10s %s\n" "Batch" "Nodes" "Running" "Dead" "Tier" "Bootstrap"
  printf "%-8s %-10s %-8s %-8s %-10s %s\n" "--------" "----------" "--------" "--------" "----------" "---------"

  local total_running=0
  for batch in $(seq 1 10); do
    local range=($(batch_range $batch))
    local start=${range[0]} end=${range[1]}
    local pid_file="$MAINNET_DIR/nodes${batch}/pids"
    local running=0 dead=0

    if [[ -f "$pid_file" ]]; then
      while read pid; do
        if kill -0 "$pid" 2>/dev/null; then
          running=$((running + 1))
        else
          dead=$((dead + 1))
        fi
      done < "$pid_file"
    fi

    local tier="Tier 1"
    [[ $batch -gt 2 && $batch -le 6 ]] && tier="Tier 2"
    [[ $batch -gt 6 ]] && tier="Tier 3"

    local status_color="$RED"
    [[ $running -gt 0 ]] && status_color="$GREEN"
    [[ $dead -gt 0 && $running -gt 0 ]] && status_color="$YELLOW"

    printf "%-8s %-10s " "$batch" "n${start}-n${end}"
    echo -en "${status_color}"
    printf "%-8s" "$running"
    echo -en "${NC}"
    printf " %-8s %-10s %s\n" "$dead" "$tier" "$(bootstrap_for_batch $batch)"

    total_running=$((total_running + running))
  done

  echo ""
  echo "Total stress nodes running: $total_running / 500"
  echo "Plus genesis nodes (seed + N1-N12): check scripts/mainnet.sh status"
}

ACTION="${1:-status}"
BATCH="${2:-}"

case "$ACTION" in
  start)
    [[ -z "$BATCH" ]] && echo "Usage: $0 start <1-10|all>" && exit 1
    if [[ "$BATCH" == "all" ]]; then
      for b in $(seq 1 10); do do_start $b; sleep 2; done
    else
      do_start "$BATCH"
    fi
    ;;
  stop)
    [[ -z "$BATCH" ]] && echo "Usage: $0 stop <1-10|all>" && exit 1
    if [[ "$BATCH" == "all" ]]; then
      do_stop_all
    else
      do_stop "$BATCH"
    fi
    ;;
  stop-all) do_stop_all ;;
  status) do_status ;;
  *)
    echo "Usage: $0 {start|stop|stop-all|status} [batch 1-10|all]"
    exit 1
    ;;
esac
