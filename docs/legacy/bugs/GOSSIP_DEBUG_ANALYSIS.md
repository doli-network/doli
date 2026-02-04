# Gossip Sync Failure Analysis

**Date**: 2026-02-04
**Issue**: Late-joining nodes fail to receive gossip blocks after initial sync completes

---

## Symptoms Summary

1. Initial sync completes successfully (503 blocks downloaded via SyncResponse protocol)
2. Node then STOPS receiving new blocks via gossip
3. Peer status updates ARE received (via request/response protocol)
4. Warning: "Not publishing a message that has already been published"
5. Node stuck at initial sync height while network advances

---

## Code Flow Analysis

### Gossip Block Reception Path

```
1. libp2p swarm receives gossip message
   ↓
2. handle_behaviour_event() [service.rs:526]
   - Match: DoliBehaviourEvent::Gossipsub(Event::Message {...})
   - Log: "Received gossip message on topic {} from {}"
   ↓
3. BLOCKS_TOPIC match [service.rs:546]
   - Block::deserialize(&message.data)
   - Log: "Failed to deserialize block" on error
   ↓
4. event_tx.send(NetworkEvent::NewBlock(block)).await [service.rs:548]
   - Sends to mpsc channel (capacity 1024)
   ↓
5. Node event loop receives event [node.rs:530-538]
   - network.next_event().await
   ↓
6. handle_network_event() [node.rs:691-703]
   - Match: NetworkEvent::NewBlock(block)
   - Log: "Received new block: {}"
   ↓
7. handle_new_block() [node.rs:983-1217]
   - Check if already known
   - Check if builds on tip
   - Apply or cache
```

### Key Observations

1. **No "Received new block" logs** after initial sync → blocks not reaching step 6
2. **"Adding peer" logs present** → peer status updates work (different protocol)
3. **"Not publishing message" warning** → gossip publish path works, but this is about SENDING

---

## Potential Root Causes

### 1. GossipSub Mesh Not Forming (HIGH PROBABILITY)

**Config** (gossip.rs:46-49):
```rust
.mesh_n(6)           // Target 6 peers
.mesh_n_low(4)       // Minimum 4 peers
.mesh_n_high(12)     // Maximum 12 peers
.mesh_outbound_min(2) // Minimum 2 outbound
```

**Problem**: Test used only 1 bootstrap peer. With fewer than `mesh_n_low` (4) peers:
- Node cannot form proper gossip mesh
- Message propagation via mesh is unreliable
- Node may only receive messages via IHAVE/IWANT gossip (slower, less reliable)

**Evidence**: The "Not publishing a message" warning suggests the node IS participating in gossip but having issues with message propagation.

### 2. Block Arrives During Sync Processing Race Condition (MEDIUM)

When initial sync is processing blocks:
1. Sync block 480 applied → `chain_state.best_hash` = hash(480)
2. Gossip block 504 arrives (prev_hash = hash(503))
3. Check: `block.header.prev_hash != state.best_hash` → MISMATCH
4. Block 504 gets CACHED, not applied
5. Later: sync completes at 503
6. Block 504 is in cache but NEVER RE-CHECKED

**Missing Code**: After applying a block, there's no code to check if any cached blocks can now be applied.

### 3. Event Channel State (LOW)

The `event_tx.send().await` uses `let _ =` to ignore results:
```rust
let _ = event_tx.send(NetworkEvent::NewBlock(block)).await;
```

If channel is full, blocks are silently dropped. But channel capacity is 1024 and peer status events DO get through, so this is unlikely.

### 4. Connection/Subscription Lost (LOW)

Topic subscriptions happen once at startup. No mechanism to re-subscribe if lost. But this would affect ALL gossip, not just blocks after sync.

---

## Checkpoint Logs to Add

### Phase 1: Verify Gossip Reception (service.rs)

```rust
// At line 540 (before topic match):
info!("[GOSSIP_DEBUG] Received gossip on topic='{}' from peer={}, data_len={}",
    topic, propagation_source, message.data.len());

// At line 546 (BLOCKS_TOPIC match):
info!("[GOSSIP_DEBUG] Processing BLOCKS_TOPIC message, data_len={}", message.data.len());

// After deserialize (line 547):
info!("[GOSSIP_DEBUG] Block deserialized: hash={}", block.hash());

// Before send (line 548):
info!("[GOSSIP_DEBUG] Sending NewBlock to event channel, block_hash={}", block.hash());
```

