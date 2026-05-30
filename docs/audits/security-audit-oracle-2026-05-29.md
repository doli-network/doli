# Security Audit Report -- Oracle Subsystem (Phase 2.1)

**Date:** 2026-05-29
**Scope:** DOLI Phase 2.1 structural-anchored oracle (M1-M11), ~1,978 LOC
**Type:** Independent 5-perspective security audit (first audit of this subsystem)
**Status:** Read-only. No fixes applied. Code is shipped but frozen (`oracle_activation_height = u64::MAX` on mainnet/devnet; testnet = 20,099).

## Executive Summary

**No prior independent security audit of the oracle subsystem existed. This IS that audit.**

Five independent auditors examined the oracle subsystem from injection, authorization, cryptography, business logic, and configuration perspectives. They produced 25 raw findings, which after deduplication and merging yield **15 unique findings**: 1 P0, 3 P1, 6 P2, 5 P3, plus 1 systemic pattern.

The oracle subsystem's core cryptographic and arithmetic foundations are sound: Ed25519 signature verification is correct, integer arithmetic uses u128 promotion and saturating operations throughout, deserialization enforces strict length checks, and the bond-weighted median algorithm is deterministic. The `OutputType::OraclePrice` is properly hard-rejected for user transactions. Replay protection via epoch commitment is effective. No new external dependencies were added.

However, the subsystem has three independent liveness blockers that would prevent it from functioning at activation, a consensus-critical undo log gap that could cause state divergence during reorgs, and several deferred validation rules that create economic manipulation vectors.

**Deployment Recommendation:**

The `oracle_activation_height = u64::MAX` freeze on mainnet is CORRECT and should NOT be changed until the following are resolved:

- **MUST fix BEFORE pinning a real activation height (P0/P1):**
  - AUDIT-P0-001: OraclePrice UTXO mutations not tracked in undo log
  - AUDIT-P1-001: `active_producers` not wired in mempool/builder ValidationContext
  - AUDIT-P1-002: Aggregator silently skips missing blocks (snap-sync divergence)
  - AUDIT-P1-003: PriceAttestation rejected by mempool fee check (missing `is_state_only`)

- **SHOULD fix BEFORE pinning (P2):**
  - AUDIT-P2-001 through AUDIT-P2-006 (sunset initialization, domain prefix, dedup, rollback, pair_id)

- **Testnet note:** Testnet has `oracle_activation_height = 20,099`. If the chain has crossed this height, AUDIT-P1-001 and AUDIT-P1-003 are actively blocking oracle functionality there. This is harmless (oracle is inert) but should be verified.

## Summary

- **P0 (Critical):** 1 finding
- **P1 (High):** 3 findings
- **P2 (Medium):** 6 findings
- **P3 (Low):** 5 findings
- **Total:** 15 unique findings (25 raw, 15 after dedup)
- **Systemic patterns:** 1

## Systemic Patterns

### SYS-001: Deferred Validation Rules Create a Pre-Activation Debt

- **Affected findings:** AUDIT-P1-001, AUDIT-P1-003, AUDIT-P2-004, AUDIT-P2-006
- **Description:** Multiple validation rules documented in the spec (Rules 4 and 5) and integration requirements (mempool wiring, fee exemption) were deferred during development with comments like "deferred to M6" or "not yet implemented." The `oracle_activation_height = u64::MAX` freeze masks these gaps, but they collectively mean the subsystem cannot function at activation without completing the deferred work. Error constants exist for the deferred rules (`ERRTX_ORACLE_002`) but are dead code (`#[allow(dead_code)]`).
- **Impact:** The oracle subsystem is structurally incomplete. Activation without completing the deferred rules would result in: (a) complete liveness failure (no attestations can enter the mempool), (b) phantom OraclePrice UTXOs for nonexistent pairs, and (c) block space waste from duplicate attestations with last-mover advantage.
- **Remediation:** Before activation, audit and close every `// deferred to M6` comment in `validation/transaction.rs:198-202`. Create a pre-activation checklist that maps each deferred rule to its implementation ticket and integration test.

## Findings

### P0: Critical

#### AUDIT-P0-001: OraclePrice UTXO Mutations Not Tracked in Undo Log -- Rollback Past Epoch Boundary Leaves Stale Oracle UTXOs

