# crypto — DOLI Cryptographic Primitives
<!-- @INDEX
ENTRY-POINTS: lines 17-30
STRUCTS: lines 32-80
FUNCTIONS: lines 82-166
ALGORITHMS: lines 168-222
DEPENDENCIES: lines 224-238
CONSTRAINTS: lines 240-263
PATTERNS: lines 265-302
VDF-CRATE: lines 304-340
-->

## ENTRY-POINTS

Primary crate entry: `crates/crypto/src/lib.rs`

Top-level re-exports (use these directly in downstream crates):
- `crypto::Hash`, `crypto::Hasher` — hashing
- `crypto::hash_with_domain` — domain-separated hashing
- `crypto::KeyPair`, `crypto::PrivateKey`, `crypto::PublicKey`, `crypto::Address` — Ed25519 keys
- `crypto::Signature` — Ed25519 signature type
- `crypto::BlsKeyPair`, `crypto::BlsPublicKey`, `crypto::BlsSecretKey`, `crypto::BlsSignature` — BLS12-381
- `crypto::bls_sign`, `crypto::bls_verify`, `crypto::bls_aggregate`, `crypto::bls_verify_aggregate`
- `crypto::bls_sign_pop`, `crypto::bls_verify_pop`, `crypto::attestation_message`
- `crypto::BlsError`, `crypto::BLS_PUBLIC_KEY_SIZE`, `crypto::BLS_SIGNATURE_SIZE`

Module map:
- `hash.rs` — BLAKE3-256 primitives (`Hash`, `Hasher`, free functions)
- `keys.rs` — Ed25519 key types (`PrivateKey`, `PublicKey`, `KeyPair`, `Address`)
- `signature.rs` — Ed25519 signing/verification (`Signature`, free functions)
- `merkle.rs` — Binary Merkle tree (`MerkleTree`, `MerkleProof`, `merkle_root`, `transaction_root`)
- `bls.rs` — BLS12-381 aggregate signatures
- `address.rs` — Bech32m address encoding (`doli1...` / `tdoli1...` / `ddoli1...`)
- `adaptor.rs` — Ed25519 adaptor signatures for atomic swaps
- `encrypted_content.rs` — AES-256-GCM + ECIES content encryption

VDF crate (separate): `crates/vdf/src/` — see VDF-CRATE section

---

## STRUCTS

### `Hash` (`hash.rs:32`)
```rust
pub struct Hash([u8; 32]);
```
- Constant-time `PartialEq` via `subtle::ConstantTimeEq`
- `Hash::ZERO` constant; `is_zero()` method
- Serializes as hex string (human-readable) or raw bytes (binary)
- Methods: `from_bytes`, `try_from_slice`, `as_bytes`, `to_vec`, `from_hex`, `to_hex`, `is_zero`, `prefix::<N>`, `xor`

### `Hasher` (`hash.rs:238`)
```rust
pub struct Hasher { inner: blake3::Hasher }
```
Incremental streaming hasher. Constructors:
- `Hasher::new()` — plain BLAKE3
- `Hasher::new_with_domain(domain: &[u8])` — length-prefixes domain then hashes it first
- `Hasher::new_keyed(key: &[u8; 32])` — keyed BLAKE3 (MAC mode)

### `PublicKey` (`keys.rs:61`)
```rust
pub struct PublicKey([u8; 32]);
```
- `from_bytes` (no validation), `try_from_slice` (validates Ed25519 curve point)
- `to_address() -> Address` — domain-separated hash of pubkey, first 20 bytes

### `PrivateKey` (`keys.rs:228`)
```rust
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct PrivateKey([u8; 32]);
```
- Auto-zeroized on drop
- Debug impl redacts: `PrivateKey([REDACTED])`
- `generate()` uses `OsRng`

### `KeyPair` (`keys.rs:323`)
Bundles `PrivateKey` + `PublicKey`. Always consistent (public derived from private).
- `KeyPair::generate()`, `KeyPair::from_seed([u8; 32])`, `KeyPair::from_private_key`

