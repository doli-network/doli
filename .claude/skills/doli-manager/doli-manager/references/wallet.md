# Wallet Operations

## Table of Contents
- Create Wallet
- Show Info
- Address Management
- Balance
- Send Coins
- Transaction History
- Sign & Verify
- Export & Import

## Create Wallet

```bash
doli new                        # default name
doli new --name producer-wallet # custom name
```

Output shows the `doli1...` bech32m address. **Back up your wallet file immediately.**

Default wallet path: `~/.doli/wallet.json`
Custom: `doli -w /path/to/wallet.json new`

## Show Info

```bash
doli info
```

Displays: address (`doli1...`), public key, number of addresses.

## Address Management

```bash
# Generate additional address
doli address --label "savings"

# List all addresses
doli addresses
```

Each address is an independent Ed25519 keypair. All displayed as `doli1...`.

## Balance

```bash
# All wallet addresses
doli balance

# Specific address (accepts doli1... or hex)
doli balance --address doli1qpzry9x8gf2tvdw0s3jn54khce6mua7l...
doli balance --address f66686eb8b98215ea35fd1b79f2db7622fa1e1a7c8ba4a01cf64200311ca8957

# Against different node
doli -r http://127.0.0.1:18545 balance   # testnet
doli -r http://127.0.0.1:28545 balance   # devnet
```

Balance types:
- **Confirmed**: spendable now
- **Unconfirmed**: pending in mempool
- **Immature**: coinbase/epoch rewards awaiting maturity (100 blocks mainnet, 10 devnet)

## Send Coins

```bash
# Send to bech32m address
doli send doli1recipient... 100

# Send to hex address (backward compatible)
doli send f66686eb8b98... 50

# Custom fee (default: 0.00001 DOLI)
doli send doli1recipient... 10 --fee 0.001

# Via specific node
doli -r http://127.0.0.1:28545 send doli1... 10
```

The CLI automatically:
1. Selects UTXOs (greedy)
2. Creates change output
3. Signs all inputs
4. Broadcasts via RPC

## Transaction History

```bash
doli history              # last 10 transactions
doli history --limit 50   # more entries
```

## Sign & Verify Messages

```bash
# Sign
doli sign "Hello DOLI"
doli sign "Hello" --address doli1specific...

# Verify (requires hex pubkey)
doli verify "Hello DOLI" <signature_hex> <pubkey_hex>
```

## Export & Import

```bash
# Export wallet to file
doli export /path/to/backup.json

# Import from file
doli import /path/to/backup.json
```

**Wallet file format**: JSON with addresses, public keys, private keys. Keep secure.

## Custom RPC Endpoint

All commands accept `-r` / `--rpc`:

```bash
doli -r http://192.168.1.100:8545 balance
doli -r http://seed1.doli.network:8545 chain
```

## Pubkey Hash Derivation

The internal address used for UTXO lookups is:
```
pubkey_hash = BLAKE3(ADDRESS_DOMAIN || public_key)
```
Where `ADDRESS_DOMAIN = "DOLI_ADDR_V1"`. This 32-byte hash is what gets encoded into `doli1...`.