- **Location:** `bins/node/src/node/apply_block/mod.rs:332-344`, `bins/node/src/node/apply_block/oracle.rs:195-231`, `bins/node/src/node/rollback.rs:122-140`
- **Vulnerability Class:** CWE-662 (Improper Synchronization) / UTXO State Integrity
- **Data Flow:** `apply_block/mod.rs:332-340` (undo finalized) -> `post_commit.rs:354` (oracle aggregator called) -> `oracle.rs:210` (`utxo.remove`) -> `oracle.rs:220` (`utxo.insert`) -- UTXO mutations occur AFTER undo data is sealed. On rollback: `rollback.rs:131-139` iterates only `undo.created_utxos` / `undo.spent_utxos` -- oracle changes absent.
- **Evidence:** Verified by code inspection. Line 340 of mod.rs writes `batch.put_undo(height, &undo)`. Line 344 calls `post_commit_actions()`. Inside post_commit.rs:354, `aggregate_oracle_prices_at_epoch_boundary` directly mutates `self.utxo_set` at oracle.rs:210 (remove) and 220 (insert). These mutations are NOT included in the undo vectors because undo was already finalized. On rollback, `rollback.rs:131-139` only reverses the undo vectors, leaving stale oracle UTXOs in the UTXO set.
- **Convergence:** 1/5 auditors (Logic). Other auditors did not trace the undo log timing. Auth deferred reorg handling to Logic. Injection mapped the UTXO mutation surface but did not trace post-commit ordering.
- **Confidence:** conf(0.7, observed) -- verified by synthesizer via direct code inspection. Severity caveat: if OraclePrice UTXOs are included in the state root, divergence would be caught at the next block apply (self-healing via block rejection). If not, divergence is silent. The state root inclusion question was not traced by any auditor and remains an OPEN GAP.
- **Impact:** After a reorg that unwinds an epoch boundary block: (1) the new-epoch OraclePrice UTXO remains in the UTXO set from the rolled-back chain, (2) the previous-epoch UTXO is not restored, (3) state root divergence between nodes that rolled back and nodes that did not. Self-healing occurs only if the new chain also reaches an epoch boundary with different attestations at the same outpoint.
- **Remediation:** Either (a) move oracle aggregation before undo finalization, passing the undo vectors down so oracle mutations are included, or (b) build a supplementary undo record for oracle changes in `post_commit_actions` and append to the batch before commit.
- **Test Strategy:** Integration test: create a chain that crosses an epoch boundary with oracle aggregation, then simulate a reorg that unwinds that boundary block. Verify that the OraclePrice UTXO is correctly reverted and the state root matches a fresh-synced node.

### P1: High

#### AUDIT-P1-001: Mempool and Block Builder Do Not Wire `active_producers` Into ValidationContext -- Oracle Liveness-Blocking at Activation

- **Location:** `crates/mempool/src/pool.rs:255-275`, `bins/node/src/node/production/assembly.rs:186-223`
- **Vulnerability Class:** CWE-862 (Missing Authorization) -- liveness class
- **Data Flow:** `PriceAttestation` tx -> `pool.add_transaction()` -> `ValidationContext::new(...)` (no `.with_producers()`) -> `validate_transaction()` -> `ctx.active_producers.contains(&signer)` at `transaction.rs:242` -> always false (empty Vec) -> REJECT
- **Evidence:** `pool.rs:255`: `ValidationContext::new(...)` has no `.with_producers()` or `.with_producers_weighted()` call. `validation/types.rs:264`: default `active_producers: Vec::new()`. `transaction.rs:242`: `if !ctx.active_producers.contains(&data.signer_pubkey)` -- always true with empty vec. Block validation at `validation_checks.rs:290` correctly calls `.with_producers_weighted(weighted)`, confirming the pattern is known but not applied to mempool/builder. Currently masked by `oracle_activation_height = u64::MAX` (height gate at `transaction.rs:207` rejects before reaching line 242).
- **Convergence:** 1/5 auditors (Auth). Config auditor found a different liveness blocker on the same path (fee check, AUDIT-P1-003). Both are independent blockers.
- **Confidence:** conf(0.7, observed)
- **Impact:** At activation, no PriceAttestation can enter the mempool or be included in locally-built blocks. Complete oracle liveness failure. Received blocks from other nodes would validate correctly (block validation wires producers), but local production and mempool admission are broken.
- **Remediation:** Wire `active_producers` (or `_weighted`) into the `ValidationContext` at `pool.rs:255-274` and `assembly.rs:186-223`. Alternatively, route PriceAttestation through `add_system_transaction` (requires adding to `is_state_only()` -- see AUDIT-P1-003).
- **Test Strategy:** Unit test: construct a `ValidationContext` with empty `active_producers`, submit a valid `PriceAttestation` from a known producer, assert rejection. Then construct with the producer included, assert acceptance.