### `Address` (`keys.rs:396`)
```rust
pub struct Address([u8; 20]);
```
- `Address::ZERO`, `Address::burn()` (deterministic burn address)
- `to_checksum_hex()` — EIP-55-style checksum (for display only)
- Derived via `hash_with_domain(ADDRESS_DOMAIN, pubkey_bytes)[..20]`

### `Signature` (`signature.rs:74`)
```rust
pub struct Signature([u8; 64]);
```
- Ed25519 signature (R || S, 32 bytes each)
- Constant-time `PartialEq`
- `r_bytes()`, `s_bytes()` — extract R and S components

### `MerkleProof` (`merkle.rs:126`)
```rust
pub struct MerkleProof {
    pub index: usize,
    pub total: usize,
    pub siblings: Vec<(Hash, bool)>,  // (hash, is_left)
}
```
- `verify(root, item) -> bool`
- `verify_hash(root, item_hash) -> bool`
- `depth() -> usize`

### `MerkleTree` (`merkle.rs:188`)
- `MerkleTree::new(items: &[&[u8]])` — returns `Option<Self>`
- `MerkleTree::from_hashes(hashes: &[Hash])` — returns `Option<Self>`
- `root() -> Hash`, `proof(index) -> Option<MerkleProof>`, `verify(index, item) -> bool`

### BLS Types (`bls.rs`)
- `BlsPublicKeyWrapped` (48 bytes, G1 point) — type alias exported as `BlsPublicKey`
  - `ZERO` constant for "no BLS key" sentinel
  - `from_bytes_unchecked` (trusted), `try_from_slice` (validates G1 curve point)
- `BlsSecretKey` (32 bytes, Zeroize+ZeroizeOnDrop)
  - `generate()`, `from_bytes(bytes) -> Result<Self, BlsError>`
- `BlsSignature` (96 bytes, G2 point)
  - `ZERO` constant; `is_zero()`
  - `from_bytes_unchecked` (trusted), `try_from_slice` (validates G2 curve point)
- `BlsKeyPair` — bundles `BlsSecretKey` + `BlsPublicKeyWrapped`
  - `proof_of_possession() -> Result<BlsSignature, BlsError>`

### Adaptor Types (`adaptor.rs`)
- `AdaptorSignature` — `(R' = R+T, s_hat)` pre-signature, 64 bytes
- `AdaptorSecret` (ZeroizeOnDrop) — secret scalar `t`, generates public point `T = t*G`

---

## FUNCTIONS

### Hash Functions (`hash.rs`)

| Function | Signature | Notes |
|---|---|---|
| `hash` | `fn hash(data: &[u8]) -> Hash` | One-shot BLAKE3 |
| `hash_with_domain` | `fn hash_with_domain(domain: &[u8], data: &[u8]) -> Hash` | Length-prefixes domain |
| `hash_many` | `fn hash_many(data: &[&[u8]]) -> Hash` | Concatenated hash, no length-prefix |
| `hash_concat` | alias for `hash_many` | Legacy |
| `derive_key` | `fn derive_key(context: &str, key_material: &[u8]) -> [u8; 32]` | BLAKE3 KDF mode |
| `hash_twice` | `fn hash_twice(data: &[u8]) -> Hash` | H(H(x)) |

### Signature Functions (`signature.rs`)

| Function | Notes |
|---|---|
| `sign(message, private_key) -> Signature` | Raw Ed25519, no domain |
| `sign_hash(hash, private_key) -> Signature` | Signs hash bytes |
| `sign_with_domain(domain, message, private_key) -> Signature` | Hashes with domain then signs |
| `sign_message(message, private_key) -> Signature` | Uses `SIGN_DOMAIN = "DOLI_SIGN_V1"` |
| `verify(message, sig, pubkey) -> Result<(), SignatureError>` | Raw verify |
| `verify_hash(hash, sig, pubkey) -> Result<(), SignatureError>` | Verifies against hash |
| `verify_with_domain(domain, message, sig, pubkey) -> Result<>` | Hashes with domain then verifies |
| `verify_message(message, sig, pubkey) -> Result<>` | Uses `SIGN_DOMAIN` |

`SignedMessage` struct: self-contained `{message, signature, public_key}` with `.verify()`.

