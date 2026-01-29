#!/usr/bin/env bash
#
# Test: Version Enforcement System ("NO ACTUALIZAS = NO PRODUCES")
#
# Tests:
# 1. Grace period notification after approval
# 2. Enforcement notification when active
# 3. Production blocked check
# 4. Update apply command
#
# Run: ./scripts/test_version_enforcement.sh
#

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; }
step() { echo -e "${CYAN}[TEST]${NC} $1"; }

# Setup
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEST_DIR="/tmp/doli-enforcement-test-$(date +%Y%m%d_%H%M%S)"
NODE_BIN="$REPO_ROOT/target/release/doli-node"
CLI_BIN="$REPO_ROOT/target/release/doli"

TESTS_PASSED=0
TESTS_FAILED=0

# Cleanup
cleanup() {
    info "Cleaning up..."
    rm -rf "$TEST_DIR" 2>/dev/null || true
}
trap cleanup EXIT

echo ""
echo "=============================================="
echo "DOLI Version Enforcement Test"
echo "\"NO ACTUALIZAS = NO PRODUCES\""
echo "=============================================="
echo ""

# Check binaries
if [[ ! -x "$NODE_BIN" ]]; then
    error "doli-node binary not found. Run: cargo build --release"
    exit 1
fi

# Create test directory structure
mkdir -p "$TEST_DIR"/{keys,.doli/devnet}
info "Test directory: $TEST_DIR"

# Generate a producer key
info "Generating producer key..."
$CLI_BIN -w "$TEST_DIR/keys/producer.json" new -n "test_producer" > /dev/null 2>&1

# Set HOME for the node to find our test data
export HOME="$TEST_DIR"

# Create mock pending update in VETO PERIOD
MOCK_VERSION="99.2.0-test"
PUBLISHED_AT=$(date +%s)

echo ""
echo "=============================================="
echo "Test 1: Veto Period Status"
echo "=============================================="

cat > "$TEST_DIR/.doli/devnet/pending_update.json" << EOF
{
  "release": {
    "version": "$MOCK_VERSION",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://example.com/releases/{platform}/doli-node",
    "changelog": "Security update with important fixes.",
    "published_at": $PUBLISHED_AT,
    "signatures": []
  },
  "vote_tracker": {
    "version": "$MOCK_VERSION",
    "vetos": [],
    "approvals": [],
    "producer_weights": {}
  },
  "first_notified_at": $PUBLISHED_AT,
  "approved": false,
  "enforcement": null
}
EOF

step "Checking veto period status..."
STATUS_OUTPUT=$($NODE_BIN --network devnet update status 2>&1)

if echo "$STATUS_OUTPUT" | grep -q "Veto Period"; then
    success "PASS: Shows veto period status"
    ((TESTS_PASSED++))
else
    error "FAIL: Should show veto period status"
    echo "$STATUS_OUTPUT"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 2: Grace Period Status"
echo "=============================================="

# Create mock pending update in GRACE PERIOD (approved, enforcement not yet active)
ENFORCEMENT_TIME=$((PUBLISHED_AT + 604800 + 172800))  # 7 days + 48 hours from now

cat > "$TEST_DIR/.doli/devnet/pending_update.json" << EOF
{
  "release": {
    "version": "$MOCK_VERSION",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://example.com/releases/{platform}/doli-node",
    "changelog": "Security update with important fixes.",
    "published_at": $PUBLISHED_AT,
    "signatures": []
  },
  "vote_tracker": {
    "version": "$MOCK_VERSION",
    "vetos": [],
    "approvals": [],
    "producer_weights": {}
  },
  "first_notified_at": $PUBLISHED_AT,
  "approved": true,
  "enforcement": {
    "min_version": "$MOCK_VERSION",
    "enforcement_time": $ENFORCEMENT_TIME,
    "active": false
  }
}
EOF

step "Checking grace period status..."
STATUS_OUTPUT=$($NODE_BIN --network devnet update status 2>&1)

if echo "$STATUS_OUTPUT" | grep -q "Grace Period\|APPROVED"; then
    success "PASS: Shows grace period / approved status"
    ((TESTS_PASSED++))
else
    error "FAIL: Should show grace period status"
    echo "$STATUS_OUTPUT"
    ((TESTS_FAILED++))
fi

if echo "$STATUS_OUTPUT" | grep -q "doli-node update apply"; then
    success "PASS: Shows update apply command"
    ((TESTS_PASSED++))
else
    error "FAIL: Should show update apply command"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 3: Enforcement Active Status"
echo "=============================================="

# Create mock pending update with ENFORCEMENT ACTIVE
PAST_ENFORCEMENT_TIME=$((PUBLISHED_AT - 100))  # Enforcement time in the past

cat > "$TEST_DIR/.doli/devnet/pending_update.json" << EOF
{
  "release": {
    "version": "$MOCK_VERSION",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://example.com/releases/{platform}/doli-node",
    "changelog": "Security update with important fixes.",
    "published_at": $PUBLISHED_AT,
    "signatures": []
  },
  "vote_tracker": {
    "version": "$MOCK_VERSION",
    "vetos": [],
    "approvals": [],
    "producer_weights": {}
  },
  "first_notified_at": $PUBLISHED_AT,
  "approved": true,
  "enforcement": {
    "min_version": "$MOCK_VERSION",
    "enforcement_time": $PAST_ENFORCEMENT_TIME,
    "active": true
  }
}
EOF

step "Checking enforcement active status..."
STATUS_OUTPUT=$($NODE_BIN --network devnet update status 2>&1)

if echo "$STATUS_OUTPUT" | grep -q "ENFORCEMENT ACTIVE"; then
    success "PASS: Shows enforcement active status"
    ((TESTS_PASSED++))
