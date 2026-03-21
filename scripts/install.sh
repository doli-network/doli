#!/bin/sh
set -e

REPO="e-weil/doli"
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

if [ "$OS" = "Darwin" ] && [ "$ARCH_LABEL" = "aarch64" ]; then
    FILE="doli-${VERSION}-${TARGET}.pkg"
    METHOD="pkg"
elif [ "$OS" = "Darwin" ]; then
    FILE="doli-${VERSION}-${TARGET}.tar.gz"
    METHOD="tarball"
elif command -v dpkg >/dev/null 2>&1; then
    FILE="doli-${VERSION}-${TARGET}.deb"
    METHOD="deb"
elif command -v rpm >/dev/null 2>&1; then
    FILE="doli-${VERSION}-${TARGET}.rpm"
    METHOD="rpm"
else
    FILE="doli-${VERSION}-${TARGET}.tar.gz"
    METHOD="tarball"
fi

URL="${GITHUB}/releases/download/${VERSION}/${FILE}"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

info "Downloading ${FILE}..."
$FETCH_OUT "${TMPDIR}/${FILE}" "$URL" || err "Download failed. Check ${GITHUB}/releases/tag/${VERSION}"

case "$METHOD" in
    pkg)
        info "Installing .pkg (requires sudo)..."
        sudo installer -pkg "${TMPDIR}/${FILE}" -target /
        ;;
    deb)
        info "Installing .deb (requires sudo)..."
        sudo dpkg -i "${TMPDIR}/${FILE}"
        ;;
    rpm)
        info "Installing .rpm (requires sudo)..."
        sudo rpm -i "${TMPDIR}/${FILE}"
        ;;
    tarball)
        info "Extracting..."
        tar -xzf "${TMPDIR}/${FILE}" -C "$TMPDIR"
        DIR=$(find "$TMPDIR" -maxdepth 1 -type d -name "doli-*" | head -1)
        [ -z "$DIR" ] && err "Failed to extract archive"
        info "Installing to /usr/local/bin (requires sudo)..."
        sudo install -m 755 "${DIR}/doli-node" /usr/local/bin/doli-node
        sudo install -m 755 "${DIR}/doli"      /usr/local/bin/doli
        ;;
esac

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
    install -d -o doli -g doli -m 0750 /var/lib/doli
    install -d -o doli -g doli -m 0750 /var/lib/doli/mainnet
    install -d -o doli -g doli -m 0750 /var/lib/doli/testnet
    install -d -o doli -g doli -m 0750 /var/log/doli

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
echo "  Get started:"
echo "    doli init                                         # create wallet + keys"
echo "    doli-node run --yes                               # sync to chain tip"
echo ""

if [ "${NEEDS_RELOGIN}" = "1" ]; then
    echo "  ${BOLD}IMPORTANT:${NC} Log out and back in for group membership to take effect."
    echo "  Then run: doli init"
    echo ""
fi
