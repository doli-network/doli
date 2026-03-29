# cli.md - DOLI Command Line Interface Reference

Complete reference for all DOLI CLI operations. Every operation described in the [WHITEPAPER.md](/WHITEPAPER.md) can be performed via CLI.

DOLI provides two binaries:
- **doli**: Wallet operations, transactions, producer management
- **doli-node**: Node operations, updates, maintainer management

---

## Running Commands

All commands must be run inside the Nix development environment:

```bash
# Enter the Nix shell first
nix --extra-experimental-features "nix-command flakes" develop

# Then run commands
cargo run -p doli -- <command>
cargo run -p doli-node -- <command>

# Or use the built binaries
doli <command>
doli-node <command>
```

---

## Quick Start

```bash
# Create a wallet (displays 24-word recovery phrase + saves to .seed.txt)
doli new

# Generate an address
doli address

# Check balance
doli balance

# Check someone else's balance (note: -w is still required)
doli -w ~/.doli/wallet.json balance --address doli1abc...

# Send coins (prompts for confirmation; use --yes to skip)
doli send <recipient> <amount>

# Become a block producer
doli producer register --bonds 1
```

---

## Global Options

All commands support these options:

| Option | Description | Default | Env Var |
|--------|-------------|---------|---------|
| `-w, --wallet <PATH>` | Wallet file path (auto-detected from network if not set) | Auto-detected | `DOLI_WALLET_FILE` |
| `-r, --rpc <URL>` | Node RPC endpoint (auto-detected from --network if not set) | Auto-detected | `DOLI_RPC_URL` |
| `-n, --network <NETWORK>` | Network: mainnet, testnet, devnet | `mainnet` | `DOLI_NETWORK` |

> **IMPORTANT:** The `-w`, `-r`, and `-n` flags must be passed **before** the subcommand (e.g., `doli -w wallet.json balance`, NOT `doli balance -w wallet.json`). If the wallet file cannot be auto-detected, you **must** specify `-w` explicitly or the CLI will fail.

**Network-specific RPC ports:**

| Network | RPC Port | Example |
|---------|----------|---------|
| Mainnet | 8500 | `--rpc http://127.0.0.1:8500` |
| Testnet | 18500 | `--rpc http://127.0.0.1:18500` |
| Devnet | 28500 | `--rpc http://127.0.0.1:28500` |

---

## 1. Wallet Management

### 1.1. Create New Wallet

Create a new wallet file with a BIP-39 seed phrase and derived Ed25519 keypair.

New wallets (version 2) generate a 24-word recovery phrase. The primary key is deterministically derived from this phrase. Write down the 24 words — they are your backup. If you lose the wallet file, you can recover using these words.

```bash
doli new [OPTIONS]

Options:
  -n, --name <NAME>    Wallet name
```

**Example:**
```bash
doli new --name my_wallet
```

**Output:**
```
  Your wallet has been created.

  Recovery phrase:

     1. abandon   2. ability   3. able      4. about    5. above    6. absent
     7. absorb    8. abstract  9. absurd   10. abuse   11. access  12. accident
    13. account  14. accuse   15. achieve  16. acid    17. across  18. act
    19. action   20. actor    21. actress  22. actual  23. adapt   24. add

  Address: doli1qpzry9x8gf2tvdw0s3jn54khce6mua7l...

  Wallet saved to: "/home/user/.doli/wallet.json"
  Seed phrase saved to: "/home/user/.doli/wallet.seed.txt"

  Write down the 24 words above, then delete the seed file:
    rm "/home/user/.doli/wallet.seed.txt"
```

The seed phrase is written to a separate `.seed.txt` file and is **not stored in the wallet JSON**. Write down the 24 words on paper, then delete the seed file. If you lose both the wallet file and the seed words, your funds are unrecoverable.

Legacy wallets (version 1, e.g. existing producer keys) continue to work unchanged.

---

### 1.2. Initialize Producer Wallet

Combines `new` + `add-bls` into one step. Creates a new wallet and generates a BLS attestation key.

```bash
doli init [OPTIONS]

Options:
      --force            Overwrite existing wallet (DANGEROUS: destroys existing keys)
      --non-producer     Skip BLS key generation (non-producer wallet)
```

**Example:**
```bash
# Initialize a producer wallet
doli init

# Initialize a non-producer wallet (no BLS key)
doli init --non-producer
```

---

### 1.3. Restore Wallet

Restore a wallet from a 24-word BIP-39 seed phrase.

```bash
doli restore [OPTIONS]

Options:
  -n, --name <NAME>    Wallet name
```

**Example:**
```bash
doli restore
# Follow the interactive prompt to enter your 24-word seed phrase
```

---

### 1.4. Add BLS Key

Add a BLS attestation key to an existing wallet. Required for producers to sign attestations and qualify for epoch rewards.

```bash
doli add-bls
```

---

### 1.5. Generate New Address

Generate a new address in the wallet. Note: additional addresses are random keypairs, not derived from the seed phrase.

```bash
doli address [OPTIONS]

Options:
  -l, --label <LABEL>    Label for the address
```

**Example:**
```bash
doli address --label "savings"
```

---

### 1.6. List Addresses

Display all addresses in the wallet.

```bash
doli addresses
```

---

### 1.7. Show Wallet Info

Display wallet metadata and summary.

```bash
doli info
```

---

### 1.8. Export Wallet

Export wallet to a file (backup).

```bash
doli export <OUTPUT>

Arguments:
  <OUTPUT>    Output file path
```

**Example:**
```bash
doli export ~/backup/wallet-backup.json
```

---

### 1.9. Import Wallet

Import wallet from a file.

```bash
doli import <INPUT>

Arguments:
  <INPUT>    Input file path
```

**Example:**
```bash
doli import ~/backup/wallet-backup.json
```

---

## 2. Balance & Transactions

### 2.1. Check Balance

Show wallet balance for all addresses or a specific address.

```bash
doli balance [OPTIONS]

Options:
  -A, --address <ADDRESS>    Show balance for a specific address only
      --all                  Show per-address breakdown (all addresses in wallet)
```

