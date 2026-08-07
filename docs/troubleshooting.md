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
    --bootstrap /dns4/seed2.doli.network/tcp/30300 \
    --bootstrap /dns4/seed3.doli.network/tcp/30300
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

### 1.10. `[STATE_CORRUPT]` — Interrupted Rebuild-From-Genesis (INC-I-156)

**Symptom:** the log carries `[STATE_CORRUPT] Interrupted rebuild-from-genesis detected on
startup`, the node syncs and accepts blocks but never produces one, and peers asking it for a
snap-sync snapshot or a state root get an error response instead of an answer.

If the message says `target height UNKNOWN (marker unreadable)`, the marker key is present but
its value could not be decoded. The read fails **closed** (INC-I-156 / AUDIT-P3-103): an
unreadable marker is treated as armed, because a node that cannot decode its own halt marker
cannot prove its ledger is intact. The remedy is the same.

**Cause:** a deep reorg or rollback with no undo data took the legacy rebuild-from-genesis path.
That path empties the durable UTXO set first and replays `1..=target_height` back into it, which
takes minutes on a real chain and holds the `chain_state` + `utxo_set` write locks throughout.
If the process is restarted inside that window — the fleet watchdog (`scripts/doli-watchdog.sh`)
does exactly this, because its `getChainInfo` probe blocks on those locks and times out after
5s — the node reboots at its old tip with a truncated ledger. A durable
`rebuild_in_progress` marker is written to `CF_META` before the wipe and removed only after the
rebuild's final `atomic_replace` succeeds, so its presence is proof the rebuild never finished.

**Fix — resync the node.** The marker lives in the state DB, so it clears with the data
directory:
```bash
sudo systemctl stop doli-node
# CHECK FIRST that no wallet.json / producer.seed.txt lives inside the data dir:
find /var/lib/doli/mainnet -name 'wallet*' -o -name '*.seed.txt'
sudo rm -rf /var/lib/doli/mainnet/state_db /var/lib/doli/mainnet/blocks
sudo systemctl start doli-node   # snap-syncs from a healthy peer
```
Restoring a checkpoint taken **before** the rebuild works too — see
[disaster-recovery.md](./disaster-recovery.md).

**A completed snap sync also clears it.** If the node reaches snap sync on its own — the likely
outcome, since an emptied UTXO set fails every subsequent block apply and that is exactly the
stuck-fork condition that escalates to snap sync — the install replaces the whole set with a
root-verified snapshot and disarms the marker automatically (INC-I-156 / AUDIT-P2-101). A snap
sync that is *rejected* (root mismatch) installs nothing and leaves the halt in place, which is
correct.

**Do not** delete the marker by hand to silence the message. It is refusing production,
snapshot service and state-root service precisely because the ledger this node would produce
from, hand to a bootstrapping peer, or vote with in the snap-sync quorum is incomplete — and
nothing downstream would catch that, since block headers carry no state root.

---

### 1.7. Disk full / ENOSPC

**Symptom:** The node stops or returns a clean error mentioning `ENOSPC` /
"No space left on device" instead of crashing with a SIGABRT core-dump. This is the
intended M1 clean-error behavior — the node fails gracefully on a full disk rather
than aborting mid-write.

**Most common cause:** an unbounded `/var/log/doli/{network}.log`. The systemd unit
appends stdout/stderr to that single file for the life of the process; without
rotation it grows without limit.

**Reclaim space immediately:**
```bash
# See what is consuming the disk
df -h /var
du -sh /var/log/doli/* /var/lib/doli/* 2>/dev/null | sort -h | tail

# Truncate the live log in place (safe under copytruncate — keeps the append fd valid)
sudo truncate -s 0 /var/log/doli/mainnet.log

# Remove old compressed rotations if present
sudo rm -f /var/log/doli/mainnet.log.*.gz
```

**Permanent fix — log rotation (installed automatically):**
`doli service install` now writes a logrotate drop-in at
`/etc/logrotate.d/doli-{network}`. Steady-state disk for logs is bounded to about
`(rotate + 1) × maxsize ≈ 1.2 GB`, plus at most one inter-rotation burst-day of
residual (architecture §D2). Re-run `sudo doli service install` on existing hosts to
adopt it, or drop the file in manually:

