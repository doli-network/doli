# DOLI Troubleshooting Guide

This guide helps diagnose and resolve common issues with DOLI nodes and wallets.

## Node Issues

### Node won't start

**Symptoms:**
- Node exits immediately
- Error messages about binding or initialization

**Solutions:**

1. **Port already in use**
   ```bash
   # Check if ports are occupied (adjust for your network)
   lsof -i :30303  # P2P port (mainnet)
   lsof -i :8545   # RPC port (mainnet)
   lsof -i :9090   # Metrics port (default)

   # Kill conflicting process or use different ports:
   ./doli-node run --p2p-port 50301 --rpc-port 28541 --metrics-port 9091
   ```

   **Multi-node setups:** Each node needs unique ports. Common setup:
   | Node | P2P Port | RPC Port | Metrics Port |
   |------|----------|----------|--------------|
   | Node 1 | 50301 | 28541 | 9091 |
   | Node 2 | 50302 | 28542 | 9092 |
   | Node 3 | 50303 | 28543 | 9093 |

2. **Permissions error**
   ```bash
   # Check data directory permissions
   ls -la ~/.doli

   # Fix permissions
   chmod 755 ~/.doli
   chmod 600 ~/.doli/node.key
   ```

3. **Corrupt data**
   ```bash
   # Backup and reinitialize (last resort)
   mv ~/.doli ~/.doli.backup
   ./doli-node init
   ```

### No peers connecting

**Symptoms:**
- `peers_connected` metric stays at 0
- "No peers" messages in logs

**Solutions:**

1. **Firewall blocking connections**
   ```bash
   # Allow P2P port
   sudo ufw allow 30303/tcp

   # Or with iptables
   sudo iptables -A INPUT -p tcp --dport 30303 -j ACCEPT
   ```

2. **NAT/Router issues**
   - Enable UPnP on your router
   - Or manually forward port 30303

3. **Bootstrap nodes unreachable**
   ```bash
   # Test connectivity to bootstrap nodes
   nc -zv seed1.doli.network 30303

   # Add additional bootstrap nodes in config
   ```

### Sync stalled

**Symptoms:**
- Height not increasing
- `sync_progress` not changing

**Solutions:**

1. **Check peer status**
   ```bash
   curl http://localhost:8545 \
     -d '{"jsonrpc":"2.0","method":"getPeers","params":{},"id":1}'
   ```

2. **Restart the node**
   ```bash
   # Stop gracefully
   kill -TERM $(pgrep doli-node)

   # Wait, then start again
   ./doli-node run
   ```

3. **Clear peer database and resync**
   ```bash
   rm ~/.doli/peers.db
   ./doli-node run
   ```

### High memory usage

**Symptoms:**
- Node consuming excessive RAM
- OOM killer terminating process

**Solutions:**

1. **Increase system memory** (if possible)

2. **Limit mempool size** in config:
   ```toml
   [mempool]
   max_count = 1000
   max_size = 5242880  # 5MB
   ```

3. **Enable swap** (temporary):
   ```bash
   sudo fallocate -l 4G /swapfile
   sudo chmod 600 /swapfile
   sudo mkswap /swapfile
   sudo swapon /swapfile
   ```

### Disk full

**Symptoms:**
- Write errors in logs
- Database corruption

**Solutions:**

1. **Check disk usage**
   ```bash
   df -h ~/.doli
   du -sh ~/.doli/*
   ```

2. **Clean up old data** (if configured for pruning)

3. **Expand storage** or move to larger disk

---

## Wallet Issues

### Can't connect to node

**Symptoms:**
- "Cannot connect to node" error
- Balance shows 0 unexpectedly

**Solutions:**

1. **Verify node is running**
   ```bash
   curl http://localhost:8545 \
     -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}'
   ```

2. **Check RPC endpoint**
   ```bash
   # Use correct endpoint
   ./doli --rpc http://localhost:8545 balance
   ```

3. **RPC disabled** - Enable in node config:
   ```toml
   [rpc]
   enabled = true
   ```

### Transaction stuck

