  DOLI Network Sync Bug Investigation Report

  Summary

  During testing of Producer 5 with 10 bonds, multiple critical sync bugs were discovered and fixed, but Producer 5 is still not earning weighted rewards due to a separate issue with heartbeat
  broadcasting.

  Bugs Fixed (Committed)

  1. Infinite Loop in run_periodic_tasks() (Critical)

  File: bins/node/src/node.rs

  Root Cause: In a previous session, a loop was added around next_request() to enable parallel body downloads. However, for the DownloadingHeaders state, next_request() always returns a new request
  without any limit, causing an infinite loop that deadlocked the node.

  Fix: Reverted to single if let Some(...) pattern instead of loop { match ... }.

  2. Body Download Responses Not Processed (Critical)

  File: crates/network/src/sync/manager.rs:470-520

  Root Cause: handle_bodies_response() was not calling body_downloader.process_response(), leaving:
  - Peer marked as "busy" forever in active_requests
  - Block hashes stuck in in_flight set, never retried

  Fix: Added body_downloader.process_response(peer, bodies) call.

  3. Timeout Cleanup Missing

  File: crates/network/src/sync/manager.rs:643

  Root Cause: cleanup() never called body_downloader.cleanup_timeouts(), so timed-out body requests were never retried.

  Fix: Added cleanup call.

  4. Incomplete Reset on Forced Resync

  File: crates/network/src/sync/manager.rs:577-600

  Root Cause: reset_local_state() didn't clear pending_headers, pending_blocks, headers_needing_bodies, or reset the downloaders, causing stale state after resync.

  Fix: Added full state cleanup.

  5. Post-Resync Grace Period Too Short

  File: bins/node/src/node.rs:2147-2175

  Root Cause: 5 seconds for devnet was insufficient for syncing before production resumed, causing nodes to produce on their own fork.

  Fix: Increased to 30 seconds for all networks, plus added slot-height diff check.

  ---
  Current Issue: Producer 5 Not Earning Weighted Rewards

  Observations

  - Producer 5 registered: 10 bonds, status "active"
  - Node running: Synced to correct height (146+)
  - Heartbeats NOT broadcasting: No "Computing heartbeat" or "Broadcasting heartbeat" logs
  - All received heartbeats rejected: "previous block hash mismatch" for ALL incoming heartbeats

  Reward Distribution Analysis
  ┌───────┬────────────────────┬────────────────────────────────┐
  │ Epoch │ Producers Rewarded │      Producer 5 Included?      │
  ├───────┼────────────────────┼────────────────────────────────┤
  │ 29    │ 4 producers        │ Yes (0.67 DOLI for 1/4 blocks) │
  ├───────┼────────────────────┼────────────────────────────────┤
  │ 33    │ 3 producers        │ No                             │
  ├───────┼────────────────────┼────────────────────────────────┤
  │ 34    │ 3 producers        │ No                             │
  └───────┴────────────────────┴────────────────────────────────┘
  Likely Root Cause

  Producer 5 node is not broadcasting heartbeats because:
  1. Node may not recognize itself as eligible to produce (bootstrap mode vs registered mode conflict)
  2. Heartbeat hash validation failing for its own chain state
  3. The "previous block hash mismatch" rejection affects its own heartbeat generation

  The heartbeat system uses prev_hash from the node's chain state. If Producer 5's state is slightly different from other nodes (even though at same height), heartbeats won't match.

  ---
  Network Status
  ┌────────────┬───────┬────────┬────────────────────────────────────────────┐
  │    Node    │ Port  │ Height │                   Status                   │
  ├────────────┼───────┼────────┼────────────────────────────────────────────┤
  │ Producer 1 │ 28545 │ 146+   │ ✅ Working, earning rewards                │
  ├────────────┼───────┼────────┼────────────────────────────────────────────┤
  │ Producer 2 │ 28546 │ 146+   │ ✅ Working, earning rewards                │
  ├────────────┼───────┼────────┼────────────────────────────────────────────┤
  │ Producer 3 │ 28547 │ 146+   │ ✅ Working (lower rewards - was restarted) │
  ├────────────┼───────┼────────┼────────────────────────────────────────────┤
  │ Producer 4 │ 28548 │ 146+   │ ✅ Working, earning rewards                │
  ├────────────┼───────┼────────┼────────────────────────────────────────────┤
  │ Producer 5 │ 28549 │ 146+   │ ⚠️ Synced but NOT producing heartbeats     │
  └────────────┴───────┴────────┴────────────────────────────────────────────┘
  Wallet Balances (Height ~146)
  ┌──────────┬─────────────┬─────────────┬─────────────┬───────┐
  │ Producer │  Confirmed  │  Immature   │    Total    │ Bonds │
  ├──────────┼─────────────┼─────────────┼─────────────┼───────┤
  │ 1        │ 155.67 DOLI │ 659.99 DOLI │ 815.67 DOLI │ 1     │
  ├──────────┼─────────────┼─────────────┼─────────────┼───────┤
  │ 2        │ 166.67 DOLI │ 666.67 DOLI │ 833.33 DOLI │ 1     │
  ├──────────┼─────────────┼─────────────┼─────────────┼───────┤
  │ 3        │ 120.00 DOLI │ 0.00 DOLI   │ 120.00 DOLI │ 1     │
  ├──────────┼─────────────┼─────────────┼─────────────┼───────┤
  │ 4        │ 166.67 DOLI │ 666.67 DOLI │ 833.33 DOLI │ 1     │
  ├──────────┼─────────────┼─────────────┼─────────────┼───────┤
  │ 5        │ 0.99 DOLI   │ 6.67 DOLI   │ 7.67 DOLI   │ 10    │
  └──────────┴─────────────┴─────────────┴─────────────┴───────┘
  Expected: Producer 5 with 10 bonds should earn ~10x more than 1-bond producers when present.

  ---
  What Would Need Further Investigation

  1. Why isn't Producer 5 broadcasting heartbeats?
    - Check if bootstrap mode logic is interfering with registered producer logic
    - Verify heartbeat eligibility checks
  2. Why are ALL heartbeats rejected with "hash mismatch"?
    - Producer 5's chain state might be microseconds behind
    - Need to investigate heartbeat validation timing
  3. Is the weighted reward calculation working correctly?
    - Epoch 29 showed Producer 5 earning rewards, but proportional to presence (1/4), not bonds
    - Need to verify if effective_weight() considers bond count

  ---
  Files Changed

  - bins/node/src/node.rs - Post-resync grace period, slot-height check
  - crates/network/src/sync/manager.rs - Body download fixes, reset cleanup