```
/var/log/doli/mainnet.log {
    maxsize 200M
    daily
    rotate 5
    copytruncate
    compress
    delaycompress
    missingok
    notifempty
}
```

`copytruncate` is load-bearing: systemd holds the append fd open, so a rename-based
rotation would leave the node writing to the rotated inode forever. Substitute the
network name (`testnet`, `devnet`, ...) on both the filename and the leading path line.

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

### 2.5. AddBond Rejected — Cap Exceeded (INC-I-080)

**Symptom:** Post-activation (mainnet `height >= 231_830`), an AddBond
transaction is rejected and the carrying block fails validation with
`[ADDBOND_CAP_EXCEEDED] AddBond cap exceeded at height=… producer=…`.

**Cause:** The producer's total bonds would exceed
`MAX_BONDS_PER_PRODUCER` (3,000). The check sums the producer's current
`bond_count` + in-flight pending AddBonds (including earlier ones in the
same block) + the requested bonds. This is the intended consensus rule —
not a fault. Pre-activation the surplus was silently clipped and the
Bond UTXOs orphaned (the bug this fixes); post-activation the AddBond is
rejected up-front so no value is lost.

**Resolution:**
1. Submit an AddBond whose `bond_count` keeps the running total
   `≤ 3,000`. Check current bonds via `getProducers`.
2. To grow influence beyond the own-bonds ceiling, use delegation
   (subject to the separate INC-I-078 `received_delegation_cap`).
3. If a block was rejected fleet-wide, the offending AddBond must be
   evicted from the mempool / not re-included; the slot is simply
   re-produced without it (deterministic across all nodes — no fork).

---

### 2.6. Wiped Producer Mints Its Own Block 1 (INC-I-149)

**Symptom:** A producer started with `--producer` on an **empty data
directory**, joining an existing chain, builds its own `height=1` block
about 30 s after start instead of waiting to sync. It then snap-syncs onto
the real chain, but the self-produced block survives below the snap horizon
as a permanent **fossil orphan**: the node disagrees with the whole fleet on
block 1 while matching it at every other height, including the tip. Gauntlet
GS-001 (`single-block1-hash`) reports `distinct block1=2`. Log signature:

```
[PROD_DIAG] BOOTSTRAP path: in_genesis=true active_empty=false height=1 slot=…
Producing block for slot … at height 1
[BLOCK_PRODUCED] hash=… height=1 parent=<genesis>
```

If peer count stays below `SNAP_MIN_PEERS` (3), the rescuing snap may be
delayed and the node can sit at `h=1` emitting `[STUCK_FORK]` and
`Empty headers … consecutive=N` until enough peers arrive.

**Cause:** The production path used **local** height as a proxy for network
age. An empty disk gives `best_height=0 → height=1`, which
`is_in_genesis()` (`crates/core/src/network/economics.rs:56`) reports as
"in genesis" — it is a pure function of local height and cannot tell "the
network is at genesis" from "my disk is empty". The peer-aware
behind-network guard that should have caught this
(`bins/node/src/node/production/mod.rs`) was itself gated on `height > 1`,
excluding the exact case its own comment described. Peer-reported height was
already known (`best_peer_height` at the network tip roughly 30 s before
every observed mint), so the node held the evidence and did not act on it.

**Resolution:** Upgrade to a build carrying the INC-I-149 fix — the
behind-network guard now covers `height == 1`, so a node whose peers report a
materially higher tip defers production instead of minting. Real fresh-genesis
bootstrap is unaffected by construction: there every peer reports height 0, so
the guard never fires and no delay is introduced.

- Pre-existing fossils are **not** repaired by the upgrade. Clear one by
  stopping the node, wiping `data/` and letting it snap-sync again (block 1
  then correctly reads `ABSENT`/snap-pruned). **Before any wipe, confirm
  `wallet.json` and `producer.seed.txt` are not inside `data/`.**