**Example:**
```bash
# Aggregate balance
doli balance

# Specific address
doli balance -A a1b2c3d4e5f6...

# Per-address breakdown
doli balance --all
```

**Output:**
```
Balances:
------------------------------------------------------------
a1b2c3d4e5f6... (primary)
  Pubkey Hash: a1b2c3d4e5f6...
  Confirmed:   100.00000000 DOLI
  Unconfirmed: 0.00000000 DOLI
  Immature:    50.00000000 DOLI
  Total:       150.00000000 DOLI
```

**Balance Types:**
| Type | Description |
|------|-------------|
| Confirmed | Spendable balance (mature UTXOs, minus mempool-spent) |
| Unconfirmed | Pending credits from mempool (incoming change outputs) |
| Immature | Coinbase/epoch rewards pending 6-block maturity |
| Bonded | Balance locked in Bond UTXOs |
| Total | confirmed + unconfirmed + immature + bonded |

---

### 2.2. Send Coins

Transfer coins to another address, optionally with a covenant condition on the output.

```bash
doli send [OPTIONS] <TO> <AMOUNT>

Arguments:
  <TO>        Recipient address (bech32m or 64-char hex)
  <AMOUNT>    Amount to send in DOLI

Options:
  -f, --fee <FEE>            Transaction fee (default: auto-calculated)
  -c, --condition <COND>     Covenant condition on the output (see below)
      --yes                  Skip confirmation prompt
```

**Condition examples:**
- `multisig(2, addr1, addr2, addr3)` -- 2-of-3 multisig
- `hashlock(hex_hash)` -- hash-locked output
- `htlc(hex_hash, lock_height, expiry_height)` -- hash time-locked contract
- `timelock(min_height)` -- time-locked output
- `vesting(addr, unlock_height)` -- vesting schedule

**Example:**
```bash
# Send 10 DOLI
doli -w ~/.doli/wallet.json send doli1abc... 10

# Send with explicit fee
doli -w ~/.doli/wallet.json send doli1abc... 10 --fee 0.001

# Send with hashlock condition
doli send doli1abc... 10 --condition "hashlock(abcd1234...)"

# Send without confirmation prompt
doli send doli1abc... 10 --yes
```

**WHITEPAPER Reference:** Section 2 (Transactions) - Transactions require valid inputs, signatures, and positive amounts.

---

### 2.3. Spend Conditioned UTXO

Spend a covenant-conditioned UTXO by providing the witness data that satisfies the condition.

```bash
doli spend <UTXO> <TO> <AMOUNT> [OPTIONS]

Arguments:
  <UTXO>      UTXO to spend: txhash:output_index
  <TO>        Recipient address
  <AMOUNT>    Amount to send (remaining goes to change)

Options:
  -w, --witness <WITNESS>    Witness data to satisfy the condition (required)
  -f, --fee <FEE>            Fee (default: auto)
```

**Witness examples:**
- `preimage(hex_secret)` -- satisfy a hashlock
- `sign(wallet1.json, wallet2.json)` -- satisfy a multisig
- `branch(right, preimage(hex_secret))` -- satisfy a branch condition

**Example:**
```bash
# Spend a hashlock UTXO
doli spend abcd1234...:0 doli1abc... 10 --witness "preimage(deadbeef...)"

# Spend a multisig UTXO
doli spend abcd1234...:0 doli1abc... 10 --witness "sign(wallet1.json, wallet2.json)"
```

---

### 2.4. Transaction History

Show recent transactions.

```bash
doli history [OPTIONS]

Options:
  -l, --limit <LIMIT>    Maximum transactions to show [default: 10]
```

**Example:**
```bash
doli history --limit 20
```

---

## 3. Chain Information

### 3.1. Chain Status

Display current blockchain status.

```bash
doli chain
```

**Output:**
```
Chain Information
------------------------------------------------------------
Network:      mainnet
Version:      1.1.11
Best Height:  12,345
Best Hash:    a1b2c3d4e5f6...
Best Slot:    12,380
Genesis Hash: 0000000000000...
Reward Pool:  5.00000000 DOLI
------------------------------------------------------------
```

**WHITEPAPER Reference:** Section 4.2 (Time Structure) - Slots, epochs, and chain progression.

---

### 3.2. Chain Verify

Verify chain integrity and compute a running BLAKE3 commitment. Two nodes with the same commitment have identical chains.

```bash
doli chain-verify
```

**Output:**
```
Chain Integrity Verification
------------------------------------------------------------
Scanning all blocks from genesis to tip...

Tip Height:       12,345
Blocks Scanned:   12,345
Complete:         YES
Missing Blocks:   0
Chain Commitment: abcd1234...
```

---

## 4. Producer Operations

These commands implement the producer lifecycle from the WHITEPAPER.

### 4.1. Register as Producer

Register as a block producer with bonds.

```bash
doli producer register [OPTIONS]

Options:
  -b, --bonds <BONDS>    Number of bonds to stake (1-3,000) [default: 1]
```

**Bond Requirements:**
- **Mainnet**: Each bond = 10 DOLI (minimum 1 bond, maximum 3,000 bonds = 30,000 DOLI)
- **Testnet**: Each bond = 1 DOLI (minimum 1 bond, maximum 3,000 bonds = 3,000 DOLI)
- **Devnet**: Each bond = 1 DOLI (for testing)

**Example:**
```bash
# Register with 1 bond (10 DOLI on mainnet, 1 DOLI on testnet/devnet)
doli producer register

# Register with 5 bonds (50 DOLI on mainnet, 5 DOLI on testnet)
doli producer register --bonds 5
```

Registration is epoch-deferred — your producer becomes active at the next epoch boundary.

**WHITEPAPER Reference:** Section 6 (Producer Registration)
- Requires activation bond (Section 6.2)
- Bond stacking up to 3,000x (Section 6.3)
- Dynamic registration difficulty (Section 6.1)

---

### 4.2. Check Producer Status

View producer status, bond vesting tiers, and pending epoch-deferred updates.

```bash
doli producer status [OPTIONS]

Options:
  -p, --pubkey <PUBKEY>    Check specific producer (default: wallet's key)
```

**Example:**
```bash
# Check your status
doli producer status

# Check another producer
doli producer status --pubkey c455c65d3e17...
```

