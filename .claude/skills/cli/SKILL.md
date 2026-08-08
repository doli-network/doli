# cli — DOLI CLI Interface (`bins/cli`)
<!-- @INDEX
ENTRY-POINTS   11-46
OPERATIONS     48-159
DATA-FLOW      161-181
DEPENDENCIES   183-198
CONSTRAINTS    200-235
PATTERNS       237-293
@/INDEX -->

## ENTRY-POINTS

Binary: `doli` (`bins/cli/src/main.rs:92` `async fn main()`)
Parser entry: `commands.rs:9` `pub(crate) struct Cli` — clap `Parser`, dispatch match at `main.rs:117-500`

Global flags (all subcommands):
- `-w, --wallet <PATH>` — wallet file; default via `paths::resolve_wallet_path()` (flag > `DOLI_WALLET_FILE` env > `{data_dir}/wallet.json`)
- `-r, --rpc <URL>` — node RPC endpoint; env `DOLI_RPC_URL`; default: mainnet=`http://127.0.0.1:8500`, testnet=`:18500`, devnet=`:28500` (`common.rs:22`)
- `-n, --network <NET>` — `mainnet|testnet|devnet`; default `mainnet`; env `DOLI_NETWORK`

Address prefixes (`common.rs:14`): mainnet=`doli`, testnet=`tdoli`, devnet=`ddoli`
Data dir (`paths.rs:14,75`): Linux=`/var/lib/doli/{network}`, macOS=`~/Library/Application Support/doli/{network}`, Windows=`%APPDATA%\doli\{network}`, legacy fallback=`~/.doli/{network}`

RPC client: `rpc_client.rs:363 RpcClient` — JSON-RPC 2.0 over HTTP POST (`reqwest`), connect timeout 10s, request timeout 30s (`rpc_client.rs:405-411`). Archiver fallback (`rpc_client.rs:507-546 call_with_archiver_fallback`): on "not found"/`-32001` errors, retries `getTransaction`/`getHistory` against seed archivers (`common.rs:33 archiver_endpoints_for_network`) + local seed `127.0.0.1:8500` when RPC targets localhost.

Module map (`main.rs:12-34`, 55 files total):
| Module | Files | Purpose |
|--------|-------|---------|
| `commands.rs` | 1 | clap `Cli`/`Commands`/all subcommand enums (1355 lines) |
| `cmd_wallet.rs` | 1 | new/restore/address/balance/send/spend/history/export/import/info/add-bls/sign/verify |
| `cmd_producer/` | 9 (mod,common,dispatch,register,status,bonds,withdrawal,exit,delegation) | producer lifecycle |
| `cmd_nft/` | 9 (mod,list,info,mint,export,batch,transfer,sell,buy) | NFT mint/trade |
| `cmd_template/` | 9 (mod,dispatch,serialize,vault,escrow,htlc_payment,subscription,agent_allowance,escrow_loan) | covenant condition templates |
| `cmd_pool.rs` + `pool_tx.rs` + `lp_select.rs` | 3 | AMM pool create/swap/add/remove |
| `cmd_channel.rs` | 1 | payment channel open/pay/close/close-finish |
| `cmd_bridge.rs` | 1 | cross-chain atomic swap HTLC |
| `cmd_chain.rs` | 1 | chain info/verify, rewards info, wipe |
| `cmd_governance.rs` | 1 | update/maintainer/release/protocol |
| `cmd_guardian.rs` | 1 | halt/resume/checkpoint/fork monitor |
| `cmd_snap.rs`, `cmd_service.rs`, `cmd_upgrade.rs`, `cmd_init.rs`, `cmd_token.rs` | 5 | fast-sync, systemd/launchd, self-update, wallet init, fungible token |
| `rpc_client.rs`, `wallet.rs`, `parsers.rs`, `paths.rs`, `common.rs` | 5 | RPC client, wallet crypto, condition/witness parsers, data-dir resolution, network globals |

**REMOVED since 2026-05-11**: `cmd_loan.rs` / `Loan` subcommand tree — lending was tombstoned (B.1, see CLAUDE.md DeFi Phase 0). No `doli loan *` commands exist in the current binary.
**NOT YET EXPOSED**: Oracle attestation (`PriceAttestation` tx, Phase 2.1) has no CLI subcommand — `oracle_activation_height = u64::MAX`, code is RPC-only (`crates/rpc/src/methods/oracle.rs`) and frozen pre-activation.

---

## OPERATIONS

### Wallet lifecycle

