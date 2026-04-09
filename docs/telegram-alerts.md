# Telegram Alerts for the DOLI Seed Guardian

The guardian toolkit can push alerts to a Telegram chat whenever it detects a
network issue — a fork, an offline node, or a lagging node. This gives
operators a real-time channel that does not depend on watching terminal output.

Alerts fire on **state transitions only**: one message when an issue starts,
one `[RECOVERED]` message when it clears. There are no repeating reminders
while an issue persists — the entry message is the page, not the first of a
stream.

## 1. Create the Telegram bot

1. Open Telegram and message **@BotFather**.
2. Send `/newbot` and follow the prompts. Pick a display name and a unique
   username ending in `bot` (e.g. `doli_guardian_bot`).
3. BotFather replies with an **HTTP API token** that looks like
   `123456789:AAH...xyz`. Save it. This is your `DOLI_TELEGRAM_BOT_TOKEN`.
4. (Recommended) Disable group privacy so the bot can read messages in groups:
   `/setprivacy` → pick your bot → `Disable`. You do not strictly need this
   for alerting (bots can always *send* messages) but it lets you add debug
   commands later.

## 2. Get the target chat ID

The chat ID is a numeric identifier. Users are positive, groups/supergroups
are negative.

**For a direct chat with yourself:**
1. Send any message to your bot in Telegram.
2. Open `https://api.telegram.org/bot<TOKEN>/getUpdates` in a browser.
3. Look for `"chat":{"id":<NUMBER>,...}`. That number is your chat ID.

**For a group:**
1. Add the bot to the group.
2. Send any message in the group.
3. Call `getUpdates` as above. The group chat ID is negative
   (e.g. `-1001234567890` for supergroups).

Save this as `DOLI_TELEGRAM_CHAT_ID`.

## 3. Test the helper

Before wiring anything up permanently, verify the helper can reach Telegram:

```bash
export DOLI_TELEGRAM_BOT_TOKEN="123456789:AAH...xyz"
export DOLI_TELEGRAM_CHAT_ID="123456789"

scripts/telegram-alert.sh \
    --severity info \
    --title "DOLI guardian test" \
    --body  "If you see this, the bot is wired up correctly."
```

You should see the message in your chat within a second. If the helper prints
`telegram-alert: send FAILED (http=401)` your token is wrong; `http=400` with
`chat not found` means the bot has never seen a message in that chat (it needs
one inbound message to learn the chat exists).

## 4. Run fork-monitor with alerts

The env vars are the only configuration fork-monitor needs. Set them and run:

```bash
export DOLI_TELEGRAM_BOT_TOKEN="123456789:AAH...xyz"
export DOLI_TELEGRAM_CHAT_ID="123456789"

# One-shot check (useful for cron):
scripts/fork-monitor.sh --testnet

# Continuous monitoring on a seed server (every 30s):
scripts/fork-monitor.sh --testnet --loop 30
```

State is persisted to `~/.doli/monitor-state/fork-monitor-<mode>.json` so the
monitor can detect transitions across restarts. Set `DOLI_MONITOR_STATE_DIR`
to override the location.

## 5. Deploy as a systemd service on a seed server

The recommended deployment is a long-lived systemd unit on a seed server
(`ai1`, `ai2`, or `ai3`) where the process is always running and has outbound
HTTPS to `api.telegram.org`.

Create `/etc/systemd/system/doli-fork-monitor.service`:

```ini
[Unit]
Description=DOLI fork/health monitor with Telegram alerts
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=doli
Group=doli
WorkingDirectory=/home/doli/doli
Environment=DOLI_TELEGRAM_BOT_TOKEN=123456789:AAH...xyz
Environment=DOLI_TELEGRAM_CHAT_ID=-1001234567890
Environment=DOLI_BEHIND_THRESHOLD=10
Environment=DOLI_MONITOR_STATE_DIR=/var/lib/doli-fork-monitor
StateDirectory=doli-fork-monitor
ExecStart=/home/doli/doli/scripts/fork-monitor.sh --testnet --loop 30
Restart=on-failure
RestartSec=10s
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

Replace the network mode (`--testnet` / `--devnet`) to match the fleet you are
monitoring. For a production mainnet deployment, use whichever mode your
fork-monitor is compiled for (the mainnet port range may differ — update the
script's port range if needed).

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now doli-fork-monitor
sudo systemctl status doli-fork-monitor
journalctl -u doli-fork-monitor -f
```

