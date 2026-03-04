---
name: workflow:stress-test
description: Black-box stress testing of the DOLI blockchain -- pushes every CLI subcommand and RPC endpoint to their breaking point to discover crashes, corrupt states, and protocol violations. Tests against any user-specified network.
---

# Workflow: Stress Test

Invoke ONLY the `stress-tester` subagent for black-box CLI vulnerability testing of the DOLI blockchain.

Optional: `--scope="category"` to focus on a specific test category.

## When to Use This

Use this command when you need to:
- Test every `doli` CLI subcommand for crashes, panics, and incorrect error handling
- Find input validation vulnerabilities (overflow, malformed addresses, injection)
- Hammer RPC endpoints with edge case parameters and concurrent requests
- Verify producer lifecycle commands handle all edge cases correctly
- Test wallet operations (create, restore, export, import) for robustness
- Run a full CLI stress test campaign before a release
- Verify error messages are accurate and helpful across all commands

## Scope Options

| Scope | What It Tests |
|-------|--------------|
| `--scope=wallet` | Wallet operations -- new, restore, address, export, import, info |
| `--scope=transactions` | Send, balance, history -- edge case amounts, invalid addresses, rapid queries |
| `--scope=producer` | Producer lifecycle -- register, bond, withdraw, exit, slash, status, bonds |
| `--scope=rewards` | Rewards commands -- list, claim, claim-all, history, info |
| `--scope=rpc` | Raw RPC endpoint testing -- malformed JSON, concurrent flood, edge parameters |
| `--scope=governance` | Update, maintainer, protocol commands |
| `--scope=signing` | Sign and verify commands with edge case inputs |
| `--scope=edge-cases` | Global flags, unknown subcommands, input injection, boundary values |
| `--scope=all` | Full campaign across all categories (default) |

## Usage Examples

```
/workflow-stress-test "run a full CLI stress test against mainnet"
/workflow-stress-test "test all producer commands for edge cases" --scope=producer
/workflow-stress-test "hammer the RPC endpoints" --scope=rpc
/workflow-stress-test "test wallet create/restore/export/import" --scope=wallet
/workflow-stress-test "find input validation vulnerabilities" --scope=edge-cases
/workflow-stress-test "test transaction edge cases (zero, overflow, invalid address)" --scope=transactions
```

## Prerequisites

Before invoking, ensure:
1. **Binaries are built**: `cargo build --release` (or binaries exist in `./target/release/`)
2. **Target network is reachable**: mainnet default at `http://seed1.doli.network:8545`
3. **Wallet exists** (or the agent will create one). Many tests run without funds (error handling, read-only commands)

## What It Produces

1. **Campaign Plan** (presented for approval before execution):
   - Structured test matrix organized by category
   - State-changing tests flagged for explicit approval
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
Phase 1: Reconnaissance     -> query target network, check wallet, record baseline
Phase 2: Campaign Design    -> design all tests with hypotheses and expected behavior
Phase 3: Present and Approve -> show campaign plan, get user approval
Phase 4: Execute Campaign    -> run tests category by category, save findings incrementally
Phase 5: Analyze and Report  -> compile findings, classify severity, identify patterns
```

## Safety Controls

### Black-Box Only
The agent NEVER reads or modifies source code. Tests are designed from protocol specifications and public documentation only.

### No Node Operations
The agent NEVER kills, stops, restarts, or modifies running nodes. It interacts ONLY through `doli` CLI commands and `curl` RPC calls.

### Approval Gates
- Campaign plan requires explicit approval before execution begins
- State-changing tests (sends, registrations) are flagged with cost/impact estimates

### Incremental Progress
Findings are saved after every single test. If context limits are reached, all completed work is preserved in the progress file.

## Severity Classification

| Severity | Meaning |
|----------|---------|
| **CRITICAL** | CLI crash (panic/segfault), state corruption, consensus violation |
| **HIGH** | Protocol violation without crash, persistent incorrect state |
| **MEDIUM** | Incorrect error handling, misleading error messages, poor degradation |
| **LOW** | Cosmetic, minor inconsistency, vague error messages |
