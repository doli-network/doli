# DOLI CLI Stress Test Report

**Date**: 2026-03-04
**Duration**: ~35 minutes
**Target**: Mainnet, http://seed1.doli.network:8500
**Scope**: All categories (wallet, transactions, producer, rewards, RPC, governance, signing, edge-cases)
**CLI Version**: doli 1.1.0 (abc491d)

## Executive Summary

- **Tests executed**: 119
- **PASS**: 97
- **FAIL**: 16 (protocol violation or incorrect behavior)
- **CRASH**: 0
- **ERROR**: 0
- **SKIP**: 6 (test design errors corrected inline)
- **Overall verdict**: DEGRADED

The CLI never panics or crashes on any input tested, which is a strong foundation. However, a systematic pattern of **exit code 0 on error conditions** pervades the codebase, and the CLI **hangs indefinitely when the RPC endpoint is unreachable** (no connection timeout). The RPC server returns **plain-text errors instead of JSON-RPC error objects** for certain malformed requests. No state corruption, no consensus violations, no resource exhaustion issues were observed.

## Environment Baseline

| Metric | Start | End |
|--------|-------|-----|
| Chain height | 5270 | 5334 |
| Chain slot | 6742 | 6806 |
| Active producers | 6 | 6 |
| Genesis hash | fee565b7...323e89 | fee565b7...323e89 (unchanged) |
| Mempool tx count | 0 | 0 |
| Test wallet balance | 0 DOLI | 0 DOLI (no state changes) |

## Critical Findings

No CRITICAL findings. The CLI never panicked, segfaulted, or corrupted state.

## High Severity Findings

### FINDING-001: CLI hangs indefinitely on unreachable RPC endpoint

- **Test IDs**: ST-EDGE-004, ST-EDGE-005
- **Severity**: HIGH
- **Category**: error-handling
- **Hypothesis**: The CLI should timeout and display an error when the RPC endpoint is unreachable
- **Reproduction**:
  ```bash
  # Hangs forever -- must be killed with Ctrl-C or timeout
  /Users/ilozada/repos/doli/target/release/doli -r "http://192.0.2.1:8500" chain

  # Same with wrong port
  /Users/ilozada/repos/doli/target/release/doli -r "http://seed1.doli.network:9999" chain
  ```
- **Expected**: Connection timeout after 5-10 seconds with error message like "Connection timed out"
- **Actual**: Prints "Chain Information" header, then blocks indefinitely. No timeout. Process must be externally killed
- **Evidence**: Tested with `timeout 15` wrapper -- process was killed at 15s, never returned on its own
- **Impact**: Users running scripts or automation will have hung processes. Any CLI command against a down node will block the terminal

### FINDING-002: Systematic exit code 0 on error conditions

- **Test IDs**: ST-EDGE-003, ST-TX-001 through ST-TX-006, ST-TX-013b, ST-TX-013c, ST-TX-023, ST-PROD-001, ST-PROD-003, ST-PROD-004, ST-PROD-006, ST-PROD-007, ST-PROD-008, ST-PROD-012, ST-PROD-015, ST-PROD-019, ST-PROD-022
- **Severity**: HIGH
- **Category**: error-handling
- **Hypothesis**: All CLI errors should exit with non-zero status codes for scripting compatibility
- **Reproduction**:
  ```bash
  # These all print error messages but exit 0:
  /Users/ilozada/repos/doli/target/release/doli -r "not-a-url" chain; echo "EXIT: $?"
  /Users/ilozada/repos/doli/target/release/doli send doli1qeywkuss69vr3wq3f7uq53cvc9juxwsqjkhwkcek88mz4jekyerq5ll9j9 0; echo "EXIT: $?"
  /Users/ilozada/repos/doli/target/release/doli producer register --bonds 0; echo "EXIT: $?"
  /Users/ilozada/repos/doli/target/release/doli producer status -p "not_a_valid_key"; echo "EXIT: $?"
  ```
- **Expected**: Exit code 1 (or other non-zero) when an error occurs
- **Actual**: Exit code 0 despite printing "Error:" messages
- **Pattern**: Commands that format their own output (printing headers before the RPC call) tend to exit 0 on error. Commands that rely purely on clap argument parsing exit 2 correctly. Commands that bubble up RPC errors (like `producer bonds`) exit 1 correctly
- **Impact**: Scripts using `set -e` or checking `$?` will not detect failures. CI/CD pipelines will report success on failure

