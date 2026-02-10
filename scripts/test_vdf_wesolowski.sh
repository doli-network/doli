#!/usr/bin/env bash
# =============================================================================
# test_vdf_wesolowski.sh - Feature 2: VDF ~55ms (Wesolowski over Class Groups)
# =============================================================================
#
# Tests DOLI's VDF system which uses TWO implementations:
#   - Hash-chain VDF (iterated SHA3): Block production (~55ms at 800K iterations)
#   - Wesolowski VDF (class groups): Producer registration (anti-Sybil)
#
# USAGE:
#   nix develop --command bash ./scripts/test_vdf_wesolowski.sh
#
# WHAT IT TESTS:
#   Phase 1: VDF crate unit tests (Wesolowski class group + proof + serialization)
#   Phase 2: Hash-chain VDF unit tests (heartbeat proofs, witness system)
#   Phase 3: Integration tests with --release (timing, anti-grinding, security)
#
# RUNTIME: ~2 minutes
# =============================================================================

set -eo pipefail

PASS=0
FAIL=0
WARN=0

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${CYAN}[$(date +%H:%M:%S)]${NC} $*"; }
pass() { echo -e "${GREEN}[PASS]${NC} $*"; PASS=$((PASS+1)); }
fail() { echo -e "${RED}[FAIL]${NC} $*"; FAIL=$((FAIL+1)); }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; WARN=$((WARN+1)); }

echo ""
echo "============================================================"
echo "  DOLI VDF Test (Wesolowski over Class Groups)"
echo "============================================================"
echo ""

# =============================================================================
# PHASE 1: VDF Crate Unit Tests (Wesolowski)
# =============================================================================
echo "------------------------------------------------------------"
echo "  PHASE 1: VDF Crate Unit Tests (Wesolowski)"
echo "------------------------------------------------------------"

log "Running vdf crate unit tests (--release)..."
if cargo test -p vdf --lib --release > /tmp/vdf_unit.log 2>&1; then
    total=$(grep "test result:" /tmp/vdf_unit.log | tail -1)
    pass "VDF crate unit tests: $total"
else
    grep -E "FAILED|panicked" /tmp/vdf_unit.log | head -5
    fail "VDF crate unit tests FAILED"
fi

# =============================================================================
# PHASE 2: Hash-Chain VDF Tests (Heartbeat/Witness)
# =============================================================================
echo ""
echo "------------------------------------------------------------"
echo "  PHASE 2: Hash-Chain VDF Tests (Heartbeat)"
echo "------------------------------------------------------------"

log "Running heartbeat VDF tests (--release)..."
if cargo test -p doli-core --lib heartbeat --release > /tmp/vdf_heartbeat.log 2>&1; then
    total=$(grep "test result:" /tmp/vdf_heartbeat.log | tail -1)
    pass "Heartbeat VDF tests: $total"
else
    grep -E "FAILED|panicked" /tmp/vdf_heartbeat.log | head -5
    fail "Heartbeat VDF tests FAILED"
fi

# =============================================================================
# PHASE 3: Integration Tests (Timing + Security)
# =============================================================================
echo ""
echo "------------------------------------------------------------"
echo "  PHASE 3: Integration Tests (Timing + Security)"
echo "------------------------------------------------------------"

log "Running integration tests (--release --nocapture)..."

if cargo test -p vdf --test wesolowski_bench --release -- --nocapture > /tmp/vdf_bench.log 2>&1; then
    total=$(grep "test result:" /tmp/vdf_bench.log | tail -1)
    pass "Integration tests: $total"

    # Extract timing data
    echo ""
    log "=== VDF Timing Results ==="

    compute_ms=$(grep "Compute:" /tmp/vdf_bench.log | head -1 | sed 's/.*Compute: //' | sed 's/ms.*//')
    verify_ms=$(grep "Verify:" /tmp/vdf_bench.log | head -1 | sed 's/.*Verify: *//' | sed 's/ms.*//')

    if [ -n "$compute_ms" ]; then
        log "  Hash-Chain VDF (800K iterations, block production):"
        log "    Compute: ${compute_ms}ms (target: ~55ms)"
        log "    Verify:  ${verify_ms}ms"

        compute_int=$(echo "$compute_ms" | tr -dc '0-9')
        if [ -n "$compute_int" ] && [ "$compute_int" -gt 0 ] 2>/dev/null; then
            if [ "$compute_int" -le 200 ]; then
                pass "Block VDF timing: ${compute_int}ms (within 4x of ~55ms target)"
            elif [ "$compute_int" -le 500 ]; then
                warn "Block VDF timing: ${compute_int}ms (slower than expected, but functional)"
            else
                fail "Block VDF timing: ${compute_int}ms (too slow, >500ms)"
            fi
        fi
    fi
else
    grep -E "FAILED|panicked" /tmp/vdf_bench.log | head -5
    fail "Integration tests FAILED"
fi

# Show individual test results
echo ""
log "Individual test results:"
grep "^test " /tmp/vdf_bench.log | while read -r line; do
    if echo "$line" | grep -q " ok$"; then
        echo -e "  ${GREEN}$line${NC}"
    elif echo "$line" | grep -q "FAILED$"; then
        echo -e "  ${RED}$line${NC}"
    else
        echo "  $line"
    fi
done

# =============================================================================
# SUMMARY
# =============================================================================
echo ""
echo "============================================================"
echo "  TEST SUMMARY"
echo "============================================================"
echo ""
echo -e "  ${GREEN}PASSED:${NC}   $PASS"
echo -e "  ${RED}FAILED:${NC}   $FAIL"
echo -e "  ${YELLOW}WARNINGS:${NC} $WARN"
echo ""

if [ "$FAIL" -eq 0 ]; then
    echo -e "  ${GREEN}OVERALL: ALL TESTS PASSED${NC}"
    exit 0
else
    echo -e "  ${RED}OVERALL: $FAIL TEST(S) FAILED${NC}"
    exit 1
fi
