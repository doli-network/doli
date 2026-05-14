#!/usr/bin/env bash
#
# OUTPUT CONTRACT: scripts/install.sh — idempotent reinstall behavior
#
# Outputs observed (via stub call-tracking files):
#   O1: install_calls — count of `install -m 755 ... /usr/bin/...` invocations from install.sh
#   O2: tar_calls    — count of `tar -xzf <tarball> -C <dir>` invocations from install.sh
#   O3: exit_code    — install.sh exit code
#   O4: skip_message — 1 if stdout contains "already installed", else 0
#
# PATHS:
#   PA: install.sh detects same version already installed → early-exit, no work performed
#   PB: install.sh proceeds with full install (download → extract → install to /usr/bin)
#
# INPUT PARTITIONS: each partition exercises a distinct branch of the version-skip logic.
#   P1: installed=6.21.19, latest=v6.21.19, no force        → PA: O1=0, O2=0, O3=0, O4=1
#   P2: installed=6.21.18, latest=v6.21.19, no force        → PB: O1>=2, O2>=1, O3=0, O4=0
#   P3: no doli-node in PATH, latest=v6.21.19               → PB: O1>=2, O2>=1, O3=0, O4=0
#   P4: installed=6.21.19, latest=v6.21.19, FORCE=1         → PB: O1>=2, O2>=1, O3=0, O4=0
#
# MATRIX: 4 partitions × 4 outputs = 16 cells, every cell has an explicit assertion below.
# Reference: INC-I-076

set -u

# -- Test scope ------------------------------------------------------
# Verifies install.sh's idempotency check (the only behavior under test).
# Test runs the Darwin branch of install.sh; the Linux-only system-user
# block is not exercised here — the version-skip logic is OS-agnostic
# and lives BEFORE the Linux branch.
case "$(uname -s)" in
    Darwin) ;;
    *)
        echo "skip: this test only runs on Darwin (install.sh Linux branch needs root)"
        exit 0
        ;;
esac

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
INSTALL_SH="$REPO_ROOT/scripts/install.sh"

[ -f "$INSTALL_SH" ] || { echo "fatal: $INSTALL_SH not found"; exit 2; }

FAIL=0
PASS_COUNT=0
FAIL_COUNT=0

assert_eq() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        printf "    PASS  %-32s expected=%s got=%s\n" "$name" "$expected" "$actual"
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        printf "    FAIL  %-32s expected=%s got=%s\n" "$name" "$expected" "$actual"
        FAIL_COUNT=$((FAIL_COUNT + 1))
        FAIL=1
    fi
}

assert_ge() {
    local name="$1" min="$2" actual="$3"
    if [ "$actual" -ge "$min" ] 2>/dev/null; then
        printf "    PASS  %-32s min=%s got=%s\n" "$name" "$min" "$actual"
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        printf "    FAIL  %-32s min=%s got=%s\n" "$name" "$min" "$actual"
        FAIL_COUNT=$((FAIL_COUNT + 1))
        FAIL=1
    fi
}

# -- Per-partition environment setup ---------------------------------
# Creates a hermetic PATH containing ONLY:
#   - stubs we install for the partition (curl, sudo, install, tar, doli-node?)
#   - whitelisted real system bins (mktemp, sed, grep, head, find, awk, etc.)
# This guarantees `command -v doli-node` only sees our stub (or nothing),
# regardless of whether /usr/bin/doli-node exists on the dev box.

make_env() {
    local installed_version="$1"   # empty → doli-node not installed
    local force_flag="$2"           # "1" → DOLI_FORCE_INSTALL=1

    TMPROOT=$(mktemp -d)
    STUB_DIR="$TMPROOT/stubs"
    WHITELIST_DIR="$TMPROOT/whitelist"
    TRACK_DIR="$TMPROOT/track"
    mkdir -p "$STUB_DIR" "$WHITELIST_DIR" "$TRACK_DIR"
    : > "$TRACK_DIR/install_calls"
    : > "$TRACK_DIR/tar_calls"

    # Whitelist real system bins by symlink — anything install.sh needs
    # that we're NOT explicitly stubbing.
    for cmd in bash sh mktemp mkdir rm sed head grep find cp awk uname id chmod chown printf echo cat tr wc dirname which command; do
        real=$(/usr/bin/which "$cmd" 2>/dev/null || true)
        [ -n "$real" ] && ln -sf "$real" "$WHITELIST_DIR/$cmd"
    done

    # Stub: doli-node (only when partition specifies an installed version)
    if [ -n "$installed_version" ]; then
        cat > "$STUB_DIR/doli-node" <<EOF
#!/bin/sh
[ "\$1" = "--version" ] && echo "doli-node $installed_version"
EOF
        chmod +x "$STUB_DIR/doli-node"
    fi

    # Stub: curl — serves API JSON or creates a fake tarball
    cat > "$STUB_DIR/curl" <<'EOF'
#!/bin/sh
OUT_PATH=""
URL=""
while [ $# -gt 0 ]; do
    case "$1" in
        -o) OUT_PATH="$2"; shift 2 ;;
        -*) shift ;;
        *)  URL="$1"; shift ;;
    esac
