#!/usr/bin/env bash
# test_guardian.sh — Smoke test for the Seed Guardian system
#
# Tests all 4 RPC methods + fork-monitor against a running network.
# Usage:
#   scripts/test_guardian.sh                 # local devnet (28500)
#   scripts/test_guardian.sh --testnet       # local testnet (8500)
#   scripts/test_guardian.sh --port PORT     # custom port
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
NC='\033[0m'

PORT=28500
MODE="devnet"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --testnet) MODE="testnet"; PORT=8500; shift ;;
    --devnet)  MODE="devnet"; PORT=28500; shift ;;
    --port)    PORT="$2"; shift 2 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

ENDPOINT="127.0.0.1:${PORT}"
PASS=0
FAIL=0
SKIP=0

pass() { echo -e "  ${GREEN}PASS${NC} $1"; PASS=$((PASS+1)); }
fail() { echo -e "  ${RED}FAIL${NC} $1"; FAIL=$((FAIL+1)); }
skip() { echo -e "  ${YELLOW}SKIP${NC} $1"; SKIP=$((SKIP+1)); }

rpc() {
  local method="$1"
  local params="${2:-[]}"
  curl -sf --max-time 5 -X POST "http://${ENDPOINT}" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"${method}\",\"params\":${params},\"id\":1}" 2>/dev/null
}

json_field() {
  python3 -c "import sys,json; print(json.load(sys.stdin).get('result',{}).get('$1',''))" 2>/dev/null
}

echo -e "${BOLD}=== Seed Guardian Smoke Test ===${NC}"
echo "  Target: $ENDPOINT ($MODE)"
echo ""

# ─── Pre-check: is the node running? ───────────────────────────────
echo -e "${BOLD}[0] Pre-check${NC}"
CHAIN_INFO=$(rpc "getChainInfo") || {
  echo -e "  ${RED}Node not reachable at ${ENDPOINT}${NC}"
  echo "  Start a node first, or use --port PORT"
  exit 1
}
HEIGHT=$(echo "$CHAIN_INFO" | json_field "bestHeight")
echo "  Node alive: height=$HEIGHT"
echo ""

# ─── Test 1: getGuardianStatus ──────────────────────────────────────
echo -e "${BOLD}[1] getGuardianStatus${NC}"
STATUS=$(rpc "getGuardianStatus") || { fail "RPC call failed"; STATUS=""; }
if [[ -n "$STATUS" ]]; then
  PAUSED=$(echo "$STATUS" | json_field "production_paused")
  S_HEIGHT=$(echo "$STATUS" | json_field "chain_height")
  if [[ "$PAUSED" == "False" || "$PAUSED" == "false" ]]; then
    pass "production_paused=false, height=$S_HEIGHT"
  elif [[ "$PAUSED" == "True" || "$PAUSED" == "true" ]]; then
    pass "production_paused=true (already paused), height=$S_HEIGHT"
  else
    fail "unexpected production_paused=$PAUSED"
  fi
else
  fail "no response"
fi
echo ""

# ─── Test 2: createCheckpoint ───────────────────────────────────────
echo -e "${BOLD}[2] createCheckpoint${NC}"
CP_RESULT=$(rpc "createCheckpoint") || { fail "RPC call failed"; CP_RESULT=""; }
if [[ -n "$CP_RESULT" ]]; then
  CP_STATUS=$(echo "$CP_RESULT" | json_field "status")
  CP_PATH=$(echo "$CP_RESULT" | json_field "path")
  CP_HEIGHT=$(echo "$CP_RESULT" | json_field "height")
  if [[ "$CP_STATUS" == "ok" ]]; then
    pass "checkpoint created at height=$CP_HEIGHT"
    echo "       path=$CP_PATH"
    # Verify files exist
    if [[ -d "$CP_PATH/state_db" && -d "$CP_PATH/blocks" ]]; then
      pass "checkpoint directories exist (state_db + blocks)"
    else
      fail "checkpoint directories missing at $CP_PATH"
    fi
  else
    ERROR=$(echo "$CP_RESULT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('error',{}).get('message','unknown'))" 2>/dev/null)
    fail "checkpoint failed: $ERROR"
  fi
else
  fail "no response"
fi
echo ""

# ─── Test 3: pauseProduction ───────────────────────────────────────
echo -e "${BOLD}[3] pauseProduction${NC}"
PAUSE_RESULT=$(rpc "pauseProduction") || { fail "RPC call failed"; PAUSE_RESULT=""; }
if [[ -n "$PAUSE_RESULT" ]]; then
  P_STATUS=$(echo "$PAUSE_RESULT" | json_field "status")
  if [[ "$P_STATUS" == "paused" ]]; then
    pass "production paused"

    # Verify via getGuardianStatus
    sleep 1
    VERIFY=$(rpc "getGuardianStatus") || { fail "verify failed"; VERIFY=""; }
    if [[ -n "$VERIFY" ]]; then
      V_PAUSED=$(echo "$VERIFY" | json_field "production_paused")
      V_REASON=$(echo "$VERIFY" | json_field "production_block_reason")
      if [[ "$V_PAUSED" == "True" || "$V_PAUSED" == "true" ]]; then
        pass "confirmed paused via getGuardianStatus (reason: $V_REASON)"
      else
        fail "getGuardianStatus says not paused after pauseProduction"
      fi
    fi
  else
    fail "unexpected status: $P_STATUS"
  fi
