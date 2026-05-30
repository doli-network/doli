# Security Synthesis Reasoning Trace

## Auditor Reports Summary

### Auditor #1 — Injection / Input Validation
- **Perspective:** Malformed deserialization, type confusion, integer overflow, RPC DoS, key injection, signing canonicalization
- **Findings:** 0 P0, 0 P1, 1 P2 (iter_all DoS), 3 P3 (block scan, no domain prefix, missing test)
- **Key evidence:** `iter_all()` at oracle.rs:352 clones entire UTXO set; `signing_message` at data.rs:763-769 lacks domain prefix vs DelegateBondData at data.rs:293-298
- **Disproved:** usize-as-u64 truncation (widening), compute_structural_share_bps overflow (clamped), RocksDB key injection (from_hex validates)
- **Gaps:** Gossip-layer rate limits, mempool admission flow, RPC rate limiting, block deserialization cost

### Auditor #2 — Auth / Authorization
- **Perspective:** Producer set membership, bond weight integrity, pair authorization, equivocation, UTXO type enforcement, sunset HALT
- **Findings:** 0 P0, 1 P1 (active_producers empty in mempool/builder), 2 P2 (sunset not restored, no pair_id check), 2 P3 (no dedup, no explicit OraclePrice spend guard)
- **Key evidence:** pool.rs:255 ValidationContext has no .with_producers(); transaction.rs:242 always rejects; init.rs:681 hardcoded false
- **Disproved:** OraclePrice user creation blocked (transaction.rs:648-661), equivocation evidence verifies both sigs
- **Gaps:** Gossip admission, reorg handling (deferred to Logic), sunset race, CLI path

### Auditor #3 — Crypto / Data Protection
- **Perspective:** Signing canonicalization, replay protection, consensus determinism, data leakage, constant integrity
- **Findings:** 0 P0, 0 P1, 1 P2 (no domain prefix), 3 P3 (HashMap nondeterminism, drift-gate strength, HashSet in RPC)
- **Key evidence:** data.rs:763-769 bare 48-byte hash vs data.rs:293-298 domain-prefixed; HashMap in mod.rs:174 feeds sorted median (safe)
- **Disproved:** Replay (epoch_number committed + enforced), float (zero f32/f64 in consensus), PII leakage (all public), BLS (N/A), encoding ambiguity (fixed 50-byte)
- **Gaps:** bond_snapshot determinism, slashing crypto, ed25519-dalek CVEs, heartbeat.rs collision

