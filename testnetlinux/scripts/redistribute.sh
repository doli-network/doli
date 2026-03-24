#!/usr/bin/env bash
# redistribute.sh — Redistribute excess DOLI from n7-n30 to unfunded nodes and n1-n6
# n7-n30 should keep only 1.01 DOLI, excess goes to:
#   1. Fund n31-n100 with 1.01 DOLI each
#   2. Return remainder to n1-n6
set -euo pipefail

CLI="/home/ilozada/repos/localdoli/testnetlinux/bin/doli"
KEYS_DIR="/home/ilozada/repos/localdoli/testnetlinux/keys"
SEED_RPC="http://127.0.0.1:8500"
BATCH_SIZE=5
BATCH_DELAY=11

get_rpc() { echo "http://127.0.0.1:$((8500 + $1))"; }

get_address() {
  local n=$1
  "$CLI" -w "$KEYS_DIR/producer_${n}.json" -r "$SEED_RPC" -n testnet addresses 2>/dev/null | grep -oP '(t?doli1)\S+' | head -1
}

get_spendable() {
  local n=$1
  "$CLI" -w "$KEYS_DIR/producer_${n}.json" -r "$SEED_RPC" -n testnet balance 2>/dev/null | grep "Spendable:" | head -1 | sed 's/.*Spendable:[[:space:]]*//' | sed 's/ .*//'
}

send_doli() {
  local from=$1 to_addr=$2 amount=$3
  local from_rpc=$(get_rpc "$from")
  local result=$("$CLI" -w "$KEYS_DIR/producer_${from}.json" -r "$from_rpc" -n testnet send "$to_addr" "$amount" 2>&1)
  if echo "$result" | grep -q "submitted successfully"; then
    local tx=$(echo "$result" | grep "TX Hash:" | awk '{print $3}')
    echo "  n$from → $amount DOLI ✓ (tx: ${tx:0:16}…)"
    return 0
  else
    echo "  n$from → FAILED: $(echo "$result" | tail -1)"
    return 1
  fi
}

# Phase 1: Fund n31-n100 with 1.01 DOLI from n7-n30
echo "=== Phase 1: Fund unfunded nodes (n31-n100) ==="
UNFUNDED=()
for n in $(seq 31 100); do
  bal=$(get_spendable "$n" || echo "0.00000000")
  bal_int=$(echo "$bal" | awk '{printf "%.0f", $1 * 100000000}')
  if [[ "$bal_int" -lt 101000000 ]] 2>/dev/null; then
    addr=$(get_address "$n" || echo "")
    if [[ -n "$addr" ]]; then
      UNFUNDED+=("$n:$addr")
    fi
  fi
done
echo "  Found ${#UNFUNDED[@]} unfunded nodes"

# Round-robin through n7-n30 as senders
SENDERS=($(seq 7 30))
si=0
batch=()

for entry in "${UNFUNDED[@]}"; do
  target_n="${entry%%:*}"
  target_addr="${entry#*:}"
  sender=${SENDERS[$si]}
  si=$(( (si + 1) % ${#SENDERS[@]} ))

  batch+=("$sender:$target_n:$target_addr")

  if [[ ${#batch[@]} -ge $BATCH_SIZE ]]; then
    echo "--- Batch: funding ${#batch[@]} nodes ---"
    pids=()
    for b in "${batch[@]}"; do
      s="${b%%:*}"; rest="${b#*:}"; tn="${rest%%:*}"; ta="${rest#*:}"
      ( send_doli "$s" "$ta" "1.01" && echo "    n$tn funded from n$s" ) &
      pids+=($!)
    done
    for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
    batch=()
    echo "  Waiting ${BATCH_DELAY}s..."
    sleep "$BATCH_DELAY"
  fi
done

# Flush remaining batch
if [[ ${#batch[@]} -gt 0 ]]; then
  echo "--- Batch: funding ${#batch[@]} nodes ---"
  pids=()
  for b in "${batch[@]}"; do
    s="${b%%:*}"; rest="${b#*:}"; tn="${rest%%:*}"; ta="${rest#*:}"
    ( send_doli "$s" "$ta" "1.01" && echo "    n$tn funded from n$s" ) &
    pids+=($!)
  done
  for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
fi

echo ""
echo "=== Phase 2: Return excess from n7-n30 to n1-n6 ==="
sleep "$BATCH_DELAY"

RETURN_TO=(1 2 3 4 5 6)
ri=0

for sender in $(seq 7 30); do
  spendable=$(get_spendable "$sender" || echo "0.00000000")
  # Keep 1.02 (1.01 + buffer for fees), send the rest
  excess=$(echo "$spendable" | awk '{x = $1 - 1.02; if (x > 0.01) print x; else print 0}')
  if [[ "$excess" == "0" ]]; then
    echo "  n$sender: nothing to return (spendable=$spendable)"
    continue
  fi

  target=${RETURN_TO[$ri]}
  ri=$(( (ri + 1) % ${#RETURN_TO[@]} ))
  target_addr=$(get_address "$target")

  echo -n "  n$sender → n$target: $excess DOLI "
  send_doli "$sender" "$target_addr" "$excess"
  sleep 2
done

echo ""
echo "=== Redistribution complete ==="
