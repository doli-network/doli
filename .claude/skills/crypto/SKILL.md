# crypto — DOLI Cryptographic Primitives (`doli-crypto` crate, leaf crate)
<!-- @INDEX
ENTRY-POINTS    11-70
OPERATIONS      72-92
DATA-FLOW       94-112
DEPENDENCIES    114-145
CONSTRAINTS     147-185
PATTERNS        187-282
@/INDEX -->

## ENTRY POINTS

Crate root: `crates/crypto/src/lib.rs:1`. Pure leaf crate — zero internal `doli-*` dependencies, no async runtime, no consensus/node types. Re-exports at `lib.rs:63-70,109`: `Hash`, `Hasher`, `hash_with_domain`, `KeyPair`, `PrivateKey`, `PublicKey`, `Address`, `Signature`, `BlsKeyPair`, `BlsPublicKey` (alias of `BlsPublicKeyWrapped`), `BlsSecretKey`, `BlsSignature`, `bls_sign`, `bls_verify`, `bls_aggregate`, `bls_verify_aggregate`, `bls_sign_pop`, `bls_verify_pop`, `BlsError`, `BLS_PUBLIC_KEY_SIZE`, `BLS_SIGNATURE_SIZE` (`lib.rs:63-70`). `attestation_message` and `BLS_ATTESTATION_DST` are NOT re-exported — the former was deleted (INC-I-178 M2 R1); the DST constant is `bls::ATTESTATION_DST` (`bls.rs:60`).

Module map:

| Module | File | Purpose |
|--------|------|---------|
| hash | `hash.rs` | BLAKE3-256 (`Hash`, `Hasher`, free fns) |
| keys | `keys.rs` | Ed25519 keys (`PrivateKey`, `PublicKey`, `KeyPair`, `Address`) |
| signature | `signature.rs` | Ed25519 sign/verify (`Signature`, `SignedMessage`) |
| merkle | `merkle.rs` | Binary Merkle tree (`MerkleTree`, `MerkleProof`) |
| bls | `bls.rs` | BLS12-381 aggregate signatures (attestation) |
| address | `address.rs` | Bech32m address encoding (`doli1.../tdoli1.../ddoli1...`) |
| adaptor | `adaptor.rs` | Ed25519 adaptor signatures (atomic swaps) |
| encrypted_content | `encrypted_content.rs` | AES-256-GCM + ECIES content encryption |

### Public API surface