- Legacy workaround, no longer needed on a fixed binary: start the wiped node
  **without** `--producer`, let it snap-sync, then restore the flag and
  restart.
- The fix has a second half: a **no-evidence gate** — a producer with
  `bootstrap_nodes` configured and no peer status yet refuses to produce.
  This wait has **no timeout** (deliberate: producing blind is the failure
  mode being fixed); a producer whose bootstrap peers are all unreachable
  waits at height 0 until one answers. `bootstrap_timeout_secs` does not
  apply here — it lives inside the `!in_genesis` branch and cannot rescue
  height 1.
- Known residual: if the FIRST peer status a wiped producer receives comes
  from a height-0 peer, evidence reads `AtGenesis` and the fossil mint is
  still possible. During fleet-wide recovery bring seeds/synced nodes up
  FIRST so wiped producers hear a real tip before their first slot.
- Verify with `getBlockByHeight(1)` across the fleet: every holder must return
  the same hash; snap-pruned nodes returning `Block not found` are expected
  and are **not** divergence.

Code: `bins/node/src/node/production/mod.rs`. Invariant: `INV-PROD-004`.
Regression: `bins/node/tests/inc_i_149_bootstrap_mint_gate.rs`. Gauntlet:
GS-001.

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
| DeFi tx submitted pre-activation | Check the rejection: error code `DEFI_NOT_ACTIVATED` means the 11 DeFi tx types (CreatePool, AddLiquidity, RemoveLiquidity, Swap, CreateLoan, RepayLoan, LiquidateLoan, LendingDeposit, LendingWithdraw, FractionalizeNft, RedeemNft) are still gated. Mainnet default is disabled (`u64::MAX`) under INC-I-088 Phase 0. Operator must roll out a binary that pins a concrete future activation height. |
| Spend of pre-existing Collateral UTXO | Rejected with `[ERRTX-DEFI001]` — Collateral UTXOs are hard-frozen until the lending subsystem is audited and un-gated (INC-I-088 Phase 0). No operator-side fix; wait for the un-gate. |

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

Prefer `scripts/node-heal.sh` (see §4.4 Step 5) — it preserves `signed_slots.db`
(slashing protection) and excludes `utxo_store/` (avoids the INC-I-027 silent
corruption rollback). Producers only.

Manual fallback (only if no trusted healthy source exists):
```bash
sudo systemctl stop doli-node
# Remove chain state (platform-specific path, or use DOLI_DATA_DIR):
#   Linux:  /var/lib/doli/mainnet/data/
#   macOS:  ~/Library/Application Support/doli/mainnet/data/
#   Legacy: ~/.doli/mainnet/data/
# IMPORTANT: do NOT delete signed_slots.db — it's slashing protection.
cd <DATA_DIR>/data
rm -rf state_db blocks utxo_store maintainer_state.bin producer_gset.bin peers.cache
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

**Step 5 — Fix & recover**. Deploy fix, rebuild each poisoned producer from a healthy node:
```bash
# Testnet preset (local launchd):
scripts/node-heal.sh --testnet --target n5 --source n3 --yes

# Mainnet / systemd (run on the target server, SSH'd in):
scripts/node-heal.sh \
    --target-data ~/.doli/mainnet/data \
    --source-data healthy-host:~/.doli/mainnet/data \
    --stop-cmd  "sudo systemctl stop doli-mainnet-n5" \
    --start-cmd "sudo systemctl start doli-mainnet-n5" \
    --target-rpc 127.0.0.1:8500 \
    --skip-source-rpc-check \
    --yes
```

`node-heal.sh` wipes the target's `data/` **except** `signed_slots.db` (preserving
slashing protection), excludes `utxo_store/` from the rsync (avoiding the
INC-I-027 silent-corruption rollback), deletes stale `producer.lock` and
`pending_update.json`, and polls the target RPC after restart to confirm
recovery. Producers only — refuses seed nodes.

**Manual fallback** (only if no trusted healthy source is available):
```bash
# On each producer:
# 1. Stop node
# 2. rm -rf data_dir/state_db data_dir/blocks data_dir/utxo_store
#    (KEEP signed_slots.db — slashing protection)
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

