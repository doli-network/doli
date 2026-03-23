#!/usr/bin/env bash
# fund-producers.sh — Send 1.01 DOLI from n1-n5 to n6-n150
# 5 operations at a time, every 11 seconds
# Skips nodes that already have >= 1.01 DOLI or have no valid wallet
set -euo pipefail

CLI="/home/ilozada/repos/localdoli/testnetlinux/bin/doli"
KEYS_DIR="/home/ilozada/repos/localdoli/testnetlinux/keys"
SEED_RPC="http://127.0.0.1:8500"
AMOUNT="${1:-1.01}"
BATCH_SIZE=5
BATCH_DELAY=11

# Sender nodes (round-robin through n1-n5)
SENDERS=(1 2 3 4 5)
sender_idx=0

get_rpc() { echo "http://127.0.0.1:$((8500 + $1))"; }

get_address() {
  local n=$1
  local wallet="$KEYS_DIR/producer_${n}.json"
  [[ ! -f "$wallet" ]] && return 1
  "$CLI" -w "$wallet" -r "$SEED_RPC" -n testnet addresses 2>/dev/null | grep -oP '(t?doli1)\S+' | head -1
}

get_balance() {
  local n=$1
  local wallet="$KEYS_DIR/producer_${n}.json"
  "$CLI" -w "$wallet" -r "$SEED_RPC" -n testnet balance 2>/dev/null | grep "Total:" | tail -1 | sed 's/.*Total:[[:space:]]*//' | sed 's/ .*//'
}

# Build list of nodes that need funding
START_NODE="${2:-6}"
END_NODE="${3:-150}"
echo "=== Scanning n${START_NODE}-n${END_NODE} for nodes needing funding (${AMOUNT} DOLI) ==="
TARGETS=()
for n in $(seq "$START_NODE" "$END_NODE"); do
  wallet="$KEYS_DIR/producer_${n}.json"
  if [[ ! -f "$wallet" ]]; then
    echo "  n$n: SKIP (no wallet file)"
    continue
  fi

  addr=$(get_address "$n" || echo "")
  if [[ -z "$addr" ]] || [[ "$addr" != doli1* && "$addr" != tdoli1* ]]; then
    echo "  n$n: SKIP (invalid address)"
    continue
  fi

  bal=$(get_balance "$n" || echo "0.00000000")
  # Compare as integers (multiply by 100000000 to avoid float issues)
  bal_int=$(echo "$bal" | awk '{printf "%.0f", $1 * 100000000}')
  threshold=$(echo "$AMOUNT" | awk '{printf "%.0f", $1 * 100000000}')
  if [[ "$bal_int" -ge "$threshold" ]] 2>/dev/null; then
    echo "  n$n: SKIP (balance: $bal DOLI)"
    continue
  fi

  TARGETS+=("$n:$addr")
done

total=${#TARGETS[@]}
echo ""
echo "=== Found $total nodes to fund ==="
echo ""

if [[ $total -eq 0 ]]; then
  echo "All nodes already funded!"
  exit 0
fi

# Send in batches of 5
batch_num=0
for ((i=0; i<total; i+=BATCH_SIZE)); do
  batch_num=$((batch_num + 1))
  batch_end=$((i + BATCH_SIZE))
  [[ $batch_end -gt $total ]] && batch_end=$total

  echo "--- Batch $batch_num: nodes $((i+1))-$batch_end of $total ---"

  pids=()
  for ((j=i; j<batch_end; j++)); do
    entry="${TARGETS[$j]}"
    n="${entry%%:*}"
    addr="${entry#*:}"

    # Round-robin sender
    sender=${SENDERS[$sender_idx]}
    sender_idx=$(( (sender_idx + 1) % ${#SENDERS[@]} ))
    sender_rpc=$(get_rpc "$sender")
    sender_wallet="$KEYS_DIR/producer_${sender}.json"

    (
      result=$("$CLI" -w "$sender_wallet" -r "$sender_rpc" send "$addr" "$AMOUNT" 2>&1)
      if echo "$result" | grep -q "submitted successfully"; then
        tx=$(echo "$result" | grep "TX Hash:" | awk '{print $3}')
        echo "  n$n ← n$sender: $AMOUNT DOLI ✓ (tx: ${tx:0:16}…)"
      else
        err=$(echo "$result" | tail -1)
        echo "  n$n ← n$sender: FAILED ($err)"
      fi
    ) &
    pids+=($!)
  done

  # Wait for batch to complete
  for pid in "${pids[@]}"; do
    wait "$pid" 2>/dev/null || true
  done

  # Delay between batches (except last)
  if [[ $batch_end -lt $total ]]; then
    echo "  Waiting ${BATCH_DELAY}s for next batch..."
    sleep "$BATCH_DELAY"
  fi
done

echo ""
echo "=== Funding complete ==="