#### AUDIT-P1-002: Aggregator Silently Skips Missing Blocks -- Snap-Synced Nodes Compute Different Medians

- **Location:** `bins/node/src/node/apply_block/oracle.rs:160-169`
- **Vulnerability Class:** CWE-754 (Improper Check for Unusual Conditions) / Consensus Divergence
- **Data Flow:** Aggregator scans `closing_epoch_start..closing_epoch_end` -> `block_store.get_block_by_height(h)` returns `Ok(None)` for missing blocks -> `continue` (silent skip) -> incomplete attestation set -> different `bond_weighted_median` result -> different OraclePrice UTXO -> state root divergence
- **Evidence:** oracle.rs:160-162: `Ok(None) => continue`. Snap sync targets are NOT constrained to epoch boundaries (confirmed: no epoch boundary alignment check in `fork_recovery.rs:280-410`). A node that snap-synced mid-epoch would be missing blocks from the first half of that epoch. Backfill is manual (MEMORY.md `feedback_backfill_procedure.md`), not automatic.
- **Convergence:** 1/5 auditors (Logic). The Logic auditor confirmed snap sync does not align to epoch boundaries.
- **Confidence:** conf(0.6, inferred) -- the condition requires specific timing (snap sync mid-epoch + epoch boundary with attestations), but is operationally realistic.
- **Impact:** A snap-synced node reaching its first epoch boundary with incomplete block history computes a different median price, producing a different OraclePrice UTXO and diverging the state root. This is a consensus fork triggered by normal operational procedure.
- **Remediation:** Either (a) abort aggregation if any block in the closing epoch is missing (convert `Ok(None) => continue` to error log + return), or (b) constrain snap sync to epoch boundary heights, or (c) require block backfill completion verification before allowing epoch-boundary aggregation.
- **Test Strategy:** Integration test: simulate a snap-synced node missing blocks in the closing epoch. Advance to epoch boundary. Verify the aggregator either aborts or produces the same result as a full-sync node.

#### AUDIT-P1-003: PriceAttestation Not in `is_state_only()` -- Fee Check Rejects at Activation

- **Location:** `crates/core/src/transaction/core.rs:463-475`, `crates/mempool/src/pool.rs:462-466`, `bins/node/src/node/validation_checks.rs:879`
- **Vulnerability Class:** CWE-684 (Incorrect Provision of Specified Functionality) -- liveness class
- **Data Flow:** `PriceAttestation` tx -> `is_state_only()` returns false (not in match list) -> routed to `add_transaction()` -> `fee = 0 - 0 = 0` (no inputs/outputs) -> `minimum_fee() = BASE_FEE(1) + 0 = 1` -> `fee(0) < min_fee(1)` -> rejected with `FeeTooLow`
- **Evidence:** `core.rs:463-475`: `is_state_only()` matches 8 types; `PriceAttestation` is absent. `core.rs:897-904`: PriceAttestation has `inputs: Vec::new(), outputs: Vec::new()`. `pool.rs:464`: `if fee < min_fee { return Err(MempoolError::FeeTooLow) }`. `consensus/constants.rs:613`: `BASE_FEE = 1`. Currently masked by height gate.
- **Convergence:** 1/5 auditors (Config). Auth auditor found a different liveness blocker on the same path (active_producers, AUDIT-P1-001). These are independent: fixing one does not fix the other.
- **Confidence:** conf(0.7, observed)
- **Impact:** When oracle is activated, PriceAttestation transactions cannot enter the mempool via any path (gossip or RPC). The oracle subsystem is non-functional. This compounds with AUDIT-P1-001 -- both must be fixed for oracle liveness.
- **Remediation:** Add `TxType::PriceAttestation` to `is_state_only()` at `core.rs:463-475`. Also add to the gossip routing exemption at `validation_checks.rs:879`.
- **Test Strategy:** Unit test: create a PriceAttestation tx, call `is_state_only()`, assert it returns true. Integration test: submit a PriceAttestation through the mempool path, verify it is not rejected with FeeTooLow.