### Merkle Functions (`merkle.rs`)

| Function | Notes |
|---|---|
| `merkle_root(items: &[&[u8]]) -> Option<Hash>` | Hashes items as leaves |
| `merkle_root_from_hashes(hashes: &[Hash]) -> Option<Hash>` | Treats each hash as a leaf |
| `transaction_root(tx_hashes: &[Hash]) -> Hash` | Returns `Hash::ZERO` on empty |

### BLS Functions (`bls.rs`)

| Function | Notes |
|---|---|
| `bls_sign(message, secret_key) -> Result<BlsSignature, BlsError>` | Uses `ATTESTATION_DST` |
| `bls_verify(message, sig, pubkey) -> Result<(), BlsError>` | Single sig verify |
| `bls_sign_pop(secret_key, public_key) -> Result<BlsSignature, BlsError>` | Proof-of-Possession, uses `POP_DST` |
| `bls_verify_pop(public_key, pop) -> Result<(), BlsError>` | Verify PoP at registration |
| `bls_aggregate(signatures: &[BlsSignature]) -> Result<BlsSignature, BlsError>` | N→1 aggregation |
| `bls_verify_aggregate(message, agg_sig, pubkeys) -> Result<(), BlsError>` | Verify N signers' aggregate |
| `attestation_message(block_hash, slot: u32) -> Vec<u8>` | Format: `block_hash(32) \|\| slot(4 BE)` |

### Address Functions (`address.rs`)

| Function | Notes |
|---|---|
| `encode(pubkey_hash: &Hash, network_prefix: &str) -> Result<String, AddressError>` | Bech32m encode |
| `decode(s: &str) -> Result<(Hash, String), AddressError>` | Returns (pubkey_hash, network_prefix) |
| `from_pubkey(pubkey_bytes: &[u8], network_prefix: &str) -> Result<String, AddressError>` | Compute hash then encode |
| `resolve(input, expected_prefix) -> Result<Hash, AddressError>` | Accepts bech32m or 64-char hex |

### Adaptor Functions (`adaptor.rs`)

| Function | Notes |
|---|---|
| `adaptor_sign(message, private_key, adaptor_point) -> Result<AdaptorSignature, AdaptorError>` | Create pre-sig; rejects identity point |
| `adaptor_verify(message, public_key, adaptor_sig, adaptor_point) -> bool` | Verify pre-sig correctness |
| `adaptor_complete(adaptor_sig, adaptor_secret) -> Signature` | Complete into valid Ed25519 sig |
| `adaptor_extract(completed_sig, adaptor_sig) -> Result<AdaptorSecret, AdaptorError>` | Extract `t = s' - s_hat` |
| `adaptor_point_to_hash(point) -> Hash` | Compress Edwards point → Hash (for BridgeHTLC) |
| `hash_to_adaptor_point(hash) -> Result<EdwardsPoint, AdaptorError>` | Decompress |

### Encrypted Content (`encrypted_content.rs`)

| Function | Notes |
|---|---|
| `generate_content_key() -> [u8; 32]` | Random AES-256-GCM key |
| `encrypt_content(key, plaintext) -> Result<(Vec<u8>, [u8; 12]), _>` | AES-256-GCM; returns (ciphertext, nonce) |
| `decrypt_content(key, ciphertext, nonce) -> Result<Vec<u8>, _>` | AES-256-GCM decrypt |
| `wrap_key(content_key, owner_pubkey) -> Result<[u8; 80], _>` | ECIES: X25519 ECDH + BLAKE3 KDF + AES-GCM |
| `unwrap_key(wrapped, owner_private_key) -> Result<[u8; 32], _>` | ECIES unwrap |
| `content_hash(plaintext) -> [u8; 32]` | BLAKE3 of plaintext |

---

## ALGORITHMS

### BLAKE3-256 (`hash.rs`)
- Algorithm: `blake3` crate, 256-bit output
- Security: 128-bit collision resistance
- Domain separation: `Hasher::new_with_domain` prepends `u32::to_le_bytes(domain.len()) || domain`
- Keyed mode: `Hasher::new_keyed(&[u8; 32])` — BLAKE3 keyed hash (MAC)
- KDF mode: `derive_key(context, key_material)` — BLAKE3 KDF mode with static context string

