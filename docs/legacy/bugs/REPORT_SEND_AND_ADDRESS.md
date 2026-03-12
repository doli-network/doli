# DOLI Transaction System Report: Send Command & Address Issues

**Report Date:** 2026-01-27
**Test Environment:** Testnet (5-node isolated network)
**Chain Height at Test:** 110+

---

## Executive Summary

During transaction system testing, two critical issues were identified that prevent coin transfers via the CLI wallet:

1. **Address format mismatch** between CLI and RPC
2. **Send command not implemented** in CLI

**Status:** BLOCKING - Cannot transfer coins via CLI

---

## 1. Working Components

The following components are functioning correctly:

| Component | Status | Notes |
|-----------|--------|-------|
| Coinbase Transactions | ✅ Working | 5 DOLI (5,000,000,000 units) per block |
| Coinbase Maturity | ✅ Working | 100 confirmations required |
| UTXO Storage | ✅ Working | UTXOs tracked correctly in storage |
| Balance Query (RPC) | ✅ Working | When using correct pubkey_hash format |
| Mempool | ✅ Working | Ready to accept transactions |
| sendTransaction RPC | ✅ Working | Accepts serialized transaction hex |

### Verified Balances

```bash
# Query balance via RPC (using pubkey_hash)
curl -s -X POST -H "Content-Type: application/json" http://127.0.0.1:18541 \
  -d '{"jsonrpc":"2.0","method":"getBalance","params":{"address":"c889e4d5feca08cba6f19cc2fdc9c60567433ae6ea604d712dea3e3fe6b48a8f"},"id":1}'

# Response:
{"jsonrpc":"2.0","result":{"confirmed":25000000000,"total":25000000000,"unconfirmed":0},"id":1}
```

---

## 2. Issue #1: Address Format Mismatch

### Description

The CLI wallet uses a 20-byte address format, but the RPC endpoints expect a 32-byte pubkey_hash (BLAKE3 hash of the public key).

### Technical Details

**Wallet File Format (`/tmp/wallet1.json`):**
```json
{
  "name": "testnet-node1",
  "version": 1,
  "addresses": [
    {
      "address": "a64150866d1b4a0167e09c308b59c46f94711c55",  // 20 bytes
      "public_key": "8aeaf1c93ee0247e618a5056bc5208790dc785a982c5bb32b9004f9503cf9fd1",
      "private_key": "7805ca0366669655cdbc36638446fccd091cebaf0f494822f613f3d662705b15",
      "label": "primary"
    }
  ]
}
```

**CLI Code (`bins/cli/src/wallet.rs`):**
```rust
pub fn primary_address(&self) -> &str {
    &self.addresses[0].address  // Returns 20-byte address
}
```

**RPC Code (`crates/rpc/src/methods.rs`):**
```rust
async fn get_balance(&self, params: Value) -> Result<Value, RpcError> {
    let params: GetBalanceParams = serde_json::from_value(params)?;

    // Expects 32-byte Hash
    let pubkey_hash = Hash::from_hex(&params.address)
        .ok_or_else(|| RpcError::invalid_params("Invalid address format"))?;

    let utxo_set = self.utxo_set.read().await;
    let confirmed = utxo_set.get_balance(&pubkey_hash, chain_state.best_height);
    // ...
}
```

**Node Block Production (`bins/node/src/node.rs`):**
```rust
// Coinbase uses BLAKE3 hash of public key (32 bytes)
let pubkey_hash = crypto_hash(our_pubkey.as_bytes());
builder.add_coinbase(height, pubkey_hash);
```

### Impact