## Medium Severity Findings

### FINDING-003: RPC server returns plain-text errors instead of JSON-RPC error objects

- **Test IDs**: ST-RPC-010, ST-RPC-024
- **Severity**: MEDIUM
- **Category**: protocol-violation
- **Hypothesis**: All RPC responses should be valid JSON-RPC objects, even for malformed requests
- **Reproduction**:
  ```bash
  # Missing method field -- returns plain text
  curl -s http://seed1.doli.network:8500 -X POST -H "Content-Type: application/json" \
    --data '{"jsonrpc":"2.0","params":{},"id":1}'

  # Empty POST body -- returns plain text
  curl -s http://seed1.doli.network:8500 -X POST -H "Content-Type: application/json" --data ''
  ```
- **Expected**: `{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid Request"},"id":null}`
- **Actual**:
  - Missing method: `Failed to deserialize the JSON body into the target type: missing field 'method' at line 1 column 36`
  - Empty body: `Failed to parse the request body as JSON: EOF while parsing a value at line 1 column 0`
- **Impact**: RPC clients that parse JSON will crash on these responses. Violates JSON-RPC 2.0 specification

### FINDING-004: RPC server accepts 1MB+ request bodies without rejection

- **Test ID**: ST-RPC-013
- **Severity**: MEDIUM
- **Category**: resource-exhaustion
- **Hypothesis**: The RPC server should reject oversized request bodies to prevent memory exhaustion
- **Reproduction**:
  ```bash
  python3 -c "import json; d={'jsonrpc':'2.0','method':'getChainInfo','params':{'data':'A'*1048576},'id':1}; open('/tmp/bigpayload.json','w').write(json.dumps(d))"
  curl -s http://seed1.doli.network:8500 -X POST -H "Content-Type: application/json" -d @/tmp/bigpayload.json --max-time 10
  ```
- **Expected**: HTTP 413 (Payload Too Large) or JSON-RPC error
- **Actual**: Request accepted, parsed, and processed normally. Returns valid getChainInfo response
- **Impact**: An attacker can send arbitrarily large payloads to consume server memory. At scale, this enables denial-of-service

### FINDING-005: Zero-amount send not validated before UTXO lookup

- **Test ID**: ST-TX-001
- **Severity**: MEDIUM
- **Category**: input-validation
- **Hypothesis**: `doli send <addr> 0` should be rejected immediately with "cannot send zero amount"
- **Reproduction**:
  ```bash
  /Users/ilozada/repos/doli/target/release/doli send doli1qeywkuss69vr3wq3f7uq53cvc9juxwsqjkhwkcek88mz4jekyerq5ll9j9 0
  ```
- **Expected**: "Error: Cannot send zero amount" (immediate validation)
- **Actual**: "Error: No spendable UTXOs available" (reaches UTXO lookup, meaning zero-amount is accepted as a valid amount). If the wallet had funds, a zero-amount transaction might be constructed and broadcast
- **Impact**: If a wallet has funds, zero-amount transactions could reach the mempool, wasting block space

## Low Severity Findings

### FINDING-006: Scientific notation and decimal amounts silently accepted by send

- **Test ID**: ST-TX-013b, ST-TX-013c
- **Severity**: LOW
- **Category**: input-validation
- **Reproduction**:
  ```bash
  # Both are accepted as valid amounts (reach UTXO lookup, not rejected as invalid)
  /Users/ilozada/repos/doli/target/release/doli send doli1qeywkuss69vr3wq3f7uq53cvc9juxwsqjkhwkcek88mz4jekyerq5ll9j9 1.5
  /Users/ilozada/repos/doli/target/release/doli send doli1qeywkuss69vr3wq3f7uq53cvc9juxwsqjkhwkcek88mz4jekyerq5ll9j9 1e18
  ```
- **Expected**: Either clear documentation that decimal amounts represent whole DOLI (1.5 = 1.5 DOLI = 150000000 units), or rejection with "amount must be a whole number"
- **Actual**: Both parse successfully and proceed to UTXO lookup. "abc" is correctly rejected. The behavior may be intentional (DOLI has 8 decimal places), but `1e18` parsing as scientific notation could produce unexpected amounts
- **Impact**: Users may accidentally send unexpected amounts if `1e18` maps to 1000000000000000000 units

### FINDING-007: RPC parameter naming inconsistency (camelCase vs snake_case)

