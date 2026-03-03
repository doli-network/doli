# cli.md - DOLI Command Line Interface Reference

Complete reference for all DOLI CLI operations. Every operation described in the [WHITEPAPER.md](/WHITEPAPER.md) can be performed via CLI.

DOLI provides two binaries:
- **doli-cli** (`doli`): Wallet operations, transactions, producer management
- **doli-node**: Node operations, updates, maintainer management

---

## Running Commands

All commands must be run inside the Nix development environment:

```bash
# Enter the Nix shell first
nix --extra-experimental-features "nix-command flakes" develop

# Then run commands
cargo run -p doli-cli -- <command>
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

# Send coins
doli send <recipient> <amount>

# Become a block producer
doli producer register --bonds 1
```

---

## Global Options

All commands support these options:

| Option | Description | Default |
|--------|-------------|---------|
| `-w, --wallet <PATH>` | Wallet file path | `~/.doli/wallet.json` |
| `-r, --rpc <URL>` | Node RPC endpoint | `http://127.0.0.1:8545` |

**Network-specific RPC ports:**

| Network | RPC Port | Example |
|---------|----------|---------|
| Mainnet | 8545 | `--rpc http://127.0.0.1:8545` |
| Testnet | 18545 | `--rpc http://127.0.0.1:18545` |
| Devnet | 28545 | `--rpc http://127.0.0.1:28545` |

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

### 1.2. Generate New Address

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

### 1.3. List Addresses

Display all addresses in the wallet.

```bash
doli addresses
```

---

### 1.4. Show Wallet Info

Display wallet metadata and summary.

```bash
doli info
```

---

### 1.5. Export Wallet

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

### 1.6. Import Wallet

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
  -a, --address <ADDRESS>    Specific address (default: all)
```

**Example:**
```bash
# All addresses
doli balance

# Specific address
doli balance --address a1b2c3d4e5f6...
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
| Confirmed | Spendable balance (mature UTXOs) |
| Unconfirmed | Pending transactions in mempool |
| Immature | Coinbase/epoch rewards pending 100-block maturity |
| Total | Sum of all balances |

---

### 2.2. Send Coins

Transfer coins to another address.

```bash
doli send [OPTIONS] <TO> <AMOUNT>

Arguments:
  <TO>        Recipient address (hex)
  <AMOUNT>    Amount to send in DOLI

Options:
  -f, --fee <FEE>    Transaction fee (default: auto-calculated)
```

**Example:**
```bash
# Send 10 DOLI
doli send c7d8e9f0a1b2... 10

# Send with explicit fee
doli send c7d8e9f0a1b2... 10 --fee 0.001
```

**WHITEPAPER Reference:** Section 2 (Transactions) - Transactions require valid inputs, signatures, and positive amounts.

---

### 2.3. Transaction History

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
Best Height:  12,345
Best Hash:    a1b2c3d4e5f6...
Best Slot:    12,380
Genesis Hash: 0000000000000...
------------------------------------------------------------
```

**WHITEPAPER Reference:** Section 4.2 (Time Structure) - Slots, epochs, and chain progression.

---

## 4. Producer Operations

These commands implement the producer lifecycle from the WHITEPAPER.

### 4.1. Register as Producer

Register as a block producer with bonds.

```bash
doli producer register [OPTIONS]

Options:
  -b, --bonds <BONDS>    Number of bonds to stake (1-10,000) [default: 1]
```

**Bond Requirements:**
- **Mainnet/Testnet**: Each bond = 10 DOLI (minimum 1 bond, maximum 10,000 bonds = 100,000 DOLI)
- **Devnet**: Each bond = 1 DOLI (for testing)

**Example:**
```bash
# Register with 1 bond (10 DOLI on mainnet/testnet, 1 DOLI on devnet)
doli producer register

# Register with 5 bonds (50 DOLI on mainnet/testnet)
doli producer register --bonds 5
```

Registration is epoch-deferred — your producer becomes active at the next epoch boundary.

**WHITEPAPER Reference:** Section 6 (Producer Registration)
- Requires activation bond (Section 6.2)
- Bond stacking up to 10,000x (Section 6.3)
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
Bond Count:    10
Bond Amount:   100.00000000 DOLI
Current Era:   1

Bonds: 10 (100.00000000 DOLI):
  Vested (0% penalty):   7 bonds
  Q3 (25% penalty):      1 bond  — ~2h 10m to 0%
  Q2 (50% penalty):      1 bond  — ~30m to 25%
  Q1 (75% penalty):      1 bond  — ~4h 40m to 50%
```