| Function/Type | Location | Signature | Description |
|---------------|----------|-----------|--------------|
| `Hash` | `hash.rs:32` | `struct Hash([u8;32])` | Constant-time-eq BLAKE3 hash newtype |
| `hash` | `hash.rs:320` | `fn hash(data: &[u8]) -> Hash` | One-shot BLAKE3 |
| `hash_with_domain` | `hash.rs:329` | `fn hash_with_domain(domain: &[u8], data: &[u8]) -> Hash` | Domain-separated hash |
| `hash_many` / `hash_concat` | `hash.rs:339` / `hash.rs:349` | `fn hash_many(data: &[&[u8]]) -> Hash` | Concatenated hash, NOT length-prefixed |
| `derive_key` | `hash.rs:358` | `fn derive_key(context: &str, key_material: &[u8]) -> [u8;32]` | BLAKE3 KDF mode |
| `hash_twice` | `hash.rs:366` | `fn hash_twice(data: &[u8]) -> Hash` | H(H(x)) commitment double-hash |
| `Hasher` | `hash.rs:238` | `struct Hasher { inner: blake3::Hasher }` | Incremental streaming hasher; `new()`/`new_with_domain()`/`new_keyed()` |
| `PublicKey` | `keys.rs:61` | `struct PublicKey([u8;32])` | Ed25519 public key |
| `PrivateKey` | `keys.rs:228` | `struct PrivateKey([u8;32])` (Zeroize+ZeroizeOnDrop) | Ed25519 seed, auto-zeroized on drop |
| `KeyPair` | `keys.rs:323` | `struct KeyPair { private, public }` | Public always derived from private |
| `Address` | `keys.rs:396` | `struct Address([u8;20])` | Truncated legacy address (20B, NOT the wire format — see address.rs) |
| `Signature` | `signature.rs:74` | `struct Signature([u8;64])` | Ed25519 signature (R\|\|S) |
| `sign` / `verify` | `signature.rs:241` / `signature.rs:297` | `fn sign(msg,&PrivateKey)->Signature`; `fn verify(msg,&Signature,&PublicKey)->Result<(),SignatureError>` | Raw Ed25519, no domain |
| `sign_with_domain` / `verify_with_domain` | `signature.rs:269` / `signature.rs:334` | `fn sign_with_domain(domain,msg,&PrivateKey)->Signature` | Domain-separated signing |
| `sign_message` / `verify_message` | `signature.rs:278` / `signature.rs:351` | same as above with `SIGN_DOMAIN` fixed | Default DOLI signing domain |
| `SignedMessage` | `signature.rs:364` | `struct SignedMessage{message,signature,public_key}` | Self-contained signed payload, `.verify()` |
| `MerkleTree` | `merkle.rs:188` | `struct MerkleTree{levels,count}` | Full tree, generates O(log n) proofs |
| `MerkleProof` | `merkle.rs:126` | `struct MerkleProof{index,total,siblings:Vec<(Hash,bool)>}` | Inclusion proof |
| `merkle_root` / `merkle_root_from_hashes` | `merkle.rs:49` / `merkle.rs:74` | `fn merkle_root(items:&[&[u8]])->Option<Hash>` | Root without keeping the tree |
| `transaction_root` | `merkle.rs:313` | `fn transaction_root(tx_hashes:&[Hash])->Hash` | Returns `Hash::ZERO` if empty |
| `BlsPublicKeyWrapped` (`BlsPublicKey`) | `bls.rs:94` | `struct([u8;48])` | BLS12-381 G1 compressed pubkey |
| `BlsSecretKey` | `bls.rs:230` | `struct([u8;32])` (Zeroize+ZeroizeOnDrop) | BLS12-381 scalar |
| `BlsSignature` | `bls.rs:329` | `struct([u8;96])` | BLS12-381 G2 compressed sig |
| `BlsKeyPair` | `bls.rs:448` | `struct{secret,public}` | + `proof_of_possession()` at `bls.rs:490` |
| `bls_sign` / `bls_verify` | `bls.rs:522` / `bls.rs:533` | `fn bls_sign(msg,&BlsSecretKey)->Result<BlsSignature,BlsError>` | Attestation sign/verify (`ATTESTATION_DST`) |
| `bls_sign_pop` / `bls_verify_pop` | `bls.rs:556` / `bls.rs:574` | `fn bls_sign_pop(&BlsSecretKey,&BlsPublicKeyWrapped)->Result<BlsSignature,BlsError>` | Proof-of-possession (`POP_DST`) |
| `bls_aggregate` / `bls_verify_aggregate` | `bls.rs:595` / `bls.rs:621` | `fn bls_aggregate(&[BlsSignature])->Result<BlsSignature,BlsError>` | N→1 aggregation / N-key verify |
| `attestation_message` | `bls.rs:654` | `fn attestation_message(&Hash,slot:u32)->Vec<u8>` | `block_hash(32)\|\|slot(4 BE)` |
| `address::encode` / `address::decode` | `address.rs:70` / `address.rs:81` | `fn encode(&Hash,&str)->Result<String,AddressError>`; `fn decode(&str)->Result<(Hash,String),AddressError>` | Bech32m (BIP-350) encode/decode |
| `address::from_pubkey` | `address.rs:112` | `fn from_pubkey(&[u8],&str)->Result<String,AddressError>` | Hash-then-encode convenience |
| `address::resolve` | `address.rs:131` | `fn resolve(&str,Option<&str>)->Result<Hash,AddressError>` | Accepts bech32m OR 64-char hex |
| `AdaptorSignature` / `AdaptorSecret` | `adaptor.rs:60` / `adaptor.rs:100` | `struct AdaptorSignature{r_prime,s_hat}`; `struct AdaptorSecret` (ZeroizeOnDrop) | Pre-signature + secret scalar `t` |
| `adaptor_sign` / `adaptor_verify` | `adaptor.rs:176` / `adaptor.rs:244` | `fn adaptor_sign(msg,&PrivateKey,&EdwardsPoint)->Result<AdaptorSignature,AdaptorError>` | Create / verify pre-signature |
| `adaptor_complete` / `adaptor_extract` | `adaptor.rs:276` / `adaptor.rs:297` | `fn adaptor_complete(&AdaptorSignature,&AdaptorSecret)->Signature`; `fn adaptor_extract(&Signature,&AdaptorSignature)->Result<AdaptorSecret,AdaptorError>` | Complete into valid sig / extract secret |
| `adaptor_point_to_hash` / `hash_to_adaptor_point` | `adaptor.rs:314` / `adaptor.rs:322` | `fn(&EdwardsPoint)->Hash`; `fn(&Hash)->Result<EdwardsPoint,AdaptorError>` | Point ↔ Hash for `BridgeHTLC.counter_hash` |
| `generate_content_key` | `encrypted_content.rs:44` | `fn() -> [u8;32]` | Random AES-256-GCM key |
| `encrypt_content` / `decrypt_content` | `encrypted_content.rs:52` / `encrypted_content.rs:68` | `fn(key,plaintext)->Result<(Vec<u8>,[u8;12]),EncryptedContentError>` | AES-256-GCM |
| `wrap_key` / `unwrap_key` | `encrypted_content.rs:114` / `encrypted_content.rs:152` | `fn wrap_key(&[u8;32],&PublicKey)->Result<[u8;80],_>`; `fn unwrap_key(&[u8;80],&PrivateKey)->Result<[u8;32],_>` | ECIES key wrap/unwrap |
| `content_hash` | `encrypted_content.rs:193` | `fn(&[u8]) -> [u8;32]` | BLAKE3 of plaintext |

