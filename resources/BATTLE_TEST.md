# DOLI Pre-Production Battle Test Plan

**Version**: 2.0.0
**Status**: PRE-LAUNCH CHECKLIST
**Last Updated**: 2026-01-27
**Target**: Mainnet Genesis (2026-02-01T00:00:00Z)

---

## Executive Summary

This document defines comprehensive testing requirements for DOLI mainnet launch readiness, organized into **29 trackable milestones** across **8 phases**.

**MVP Status**: Feature complete with critical pre-launch items identified.

---

## Critical Blockers (MUST FIX FIRST)

| ID | Issue | Severity | Status |
|----|-------|----------|--------|
| BLK-001 | Placeholder maintainer keys in updater | **CRITICAL** | [ ] |
| BLK-002 | RPC sync status not implemented | HIGH | [ ] |
| BLK-003 | Unconfirmed balance calculation missing | HIGH | [ ] |
| BLK-004 | Error handling audit (46 files with unwrap) | MEDIUM | [ ] |

---

## Milestone Overview

```
PHASE 0: CRITICAL BLOCKERS
  └── M0: Fix Critical Blockers ────────────────────────┐
                                                        │
PHASE 1: FOUNDATION (parallel after M0)                 │
  ├── M1: Crypto - Hash & Signatures ◄──────────────────┤
  ├── M2: Crypto - VDF & Merkle ◄───────────────────────┤
  ├── M3: Core TX Types 0-4 ◄───────────────────────────┤
  ├── M4: Core TX Types 5-9 ◄───────────────────────────┤
  ├── M5: Wallet & CLI ◄────────────────────────────────┤
  └── M6: Node & RPC ◄──────────────────────────────────┘
                │
PHASE 2: SECURITY & CONSENSUS
  ├── M7: Double-Spend & Sybil ◄──── (M1, M2)
  ├── M8: Equivocation & Forks ◄──── (M1, M2)
  ├── M9: Grinding & Nothing-at-Stake ◄── (M1, M2)
  ├── M10: Eclipse & DoS ◄──── (M6)
  └── M11: Partition & Peer Scoring ◄── (M6)

PHASE 3: ECONOMICS & PERFORMANCE
  ├── M12: Emission & Halving ◄──── (M3, M4)
  ├── M13: Bonds & Slashing ◄──── (M3, M4)
  ├── M14: TX Throughput Stress ◄── (M3, M4, M6)
  └── M15: VDF & Storage Perf ◄──── (M2, M6)

PHASE 4: EDGE CASES (parallel)
  ├── M16: Temporal Boundaries
  ├── M17: Amount & Overflow
  ├── M18: Bond Lifecycle
  └── M19: Serialization & Fuzz ◄── (M1, M2, M3, M4)

PHASE 5: OPERATIONAL (parallel)
  ├── M20: Monitoring & Alerting
  ├── M21: Backup & Recovery
  └── M22: Security Hardening

PHASE 6: REGRESSION
  ├── M23: Previously Fixed Bugs
  └── M24: Full Test Suite ◄──── (M23)

PHASE 7: PRODUCTION DRY-RUN (7 days, sequential)
  ├── M25: Day 1-2 Setup ◄──── (M24)
  ├── M26: Day 3-4 Transactions ◄── (M25)
  ├── M27: Day 5-6 Stress ◄──── (M26)
  └── M28: Day 7 Sign-off ◄──── (M27)
```

---

## Phase 0: Critical Blockers

### Milestone 0: Fix Critical Blockers

**Priority**: BLOCKING - Must complete before any other testing
**Dependencies**: None

| Task | Description | Command/Location | Status |
|------|-------------|------------------|--------|
| Replace maintainer keys | Replace 5 placeholder keys with real Ed25519 public keys | `crates/updater/src/lib.rs` | [ ] |
| Implement sync status | Add sync state tracking to RPC | `crates/rpc/src/` | [ ] |
| Add unconfirmed balance | Calculate balance including mempool | `crates/rpc/src/` | [ ] |
| Audit unwrap calls | Review 46 files for panic-safe error handling | `grep -r "unwrap()" crates/` | [ ] |

**Verification**:
```bash
# Check placeholder keys are replaced
grep -r "0000000000" crates/updater/src/
# Expected: No matches

# Verify sync status endpoint works
curl localhost:8545 -d '{"jsonrpc":"2.0","method":"getNetworkInfo","id":1}' | jq '.result.syncing'
# Expected: true/false (not null)
```

**Exit Criteria**: All 4 blockers resolved, verified in code review

---

## Phase 1: Foundation Testing

### Milestone 1: Cryptographic Security - Hash & Signatures

**Priority**: High
**Dependencies**: M0
**Estimated Duration**: 2-4 hours