| Task | Steps | Command | Inputs | Success |
|------|-------|---------|--------|---------|
| Create a wallet | 1. run `new` 2. write down 24-word phrase | `doli new [--name N]` (`cmd_wallet.rs:10`) | none | `wallet.json` + `wallet.seed.txt` (0600) written; bech32 address printed |
| Set up a producer wallet in one step | 1. run `init` | `doli init [--force] [--non-producer]` (`cmd_init.rs`, dispatched `commands.rs:30`) | none | wallet + BLS key created (combines `new`+`add-bls`) |
| Restore from seed phrase | 1. run `restore` 2. paste 24 words at stdin prompt | `doli restore [--name N]` (`cmd_wallet.rs:68`) | 24-word BIP-39 phrase (via stdin, not arg — avoids shell history) | same address/funds/Ed25519 key, but a **NEW RANDOM BLS key** (`wallet.rs:93`) — does NOT restore a registered producer identity (INC-I-162). Bails if the wallet file exists (`cmd_wallet.rs:69`) |
| Check balance | 1. run `balance` | `doli balance [-A ADDR] [--all]` (`cmd_wallet.rs:137`) | none | Spendable/Bonded/Activating/Immature/Pending breakdown; RPC `getBalance`+`getProducers` |
| Send DOLI | 1. `send <to> <amount>` | `doli send <TO> <AMOUNT> [--fee F] [--condition C] [--yes]` (`cmd_wallet.rs:324`) | bech32 `TO` (raw 64-hex rejected — `cmd_wallet.rs:346`), decimal amount ≤8dp | tx broadcast, hash printed; fee auto = size-scaled `minimum_fee()` (INC-I-099, `cmd_wallet.rs:1004`), not flat |
| Attach a covenant to a payment | 1. `send ... --condition "cond(...)"` | condition grammar (`parsers.rs:25`): `multisig(t,a1,a2,..)`, `hashlock(hex)`, `htlc(hex,lock,expiry)`, `timelock(h)`, `timelock_expiry(h)`, `vesting(addr,h)`, `and(c1,c2)`, `or(c1,c2)`, `threshold(n,c1,c2,..)`, `amount_guard(min,idx)`, `output_type_guard(type,idx)`, `recipient_guard(addr,idx)` | condition string | output built with matching `OutputType`; guard-conditions on mainnet print a WARNING (guards not activated, `guards_activation_height=MAX`, `cmd_wallet.rs:1012`) |
| Spend a covenant UTXO (single output) | 1. `spend <utxo> <to> <amount> --witness W` | `doli spend <UTXO> [<TO> <AMOUNT>] --witness W [--fee F] [--output SPEC]... [--yes]` (`cmd_wallet.rs:561`) | UTXO `txhash:idx`; witness `preimage(hex)` / `sign(w1.json,w2.json)` / `branch(right,preimage(hex))` / `none()` | tx broadcast; legacy positional path == pre-multi-output behavior |
| Spend a covenant UTXO (multi-output) | 1. `spend <utxo> --witness W --output 0:normal:addr:amt --output 1:...` | same cmd, `--output` repeatable, mutually exclusive with positional TO/AMOUNT (`cmd_wallet.rs:579`) | max 8 outputs (`MAX_SPEND_OUTPUTS`), contiguous indices from 0, types: `normal,multisig,hashlock,htlc,vesting,nft` (protocol-internal types `bond,bridgehtlc,pool,lpshare,zkrollup,encryptedcontent,fungibleasset` rejected) | tx broadcast; warns if computed fee >1% of input or >10000 units (S3 mitigation) |
| View tx history | 1. `history` | `doli history [--limit N]` default 10 (`cmd_wallet.rs:720`) | none | RPC `getHistory` w/ archiver fallback |
| Export/import wallet | 1. `export <path>` / `import <path>` | `cmd_wallet.rs:784,793` | file path | wallet file copied/loaded |
| Add BLS key (needed before producer register) | 1. `add-bls` | `doli add-bls` (`cmd_wallet.rs:825`) | none | BLS keypair added; must restart node to load |
| Sign/verify a message | 1. `sign <msg>` / `verify <msg> <sig> <pubkey>` | `cmd_wallet.rs:848,859` | message string | Ed25519 signature hex |

### Producer lifecycle (`cmd_producer/`)

