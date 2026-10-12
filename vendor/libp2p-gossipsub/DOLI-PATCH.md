# DOLI local patch — libp2p-gossipsub 0.46.1 (INC-I-237)

Upstream: crates.io `libp2p-gossipsub` 0.46.1 (pulled by `libp2p` 0.53), copied verbatim.
Wired in via `[patch.crates-io]` in the workspace root `Cargo.toml`.

## What changed (`src/handler.rs`, every hunk marked `// DOLI INC-I-237:`)
- Per-connection `send_queue` is bounded at `MAX_SEND_QUEUE_BYTES` (4 MiB, protobuf encoded size).
- Over the cap, RPCs with publishes or IHAVE/IWANT-only control are dropped and counted;
  RPCs with subscriptions or GRAFT/PRUNE are always queued.
- Drops log at warn (target `libp2p_gossipsub::handler`, text "INC-I-237 send_queue full"):
  once per overflow episode, plus a running total every 1000 drops.
- The per-pop `shrink_to_fit()` now runs only when capacity > 4 × len and capacity > 64.

## M7.1 — retention readout (measurement only, no behaviour change)
- `src/time_cache.rs`: `TimeCacheStats`, `RetentionStats`, `table_bytes`; `TimeCache::stats`;
  `DuplicateCache` and its `new` / `insert` made `pub` (`#[doc(hidden)]`), plus `stats`;
  `map_capacity_hwm` (max `capacity()` read in `entry`, because tombstones hide allocated slots).
- `src/mcache.rs`: running `held_bytes` (put / remove / shift), `msgs_capacity_hwm` (put), `retention()`.
- `src/peer_score.rs`: `PeerScore::deliveries_stats`.
- `src/behaviour.rs`: `Behaviour::retention_stats` (`#[doc(hidden)]`), len/capacity reads only.
- `src/lib.rs`: `#[doc(hidden)]` re-export of `DuplicateCache`, `RetentionStats`, `TimeCacheStats`.

## Why
Upstream queue is unbounded: a slow-draining peer grew it without limit (OOM on mainnet).
Judge: `cargo test -p network --test it inc_i_237`.

## Dropping the patch
Upgrade to a libp2p whose gossipsub (>= 0.48) has bounded send queues, delete this
directory, the `[patch.crates-io]` entry and the `exclude` entry, then re-run the judge test.