| Test | Description | Command | Status |
|------|-------------|---------|--------|
| Hash determinism | Same input = same output | `cargo test -p crypto hash_determinism` | [ ] |
| Hash collision | Different inputs = different outputs | `cargo test -p crypto hash_collision` | [ ] |
| Hash zero input | Empty input handled | `cargo test -p crypto hash_zero` | [ ] |
| Hash large input | 1MB+ input handled | `cargo test -p crypto hash_large` | [ ] |
| Domain separation | Domain tags unique | `cargo test -p crypto domain_separation` | [ ] |
| Sign/verify roundtrip | Valid signature verifies | `cargo test -p crypto sign_verify` | [ ] |
| Invalid sig rejection | Modified signature fails | `cargo test -p crypto invalid_signature` | [ ] |
| Wrong key rejection | Signature with wrong key fails | `cargo test -p crypto wrong_key` | [ ] |
| Deterministic sigs | Same message = same signature | `cargo test -p crypto deterministic_sig` | [ ] |
| Key serialization | Serialize/deserialize keys | `cargo test -p crypto key_serde` | [ ] |
| Private key zeroize | Keys cleared on drop | `cargo test -p crypto zeroize` | [ ] |

**Fuzz Testing**:
```bash
cd testing/fuzz
cargo +nightly fuzz run fuzz_hash -- -runs=1000000
cargo +nightly fuzz run fuzz_signature -- -runs=1000000
```

**Exit Criteria**: All tests pass, fuzz runs complete with 0 crashes

---

### Milestone 2: Cryptographic Security - VDF & Merkle

**Priority**: High
**Dependencies**: M0
**Estimated Duration**: 2-4 hours

| Test | Description | Command | Status |
|------|-------------|---------|--------|
| VDF compute/verify | Computed VDF verifies | `cargo test -p vdf roundtrip` | [ ] |
| VDF invalid proof | Tampered proof fails | `cargo test -p vdf invalid_proof` | [ ] |
| VDF wrong input | Wrong preimage fails | `cargo test -p vdf wrong_input` | [ ] |
| VDF iteration count | Short VDF fails T check | `cargo test -p vdf iteration_check` | [ ] |
| VDF determinism | Same input = same output | `cargo test -p vdf determinism` | [ ] |
| Selection seed vectors | Hardcoded test vectors pass | `cargo test -p vdf selection_seed` | [ ] |
| Registration difficulty | T increases with producer count | `cargo test -p vdf registration_difficulty` | [ ] |
| Merkle empty tree | Root is zero | `cargo test -p crypto merkle_empty` | [ ] |
| Merkle single element | Root is element hash | `cargo test -p crypto merkle_single` | [ ] |
| Merkle proof verify | Valid proofs verify | `cargo test -p crypto merkle_proof` | [ ] |
| Merkle invalid proof | Tampered proofs fail | `cargo test -p crypto merkle_invalid` | [ ] |

**Fuzz Testing**:
```bash
cd testing/fuzz
cargo +nightly fuzz run fuzz_vdf_verify -- -runs=100000
cargo +nightly fuzz run fuzz_merkle -- -runs=500000
```

**Exit Criteria**: All tests pass, fuzz runs complete with 0 crashes

---

### Milestone 3: Core Transactions - Types 0-4

**Priority**: High
**Dependencies**: M0
**Estimated Duration**: 2-3 hours

| TX Type | ID | Test | Status |
|---------|-----|------|--------|
| Transfer | 0 | `cargo test -p core tx_transfer` | [ ] |
| Registration | 1 | `cargo test -p core tx_registration` | [ ] |
| Exit | 2 | `cargo test -p core tx_exit` | [ ] |
| ClaimReward | 3 | `cargo test -p core tx_claim_reward` | [ ] |
| ClaimBond | 4 | `cargo test -p core tx_claim_bond` | [ ] |

**Additional Validation Tests**:

| Test | Description | Status |
|------|-------------|--------|
| Input references valid UTXO | `cargo test -p core utxo_reference` | [ ] |
| Signature matches pubkey | `cargo test -p core sig_verification` | [ ] |
| Sum inputs >= sum outputs | `cargo test -p core balance_check` | [ ] |
| All amounts positive | `cargo test -p core positive_amounts` | [ ] |
| Fee calculation correct | `cargo test -p core fee_calculation` | [ ] |

**Exit Criteria**: All 5 TX types validated, all validation tests pass

---

### Milestone 4: Core Transactions - Types 5-9

**Priority**: High
**Dependencies**: M0
**Estimated Duration**: 2-3 hours

| TX Type | ID | Test | Status |
|---------|-----|------|--------|
| SlashProducer | 5 | `cargo test -p core tx_slash_producer` | [ ] |
| Coinbase | 6 | `cargo test -p core tx_coinbase` | [ ] |
| AddBond | 7 | `cargo test -p core tx_add_bond` | [ ] |
| RequestWithdrawal | 8 | `cargo test -p core tx_request_withdrawal` | [ ] |
| ClaimWithdrawal | 9 | `cargo test -p core tx_claim_withdrawal` | [ ] |

