# cli — DOLI CLI Interface
<!-- @INDEX
ENTRY-POINTS: lines 15-35
COMMANDS: lines 38-450
DATA-FLOWS: lines 453-490
DEPENDENCIES: lines 493-510
CONSTRAINTS: lines 513-535
PATTERNS: lines 538-575
-->

## ENTRY-POINTS

Binary: `doli` (`bins/cli/src/main.rs:91`)
Parser entry: `commands.rs:Cli` (clap Parser struct)

Global flags (apply to all subcommands):
- `-w, --wallet <PATH>` — wallet file; default: auto-detected via `paths::resolve_wallet_path()` (priority: flag > `DOLI_WALLET_FILE` env > `{data_dir}/wallet.json`)
- `-r, --rpc <URL>` — node RPC endpoint; env: `DOLI_RPC_URL`; auto-default: mainnet=`http://127.0.0.1:8500`, testnet=`http://127.0.0.1:18500`, devnet=`http://127.0.0.1:28500`
- `-n, --network <NET>` — `mainnet|testnet|devnet`; default: `mainnet`; env: `DOLI_NETWORK`

Address prefixes by network: mainnet=`doli`, testnet=`tdoli`, devnet=`ddoli`

Data dir resolution (Linux: `/var/lib/doli/{network}`, macOS: `~/Library/Application Support/doli/{network}`, legacy: `~/.doli/{network}`)

RPC client: `rpc_client.rs:RpcClient` — JSON-RPC 2.0 over HTTP POST; connect timeout 10s, request timeout 30s. Archiver fallback: on "not found" errors retries against seed endpoints (`seed1.doli.network:8500`, `seed2.doli.network:8500`).

---

## COMMANDS

### Wallet Management

**`init`** (`cmd_init.rs`) — `commands.rs:31`
```
doli init [--force] [--non-producer]
```
Combines `new` + `add-bls`. Creates wallet file and BLS key.
- `--force` — overwrite existing wallet (DANGEROUS: destroys existing keys)
- `--non-producer` — skip BLS key generation
No RPC calls.

**`new`** (`cmd_wallet.rs:10`) — `commands.rs:41`
```
doli new [--name <NAME>]
```
Generates a new wallet with random Ed25519 keypair. Writes `wallet.json` and `wallet.seed.txt` (chmod 600).
No RPC calls.

**`restore`** (`cmd_wallet.rs:68`) — `commands.rs:47`
```
doli restore [--name <NAME>]
```
Restore from 24-word BIP-39 seed phrase (read from stdin to avoid shell history exposure).
No RPC calls.

**`address`** (`cmd_wallet.rs:103`) — `commands.rs:53` — MUTATING
```
doli address [--label <LABEL>]
```
Generates a new non-HD address and saves to wallet. WARNING: Do not use to inspect existing addresses — it creates a new one.
No RPC calls.

**`addresses`** (`cmd_wallet.rs:119`) — `commands.rs:59`
```
doli addresses
```
Shows primary address only (secondary addresses exist for UTXO consolidation but are hidden from users).
No RPC calls.

**`balance`** (`cmd_wallet.rs:137`) — `commands.rs:63`
```
doli balance [-A <ADDRESS>] [--all]
```
RPC calls: `getBalance`, `getProducers` (to show bonded/activating amounts).
- `-A, --address` — show only for specific address
- `--all` — per-address breakdown

**`send`** (`cmd_wallet.rs:324`) — `commands.rs:74`
```
doli send <TO> <AMOUNT> [--fee <FEE>] [--condition <COND>] [--yes]
```
RPC calls: `getChainInfo` (ping), `getUtxos`, `sendTransaction`.
- `<TO>` — must be bech32 address (`doli1...`); raw 64-char hex rejected
- `<AMOUNT>` — decimal DOLI string (8 decimal places max)
- `--fee` — default: 1 satoshi flat fee
- `--condition` — covenant: `multisig(2,a1,a2,a3)`, `hashlock(hex)`, `htlc(hex,lock_h,expiry_h)`, `timelock(min_h)`, `vesting(addr,unlock_h)`
- `--yes` — skip confirmation prompt
Change always goes to primary wallet address. Selects UTXOs FIFO preferring confirmed.

**`spend`** (`cmd_wallet.rs:548`) — `commands.rs:100`
```
doli spend <UTXO> <TO> <AMOUNT> --witness <WITNESS> [--fee <FEE>]
```
RPC calls: `getChainInfo` (ping), `sendTransaction`.
Spend a covenant-conditioned UTXO. UTXO format: `txhash:output_index`.
Witness forms: `preimage(hex_secret)`, `sign(wallet1.json,wallet2.json)`, `branch(right,preimage(hex))`, `none()`

