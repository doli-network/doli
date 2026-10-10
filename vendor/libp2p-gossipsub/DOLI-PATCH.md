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

## Why
Upstream queue is unbounded: a slow-draining peer grew it without limit (OOM on mainnet).
Judge: `cargo test -p network --test it inc_i_237`.

## Dropping the patch
Upgrade to a libp2p whose gossipsub (>= 0.48) has bounded send queues, delete this
directory, the `[patch.crates-io]` entry and the `exclude` entry, then re-run the judge test.
