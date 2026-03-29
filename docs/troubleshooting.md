# troubleshooting.md - Common Issues and Solutions

This guide helps diagnose and resolve common issues with DOLI nodes and wallets.

---

## 1. Node Issues

### 1.1. Node Won't Start

**Symptom:** Node exits immediately or fails to start.

| Possible Cause | Solution |
|----------------|----------|
| Port already in use | Check for existing process: `lsof -i :30300` |
| Corrupt database | Remove and resync: `rm -rf <DATA_DIR>/data/` (see note below) |
| Missing dependencies | Reinstall via `nix develop` or install manually |

**Data directory locations:**
- Linux: `/var/lib/doli/{network}/`
- macOS: `~/Library/Application Support/doli/{network}/`
- Legacy fallback: `~/.doli/{network}/`
- Override: set `DOLI_DATA_DIR` env var or use `--data-dir` flag
| Insufficient permissions | Check data directory permissions |
| Out of disk space | Free up space or move data directory |

**Check logs:**
```bash
# If using systemd
sudo journalctl -u doli-node -n 100

# If running directly
./target/release/doli-node run 2>&1 | tee node.log
```

---

### 1.2. Node Crashes on Restart (RocksDB LOCK File)

**Symptom:** Node crashes with `Trace/BPT trap: 5` (SIGTRAP) immediately after restarting a previously killed node. Common on macOS.

**Cause:** When a node is killed (SIGTERM/SIGKILL), RocksDB may leave stale `LOCK` files in the data directory. The new process crashes when RocksDB tries to open the database and finds the lock held.

**Solution:**
```bash
# Remove stale LOCK files before restart
rm -f ~/.doli/<NETWORK>/data/node*/blocks/LOCK
rm -f ~/.doli/<NETWORK>/data/node*/signed_slots.db/LOCK

# Then restart the node normally
```

**For devnet nodes:**
```bash
# Remove LOCK files for a specific node (e.g., node 3)
DD=~/.doli/devnet
rm -f $DD/data/node3/blocks/LOCK
rm -f $DD/data/node3/signed_slots.db/LOCK
```

**Prevention:** Graceful shutdown via `doli-node devnet stop` avoids stale LOCK files. This only occurs after forced kills.

---

### 1.3. Debug Build (Wrong Binary)

**Symptom:** Node syncs slowly, VDF timeouts, block production misses slots, binary is ~17MB instead of ~8MB.

**Cause:** Built with `cargo build` instead of `cargo build --release`. Debug builds are ~10x slower for VDF computation.

**Diagnosis:**
```bash
# Check binary size — release should be ~8-9MB, debug is ~15-20MB
ls -lh $(which doli-node || echo ./target/release/doli-node)
```

**Fix:**
```bash
cargo build --release
# Replace old binary
sudo cp target/release/doli-node /usr/local/bin/
sudo systemctl restart doli-node
```

> `--release` is mandatory for production. Debug builds will cause fork divergence because VDF proofs take too long to compute within the 10-second slot window.

---

### 1.4. Node Not Syncing

**Symptom:** Chain height not increasing, stuck at old block.

| Possible Cause | Solution |
|----------------|----------|
| No peers connected | Check firewall, add bootstrap nodes |
| Network mismatch | Verify correct network flag |
| Clock skew | Sync system clock with NTP |
| Banned by peers | Check peer scoring, restart node |

**Diagnostics:**
```bash
# Check peer count
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}'

# Check sync status
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
```

**Add bootstrap nodes:**
```bash
./target/release/doli-node run \
    --bootstrap /dns4/seed1.doli.network/tcp/30300 \
    --bootstrap /dns4/seed2.doli.network/tcp/30300
```

---

### 1.5. Low Peer Count

**Symptom:** Fewer than 5 peers connected.

| Possible Cause | Solution |
|----------------|----------|
| Firewall blocking P2P port | Open port 30300 (or network-specific) |
| NAT not traversable | Enable port forwarding on router |
| DHT disabled | Remove `--no-dht` flag if present |
| Bootstrap nodes unreachable | Try alternative bootstrap nodes |

**Check firewall:**
```bash
# UFW
sudo ufw status

# iptables
sudo iptables -L -n | grep 30300
```

**Test port accessibility:**
```bash
# From another machine
nc -zv your-node-ip 30300
```

---

### 1.6. High Memory Usage