## OPERATIONS

| Task | Steps | Commands/Functions | Inputs | Success |
|------|-------|---------------------|--------|---------|
| Hash data with domain separation | 1. pick domain tag 2. call `hash_with_domain` | `hash_with_domain(TX_DOMAIN, &bytes)` | domain const (`lib.rs:87-103`), byte slice | Deterministic `Hash`; different domains never collide (`hash.rs:452-461` test) |
| Hash incrementally (multi-field struct) | 1. `Hasher::new_with_domain` 2. `update_with_length` per field 3. `finalize` | `Hasher::new_with_domain()`, `.update_with_length()`, `.finalize()` | domain bytes, field byte slices | Unambiguous hash regardless of field boundaries |
| Generate an Ed25519 keypair | 1. `KeyPair::generate()` | `KeyPair::generate()` | OS randomness (`OsRng`) | `KeyPair{private,public}`; address derivable via `.address()` |
| Sign a transaction/message (Ed25519) | 1. pick domain (`TX_DOMAIN`/`BLOCK_DOMAIN`/custom) 2. `sign_with_domain` | `sign_with_domain(domain, &msg, &private_key)` | domain, message bytes, `PrivateKey` | 64-byte `Signature` |
| Verify an Ed25519 signature | 1. `verify_with_domain` with the SAME domain used to sign | `verify_with_domain(domain, &msg, &sig, &pubkey)` | domain, message, `Signature`, `PublicKey` | `Ok(())`, else `Err(SignatureError::VerificationFailed)` |
| Sign an attestation (BLS) | 1. build `attestation_message(block_hash, slot)` 2. `bls_sign` | `attestation_message()`, `bls_sign(&msg, &secret)` | block hash, slot (u32), `BlsSecretKey` | 96-byte `BlsSignature` |
| Aggregate + verify attestations (BLS) | 1. collect per-producer sigs over the SAME message 2. `bls_aggregate` 3. `bls_verify_aggregate` with ALL corresponding pubkeys | `bls_aggregate(&sigs)`, `bls_verify_aggregate(&msg, &agg, &pubkeys)` | signature slice, message, pubkey slice (must match signer set exactly) | one 96-byte sig verifies iff EVERY listed key signed |
| Register a BLS key (rogue-key defense) | 1. `BlsKeyPair::generate()` 2. `proof_of_possession()` 3. registrar calls `bls_verify_pop` before accepting the key | `kp.proof_of_possession()`, `bls_verify_pop(&pubkey, &pop)` | `BlsKeyPair` | PoP accepted BEFORE key is used in any aggregate — CONSTRAINTS #3 |
| Build a Merkle tree + inclusion proof | 1. `MerkleTree::new(&items)` 2. `tree.proof(index)` | `MerkleTree::new()`, `.proof(index)` | slice of byte-slice items (non-empty) | `Some(MerkleProof)` with O(log n) siblings; `None` if empty input or index OOB |
| Verify a Merkle inclusion proof | 1. `proof.verify(&root, item)` | `MerkleProof::verify(root, item)` | root `Hash`, `MerkleProof`, original item bytes | `true`/`false`, no panics |
| Compute a block's transaction root | 1. `transaction_root(&tx_hashes)` | `transaction_root(&[Hash]) -> Hash` | slice of tx hashes | `Hash::ZERO` if empty, else Merkle root |
| Create/verify an adaptor pre-signature (atomic swap) | 1. counterparty generates `AdaptorSecret`, publishes point T (non-identity) 2. signer calls `adaptor_sign(msg,&priv,&T)` 3. peer calls `adaptor_verify` | `adaptor_sign()`, `adaptor_verify()` | message, `PrivateKey`, adaptor point `T` | valid pre-sig; `Err(AdaptorError::IdentityPoint)` if T is the identity element |
| Complete an adaptor signature / extract secret | 1. holder of `t` calls `adaptor_complete` 2. counterparty calls `adaptor_extract` on (completed sig, pre-sig) | `adaptor_complete(&presig,&secret)`, `adaptor_extract(&sig,&presig)` | `AdaptorSignature` + `AdaptorSecret`, or completed `Signature` + `AdaptorSignature` | completed sig verifies as standard Ed25519; extracted `t` equals original `AdaptorSecret` |
| Encrypt content for an owner (ECIES) | 1. `generate_content_key()` 2. `encrypt_content(key, plaintext)` 3. `wrap_key(key, &owner_pubkey)` | `generate_content_key()`, `encrypt_content()`, `wrap_key()` | plaintext bytes, owner `PublicKey` | ciphertext + 12B nonce, plus an 80-byte wrapped key for on-chain storage |
| Decrypt / unwrap a content key | 1. `unwrap_key(wrapped, &owner_priv)` 2. `decrypt_content(key, ciphertext, nonce)` | `unwrap_key()`, `decrypt_content()` | 80-byte wrapped key, owner `PrivateKey`, ciphertext, nonce | recovered plaintext; `Err(KeyUnwrapFailed)`/`Err(DecryptionFailed)` on wrong key |
| Re-wrap a content key on transfer (NFT/MintAsset) | 1. old owner `unwrap_key` 2. `wrap_key` with new owner's pubkey | `unwrap_key()` then `wrap_key()` | old owner `PrivateKey`, new owner `PublicKey` | new owner can `unwrap_key` the re-wrapped blob; old wrapped blob still only opens with old key |
| Encode/decode/resolve a user-facing address | 1. `address::encode`/`decode` for bech32m 2. `address::resolve` accepts either bech32m or hex | `address::encode()`, `address::decode()`, `address::resolve()` | `Hash` (32-byte pubkey_hash) or user-supplied string, network prefix (`doli`/`tdoli`/`ddoli`) | address string, or resolved `Hash`; `Err(NetworkMismatch)` if prefix disagrees |