### P2: Medium

#### AUDIT-P2-001: `getOracleStatus` Performs Unbounded Full UTXO Set Scan on Public RPC

- **Location:** `crates/rpc/src/methods/oracle.rs:350-363`
- **Vulnerability Class:** CWE-400 (Uncontrolled Resource Consumption)
- **Data Flow:** `RPC client (untrusted)` -> `getOracleStatus({})` -> `utxo_set.read().await` -> `iter_all()` at `utxo/set.rs:144` -> clones entire UTXO set as `Vec<(Outpoint, UtxoEntry)>` -> filters for `OraclePrice` -> `.max()` for `last_update_height`
- **Evidence:** oracle.rs:352: `utxo_set.iter_all().into_iter()`. The method is NOT in `ADMIN_METHODS` (server.rs:31-46), unlike other full-scan RPCs (`getStateSnapshot`, `getStateRootDebug`). No RPC rate limiting found. With Phase 2.1 having only one pair (DOLI/USD), a targeted `utxo_set.get(&oracle_price_outpoint(&pair_id))` would be O(1) instead of O(N).
- **Convergence:** 2/5 auditors (Injection + Config)
  - Injection: traced `iter_all()` -> `utxo/set.rs:144` clone path, identified O(N) allocation
  - Config: compared against ADMIN_METHODS list, noted asymmetric protection
  - INDEPENDENT? YES (allocation analysis vs access control analysis)
- **Confidence:** conf(0.8, converged)
- **Impact:** Sustained `getOracleStatus` calls cause memory spikes and hold the UTXO set read lock, blocking block application (UTXO writes). Severity depends on production UTXO set size (unknown -- flagged as gap).
- **Remediation:** Replace `iter_all()` with targeted `utxo_set.get(&oracle_price_outpoint(&pair_id))` lookups for known pair_ids, or cache `last_update_height` at the aggregator write site. Alternatively, add `getOracleStatus` to `ADMIN_METHODS`.
- **Test Strategy:** Benchmark: measure `getOracleStatus` execution time and memory allocation with varying UTXO set sizes (1K, 10K, 100K). Regression test: verify the targeted lookup produces the same result as the full scan.

#### AUDIT-P2-002: Missing Domain Separation in PriceAttestation Signing Message

- **Location:** `crates/core/src/transaction/data.rs:763-769`
- **Vulnerability Class:** CWE-345 (Insufficient Verification of Data Authenticity) / Weak Domain Separation
- **Data Flow:** `signing_message()` computes `BLAKE3(pair_id[32] || price_cents[8] || epoch_number[8])` = 48-byte preimage with NO domain prefix. Compare `DelegateBondData::signing_message` at `data.rs:293-298` which uses `DELEGATE_BOND_SIGNING_DOMAIN` prefix.
- **Evidence:** data.rs:763-769: bare `crypto::hash::hash(&buf)` on 48 bytes. data.rs:710-717: code comment acknowledges deviation, cites "spec approved 5/5 evaluators" -- but this is a process artifact, not a security justification. The spec approval predates the security audit. Current mitigation: length uniqueness (48 bytes vs other signing messages) provides implicit defense. No 48-byte attacker-controlled `crypto::hash::hash()` call found in the current codebase. The signing message also does NOT commit to a chain identifier (testnet attestation could replay on mainnet if same pair_id + same epoch number).
- **Convergence:** 2/5 auditors (Injection + Crypto), with cross-signals from Auth and Logic
  - Injection: compared against DelegateBondData pattern
  - Crypto: analyzed BLAKE3 collision resistance + length-uniqueness defense
  - INDEPENDENT? PARTIALLY (same code comparison, but Crypto added independent cryptographic analysis)