- CLI `balance` command returns 0 (can't find UTXOs)
- CLI `send` command fails to find spendable UTXOs
- CLI `history` command returns no transactions

### Error Observed

```bash
$ ./target/release/doli -w /tmp/wallet1.json -r http://127.0.0.1:18541 balance
Warning: Cannot connect to node at http://127.0.0.1:18541
```

The actual issue is that the RPC call fails with "Invalid address format" because the 20-byte address doesn't parse as a 32-byte Hash.

### Recommended Fix

**Option A: Update CLI to use pubkey_hash**

Modify `bins/cli/src/wallet.rs`:
```rust
use doli_crypto::hash::crypto_hash;

impl Wallet {
    /// Get the pubkey_hash for RPC calls
    pub fn primary_pubkey_hash(&self) -> String {
        let pubkey_bytes = hex::decode(&self.addresses[0].public_key).unwrap();
        let hash = crypto_hash(&pubkey_bytes);
        hash.to_hex()
    }
}
```

**Option B: Update RPC to accept both formats**

Modify `crates/rpc/src/methods.rs`:
```rust
async fn get_balance(&self, params: Value) -> Result<Value, RpcError> {
    let params: GetBalanceParams = serde_json::from_value(params)?;

    // Try 32-byte hash first, then 20-byte address
    let pubkey_hash = if params.address.len() == 64 {
        Hash::from_hex(&params.address)
    } else if params.address.len() == 40 {
        // Convert 20-byte address to lookup format
        Address::from_hex(&params.address)
            .map(|a| /* lookup by address */)
    } else {
        None
    }.ok_or_else(|| RpcError::invalid_params("Invalid address format"))?;
    // ...
}
```

---

## 3. Issue #2: Send Command Not Implemented

### Description

The CLI `send` command is a placeholder that does not actually build or broadcast transactions.

### Technical Details

**File:** `bins/cli/src/main.rs` (lines 287-355)

```rust
async fn cmd_send(
    wallet_path: &PathBuf,
    rpc_endpoint: &str,
    to: &str,
    amount: &str,
    fee: Option<String>,
) -> Result<()> {
    // ... validation code ...

    // Note: Full transaction building would require more implementation
    // This is a placeholder showing the structure
    println!();
    println!("Transaction building requires full implementation.");
    println!("The following UTXOs would be used:");
    for utxo in utxos.iter().take(5) {
        println!("  {}:{} - {}", utxo.tx_hash, utxo.output_index, format_balance(utxo.amount));
    }

    Ok(())
}
```

### Missing Implementation

To complete the send command, the following needs to be implemented:

1. **Transaction Building**
   - Select UTXOs to cover amount + fee
   - Create transaction inputs from UTXOs
   - Create transaction outputs (recipient + change)

2. **Transaction Signing**
   - Sign each input with the corresponding private key
   - Use Ed25519 signatures

3. **Transaction Serialization**
   - Serialize transaction to bytes
   - Convert to hex for RPC

4. **RPC Submission**
   - Call `sendTransaction` RPC with hex-encoded transaction

### Recommended Implementation

```rust
async fn cmd_send(
    wallet_path: &PathBuf,
    rpc_endpoint: &str,
    to: &str,
    amount: &str,
    fee: Option<String>,
) -> Result<()> {
    let wallet = Wallet::load(wallet_path)?;
    let rpc = RpcClient::new(rpc_endpoint);

    // Parse amounts
    let amount_units = coins_to_units(amount.parse()?);
    let fee_units = fee.map(|f| coins_to_units(f.parse().unwrap())).unwrap_or(1000);

    // Get spendable UTXOs
    let pubkey_hash = wallet.primary_pubkey_hash();
    let utxos = rpc.get_utxos(&pubkey_hash, true).await?;

    // Select UTXOs (simple greedy selection)
    let mut selected = Vec::new();
    let mut total_input = 0u64;
    let required = amount_units + fee_units;

    for utxo in utxos {
        selected.push(utxo.clone());
        total_input += utxo.amount;
        if total_input >= required {
            break;
        }
    }

    if total_input < required {
        return Err(anyhow!("Insufficient balance"));
    }

    // Build transaction
    let mut tx_builder = TransactionBuilder::new();

    // Add inputs
    for utxo in &selected {
        let outpoint = Outpoint {
            tx_hash: Hash::from_hex(&utxo.tx_hash).unwrap(),
            index: utxo.output_index as u32,
        };
        tx_builder.add_input(outpoint);
    }

    // Add recipient output
    let recipient_hash = Hash::from_hex(to)?;
    tx_builder.add_output(Output::new(amount_units, recipient_hash));

    // Add change output if needed
    let change = total_input - required;
    if change > 0 {
        let change_hash = Hash::from_hex(&pubkey_hash)?;
        tx_builder.add_output(Output::new(change, change_hash));
    }

    // Sign transaction
    let keypair = wallet.primary_keypair()?;
    let tx = tx_builder.sign(&keypair)?;

    // Serialize and send
    let tx_hex = hex::encode(tx.serialize());
    let tx_hash = rpc.send_transaction(&tx_hex).await?;

    println!("Transaction sent: {}", tx_hash);
    Ok(())
}
```

---

## 4. Workaround

Until the CLI is fixed, transactions can be sent by:

1. Building a `Transaction` object in Rust code
2. Signing with the private key
3. Serializing to hex
4. Calling the `sendTransaction` RPC directly

```bash
curl -X POST -H "Content-Type: application/json" http://127.0.0.1:18541 \
  -d '{"jsonrpc":"2.0","method":"sendTransaction","params":{"tx":"<hex-encoded-signed-transaction>"},"id":1}'
```

---

## 5. Test Environment Details

### Network Configuration

| Node | Port (P2P) | Port (RPC) | Producer Key (first 16 chars) |
|------|------------|------------|------------------------------|
| 1    | 40301      | 18541      | c889e4d5feca08cb |
| 2    | 40302      | 18542      | 34afd3cfce725084 |
| 3    | 40300      | 18543      | 0ae3e870... |
| 4    | 40304      | 18544      | 048d75a9... |
| 5    | 40305      | 18500      | d575bf96... |

### Coinbase Maturity Verification

```bash
# UTXOs before maturity (height < 100)
{"spendable": false, "height": 1, "amount": 5000000000}

# UTXOs after maturity (height > 100)
{"spendable": true, "height": 1, "amount": 5000000000}
```

---

## 6. Files Requiring Changes

| File | Issue | Change Required |
|------|-------|-----------------|
| `bins/cli/src/wallet.rs` | Address format | Add `primary_pubkey_hash()` method |
| `bins/cli/src/main.rs` | Send not implemented | Implement transaction building |
| `bins/cli/src/rpc_client.rs` | Uses address | Update to use pubkey_hash |
| `crates/rpc/src/methods.rs` | (Optional) | Accept both address formats |

---

## 7. Priority

| Issue | Priority | Effort | Impact |
|-------|----------|--------|--------|
| Address mismatch | HIGH | LOW | Blocks all CLI wallet operations |
| Send implementation | HIGH | MEDIUM | Blocks coin transfers |

---

*Report generated: 2026-01-27*
*DOLI Node Version: v0.1.0*
