# DOLI Genesis & Full Chain Reset

> Last updated: 2026-03-15 | Layout v6 (5 servers)

Complete procedure for resetting and relaunching both DOLI networks from a new genesis.

---

## Infrastructure

| Server | Network | Nodes | Binary Paths |
|--------|---------|-------|--------------|
| ai1 | Testnet | NT1-NT5 + testnet seed + mainnet seed | `/testnet/bin/`, `/mainnet/bin/` |
| ai2 | Mainnet | N1-N5 + mainnet seed + testnet seed + swap bot + explorer | `/mainnet/bin/`, `/testnet/bin/` |
| ai3 | Both | Mainnet seed3 + Testnet seed3 + SANTIAGO (mainnet producer) | `/mainnet/bin/`, `/testnet/bin/` |
| ai4 | Mainnet | N6-N12 | `/mainnet/bin/` |
| ai5 | Testnet | NT6-NT12 | `/testnet/bin/` |

### Seed Topology (6 seeds total)

| Seed | Server | Network | P2P Port | RPC Port | Data Dir | Archive Dir |
|------|--------|---------|----------|----------|----------|-------------|
| Seed1 | ai1 | Mainnet | 30300 | 8500 | `/mainnet/seed/data` | `/mainnet/seed/blocks` |
| Seed1 | ai1 | Testnet | 40300 | 18500 | `/testnet/seed/data` | `/testnet/seed/blocks` |
| Seed2 | ai2 | Mainnet | 30300 | 8500 | `/mainnet/seed/data` | `/mainnet/seed/blocks` |
| Seed2 | ai2 | Testnet | 40300 | 18500 | `/testnet/seed/data` | `/testnet/seed/blocks` |
| Seed3 | ai3 | Mainnet | 30300 | 8500 | `/mainnet/seed/data` | `/mainnet/seed/blocks` |
| Seed3 | ai3 | Testnet | 40300 | 18500 | `/testnet/seed/data` | `/testnet/seed/blocks` |

Service names: `doli-mainnet-seed`, `doli-testnet-seed` on each server.
SANTIAGO service: `doli-mainnet-santiago` (ai3, ports P2P=30313 RPC=8513 Metrics=9013).

---

## Genesis Timestamp Sources

There are exactly **4 files** that must be updated for a genesis reset. Miss one and nodes will compute different genesis hashes or tests will fail.

| # | File | Field | Notes |
|---|------|-------|-------|
| 1 | `chainspec.mainnet.json` | `genesis.timestamp` + `genesis.message` | Canonical mainnet chainspec |
| 2 | `chainspec.testnet.json` | `genesis.timestamp` + `genesis.message` | Canonical testnet chainspec |
| 3 | `crates/core/src/consensus/constants.rs` | `GENESIS_TIME` (line ~25) | **Mainnet is LOCKED at runtime** — cannot be overridden by env vars |
| 4 | `crates/core/src/network_params/defaults.rs` | Testnet `genesis_time:` (line ~97) | Hardcoded `u64`, must match chainspec.testnet.json |

**How it flows:**
- Mainnet `defaults.rs` uses `consensus::GENESIS_TIME` (not hardcoded) — so updating `constants.rs` covers mainnet defaults automatically.
- Testnet `defaults.rs` has its own hardcoded value — must be updated separately.
- `genesis_hash = BLAKE3(timestamp || network_id || slot_duration || message)` — included in every block header. Different timestamp = incompatible chain.
- Two unit tests (`test_genesis_time_matches_chainspec`, `test_testnet_genesis_time_matches_chainspec`) verify the JSON and code stay in sync.

### Priority Override Chain (testnet/devnet only)

1. `DOLI_GENESIS_TIME` env var (highest)
2. `~/.doli/{network}/.env` file
3. Chainspec JSON
4. Hardcoded defaults in `defaults.rs` (lowest)

Mainnet ignores all overrides — `GENESIS_TIME` constant is the only source.

---

## Full Chain Reset Procedure

### Phase 1: Stop & Wipe (ALL 3 servers)

**1. Inventory all running processes:**
```bash
# On EACH of ai1, ai2, ai3:
pgrep -a doli-node
```
Record everything. Don't rely on service names — standby nodes may be running too.

**2. Verify SSH to all 3 servers** before stopping anything. If a server is unreachable, decide whether to proceed BEFORE stopping nodes.

**3. Stop ALL doli services simultaneously:**
```bash
# ai1 (testnet NT1-NT5 + both seeds):
sudo systemctl stop doli-testnet-{nt1,nt2,nt3,nt4,nt5}
sudo systemctl stop doli-testnet-seed doli-mainnet-seed

# ai2 (mainnet N1-N5 + both seeds):
sudo systemctl stop doli-mainnet-{n1,n2,n3,n4,n5}
sudo systemctl stop doli-mainnet-seed doli-testnet-seed

# ai3 (both seeds + SANTIAGO):
sudo systemctl stop doli-mainnet-seed doli-testnet-seed doli-mainnet-santiago

# ai4 (mainnet N6-N12):
sudo systemctl stop doli-mainnet-{n6,n7,n8,n9,n10,n11,n12}

# ai5 (testnet NT6-NT12):
sudo systemctl stop doli-testnet-{nt6,nt7,nt8,nt9,nt10,nt11,nt12}
```