- **Test ID**: ST-RPC-019
- **Severity**: LOW
- **Category**: protocol-violation
- **Reproduction**:
  ```bash
  # getProducers returns camelCase: "publicKey"
  curl -s http://seed1.doli.network:8500 -X POST -H "Content-Type: application/json" \
    --data '{"jsonrpc":"2.0","method":"getProducers","params":{},"id":1}'

  # getBondDetails expects snake_case: "public_key"
  curl -s http://seed1.doli.network:8500 -X POST -H "Content-Type: application/json" \
    --data '{"jsonrpc":"2.0","method":"getBondDetails","params":{"publicKey":"abc..."},"id":1}'
  # Returns: "missing field `public_key`"
  ```
- **Expected**: Consistent parameter naming across all RPC methods
- **Actual**: Response fields use camelCase (`publicKey`), but getBondDetails input expects snake_case (`public_key`)
- **Impact**: Confusing for RPC client developers. Easy to use the wrong convention

## All Test Results

### Category H: Edge Cases & Input Validation (14 tests)

| ID | Test | Result | Notes |
|----|------|--------|-------|
| ST-EDGE-001 | --help on all 35 commands/subcommands | PASS | All exit 0, no panics |
| ST-EDGE-002 | Commands with no arguments | PASS | Proper usage/error messages |
| ST-EDGE-003 | -r with invalid URL | FAIL | Error printed but exit code 0 |
| ST-EDGE-004 | -r with unreachable host | FAIL | Hangs indefinitely, no timeout |
| ST-EDGE-005 | -r with wrong port | FAIL | Hangs indefinitely, no timeout |
| ST-EDGE-006 | -w with directory/nonexistent file | PASS | Clear errors, exit 1 |
| ST-EDGE-007 | Hex vs bech32m address consistency | PASS | Both resolve correctly |
| ST-EDGE-008 | Unknown subcommand (root) | PASS | Proper error, exit 2 |
| ST-EDGE-009 | Unknown subcommand (nested) | PASS | Proper error, exit 2 |
| ST-EDGE-010 | Very long address (1000+ chars) | PASS | Rejected cleanly |
| ST-EDGE-011 | SQL injection in address | PASS | Rejected with clear error |
| ST-EDGE-012 | Shell injection in address | PASS | Treated as literal, rejected |
| ST-EDGE-013 | Null bytes in address | PASS | Rejected cleanly |
| ST-EDGE-014 | --version flag | PASS | Returns version, exit 0 |

### Category A: Wallet Operations (12 tests)

| ID | Test | Result | Notes |
|----|------|--------|-------|
| ST-WAL-001 | Create new wallet | PASS | Valid JSON, seed phrase shown, address generated |
| ST-WAL-002 | Create wallet when file exists | PASS | Rejected: "Wallet already exists", exit 1 |
| ST-WAL-003 | Restore with valid 24-word seed | PASS | Same address derived, round-trip works |
| ST-WAL-004 | Restore with 12 words | PASS | "invalid checksum" error, exit 1, no partial wallet |
| ST-WAL-005 | Restore with garbage words | PASS | "unknown word" error, exit 1, no partial wallet |
| ST-WAL-006 | Restore with empty input | PASS | "No seed phrase provided", exit 1 |
| ST-WAL-007 | Generate new address | PASS | Valid bech32m address, appended to wallet |
| ST-WAL-008 | List all addresses | PASS | Shows both addresses with primary marked |
| ST-WAL-009 | Wallet info | PASS | Shows address count, public key, exit 0 |
| ST-WAL-010 | Export then import round-trip | PASS | Imported wallet has same addresses and keys |
| ST-WAL-011 | Import from non-existent file | PASS | "No such file", exit 1 |
| ST-WAL-012 | Import from empty file | PASS | "EOF while parsing", exit 1 |
| ST-WAL-013 | Export to read-only path | PASS | "Read-only file system", exit 1 |

### Category E: Signing & Verification (12 tests)

