# Protocol Sync Findings (2026-03-29)

Source of truth: code in `crates/core/src/`, `crates/network/src/`, `bins/node/src/node/`.

## specs/protocol.md Drift

### 1. Block Header size comment (line 93)
- **Doc says**: `slot (4) + producer(32)` in header size calculation
- **Code says**: `slot` is `Slot` type which is `u32` = 4 bytes. Correct in doc.
- **Verdict**: No drift.

### 2. Transaction type `extra_data` field in docs/protocol.md
- **Doc says** (line 280): `data: Option<TxData>` in Transaction struct
- **Code says**: `extra_data: Vec<u8>` in Transaction struct
- **Drift**: Field name and type wrong in docs/protocol.md.
- **Fix**: Change to `extra_data: Vec<u8>`.

### 3. GossipSub topic count in docs/protocol.md
- **Doc says** (line 17): "7 topics"
- **Code says**: 8 topic constants defined, 7 subscribed in `subscribe_to_topics()`. The `/doli/t1/blocks/1` topic is defined but not in the default subscription.
- **Drift**: The doc table at section 6.1 lists 7 topics. Section 7.2 in specs/protocol.md lists 8. The overview should say "8 topics (7 subscribed by default + 1 tier-1 optional)".
- **Fix**: Update overview to "8 topics" and note tier1 as optional subscription.

### 4. StatusRequest in docs/protocol.md
- **Doc says** (line 76): `producer_pubkey: Option<[u8; 32]>` in StatusRequest
- **Code says**: `producer_pubkey: Option<PublicKey>` with `#[serde(default)]` — same semantics.
- **Verdict**: Accurate representation.

### 5. StatusResponse in docs/protocol.md
- **Doc says** (line 92): `best_slot: u32` in StatusResponse
- **Code says**: StatusResponse has `best_slot: u32` — matches.
- **Verdict**: Correct.

### 6. Tier1 topic in docs/protocol.md
- **Doc table** at section 6.1 lists 7 topics correctly (7 mandatory subscriptions)
- But section 7.2 in specs/protocol.md lists 8 topics including tier1
- Both need to distinguish mandatory vs optional subscription.

### 7. Peer scoring values in docs/protocol.md (line 397)
- **Doc says**: "Valid block received: +10", "Valid transaction received: +1"
- **Code says** (scoring.rs): PeerScore only has `valid_blocks` and `valid_transactions` counters. The `add()` method is called from `record_infraction()`. There's no explicit +10/+1 bonus — the code has `record_valid_block()` and `record_valid_tx()` which add +5 and +1 respectively.
- **Need to check**: Let me verify the actual bonus values.

### 8. Testnet genesis time in specs/protocol.md
- **Doc says** (line 1370): `1774749145 (testnet v96)`
- **Code says** (defaults.rs line 102): `1774749145` — testnet v96 genesis
- **Verdict**: Matches.

## docs/protocol.md Drift

### D1. Transaction struct `data` field
- **Doc says** (line 280): `data: Option<TxData>`
- **Code says**: `extra_data: Vec<u8>`
- **Fix**: Change field name and type.

### D2. Overview topic count
- **Doc says** (line 17): "7 topics"
- **Code says**: 8 defined, 7 subscribed by default
- **Fix**: Say "8 topics (7 default + 1 tier-1)"

## Summary of Fixes Applied

1. docs/protocol.md: Fix Transaction struct field from `data: Option<TxData>` to `extra_data: Vec<u8>`
2. docs/protocol.md: Fix topic count from "7" to "8"
3. Both files: Verify all consensus constants match code