**`history`** (`cmd_wallet.rs:649`) — `commands.rs:124`
```
doli history [--limit <N>]     # default: 10
```
RPC calls: `getChainInfo` (ping), `getHistory`. Archiver fallback active.

**`export`** (`cmd_wallet.rs:713`) — `commands.rs:130`
```
doli export <OUTPUT_PATH>
```
No RPC calls.

**`import`** (`cmd_wallet.rs:722`) — `commands.rs:136`
```
doli import <INPUT_PATH>
```
No RPC calls.

**`info`** (`cmd_wallet.rs:732`) — `commands.rs:143`
```
doli info
```
Shows wallet name, addresses count, primary address bech32, public key, BLS key (or "none").
No RPC calls.

**`add-bls`** (`cmd_wallet.rs:754`) — `commands.rs:146`
```
doli add-bls
```
Add BLS attestation key to existing wallet. Required before producer registration.
No RPC calls.

**`sign`** (`cmd_wallet.rs:777`) — `commands.rs:149`
```
doli sign <MESSAGE> [--address <ADDR>]
```
No RPC calls.

**`verify`** (`cmd_wallet.rs:788`) — `commands.rs:160`
```
doli verify <MESSAGE> <SIGNATURE_HEX> <PUBKEY>
```
No RPC calls.

---

### Producer Commands (`cmd_producer/`)

**`producer register`** (`cmd_producer/register.rs:8`) — `commands.rs:817`
```
doli producer register [--bonds <N>]    # default: 1, range: 1-10000
```
RPC calls: `getChainInfo` (ping + height), `getProducer`, `getNetworkParams`, `getUtxos`, `sendTransaction`, `getEpochInfo` (ETA display).
Computes registration VDF (~5s hash-chain, T_REGISTER_BASE iterations).
Requires BLS key in wallet (`doli add-bls` first).
Epoch-deferred: activates at next epoch boundary.
Bond outputs created with `creation_slot=0` — node stamps real slot at `apply_block()`.

**`producer status`** (`cmd_producer/status.rs:10`) — `commands.rs:824`
```
doli producer status [--pubkey <HEX>]
```
RPC calls: `getChainInfo` (ping), `getProducer`, `getBondDetails`, `getEpochInfo`.
Shows bond vesting tiers (Q1/Q2/Q3/vested), pending updates, epoch ETA.

**`producer bonds`** (`cmd_producer/status.rs:184`) — `commands.rs:831`
```
doli producer bonds [--pubkey <HEX>]
```
RPC calls: `getChainInfo` (ping), `getBondDetails`.
Per-bond table: creation slot, age, quarter (Q1-Q4+), penalty%, time to next tier.
Bond penalty tiers: Q1 (0-1 vesting_quarter: 75%), Q2 (1-2: 50%), Q3 (2-3: 25%), vested (3+: 0%).

**`producer list`** (`cmd_producer/status.rs:270`) — `commands.rs:838`
```
doli producer list [--active] [--format table|json|csv]
```
RPC calls: `getChainInfo` (ping), `getProducers`.

**`producer add-bond`** (`cmd_producer/bonds.rs`) — `commands.rs:851`
```
doli producer add-bond --count <N>      # range: 1-10000
```
RPC calls: ping, `getNetworkParams`, `getUtxos`, `sendTransaction`, `getEpochInfo`.
Epoch-deferred.

**`producer request-withdrawal`** (`cmd_producer/withdrawal.rs:10`) — `commands.rs:857`
```
doli producer request-withdrawal --count <N> [--destination <HEX_OR_BECH32>]
```
RPC calls: ping, `getBondDetails`, `getUtxos`, `getNetworkParams`, `sendTransaction`, `getEpochInfo`.
FIFO withdrawal (oldest bonds first). Vesting penalty applies.
Destination defaults to wallet address. Accepts bech32 (`doli1...`) or hex pubkey_hash.
Creates RequestWithdrawal tx with bond UTXOs + normal UTXO (fee).

**`producer simulate-withdrawal`** (`cmd_producer/bonds.rs`) — `commands.rs:869`
```
doli producer simulate-withdrawal --count <N>
```
RPC calls: ping, `getBondDetails`.
Dry run — shows FIFO breakdown and net amount, no transaction submitted.