**Output:**
```
Producer Status
------------------------------------------------------------

Address:       doli17engd6utnqs4ag6l6xme7tdhvgh6rcd8ezay5qw0vssqxyw239ts9dygef
Status:        active
Registered at: block 0
Bond Count:    10 (from UTXO set)
Current Era:   1

Bonds: 10 (100.00000000 DOLI):
  Vested (0% penalty):   7 bonds
  Q3 (25% penalty):      1 bond  — ~2h 10m to 0%
  Q2 (50% penalty):      1 bond  — ~30m to 25%
  Q1 (75% penalty):      1 bond  — ~4h 40m to 50%
```

Each bond tier shows time remaining until the oldest bond in that tier graduates to the next tier (lower penalty).

**Rewards:** Pooled epoch distribution — coinbase goes to reward pool, EpochReward tx distributes bond-weighted shares at epoch boundaries. No manual claim needed.

**WHITEPAPER Reference:** Section 7.2 (Deterministic Rewards) - All producers earn identical ROI percentage.

---

### 4.3. Per-Bond Vesting Details

Show detailed per-bond vesting information (age, quarter, penalty, time to next tier).

```bash
doli producer bonds [OPTIONS]

Options:
  -p, --pubkey <PUBKEY>    Check specific producer (default: wallet's key)
```

**Example:**
```bash
doli producer bonds
```

**Output:**
```
Bond Details (5 bonds, 50.00000000 DOLI staked)
------------------------------------------------------------
 #    Outpoint              Created (slot)   Age          Quarter    Penalty  Time to Next
 -----------------------------------------------------------------------------------------
 1    a1b2c3d4...:0         slot 361         23h 45m      Q4+        0%       Fully vested
 2    a1b2c3d4...:1         slot 361         23h 45m      Q4+        0%       Fully vested
 3    e5f6a7b8...:0         slot 2500        12h 10m      Q3         25%      ~5h 50m to 0%
 4    c9d0e1f2...:0         slot 5000        5h 30m       Q2         50%      ~30m to 25%
 5    f3a4b5c6...:0         slot 8000        1h 20m       Q1         75%      ~4h 40m to 50%
```

Bond data is derived from Bond UTXOs in the UTXO set (`output_type=1`, `lock_until=u64::MAX`). The `creation_slot` is stored in the Bond UTXO's `extra_data` field (4 bytes, little-endian u32).

**Vesting schedule** (1-day, quarter-based):

| Quarter | Age | Penalty |
|---------|-----|---------|
| Q1 | 0-6h | 75% burn |
| Q2 | 6-12h | 50% burn |
| Q3 | 12-18h | 25% burn |
| Q4+ | 18h+ | 0% (fully vested) |

---

### 4.4. List All Producers

Display all producers in the network.

```bash
doli producer list [OPTIONS]

Options:
  -a, --active              Show only active producers
      --format <FORMAT>     Output format: table (default), json, csv
```

**Example:**
```bash
# All producers
doli producer list

# Active only
doli producer list --active

# JSON output
doli producer list --format json
```

---

### 4.5. Add Bonds (Bond Stacking)

Increase stake by adding more bonds. Activation is epoch-deferred.

```bash
doli producer add-bond --count <COUNT>

Options:
  -c, --count <COUNT>    Number of bonds to add (1-3,000)
```

**Example:**
```bash
# Add 3 more bonds (30 DOLI on mainnet/testnet)
doli producer add-bond --count 3
```

**WHITEPAPER Reference:** Section 6.3 (Bond Stacking) - More bonds = more block production slots in deterministic rotation.

---

### 4.6. Withdraw Bonds

Withdraw bonds instantly using FIFO order (oldest first). The withdrawal transaction consumes Bond UTXOs and creates a Normal output with the net amount (after penalty). Penalty is burned via UTXO accounting (inputs - outputs = burn). Funds are available immediately in the same block.

```bash
doli producer request-withdrawal --count <COUNT> [OPTIONS]

Options:
  -c, --count <COUNT>              Number of bonds to withdraw
  -d, --destination <DESTINATION>  Destination address (default: wallet address)
```

**Example:**
```bash
# Withdraw 2 bonds
doli producer request-withdrawal --count 2

# Withdraw to a different address
doli producer request-withdrawal --count 2 --destination doli1abc...
```

**Output shows FIFO breakdown before submitting:**
```
Your bonds (5 total):
  2 bonds — vested (0% penalty)
  1 bonds — Q3 (25% penalty)
  1 bonds — Q2 (50% penalty)
  1 bonds — Q1 (75% penalty)

Withdrawing 3 bonds (FIFO — oldest first):
  2 x vested (0% penalty): 20.00000000 DOLI -> 20.00000000 DOLI (0.00000000 DOLI burned)
  1 x Q3 (25% penalty): 10.00000000 DOLI -> 7.50000000 DOLI (2.50000000 DOLI burned)
  --------------------------------------------------
  Total: 30.00000000 DOLI -> 27.50000000 DOLI (2.50000000 DOLI burned)

You receive: 27.50000000 DOLI
Penalty burned: 2.50000000 DOLI
```

---

### 4.7. Simulate Withdrawal (Dry Run)

Preview the FIFO penalty breakdown without submitting a transaction.

```bash
doli producer simulate-withdrawal --count <COUNT>

Options:
  -c, --count <COUNT>    Number of bonds to simulate withdrawing
```

**Example:**
```bash
doli producer simulate-withdrawal --count 3
```

Output is identical to `request-withdrawal` but clearly marked as a dry run with no transaction submitted.

---

### 4.8. Exit Producer Set

Exit the producer set completely. Uses 1-day vesting with quarter-based penalties.

```bash
doli producer exit [OPTIONS]

Options:
      --force    Force early exit with penalty
```

**Exit Penalty (1-day vesting):**

| Quarter | Age | Penalty |
|---------|-----|---------|
| Q1 | 0-6h | 75% burn |
| Q2 | 6-12h | 50% burn |
| Q3 | 12-18h | 25% burn |
| Q4+ | 18h+ | 0% |

**Example:**
```bash
# Exit without penalty (after 1 day)
doli producer exit

# Force early exit (with penalty)
doli producer exit --force
```

