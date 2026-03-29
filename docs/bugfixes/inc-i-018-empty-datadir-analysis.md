# INC-I-018: Node Cannot Bootstrap From Empty Data Dir — Analysis

## Bug Report
- **Severity**: High
- **Symptoms**: Node exits silently (no output, no logs, no crash report) when started with an empty data dir
- **Impact**: Blocks --no-snap-sync operational testing. Blocks any recovery that requires data wipe.
- **Branch**: integrate/sync-fixes

## Fundamentals Check
- **Build**: PASS — `cargo build --release` succeeds
- **Reproduction**: CANNOT REPRODUCE

## Reproduction Attempts

All tested on `integrate/sync-fixes` branch, binary v4.9.8:

| # | Scenario | Data Dir | Result |
|---|----------|----------|--------|
| 1 | `--network devnet run --no-dht` | Empty | Starts, enters event loop, logs normally |
| 2 | `--network testnet run --no-dht --no-snap-sync` | Empty | Starts, enters event loop, logs normally |
| 3 | `--network testnet run --producer` (no key file) | Empty | Exits with clear error: "requires a wallet" |
| 4 | Exact seed plist: `--network testnet run --relay-server --no-snap-sync --yes` | Empty | Starts normally |
| 5 | Exact producer plist: `--network testnet run --producer --producer-key ~/testnet/keys/producer_1.json --force-start --yes` | Empty | Starts normally, VDF calibration, enters loop |

In ALL cases, the node produces output to stderr (tracing subscriber) immediately upon startup.

## Code Path Analysis

The `Node::new()` path (init.rs) handles empty data dirs correctly:
1. `create_dir_all(&data_dir)` creates the directory
2. `BlockStore::open()` / `StateDb::open()` create new RocksDB instances
3. Migration check: no old files → skip
4. `state_db.get_chain_state()` → None → creates fresh `ChainState::new(genesis_hash)`
5. ProducerSet: uses hardcoded genesis producers (testnet) or empty set (devnet)
6. All subsequent initialization succeeds

No early exits, no silent `process::exit()`, no unhandled panics in the init path.

## Possible Explanations for "Silent Exit"

1. **launchd log truncation**: If the launchd service was stopped/restarted rapidly, `StandardOutPath` might be truncated between restarts. Check `~/testnet/logs/*.log` file size.

2. **Binary path mismatch**: The plist uses `${NODE_BIN}` which expands to the absolute build path at install time. If the binary was rebuilt to a different path or the plist wasn't reinstalled after a build, launchd would silently fail.

3. **Port conflict**: If another node is already listening on the same P2P/RPC port, the startup could fail. However, this would still produce log output.

4. **Different binary version**: If `~/testnet/bin/doli-node` is used instead of the build path, and it's an older binary with a different init path.

5. **launchd ThrottleInterval**: macOS throttles services that exit too quickly (within 10 seconds). After repeated rapid exits, launchd may refuse to start the service at all — this would look like "no output, no crash".

## Triage Verdict

```
TRIAGE VERDICT
Path: NEEDS USER INPUT
Confidence: conf(0.3, unable to reproduce)
Reasoning: The code handles empty data dirs correctly in all tested configurations.
           The "silent exit" symptom cannot be reproduced via direct execution.
           Need exact reproduction steps from the operator.
```

## Questions for Reporter

1. **Exact command**: What is the exact command or launchd service being run?
2. **Where are you checking logs?**: `~/testnet/logs/*.log`? Terminal stderr? journalctl?
3. **Was the binary rebuilt after the data wipe?**: Did you reinstall the launchd plists?
4. **What was wiped?**: Just `~/testnet/n1/data/`? Or the entire `~/testnet/n1/` directory?
5. **Is there a `.env` override?**: Check `~/.doli/testnet/.env` for any DOLI_* vars that might affect startup
