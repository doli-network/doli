---
name = "cli"
description = "DOLI CLI complete reference — all commands, subcommands, flags, and examples. DO NOT read the whole file. Grep for the command/topic you need."
trigger = "CLI|doli command|how to send|how to bond|producer command|nft command|bridge command|pool command|loan command|channel command|guardian command|service command|snap|wipe|chain-verify|rewards command|add-bond|register producer|slash|withdrawal"
---

# DOLI CLI Reference

> **DO NOT read this entire file.** Grep for the command/topic you need.
> Source of truth: `bins/cli/src/commands.rs`. If this file drifts, the code wins.

## Global Options

```
doli [OPTIONS] <COMMAND>
  -w, --wallet <PATH>      Wallet file (default: auto-detected from network)
  -r, --rpc <URL>          Node RPC endpoint (auto-detected from --network if not set) [env: DOLI_RPC_URL]
  -n, --network <NET>      Network: mainnet|testnet|devnet (default: mainnet) [env: DOLI_NETWORK]
```

## INDEX — Top-Level Commands

| Line | Command | Description |
|------|---------|-------------|
| 40 | init | Initialize producer wallet (new + add-bls) |
| 55 | new | Create a new wallet |
| 60 | restore | Restore wallet from 24-word seed |
| 65 | address | Generate new address (MUTATING — do NOT use to inspect) |
| 75 | addresses | List all addresses (read-only) |
| 80 | balance | Show wallet balance (read-only) |
| 95 | send | Send coins (optional covenant condition) |
| 115 | spend | Spend a covenant-conditioned UTXO |
| 135 | history | Show transaction history |
| 145 | export | Export wallet to file |
| 150 | import | Import wallet from file |
| 155 | info | Show wallet info |
| 160 | add-bls | Add BLS attestation key to wallet |
| 165 | sign | Sign a message |
| 175 | verify | Verify a signature |
| 185 | producer | Producer subcommands (register, bonds, exit, slash, delegate) |
| 225 | rewards | Rewards subcommands (list, claim, claim-all, history, info) |
| 245 | chain | Show chain information |
| 250 | chain-verify | Verify chain integrity + compute chain commitment |
| 260 | update | Update governance (check, status, vote, apply, rollback) |
| 280 | maintainer | Maintainer governance (list) |
| 290 | upgrade | Upgrade doli binaries to latest release |
| 305 | release | Release management (sign) |
| 315 | protocol | Protocol activation (sign, activate) |
| 330 | nft | NFT operations (mint, transfer, buy, sell, list, info, fractionalize) |
| 400 | issue-token | Issue a fungible token |
| 415 | token-info | Show token info from a UTXO |
| 425 | bridge-swap | Initiate cross-chain atomic swap |
| 445 | bridge-status | Check swap status |
| 465 | bridge-buy | Buy into existing swap |
| 490 | bridge-watch | Run bridge watcher daemon |
| 505 | bridge-list | List active bridge swaps |
| 515 | bridge-lock | Lock DOLI in bridge HTLC (advanced) |
| 560 | bridge-claim | Claim bridge HTLC with preimage |
| 575 | bridge-refund | Refund bridge HTLC after expiry |
| 585 | pool | AMM pool operations (create, swap, add, remove, list, info) |
| 630 | loan | Lending operations (deposit, withdraw, create, repay, liquidate, list, info) |
| 680 | channel | Payment channels (open, pay, close, list, info) |
| 720 | service | Manage doli-node systemd/launchd service |
| 755 | guardian | Seed Guardian (status, halt, resume, checkpoint, monitor) |
| 790 | snap | Fast-sync: wipe + download verified state snapshot |
| 810 | wipe | Wipe chain data for fresh resync (preserves keys/ and .env) |

---

## init
```
doli init [--force] [--non-producer]
```
Initialize a new producer wallet (combines `new` + `add-bls`).
- `--force` — overwrite existing wallet (DANGEROUS)
- `--non-producer` — skip BLS key generation

## new
```
doli new [--name <NAME>]
```

## restore
```
doli restore [--name <NAME>]
```
Restore wallet from 24-word seed phrase.

## address (MUTATING)
```
doli address [--label <LABEL>]
```
**WARNING**: Generates a new random (non-HD) address and saves to wallet. To inspect addresses, use `balance` or `addresses`.

## addresses
```
doli addresses
```
List all addresses in wallet (read-only).

## balance
```
doli balance [-A <ADDRESS>] [--all]
```
- `-A, --address` — show balance for specific address only
- `--all` — show per-address breakdown

## send
```
doli send <TO> <AMOUNT> [--fee <FEE>] [--condition <COND>] [--yes]
```
Conditions: `multisig(2, a1, a2, a3)`, `hashlock(hex)`, `htlc(hex, lock_h, expiry_h)`, `timelock(min_h)`, `vesting(addr, unlock_h)`

