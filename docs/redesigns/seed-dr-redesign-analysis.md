# Seed Node Disaster Recovery — Redesign Analysis

> Analyst output for omega-redesign workflow (2026-04-24)

## Problem Statement

Seed nodes are the last line of defense when the entire DOLI network goes down. The seed must be able to fully recover the chain from its own local backups — zero external peers. The current DR path has gaps: data stores are disconnected, the UTXO store is not checkpointed, and the `recover` command is broken.

## Current Architecture: Four Data Stores

### 1. blocks (`data_dir/blocks/`)
- **Contents**: Headers, bodies, height index, hash-to-height, slot index, tx indexes (6 RocksDB CFs)
- **Checkpointed**: YES (via `create_checkpoint()`)
- **Rebuildable**: Only from archive flat files or peer RPC

### 2. state_db (`data_dir/state_db/`)
- **Contents across 6 column families**:
  - `cf_utxo` — authoritative UTXO set
  - `cf_utxo_by_pubkey` — secondary UTXO index
  - `cf_producers` — registered producer records
  - `cf_exit_history` — producer exit tracking
  - `cf_meta` — ChainState, EpochState, pending updates, chain commitment, last_applied canary
  - `cf_undo` — reverse diffs for rollback
- **Checkpointed**: YES (via `create_checkpoint()`)
- **Rebuildable**: Partially via `recover` (BROKEN). Fully only via normal `apply_block()` sync.

### 3. utxo_store (`data_dir/utxo_store/`)
- **Contents**: Performance cache of UTXO set (mirrors `cf_utxo`)
- **Checkpointed**: NO
- **Rebuildable**: YES — automatically from state_db via INC-I-027 self-heal at startup

### 4. archive (configured via `--archive-to`)
- **Contents**: Individual `.block` files + `.blake3` checksums + `manifest.json`
- **Checkpointed**: NO (independent system)
- **Rebuildable**: YES — from blocks store via archiver `catch_up()`

## Two Recovery Paths

### Path A: Checkpoint Restore — WORKS

| Step | state_db | blocks | utxo_store | Integrity |
|------|----------|--------|------------|-----------|
| Stop seed | At tip H | At tip H | At tip H | Consistent |
| Wipe data_dir | GONE | GONE | GONE | N/A |
| cp checkpoint/state_db | At height C | — | — | — |
| cp checkpoint/blocks | At height C | At height C | — | — |
| Start node | Loaded at C | Loaded at C | Self-heals from state_db | Consistent at C |
| **Result** | **WORKS** | **WORKS** | **WORKS** | Needs peers only for C+1..tip |

### Path B: Archive Restore — BROKEN (5 bugs)

| Step | state_db | blocks | utxo_store | Integrity |
|------|----------|--------|------------|-----------|
| Stop seed | At tip H | At tip H | At tip H | Consistent |
| Wipe data_dir | GONE | GONE | GONE | N/A |
| `restore --from /archive/` | Absent | 1..archive_tip | Absent | — |
| `recover --yes` | Writes legacy files | Unchanged | Absent | BROKEN |
| Start → migration | Wrong genesis, incomplete producers, 0 UTXOs | Loaded | Empty | BROKEN |

## Five Bugs in `recover`

1. **Wrong genesis_hash** (`chain.rs:325`): `hash(b"DOLI Genesis")` instead of chainspec genesis
2. **UTXO path mismatch** (`chain.rs:273`): writes to `utxo`, init looks for `utxo.bin`
3. **Writes legacy files, not state_db**: chain_state.bin/producers.bin instead of RocksDB CFs
4. **Only 3/10+ tx types processed**: Missing AddBond, DelegateBond, RevokeDelegation, EpochReward, ProtocolActivation, AddMaintainer, RemoveMaintainer
5. **No epoch state rebuilt**: Missing bond_snapshot, attestation accumulators, producer lists

## Capability Inventory

### CLI Flags: `--auto-checkpoint N`, `--archive-to /path/`, `--recovery-mode`
### CLI Commands: `recover`, `restore --from`, `restore --from-rpc`, `restore --backfill`, `reindex`, `truncate`
### RPC Methods: `createCheckpoint`, `getGuardianStatus`, `enterRecoveryMode`, `exitRecoveryMode`, `pauseProduction`, `resumeProduction`, `backfillFromPeer`, `verifyChainIntegrity`, `getBlockRaw`
### Self-Heal: UTXO self-heal (init.rs:59-76), legacy migration (init.rs:236-302), body gap recovery (init.rs:96-190), genesis validation (init.rs:313-328)

## Gap Analysis

| Gap | Severity | Description |
|-----|----------|-------------|
| G1 | Critical | `recover` is broken — 5 bugs make archive restore unusable |
| G2 | High | No unified recovery command — checkpoint restore is manual `cp` |
| G3 | Medium | Checkpoint and archive are unconnected systems |
| G4 | Low | utxo_store not in checkpoint (self-heal covers it) |
| G5 | Medium | No post-restore verification step |
| G6 | Inherent | Blocks between checkpoint height and tip must come from archive or peers |

## Acceptance Criteria (MoSCoW)

| ID | Priority | Requirement |
|----|----------|-------------|
| REQ-DR-001 | Must | Checkpoint restore starts node without peers |
| REQ-DR-002 | Must | `recover` writes to state_db directly |
| REQ-DR-003 | Must | `recover` uses correct genesis_hash |
| REQ-DR-004 | Must | `recover` processes ALL state-mutating tx types |
| REQ-DR-005 | Should | `recover` rebuilds epoch state |
| REQ-DR-006 | Should | DR docs clearly state checkpoint contents |
| REQ-DR-007 | Could | Archive/checkpoint cross-referenced |
| REQ-DR-008 | Could | Post-recovery verification step |
| REQ-DR-009 | Could | Unified `restore-checkpoint` command |
| REQ-DR-010 | Must | Automated regression test for checkpoint restore |

## Key Code References

- `bins/node/src/operations/chain.rs:192-466` — recover_chain_state()
- `bins/node/src/operations/restore.rs` — restore commands
- `bins/node/src/node/init.rs:40-94` — UTXO self-heal
- `bins/node/src/node/init.rs:236-302` — legacy migration
- `bins/node/src/node/periodic.rs:654-746` — checkpoint creation
- `crates/rpc/src/methods/guardian.rs` — checkpoint RPC
- `crates/storage/src/archiver.rs` — block archiver