Without `--force`, the command shows the current penalty and time remaining before allowing exit.

**WHITEPAPER Reference:** Section 6.4 (Bond Lifecycle)
- Penalty burns are permanent (deflationary)

---

### 4.9. Submit Slashing Evidence

Report double production (equivocation) for slashing.

```bash
doli producer slash --block1 <HASH> --block2 <HASH>

Options:
      --block1 <HASH>    First conflicting block hash
      --block2 <HASH>    Second conflicting block hash (same slot)
```

**Example:**
```bash
doli producer slash \
  --block1 a1b2c3d4e5f6... \
  --block2 c7d8e9f0a1b2...
```

**WHITEPAPER Reference:** Section 10.3 (Double Production)
- 100% of bond burned permanently
- Immediate exclusion from producer set
- Only unambiguously intentional infractions are slashed

---

## 5. Rewards

Block rewards in DOLI work via a pooled epoch model: each block's coinbase goes to the reward pool, and at epoch boundaries, the pool is drained and distributed bond-weighted to qualified producers.

### 5.1. Rewards Subcommands

The `doli rewards` command group has 5 subcommands:

```bash
doli rewards list              # List claimable epochs
doli rewards claim <EPOCH>     # Claim for a specific epoch
  -r, --recipient <ADDRESS>    # Recipient address (default: wallet)
doli rewards claim-all         # Claim all available
  -r, --recipient <ADDRESS>    # Recipient address (default: wallet)
doli rewards history           # Show claim history
  -l, --limit <N>              # Max entries (default: 20)
doli rewards info              # Current epoch info
```

**Note:** Currently, `list`, `claim`, `claim-all`, and `history` print informational messages redirecting to `doli balance`, because epoch rewards are distributed automatically and no manual claiming is needed. Only `rewards info` queries live epoch data.

### 5.2. Show Epoch Info

Display current epoch status and configuration.

```bash
doli rewards info
```

**Output:**
```
Reward Epoch Information
------------------------------------------------------------
Current Height:      2950
Current Epoch:       8
Last Complete Epoch: 7
Blocks per Epoch:    360
Blocks Remaining:    290
Block Reward:        1.00000000 DOLI
------------------------------------------------------------
```

---

## 6. Signing & Verification

### 6.1 Sign a Message

Create a cryptographic signature for a message.

```bash
doli sign [OPTIONS] <MESSAGE>

Arguments:
  <MESSAGE>    Message to sign

Options:
  -a, --address <ADDRESS>    Address to sign with
```

**Example:**
```bash
doli sign "Hello, world!" --address a1b2c3d4...
```

---

### 6.2 Verify a Signature

Verify a message signature.

```bash
doli verify <MESSAGE> <SIGNATURE> <PUBKEY>

Arguments:
  <MESSAGE>      Message that was signed
  <SIGNATURE>    Signature (hex)
  <PUBKEY>       Public key (hex)
```

**Example:**
```bash
doli verify "Hello, world!" 3045... c455c65d3e17...
```

---

## 7. Producer Keys as Wallets

Producer key files (`.json`) have the same format as wallet files and can be used with `-w` to query balances, send coins, or perform any wallet operation.

### 7.1 Key File Locations

Producer key files follow a consistent path pattern per network:

| Network | Pattern | Example |
|---------|---------|---------|
| Mainnet | `/mainnet/n{N}/keys/producer.json` | `/mainnet/n1/keys/producer.json` |
| Testnet | `/testnet/nt{N}/keys/producer.json` | `/testnet/nt1/keys/producer.json` |

Mainnet producers N1-N12 are on ai2. Testnet producers NT1-NT12 are on ai1. See the ops skill for server details.

### 7.2 Key File Format

**Version 1 (legacy — existing producer keys):**
```json
{
  "name": "producer_1",
  "version": 1,
  "addresses": [
    {
      "address": "f66686eb...",
      "public_key": "202047256a8072a8...",
      "private_key": "3f34d5fda9877fba..."
    }
  ]
}
```

**Version 2 (new wallets — BIP-39 derived key):**
```json
{
  "name": "default",
  "version": 2,
  "addresses": [
    {
      "address": "a1b2c3d4...",
      "public_key": "c455c65d3e17...",
      "private_key": "3f34d5fda9877fba..."
    }
  ]
}
```

| Field | Description |
|-------|-------------|
| `version` | 1 = legacy, 2 = BIP-39 derived key |
| `address` | Truncated 20-byte address (40 hex chars) |
| `public_key` | Ed25519 public key (64 hex chars) — used as producer identity |
| `private_key` | Ed25519 private key — **never share this** |

In v2 wallets, the primary key is derived from a BIP-39 seed phrase: `Ed25519_seed = BIP39_seed("")[:32]`. The seed phrase is **not stored** in the wallet file — it is written to a separate `.seed.txt` file at creation time. Write it down on paper, then delete the file. Additional addresses are random (not seed-derived).

The **pubkey hash** used for balance queries is derived from `public_key` via domain-separated BLAKE3: `BLAKE3(le32(12) || "DOLI_ADDR_V1" || pubkey_bytes)`.

### 7.3 Querying Producer Balances

**IMPORTANT:** The `-w` flag is **always required** — even when using `--address` to check someone else's balance. Without `-w`, the CLI fails with `Error: No such file or directory (os error 2)`.

```bash
# Query balance using a producer key file as wallet
doli -w /path/to/producer.json balance

# Query from a remote node
doli -w /path/to/producer.json -r http://127.0.0.1:8546 balance

# Query a specific address — still needs -w for RPC connection
doli -w /path/to/any_wallet.json balance --address <64-char-pubkey-hash-or-bech32>
```

**Example — check all producers from omegacortex:**

```bash
for i in 1 2 3; do
  echo "=== P$i ==="
  doli -w ~/.doli/mainnet/keys/producer_$i.json balance
done
```

**Example — check an external address:**

```bash
doli -w ~/.doli/mainnet/keys/producer_1.json balance --address doli1abc...
```

### 7.4 Sending From a Producer Wallet

Producer keys are full wallets. You can send coins directly (sends immediately, no confirmation):