| Task | Steps | Command | Inputs | Success |
|------|-------|---------|--------|---------|
| Register as producer | 1. `add-bls` (once) 2. `producer register --bonds N` | `doli producer register [--bonds N]` default 1, range 1-10000 (`cmd_producer/register.rs:8`, dispatch `commands.rs:765`) | BLS key in wallet; N bonds funded | Registration tx submitted; ~5s hash-chain VDF computed (`T_REGISTER_BASE`); epoch-deferred activation |
| Check producer status | 1. `producer status` | `doli producer status [--pubkey HEX]` (`cmd_producer/status.rs:11`) | none (defaults to wallet key) | vesting tiers (Q1-Q3/vested), pending updates, epoch ETA printed |
| Show per-bond vesting table | 1. `producer bonds` | `doli producer bonds [--pubkey HEX]` (`cmd_producer/status.rs:185`) | none | table: creation slot, age, quarter, penalty%, time-to-next |
| Show vesting calendar (dates) | 1. `producer vesting-summary` | `doli producer vesting-summary [--pubkey HEX]` (`cmd_producer/status.rs:271`) — **NEW, not in prior skill rev** | none | earliest penalty drop date + full-vest dates + withdraw-now impact estimate |
| List network producers | 1. `producer list` | `doli producer list [--active] [--format table\|json\|csv]` (`cmd_producer/status.rs:434`) | none | producer table |
| Stack more bonds | 1. `producer add-bond --count N` | `doli producer add-bond --count N` 1-10000 (`cmd_producer/bonds.rs:9`) | funded wallet | AddBond tx; epoch-deferred |
| Withdraw bonds (FIFO, keep producing) | 1. `producer simulate-withdrawal --count N` (preview) 2. `producer request-withdrawal --count N` | `cmd_producer/withdrawal.rs:10`, dry-run `cmd_producer/bonds.rs:152` | bond count | funds returned immediately; bonds removed from ProducerSet at next epoch |
| Exit producer set entirely | 1. `producer exit` (preview) 2. `producer exit --force` | `doli producer exit [--force]` (`cmd_producer/exit.rs:10`) | none | without `--force`: shows FIFO breakdown only; with: submits withdrawal of ALL available bonds |
| Report equivocation (slashing evidence) | 1. `producer slash --block1 H1 --block2 H2` | `cmd_producer/exit.rs:204` | two block hashes, same slot+producer, different hash | verified via `getBlockByHash`×2; informational — node auto-detects/submits real slashing |
| Delegate bond weight | 1. `producer delegate <pubkey> --bonds N` | `cmd_producer/delegation.rs:12`, range 1-`MAX_BONDS_PER_PRODUCER` | active self + active delegatee, not self-delegation | DelegateBond tx (Ed25519-signed payload, INC-I-078 M2); delegatee keeps 10%, delegator 90%; epoch-deferred |
| Revoke delegation | 1. `producer revoke-delegation` | `cmd_producer/delegation.rs:154` | active delegation must exist | RevokeDelegation tx; unbonding delay applies |
| Check delegation status | 1. `producer delegation-status` | `cmd_producer/status.rs` (dispatch `commands.rs:860`) | none | outgoing/received delegations, effective selection weight |

### AMM Pool operations (`cmd_pool.rs`, `pool_tx.rs`, `lp_select.rs`)

| Task | Steps | Command | Inputs | Success |
|------|-------|---------|--------|---------|
| Create a pool | 1. `pool create --asset HEX --doli AMT --tokens AMT` | `cmd_pool.rs:248`, fee default 30bps, max `POOL_MAX_FEE_BPS` | asset id hex, DOLI+token amounts >0 | CreatePool tx; pool_id = `compute_pool_id(ZERO, asset_b, fee_bps)` (D2: fee_bps part of identity); refuses duplicate pool (P1-006); creator receives `lp_shares - MINIMUM_LIQUIDITY` (D1 lock, rejects if remainder ≤0, P0-005) |
| Swap through a pool | 1. `pool swap --pool HEX --amount AMT --direction a2b\|b2a` | `cmd_pool.rs:502` | pool id, amount (DOLI notation for a2b, raw units for b2a), optional `--min-out` slippage guard | Swap tx; `a2b`=DOLI→token, `b2a`=token→DOLI; constant-product `compute_swap` |
| Add liquidity | 1. `pool add --pool HEX --doli AMT --tokens AMT` | `cmd_pool.rs:867` | matching ratio (or accepts disproportional per `compute_lp_shares`) | AddLiquidity tx, new LP shares minted |
| Remove liquidity | 1. `pool remove --pool HEX --shares N` | `cmd_pool.rs:1120` | LP shares to burn; optional `--min-doli`/`--min-tokens` | RemoveLiquidity tx; LP UTXO selection restricted to target pool_id (`lp_select.rs:25`, rejects foreign-pool LP UTXOs — INC-I-095 `[MPTX007]`); signs via `pool_tx::sign_with_covenant_witnesses` |
| List / inspect pools | 1. `pool list` / `pool info <ID>` (or `--pool ID`) | `cmd_pool.rs:138,194` | pool id optional for list | RPC `getPoolList`/`getPoolInfo` (raw JSON, not typed struct) |