### 4.5. Fleet-Wide CPU + Network Spike (Gossip Re-Forward Storm)

**Symptom:** Daily or otherwise periodic fleet-wide CPU spike with a symmetric inbound/outbound network spike across all nodes, accompanied by a flood of `Unexpected delivery trace` log lines. The chain stays live, but every node burns CPU and bandwidth in synchronized bursts.

**Cause:** Gossip messages on un-gated topics (attestations, heartbeats, headers, votes, transactions) were `Accept`ed by default and re-forwarded. Once libp2p's 60s duplicate cache expired, a re-delivered copy (age 60–120s) passed the dedup check and was re-forwarded to the whole mesh — a self-amplifying duplicate re-forward storm. Attestations, being the most frequent message, were the leading source.

**Resolution:** Fixed by the INC-I-142 unified gossip staleness/dedup gate (`v > 6.23.10`). All five topics now route through `classify_gossip()` (`crates/network/src/gossip/staleness.rs`), which applies a PRIMARY raw-bytes identity dedup — `blake3(topic_discriminant || raw_message_bytes)` against a 180s bounded `SeenCache` — that closes the 60–120s re-delivery window independent of libp2p's duplicate cache, plus a SECONDARY generous age filter. The change is node-local (no activation height, rolling-safe). Upgrade all nodes to a build past v6.23.10.

---

### 4.6. Repeated `[FINALITY_GUARD] refusing ShallowRollback` Spam (INC-I-143)

**Symptom:** A node wedged one block behind the network tip logs `[FINALITY_GUARD] refusing ShallowRollback target_h=… (finality=…, local_tip=…)` hundreds of times (seed1 logged 454) and never recovers. The gap stays at 1, production is stalled, and no snap or rollback ever fires.

**Cause:** The node finalized one branch of a genuine sibling fork (`finality == local_tip`), so a depth-1 rollback target (`local_height − 1`) is below finality and the INV-SYNC-008 guard correctly refuses it. Pre-fix the recovery coordinator then returned `RecoveryAction::None` and re-evaluated the same unchanged state every tick — a hot refusal livelock that could not fetch the competing sibling.

**Resolution:** Fixed by INC-I-143 (D4). The coordinator now emits a non-destructive `RecoveryAction::SiblingFetch { height: local_tip }` instead of `None` — a `GetBlockByHeight` request to up to 3 top peers, logged as `[FINALITY_GUARD] … INC-I-143 non-destructive SiblingFetch attempt N/3`. The fetched sibling flows through normal block handling where the wedge-escape re-evaluates it via `plan_reorg`. It is bounded to 3 consecutive attempts (~90s at the 30s cooldown), then falls through to standard escalation. If you see a handful of SiblingFetch lines followed by recovery, that is the fix working; the finality guard's strict `<` is UNCHANGED (it never rolls back below finality). Upgrade to a build carrying INC-I-143.

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

### 7.3. SnapSync Admission (INC-I-139, INC-I-152)

**How a node escalates to snap now:** one evidence-gated funnel plus a bootstrap window. `start_sync()` (`decision.rs`) reaches `SnapCollecting` only with `peers ≥ 3`, `snap.attempts < 3`, snap enabled, AND one of **three** gated doors:

- **(a)** `local_height == 0` — classic bootstrap.
- **(c)** `0 < local_height ≤ genesis_blocks` **and** gap > `SNAP_SYNC_GAP_MIN(500)` — the bootstrap genesis window (INC-I-152). Neither conjunct admits alone.
- **(b)** `needs_genesis_resync`, set only by `request_genesis_resync()`. Every feeder of that gate needs corroborated evidence: ≥10 consecutive empty headers with gap ≥ `MINOR_FORK_GAP_MAX(50)`, an explicit deep-fork signal, ≥3 apply failures, all-peers-blacklisted, a height-offset signature — **or** gap ≥ `SNAP_SYNC_GAP_MIN(500)`.