### Ed25519 Signatures (`signature.rs`, `keys.rs`)
- Library: `ed25519_dalek` crate
- Key size: 32-byte seed → 32-byte public key; 64-byte signature
- Deterministic: same key + message = same signature always
- Domain separation applied at hash level before signing (not at Ed25519 level)
- Signing flow: `sign_with_domain` → `hash_with_domain(domain, msg)` → `sign(hash_bytes, private_key)`

### Binary Merkle Tree (`merkle.rs`)
- Leaf node: `H(0x00 || data)` — prefix `0x00` prevents second-preimage attacks
- Internal node: `H(0x01 || left || right)` — prefix `0x01`
- Odd-length level: last element paired with itself (standard Bitcoin-style)
- `transaction_root` returns `Hash::ZERO` for empty tx list
- Proofs: `O(log n)` sibling list; each entry `(Hash, is_left: bool)`

### BLS12-381 (`bls.rs`)
- Library: `blst` crate (Supranational), same as Ethereum consensus clients
- Curve: BLS12-381, `min_pk` mode (48-byte pubkeys G1, 96-byte sigs G2)
- DSTs:
  - Attestation: `b"BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_DOLI_ATTEST_V1"` (`ATTESTATION_DST`)
  - PoP: `b"BLS_POP_BLS12381G2_XMD:SHA-256_SSWU_RO_DOLI_POP_V1"` (module-private)
- Aggregation: N signatures → 1 signature (96 bytes), always same size
- PoP requirement: every registered BLS pubkey must have a verified PoP to prevent rogue key attacks
- Attestation message format: `block_hash(32 bytes) || slot(4 bytes BE)`

### Bech32m Addresses (`address.rs`)
- Standard: BIP-350 Bech32m
- HRPs: `"doli"` (mainnet), `"tdoli"` (testnet), `"ddoli"` (devnet)
- Payload: 32-byte `pubkey_hash = BLAKE3_domain(ADDRESS_DOMAIN, pubkey_bytes)`
- NOTE: The 32-byte `pubkey_hash` is what the UTXO set stores internally; it is NOT the 20-byte `Address` from `keys.rs` (which is a truncated legacy format). The `address.rs` module is the wire format.

### Adaptor Signatures (`adaptor.rs`)
- Curve: Curve25519 (Ed25519 arithmetic via `curve25519_dalek`)
- Protocol: pre-signature `(R' = R+T, s_hat = r + H(R', A, m) * a)`; not valid Ed25519
- Nonce: deterministic — `BLAKE3_domain(ADAPTOR_NONCE_DOMAIN, nonce_prefix || T_compressed || message)`
  - T is included in nonce to prevent private key extraction when same message is signed with different T
- Challenge: SHA-512 (Ed25519 standard) over `R' || A || message`, reduced mod order
- Extraction: `t = s' - s_hat` (trivially computable from completed sig + pre-sig)
- Used for: Monero atomic swaps via BridgeHTLC (`counter_hash` stores compressed adaptor point)

### ECIES Key Wrapping (`encrypted_content.rs`)
- Key encapsulation: X25519 ECDH (ephemeral sender, static recipient)
- Ed25519→X25519 conversion: `SHA-512(ed25519_seed)[..32]` clamped → X25519 static secret; birational Edwards→Montgomery for public key
- KDF: `BLAKE3(shared_secret)` → 32-byte AES key
- Encryption: AES-256-GCM; nonce = first 12 bytes of ephemeral X25519 public key
- Wire format: `ephemeral_public(32) || encrypted_key_with_tag(48)` = 80 bytes total

---

## DEPENDENCIES

