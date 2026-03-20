# Incident Report: DHT Poisoning via Unstable Peer Connections

**Date**: 2026-03-20
**Severity**: High
**Duration**: ~2 hours (04:30 - 06:30 UTC)
**Impact**: N6 offline (block store corruption), N12 degraded (13 blocks behind, 167 sync failures)
**Root Cause**: Unstable external peer poisoned DHT routing table, inflating sync_fails on healthy nodes

---

## Timeline

| Time (UTC) | Event |
|------------|-------|
| ~04:16 | External node (Andres, peer `12D3KooWC4hoLbPp`) joins mainnet with correct genesis (`f2308a15`) |
| ~04:16 | Node connects to seeds, TCP handshake succeeds, but connections drop within 100ms |
| ~04:16 | Pattern repeats every ~1 second: connect → 2 established → 100ms → closed → repeat |
| ~04:17 | Peer registered in DHT routing table of all seeds (`[DHT] Routing updated for peer: C4hoLbPp`) |
| ~04:27 | Another node on same LAN (`10.1.1.186`) connects as testnet (network=2), gets rejected |
| ~04:30 | N12 discovers Andres via DHT, attempts sync, connection fails → `sync_fails` increments |
| ~05:04 | N12 at 113 sync_fails, "Deep fork detected: peers consistently reject our chain tip" |
| ~05:05 | N12 at 167 sync_fails, stuck in fork recovery loop, falling further behind |
| ~05:10 | N6 crashes with "Block store genesis mismatch" (corrupted during recovery attempts) |
| ~05:13 | N12 crashes with "Block store genesis mismatch" on restart |
| ~05:14 | Manual wipe + restart of N6 and N12 resolves the issue |
| ~05:18 | Both nodes fully synced |

## Root Cause Analysis

### Primary: DHT Poisoning via Unstable Peer

An external node joined the mainnet P2P network with:
- Correct genesis hash (passed handshake validation)
- Valid peer ID and libp2p identity
- Functional TCP connectivity to seeds (port 30300 reachable)

But the node's home NAT/router terminated connections within ~100ms of establishment. The rapid connect/disconnect cycle caused:

1. **DHT propagation**: The peer was added to the Kademlia routing table of all seeds and propagated to producers via DHT queries
2. **Sync target selection**: N6 and N12 selected this peer for header-first sync (it appeared as a valid mainnet peer)
3. **sync_fails inflation**: Each failed sync attempt incremented `sync_fails` counter
4. **False fork detection**: At 100+ sync_fails, the periodic health check triggered "Deep fork detected" — the node incorrectly concluded its own chain was rejected by peers
5. **Recovery cascade**: Fork recovery attempted rollbacks, which corrupted the block store when combined with rapid peer churn

### Secondary: Testnet Node on Same LAN

A second node on `10.1.1.186` was running testnet (network=2) on the same local network. This node repeatedly connected to mainnet seeds (port 30300) and was rejected with "network mismatch". While this didn't directly cause the fork, it added noise to the peer table and connection logs.

### Contributing Factor: sync_fails Design Flaw

The `sync_fails` counter increments on ANY failed sync interaction, including:
- Connection timeout to unreachable peer
- Peer disconnects before responding
- DNS resolution failure
- Peer at height 0 (syncing/empty)

This conflates "our chain is rejected" with "peer is unreachable/broken" — two fundamentally different conditions. The fork detection threshold treats all sync_fails equally.

## Attack Vector Assessment

This incident reveals a **low-cost eclipse/degradation attack**:

| Property | Value |
|----------|-------|
| Cost | Zero (no stake required) |
| Skill | Low (script that connects/disconnects) |
| Detection | Hard (peer has valid genesis, valid identity) |
| Impact | Can degrade sync of targeted producers |
| Amplification | DHT propagates attacker to all nodes automatically |

**Attack recipe:**
1. Run a node with correct genesis hash and `--network mainnet`
2. Accept TCP connections but RST/close after ~100ms (or use flaky NAT)
3. Repeat — DHT automatically propagates you to all nodes
4. Target nodes accumulate sync_fails → enter false fork recovery → fall behind

With 10-20 such nodes, an attacker could trigger "Deep fork detected" on all producers simultaneously, halting block production without any stake.

## Recommendations

### P0 — Critical (implement immediately)

1. **Separate sync_fails by cause**: Connection failures (timeout, RST, DNS) should NOT increment the same counter as block validation failures. Only count "peer responded with blocks that don't match our chain" as evidence of fork.

2. **Minimum connection duration for DHT**: Don't add peers to DHT routing table unless they maintain a connection for at least 5 seconds. Peers that disconnect in <1s should never enter the routing table.

3. **Ignore height-0 peers in sync decisions**: Peers reporting height=0 and slot=0 are syncing themselves — they are not evidence of network state. Exclude them from best_peer_height, net_tip, and fork detection calculations.

### P1 — High (implement this week)

4. **Rate-limit reconnections per peer ID**: If the same peer ID connects/disconnects more than 5 times in 60 seconds, ban it for 10 minutes. This prevents the rapid churn that poisons the peer table.

5. **Decay sync_fails over time**: sync_fails should decay (e.g., halve every 5 minutes) rather than accumulate indefinitely. A burst of failures from one bad peer shouldn't permanently degrade fork detection.

6. **Fork detection requires block evidence**: "Deep fork detected" should only trigger when peers send VALID blocks that don't build on our chain — not when connections fail. Connection failures indicate network problems, not chain disagreement.

### P2 — Medium (implement this month)

7. **Peer scoring for connection stability**: Track connection duration per peer. Peers with average connection < 5s get a negative score. Below a threshold, stop dialing them and remove from DHT.

8. **Separate "unreachable" from "disagreeing" peers**: Maintain two peer categories. Unreachable peers trigger reconnection backoff. Disagreeing peers trigger fork recovery. Never conflate the two.

9. **DHT query result filtering**: When selecting sync targets from DHT results, prefer peers with known height > 0 and connection history > 30s.

### P3 — Low (backlog)

10. **Authenticated peer reputation**: Persist peer reputation across restarts. Peers that consistently connect/disconnect get longer bans on each restart.

11. **Alarm/monitoring**: Alert when sync_fails exceeds 50 within 5 minutes — this indicates either a network issue or an attack in progress.

## Lessons Learned

1. **Home NAT is hostile**: Residential internet connections can create the exact same pattern as a deliberate attack. The protocol must handle both gracefully.

2. **DHT is a trust amplifier**: A single unstable peer gets propagated to the entire network via Kademlia. DHT entries should require minimum connection quality.

3. **sync_fails is a blunt instrument**: Conflating connection failures with chain disagreement creates a cheap attack surface. These are fundamentally different signals that require different responses.

4. **Test with external nodes before production**: We should have tested external node connectivity in a staging environment before allowing public peers.

---

**Author**: DOLI Core Team
**Status**: Resolved (manual wipe of N6, N12)
**Follow-up**: P0 recommendations tracked for next release