done
if [ -n "$OUT_PATH" ]; then
    # Tarball download — fabricate a tarball with the expected layout.
    FAKE=$(mktemp -d)
    TARGET_DIR="doli-v6.21.19-aarch64-apple-darwin"
    mkdir -p "$FAKE/$TARGET_DIR"
    echo "fake-doli-node-binary" > "$FAKE/$TARGET_DIR/doli-node"
    echo "fake-doli-binary"      > "$FAKE/$TARGET_DIR/doli"
    /usr/bin/tar -czf "$OUT_PATH" -C "$FAKE" "$TARGET_DIR"
    rm -rf "$FAKE"
    exit 0
fi
# API call → return latest tag JSON
echo '{"tag_name":"v6.21.19"}'
EOF
    chmod +x "$STUB_DIR/curl"

    # Stub: sudo — drop privilege requirement, exec command directly
    cat > "$STUB_DIR/sudo" <<'EOF'
#!/bin/sh
exec "$@"
EOF
    chmod +x "$STUB_DIR/sudo"

    # Stub: install — log every invocation, do nothing
    cat > "$STUB_DIR/install" <<EOF
#!/bin/sh
echo "\$*" >> "$TRACK_DIR/install_calls"
EOF
    chmod +x "$STUB_DIR/install"

    # Stub: tar — log invocation, then exec real tar
    cat > "$STUB_DIR/tar" <<EOF
#!/bin/sh
echo "\$*" >> "$TRACK_DIR/tar_calls"
exec /usr/bin/tar "\$@"
EOF
    chmod +x "$STUB_DIR/tar"

    SANDBOX_PATH="$STUB_DIR:$WHITELIST_DIR"

    if [ "$force_flag" = "1" ]; then
        FORCE_ENV="DOLI_FORCE_INSTALL=1"
    else
        FORCE_ENV=""
    fi
}

cleanup_env() {
    rm -rf "$TMPROOT"
}

run_install() {
    # Use env -i so the install.sh script inherits ONLY the variables we hand it.
    env -i HOME="$HOME" USER="$USER" PATH="$SANDBOX_PATH" $FORCE_ENV bash "$INSTALL_SH"
}

count_lines() {
    wc -l < "$1" | tr -d '[:space:]'
}

# -- P1: same version installed, no force → SKIP --------------------
echo "[P1] installed=6.21.19  latest=v6.21.19  force=no   → expect SKIP"
make_env "6.21.19" ""
OUT=$(run_install 2>&1)
EC=$?
INSTALL_CALLS=$(count_lines "$TRACK_DIR/install_calls")
TAR_CALLS=$(count_lines "$TRACK_DIR/tar_calls")
SKIP_MSG=0; echo "$OUT" | grep -q "already installed" && SKIP_MSG=1
assert_eq "P1.exit_code"    "0" "$EC"
assert_eq "P1.install_calls" "0" "$INSTALL_CALLS"
assert_eq "P1.tar_calls"    "0" "$TAR_CALLS"
assert_eq "P1.skip_message" "1" "$SKIP_MSG"
cleanup_env
echo ""

# -- P2: different version installed → PROCEED -----------------------
echo "[P2] installed=6.21.18  latest=v6.21.19  force=no   → expect PROCEED"
make_env "6.21.18" ""
OUT=$(run_install 2>&1)
EC=$?
INSTALL_CALLS=$(count_lines "$TRACK_DIR/install_calls")
TAR_CALLS=$(count_lines "$TRACK_DIR/tar_calls")
SKIP_MSG=0; echo "$OUT" | grep -q "already installed" && SKIP_MSG=1
assert_eq "P2.exit_code"    "0" "$EC"
assert_ge "P2.install_calls" "2" "$INSTALL_CALLS"
assert_ge "P2.tar_calls"    "1" "$TAR_CALLS"
assert_eq "P2.skip_message" "0" "$SKIP_MSG"
cleanup_env
echo ""

# -- P3: no doli-node in PATH → PROCEED ------------------------------
echo "[P3] installed=(none)   latest=v6.21.19  force=no   → expect PROCEED"
make_env "" ""
OUT=$(run_install 2>&1)
EC=$?
INSTALL_CALLS=$(count_lines "$TRACK_DIR/install_calls")
TAR_CALLS=$(count_lines "$TRACK_DIR/tar_calls")
SKIP_MSG=0; echo "$OUT" | grep -q "already installed" && SKIP_MSG=1
assert_eq "P3.exit_code"    "0" "$EC"
assert_ge "P3.install_calls" "2" "$INSTALL_CALLS"
assert_ge "P3.tar_calls"    "1" "$TAR_CALLS"
assert_eq "P3.skip_message" "0" "$SKIP_MSG"
cleanup_env
echo ""

# -- P4: same version installed + force → PROCEED --------------------
echo "[P4] installed=6.21.19  latest=v6.21.19  force=yes  → expect PROCEED"
make_env "6.21.19" "1"
OUT=$(run_install 2>&1)
EC=$?
INSTALL_CALLS=$(count_lines "$TRACK_DIR/install_calls")
TAR_CALLS=$(count_lines "$TRACK_DIR/tar_calls")
SKIP_MSG=0; echo "$OUT" | grep -q "already installed" && SKIP_MSG=1
assert_eq "P4.exit_code"    "0" "$EC"
assert_ge "P4.install_calls" "2" "$INSTALL_CALLS"
assert_ge "P4.tar_calls"    "1" "$TAR_CALLS"
assert_eq "P4.skip_message" "0" "$SKIP_MSG"
cleanup_env
echo ""

echo "════════════════════════════════════════════════════════════"
echo "  PASSED: $PASS_COUNT      FAILED: $FAIL_COUNT"
echo "════════════════════════════════════════════════════════════"

exit $FAIL
