#!/bin/sh
# Post-install script for doli .deb and .rpm packages
# Creates system user, directories, polkit rule, and adds installing user to doli group
set -e

# 1. Create doli system user + group
if ! id -u doli >/dev/null 2>&1; then
    useradd --system --home-dir /var/lib/doli --shell /usr/sbin/nologin --create-home doli
fi

# 2. Add the installing user to the doli group (for CLI access to data dirs)
REAL_USER="${SUDO_USER:-$USER}"
if [ -n "$REAL_USER" ] && [ "$REAL_USER" != "root" ]; then
    if ! id -nG "$REAL_USER" 2>/dev/null | grep -qw doli; then
        usermod -aG doli "$REAL_USER" 2>/dev/null || true
    fi
fi

# 3. Create standard directories
install -d -o doli -g doli -m 2770 /var/lib/doli
install -d -o doli -g doli -m 2770 /var/lib/doli/mainnet
install -d -o doli -g doli -m 2770 /var/lib/doli/testnet
install -d -o doli -g doli -m 2770 /var/log/doli

# 4. Install polkit rule for passwordless service control
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
fi

# 5. Sudoers rule for passwordless binary updates by doli user
cat > /etc/sudoers.d/doli-update <<'SUDOERS'
# Allow doli user to update doli binaries without password
doli ALL=(root) NOPASSWD: /usr/bin/rm -f /usr/bin/doli-node
doli ALL=(root) NOPASSWD: /usr/bin/rm -f /usr/bin/doli
doli ALL=(root) NOPASSWD: /usr/bin/cp /tmp/doli-update-binary /usr/bin/doli-node
doli ALL=(root) NOPASSWD: /usr/bin/cp /tmp/doli-update-binary /usr/bin/doli
SUDOERS
chmod 440 /etc/sudoers.d/doli-update

# 6. Symlink to /usr/local/bin for consistency with tarball installs
if [ -f /usr/bin/doli-node ] && [ ! -f /usr/local/bin/doli-node ]; then
    ln -sf /usr/bin/doli-node /usr/local/bin/doli-node
fi
if [ -f /usr/bin/doli ] && [ ! -f /usr/local/bin/doli ]; then
    ln -sf /usr/bin/doli /usr/local/bin/doli
fi