**Slash-specific Tests**:

| Test | Description | Status |
|------|-------------|--------|
| Equivocation proof valid | Two blocks same slot | [ ] |
| Bond burned completely | 100% destroyed | [ ] |
| Producer removed from set | Immediate exclusion | [ ] |

**Exit Criteria**: All 5 TX types validated, slashing mechanics verified

---

### Milestone 5: Wallet & CLI Operations

**Priority**: High
**Dependencies**: M0
**Estimated Duration**: 1-2 hours

| Operation | Command | Expected | Status |
|-----------|---------|----------|--------|
| Create wallet | `doli-cli wallet new` | New keypair generated | [ ] |
| Import wallet | `doli-cli wallet import` | Restore from mnemonic | [ ] |
| Check balance | `doli-cli wallet balance <addr>` | Correct balance | [ ] |
| Send transaction | `doli-cli wallet send` | TX broadcast, confirmed | [ ] |
| List UTXOs | `doli-cli wallet utxos <addr>` | All UTXOs listed | [ ] |
| Export keys | `doli-cli wallet export` | Keys exported securely | [ ] |

**CLI Error Handling**:

| Test | Expected | Status |
|------|----------|--------|
| Invalid address format | Clear error message | [ ] |
| Insufficient balance | Clear error message | [ ] |
| Network unreachable | Timeout with message | [ ] |
| Invalid mnemonic | Rejection with reason | [ ] |

**Exit Criteria**: All wallet operations work, error handling graceful

---

### Milestone 6: Node Operations & RPC

**Priority**: High
**Dependencies**: M0
**Estimated Duration**: 2-3 hours

**Node Operations**:

| Operation | Command | Expected | Status |
|-----------|---------|----------|--------|
| Start node | `doli-node run` | Starts, syncs | [ ] |
| Start producer | `doli-node run --producer` | Produces blocks | [ ] |
| Connect testnet | `doli-node --network testnet run` | Connects, syncs | [ ] |
| Devnet isolation | `doli-node --network devnet --no-dht run` | No external peers | [ ] |
| Graceful shutdown | `SIGTERM` | Clean exit, DB flush | [ ] |

**RPC Endpoints**:

| Endpoint | Method | Test | Status |
|----------|--------|------|--------|
| getBlockByHash | POST | `curl -d '{"method":"getBlockByHash"...}'` | [ ] |
| getBlockByHeight | POST | `curl -d '{"method":"getBlockByHeight"...}'` | [ ] |
| getTransaction | POST | `curl -d '{"method":"getTransaction"...}'` | [ ] |
| sendTransaction | POST | `curl -d '{"method":"sendTransaction"...}'` | [ ] |
| getBalance | POST | `curl -d '{"method":"getBalance"...}'` | [ ] |
| getUtxos | POST | `curl -d '{"method":"getUtxos"...}'` | [ ] |
| getMempoolInfo | POST | `curl -d '{"method":"getMempoolInfo"...}'` | [ ] |
| getNetworkInfo | POST | `curl -d '{"method":"getNetworkInfo"...}'` | [ ] |
| getChainInfo | POST | `curl -d '{"method":"getChainInfo"...}'` | [ ] |

**RPC Test Script**:
```bash
#!/bin/bash
for method in getBlockByHeight getChainInfo getNetworkInfo getMempoolInfo; do
  echo "Testing $method..."
  curl -s -X POST http://localhost:8545 \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":{\"height\":0},\"id\":1}" \
    | jq '.result // .error'
done
```

**Exit Criteria**: All node ops work, all RPC endpoints return valid responses

---

## Phase 2: Security & Consensus Testing

### Milestone 7: Consensus - Double-Spend & Sybil Attacks

**Priority**: Critical
**Dependencies**: M1, M2
**Estimated Duration**: 3-4 hours

**Double-Spend Tests**:

| Attack | Test Scenario | Expected | Status |
|--------|---------------|----------|--------|
| Race attack | Two TXs same UTXO simultaneously | Only first confirmed | [ ] |
| Finney attack | Pre-mine block with double-spend | VDF prevents | [ ] |
| 51% attack | Majority producer reorg attempt | Weight-based fork resists | [ ] |

**Sybil Attack Tests**:

| Attack | Defense | Test | Status |
|--------|---------|------|--------|
| Mass registration | Chained VDF | `cargo test -p core sybil_mass_reg` | [ ] |
| Cheap identity flood | Bond requirement | `cargo test -p core sybil_cheap_id` | [ ] |
| Registration grinding | Chained VDF input | `cargo test -p core sybil_grinding` | [ ] |