```bash
doli -w ~/.doli/mainnet/keys/producer_1.json send doli1recipient... 100
```

### 7.5 Batch Scripting Pattern

When scripting multiple CLI calls, always use `-w` and check for errors:

```bash
CLI=~/repos/doli/target/release/doli
W=~/.doli/mainnet/keys/producer_1.json

# Send to multiple addresses (2s delay between for mempool propagation)
for addr in doli1aaa... doli1bbb... doli1ccc...; do
  $CLI -w $W send "$addr" 11 2>&1
  sleep 2
done

# Check balances for external addresses (always needs -w)
for addr in doli1aaa... doli1bbb...; do
  $CLI -w $W balance --address "$addr" 2>&1
done
```

---

## 8. Complete Workflow Examples

### 8.1 New User Workflow

```bash
# 1. Create wallet (write down the 24 recovery words!)
doli new --name "my_doli_wallet"

# 2. Write down the seed phrase, then delete the seed file
rm ~/.doli/wallet.seed.txt

# 3. Generate receiving address
doli address --label "main"

# 4. View address
doli addresses

# 5. After receiving funds, check balance
doli balance
```

### 8.2 Becoming a Producer

```bash
# 1. Ensure you have enough DOLI (10 per bond on mainnet, 1 per bond on testnet)
doli balance

# 2. Register with 1 bond
doli producer register --bonds 1

# 3. Check registration status
doli producer status

# 4. Later, add more bonds for more block slots
doli producer add-bond --count 4
```

### 8.3 Exiting as a Producer

```bash
# 1. Check current status and bond vesting
doli producer status

# 2. Preview withdrawal penalty (dry run)
doli producer simulate-withdrawal --count 2

# 3. Withdraw bonds (instant payout, FIFO penalty)
doli producer request-withdrawal --count 2

# 4. Or exit completely
doli producer exit
```

### 8.4 Reporting Equivocation

If you observe double production (same producer, same slot, different blocks):

```bash
# 1. Identify the conflicting blocks (both should be for same slot)
# You'll need the block hashes from your node logs or explorer

# 2. Submit slashing evidence
doli producer slash \
  --block1 a1b2c3d4... \
  --block2 e5f6a7b8...

# The malicious producer's bond will be burned
```

---

## 9. Node Commands (doli-node)

The `doli-node` binary provides node management, update voting, and maintainer operations.

### 9.1 Running the Node

```bash
doli-node run [OPTIONS]

Options:
  --producer               Enable block production
  --producer-key <PATH>    Path to producer key file (required with --producer)
  --no-auto-update         Disable automatic updates
  --update-notify-only     Only notify about updates, don't apply
  --p2p-port <PORT>        P2P listen port
  --rpc-port <PORT>        RPC listen port
  --bootstrap <ADDR>       Bootstrap node multiaddr
  --no-dht                 Disable DHT discovery
  --chainspec <PATH>       Path to chainspec JSON file
```

**Example:**
```bash
# Run a non-producing node
doli-node --network testnet run

# Run a producing node
doli-node --network testnet run --producer --producer-key wallet.json
```

### 9.2 Node Management

```bash
# Initialize data directory
doli-node init --network mainnet

# Show node status
doli-node status

# Import blocks from file
doli-node import <path>

# Export blocks to file
doli-node export <path> --from 0 --to 1000

# Rebuild height index from block headers (fixes corrupt index)
doli-node reindex

# Recover chain state from blocks (rebuild UTXO/producers/state)
doli-node recover [--yes]

# Full repair pipeline (corrupt state with intact blocks):
#   reindex rebuilds height_index → recover replays blocks → start node
doli-node --network NETWORK --data-dir /path/to/data reindex
doli-node --network NETWORK --data-dir /path/to/data recover --yes
```

---

## 10. Update Commands (doli update)

The update system uses 3/5 maintainer multisig (via SIGNATURES.json in GitHub Releases) with a 2-epoch (~2h) veto period and 40% threshold.

### 10.1 Check for Updates

```bash
doli update check
```

### 10.2 View Update Status

```bash
doli update status
```

### 10.3 Vote on Updates

Producers can vote to approve or veto pending updates.

```bash
doli update vote --approve --version <VERSION>
doli update vote --veto --version <VERSION>
```

**WHITEPAPER Reference:** Section 15 (Governance) - 40% weighted veto threshold.

### 10.4 View Vote Status

```bash
doli update votes --version <VERSION>
```

### 10.5 Apply Update

```bash
doli update apply
```

### 10.6 Rollback

```bash
doli update rollback
```

---

## 11. Maintainer Commands (doli maintainer)

Maintainers are the first 5 registered producers. Changes require 3/5 multisig.

### 11.1 List Maintainers

```bash
doli maintainer list
```

**WHITEPAPER Reference:** Section 15.1 (Maintainer Bootstrap) - First 5 producers become maintainers.

---

## 12. NFT Operations

All NFT commands live under `doli nft` with action flags.

### 12.1. List NFTs

```bash
doli nft --list
doli nft -l
```

Shows all NFTs owned by the wallet (token ID, content, UTXO reference).

### 12.2. NFT Info

```bash
doli nft --info <UTXO>
doli nft -i 7e445810be1e5ecc...:0
```

Displays token ID, content hash/URI, owner address, value, and spending condition.

### 12.3. Mint NFT

```bash
doli nft --mint <CONTENT> [OPTIONS]

Options:
  -c, --condition <COND>    Spending condition (default: signature of minter)
  -a, --amount <DOLI>       DOLI value to attach (default: 0)
      --royalty <PERCENT>   Royalty for the creator (e.g. 5 = 5%)
      --data <HEX>          Raw binary data (hex) to embed on-chain
```

**Example:**
```bash
doli nft -m "ipfs://QmMyArtwork"
doli nft -m "hello-world" --amount 0.01 --royalty 5
doli nft -m "my-art" --data "deadbeef..."
```

Creates a new unique NFT. Content can be an IPFS CID, HTTP URL, hex hash, or plain text.
Default condition: signature of the minter (only minter can transfer).

### 12.4. Transfer NFT

```bash
doli nft --transfer <UTXO> --to <ADDRESS>
doli nft -t 7e445810be1e5ecc...:0 --to tdoli1abc...
```