**`producer exit`** (`cmd_producer/exit.rs:10`) — `commands.rs:878`
```
doli producer exit [--force]
```
RPC calls: ping, `getProducer`, `getBondDetails`, `getUtxos`, `getNetworkParams`, `sendTransaction`.
Without `--force`: shows breakdown and exits without submitting. With `--force`: submits withdrawal of ALL available bonds.
Epoch-deferred removal from producer set.

**`producer slash`** (`cmd_producer/exit.rs:204`) — `commands.rs:885`
```
doli producer slash --block1 <HASH> --block2 <HASH>
```
RPC calls: ping, `getBlockByHash` (twice).
Verifies equivocation (same slot, same producer, different hash). NOTE: Currently informational only — actual on-chain slashing submission requires raw block data with VDF proofs; nodes auto-detect and submit internally.

**`producer delegate`** (`cmd_producer/delegation.rs:11`) — `commands.rs:894`
```
doli producer delegate <DELEGATEE_PUBKEY_HEX> --bonds <N>   # range: 1-100
```
RPC calls: ping, `getProducer` (self), `getProducer` (delegatee), `getNetworkParams`, `sendTransaction`, `getEpochInfo`.
Cannot self-delegate. Both parties must be active producers. Epoch-deferred.
Reward split: delegatee keeps 10%, delegator receives 90%.

**`producer revoke-delegation`** (`cmd_producer/delegation.rs:140`) — `commands.rs:904`
```
doli producer revoke-delegation
```
RPC calls: ping, `getProducer`, `sendTransaction`, `getEpochInfo`.
Epoch-deferred. Unbonding delay (DELEGATION_UNBONDING_SLOTS) applies after revocation.

**`producer delegation-status`** (`cmd_producer/delegation.rs:223`) — `commands.rs:907`
```
doli producer delegation-status [--address <PUBKEY_HEX>]
```
RPC calls: ping, `getProducer`.
Shows: outgoing delegation, delegated bonds, received delegations (delegator hash, bond count), effective selection weight.

---

### Chain Commands (`cmd_chain.rs`)

**`chain`** (`cmd_chain.rs:8`) — `commands.rs:184`
```
doli chain
```
RPC calls: `getChainInfo`.
Shows: network, best height, best slot, best hash, genesis hash, reward pool balance.

**`chain-verify`** (`cmd_chain.rs:31`) — `commands.rs:186`
```
doli chain-verify
```
RPC calls: `verifyChainIntegrity`.
Scans all blocks 1..tip, computes BLAKE3 chain commitment: `commitment[N] = BLAKE3(commitment[N-1] || block_hash[N])`.
Two nodes with same commitment have identical chains.

---

### Rewards Commands (`cmd_chain.rs:74`)

**`rewards list`** — `commands.rs:916`
**`rewards claim`** — `commands.rs:921`
**`rewards claim-all`** — `commands.rs:930`
**`rewards history`** — `commands.rs:936`
```
doli rewards list
doli rewards claim <EPOCH> [--recipient <ADDR>]
doli rewards claim-all [--recipient <ADDR>]
doli rewards history [--limit <N>]    # default: 20
```
NOTE: All rewards are distributed automatically via coinbase. These subcommands print informational messages only — no claiming transaction needed.

**`rewards info`** (`cmd_chain.rs:111`) — `commands.rs:943`
```
doli rewards info
```
RPC calls: ping, `getEpochInfo`.
Shows: current height/epoch, last complete epoch, blocks per epoch, blocks remaining, epoch range, block reward, progress bar.

---

### Governance Commands (`cmd_governance.rs`)

**`update check`** (`cmd_governance.rs:237`) — `commands.rs:949`
```
doli update check
```
RPC calls: ping, `getNodeInfo`, `getUpdateStatus`.

**`update status`** (`cmd_governance.rs:281`) — `commands.rs:953`
```
doli update status
```
RPC calls: ping, `getUpdateStatus`.
Shows veto period, veto count, veto percent (threshold: 40%).

**`update vote`** (`cmd_governance.rs:325`) — `commands.rs:957`
```
doli update vote --version <VER> (--veto | --approve)
```
RPC calls: ping, `submitVote`.
Signs message `"{version}:{vote_type}:{timestamp}"` with wallet key.

**`update votes`** (`cmd_governance.rs:374`) — `commands.rs:964`
```
doli update votes --version <VER>
```
RPC calls: ping, `getUpdateStatus`.

**`update apply`** / **`update rollback`** — `commands.rs:970,974`
```
doli update apply
doli update rollback
```
Informational only (no RPC calls). Updates apply automatically after 7-day veto period. Rollback restores from `~/.doli/doli-node.backup`.