**4. Verify nothing remains:**
```bash
# On EACH server:
pgrep -a doli-node   # must return nothing
```

**5. Wipe ALL data dirs.** The runtime data lives inside `<node>/data/`:

```bash
# ai1 — testnet NT1-NT5 + both seeds:
for node in /testnet/nt{1..5} /testnet/seed /mainnet/seed; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done

# ai2 — mainnet N1-N5 + both seeds:
for node in /mainnet/n{1..5} /mainnet/seed /testnet/seed; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done

# ai3 — both seeds + SANTIAGO:
for node in /mainnet/seed /testnet/seed /mainnet/santiago; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done

# ai4 — mainnet N6-N12:
for node in /mainnet/n{6..12}; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done

# ai5 — testnet NT6-NT12:
for node in /testnet/nt{6..12}; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done
```

**KEEP ONLY**: `keys/` directories. Binaries are shared at `/mainnet/bin/` and `/testnet/bin/`.

**6. Wipe archive dirs** (optional — for a fully clean restart):
```bash
# On ALL 3 servers:
rm -rf /mainnet/seed/blocks/* /testnet/seed/blocks/*
```

**7. Verify wipe:**
```bash
# Spot-check on each server:
ls <node>/data/    # must be empty
ls <node>/         # must show only keys/ (and empty data/)
```

If `signed_slots.db` survives anywhere, nodes will hit slashing protection and refuse to produce.

---

### Phase 2: Update Genesis

**8. Calculate new genesis timestamp** = NOW + 6 minutes:
```bash
echo $(( $(date +%s) + 360 ))
```

**9. Update ALL 4 sources** with the new timestamp:

```bash
# 1. chainspec.mainnet.json → genesis.timestamp + genesis.message
# 2. chainspec.testnet.json → genesis.timestamp + genesis.message
# 3. crates/core/src/consensus/constants.rs → GENESIS_TIME constant
# 4. crates/core/src/network_params/defaults.rs → testnet genesis_time field
```

**10. Grep for the OLD timestamp** — 0 matches must remain:
```bash
grep -r "OLD_TIMESTAMP" --include="*.rs" --include="*.json" .
```

**11. Run sync tests:**
```bash
cargo test test_genesis_time_matches_chainspec test_testnet_genesis_time_matches_chainspec
```
Both must pass.

**12. Commit & push** (no sensitive info):
```bash
git add chainspec.mainnet.json chainspec.testnet.json \
  crates/core/src/consensus/constants.rs \
  crates/core/src/network_params/defaults.rs
git commit --author "Ivan D. Lozada <ivan@doli.network>" \
  -m "chain reset: new genesis $(date -u +%Y-%m-%d)"
git push
```

---

### Phase 3: Compile & Deploy (ALL 3 servers)

**13. Compile on ai2** (dedicated CPU, 0% steal):
```bash
cd ~/repos/doli && git pull && cargo build --release
```

**14. Record checksums:**
```bash
md5sum target/release/doli-node target/release/doli
```

**15. Deploy on ai2** (local):
```bash
cp target/release/doli-node /mainnet/bin/doli-node
cp target/release/doli      /mainnet/bin/doli
cp target/release/doli-node /testnet/bin/doli-node
cp target/release/doli      /testnet/bin/doli
```

**16. Transfer to ai1:**
```bash
ssh ai1 "cat > /mainnet/bin/doli-node" < target/release/doli-node
ssh ai1 "cat > /mainnet/bin/doli"      < target/release/doli
ssh ai1 "cat > /testnet/bin/doli-node" < target/release/doli-node
ssh ai1 "cat > /testnet/bin/doli"      < target/release/doli
ssh ai1 "chmod +x /mainnet/bin/{doli-node,doli} /testnet/bin/{doli-node,doli}"
```

**17. Transfer to ai3:**
```bash
ssh ai3 "cat > /mainnet/bin/doli-node" < target/release/doli-node
ssh ai3 "cat > /mainnet/bin/doli"      < target/release/doli
ssh ai3 "cat > /testnet/bin/doli-node" < target/release/doli-node
ssh ai3 "cat > /testnet/bin/doli"      < target/release/doli
ssh ai3 "chmod +x /mainnet/bin/{doli-node,doli} /testnet/bin/{doli-node,doli}"
```

**18. Verify checksums** on all 3 servers (12 binaries total):
```bash
# ai2 (local):
md5sum /mainnet/bin/doli-node /mainnet/bin/doli /testnet/bin/doli-node /testnet/bin/doli
# ai1:
ssh ai1 "md5sum /mainnet/bin/doli-node /mainnet/bin/doli /testnet/bin/doli-node /testnet/bin/doli"
# ai3:
ssh ai3 "md5sum /mainnet/bin/doli-node /mainnet/bin/doli /testnet/bin/doli-node /testnet/bin/doli"
```
All 12 must match build output.