**Rust crates (`crates/crypto/Cargo.toml`):**
- `blake3` — BLAKE3 hashing
- `ed25519_dalek` — Ed25519 signing/verification
- `blst` — BLS12-381 (Supranational library, same as Ethereum)
- `curve25519_dalek` — Ed25519/X25519 arithmetic (used by adaptor + ECIES)
- `x25519_dalek` — X25519 ECDH for ECIES
- `aes-gcm` — AES-256-GCM encryption
- `sha2` — SHA-512 (Ed25519 key expansion, adaptor challenge, ECIES conversion)
- `bech32` — Bech32m address encoding
- `subtle` — Constant-time comparisons (`ConstantTimeEq`)
- `zeroize` — Secret memory zeroing (`Zeroize`, `ZeroizeOnDrop`)
- `rand` with `OsRng` — cryptographic randomness
- `hex` — hex encoding/decoding
- `serde` — serialization (human-readable = hex; binary = raw bytes)
- `thiserror` — error types
- `proptest` (dev) — property-based tests

---

## CONSTRAINTS

### Security invariants (NEVER violate)
1. **Constant-time equality**: `Hash`, `PublicKey`, `Signature`, `Address` all implement `ConstantTimeEq`. Never use `==` on raw byte arrays for secret comparison — use the types.
2. **Private key zeroization**: `PrivateKey`, `BlsSecretKey`, `AdaptorSecret` auto-zeroize on drop. Do not hold raw byte arrays of private keys.
3. **BLS PoP requirement**: Never accept a BLS public key for aggregation without verifying its PoP first. `bls_verify_pop` must be called at registration. Skipping PoP allows rogue key attacks.
4. **BLS zero key**: `BlsPublicKeyWrapped::ZERO` is a sentinel for "no BLS key" — not a valid curve point. Do not pass it to `bls_verify` or `bls_verify_aggregate`.
5. **Adaptor identity point**: `adaptor_sign` rejects `EdwardsPoint::identity()`. An identity adaptor point leaks the private key.
6. **Domain separation**: Use `hash_with_domain` or `sign_with_domain` (not raw `hash`/`sign`) when hashing for a specific purpose. The domain constants in `lib.rs` (`TX_DOMAIN`, `BLOCK_DOMAIN`, etc.) are canonical.
7. **Adaptor nonce must include T**: The nonce for adaptor signing includes the adaptor point T. If you re-implement the nonce, including T is mandatory — omitting it allows private key recovery.

### API constraints
- `Hash::ZERO` / `BlsPublicKeyWrapped::ZERO` / `BlsSignature::ZERO` are sentinel values, not cryptographic objects
- `from_bytes_unchecked` on BLS types skips curve validation — only use for deserializing previously validated data
- `PublicKey::from_bytes` does NOT validate the curve point; use `try_from_slice` for untrusted input
- `Hash::from_hex` returns `Option`; `Signature::from_hex` and key types return `Result`
- `merkle_root` returns `None` for empty input; `transaction_root` returns `Hash::ZERO`
- `MerkleTree::proof(index)` returns `None` if index >= tree length (not out-of-range panic)
- `bls_aggregate` and `bls_verify_aggregate` return `Err(BlsError::EmptyAggregation)` on empty input
- `Hasher::new_with_domain` is NOT the same as `Hasher::new()` then `update(domain)` — the former length-prefixes the domain
- `hash_many` does NOT length-prefix individual chunks — ambiguous for variable-length fields; use `Hasher::update_with_length` instead

### Size constants (`lib.rs`)
| Constant | Value | Type |
|---|---|---|
| `HASH_SIZE` | 32 | `Hash` bytes |
| `PUBLIC_KEY_SIZE` | 32 | Ed25519 pubkey |
| `PRIVATE_KEY_SIZE` | 32 | Ed25519 seed |
| `SIGNATURE_SIZE` | 64 | Ed25519 signature |
| `ADDRESS_SIZE` | 20 | Truncated address (legacy, keys.rs) |
| `BLS_PUBLIC_KEY_SIZE` | 48 | BLS12-381 G1 point |
| `BLS_SIGNATURE_SIZE` | 96 | BLS12-381 G2 point |

---

## PATTERNS

### Standard transaction hashing
```rust
use crypto::{hash_with_domain, TX_DOMAIN};
let tx_hash = hash_with_domain(TX_DOMAIN, &tx_bytes);
```

### Standard block hashing
```rust
use crypto::{hash_with_domain, BLOCK_DOMAIN};
let block_hash = hash_with_domain(BLOCK_DOMAIN, &block_bytes);
```

