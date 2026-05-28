# Cargo Adversarial Findings Report (RUN_ID: 378)

## Scope
- File: `crates/network/src/gossip/config.rs` (gossip transmit cap)
- File: `crates/core/src/consensus/constants.rs` (consensus block size)
- Cross-layer invariant: gossip transport cap vs consensus validation cap

## Baseline
- Existing gossip module unit tests: 8 (all pass)
- No existing coverage for the gossip-cap-vs-block-size invariant
- Tooling: standard `#[test]` (proptest not needed for this invariant)

## Attack Surface Map
| Priority | Surface | Items Found | Written | Bugs Found |
|----------|---------|-------------|---------|------------|
| P0 | Cross-layer size mismatch | 1 | 3 | 1 (confirmed) |

## Bugs Found

### GOSSIP-SIZE-001: Gossip transmit cap (1 MB) < consensus block size (2 MB)
- **Location**: `crates/network/src/gossip/config.rs:89` (was bare `1024 * 1024`, now `GOSSIP_MAX_TRANSMIT_SIZE`)
- **Counterpart**: `crates/core/src/consensus/constants.rs:430` (`BASE_BLOCK_SIZE = 2_000_000`)
- **Trigger**: Any block with serialized size between 1,048,576 and 2,000,000 bytes
- **Symptom**: `gossipsub.publish()` returns `PublishError::MessageTooLarge`. Error is logged at WARN and dropped (`crates/network/src/service/command_handling.rs:47-52`). Block never reaches any peer. Producer slot goes empty. Next producer forks the gap.
- **Severity**: critical (latent fork trigger)
- **Reproduction**: `cargo test -p network test_gossip_transmit_cap -- --nocapture`

## Refactoring Applied
- Extracted `1024 * 1024` literal into `pub const GOSSIP_MAX_TRANSMIT_SIZE: usize = 1024 * 1024`
- Builder now references the const instead of the bare literal
- Pure no-op refactor: same value, same behavior, now testable

## Recommended Fix Options
1. Raise the gossip cap to match `MAX_BLOCK_SIZE_CAP` (32 MB)
2. Switch fresh-block propagation to announce-then-fetch (gossip header, fetch body via sync protocol)
3. Hybrid: raise to 2 MB now, implement announce-then-fetch before Era 1
