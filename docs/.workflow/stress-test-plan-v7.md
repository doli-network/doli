# DOLI Stress Test Plan v7 — Post INC-I-014 Validation

**Date**: 2026-03-27
**Machine**: Mac Studio, 128 GB RAM, 16 cores, macOS Darwin 25.2.0, 291 GB disk free
**Binary**: doli-node 4.0.4 (MUST rebuild — see Pre-flight Step 1)
**Network**: Local testnet, all on 127.0.0.1
**Goal**: Validate INC-I-014 9-fix commit, find safe scaling ceiling, stress sync + fork recovery

---

## Table of Contents

1. [Pre-flight Checks](#1-pre-flight-checks)
2. [Phase 1: Baseline (6 nodes)](#2-phase-1-baseline-6-nodes)
3. [Phase 2: Gradual Scaling (up to 200+ nodes)](#3-phase-2-gradual-scaling)
4. [Phase 3: Sync Testing](#4-phase-3-sync-testing)
5. [Phase 4: Fork Recovery](#5-phase-4-fork-recovery)
6. [Phase 5: Stress Ceiling](#6-phase-5-stress-ceiling)
7. [Monitoring Commands](#7-monitoring-commands)
8. [Red Flags — Immediate Stop Conditions](#8-red-flags)
9. [Post-Test Analysis](#9-post-test-analysis)
10. [Env Override Quick Reference](#10-env-override-quick-reference)

---

## 1. Pre-flight Checks

Run these in order. Every check must pass before proceeding.

### Step 1: Rebuild binary with latest 9-fix commit

The current binary (5a32d55f) does NOT include commit 3bcc90b3 which has the 9 INC-I-014 fixes.
This is the most important pre-flight step.

```bash
# Verify you're on the right branch
cd /Users/isudoajl/ownCloud/Projects/doli
git log --oneline -1
# MUST show: 3bcc90b3 fix(network): 9 fixes for RAM explosion at 200+ nodes (INC-I-014)

# Build
cargo build --release 2>&1 | tail -3

# Verify the new binary
./target/release/doli-node --version
# Should show a commit hash starting with 3bcc90b

# Copy to testnet path (CRITICAL — stress-batch.sh uses ~/testnet/bin/doli-node)
cp target/release/doli-node ~/testnet/bin/doli-node

# Confirm
~/testnet/bin/doli-node --version
```

**STOP if the binary commit does not match 3bcc90b.** All test results will be invalid without the fixes.

### Step 2: Stop all existing nodes

```bash
cd /Users/isudoajl/ownCloud/Projects/localdoli

# Check what's currently running
scripts/stop-all.sh --dry-run

# Stop everything (except explorer and ram-watchdog which are preserved)
scripts/stop-all.sh

# Verify zero doli-node processes
pgrep -c doli-node 2>/dev/null || echo "0 processes — clean"
```

### Step 3: System resource checks

```bash
# File descriptor limit — must be unlimited or >= 65535
ulimit -n
# If not "unlimited", fix: ulimit -n 65535

# Disk space — need at least 100 GB free for 200+ node data dirs
df -h /Users/isudoajl | tail -1
# STOP if less than 100 GB free

# Check system RAM is actually 128 GB
sysctl -n hw.memsize | awk '{printf "%.0f GB\n", $1/1073741824}'

# Check no other heavy processes consuming RAM
ps aux --sort=-%mem | head -5

# Verify the RAM watchdog is running (it should survive stop-all.sh)
launchctl list | grep ram-watchdog
# If not running:
launchctl load ~/Library/LaunchAgents/network.doli.ram-watchdog.plist
```

### Step 4: Clean stale data from previous stress tests

```bash
# Remove previous stress batch data dirs (NOT the genesis node data)
rm -rf ~/testnet/nodes1 ~/testnet/nodes2 ~/testnet/nodes3 ~/testnet/nodes4
rm -rf ~/testnet/nodes5 ~/testnet/nodes6 ~/testnet/nodes7 ~/testnet/nodes8
rm -rf ~/testnet/nodes9 ~/testnet/nodes10
rm -rf ~/testnet/stress_b*

# Remove stale PID files
rm -f ~/testnet/nodes*/pids 2>/dev/null

# Clean old stress logs (keep genesis logs)
rm -rf ~/testnet/logs/nodes*
rm -rf ~/testnet/logs/stress_b*
rm -f ~/testnet/logs/stress-monitor.log
rm -f ~/testnet/logs/stress-gradual.log

# Rotate existing genesis node logs
scripts/rotate-logs.sh
```

### Step 5: Verify producer keys exist

```bash
# Stress nodes use producer_13.json through producer_512.json
# Verify the range we'll need
ls ~/testnet/keys/producer_13.json ~/testnet/keys/producer_62.json \
   ~/testnet/keys/producer_112.json ~/testnet/keys/producer_212.json \
   2>/dev/null | wc -l
# Must be 4 (all exist)

# Count total keys available
ls ~/testnet/keys/producer_*.json | wc -l
# Should be 512
```

### Step 6: Verify launchd services for genesis nodes

```bash
# Genesis nodes (seed + N1-N5) are managed by launchd
ls ~/Library/LaunchAgents/network.doli.testnet-seed.plist \
   ~/Library/LaunchAgents/network.doli.testnet-n1.plist \
   ~/Library/LaunchAgents/network.doli.testnet-n2.plist \
   ~/Library/LaunchAgents/network.doli.testnet-n3.plist \
   ~/Library/LaunchAgents/network.doli.testnet-n4.plist \
   ~/Library/LaunchAgents/network.doli.testnet-n5.plist \
   2>/dev/null | wc -l
# Must be 6
```

### Step 7: Record pre-test baseline

```bash
# Capture initial system state for comparison
echo "=== PRE-TEST BASELINE $(date) ===" | tee ~/testnet/logs/stress-test-v7-baseline.log
echo "Binary: $(~/testnet/bin/doli-node --version)" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Disk free: $(df -h /Users/isudoajl | tail -1 | awk '{print $4}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "RAM total: $(sysctl -n hw.memsize | awk '{printf "%.0f GB", $1/1073741824}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "ulimit -n: $(ulimit -n)" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "CPU cores: $(sysctl -n hw.logicalcpu)" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

**Pre-flight checklist:**

| Check | Expected | Command to verify |
|-------|----------|-------------------|
| Binary commit | 3bcc90b* | `~/testnet/bin/doli-node --version` |
| Zero doli-node processes | 0 | `pgrep -c doli-node` (returns error = 0) |
| ulimit -n | unlimited or >= 65535 | `ulimit -n` |
| Disk free | >= 100 GB | `df -h /Users/isudoajl` |
| RAM watchdog running | loaded | `launchctl list \| grep ram-watchdog` |
| Producer keys | 512 | `ls ~/testnet/keys/producer_*.json \| wc -l` |
| Stale batch dirs cleaned | 0 dirs | `ls -d ~/testnet/nodes* 2>/dev/null \| wc -l` |

---

## 2. Phase 1: Baseline (6 Nodes)

**Objective**: Establish resource baselines for seed + 5 producers. Confirm chain is advancing, all peers connected, state roots converging.

**Duration**: 5 minutes after all nodes are synced.

**Success criteria**:
- All 6 nodes responding to RPC
- All nodes at same height (within 2 blocks)
- Seed has 5 peers; each producer has >= 1 peer
- RAM total < 1 GB for 6 nodes
- State roots match across all nodes
- Chain is advancing (new blocks every slot)

### Step 1: Start genesis nodes

```bash
cd /Users/isudoajl/ownCloud/Projects/localdoli

# Start seed first, then producers
scripts/testnet.sh start seed
sleep 5
scripts/testnet.sh start n1 n2 n3 n4 n5
```

### Step 2: Wait for sync and verify

```bash
# Wait 30 seconds for all nodes to connect and start producing
sleep 30

# Check status
scripts/status.sh

# Verify all nodes have peers
for port in 8500 8501 8502 8503 8504 8505; do
  peers=$(curl -sf --max-time 2 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | \
    python3 -c "import sys,json; print(json.load(sys.stdin)['result']['peerCount'])" 2>/dev/null || echo "OFFLINE")
  echo "Port $port: $peers peers"
done
```

### Step 3: Record baseline metrics

```bash
# Record resource usage for 6 nodes
echo "=== PHASE 1 BASELINE $(date) ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node)" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "RAM (MB): $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.0f", sum/1024}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Per-node RSS:" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
ps aux | grep '[d]oli-node' | awk '{printf "  PID %s: %.0f MB\n", $2, $6/1024}' | tee -a ~/testnet/logs/stress-test-v7-baseline.log

# Verify state roots match
for port in 8500 8501 8502 8503 8504 8505; do
  curl -sf --max-time 2 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}' | \
    python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(f'Port {\"$port\"}: h={r[\"height\"]} root={r[\"stateRoot\"][:16]}...')" 2>/dev/null
done
```

### Step 4: Check MEM-BOOT log entries

```bash
# Verify each node logged correct connection params
for log in ~/testnet/logs/seed.log ~/testnet/logs/n{1,2,3,4,5}.log; do
  echo "=== $(basename $log) ==="
  grep 'MEM-BOOT' "$log" | tail -1
done
# Expected: max_peers=25 conn_limit=35 pending_limit=5 yamux_window=256KB idle_timeout=300s
```

**Phase 1 checkpoint — record these values before proceeding:**

| Metric | Value |
|--------|-------|
| Node count | 6 |
| Total RAM (MB) | ___ |
| Per-node avg RSS (MB) | ___ |
| Seed height | ___ |
| All state roots match | YES/NO |
| Peer count (seed) | ___ |

**STOP if**: state roots do not match, any node has 0 peers, RAM > 2 GB for 6 nodes, or chain is not advancing.

---

## 3. Phase 2: Gradual Scaling

**Objective**: Add nodes in controlled increments. At each checkpoint, verify convergence, measure RAM growth rate, detect non-linear RAM behavior.

**Duration**: 2-3 minutes stabilization per increment.

**Approach**: Use `stress-batch.sh` with `--count` for precise control. Add 10 nodes at a time within batch 1, then scale by 20s.

**Key RAM budget** (128 GB total, 80 GB watchdog threshold):
- Per-node at max_peers=25: ~121 MB (baseline 80 MB + 25 conns * 256 KB Yamux + buffers)
- Linear 200 nodes: ~24 GB
- Non-linear overhead (gossip, pending, DHT): historically 2-3x
- Safe operating target: 60 GB (50% of watchdog threshold)
- Investigate zone: 60-72 GB (75-90% of threshold)

### Start monitoring (in a dedicated terminal)

```bash
cd /Users/isudoajl/ownCloud/Projects/localdoli

# Terminal 1: Real-time resource monitor (threshold=80GB, sample every 5s)
scripts/stress-monitor.sh 80 5
```

### Increment 1: +10 nodes (total: 16)

```bash
cd /Users/isudoajl/ownCloud/Projects/localdoli

# Start first 10 nodes of batch 1 (n13-n22)
scripts/stress-batch.sh start 1 --count 10

# Wait 120 seconds for connections to stabilize + snap sync
sleep 120

# Check status
scripts/stress-batch.sh status
scripts/status.sh
```

**Checkpoint 2.1** (record in baseline log):
```bash
echo "=== CHECKPOINT 2.1: 16 nodes $(date) ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node)" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "TCP: $(lsof -i TCP -n -P 2>/dev/null | grep -c doli-node)" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "RSS range: $(ps aux | grep '[d]oli-node' | awk '{mb=$6/1024; if(NR==1||mb<min)min=mb; if(NR==1||mb>max)max=mb} END{printf "min=%.0f max=%.0f MB", min, max}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
# Seed height
curl -sf -X POST http://127.0.0.1:8500 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
  python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(f'Seed: h={r[\"bestHeight\"]} slot={r[\"bestSlot\"]}')" | \
  tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

**Success**: All 16 nodes synced (within 5 blocks of seed), RAM < 3 GB.

### Increment 2: +10 nodes (total: 26)

```bash
scripts/stress-batch.sh add 1 10
# This adds n23-n32 to batch 1

sleep 120

# Checkpoint 2.2
echo "=== CHECKPOINT 2.2: 26 nodes $(date) ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

**Success**: All 26 nodes synced, RAM < 5 GB.

### Increment 3: +20 nodes (total: 46)

```bash
scripts/stress-batch.sh add 1 20
# Adds n33-n52 to batch 1

sleep 180

# Checkpoint 2.3
echo "=== CHECKPOINT 2.3: 46 nodes $(date) ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

**Success**: All 46 nodes synced, RAM < 8 GB. Check for MEM-CONN-BUDGET warnings in seed log.

```bash
# Check seed for high pending or high eviction churn
grep -c 'HIGH PENDING' ~/testnet/logs/seed.log
grep -c 'HIGH EVICTION' ~/testnet/logs/seed.log
```

### Increment 4: Fill batch 1 (total: 56)

```bash
# Add remaining 10 to fill batch 1 (n53-n62)
scripts/stress-batch.sh add 1 10

sleep 180

# Checkpoint 2.4
echo "=== CHECKPOINT 2.4: 56 nodes $(date) ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

### Increment 5: Batch 2 start — +50 nodes (total: 106)

This is the critical threshold. v6 hit 9.1 GB at 106 nodes. With the 9 new fixes, it should be lower.

```bash
# Start batch 2 in full (n63-n112)
scripts/stress-batch.sh start 2

# Wait 5 minutes — 50 nodes all snap-syncing simultaneously creates a burst
sleep 300

# Checkpoint 2.5 — THE CRITICAL CHECKPOINT
echo "=== CHECKPOINT 2.5: 106 nodes $(date) ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log

# Compare to v6: was 9.1 GB at 106 nodes
# Expected with fixes: 5-8 GB
```

**Critical analysis at 106 nodes:**
```bash
# Check connection budget across a sample of nodes
for port in 8500 8513 8530 8550 8570 8590 8610; do
  peers=$(curl -sf --max-time 2 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | \
    python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(f'peers={r[\"peerCount\"]} syncing={r[\"syncing\"]}')" 2>/dev/null || echo "OFFLINE")
  echo "Port $port: $peers"
done

# Count HIGH PENDING warnings in seed log (key INC-I-014 indicator)
echo "HIGH PENDING warnings: $(grep -c 'HIGH PENDING' ~/testnet/logs/seed.log 2>/dev/null || echo 0)"
echo "HIGH EVICTION warnings: $(grep -c 'HIGH EVICTION' ~/testnet/logs/seed.log 2>/dev/null || echo 0)"
echo "Rejected fork tips: $(grep -c 'rejected_fork_tips' ~/testnet/logs/seed.log 2>/dev/null || echo 0)"

# Check for stuck nodes (height 0 or way behind)
for port in $(seq 8513 8612); do
  h=$(curl -sf --max-time 1 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
    python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])" 2>/dev/null || echo "OFFLINE")
  [[ "$h" == "OFFLINE" || "$h" == "0" ]] && echo "STUCK/OFFLINE: port $port h=$h"
done
```

**Success at 106 nodes**:
- RAM < 10 GB (improvement over v6's 9.1 GB despite additional fix overhead)
- Zero HIGH PENDING warnings (or < 5)
- Zero HIGH EVICTION CHURN warnings (or < 5)
- Less than 5% stuck nodes (< 5 of 100 stress nodes)
- Chain still advancing

**STOP if**: RAM > 15 GB at 106 nodes (regression from v6) or > 20% nodes stuck.

### Increment 6: +50 nodes (total: 156)

This is where v6 exploded (51.5 GB at 156). The 9 fixes target exactly this range.

```bash
# Start batch 3 (Tier 2 — bootstraps to random Tier 1 node)
scripts/stress-batch.sh start 3

# Wait 5 minutes
sleep 300

# Checkpoint 2.6 — THE V6 FAILURE POINT
echo "=== CHECKPOINT 2.6: 156 nodes $(date) ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log

# CRITICAL: If RAM > 30 GB at 156 nodes, the pending connection fix didn't work
# Expected: 15-25 GB (linear from 106-node measurement)
```

**Decision point at 156 nodes**:
- RAM < 20 GB: EXCELLENT. Proceed to 200+
- RAM 20-30 GB: GOOD. Proceed cautiously
- RAM 30-50 GB: CONCERNING. Wait 5 more minutes. If still growing, stop batch 3 and analyze
- RAM > 50 GB: REGRESSION. Stop immediately, save logs, analyze

### Increment 7: +50 nodes (total: 206)

```bash
scripts/stress-batch.sh start 4

sleep 300

echo "=== CHECKPOINT 2.7: 206 nodes $(date) ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

**v6 reference**: 206 nodes hit 15-28 GB after fixes (per commit message). Target is same or lower.

### Soak test at 200+ nodes

If 206 nodes are stable, let them run for 10 minutes and check for RAM drift:

```bash
# Record RAM every 60s for 10 minutes
for i in $(seq 1 10); do
  echo "Soak +${i}min: $(pgrep -c doli-node) nodes, $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}') RAM" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
  sleep 60
done
```

**Success**: RAM does not grow by more than 2 GB over 10 minutes (no monotonic leak).

---

## 4. Phase 3: Sync Testing

**Objective**: Verify snap sync and header-first sync work correctly under load. Nodes that restart should converge to the canonical chain.

**Prerequisites**: Phase 2 stable at 100+ nodes.

### Test 3.1: Kill and restart 10 random stress nodes (snap sync)

```bash
# Pick 10 random stress nodes from batch 1, stop them by killing PIDs
# We read PIDs from the batch pid file
PIDS=($(head -10 ~/testnet/nodes1/pids))
for pid in "${PIDS[@]}"; do
  kill "$pid" 2>/dev/null || true
  echo "Killed PID $pid"
done

# Wait 10 seconds
sleep 10

# Wipe their data (force snap sync on restart)
for i in $(seq 13 22); do
  rm -rf ~/testnet/nodes1/n${i}/data/blocks ~/testnet/nodes1/n${i}/data/state_db
done

# Restart them manually (can't use stress-batch.sh start because batch has other nodes)
for i in $(seq 13 22); do
  p2p=$((30300 + i))
  rpc=$((8500 + i))
  metrics=$((9000 + i))
  env RUST_LOG=warn ~/testnet/bin/doli-node \
    --network testnet \
    --data-dir ~/testnet/nodes1/n${i}/data \
    run \
    --producer \
    --producer-key ~/testnet/keys/producer_${i}.json \
    --p2p-port $p2p \
    --rpc-port $rpc \
    --metrics-port $metrics \
    --bootstrap "/ip4/127.0.0.1/tcp/30300" \
    --yes \
    --force-start >> ~/testnet/logs/nodes1/n${i}.log 2>&1 &
  echo "Restarted n${i} (PID $!)"
done

# Wait 3 minutes for snap sync
sleep 180
```

**Verify snap sync completed:**
```bash
SEED_H=$(curl -sf -X POST http://127.0.0.1:8500 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])")

echo "Seed height: $SEED_H"
for port in $(seq 8513 8522); do
  h=$(curl -sf --max-time 2 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
    python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])" 2>/dev/null || echo "OFFLINE")
  behind=$((SEED_H - ${h:-0}))
  echo "Port $port: h=$h (behind=$behind)"
done
```

**Success criteria**:
- All 10 restarted nodes reach within 5 blocks of seed within 3 minutes
- No node stuck at height 0
- No "post-snap header deadlock" in logs: `grep 'GetHeaders.*empty' ~/testnet/logs/nodes1/n{13..22}.log`

### Test 3.2: Restart a producer mid-block (header-first sync)

```bash
# Stop N2 (a genesis producer, launchd-managed)
scripts/testnet.sh stop n2

sleep 5

# N2's data is intact — on restart it should header-first sync the gap
scripts/testnet.sh start n2

sleep 60

# Check N2 caught up
curl -sf -X POST http://127.0.0.1:8502 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
  python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(f'N2: h={r[\"bestHeight\"]} slot={r[\"bestSlot\"]}')"
```

**Success criteria**: N2 syncs back within 60 seconds, no fork created.

### Test 3.3: Verify state root convergence after sync

```bash
# Compare state roots across seed, N1, N2, and a few stress nodes
for port in 8500 8501 8502 8513 8530 8550; do
  curl -sf --max-time 2 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}' | \
    python3 -c "
import sys,json
r=json.load(sys.stdin)['result']
print(f'Port $port: h={r[\"height\"]} root={r[\"stateRoot\"][:16]}... utxo={r[\"utxoHash\"][:16]}... ps={r[\"psHash\"][:16]}...')
" 2>/dev/null || echo "Port $port: OFFLINE"
done
```

**Success criteria**: All online nodes at the same height have identical state roots.

---

## 5. Phase 4: Fork Recovery

**Objective**: Induce natural forks by stopping multiple producers simultaneously, then verify all nodes converge on the same chain.

**Prerequisites**: Phase 2 and 3 passed.

### Test 4.1: Stop 3 of 5 genesis producers (minority remains)

```bash
# Stop N1, N3, N5 (keep N2, N4 producing)
scripts/testnet.sh stop n1
scripts/testnet.sh stop n3
scripts/testnet.sh stop n5

# Record the current heights and hashes
for port in 8500 8502 8504; do
  curl -sf --max-time 2 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
    python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(f'Port $port: h={r[\"bestHeight\"]} hash={r[\"bestHash\"][:16]}')" 2>/dev/null
done

# Wait 60 seconds (some slots will be missed)
sleep 60

# Restart the stopped producers
scripts/testnet.sh start n1
scripts/testnet.sh start n3
scripts/testnet.sh start n5

# Wait 120 seconds for convergence
sleep 120
```

**Verify convergence:**
```bash
scripts/status.sh

# All 6 genesis nodes should be at the same height and hash
for port in 8500 8501 8502 8503 8504 8505; do
  curl -sf --max-time 2 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
    python3 -c "import sys,json; r=json.load(sys.stdin)['result']; print(f'Port $port: h={r[\"bestHeight\"]} hash={r[\"bestHash\"][:16]}')" 2>/dev/null
done
```

**Success criteria**:
- All nodes converge to the same best hash within 2 minutes
- No stuck-fork conditions (N1/N3/N5 do not remain on a different chain)
- Chain is advancing after all producers restart

### Test 4.2: Network partition simulation (stop seed for 30s)

```bash
# Stop the seed — stress nodes lose their bootstrap peer
scripts/testnet.sh stop seed

echo "Seed stopped at $(date). Waiting 30s..."
sleep 30

# Restart seed
scripts/testnet.sh start seed
sleep 60

# Check if stress nodes reconnected
SEED_PEERS=$(curl -sf -X POST http://127.0.0.1:8500 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['result']['peerCount'])" 2>/dev/null)
echo "Seed peers after restart: $SEED_PEERS"
```

**Success criteria**: Seed regains peers within 60 seconds. Stress nodes that had seed as their only bootstrap peer recover through peer exchange.

### Test 4.3: Stop and restart an entire batch (mass recovery)

```bash
# Stop batch 1 (all 50 nodes)
scripts/stress-batch.sh stop 1

sleep 10

# Restart batch 1
scripts/stress-batch.sh start 1

# Wait 5 minutes for recovery
sleep 300

# Count synced vs stuck
SEED_H=$(curl -sf -X POST http://127.0.0.1:8500 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])")

synced=0 stuck=0
for port in $(seq 8513 8562); do
  h=$(curl -sf --max-time 1 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
    python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])" 2>/dev/null || echo "0")
  behind=$((SEED_H - ${h:-0}))
  if [[ $behind -le 10 ]]; then
    synced=$((synced + 1))
  else
    stuck=$((stuck + 1))
    echo "BEHIND: port $port h=$h (behind=$behind)"
  fi
done
echo "Batch 1 recovery: $synced synced, $stuck stuck (of 50)"
```

**Success criteria**: >= 90% of nodes sync within 5 minutes (>= 45 of 50).

---

## 6. Phase 5: Stress Ceiling

**Objective**: Push beyond 206 nodes to find the hard ceiling where either RAM exceeds budget or nodes stop converging.

**Prerequisites**: Phase 2-4 passed at 200+ nodes with RAM < 40 GB.

**CRITICAL**: Watch `stress-monitor.sh` output at all times during this phase. Be ready to `scripts/stress-batch.sh stop-all` at any moment.

### Add batch 5 (+50 = total 256)

```bash
scripts/stress-batch.sh start 5
sleep 300

echo "=== CEILING TEST: 256 nodes ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

### Add batch 6 (+50 = total 306)

```bash
scripts/stress-batch.sh start 6
sleep 300

echo "=== CEILING TEST: 306 nodes ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

At Tier 3 (batches 7+), bootstrap topology changes — nodes bootstrap through Tier 2 nodes instead of seed or Tier 1.

### Add batch 7 (Tier 3, +50 = total 356)

```bash
scripts/stress-batch.sh start 7
sleep 300

echo "=== CEILING TEST: 356 nodes ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Nodes: $(pgrep -c doli-node) RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

### Continue adding batches until a limit is hit

For each batch 8, 9, 10:
```bash
# Template — change batch number
scripts/stress-batch.sh start <BATCH>
sleep 300
echo "=== CEILING TEST: $(pgrep -c doli-node) nodes ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "RAM: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

### Ceiling detection criteria

The ceiling is reached when ANY of these occur:
1. **RAM exceeds 72 GB** (90% of watchdog threshold) -- approaching hard limit
2. **RAM grows > 5 GB per 5 minutes** with stable node count -- non-linear leak
3. **More than 20% of nodes stuck** (0 peers or > 50 blocks behind) -- network not converging
4. **Chain stops advancing** for > 2 minutes with producers online
5. **CPU consistently > 1400%** (87.5% of 16 cores) -- compute bottleneck
6. **stress-monitor.sh shows TCP connections > 50,000** -- FD exhaustion risk

### Record the ceiling

```bash
echo "=== CEILING REACHED ===" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Max stable nodes: $(pgrep -c doli-node)" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "RAM at ceiling: $(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.1f GB", sum/1048576}')" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Reason: <fill in>" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
echo "Time: $(date)" | tee -a ~/testnet/logs/stress-test-v7-baseline.log
```

---

## 7. Monitoring Commands

Run these in **separate terminal windows** during all phases.

### Terminal 1: Real-time resource monitor

```bash
cd /Users/isudoajl/ownCloud/Projects/localdoli
scripts/stress-monitor.sh 80 5
```

Output includes: node count, RAM (GB), CPU%, TCP connections, per-node RSS stats, seed height.
Yellow = 64 GB+, Red = 80 GB+ (triggers auto-kill).

### Terminal 2: Seed log (MEM-CONN-BUDGET)

```bash
tail -f ~/testnet/logs/seed.log | grep --line-buffered 'MEM-CONN\|MEM-BOOT\|HIGH PENDING\|HIGH EVICTION\|rejected_fork'
```

### Terminal 3: Quick health snapshot (run on demand)

```bash
cd /Users/isudoajl/ownCloud/Projects/localdoli
scripts/status.sh
```

### Terminal 4: Per-node RSS top-10 (run on demand)

```bash
ps aux | grep '[d]oli-node' | awk '{printf "%s\t%.0f MB\n", $2, $6/1024}' | sort -t$'\t' -k2 -rn | head -10
```

### Ad-hoc: Count stuck nodes (0 peers or height 0)

```bash
stuck=0
for port in $(seq 8513 8612); do
  info=$(curl -sf --max-time 1 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getNetworkInfo","params":{},"id":1}' 2>/dev/null)
  peers=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['peerCount'])" 2>/dev/null || echo "0")
  [[ "$peers" == "0" ]] && stuck=$((stuck + 1))
done
echo "Stuck (0 peers): $stuck / 100"
```

### Ad-hoc: RAM growth rate (sample every 30s for 5 min)

```bash
prev=0
for i in $(seq 1 10); do
  ram=$(ps aux | grep '[d]oli-node' | awk '{sum+=$6} END {printf "%.0f", sum/1024}')
  delta=$((ram - prev))
  prev=$ram
  echo "$(date +%H:%M:%S) RAM=${ram}MB delta=${delta}MB nodes=$(pgrep -c doli-node 2>/dev/null || echo 0)"
  sleep 30
done
```

---

## 8. Red Flags -- Immediate Stop Conditions

If ANY of these occur, immediately run `scripts/stress-batch.sh stop-all` and save logs.

| Red Flag | Detection | Action |
|----------|-----------|--------|
| RAM > 80 GB | stress-monitor.sh auto-detects | Watchdog kills automatically. Save logs. |
| RAM grows > 10 GB in 60 seconds | stress-monitor.sh WARNING | `scripts/stress-batch.sh stop-all` immediately. This is the INC-I-014 pattern. |
| TCP > 50,000 connections | stress-monitor.sh TCP count | `scripts/stress-batch.sh stop-all`. FD exhaustion imminent. |
| Seed height stalled > 2 minutes | stress-monitor.sh seed_h not changing | Check if producers are alive. If yes, network partition. |
| macOS kernel warning / swap pressure | Activity Monitor or `vm_stat` | `scripts/stress-batch.sh stop-all`. macOS compressed memory thrashing is fatal. |
| > 50% nodes at 0 peers | Ad-hoc stuck node check | Something broke in bootstrap/discovery. Stop, analyze. |
| Node crashes with SIGKILL (OOM) | `dmesg` or Console.app | macOS killed a node for memory. RAM budget exceeded. |
| Disk space < 20 GB free | `df -h` | `scripts/stress-batch.sh stop-all`. Clean data dirs. |

**Emergency stop command:**
```bash
cd /Users/isudoajl/ownCloud/Projects/localdoli && scripts/stop-all.sh
```

**NEVER use pkill or kill -9 on doli-node processes managed by launchd** -- launchd respawns them immediately, causing chain splits. Use the scripts.

---

## 9. Post-Test Analysis

After all testing phases complete (or after hitting a ceiling), perform these analyses.

### 9.1: Save comprehensive log snapshot

```bash
REPORT_DIR=~/testnet/logs/stress-v7-report-$(date +%Y%m%d-%H%M%S)
mkdir -p "$REPORT_DIR"

# Copy baseline measurements
cp ~/testnet/logs/stress-test-v7-baseline.log "$REPORT_DIR/"

# Copy seed log (MEM entries)
grep 'MEM-\|HIGH\|rejected_fork\|EVICTION' ~/testnet/logs/seed.log > "$REPORT_DIR/seed-mem-events.log" 2>/dev/null

# Copy stress monitor log
cp ~/testnet/logs/stress-monitor.log "$REPORT_DIR/" 2>/dev/null

# Sample 5 random stress node logs (first 100 lines each)
for i in 13 30 50 80 100; do
  head -100 ~/testnet/logs/nodes1/n${i}.log > "$REPORT_DIR/sample-n${i}.log" 2>/dev/null
done

echo "Report saved to $REPORT_DIR"
```

### 9.2: Analyze RAM scaling curve

From the baseline log, extract all checkpoint data and plot the scaling curve:
```bash
grep 'CHECKPOINT\|CEILING' ~/testnet/logs/stress-test-v7-baseline.log
```

Key questions:
- Is RAM growth linear? (nodes vs GB should be a straight line)
- Where does non-linearity begin? (that is the pending-connection threshold)
- What is the per-node marginal cost at each tier? (should be ~120 MB)

### 9.3: Check for INC-I-014 regression indicators

```bash
# Count high-pending warnings (should be near zero with pending_limit=5)
echo "HIGH PENDING events:"
grep -c 'HIGH PENDING' ~/testnet/logs/seed.log 2>/dev/null || echo 0

# Count eviction churn warnings (should be low with rate-limited eviction)
echo "HIGH EVICTION CHURN events:"
grep -c 'HIGH EVICTION' ~/testnet/logs/seed.log 2>/dev/null || echo 0

# Count rejected fork tip hits (new in 9-fix commit)
echo "Rejected fork tip cache hits:"
grep -c 'rejected_fork_tips' ~/testnet/logs/seed.log 2>/dev/null || echo 0

# Check if kademlia peer removal worked (no re-dial of evicted peers)
echo "DHT bootstrap skips (peer table full):"
grep -c 'DHT bootstrap skip' ~/testnet/logs/seed.log 2>/dev/null || echo 0
```

### 9.4: Verify no state divergence

After all tests, check that all online nodes at the same height have identical state:
```bash
# Get seed height
SEED_H=$(curl -sf -X POST http://127.0.0.1:8500 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":{},"id":1}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['result']['bestHeight'])")

SEED_ROOT=$(curl -sf -X POST http://127.0.0.1:8500 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}' | \
  python3 -c "import sys,json; print(json.load(sys.stdin)['result']['stateRoot'])")

echo "Seed: h=$SEED_H root=$SEED_ROOT"

# Check 20 random stress nodes
diverged=0
for port in 8513 8520 8530 8540 8550 8560 8570 8580 8590 8600 8515 8525 8535 8545 8555 8565 8575 8585 8595 8605; do
  info=$(curl -sf --max-time 2 -X POST http://127.0.0.1:$port \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"getStateRootDebug","params":{},"id":1}' 2>/dev/null)
  h=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['height'])" 2>/dev/null || echo "OFFLINE")
  root=$(echo "$info" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['stateRoot'])" 2>/dev/null || echo "OFFLINE")
  if [[ "$h" == "$SEED_H" && "$root" != "$SEED_ROOT" ]]; then
    echo "DIVERGED: port $port h=$h root=$root"
    diverged=$((diverged + 1))
  fi
done
echo "State divergence check: $diverged / 20 sampled"
```

**Any state divergence at the same height is a critical bug.**

### 9.5: Cleanup

```bash
cd /Users/isudoajl/ownCloud/Projects/localdoli

# Stop all stress nodes
scripts/stress-batch.sh stop-all

# Keep genesis nodes running (or stop them too if done)
# scripts/testnet.sh stop all

# Clean stress data (optional — saves disk)
# rm -rf ~/testnet/nodes1 ~/testnet/nodes2 ... ~/testnet/nodes10
```

---

## 10. Env Override Quick Reference

These environment variables tune network behavior without rebuilding. Set them in the shell before running `stress-batch.sh` or in the script's `env_prefix`.

| Variable | Default (testnet) | Description | When to tune |
|----------|-------------------|-------------|--------------|
| `DOLI_MAX_PEERS` | 25 | Max peers in gossip/Kademlia table | If stuck nodes need more peers, try 30. If RAM too high, try 15. |
| `DOLI_CONN_LIMIT` | max_peers + 10 (35) | Max established libp2p connections total | Raise if bootstrap fails. Lower if RAM leaks from ghost connections. |
| `DOLI_PENDING_LIMIT` | 5 | Max pending (handshaking) connections per direction | Raise (10-15) if bootstrap too slow. Lower (3) if RAM spike during connect storm. |
| `DOLI_IDLE_TIMEOUT_SECS` | 300 (testnet) | Seconds before idle connections are dropped | Lower (120) to free RAM faster. Higher (600) for connection stability. |
| `DOLI_YAMUX_WINDOW` | 262144 (256 KB) | Yamux receive window per connection | Lower (131072 = 128 KB) to halve per-connection RAM. Higher (524288 = 512 KB) for throughput. |
| `DOLI_BOOTSTRAP_SLOTS` | 10 | Temporary connection slots for DHT bootstrap peers | Lower (5) if too many bootstrap connections. Higher (20) if nodes can't find peers. |

**Example: Run with aggressive RAM saving:**
```bash
DOLI_MAX_PEERS=15 DOLI_CONN_LIMIT=25 DOLI_PENDING_LIMIT=3 DOLI_YAMUX_WINDOW=131072 DOLI_IDLE_TIMEOUT_SECS=120 \
  scripts/stress-batch.sh start 1 --count 10
```

**Example: Run with aggressive bootstrap (helps stuck nodes):**
```bash
DOLI_MAX_PEERS=30 DOLI_PENDING_LIMIT=10 DOLI_BOOTSTRAP_SLOTS=20 \
  scripts/stress-batch.sh start 1 --count 10
```

---

## Summary Table — Expected Outcomes

| Phase | Nodes | Expected RAM | Key Metric | Pass/Fail Threshold |
|-------|-------|-------------|------------|---------------------|
| 1 Baseline | 6 | < 1 GB | All synced, roots match | Any failure = STOP |
| 2.1 | 16 | < 3 GB | All synced | > 5% stuck = STOP |
| 2.3 | 46 | < 8 GB | RAM linear | Non-linear growth = investigate |
| 2.5 (v6 match) | 106 | < 10 GB | Beats v6's 9.1 GB | > 15 GB = regression |
| 2.6 (v6 fail point) | 156 | < 25 GB | Beats v6's 51.5 GB | > 35 GB = pending fix failed |
| 2.7 | 206 | < 30 GB | Matches commit claim 15-28 GB | > 40 GB = investigate |
| 3 Sync | N/A | stable | All restarted nodes sync | > 10% stuck = sync bug |
| 4 Fork | N/A | stable | All converge to same hash | Any divergence = critical |
| 5 Ceiling | 300+ | < 72 GB | Find the limit | RAM watchdog trigger = ceiling found |
