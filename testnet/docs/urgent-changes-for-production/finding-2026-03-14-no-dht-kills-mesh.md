# Finding: --no-dht Breaks Mesh Topology and Prevents Sync

**Date:** 2026-03-14
**Severity:** Critical (prevents new nodes from joining the network)
**Status:** Fixed in stress-batch.sh. Code review needed for production defaults.

## Summary

Stress test nodes launched with `--no-dht` could never discover more than 1 peer (their bootstrap). This caused: snap sync deadlock (needs 3 peers), gossip mesh starvation (mesh_n=6 but only 1 peer), and permanent sync failure for 20% of nodes. Removing `--no-dht` fixed all sync issues — 50/50 nodes synced in 60 seconds.

## Evidence

| Config | Synced after 90s | Peer count | Snap sync |
|--------|-----------------|------------|-----------|
| `--no-dht` | 39/50 (78%) | 0-1 per node | Deadlocked (1/3 quorum) |
| DHT enabled | 50/50 (100%) | 3-50 per node | Completes in <30s |

## Root Cause Chain

```
--no-dht
  → Kademlia disabled
  → Node knows only 1 peer (bootstrap)
  → Snap sync quorum: max(3, peers/2+1) = 3 → needs 3 peers, has 1 → waits forever
  → GossipSub mesh_n=6 → wants 6 mesh peers, has 1 → degraded gossip
  → No peer exchange → node never discovers other nodes on same tier
  → If bootstrap node dies → node has 0 peers → permanent isolation
```

## Where `--no-dht` Is Used

| Location | Context | Should Change? |
|----------|---------|----------------|
| `scripts/stress-batch.sh:121` | Stress test nodes | **Fixed** — removed |
| `scripts/launch_testnet.sh:150` | Local 2-node testnet | OK — only 2 nodes, DHT unnecessary |
| `scripts/deploy_producers.sh:485` | Devnet producer deployment | **Review** — should be removed for >5 nodes |
| Launchd plists (`install-local-services.sh`) | N1-N12 genesis producers | OK — genesis producers bootstrap to seed, seed has DHT |

## Code Review Needed

### `crates/network/src/discovery.rs`

DHT is configured with:
- Protocol: `/doli/kad/1.0.0`
- Replication factor: 20
- Mode: Server

Verify that on localhost (all nodes on 127.0.0.1), Kademlia doesn't collapse all nodes into the same bucket. The k-bucket routing table uses peer ID distance, not IP, so this should be fine — but verify under 100+ nodes.

### `crates/network/src/service.rs:234-244`

Connection limits are `max_peers=50` for both incoming and outgoing. With DHT enabled and 100 nodes, each node will discover and attempt to connect to many peers. Verify that:
1. Connection limit correctly caps at 50 without thrashing
2. DHT queries don't count against the connection limit
3. Peer eviction (new code) correctly evicts low-value peers when DHT discovers better ones

### `crates/network/src/nat.rs`

AutoNAT and DCUtR are configured for relay traversal. On localhost, all nodes appear "direct" (no NAT). Verify AutoNAT doesn't interfere with local testing — it probes external reachability which will fail on 127.0.0.1.

### `crates/core/src/network_params.rs:360-510`

Mainnet defaults have `bootstrap_nodes` pointing to `seed1.doli.network` etc. For production, ensure:
1. DHT is always enabled (no flag to disable it in production builds)
2. Bootstrap nodes are seeds, not producers — seeds survive producer churn
3. Minimum 3 bootstrap nodes configured for quorum resilience

## Recommendation

`--no-dht` should be restricted to single-node devnet and 2-node testnet. Any deployment with >5 nodes must have DHT enabled. Consider adding a startup warning:

```rust
if config.disable_dht && active_producers > 5 {
    warn!("DHT disabled with {} producers — peer discovery will be limited to bootstrap nodes only", active_producers);
}
```