Transfers an NFT to a new owner. Fee is auto-included from wallet's spendable UTXOs.

### 12.5. Sell NFT (Create Offer)

```bash
doli nft --sell <UTXO> --price <DOLI> -o <FILE>
doli nft -s 7e445810be1e5ecc...:0 --price 10.0 -o my_nft.offer
```

Creates a JSON offer file with the NFT details and asking price. Share this file with potential buyers.

### 12.6. Buy NFT (Direct)

```bash
doli nft --buy <UTXO> --price <DOLI> --seller-wallet <PATH>
doli nft -b 7e445810be1e5ecc...:0 --price 10.0 --seller-wallet /path/to/seller.json
```

Atomic purchase: single transaction with NFT transfer + payment. Both buyer and seller sign.
Requires both wallets to be accessible on the same machine (for testing/internal use).

### 12.7. Buy NFT (From Offer)

```bash
doli nft --from <FILE> --seller-wallet <PATH>
doli nft --from my_nft.offer --seller-wallet /path/to/seller.json
```

Completes a purchase from a sell offer file. The offer provides UTXO, price, and token details.
If the offer is a signed PSBT (`--sell-sign` was used), no `--seller-wallet` is needed.

### 12.8. Sell-Sign NFT (PSBT)

Create a pre-signed sell offer. The seller signs their NFT input; the buyer completes later without needing the seller's wallet.

```bash
doli nft --sell-sign <UTXO> --price <DOLI> --to <BUYER_ADDRESS> -o <FILE>
```

### 12.9. Export NFT Content

Extract on-chain NFT content to a file.

```bash
doli nft --export <UTXO> -o <FILE>
```

### 12.10. Batch Mint NFTs

Mint multiple NFTs from a JSON manifest file.

```bash
doli nft --batch-mint <MANIFEST_FILE> [--yes]
```

---

## 13. Fungible Token Operations

### 13.1. Issue Token

Issue a new fungible token (meme coin, stablecoin, etc.) with a fixed supply.

```bash
doli issue-token <TICKER> [OPTIONS]

Arguments:
  <TICKER>    Token ticker (e.g. DOGEOLI, max 16 chars)

Options:
      --supply <SUPPLY>       Total supply (fixed at issuance, in token units)
  -c, --condition <COND>      Optional spending condition (default: signature of issuer)
```

**Example:**
```bash
doli issue-token DOGEOLI --supply 1000000000
```

---

### 13.2. Token Info

Show metadata for a fungible asset UTXO.

```bash
doli token-info <UTXO>

Arguments:
  <UTXO>    UTXO containing the fungible asset: txhash:output_index
```

**Example:**
```bash
doli token-info abcd1234...:0
```

---

## 14. Pool Operations (AMM)

### 14.1. Create Pool

Create a new AMM pool with initial liquidity.

```bash
doli pool create [OPTIONS]

Options:
      --asset <ASSET>     FungibleAsset ID (hex) for the token side
      --doli <AMOUNT>     Initial DOLI amount
      --tokens <AMOUNT>   Initial token amount (raw token units)
      --fee <BPS>         Fee in basis points (default: 30 = 0.3%)
      --yes               Skip confirmation
```

**Example:**
```bash
doli pool create --asset abcd... --doli 100 --tokens 1000000 --fee 30 --yes
```

---

### 14.2. Swap

Swap assets through a pool.

```bash
doli pool swap [OPTIONS]

Options:
      --pool <POOL_ID>       Pool ID (hex)
      --amount <AMOUNT>      Amount to swap
      --direction <DIR>      Direction: a2b (DOLI->token) or b2a (token->DOLI)
      --min-out <AMOUNT>     Minimum output (slippage protection)
      --yes                  Skip confirmation
```

**Example:**
```bash
doli pool swap --pool abcd... --amount 10 --direction a2b --min-out 9000 --yes
```

---

### 14.3. Add Liquidity

```bash
doli pool add [OPTIONS]

Options:
      --pool <POOL_ID>     Pool ID (hex)
      --doli <AMOUNT>      DOLI amount to add
      --tokens <AMOUNT>    Token amount to add (raw units)
      --yes                Skip confirmation
```

---

### 14.4. Remove Liquidity

```bash
doli pool remove [OPTIONS]

Options:
      --pool <POOL_ID>         Pool ID (hex)
      --shares <AMOUNT>        LP shares to burn (raw units)
      --min-doli <AMOUNT>      Minimum DOLI to receive
      --min-tokens <AMOUNT>    Minimum tokens to receive
      --yes                    Skip confirmation
```

---

### 14.5. List Pools

```bash
doli pool list
```

---

### 14.6. Pool Info

```bash
doli pool info <POOL_ID>

Arguments:
  <POOL_ID>    Pool ID (hex)
```

---

## 15. Lending Operations

### 15.1. Deposit

Deposit DOLI into a lending pool to earn interest.

```bash
doli loan deposit --pool <POOL_ID> --amount <AMOUNT> [--yes]
```

---

### 15.2. Withdraw

Withdraw DOLI + earned interest from a lending pool.

```bash
doli loan withdraw <DEPOSIT_UTXO> [--yes]

Arguments:
  <DEPOSIT_UTXO>    LendingDeposit UTXO: txhash:output_index
```

---

### 15.3. Create Loan

Create a collateralized loan (borrow DOLI against tokens).

```bash
doli loan create [OPTIONS]

Options:
      --pool <POOL_ID>            Lending pool ID (hex)
      --collateral <AMOUNT>       Collateral amount (token units)
      --borrow <AMOUNT>           Amount of DOLI to borrow
      --interest-rate <BPS>       Interest rate in basis points (default: 500 = 5%)
      --yes                       Skip confirmation
```

---

### 15.4. Repay Loan

Repay a loan and recover collateral.

```bash
doli loan repay <LOAN_UTXO> [--yes]

Arguments:
  <LOAN_UTXO>    Collateral UTXO: txhash:output_index
```

---

### 15.5. Liquidate Loan

Liquidate an undercollateralized loan.