**`maintainer list`** (`cmd_governance.rs:431`) — `commands.rs:986`
```
doli maintainer list
```
RPC calls: ping, `getMaintainerSet`.
Shows threshold (3/5) and maintainer pubkeys. Derived from first 5 registered producers.

**`release sign`** (`cmd_governance.rs:18`) — `commands.rs:991`
```
doli release sign --version <VER> [--key <PATH>]
```
Downloads `CHECKSUMS.txt` from GitHub. Signs `"{version}:{sha256(CHECKSUMS.txt)}"` with Ed25519.
Outputs JSON signature block: `{"public_key": "...", "signature": "..."}`.
No node RPC calls.

**`protocol sign`** (`cmd_governance.rs:87`) — `commands.rs:1012`
```
doli protocol sign --version <N> --epoch <N> [--key <PATH>]
```
Signs `"activate:{version}:{epoch}"` with Ed25519. Outputs JSON signature block.
No node RPC calls.

**`protocol activate`** (`cmd_governance.rs:130`) — `commands.rs:1030`
```
doli protocol activate --version <N> --epoch <N> --description <TEXT> --signatures <FILE>
```
RPC calls: `sendTransaction`.
Requires 3+ signatures in JSON array file. Builds ProtocolActivation transaction.

---

### NFT Commands (`cmd_nft/`)

**`nft --list`** (`cmd_nft/list.rs`) — `commands.rs:237`
```
doli nft --list
```
RPC calls: `getUtxos`.

**`nft --info`** (`cmd_nft/info.rs`) — `commands.rs:243`
```
doli nft --info <UTXO>
```
RPC calls: `getTransaction`. Archiver fallback active.

**`nft --mint`** (`cmd_nft/mint.rs`) — `commands.rs:248`
```
doli nft --mint <CONTENT> [--condition <C>] [--amount <A>] [--royalty <PCT>] [--content-type <MIME>] [--data <HEX>] [--data-file <PATH>]
```
RPC calls: `getUtxos`, `sendTransaction`.
`--data-file` reads raw bytes from file and converts to hex.
Royalty in percent (e.g., `5` = 5%, max 25%).

**`nft --export`** (`cmd_nft/export.rs`) — `commands.rs:275`
```
doli nft --export <UTXO> -o <FILE>
```
RPC calls: `getTransaction`. Archiver fallback active. Extracts on-chain content to local file.

**`nft --batch-mint`** (`cmd_nft/batch.rs`) — `commands.rs:279`
```
doli nft --batch-mint <MANIFEST_FILE> [--yes]
```
RPC calls: `getUtxos`, `sendTransaction` (per NFT).

**`nft --transfer`** (`cmd_nft/transfer.rs`) — `commands.rs:281`
```
doli nft --transfer <UTXO> --to <ADDR> [--witness <W>]
```
RPC calls: `getUtxos`, `getTransaction`, `sendTransaction`. Archiver fallback active.
For EncryptedContent NFTs: re-wraps ECIES key for new owner.

**`nft --sell`** (`cmd_nft/sell.rs`) — `commands.rs:284`
```
doli nft --sell <UTXO> --price <DOLI> -o <FILE>
```
RPC calls: `getTransaction`. Creates unsigned sell offer JSON file (no broadcast).

**`nft --sell-sign`** (`cmd_nft/sell.rs`) — `commands.rs:290`
```
doli nft --sell-sign <UTXO> --price <DOLI> --to <BUYER_ADDR> -o <FILE>
```
RPC calls: `getTransaction`. Creates signed PSBT sell offer (seller pre-signs NFT input). Buyer can complete without seller wallet.

**`nft --buy`** (`cmd_nft/buy.rs`) — `commands.rs:300`
```
doli nft --buy <UTXO> --price <DOLI> --seller-wallet <PATH>
```
RPC calls: `getTransaction`, `getUtxos`, `sendTransaction`. Archiver fallback.

**`nft --from`** (`cmd_nft/buy.rs:cmd_nft_buy_from_offer`) — `commands.rs:307`
```
doli nft --from <OFFER_FILE> [--seller-wallet <PATH>]
```
RPC calls: `getTransaction`, `getUtxos`, `sendTransaction`. Archiver fallback.
For signed PSBT offers: `--seller-wallet` not needed.

**`nft --fractionalize`** (`cmd_nft/fractionalize.rs`) — `commands.rs:315`
```
doli nft --fractionalize <TOKEN_ID> --shares <N> [--ticker <NAME>]
```
RPC calls: `getUtxos`, `getTransaction`, `sendTransaction`. Archiver fallback.
Converts NFT into `N` fungible shares (ticker default: `FRAC`).