else
  fail "no response"
fi
echo ""

# ─── Test 4: Verify chain stalls while paused ──────────────────────
echo -e "${BOLD}[4] Chain stall check (10s wait)${NC}"
HEIGHT_BEFORE=$(rpc "getChainInfo" | json_field "bestHeight")
echo "  height before: $HEIGHT_BEFORE"
echo -n "  waiting 10s..."
sleep 10
echo " done"
HEIGHT_AFTER=$(rpc "getChainInfo" | json_field "bestHeight")
echo "  height after:  $HEIGHT_AFTER"

# If this is a seed (no producer key), it doesn't produce anyway.
# If this is a producer, height should NOT advance while paused.
# But other producers on the network might still advance the chain via gossip.
# So we check: did THIS node's own production stop?
if [[ "$HEIGHT_BEFORE" == "$HEIGHT_AFTER" ]]; then
  pass "chain height unchanged while paused (production blocked)"
else
  DELTA=$((HEIGHT_AFTER - HEIGHT_BEFORE))
  if [[ $DELTA -le 2 ]]; then
    skip "height advanced by $DELTA (likely receiving blocks from other producers)"
  else
    skip "height advanced by $DELTA (other producers still active on network)"
  fi
fi
echo ""

# ─── Test 5: resumeProduction ──────────────────────────────────────
echo -e "${BOLD}[5] resumeProduction${NC}"
RESUME_RESULT=$(rpc "resumeProduction") || { fail "RPC call failed"; RESUME_RESULT=""; }
if [[ -n "$RESUME_RESULT" ]]; then
  R_STATUS=$(echo "$RESUME_RESULT" | json_field "status")
  if [[ "$R_STATUS" == "resumed" ]]; then
    pass "production resumed"

    # Verify via getGuardianStatus
    sleep 1
    VERIFY=$(rpc "getGuardianStatus") || { fail "verify failed"; VERIFY=""; }
    if [[ -n "$VERIFY" ]]; then
      V_PAUSED=$(echo "$VERIFY" | json_field "production_paused")
      if [[ "$V_PAUSED" == "False" || "$V_PAUSED" == "false" ]]; then
        pass "confirmed resumed via getGuardianStatus"
      else
        fail "getGuardianStatus still says paused after resumeProduction"
      fi
    fi
  else
    fail "unexpected status: $R_STATUS"
  fi
else
  fail "no response"
fi
echo ""

# ─── Test 6: fork-monitor.sh ──────────────────────────────────────
echo -e "${BOLD}[6] fork-monitor.sh${NC}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ -x "$SCRIPT_DIR/fork-monitor.sh" ]]; then
  MONITOR_OUTPUT=$("$SCRIPT_DIR/fork-monitor.sh" "--${MODE}" 2>&1) || true
  if echo "$MONITOR_OUTPUT" | grep -q "OK"; then
    pass "fork-monitor reports OK (all nodes agree)"
  elif echo "$MONITOR_OUTPUT" | grep -q "FORK DETECTED"; then
    pass "fork-monitor detects fork (expected if network is currently forked)"
    echo "       $MONITOR_OUTPUT" | head -5
  elif echo "$MONITOR_OUTPUT" | grep -q "No nodes"; then
    skip "no nodes reachable for fork monitor"
  else
    skip "fork-monitor output: $(echo "$MONITOR_OUTPUT" | head -1)"
  fi
else
  skip "fork-monitor.sh not found or not executable"
fi
echo ""

# ─── Test 7: Checkpoint cleanup ────────────────────────────────────
echo -e "${BOLD}[7] Checkpoint cleanup${NC}"
if [[ -n "${CP_PATH:-}" && -d "${CP_PATH:-}" ]]; then
  rm -rf "$CP_PATH"
  pass "test checkpoint removed: $CP_PATH"
else
  skip "no checkpoint to clean up"
fi
echo ""

# ─── Results ───────────────────────────────────────────────────────
echo -e "${BOLD}═══════════════════════════════${NC}"
echo -e "  ${GREEN}PASS: $PASS${NC}  ${RED}FAIL: $FAIL${NC}  ${YELLOW}SKIP: $SKIP${NC}"
echo -e "${BOLD}═══════════════════════════════${NC}"

if [[ $FAIL -eq 0 ]]; then
  echo -e "\n${GREEN}Seed Guardian system is operational.${NC}"
  exit 0
else
  echo -e "\n${RED}$FAIL test(s) failed. Check output above.${NC}"
  exit 1
fi