## DATA FLOW

| Input | Transform | Output | Location |
|-------|-----------|--------|----------|
| raw bytes (tx/block/any) | BLAKE3 over `u32-LE-len(domain) \|\| domain \|\| data` | 32-byte `Hash` | `hash.rs:257-263,329-333` |
| Ed25519 seed (32B) | `SigningKey::generate`/`from_bytes` → derive verifying key | `PrivateKey` → `PublicKey` (bundled as `KeyPair`) | `keys.rs:245-266,328-344` |
| `PublicKey` bytes | `hash_with_domain(ADDRESS_DOMAIN, pubkey)` truncated to 20B | legacy `Address` | `keys.rs:128-133` |
| `Hash` (32-byte pubkey_hash) | Bech32m encode with HRP (`doli`/`tdoli`/`ddoli`) | wire-format address string | `address.rs:70-74` |
| message + `PrivateKey` | domain-hash then Ed25519 sign over the hash bytes | 64-byte `Signature` | `signature.rs:269-272` |
| message + `BlsSecretKey` | `blst` sign with `ATTESTATION_DST` | 96-byte `BlsSignature` | `bls.rs:522-526` |
| N `BlsSignature` values | `blst::AggregateSignature` fold (pairwise point addition) | 1 `BlsSignature` (always 96B regardless of N) | `bls.rs:595-610` |
| list of byte-slice items | leaf-hash (`0x00\|\|data`), then pairwise internal-hash (`0x01\|\|L\|\|R`) bottom-up; odd level pairs last item with itself | root `Hash` (+ `Vec<Vec<Hash>>` levels if using `MerkleTree`) | `merkle.rs:91-119,224-233` |
| tree + leaf index | walk stored levels collecting sibling hash + left/right flag per level | `MerkleProof{index,total,siblings}` | `merkle.rs:266-298` |
| message + `PrivateKey` + adaptor point T | deterministic nonce `r = H(domain\|\|nonce_prefix\|\|T\|\|msg)`, challenge `e = SHA512(R'\|\|A\|\|msg)`, `s_hat = r + e*a` | `AdaptorSignature{R'=R+T, s_hat}` (NOT a valid Ed25519 sig) | `adaptor.rs:176-234` |
| `AdaptorSignature` + `AdaptorSecret t` | `s' = s_hat + t` | valid Ed25519 `Signature` (verifiable by standard `verify()`) | `adaptor.rs:276-288` |
| completed `Signature` + `AdaptorSignature` | `t = s' - s_hat` | recovered `AdaptorSecret` | `adaptor.rs:297-308` |
| plaintext + random 32B key | AES-256-GCM encrypt with random 12B nonce | ciphertext + nonce | `encrypted_content.rs:52-65` |
| content key + owner `PublicKey` | ephemeral X25519 keypair → ECDH → BLAKE3 KDF → AES-256-GCM wrap (nonce = first 12B of ephemeral pubkey) | 80-byte wrapped key (`ephemeral_pub(32) \|\| enc_key+tag(48)`) | `encrypted_content.rs:114-146` |
| wrapped key (80B) + owner `PrivateKey` | Ed25519→X25519 conversion (SHA-512 + clamp), ECDH, BLAKE3 KDF, AES-GCM decrypt | 32-byte content key | `encrypted_content.rs:152-190` |