- **Confidence:** conf(0.75, converged)
- **Impact:** No current exploit path. Future risk if another non-domained 48-byte signing message is added, or if the same producers operate on mainnet and testnet with overlapping epoch numbers and pair_ids. This becomes PERMANENT after activation (format is consensus-frozen).
- **Remediation:** Add `b"PRICE_ATTESTATION"` prefix to `signing_message()` BEFORE activation. Update spec simultaneously. Cost is negligible pre-activation; impossible post-activation without a consensus-breaking change.
- **Test Strategy:** Unit test: verify `signing_message()` output includes the domain prefix. Golden vector test: pin a known (pair_id, price, epoch) -> expected hash with prefix.

#### AUDIT-P2-003: `oracle_sunset_triggered` Flag Not Restored From Persisted State on Node Restart

- **Location:** `bins/node/src/node/init.rs:681` (also 1164, 1365)
- **Vulnerability Class:** CWE-665 (Improper Initialization)
- **Data Flow:** Node startup -> `AtomicBool::new(false)` (hardcoded) -> flag = false regardless of persisted `OracleSunsetState` -> validation accepts PriceAttestations -> next epoch boundary re-derives (up to 60 min on mainnet)
- **Evidence:** `init.rs:681,1164,1365`: all hardcoded `Arc::new(AtomicBool::new(false))`. DB persistence methods exist (`state_db/queries.rs:526-544`). Aggregator persists at `apply_block/oracle.rs:107-118`. Startup never reads. grep confirms zero calls to `get_oracle_sunset_state` in `init.rs`.
- **Convergence:** 2/5 auditors (Auth + Logic)
  - Auth: traced validation gate at transaction.rs:218, identified acceptance window
  - Logic: traced epoch boundary re-derivation timing, identified state machine gap
  - INDEPENDENT? YES (validation bypass analysis vs state machine analysis)
- **Confidence:** conf(0.85, converged)
- **Impact:** After restart during a sunset HALT, the node accepts PriceAttestations for up to one epoch (~60 min mainnet). If this node is a block producer, it could include these txs in a block that peers reject (their sunset flag is correct), causing a fork. Coordinated fleet restart could briefly re-enable a sunset oracle.
- **Remediation:** In `Node::new()`, after constructing `state_db`, read `get_oracle_sunset_state()`, compute health for the current epoch, then `oracle_sunset_triggered.store(...)`.
- **Test Strategy:** Integration test: persist a sunset state, restart the node, verify the atomic flag is correctly initialized before any transaction validation occurs.

#### AUDIT-P2-004: Validation Rule 5 (At-Most-One PriceAttestation Per Attester Per Epoch Per Pair) Not Enforced

- **Location:** `crates/core/src/validation/transaction.rs:198-202`, `crates/core/src/validation/errors_oracle.rs:38`
- **Vulnerability Class:** CWE-799 (Improper Control of Interaction Frequency)
- **Data Flow:** Producer submits multiple PriceAttestations for same epoch+pair_id -> all pass validation (no dedup check) -> all included in blocks -> aggregator deduplicates at epoch boundary using LAST attestation per attester
- **Evidence:** `transaction.rs:198-202`: comment says "Rules deferred to M6." `errors_oracle.rs:38`: `ERRTX_ORACLE_002` defined with `#[allow(dead_code)]` -- never used for rejection. `oracle/mod.rs:171-178`: `dedupe_latest_per_attester` uses `HashMap::insert` (LAST wins) as defense-in-depth, not rejection. grep for `ORACLE002` in `bins/node/` returns zero matches.
- **Convergence:** 2/5 auditors (Auth + Logic)
  - Auth: found dead error constant, noted block space waste
  - Logic: analyzed last-mover advantage economic impact
  - INDEPENDENT? YES (authorization gap vs economic analysis)
- **Confidence:** conf(0.85, converged)
- **Impact:** Block space waste (DoS-adjacent). Last-mover advantage: a producer can update their price after seeing others' attestations, manipulating the median. No consensus divergence (aggregator dedup is deterministic). Equivocation slashing only catches DIFFERENT prices for the same epoch, not duplicate submissions.
- **Remediation:** Implement block-scope or mempool-scope dedup: track `(attester_hash, epoch, pair_id)` tuples, emit `ERRTX_ORACLE_002` on duplicate. The error constant already exists.
- **Test Strategy:** Unit test: submit two PriceAttestations from the same attester for the same epoch+pair. Verify the second is rejected with `ERRTX_ORACLE_002`.