**Integration Test**:
```bash
cargo test -p integration double_spend -- --nocapture
```

**Exit Criteria**: All attacks fail as expected, defenses verified

---

### Milestone 8: Consensus - Equivocation & Fork Attacks

**Priority**: Critical
**Dependencies**: M1, M2
**Estimated Duration**: 3-4 hours

**Equivocation Tests**:

| Test | Expected | Status |
|------|----------|--------|
| Detect double block same slot | EquivocationProof generated | [ ] |
| Slash transaction creation | Automatic slash TX | [ ] |
| Bond burn verification | 100% bond destroyed | [ ] |
| Producer exclusion | Removed from set | [ ] |
| Re-registration penalty | 2x VDF time | [ ] |

**Fork Attack Tests**:

| Attack | Defense | Test | Status |
|--------|---------|------|--------|
| Low-weight fork | Weight-based choice | `cargo test -p core fork_weight` | [ ] |
| Long-range attack | 4-year bond lock | `cargo test -p core long_range` | [ ] |
| Private chain | VDF time requirements | `cargo test -p core private_chain` | [ ] |

**Integration Test**:
```bash
cargo test -p integration equivocation_detection -- --nocapture
cargo test -p integration reorg_test -- --nocapture
cargo test -p integration attack_reorg_test -- --nocapture
```

**Exit Criteria**: Equivocation detected and slashed, fork attacks fail

---

### Milestone 9: Consensus - Grinding & Nothing-at-Stake

**Priority**: Critical
**Dependencies**: M1, M2
**Estimated Duration**: 2-3 hours

**Grinding Attack Tests**:

| Attack | Defense | Verification | Status |
|--------|---------|--------------|--------|
| Block grinding | Epoch lookahead | Selection independent of block | [ ] |
| VDF input grinding | prev_hash in input | Cannot pre-compute | [ ] |
| Selection seed manipulation | Deterministic seed | `cargo test -p vdf selection_seed_determinism` | [ ] |

**Nothing-at-Stake Tests**:

| Scenario | Defense | Test | Status |
|----------|---------|------|--------|
| Multi-chain production | Equivocation slashing | `cargo test -p core nothing_at_stake` | [ ] |
| Simultaneous blocks | 100% bond burn | Covered in M8 | [ ] |

**Verification**:
```bash
# Verify selection is deterministic
cargo test -p core producer_selection_deterministic

# Verify epoch lookahead
cargo test -p core epoch_lookahead
```

**Exit Criteria**: Grinding impossible, nothing-at-stake penalized

---

### Milestone 10: Network Security - Eclipse & DoS

**Priority**: High
**Dependencies**: M6
**Estimated Duration**: 2-3 hours

**Eclipse Attack Tests**:

| Test | Expected | Status |
|------|----------|--------|
| Single attacker peer | Diversity warning | [ ] |
| All peers same /16 | Diversity violation | [ ] |
| Sudden disconnection | Graceful degradation | [ ] |

**DoS Attack Tests**:

| Attack | Defense | Test | Status |
|--------|---------|------|--------|
| Message flooding | Rate limiting | `cargo test -p network rate_limiting` | [ ] |
| Invalid block spam | -100 peer score | `cargo test -p network invalid_block_penalty` | [ ] |
| Invalid TX spam | -20 peer score | `cargo test -p network invalid_tx_penalty` | [ ] |
| Connection exhaustion | Max peer limits | `cargo test -p network connection_limits` | [ ] |

**Stress Test**:
```bash
cargo test -p integration mempool_stress -- --nocapture
cargo test -p integration malicious_peer -- --nocapture
```

**Exit Criteria**: Eclipse detected, DoS mitigated, bad peers disconnected

---

### Milestone 11: Network Security - Partition & Peer Scoring

**Priority**: High
**Dependencies**: M6
**Estimated Duration**: 2-3 hours

**Network Partition Tests**:

| Scenario | Expected | Status |
|----------|----------|--------|
| 50/50 partition | Both sides continue | [ ] |
| Partition heals | Heavier chain wins | [ ] |
| Minority partition | Stops at orphan limit | [ ] |

**Peer Scoring Tests**:

| Infraction | Penalty | Test | Status |
|------------|---------|------|--------|
| InvalidBlock | -100 | `cargo test -p network score_invalid_block` | [ ] |
| InvalidTransaction | -20 | `cargo test -p network score_invalid_tx` | [ ] |
| Timeout | -5 to -50 | `cargo test -p network score_timeout` | [ ] |
| Spam | -50 | `cargo test -p network score_spam` | [ ] |
| Duplicate | -5 | `cargo test -p network score_duplicate` | [ ] |
| MalformedMessage | -30 | `cargo test -p network score_malformed` | [ ] |

**Integration Test**:
```bash
cargo test -p integration partition_heal -- --nocapture
```

