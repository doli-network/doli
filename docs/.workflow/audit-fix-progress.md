# Oracle Audit Fix Progress

**Audit:** `docs/audits/security-audit-oracle-2026-05-29.md`
**Branch:** `defi/foundations`
**Scope:** All 15 findings + cargo audit dep advisories (Option C)
**Activation height policy:** unchanged — `oracle_activation_height = u64::MAX` on mainnet/devnet throughout.

## Status Legend
- ⏳ PENDING — not started
- 🔄 IN-PROGRESS — doctor running
- ✅ FIXED — doctor returned OK, committed, tests green
- ⚠️ ESCALATED — doctor hit hydra / architectural issue, deferred
- 🚫 SKIPPED — explicitly skipped (with reason)

## P0 → P1 (downgrade verified via `test_oracle_price_changes_state_root`)
| ID | Status | Title | Files |
|----|--------|-------|-------|
| AUDIT-P0-001 (→P1) | ⏳ | OraclePrice UTXO mutations not in undo log | `apply_block/{mod.rs,oracle.rs,post_commit.rs}`, `rollback.rs` |

## P1
| ID | Status | Title | Files |
|----|--------|-------|-------|
| AUDIT-P1-001 | ⏳ | Wire `active_producers` into mempool + builder ValidationContext | `mempool/pool.rs`, `node/production/assembly.rs` |
| AUDIT-P1-002 | ⏳ | Aggregator silently skips missing blocks → snap-sync divergence | `apply_block/oracle.rs:160` |
| AUDIT-P1-003 | ⏳ | `PriceAttestation` missing from `is_state_only()` → fee rejection | `transaction/core.rs`, `validation_checks.rs:879` |

## P2
| ID | Status | Title | Files |
|----|--------|-------|-------|
| AUDIT-P2-001 | ⏳ | `getOracleStatus` unbounded UTXO scan → DoS | `rpc/methods/oracle.rs:350-363` |
| AUDIT-P2-002 | ⏳ | Missing domain separation in PriceAttestation signing message | `transaction/data.rs:763-769`, `specs/oracle-structural-anchored-economics.md` |
| AUDIT-P2-003 | ⏳ | `oracle_sunset_triggered` not restored on restart | `node/init.rs:681,1164,1365` |
| AUDIT-P2-004 | ⏳ | Validation Rule 5 (per-attester-per-epoch dedup) not enforced | `validation/transaction.rs:198-202`, `errors_oracle.rs:38` |
| AUDIT-P2-005 | ⏳ | Rollback does not reset sunset state | `node/rollback.rs` |
| AUDIT-P2-006 | ⏳ | Validation Rule 4 (`pair_id` → AMM pool) not enforced | `validation/transaction.rs:199` |

## P3
| ID | Status | Title | Files |
|----|--------|-------|-------|
| AUDIT-P3-001 | ⏳ | HashMap → BTreeMap in consensus-path aggregator | `oracle/mod.rs:174-178`, `apply_block/oracle.rs:94,158` |
| AUDIT-P3-002 | ⏳ | M11 drift gate dual-edit defense (pinned hash) | `rpc/methods/oracle_status.rs`, `tests_oracle_m11.rs` |
| AUDIT-P3-003 | ⏳ | `compute_structural_share_bps` uses live ProducerSet (should be closing epoch) | `apply_block/oracle.rs:84-102` |
| AUDIT-P3-004 | ⏳ | Missing test for ERRTX-ORACLE004 (user OraclePrice rejection) | `validation/transaction.rs:648-661` |
| AUDIT-P3-005 | ⏳ | No node-level integration test for oracle | `bins/node/tests/` |

## Speculative (manual review only)
| ID | Status | Description |
|----|--------|-------------|
| SPEC-001 | ✅ RESOLVED | OraclePrice IS in state root (verified via `test_oracle_price_changes_state_root`); downgrades P0-001 to P1 |
| SPEC-002 | ⏳ | `heartbeat.rs` 48-byte preimage collision — manual review needed |

## Dep advisories (cargo audit transitive)
| Advisory | Status | Notes |
|----------|--------|-------|
| RUSTSEC-2026-0119 (hickory-proto) | ⏳ | Bump libp2p |
| RUSTSEC-2024-0437 (protobuf) | ⏳ | Bump libp2p |
| RUSTSEC-2025-0009 (ring) | ⏳ | Bump libp2p |
| RUSTSEC-2026-0104 / 0098 (rustls-webpki) | ⏳ | Bump libp2p |

## Priority Gates
- After P1 batch → full test suite + cargo clippy `-D warnings`
- After P2 batch → full test suite + cargo clippy `-D warnings`
- After P3 batch → full test suite + cargo clippy `-D warnings`
- Final → run all oracle unit tests + integration test + drift-gate test
