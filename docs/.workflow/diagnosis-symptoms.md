# Diagnosis Symptoms: Why Every Sync Fix Creates a New Bug

## Date: 2026-03-17
## Scope: Structural architecture of event loop, SyncManager, GSet gossip

---

## S1: Chain Stall After Deploying 50 Sync-Only Nodes

- **What happens**: Chain stalled at h=1133, s=1561 (428 missed blocks). No blocks produced despite 13 registered producers being online.
- **When**: Immediately after adding ~100 stress test (sync-only) nodes to a 13-producer network.
- **Deterministic**: Yes -- reproducible by adding a large number of syncing peers.
- **Failure boundary**: ALL producers stopped producing. Sync-only nodes unaffected (they have nothing to produce).

### Observed behavior:
1. Producers flooded with "GSet merge_one ... DUPLICATE" log lines from 100+ peers
2. Event loop consumed processing `ProducerAnnouncementsReceived` network events
3. Production timer never fires because `biased select!` gives network events absolute priority
4. Several original producers (n3, n4, n8) fell behind and could not catch up
5. n10 stuck at genesis (height 0)

## S2: Prior Pattern -- Fix One Bug, Another Appears

- **What happens**: Each targeted sync fix resolves its specific scenario but unmasks a new failure mode.
- **Timeline**:
  - Fix snap sync cascade (dd77c7e) -> production gate deadlock appears
  - Fix production gate deadlock -> fork sync weight rejection appears
  - Fix fork recovery bugs (4 bugs, 39169ae) -> deploy stress nodes -> chain stalls from gossip flooding
  - Each fix is correct for its target; the system breaks in a different place.
- **When**: After every deploy that changes sync/production behavior, especially under load.
- **Deterministic**: The specific failure changes each time, but the pattern (fix -> new failure) is consistent.

## S3: Key Observation

The gossip flooding stall is NOT one of the 4 bugs that were fixed in 39169ae. It is an entirely different failure class:
- The 4 bugs were in fork sync recovery (weight comparison, blacklisting, recovery gating, peer selection)
- The stall is in the event loop architecture (gossip message amplification starving the production timer)
- These are orthogonal. Fixing the 4 sync bugs exposed the gossip problem because the network had never been tested at this scale before.

## S4: What Changed Between "Working" and "Broken"

| Before (working) | After (broken) |
|---|---|
| 13 nodes, all producers | 112 nodes (13 producers + ~100 sync-only) |
| GossipSub mesh_n=6, ~12 mesh peers | GossipSub mesh_n=16 (sqrt(106)*1.5), ~32 mesh peers |
| ~12 GSet gossip messages per round | ~100+ GSet gossip messages per round |
| Production timer fires every 1s | Production timer starved by network events |
| Event channel (1024) rarely full | Event channel potentially saturated |