**Exit Criteria**: Partition recovery works, scoring accurate

---

## Phase 3: Economics & Performance

### Milestone 12: Economic Model - Emission & Halving

**Priority**: High
**Dependencies**: M3, M4
**Estimated Duration**: 2 hours

**Emission Schedule Tests**:

| Era | Reward | Cumulative | Test | Status |
|-----|--------|------------|------|--------|
| 0 | 5.0 DOLI | 10,512,000 | `cargo test -p core emission_era_0` | [ ] |
| 1 | 2.5 DOLI | 15,768,000 | `cargo test -p core emission_era_1` | [ ] |
| 2 | 1.25 DOLI | 18,396,000 | `cargo test -p core emission_era_2` | [ ] |
| 3 | 0.625 DOLI | 19,710,000 | `cargo test -p core emission_era_3` | [ ] |

**Additional Tests**:

| Test | Command | Status |
|------|---------|--------|
| Halving calculation | `cargo test -p core block_reward_halving` | [ ] |
| Total supply cap | `cargo test -p core total_supply_cap` | [ ] |
| Coinbase maturity (100 blocks) | `cargo test -p core coinbase_maturity` | [ ] |

**Exit Criteria**: Emission matches whitepaper, supply capped at 21,024,000

---

### Milestone 13: Economic Model - Bonds & Slashing

**Priority**: High
**Dependencies**: M3, M4
**Estimated Duration**: 2-3 hours

**Bond Requirement Tests**:

| Era | Bond | Test | Status |
|-----|------|------|--------|
| 0 | 1,000 DOLI | `cargo test -p core bond_era_0` | [ ] |
| 1 | 700 DOLI | `cargo test -p core bond_era_1` | [ ] |
| 2 | 490 DOLI | `cargo test -p core bond_era_2` | [ ] |

**Bond Stacking Tests**:

| Test | Expected | Status |
|------|----------|--------|
| Add bond (1-100) | Count increases | [ ] |
| Anti-whale cap | Rejects >100 | [ ] |
| Round-robin allocation | Proportional slots | [ ] |
| Equal ROI % | Same % all producers | [ ] |

**Slashing Tests**:

| Violation | Penalty | Destination | Status |
|-----------|---------|-------------|--------|
| Double production | 100% | Burned | [ ] |
| Early exit (50%) | 50% | Rewards pool | [ ] |
| Early exit (25%) | 25% | Rewards pool | [ ] |

**Integration Test**:
```bash
cargo test -p integration bond_stacking -- --nocapture
```

**Exit Criteria**: Bonds scale correctly, slashing works as specified

---

### Milestone 14: Stress Test - Transaction Throughput

**Priority**: Medium
**Dependencies**: M3, M4, M6
**Estimated Duration**: 2-3 hours

**Throughput Targets**:

| Metric | Target | Test | Status |
|--------|--------|------|--------|
| TPS (Era 0, 1MB) | 66 TPS | Load test | [ ] |
| TX validation rate | 10,000/sec | Benchmark | [ ] |
| Signature verification | 15,000/sec | Benchmark | [ ] |

**Network Stress**:

| Test | Parameters | Expected | Status |
|------|------------|----------|--------|
| High peer count | 100+ peers | Stable | [ ] |
| Message flooding | 10,000 msg/sec | Rate limit activates | [ ] |
| Large mempool | 50,000 TXs | Memory stable | [ ] |

**Memory Stress**:

| Test | Limit | Expected | Status |
|------|-------|----------|--------|
| 100K blocks loaded | 4GB RAM | No OOM | [ ] |
| Large reorg (100 blocks) | 2GB RAM | Completes | [ ] |
| Mempool full (50K TXs) | 1GB | Eviction works | [ ] |

**Benchmark Commands**:
```bash
cd testing/benchmarks
cargo bench tx_throughput
cargo bench tx_validation
cargo bench sig_verification
```

**Exit Criteria**: All targets met, no OOM under stress

---

### Milestone 15: Stress Test - VDF & Storage Performance

**Priority**: Medium
**Dependencies**: M2, M6
**Estimated Duration**: 2-3 hours

**VDF Performance**:

| Test | Target | Status |
|------|--------|--------|
| Block VDF (~10M iter) | < 800ms | [ ] |
| Registration VDF (600M iter) | ~10 min | [ ] |
| VDF verification | < 100ms | [ ] |

**Storage Performance**:

| Test | Target | Status |
|------|--------|--------|
| Block write latency | < 10ms | [ ] |
| UTXO lookup | < 1ms | [ ] |
| Batch write (1000 TXs) | < 100ms | [ ] |
| DB size after 10K blocks | < 1GB | [ ] |

**Benchmark Commands**:
```bash
cd testing/benchmarks
cargo bench vdf_compute
cargo bench vdf_verify
cargo bench storage_write
cargo bench utxo_lookup
```

