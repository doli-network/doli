# Post-Mortem: Total Network Partition (2026-03-11)

**Severity:** Critical — chain state lost, required chain reset
**Duration:** ~3 hours
**Impact:** Original chain (E22, 8192 DOLI, 14 producers) destroyed. Network restarted from new genesis.

---

## Timeline

| Time | Event |
|------|-------|
| T+0 | Chain reset performed. Data wiped on all nodes. Nodes restarted. |
| T+5m | Monitoring shows "unified genesis", all nodes at same height. Appears healthy. |
| T+30m | Fork detected: 10 nodes on hash A, 4 nodes on hash B, 1 stuck at genesis. |
| T+35m | Investigation reveals **every node has 0 peers**. Network is 15 isolated solo chains. |
| T+40m | Diagnosis: service files inconsistent. 7 nodes lack `--bootstrap` flag. |
| T+45m | Recovery attempt: wipe dead nodes, fix service files, restart one-by-one. |
| T+60m | **Recovery makes it worse.** Wiped nodes snap-sync to a solo chain (5 genesis producers only). |
| T+70m | N2 and Seed1 also fall to height 0 (peer cache poisoning). More wipes. |
| T+90m | Original chain (14 producers, 8192 DOLI, E22) confirmed lost. No node has it. |
| T+120m | Decision: accept loss, let current chain grow. New chain recovers to E9. |

## Root Cause

### Primary: Inconsistent service files

Two generations of systemd service files coexisted:

```
Old-style (N1-N3, N6, N8, seeds):
  --bootstrap /dns4/seed1.doli.network/tcp/30300
  --bootstrap /dns4/seed2.doli.network/tcp/30300
  --yes --force-start

New-style (N4, N5, N7, N9, N10, N11, N12):
  --network mainnet
  (NO --bootstrap, NO --force-start, NO logging)
```

The new-style nodes had **zero peer discovery capability**. After a data wipe (which destroys the libp2p peer identity and cached peer store), these nodes could not find any other node on the network.

### Secondary: libp2p peer ID mismatch cascade

After the data wipe, every node generated a new peer ID. Nodes that DID have `--bootstrap` still failed to connect because:

1. Node A connects to seed, learns about Node B's **old** peer ID via peer exchange
2. Node A tries to connect to Node B at its address but finds a **new** peer ID
3. libp2p rejects the connection: `Unexpected peer ID`
4. This propagates — every cached peer ID is wrong, every connection attempt fails

### Tertiary: Recovery destroyed the original chain

Wiping data on "dead" nodes to restart them also destroyed the chain state on those nodes. The nodes then snap-synced from whichever peer they found — typically a solo chain produced by a small subset of genesis producers. The original chain (with all registration transactions, epoch rewards, and bond operations) was irretrievably lost.

### Contributing: Monitoring created false confidence

The dashboard aggregated height/producer data from 15 independent solo chains. Because all chains started from the same genesis at the same time, they were at roughly the same height, giving the appearance of a healthy unified network. The fork detection caught the hash mismatch, but the underlying problem (total partition) was much worse than the "4 nodes forked" that was visible.

## Lessons Learned

### 1. Service files are consensus-critical infrastructure

A missing `--bootstrap` flag is as dangerous as a wrong genesis hash. Service files must be:
- Generated from a single template (not hand-edited)
- Validated before every chain reset
- Identical in structure across all nodes (only ports/paths differ)

### 2. Never wipe data as a first recovery step

Wiping data destroys information that may be irrecoverable. The correct sequence:
1. **Diagnose** — check peers, logs, state roots
2. **Isolate** — stop the broken node
3. **Preserve** — back up data before any destructive action
4. **Fix** — address the root cause (service file, peer config, etc.)
5. **Restart** — with the fix applied
6. **Wipe only as last resort** — and only after confirming healthy peers exist to sync from

### 3. Verify peer connectivity before declaring a reset successful

After any chain reset, the FIRST check must be: does every node have >= 1 peer? If any node has 0 peers, STOP. Do not proceed until peer connectivity is confirmed across all nodes.

### 4. A monitoring dashboard is not a health check

Aggregated metrics from isolated nodes can look healthy. A proper post-reset health check must verify:
- [ ] All nodes have peers (`getPeerInfo` returns non-empty)
- [ ] All nodes share the same genesis hash
- [ ] All nodes share the same block 1 hash
- [ ] Block heights are advancing and converging
- [ ] No "Unexpected peer ID" errors in logs

### 5. Solo chains are indistinguishable from healthy chains

A node producing blocks alone looks exactly like a node in a healthy network — same height progression, same epoch transitions, same reward distribution. The only difference is it has 0 peers. This must be checked explicitly.

## Prevention

### Implemented

| Fix | File | Description |
|-----|------|-------------|
| Service file generator | `scripts/install-services.sh` | Single template for all nodes. Enforces --bootstrap, --force-start, logging. |
| Pre-reset validator | `scripts/chain-reset.sh` | Validates ALL service files before allowing reset. Checks peer connectivity after. |
| Post-reset health check | `scripts/health-check.sh` | Verifies peers, genesis, block 1, hash convergence across all nodes. |

### Rules (added to doli-ops skill)

1. **NEVER hand-edit service files.** Use `scripts/install-services.sh`.
2. **NEVER wipe node data without backing up first.**
3. **After any reset:** run `scripts/health-check.sh` and confirm all green before walking away.
4. **If any node has 0 peers:** STOP everything and fix peer discovery before continuing.

---

*Filed by: E. Weil, 2026-03-11*