No bare gap-over-threshold admits snap on any path. `genesis_blocks` is the network's genesis window (mainnet 360, testnet 36, devnet 40), passed from `NetworkParams` into the sync manager at startup.

**When a node snaps unexpectedly, check (in order):**
1. `consecutive_empty_headers` — is the node actually seeing sustained empty headers (real stall), or did a progress reset fail to fire? Legitimate resets: genuine block apply, gap≤3 gossip-wait, valid connecting-headers, anti-cascade, post-rollback/post-snap grace, genesis. No admission or request-dispatch path may zero it.
2. Gap size — a snap at gap < 500 must carry full evidence (empties≥10 + fork signal). A snap at gap ≥ 500 is a legitimate forward-large-gap catch-up (Gate-1 exempts emergency ∪ forward-large-gap).
3. Local height — a snap from `1 ≤ h ≤ genesis_blocks` at gap > 500 is door (c) and is expected right after a wipe. A snap from `h > genesis_blocks` must be door (b): look for the resync reason in the log.
4. `snap.attempts` — never reset by any admission/redirect path; a re-armed attempts counter signals a bug.

**What a wiped node should look like now (INC-I-152 fast path):** after `doli wipe`, the node parks briefly (`[SNAP_SYNC] Bootstrap node (h=N): waiting for … peer(s) for snap sync (k/3, gap=…)`) until 3 peers connect, then snaps and catches up — expect roughly 15-30s wipe-to-synced on mainnet, not minutes. The `h=N` in that line is **not** an error: Orphan Chase applies genesis blocks 1..N within seconds of a wipe, and door (c) is exactly what keeps the node eligible for the fast path while it does. A wiped node that instead reports `Starting sync epoch …` and sits at a low height for minutes has fallen back to header-first — that is the INC-I-152 symptom (measured: 129,822 headers at 500 per 1s tick ≈ 260s, 92% of a 4m43s wipe-to-synced). Check that the binary carries the fix and that `genesis_blocks` reached the sync manager (a `0` window disables door (c) entirely).

**`Bootstrap node (h=N) waited 60s for snap peers but only have k`:** the hold timed out — fewer than 3 peers were reachable, and the node has committed to header-first. This is a peer-availability problem, not an admission problem. Check seed reachability and P2P connectivity; do not loosen the gate.

> **Testnet/devnet operators — admission is weaker than mainnet's (accepted residual, AUDIT-P1-004).** On mainnet, `genesis_blocks` is env-locked at 360 and nothing lowers a healthy node into `[1, 360]`, so door (c) only ever admits freshly wiped or genuinely new nodes. On testnet (36), devnet (40), or with a `DOLI_GENESIS_BLOCKS` override, a chain whose TIP is ≤ `genesis_blocks` makes **every** node window-resident, and the `gap > 500` conjunct comes from a single peer's advertised height — an unvalidated claim a sybil peer can forge. On those networks, treat an unexplained snap on a young chain as plausible rather than impossible, and check the peer set before assuming a code defect.

**INC-I-139 in 5 lines:** (1) `should_snap` had three admission authorities, one an ungated bare-gap OR-term, letting a gap=51 minor-fork wedge snap with no fork evidence. (2) A dispatch-time reset zeroed the evidence counter every request, starving legitimate escalation. (3) A redirect path (A1) silently reset `snap.attempts`. (4) Phase 1 (RUN 455) consolidated to one funnel by subtraction: deleted the bare-gap term, removed the dispatch reset, deleted A1, added the Gate-1 forward-large-gap classification companion, demoted the threshold to an enable-sentinel. (5) Result: recurrence class INC-I-005/033/138 closed at the admission surface — no node snaps without corroborated evidence.

**INC-I-152 in 3 lines:** (1) Keying bootstrap admission on `local_height == 0` became a fencepost bug once Orphan Chase existed — a wiped node applies genesis blocks 1..14 within ~10s, loses door (a), and commits to a ~260s header walk (92% of a measured 4m43s wipe-to-synced). (2) Door (c) treats `0 < h ≤ genesis_blocks` + gap > 500 as bootstrap-shaped, and both bootstrap holds were widened the same way so the node still parks long enough to reach a 3-peer snap quorum. (3) It is not a re-opened Route A: the window is a shape predicate conjoined with the existing 500-block floor, it resets neither `snap.attempts` nor the evidence counter, and an unplumbed `genesis_blocks = 0` is bit-identical to the old behavior.