### Sign a transaction (recommended)
```rust
use crypto::signature::{sign_with_domain, verify_with_domain};
use crypto::TX_DOMAIN;
let sig = sign_with_domain(TX_DOMAIN, &tx_bytes, private_key);
verify_with_domain(TX_DOMAIN, &tx_bytes, &sig, public_key)?;
```

### BLS attestation flow (production)
```rust
use crypto::{attestation_message, bls_sign, bls_aggregate, bls_verify_aggregate};
// Each producer signs:
let msg = attestation_message(&block_hash, slot);
let sig = bls_sign(&msg, secret_key)?;
// Block producer aggregates all sigs:
let agg = bls_aggregate(&sigs)?;
// Validators verify:
bls_verify_aggregate(&msg, &agg, &pubkeys)?;
```

### Address resolution (CLI/RPC)
```rust
use crypto::address;
// From bech32m or hex:
let pubkey_hash = address::resolve(user_input, Some("doli"))?;
// Encode to bech32m:
let addr_str = address::encode(&pubkey_hash, "doli")?;
```

### Merkle root for block
```rust
use crypto::merkle::transaction_root;
let tx_hashes: Vec<Hash> = txs.iter().map(|tx| tx.hash()).collect();
let root = transaction_root(&tx_hashes); // Hash::ZERO if empty
```

### Incremental hashing (multi-field struct)
```rust
use crypto::Hasher;
let mut h = Hasher::new_with_domain(b"DOLI_MY_TYPE_V1");
h.update_with_length(field1_bytes); // length-prefixed for unambiguous encoding
h.update_with_length(field2_bytes);
let hash = h.finalize();
```

### ECIES content encryption (MintAsset / NFT)
```rust
use crypto::encrypted_content::{generate_content_key, encrypt_content, wrap_key, unwrap_key};
let content_key = generate_content_key();
let (ciphertext, nonce) = encrypt_content(&content_key, plaintext)?;
let wrapped = wrap_key(&content_key, &owner_pubkey)?; // 80 bytes on-chain
// Transfer: unwrap with old key, re-wrap with new owner's key
```

---

## VDF-CRATE

**Status: The `crates/vdf` crate is NOT used in production consensus.**

The consensus-critical VDF is the BLAKE3 hash-chain implemented in `doli_core::tpop::heartbeat` (`hash_chain_vdf` / `verify_hash_chain_vdf`). The `vdf` crate provides:

1. **Input builders** (`vdf/src/lib.rs`) — used by consensus code:
   - `block_input(prev_hash, merkle_root, slot, producer) -> Hash` — VDF input for block production
   - `registration_input(public_key, epoch) -> Hash` — VDF input for producer registration
   - `selection_seed(prev_hash, slot) -> Hash` — leader selection seed (prefix `b"SEED"`)
   - `registration_difficulty(_count) -> u64` — fixed at `T_REGISTER_BASE = 1_000`

2. **Data types** (wire-format compatibility):
   - `VdfOutput` (`vdf.rs`) — serialized VDF result; `{value: Vec<u8>}` with length-prefixed encoding
   - `VdfProof` (`proof.rs`) — always empty for hash-chain VDF; retained for block header format

3. **Constants**:
   - `T_BLOCK = 800_000` — iterations for block production (~55ms)
   - `T_REGISTER_BASE = 1_000` — iterations for registration (anti-flash-attack barrier)
   - `T_REGISTER_CAP = 1_000` — same as base (no escalation)

**CRITICAL**: `selection_seed` output matches the BLAKE3 test vectors in `hash.rs` (slot 0 → `f3b4b63b...`, slot 1 → `ac1d2a15...`). These are consensus-pinned values — do not change the format.

**CRITICAL**: The Wesolowski class group VDF (Pietrzak/RSA) mentioned in old docs has been removed. The only VDF in use is iterated BLAKE3. `VdfProof::pi` is always empty in production blocks.

Slot timing is NTP/wall-clock based. Faster hardware does NOT mean faster block production. VDF is purely an anti-grinding delay.
