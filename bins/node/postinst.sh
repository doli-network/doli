#!/bin/sh
# Post-install script for doli .deb and .rpm packages
# Creates system user, directories, and polkit rule — same as install.sh
set -e

# 1. Create doli system user + group
if ! id -u doli >/dev/null 2>&1; then
    useradd --system --home-dir /var/lib/doli --shell /usr/sbin/nologin --create-home doli
fi

# 2. Create standard directories
install -d -o doli -g doli -m 0750 /var/lib/doli
install -d -o doli -g doli -m 0750 /var/lib/doli/mainnet
install -d -o doli -g doli -m 0750 /var/lib/doli/testnet
install -d -o doli -g doli -m 0750 /var/log/doli

# 3. Install polkit rule for passwordless service control
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

# 4. Symlink to /usr/local/bin for consistency with tarball installs
if [ -f /usr/bin/doli-node ] && [ ! -f /usr/local/bin/doli-node ]; then
    ln -sf /usr/bin/doli-node /usr/local/bin/doli-node
fi
if [ -f /usr/bin/doli ] && [ ! -f /usr/local/bin/doli ]; then
    ln -sf /usr/bin/doli /usr/local/bin/doli
fi