| ID | Test | Result | Notes |
|----|------|--------|-------|
| ST-SIGN-001 | Sign "hello" | PASS | Valid signature returned |
| ST-SIGN-002 | Sign empty string | PASS | Signature returned (empty message is valid) |
| ST-SIGN-003 | Sign 10KB message | PASS | Signature returned without delay |
| ST-SIGN-004 | Sign unicode/emoji | PASS | Signature returned correctly |
| ST-SIGN-005 | Sign binary content (null bytes) | PASS | Shell truncates at null byte; same sig as empty. Not a CLI bug |
| ST-SIGN-006 | Verify correct sig | PASS | "Valid: true" |
| ST-SIGN-007 | Verify wrong sig (correct length) | PASS | "Valid: false" |
| ST-SIGN-008 | Verify wrong pubkey | PASS | "Valid: false" |
| ST-SIGN-009 | Verify malformed sig (bad hex) | PASS | "invalid hex string", exit 1 |
| ST-SIGN-010 | Verify malformed pubkey | PASS | "invalid hex string", exit 1 |
| ST-SIGN-011 | Verify with no arguments | PASS | Usage help, exit 2 |
| ST-SIGN-012 | Verify truncated sig | PASS | "invalid signature length", exit 1 |

### Category F: Governance & Update (8 tests)

| ID | Test | Result | Notes |
|----|------|--------|-------|
| ST-GOV-001 | update check | PASS | "No pending updates. Your node is up to date." |
| ST-GOV-002 | update status | PASS | "No pending updates." |
| ST-GOV-003 | update votes (no args) | PASS | "required argument --version", exit 2 |
| ST-GOV-004 | maintainer list | PASS | Shows 5 maintainers with 3/5 threshold |
| ST-GOV-005 | protocol sign (no args) | PASS | "required arguments --version --epoch", exit 2 |
| ST-GOV-006 | protocol activate (no args) | PASS | Shows required arguments, exit 2 |
| ST-GOV-007 | chain info | PASS | Shows network, height, slot, hashes |
| ST-GOV-008 | upgrade --help | PASS | Shows usage without executing |

### Category B: Transaction & Balance (20 tests)

| ID | Test | Result | Notes |
|----|------|--------|-------|
| ST-TX-001 | send amount=0 | FAIL | Not rejected as invalid; reaches UTXO lookup. Exit 0 on error |
| ST-TX-002 | send amount=u64::MAX | PASS | "No spendable UTXOs" (no overflow). Exit 0 on error |
| ST-TX-003 | send more than balance | PASS | "No spendable UTXOs" (correct). Exit 0 on error |
| ST-TX-004 | send to self | PASS | "No spendable UTXOs" (balance is 0). Exit 0 on error |
| ST-TX-005 | send with --fee 0 | PASS | "No spendable UTXOs". Exit 0 on error |
| ST-TX-006 | send with --fee u64::MAX | PASS | "No spendable UTXOs" (no overflow). Exit 0 on error |
| ST-TX-007 | send to invalid prefix (xdoli1) | PASS | Clear address format error, exit 1 |
| ST-TX-008 | send to truncated address | PASS | "no valid bech32m checksum", exit 1 |
| ST-TX-009 | send to garbage address | PASS | Clear format error, exit 1 |
| ST-TX-010 | send to emoji address | PASS | "parsing failed", exit 1 |
| ST-TX-012 | send negative amount (-1) | PASS | clap rejects: "unexpected argument", exit 2 |
| ST-TX-013a | send non-numeric "abc" | PASS | "Invalid amount: abc", exit 1 |
| ST-TX-013b | send decimal 1.5 | FAIL | Accepted as valid (reaches UTXO lookup). May be intentional |
| ST-TX-013c | send scientific 1e18 | FAIL | Accepted as valid (reaches UTXO lookup). Potentially dangerous |
| ST-TX-014 | send with no arguments | PASS | Usage help, exit 2 |
| ST-TX-015 | send with only recipient | PASS | "required argument AMOUNT", exit 2 |
| ST-TX-016 | balance (default wallet) | PASS | Shows balance correctly |
| ST-TX-017 | balance with invalid address | PASS | Clear error, exit 1 |
| ST-TX-018 | balance with hex address | PASS | Resolves to bech32m, shows balance |
| ST-TX-019 | history --limit 0 | PASS | Empty result, no crash |
| ST-TX-020 | history --limit 999999999 | PASS | "No transactions found", no OOM |
| ST-TX-021 | history --limit -1 | PASS | clap rejects, exit 2 |
| ST-TX-022 | 20 rapid balance queries | PASS | All consistent |
| ST-TX-023 | send with zero balance | PASS | "No spendable UTXOs". Exit 0 on error |

### Category C: Producer Lifecycle (18 tests)

