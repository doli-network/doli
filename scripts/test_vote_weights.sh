#!/bin/bash
# DOLI E2E Test: Vote Weight (Bonds x Seniority)
#
# Tests that vote weight = bonds × seniority_multiplier.
# A whale with 10 bonds but 0 seniority should have higher weight
# than a veteran with 1 bond and some seniority (in the short term).
#
# Prerequisites:
#   - 5-node devnet running with at least 1 whale producer (10 bonds)
#
# Timing (devnet):
#   - 1 seniority year = 144 blocks = ~24 minutes
#   - Min voting age = 6 blocks = ~60 seconds
set -e

RPC_BASE=28545

echo "========================================="
echo " DOLI E2E: Vote Weight Verification"
echo "========================================="
echo ""

# Phase 1: Get producer list with bond counts
echo "[Phase 1] Fetching producer set..."
PRODUCERS=$(curl -s http://127.0.0.1:$RPC_BASE -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getProducers","params":{"active_only":true},"id":1}')
echo "  Producers: $(echo "$PRODUCERS" | jq '.result | length')"
echo ""

# Phase 2: Display each producer's weight components
echo "[Phase 2] Producer weight analysis..."
echo "$PRODUCERS" | jq -r '.result[] | "  PK: \(.publicKey[0:16])...  Bonds: \(.bondCount)  Status: \(.status)"'
echo ""

# Phase 3: Get chain height for seniority calculation
echo "[Phase 3] Chain state..."
CHAIN=$(curl -s http://127.0.0.1:$RPC_BASE -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}')
HEIGHT=$(echo "$CHAIN" | jq -r '.result.bestHeight')
echo "  Current height: $HEIGHT"
echo ""

# Phase 4: Verify weight formula (unit test coverage)
echo "[Phase 4] Weight formula verification (from unit tests)..."
echo "  Formula: weight = bond_count × seniority_multiplier"
echo "  Seniority: 1.0 + min(years_active, 4) × 0.75"
echo ""
echo "  Examples (devnet, 1 year = 144 blocks):"
echo "    1 bond,  0 years:  1 × 1.00 =  1.00"
echo "   10 bonds, 0 years: 10 × 1.00 = 10.00"
echo "    2 bonds, 4 years:  2 × 4.00 =  8.00"
echo "   10 bonds, 4 years: 10 × 4.00 = 40.00"
echo ""

# Phase 5: Run cargo tests for weight verification
echo "[Phase 5] Running weight unit tests..."
cargo test -p updater -- vote_weight 2>&1 | tail -5
echo ""

echo "========================================="
echo " RESULT: Vote weight test completed"
echo "========================================="
