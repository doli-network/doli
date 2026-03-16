#!/usr/bin/env bash
# install-stress-services.sh — Create launchd plists for stress test batches
#
# Creates 10 LaunchAgents, each managing one batch of 50 nodes via stress-batch.sh
#
# Usage:
#   scripts/install-stress-services.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAUNCH_AGENTS_DIR="$HOME/Library/LaunchAgents"
LOG_DIR="$HOME/testnet/logs"

mkdir -p "$LAUNCH_AGENTS_DIR" "$LOG_DIR"

for batch in $(seq 1 10); do
  local_label="network.doli.stress-batch${batch}"
  plist="$LAUNCH_AGENTS_DIR/${local_label}.plist"

  cat > "$plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>${local_label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/bash</string>
        <string>${SCRIPT_DIR}/stress-batch.sh</string>
        <string>start</string>
        <string>${batch}</string>
    </array>
    <key>RunAtLoad</key>
    <false/>
    <key>KeepAlive</key>
    <false/>
    <key>StandardOutPath</key>
    <string>${LOG_DIR}/stress-batch${batch}-svc.log</string>
    <key>StandardErrorPath</key>
    <string>${LOG_DIR}/stress-batch${batch}-svc.log</string>
</dict>
</plist>
EOF
  echo "  Installed batch $batch → $plist"
done

echo ""
echo "Done. Manage with:"
echo "  scripts/stress-batch.sh start <1-10|all>"
echo "  scripts/stress-batch.sh stop <1-10|all>"
echo "  scripts/stress-batch.sh status"
echo ""
echo "Tier layout:"
echo "  Batch 1-2:   Tier 1 (100 nodes → seed)"
echo "  Batch 3-6:   Tier 2 (200 nodes → Tier 1)"
echo "  Batch 7-10:  Tier 3 (200 nodes → Tier 2)"