**`nft --redeem`** (`cmd_nft/redeem.rs`) — `commands.rs:326`
```
doli nft --redeem <TOKEN_ID>
```
RPC calls: `getUtxos`, `sendTransaction`.
Burns all fraction shares to recover whole NFT.

---

### Token Commands (`cmd_token.rs`)

**`issue-token`** — `commands.rs:344`
```
doli issue-token <TICKER> --supply <N> [--condition <C>]
```
RPC calls: `getUtxos`, `sendTransaction`.
Fixed total supply at issuance. Ticker max 16 chars.

**`token-info`** — `commands.rs:360`
```
doli token-info <UTXO>
```
RPC calls: `getTransaction`. Archiver fallback.

---

### Bridge Commands (`cmd_bridge.rs`)

**`bridge-swap`** — `commands.rs:365`
```
doli bridge-swap <AMOUNT> --chain <CHAIN> --to <ADDR> [--counter-rpc <URL>] [--confirmations <N>]
```
Chains: `bitcoin`, `ethereum`, `monero`, `litecoin`, `cardano`, `bsc`
RPC calls: `getUtxos`, `sendTransaction`.
Initiates complete atomic swap: generates preimage, locks DOLI in bridge HTLC.

**`bridge-status`** — `commands.rs:387`
```
doli bridge-status <SWAP_ID> [--btc-rpc <URL>] [--eth-rpc <URL>] [--auto]
```
RPC calls: `getTransaction` (local + archiver fallback). `--auto` triggers auto-claim/refund.

**`bridge-buy`** — `commands.rs:405`
```
doli bridge-buy <SWAP_ID> [--preimage <HEX>] [--btc-rpc <URL>] [--eth-rpc <URL>] [--to <ADDR>] [--yes]
```
Counterparty/buyer side. Can auto-detect preimage from BTC/ETH chain.

**`bridge-watch`** — `commands.rs:431`
```
doli bridge-watch [--btc-rpc <URL>] [--eth-rpc <URL>] [--interval <SECS>]   # default: 10s
```
Watcher daemon: monitors swaps, auto-detects preimages, claims/refunds.

**`bridge-list`** — `commands.rs:447`
```
doli bridge-list [--chain <CHAIN>] [--blocks <N>]     # default: 100 blocks
```
RPC calls: `getChainInfo`, block scan.

**`bridge-lock`** (advanced) — `commands.rs:457`
```
doli bridge-lock <AMOUNT> (--hash <HEX> | --preimage <HEX>) --lock <H> --expiry <H> --chain <CHAIN> --to <ADDR> --counter-hash <HEX> [--multisig-threshold <N> --multisig-keys <ADDRS>] [--yes]
```
Manual HTLC creation. `--hash` and `--preimage` are mutually exclusive. Hashlock computed via `BLAKE3(HASHLOCK_DOMAIN, preimage)`.

**`bridge-claim`** — `commands.rs:503`
```
doli bridge-claim <UTXO> --preimage <HEX> [--to <ADDR>] [--yes]
```
Receiver-side: claim bridge HTLC with preimage.

**`bridge-refund`** — `commands.rs:521`
```
doli bridge-refund <UTXO> [--yes]
```
Sender-side: refund HTLC after expiry height.

---

### Pool Commands (`cmd_pool.rs`)

**`pool create`** — `commands.rs:597`
```
doli pool create --asset <ASSET_ID_HEX> --doli <AMOUNT> --tokens <AMOUNT> [--fee <BPS>] [--yes]
```
Fee default: 30 bps (0.3%). Creates AMM pool with initial liquidity.

**`pool swap`** — `commands.rs:621`
```
doli pool swap --pool <HEX> --amount <AMOUNT> --direction a2b|b2a [--min-out <AMOUNT>] [--yes]
```
`a2b` = DOLI→token, `b2a` = token→DOLI.

**`pool add`** — `commands.rs:642`
```
doli pool add --pool <HEX> --doli <AMOUNT> --tokens <AMOUNT> [--yes]
```

**`pool remove`** — `commands.rs:663`
```
doli pool remove --pool <HEX> --shares <AMOUNT> [--min-doli <AMOUNT>] [--min-tokens <AMOUNT>] [--yes]
```

**`pool list`** / **`pool info`** — `commands.rs:684,686`
```
doli pool list
doli pool info <POOL_ID>
```

---

### Loan Commands (`cmd_loan.rs`)

