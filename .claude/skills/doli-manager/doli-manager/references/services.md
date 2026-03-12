# System Service Creation

## Table of Contents
- Auto-Detection
- systemd (Linux)
- launchd (macOS)
- Service Management

## Auto-Detection

Detect OS and create the appropriate service:

```bash
OS=$(uname -s)
if [ "$OS" = "Linux" ]; then
    echo "Creating systemd service..."
    # See systemd section below
elif [ "$OS" = "Darwin" ]; then
    echo "Creating launchd service..."
    # See launchd section below
fi
```

Or use the bundled script: `scripts/create-service.sh`

## systemd (Linux)

### Producer Node Service

Create `/etc/systemd/system/doli-mainnet.service`:

```ini
[Unit]
Description=DOLI Mainnet Producer Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=doli
Group=doli
ExecStart=/opt/doli/target/release/doli-node \
  --data-dir /home/doli/.doli/mainnet/data run \
  --producer --producer-key /home/doli/.doli/mainnet/keys/producer.json \
  --chainspec /home/doli/.doli/mainnet/chainspec.json \
  --no-auto-update --yes --force-start
Restart=on-failure
RestartSec=10
LimitNOFILE=65536

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/home/doli/.doli
ProtectHome=read-only

[Install]
WantedBy=multi-user.target
```

### Non-Producer Node Service

Same but remove `--producer`, `--producer-key`, and `--force-start`.

### Multi-Node on Same Host (port offset)

For N2 on the same host as N1, add port flags:

```ini
ExecStart=/opt/doli/target/release/doli-node \
  --data-dir /home/doli/.doli/mainnet/node2/data run \
  --producer --producer-key /home/doli/.doli/mainnet/keys/producer_2.json \
  --p2p-port 30301 --rpc-port 8501 --metrics-port 9001 \
  --bootstrap /ip4/127.0.0.1/tcp/30300 \
  --no-auto-update --yes --force-start
```

### Testnet Service

```ini
ExecStart=/opt/doli/target/release/doli-node \
  --network testnet run --yes
```

### Enable & Start

```bash
sudo systemctl daemon-reload
sudo systemctl enable doli-mainnet
sudo systemctl start doli-mainnet

# Check status
sudo systemctl status doli-mainnet

# View logs
journalctl -u doli-mainnet -f
journalctl -u doli-mainnet --since "10 min ago"
```

### Dedicated User (recommended)

```bash
sudo useradd -r -m -s /bin/bash doli
sudo mkdir -p /home/doli/.doli/mainnet/keys
sudo chown -R doli:doli /home/doli/.doli
```

## launchd (macOS)

### Producer Node Service

Create `~/Library/LaunchAgents/network.doli.mainnet.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>network.doli.mainnet</string>

    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/doli-node</string>
        <string>--data-dir</string>
        <string>/Users/USERNAME/.doli/mainnet/data</string>
        <string>run</string>
        <string>--producer</string>
        <string>--producer-key</string>
        <string>/Users/USERNAME/.doli/mainnet/keys/producer.json</string>
        <string>--chainspec</string>
        <string>/Users/USERNAME/.doli/mainnet/chainspec.json</string>
        <string>--no-auto-update</string>
        <string>--yes</string>
        <string>--force-start</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>

    <key>StandardOutPath</key>
    <string>/Users/USERNAME/.doli/mainnet/node.log</string>

    <key>StandardErrorPath</key>
    <string>/Users/USERNAME/.doli/mainnet/node.err</string>

    <key>ThrottleInterval</key>
    <integer>10</integer>
</dict>
</plist>
```

Replace `USERNAME` with actual username.

### Non-Producer

Remove `--producer`, `--producer-key`, `--force-start` from ProgramArguments.

### Install & Start

```bash
# Copy binary
sudo cp target/release/doli-node /usr/local/bin/

# Load service
launchctl load ~/Library/LaunchAgents/network.doli.mainnet.plist

# Start
launchctl start network.doli.mainnet

# Check status
launchctl list | grep doli

# Stop
launchctl stop network.doli.mainnet

# Unload (disable)
launchctl unload ~/Library/LaunchAgents/network.doli.mainnet.plist
```

### View Logs

```bash
tail -f ~/.doli/mainnet/node.log
tail -f ~/.doli/mainnet/node.err
```

## Service Management Summary

| Action | systemd (Linux) | launchd (macOS) |
|--------|----------------|-----------------|
| Start | `sudo systemctl start doli-mainnet` | `launchctl start network.doli.mainnet` |
| Stop | `sudo systemctl stop doli-mainnet` | `launchctl stop network.doli.mainnet` |
| Restart | `sudo systemctl restart doli-mainnet` | stop + start |
| Status | `sudo systemctl status doli-mainnet` | `launchctl list \| grep doli` |
| Logs | `journalctl -u doli-mainnet -f` | `tail -f ~/.doli/mainnet/node.log` |
| Enable | `sudo systemctl enable doli-mainnet` | `RunAtLoad = true` in plist |
| Disable | `sudo systemctl disable doli-mainnet` | `launchctl unload <plist>` |
