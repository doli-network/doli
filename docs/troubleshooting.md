# troubleshooting.md - Common Issues and Solutions

This guide helps diagnose and resolve common issues with DOLI nodes and wallets.

---

## 1. Node Issues

### 1.1. Node Won't Start

**Symptom:** Node exits immediately or fails to start.

| Possible Cause | Solution |
|----------------|----------|
| Port already in use | Check for existing process: `lsof -i :30303` |
| Corrupt database | Remove and resync: `rm -rf ~/.doli/mainnet/db/` |
| Missing dependencies | Reinstall via `nix develop` or install manually |
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
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}'

# Check sync status
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
```

**Add bootstrap nodes:**
```bash
./target/release/doli-node run \
    --bootstrap /dns4/seed1.doli.network/tcp/30303 \
    --bootstrap /dns4/seed2.doli.network/tcp/30303
```

---

### 1.5. Low Peer Count

**Symptom:** Fewer than 5 peers connected.

| Possible Cause | Solution |
|----------------|----------|
| Firewall blocking P2P port | Open port 30303 (or network-specific) |
| NAT not traversable | Enable port forwarding on router |
| DHT disabled | Remove `--no-dht` flag if present |
| Bootstrap nodes unreachable | Try alternative bootstrap nodes |

**Check firewall:**
```bash
# UFW
sudo ufw status

# iptables
sudo iptables -L -n | grep 30303
```

**Test port accessibility:**
```bash
# From another machine
nc -zv your-node-ip 30303
```

---

### 1.6. High Memory Usage

**Symptom:** Node consuming excessive RAM.

| Possible Cause | Solution |
|----------------|----------|
| Large mempool | Restart node (mempool clears on restart) |
| Many peers | Use `--max-peers 25` flag |
| Memory leak | Update to latest version |

**Reduce resource usage with CLI flags:**
```bash
./doli-node --max-peers 25 run
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
curl -X POST http://127.0.0.1:8545 \
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
./target/release/doli wallet balance
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
curl -X POST http://127.0.0.1:8545 \
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
./target/release/doli wallet addresses
```

**Check via RPC:**
```bash
curl -X POST http://127.0.0.1:8545 \
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
curl http://127.0.0.1:8545
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
ping boot1.doli.network

# Test P2P port connectivity
nc -zv boot1.doli.network 30303
```

---

### 4.2. Chain Fork

**Symptom:** Your chain diverges from network consensus.

| Possible Cause | Solution |
|----------------|----------|
| Isolated from network | Add more peers |
| Software bug | Update to latest version |
| Intentional attack | Verify with multiple sources |

**Check against known block:**
```bash
# Compare your block at height X with explorer
curl -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getBlockByHeight","params":{"height":12345},"id":1}'
```

**Force resync if needed:**
```bash
sudo systemctl stop doli-node
rm -rf ~/.doli/mainnet/db/
sudo systemctl start doli-node
```

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

**Symptom:** Peers reject connection due to version.

**Solution:**
```bash
# Check current version
./target/release/doli-node --version

# Update to latest
cd doli
git pull
cargo build --release
```

---

## 6. Diagnostic Commands

### Quick Health Check

```bash
#!/bin/bash
# health_check.sh

echo "=== Node Status ==="
systemctl status doli-node --no-pager | head -5

echo -e "\n=== Chain Info ==="
curl -s -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | jq

echo -e "\n=== Network Info ==="
curl -s -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | jq

echo -e "\n=== Mempool ==="
curl -s -X POST http://127.0.0.1:8545 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getMempoolInfo","params":{},"id":1}' | jq

echo -e "\n=== Disk Usage ==="
du -sh ~/.doli/mainnet/db/

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

## 7. Getting Help

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

*Last updated: January 2026*
