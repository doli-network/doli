#!/bin/sh
set -e

REPO="doli-network/doli"
GITHUB="https://github.com/${REPO}"
API="https://api.github.com/repos/${REPO}/releases/latest"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info() { printf "${CYAN}==>${NC} %s\n" "$1"; }
ok()   { printf "${GREEN}==>${NC} %s\n" "$1"; }
err()  { printf "${RED}error:${NC} %s\n" "$1" >&2; exit 1; }

OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
    Darwin) OS_LABEL="macOS" ;;
    Linux)  OS_LABEL="Linux" ;;
    *)      err "Unsupported OS: $OS. See ${GITHUB}/releases" ;;
esac

case "$ARCH" in
    x86_64|amd64)  ARCH_LABEL="x86_64" ;;
    aarch64|arm64) ARCH_LABEL="aarch64" ;;
    *)             err "Unsupported architecture: $ARCH" ;;
esac

case "${OS}-${ARCH_LABEL}" in
    Darwin-aarch64) TARGET="aarch64-apple-darwin" ;;
    Darwin-x86_64)  TARGET="aarch64-apple-darwin" ;;
    Linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
    Linux-aarch64)  TARGET="aarch64-unknown-linux-gnu" ;;
esac

info "Platform: ${OS_LABEL} ${ARCH_LABEL}"

info "Fetching latest release..."

if command -v curl >/dev/null 2>&1; then
    FETCH="curl -sSfL"
    FETCH_OUT="curl -sSfL -o"
elif command -v wget >/dev/null 2>&1; then
    FETCH="wget -qO-"
    FETCH_OUT="wget -qO"
else
    err "curl or wget is required"
fi

RELEASE_JSON=$($FETCH "$API") || err "Failed to fetch release info. Check ${GITHUB}/releases"

VERSION=$(printf '%s' "$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
[ -z "$VERSION" ] && err "Could not determine latest version"

info "Latest version: ${VERSION}"

# Flags: --force (reinstall same version), --no-restart (leave running units alone).
# Env equivalents keep the `curl ... | sudo sh` one-liner usable: DOLI_FORCE_INSTALL=1,
# DOLI_NO_RESTART=1. With args: curl ... | sudo sh -s -- --force
FORCE="${DOLI_FORCE_INSTALL:-}"
NO_RESTART="${DOLI_NO_RESTART:-}"
for arg in "$@"; do
    case "$arg" in
        --force)      FORCE=1 ;;
        --no-restart) NO_RESTART=1 ;;
        *) err "Unknown argument: $arg (accepted: --force, --no-restart)" ;;
    esac
done
RPC_PORT="${DOLI_RPC_PORT:-8500}"

# Running node detection (Linux/systemd). A binary replaced on disk does NOT change the
# running process: the node keeps executing the old version until its unit restarts.
UNITS=""
if [ "$OS" = "Linux" ] && command -v systemctl >/dev/null 2>&1; then
    UNITS=$(systemctl list-units --type=service --state=active --no-legend --plain 2>/dev/null \
            | awk '{print $1}' | grep '^doli' | tr '\n' ' ' | sed 's/ *$//')
fi
UNIT_COUNT=$(printf '%s' "$UNITS" | wc -w | tr -d ' ')

rpc_call() {
    command -v curl >/dev/null 2>&1 || return 1
    curl -s -m 5 -X POST -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":[]}" \
        "http://127.0.0.1:${RPC_PORT}" 2>/dev/null
}
rpc_version() { rpc_call getNodeInfo  | sed -n 's/.*"version":"\([^"]*\)".*/\1/p'; }
rpc_height()  { rpc_call getChainInfo | sed -n 's/.*"bestHeight":\([0-9]*\).*/\1/p'; }