### Payment channels (`cmd_channel.rs`)

| Task | Steps | Command | Inputs | Success |
|------|-------|---------|--------|---------|
| Open a channel | 1. `channel open <peer> <capacity>` | `cmd_channel.rs:42` (`ChannelCommands::Open` handler at line ~71) | distinct peer address (self-channel rejected, P1-007), capacity ≥ `min_channel_capacity` | 2-of-2 funding tx broadcast; `ChannelRecord` saved to store next to wallet file |
| Pay through a channel | 1. `channel pay <id> <amount>` | line ~245 | channel must be active (auto-refreshes `FundingBroadcast`→active on-demand via confirmation check, INC-I-097) | off-chain balance update + commitment number increment |
| **Cooperatively close** a channel (2-step) | 1. opener: `channel close <id> -o offer.json` 2. counterparty: `channel close-finish offer.json` | `ChannelCommands::Close`/`CloseFinish` (`cmd_channel.rs:326,422`) | both parties' wallets (2-of-2 covenant) | step1 writes signed offer file; step2 co-signs + broadcasts close tx |
| List / inspect channels | 1. `channel list [--all]` / `channel info <id>` | line ~488,529 | channel id prefix | table of state/balances/capacity |

**DRIFT NOTE vs prior skill rev**: `channel close --force` (unilateral force-close) is **NOT implemented** — it hard-errors directing the user to the 2-step cooperative flow (`cmd_channel.rs:332-350`). Trustless force-close (pre-signed commitments/penalty/watchtower) is an unbuilt roadmap item (INC-I-093).

### Covenant templates (`cmd_template/`) — NEW since prior skill rev

| Task | Steps | Command | Inputs | Success |
|------|-------|---------|--------|---------|
| Delayed-withdrawal vault | dry-run prints condition string; `--send` broadcasts | `doli template vault --owner A --cosigner B --unlock-height H [--send --to R --amount AMT]` (`cmd_template/vault.rs:15`) | owner/cosigner addr, unlock height | condition = `vault(owner,cosigner,unlock_height)`: owner-solo after height OR 2-of-2 anytime |
| Multi-party escrow with refund | same dry-run/`--send` pattern | `doli template escrow --parties A,B,C --threshold M --timeout H --refund R [--send ...]` (`cmd_template/escrow.rs`) | comma-separated parties, m-of-n threshold, refund addr | m-of-n release OR refund after timeout |
| HTLC payment | same pattern | `doli template htlc-payment --hash HEX --lock H --expiry H --refund R [--send ...]` (`cmd_template/htlc_payment.rs`) | BLAKE3 hash, lock/expiry heights | claim w/ preimage after lock, refund after expiry |
| Subscription (bounded recurring payment) | same pattern | `doli template subscription --recipient R --amount MIN --output-index I --start H --end H [--send --send-amount ...]` (`cmd_template/subscription.rs`) | recipient must receive ≥amount at given output index within [start,end] | time-gated guard condition |
| Agent allowance (bounded delegation) | same pattern | `doli template agent-allowance --agent A --recipient R --amount MIN --output-index I [--send ...]` (`cmd_template/agent_allowance.rs`) | agent addr, recipient, min amount | agent can only pay to fixed recipient, min amount |
| Escrow-loan (OTC, guard-based) | same pattern | `doli template escrow-loan --lender L --repay-amount MIN --deadline H [--send ...]` (`cmd_template/escrow_loan.rs`) | lender addr, min repay, deadline height | lender reclaims collateral after deadline if unrepaid |

All template commands: default = **dry-run**, prints the CLI condition string only (`cmd_template/dispatch.rs:16-111`); `--send` requires `--to`+`--amount` and delegates to `cmd_wallet::cmd_send` with the built condition (`vault.rs:47` `send_with_condition`).

### NFT operations (`cmd_nft/`)