**Symptom:** Node consuming excessive RAM.

| Possible Cause | Solution |
|----------------|----------|
| Large mempool | Restart node (mempool clears on restart) |
| Many peers | Set `DOLI_MAX_PEERS=25` env var |
| Memory leak | Update to latest version |
| Peer eviction churn | Set `DOLI_EVICTION_GRACE_SECS=30` (default: 30s) |

**Reduce resource usage with env vars (in .env or shell):**
```bash
DOLI_MAX_PEERS=25 doli-node run
# Or set in .env file in data directory
```

---

### 1.7. Database Corruption

**Symptom:** Node crashes with database errors.

**Solution:**
```bash
# Stop node
sudo systemctl stop doli-node

# Backup existing data (optional)
mv ~/.doli/mainnet/db ~/.doli/mainnet/db.bak

# Restart node (will resync)
sudo systemctl start doli-node
```

---

### 1.8. Node Stuck at Height 0 (Corrupt State with Intact Blocks)

**Symptom:** Node reports `bestHeight: 0` via RPC but logs show `"Block already in store, skipping apply"` and `"Sync chain mismatch: first pending header doesn't build on local tip"`. The node downloads blocks from peers, finds them already stored, skips them, clears the sync queue, and loops forever.

**Cause:** The height index (RocksDB) is corrupt or was reset, but the block data (headers + bodies) is intact. The node's state thinks it's at genesis, sync downloads blocks but can't apply them because they already exist in the block store, and the height index can't map heights to blocks because it's empty.

**Diagnosis:**
```bash
# Check if node reports height 0 but has blocks in the store
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq '.result.bestHeight'
# Returns 0

# Check block store has data (RocksDB SST files exist)
ls -la /path/to/data/blocks/*.sst
# SST files present = blocks exist but index is broken
```

**Fix — `reindex` → `recover` pipeline (no data loss):**
```bash
# 1. Stop the node
sudo systemctl stop doli-testnet-nt6

# 2. Rebuild the height index from raw block headers
#    Scans ALL headers by hash, finds the chain tip, walks backwards
#    via prev_hash to assign correct heights. Does NOT touch block data.
doli-node --network testnet --data-dir /testnet/nt6/data reindex

# 3. Rebuild UTXO set, producer registry, and chain state from blocks
#    Replays every block in order using the now-correct height index.
doli-node --network testnet --data-dir /testnet/nt6/data recover --yes

# 4. Restart the node — it will sync remaining blocks from peers
sudo systemctl start doli-testnet-nt6
```

**Why not just wipe?** The `reindex → recover` pipeline preserves all existing block data. Wiping forces a full resync (or snap sync with gaps). This pipeline rebuilds the index and state from what's already on disk — faster and no data loss.

**Common cause:** Dirty shutdown during binary upgrade (stop + copy + start) when the process didn't flush the height index to disk before termination.

---

### 1.9. Missing Historical Blocks (Snap Sync Gaps)

**Symptom:** Node is synced and producing, but `getBlockByHeight` returns "Block not found" for early heights. The explorer shows `GAP 1→N` in the chain column.

**Cause:** Node joined via snap sync, which downloads only recent state and skips historical blocks. The node works correctly but lacks blocks before the snap sync point.

**Fix — Hot backfill (no restart needed):**
```bash
# Backfill from any seed node's RPC — runs in the background while the node operates
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"backfillFromPeer","params":{"rpc_url":"http://127.0.0.1:SEED_PORT"},"id":1}'

# Monitor progress
curl -s -X POST http://127.0.0.1:PORT -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"backfillStatus","params":{},"id":1}'
```

**Fix — Offline backfill (requires restart):**
```bash
sudo systemctl stop doli-node
doli-node --network mainnet restore --from-rpc http://seed2.doli.network:8500 --backfill --yes
sudo systemctl start doli-node
```

See [disaster-recovery.md](./disaster-recovery.md) for all recovery methods.

---

## 2. Producer Issues

### 2.1. Not Producing Blocks

**Symptom:** Producer is registered but not creating blocks.

| Possible Cause | Solution |
|----------------|----------|
| Node not synced | Wait for sync to complete |
| Not in active set | Wait until next epoch boundary |
| Wrong key file | Verify `--producer-key` path |
| VDF computation slow | Check CPU performance |
| Already produced for slot | Only one block per slot allowed |

**Check producer status:**
```bash
./target/release/doli producer status
```

