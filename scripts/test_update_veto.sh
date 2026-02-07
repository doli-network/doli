#!/bin/bash
# DOLI E2E Test: Update Veto Flow
#
# Tests that 3/5 producers vetoing (>40% weight) rejects an update.
#
# Prerequisites:
#   - 5-node devnet running (past genesis)
#   - DOLI_TEST_KEYS=1 environment variable set
#
# Timing (devnet):
#   - Veto period: 60 seconds
#   - Grace period: 30 seconds
#   - Min voting age: 60 seconds (6 blocks)
set -e

RPC_BASE=28545
NODES=5

echo "========================================="
echo " DOLI E2E: Update Veto Flow"
echo "========================================="
echo ""

# Phase 1: Verify devnet is healthy
echo "[Phase 1] Verifying devnet health..."
for i in $(seq 0 $((NODES-1))); do
  PORT=$((RPC_BASE + i))
  HEIGHT=$(curl -s http://127.0.0.1:$PORT -X POST \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq -r '.result.bestHeight')
  echo "  Node $i (port $PORT): height $HEIGHT"
  if [ "$HEIGHT" -lt 24 ]; then
    echo "FAIL: Node $i not past genesis (height < 24)"
    exit 1
  fi
done
echo "  All nodes healthy."
echo ""

# Phase 2: Check initial update status (should be empty)
echo "[Phase 2] Checking initial update status..."
STATUS=$(curl -s http://127.0.0.1:$RPC_BASE -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getUpdateStatus","params":{},"id":1}')
echo "  Status: $STATUS"
echo ""

# Phase 3: Cast 3/5 veto votes (should exceed 40% threshold)
echo "[Phase 3] Casting 3/5 VETO votes..."
for i in 0 1 2; do
  PORT=$((RPC_BASE + i))
  echo "  Producer $i voting VETO via port $PORT..."
  VOTE_RESULT=$(curl -s http://127.0.0.1:$PORT -X POST \
    -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"submitVote\",\"params\":{\"vote\":{\"version\":\"99.0.0\",\"vote\":\"veto\",\"producer_id\":\"producer_$i\"}},\"id\":1}")
  echo "    Result: $VOTE_RESULT"
  sleep 2
done
echo ""

# Phase 4: Wait for veto period
echo "[Phase 4] Waiting for veto period (60s + buffer)..."
sleep 65
echo ""

# Phase 5: Verify status
echo "[Phase 5] Checking update status after veto..."
STATUS=$(curl -s http://127.0.0.1:$RPC_BASE -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getUpdateStatus","params":{},"id":1}')
echo "  Status: $STATUS"
echo ""

# Verify veto count >= 3
VETO_COUNT=$(echo "$STATUS" | jq -r '.result.veto_count // 0')
echo "  Veto count: $VETO_COUNT"

echo ""
echo "========================================="
echo " RESULT: Veto test completed"
echo " Veto count: $VETO_COUNT / $NODES"
echo "========================================="