| Task | Steps | Command | Inputs | Success |
|------|-------|---------|--------|---------|
| List owned NFTs | `nft --list` | `cmd_nft/list.rs:8` | none | RPC `getUtxos` |
| Inspect an NFT | `nft --info <UTXO>` | `cmd_nft/info.rs:6` | UTXO ref | RPC `getTransaction` + archiver fallback |
| Mint an NFT | `nft --mint <CONTENT> [--condition C] [--amount A] [--royalty PCT] [--content-type MIME] [--data HEX] [--data-file PATH]` | `cmd_nft/mint.rs:12` | content string; `--data-file` reads raw bytes (main.rs:263-270) | mint tx; royalty max 25% (`MAX_ROYALTY_BPS`) |
| Batch mint | `nft --batch-mint <MANIFEST.json> [--yes]` | `cmd_nft/batch.rs` (entry struct `BatchEntry` line 12) | JSON array of `{content,data?,royalty?}` | one tx per NFT |
| Export NFT content | `nft --export <UTXO> -o FILE` | `cmd_nft/export.rs:8` | UTXO ref | extracts on-chain bytes to file (archiver fallback) |
| Transfer an NFT | `nft --transfer <UTXO> --to ADDR [--witness W]` | `cmd_nft/transfer.rs:12` | UTXO, recipient | EncryptedContent NFTs: ECIES key re-wrapped for new owner |
| Sell (unsigned offer) | `nft --sell <UTXO> --price P -o FILE` | `cmd_nft/sell.rs:29` | UTXO, price | offer JSON file, buyer needs `--seller-wallet` |
| Sell (signed PSBT offer) | `nft --sell-sign <UTXO> --price P --to BUYER -o FILE` | `cmd_nft/sell.rs` | UTXO, price, buyer addr | seller pre-signs; buyer completes w/o seller wallet |
| Buy directly | `nft --buy <UTXO> --price P --seller-wallet PATH` | `cmd_nft/buy.rs:13` | seller wallet path | atomic single tx, both sides signed |
| Buy from offer file | `nft --from <OFFER.json> [--seller-wallet PATH]` | `cmd_nft/buy.rs` | offer file (signed offers need no seller wallet) | tx broadcast |
| Fractionalize NFT into shares | `nft --fractionalize <TOKEN_ID> --shares N [--ticker T]` | `commands.rs:315` (handler in a token-related module) | token id, share count | N fungible shares (ticker default `FRAC`) |
| Redeem fractions back to NFT | `nft --redeem <TOKEN_ID>` | `commands.rs:326` | must hold all shares | burns shares, NFT returned |

### Fungible tokens, bridge, chain, governance, guardian, service (mostly unchanged behavior — verified line-stable vs prior skill rev)

| Task | Command | Location | Notes |
|------|---------|----------|-------|
| Issue a fungible token | `doli issue-token <TICKER> --supply N [--condition C]` | `cmd_token.rs:10` | ticker ≤16 chars, fixed supply at issuance |
| Inspect a token UTXO | `doli token-info <UTXO>` | `cmd_token.rs` | archiver fallback |
| Initiate cross-chain swap | `doli bridge-swap <AMOUNT> --chain C --to ADDR [...]` | `cmd_bridge.rs` | chains: bitcoin/ethereum/monero/litecoin/cardano/bsc |
| Check/auto-resolve swap | `doli bridge-status <SWAP_ID> [--auto]` | `cmd_bridge.rs` | on-chain state read, no watcher required |
| Buy into a swap | `doli bridge-buy <SWAP_ID> [--preimage HEX] [...]` | `cmd_bridge.rs` | counterparty side, can auto-detect preimage |
| Run bridge watcher | `doli bridge-watch [--interval SECS]` | `cmd_bridge.rs` | daemon: auto-claim/refund |
| List/lock/claim/refund HTLCs | `doli bridge-list/-lock/-claim/-refund` | `cmd_bridge.rs` | manual HTLC ops, advanced |
| Show chain info | `doli chain` | `cmd_chain.rs:8` | RPC `getChainInfo` |
| Verify chain integrity | `doli chain-verify` | `cmd_chain.rs:31` | BLAKE3 running commitment, all blocks 1..tip |
| Epoch/reward info | `doli rewards info` (list/claim/claim-all/history are informational-only) | `cmd_chain.rs:74,111` | rewards are automatic via coinbase; no claim tx needed |
| Wipe chain data (resync) | `doli wipe [--network N] [--yes]` | `cmd_chain.rs:157` | preserves `WIPE_PRESERVE` list |
| Fast-sync (snap) | `doli snap [--seed URL]... [--trust] [--no-restart]` | `cmd_snap.rs:23` | 2-of-N seed state-root consensus (or `--trust` single) |
| Check/vote on updates | `doli update check/status/vote/votes/apply/rollback` | `cmd_governance.rs` | veto threshold 40% |
| List maintainers | `doli maintainer list` | `cmd_governance.rs:` | first 5 registered producers, 3/5 threshold |
| Sign a release / protocol activation | `doli release sign` / `doli protocol sign` / `doli protocol activate` | `cmd_governance.rs:9,` | maintainer workflow, 3+ signatures required for activate |
| Guardian: status/halt/resume/checkpoint | `doli guardian status/halt/resume/checkpoint` | `cmd_guardian.rs:13-` | wraps `getGuardianStatus`/`pauseProduction`/`resumeProduction`/`createCheckpoint` |
| Guardian: fork monitor | `doli guardian monitor --endpoint URL... [--loop SECS]` | `cmd_guardian.rs` (`GuardianCommands::Monitor`, `endpoints: Vec<String>` required, `commands.rs:1022`) | groups by `best_hash`; reports FORK DETECTED if >1 distinct tip |
| Manage node service | `doli service install/uninstall/start/stop/restart/status/logs` | `cmd_service.rs` | systemd (Linux, needs root) / launchd (macOS, user-scoped) |
| Self-upgrade binaries | `doli upgrade [--version V] [--service NAME]` | `cmd_upgrade.rs` | verifies CHECKSUMS.txt against 3/5 maintainer signatures |