# Verify that the RUNNING node serves the expected version and advances. Single-unit hosts
# only (the RPC port of a multi-unit host is not knowable here). Returns 1 on any failure.
verify_running() {
    _want="$1"; _v=""; _i=0
    while [ $_i -lt 9 ]; do
        _v=$(rpc_version); [ -n "$_v" ] && break
        sleep 10; _i=$((_i+1))
    done
    if [ "$_v" != "$_want" ]; then
        printf "${RED}==>${NC} running node reports version '%s', expected %s\n" "$_v" "$_want" >&2
        return 1
    fi
    _h0=$(rpc_height); info "Running ${_v}; checking that the chain height advances (60 s)..."
    sleep 60
    _h1=$(rpc_height)
    if [ -z "$_h0" ] || [ -z "$_h1" ] || [ "$_h1" -le "$_h0" ]; then
        printf "${RED}==>${NC} chain height did not advance (%s -> %s)\n" "$_h0" "$_h1" >&2
        return 1
    fi
    ok "Running node: version ${_v}, height ${_h0} -> ${_h1}"
}

LATEST_BARE=${VERSION#v}
RESTART_ONLY=0
if [ -z "$FORCE" ] && command -v doli-node >/dev/null 2>&1; then
    INSTALLED=$(doli-node --version 2>/dev/null | awk '{print $2}')
    if [ -n "$INSTALLED" ] && [ "$INSTALLED" = "$LATEST_BARE" ]; then
        RUNNING=""
        [ "$UNIT_COUNT" = "1" ] && RUNNING=$(rpc_version)
        if [ -n "$RUNNING" ] && [ "$RUNNING" != "$LATEST_BARE" ] && [ -z "$NO_RESTART" ]; then
            info "DOLI ${VERSION} is on disk but ${UNITS} still runs ${RUNNING} — restarting it"
            RESTART_ONLY=1
        else
            ok "DOLI ${VERSION} already installed at $(command -v doli-node) — nothing to do"
            [ -n "$RUNNING" ] && echo "    Running node reports ${RUNNING}."
            echo "    Re-run with --force or DOLI_FORCE_INSTALL=1 to reinstall."
            exit 0
        fi
    fi
    [ -n "$INSTALLED" ] && [ "$RESTART_ONLY" = "0" ] && info "Currently installed: ${INSTALLED}, upgrading to ${LATEST_BARE}"
fi

if [ "$RESTART_ONLY" = "1" ]; then
    systemctl restart $UNITS
    verify_running "$LATEST_BARE" || err "Restart of ${UNITS} did not bring up ${VERSION}. Check: journalctl -u ${UNITS}"
    ok "DOLI ${VERSION} running (${UNITS} restarted)"
    exit 0
fi

# Always use tarball — it's the only format guaranteed to exist in every release.
# .deb and .rpm are optional and may not be built for every version.
FILE="doli-${VERSION}-${TARGET}.tar.gz"
METHOD="tarball"

URL="${GITHUB}/releases/download/${VERSION}/${FILE}"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

info "Downloading ${FILE}..."
$FETCH_OUT "${TMPDIR}/${FILE}" "$URL" || err "Download failed. Check ${GITHUB}/releases/tag/${VERSION}"

# ISSUE-174 #3: verify SHA-256 of tarball against CHECKSUMS.txt from the same release.
# CHECKSUMS.txt is signed and shipped by the release CI; the auto-updater already
# verifies releases this way. We fail closed if either the file or the hash is missing.
CHECKSUMS_URL="${GITHUB}/releases/download/${VERSION}/CHECKSUMS.txt"
info "Verifying integrity..."
$FETCH_OUT "${TMPDIR}/CHECKSUMS.txt" "$CHECKSUMS_URL" \
    || err "Could not download CHECKSUMS.txt from ${CHECKSUMS_URL}. Refusing to install unverified binary."

EXPECTED_HASH=$(grep " ${FILE}$" "${TMPDIR}/CHECKSUMS.txt" | awk '{print $1}' | head -1)
[ -z "$EXPECTED_HASH" ] && err "CHECKSUMS.txt does not contain an entry for ${FILE}. Refusing to install."

if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_HASH=$(sha256sum "${TMPDIR}/${FILE}" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL_HASH=$(shasum -a 256 "${TMPDIR}/${FILE}" | awk '{print $1}')
else
    err "sha256sum/shasum not found — cannot verify integrity. Install coreutils or perl."
fi

if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
    err "Checksum mismatch for ${FILE}: expected ${EXPECTED_HASH}, got ${ACTUAL_HASH}"
fi
ok "Checksum OK (${ACTUAL_HASH})"

info "Extracting..."
tar -xzf "${TMPDIR}/${FILE}" -C "$TMPDIR"
DIR=$(find "$TMPDIR" -maxdepth 1 -type d -name "doli-*" | head -1)
[ -z "$DIR" ] && err "Failed to extract archive"
# Back up the binaries currently on disk so a failed upgrade can be reverted.
BK=".pre-${VERSION}-backup"
[ -f /usr/bin/doli-node ] && cp -p /usr/bin/doli-node "/usr/bin/doli-node${BK}"
[ -f /usr/bin/doli ]      && cp -p /usr/bin/doli      "/usr/bin/doli${BK}"

restore_backups() {
    printf "${RED}==>${NC} Restoring previous binaries\n" >&2
    [ -f "/usr/bin/doli-node${BK}" ] && install -m 755 "/usr/bin/doli-node${BK}" /usr/bin/doli-node
    [ -f "/usr/bin/doli${BK}" ]      && install -m 755 "/usr/bin/doli${BK}"      /usr/bin/doli
    [ -n "$UNITS" ] && systemctl start $UNITS 2>/dev/null
    err "Upgrade to ${VERSION} failed; previous binaries restored and ${UNITS:-no unit} restarted."
}

STOPPED=""
if [ -n "$UNITS" ] && [ -z "$NO_RESTART" ]; then
    info "Stopping ${UNITS}..."
    systemctl stop $UNITS
    STOPPED="$UNITS"
fi

info "Installing to /usr/bin..."
sudo install -m 755 "${DIR}/doli-node" /usr/bin/doli-node || restore_backups
sudo install -m 755 "${DIR}/doli"      /usr/bin/doli      || restore_backups

if [ -n "$STOPPED" ]; then
    info "Starting ${STOPPED}..."
    systemctl start $STOPPED || restore_backups
    sleep 5
    for u in $STOPPED; do
        systemctl is-active --quiet "$u" || { printf "${RED}==>${NC} %s is not active after start\n" "$u" >&2; restore_backups; }
    done
    if [ "$UNIT_COUNT" = "1" ]; then
        verify_running "$LATEST_BARE" || restore_backups
    else
        ok "Restarted ${STOPPED} (multi-unit host: running version not checked over RPC)"
    fi
fi

# ---------------------------------------------------------------------------
# Install agent skills to ~/.doli/skills/
# ---------------------------------------------------------------------------
SKILL_COUNT=0
REAL_USER="${SUDO_USER:-$USER}"
REAL_HOME=$(eval echo "~$REAL_USER")
SKILLS_DIR="$REAL_HOME/.doli/skills"

if [ -d "${DIR}/skills" ]; then
    info "Installing agent skills to ${SKILLS_DIR}..."
    rm -rf "$SKILLS_DIR"
    mkdir -p "$SKILLS_DIR"
    cp -r "${DIR}/skills/"* "$SKILLS_DIR/"
    SKILL_COUNT=$(find "$SKILLS_DIR" -name "SKILL.md" | wc -l | tr -d ' ')
    # Fix ownership if running as sudo
    if [ -n "$SUDO_USER" ]; then
        chown -R "$REAL_USER" "$REAL_HOME/.doli"
    fi
fi

# ---------------------------------------------------------------------------
# Linux-only: create system user, group, directories, and polkit rule
# ---------------------------------------------------------------------------
NEEDS_RELOGIN=0

if [ "$OS" = "Linux" ]; then

    # 1. Create doli system user + group (if not already exists)
    if ! id -u doli >/dev/null 2>&1; then
        useradd --system --home-dir /var/lib/doli --shell /usr/sbin/nologin --create-home doli
        info "Created system user 'doli'"
    fi

    # 2. Add the current (real) user to the doli group
    REAL_USER="${SUDO_USER:-$USER}"
    if [ -n "$REAL_USER" ] && [ "$REAL_USER" != "root" ]; then
        if ! id -nG "$REAL_USER" | grep -qw doli; then
            usermod -aG doli "$REAL_USER"
            info "Added '$REAL_USER' to 'doli' group"
            NEEDS_RELOGIN=1
        fi
    fi

    # 3. Create standard directories with correct ownership
    #    Mode 2770: setgid + group-writable so doli group members can run `doli init` without sudo
    install -d -o doli -g doli -m 2770 /var/lib/doli
    install -d -o doli -g doli -m 2770 /var/lib/doli/mainnet
    install -d -o doli -g doli -m 2770 /var/lib/doli/testnet
    install -d -o doli -g doli -m 2770 /var/log/doli

    # 4. Install polkit rule for passwordless service control by doli group
    if [ -d /etc/polkit-1/rules.d ]; then
        cat > /etc/polkit-1/rules.d/50-doli.rules <<'POLKIT'
polkit.addRule(function(action, subject) {
    if (action.id == "org.freedesktop.systemd1.manage-units" &&
        action.lookup("unit").indexOf("doli-") == 0 &&
        subject.isInGroup("doli")) {
        return polkit.Result.YES;
    }
});
POLKIT
        info "Installed polkit rule for doli service management"
    fi

    # 5. Sudoers rule for passwordless binary updates by doli user
    #    The auto-updater runs as 'doli' but binaries live in /usr/bin/ (root-owned).
    #    This allows `sudo cp` on a fixed staging path without password prompt.
    #
    #    ISSUE-174 #7: the staging path lives inside /var/lib/doli (doli:doli, 2770)
    #    instead of /tmp. /tmp is world-writable with predictable names, which
    #    let any local user win a TOCTOU race against the sudo cp. The new
    #    staging dir restricts pre-create access to the doli group, and the
    #    updater opens the file with O_NOFOLLOW to defeat symlink swaps.
    cat > /etc/sudoers.d/doli-update <<'SUDOERS'
# Allow doli user to update doli binaries without password
doli ALL=(root) NOPASSWD: /usr/bin/rm -f /usr/bin/doli-node
doli ALL=(root) NOPASSWD: /usr/bin/rm -f /usr/bin/doli
doli ALL=(root) NOPASSWD: /usr/bin/cp /var/lib/doli/update.bin /usr/bin/doli-node
doli ALL=(root) NOPASSWD: /usr/bin/cp /var/lib/doli/update.bin /usr/bin/doli
SUDOERS
    chmod 440 /etc/sudoers.d/doli-update
    info "Installed sudoers rule for auto-update"

fi

# ---------------------------------------------------------------------------
# Success output
# ---------------------------------------------------------------------------
echo ""
ok "DOLI ${VERSION} installed"
echo ""
printf "  ${BOLD}doli-node${NC}  %s\n" "$(command -v doli-node)"
printf "  ${BOLD}doli${NC}      %s\n" "$(command -v doli)"
echo ""
if [ -n "$STOPPED" ]; then
    printf "  ${BOLD}Restarted${NC}  %s (backups: /usr/bin/doli-node%s, /usr/bin/doli%s)\n" "$STOPPED" "$BK" "$BK"
    echo ""
elif [ -n "$UNITS" ]; then
    printf "  ${BOLD}IMPORTANT:${NC} %s still runs the previous binary. Restart it to finish the upgrade:\n" "$UNITS"
    echo "    sudo systemctl restart ${UNITS}"
    echo ""
else
    echo "  Get started:"
    echo "    doli init                                         # create wallet + keys"
    echo "    sudo doli service install                         # start node as system service"
    echo "    doli chain                                        # check sync progress"
    echo ""
fi

if [ "$SKILL_COUNT" -gt 0 ] 2>/dev/null; then
    printf "  ${BOLD}Agent Skills${NC}  %s (%s skills)\n" "$SKILLS_DIR" "$SKILL_COUNT"
    echo ""
    echo "  AI agents (Claude Code, Cursor, etc.) can use these skills to operate"
    echo "  your DOLI node. Point your agent to:"
    echo "    ${SKILLS_DIR}/SKILLS-INDEX.md"
    echo ""
fi

if [ "${NEEDS_RELOGIN}" = "1" ]; then
    echo "  ${BOLD}IMPORTANT:${NC} Log out and back in for group membership to take effect."
    echo "  Then run: doli init"
    echo ""
fi