### Auditor #4 — Business Logic / State
- **Perspective:** Median correctness, aggregation lifecycle, UTXO consume-and-recreate, dedup, sunset state machine, reorg interaction
- **Findings:** 1 P0 (undo log gap), 1 P1 (snap-sync missing blocks), 3 P2 (sunset not restored, Rule 5 not enforced, rollback doesn't reset sunset), 1 P3 (registered_at epoch mismatch)
- **Key evidence:** mod.rs:332-340 finalizes undo BEFORE oracle.rs:210,220 mutates UTXO set; oracle.rs:160-162 silently skips missing blocks
- **Kill tests passed:** HashMap iteration order safe (per-pair independent), let _ = remove intentional (first epoch)
- **Gaps:** State root inclusion of OraclePrice UTXOs, integration tests, mempool dedup, re-apply after rollback

### Auditor #5 — Supply Chain / Configuration
- **Perspective:** Activation-height safety, protocol version, RPC DoS, dependency hygiene, drift-gate, logging, test coverage
- **Findings:** 0 P0, 0 P1, 2 P2 (iter_all DoS, fee check blocks PriceAttestation), 4 P3 (doc drift, silent env parse, no integration tests, transitive dep advisories)
- **Key evidence:** core.rs:463-475 is_state_only missing PriceAttestation; pool.rs:464 rejects fee=0; server.rs:31-46 does not list getOracleStatus in ADMIN_METHODS
- **Disproved/Clean:** mainnet env override locked, protocol version not bumped, no genesis deps, drift gate active, logging clean, epoch range bounded, no new deps
- **Gaps:** Testnet activation state, production UTXO set size, end-to-end mempool test

## Deduplication Log

### Cluster 1: `getOracleStatus` iter_all DoS
- SEC-INJ-001 (Injection, P2, conf 0.7) + SEC-CONFIG-002 (Config, P2, conf 0.7)
- **Same location:** oracle.rs:350-363
- **Same vulnerability class:** CWE-400
- **Merge decision:** MERGE. Both independently identified the same `iter_all()` call on the same line, both noted it's unauthenticated (not in ADMIN_METHODS), both proposed same remediation (targeted lookup or caching). Config auditor added the ADMIN_METHODS asymmetry detail (other full-scan RPCs are admin-protected).
- **Merged finding:** AUDIT-P2-001, conf boosted to conf(0.8, converged)

### Cluster 2: `signing_message` missing domain prefix
- SEC-INJ-003 (Injection, P3, conf 0.6) + SEC-CRYPTO-001 (Crypto, P2, conf 0.6) + Auth cross-signal + Logic cross-signal
- **Same location:** data.rs:763-769
- **Same vulnerability class:** CWE-345 / weak domain separation
- **Merge decision:** MERGE. Three auditors independently identified the same gap. Injection called it P3 (design hygiene), Crypto called it P2 (must fix pre-activation). Both noted the DelegateBondData comparison at data.rs:293-298. Logic auditor noted the additional missing chain-identifier commitment.
- **Merged finding:** AUDIT-P2-002, take higher severity (P2), conf boosted to conf(0.75, converged)

### Cluster 3: `oracle_sunset_triggered` not restored on restart
- SEC-AUTH-002 (Auth, P2, conf 0.7) + SEC-LOGIC-003 (Logic, P2, conf 0.7)
- **Same location:** init.rs:681
- **Same vulnerability class:** CWE-665
- **Merge decision:** MERGE. Both independently identified the hardcoded `AtomicBool::new(false)`, both noted `state_db.get_oracle_sunset_state()` exists but is never called on startup, both proposed the same remediation.
- **Merged finding:** AUDIT-P2-003, conf boosted to conf(0.85, converged)

### Cluster 4: Rule 5 dedup not enforced (ERRTX_ORACLE_002 dead)
- SEC-AUTH-004 (Auth, P3, conf 0.7) + SEC-LOGIC-004 (Logic, P2, conf 0.7)
- **Same location:** transaction.rs:198-202, errors_oracle.rs:38
- **Same vulnerability class:** CWE-799
- **Merge decision:** MERGE. Both identified the dead error constant and the missing enforcement. Logic auditor rated P2 (last-mover advantage), Auth rated P3 (block space waste). Both noted aggregator dedup handles correctness but not efficiency.
- **Merged finding:** AUDIT-P2-004, take higher severity (P2), conf boosted to conf(0.85, converged)

### Cluster 5: HashMap in consensus-path aggregator
- SEC-CRYPTO-002 (Crypto, P3, conf 0.5) — single auditor
- Both Logic and Crypto auditors analyzed HashMap safety independently. Logic concluded safe (kill test). Crypto concluded safe but flagged as maintenance hazard.
- **No merge needed:** Single finding class. No boost.
- **Finding:** AUDIT-P3-005

### Cluster 6: Rollback doesn't reset sunset state
- SEC-LOGIC-005 (Logic, P2, conf 0.6) — single auditor with Logic-specific evidence
- Auth auditor noted "reorg handling deferred to Logic" in gaps. No independent evidence from other auditors.
- **No merge:** Single-auditor finding, keeps stated confidence.
- **Finding:** AUDIT-P2-005

### Cluster 7: No integration tests for oracle
- SEC-INJ-004 (Injection, P3 — missing ORACLE004 test), SEC-CONFIG-010 (Config, P3 — no node-level integration tests)
- **Different scope:** Injection flagged a specific test gap (ERRTX-ORACLE004 regression). Config flagged the broader integration test gap.
- **Merge decision:** Keep as separate findings. SEC-INJ-004 is a specific regression test gap; SEC-CONFIG-010 is a systemic integration test gap.
- **Findings:** AUDIT-P3-006 (specific), AUDIT-P3-007 (systemic)

## Convergence Analysis

### Convergence Matrix

```
                           Inject  Auth  Crypto  Logic  Config
AUDIT-P0-001 (undo log):    -       -      -      Y      -     -> 1/5 (single auditor)
AUDIT-P1-001 (producers):   -       Y      -      -      -     -> 1/5 (single auditor)
AUDIT-P1-002 (snap-sync):   -       -      -      Y      -     -> 1/5 (single auditor)
AUDIT-P1-003 (fee check):   -       -      -      -      Y     -> 1/5 (single auditor)
AUDIT-P2-001 (iter_all):    Y       -      -      -      Y     -> 2/5 converged
AUDIT-P2-002 (no domain):   Y       -      Y      -      -     -> 2/5 converged (+ cross-signals from Auth, Logic)
AUDIT-P2-003 (sunset init): -       Y      -      Y      -     -> 2/5 converged
AUDIT-P2-004 (no dedup):    -       Y      -      Y      -     -> 2/5 converged
AUDIT-P2-005 (rollback):    -       -      -      Y      -     -> 1/5 (single auditor)
AUDIT-P2-006 (pair_id):     -       Y      -      -      -     -> 1/5 (single auditor)
```

### Convergence Independence Checks

**AUDIT-P2-001 (iter_all DoS):**
- Auditor #1 (Injection): traced `iter_all()` -> `utxo/set.rs:144` clone path, measured O(N) allocation
- Auditor #5 (Config): compared against ADMIN_METHODS list at server.rs:31-46, noted asymmetry with other full-scan RPCs
- INDEPENDENT? YES — different evidence paths (memory allocation vs access control asymmetry)
- True convergence -> conf boost applies

**AUDIT-P2-002 (no domain prefix):**
- Auditor #1 (Injection): compared data.rs:763-769 vs data.rs:293-298 (DelegateBondData)
- Auditor #3 (Crypto): same comparison, plus analyzed BLAKE3 collision resistance + length-uniqueness implicit defense
- INDEPENDENT? PARTIALLY — same primary evidence (code comparison), but Crypto added independent cryptographic analysis
- Partial convergence -> conf boost applies (smaller)

**AUDIT-P2-003 (sunset not restored):**
- Auditor #2 (Auth): grep for init.rs hardcoded false, traced validation gate at transaction.rs:218
- Auditor #4 (Logic): grep for init.rs, traced epoch boundary re-derivation window
- INDEPENDENT? YES — Auth focused on validation bypass, Logic focused on state machine gap
- True convergence -> conf boost applies

**AUDIT-P2-004 (no dedup):**
- Auditor #2 (Auth): found dead ERRTX_ORACLE_002, noted block space waste
- Auditor #4 (Logic): found same dead constant, additionally analyzed last-mover advantage semantics
- INDEPENDENT? YES — Auth focused on authorization gap, Logic focused on economic impact
- True convergence -> conf boost applies

## Contradiction Analysis

### Potential Contradiction 1: SEC-LOGIC-001 severity
- Logic auditor rated P0 (conf 0.7). No other auditor found this vulnerability.
- Injection auditor mapped the UTXO mutation surface but did not trace the undo log timing.
- Auth auditor deferred reorg handling to Logic.
- **Resolution:** I verified the code directly. apply_block/mod.rs:332-340 finalizes undo BEFORE post_commit.rs:354 calls the oracle aggregator. oracle.rs:210,220 mutate the UTXO set AFTER undo finalization. rollback.rs:131-139 only iterates undo vectors. The claim is verified by code inspection. No contradiction — other auditors simply didn't trace this path.
- **Caveat:** The Logic auditor noted a GAP — whether OraclePrice UTXOs are in the state root. If yes, divergence would be caught at the next block apply (self-healing via rejection). If no, divergence is silent. This gap is unresolved and affects severity. I maintain P0 classification with the caveat that state-root inclusion could downgrade to P1.

### Potential Contradiction 2: Domain prefix severity
- Injection rated P3 (design hygiene). Crypto rated P2 (must fix pre-activation).
- **Resolution:** Both agree on the same evidence. The disagreement is severity assessment. Crypto's reasoning is stronger: this is a defense-in-depth gap that becomes PERMANENT after activation (signing_message format is consensus-frozen). Pre-activation fix cost is negligible. The urgency of "must fix before activation" justifies P2. No contradiction — just different severity thresholds.

### Potential Contradiction 3: SEC-AUTH-001 vs SEC-CONFIG-003 — same or different?
- Auth found: `active_producers` empty in mempool -> PriceAttestation rejected at producer check (transaction.rs:242)
- Config found: PriceAttestation not in `is_state_only()` -> rejected at fee check (pool.rs:464)
- **Resolution:** These are DIFFERENT bugs on DIFFERENT code paths. Both independently block PriceAttestation from entering the mempool. Even if one is fixed, the other still blocks. They are RELATED (both are liveness blockers at activation) but DISTINCT (different root causes, different locations, different fixes needed). Listed as separate findings.

No unresolved contradictions found.

## Coverage Analysis

| Perspective | Report Depth | Coverage Quality | Gaps |
|-------------|-------------|-----------------|------|
| Injection | Thorough (109 lines) | Strong — traced all deserialization, overflow, RPC input paths | Gossip-layer rate limits, mempool admission flow |
| Auth | Thorough (100 lines) | Strong — traced membership checks, sunset, type guards | Gossip admission, reorg handling, CLI path |
| Crypto | Thorough (118 lines) | Strong — traced signing, replay, determinism, constants | bond_snapshot determinism, ed25519-dalek CVEs, heartbeat collision |
| Logic | Thorough (187 lines) | Deep — traced undo log ordering, snap-sync gap, sunset state machine | State root inclusion, integration tests, re-apply after rollback |
| Config | Thorough (154 lines) | Broad — covered activation safety, dependency audit, test coverage | Testnet live state, production UTXO set size |

**No thin perspectives.** All 5 auditors produced substantive reports with verifiable evidence.

**Blind spot check:** Gossip-layer admission and rate limiting were flagged as gaps by 3/5 auditors (Injection, Auth, Config). This is a consistent gap across perspectives but is OUTSIDE the oracle audit scope (it's a pre-existing codebase-wide concern). The oracle subsystem inherits the existing gossip protections (or lack thereof).

**Cross-perspective gap:** The question of whether OraclePrice UTXOs are included in the state root computation was flagged by Logic auditor but not traced by any auditor. This is critical for determining SEC-LOGIC-001 severity.