Each bond tier shows time remaining until the oldest bond in that tier graduates to the next tier (lower penalty).

**Rewards:** Automatically distributed via coinbase (1 DOLI per block). No manual claim needed.

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
 #    Created (slot)   Age          Quarter    Penalty  Time to Next
 ----------------------------------------------------------------------
 1    slot 361         23h 45m      Q4+        0%       Fully vested
 2    slot 361         23h 45m      Q4+        0%       Fully vested
 3    slot 2500        12h 10m      Q3         25%      ~5h 50m to 0%
 4    slot 5000        5h 30m       Q2         50%      ~30m to 25%
 5    slot 8000        1h 20m       Q1         75%      ~4h 40m to 50%
```

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
  -a, --active    Show only active producers
```

**Example:**
```bash
# All producers
doli producer list

# Active only
doli producer list --active
```

---

### 4.5. Add Bonds (Bond Stacking)

Increase stake by adding more bonds. Activation is epoch-deferred.

```bash
doli producer add-bond --count <COUNT>

Options:
  -c, --count <COUNT>    Number of bonds to add (1-10,000)
```

**Example:**
```bash
# Add 3 more bonds (30 DOLI on mainnet/testnet)
doli producer add-bond --count 3
```

**WHITEPAPER Reference:** Section 6.3 (Bond Stacking) - More bonds = more block production slots in deterministic rotation.

---

### 4.6. Withdraw Bonds

Withdraw bonds instantly using FIFO order (oldest first). Each bond's penalty depends on its individual vesting age. Funds are available immediately; bonds are removed at the next epoch boundary.

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

Block rewards in DOLI work like Bitcoin: producers receive rewards automatically
when they produce a block via the coinbase transaction. **No claiming is needed.**

Per WHITEPAPER.md Section 9.1:
- Initial reward: 1 DOLI/block
- Reward maturity: 100 confirmations (Section 9.2)
- Halving interval: 12,614,400 blocks (~4 years)

### 5.1. How Rewards Work

When a producer creates a block:
1. A coinbase transaction is included as the first transaction
2. The coinbase pays 1 DOLI (Era 1) directly to the producer
3. The reward is spendable after 100 confirmations (maturity)

This is identical to Bitcoin's reward model. There is no epoch-based claiming
or presence tracking - rewards are deterministic and immediate.

### 5.2. Checking Your Balance

To see your accumulated rewards:

```bash
doli wallet balance
```

Rewards appear as "pending" until they reach 100 confirmations, then become
"confirmed" and spendable.

### 5.3. Deprecated Commands

The following commands print an informative message redirecting to `doli balance`:

| Command | Message |
|---------|---------|
| `doli rewards list` | "Rewards are distributed automatically via coinbase" |
| `doli rewards claim` | Same — no claiming needed |
| `doli rewards claim-all` | Same — no claiming needed |
| `doli rewards history` | "Use 'doli history' to see received rewards" |

These commands existed for a weighted presence reward system that was removed
in favor of the simpler Bitcoin-like coinbase model.

---

### 5.5. Show Epoch Info

Display current epoch status and configuration.

```bash
doli rewards info
```

**Output:**
```
Epoch Information
------------------------------------------------------------
Current Epoch:          8
Current Height:         2950
Epoch Progress:         190/360 (52.8%)
Blocks per Epoch:       360
Next Epoch Starts At:   2880

Last Complete Epoch:    7
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

| Node | Host | Key Path |
|------|------|----------|
| N1 | omegacortex | `/home/ilozada/.doli/mainnet/keys/producer_1.json` |
| N2 | omegacortex | `/home/ilozada/.doli/mainnet/keys/producer_2.json` |
| N3 | N3-VPS (147.93.84.44) | `/home/ilozada/.doli/mainnet/keys/producer_3.json` |
| N4 | pro-KVM1 (72.60.70.166) | `/home/isudoajl/.doli/mainnet/keys/producer_4.json` |
| N5 | fpx (72.60.115.209) | `/home/isudoajl/.doli/mainnet/keys/producer_5.json` |

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

```bash
# Query balance using a producer key file as wallet
doli -w /path/to/producer.json balance

# Query from a remote node
doli -w /path/to/producer.json -r http://127.0.0.1:8546 balance