## DEPENDENCIES

**This Domain Uses** (external crates only — `doli-crypto` is a leaf crate with zero internal `doli-*` dependencies; see `Cargo.toml`):

| This Domain Uses | Skill File | What For |
|-------------------|-----------|----------|
| `blake3` | external, no skill | Core BLAKE3-256 hashing primitive |
| `ed25519_dalek` | external | Ed25519 signing/verification, key expansion |
| `blst` (Supranational) | external | BLS12-381 `min_pk` aggregate signatures (same lib as Ethereum consensus clients) |
| `curve25519_dalek` | external | Edwards-point arithmetic for adaptor sigs + Ed25519→X25519 birational conversion |
| `x25519_dalek` | external | X25519 ECDH for ECIES content-key wrapping |
| `aes-gcm` | external | AES-256-GCM authenticated encryption |
| `sha2` | external | SHA-512 (Ed25519 key expansion, adaptor challenge, ECIES conversion) |
| `bech32` | external | Bech32m address encoding (BIP-350) |
| `subtle` | external | Constant-time equality (`ConstantTimeEq`) |
| `zeroize` | external | Secret zeroization on drop |
| `rand` (`OsRng`) | external | CSPRNG for key/nonce/scalar generation |
| `hex`, `serde`, `thiserror` | external | Hex encoding, (de)serialization, error types |
| `crates/vdf` | N/A — sibling leaf crate, NOT a dependency of `crypto` | Out of this domain's scope. Per project docs, DOLI does NOT use VDF in production despite the crate existing — see CONSTRAINTS |