#### AUDIT-P2-005: Rollback Does Not Reset `oracle_sunset_triggered` or `OracleSunsetState`

- **Location:** `bins/node/src/node/rollback.rs` (zero oracle references)
- **Vulnerability Class:** CWE-665 (Improper Initialization after State Change)
- **Data Flow:** Epoch boundary B sets sunset state -> rollback unwinds past B -> `oracle_sunset_triggered` atomic retains the rolled-back value -> `OracleSunsetState` in state_db retains the rolled-back value -> validation decisions on new chain are based on stale sunset state
- **Evidence:** grep for `oracle_sunset` or `sunset_triggered` in `rollback.rs` returns zero matches. The rollback code restores `epoch_state` from undo data (rollback.rs:277-279) but does not touch `OracleSunsetState` or the atomic flag.
- **Convergence:** 1/5 auditors (Logic)
- **Confidence:** conf(0.6, observed) -- single auditor, but evidence is clear (zero oracle references in rollback code is verifiable)
- **Impact:** Between rollback and next epoch boundary, the sunset flag may be incorrect. Could cause a node to reject or accept PriceAttestations incorrectly, potentially causing a fork if the node is a producer. Severity is modulated by the rarity of reorgs at epoch boundaries with differing structural share.
- **Remediation:** On rollback past an epoch boundary, reload OracleSunsetState for the target height and update the atomic flag.
- **Test Strategy:** Integration test: set sunset state at epoch boundary, simulate rollback past that boundary, verify the atomic flag is reset to the pre-boundary state.

#### AUDIT-P2-006: No Validation of `pair_id` Against Existing AMM Pools (Rule 4 Not Enforced)

- **Location:** `crates/core/src/validation/transaction.rs:199-202`, `bins/node/src/node/apply_block/oracle.rs:158-188`
- **Vulnerability Class:** CWE-862 (Missing Authorization) -- pair-level
- **Data Flow:** Producer attests arbitrary `pair_id` (including nonexistent) -> passes validation (no pool existence check) -> aggregator creates OraclePrice UTXO for phantom pair -> exposed by `getOraclePrice` RPC -> corrupts downstream consumers
- **Evidence:** `transaction.rs:199`: comment says Rule 4 (`pair_id corresponds to AMM pool with liquidity >= MINIMUM_LIQUIDITY`) is deferred. `tests_oracle.rs:14-17` confirms not implemented. Aggregator at oracle.rs:158-188 collects ALL PriceAttestations by pair_id without pool existence check.
- **Convergence:** 1/5 auditors (Auth)
- **Confidence:** conf(0.7, observed)
- **Impact:** Producers can create OraclePrice UTXOs for pairs that have no corresponding AMM pool, polluting the UTXO set. Downstream DeFi consumers (lending, liquidation) that rely on oracle prices would consume phantom data. With Phase 2.1's 12 structural producers, the attack requires at least one malicious producer.
- **Remediation:** Before activation, implement pair_id existence + min liquidity check in `validate_transaction_with_utxos` or a block-level pre-flight.
- **Test Strategy:** Unit test: submit a PriceAttestation with a pair_id that has no corresponding AMM pool. Verify rejection.

### P3: Low

#### AUDIT-P3-001: HashMap Nondeterminism in Consensus-Path Aggregator (Safe Today, Maintenance Hazard)

- **Location:** `crates/core/src/oracle/mod.rs:174-178`, `bins/node/src/node/apply_block/oracle.rs:94,158`
- **Vulnerability Class:** CWE-330 (Nondeterminism in Consensus Path)
- **Description:** `dedupe_latest_per_attester` uses `HashMap` whose `into_values()` iteration order is nondeterministic. Output feeds `bond_weighted_median` which sorts by price_cents, making the final result deterministic. `by_pair` HashMap iterates pairs independently -- each pair mutates a unique UTXO key. Currently safe. A future change adding an order-sensitive secondary effect would introduce a consensus fork.
- **Remediation:** Switch to `BTreeMap` in `dedupe_latest_per_attester` and `by_pair`. Eliminates the class with no measurable cost at <100 attesters.