**`loan deposit`** — `commands.rs:697`
```
doli loan deposit --pool <HEX> --amount <AMOUNT> [--yes]
```

**`loan withdraw`** — `commands.rs:709`
```
doli loan withdraw <DEPOSIT_UTXO> [--yes]
```

**`loan create`** — `commands.rs:717`
```
doli loan create --pool <HEX> --collateral <TOKEN_UNITS> --borrow <DOLI> [--interest-rate <BPS>] [--yes]
```
Default interest: 500 bps (5%).

**`loan repay`** — `commands.rs:733`
```
doli loan repay <LOAN_UTXO> [--yes]
```

**`loan liquidate`** — `commands.rs:741`
```
doli loan liquidate <LOAN_UTXO> [--yes]
```

**`loan list`** / **`loan info`** — `commands.rs:748,755`
```
doli loan list [--borrower <HEX>]
doli loan info <LOAN_UTXO>
```

---

### Channel Commands (`cmd_channel.rs`)

**`channel open`** — `commands.rs:766`
```
doli channel open <PEER_ADDR> <CAPACITY_DOLI> [--fee <FEE>]
```

**`channel pay`** — `commands.rs:779`
```
doli channel pay <CHANNEL_ID_HEX> <AMOUNT_DOLI>
```

**`channel close`** — `commands.rs:787`
```
doli channel close <CHANNEL_ID_HEX> [--fee <FEE>] [--force]
```
`--force` = unilateral close using latest commitment tx.

**`channel list`** / **`channel info`** — `commands.rs:802,808`
```
doli channel list [--all]
doli channel info <CHANNEL_ID_HEX>
```

---

### Service Commands (`cmd_service.rs`)

**`service install`** — `commands.rs:1081`
```
doli service install [--network <NET>] [--name <NAME>] [--data-dir <PATH>] [--producer-key <PATH>] [--p2p-port <PORT>] [--rpc-port <PORT>]
```
Requires root/sudo on Linux. Installs systemd unit (Linux) or launchd plist (macOS).
Auto-detects wallet: if found → producer mode; if not → full node mode.
Service user: uses `doli` system user if exists, otherwise falls back to `SUDO_USER`.
Logs: Linux=`/var/log/doli/{network}.log`, macOS=`~/Library/Logs/doli/{network}.log`

**`service uninstall`** / **`service start`** / **`service stop`** / **`service restart`** — `commands.rs:1108-1135`
```
doli service uninstall [--name <NAME>]
doli service start [--name <NAME>]
doli service stop [--name <NAME>]
doli service restart [--name <NAME>]
```
Default service name: `doli-{network}` (Linux) / `network.doli.{network}` (macOS).

**`service status`** — `commands.rs:1137`
```
doli service status [--name <NAME>]
```

**`service logs`** — `commands.rs:1144`
```
doli service logs [--name <NAME>] [--follow] [-n <LINES>]    # default: 50 lines
```
Checks log file first, falls back to `journalctl` (Linux).

---

### Guardian Commands (`cmd_guardian.rs`)

**`guardian status`** (`cmd_guardian.rs:27`) — `commands.rs:1053`
```
doli guardian status
```
RPC calls: `getGuardianStatus`.
Shows: production state (ACTIVE/PAUSED), chain height/slot/hash, last checkpoint, last healthy checkpoint.

**`guardian halt`** (`cmd_guardian.rs:88`) — `commands.rs:1055`
```
doli guardian halt [--yes]
```
RPC calls: `pauseProduction`.

**`guardian resume`** (`cmd_guardian.rs:128`) — `commands.rs:1063`
```
doli guardian resume
```
RPC calls: `resumeProduction`.

**`guardian checkpoint`** (`cmd_guardian.rs:157`) — `commands.rs:1065`
```
doli guardian checkpoint
```
RPC calls: `createCheckpoint`.
Creates RocksDB checkpoint (hot backup). Returns: status, height, path.

**`guardian monitor`** (`cmd_guardian.rs:191`) — `commands.rs:1068`
```
doli guardian monitor --endpoint <URL> [--endpoint <URL>...] [--loop <SECS>]
```
RPC calls: `getChainInfo` on each endpoint.
Groups nodes by `best_hash`. Reports FORK DETECTED if >1 distinct tip. `--loop` for continuous monitoring.

---

### Snap/Wipe Commands