## spend
```
doli spend <UTXO> <TO> <AMOUNT> --witness <WITNESS> [--fee <FEE>]
```
UTXO format: `txhash:output_index`. Witness: `preimage(hex)`, `sign(w1.json, w2.json)`, `branch(right, preimage(hex))`

## history
```
doli history [--limit <N>]        # default: 10
```

## export / import
```
doli export <OUTPUT_PATH>
doli import <INPUT_PATH>
```

## info
```
doli info
```

## add-bls
```
doli add-bls
```
Add BLS attestation key to an existing wallet.

## sign / verify
```
doli sign <MESSAGE> [--address <ADDR>]
doli verify <MESSAGE> <SIGNATURE_HEX> <PUBKEY>
```

---

## producer register
```
doli producer register [--bonds <N>]    # default: 1, range: 1-10000
```

## producer status
```
doli producer status [--pubkey <HEX>]
```

## producer bonds
```
doli producer bonds [--pubkey <HEX>]
```
Show per-bond vesting details.

## producer list
```
doli producer list [--active] [--format table|json|csv]
```

## producer add-bond
```
doli producer add-bond --count <N>      # range: 1-10000
```

## producer request-withdrawal
```
doli producer request-withdrawal --count <N> [--destination <HEX>]
```
FIFO order, vesting penalty applies.

## producer simulate-withdrawal
```
doli producer simulate-withdrawal --count <N>
```
Dry run — no transaction submitted.

## producer exit
```
doli producer exit [--force]
```
`--force` for early exit with penalty.

## producer slash
```
doli producer slash --block1 <HASH> --block2 <HASH>
```
Submit equivocation evidence (two blocks for same slot).

## producer delegate
```
doli producer delegate <DELEGATEE_PUBKEY> --bonds <N>   # range: 1-100
```
Delegate bond weight to another producer. Epoch-deferred — takes effect at next epoch boundary.
Reward split: delegatee keeps 10%, delegator receives 90%.

## producer revoke-delegation
```
doli producer revoke-delegation
```
Revoke active delegation. Epoch-deferred. DELEGATION_UNBONDING_SLOTS delay (~7 days) applies after revocation.

## producer delegation-status
```
doli producer delegation-status [--address <PUBKEY>]
```
Show delegation state: outgoing delegation, received delegations, effective selection weight.

---

## rewards list
```
doli rewards list
```
List all claimable epochs with estimated rewards.

## rewards claim
```
doli rewards claim <EPOCH> [--recipient <ADDR>]
```

## rewards claim-all
```
doli rewards claim-all [--recipient <ADDR>]
```

## rewards history
```
doli rewards history [--limit <N>]      # default: 20
```

## rewards info
```
doli rewards info
```
Show current epoch info and BLOCKS_PER_REWARD_EPOCH.

---

## update check / status / apply / rollback
```
doli update check
doli update status
doli update vote --version <VER> (--veto | --approve)
doli update votes --version <VER>
doli update apply
doli update rollback
```

## maintainer list
```
doli maintainer list
```

## upgrade
```
doli upgrade [--version <VER>] [--yes] [--doli-node-path <PATH>] [--service <NAME>]
```

## release sign
```
doli release sign --version <VER> [--key <PATH>]
```
Signs `{version}:{sha256(CHECKSUMS.txt)}` with producer key.

## protocol sign
```
doli protocol sign --version <N> --epoch <N> [--key <PATH>]
```

## protocol activate
```
doli protocol activate --version <N> --epoch <N> --description <TEXT> --signatures <FILE>
```
Requires 3/5 maintainer signatures.

---

## nft
```
doli nft --list                                          # list owned NFTs
doli nft --info <UTXO>                                   # show NFT info
doli nft --mint <CONTENT> [--condition <C>] [--amount <A>] [--royalty <PCT>] [--data <HEX>] [--data-file <PATH>]
doli nft --transfer <UTXO> --to <ADDR> [--witness <W>]
doli nft --sell <UTXO> --price <DOLI> -o <FILE>          # unsigned offer
doli nft --sell-sign <UTXO> --price <DOLI> --to <ADDR> -o <FILE>  # signed PSBT offer
doli nft --buy <UTXO> --price <DOLI> --seller-wallet <PATH>
doli nft --from <OFFER_FILE>                             # buy from offer
doli nft --export <UTXO>                                 # extract content
doli nft --batch-mint <MANIFEST_FILE> [--yes]
doli nft --fractionalize <TOKEN_ID> --shares <N> [--ticker <NAME>]
doli nft --redeem <TOKEN_ID>
```

## issue-token
```
doli issue-token <TICKER> --supply <N> [--condition <C>]
```

## token-info
```
doli token-info <UTXO>
```

---

## bridge-swap
```
doli bridge-swap <AMOUNT> --chain <CHAIN> --to <ADDR> [--counter-rpc <URL>] [--confirmations <N>]
```
Chains: bitcoin, ethereum, monero, litecoin, cardano, bsc

## bridge-status
```
doli bridge-status <SWAP_ID> [--btc-rpc <URL>] [--eth-rpc <URL>] [--auto]
```