**Verify in active set:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getProducers","params":{"active_only":true},"id":1}'
```

---

### 2.2. VDF Computation Too Slow

**Symptom:** VDF takes longer than 8 seconds, missing slots.

| Possible Cause | Solution |
|----------------|----------|
| CPU throttling | Disable power saving modes |
| Shared hosting | Use dedicated hardware |
| Background processes | Reduce CPU load |
| Slow CPU | Upgrade hardware |

**Check VDF timing:**
```bash
grep "VDF computed" /var/log/doli-node.log | tail -20
# Times should be < 8000ms
```

**Disable CPU throttling (Linux):**
```bash
# Check current governor
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor

# Set to performance
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
```

---

### 2.3. Registration Failed

**Symptom:** Producer registration transaction rejected.

| Possible Cause | Solution |
|----------------|----------|
| Insufficient balance | Ensure bond amount + fees available |
| Already registered | Check if pubkey already in producer set |
| VDF proof invalid | Retry registration |
| Wrong network | Verify network selection |

**Check balance:**
```bash
doli balance
```

---

### 2.4. Slashing Warning

**Symptom:** Log shows slashing-related warnings.

**CRITICAL:** Never run two producer instances with the same key!

**If you see warnings:**
1. Immediately stop all producer instances
2. Verify only one instance exists
3. Check for equivocation proofs in logs
4. If slashed, bond is permanently lost

**Check for duplicate processes:**
```bash
ps aux | grep doli-node
```

---

## 3. Wallet Issues

### 3.1. Transaction Not Confirming

**Symptom:** Sent transaction, not appearing in blocks.

| Possible Cause | Solution |
|----------------|----------|
| Fee too low | Resend with higher fee |
| Node not synced | Wait for sync |
| Invalid transaction | Check error in mempool |
| Network congestion | Wait or increase fee |

**Check mempool:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getMempoolInfo","params":{},"id":1}'
```

---

### 3.2. Balance Shows Zero

**Symptom:** Wallet shows 0 balance but funds were sent.

| Possible Cause | Solution |
|----------------|----------|
| Wrong address | Verify address is correct |
| Node not synced | Wait for sync completion |
| Wrong network | Check network selection |
| UTXOs not indexed | Rescan (restart node) |

**Verify address:**
```bash
doli addresses
```

**Check via RPC:**
```bash
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBalance","params":{"address":"YOUR_ADDRESS"},"id":1}'
```

---

### 3.3. Cannot Connect to Node

**Symptom:** CLI cannot reach RPC endpoint.

| Possible Cause | Solution |
|----------------|----------|
| Node not running | Start the node |
| Wrong RPC port | Check `--rpc` flag |
| RPC disabled | Enable in config |
| Firewall blocking | Check localhost access |

**Test RPC:**
```bash
curl http://127.0.0.1:8500
# Should return JSON-RPC error (method not found), not connection refused
```

---

### 3.4. Lost Wallet File

**Symptom:** Cannot access funds, wallet file deleted.

**If you have backup:**
```bash
cp ~/backup/wallet.json ~/.doli/wallet.json
```

**If no backup:**
- Funds are permanently lost
- There is no recovery mechanism
- This is by design (immutability)

**Prevention:**
- Always backup `~/.doli/wallet.json`
- Store backups in multiple secure locations
- Consider hardware wallet integration (future)

---

## 4. Network Issues

### 4.1. Connection Timeouts

**Symptom:** Frequent peer disconnections, sync failures.

| Possible Cause | Solution |
|----------------|----------|
| Network instability | Check internet connection |
| Firewall issues | Verify P2P port open |
| DNS problems | Try IP-based bootstrap |
| ISP blocking | Use VPN |

**Test network:**
```bash
# Ping bootstrap nodes
ping seed1.doli.network

# Test P2P port connectivity
nc -zv seed1.doli.network 30300
```

---

### 4.2. Chain Fork

**Symptom:** Your chain diverges from network consensus.

| Possible Cause | Solution |
|----------------|----------|
| Isolated from network | Add more peers |
| Software bug | Update to latest version |
| Consensus-critical rolling deployment | See below |
| Intentional attack | Verify with multiple sources |

**Check against known block:**
```bash
# Compare your block at height X with explorer
curl -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBlockByHeight","params":{"height":12345},"id":1}'
```

