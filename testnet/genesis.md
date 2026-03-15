# FULL CHAIN RESET PROCEDURE (Local Mac)

## Phase 1: Stop & Wipe

1. **Stop ALL doli services** — producers, seeds, swap, explorer:
   ```
   scripts/mainnet.sh stop all
   ```
   Verify no doli-node processes remain: `pgrep -f doli-node` must return nothing. If any survive: `pkill -9 -f doli-node`

2. **Wipe ALL data** — the services use `--data-dir <node>/data`, so runtime data lives inside `<node>/data/` subdirectories. Wipe `data/` first (it contains `signed_slots.db` which causes slashing protection blocks if stale), then clean top-level leftovers:
   - **PRIMARY**: `find ~/mainnet/<node>/data -mindepth 1 -delete` — this is the critical wipe. Everything the running node writes goes here: `signed_slots.db`, `state_db`, `blocks`, `chain_state.bin`, `node_key`, `producer_gset.bin`, `peers.cache`, `producer.lock`
   - **SECONDARY**: clean `~/mainnet/<node>/` top-level stale files from older layouts: `chain_state.bin`, `producers.bin`, `utxo.bin`, `producer_gset.bin`, `peers.cache`, `producer.lock`, `chainspec.json`, `node_key`, `maintainer_state.bin`, `blocks/`, `signed_slots.db/`, `utxo_rocks/`, `state_db/`
   - **KEEP ONLY**: `~/mainnet/keys/` (shared key dir), `~/mainnet/bin/`, `~/mainnet/explorer/`, `~/mainnet/doli-swap-bot/`
   - **Apply to**: ALL nodes — `~/mainnet/seed/`, `~/mainnet/n1/` through `~/mainnet/n12/`
   - **Also wipe**: `~/mainnet/seed/blocks/` (archive data)

3. **Verify**: `ls ~/mainnet/<node>/data/` must be empty for every node. `ls ~/mainnet/<node>/` must show only `data/` (empty dir) and optionally `keys/`. If `signed_slots.db` survives, nodes will hit slashing protection and refuse to produce.

## Phase 2: Update Genesis

4. **Calculate new genesis timestamp** = NOW + 10 minutes (Unix epoch)

5. **Update ALL 4 sources** — miss one and the embedded chainspec will be stale:
   - `chainspec.mainnet.json` → timestamp + message
   - `chainspec.testnet.json` → timestamp + message
   - `crates/core/src/consensus.rs` → `GENESIS_TIME` constant
   - `crates/core/src/network_params.rs` → mainnet `genesis_time` field

6. **Grep for the OLD timestamp** across the entire repo — 0 matches must remain

7. **Commit** (no sensitive key info): `--author "Ivan D. Lozada <ivan@doli.network>"`

## Phase 3: Compile

8. `cargo build --release -p doli-node -p doli-cli`

9. **Record hash**: `md5 target/release/doli-node target/release/doli`

10. **No deploy step** — launchd plists already point to `target/release/doli-node`. Single machine, single binary.

## Phase 4: Start & Verify

11. **DO NOT start nodes until Phase 1-3 are fully verified** — binary built clean, all data dirs empty, genesis timestamp updated in all 4 sources

12. **Start seed first**, wait 5s, then producers:
    ```
    scripts/mainnet.sh start seed
    sleep 5
    scripts/mainnet.sh start n1 n2 n3 n4 n5
    ```

13. **Immediately check** `getChainInfo` via JSON-RPC POST to `http://127.0.0.1:8500/` (seed) — `bestHeight` and `bestSlot` MUST be 0

14. **Wait for genesis**, then check again — `bestHeight` and `bestSlot` should start incrementing from 1. If any node shows `bestHeight=0` after genesis passed, check logs for `SLASHING PROTECTION` — means `signed_slots.db` wasn't wiped:
    ```
    scripts/mainnet.sh status
    scripts/mainnet.sh logs seed
    scripts/mainnet.sh logs n1
    ```

15. Once chain is producing, start remaining producers + services:
    ```
    scripts/mainnet.sh start n6 n7 n8 n9 n10 n11 n12
    scripts/mainnet.sh start swap explorer
    ```

16. **Final verify**: `scripts/mainnet.sh status` — all nodes synced, same height/hash. Explorer at `http://localhost:8080/network.html` shows green.

## Key Notes

- No SSH, no systemd, no cross-server coordination
- Single binary (`target/release/doli-node`), no md5 comparison across machines
- `scripts/mainnet.sh` replaces `systemctl` for all start/stop/status
- All ports on `127.0.0.1` (seed=8500, N{i}=8500+i)
- Explorer at `http://localhost:8080/`, network status at `http://localhost:8080/network.html`