# Query a specific address by pubkey hash (no wallet file needed on disk,
# but a default wallet must exist at ~/.doli/wallet.json)
doli balance --address <64-char-pubkey-hash>
```

**Example — check all producers from omegacortex:**

```bash
for i in 1 2 3; do
  echo "=== P$i ==="
  doli -w ~/.doli/mainnet/keys/producer_$i.json balance
done
```

### 7.4 Sending From a Producer Wallet

Producer keys are full wallets. You can send coins directly:

```bash
doli -w ~/.doli/mainnet/keys/producer_1.json send <recipient-pubkey-hash> 100
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
# 1. Ensure you have enough DOLI (100+ per bond on mainnet/testnet)
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

### 11.1 Running the Node

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

### 11.2 Node Management

```bash
# Initialize data directory
doli-node init --network mainnet

# Show node status
doli-node status

# Import blocks from file
doli-node import <path>

# Export blocks to file
doli-node export <path> --from 0 --to 1000

# Recover chain state from blocks
doli-node recover [--yes]
```

---

## 10. Update Commands (doli-node update)

The update system uses 3/5 maintainer multisig with a 7-day veto period and 40% threshold.

### 11.1 Check for Updates

```bash
doli-node update check
```

### 11.2 View Update Status

```bash
doli-node update status
```

### 11.3 Vote on Updates

Producers can vote to approve or veto pending updates.

```bash
doli-node update vote --approve --key <producer-key.json>
doli-node update vote --veto --key <producer-key.json>
```

**WHITEPAPER Reference:** Section 15 (Governance) - 40% weighted veto threshold.

### 11.4 View Vote Status

```bash
doli-node update votes [--version <VERSION>]
```

### 11.5 Apply Update

```bash
doli-node update apply [--force]
```

### 10.6 Rollback

```bash
doli-node update rollback
```

### 10.7 Verify Release

```bash
doli-node update verify --version <VERSION>
```

---

## 11. Maintainer Commands (doli-node maintainer)

Maintainers are the first 5 registered producers. Changes require 3/5 multisig.

### 11.1 List Maintainers

```bash
doli-node maintainer list
```

### 11.2 Add Maintainer

Propose adding a new maintainer (requires 3/5 signatures).

```bash
doli-node maintainer add --target <PUBKEY> --key <maintainer-key.json>
```

### 11.3 Remove Maintainer

Propose removing a maintainer (requires 3/5 signatures).

```bash
doli-node maintainer remove --target <PUBKEY> --key <maintainer-key.json> [--reason "reason"]
```

### 11.4 Sign Proposal

Sign a pending maintainer change proposal.

```bash
doli-node maintainer sign --proposal-id <ID> --key <maintainer-key.json>
```

### 11.5 Verify Maintainer Status

```bash
doli-node maintainer verify --pubkey <PUBKEY>
```

**WHITEPAPER Reference:** Section 15.1 (Maintainer Bootstrap) - First 5 producers become maintainers.

---

## 12. WHITEPAPER Operations Mapping

| WHITEPAPER Section | CLI Command |
|--------------------|-------------|
| 2. Transactions | `doli send`, `doli history` |
| 4.2 Time Structure | `doli chain` |
| 6. Producer Registration | `doli producer register` |
| 6.2 Activation Bond | `doli producer register --bonds N` |
| 6.3 Bond Stacking | `doli producer add-bond`, `doli producer bonds` |
| 6.4 Bond Lifecycle | `doli producer exit`, `doli producer request-withdrawal`, `doli producer simulate-withdrawal` |
| 7. Producer Selection | `doli producer status`, `doli producer list` |
| 9.1 Emission/Rewards | `doli balance` (rewards are automatic via coinbase) |
| 10.3 Double Production | `doli producer slash` |
| 14. Privacy (new keys) | `doli address` |
| 15. Governance | `doli-node update vote`, `doli-node maintainer` |

---

## 13. Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DOLI_WALLET` | Default wallet path | `~/.doli/wallet.json` |
| `DOLI_RPC` | Default RPC endpoint | `http://127.0.0.1:8545` |

---

## 14. Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Network/RPC error |
| 4 | Wallet error |

---

## See Also

- [WHITEPAPER.md](/WHITEPAPER.md) - Protocol specification
- [running_a_node.md](./running_a_node.md) - Node operation guide
- [becoming_a_producer.md](./becoming_a_producer.md) - Detailed producer guide
- [rpc_reference.md](./rpc_reference.md) - RPC API documentation

---

*Last updated: March 2026*