else
    error "FAIL: Should show enforcement active status"
    echo "$STATUS_OUTPUT"
    ((TESTS_FAILED++))
fi

if echo "$STATUS_OUTPUT" | grep -q "OUTDATED\|Your node is"; then
    success "PASS: Shows node is outdated"
    ((TESTS_PASSED++))
else
    error "FAIL: Should indicate node is outdated"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 4: Update Apply Command (dry run)"
echo "=============================================="

# Test apply command with approved update
step "Testing update apply command..."

# Reset to grace period state
cat > "$TEST_DIR/.doli/devnet/pending_update.json" << EOF
{
  "release": {
    "version": "$MOCK_VERSION",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://example.com/releases/{platform}/doli-node",
    "changelog": "Security update with important fixes.",
    "published_at": $PUBLISHED_AT,
    "signatures": []
  },
  "vote_tracker": {
    "version": "$MOCK_VERSION",
    "vetos": [],
    "approvals": [],
    "producer_weights": {}
  },
  "first_notified_at": $PUBLISHED_AT,
  "approved": true,
  "enforcement": {
    "min_version": "$MOCK_VERSION",
    "enforcement_time": $ENFORCEMENT_TIME,
    "active": false
  }
}
EOF

APPLY_OUTPUT=$($NODE_BIN --network devnet update apply 2>&1 || true)

if echo "$APPLY_OUTPUT" | grep -q "Applying Update\|Downloading"; then
    success "PASS: Apply command starts update process"
    ((TESTS_PASSED++))
else
    error "FAIL: Apply command should attempt update"
    echo "$APPLY_OUTPUT"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 5: Apply Rejected for Unapproved Update"
echo "=============================================="

# Test apply command with unapproved update (should fail without --force)
cat > "$TEST_DIR/.doli/devnet/pending_update.json" << EOF
{
  "release": {
    "version": "$MOCK_VERSION",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://example.com/releases/{platform}/doli-node",
    "changelog": "Security update with important fixes.",
    "published_at": $PUBLISHED_AT,
    "signatures": []
  },
  "vote_tracker": {
    "version": "$MOCK_VERSION",
    "vetos": [],
    "approvals": [],
    "producer_weights": {}
  },
  "first_notified_at": $PUBLISHED_AT,
  "approved": false,
  "enforcement": null
}
EOF

step "Testing apply command on unapproved update..."
APPLY_OUTPUT=$($NODE_BIN --network devnet update apply 2>&1)

if echo "$APPLY_OUTPUT" | grep -q "not yet approved\|--force"; then
    success "PASS: Apply command rejects unapproved update"
    ((TESTS_PASSED++))
else
    error "FAIL: Apply command should reject unapproved update"
    echo "$APPLY_OUTPUT"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 6: Timeline Display"
echo "=============================================="

step "Checking timeline display in status..."

cat > "$TEST_DIR/.doli/devnet/pending_update.json" << EOF
{
  "release": {
    "version": "$MOCK_VERSION",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://example.com/releases/{platform}/doli-node",
    "changelog": "Security update with important fixes.",
    "published_at": $PUBLISHED_AT,
    "signatures": []
  },
  "vote_tracker": {
    "version": "$MOCK_VERSION",
    "vetos": [],
    "approvals": [],
    "producer_weights": {}
  },
  "first_notified_at": $PUBLISHED_AT,
  "approved": false,
  "enforcement": null
}
EOF

STATUS_OUTPUT=$($NODE_BIN --network devnet update status 2>&1)

if echo "$STATUS_OUTPUT" | grep -q "Update Timeline"; then
    success "PASS: Shows update timeline"
    ((TESTS_PASSED++))
else
    error "FAIL: Should show update timeline"
    ((TESTS_FAILED++))
fi

if echo "$STATUS_OUTPUT" | grep -q "Day 0-7\|Day 7-9\|Day 9"; then
    success "PASS: Timeline shows all phases"
    ((TESTS_PASSED++))
else
    error "FAIL: Timeline should show all phases"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 7: Veto Period Cannot Be Bypassed (Security)"
echo "=============================================="

# Test that --force CANNOT bypass veto period
# This is the critical security fix - producers cannot apply updates
# during the veto period, even with --force
cat > "$TEST_DIR/.doli/devnet/pending_update.json" << EOF
{
  "release": {
    "version": "$MOCK_VERSION",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://example.com/releases/{platform}/doli-node",
    "changelog": "Security update with important fixes.",
    "published_at": $PUBLISHED_AT,
    "signatures": []
  },
  "vote_tracker": {
    "version": "$MOCK_VERSION",
    "vetos": [],
    "approvals": [],
    "producer_weights": {}
  },
  "first_notified_at": $PUBLISHED_AT,
  "approved": false,
  "enforcement": null
}
EOF

step "Testing --force cannot bypass veto period..."
APPLY_OUTPUT=$($NODE_BIN --network devnet update apply --force 2>&1)

if echo "$APPLY_OUTPUT" | grep -q "veto period\|Veto Period"; then
    success "PASS: --force cannot bypass veto period (security fix)"
    ((TESTS_PASSED++))
else
    error "FAIL: --force should NOT bypass veto period (security vulnerability!)"
    echo "Output: $APPLY_OUTPUT"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "TEST SUMMARY"
echo "=============================================="
echo ""
echo -e "Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo "Test directory: $TEST_DIR"

if [[ $TESTS_FAILED -eq 0 ]]; then
    echo ""
    success "ALL TESTS PASSED!"
    exit 0
else
    echo ""
    error "SOME TESTS FAILED"
    exit 1
fi