Code: `decision.rs`, `dispatch.rs`, `production_gate.rs`, `recovery.rs`, `types.rs` (`SyncConfig.genesis_blocks`), `bins/node/src/node/init.rs` (plumbing). Invariant: `INV-SYNC-011` (extended, all-paths; amended by INC-I-152). Spec: `specs/sync-snap-admission-architecture.md`.

---

### 7.4. Snap Snapshot Refused (`[SNAP_SYNC] F4 REFUSE`, INC-I-143)

**Symptom:** A node in snap sync logs `[SNAP_SYNC] F4 REFUSE (root): response_root=… != quorum_root=…` or `[SNAP_SYNC] F4 REFUSE (height): anchor (…, h=…) corroborated by only N/M peers`, retries alternate peers, and snap completes more slowly than before (or falls back to header-first).

**Cause / meaning:** This is the INC-I-143 F4 admission gate working, not an error. Before the fix, `handle_snap_snapshot` logged a served-root ≠ quorum-root mismatch at `info!` and ACCEPTED the snapshot anyway, and installed it at the serving peer's raw current-tip height. That is how seed1 spliced a forked anchor at a −1 height offset with a 45-block hole → permanent INTEGRITY −1. Two gates now guard admission: (1) the served `response_root` MUST equal the quorum-agreed root; (2) the anchor's `(block_hash, block_height)` MUST be corroborated by a STATUS quorum of connected peers. A failure of either increments `snap.integrity_refusals` and picks an alternate peer, then falls back to header-first if no alternates remain.

**What to do:** A few F4 REFUSE lines followed by a successful admit from an alternate peer is normal and correct — the node refused an uncorroborated anchor and found a corroborated one. Slower-but-correct is the intended trade. Persistent refusals across many peers mean genuine fleet tip-fragmentation (no quorum best_hash) — investigate the fork itself, do NOT loosen the gate. Code: `crates/network/src/sync/manager/snap_sync.rs` (gates), `types.rs` (`integrity_refusals`).

---

### 7.5. `[ECON_EPOCH_INPUTS_MISMATCH]` at an Epoch Boundary (INC-I-143 D5)

**Symptom:** At an epoch boundary a node halts (E302) with `[ECON_EPOCH_INPUTS_MISMATCH] EpochReward pool inputs mismatch at height=…: expected N inputs, got M (K differing outpoints). missing_from_actual (…): [outpoint:idx, …]; unexpected_in_actual (…): [outpoint:idx, …]`. The old message read only `expected 360 inputs, got 360` — equal counts, no clue what differed.

**Cause / meaning:** The compare is on the SET of pool-input outpoints the EpochReward transaction consumes, not on their count. Two nodes with divergent chain histories (e.g. either side of a sibling fork) can both have 360 pool UTXOs that are DIFFERENT outpoints — the counts match while the sets diverge. The halt is a divergent-pool-view symptom of an upstream fork, not a counting bug. Pre-fix the counts-only message masked this and cost triage time (INC-I-143 §5).

**What to do:** Read the `missing_from_actual` / `unexpected_in_actual` outpoint lists (bounded to 5 per side) — they name the exact UTXOs that differ, pointing at the block/epoch where the two histories forked. Fix the underlying fork/splice (see 7.4 and 4.6); the ECON halt clears once the node is on the corroborated chain. Code: `format_epoch_inputs_mismatch` in `bins/node/src/node/validation_checks.rs`.

---

### 7.6. Divergent `chainCommitment` With `missingCount=0` and Identical State Roots (INC-I-144)

**Symptom:** Nodes in full consensus (identical tip hash, byte-identical `getStateRootDebug`) return **different** `chainCommitment` values from `verifyChainIntegrity` over the same range, each with `missingCount=0`. Spot checks show `getBlockByHeight(h)` returning a block whose slot/producer differs across nodes at some heights, and a `prevHash` linkage walk over the by-height index breaks mid-range.

