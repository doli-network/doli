#!/usr/bin/env bash
#
# Test: Update Notification System
#
# Tests:
# 1. Mandatory notification display when update is detected
# 2. CLI vote command auto-detects pending version
# 3. Update status command shows full details
#
# Run: ./scripts/test_update_notification.sh
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
TEST_DIR="/tmp/doli-notification-test-$(date +%Y%m%d_%H%M%S)"
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
echo "DOLI Update Notification System Test"
echo "=============================================="
echo ""

# Check binaries
if [[ ! -x "$NODE_BIN" ]]; then
    error "doli-node binary not found. Run: cargo build --release"
    exit 1
fi

if [[ ! -x "$CLI_BIN" ]]; then
    error "doli CLI binary not found. Run: cargo build --release"
    exit 1
fi

# Create test directory structure
mkdir -p "$TEST_DIR"/{keys,.doli/devnet}
info "Test directory: $TEST_DIR"

# Generate a producer key
info "Generating producer key..."
$CLI_BIN -w "$TEST_DIR/keys/producer.json" new -n "test_producer" > /dev/null 2>&1

PRODUCER_PUBKEY=$(cat "$TEST_DIR/keys/producer.json" | jq -r '.addresses[0].public_key')
info "Producer pubkey: ${PRODUCER_PUBKEY:0:16}..."

# Create mock pending update
MOCK_VERSION="99.1.0-test"
PUBLISHED_AT=$(date +%s)

cat > "$TEST_DIR/.doli/devnet/pending_update.json" << EOF
{
  "release": {
    "version": "$MOCK_VERSION",
    "binary_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "binary_url_template": "https://example.com/releases/{platform}/doli-node",
    "changelog": "Test release for notification system.\n\nChanges:\n- New notification banner for pending updates\n- Auto-detect pending version in vote command",
    "published_at": $PUBLISHED_AT,
    "signatures": []
  },
  "vote_tracker": {
    "version": "$MOCK_VERSION",
    "vetos": [],
    "approvals": [],
    "producer_weights": {}
  },
  "first_notified_at": $PUBLISHED_AT
}
EOF

success "Mock pending update created: v$MOCK_VERSION"

# Set HOME for the node to find our test data
export HOME="$TEST_DIR"

echo ""
echo "=============================================="
echo "Test 1: Update status shows pending update"
echo "=============================================="

step "Running 'doli-node update status'..."

STATUS_OUTPUT=$($NODE_BIN --network devnet update status 2>&1)

echo "$STATUS_OUTPUT"
echo ""

# Check for expected content
if echo "$STATUS_OUTPUT" | grep -q "Pending Update"; then
    success "PASS: Status shows pending update header"
    ((TESTS_PASSED++))
else
    error "FAIL: Status doesn't show pending update"
    ((TESTS_FAILED++))
fi

if echo "$STATUS_OUTPUT" | grep -q "$MOCK_VERSION"; then
    success "PASS: Status shows correct version ($MOCK_VERSION)"
    ((TESTS_PASSED++))
else
    error "FAIL: Status doesn't show correct version"
    ((TESTS_FAILED++))
fi

if echo "$STATUS_OUTPUT" | grep -q "Changelog"; then
    success "PASS: Status shows changelog section"
    ((TESTS_PASSED++))
else
    error "FAIL: Status doesn't show changelog"
    ((TESTS_FAILED++))
fi

if echo "$STATUS_OUTPUT" | grep -q "How to Vote"; then
    success "PASS: Status shows voting instructions"
    ((TESTS_PASSED++))
else
    error "FAIL: Status doesn't show voting instructions"
    ((TESTS_FAILED++))
fi

if echo "$STATUS_OUTPUT" | grep -q "40%"; then
    success "PASS: Status shows veto threshold (40%)"
    ((TESTS_PASSED++))
else
    error "FAIL: Status doesn't show veto threshold"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 2: Vote command auto-detects version"
echo "=============================================="

step "Running 'doli-node update vote --veto' without specifying version..."

VOTE_OUTPUT=$($NODE_BIN --network devnet update vote --veto --key "$TEST_DIR/keys/producer.json" 2>&1)

echo "$VOTE_OUTPUT"
echo ""

# Check that it detected the version
if echo "$VOTE_OUTPUT" | grep -q "Voting on update: v$MOCK_VERSION"; then
    success "PASS: Vote command auto-detected version $MOCK_VERSION"
    ((TESTS_PASSED++))
else
    error "FAIL: Vote command didn't auto-detect version"
    ((TESTS_FAILED++))
fi

# Check that it created a signed vote
if echo "$VOTE_OUTPUT" | grep -q '"vote": "Veto"'; then
    success "PASS: Vote message contains veto vote"
    ((TESTS_PASSED++))
else
    error "FAIL: Vote message doesn't contain veto vote"
    ((TESTS_FAILED++))
fi

if echo "$VOTE_OUTPUT" | grep -q '"signature":'; then
    success "PASS: Vote message is signed"
    ((TESTS_PASSED++))
else
    error "FAIL: Vote message is not signed"
    ((TESTS_FAILED++))
fi

if echo "$VOTE_OUTPUT" | grep -q "submitVote"; then
    success "PASS: Vote output shows RPC submission instructions"
    ((TESTS_PASSED++))
else
    error "FAIL: Vote output doesn't show RPC instructions"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 3: Approve vote works"
echo "=============================================="

step "Testing approve vote..."

APPROVE_OUTPUT=$($NODE_BIN --network devnet update vote --approve --key "$TEST_DIR/keys/producer.json" 2>&1)

if echo "$APPROVE_OUTPUT" | grep -q '"vote": "Approve"'; then
    success "PASS: Approve vote created successfully"
    ((TESTS_PASSED++))
else
    error "FAIL: Approve vote not created"
    echo "$APPROVE_OUTPUT"
    ((TESTS_FAILED++))
fi

echo ""
echo "=============================================="
echo "Test 4: No pending update scenario"
echo "=============================================="

step "Testing with no pending update..."

# Remove the pending update file
rm -f "$TEST_DIR/.doli/devnet/pending_update.json"

NO_UPDATE_STATUS=$($NODE_BIN --network devnet update status 2>&1)

if echo "$NO_UPDATE_STATUS" | grep -q "No pending updates"; then
    success "PASS: Status correctly shows no pending updates"
    ((TESTS_PASSED++))
else
    error "FAIL: Status should show no pending updates"
    echo "$NO_UPDATE_STATUS"
    ((TESTS_FAILED++))
fi

NO_UPDATE_VOTE=$($NODE_BIN --network devnet update vote --veto --key "$TEST_DIR/keys/producer.json" 2>&1)

if echo "$NO_UPDATE_VOTE" | grep -q "No pending update found"; then
    success "PASS: Vote command correctly handles no pending update"
    ((TESTS_PASSED++))
else
    error "FAIL: Vote command should say no pending update"
    echo "$NO_UPDATE_VOTE"
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
