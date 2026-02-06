# REPORT: Transaction Propagation & Mempool Poisoning During Producer Deployment

**Date**: 2026-02-06
**Component**: `devnet add-producer`, mempool, gossip propagation
**Severity**: High (blocks producer scaling on devnet)
**Status**: Resolved

---

## Executive Summary

When deploying 10 new producers via `doli-node devnet add-producer --count 10 --fund-amount 45 -b 20`, only 4 out of 10 completed successfully. The remaining 6 failed due to funding transaction timeouts. Manual recovery attempts revealed a **mempool poisoning** problem: invalid or conflicting transactions became permanently stuck in node 0's mempool, blocking all subsequent transactions from the same wallets across the entire network.

---

## Timeline

| Time | Action | Result |
|------|--------|--------|
| T+0 | `add-producer --count 10 --fund-amount 45 -b 20` | Producers 10, 11, 12, 15 succeeded. Producers 13, 14, 16-19 timed out on funding |
| T+5m | Check timed-out producers' balances | Producers 13, 14 received funds (45 DOLI). Producers 16-19 have 0 balance |
| T+6m | Manual registration of 13, 14 via node 0 RPC (28545) | Submitted successfully |
| T+7m | Manual funding of 16-19 from genesis wallets via node 0 | Producer 16 submitted OK, 17-19 got **double-spend mempool errors** |
| T+8m | Wait 15s, retry 17-19 from wallets 4,5,6 via node 0 | Wallet 4: double-spend. Wallets 5,6: submitted OK |
| T+9m | Wait 20s, check balances of 16-19 | All still 0 DOLI despite successful submissions |
| T+10m | Discover node 0 has **10 stuck mempool txs** | Other nodes (1-14) have 0 txs in mempool |
| T+12m | Fund via alternative RPCs (nodes 2,3,4) | Some succeed, some double-spend |
| T+15m | Fund from wallets 7,8,9 via nodes 7,8,9 (clean mempools) | All 3 submitted successfully, **confirmed within 25s** |
| T+18m | Register 16,17,18,19 via node 7 RPC (28552) | All submitted successfully |
| T+20m | Check producer list | **Still 14 producers** - registrations not confirmed |
| T+22m | Register all 6 via node 10 RPC (28555) | All submitted successfully, still not confirmed |
| T+25m | Restart node 0 to clear mempool | Mempool cleared to 0 txs |
| T+26m | Register all 6 via clean node 0 | All submitted, 6 txs in mempool |
| T+28m | Node 0 has **0 peers** after restart | Txs cannot propagate via gossip |
| T+30m | Attempt to restart node 0 with `--bootstrap` | User interrupted |

---

## Root Causes Identified

### 1. Funder Wallet Rotation Selects Underfunded Wallets

**Code**: `bins/node/src/devnet.rs` - `add_producer()` funder rotation

The `add-producer` command rotates funding across `config.node_count` wallets (0 through N-1). After the first batch of 5 producers was added (producers 5-9), `node_count` became 10. In the second batch, funder rotation cycled across wallets 0-9:

| Producer | Funder Wallet | Wallet Balance at Time | Fund Amount | Result |
|----------|--------------|----------------------|-------------|--------|
| 10 | wallet 0 (genesis) | ~640 DOLI | 45 DOLI | OK |
| 11 | wallet 1 (genesis) | ~640 DOLI | 45 DOLI | OK |
| 12 | wallet 2 (genesis) | ~640 DOLI | 45 DOLI | OK |
| 13 | wallet 3 (genesis) | ~640 DOLI | 45 DOLI | Timeout (tx valid but slow) |
| 14 | wallet 4 (genesis) | ~640 DOLI | 45 DOLI | Timeout (tx valid but slow) |
| 15 | wallet 5 (batch 1) | ~3 DOLI + rewards | 45 DOLI | OK (had earned enough rewards) |
| 16 | wallet 6 (batch 1) | ~3 DOLI + rewards | 45 DOLI | Timeout |
| 17 | wallet 7 (batch 1) | ~3 DOLI + rewards | 45 DOLI | Timeout |
| 18 | wallet 8 (batch 1) | ~3 DOLI + rewards | 45 DOLI | Timeout |
| 19 | wallet 9 (batch 1) | ~3 DOLI + rewards | 45 DOLI | Timeout |

**Problem**: Wallets 5-9 were newly created producers with only ~3 DOLI remaining after their own bond registration. They may or may not have accumulated enough block rewards by the time the batch reached them. The CLI constructs and submits the transaction before server-side balance validation, so the tx enters the mempool even if it's ultimately unspendable.

### 2. Mempool Poisoning: Invalid Txs Persist Indefinitely

**Component**: `crates/mempool/src/pool.rs`

Transactions that reference valid UTXOs but have insufficient total value (or become invalid due to UTXO reuse) are admitted to the mempool and **never evicted**. Observations:

