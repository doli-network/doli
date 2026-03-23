# Urgent Changes for Production

Issues discovered during local stress testing on 2026-03-14 that affect production mainnet.
Each document contains: root cause, evidence, affected files, exact fix, and reproduction steps.

## P0 — Critical (chain halt risk)

| Issue | File | Description |
|-------|------|-------------|
| [DHT Required for Mesh](finding-2026-03-14-no-dht-kills-mesh.md) | `discovery.rs`, `service.rs` | `--no-dht` prevents peer discovery → snap sync deadlock (1/3 quorum), gossip starvation (1/6 mesh), 20% of nodes permanently stuck. **DHT must be enabled for any deployment >5 nodes.** |
| [Gossip Mesh Dilution](incident-2026-03-14-gossip-mesh-dilution.md) | `service.rs`, `gossip/config.rs` | Non-producer peers consume all peer slots → producers isolated from each other's gossip mesh → solo blocks → recurring forks. **14.3% isolation probability per producer per heartbeat.** |
| [Tier 1 Death → Chain Halt](incident-2026-03-14-tier1-death.md) | `production.rs:363,746`, `scheduler.rs` | Killing top-bonded producers halts chain permanently. Scheduler is bond-weighted with no liveness awareness. 100 online producers couldn't fill slots owned by 5 dead whales. **Partially fixed** — emergency round-robin activates after 30s stall, but throughput is 3% (1 block/min vs 6 block/min). |
| [Block Building Timeout](incident-2026-03-14-chain-stall.md#p0-block-building-must-be-time-budgeted) | `production.rs:1024-1048` | Mempool TX validation has no time budget. 50+ TXs → block building exceeds slot window → chain halts. |
| [Dead Peer Reconnection](incident-2026-03-14-chain-stall.md#p0-peer-reconnection-backoff-for-dead-peers) | `service.rs` | Dead peers retried every tick with no backoff. 50 dead peers = 200+ failed TCP/sec, starves event loop. |

## P1 — High

| Issue | File | Description |
|-------|------|-------------|
| Registration VDF Caching | `validation.rs` | Same VDF verified on every block attempt. Cache result to avoid redundant CPU work. |
| Production Offset | `production.rs` | Block production starts at random offset (0-9s into slot). Should start at slot boundary. |
| Auto Backfill After Snap Sync | `fork_recovery.rs` | Snap sync restores state but not block store → integrity gaps (-1200+). Requires manual `backfillFromPeer`. Should be automatic. |

## P2 — Medium

| Issue | File | Description |
|-------|------|-------------|
| Mempool TX Priority | `mempool/` | Expensive TXs (Registration) should be deprioritized vs simple Transfers in `select_for_block()`. |

## Already Fixed (local branch)

| Fix | File | What |
|-----|------|------|
| Layer 6.5 reorg loop | `sync/manager.rs:1082` | Sync state gate + lag > 3 blocks → unconditional production block. Old 60s timeout caused infinite fork→reorg. |
| Zombie peer eviction | `peer.rs`, `service.rs:764` | `eviction_score()` + `find_evictable_peer()` — evicts h=0 zombies when new peers connect at max_peers. |
| Emergency round-robin | `production.rs:363-375` | `chain_stalled` detection (>3 empty slots) → falls back to `bootstrap_schedule_with_liveness()` bypassing bond-weighted epoch scheduler. |
| Resync grace cancellation | `sync/manager.rs:989` | Cancel post-recovery grace period when chain stalled >30s. |
| Circuit breaker relaxed | `sync/manager.rs:529` | `max_solo_production_secs: 86400` for local dev. |
| `--no-dht` removed | `scripts/stress-batch.sh` | DHT enabled for stress test nodes — peer discovery works, 50/50 sync in 60s. |

## Incident Reports (chronological)

1. [Stress Test Fork](../incident-2026-03-14-stress-test-fork.md) — N1 forked when 50 nodes bootstrapped simultaneously
2. [Chain Stall](incident-2026-03-14-chain-stall.md) — 50 registration TXs flooded mempool → block building timeout → all producers stalled
3. [Gossip Mesh Dilution](incident-2026-03-14-gossip-mesh-dilution.md) — Non-producers filled peer slots → producers isolated → recurring forks
4. [Tier 1 Death](incident-2026-03-14-tier1-death.md) — Killing N1-N5 (91% bonds) halted chain → emergency round-robin partially fixed
5. [DHT Required](finding-2026-03-14-no-dht-kills-mesh.md) — `--no-dht` prevented peer discovery → snap sync deadlock → 20% nodes stuck