### Phase 2: Verify Event Loop Reception (node.rs)

```rust
// At line 537 (after event received):
info!("[GOSSIP_DEBUG] Event loop received: {:?}", event_type_summary(&event));

// At line 691 (NewBlock handling):
info!("[GOSSIP_DEBUG] handle_network_event: NewBlock hash={}", block.hash());

// At line 983 (handle_new_block entry):
info!("[GOSSIP_DEBUG] handle_new_block: hash={}, prev_hash={}",
    block_hash, block.header.prev_hash);

// At line 1001 (prev_hash check):
info!("[GOSSIP_DEBUG] Tip comparison: block_prev={}, our_tip={}, match={}",
    block.header.prev_hash, state.best_hash,
    block.header.prev_hash == state.best_hash);
```

### Phase 3: Verify Sync State (sync/manager.rs)

```rust
// At add_peer (line 339):
info!("[SYNC_DEBUG] add_peer: peer={}, height={}, our_height={}, state={:?}",
    peer, height, self.local_height, self.state);

// At should_sync check (line 374):
info!("[SYNC_DEBUG] should_sync check: state={:?}, should_sync={}, will_sync={}",
    self.state, self.should_sync(),
    matches!(self.state, SyncState::Idle | SyncState::Synchronized) && self.should_sync());

// At update_local_tip (line 310):
info!("[SYNC_DEBUG] update_local_tip: height={}, best_peer_height={}, transitioning_to_synchronized={}",
    height, status.best_height, height >= status.best_height);
```

---

## Recommended Fix Approaches

### Fix 1: Add Cached Block Re-Check After Apply

After `apply_block()` completes, check if any cached fork blocks now build on the new tip:

```rust
// In apply_block() after updating chain_state (around line 1799):
// Check if any cached blocks can now be applied
let blocks_to_apply: Vec<Block> = {
    let cache = self.fork_block_cache.read().await;
    let new_hash = block_hash; // The block we just applied
    cache.values()
        .filter(|b| b.header.prev_hash == new_hash)
        .cloned()
        .collect()
};

for cached_block in blocks_to_apply {
    self.fork_block_cache.write().await.remove(&cached_block.hash());
    // Recursive apply (be careful of stack depth)
    if let Err(e) = self.handle_new_block(cached_block).await {
        warn!("Failed to apply cached block: {}", e);
    }
}
```

### Fix 2: Reduce Gossipsub Mesh Requirements

For better single-peer operation, reduce mesh requirements in devnet:

```rust
// In gossip.rs, make mesh params network-aware:
let (mesh_n, mesh_n_low, mesh_outbound_min) = match network {
    Network::Devnet => (2, 1, 0),  // Relaxed for small devnets
    _ => (6, 4, 2),                // Standard for production
};
```

### Fix 3: Add Gossip Message Logging

Temporarily enable debug logging for gossipsub to see all messages:

```bash
RUST_LOG=libp2p_gossipsub=debug cargo run ...
```

---

## Testing Protocol

1. Start fresh devnet with 5 genesis producers
2. Let chain advance to height 50+
3. Start new producer node with checkpoint logs
4. Observe logs to identify exactly where flow breaks:
   - If [GOSSIP_DEBUG] messages in service.rs → gossip works, issue in node
   - If NO [GOSSIP_DEBUG] messages → gossip reception failing
   - If [SYNC_DEBUG] shows re-sync triggered → sync path working
   - If blocks cached but never applied → cached block issue

---

## Related Code Files

| File | Relevant Lines |
|------|----------------|
| `crates/network/src/gossip.rs` | 38-65 (config), 94-101 (publish) |
| `crates/network/src/service.rs` | 540-551 (gossip handling), 254-256 (next_event) |
| `bins/node/src/node.rs` | 691-703 (NewBlock), 983-1217 (handle_new_block) |
| `crates/network/src/sync/manager.rs` | 300-335 (update_local_tip), 374-402 (sync triggers) |