**Exit Criteria**: All performance targets met

---

## Phase 4: Edge Case Testing

### Milestone 16: Edge Cases - Temporal Boundaries

**Priority**: Medium
**Dependencies**: None (can run parallel)
**Estimated Duration**: 1-2 hours

| Case | Test | Expected | Status |
|------|------|----------|--------|
| Genesis block | Height 0 | Valid genesis | [ ] |
| Era boundary | Block at transition | Correct reward | [ ] |
| Epoch boundary | Slot at epoch end | Producer set updates | [ ] |
| Slot boundary | Block at exact end | Accepted in window | [ ] |
| Clock drift +120s | Max tolerance | Accepted | [ ] |
| Clock drift +121s | Beyond tolerance | Rejected | [ ] |
| Leap second | Time adjustment | Handled correctly | [ ] |

**Exit Criteria**: All boundary conditions handled correctly

---

### Milestone 17: Edge Cases - Amount & Overflow

**Priority**: Medium
**Dependencies**: None (can run parallel)
**Estimated Duration**: 1-2 hours

| Case | Expected | Status |
|------|----------|--------|
| Zero amount transfer | Rejected | [ ] |
| MAX_AMOUNT transfer | Accepted | [ ] |
| Overflow (MAX+1) | Rejected, no panic | [ ] |
| Dust output (1 sat) | Accepted | [ ] |
| Total supply exceeded | Rejected | [ ] |
| Negative amount attempt | Rejected (type safety) | [ ] |

**Overflow Protection Verification**:
```bash
# Count overflow protection calls
grep -rE "saturating_|checked_" crates/core/src/ | wc -l
# Expected: 70+

# Verify no unchecked arithmetic on amounts
cargo clippy -p core 2>&1 | grep -i "overflow"
```

**Exit Criteria**: No panics on edge values, overflow protected

---

### Milestone 18: Edge Cases - Bond Lifecycle

**Priority**: Medium
**Dependencies**: None (can run parallel)
**Estimated Duration**: 1-2 hours

| Case | Expected | Status |
|------|----------|--------|
| Exit at exactly 4 years | 0% penalty | [ ] |
| Exit at 3y 364d | ~0.07% penalty | [ ] |
| Exit at 0 days | 100% penalty | [ ] |
| Renewal at era boundary | New era bond amount | [ ] |
| Double exit request | Second rejected | [ ] |
| Exit cancellation | Returns to active | [ ] |
| Renewal during grace | Priority penalty | [ ] |
| Forced exit after grace | Bond released after unbonding | [ ] |

**Exit Criteria**: All lifecycle states handled correctly

---

### Milestone 19: Edge Cases - Serialization & Fuzz

**Priority**: Medium
**Dependencies**: M1, M2, M3, M4
**Estimated Duration**: 2-4 hours

| Case | Expected | Status |
|------|----------|--------|
| Empty transaction | Deserialize fails | [ ] |
| Oversized block (>1MB) | Rejected | [ ] |
| Malformed VDF proof | Deserialize fails | [ ] |
| Unicode in fields | Handled correctly | [ ] |
| Truncated data | Error, no crash | [ ] |
| Random bytes | Error, no crash | [ ] |

**Fuzz All Deserializers**:
```bash
cd testing/fuzz
cargo +nightly fuzz run fuzz_block_deserialize -- -runs=100000
cargo +nightly fuzz run fuzz_tx_deserialize -- -runs=100000
cargo +nightly fuzz run fuzz_vdf_verify -- -runs=100000
```

**Exit Criteria**: Zero crashes in 100K+ fuzz iterations each

---

## Phase 5: Operational Readiness

### Milestone 20: Operational - Monitoring & Alerting

**Priority**: Medium
**Dependencies**: None (can run parallel)
**Estimated Duration**: 2-3 hours

**Metrics Availability**:

| Metric | Available | Dashboard | Status |
|--------|-----------|-----------|--------|
| Block height | [ ] | [ ] | [ ] |
| Peer count | [ ] | [ ] | [ ] |
| Mempool size | [ ] | [ ] | [ ] |
| VDF timing | [ ] | [ ] | [ ] |
| Sync status | [ ] | [ ] | [ ] |
| Producer status | [ ] | [ ] | [ ] |

**Alerting Rules**:

| Alert | Trigger | Severity | Status |
|-------|---------|----------|--------|
| Node offline | No heartbeat 5min | Critical | [ ] |
| Sync behind | >10 blocks | Warning | [ ] |
| Fork detected | >20 orphans | Critical | [ ] |
| Peer count low | <3 peers | Warning | [ ] |
| VDF too slow | >55s | Warning | [ ] |
| Equivocation | Any | Critical | [ ] |

**Exit Criteria**: All metrics exposed, alerts configured

---

