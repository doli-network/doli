# FULL CHAIN RESET PROCEDURE (Linux — Testnet)

**Code repo**: `~/repos/doli` (branch `DPFAESI`) — all builds and genesis changes happen here.
**Operational repo**: `~/repos/localdoli` — runtime data, keys, scripts, explorer only. No source code.

## Phase 0: Branch Verification

0. **Confirm doli repo is on DPFAESI**:
   ```
   cd ~/repos/doli && git checkout DPFAESI && git pull origin DPFAESI
   ```

## Phase 1: Stop & Wipe

1. **Stop ALL doli services** — producers, seeds, swap, explorer:
   ```
   ~/repos/localdoli/testnetlinux/scripts/testnet.sh stop all
   ```
   Verify no doli-node processes remain: `pgrep -f doli-node` must return nothing. If any survive: `pkill -9 -f doli-node`

2. **Wipe ALL data** — the services use `--data-dir <node>/data`, so runtime data lives inside `<node>/data/` subdirectories. Wipe `data/` first (it contains `signed_slots.db` which causes slashing protection blocks if stale), then clean top-level leftovers:
   - **PRIMARY**: `find ~/repos/localdoli/testnetlinux/<node>/data -mindepth 1 -delete` — this is the critical wipe. Everything the running node writes goes here: `signed_slots.db`, `state_db`, `blocks`, `chain_state.bin`, `node_key`, `producer_gset.bin`, `peers.cache`, `producer.lock`
   - **SECONDARY**: clean `~/repos/localdoli/testnetlinux/<node>/` top-level stale files from older layouts: `chain_state.bin`, `producers.bin`, `utxo.bin`, `producer_gset.bin`, `peers.cache`, `producer.lock`, `chainspec.json`, `node_key`, `maintainer_state.bin`, `blocks/`, `signed_slots.db/`, `utxo_rocks/`, `state_db/`
   - **KEEP ONLY**: `keys/`, `explorer/`, `doli-swap-bot/`, `scripts/`
   - **Apply to**: `seed/`, `n1/` through `n5/`
   - **Also wipe**: `seed/blocks/` (archive data)

3. **Verify**: `ls ~/repos/localdoli/testnetlinux/<node>/data/` must be empty for seed and n1-n5. If `signed_slots.db` survives, nodes will hit slashing protection and refuse to produce.

## Phase 2: Update Genesis (in ~/repos/doli)

4. **Calculate new genesis timestamp** = NOW + 10 minutes (Unix epoch):
   ```
   echo $(($(date +%s) + 600))
   ```

5. **Update ALL 3 sources in ~/repos/doli** — miss one and the embedded chainspec will be stale:
   - `chainspec.testnet.json` → timestamp + message
   - `crates/core/src/consensus/constants.rs` → `GENESIS_TIME` constant (mainnet only — skip if testnet-only reset)
   - `crates/core/src/network_params/defaults.rs` → testnet `genesis_time` field

6. **Grep for the OLD timestamp** across `~/repos/doli` — 0 matches must remain

7. **Commit on DPFAESI**: `--author "Ivan D. Lozada <ivan@doli.network>"`

## Phase 3: Compile (from ~/repos/doli)

8. **Build from doli repo**:
   ```
   cd ~/repos/doli && git branch --show-current   # must say DPFAESI
   cargo build --release -p doli-node -p doli-cli
   ```

9. **Record hash**: `sha256sum ~/repos/doli/target/release/doli-node ~/repos/doli/target/release/doli`

10. **No deploy step** — systemd user services point to `~/repos/doli/target/release/doli-node`. Single machine, single binary.

## Phase 4: Start & Verify

11. **DO NOT start nodes until Phase 1-3 are fully verified** — binary built clean, all data dirs empty, genesis timestamp updated in all sources

12. **Start seed first**, wait 5s, then producers N1-N5 only:
    ```
    ~/repos/localdoli/testnetlinux/scripts/testnet.sh start seed
    sleep 5
    ~/repos/localdoli/testnetlinux/scripts/testnet.sh start n1 n2 n3 n4 n5
    ```

13. **Immediately check** `getChainInfo` via JSON-RPC POST to `http://127.0.0.1:8500/` (seed) — `bestHeight` and `bestSlot` MUST be 0

14. **Wait for genesis**, then check again — `bestHeight` and `bestSlot` should start incrementing from 1. If any node shows `bestHeight=0` after genesis passed, check logs for `SLASHING PROTECTION` — means `signed_slots.db` wasn't wiped:
    ```
    ~/repos/localdoli/testnetlinux/scripts/testnet.sh status
    ~/repos/localdoli/testnetlinux/scripts/testnet.sh logs seed
    ~/repos/localdoli/testnetlinux/scripts/testnet.sh logs n1
    ```

15. Once chain is producing, optionally start explorer:
    ```
    ~/repos/localdoli/testnetlinux/scripts/testnet.sh start explorer
    ```

16. **Final verify**: `~/repos/localdoli/testnetlinux/scripts/testnet.sh status` — all nodes synced, same height/hash. Explorer at `http://localhost:8080/network.html` shows green.

## Testnet Parameters

- **Code repo**: `~/repos/doli` (branch `DPFAESI`)
- **Operational repo**: `~/repos/localdoli` (no source code)
- **Binary**: `~/repos/doli/target/release/doli-node`
- **CLI**: `~/repos/doli/target/release/doli`
- **Bond unit**: 0.01 DOLI (1,000,000 base units)
- **Unbonding period**: 720 blocks (2 epochs)
- **Network**: `--network testnet`
- **Genesis nodes**: seed + N1-N5 only
- **All ports on 127.0.0.1**: seed=8500, N{i}=8500+i

## Key Notes

- No SSH, no cross-server coordination — everything runs locally
- Uses **systemd user services** (no sudo) — `testnetlinux/scripts/testnet.sh` replaces `launchctl`
- Binary always built from `~/repos/doli` (DPFAESI branch), never from localdoli
- `~/repos/localdoli` contains ONLY runtime data, keys, scripts, and explorer — no source code
- Only seed + N1-N5 at genesis. Additional producers (N6-N12) can join later via registration
- Explorer at `http://localhost:8080/`, network status at `http://localhost:8080/network.html`