**Force resync if needed:**
```bash
sudo systemctl stop doli-node
# Remove chain data (platform-specific path, or use DOLI_DATA_DIR):
#   Linux:  /var/lib/doli/mainnet/data/
#   macOS:  ~/Library/Application Support/doli/mainnet/data/
#   Legacy: ~/.doli/mainnet/data/
rm -rf <DATA_DIR>/data/
sudo systemctl start doli-node
```

### 4.3. Consensus-Critical Fork (Network-Wide Split)

**Symptom:** Multiple groups of nodes producing blocks but heights/hashes diverge across groups. Nodes stay connected but silently reject each other's blocks.

**Cause:** A consensus-critical change (scheduling, validation, rewards) was deployed with a rolling restart, creating nodes running incompatible binary versions simultaneously.

**This is NOT recoverable via normal reorg.** Both sides reject each other's blocks as `InvalidProducer`. The fork grows past `MAX_REORG_DEPTH` within minutes.

**Resolution:** Full genesis reset required. See `docs/infrastructure.md` section "Consensus-Critical Deployment" for the correct procedure.

**Prevention:** NEVER use rolling restarts for consensus-critical changes. Always use simultaneous deployment (stop ALL, deploy ALL, start ALL).

> See `docs/legacy/bugs/REPORT_HA_FAILURE.md` for the full incident analysis.

### 4.4. Fork Detection & Seed Guardian Recovery

**Symptom:** Fork monitor detects multiple chain tips, or the status dashboard shows nodes on different hashes.

**Use the Seed Guardian system** to contain damage and recover:

**Step 1 — Detect** (continuous monitoring):
```bash
scripts/fork-monitor.sh --testnet --loop 30
```

**Step 2 — Halt** (stop all producers, seeds keep running):
```bash
scripts/emergency-halt.sh --testnet
```

**Step 3 — Backup** (checkpoint seed DB before any fix):
```bash
scripts/seed-backup.sh --testnet
```

**Step 4 — Investigate** the root cause. Check logs, compare chain tips:
```bash
curl -sf http://127.0.0.1:8500 -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getGuardianStatus","params":[],"id":1}'
```

**Step 5 — Fix & recover**. Deploy fix, wipe producer data, snap-sync from seeds:
```bash
# On each producer:
# 1. Stop node
# 2. rm -rf data_dir/state_db data_dir/blocks
# 3. Deploy fixed binary
# 4. Start node (will snap-sync from seed)
```

**Step 6 — Resume**:
```bash
scripts/emergency-resume.sh --testnet
```

**Key principle:** Seeds are the canonical chain authority. Producers are disposable — they can always be wiped and snap-synced from seeds. Never wipe seed data without a checkpoint.

**Auto-checkpoint (recommended for seeds):** Start seeds with `--auto-checkpoint 100` to create automatic RocksDB snapshots every 100 blocks. Keeps last 5, rotates oldest. Each checkpoint includes a `health.json` with peer consensus data.

**Finding the last healthy state after a regression:**
```bash
# 1. Query the seed for the last known-good checkpoint
curl -sf http://127.0.0.1:8500 -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getGuardianStatus","params":[],"id":1}' | jq '.result.last_healthy_checkpoint'
# Returns e.g. "h24818-1743279000" — all peers agreed at that height

# 2. Or scan health.json files manually
for d in data_dir/checkpoints/h*; do
  echo -n "$d: "; cat "$d/health.json" | python3 -c "import sys,json; h=json.load(sys.stdin); print(f'healthy={h[\"healthy\"]} peers={h[\"peer_count\"]} agreeing={h[\"peers_agreeing\"]}')"
done
```

**Restoring from a healthy checkpoint:**
```bash
# 1. Stop the seed
# 2. Identify the last healthy checkpoint:
#    getGuardianStatus → last_healthy_checkpoint field
#    OR: grep '"healthy": true' data_dir/checkpoints/*/health.json
# 3. Restore:
rm -rf data_dir/state_db data_dir/blocks
cp -r data_dir/checkpoints/h{HEIGHT}-{TS}/state_db data_dir/state_db
cp -r data_dir/checkpoints/h{HEIGHT}-{TS}/blocks data_dir/blocks
# 4. Restart the seed — it resumes from the healthy checkpoint
```

**RPC methods:** `pauseProduction`, `resumeProduction`, `createCheckpoint`, `getGuardianStatus`

---