**`snap`** (`cmd_snap.rs:23`) — `commands.rs:495`
```
doli snap [--data-dir <PATH>] [--seed <URL>...] [--no-restart] [--trust]
```
RPC calls (on seeds): `getStateRootDebug`, `getStateSnapshot`.
1. Verifies state root consensus (2/3 seeds must agree unless `--trust`)
2. Stops node service (unless `--no-restart`)
3. Wipes chain data (preserves: `keys/`, `.env`, `node_key`, `wallet.json`, `wallet.seed.txt`, `config.toml`)
4. Downloads state snapshot (chainState, utxoSet, producerSet, epochBondSnapshot, epochAccumulators)
5. Verifies integrity: recomputes state root from bytes
6. Applies to `state_db/`
7. On Linux: fixes file ownership to `doli:doli` user
8. Restarts matching service

Seed defaults: mainnet=`seed1/seed2/seeds.doli.network:8500`, testnet=`seeds.testnet.doli.network:18500`, devnet=`127.0.0.1:8500`.

**`wipe`** (`cmd_chain.rs:157`) — `commands.rs:487`
```
doli wipe [--network <NET>] [--data-dir <PATH>] [--yes]
```
No RPC calls.
Deletes everything in data dir EXCEPT: `keys/`, `.env`, `wallet.json`, `wallet.seed.txt`, `node_key`, `config.toml`.
Stops matching systemd service before wipe. Restarts it after (if service unit file exists).

---

### Upgrade Command (`cmd_upgrade.rs`)

**`upgrade`** — `commands.rs:205`
```
doli upgrade [--version <VER>] [--yes] [--doli-node-path <PATH>] [--service <NAME>]
```
Fetches GitHub release, downloads tarball, verifies CHECKSUMS.txt against maintainer signatures (3/5 required), replaces binary, restarts service.
`--service` — restart only this specific service (for multi-node server deployments).

---

## DATA-FLOWS

**Transaction submission path:**
1. CLI selects UTXOs via `getUtxos` RPC
2. Builds `Transaction` struct (doli_core)
3. Signs inputs with Ed25519 keypair from wallet (BIP-143 per-input signing hash)
4. Serializes to bytes → hex
5. Submits via `sendTransaction` RPC → node broadcasts to gossip network

**Balance display:**
- `getUtxos` returns spendable UTXO amounts
- `getProducers` maps pubkey_hash → bond_amount (active) and pending_activation_amount (pending)
- Displayed breakdown: Spendable / Bonded (in ProducerSet) / Activating / Immature / Pending (mempool)

**Bond lifecycle in CLI:**
1. `producer register --bonds N` → Registration tx → epoch boundary → active in ProducerSet
2. `producer add-bond --count N` → AddBond tx → epoch boundary → additional bonds in ProducerSet
3. `producer request-withdrawal --count N` → RequestWithdrawal tx → funds returned immediately, bonds removed at epoch boundary
4. `producer exit --force` → same as request-withdrawal for ALL bonds

**NFT sell/buy flow:**
- Unsigned: `nft --sell` writes offer JSON file. Buyer needs `--seller-wallet` for `--buy`
- Signed PSBT: `nft --sell-sign` pre-signs seller's NFT input. Buyer uses `--from offer.json` without seller wallet
- EncryptedContent NFTs: re-wraps ECIES content key for new owner during transfer/buy

**Snap sync flow:**
- Queries `getStateRootDebug` on 2+ seeds for consensus
- Downloads `getStateSnapshot` (chainState + utxoSet + producerSet + bond/attestation snapshots)
- Recomputes state root locally to verify integrity
- Writes directly to `state_db/` via `storage::StateDb::atomic_replace()`

**Archiver fallback:** On "not found" RPC errors, `RpcClient` automatically retries against seed archivers. Activated for: `getTransaction`, `getHistory`.

---

## DEPENDENCIES

**Crates used by CLI:**
- `doli_core` — `Transaction`, `Input`, `Output`, `TxType`, consensus constants
- `crypto` — `KeyPair`, `PublicKey`, `Signature`, `Hash`, `address`, `signature`, `bls_sign_pop`, `encrypted_content`
- `storage` — `StateDb`, `UtxoSet`, `ChainState`, `ProducerSet`, `snapshot::compute_state_root_from_bytes`
- `vdf` — `registration_input`, `T_REGISTER_BASE`
- `updater` — `fetch_github_release`, `download_checksums_txt`, `sign_release_hash`, `current_version`
- `bincode` — serialization for snapshot data
- `reqwest` — HTTP client for RPC calls (tokio async)
- `clap` — argument parsing

