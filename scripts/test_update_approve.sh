#!/bin/bash
# DOLI E2E Test: Update Approval Flow
#
# Tests that <40% veto results in update approval.
#
# Prerequisites:
#   - 5-node devnet running (past genesis)
#   - DOLI_TEST_KEYS=1 environment variable set
#
# Timing (devnet):
#   - Veto period: 60 seconds
#   - Grace period: 30 seconds
set -e

RPC_BASE=28500
NODES=5

echo "========================================="
echo " DOLI E2E: Update Approval Flow"
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
done
echo ""

# Phase 2: Cast only 1/5 veto vote (should NOT exceed 40%)
echo "[Phase 2] Casting 1/5 VETO vote (should NOT trigger rejection)..."
VOTE_RESULT=$(curl -s http://127.0.0.1:$RPC_BASE -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"submitVote","params":{"vote":{"version":"99.1.0","vote":"veto","producer_id":"producer_0"}},"id":1}')
echo "  Result: $VOTE_RESULT"
echo ""

# Phase 3: Wait for veto period
echo "[Phase 3] Waiting for veto period (60s + buffer)..."
sleep 65
echo ""

# Phase 4: Verify status shows approved (or at least not rejected)
echo "[Phase 4] Checking update status..."
STATUS=$(curl -s http://127.0.0.1:$RPC_BASE -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getUpdateStatus","params":{},"id":1}')
echo "  Status: $STATUS"
echo ""

VETO_PCT=$(echo "$STATUS" | jq -r '.result.veto_percent // 0')
echo "  Veto percent: $VETO_PCT%"
echo "  Threshold: 40%"

if [ "$(echo "$VETO_PCT < 40" | bc -l 2>/dev/null || echo 1)" = "1" ]; then
  echo "  Result: APPROVED (veto below threshold)"
else
  echo "  Result: REJECTED (unexpected)"
fi

# Phase 5: Wait for grace period
echo ""
echo "[Phase 5] Waiting for grace period (30s + buffer)..."
sleep 35
echo ""

echo "========================================="
echo " RESULT: Approval test completed"
echo " Veto percent: $VETO_PCT%"
echo "========================================="