## bridge-buy
```
doli bridge-buy <SWAP_ID> [--preimage <HEX>] [--btc-rpc <URL>] [--eth-rpc <URL>] [--to <ADDR>] [--yes]
```

## bridge-watch
```
doli bridge-watch [--btc-rpc <URL>] [--eth-rpc <URL>] [--interval <SECS>]   # default: 10s
```

## bridge-list
```
doli bridge-list [--chain <CHAIN>] [--blocks <N>]       # default: 100 blocks
```

## bridge-lock (advanced)
```
doli bridge-lock <AMOUNT> (--hash <HEX> | --preimage <HEX>) --lock <H> --expiry <H> --chain <CHAIN> --to <ADDR> --counter-hash <HEX> [--multisig-threshold <N> --multisig-keys <ADDRS>] [--yes]
```

## bridge-claim
```
doli bridge-claim <UTXO> --preimage <HEX> [--to <ADDR>] [--yes]
```

## bridge-refund
```
doli bridge-refund <UTXO> [--yes]
```

---

## pool create
```
doli pool create --asset <HEX> --doli <AMOUNT> --tokens <AMOUNT> [--fee <BPS>] [--yes]
```
Fee default: 30 bps (0.3%).

## pool swap
```
doli pool swap --pool <HEX> --amount <AMOUNT> --direction a2b|b2a [--min-out <AMOUNT>] [--yes]
```

## pool add
```
doli pool add --pool <HEX> --doli <AMOUNT> --tokens <AMOUNT> [--yes]
```

## pool remove
```
doli pool remove --pool <HEX> --shares <AMOUNT> [--min-doli <AMOUNT>] [--min-tokens <AMOUNT>] [--yes]
```

## pool list / info
```
doli pool list
doli pool info <POOL_ID>
```

---

## loan deposit
```
doli loan deposit --pool <HEX> --amount <AMOUNT> [--yes]
```

## loan withdraw
```
doli loan withdraw <DEPOSIT_UTXO> [--yes]
```

## loan create
```
doli loan create --pool <HEX> --collateral <AMOUNT> --borrow <AMOUNT> [--interest-rate <BPS>] [--yes]
```
Default interest: 500 bps (5%).

## loan repay
```
doli loan repay <LOAN_UTXO> [--yes]
```

## loan liquidate
```
doli loan liquidate <LOAN_UTXO> [--yes]
```

## loan list / info
```
doli loan list [--borrower <HEX>]
doli loan info <LOAN_UTXO>
```

---

## channel open
```
doli channel open <PEER_ADDR> <CAPACITY> [--fee <FEE>]
```

## channel pay
```
doli channel pay <CHANNEL_ID> <AMOUNT>
```

## channel close
```
doli channel close <CHANNEL_ID> [--fee <FEE>] [--force]
```

## channel list / info
```
doli channel list [--all]
doli channel info <CHANNEL_ID>
```

---

## service install
```
doli service install [--network <NET>] [--name <NAME>] [--data-dir <PATH>] [--producer-key <PATH>] [--p2p-port <PORT>] [--rpc-port <PORT>]
```

## service uninstall / start / stop / restart / status
```
doli service uninstall [--name <NAME>]
doli service start [--name <NAME>]
doli service stop [--name <NAME>]
doli service restart [--name <NAME>]
doli service status [--name <NAME>]
```

## service logs
```
doli service logs [--name <NAME>] [--follow] [-n <LINES>]   # default: 50 lines
```

---

## guardian status
```
doli guardian status
```

## guardian halt
```
doli guardian halt [--yes]
```

## guardian resume
```
doli guardian resume
```

## guardian checkpoint
```
doli guardian checkpoint
```

## guardian monitor
```
doli guardian monitor --endpoint <URL> [--endpoint <URL>...] [--loop <SECS>]
```

---

## snap
```
doli snap [--data-dir <PATH>] [--seed <URL>...] [--no-restart] [--trust]
```
Fast-sync: wipes chain data and downloads verified state snapshot.

## wipe
```
doli wipe [--network <NET>] [--data-dir <PATH>] [--yes]
```
Wipes chain data for fresh resync. Preserves `keys/` and `.env`.

---

## chain / chain-verify
```
doli chain
doli chain-verify
```
`chain-verify` scans all blocks and computes `commitment[N] = BLAKE3(commitment[N-1] || block_hash[N])`.

---

## doli-node Global Options

```
doli-node [OPTIONS] <COMMAND>
  -n, --network <NET>        Network: mainnet|testnet|devnet (default: mainnet)
  -c, --config <PATH>        Configuration file (default: config.toml)
  -d, --data-dir <PATH>      Data directory (overrides network default)
      --log-level <LEVEL>    Log verbosity: trace|debug|info|warn|error (default: warn)
```

**Log level usage:**
- Default (`warn`) — quiet, only warnings and errors
- `--log-level info` — full operational logging (block production, sync, gossip)
- `--log-level debug` — development diagnostics
- `--log-level trace` — maximum verbosity (very noisy)