### Milestone 21: Operational - Backup & Recovery

**Priority**: Medium
**Dependencies**: None (can run parallel)
**Estimated Duration**: 2-3 hours

| Test | Procedure | Expected | Status |
|------|-----------|----------|--------|
| DB corruption recovery | Kill -9 during write | Recovers on restart | [ ] |
| Key backup restore | Restore from mnemonic | Wallet accessible | [ ] |
| Cold start sync | Fresh node syncs | Reaches tip | [ ] |
| Snapshot restore | Restore from snapshot | Resumes correctly | [ ] |
| Data directory backup | Copy data dir | Node starts from backup | [ ] |

**Log Analysis**:

| Log Type | Retention | Searchable | Status |
|----------|-----------|------------|--------|
| Block production | 30 days | [ ] | [ ] |
| Peer connections | 7 days | [ ] | [ ] |
| Errors/warnings | 90 days | [ ] | [ ] |
| Security events | 1 year | [ ] | [ ] |

**Exit Criteria**: Recovery works for all scenarios

---

### Milestone 22: Operational - Security Hardening

**Priority**: Medium
**Dependencies**: None (can run parallel)
**Estimated Duration**: 1-2 hours

| Item | Check | Status |
|------|-------|--------|
| Firewall enabled | `ufw status` | [ ] |
| RPC localhost only | Config check | [ ] |
| Non-root user | `whoami` != root | [ ] |
| Key permissions 600 | `stat ~/.doli/*.key` | [ ] |
| Data dir permissions 700 | `stat ~/.doli` | [ ] |
| NTP synchronized | `chronyc tracking` | [ ] |
| No debug builds | `cargo build --release` | [ ] |
| Secrets not in logs | Grep logs for keys | [ ] |

**Exit Criteria**: All hardening items verified

---

## Phase 6: Regression Testing

### Milestone 23: Regression - Previously Fixed Bugs

**Priority**: High
**Dependencies**: None
**Estimated Duration**: 1-2 hours

| Bug ID | Description | Test | Status |
|--------|-------------|------|--------|
| FORK-001 | Fork detection loop | `cargo test -p integration fork_no_loop` | [ ] |
| CLI-001 | Format mismatch | `cargo test -p cli format_test` | [ ] |
| DHT-001 | External peer contamination | `cargo test -p network no_dht_isolation` | [ ] |
| VDF-001 | Fixed T_BLOCK in validation | `cargo test -p core vdf_era_dependent` | [ ] |

**Exit Criteria**: All previously fixed bugs still fixed

---

### Milestone 24: Regression - Full Test Suite

**Priority**: High
**Dependencies**: M23
**Estimated Duration**: 1-2 hours

**Complete Test Run**:
```bash
#!/bin/bash
set -e

echo "=== Unit Tests ==="
cargo test 2>&1 | grep -E "test result|passed|failed"

echo "=== Integration Tests ==="
cargo test -p integration 2>&1 | grep -E "test result|passed|failed"

echo "=== Clippy ==="
cargo clippy --all-targets 2>&1 | grep -E "warning|error" | head -20

echo "=== Format Check ==="
cargo fmt --check

echo "=== Summary ==="
cargo test 2>&1 | tail -5
```

**Expected Results**:

| Suite | Expected | Status |
|-------|----------|--------|
| Unit tests | 427/427 pass | [ ] |
| Integration tests | 8/8 pass | [ ] |
| Clippy | 0 errors | [ ] |
| Format | Pass | [ ] |

**Exit Criteria**: 100% test pass rate, no linting errors

---

## Phase 7: Production Dry-Run

### Milestone 25: Production Dry-Run - Day 1-2

**Priority**: Critical
**Dependencies**: M24
**Duration**: 2 days

**Day 1 - Setup**:

| Task | Expected | Status |
|------|----------|--------|
| Deploy 5 producers | All connected | [ ] |
| Deploy 20 full nodes | All syncing | [ ] |
| Monitoring active | Dashboards live | [ ] |
| Genesis block produced | Height 0 valid | [ ] |

**Day 2 - Stability**:

| Check | Criteria | Status |
|-------|----------|--------|
| All producers connected | 5/5 online | [ ] |
| Blocks produced | 100+ blocks | [ ] |
| No reorgs | 0 reorgs > 1 block | [ ] |
| Sync healthy | All nodes at tip | [ ] |

**Exit Criteria**: Network stable, 100+ blocks, zero critical issues

---

### Milestone 26: Production Dry-Run - Day 3-4

**Priority**: Critical
**Dependencies**: M25
**Duration**: 2 days

**Day 3 - Transactions**:

| Task | Expected | Status |
|------|----------|--------|
| Submit 100 test TXs | All confirmed | [ ] |
| Transfer between wallets | Balances correct | [ ] |
| Fee collection | Producers receive fees | [ ] |