## 5. Update Issues

### 5.1. Auto-Update Failed

**Symptom:** Update notification but node not updating.

| Possible Cause | Solution |
|----------------|----------|
| Update vetoed | Check veto status |
| Download failed | Retry after network fix |
| Permission denied | Check binary path permissions |
| Disk full | Free up space |

**Check update status:**
```bash
./target/release/doli-node update status
```

**Manual update:**
```bash
cd doli
git pull
cargo build --release
sudo cp target/release/doli-node /usr/local/bin/
sudo systemctl restart doli-node
```

---

### 5.2. Version Mismatch

**Symptom:** Peers disconnect immediately after handshake. Logs show:
```
Protocol version mismatch with peer <id>: we require >= X, they report Y
```

**Cause:** The remote peer is running a protocol version below the local node's `MIN_PEER_PROTOCOL_VERSION`. This happens after a network upgrade bumps the minimum required version.

**Solution:**
```bash
# Check current version
./target/release/doli-node --version

# Update to latest
cd doli
git pull
cargo build --release
```

**Note:** If you see `HARD FORK ACTIVE: binary version X is too old for height Y` in the logs, a compile-time hard fork has activated and your binary must be upgraded to resume block production.

---

## 6. Diagnostic Commands

### Quick Health Check

```bash
#!/bin/bash
# health_check.sh

echo "=== Node Status ==="
systemctl status doli-node --no-pager | head -5

echo -e "\n=== Chain Info ==="
curl -s -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq

echo -e "\n=== Network Info ==="
curl -s -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | jq

echo -e "\n=== Mempool ==="
curl -s -X POST http://127.0.0.1:8500 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getMempoolInfo","params":{},"id":1}' | jq

echo -e "\n=== Disk Usage ==="
# Adjust path for your platform (see Section 1.1 for locations)
du -sh /var/lib/doli/mainnet/data/ 2>/dev/null || du -sh ~/Library/Application\ Support/doli/mainnet/data/ 2>/dev/null || du -sh ~/.doli/mainnet/data/

echo -e "\n=== Memory Usage ==="
ps aux | grep doli-node | grep -v grep
```

### Log Analysis

```bash
# Recent errors
grep -i error /var/log/doli-node.log | tail -20

# VDF timing
grep "VDF" /var/log/doli-node.log | tail -10

# Peer connections
grep -i "peer\|connect" /var/log/doli-node.log | tail -20

# Block production (for producers)
grep -i "produced\|block" /var/log/doli-node.log | tail -20
```

---

## 7. Sync Recovery & State Root Divergence

### 7.1. Recovery Order

When a node falls out of sync or produces a fork:

1. **rollback_one_block()** — first option on all networks. Uses undo data (O(1)) if available, rebuild-from-genesis as fallback.
2. **Snap sync** — only as fallback if rollback fails repeatedly. Quorum: `max(3, tip_eligible_peers/2 + 1)`.
3. **Genesis mismatch peers** — get 1-hour silent cooldown (won't attempt sync from them).

Code: `bins/node/src/node/rollback.rs` (rollback), `crates/network/src/sync/manager.rs` (snap sync)

### 7.2. State Root Divergence (Snap Sync Failure)

**Symptom:** Snap sync never completes — quorum is never reached because nodes disagree on state root.

**Cause:** State roots diverge when the dual UTXO paths (in-memory vs disk) produce different results. Most common cause: Bond `extra_data` stamping mismatch.

**Diagnosis:**
```bash
# Compare state roots at same height across all nodes
curl -s -X POST http://<node>:<port> \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}' | jq

# Find divergent UTXOs between two nodes
curl -s -X POST http://<node>:<port> \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getUtxoDiff","params":{"peer_url":"http://<other_node>:<port>"},"id":1}' | jq
```

**Resolution:** Fix the code path that diverges (verify both UTXO paths in `apply_block()`), then chain reset. See `docs/architecture.md §9.1` for the dual UTXO path invariant.

---

## 8. Getting Help

### Resources

- **Documentation:** This repository's docs folder
- **Issues:** https://github.com/e-weil/doli/issues
- **Community:** (add Discord/Telegram links)

### Reporting Bugs

Include in bug reports:
1. DOLI version (`doli-node --version`)
2. Operating system and version
3. Relevant log excerpts
4. Steps to reproduce
5. Expected vs actual behavior

---

*Last updated: March 2026*
