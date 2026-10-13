# DOLI local patch — libp2p-gossipsub 0.46.1 (INC-I-237)

Upstream: crates.io `libp2p-gossipsub` 0.46.1 (pulled by `libp2p` 0.53), copied verbatim.
Wired in via `[patch.crates-io]` in the workspace root `Cargo.toml`.

## What changed (`src/handler.rs`, every hunk marked `// DOLI INC-I-237:`)
- CI lint hygiene (`RUSTFLAGS=-D warnings` on current stable): `generated/compat/pb.rs` allows
  `dead_code` (the compat `Message` is only decoded); `time_cache.rs` `entry()` spells the
  elided lifetime as `Entry<'_, Key, Value>`. No behaviour change.
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

## M7.2 — count caps on the duplicate and peer-score `deliveries` caches
- `src/time_cache.rs`: `TimeCache.max_entries` (`usize::MAX` = uncapped, `new` unchanged) and
  `TimeCache::new_capped`; in `entry`, a NEW key at `len >= max_entries` evicts the oldest id first
  (`make_room`); with zero growth slack and `len >= max_entries / 2` the map is rebuilt at
  `with_capacity(max_entries)` (drops tombstones; the table never doubles past the cap's size).
  TTL expiry unchanged. `TimeCacheStats.max_entries`; `DuplicateCache::new_capped` (`#[doc(hidden)]` type).
- `src/config.rs`: `Config::{duplicate_cache_max_entries, peer_score_deliveries_max_entries}` and the
  `#[doc(hidden)]` `ConfigBuilder` setters; defaults `usize::MAX` (upstream behaviour).
- `src/behaviour.rs`: the duplicate cache is built with the configured cap; `with_peer_score*`
  passes the deliveries cap to `PeerScore`.
- `src/peer_score.rs`: `PeerScore::with_deliveries_max_entries`.
- Judge also covers: `cargo nextest run -p network --test it -E 'test(inc_i_237_m7_2)'`.

## Why
Upstream queue is unbounded: a slow-draining peer grew it without limit (OOM on mainnet).
Judge: `cargo test -p network --test it inc_i_237`.

## Dropping the patch
Upgrade to a libp2p whose gossipsub (>= 0.48) has bounded send queues, delete this
directory, the `[patch.crates-io]` entry and the `exclude` entry, then re-run the judge test.