**Day 4 - Load Increase**:

| Check | Criteria | Status |
|-------|----------|--------|
| 1000+ TXs confirmed | All successful | [ ] |
| Mempool handling | No overflow | [ ] |
| RPC responsive | <1s response time | [ ] |

**Exit Criteria**: 1000+ TXs processed, network stable

---

### Milestone 27: Production Dry-Run - Day 5-6

**Priority**: Critical
**Dependencies**: M26
**Duration**: 2 days

**Day 5 - Stress Test**:

| Task | Expected | Status |
|------|----------|--------|
| 10x normal TX load | Network handles | [ ] |
| Simulate partition | Both sides continue | [ ] |
| Heal partition | Reorg to heavier chain | [ ] |

**Day 6 - Failover**:

| Check | Criteria | Status |
|-------|----------|--------|
| Stop 2 producers | Network continues | [ ] |
| Fallback activation | Backup producers work | [ ] |
| Restart producers | Resume production | [ ] |
| No double production | Zero equivocations | [ ] |

**Exit Criteria**: Stress handled, failover works, zero equivocations

---

### Milestone 28: Production Dry-Run - Day 7 & Sign-off

**Priority**: Critical
**Dependencies**: M27
**Duration**: 1 day

**Final Validation**:

| Check | Criteria | Status |
|-------|----------|--------|
| 7 days continuous operation | Yes | [ ] |
| Zero critical issues | Yes | [ ] |
| Zero unhandled panics | Check logs | [ ] |
| All producers operational | 5/5 | [ ] |
| Chain height correct | Expected blocks | [ ] |
| All test TXs confirmed | Yes | [ ] |

**Go/No-Go Criteria**:

**Must Pass (Blocking)**:
- [ ] All unit tests pass (427/427)
- [ ] All integration tests pass (8/8)
- [ ] No critical security issues
- [ ] Placeholder maintainer keys replaced
- [ ] RPC sync status implemented
- [ ] 7-day testnet run successful
- [ ] Zero unhandled panics in logs

**Should Pass (Non-blocking)**:
- [ ] Fuzz tests 1M+ iterations without crash
- [ ] Performance benchmarks meet targets
- [ ] Documentation complete
- [ ] Monitoring dashboards ready

**Sign-Off**:

| Role | Name | Date | Signature |
|------|------|------|-----------|
| Lead Developer | __________ | ________ | ________ |
| Security Reviewer | __________ | ________ | ________ |
| QA Lead | __________ | ________ | ________ |
| Project Lead | __________ | ________ | ________ |

**Exit Criteria**: All blocking criteria met, sign-offs obtained

---

## Appendix A: Quick Reference Commands

```bash
# Enter nix environment (REQUIRED)
nix develop

# Run all unit tests
cargo test 2>&1 | grep -E "test result" | tail -1

# Run specific crate
cargo test -p crypto
cargo test -p vdf
cargo test -p core
cargo test -p network
cargo test -p storage

# Integration tests
cargo test -p integration

# Fuzz tests
cd testing/fuzz
cargo +nightly fuzz run fuzz_block_deserialize -- -runs=100000

# Benchmarks
cd testing/benchmarks
cargo bench

# Linting
cargo clippy --all-targets

# Format
cargo fmt --check
```

---

## Appendix B: Milestone Dependency Graph

```
M0 ─────┬──► M1 ──┬──► M7
        │         ├──► M8
        ├──► M2 ──┼──► M9
        │         │
        ├──► M3 ──┼──► M12
        │         ├──► M13
        ├──► M4 ──┼──► M14
        │         │
        ├──► M5   │
        │         │
        └──► M6 ──┼──► M10
                  ├──► M11
                  └──► M15

M16, M17, M18 ──► (parallel, no deps)

M19 ◄── M1, M2, M3, M4

M20, M21, M22 ──► (parallel, no deps)

M23 ──► M24 ──► M25 ──► M26 ──► M27 ──► M28
```

---

## Appendix C: Progress Tracker

| Phase | Milestones | Completed | Status |
|-------|------------|-----------|--------|
| 0: Blockers | M0 | 0/1 | [ ] |
| 1: Foundation | M1-M6 | 0/6 | [ ] |
| 2: Security | M7-M11 | 0/5 | [ ] |
| 3: Economics | M12-M15 | 0/4 | [ ] |
| 4: Edge Cases | M16-M19 | 0/4 | [ ] |
| 5: Operational | M20-M22 | 0/3 | [ ] |
| 6: Regression | M23-M24 | 0/2 | [ ] |
| 7: Dry-Run | M25-M28 | 0/4 | [ ] |
| **TOTAL** | **29** | **0/29** | [ ] |

---

*Document Version: 2.0.0*
*Last Updated: 2026-01-27*
*Next Review: Before mainnet genesis*
