# Monitoring & Troubleshooting

## Table of Contents
- Health Check
- Common Issues
- Fork Detection
- Sync Problems
- RocksDB Issues
- Clock Drift
- Log Analysis

## Health Check

### Quick Status (All Nodes)

```bash
# Local node
doli chain

# Remote — omegacortex nodes (N1/N2/N6 + archiver)
ssh ilozada@omegacortex.ai "for p in 8545 8546 8547 8548; do \
  echo \"port \$p: \$(curl -s -X POST http://127.0.0.1:\$p \
  -H 'Content-Type: application/json' \
  -d '{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":{},\"id\":1}' \
  | jq -c '.result | {h: .bestHeight, s: .bestSlot, hash: .bestHash[0:16]}')\"; done"
# Port mapping: 8545=N1, 8546=N2, 8547=N6, 8548=Archiver
```

### Network Info

```bash
curl -s -X POST http://127.0.0.1:8545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | jq .
```

Check: `peerCount > 0`, `syncing: false` when healthy.

### Process Check

```bash
# Local
pgrep -la doli-node

# Remote nodes
ssh ilozada@omegacortex.ai "pgrep -la doli-node"
# N4/N5: direct from Mac (omegacortex cannot reach them!)
ssh -p 50790 ilozada@72.60.115.209 'sudo pgrep -la doli-node'  # N4
ssh -p 50790 ilozada@72.60.70.166 'sudo pgrep -la doli-node'   # N5
```

## Common Issues

### Node Not Producing Blocks

1. Check producer status: `doli producer status`
2. Verify process running with `--producer` flag
3. Check slot assignment — may not be your turn
4. Verify clock sync (see Clock Drift below)
5. Check peer count > 0

### No Peers Connecting

1. Check firewall: P2P port (30303) must be open
2. Check bootstrap: `--bootstrap /ip4/72.60.228.233/tcp/30303`
3. Verify DNS seeds resolve: `dig seed1.doli.network`
4. Check if behind NAT — may need relay mode

### Insufficient Balance for Registration

- Bond unit is network-specific (10 DOLI mainnet, 1 DOLI devnet)
- Need `bond_unit * count + fee` spendable balance
- Coinbase outputs need 100 confirmations before spendable
- Check immature balance: `doli balance`

## Fork Detection

**Symptom**: Nodes at different heights or different hashes at same height.

```bash
# Compare all nodes (run twice 15s apart)
for p in 8545 8546 8547; do
  curl -s -X POST http://127.0.0.1:$p \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' \
    | jq -c '.result | {h: .bestHeight, hash: .bestHash[0:16]}'
done
```

**Fix**: Wipe the forked node and resync:

```bash
# Stop forked node via systemd (NEVER use kill/pkill on production nodes)
sudo systemctl stop doli-mainnet-nodeN

# Delete state (keep keys!)
rm -f chain_state.bin producers.bin utxo.bin
rm -rf blocks/ signed_slots.db/

# Restart — will resync from peers
sudo systemctl start doli-mainnet-nodeN
```

## Sync Problems

### Stuck at Height 0

- No peers. Check bootstrap and firewall.
- Wrong chainspec. Verify `genesis.timestamp` matches network.

### Stuck at Specific Height

- Body download stall. Restart node.
- Peer has incomplete chain. More peers needed.
- Check logs for `[SYNC]` errors.

### Snap Sync Not Triggering

Snap sync requires:
- Node > 1000 blocks behind
- 3+ connected peers
- State root quorum from 2+ peers

If <3 peers, falls back to header-first sync.

## RocksDB Issues

### LOCK File

```
Error: IO error: lock /path/blocks/LOCK: Resource temporarily unavailable
```

Another process has the database open. Kill the old process:

```bash
pgrep -la doli-node
kill <old_pid>
```

Do NOT just delete the LOCK file — data corruption risk.

### Corruption

```bash
# Recovery mode rebuilds state from blocks
doli-node --network mainnet recover --yes
```

If recovery fails, wipe and resync.

## Clock Drift

Max allowed: 1 second (`MAX_DRIFT=1`), 200ms precision (`MAX_DRIFT_MS=200`).

### Check Clock

```bash
# Compare local time vs NTP
date -u
ntpdate -q pool.ntp.org 2>/dev/null || chronyc tracking 2>/dev/null
```

### Fix Clock

```bash
# Linux (systemd-timesyncd)
sudo timedatectl set-ntp true
sudo systemctl restart systemd-timesyncd

# Linux (chrony)
sudo chronyc makestep

# macOS
sudo sntp -sS pool.ntp.org
```

Blocks with timestamps drifting >1s from expected slot time are rejected.

## Log Analysis

### Key Log Prefixes

| Prefix | Meaning |
|--------|---------|
| `[SYNC]` | Sync operations |
| `[SNAP_SYNC]` | Snap sync state download |
| `[BLOCK]` | Block production/validation |
| `[GOSSIP]` | Gossipsub messages |
| `[EQUIVOCATION]` | Double-production detected |
| `[REORG]` | Chain reorganization |
| `[VDF]` | VDF proof computation |

### Filter Important Logs

```bash
# Errors and warnings only
grep -iE "error|warn|fail|panic" /tmp/node.log | tail -30

# Block production
grep "\[BLOCK\]" /tmp/node.log | tail -20

# Sync progress
grep "\[SYNC\]" /tmp/node.log | tail -20
```