- Node 0 accumulated **10 stuck transactions** that persisted through 400+ blocks (~1200+ seconds)
- These txs were never included in any block (likely failing validation during block assembly)
- They were never removed from the mempool via any expiry or cleanup mechanism
- They blocked all new transactions from the same wallets with "double spend with mempool transaction" errors

**Expected behavior**: The mempool should either:
1. Reject transactions with insufficient balance at submission time
2. Evict transactions that have been in the mempool beyond a TTL (e.g., 100 blocks)
3. Evict transactions whose input UTXOs have been spent by confirmed blocks

### 3. Mempool Isolation: No Cross-Node Propagation of Stuck Txs

**Component**: `crates/network/src/gossip.rs`

The stuck mempool txs on node 0 did **not** propagate to other nodes. This was actually beneficial (other nodes remained clean), but it reveals inconsistent behavior:

| Node | Mempool State | Explanation |
|------|--------------|-------------|
| Node 0 (28545) | 10 stuck txs | Original submissions accumulated here |
| Node 1 (28546) | 0-3 txs | Some manual retry txs submitted here |
| Nodes 2-14 | 0 txs | Clean — stuck txs never propagated |

When valid txs were submitted to clean nodes, those txs **also failed to propagate** back to node 0 or to other block-producing nodes in a timely manner. This created a situation where:
- New valid txs existed only on the node they were submitted to
- Block producers (which include node 0) never received these txs
- Txs could only be mined when the specific receiving node produced a block

### 4. Bootstrap Node Restart Causes Network Isolation

**Component**: `bins/node/src/main.rs`, `crates/network/src/service.rs`

When node 0 (the bootstrap/seed node) was restarted to clear its mempool:

- Node 0 started with **0 peers** because it has no `--bootstrap` flag (it IS the bootstrap)
- `--no-dht` prevents peer discovery via DHT
- Other nodes have `--bootstrap /ip4/127.0.0.1/tcp/50303` but don't aggressively reconnect
- Node 0 became isolated: producing blocks on its own fork, unable to gossip mempool txs
- Result: 6 registration txs sat in node 0's mempool with no way to reach block producers

### 5. UTXO Double-Spend Cascading

When the same genesis wallet is used from multiple RPC endpoints, conflicting transactions are created:

```
Wallet 1 has UTXO-A (595 DOLI)
  ├─ Node 0: tx1 uses UTXO-A → send 45 to producer_16 (stuck in mempool)
  ├─ Node 2: tx2 uses UTXO-A → send 45 to producer_16 (submitted OK)
  └─ Node 1: tx3 uses UTXO-A → send 45 to producer_16 (submitted OK)
```

Three conflicting txs using the same UTXO exist across different nodes. Only one can ever confirm. The others become permanently stuck in their respective mempools until restart.

---

## Impact

1. **Producer deployment at scale is unreliable**: Any batch >5 producers risks partial failures
2. **Manual recovery is extremely difficult**: Stuck mempool txs block all retry attempts from the same wallets
3. **No self-healing**: The mempool never cleans up stuck transactions
4. **Bootstrap node is a single point of failure**: Restarting it causes network isolation

---

## Proposed Fixes

### Fix 1: Mempool TTL (High Priority)

Add a time-to-live for mempool transactions. Transactions older than N blocks (e.g., 100) should be automatically evicted.

```
Location: crates/mempool/src/pool.rs
Change: Add `submitted_at_height` field to mempool entries, evict during block application
```

### Fix 2: Mempool UTXO Validation on Block Application (High Priority)

When a new block is applied, scan the mempool and evict any transactions whose input UTXOs have been spent by the block or are no longer valid.

```
Location: crates/mempool/src/pool.rs
Change: Hook into block application to prune invalidated txs
```

### Fix 3: Balance Pre-Check at Submission (Medium Priority)

Before accepting a transaction into the mempool, verify that the referenced UTXOs exist AND that the total input value covers the output value + fees.

```
Location: crates/mempool/src/pool.rs or RPC handler
Change: Add balance sufficiency check before mempool insertion
```

### Fix 4: Funder Rotation Should Only Use Genesis Wallets (Medium Priority)

The `add-producer` funder rotation should only use genesis wallets (index 0 through initial_node_count-1), not recently-added producers that may have insufficient balance.

```
Location: bins/node/src/devnet.rs - add_producer()
Change: Read initial genesis count from chainspec, only rotate across those wallets
```

### Fix 5: Peer Reconnection After Bootstrap Restart (Medium Priority)

When the bootstrap node restarts, it should either:
- Accept incoming connections from nodes that have it as their bootstrap (already works, but slowly)
- Have a mechanism to connect to known peers from its peer database
- Other nodes should more aggressively reconnect to their bootstrap address

```
Location: crates/network/src/service.rs
Change: Periodic bootstrap reconnection attempts
```