---

## DATA-FLOW

**Transaction submission (generic path)**: RPC `getUtxos` (filter by `output_type`/`spendable`) → build `doli_core::{Input,Output,Transaction}` → `tx.signing_message_for_input(i)` per-input BIP-143-style hash → `signature::sign_hash` with wallet keypair → for conditioned inputs, `tx.set_covenant_witnesses(&witnesses)` (SegWit-style, evaluated via `Witness::decode`, NOT `input.signature`) → serialize → hex → RPC `sendTransaction`.

**Pool tx signing (covenant-aware, INC-I-095)**: `pool_tx::sign_with_covenant_witnesses(tx, keypair, conditioned: &[bool])` (`pool_tx.rs:15`) signs every input AND attaches a real `Witness` for `conditioned[i]==true` (LPShare/FungibleAsset inputs), empty witness otherwise (Pool input is signature-exempt under RC-A). Omitting the witness for a conditioned input → `[MPTX007]` mempool rejection.

**LP share selection (INC-I-095)**: `lp_select::select_lp_share_utxos` (`lp_select.rs:25`) filters wallet's `lpShare` UTXOs by embedded `pool_id` before spending — a wallet holding shares from multiple pools must never mix them into one `pool remove` tx.

**Balance display** (`cmd_wallet.rs:137`): `getUtxos` → spendable normal-output sum; `getProducers` → bond_amount (status=active → "Bonded", status=pending → "Activating"); coinbase/reward `immature` and mempool `unconfirmed` from `getBalance`.

**Bond lifecycle in CLI**: `producer register --bonds N` → Registration tx → epoch boundary → active in ProducerSet. `producer add-bond --count N` → AddBond tx → epoch boundary. `producer request-withdrawal --count N` (FIFO oldest-first, `compute_fifo_breakdown` in `cmd_producer/common.rs`) → funds returned immediately in the withdrawal tx payout, ProducerSet bond removal is epoch-deferred. `producer exit --force` == request-withdrawal for ALL available bonds.

**Channel funding→close flow**: `channel open` builds+broadcasts a 2-of-2 funding tx, records `ChannelRecord` (`ChannelState::FundingBroadcast`) in a local JSON store next to the wallet file. `channel pay` refreshes state to active on first use if confirmations suffice (INC-I-097), updates off-chain balances + commitment number. `channel close` builds a signed cooperative-close offer file (own signature only); `channel close-finish` (counterparty) verifies + co-signs + broadcasts.

**NFT sell/buy flow**: Unsigned offer (`nft --sell`) → buyer needs `--seller-wallet` for `--buy`/`--from`. Signed PSBT (`nft --sell-sign`) pre-signs seller's NFT input → buyer completes via `--from offer.json` with no seller wallet needed. EncryptedContent NFTs re-wrap the ECIES content key for the new owner (`cmd_nft/mod.rs:46 build_ec_output_for_buyer`) — unwraps with seller's private key (zeroized after use, AUDIT-CRYPTO-001), re-wraps with buyer's pubkey.

**Snap sync flow** (`cmd_snap.rs`): query `getStateRootDebug` on 2+ seeds (or 1 with `--trust`) → consensus check → stop service (unless `--no-restart`) → wipe (preserve `keys/`, `.env`, `node_key`, `wallet.json`, `wallet.seed.txt`, `config.toml`) → download `getStateSnapshot` → recompute state root locally to verify integrity → write to `state_db/` → restart service.

**Archiver fallback** (`rpc_client.rs:507`): activated for `getTransaction`/`getHistory` only, on "not found"/`-32001` errors; retries each configured archiver endpoint in order.

---

## DEPENDENCIES