---

### Phase 4: Start & Verify

**DO NOT start nodes until Phases 1-3 are fully verified** — binaries match, data dirs empty, no stale `signed_slots.db`.

**19. Start seeds first** (all 3 servers, both networks):
```bash
# ai1:
sudo systemctl start doli-mainnet-seed doli-testnet-seed
# ai2:
sudo systemctl start doli-mainnet-seed doli-testnet-seed
# ai3:
sudo systemctl start doli-mainnet-seed doli-testnet-seed
```
Wait ~10 seconds for seeds to peer with each other.

**20. Start genesis producers** (only nodes with genesis wallets — SANTIAGO cannot start here):
```bash
# ai2 — mainnet (active first, then standby):
sudo systemctl start doli-mainnet-{n1,n2,n3,n4,n5}
# (then n6-n12 as needed)

# ai1 — testnet (active first, then standby):
sudo systemctl start doli-testnet-{nt1,nt2,nt3,nt4,nt5}
# (then nt6-nt12 as needed)
```

**21. Immediate health check** via `getChainInfo`:
```bash
curl -s -X POST http://127.0.0.1:<port>/ \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getChainInfo","params":[],"id":1}' | jq .result
```

| Check | Expected |
|-------|----------|
| Before genesis time | `h=0`, `s=0` |
| After genesis time | `h` and `s` incrementing from 1 |
| `h=0` after genesis passed | Check logs for `SLASHING PROTECTION` — means `signed_slots.db` wasn't wiped |

**22. Full status check:**
```bash
scripts/status.sh all
```

---

## Consensus Parameters (from code)

| Parameter | Value | Source |
|-----------|-------|--------|
| Slot duration | 10s | `constants.rs:SLOT_DURATION` |
| Epoch length | 360 blocks (1 hour) | `constants.rs:SLOTS_PER_EPOCH` |
| Block reward | 1 DOLI (100,000,000 atomic) | `constants.rs:INITIAL_REWARD` |
| Bond unit | 10 DOLI (1,000,000,000 atomic) | `constants.rs:BOND_UNIT` |
| Max bonds per producer | 3,000 (30,000 DOLI max) | `constants.rs:MAX_BONDS_PER_PRODUCER` |
| Halving interval | ~4 years (12,614,400 blocks) | `constants.rs:BLOCKS_PER_ERA` |
| Total supply | 25,228,800 DOLI | `constants.rs:TOTAL_SUPPLY` |
| Unbonding period | ~7 days (60,480 blocks) | `constants.rs:UNBONDING_PERIOD` |
| Genesis open registration | 1 hour (360 blocks) | `defaults.rs:genesis_blocks` |
| Bootstrap grace period | 15 seconds | `constants.rs:BOOTSTRAP_GRACE_PERIOD_SECS` |
| Fallback timeout | 2s per rank, 5 ranks | `constants.rs:FALLBACK_TIMEOUT_MS` |
| Coinbase maturity | 6 blocks | `constants.rs:COINBASE_MATURITY` |

### Testnet Overrides

| Parameter | Mainnet | Testnet |
|-----------|---------|---------|
| P2P port | 30300 | 40300 |
| RPC port | 8500 | 18500 |
| Metrics port | 9000 | 19000 |
| Vesting quarter | 1 year (3,153,600 slots) | 6 hours (2,160 slots) |
| Bootstrap nodes | seed1/seed2/seeds.doli.network | bootstrap1/bootstrap2/seeds.testnet.doli.network |

### Genesis Hash

```
genesis_hash = BLAKE3(timestamp_le || network_id_le || slot_duration_le || message_bytes)
```

Included in every block header (v2+). Nodes reject blocks with a different genesis hash immediately.

---

## DNS

| Record | Purpose |
|--------|---------|
| `seed1.doli.network` | ai1 mainnet P2P |
| `seed2.doli.network` | ai2 mainnet P2P |
| `seeds.doli.network` | Round-robin ai1+ai2+ai3 (mainnet) |
| `bootstrap1.testnet.doli.network` | ai1 testnet P2P |
| `bootstrap2.testnet.doli.network` | ai2 testnet P2P |
| `seeds.testnet.doli.network` | Round-robin ai1+ai2+ai3 (testnet) |

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Node stuck at h=0 after genesis | `signed_slots.db` not wiped | Stop, `find <node>/data -mindepth 1 -delete`, restart |
| Nodes can't peer | Seeds not started first, or different genesis hash | Verify all 6 seeds running, check genesis timestamp matches across all 4 sources |
| "SLASHING PROTECTION" in logs | Stale `signed_slots.db` from previous chain | Wipe `<node>/data/signed_slots.db` |
| Different genesis hash across nodes | One of the 4 timestamp sources wasn't updated | Grep for old timestamp, rebuild, redeploy |
| md5 mismatch after transfer | Incomplete SSH transfer | Re-transfer binary, verify again |

---

**Document Version**: 3.0
**Author**: DOLI Core Team