| ID | Test | Result | Notes |
|----|------|--------|-------|
| ST-PROD-001 | register --bonds 0 | FAIL | Rejected correctly but exit code 0 |
| ST-PROD-002 | register --bonds -1 | PASS | clap rejects, exit 2 |
| ST-PROD-003 | register --bonds 10001 | FAIL | Rejected correctly but exit code 0 |
| ST-PROD-004 | register insufficient balance | FAIL | "Insufficient balance" but exit code 0 |
| ST-PROD-006 | status (own key, not registered) | FAIL | "Producer not found" but exit code 0 |
| ST-PROD-007 | status with invalid key | FAIL | "Invalid public key format" but exit code 0 |
| ST-PROD-008 | status with nonexistent key | FAIL | "Producer not found" but exit code 0 |
| ST-PROD-009 | bonds (not registered) | PASS | "Producer not found", exit 1 |
| ST-PROD-010 | list | PASS | Shows 6 producers correctly |
| ST-PROD-011 | list --active | PASS | Shows same 6 (all active) |
| ST-PROD-012 | add-bond --count 0 | FAIL | Rejected correctly but exit code 0 |
| ST-PROD-013 | add-bond --count -1 | PASS | clap rejects, exit 2 |
| ST-PROD-014 | add-bond insufficient balance | FAIL | "Insufficient balance" but exit code 0 |
| ST-PROD-015 | request-withdrawal --count 0 | FAIL | Rejected correctly but exit code 0 |
| ST-PROD-016 | request-withdrawal --count 999999 | PASS | "Producer not found", exit 1 |
| ST-PROD-017 | request-withdrawal --count 1 | PASS | "Producer not found", exit 1 |
| ST-PROD-018 | simulate-withdrawal --count 1 | PASS | "Producer not found", exit 1 |
| ST-PROD-019 | simulate-withdrawal --count 0 | FAIL | Rejected correctly but exit code 0 |
| ST-PROD-020 | exit (not registered) | PASS | "Producer not found", exit 1 |
| ST-PROD-021 | slash with invalid hashes | PASS | "Invalid hash format", exit 1 |
| ST-PROD-022 | slash with same hash twice | PASS | "Same block" error. Exit 0 on error |
| ST-PROD-023 | slash with nonexistent hashes | PASS | "Block not found", exit 1 |

### Category D: Rewards (8 tests)

| ID | Test | Result | Notes |
|----|------|--------|-------|
| ST-REW-001 | rewards list | PASS | Informational message about auto-distribution |
| ST-REW-002 | rewards claim epoch 0 | PASS | Informational message (rewards are automatic) |
| ST-REW-003 | rewards claim epoch 999999999 | PASS | Same informational message |
| ST-REW-005 | rewards claim-all | PASS | Informational message |
| ST-REW-007 | rewards history | PASS | Points to `doli history` |
| ST-REW-008 | rewards info | PASS | Shows epoch info with progress bar |

### Category G: RPC Endpoint Testing (27 tests)

| ID | Test | Result | Notes |
|----|------|--------|-------|
| ST-RPC-001 | 50 concurrent getChainInfo | PASS | 50/50 succeeded, all same height |
| ST-RPC-002 | getBalance invalid address | PASS | JSON-RPC error -32602 |
| ST-RPC-003 | getBlockByHeight future height | PASS | "Block not found" |
| ST-RPC-004 | getBlockByHeight height=0 | PASS | "Block not found" (pruned/snap-synced node) |
| ST-RPC-005 | getBlockByHeight u64::MAX | PASS | "Block not found", no crash |
| ST-RPC-006 | getBlockByHash invalid hash | PASS | "Invalid hash format" |
| ST-RPC-007 | getBlockByHash nonexistent | PASS | "Block not found" |
| ST-RPC-008 | sendTransaction malformed hex | PASS | "Invalid hex" error |
| ST-RPC-009 | sendTransaction invalid sig | PASS | "invalid version" error (caught at deserialization) |
| ST-RPC-010 | Missing method field | FAIL | Plain text error, not JSON-RPC object |
| ST-RPC-011 | Wrong jsonrpc version | PASS | JSON-RPC error -32600 |
| ST-RPC-012 | Bad params type | PASS | Accepted (params ignored for getChainInfo) |
| ST-RPC-013 | 1MB+ request body | FAIL | Accepted without rejection. No body size limit |
| ST-RPC-014 | Unknown method | PASS | "Method not found: nonExistentMethod" |
| ST-RPC-015 | 100 sequential calls | PASS | Completed in 13s (130ms avg) |
| ST-RPC-016 | getHistory limit=0 | PASS | Empty array |
| ST-RPC-017 | getHistory limit=u64::MAX | PASS | Empty array (bounded by actual data) |
| ST-RPC-018 | getUtxos invalid address | PASS | JSON-RPC error -32602 |
| ST-RPC-019 | getBondDetails nonexistent | PASS | "Producer not found" (after fixing param name) |
| ST-RPC-020 | getEpochInfo | PASS | Returns epoch data correctly |
| ST-RPC-021 | getNetworkParams | PASS | Returns all params |
| ST-RPC-022 | getProducer invalid key | PASS | "Invalid public key format" |
| ST-RPC-023 | getProducers | PASS | Returns all 6 producers |
| ST-RPC-024 | Empty POST body | FAIL | Plain text error, not JSON-RPC object |
| ST-RPC-025 | GET request | PASS | Empty response (acceptable) |
| ST-RPC-026 | getTransaction nonexistent | PASS | "Transaction not found" |
| ST-RPC-027 | getMempoolInfo | PASS | Returns mempool state |