```bash
doli loan liquidate <LOAN_UTXO> [--yes]

Arguments:
  <LOAN_UTXO>    Collateral UTXO: txhash:output_index
```

---

### 15.6. List Loans

```bash
doli loan list [OPTIONS]

Options:
      --borrower <ADDRESS>    Filter by borrower address (hex)
```

---

### 15.7. Loan Info

```bash
doli loan info <LOAN_UTXO>

Arguments:
  <LOAN_UTXO>    Collateral UTXO: txhash:output_index
```

---

## 16. Payment Channel Operations

### 16.1. Open Channel

Open a payment channel with a counterparty.

```bash
doli channel open <PEER> <CAPACITY> [OPTIONS]

Arguments:
  <PEER>        Counterparty address (doli1... or hex pubkey_hash)
  <CAPACITY>    Channel capacity in DOLI

Options:
  -f, --fee <FEE>    Fee for the funding transaction (default: auto)
```

---

### 16.2. Channel Pay

Send a payment through a channel.

```bash
doli channel pay <CHANNEL> <AMOUNT>

Arguments:
  <CHANNEL>    Channel ID (hex, first 16 chars sufficient)
  <AMOUNT>     Amount to send in DOLI
```

---

### 16.3. Close Channel

Cooperatively close a channel.

```bash
doli channel close <CHANNEL> [OPTIONS]

Arguments:
  <CHANNEL>    Channel ID (hex)

Options:
  -f, --fee <FEE>    Fee for close transaction (default: auto)
      --force         Force close (unilateral, uses latest commitment tx)
```

---

### 16.4. List Channels

```bash
doli channel list [OPTIONS]

Options:
  -a, --all    Show all channels including closed
```

---

### 16.5. Channel Info

```bash
doli channel info <CHANNEL>

Arguments:
  <CHANNEL>    Channel ID (hex)
```

---

## 17. Bridge Operations (Cross-Chain Atomic Swaps)

### 17.1. Bridge Swap

Initiate a complete cross-chain atomic swap. Generates a preimage and locks DOLI in a BridgeHTLC.

```bash
doli bridge-swap <AMOUNT> [OPTIONS]

Arguments:
  <AMOUNT>    Amount of DOLI to lock

Options:
      --chain <CHAIN>              Target chain: bitcoin, ethereum, monero, litecoin, cardano, bsc
      --to <ADDRESS>               Recipient address on the target chain
      --counter-rpc <URL>          Counter-chain RPC endpoint
      --confirmations <N>          Required confirmations on counter-chain (default: 6 BTC, 12 ETH)
```

---

### 17.2. Bridge Status

Check the status of a bridge swap.

```bash
doli bridge-status <SWAP_ID> [OPTIONS]

Arguments:
  <SWAP_ID>    Swap ID: txhash:output_index

Options:
      --btc-rpc <URL>     Bitcoin Core RPC endpoint
      --eth-rpc <URL>     Ethereum RPC endpoint
      --auto              Auto-claim/refund when conditions are met
```

---

### 17.3. Bridge Buy

Buy into an existing bridge swap (counterparty side).

```bash
doli bridge-buy <SWAP_ID> [OPTIONS]

Arguments:
  <SWAP_ID>    Swap ID: txhash:output_index

Options:
      --preimage <HEX>     Preimage (64 hex chars) if already known
      --btc-rpc <URL>      Bitcoin Core RPC for auto-detecting preimage
      --eth-rpc <URL>      Ethereum RPC for auto-detecting preimage
      --to <ADDRESS>       Send claimed DOLI to this address
      --yes                Skip confirmation
```

---

### 17.4. Bridge List

List active bridge HTLC swaps on-chain.

```bash
doli bridge-list [OPTIONS]

Options:
      --chain <CHAIN>    Filter by target chain
      --blocks <N>       Number of recent blocks to scan (default: 100)
```

---

### 17.5. Bridge Lock (Advanced)

Lock DOLI in a bridge HTLC manually.

```bash
doli bridge-lock <AMOUNT> [OPTIONS]

Arguments:
  <AMOUNT>    Amount of DOLI to lock

Options:
      --hash <HEX>                   BLAKE3 hashlock hash (64 hex chars)
      --preimage <HEX>               Preimage (auto-computes hash)
      --lock <HEIGHT>                Lock height (claim available after)
      --expiry <HEIGHT>              Expiry height (refund available after)
      --chain <CHAIN>                Target chain
      --to <ADDRESS>                 Recipient on target chain
      --counter-hash <HEX>           Counter-chain hash
      --multisig-threshold <N>       Multisig threshold
      --multisig-keys <KEYS>         Comma-separated multisig key addresses
      --yes                          Skip confirmation
```

---

### 17.6. Bridge Claim

Claim a bridge HTLC with the preimage (receiver side).

```bash
doli bridge-claim <UTXO> [OPTIONS]

Arguments:
  <UTXO>    UTXO: txhash:output_index

Options:
      --preimage <HEX>    Preimage (64 hex chars)
      --to <ADDRESS>      Send claimed funds to this address
      --yes               Skip confirmation
```

---

### 17.7. Bridge Refund

Refund a bridge HTLC after expiry (sender side).

```bash
doli bridge-refund <UTXO> [--yes]

Arguments:
  <UTXO>    UTXO: txhash:output_index
```

---

## 18. Upgrade

Upgrade doli binaries to the latest release.

```bash
doli upgrade [OPTIONS]

Options:
      --version <VERSION>           Target version (default: latest)
      --yes                         Skip confirmation
      --doli-node-path <PATH>       Custom path to doli-node binary
      --service <NAME>              Restart only this systemd service
```

**Example:**
```bash
# Upgrade to latest
doli upgrade --yes

# Upgrade to specific version
doli upgrade --version v1.1.12 --yes
```

---

## 19. Release Management

### 19.1. Release Sign

Sign a release (maintainer workflow). Signs the message `{version}:{sha256(CHECKSUMS.txt)}` with a producer key.

```bash
doli release sign [OPTIONS]

Options:
      --version <VERSION>    Release version to sign (e.g., v1.0.27)
      --key <PATH>           Path to producer key file (overrides -w wallet)
```

---

## 20. Protocol Activation

### 20.1. Protocol Sign