**Used By** ([UNCLEAR] — this session's `Grep`/`Glob` tools failed with `ENOENT: rg` (ripgrep binary missing from environment); cross-crate call sites could NOT be verified by search. Rows below are inferred from workspace membership in `Cargo.toml:19-31` and doc-comment references only — flagged for synthesizer re-verification with a working grep):

| Used By | Skill File | What For |
|---------|-----------|----------|
| `doli-core` (`crates/core`) | [UNCLEAR — verify] | Transaction/block hashing (`TX_DOMAIN`/`BLOCK_DOMAIN`), signature verification, `Address`/`PublicKey` types in `validation.rs`/`transaction.rs`/`block.rs` |
| `crates/storage` | [UNCLEAR — verify] | `Hash` as UTXO/state DB keys, `Address` for UTXO ownership fields |
| `crates/network` | [UNCLEAR — verify] | peer/message signing, gossip payload hashing |
| `crates/rpc` | [UNCLEAR — verify] | `address::resolve` for RPC address params, key/signature (de)serialization in responses |
| `crates/mempool` | [UNCLEAR — verify] | transaction hash/signature validation |
| `crates/channels`, `crates/bridge` | [UNCLEAR — verify] | `adaptor.rs` doc comment (`adaptor.rs:310-316`) references `BridgeHTLC.counter_hash` for Monero atomic swaps |
| `bins/node` | [UNCLEAR — verify] | producer key management, BLS attestation signing/aggregation (`bls_sign`/`bls_verify_aggregate`) during block production |
| `bins/cli` | [UNCLEAR — verify] | wallet key generation, transaction signing, address encode/decode for user-facing commands |

## CONSTRAINTS

### Security invariants (NEVER violate)

| Constraint | Type | Location | Detail |
|-----------|------|----------|--------|
| Constant-time equality on secret-bearing types | invariant | `hash.rs:121-125`, `keys.rs:141-145,503-507`, `signature.rs:145-149` | `Hash`, `PublicKey`, `Signature`, `Address` implement `ConstantTimeEq`. Never compare raw byte arrays of secrets with `==` — use these types |
| Private key zeroization | invariant | `keys.rs:227`, `bls.rs:229`, `adaptor.rs:99` | `PrivateKey`, `BlsSecretKey`, `AdaptorSecret` derive `Zeroize`+`ZeroizeOnDrop`. Never hold raw byte arrays of private keys outside these types |
| BLS PoP requirement | security | `bls.rs:565-586` | Never accept a BLS public key for aggregation without verifying its PoP (`bls_verify_pop`) at registration first. Skipping PoP allows rogue-key attacks against `bls_verify_aggregate` |
| BLS zero-key sentinel | edge-case | `bls.rs:96-98,135-137` | `BlsPublicKeyWrapped::ZERO` means "no BLS key" — NOT a valid curve point. Never pass it to `bls_verify`/`bls_verify_aggregate` |
| Adaptor identity point rejected | security | `adaptor.rs:181-184` | `adaptor_sign` rejects `EdwardsPoint::identity()`. An identity adaptor point would make private-key extraction trivial |
| Domain separation mandatory | invariant | `lib.rs:87-103` | Use `hash_with_domain`/`sign_with_domain`, never raw `hash`/`sign`, when hashing/signing for a specific purpose. `TX_DOMAIN`, `BLOCK_DOMAIN`, etc. in `lib.rs` are canonical and must not change post-activation |
| Adaptor nonce must include T | security | `adaptor.rs:207-215` (comment: `AUDIT-ADAPT-001`) | The nonce for adaptor signing includes the adaptor point T. Omitting T from a re-implementation allows private-key recovery when the same message is signed with different T values |

### API constraints

- `Hash::ZERO` / `BlsPublicKeyWrapped::ZERO` / `BlsSignature::ZERO` are sentinel values, not cryptographic objects — do not treat as valid curve points/hashes in security-relevant checks.
- `from_bytes_unchecked` on BLS types (`bls.rs:105,339`) skips curve validation — only use for deserializing previously-validated data (e.g. round-tripping already-checked storage).
- `PublicKey::from_bytes` (`keys.rs:69`) does NOT validate the curve point; use `try_from_slice` (`keys.rs:79`) for untrusted/wire input.
- `Hash::from_hex` returns `Option`; `Signature::from_hex`/key types return `Result`.
- `merkle_root`/`MerkleTree::new` return `None` for empty input; `transaction_root` returns `Hash::ZERO` instead of `None`.
- `MerkleTree::proof(index)` returns `None` if `index >= self.count` — no out-of-range panic (`merkle.rs:266-269`).
- `bls_aggregate`/`bls_verify_aggregate` return `Err(BlsError::EmptyAggregation)` on empty input (`bls.rs:596-598,626-628`).
- `Hasher::new_with_domain` is NOT equivalent to `Hasher::new()` then `update(domain)` — the former length-prefixes the domain with a `u32` LE length before hashing it (`hash.rs:256-263`).
- `hash_many`/`hash_concat` do NOT length-prefix individual chunks — ambiguous for variable-length fields; use `Hasher::update_with_length` (`hash.rs:284-288`) for unambiguous multi-field encoding.
- `address::resolve` accepts 64-char hex as a fallback (`address.rs:152-156`) — this is ambiguous (pubkey vs pubkey_hash) for user-facing input; callers exposing raw user text (e.g. CLI `send`) must reject hex before calling `resolve`.
- `crates/vdf` is out of this domain's scope (not analyzed here); per project docs the crate exists but production consensus does NOT use VDF — verify against `crates/vdf` skill/domain before relying on this.

### Size constants (`lib.rs:72-85`)

| Constant | Value | Type |
|---------|-------|------|
| `HASH_SIZE` | 32 | `Hash` bytes |
| `PUBLIC_KEY_SIZE` | 32 | Ed25519 pubkey |
| `PRIVATE_KEY_SIZE` | 32 | Ed25519 seed |
| `SIGNATURE_SIZE` | 64 | Ed25519 signature |
| `ADDRESS_SIZE` | 20 | Truncated legacy address (`keys.rs`) |
| `BLS_PUBLIC_KEY_SIZE` | 48 | BLS12-381 G1 point (`bls.rs:37`) |
| `BLS_SIGNATURE_SIZE` | 96 | BLS12-381 G2 point (`bls.rs:40`) |

## PATTERNS

### Standard transaction hashing
```rust
use crypto::{hash_with_domain, TX_DOMAIN};
let tx_hash = hash_with_domain(TX_DOMAIN, &tx_bytes);
```
Location: `lib.rs:93-94` (domain const), `hash.rs:329` (function).

### Standard block hashing
```rust
use crypto::{hash_with_domain, BLOCK_DOMAIN};
let block_hash = hash_with_domain(BLOCK_DOMAIN, &block_bytes);
```
Location: `lib.rs:96-97`.

### Sign a transaction (recommended)
```rust
use crypto::signature::{sign_with_domain, verify_with_domain};
use crypto::TX_DOMAIN;
let sig = sign_with_domain(TX_DOMAIN, &tx_bytes, private_key);
verify_with_domain(TX_DOMAIN, &tx_bytes, &sig, public_key)?;
```
Location: `signature.rs:269,334`.

### BLS attestation flow (shipped, enforced only at/after the activation height)
```rust
use crypto::{bls_sign, bls_aggregate, bls_verify_aggregate};
use doli_core::attestation::bls_attest_msg;
// Each attester signs the 32-byte attested block hash ALONE (no slot):
let msg = bls_attest_msg(&block_hash);          // attestation/message.rs
let sig = bls_sign(&msg, secret_key)?;          // DST = ATTESTATION_DST
// The next block's producer aggregates the signatures it pooled for its PARENT:
let agg = bls_aggregate(&sigs)?;
// Validators verify against the set-bit producers' on-chain keys:
bls_verify_aggregate(&bls_attest_msg(&parent_hash), &agg, &pubkeys)?;
```
Location: `bls.rs:60` (`ATTESTATION_DST`), `577` (`bls_sign`), `650` (`bls_aggregate`),
`676` (`bls_verify_aggregate`); message in `crates/core/src/attestation/message.rs`.

`crypto::attestation_message` (the old block-hash-**plus-slot** form) was **DELETED** (INC-I-178
M2 R1) — the BLS preimage is the block hash and nothing else, or attesters would not aggregate.
The Ed25519 half still signs `ATTESTATION_DOMAIN ‖ block_hash ‖ slot` and is unchanged.
This flow is wired into consensus validation only **at and after**
`inc_i_178_attestation_bls_activation_height`, which is `u64::MAX` on mainnet, testnet and
devnet today; below it the verifier module is inert.

### Address resolution (CLI/RPC)
```rust
use crypto::address;
// From bech32m or hex:
let pubkey_hash = address::resolve(user_input, Some("doli"))?;
// Encode to bech32m:
let addr_str = address::encode(&pubkey_hash, "doli")?;
```
Location: `address.rs:131,70`.

### Merkle root for a block
```rust
use crypto::merkle::transaction_root;
let tx_hashes: Vec<Hash> = txs.iter().map(|tx| tx.hash()).collect();
let root = transaction_root(&tx_hashes); // Hash::ZERO if empty
```
Location: `merkle.rs:313`.

### Incremental hashing (multi-field struct, unambiguous encoding)
```rust
use crypto::Hasher;
let mut h = Hasher::new_with_domain(b"DOLI_MY_TYPE_V1");
h.update_with_length(field1_bytes); // length-prefixed for unambiguous encoding
h.update_with_length(field2_bytes);
let hash = h.finalize();
```
Location: `hash.rs:257,284`.

### ECIES content encryption (MintAsset / NFT)
```rust
use crypto::encrypted_content::{generate_content_key, encrypt_content, wrap_key, unwrap_key};
let content_key = generate_content_key();
let (ciphertext, nonce) = encrypt_content(&content_key, plaintext)?;
let wrapped = wrap_key(&content_key, &owner_pubkey)?; // 80 bytes, on-chain
// Transfer: unwrap with old owner's key, re-wrap with new owner's key
```
Location: `encrypted_content.rs:44,52,114`.

### Atomic swap adaptor signature (Monero bridge)
```rust
use crypto::adaptor::{adaptor_sign, adaptor_verify, adaptor_complete, adaptor_extract};
let t = AdaptorSecret::generate();
let adaptor_point = t.public_point(); // T = t*G, published to counterparty
let pre_sig = adaptor_sign(&msg, private_key, &adaptor_point)?; // rejects identity T
assert!(adaptor_verify(&msg, public_key, &pre_sig, &adaptor_point));
let real_sig = adaptor_complete(&pre_sig, &t); // valid Ed25519 sig
let extracted_t = adaptor_extract(&real_sig, &pre_sig)?; // == t
```
Location: `adaptor.rs:176,244,276,297`.
