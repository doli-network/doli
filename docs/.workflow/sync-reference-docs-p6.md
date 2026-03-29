# Sync Pass 6 — Reference Docs (Workflow Run 95)

**Date**: 2026-03-29
**Scope**: 4 documentation files vs source code
**Focus**: RPC method signatures/params/return fields, CLI subcommands/flags, error codes, reward formulas/constants, wire protocol message types, GossipSub topics

---

## Files Compared

### 1. `docs/rpc_reference.md` vs `crates/rpc/src/methods/` (16 files)

**Result: NO DRIFT FOUND**

Verified all 39 RPC methods against their handler implementations:
- `dispatch.rs` — method routing (39 methods)
- `block.rs` — getBlockByHash, getBlockByHeight, getBlockRaw
- `balance.rs` — getBalance (5-field response), getUtxos (with pending mempool)
- `network.rs` — getMempoolInfo, getNetworkInfo, getPeerInfo, getChainInfo, getNodeInfo, getEpochInfo, getNetworkParams
- `producer.rs` — getProducer, getProducers (with pending registrations), getBondDetails (FIFO vesting)
- `transaction.rs` — getTransaction (mempool-first), sendTransaction (state-only handling)
- `history.rs` — getHistory (28 tx type string mappings)
- `governance.rs` — submitVote, getUpdateStatus, getMaintainerSet, submitMaintainerChange
- `backfill.rs` — backfillFromPeer, backfillStatus, verifyChainIntegrity
- `stats.rs` — getChainStats, getStateRootDebug, getUtxoDiff, getMempoolTransactions
- `schedule.rs` — getSlotSchedule, getProducerSchedule, getAttestationStats
- `snapshot.rs` — getStateSnapshot
- `pool.rs` — getPoolInfo, getPoolList, getPoolPrice, getSwapQuote
- `lending.rs` — getLoanInfo, getLoanList

Also verified:
- Response type structs in `crates/rpc/src/types/` (chain.rs, producer.rs, block.rs, protocol.rs)
- Error codes in `crates/rpc/src/error.rs` (-32700 through -32007)

### 2. `docs/cli.md` vs `bins/cli/src/` (commands.rs, 1063 lines)

**Result: NO DRIFT FOUND**

Verified all subcommands, flags, env vars, and argument types against clap definitions. All 15+ top-level commands and their subcommands match exactly.

### 3. `docs/rewards.md` vs `bins/node/src/node/rewards.rs`, consensus constants, `crates/storage/src/producer.rs`

**Result: NO DRIFT FOUND**

Verified:
- All constants in Section 10 against `crates/core/src/consensus/constants.rs`
- Network-specific parameters in Section 6 against `crates/core/src/network_params/defaults.rs`
- Reward flow (pooled coinbase, attestation tracking, epoch boundary distribution, delegation split)
- Never-burn 3-tier fallback (Tier 1: 90% threshold, Tier 2: 80% of median, Tier 3: accumulate)
- Epoch bond snapshot usage (frozen from UTXO set, not live state)
- EpochReward transaction structure (TxType=10)

### 4. `docs/protocol.md` vs `crates/network/src/` + `crates/core/src/transaction.rs` + `crates/core/src/block.rs`

**Result: 1 DRIFT FOUND AND FIXED**

Verified against source files:
- `crates/network/src/protocols/status.rs` — StatusRequest/StatusResponse structs
- `crates/network/src/protocols/sync.rs` — SyncRequest/SyncResponse enums (7 request types, 6 response types)
- `crates/network/src/protocols/txfetch.rs` — TxFetch protocol
- `crates/network/src/gossip/mod.rs` — 8 GossipSub topics (all topic strings verified)
- `crates/network/src/gossip/config.rs` — GossipSub parameters (heartbeat, mesh, scoring)
- `crates/network/src/gossip/publish.rs` — publish functions, tx batch/announce encoding
- `crates/network/src/transport.rs` — TCP + Noise + Yamux stack
- `crates/network/src/scoring.rs` — Peer scoring penalties and range
- `crates/network/src/rate_limit.rs` — Rate limit defaults
- `crates/network/src/discovery.rs` — Kademlia DHT parameters
- `crates/network/src/messages.rs` — Message enum
- `crates/network/src/service/types.rs` — NetworkEvent enum (20 variants)
- `crates/core/src/transaction/types.rs` — TxType enum (28 variants), OutputType enum (13 types)
- `crates/core/src/block.rs` — BlockHeader struct (9 fields)
- `crates/vdf/src/lib.rs` — VDF block_input function, T_BLOCK constant

#### Drift #1 (FIXED): VDF iterations for devnet

**Section 9, line 366**
- **Doc said**: "Network default `vdf_iterations`: 1,000 (all networks via NetworkParams)"
- **Code says**: Mainnet/testnet use 1,000 but devnet uses `vdf_iterations: 1` (`crates/core/src/network_params/defaults.rs` line 185)
- **Fix**: Updated to "1,000 (mainnet/testnet via NetworkParams); devnet uses 1 iteration for fast development"

---

## Summary

| Document | Drift Items | Fixed |
|----------|-------------|-------|
| `docs/rpc_reference.md` | 0 | 0 |
| `docs/cli.md` | 0 | 0 |
| `docs/rewards.md` | 0 | 0 |
| `docs/protocol.md` | 1 | 1 |
| **Total** | **1** | **1** |

All 4 documentation files are now synchronized with the codebase.
