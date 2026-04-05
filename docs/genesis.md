# DOLI Genesis & Full Chain Reset

> Last updated: 2026-03-23 | Layout v9

Complete procedure for resetting and relaunching DOLI networks from a new genesis.

---

## Network Topology

### Mainnet (5 servers: ai1, ai2, ai3, ai4, ai5)

| Server | Role | Nodes |
|--------|------|-------|
| ai1 | Seed + Producers | Mainnet seed1 + N1, N2, N3 |
| ai2 | Seed + Producers + Build | Mainnet seed2 + N4, N5 |
| ai3 | Seed + Producers | Mainnet seed3 + SANTIAGO, IVAN |
| ai4 | Producers | N6, N7, N8 |
| ai5 | Producers | N9, N10, N11, N12 |

### Testnet (3 servers: ai1, ai2, ai5)

| Server | Role | Nodes |
|--------|------|-------|
| ai1 | Seed + Producers | Testnet seed1 + NT1-NT5 |
| ai2 | Seed | Testnet seed2 |
| ai5 | Producers | NT6-NT12 |

### Seed Topology (6 seeds total)

| Seed | Server | Network | P2P Port | RPC Port | Data Dir |
|------|--------|---------|----------|----------|----------|
| Seed1 | ai1 | Mainnet | 30300 | 8500 | `/mainnet/seed/data` |
| Seed1 | ai1 | Testnet | 40300 | 18500 | `/testnet/seed/data` |
| Seed2 | ai2 | Mainnet | 30300 | 8500 | `/mainnet/seed/data` |
| Seed2 | ai2 | Testnet | 40300 | 18500 | `/testnet/seed/data` |
| Seed3 | ai3 | Mainnet | 30300 | 8500 | `/mainnet/seed/data` |
| Seed3 | ai3 | Testnet | 40300 | 18500 | `/testnet/seed/data` |

Service names: `doli-mainnet-seed`, `doli-testnet-seed` on each server.

---

## Genesis Timestamp Sources

### Mainnet-only reset (2 files)

| # | File | Field | Notes |
|---|------|-------|-------|
| 1 | `chainspec.mainnet.json` | `genesis.timestamp` + `genesis.message` | Canonical mainnet chainspec |
| 2 | `crates/core/src/consensus/constants.rs` | `GENESIS_TIME` (line ~25) | **LOCKED at runtime** — cannot be overridden |

Also update `chainspec.rs:ChainSpec::mainnet()` genesis message to match JSON.

### Testnet-only reset (2 files)

| # | File | Field | Notes |
|---|------|-------|-------|
| 1 | `chainspec.testnet.json` | `genesis.timestamp` + `genesis.message` | Canonical testnet chainspec |
| 2 | `crates/core/src/network_params/defaults.rs` | Testnet `genesis_time:` (line ~97) | Hardcoded `u64`, must match JSON |

### Full reset (all 4 files)

Update all 4 files above.

**How it flows:**
- Mainnet `defaults.rs` uses `consensus::GENESIS_TIME` (not hardcoded) — updating `constants.rs` covers mainnet defaults automatically.
- Testnet `defaults.rs` has its own hardcoded value — must be updated separately.
- `genesis_hash = BLAKE3(timestamp || network_id || slot_duration || message)` — included in every block header.
- Two unit tests verify sync: `cargo test -p doli-core test_genesis_time`

---

## Mainnet Chain Reset Procedure

### Phase 1: Stop & Wipe (ai1, ai2, ai3 only)

**1. Inventory ALL running processes on ALL servers:**
```bash
# On EACH of ai1, ai2, ai3, ai4, ai5:
for server in ai1 ai2 ai3 ai4 ai5; do
  echo "=== $server ===" && ssh $server "pgrep -a doli-node | grep mainnet || echo 'clean'"
done
```

**2. Stop mainnet services:**
```bash
# ai1 — seed + N1-N3:
sudo systemctl stop doli-mainnet-seed doli-mainnet-{n1,n2,n3}

# ai2 — seed + N4-N5:
sudo systemctl stop doli-mainnet-seed doli-mainnet-{n4,n5}

# ai3 — seed + named producers:
sudo systemctl stop doli-mainnet-seed doli-mainnet-santiago doli-mainnet-ivan

# ai4 — N6-N8:
sudo systemctl stop doli-mainnet-{n6,n7,n8}

# ai5 — N9-N12:
sudo systemctl stop doli-mainnet-{n9,n10,n11,n12}
```

**3. Verify nothing remains:**
```bash
for server in ai1 ai2 ai3 ai4 ai5; do
  echo "=== $server ===" && ssh $server "pgrep -a doli-node | grep mainnet || echo 'clean'"
done
```