**Crates used (imports, not `doli_core` at compile-boundary — CLI links `doli_core` directly for tx building, but wallet crypto flows through `crypto` crate; also links `channels` crate for payment-channel logic):**
- `doli_core` — `Transaction`, `Input`, `Output`, `OutputType`, `Condition`, `TxType`, consensus constants (`BASE_FEE`, `FEE_PER_BYTE`, `FEE_DIVISOR`, `MINIMUM_LIQUIDITY`, `MAX_BONDS_PER_PRODUCER`, `POOL_MAX_FEE_BPS`, `MAX_ROYALTY_BPS`, `COINBASE_MATURITY`)
- `crypto` — `KeyPair`, `PublicKey`, `Signature`, `Hash`, `address::{resolve,encode,from_pubkey}`, `signature::sign_hash`, `bls_sign_pop`, `BlsKeyPair`, `encrypted_content::{wrap_key,unwrap_key}`, `hash::hash_with_domain`
- `channels` — `close::{build_cooperative_close_offer, finalize_cooperative_close_offer}`, `commitment::derive_channel_seed`, `config::ChannelConfig`, `funding::build_funding_tx_with_change`, `store::ChannelStore`, `types::{ChannelId,ChannelState,ChannelBalance,FundingOutpoint}`, `rpc::RpcClient` (used by `cmd_channel.rs` only)
- `vdf` — registration hash-chain (`T_REGISTER_BASE`) used in `cmd_producer/register.rs`
- `updater` — `fetch_github_release`, `download_checksums_txt`, `sign_release_hash`, `current_version` (used by `cmd_upgrade.rs`, `cmd_governance.rs` release/protocol)
- `bip39` — mnemonic generation/recovery (`wallet.rs`)
- `reqwest`, `tokio`, `clap`, `bincode`, `hex`, `zeroize`, `chrono`

**Key RPC methods called** (see `.claude/skills/doli-network/SKILL.md` for full RPC reference): `getBalance`, `getUtxos`/`getUtxosJson`, `sendTransaction`, `getChainInfo`, `verifyChainIntegrity`, `getNetworkParams`, `getTransaction`, `getHistory`, `getProducer`, `getProducers`, `getBondDetails`, `getEpochInfo`, `getBlockByHash`, `getUpdateStatus`, `getNodeInfo`, `submitVote`, `getMaintainerSet`, `getGuardianStatus`, `pauseProduction`, `resumeProduction`, `createCheckpoint`, `getPoolList`, `getPoolInfo` (raw JSON via `call_raw`, not typed), `getStateRootDebug`, `getStateSnapshot`.

**Used by**: nothing internal to the repo imports `bins/cli` as a library except its own test binary; this is a standalone binary crate. External consumers: end users, `scripts/*.sh` (testnet/devnet automation), systemd/launchd units installed by `service install`.

---

## CONSTRAINTS

**Security (enforced in code):**
- `send`/`spend` reject raw 64-hex addresses — bech32 only (`cmd_wallet.rs:346`; INC/bug: 32 DOLI burned 2026-03-22 from pubkey/pubkey_hash ambiguity)
- `nft --buy` validates buyer pubkey is a valid Ed25519 curve point before ECIES (AUDIT-CRYPTO-002, `cmd_nft/mod.rs:37`)
- EncryptedContent `extra_data` bounds-checked before parsing (AUDIT-CRYPTO-010, `cmd_nft/mod.rs:70`)
- Content key zeroized after ECIES re-wrap (AUDIT-CRYPTO-001, `cmd_nft/mod.rs:97`)
- `producer delegate` rejects self-delegation; `channel open` rejects self-channel (P1-007, `cmd_channel.rs:32`)
- `producer register` rejects already-active/pending producers
- Guard conditions (`amount_guard`/`output_type_guard`/`recipient_guard`) on mainnet print a warning but do not block — `guards_activation_height = MAX` so the tx WILL be rejected by mainnet nodes today (`cmd_wallet.rs:1012`)
- `pool create` rejects duplicate pools before broadcast (P1-006) and rejects a creator LP-share remainder ≤0 (P0-005, `MINIMUM_LIQUIDITY` lock)
- `pool remove` restricts LP UTXO selection to the target pool_id, never spends foreign-pool LP shares (INC-I-095)

**Bond/delegation constraints:**
- Registration / AddBond: 1-10000 bonds
- RequestWithdrawal: ≤ available bonds (total − withdrawal_pending)
- Delegate: 1-`MAX_BONDS_PER_PRODUCER` (consensus constant, `doli_core::consensus`)
- Protocol activation requires exactly 3+ maintainer signatures