#### AUDIT-P3-002: M11 Drift-Gate Passes on Coordinated Spec+Constant Dual Edit

- **Location:** `crates/rpc/src/methods/tests_oracle_m11.rs:74-119`, `crates/rpc/src/methods/oracle_status.rs:167-191`
- **Vulnerability Class:** Drift-gate strength (defense-in-depth)
- **Description:** The byte-equality test between spec and constant detects accidental drift but not coordinated malicious edits to both files. By design -- the gate prevents drift, not collusion.
- **Remediation:** Add a pinned BLAKE3 hash of the disclosure text as a third tripwire, making coordinated changes require updating three places (spec, constant, hash).

#### AUDIT-P3-003: `compute_structural_share_bps` Uses `registered_at` From Post-Mutation ProducerSet

- **Location:** `bins/node/src/node/apply_block/oracle.rs:84-102`
- **Vulnerability Class:** CWE-682 (Incorrect Calculation)
- **Description:** `registered_at_map` is built from the live ProducerSet (post-deferred-mutation), but `bond_snapshot` is from the closing epoch. A producer exited at this boundary would be excluded from `total_bonds_eligible` but their structural bonds might remain, slightly inflating the structural share metric. All nodes compute the same (slightly wrong) value deterministically -- no consensus divergence.
- **Remediation:** Build `registered_at_map` from `epoch_state.producer_list` (closing epoch) rather than the live ProducerSet.

#### AUDIT-P3-004: Missing Test for ERRTX-ORACLE004 (User OraclePrice Output Rejection)

- **Location:** `crates/core/src/validation/transaction.rs:648-661`
- **Vulnerability Class:** Test coverage gap
- **Description:** The production code correctly rejects user-created OraclePrice outputs. However, zero tests exist for the `ERRTX-ORACLE004` error code. The discriminant `15` is parsed from `from_u8(15)` during deserialization, meaning an attacker CAN construct the value. If the validation arm is accidentally removed during refactoring, there would be no regression test.
- **Remediation:** Add a validation test that constructs a Transfer transaction with an `OutputType::OraclePrice` output and asserts rejection with `ERRTX-ORACLE004`.

#### AUDIT-P3-005: No Node-Level Integration Tests for Oracle Subsystem

- **Location:** `bins/node/tests/` (no oracle test files)
- **Vulnerability Class:** Test coverage gap (systemic)
- **Description:** Unit tests in `oracle/tests.rs` and `tests_oracle.rs` exercise pure functions. RPC tests exercise handlers with mock contexts. But no test exercises the full path: gossip -> mempool -> block -> epoch boundary aggregation -> OraclePrice UTXO. The liveness blockers (AUDIT-P1-001, AUDIT-P1-003) would have been caught by an integration test.
- **Remediation:** Add at least one integration test that pins `oracle_activation_height = 0`, submits a PriceAttestation through the full node, advances to epoch boundary, and verifies the OraclePrice UTXO exists.

## Speculative Findings (low-confidence, requires manual review)

#### SPEC-001: OraclePrice UTXOs May or May Not Be in State Root

- **Source:** Logic auditor gap (SEC-LOGIC-001 caveat)
- **Confidence:** conf(0.4, inferred) -- no auditor traced the state root computation
- **Description:** If OraclePrice UTXOs are included in the state root, then AUDIT-P0-001 (undo log gap) would be caught by state root verification at the next block apply (self-healing via rejection), downgrading it to P1. If they are NOT in the state root, divergence is silent and persistent, confirming P0. No auditor traced the state root computation to answer this question.
- **Recommendation:** Manual review of `crates/storage/src/snapshot.rs` to determine whether `OutputType::OraclePrice` UTXOs contribute to the state root hash. This is the single highest-leverage verification for the entire audit.

#### SPEC-002: `heartbeat.rs` May Have 48-Byte Structural Collision With PriceAttestation Preimage