**4. Wipe ALL mainnet data dirs:**
```bash
# ai1 — seed + N1-N3:
for node in /mainnet/seed /mainnet/n{1..3}; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done

# ai2 — seed + N4-N5:
for node in /mainnet/seed /mainnet/n{4..5}; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done

# ai3 — seed + named producers:
for node in /mainnet/seed /mainnet/santiago /mainnet/ivan; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done

# ai4 — N6-N8:
for node in /mainnet/n{6..8}; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done

# ai5 — N9-N12:
for node in /mainnet/n{9..12}; do
  find "$node/data" -mindepth 1 -delete 2>/dev/null
  rm -rf "$node"/{chain_state.bin,producers.bin,utxo.bin,producer_gset.bin,peers.cache,producer.lock,chainspec.json,node_key,maintainer_state.bin,blocks,signed_slots.db,utxo_rocks,state_db} 2>/dev/null
done
```

**KEEP ONLY**: `keys/` directories.

**5. Verify wipe:**
```bash
for server in ai1 ai2 ai3 ai4 ai5; do
  echo "=== $server ===" && ssh $server 'for node in /mainnet/seed /mainnet/n{1..12}; do [ -d "$node" ] && echo "$node: $(ls "$node/" 2>/dev/null | tr "\n" " ")"; done'
done
```
Must show only `data keys` (data dir empty).

If `signed_slots.db` survives anywhere, nodes will hit slashing protection and refuse to produce.

---

### Phase 2: Update Genesis (local machine)

**6. Calculate new genesis timestamp** = NOW + 15 minutes (enough for compile + deploy):
```bash
echo $(( $(date +%s) + 900 ))
```

**7. Update 3 files** with the new timestamp:
- `chainspec.mainnet.json` → `genesis.timestamp` + `genesis.message`
- `crates/core/src/consensus/constants.rs` → `GENESIS_TIME`
- `crates/core/src/chainspec.rs` → `ChainSpec::mainnet()` genesis message

**8. Update genesis hash** — the hash changes when timestamp changes:
```bash
cargo test -p doli-core --lib -- test_mainnet_genesis_hash_hardcoded 2>&1 | grep "Got "
```
Copy the new hash from the output and update `crates/core/src/chainspec.rs` line ~352 (`test_mainnet_genesis_hash_hardcoded`). Verify:
```bash
cargo test -p doli-core --lib -- test_mainnet_genesis_hash_hardcoded
```
Must pass. If it fails, the binary will have an inconsistent genesis hash and nodes will reject each other's blocks.

**9. Run sync tests:**
```bash
cargo test -p doli-core test_genesis_time
```

**10. Commit & push:**
```bash
git add chainspec.mainnet.json crates/core/src/consensus/constants.rs crates/core/src/chainspec.rs
git commit --author "Ivan D. Lozada <ivan@doli.network>" \
  -m "chain reset: mainnet genesis TIMESTAMP (YYYY-MM-DD)"
git push
```

---

### Phase 3: Compile & Deploy (ai2 → ai1, ai3)

**11. Compile on ai2:**
```bash
ssh ai2 "source ~/.cargo/env && cd ~/repos/doli && git pull && cargo build --release"
```

**12. Record checksums:**
```bash
ssh ai2 "md5sum ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli"
```

**12. Deploy on ai2 (local):**
```bash
ssh ai2 "sudo cp ~/repos/doli/target/release/doli-node /mainnet/bin/doli-node && sudo cp ~/repos/doli/target/release/doli /mainnet/bin/doli"
```

**13. Transfer to ai1 and ai3:**
```bash
for server in ai1 ai3; do
  ssh ai2 "cat ~/repos/doli/target/release/doli-node" | ssh $server "cat > /tmp/doli-node && sudo cp /tmp/doli-node /mainnet/bin/doli-node && sudo chmod +x /mainnet/bin/doli-node && rm /tmp/doli-node"
  ssh ai2 "cat ~/repos/doli/target/release/doli" | ssh $server "cat > /tmp/doli && sudo cp /tmp/doli /mainnet/bin/doli && sudo chmod +x /mainnet/bin/doli && rm /tmp/doli"
done
```

**14. Verify binaries on ALL servers (ai1-ai5) with `ls -l` and `md5sum`:**

Even servers that are not part of this network reset may have stale binaries that get used later. Verify everywhere.

```bash
echo "=== BUILD ===" && ssh ai2 "md5sum ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli"
echo ""
for server in ai1 ai2 ai3 ai4 ai5; do
  echo "=== $server ==="
  ssh $server "ls -l /mainnet/bin/doli-node /mainnet/bin/doli /testnet/bin/doli-node /testnet/bin/doli 2>/dev/null"
  ssh $server "md5sum /mainnet/bin/doli-node /mainnet/bin/doli /testnet/bin/doli-node /testnet/bin/doli 2>/dev/null"
  echo ""
done
```