**Key types (`rpc_client.rs`):**
- `Balance` — `{confirmed, unconfirmed, immature, total}`
- `Utxo` — `{tx_hash, output_index, amount, output_type, lock_until, height, spendable, pending, asset}`
- `ProducerInfo` — `{public_key, registration_height, bond_amount, bond_count, status, era, pending_withdrawals, pending_updates, delegated_to, delegated_bonds, received_delegations, selection_weight}`
- `BondDetailsInfo` — `{bond_count, total_staked, summary{q1,q2,q3,vested}, bonds[BondEntryInfo], withdrawal_pending_count, vesting_quarter_slots, vesting_period_slots}`
- `EpochInfoResponse` — `{current_height, current_epoch, last_complete_epoch, blocks_per_epoch, blocks_remaining, epoch_start_height, epoch_end_height, block_reward}`

**Amount conversion (`rpc_client.rs:824-854`):**
- `units_to_coins(u64) -> String` — base units → `"1.23456789"` (pure integer, no f64)
- `coins_to_units(&str) -> Result<u64, String>` — `"1.23456789"` → base units (max 8 decimal places, overflow checked)
- 1 DOLI = 100_000_000 base units

---

## CONSTRAINTS

**Security constraints (enforced in code):**
- `send` rejects raw 64-char hex addresses — must use bech32 (`doli1...`) to avoid pubkey vs pubkey_hash ambiguity (bug where 32 DOLI burned 2026-03-22)
- `nft --buy` validates buyer pubkey is valid Ed25519 curve point before ECIES (AUDIT-CRYPTO-002)
- EncryptedContent `extra_data` bounds-checked before parsing (AUDIT-CRYPTO-010)
- `producer delegate` rejects self-delegation
- `producer register` rejects already-active/pending producers

**Bond constraints:**
- Registration: 1-10000 bonds
- AddBond: 1-10000 count
- RequestWithdrawal: must be ≤ `available_bonds` (total - withdrawal_pending)
- Delegate: 1-100 bonds (delegation cap)
- Protocol requires exactly 3+ maintainer signatures for ProtocolActivation

**Epoch-deferred operations** (take effect at NEXT epoch boundary):
- `producer register`, `producer add-bond`, `producer request-withdrawal`, `producer exit`, `producer delegate`, `producer revoke-delegation`

**Fee policy:** Flat fee of 1 satoshi (1 base unit) for all transactions, except registration which includes `BASE_FEE + (bonds * 4 bytes * FEE_PER_BYTE / FEE_DIVISOR)`.

**WIPE_PRESERVE list** (`cmd_chain.rs:292`): `keys`, `.env`, `wallet.json`, `wallet.seed.txt`, `node_key`, `config.toml` — NEVER deleted by `wipe` or `snap`.

**Snap sync consensus rule:** Requires 2+ seeds to agree on state root. Single seed allowed with `--trust` flag only.

**Service command:** Not supported on Windows. Requires root/sudo on Linux. macOS uses user-scoped launchd (no sudo needed).

**Linux group re-exec:** On Linux, if `doli` group exists but session doesn't have it active, CLI re-execs itself via `sg doli -c "..."` (guarded by `DOLI_SG_REEXEC` env to prevent recursion).

---

## PATTERNS

**Check if registered before register:**
```
doli producer status   # shows "Not registered" if not found
```

**Full producer setup from scratch:**
```
doli init                         # creates wallet + BLS key
doli balance                      # verify wallet funded
doli producer register --bonds 1  # register (VDF takes ~5s)
doli producer status              # check (shows "pending" until epoch boundary)
```

**Safe snap sync (avoid service stop issues):**
```
sudo doli snap --network mainnet
# or with custom seed:
sudo doli snap --seed http://127.0.0.1:8500 --trust
```

**Check fork across nodes:**
```
doli guardian monitor --endpoint http://n1:8500 --endpoint http://n2:8500 --loop 30
```

**Sell NFT workflow (signed PSBT — buyer needs no seller wallet):**
```
# Seller creates signed offer:
doli nft --sell-sign <UTXO> --price 100 --to <BUYER_ADDR> -o offer.json
# Buyer completes:
doli nft --from offer.json
```

**Withdraw all bonds (graceful exit):**
```
doli producer simulate-withdrawal --count <N>   # preview first
doli producer request-withdrawal --count <N>    # submit
```

**Network-specific invocations:**
```
doli -n testnet balance
doli -n devnet producer status
DOLI_NETWORK=testnet doli balance
```

**RPC client archiver fallback pattern:**
`getTransaction` and `getHistory` calls automatically retry against `seed1/seed2/seeds.doli.network:8500` if local node returns "not found" (common post-snap-sync).

**UTXO reference format:** Always `txhash:output_index` (e.g., `abc123def456:0`).