**Epoch-deferred operations** (effective at NEXT epoch boundary): `producer register`, `add-bond`, `request-withdrawal`, `exit`, `delegate`, `revoke-delegation`.

**Fee policy:** `send` auto-fee = node's size-scaled `Transaction::minimum_fee()` (INC-I-099, depends on output `extra_data` length — covenant conditions cost more). `spend` (legacy single-output and multi-output) defaults to flat 1 base-unit fee unless `--fee` given. Registration/pool/bond txs use `BASE_FEE + extra_bytes * FEE_PER_BYTE / FEE_DIVISOR`.

**Spend multi-output cap:** `MAX_SPEND_OUTPUTS = 8` (`cmd_wallet.rs:871`); output indices must be contiguous from 0, no gaps/duplicates (`validate_output_specs`); protocol-internal output types (`bond, bridgehtlc, pool, lpshare, zkrollup, encryptedcontent, fungibleasset`) cannot be constructed via `spend --output`.

**Channel force-close:** NOT implemented in this build — `--force` on `channel close` always errors. Only 2-step cooperative close (`close` + `close-finish`) is supported.

**WIPE_PRESERVE list** (`cmd_chain.rs`): `keys/`, `.env`, `wallet.json`, `wallet.seed.txt`, `node_key`, `config.toml` — never deleted by `wipe` or `snap`.

**Snap sync consensus:** requires 2+ seeds to agree on state root; `--trust` allows a single seed (local/dev only).

**Service command:** not supported on Windows; root/sudo required on Linux; macOS uses user-scoped launchd (no sudo). Linux group re-exec: if user is in `doli` group but session hasn't picked it up, CLI re-execs via `sg doli -c "..."` (guarded by `DOLI_SG_REEXEC` env, `main.rs:44-90`).

**Loan/Oracle:** No `doli loan *` commands exist (lending tombstoned, B.1). No `doli oracle *` commands exist (Phase 2.1 frozen pre-activation, RPC-only).

---

## PATTERNS

**Template dry-run/send duality** (all 6 `cmd_template/*`): default prints the CLI condition string only; `--send --to R --amount A` builds+broadcasts via `cmd_wallet::cmd_send` internally, inheriting its mainnet-guard warnings (`cmd_template/vault.rs:47`).

**Covenant witness assembly for conditioned inputs**: any tx spending a conditioned output (LPShare, FungibleAsset, NFT, etc.) must call `tx.set_covenant_witnesses(&witnesses)` with one entry per input — empty `Vec::new()` for unconditioned/signature-only inputs, a real `Witness::encode()` blob for conditioned ones. See `pool_tx::sign_with_covenant_witnesses` for the canonical helper; hand-rolled versions exist in `cmd_pool.rs` (create/swap/add) predating the helper.

**Full producer setup from scratch:**
```
doli init                         # wallet + BLS key
doli balance                      # verify funded
doli producer register --bonds 1  # VDF ~5s
doli producer status              # "pending" until epoch boundary
```

**Cooperative channel close (2-step, no force-close):**
```
# opener
doli channel close <id> -o close.json
# counterparty
doli channel close-finish close.json
```

**Sell NFT via signed PSBT (buyer needs no seller wallet):**
```
doli nft --sell-sign <UTXO> --price 100 --to <BUYER_ADDR> -o offer.json
doli nft --from offer.json
```

**Withdraw all bonds (graceful exit):**
```
doli producer simulate-withdrawal --count N   # preview
doli producer request-withdrawal --count N    # submit
```

**Create + swap in an AMM pool:**
```
doli pool create --asset <HEX> --doli 1000 --tokens 500000 --fee 30
doli pool swap --pool <HEX> --amount 10 --direction a2b --min-out 4900
```

**Check fork across nodes:**
```
doli guardian monitor --endpoint http://n1:8500 --endpoint http://n2:8500 --loop 30
```

**Network-specific invocations:**
```
doli -n testnet balance
doli -n devnet producer status
DOLI_NETWORK=testnet doli balance
```

**RPC archiver fallback:** `getTransaction`/`getHistory` auto-retry against `seed1/seed2/seeds.doli.network:8500` (mainnet) or `seeds.testnet.doli.network:18500` (testnet) on "not found" (post-snap-sync common case).

**UTXO reference format:** always `txhash:output_index` (e.g. `abc123def456:0`).

**Condition grammar composability:** boolean composition (`and`/`or`/`threshold`) uses `split_top_level` for comma-splitting so nested sub-conditions parse correctly — do not flat-split `args_str` on `,` for these three (`parsers.rs:65`, S1 CRITICAL comment).