Sign a protocol activation (one maintainer at a time).

```bash
doli protocol sign [OPTIONS]

Options:
      --version <VERSION>    Protocol version to activate (e.g., 2)
      --epoch <EPOCH>        Epoch at which activation occurs
      --key <PATH>           Path to producer key file
```

---

### 20.2. Protocol Activate

Submit a protocol activation transaction (requires 3/5 signatures).

```bash
doli protocol activate [OPTIONS]

Options:
      --version <VERSION>           Protocol version to activate
      --epoch <EPOCH>               Activation epoch
      --description <DESC>          Description of consensus changes
      --signatures <PATH>           Path to JSON file with collected signatures
```

---

## 21. Service Management

Manage the doli-node system service (systemd on Linux, launchd on macOS).

### 21.1. Install Service

```bash
doli service install [OPTIONS]

Options:
  -n, --network <NETWORK>          Network (default: mainnet)
      --name <NAME>                Custom service name (default: doli-{network})
      --data-dir <PATH>            Data directory
      --producer-key <PATH>        Path to producer key file
      --p2p-port <PORT>            P2P listen port
      --rpc-port <PORT>            RPC listen port
```

### 21.2. Uninstall Service

```bash
doli service uninstall [--name <NAME>]
```

### 21.3. Start/Stop/Restart

```bash
doli service start [--name <NAME>]
doli service stop [--name <NAME>]
doli service restart [--name <NAME>]
```

### 21.4. Service Status

```bash
doli service status [--name <NAME>]
```

### 21.5. Service Logs

```bash
doli service logs [OPTIONS]

Options:
      --name <NAME>        Custom service name
  -f, --follow             Follow log output (like tail -f)
  -n, --lines <N>          Number of lines to show (default: 50)
```

---

## 22. Snap Sync

Fast-sync: wipe chain data and download a verified state snapshot from the network.

```bash
doli snap [OPTIONS]

Options:
  -d, --data-dir <PATH>     Data directory (for multi-node setups)
      --seed <URL>           Custom seed RPC endpoint(s) (can specify multiple)
      --no-restart           Skip service stop/restart
      --trust                Trust a single seed without consensus verification
```

**Example:**
```bash
# Snap sync from network defaults
doli -n testnet snap

# Snap sync from a specific seed
doli snap --seed http://127.0.0.1:8500 --trust
```

---

## 23. Wipe

Wipe chain data for a fresh resync (preserves keys/ and .env).

```bash
doli wipe [OPTIONS]

Options:
  -n, --network <NETWORK>    Network (default: mainnet)
  -d, --data-dir <PATH>      Data directory (overrides network default)
      --yes                   Skip confirmation prompt
```

**Example:**
```bash
doli wipe --network testnet --yes
```

---

## 24. WHITEPAPER Operations Mapping

| WHITEPAPER Section | CLI Command |
|--------------------|-------------|
| 2. Transactions | `doli send`, `doli spend`, `doli history` |
| 4.2 Time Structure | `doli chain`, `doli chain-verify` |
| 6. Producer Registration | `doli producer register` |
| 6.2 Activation Bond | `doli producer register --bonds N` |
| 6.3 Bond Stacking | `doli producer add-bond`, `doli producer bonds` |
| 6.4 Bond Lifecycle | `doli producer exit`, `doli producer request-withdrawal`, `doli producer simulate-withdrawal` |
| 7. Producer Selection | `doli producer status`, `doli producer list` |
| 9.1 Emission/Rewards | `doli balance`, `doli rewards info` |
| 10.3 Double Production | `doli producer slash` |
| 12. NFTs/Covenants | `doli nft --mint`, `doli nft --transfer`, `doli nft --buy`, `doli nft --sell` |
| 12. Fungible Tokens | `doli issue-token`, `doli token-info` |
| 13. AMM Pools | `doli pool create`, `doli pool swap`, `doli pool add`, `doli pool remove` |
| 13. Lending | `doli loan create`, `doli loan repay`, `doli loan liquidate` |
| 13. Payment Channels | `doli channel open`, `doli channel pay`, `doli channel close` |
| 14. Cross-Chain Bridge | `doli bridge-swap`, `doli bridge-buy`, `doli bridge-claim` |
| 14. Privacy (new keys) | `doli address` |
| 15. Governance | `doli update vote`, `doli maintainer list`, `doli protocol activate` |

---

## 25. Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DOLI_WALLET_FILE` | Default wallet file path | Auto-detected from network |
| `DOLI_RPC_URL` | Default RPC endpoint | Auto-detected from network |
| `DOLI_NETWORK` | Network (mainnet, testnet, devnet) | `mainnet` |
| `DOLI_MAX_PEERS` | Maximum peer connections (node) | `50` |
| `DOLI_EVICTION_GRACE_SECS` | Peer eviction grace period in seconds (node) | `30` |
| `DOLI_DATA_DIR` | Override data directory (node) | Platform-specific |

---

## 26. Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Network/RPC error |
| 4 | Wallet error |

---

## Gotchas

Common pitfalls that cause silent failures or unexpected behavior:

| Trap | Detail |
|------|--------|
| `--wallet` position | Must go BEFORE subcommand: `doli --wallet f.json send ...` not `doli send --wallet f.json ...` |
| `doli send` confirmation | Prompts for confirmation by default. Use `--yes` to skip. |
| `register` vs `add-bond` | `register --bonds N` vs `add-bond --count N` — different flag names |
| RPC balance fields | Use `confirmed`, `immature`, `total`, `unconfirmed`. NOT `balance` or `spendable` |
| `getProducer` address | Returns `addressHash` — use THAT for `getBalance`, not `publicKey` |
| Raw values | Divide raw RPC values by 1e8 for DOLI display |
| BLS key | `Option<BlsKeyPair>` — node starts without it but attestation signing fails silently |

---

## See Also

- [WHITEPAPER.md](/WHITEPAPER.md) - Protocol specification
- [running_a_node.md](./running_a_node.md) - Node operation guide
- [becoming_a_producer.md](./becoming_a_producer.md) - Detailed producer guide
- [rpc_reference.md](./rpc_reference.md) - RPC API documentation

---

*Last updated: March 2026*