### Fix 6: Transaction Propagation Improvement (Low Priority)

Ensure that transactions submitted to any node eventually reach all other nodes via gossip, regardless of mempool conflicts on specific nodes.

```
Location: crates/network/src/gossip.rs
Change: Relay new txs even if local mempool has conflicts (let receiving node decide)
```

---

## Workarounds (Current)

1. **Use different RPC endpoints**: Submit txs to nodes with clean mempools (not node 0)
2. **Use different funder wallets**: Avoid wallets that have any pending mempool txs
3. **Don't restart node 0**: It causes network isolation. If you must, add `--bootstrap /ip4/127.0.0.1/tcp/50304` to reconnect
4. **Deploy in small batches**: `--count 3` is more reliable than `--count 10`
5. **Wait between batches**: Allow 30+ seconds between batches for block confirmation

---

## Reproduction Steps

```bash
# 1. Start a fresh 5-node devnet
doli-node devnet init --nodes 5
doli-node devnet start

# 2. Add 5 producers (succeeds, creates wallets 5-9 with ~3 DOLI each)
doli-node devnet add-producer --count 5 --fund-amount 5 -b 1

# 3. Immediately try adding 10 more with high fund amount
#    This will fail for producers funded from wallets 5-9 (insufficient balance)
doli-node devnet add-producer --count 10 --fund-amount 45 -b 20

# 4. Observe: some producers timeout, stuck txs accumulate in node 0's mempool
curl -s http://127.0.0.1:28545 -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"getMempoolInfo","params":[],"id":1}'
# Shows txCount > 0 and stays there indefinitely

# 5. Try to manually fund a timed-out producer from a genesis wallet
#    Will fail with "double spend with mempool transaction"
```

---

## Related Files

| File | Relevance |
|------|-----------|
| `crates/mempool/src/pool.rs` | Mempool insertion, eviction logic |
| `crates/mempool/src/policy.rs` | Transaction acceptance policy |
| `crates/network/src/gossip.rs` | Transaction gossip propagation |
| `crates/network/src/service.rs` | Peer connection management |
| `bins/node/src/devnet.rs` | `add_producer()` funder rotation logic |
| `bins/node/src/main.rs` | CLI args, bootstrap configuration |

---

## Test Plan

Once fixes are implemented:

1. **Mempool TTL test**: Submit an invalid tx, verify it's evicted after N blocks
2. **UTXO cleanup test**: Spend a UTXO in a block, verify conflicting mempool txs are removed
3. **Large batch deployment test**: `add-producer --count 20` should complete without stuck txs
4. **Bootstrap restart test**: Restart node 0, verify it reconnects and propagates txs within 30s
5. **Cross-node tx propagation test**: Submit tx to node 5, verify it appears in node 0's mempool

---

## Resolution (2026-02-06)

### Root Cause Analysis

Deep analysis revealed two true root causes — everything else in this report cascades from them:

**Root Cause 1: RPC-submitted transactions were never broadcast to the network**
- `bins/node/src/node.rs`: `start_rpc()` created the RPC context without wiring up `.with_broadcast()`
- The broadcast callback in `rpc/methods.rs` defaulted to a no-op `Arc::new(|_| {})`
- Every tx submitted via RPC entered ONLY the local mempool and could only be mined when that specific node produced a block

**Root Cause 2: `revalidate()` was only called after reorgs, never after normal block application**
- `bins/node/src/node.rs`: `apply_block()` called `remove_for_block()` but not `revalidate()`
- When a block spent a UTXO, only the block's own tx was removed; conflicting mempool txs referencing the same UTXO stayed forever
- These stuck txs blocked all future txs from the same wallet via the `spent_outputs` double-spend guard

### Fixes Applied

| File | Change |
|------|--------|
| `bins/node/src/node.rs` | Wired `.with_broadcast()` in `start_rpc()` — clones the network command sender and spawns async broadcast |
| `bins/node/src/node.rs` | Added `revalidate()` call after `remove_for_block()` in `apply_block()` |
| `crates/mempool/src/pool.rs` | Updated `revalidate()` log message to be generic (was "after reorg") |

### Proposed fixes from this report — status

| Fix | Status | Notes |
|-----|--------|-------|
| Fix 1 (Mempool TTL) | Not needed | Root cause 2 fix eliminates stuck txs; TTL is defense-in-depth for a future PR |
| Fix 2 (UTXO cleanup on block) | **Applied** | `revalidate()` now called after every block application |
| Fix 3 (Balance pre-check) | Not needed | Already validated during block assembly; mempool accepts optimistically by design |
| Fix 4 (Funder rotation) | Not needed | `config.node_count` is set only during `devnet init` and never updated |
| Fix 5 (Peer reconnection) | Separate issue | Not related to tx propagation root causes |
| Fix 6 (Tx propagation) | **Applied** | RPC broadcast callback now wired up — txs gossip to all peers |