**Security note**: the bot token is a secret. Do not commit it to any repo.
The systemd unit above reads it from `Environment=` lines; prefer
`EnvironmentFile=/etc/default/doli-fork-monitor` (file mode `0600`, owned by
the service user) if the unit itself is world-readable.

## 6. What triggers an alert

| Event | Severity | Trigger |
|---|---|---|
| `FORK DETECTED` | critical | First poll where 2+ reachable nodes disagree on `bestHash`. |
| `FORK STATE CHANGED` | critical | A fork was already active, but the group set changed (new nodes joined a fork, or a new fork group appeared). |
| `FORK RECOVERED` | recovery | A previously-forked network is now showing a single chain tip across all reachable nodes. |
| `NODE OFFLINE` | warning | One or more nodes that were previously reachable are now unreachable via RPC. |
| `NODE RECOVERED` | recovery | An offline node is reachable again. |
| `NODE BEHIND` | warning | A node's `bestHeight` lags `max_fleet_height` by >= `DOLI_BEHIND_THRESHOLD` (default 10 blocks). Offline nodes are excluded — they are already reported via `NODE OFFLINE`. |
| `NODE CAUGHT UP` | recovery | A previously-behind node has caught up to within the threshold. |

## 7. Tuning

- **`DOLI_BEHIND_THRESHOLD`**: default 10 blocks. Too low = false positives
  during normal gossip latency; too high = real stuck nodes go unreported.
  At 10-second slots, 10 blocks ≈ 100 seconds of lag.
- **Poll interval (`--loop SECS`)**: default 30s. Shorter = faster detection
  and faster recovery messages; longer = less noise from transient blips.
  30s is a good baseline.
- **State reset**: `rm ~/.doli/monitor-state/fork-monitor-<mode>.json` to
  clear history. The next poll becomes the new baseline — any current issue
  will fire an alert on the first poll after the reset.

## 8. Redundancy

Running fork-monitor on a single host creates a single point of alerting
failure. If that host dies, you lose visibility. Two options:

1. **Run on two seeds with independent state directories.** Both will alert
   independently — you get two messages per event, but if either host is
   healthy you stay informed. Acceptable for low-volume alerts like these.
2. **Run on two seeds sharing the state file via NFS or a cron-synced copy.**
   Avoids duplicate alerts but requires locking discipline. Not recommended
   unless you already have a shared state store.

Pick (1) for simplicity. Telegram messages are cheap; operator attention is
not.

## 9. Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `telegram-alert: DOLI_TELEGRAM_BOT_TOKEN ... not set — skipping` | env vars missing | Export them in your shell or in the systemd unit |
| `send FAILED (http=401) Unauthorized` | wrong token | Re-check with `@BotFather` → `/token` |
| `send FAILED (http=400) chat not found` | bot has never seen a message in that chat | Send one message to the bot (or in the group) first |
| `send FAILED (http=403) bot was blocked by the user` | the user blocked the bot | Unblock it and send a message |
| Alerts fire on every poll (no dedup) | state file is unwritable | Check `DOLI_MONITOR_STATE_DIR` permissions |
| No alert on fork detected | state file already reflects the fork (prior run cached it) | Alerts only fire on transitions; a fork present at both polls will only page once, when it was first seen |

## 10. Future extensions

The `telegram-alert.sh` helper is generic — any guardian script can call it.
Candidates for future integration:

- `emergency-halt.sh` / `emergency-resume.sh`: page when production is paused
  or resumed (high-value operational events)
- `seed-backup.sh`: page on backup failures (silent backups are dangerous)
- `node-heal.sh`: page when a heal runs and succeeds/fails
- Chain-stall detection: page when `max_fleet_height` has not advanced for >N
  seconds (separate from fork detection)

None of these are implemented yet. Add them when the operational need is real,
not preemptively.