Check:
- **Date**: `ls -l` timestamp must be from the current deploy (not yesterday/last week)
- **md5**: must match build output for every binary on every server
- **Stale binaries**: if any server shows an old date or mismatched md5, redeploy before proceeding

---

### Phase 4: Start & Verify

**DO NOT start nodes until Phases 1-3 are fully verified.**

**15. Start seeds first (ai1, ai2, ai3):**
```bash
for server in ai1 ai2 ai3; do
  ssh $server "sudo systemctl start doli-mainnet-seed"
done
```
Wait ~10 seconds for seeds to peer.

**16. Start producers (ai1, ai2, ai3, ai4, ai5):**
```bash
ssh ai1 "sudo systemctl start doli-mainnet-{n1,n2,n3}"
ssh ai2 "sudo systemctl start doli-mainnet-{n4,n5}"
ssh ai3 "sudo systemctl start doli-mainnet-santiago doli-mainnet-ivan"
ssh ai4 "sudo systemctl start doli-mainnet-{n6,n7,n8}"
ssh ai5 "sudo systemctl start doli-mainnet-{n9,n10,n11,n12}"
```

**17. Health check:**
```bash
ssh ai2 'curl -s -X POST http://127.0.0.1:8500/ -H "Content-Type: application/json" -d "{\"jsonrpc\":\"2.0\",\"method\":\"getChainInfo\",\"params\":[],\"id\":1}" | jq .result'
```

| Check | Expected |
|-------|----------|
| Before genesis time | `h=0`, `s=0` |
| After genesis time | `h` and `s` incrementing |
| `h=0` after genesis passed | Check logs for `SLASHING PROTECTION` — `signed_slots.db` wasn't wiped |

---

## Testnet Chain Reset Procedure

Same structure but different servers:

- **Stop/wipe**: ai1 (NT1-NT5 + testnet seed), ai2 (testnet seed), ai3 (testnet seed), ai5 (NT6-NT12)
- **Update**: `chainspec.testnet.json` + `defaults.rs` testnet `genesis_time`
- **Deploy**: ai1, ai2, ai3, ai5 (`/testnet/bin/`)
- **Start seeds**: ai1, ai2, ai3
- **Start producers**: ai1 (NT1-NT5), ai5 (NT6-NT12)

---

## Consensus Parameters (from code)

| Parameter | Value | Source |
|-----------|-------|--------|
| Slot duration | 10s | `constants.rs:SLOT_DURATION` |
| Epoch length | 360 blocks (1 hour) | `constants.rs:SLOTS_PER_EPOCH` |
| Block reward | 1 DOLI (100,000,000 atomic) | `constants.rs:INITIAL_REWARD` |
| Bond unit | 10 DOLI (1,000,000,000 atomic) | `constants.rs:BOND_UNIT` |
| Vesting | 4 years (3,153,600 slots/quarter) | `constants.rs:VESTING_QUARTER_SLOTS` |
| Max bonds per producer | 3,000 (30,000 DOLI max) | `constants.rs:MAX_BONDS_PER_PRODUCER` |
| Halving interval | ~4 years (12,614,400 blocks) | `constants.rs:BLOCKS_PER_ERA` |
| Total supply | 25,228,800 DOLI | `constants.rs:TOTAL_SUPPLY` |
| Unbonding period | ~7 days (60,480 blocks) | `constants.rs:UNBONDING_PERIOD` |
| Genesis open registration | 1 hour (360 blocks) | `defaults.rs:genesis_blocks` |
| Coinbase maturity | 6 blocks | `constants.rs:COINBASE_MATURITY` |

### Testnet Overrides

| Parameter | Mainnet | Testnet |
|-----------|---------|---------|
| Bond unit | 10 DOLI (1,000,000,000) | 1 DOLI (100,000,000) |
| P2P port | 30300 | 40300 |
| RPC port | 8500 | 18500 |
| Metrics port | 9000 | 19000 |
| Vesting quarter | 1 year (3,153,600 slots) | 6 hours (2,160 slots) |
| Unbonding | 60,480 blocks (~7 days) | 72 blocks (2 epochs) |

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Node stuck at h=0 after genesis | `signed_slots.db` not wiped | Stop, `find <node>/data -mindepth 1 -delete`, restart |
| Nodes can't peer | Seeds not started first, or different genesis hash | Verify all 3 seeds running, check genesis timestamp matches |
| "SLASHING PROTECTION" in logs | Stale `signed_slots.db` from previous chain | Wipe `<node>/data/signed_slots.db` |
| Different genesis hash across nodes | Timestamp source not updated | Grep for old timestamp, rebuild, redeploy |
| md5 mismatch after transfer | Incomplete SSH transfer | Re-transfer binary, verify again |
| "Text file busy" on deploy | Stale doli-node process still running | `pgrep -a doli-node` on that server, kill it, then deploy |

---

**Document Version**: 4.0
**Author**: DOLI Core Team