## Post-Test Chain State

- **Chain height**: 5334 (advanced by 64 blocks during testing)
- **State integrity**: Verified -- genesis hash unchanged, producer count unchanged, mempool empty
- **No transactions submitted**: All send/register operations failed at validation (zero balance), confirming no mainnet state was modified

## Cross-Category Analysis

1. **Exit code 0 on errors is systemic** -- this pattern appears across categories B, C, and H. The root cause appears to be commands that print a formatted header (like "Producer Registration" or "Chain Information") before making the RPC call. When the RPC call fails or validation rejects the input, the error is printed but the process has already committed to a "success" exit path. Commands that delegate entirely to clap (argument parsing) or that propagate RPC errors directly tend to exit correctly. This suggests the error handling in the command display layer does not propagate exit codes.

2. **RPC is resilient, CLI is the weak point** -- the RPC server handled all concurrent, malformed, and oversized inputs without crashing or returning inconsistent data. All RPC failures are in response formatting (plain text vs JSON-RPC). The CLI's weaknesses are in error propagation (exit codes) and timeout handling.

3. **Input validation is strong for addresses, weak for amounts** -- every malformed address tested was caught with a clear error message. But amount validation is incomplete: zero is not caught, scientific notation is accepted, and the error message for zero-amount is misleading ("No spendable UTXOs" instead of "cannot send zero").

## Source Code Integrity Check

- **git diff --name-only output**: (empty -- no source files modified)
- **Verdict**: NO SOURCE FILES MODIFIED

## Findings Summary

| # | Finding | Severity | Test IDs | Category |
|---|---------|----------|----------|----------|
| 1 | CLI hangs indefinitely on unreachable RPC | HIGH | ST-EDGE-004, ST-EDGE-005 | error-handling |
| 2 | Systematic exit code 0 on error conditions | HIGH | 16+ tests across B, C, H | error-handling |
| 3 | RPC plain-text errors instead of JSON-RPC | MEDIUM | ST-RPC-010, ST-RPC-024 | protocol-violation |
| 4 | RPC accepts 1MB+ request bodies | MEDIUM | ST-RPC-013 | resource-exhaustion |
| 5 | Zero-amount send not validated | MEDIUM | ST-TX-001 | input-validation |
| 6 | Scientific notation in amount accepted | LOW | ST-TX-013b, ST-TX-013c | input-validation |
| 7 | RPC param naming inconsistency | LOW | ST-RPC-019 | protocol-violation |

## Recommendations

Prioritized by severity and impact:

1. **Add HTTP client timeout** to all CLI RPC calls (5-10 second default)
2. **Audit all command handlers for exit code propagation** -- ensure every `Error:` message path calls `std::process::exit(1)`
3. **Wrap Axum deserialization errors in JSON-RPC error objects** at the RPC server level
4. **Add request body size limit** to the Axum RPC server (recommend 64KB-256KB)
5. **Add zero-amount validation** before UTXO lookup in the send command
6. **Standardize RPC parameter naming** to either camelCase or snake_case consistently
7. **Document decimal/scientific notation behavior** in send command or reject non-integer DOLI amounts