- **Source:** Crypto auditor gap
- **Confidence:** conf(0.3, inferred)
- **Description:** If `heartbeat.rs` uses `crypto::hash::hash()` on a 48-byte buffer with `[Hash[32] || u64 || u64]` structure, and the same producer signs both heartbeats and price attestations, a cross-type signature reuse could be possible (given AUDIT-P2-002's missing domain prefix).
- **Recommendation:** Manual review of `heartbeat.rs` signing path to verify no structural collision.

## Contradictions

No unresolved contradictions. All disagreements were resolved with evidence:

1. **Domain prefix severity** (Injection P3 vs Crypto P2): Resolved in favor of P2. Both agree on evidence; Crypto's reasoning is stronger -- the format becomes consensus-frozen at activation, making pre-activation fix cost negligible vs permanent gap.

2. **AUDIT-P1-001 vs AUDIT-P1-003**: These appeared potentially overlapping (both block PriceAttestation from mempool) but are confirmed as INDEPENDENT bugs requiring SEPARATE fixes. Active_producers check at transaction.rs:242 and fee check at pool.rs:464 are different code paths. Fixing one does not fix the other.

## Coverage Gaps

1. **State root inclusion of OraclePrice UTXOs** -- no auditor traced this. Critical for AUDIT-P0-001 severity. See SPEC-001.
2. **Gossip-layer rate limiting** -- 3/5 auditors flagged this gap. Pre-existing codebase-wide concern, not oracle-specific.
3. **Mempool admission flow** -- full path from `handle_network_event` to mempool insertion not traced by any auditor.
4. **`heartbeat.rs` signing collision** -- 48-byte structural overlap with PriceAttestation preimage not verified. See SPEC-002.
5. **`ed25519-dalek` CVE state** -- crate version (2.2.0) not checked against known advisories.
6. **Production UTXO set size** -- affects AUDIT-P2-001 severity but is unknown.
7. **Testnet activation state** -- whether testnet has crossed h=20,099 not verified.
8. **Re-application after rollback** -- whether the node automatically re-runs the oracle aggregator on re-applied blocks after rollback (potential self-healing for AUDIT-P0-001) was not traced.

## Verified Defenses (Clean)

The following areas were verified as correctly implemented by one or more auditors:

- **OraclePrice user creation** hard-rejected at `transaction.rs:648-661` (ERRTX-ORACLE004)
- **Mainnet env override** locked at `env_loader.rs:353-360` (testnet/devnet only)
- **Replay protection** via `epoch_number` commitment in `signing_message()` + exact epoch match in validation
- **Protocol version** not bumped (CURRENT_PROTOCOL_VERSION=8, EPOCH_STATE_FORMAT_VERSION=1 -- unchanged)
- **No genesis dependencies** -- forward-only activation, no genesis reset needed
- **M11 drift gate** active, not `#[ignore]`, correct spec path
- **Logging hygiene** -- no private keys, signatures, or PII in oracle log lines
- **Integer arithmetic** -- u128 promotion, saturating operations, `.min(10_000)` clamp throughout
- **Deserialization** -- strict 144-byte length check for PriceAttestationData, strict 50-byte for OraclePrice UTXO
- **No new external deps** -- oracle reuses existing crypto stack
- **Ed25519 signature verification** -- correct for both attestation and equivocation evidence
- **Bond-weighted median** -- integer-only, deterministic, lower-median tie-break documented and tested

## Transitive Dependency Advisories

6 advisories found via `cargo audit`, all in transitive dependencies (none oracle-specific):
- `hickory-proto 0.24.4` (RUSTSEC-2026-0119): CPU exhaustion via O(n^2) name compression
- `protobuf 2.28.0` (RUSTSEC-2024-0437): Uncontrolled recursion crash
- `ring 0.16.20` (RUSTSEC-2025-0009): AES panic with overflow checks
- `rustls-webpki 0.101.7` (RUSTSEC-2026-0104, RUSTSEC-2026-0098): CRL parsing panic + URI bypass

Recommendation: upgrade `libp2p` and `prometheus` to pull fixes. Not oracle-specific but affects the node binary.

## Synthesis Quality Gate

```
SYNTHESIS QUALITY GATE
Auditors completed:           5/5
Total raw findings:           25 (before dedup)
Total unique findings:        15 (after dedup)
Convergence clusters:         4
Contradictions found:         2
Contradictions resolved:      2/2
Attack perspectives covered:  Injection, Auth, Crypto, Logic, Config
Attack perspectives thin:     None (all substantive)
Systemic patterns detected:   1 (deferred validation debt)
```