**Symptoms:**
- Transaction pending indefinitely
- Not appearing in blocks

**Solutions:**

1. **Check fee** - May be too low
   ```bash
   # Get mempool info
   curl http://localhost:8545 \
     -d '{"jsonrpc":"2.0","method":"getMempoolInfo","params":{},"id":1}'
   ```

2. **Replace with higher fee** (RBF not currently supported, wait for expiry)

3. **Check for UTXO conflicts** - Another transaction may have spent inputs

### Wrong balance

**Symptoms:**
- Balance doesn't match expected
- Missing transactions

**Solutions:**

1. **Wait for sync**
   ```bash
   # Check sync status
   curl http://localhost:8545 \
     -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}'
   ```

2. **Verify address** - Make sure you're checking the correct address

3. **Check for unconfirmed transactions**

---

## Producer Issues

### Not producing blocks

**Symptoms:**
- Assigned slots being missed
- Other producers taking over

**Solutions:**

1. **Verify registration**
   ```bash
   ./doli producer status
   ```

2. **Check producer key is loaded**
   ```bash
   # Ensure --producer-key flag is set
   ./doli-node run --producer --producer-key /path/to/key
   ```

3. **VDF computation too slow**
   - Check hardware meets requirements
   - Run benchmarks: `cargo run -p doli-benchmarks -- full`

### Getting slashed

**Symptoms:**
- Bond reduced
- "Slashing" events in logs

**Causes and solutions:**

1. **Double production** - Review setup for multiple nodes using same key

2. **Invalid blocks** - Check node logs for validation errors

### Fallback producing all blocks

**Symptoms:**
- Primary producer offline
- Secondary/tertiary taking slots

**Solutions:**

1. **Check primary's uptime**

2. **Verify network connectivity**

3. **Check VDF timing** - Should complete in ~700ms (the VDF calibration adjusts automatically)

---

## Bootstrap Mode Issues (Devnet)

### Chain splits / "Block doesn't build on tip"

**Symptoms:**
- Multiple nodes producing different chains
- "Block doesn't build on tip" warnings
- Chain height differs between nodes

**Causes:**

1. **Nodes not connected** - `--bootstrap` flag missing or incorrect
2. **Sync not complete** - Nodes produce before syncing with peers
3. **Different chain states** - Nodes compute different leader elections

**Solutions:**

1. **Ensure bootstrap connectivity**
   ```bash
   # Node 2 must connect to node 1 before it will produce
   ./doli-node --network devnet run --producer --bootstrap /ip4/127.0.0.1/tcp/50301 ...

   # Check logs for "Peer connected" message
   # Node will sync before producing
   ```

2. **Verify sync is working** - New nodes use sync-before-produce logic. They will not produce until their chain is within 2 slots of their peers.

3. **Verify nodes are connected**
   ```bash
   # Check peer count (should be > 0 for non-seed nodes)
   curl http://localhost:28541 -d '{"jsonrpc":"2.0","method":"getPeers","id":1}'
   ```

### "Duplicate block" warnings

**Symptoms:**
- Logs show "Failed to broadcast block: Duplicate"
- Block still appears in chain

**Explanation:**
This is **expected behavior** in GossipSub. When a node receives its own block back from the network, GossipSub's deduplication marks it as a duplicate. The block was already processed locally.

**Not a problem** - The block is successfully in the chain. This warning can be ignored.

### Both nodes producing for same slot

**Symptoms:**
- Two blocks with same slot number
- One block gets orphaned

**Causes:**

1. **Race condition** - Both nodes are eligible and produce before seeing the other's block
2. **Network latency** - Block doesn't propagate fast enough

**Solutions:**

1. **Verify time synchronization** - Nodes should have synchronized clocks
   ```bash
   timedatectl status
   ```

2. **Increase network connectivity** - Ensure nodes can reach each other quickly

3. **Accept occasional races** - In bootstrap mode with multiple producers, occasional races are normal. The network converges on one chain.

### "Bootstrap sync: deferring production" message

**Symptoms:**
- Node logs "Bootstrap sync: deferring production (our slot=X, peer slot=Y)"
- Node not producing blocks immediately after start

