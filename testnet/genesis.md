# FULL CHAIN RESET PROCEDURE (Local Mac — Testnet)

## Phase 1: Stop & Wipe

1. **Stop ALL doli services** — producers, seeds, swap, explorer:
   ```
   scripts/testnet.sh stop all
   ```
   Verify no doli-node processes remain: `pgrep -f doli-node` must return nothing. If any survive: `pkill -9 -f doli-node`

2. **Wipe ALL data** — the services use `--data-dir <node>/data`, so runtime data lives inside `<node>/data/` subdirectories. Wipe `data/` first (it contains `signed_slots.db` which causes slashing protection blocks if stale), then clean top-level leftovers:
   - **PRIMARY**: `find ~/testnet/<node>/data -mindepth 1 -delete` — this is the critical wipe. Everything the running node writes goes here: `signed_slots.db`, `state_db`, `blocks`, `chain_state.bin`, `node_key`, `producer_gset.bin`, `peers.cache`, `producer.lock`
   - **SECONDARY**: clean `~/testnet/<node>/` top-level stale files from older layouts: `chain_state.bin`, `producers.bin`, `utxo.bin`, `producer_gset.bin`, `peers.cache`, `producer.lock`, `chainspec.json`, `node_key`, `maintainer_state.bin`, `blocks/`, `signed_slots.db/`, `utxo_rocks/`, `state_db/`
   - **KEEP ONLY**: `~/testnet/keys/` (shared key dir), `~/testnet/bin/`, `~/testnet/explorer/`, `~/testnet/doli-swap-bot/`
   - **Apply to**: `~/testnet/seed/`, `~/testnet/n1/` through `~/testnet/n5/`
   - **Also wipe**: `~/testnet/seed/blocks/` (archive data)

3. **Verify**: `ls ~/testnet/<node>/data/` must be empty for seed and n1-n5. If `signed_slots.db` survives, nodes will hit slashing protection and refuse to produce.

## Phase 2: Update Genesis

4. **Calculate new genesis timestamp** = NOW + 10 minutes (Unix epoch)

5. **Update ALL 3 sources** — miss one and the embedded chainspec will be stale:
   - `chainspec.testnet.json` → timestamp + message
   - `crates/core/src/consensus/constants.rs` → `GENESIS_TIME` constant
   - `crates/core/src/network_params/defaults.rs` → testnet `genesis_time` field

6. **Grep for the OLD timestamp** across the entire repo — 0 matches must remain

7. **Commit** (no sensitive key info): `--author "Ivan D. Lozada <ivan@doli.network>"`

## Phase 3: Compile

8. `cargo build --release -p doli-node -p doli-cli`

9. **Record hash**: `md5 target/release/doli-node target/release/doli`

10. **No deploy step** — launchd plists already point to `target/release/doli-node`. Single machine, single binary.

## Phase 4: Start & Verify

11. **DO NOT start nodes until Phase 1-3 are fully verified** — binary built clean, all data dirs empty, genesis timestamp updated in all 3 sources

12. **Start seed first**, wait 5s, then producers N1-N5:
    ```
    scripts/testnet.sh start seed
    sleep 5
    scripts/testnet.sh start n1 n2 n3 n4 n5
    ```

13. **Immediately check** `getChainInfo` via JSON-RPC POST to `http://127.0.0.1:8500/` (seed) — `bestHeight` and `bestSlot` MUST be 0

14. **Wait for genesis**, then check again — `bestHeight` and `bestSlot` should start incrementing from 1. If any node shows `bestHeight=0` after genesis passed, check logs for `SLASHING PROTECTION` — means `signed_slots.db` wasn't wiped:
    ```
    scripts/testnet.sh status
    scripts/testnet.sh logs seed
    scripts/testnet.sh logs n1
    ```

15. Once chain is producing, optionally start explorer:
    ```
    scripts/testnet.sh start explorer
    ```

16. **Final verify**: `scripts/testnet.sh status` — all nodes synced, same height/hash. Explorer at `http://localhost:8080/network.html` shows green.

## Testnet Parameters

- **Bond unit**: 0.01 DOLI (1,000,000 base units)
- **Unbonding period**: 720 blocks (2 epochs)
- **Network**: `--network testnet`
- **Genesis nodes**: seed + N1-N5 only
- **All ports on 127.0.0.1**: seed=8500, N{i}=8500+i

## Key Notes

- No SSH, no systemd, no cross-server coordination
- Single binary (`target/release/doli-node`), no md5 comparison across machines
- `scripts/testnet.sh` replaces `systemctl` for all start/stop/status
- Only seed + N1-N5 at genesis. Additional producers (N6-N12) can join later via registration
- Explorer at `http://localhost:8080/`, network status at `http://localhost:8080/network.html`
