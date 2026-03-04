---
name: workflow:stress-test
description: Black-box stress testing of the DOLI blockchain -- pushes CLI, RPC, and node operations to their breaking point to discover crashes, corrupt states, and protocol violations.
---

# Workflow: Stress Test

Invoke ONLY the `stress-tester` subagent for black-box stress testing of the DOLI blockchain against a local devnet.

Optional: `--scope="category"` to focus on a specific test category.

## When to Use This

Use this command when you need to:
- Verify the blockchain survives adversarial conditions before a release
- Find crash bugs, state corruption, or protocol violations through the public interface
- Test edge cases (zero amounts, overflow values, boundary conditions)
- Hammer RPC endpoints under sustained load
- Test producer lifecycle under stress (rapid bond/unbond cycling)
- Verify crash recovery and node resilience
- Run a full stress test campaign before mainnet deployment

## Scope Options

| Scope | What It Tests |
|-------|--------------|
| `--scope=cli` | Wallet and transaction commands -- sends, balances, edge case amounts |
| `--scope=rpc` | Raw RPC endpoint hammering -- concurrent requests, malformed input, flood |
| `--scope=producer` | Producer lifecycle -- register, bond, withdraw, exit, slash cycling |
| `--scope=consensus` | Block production under stress, node kills, partition simulation |
| `--scope=edge-cases` | Boundary values, overflow attempts, epoch/slot transitions |
| `--scope=all` | Full campaign across all categories (default) |

## Usage Examples

```
/workflow:stress-test "run a full stress test campaign against devnet"
/workflow:stress-test "hammer the RPC endpoints" --scope=rpc
/workflow:stress-test "test producer bond/unbond cycling under stress" --scope=producer
/workflow:stress-test "find edge cases in transaction handling" --scope=edge-cases
/workflow:stress-test "test node crash recovery" --scope=consensus
/workflow:stress-test "run transaction stress tests with 1000 rapid sends" --scope=cli
```

## Prerequisites

Before invoking, ensure:
1. **Binaries are built**: `cargo build --release` (or the binaries exist in `./target/release/`)
2. **Devnet is running** (or approve the agent to start one): `doli-node devnet init --nodes 3 && doli-node devnet start`
3. **Test wallets have funds** (the agent will create/fund wallets if needed)

## What It Produces

1. **Campaign Plan** (presented for approval before execution):
   - Structured test matrix organized by category
   - Destructive tests flagged for explicit approval
   - Estimated execution time per category

2. **Stress Test Report** saved to `docs/stress-tests/stress-report-[YYYY-MM-DD].md`:
   - All test results (PASS/FAIL/CRASH/ERROR/SKIP)
   - Critical findings with exact reproduction commands
   - Post-test chain state integrity verification
   - Cross-category analysis and severity classification
   - Recommendations for hardening

3. **Progress file** at `docs/.workflow/stress-test-progress.md` (incremental, updated after each test)

## Methodology

```
Phase 1: Reconnaissance     -> query devnet state, create test wallets, record baseline
Phase 2: Campaign Design    -> design all tests with hypotheses and expected behavior
Phase 3: Present and Approve -> show campaign plan, get user approval
Phase 4: Execute Campaign    -> run tests category by category, save findings incrementally
Phase 5: Analyze and Report  -> compile findings, classify severity, identify patterns
```

## Safety Controls

### Black-Box Only
The agent NEVER reads or modifies source code. Tests are designed from protocol specifications and public documentation only.

### Devnet Only
All tests run against local devnet (localhost:28545). The agent verifies the target is localhost before every test category. NEVER targets mainnet or testnet.

### Approval Gates
- Campaign plan requires explicit approval before execution begins
- Destructive tests (node kills, disk fill) are individually flagged and require approval
- Long-running tests (sustained load for minutes) are flagged with time estimates

### Incremental Progress
Findings are saved after every single test. If context limits are reached or a crash occurs, all completed work is preserved in the progress file.

## Severity Classification

| Severity | Meaning |
|----------|---------|
| **CRITICAL** | Node crash, state corruption, data loss, consensus violation |
| **HIGH** | Protocol violation without crash, persistent incorrect state |
| **MEDIUM** | Incorrect error handling, poor degradation, resource leak |
| **LOW** | Cosmetic, minor inconsistency, vague error messages |