**Explanation:**
This is **intentional and correct**. The node detected that its chain is behind connected peers and is waiting for sync to complete before producing. This prevents chain splits.

**How sync-before-produce works:**
1. Node connects to peers via `--bootstrap` flag
2. Node receives peer status (their chain height/slot)
3. If our chain is behind by more than 2 slots, defer production
4. Once synced (within 2 slots), begin producing

**Solution:** Wait for sync to complete. Check logs for sync progress. If sync is stuck, see "Sync stalled" section above.

---

## Fork Detection Issues

### Chain keeps resetting to genesis

**Symptoms:**
- Logs show "Detected fork: X blocks cached... Triggering forced resync"
- Chain height repeatedly goes back to 0
- "Chain reset to genesis" messages every few minutes

**Causes:**

1. **External peer interference** - Nodes from a different chain (different genesis) are sending blocks that don't connect to your chain
2. **Network partition** - Your test nodes are on a different fork than external peers

**Solutions:**

1. **Use `--no-dht` flag for isolated testing**
   ```bash
   # Isolate your test network from external peers
   ./doli-node --network testnet run \
       --producer \
       --producer-key wallet.json \
       --bootstrap /ip4/127.0.0.1/tcp/40301 \
       --no-dht
   ```

2. **Check fork detection diagnostics** - When fork detection triggers, the logs show:
   - Your chain tip (hash, height, slot)
   - Sample of orphan blocks (up to 5)
   - Pattern analysis (e.g., "X orphans share parent - likely external chain")

3. **Verify genesis hash matches** - Nodes on different chains will produce blocks that can't be applied

**Fork detection thresholds:**
| Network | Threshold | Slot Time | Coverage |
|---------|-----------|-----------|----------|
| Mainnet | 60 blocks | 60s | 1 hour |
| Testnet | 30 blocks | 10s | 5 minutes |
| Devnet | 20 blocks | 5s | ~2 minutes |

The threshold represents how many orphan blocks must accumulate before the node triggers a forced resync. Higher thresholds prevent false positives from temporary network issues.

### External peers connecting unexpectedly

**Symptoms:**
- Nodes connect to peers you didn't configure
- Peer IDs you don't recognize in logs
- Fork detection triggered by blocks from external sources

**Cause:** DHT (Kademlia) peer discovery finds nodes on the same network ID

**Solution:** Use `--no-dht` to disable DHT discovery:
```bash
./doli-node run --no-dht --bootstrap /ip4/known.peer/tcp/40303
```

With `--no-dht`, nodes only connect to explicitly provided bootstrap addresses.

---

## Network Issues

### Eclipse attack suspected

**Symptoms:**
- All peers from same IP range
- Seeing different chain than network

**Solutions:**

1. **Check peer diversity**
   ```bash
   curl http://localhost:8545 \
     -d '{"jsonrpc":"2.0","method":"getPeers","params":{},"id":1}'
   ```

2. **Add diverse bootstrap nodes**

3. **Enable diversity protection** (default):
   ```toml
   [network]
   enable_diversity = true
   max_per_prefix = 3
   ```

### Getting rate limited

**Symptoms:**
- "Rate limit exceeded" errors
- Dropped connections

**Solutions:**

1. **Reduce request frequency**

2. **Check for loops in client code**

3. **Contact node operator** if using public RPC

---

## Diagnostic Commands

```bash
# Check node health
curl http://localhost:9090/metrics | grep doli_

# View recent logs
journalctl -u doli-node -f

# Check resource usage
top -p $(pgrep doli-node)

# Network connectivity
netstat -an | grep 30303

# Disk I/O
iotop -p $(pgrep doli-node)
```

---

## Getting Help

If these solutions don't resolve your issue:

1. **Check logs** with `--log-level debug`
2. **Search issues** at https://github.com/e-weil/doli/issues
3. **Ask in Discord** (link in README)
4. **Open an issue** with:
   - Node version
   - Operating system
   - Relevant log output
   - Steps to reproduce
