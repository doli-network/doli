# cli.md - DOLI Command Line Interface Reference

Complete reference for all DOLI CLI operations. Every operation described in the [WHITEPAPER.md](/WHITEPAPER.md) can be performed via CLI.

---

## Quick Start

```bash
# Create a wallet
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

Create a new wallet file with a fresh keypair.

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
Created new wallet: ~/.doli/wallet.json
Your first address: a1b2c3d4e5f6...

IMPORTANT: Back up your wallet file!
```

---

### 1.2. Generate New Address

Generate a new address in the wallet.

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
  -b, --bonds <BONDS>    Number of bonds to stake (1-100) [default: 1]
```

**Bond Requirements (Era 1):**
- Each bond = 1,000 DOLI
- Minimum stake = 1 bond (1,000 DOLI)
- Maximum stake = 100 bonds (100,000 DOLI)

**Example:**
```bash
# Register with 1 bond (1,000 DOLI)
doli producer register

# Register with 5 bonds (5,000 DOLI)
doli producer register --bonds 5
```

**WHITEPAPER Reference:** Section 6 (Producer Registration)
- Requires activation bond (Section 6.2)
- Bond stacking up to 100x (Section 6.3)
- Dynamic registration difficulty (Section 6.1)

---

### 4.2. Check Producer Status

View producer status, rewards, and pending withdrawals.

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
Public Key:           c455c65d3e17c07f...
Status:               active
Registration Height:  1,234
Bond Amount:          5,000.00000000 DOLI
Bond Count:           5
Blocks Produced:      1,247
Pending Rewards:      0.00000000 DOLI (auto-distributed)
Era:                  1

Pending Withdrawals:  None
------------------------------------------------------------
```

**Rewards:** Automatically distributed at epoch boundaries as UTXOs. No manual claim needed.

**WHITEPAPER Reference:** Section 7.2 (Deterministic Rewards) - All producers earn identical ROI percentage.

---

### 4.3. List All Producers

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

### 4.4. Add Bonds (Bond Stacking)

Increase stake by adding more bonds.

```bash
doli producer add-bond --count <COUNT>

Options:
  -c, --count <COUNT>    Number of bonds to add (1-100)
```

**Example:**
```bash
# Add 3 more bonds (3,000 DOLI)
doli producer add-bond --count 3
```

**WHITEPAPER Reference:** Section 6.3 (Bond Stacking) - More bonds = more block production slots in deterministic rotation.

---

### 4.5. Request Withdrawal

Request to withdraw bonds. Starts a ~7-day withdrawal delay period (60,480 blocks).

```bash
doli producer request-withdrawal --count <COUNT> [OPTIONS]

Options:
  -c, --count <COUNT>              Number of bonds to withdraw
  -d, --destination <DESTINATION>  Destination address for funds
```

**Example:**
```bash
# Request withdrawal of 2 bonds
doli producer request-withdrawal --count 2

# Specify destination address
doli producer request-withdrawal --count 2 --destination a1b2c3d4...
```

**WHITEPAPER Reference:** Section 6.4 (Bond Lifecycle)
- Withdrawal delay: ~7 days (60,480 blocks)
- Early exit incurs proportional penalty

---

### 4.6. Claim Withdrawal

Claim funds after unbonding period completes.

```bash
doli producer claim-withdrawal [OPTIONS]

Options:
  -i, --index <INDEX>    Withdrawal index [default: 0]
```

**Example:**
```bash
# Claim first pending withdrawal
doli producer claim-withdrawal

# Claim specific withdrawal
doli producer claim-withdrawal --index 1
```

Use `doli producer status` to see pending withdrawals and their claimable status.

---

### 4.7. Exit Producer Set

Exit the producer set completely.

```bash
doli producer exit [OPTIONS]

Options:
      --force    Force early exit with penalty
```

**Exit Penalty (Early Exit):**
```
penalty_pct = (time_remaining × 100) / T_commitment
return = bond × (100 - penalty_pct) / 100
```

**Example:**
```bash
# Normal exit (after 4-year commitment)
doli producer exit

# Force early exit (with penalty)
doli producer exit --force
```

**WHITEPAPER Reference:** Section 6.4 (Bond Lifecycle)
- 4-year commitment period
- Early exit penalties recycle to reward pool

---

### 4.8. Submit Slashing Evidence

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

The following commands are deprecated and non-functional:

| Command | Status |
|---------|--------|
| `doli rewards list` | Deprecated - returns empty |
| `doli rewards claim` | Deprecated - nothing to claim |
| `doli rewards claim-all` | Deprecated - nothing to claim |
| `doli rewards history` | Deprecated - no claim history |

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

## 7. Complete Workflow Examples

### 7.1 New User Workflow

```bash
# 1. Create wallet
doli new --name "my_doli_wallet"

# 2. Generate receiving address
doli address --label "main"

# 3. View address
doli addresses

# 4. After receiving funds, check balance
doli balance
```

### 7.2 Becoming a Producer

```bash
# 1. Ensure you have enough DOLI (1,000+ per bond)
doli balance

# 2. Register with 1 bond
doli producer register --bonds 1

# 3. Check registration status
doli producer status

# 4. Later, add more bonds for more block slots
doli producer add-bond --count 4
```

### 7.3 Exiting as a Producer

```bash
# 1. Check current status and pending withdrawals
doli producer status

# 2. Request withdrawal of some bonds
doli producer request-withdrawal --count 2

# 3. Wait for unbonding period (check status periodically)
doli producer status

# 4. After unbonding completes, claim funds
doli producer claim-withdrawal

# 5. Or exit completely
doli producer exit
```

### 7.4 Reporting Equivocation

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

## 8. WHITEPAPER Operations Mapping

| WHITEPAPER Section | CLI Command |
|--------------------|-------------|
| 2. Transactions | `doli send`, `doli history` |
| 4.2 Time Structure | `doli chain` |
| 6. Producer Registration | `doli producer register` |
| 6.2 Activation Bond | `doli producer register --bonds N` |
| 6.3 Bond Stacking | `doli producer add-bond` |
| 6.4 Bond Lifecycle | `doli producer exit`, `doli producer request-withdrawal`, `doli producer claim-withdrawal` |
| 7. Producer Selection | `doli producer status`, `doli producer list` |
| 9.1 Emission/Rewards | `doli wallet balance` (rewards are automatic via coinbase) |
| 10.3 Double Production | `doli producer slash` |
| 14. Privacy (new keys) | `doli address` |

---

## 9. Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `DOLI_WALLET` | Default wallet path | `~/.doli/wallet.json` |
| `DOLI_RPC` | Default RPC endpoint | `http://127.0.0.1:8545` |

---

## 10. Exit Codes

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

*Last updated: January 2026*
