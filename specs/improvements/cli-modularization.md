# CLI main.rs Modularization

## Scope
`bins/cli/src/main.rs` (6,957 lines) → 13 domain modules, ~500 lines each where practical.

## Module Split

| # | Module | Contents | ~Lines |
|---|--------|----------|--------|
| 1 | `main.rs` | mod declarations, main(), dispatch, tests | ~280 |
| 2 | `commands.rs` | Cli struct + 9 Subcommand enums | ~660 |
| 3 | `common.rs` | ADDRESS_PREFIX, address_prefix(), expand_tilde(), network helpers | ~50 |
| 4 | `parsers.rs` | parse_condition, resolve_to_hash, condition_to_output_type, parse_witness | ~210 |
| 5 | `cmd_wallet.rs` | 14 wallet functions (cmd_new through cmd_verify) | ~730 |
| 6 | `cmd_producer.rs` | cmd_producer + FifoBreakdown + helpers | ~1300 |
| 7 | `cmd_nft.rs` | 10 NFT functions | ~1395 |
| 8 | `cmd_upgrade.rs` | cmd_upgrade + service restart helpers + find_doli_node_path | ~470 |
| 9 | `cmd_governance.rs` | release + protocol + update + maintainer (7 functions) | ~460 |
| 10 | `cmd_chain.rs` | cmd_chain, cmd_chain_verify, cmd_rewards, cmd_wipe | ~275 |
| 11 | `cmd_bridge.rs` | cmd_bridge_lock, cmd_bridge_claim, cmd_bridge_refund | ~400 |
| 12 | `cmd_token.rs` | cmd_issue_token, cmd_token_info | ~225 |
| 13 | `cmd_channel.rs` | cmd_channel | ~485 |

## Requirements

| ID | Priority | Module | Acceptance |
|----|----------|--------|------------|
| REQ-CLI-001 | Must | commands.rs | All clap types extracted, --help identical |
| REQ-CLI-002 | Must | common.rs | Shared utils accessible from all modules |
| REQ-CLI-003 | Must | parsers.rs | Condition/witness parsers shared correctly |
| REQ-CLI-004 | Must | cmd_wallet.rs | 14 wallet functions, all commands work |
| REQ-CLI-005 | Must | cmd_producer.rs | Producer commands work identically |
| REQ-CLI-006 | Must | cmd_nft.rs | NFT commands work identically |
| REQ-CLI-008 | Must | cmd_upgrade.rs | Platform #[cfg] blocks preserved |
| REQ-CLI-009 | Must | cmd_governance.rs | Governance commands work identically |
| REQ-CLI-010 | Should | cmd_chain.rs | Chain/rewards/wipe commands work |
| REQ-CLI-011 | Must | cmd_bridge.rs | Bridge commands work identically |
| REQ-CLI-012 | Should | cmd_token.rs | Token commands work identically |
| REQ-CLI-013 | Must | cmd_channel.rs | Channel commands work identically |
| REQ-CLI-014 | Must | main.rs | < 300 lines, entrypoint only |
| REQ-CLI-016 | Must | All | cargo build && clippy && fmt && test pass |

## Constraints
- Zero logic changes — every function moved verbatim
- Each extraction = separate commit passing full gate
- All re-exports use pub(crate) visibility
- Commit author: --author "Ivan D. Lozada <ivan@doli.network>"

## Execution Order
1. common.rs → 2. parsers.rs → 3. commands.rs → 4. cmd_wallet.rs → 5. cmd_upgrade.rs → 6. cmd_governance.rs → 7. cmd_producer.rs → 8. cmd_nft.rs → 9. cmd_bridge.rs → 10. cmd_token.rs → 11. cmd_channel.rs → 12. cmd_chain.rs → 13. Verify main.rs < 300 lines
