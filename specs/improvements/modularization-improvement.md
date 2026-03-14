# Requirements: Node.rs & Gossip.rs Modularization

## Scope
- `bins/node/src/node.rs` (7,910 lines) — the "god module"
- `crates/network/src/gossip.rs` (823 lines)

## Summary
Split two oversized Rust files into cohesive sub-modules where no single file exceeds 500 lines. The split is purely structural: no behavior changes, no public API changes, no new features. Every function moves intact into a new home.

## Proposed Module Split

### gossip.rs -> 4 sub-modules (crates/network/src/gossip/)

| Module | Contents | ~Lines |
|--------|----------|--------|
| `gossip/mod.rs` | Topic constants, GossipError, MeshConfig, PROTECTED_TOPICS, re-exports | ~60 |
| `gossip/config.rs` | new_gossipsub*, subscribe_to_topics*, topics_for_tier, reconfigure_topics_for_tier, compute_dynamic_mesh, region_topic | ~310 |
| `gossip/publish.rs` | All 9 publish functions, encode_tx_batch, decode_tx_message, TX_MSG_BATCH | ~170 |
| `gossip/tests.rs` | All 13 unit tests | ~264 |

### node.rs -> 12 sub-modules (bins/node/src/node/)

| Module | Contents | ~Lines | Risk |
|--------|----------|--------|------|
| `node/mod.rs` | Node struct, pub setters/getters, module declarations | ~180 | Low |
| `node/init.rs` | Node::new() | ~500 | Medium |
| `node/startup.rs` | run(), start_network(), start_rpc(), recompute_tier(), create_and_broadcast_attestation() | ~480 | Low |
| `node/event_loop.rs` | run_event_loop(), handle_network_event() | ~490 | Low |
| `node/block_handling.rs` | handle_new_block(), execute_reorg(), execute_fork_sync_reorg() | ~480 | HIGH |
| `node/fork_recovery.rs` | All 9 recovery/resync functions | ~490 | HIGH |
| `node/validation.rs` | check_producer_eligibility(), validate_block_*, handle_new_transaction(), handle_sync_request() | ~490 | Medium |
| `node/apply_block.rs` | apply_block() — the core state transition | ~1012* | EXTREME |
| `node/production.rs` | try_produce_block() — block production | ~1352* | EXTREME |
| `node/rewards_and_rollback.rs` | calculate_epoch_rewards(), rollback_one_block(), rebuild_producer_set_from_blocks(), handle_equivocation() | ~480 | EXTREME |
| `node/periodic.rs` | run_periodic_tasks(), maybe_bootstrap_maintainer_set(), flush_finalized_to_archive() | ~490 | Low-Medium |
| `node/genesis.rs` | derive_genesis_producers_from_chain(), genesis_bls_pubkeys(), consume_genesis_bond_utxos() | ~230 | HIGH |

*`apply_block.rs` and `production.rs` contain single monolithic functions that cannot be split without changing semantics. Documented exception to the 500-line target.

## Execution Order
1. gossip.rs split (smaller, lower risk, builds confidence)
2. node/mod.rs + node/genesis.rs (struct + leaf module)
3. node/init.rs (constructor)
4. node/startup.rs → event_loop.rs → validation.rs
5. node/apply_block.rs (critical core)
6. node/production.rs (block production)
7. node/block_handling.rs → fork_recovery.rs
8. node/rewards_and_rollback.rs → periodic.rs
9. CLAUDE.md code map update

## Constraints
- Zero logic changes — every function moved verbatim
- Each extraction = separate commit passing full test suite
- No reformatting of consensus-critical code
- All re-exports preserve existing import paths
- `cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test` after each step

## What Will NOT Change
- Public API (Node::new, Node::run, etc.)
- Behavior of any function
- State handling (Node struct stays in mod.rs)
- Gossip public API (re-exports in lib.rs)
- Test behavior