**Cause / meaning:** Height-index "fossils". Before the INC-I-144 fix, `height_index`/`hash_to_height` were written only on the apply path (`set_canonical_chain`); no rollback or reorg path removed entries, so `index[h]` kept the last block applied at h on *whatever branch the node was following*. The lazy self-heal (the winning branch re-applying through the range) could be permanently revoked by the INC-I-025 `snap_horizon` floor after a snap-anchor jump — freezing stale orphan segments (mainnet seed2 h7222–7227). This is a node-local index defect, NOT a consensus fork: state roots stay identical.

**What to do:** Upgrade to a build carrying INC-I-144 (`remove_canonical_entry` + purge-above-tip in `set_canonical_chain`) — this prevents new fossils; heights rewound thereafter return `None` until the canonical block is applied (fail-visible-missing, self-healing). Pre-existing fossils are NOT repaired automatically: locate stale ranges by binary-searching ranged `verifyChainIntegrity` calls between nodes, then run an offline reindex on the affected node (verify canonical headers exist first). `backfillFromPeer` CANNOT fix a fossil range sandwiched below an already-correct region — its divergence finder stops at the first tip match. Code: `crates/storage/src/block_store/writes.rs`, `bins/node/src/node/rollback.rs`, `bins/node/src/node/block_handling.rs`. Invariant: `INV-STORAGE-002`. Regression: `crates/storage/tests/inc_i_144_rollback_index_fossil_test.rs`.

---

## 8. Getting Help

### Resources

- **Documentation:** This repository's docs folder
- **Issues:** https://github.com/e-weil/doli/issues
- **Community:** (add Discord/Telegram links)

### Gossip Validation Misbehavior (INC-I-114)

**Symptom: A gossip topic appears silently dead — no messages arrive on that topic.**

**Cause:** With `validate_messages=true`, every gossipsub message is held until the application calls `report_message_validation_result()`. If the application fails to report a verdict for a topic (e.g., a new topic is added but the handler doesn't report), messages on that topic are never forwarded and the topic appears dead. This is a liveness failure, not a crash.

**Diagnosis:**
1. Check logs for `[GOSSIP_VALIDATE] report failed` — indicates the report call itself errored.
2. Check if the topic is matched in `behaviour_events.rs` — unmatched topics fall through to the `_ => {}` wildcard, which still gets the unconditional `Accept` for non-block topics. If a new block-body topic was added without updating `is_block_body_topic`, it would get unconditional Accept (safe, but no staleness filtering).
3. If a non-block topic was mistakenly added to the block-body topic check, its payloads would fail `Block::deserialize` and be Rejected — triggering P4 peer-score penalties on honest peers and eventually causing mesh expulsion cascades.

**Resolution:** Ensure all gossip topics have a validation report path. Block-body topics (`/doli/blocks/1`, `/doli/t1/blocks/1`, `/doli/r{N}/blocks/1`) go through `classify_block_gossip()`. All other topics (headers, transactions, producers, votes, heartbeats, attestations) get unconditional `Accept`.

**Symptom: Fleet-wide gossip amplification / OOM under load.**

**Cause (INC-I-114):** Before the `validate_messages=true` fix, gossipsub auto-forwarded messages after dedup cache expiry (60s). Under sustained load with 2MB blocks, stale blocks were re-forwarded through the mesh, multiplying traffic exponentially. The fix holds all messages until the application explicitly accepts them, preventing stale block re-forwarding.

**Diagnosis:** Look for `[GOSSIP_VALIDATE] stale/invalid block` log lines — these indicate the staleness filter is working. If absent during a storm, check that `NetworkConfig.genesis_time` is set (0 = staleness disabled, fail-open).

### Reporting Bugs

Include in bug reports:
1. DOLI version (`doli-node --version`)
2. Operating system and version
3. Relevant log excerpts
4. Steps to reproduce
5. Expected vs actual behavior

---

*Last updated: June 2026*
