# Diagnostic Reasoning Trace: INC-I-015

## Explorer Round 1

- H1: Reactive scheduling diverges from different block sequences -- conf(0.65, inferred) -- Core hypothesis: different block sequences produce different scheduled flags
- H2: Undo-based rollback restores stale ProducerSet pre-epoch-boundary -- conf(0.55, inferred) -- Undo snapshot might not correctly capture epoch boundary state
- H3: missed_slot_scheduler in apply_block step 1 vs step 2 uses different lists -- conf(0.35, assumed) -- Potential ordering issue in reactive scheduling code
- H4: Block store contamination from fork blocks -- conf(0.50, inferred) -- get_block_by_height after rollback might return fork blocks
- H5: try_apply_cached_chain bypasses proper validation -- conf(0.45, inferred) -- Cached chain application might use diverged ProducerSet
- H6: execute_reorg Light mode skips bootstrap but uses diverged scheduled -- conf(0.40, inferred) -- Light mode validation might not catch scheduling divergence
- H7: prev_slot diverges because chain_state set differently during reorg -- conf(0.60, inferred) -- Different fork blocks have different slots, changing reactive scheduling

## Skeptic Round 1

- H1: SURVIVES -- conf(0.65, inferred) -- Connection storm demonstrably caused different block processing paths
- H2: WEAKENED -- conf(0.30, inferred) -- Code analysis shows undo snapshot + rebuild correctly handles epoch boundaries. Merges into H4.
- H3: ELIMINATED -- conf(0.0, inferred) -- Code handles ordering with pre_step1_scheduled snapshot matching apply_block's logic
- H4: SURVIVES -- conf(0.55, inferred) -- Plausible block store contamination after rollback
- H5: SURVIVES -- conf(0.45, inferred) -- Log evidence confirms N1 used try_apply_cached_chain and got "invalid producer for slot"
- H6: WEAKENED -- conf(0.30, inferred) -- Seed logs show rollback, not explicit reorg
- H7: SURVIVES -- conf(0.70, inferred) -- Strongest hypothesis: different blocks -> different scheduling

## Explorer Round 2 (refined)

- H7a: Seed block store has fork blocks, causing divergent rebuild -- conf(0.70, inferred)
  - After rollback, block store still has old blocks. rebuild_scheduled_from_blocks reads from store.
  - BUT: Code analysis shows rollback restores undo snapshot and rebuild replays from block store blocks that are still canonical (the rolled-back heights' blocks are the ones that WERE applied). This is correct.
  - The REAL issue is not in rebuild but in what happens AFTER rollback when new blocks arrive.

- H7b: apply_block skip-already-stored prevents re-application after rollback -- conf(0.80, inferred)
  - NEW DISCOVERY: In apply_block lines 27-55, the "already applied" guard uses `stored_height < height`.
  - After rollback from h=87 to h=85, blocks at h=86-87 are in the store with height indexes intact.
  - When canonical block at h=86 arrives via sync, `stored_height (86) < height (86)` is FALSE.
  - Block is SKIPPED as "already applied" -- but it was undone by rollback!
  - This forces the node to accept ONLY NEW blocks (from different forks), amplifying divergence.

## Skeptic Round 2

- H7a: SUBSUMED into H7b -- the real mechanism is the skip bug, not the rebuild
- H7b: CONFIRMED -- conf(0.85, inferred)
  - The skip condition `stored_height < height` does not account for rollback
  - After rollback, `stored_height == height` for rolled-back blocks, causing false "already applied" detection
  - This is the trigger mechanism that forces the seed onto a different chain

## Analogist

- Pattern: "phantom divergence from non-deterministic state reduction" in distributed systems
  - The scheduled flag is like a CRDT that requires causal delivery
  - Two nodes processing different events accumulate different state
  - Unlike explicit consensus state, the scheduled flag has no reconciliation mechanism

- Diagnostic approach from distributed databases:
  - "State derived from event history diverges when nodes have different event histories"
  - Solution pattern: "epoch-bounded state" -- reset derived state at boundaries, only accumulate within bounded windows
  - This matches the proposed epoch-bounded scheduling reset fix

## Diagnostic Tests Designed

### Test 1: Verify the skip-already-stored bug
- Setup: Start seed + 1 producer. Wait for h=5.
- Rollback: Force seed to roll back 2 blocks (h=5 -> h=3) via debug RPC or kill+restart.
- Observe: Does the seed re-apply blocks at h=4 and h=5, or does it skip them?
- Expected (if bug confirmed): Blocks at h=4-5 are skipped, state stays at h=3.
- Expected (if bug disproven): Blocks at h=4-5 are re-applied, state advances to h=5.

### Test 2: Verify scheduling divergence from different blocks
- Setup: Two nodes (A, B) processing the same chain to h=10.
- Inject divergence: At h=11, feed node A block X (producer P1, slot S) and node B block Y (producer P2, same slot S but different producer).
- Observe: Do nodes A and B compute different `scheduled_producers_at_height(12)`?
- Expected (if divergence confirmed): Different scheduled sets -> different rank assignments.

### Test 3: Reproduce full incident
- Setup: Seed + 5 producers on localhost.
- Stress: Deploy 100 stress nodes in 2 batches (mimicking the incident).
- Observe: Does the seed experience rollback? Does "invalid producer for slot" appear?
- This is a full reproduction of the incident conditions.

## Results

Tests not yet run -- diagnosis is based on code analysis. The code evidence is strong enough (conf 0.85) for root cause confirmation without runtime experiments, but Test 1 would elevate to conf(0.95, measured).
