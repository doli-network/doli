# DOLI Engine Parts Inventory

> Every piece of the DOLI blockchain engine, organized by subsystem.
> Verified 2026-04-19 by 50 parallel agents reading every source file (~100k+ lines across 11 crates).

---

## 1. CRYPTO (`crates/crypto`)

### Hashing

- Hash — 32-byte BLAKE3-256 digest (constant-time equality via ConstantTimeEq); newtype over `[u8; 32]`
- Hash::ZERO — zero constant (all bytes 0x00)
- Hash::from_bytes() — create from raw bytes array without validation
- Hash::zero() — deprecated alias for ZERO constant
- Hash::try_from_slice() — create from byte slice; returns None if not 32 bytes
- Hash::as_bytes() — borrow underlying `[u8; 32]`
- Hash::to_vec() — clone into owned Vec
- Hash::from_hex() — parse lowercase hex string; returns None on failure
- Hash::to_hex() — encode as lowercase hex string
- Hash::is_zero() — constant-time check for all-zero value
- Hash::prefix() — take first N bytes as `[u8; N]`
- Hash::xor() — XOR two hashes (for accumulators)
- Hasher — incremental BLAKE3 hasher wrapping `blake3::Hasher`
- Hasher::new() — plain hasher
- Hasher::new_with_domain() — length-prefixed domain separation
- Hasher::new_keyed() — keyed hasher for MAC operations
- Hasher::update() — feed data bytes
- Hasher::update_with_length() — feed length-prefixed data
- Hasher::finalize() — produce final Hash
- Hasher::reset() — reset to initial state
- hash() — one-shot BLAKE3-256 hash of a byte slice
- hash_with_domain() — domain-separated one-shot hash
- hash_many() — hash multiple byte slices without allocating concat
- hash_concat() — legacy alias for hash_many; accepts `&[&[u8]]` (not two Hash values)
- derive_key() — BLAKE3 key derivation mode
- hash_twice() — double hash H(H(x))

### Keys

- KeyError — errors from key operations
  - KeyError::InvalidLength — wrong byte count with expected and got fields
  - KeyError::InvalidKey — bytes do not represent a valid Ed25519 key
  - KeyError::InvalidHex — hex decode failure
- PublicKey — 32-byte Ed25519 public key (constant-time comparison via ConstantTimeEq); newtype over `[u8; 32]`
- PublicKey::from_bytes() — create without curve validation
- PublicKey::try_from_slice() — validated from slice; checks valid curve point
- PublicKey::as_bytes() — borrow underlying bytes
- PublicKey::to_vec() — clone into Vec
- PublicKey::from_hex() — parse hex with curve validation
- PublicKey::to_hex() — encode as lowercase hex
- PublicKey::to_address() — derive 20-byte Address via domain-separated BLAKE3
- PrivateKey — 32-byte Ed25519 seed (ZeroizeOnDrop, never displayed); newtype over `[u8; 32]`
- PrivateKey::from_bytes() — from raw bytes
- PrivateKey::generate() — secure random via OsRng
- PrivateKey::as_bytes() — borrow bytes (handle carefully)
- PrivateKey::public_key() — derive corresponding PublicKey
- PrivateKey::from_hex() — parse hex
- PrivateKey::to_hex() — encode as hex (handle carefully)
- KeyPair — paired PrivateKey + PublicKey; guarantees they match
- KeyPair::generate() — generate fresh random keypair
- KeyPair::from_private_key() — create from existing PrivateKey; derives public
- KeyPair::from_seed() — create from raw 32-byte seed
- KeyPair::private_key() — reference to PrivateKey
- KeyPair::public_key() — reference to PublicKey
- KeyPair::address() — derive Address from public key
- Address — 20-byte address derived from public key hash; newtype over `[u8; 20]`
- Address::ZERO — all-zero address constant
- Address::from_bytes() — from raw bytes
- Address::try_from_slice() — from slice with length check
- Address::as_bytes() — borrow underlying bytes
- Address::to_vec() — clone into Vec
- Address::burn() — canonical burn address (BLAKE3 of "DOLI_BURN_ADDRESS_V1")
- Address::is_zero() — check for all-zero address
- Address::is_burn() — check for burn address
- Address::from_hex() — parse hex
- Address::to_hex() — encode as lowercase hex
- Address::to_checksum_hex() — EIP-55 style checksummed hex

### Signatures

- SignatureError — errors from signature operations
  - SignatureError::InvalidLength — wrong byte count with expected and got fields
  - SignatureError::VerificationFailed — signature did not verify
  - SignatureError::KeyError — underlying key error
  - SignatureError::InvalidHex — hex decode failure
- Signature — 64-byte Ed25519 signature (constant-time comparison via ConstantTimeEq); newtype over `[u8; 64]`
- Signature::from_bytes() — create from raw bytes
- Signature::try_from_slice() — from slice with length check
- Signature::as_bytes() — borrow underlying bytes
- Signature::to_vec() — clone into Vec
- Signature::from_hex() — parse hex
- Signature::to_hex() — encode as lowercase hex
- Signature::r_bytes() — first 32 bytes (R component)
- Signature::s_bytes() — last 32 bytes (S component)
- SignedMessage — self-contained signed data with message, signature, and signer public key
- SignedMessage::message — the signed message bytes (`Vec<u8>`)
- SignedMessage::signature — Ed25519 Signature
- SignedMessage::public_key — signer's PublicKey
- SignedMessage::new() — sign message with default domain
- SignedMessage::verify() — verify self-contained signature
- sign() — sign raw bytes with Ed25519
- sign_hash() — sign pre-computed Hash
- sign_with_domain() — domain-separated sign
- sign_message() — sign with DOLI default domain (DOLI_SIGN_V1)
- verify() — verify signature against public key and raw bytes
- verify_hash() — verify signature against pre-computed Hash
- verify_with_domain() — domain-separated verify
- verify_message() — verify with DOLI default domain

### Merkle Trees

- MerkleTree — full binary Merkle tree; stores all levels for efficient proof generation
- MerkleTree::new() — build tree from raw items (hashes each item internally)
- MerkleTree::from_hashes() — build tree from pre-computed hashes
- MerkleTree::root() — get root Hash
- MerkleTree::len() — number of items
- MerkleTree::is_empty() — true if empty
- MerkleTree::depth() — number of tree levels
- MerkleTree::proof() — generate inclusion MerkleProof for item at index
- MerkleTree::verify() — verify raw item at index against tree root
- MerkleProof — inclusion proof with index, total, and sibling path
- MerkleProof::index — item index in original list
- MerkleProof::total — total item count in tree
- MerkleProof::siblings — sibling hashes with is_left flag, leaf to root (`Vec<(Hash, bool)>`)
- MerkleProof::verify() — verify raw item against root hash
- MerkleProof::verify_hash() — verify pre-computed item hash against root hash
- MerkleProof::depth() — number of levels in the tree
- merkle_root() — compute root from raw items (hashes each item to create leaves); returns None if empty
- merkle_root_from_hashes() — compute root from pre-computed hashes; returns None if empty
- transaction_root() — transaction commitment root; returns Hash::ZERO for empty

### Addresses (Bech32m)

- AddressError — errors from bech32m address operations
  - AddressError::Bech32 — bech32 encoding/decoding failure
  - AddressError::InvalidLength — decoded data wrong length
  - AddressError::UnknownPrefix — HRP not in known set
  - AddressError::NetworkMismatch — address for wrong network
  - AddressError::InvalidFormat — generic parse failure
- encode() — encode 32-byte pubkey hash as bech32m address with given network prefix
- decode() — decode bech32m address to (Hash, network_prefix)
- from_pubkey() — derive bech32m address directly from raw public key bytes
- resolve() — universal resolver accepting bech32m string or 64-char hex

### BLS Cryptography

- BLS_PUBLIC_KEY_SIZE — 48; compressed G1 point size in bytes
- BLS_SIGNATURE_SIZE — 96; compressed G2 point size in bytes
- BLS_ATTESTATION_DST — BLS domain separation tag for attestation signing (re-exported from `bls::ATTESTATION_DST`)
- BlsError — errors from BLS operations
  - BlsError::InvalidSecretKey — bytes not a valid BLS scalar
  - BlsError::InvalidPublicKey — bytes not a valid G1 point
  - BlsError::InvalidSignature — bytes not a valid G2 point
  - BlsError::VerificationFailed — signature did not verify
  - BlsError::InvalidProofOfPossession — proof-of-possession verification failed
  - BlsError::EmptyAggregation — no signatures provided for aggregation
  - BlsError::InvalidHex — hex decode failure
- BlsPublicKeyWrapped — 48-byte compressed G1 point; newtype over `[u8; 48]`; re-exported as `BlsPublicKey`
- BlsPublicKeyWrapped::ZERO — all-zero constant; sentinel for "no BLS key"
- BlsPublicKeyWrapped::from_bytes_unchecked() — create without curve validation (for trusted data)
- BlsPublicKeyWrapped::try_from_slice() — validated from slice; checks valid G1 point
- BlsPublicKeyWrapped::as_bytes() — borrow underlying bytes
- BlsPublicKeyWrapped::is_zero() — check for sentinel zero value
- BlsPublicKeyWrapped::from_hex() — parse hex with G1 validation
- BlsPublicKeyWrapped::to_hex() — encode as lowercase hex
- BlsSecretKey — 32-byte BLS secret key (ZeroizeOnDrop); newtype over `[u8; 32]`
- BlsSecretKey::generate() — secure random via OsRng + blst key_gen
- BlsSecretKey::from_bytes() — from bytes with scalar validation
- BlsSecretKey::as_bytes() — borrow bytes (handle carefully)
- BlsSecretKey::public_key() — derive BlsPublicKeyWrapped
- BlsSecretKey::from_hex() — parse hex with validation
- BlsSecretKey::to_hex() — encode as hex (handle carefully)
- BlsSignature — 96-byte compressed G2 point; newtype over `[u8; 96]`
- BlsSignature::ZERO — all-zero constant; sentinel for empty/unset signature
- BlsSignature::from_bytes_unchecked() — create without G2 validation
- BlsSignature::try_from_slice() — validated from slice; checks valid G2 point
- BlsSignature::as_bytes() — borrow underlying bytes
- BlsSignature::is_zero() — check for sentinel zero value
- BlsSignature::from_hex() — parse hex with G2 validation
- BlsSignature::to_hex() — encode as hex
- BlsKeyPair — paired BlsSecretKey + BlsPublicKeyWrapped
- BlsKeyPair::generate() — generate fresh random BLS keypair
- BlsKeyPair::from_secret_key() — create from existing BlsSecretKey; derives public key
- BlsKeyPair::secret_key() — reference to BlsSecretKey
- BlsKeyPair::public_key() — reference to BlsPublicKeyWrapped
- BlsKeyPair::proof_of_possession() — generate PoP signature over public key
- bls_sign() — sign message bytes with ATTESTATION_DST
- bls_verify() — verify single BLS signature
- bls_sign_pop() — sign public key bytes with POP_DST for proof-of-possession
- bls_verify_pop() — verify proof-of-possession signature
- bls_aggregate() — aggregate N BLS signatures into one 96-byte aggregate
- bls_verify_aggregate() — fast_aggregate_verify for same-message case
- attestation_message() — canonical attestation message: block_hash bytes || slot (4 bytes BE)

### Adaptor Signatures (Atomic Swaps)

- AdaptorError — errors from adaptor signature operations
  - AdaptorError::IdentityPoint — adaptor point is the group identity (insecure)
  - AdaptorError::InvalidPoint — bytes do not represent a valid curve point
  - AdaptorError::InvalidScalar — bytes are not a valid canonical scalar
- AdaptorSignature — adaptor pre-signature `(R', s_hat)` where `R' = R + T` and `s_hat = r + H(R', A, m) * a`
- AdaptorSignature::to_bytes() — serialize as `[R' || s_hat]` (64 bytes)
- AdaptorSignature::from_bytes() — deserialize and validate point
- AdaptorSecret — secret scalar `t` where `T = t * G`; ZeroizeOnDrop
- AdaptorSecret::generate() — generate random scalar via OsRng
- AdaptorSecret::from_bytes() — from canonical scalar bytes with validation
- AdaptorSecret::as_bytes() — borrow raw bytes
- AdaptorSecret::public_point() — compute `T = t * G` as uncompressed EdwardsPoint
- AdaptorSecret::public_point_compressed() — compute and compress adaptor point as CompressedEdwardsY
- adaptor_sign() — create adaptor pre-signature; rejects identity point; uses deterministic nonce via BLAKE3
- adaptor_verify() — verify pre-signature consistency (does NOT verify a complete Ed25519 signature)
- adaptor_complete() — complete pre-signature into valid Ed25519 Signature: `s' = s_hat + t`
- adaptor_extract() — extract secret `t = s' - s_hat` from completed signature and pre-signature
- adaptor_point_to_hash() — compress adaptor EdwardsPoint into Hash (for BridgeHTLC counter_hash field)
- hash_to_adaptor_point() — decompress Hash back to EdwardsPoint

---

## 2. VDF (`crates/vdf`)

### Constants

- T_BLOCK = 800,000 — hash-chain VDF iterations for block production (~55ms)
- T_REGISTER_BASE = 1,000 — fixed VDF iterations for registration (negligible delay)
- T_REGISTER_CAP = 1,000 — maximum registration VDF iterations (same as base; no escalation)
- DISCRIMINANT_BITS = 2,048 — class group discriminant bit size (~112-bit security)

### Core

- VdfError — errors from VDF operations
  - VdfError::InvalidInput — input hash is invalid
  - VdfError::InvalidProof — proof format is invalid
  - VdfError::VerificationFailed — proof does not match output
  - VdfError::ComputationError — internal computation error
  - VdfError::InvalidTimeParameter — t is zero or otherwise invalid
- VdfParams — discriminant and bit size for Wesolowski class group VDF
- VdfParams::discriminant — the imaginary quadratic discriminant (negative, ≡ 1 mod 4)
- VdfParams::discriminant_bits — number of bits in discriminant
- VdfParams::default_params() — standard DOLI discriminant (2048-bit, seed "DOLI_VDF_DISCRIMINANT_V1")
- VdfParams::with_seed() — custom discriminant for testing (bits, seed)
- VdfOutput — serialized class group element result `y = x^(2^t)`
- VdfOutput::value — serialized class group element bytes (`Vec<u8>`)
- VdfOutput::new() — create from ClassGroupElement
- VdfOutput::size() — byte count of serialized output
- VdfOutput::to_bytes() — length-prefixed serialization
- VdfOutput::from_bytes() — deserialize length-prefixed bytes; returns None if malformed
- VdfOutput::to_hex() — hex encode value bytes
- VdfOutput::from_hex() — hex decode
- VdfProof — Wesolowski proof: single group element `π = x^(floor(2^t / l))`
- VdfProof::pi — serialized proof group element (`Vec<u8>`)
- VdfProof::new() — create from ClassGroupElement
- VdfProof::empty() — placeholder empty proof (fails verification)
- VdfProof::is_empty() — check if proof is the empty placeholder
- VdfProof::size() — byte count of proof
- VdfProof::to_bytes() — length-prefixed serialization (4 bytes LE + data)
- VdfProof::from_bytes() — deserialize; returns None if malformed
- VdfProof::to_hex() — hex encode pi bytes
- VdfProof::from_hex() — hex decode
- VdfProof::display_hex() — truncated hex for display (first/last 8 chars with "...")
- compute() — compute VDF output and proof with default params; t must be > 0
- verify() — verify VDF output against proof with default params; checks `y == π^l · x^r`
- compute_with_params() — compute with custom VdfParams (for testing)
- verify_with_params() — verify with custom VdfParams
- block_input() — construct deterministic VDF input for block production using domain "DOLI_VDF_BLOCK_V1"
- registration_input() — construct VDF input for producer registration using domain "DOLI_VDF_REGISTER_V1"
- selection_seed() — derive leader selection seed using domain "SEED"
- registration_difficulty() — returns fixed T_REGISTER_BASE regardless of registered producer count (scaling was removed; parameter is unused)

### Class Group

- ClassGroupError — errors in class group operations
  - ClassGroupError::InvalidDiscriminant — discriminant not negative and ≡ 1 mod 4
  - ClassGroupError::InvalidElement — not a valid reduced binary quadratic form
  - ClassGroupError::SerializationError — serialization or deserialization failure
  - ClassGroupError::ArithmeticError — mathematical operation failure
- ClassGroupElement — reduced binary quadratic form `(a, b, c)` with `Δ = b² - 4ac`; `a` and discriminant stored, `c` computed on demand
- ClassGroupElement::a — coefficient a (always positive)
- ClassGroupElement::b — coefficient b
- ClassGroupElement::discriminant (field) — discriminant Δ (always negative)
- ClassGroupElement::new() — create and reduce; validates discriminant and a; computes c from `(b² - Δ) / 4a`
- ClassGroupElement::discriminant() — method: borrow discriminant reference
- ClassGroupElement::c() — compute coefficient c = `(b² - Δ) / 4a`
- ClassGroupElement::identity() — principal form: `(1, 1, (1-Δ)/4)` for Δ ≡ 1 mod 4
- ClassGroupElement::from_hash() — deterministic hash-to-group mapping using "DOLI_CLASS_GROUP_HASH_TO_GROUP_V1"
- ClassGroupElement::compose() — group composition of two elements via Dirichlet/Cohen algorithm 5.4.7
- ClassGroupElement::square() — self ∘ self; core VDF squaring operation
- ClassGroupElement::pow() — square-and-multiply exponentiation; handles negative exponents via inverse
- ClassGroupElement::inverse() — `(a, b, c)` → `(a, -b, c)` then reduce
- ClassGroupElement::is_identity() — true if a == 1
- ClassGroupElement::to_bytes() — serialize as `[a_len(4)][a_bytes][b_sign(1)][b_len(4)][b_bytes]`
- ClassGroupElement::from_bytes() — deserialize and reduce from bytes + discriminant
- generate_discriminant() — derive negative fundamental discriminant (≡ 1 mod 4) of given bit size from seed using "DOLI_DISCRIMINANT_EXPANSION_V1" hash chaining
- pow2_mod() — compute `2^t mod l` using rug optimized modular exponentiation
- div_2pow_by_l() — compute `floor(2^t / l)`; direct for t ≤ 128, iterative doubling for t > 128

> Compiled from agent reports A01–A16 against the current source.
> Every error in the old spec has been fixed. Every gap has been filled.
> Format: `- Name — description` per item. Methods listed under their owner type.

---

## 3. CORE (`crates/core`)

### Types (`crates/core/src/types.rs`)
- type Amount — u64 alias; base units (1 coin = 10^8 base units)
- type BlockHeight — u64 alias; 0-indexed block height
- type Slot — u32 alias; time-based slot number
- type Epoch — u32 alias; 360 slots = 1 epoch = 1 hour at 10s/slot
- type Era — u32 alias; 12,614,400 blocks = 1 era ≈ 4 years
- const DECIMALS: u32 = 8 — number of decimal places
- const UNITS_PER_COIN: Amount = 100_000_000 — base units per coin
- fn coins_to_units — const fn; converts coin count to base units
- fn units_to_coins — const fn; converts base units to coins (truncating)
- struct DisplayAmount — newtype wrapper for human-readable amount formatting (N.NNNNNNNN)

---

### Block (`crates/core/src/block.rs`)
- struct BlockHeader — wire-format block header; fields: version, prev_hash, merkle_root, presence_root, genesis_hash, timestamp, slot, producer, vdf_output, vdf_proof, missed_producers, data_root, fork_id (NOTE: NO height field; NO attestation_count field — both are spec errors in the old version)
- struct Block — complete block; fields: header, transactions, aggregate_bls_signature, attestation_bitfield
- struct BlockBuilder — builder for constructing new blocks; fields: prev_hash, prev_slot, producer, transactions, params, genesis_hash, presence_root, missed_producers, fork_id

#### BlockHeader methods
- fn BlockHeader::hash — compute BLAKE3 block hash committing to all header fields; conditionally includes fork_id only when non-zero
- fn BlockHeader::vdf_input — compute VDF input hash from prev_hash, merkle_root, slot, and producer
- fn BlockHeader::serialize — bincode serialization; returns empty vec on error
- fn BlockHeader::deserialize — bincode deserialization; returns None on error
- fn BlockHeader::size — approximate byte size
- fn BlockHeader::attestation_commitment — returns Some(presence_root) for version >= 2, None for version 1

#### Block methods
- fn Block::new — construct Block with empty aggregate_bls_signature and attestation_bitfield
- fn Block::hash — delegate to header.hash()
- fn Block::prev_hash — accessor for header.prev_hash
- fn Block::slot — accessor for header.slot
- fn Block::timestamp — accessor for header.timestamp
- fn Block::producer — accessor for header.producer
- fn Block::is_genesis — true if header.prev_hash is zero
- fn Block::compute_merkle_root — compute Merkle root from self.transactions
- fn Block::verify_merkle_root — compare stored vs. computed Merkle root
- fn Block::coinbase — first transaction in block
- fn Block::total_fees — always returns 0 (UTXO lookup placeholder)
- fn Block::serialize — bincode serialization
- fn Block::deserialize — bincode deserialization
- fn Block::size — exact serialized byte length

#### BlockBuilder methods
- fn BlockBuilder::new — initialize builder with ConsensusParams::mainnet() defaults
- fn BlockBuilder::with_params — override consensus params and sync genesis_hash from them
- fn BlockBuilder::with_presence_root — set attestation bitfield commitment
- fn BlockBuilder::with_missed_producers — set on-chain liveness exclusion list
- fn BlockBuilder::with_fork_id — set fork identity hash
- fn BlockBuilder::add_transaction — append transaction to list
- fn BlockBuilder::add_coinbase — insert coinbase at index 0 with block_reward(height)
- fn BlockBuilder::add_coinbase_with_extra — insert coinbase at index 0 with block_reward(height) + extra_amount (fee routing)
- fn BlockBuilder::build — finalize: compute slot, enforce slot monotonicity, compute merkle_root and data_root, return header + transactions; returns None on slot monotonicity violation

#### Free functions
- fn compute_merkle_root — BLAKE3 binary Merkle tree over transaction hashes; empty slice returns BLAKE3(""); odd-length levels duplicate last leaf

---

### Transaction Types (30 active variants; discriminant values are wire-critical)
- Transfer = 0 — regular coin transfer; no inputs = coinbase
- Registration = 1 — register as block producer
- Exit = 2 — initiate unbonding / producer exit
- ClaimReward = 3 — claim accumulated pending rewards
- ClaimBond = 4 — claim bond after unbonding period
- SlashProducer = 5 — slash a misbehaving producer with evidence
- Coinbase = 6 — TOMBSTONE / wire-compat reserved; DO NOT USE; real coinbases are Transfer with no inputs
- AddBond = 7 — add bonds to increase stake
- RequestWithdrawal = 8 — instant bond withdrawal with vesting penalty
- ClaimWithdrawal = 9 — TOMBSTONE / wire-compat reserved; DO NOT REUSE (was ClaimWithdrawal)
- EpochReward = 10 — automatic epoch-boundary reward distribution
- RemoveMaintainer = 11 — remove maintainer (3/5 multisig)
- AddMaintainer = 12 — add maintainer (3/5 multisig)
- DelegateBond = 13 — delegate bond weight to Tier-1/2 validator
- RevokeDelegation = 14 — revoke delegation (unbonding delay applies)
- ProtocolActivation = 15 — schedule protocol activation (3/5 multisig)
- (discriminant 16 — unassigned gap)
- MintAsset = 17 — mint units of a fungible asset (issuer-only)
- BurnAsset = 18 — burn fungible asset units
- CreatePool = 19 — create AMM pool with initial liquidity
- AddLiquidity = 20 — add liquidity to pool
- RemoveLiquidity = 21 — remove liquidity, burn LP shares
- Swap = 22 — swap assets through AMM pool
- (discriminant 23 — unassigned gap)
- CreateLoan = 24 — create collateralized loan
- RepayLoan = 25 — repay loan and recover collateral
- LiquidateLoan = 26 — liquidate undercollateralized loan
- LendingDeposit = 27 — deposit DOLI into lending pool
- LendingWithdraw = 28 — withdraw DOLI + interest from lending pool
- FractionalizeNft = 29 — lock NFT and mint fraction tokens
- RedeemNft = 30 — burn all fraction tokens, unlock NFT
- ZKSettle = 31 — L2 ZK settlement; gated by ZK_SETTLE_ACTIVATION_HEIGHT

#### TxType methods
- fn TxType::from_u32 — converts u32 discriminant to Option<TxType>; returns None for gaps (16, 23) and unknown values

---

### Output Types (14 variants)
- Normal = 0 — standard single-signature spendable output
- Bond = 1 — time-locked bond; protocol-governed withdrawal only
- Multisig = 2 — threshold-of-N signatures; also used for escrow
- Hashlock = 3 — requires preimage reveal
- HTLC = 4 — hashlock + timelock OR expiry refund
- Vesting = 5 — signature + timelock (vesting schedule)
- NFT = 6 — non-fungible token with metadata + covenant conditions
- FungibleAsset = 7 — user-issued fixed-supply token
- BridgeHTLC = 8 — cross-chain atomic swap with target-chain metadata
- Pool = 9 — AMM pool output; reserves + TWAP state in extra_data
- LPShare = 10 — transferable liquidity provider share
- Collateral = 11 — locked loan collateral
- LendingDeposit = 12 — lending pool deposit receipt; earns interest
- ZKRollup = 13 — L2 committed state; consumable only by ZKSettle

#### OutputType methods
- fn OutputType::from_u8 — converts u8 to Option<OutputType>
- fn OutputType::is_conditioned — true for Multisig, Hashlock, HTLC, Vesting, NFT, FungibleAsset, BridgeHTLC
- fn OutputType::is_native_amount — true when amount field is denominated in native DOLI (not token units)

---

### SighashType
- enum SighashType (repr u8) — controls which parts of the transaction an input signature covers
  - All = 0 — sign all inputs + all outputs (default; backwards-compatible)
  - AnyoneCanPay = 1 — sign only this input + committed outputs
- fn SighashType::from_u8 — converts u8 to Option<SighashType>

---

### Transaction Structure (`crates/core/src/transaction/`)

#### struct Transaction
- field version: u32
- field tx_type: TxType
- field inputs: Vec<Input>
- field outputs: Vec<Output>
- field extra_data: Vec<u8>

#### Transaction constructors
- fn Transaction::new_transfer — creates Transfer transaction
- fn Transaction::new_coinbase — creates coinbase (Transfer, no inputs); extra_data = height_le8 || slot_le4 for uniqueness
- fn Transaction::new_epoch_reward_coinbase — epoch reward batch distribution; extra_data = height_le8 || epoch_le8
- fn Transaction::new_epoch_reward — single-recipient EpochReward with EpochRewardData in extra_data
- fn Transaction::new_registration — creates Registration transaction; creates bond_count Bond outputs
- fn Transaction::new_exit — Exit; no inputs/outputs; ExitData in extra_data
- fn Transaction::new_claim_reward — ClaimReward; no inputs; ClaimData in extra_data
- fn Transaction::new_claim_bond — ClaimBond; no inputs; ClaimBondData in extra_data
- fn Transaction::new_slash_producer — SlashProducer; no inputs/outputs; SlashData in extra_data
- fn Transaction::new_add_bond — AddBond; creates bond_count Bond outputs; AddBondData in extra_data
- fn Transaction::new_request_withdrawal — RequestWithdrawal; consumes Bond inputs, creates Normal output; WithdrawalRequestData in extra_data
- fn Transaction::new_remove_maintainer — RemoveMaintainer; no inputs/outputs; MaintainerChangeData in extra_data
- fn Transaction::new_add_maintainer — AddMaintainer; no inputs/outputs; MaintainerChangeData in extra_data
- fn Transaction::new_delegate_bond — DelegateBond; no inputs/outputs
- fn Transaction::new_revoke_delegation — RevokeDelegation; no inputs/outputs
- fn Transaction::new_protocol_activation — ProtocolActivation; no inputs/outputs; requires 3/5 maintainer multisig

#### Transaction predicate checks
- fn Transaction::is_coinbase — Transfer + no inputs + exactly 1 output
- fn Transaction::is_epoch_reward_coinbase — EpochReward + at least 1 output
- fn Transaction::is_reward_minting — is_coinbase() || is_epoch_reward_coinbase()
- fn Transaction::is_exit — TxType::Exit
- fn Transaction::is_registration — TxType::Registration
- fn Transaction::is_claim_reward — TxType::ClaimReward
- fn Transaction::is_epoch_reward — TxType::EpochReward
- fn Transaction::is_claim_bond — TxType::ClaimBond
- fn Transaction::is_slash_producer — TxType::SlashProducer
- fn Transaction::is_add_bond — TxType::AddBond
- fn Transaction::is_request_withdrawal — TxType::RequestWithdrawal
- fn Transaction::is_remove_maintainer — TxType::RemoveMaintainer
- fn Transaction::is_add_maintainer — TxType::AddMaintainer
- fn Transaction::is_maintainer_change — is_remove_maintainer() || is_add_maintainer()
- fn Transaction::is_delegate_bond — TxType::DelegateBond
- fn Transaction::is_revoke_delegation — TxType::RevokeDelegation
- fn Transaction::is_protocol_activation — TxType::ProtocolActivation
- fn Transaction::is_state_only — true for Exit, ClaimReward, ClaimBond, SlashProducer, DelegateBond, RevokeDelegation, AddMaintainer, RemoveMaintainer (no UTXO inputs by design)

#### Transaction extra_data parsers
- fn Transaction::epoch_reward_data — parse EpochRewardData; None if wrong tx_type
- fn Transaction::exit_data — parse ExitData
- fn Transaction::claim_data — parse ClaimData
- fn Transaction::claim_bond_data — parse ClaimBondData
- fn Transaction::slash_data — parse SlashData
- fn Transaction::add_bond_data — parse AddBondData
- fn Transaction::withdrawal_request_data — parse WithdrawalRequestData
- fn Transaction::registration_data — parse RegistrationData via bincode
- fn Transaction::maintainer_change_data — parse MaintainerChangeData
- fn Transaction::delegate_bond_data — parse DelegateBondData
- fn Transaction::revoke_delegation_data — parse RevokeDelegationData
- fn Transaction::protocol_activation_data — parse ProtocolActivationData

#### Transaction hashing and signing
- fn Transaction::hash — BLAKE3 canonical tx hash; includes all fields including extra_data; excludes signature bytes
- fn Transaction::signing_message — BLAKE3 hash excluding extra_data (SegWit-style: witnesses excluded from signing message)
- fn Transaction::signing_message_for_input — BIP-143-style per-input hash; respects sighash_type (All vs AnyoneCanPay); AnyoneCanPay also respects committed_output_count

#### Transaction witness / covenant
- fn Transaction::set_covenant_witnesses — encode per-input witness bytes into extra_data; length prefix: u16 LE (<65535) or 0xFFFF escape + u32 LE (>=65535)
- fn Transaction::get_covenant_witness — decode single witness from extra_data by input index

#### Transaction fee and accounting
- fn Transaction::minimum_fee — BASE_FEE + (sum_output_extra_data_bytes × FEE_PER_BYTE) / FEE_DIVISOR
- fn Transaction::total_output — sum of amount across all outputs where output_type.is_native_amount() is true

#### Transaction serialization
- fn Transaction::serialize — bincode serialization
- fn Transaction::deserialize — bincode deserialization
- fn Transaction::size — byte size via serialize()

---

### struct Input (`crates/core/src/transaction/types.rs`)
- field prev_tx_hash: Hash — hash of the transaction containing the output
- field output_index: u32 — index of the output in that transaction
- field signature: Signature — Ed25519 signature proving ownership
- field sighash_type: SighashType — what this signature covers (default: All)
- field committed_output_count: u32 — AnyoneCanPay partial commit count (0 = all outputs)
- field public_key: Option<PublicKey> — spender pubkey for P2PKH; None pre-P0-001 fork

#### Input methods
- fn Input::new — default All sighash, no pubkey
- fn Input::new_anyone_can_pay — AnyoneCanPay, no partial commit
- fn Input::new_anyone_can_pay_partial — AnyoneCanPay with partial output commitment
- fn Input::with_public_key — builder: attach spender pubkey
- fn Input::outpoint — return (prev_tx_hash, output_index) tuple
- fn Input::serialize_for_signing — deterministic bytes for signature construction

---

### struct Output (`crates/core/src/transaction/output.rs`)
- field output_type: OutputType
- field amount: Amount — native DOLI units, token units, or LP shares depending on output_type
- field pubkey_hash: Hash — hash of recipient's public key
- field lock_until: BlockHeight — 0 for unlocked; >0 for bonds
- field extra_data: Vec<u8> — type-specific; max era-dependent size (see max_extra_data_size)

#### Output size constants
- const BASE_EXTRA_DATA_SIZE: usize = 524_288 — 512 KB; base limit for output extra_data (Era 0)
- const MAX_EXTRA_DATA_SIZE_CAP: usize = 8_388_608 — 8 MB; hard cap at Era 4+
- const MAX_EXTRA_DATA_SIZE: usize = BASE_EXTRA_DATA_SIZE — legacy alias; use max_extra_data_size(height) for validation
- fn max_extra_data_size — era-aware extra_data limit (Era 0=512KB, doubles per era, capped Era 4+ at 8MB)

#### Output NFT constants
- const NFT_METADATA_VERSION: u8 = 1 — NFT metadata without royalties
- const NFT_METADATA_VERSION_ROYALTY: u8 = 2 — NFT metadata with royalty fields
- const NFT_METADATA_HEADER_SIZE: usize = 33 — 1B version + 32B token_id
- const NFT_ROYALTY_SIZE: usize = 34 — 32B creator_hash + 2B royalty_bps
- const MAX_ROYALTY_BPS: u16 = 5000 — max royalty 50%

#### Output fungible asset constants
- const FUNGIBLE_ASSET_VERSION: u8 = 1 — fungible asset metadata version
- const FUNGIBLE_ASSET_HEADER_SIZE: usize = 42
- const MAX_TICKER_LEN: usize = 12 — maximum fungible asset ticker length

#### Output bridge HTLC constants
- const BRIDGE_HTLC_VERSION_V1: u8 = 1 — bridge HTLC v1 (no counter_hash)
- const BRIDGE_HTLC_VERSION_V2: u8 = 2 — bridge HTLC v2 (with counter_hash)
- const BRIDGE_HTLC_CURRENT_VERSION: u8 = BRIDGE_HTLC_VERSION_V2
- const BRIDGE_HTLC_COUNTER_HASH_SIZE: usize = 32
- const BRIDGE_HTLC_HEADER_SIZE: usize = 3 — 1B version + 1B target_chain + 1B addr_len
- const BRIDGE_CHAIN_BITCOIN: u8 = 1
- const BRIDGE_CHAIN_ETHEREUM: u8 = 2
- const BRIDGE_CHAIN_MONERO: u8 = 3
- const BRIDGE_CHAIN_LITECOIN: u8 = 4
- const BRIDGE_CHAIN_CARDANO: u8 = 5
- const BRIDGE_CHAIN_BSC: u8 = 6 — Binance Smart Chain (EVM-compatible)

#### Output pool/LP constants
- const POOL_VERSION: u8 = 1
- const POOL_METADATA_SIZE: usize = 116
- const POOL_ID_DOMAIN: &[u8] = b"DOLI_POOL"
- const POOL_DEFAULT_FEE_BPS: u16 = 30 — default pool fee 0.3%
- const POOL_MAX_FEE_BPS: u16 = 1000 — max pool fee 10%
- const LP_SHARE_VERSION: u8 = 1
- const LP_SHARE_METADATA_SIZE: usize = 33 — 1B version + 32B pool_id

#### Output collateral/lending constants
- const COLLATERAL_VERSION: u8 = 1
- const COLLATERAL_METADATA_SIZE: usize = 113
- const COLLATERAL_DEFAULT_LIQUIDATION_BPS: u16 = 15000 — 150% default liquidation ratio
- const COLLATERAL_MIN_LIQUIDATION_BPS: u16 = 12000 — 120% minimum liquidation ratio
- const COLLATERAL_DEFAULT_INTEREST_BPS: u16 = 500 — 5% annual default interest rate
- const COLLATERAL_MAX_INTEREST_BPS: u16 = 5000 — 50% annual max interest rate
- const COLLATERAL_MAX_LTV_BPS: u16 = 6667 — 66.67% max LTV at creation
- const COLLATERAL_LIQUIDATION_LTV_BPS: u16 = 8333 — 83.33% liquidation threshold
- const LOAN_ID_DOMAIN: &[u8] = b"DOLI_LOAN"
- const LENDING_DEPOSIT_VERSION: u8 = 1
- const LENDING_DEPOSIT_METADATA_SIZE: usize = 37 — 1B version + 32B lending_pool_id + 4B deposit_slot
- const LENDING_POOL_ID_DOMAIN: &[u8] = b"DOLI_LENDING_POOL"

#### Output fractionalization constants
- const FRAC_MARKER: u8 = 0x46 — fractionalization marker byte ('F') appended to NFT extra_data
- const FRAC_DOMAIN: &[u8] = b"DOLI_FRAC"
- const FRAC_METADATA_SIZE: usize = 41 — 1B marker + 32B asset_id + 8B total_shares

#### Output constructors and metadata methods
- fn Output::normal — create Normal output
- fn Output::bond — Bond output; creation_slot encoded as 4-byte LE in extra_data
- fn Output::bond_creation_slot — extract creation_slot from Bond extra_data
- fn Output::conditioned — generic conditioned output builder
- fn Output::multisig — Multisig output
- fn Output::hashlock — Hashlock output
- fn Output::htlc — HTLC output
- fn Output::vesting — Vesting output (signature + timelock)
- fn Output::nft — NFT output (v1, no royalty)
- fn Output::compute_nft_token_id — BLAKE3("DOLI_NFT" || creator_hash || nonce)
- fn Output::nft_metadata — extract (token_id, content_hash) from NFT extra_data; handles v1 and v2
- fn Output::nft_with_royalty — NFT output with royalty metadata (v2)
- fn Output::nft_royalty — extract (creator_pubkey_hash, royalty_bps) for v2 NFT
- fn Output::fungible_asset — fungible asset output
- fn Output::compute_asset_id — BLAKE3("DOLI_ASSET" || tx_hash || index)
- fn Output::fungible_asset_metadata — extract (asset_id, total_supply, ticker)
- fn Output::bridge_htlc — BridgeHTLC v2 output
- fn Output::bridge_htlc_metadata — extract (target_chain, target_address, counter_hash); handles v1 and v2
- fn Output::bridge_chain_name — human-readable chain name for bridge chain ID
- fn Output::condition — decode spending condition from extra_data; None for Normal/Bond
- fn Output::is_spendable_at — true when height >= lock_until
- fn Output::serialize — deterministic bytes for hashing; u16 length for <=64KB extra_data, escape marker + u32 for >64KB
- fn Output::compute_pool_id — BLAKE3("DOLI_POOL" || min || max); canonical pair ordering
- fn Output::pool — Pool output
- fn Output::pool_metadata — decode pool extra_data into PoolMetadata
- fn Output::lp_share — LPShare output
- fn Output::lp_share_metadata — extract pool_id from LPShare extra_data
- fn Output::collateral — Collateral output; loan_addr deterministically derived
- fn Output::collateral_metadata — decode collateral extra_data
- fn Output::lending_deposit — LendingDeposit output
- fn Output::lending_deposit_metadata — decode lending deposit extra_data
- fn Output::compute_lending_pool_id — BLAKE3("DOLI_LENDING_POOL" || amm_pool_id)
- fn Output::is_fractionalized — true if NFT extra_data has valid fractionalization metadata appended
- fn Output::fractionalization_metadata — extract (fraction_asset_id, total_shares) from fractionalized NFT
- fn Output::fraction_asset_id — BLAKE3("DOLI_FRAC" || token_id)
- fn Output::build_fractionalized_extra_data — append fractionalization metadata to existing NFT extra_data
- fn Output::strip_fractionalization_metadata — remove fractionalization trailer, return original extra_data

---

### Output Metadata Structs

#### struct PoolMetadata — decoded pool output extra_data (lives in transaction/output.rs)
- field pool_id: Hash
- field asset_b_id: Hash
- field reserve_a: Amount
- field reserve_b: Amount
- field total_lp_shares: Amount
- field cumulative_price: u128
- field last_update_slot: u32
- field fee_bps: u16
- field creation_slot: u32
- field status: u8

#### struct CollateralMetadata — decoded collateral output extra_data
- field pool_id: Hash
- field borrower_hash: Hash
- field principal: Amount
- field interest_rate_bps: u16
- field creation_slot: u32
- field liquidation_ratio_bps: u16
- field collateral_asset_id: Hash

#### struct LendingDepositMetadata — decoded lending deposit output extra_data
- field lending_pool_id: Hash
- field deposit_slot: u32

---

### Transaction Data Structs (`crates/core/src/transaction/data.rs`)

- struct RegistrationData — extra_data for Registration tx; fields: public_key, epoch, vdf_output, vdf_proof, prev_registration_hash, sequence_number, bond_count, bls_pubkey (48 bytes), bls_pop (96 bytes)
- struct ExitData — extra_data for Exit tx; field: public_key
- struct ClaimData — extra_data for ClaimReward tx; field: public_key
- struct ClaimBondData — extra_data for ClaimBond tx; field: public_key
- enum SlashingEvidence — evidence of producer misbehavior; variant: DoubleProduction { block_header_1: BlockHeader, block_header_2: BlockHeader }
- struct SlashData — extra_data for SlashProducer tx; fields: producer_pubkey, evidence (SlashingEvidence), reporter_signature
- struct AddBondData — extra_data for AddBond tx; fields: producer_pubkey, bond_count
  - fn AddBondData::new — constructor
  - fn AddBondData::total_amount_for_network — network-aware total bond amount
  - fn AddBondData::total_amount — DEPRECATED; use total_amount_for_network
  - fn AddBondData::to_bytes — serialize to bytes (32B pubkey + 4B bond_count)
  - fn AddBondData::from_bytes — deserialize; returns None if < 36 bytes
- struct DelegateBondData — extra_data for DelegateBond tx; fields: delegator, delegate, bond_count
  - fn DelegateBondData::new — constructor
  - fn DelegateBondData::to_bytes — serialize (32B delegator + 32B delegate + 4B bond_count)
  - fn DelegateBondData::from_bytes — deserialize; returns None if < 68 bytes
- struct RevokeDelegationData — extra_data for RevokeDelegation tx; fields: delegator, delegate
  - fn RevokeDelegationData::new — constructor
  - fn RevokeDelegationData::to_bytes — serialize (32B delegator + 32B delegate)
  - fn RevokeDelegationData::from_bytes — deserialize; returns None if < 64 bytes
- struct WithdrawalRequestData — extra_data for RequestWithdrawal tx; fields: producer_pubkey, bond_count, destination
  - fn WithdrawalRequestData::new — constructor
  - fn WithdrawalRequestData::to_bytes — serialize (32B pubkey + 4B bond_count + 32B destination)
  - fn WithdrawalRequestData::from_bytes — deserialize; returns None if < 68 bytes
- struct EpochRewardData — extra_data for EpochReward tx; fields: epoch (u64), recipient (PublicKey)
  - fn EpochRewardData::new — constructor
  - fn EpochRewardData::to_bytes — serialize (8B epoch LE + 32B pubkey)
  - fn EpochRewardData::from_bytes — deserialize; returns None if < 40 bytes
- const ZK_ROLLUP_DATA_VERSION: u16 = 1 — current ZkRollupData layout version
- const MAX_VERIFYING_KEY_SIZE: usize = 204_800 — 200 KB max verifying key size
- const MAX_ZK_PROOF_SIZE: usize = 409_600 — 400 KB max proof size per ZKSettle transaction
- struct ZkRollupData — ZK rollup UTXO extra_data payload; fields: version, rollup_id, proof_system_id (1=Plonky2, 2=Halo2, 3=Groth16, 4=Risc0), verifying_key, state_root, metadata
  - fn ZkRollupData::new — constructor; sets version = ZK_ROLLUP_DATA_VERSION
  - fn ZkRollupData::to_bytes — explicit little-endian layout serialization (not bincode) for stable data_root
  - fn ZkRollupData::from_bytes — deserialize; minimum 76 bytes; rejects vk_len > MAX_VERIFYING_KEY_SIZE

---

### Legacy Compatibility (`crates/core/src/transaction/legacy.rs`)
- struct LegacyInput — v3.5.0 Input (prev_tx_hash, output_index, signature; no sighash_type)
  - fn LegacyInput::into_current — convert to current Input (sighash_type=All, committed_output_count=0, public_key=None)
- struct LegacyTransaction — v3.5.0 Transaction using LegacyInput
  - fn LegacyTransaction::into_current — convert all LegacyInputs to current Inputs
- struct LegacyBlock — v3.5.0 Block using LegacyTransaction
  - fn LegacyBlock::into_current — convert all LegacyTransactions; sets attestation_bitfield = Vec::new()
- struct LegacyInputV2 — v3.6.0 Input (adds sighash_type; no committed_output_count)
  - fn LegacyInputV2::into_current — convert; committed_output_count=0, public_key=None
- struct LegacyTransactionV2 — v3.6.0 Transaction using LegacyInputV2
  - fn LegacyTransactionV2::into_current — convert
- struct LegacyBlockV2 — v3.6.0 Block using LegacyTransactionV2
  - fn LegacyBlockV2::into_current — convert; sets attestation_bitfield = Vec::new()
- struct LegacyInputV3 — v3.7.1 Input (adds committed_output_count; no public_key); pre-P0-001
  - fn LegacyInputV3::into_current — convert; public_key=None
- struct LegacyTransactionV3 — v3.7.1 Transaction using LegacyInputV3
  - fn LegacyTransactionV3::into_current — convert
- struct LegacyBlockV3 — v3.7.1 Block using LegacyTransactionV3
  - fn LegacyBlockV3::into_current — convert; sets attestation_bitfield = Vec::new()
- fn deserialize_block_compat — backwards-compatible block deserialization; tries current (v5.1.0+), then LegacyBlockV3 (v3.7.1), then LegacyBlockV2 (v3.6.0), then LegacyBlock (v3.5.0)

---

### Consensus Constants (`crates/core/src/consensus/constants.rs`)

#### Genesis / Checkpoint
- const GENESIS_TIME: u64 = 1_776_332_817 — genesis timestamp (must match chainspec.mainnet.json)
- const CHECKPOINT_HEIGHT: u64 = 0 — trusted block height for fast initial sync
- const CHECKPOINT_HASH: &str — trusted block hash (64 hex zeros)
- const CHECKPOINT_STATE_ROOT: &str — state root at checkpoint height
- fn reward_pool_pubkey_hash — deterministic reward pool address (no private key)

#### Hard fork activation heights (all currently = 0)
- const EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT: u64 = 0 — EpochReward tx must include explicit pool UTXO inputs
- const BITFIELD_BODY_ACTIVATION_HEIGHT: u64 = 0 — attestation bitfield moves from header to body
- const TIER_SYSTEM_ACTIVATION_HEIGHT: u64 = 0 — active producers cap enforcement
- const ACTIVE_PRODUCERS_CAP: usize = 50 — max producers in round-robin after TIER_SYSTEM_ACTIVATION_HEIGHT
- const UNIQUE_COINBASE_ACTIVATION_HEIGHT: u64 = 0 — coinbase extra_data = height ++ slot
- const SNAP_HEADER_ACTIVATION_HEIGHT: u64 = 0 — snap sync includes anchor block header
- const TIER_PROMOTION_ACTIVATION_HEIGHT: u64 = 0 — active list sorted by attestation_count desc
- const MIN_ATTESTATION_MINUTES: usize = 30 — minimum attestation minutes to stay in active list
- const REWARDS_EPOCH_LIST_FIX_HEIGHT: u64 = 13_320 — rewards decode with epoch_state.producer_list (epoch 37 boundary)
- const FULL_BITFIELD_DECODE_HEIGHT: u64 = 14_000 — decode ALL indices including mid-epoch activated producers (Full Bitfield Decode stability pillar)

#### Protocol versioning
- const INITIAL_PROTOCOL_VERSION: u32 = 1 — initial protocol version at genesis
- fn is_protocol_active — gate check for versioned consensus code

#### Proof of Time parameters
- const SLOT_DURATION: u64 = 10 — slot duration in seconds
- const SLOTS_PER_EPOCH: u32 = 360 — slots per consensus epoch (1 hour)
- const SLOTS_PER_REWARD_EPOCH: u32 = 360 — slots per reward epoch
- const BLOCKS_PER_REWARD_EPOCH: BlockHeight = 360 — blocks per reward epoch (primary constant)
- const SLOTS_PER_YEAR: u32 = 3_153_600 — slots per year (for seniority weight)
- const MIN_PRESENCE_RATE: u32 = 50 — minimum presence rate percentage to stay active
- const MIN_ATTESTATION_RATE: u32 = MIN_PRESENCE_RATE — alias for backward compatibility

#### Epoch reward qualification
- const SLOTS_PER_ERA: BlockHeight = 12_614_400 — slots per era (~4 years, halving interval)
- const BLOCKS_PER_ERA: BlockHeight = SLOTS_PER_ERA — alias
- const HALVING_INTERVAL: BlockHeight = SLOTS_PER_ERA — alias
- const BOOTSTRAP_BLOCKS: BlockHeight = 60_480 — bootstrap phase duration (~1 week)
- const LIVENESS_WINDOW_MIN: u64 = 500 — minimum liveness window for stale detection
- const SEED_CONFIRMATION_DEPTH: u64 = 6 — seed nodes serve blocks this many deep
- const REENTRY_INTERVAL: u32 = 50 — slots between re-entry opportunities for stale producers
- const INACTIVITY_LEAK_START: u64 = 360 — missed slots before inactivity leak begins (1 epoch)
- const INACTIVITY_LEAK_RATE: u64 = 10 — effective bond decay rate per epoch (10%)
- const INACTIVITY_LEAK_FLOOR: u64 = 1 — minimum bond floor (never zeroed)
- const BOOTSTRAP_GRACE_PERIOD_SECS: u64 = 15 — wait at genesis before production
- const MAX_DRIFT: u64 = 1 — max clock drift in seconds
- const MAX_DRIFT_MS: u64 = 200 — max clock drift in milliseconds
- const NETWORK_MARGIN_MS: u64 = 200 — network margin in milliseconds
- const NETWORK_MARGIN: u64 = 1 — network margin in seconds (backward compat)
- const MAX_FUTURE_SLOTS: u64 = 1 — max slots ahead a block can be accepted
- const MAX_PAST_SLOTS: u64 = 192 — max slots behind a block can be accepted (32 minutes)

#### Emission
- const INITIAL_REWARD: Amount = 100_000_000 — initial block reward (1 DOLI = 100M base units)
- const INITIAL_BLOCK_REWARD: Amount = INITIAL_REWARD — alias
- const BLOCK_REWARD_POOL: Amount = INITIAL_REWARD — alias
- const EPOCH_REWARD_POOL: Amount = SLOTS_PER_REWARD_EPOCH * INITIAL_REWARD — epoch pool total (360 DOLI)
- const COINBASE_MATURITY: BlockHeight = 6 — confirmations before spending coinbase
- const TOTAL_SUPPLY: Amount = 2_522_880_000_000_000 — total supply (25,228,800 DOLI)

#### Block size
- const BASE_BLOCK_SIZE: usize = 2_000_000 — base block size Era 0 (2 MB)
- const MAX_BLOCK_SIZE_CAP: usize = 32_000_000 — max block size cap Era 4+ (32 MB)
- fn max_block_size — doubles per era, capped at 32 MB

#### Bond stacking
- const BOND_UNIT: Amount = 1_000_000_000 — 1 bond = 10 DOLI
- const INITIAL_BOND: Amount = BOND_UNIT — alias
- const MAX_BONDS_PER_PRODUCER: u32 = 3_000 — max bonds per producer (30,000 DOLI max)
- const YEAR_IN_SLOTS: Slot = 3_153_600 — 1 year in slots
- const VESTING_QUARTER_SLOTS: Slot = 3_153_600 — 1 vesting quarter = 1 year (mainnet)
- const VESTING_PERIOD_SLOTS: Slot = 4 * VESTING_QUARTER_SLOTS — full vesting period (4 years)
- const COMMITMENT_PERIOD: BlockHeight = VESTING_PERIOD_SLOTS — commitment period
- const UNBONDING_PERIOD: BlockHeight = 60_480 — exit delay (~7 days)
- const BOND_LOCK_BLOCKS: BlockHeight = COMMITMENT_PERIOD — lock duration
- fn withdrawal_penalty_rate — 0/25/50/75 based on vesting quarter (mainnet)
- fn withdrawal_penalty_rate_with_quarter — network-aware variant

#### Presence score
- type PresenceScore = u64 — presence score type alias
- const MIN_PRESENCE_SCORE: PresenceScore = 1
- const MAX_PRESENCE_SCORE: PresenceScore = 10_000
- const INITIAL_PRESENCE_SCORE: PresenceScore = 100
- const SCORE_PRODUCE_BONUS: PresenceScore = 1 — bonus for successful block production
- const SCORE_MISS_PENALTY: PresenceScore = 2 — penalty for missed slot

#### Failure thresholds
- const MAX_FAILURES: u32 = 50 — max consecutive missed slots before inactive
- const INACTIVITY_THRESHOLD: u32 = MAX_FAILURES — alias
- const EXCLUSION_SLOTS: Slot = 60_480 — slashing exclusion period (7 days)
- const REWARD_MATURITY: BlockHeight = 6 — reward maturity confirmations

#### Fallback timing (current, consensus-critical)
- const MAX_FALLBACK_PRODUCERS: usize = 2 — primary + single fallback
- const FALLBACK_TIMEOUT_MS: u64 = 2_000 — sequential 2s exclusive window per rank
- const MAX_FALLBACK_RANKS: usize = 2 — rank 0 = primary, rank 1 = fallback

#### Delegation
- const DELEGATE_REWARD_PCT: u32 = 10 — percentage of block reward kept by delegate
- const STAKER_REWARD_PCT: u32 = 90 — percentage distributed to delegators
- const DELEGATION_UNBONDING_SLOTS: u64 = 60_480 — delegation revocation delay (~7 days)
- const ELIGIBLE_PRODUCER_POOL: usize = 5 — eligible producer pool size

#### Fee system
- const BASE_FEE: Amount = 1 — minimum transaction fee (1 satoshi)
- const FEE_PER_BYTE: Amount = 1 — fee per byte of extra_data
- const FEE_DIVISOR: Amount = 100 — effective rate = FEE_PER_BYTE / FEE_DIVISOR = 0.01 sats/byte

---

### Consensus Parameters (`crates/core/src/consensus/params.rs`)

#### struct ConsensusParams (15 fields)
- field genesis_time: u64
- field slot_duration: u64
- field slots_per_epoch: u32
- field slots_per_reward_epoch: u32
- field attestation_interval: u32
- field min_attestation_rate: u32
- field blocks_per_era: BlockHeight
- field bootstrap_blocks: BlockHeight
- field bootstrap_grace_period_secs: u64
- field initial_reward: Amount
- field initial_bond: Amount
- field base_block_size: usize
- field max_block_size_cap: usize
- field reward_mode: RewardMode
- field genesis_hash: crypto::Hash

#### ConsensusParams methods
- fn ConsensusParams::mainnet — mainnet parameters (EpochPool, genesis from chainspec)
- fn ConsensusParams::testnet — testnet parameters
- fn ConsensusParams::devnet — devnet parameters (1s slots, 30s reward epoch, 576 blocks/era)
- fn ConsensusParams::for_network — dispatcher to mainnet/testnet/devnet
- fn ConsensusParams::apply_chainspec — apply chainspec overrides; mainnet is locked
- fn ConsensusParams::covenants_activation_height — devnet/testnet=0, mainnet=2000
- fn ConsensusParams::guards_activation_height — devnet/testnet=0, mainnet=u64::MAX
- fn ConsensusParams::max_block_size — doubles per era up to cap (5 era cap)
- fn ConsensusParams::timestamp_to_slot — 0 for pre-genesis; capped at Slot::MAX
- fn ConsensusParams::slot_to_timestamp — genesis_time + slot * slot_duration
- fn ConsensusParams::slot_to_epoch — slot / slots_per_epoch
- fn ConsensusParams::height_to_era — capped at Era::MAX
- fn ConsensusParams::block_reward — halving per era; 0 after era 63
- fn ConsensusParams::bond_amount — 70% per era (u128 arithmetic); 0 after era 20
- fn ConsensusParams::is_bootstrap — height < bootstrap_blocks
- fn ConsensusParams::slot_to_reward_epoch — slot / slots_per_reward_epoch
- fn ConsensusParams::is_reward_epoch_boundary — slot > 0 && multiple of slots_per_reward_epoch
- fn ConsensusParams::reward_epoch_start_slot — epoch * slots_per_reward_epoch
- fn ConsensusParams::reward_epoch_end_slot — (epoch+1) * slots_per_reward_epoch
- fn ConsensusParams::total_epoch_reward — block_reward * slots_per_reward_epoch
- fn ConsensusParams::for_stress_test — fast params for stress testing

---

### VDF Constants (`crates/core/src/consensus/vdf.rs`)
- const VDF_DISCRIMINANT_BITS: u32 = 1024 — discriminant bits for VDF proofs
- const T_BLOCK: u64 = 800_000 — block VDF iterations (~55ms on reference hardware)
- const T_BLOCK_BASE: u64 = T_BLOCK — legacy alias
- const T_BLOCK_CAP: u64 = T_BLOCK — max T value for blocks (fixed, no scaling)
- const VDF_TARGET_MS: u64 = 55 — VDF target duration in milliseconds
- const VDF_DEADLINE_MS: u64 = 2_000 — VDF must complete within fallback window
- const T_REGISTER_BASE: u64 = 1_000 — registration VDF iterations (fixed, ~0.07ms)
- const R_TARGET: u32 = 10 — target registrations per epoch
- const R_CAP: u32 = 100 — max registrations per epoch
- const T_REGISTER_CAP: u64 = 5_000_000 — max registration VDF time
- fn t_block — returns T_BLOCK (fixed; height param ignored)
- fn construct_vdf_input — HASH("DOLI_VDF_BLOCK_V1" || prev_hash || tx_root || slot_le || producer_key)

---

### Registration (`crates/core/src/consensus/registration.rs`)
- const MAX_REGISTRATIONS_PER_BLOCK: u32 = 5 — anti-spam block limit
- const BASE_REGISTRATION_FEE: Amount = 100_000 — 0.001 DOLI base fee
- const MAX_FEE_MULTIPLIER_X100: u32 = 1000 — 10x cap
- const MAX_REGISTRATION_FEE: Amount = BASE_REGISTRATION_FEE * 10 — 0.01 DOLI absolute cap
- fn fee_multiplier_x100 — const fn; deterministic 8-tier table: 0-4→100, 5-9→150, 10-19→200, 20-49→300, 50-99→450, 100-199→650, 200-299→850, 300+→1000
- fn registration_fee — BASE_FEE * multiplier / 100, capped at MAX
- fn registration_fee_for_network — per-network base fee, same 10x cap
- struct PendingRegistration — fields: public_key, bond_amount, fee_paid, submitted_at, prev_registration_hash, sequence_number
- struct RegistrationQueue — FIFO queue with per-block limit enforcement
  - fn RegistrationQueue::new — empty queue
  - fn RegistrationQueue::current_fee — current registration fee
  - fn RegistrationQueue::current_fee_for_network — network-specific fee
  - fn RegistrationQueue::pending_count — pending.len()
  - fn RegistrationQueue::can_add_to_block — current_block_count < MAX_REGISTRATIONS_PER_BLOCK
  - fn RegistrationQueue::can_add_to_block_for_network — uses network limit
  - fn RegistrationQueue::submit — checks fee, pushes to pending
  - fn RegistrationQueue::submit_for_network — network-aware fee check
  - fn RegistrationQueue::begin_block — resets current_block_count
  - fn RegistrationQueue::next_registration — FIFO, respects MAX_REGISTRATIONS_PER_BLOCK
  - fn RegistrationQueue::next_registration_for_network — network-aware limit
  - fn RegistrationQueue::mark_processed — returns fee_paid (caller burns if invalid)
  - fn RegistrationQueue::pending_registrations — immutable slice
  - fn RegistrationQueue::prune_expired — removes aged registrations
  - fn RegistrationQueue::clear — clear queue and counter

---

### Bonds (`crates/core/src/consensus/bonds.rs`)

#### struct BondEntry
- field creation_slot: Slot
- field amount: Amount
- fn BondEntry::new — creates bond with BOND_UNIT amount
- fn BondEntry::age — saturating_sub(creation_slot)
- fn BondEntry::penalty_rate — uses mainnet VESTING_QUARTER_SLOTS
- fn BondEntry::penalty_rate_with_quarter — network-aware
- fn BondEntry::withdrawal_amount — (net, penalty), mainnet quarter
- fn BondEntry::withdrawal_amount_with_quarter — network-aware
- fn BondEntry::is_vested — mainnet quarter
- fn BondEntry::is_vested_with_quarter — network-aware

#### struct ProducerBonds
- field bonds: Vec<BondEntry> — sorted by creation_slot, oldest first
- fn ProducerBonds::new — empty bond holdings
- fn ProducerBonds::bond_count — bonds.len() as u32
- fn ProducerBonds::total_staked — bonds.len() * BOND_UNIT
- fn ProducerBonds::selection_weight — same as bond_count()
- fn ProducerBonds::add_bonds — checks MAX_BONDS_PER_PRODUCER
- fn ProducerBonds::request_withdrawal — FIFO, mainnet quarter
- fn ProducerBonds::request_withdrawal_with_quarter — FIFO, network-aware
- fn ProducerBonds::maturity_summary — mainnet quarter
- fn ProducerBonds::maturity_summary_with_quarter — network-aware
- fn ProducerBonds::total_withdrawal_penalty — mainnet quarter
- fn ProducerBonds::total_withdrawal_penalty_with_quarter — network-aware

#### struct WithdrawalResult
- field bond_count: u32
- field net_amount: Amount
- field penalty_amount: Amount
- field destination: crypto::Hash

#### struct BondsMaturitySummary — quarter-based summary
- field q1: u32 — bonds in Q1 (75% penalty)
- field q2: u32 — bonds in Q2 (50% penalty)
- field q3: u32 — bonds in Q3 (25% penalty)
- field vested: u32 — fully vested bonds (0% penalty)

#### enum BondError (5 variants)
- MaxBondsExceeded { current, requested, max }
- InsufficientBonds { requested, available }
- ZeroWithdrawal
- NoClaimableWithdrawal
- InvalidAmount { amount }

---

### Exit & Slashing (`crates/core/src/consensus/exit.rs`)

#### enum PenaltyDestination
- Burn — default; penalty is burned (deflationary)
- RewardPool — deprecated, legacy compatibility

#### enum RewardMode
- DirectCoinbase — reward goes directly to producer per block
- EpochPool — default; rewards accumulate and distribute at epoch boundary

#### struct ExitTerms
- field return_amount: Amount
- field penalty_amount: Amount
- field penalty_destination: PenaltyDestination
- field is_early_exit: bool
- field commitment_percent: u8

#### struct SlashResult
- field burned_amount: Amount
- field excluded: bool

#### Free functions
- fn calculate_exit — legacy single-bond API, mainnet quarter
- fn calculate_exit_with_quarter — network-aware
- fn calculate_slash — 100% burned, excluded=true

---

### ProducerState (`crates/core/src/consensus/producer_state.rs`)
- struct ProducerState — fields: pubkey_hash, presence_score, blocks_produced, blocks_missed, last_produced_slot, registered_slot
  - fn ProducerState::new — INITIAL_PRESENCE_SCORE, all counters zero
  - fn ProducerState::presence_rate — (blocks_produced * 100) / total; 100 if no blocks
  - fn ProducerState::meets_minimum — presence_rate() >= MIN_PRESENCE_RATE
  - fn ProducerState::record_produced — increments blocks_produced, updates score
  - fn ProducerState::record_missed — increments blocks_missed, decreases score
  - fn ProducerState::is_active — checks last_produced_slot or registered_slot

---

### Reward Epoch Utilities (`crates/core/src/consensus/reward_epoch.rs` — pub mod)
- fn from_height — height / BLOCKS_PER_REWARD_EPOCH
- fn boundaries — (start, exclusive_end)
- fn is_complete — current_height >= end
- fn current — alias for from_height
- fn last_complete — None if epoch 0; Some(current_epoch - 1)
- fn is_epoch_start — height.is_multiple_of(BLOCKS_PER_REWARD_EPOCH)
- fn blocks_per_epoch — returns BLOCKS_PER_REWARD_EPOCH constant
- fn complete_epochs — 0 if height < BLOCKS_PER_REWARD_EPOCH; else from_height
- fn from_height_with — network-aware variant (blocks_per_epoch param)
- fn boundaries_with — network-aware variant
- fn is_complete_with — network-aware variant
- fn last_complete_with — network-aware variant
- fn is_epoch_start_with — network-aware variant
- fn complete_epochs_with — network-aware variant

---

### Stress Test (`crates/core/src/consensus/stress.rs`)
- struct StressTestParams — fields: producer_count, slot_duration_secs, vdf_iterations, bond_per_producer, block_reward
  - fn StressTestParams::extreme_600 — 600 producers, 1s slots, 100K VDF, 1 DOLI bond
  - fn StressTestParams::with_producers — N producers, 2s slots, 500K VDF, 1 DOLI bond
  - fn StressTestParams::expected_blocks_per_producer_per_hour
  - fn StressTestParams::expected_reward_per_producer_per_hour
  - fn StressTestParams::total_bond_locked
  - fn StressTestParams::expected_time_between_blocks_secs
  - fn StressTestParams::network_efficiency
  - fn StressTestParams::slots_for_majority_attack
  - fn StressTestParams::summary — formatted multi-section report string
- fn ConsensusParams::for_stress_test — fast params: 60 slots/epoch, 10K blocks/era, 10 bootstrap blocks

---

### Selection (deprecated) (`crates/core/src/consensus/selection.rs`)
All items are marked deprecated — use DeterministicScheduler for consensus-critical code.
- fn select_producer_for_slot — DEPRECATED; sorts by pubkey, ticket_index = slot % total_tickets
- fn eligible_rank_at_ms — DEPRECATED
- fn is_rank_eligible_at_ms — DEPRECATED
- fn is_producer_eligible_ms — DEPRECATED
- fn allowed_producer_rank — DEPRECATED
- fn allowed_producer_rank_ms — DEPRECATED
- fn is_producer_eligible — DEPRECATED
- fn get_producer_rank — DEPRECATED

---

### Scheduler (`crates/core/src/scheduler.rs`)
- const MAX_FALLBACK_RANK: usize — maximum fallback rank index (= MAX_FALLBACK_RANKS - 1 = 1)

#### struct ScheduledProducer
- field pubkey: PublicKey
- field bond_units: u32
- fn ScheduledProducer::new — direct constructor
- fn ScheduledProducer::from_bond_amount — converts raw bond amount to units by integer division

#### struct DeterministicScheduler — epoch-frozen weighted round-robin
- fn DeterministicScheduler::new — sorts by pubkey, filters zero-bond, computes ticket_boundaries and total_bonds
- fn DeterministicScheduler::empty — creates empty scheduler
- fn DeterministicScheduler::producer_count — number of active producers
- fn DeterministicScheduler::total_bonds — total bond units
- fn DeterministicScheduler::is_empty — true when no producers
- fn DeterministicScheduler::producers — immutable slice of all scheduled producers
- fn DeterministicScheduler::select_producer — selects producer for a slot at a given rank; applies evenly-distributed offset total_bonds * rank / MAX_FALLBACK_RANKS; returns None if empty or rank > MAX_FALLBACK_RANK
- fn DeterministicScheduler::eligible_producers — returns exactly the one producer whose exclusive 2s window matches elapsed time
- fn DeterministicScheduler::producer_rank — returns the rank (0..=MAX_FALLBACK_RANK) at which a producer is scheduled; None if not in top ranks
- fn DeterministicScheduler::is_producer_eligible — second-precision eligibility check
- fn DeterministicScheduler::is_producer_eligible_ms — millisecond-precision eligibility check
- fn DeterministicScheduler::slots_until_next — slots until the producer's next primary (rank 0) slot; None if not in scheduler
- fn DeterministicScheduler::stats — returns SchedulerStats

#### struct SchedulerStats
- field producer_count: usize
- field total_bonds: u64
- field min_bonds: u32
- field max_bonds: u32
- field avg_bonds: f64

---

### Epoch State (`crates/core/src/epoch_state.rs`)

#### struct EpochDerivationInput
- field active_producers: Vec<PublicKey>
- field bond_counts: HashMap<Hash, u64>
- field blocks_per_epoch: u64
- field snap_attestation_skip_height: u64
- field height: u64
- field epoch: u64
- field registered_at: HashMap<PublicKey, u64>

#### struct BlockAccumulationInput
- field producer: PublicKey
- field slot: u32
- field has_attestation_data: bool
- field attested_indices: Vec<usize>

#### struct EpochState
- field epoch: u64
- field bond_snapshot: HashMap<Hash, u64> — epoch-locked bond snapshot
- field producer_list: Vec<PublicKey> — frozen, attestation-filtered producer list; used for bitfield index alignment
- field active_list: Vec<PublicKey> — subset entering round-robin; first ACTIVE_PRODUCERS_CAP by registered_at after TIER_SYSTEM_ACTIVATION_HEIGHT
- field attested_sets: [HashSet<PublicKey>; 3] — rolling 3-epoch attestation sets
- field attestation_accum: [HashMap<PublicKey, HashSet<u32>>; 3] — incremental attestation tracker
- field blocks_produced: HashMap<PublicKey, u32>
- fn EpochState::genesis — creates genesis epoch state (epoch=0, all collections empty)
- fn EpochState::accumulate_block — called for every applied block; updates attestation state
- fn EpochState::derive_at_boundary — canonical pure function for epoch transitions (NOT derive_next_epoch)
- fn EpochState::serialize — bincode serialization; NOTE: HashMap/HashSet iteration order non-deterministic; not for cross-node byte comparison
- fn EpochState::deserialize — bincode deserialization
- fn EpochState::hash — deterministic fingerprint via epoch_state_hash(); sorts all HashMap/HashSet entries before hashing

#### Free functions
- fn epoch_state_hash — deterministic hash over all EpochState fields; sorts before hashing; extracted so snapshot.rs can call without an EpochState instance

---

### Network (`crates/core/src/network/`)

#### enum Network (repr u32)
- Mainnet = 1 — production network
- Testnet = 2 — public test network
- Devnet = 99 — local development network

#### Network methods
- fn Network::id — return numeric network ID
- fn Network::params — load (cached) NetworkParams for this network
- fn Network::name — "mainnet" / "testnet" / "devnet"
- fn Network::address_prefix — bech32m HRP: "doli" / "tdoli" / "ddoli"
- fn Network::magic_bytes — P2P protocol magic bytes (4 bytes, unique per network)
- fn Network::default_p2p_port — delegates to params()
- fn Network::default_rpc_port — delegates to params()
- fn Network::default_metrics_port — delegates to params()
- fn Network::data_dir_name — data directory suffix
- fn Network::is_test — true for Testnet or Devnet
- fn Network::all — returns slice [Mainnet, Testnet, Devnet]
- fn Network::from_id — parse from numeric ID (1, 2, 99 → Some; else None)
- fn Network::genesis_time — genesis Unix timestamp
- fn Network::initial_bond — initial producer bond = 1 bond unit
- fn Network::bond_unit — minimum bond granularity (10 DOLI mainnet, 1 DOLI testnet/devnet)
- fn Network::initial_reward — per-block reward in base units
- fn Network::genesis_blocks — genesis phase length in blocks
- fn Network::is_in_genesis — true if genesis_blocks > 0 && height <= genesis_blocks
- fn Network::automatic_genesis_bond — bond auto-assigned to genesis producers at genesis+1
- fn Network::coinbase_maturity — blocks until coinbase is spendable
- fn Network::max_registrations_per_block — anti-spam limit
- fn Network::registration_base_fee — base fee for registration
- fn Network::max_registration_fee — registration fee cap
- fn Network::slot_duration — slot duration in seconds
- fn Network::bootstrap_blocks — initial bootstrap phase block count
- fn Network::bootstrap_grace_period_secs — wait time at genesis before block production begins
- fn Network::slots_per_reward_epoch — slot-based epoch length (legacy)
- fn Network::blocks_per_reward_epoch — block-height-based epoch length (primary constant)
- fn Network::epoch_reward_pool — total rewards per epoch
- fn Network::bootstrap_nodes — default P2P bootstrap multiaddrs
- fn Network::bootnode_enrs — default Discv5 bootnode ENRs
- fn Network::blocks_per_year — simulated year in blocks (3,153,600 mainnet/testnet; 144 devnet)
- fn Network::blocks_per_month — blocks_per_year / 12
- fn Network::blocks_per_era — blocks_per_year * 4
- fn Network::commitment_period — 4 years in blocks
- fn Network::exit_history_retention — 8 years in blocks
- fn Network::inactivity_threshold — blocks without activity before penalty
- fn Network::unbonding_period — blocks between withdrawal request and claim
- fn Network::seniority_maturity_blocks — blocks to reach full 4x vote weight
- fn Network::seniority_step_blocks — blocks per seniority step (1 year)
- fn Network::vdf_enabled — true for all networks (hash-chain VDF, devnet uses fast params)
- fn Network::vdf_iterations — block production VDF iteration count
- fn Network::vdf_discriminant_bits — class group discriminant size: 2048 mainnet/testnet, 256 devnet
- fn Network::vdf_seed — network-unique seed for discriminant generation
- fn Network::vdf_params — cached VdfParams
- fn Network::vdf_target_time_ms — target VDF proof time in ms: 55 for all networks
- fn Network::heartbeat_vdf_iterations — VDF iterations for heartbeat proofs (~800K = 55ms)
- fn Network::vdf_register_iterations — VDF iterations for registration proof (1000 = 0.07ms)
- fn Network::veto_period_secs — window in which maintainers can veto an update
- fn Network::grace_period_secs — window after veto expires before enforcement starts
- fn Network::min_voting_age_secs — minimum producer registration age before voting
- fn Network::min_voting_age_blocks — min_voting_age_secs converted to blocks
- fn Network::update_check_interval_secs — interval between auto-update checks
- fn Network::crash_window_secs — window for crash counting that triggers rollback
- fn Network::crash_threshold — hardcoded 3 for all networks
- fn Network::veto_period_blocks — veto_period_secs / slot_duration

- struct NetworkParseError(String) — error returned when parsing an unknown network name string
- impl FromStr for Network — parses "mainnet"/"main", "testnet"/"test", "devnet"/"dev"/"local" (case-insensitive)
- impl Display for Network — formats as self.name()

---

### NetworkParams (`crates/core/src/network_params/`)
- struct NetworkParams — 43 fields covering networking, timing, economics, VDF, updates, gossip mesh, hard fork gates, fallback timing, vesting, presence
  - Networking: default_p2p_port, default_rpc_port, default_metrics_port, bootstrap_nodes, bootnode_enrs, max_peers
  - Timing: slot_duration, genesis_time, veto_period_secs, grace_period_secs, bootstrap_grace_period_secs, unbonding_period, inactivity_threshold
  - Economics: bond_unit, initial_reward, registration_base_fee, max_registration_fee, automatic_genesis_bond, genesis_blocks
  - VDF (locked mainnet): vdf_iterations, heartbeat_vdf_iterations, vdf_register_iterations
  - Time structure: blocks_per_year, blocks_per_reward_epoch, coinbase_maturity, slots_per_reward_epoch, bootstrap_blocks
  - Update system: min_voting_age_secs, update_check_interval_secs, crash_window_secs, max_registrations_per_block
  - Presence (telemetry): presence_window_ms
  - Fallback timing (locked mainnet): fallback_timeout_ms, max_fallback_ranks, network_margin_ms
  - Vesting (locked mainnet): vesting_quarter_slots
  - Hard fork gates: sig_verification_height, snap_attestation_skip_height, inc_i_026_scheduler_activation_height, fork_id_activation_height
  - Gossip mesh (locked mainnet): mesh_n, mesh_n_low, mesh_n_high, gossip_lazy
- fn NetworkParams::load — load (and cache via OnceLock) params for given network
- fn NetworkParams::defaults — return fully-populated hardcoded defaults for network
- fn NetworkParams::blocks_per_month — derived: blocks_per_year / 12
- fn NetworkParams::blocks_per_era — derived: blocks_per_year * 4
- fn NetworkParams::commitment_period — derived: same as blocks_per_era()
- fn NetworkParams::exit_history_retention — derived: blocks_per_era() * 2
- fn NetworkParams::seniority_maturity_blocks — derived: blocks_per_year * 4
- fn NetworkParams::seniority_step_blocks — derived: blocks_per_year
- fn NetworkParams::min_voting_age_blocks — derived: min_voting_age_secs / slot_duration
- fn NetworkParams::veto_period_blocks — derived: veto_period_secs / slot_duration
- fn load_env_for_network — load {data_dir}/.env into process env; fallback to ~/.doli/{network_name}/.env
- fn get_default_data_dir — returns ~/.doli/{network_name}
- fn init_env_for_network — convenience: get_default_data_dir + load_env_for_network
- fn apply_chainspec_defaults — load chainspec JSON and set env vars only if not already set; skipped for Mainnet

---

### Config Validation (`crates/core/src/config_validation.rs`)
- enum LockedParam — 13 variants identifying mainnet-locked configuration parameters: SlotDuration, GenesisTime, BondUnit, InitialReward, UnbondingPeriod, VdfIterations, HeartbeatVdfIterations, VdfRegisterIterations, BlocksPerYear, BlocksPerRewardEpoch, CoinbaseMaturity, AutomaticGenesisBond, GenesisBlocks
  - fn LockedParam::env_var — return the environment variable name for this locked parameter
- const MAINNET_LOCKED_PARAMS: &[LockedParam] — complete slice of all 13 locked parameters for mainnet
- fn check_locked_params — check for any attempted overrides of locked mainnet parameters via environment variables
- fn validate_params — validate that loaded NetworkParams values are within acceptable ranges

---

### Chainspec (`crates/core/src/chainspec.rs`)
- struct ChainSpec — fields: name, id, network, genesis (GenesisSpec), consensus (ConsensusSpec), genesis_producers (Vec<GenesisProducer>)
  - fn ChainSpec::load — read JSON file, deserialize, validate
  - fn ChainSpec::save — pretty-print JSON to file
  - fn ChainSpec::validate — validate genesis producer pubkeys and consensus params
  - fn ChainSpec::get_genesis_producers — decode hex pubkeys into (PublicKey, bond_count) pairs
  - fn ChainSpec::genesis_hash — BLAKE3(timestamp_le || network_id_le || slot_duration_le || message_bytes)
  - fn ChainSpec::has_genesis_producers — true if genesis_producers is non-empty
  - fn ChainSpec::mainnet — built-in mainnet spec
  - fn ChainSpec::testnet — built-in testnet spec
  - fn ChainSpec::devnet — built-in devnet spec
- struct GenesisSpec — fields: timestamp: u64, message: String, initial_reward: u64
- struct ConsensusSpec — fields: slot_duration: u64, slots_per_epoch: u32, bond_amount: u64 (NOTE: NOT blocks_per_reward_epoch, max_block_size, vdf_iterations, or genesis_time — those were spec errors)
- struct GenesisProducer — fields: name: String, public_key: String (hex), bond_count: u32 (default=1)
- enum ChainSpecError — 6 variants: IoError, ParseError, SerializeError, InvalidPubkey, PlaceholderKey, InvalidParam

---

### Genesis (`crates/core/src/genesis.rs`)
- struct GenesisConfig — fields: network: Network, timestamp: u64, reward: Amount, message: &'static str
  - fn GenesisConfig::mainnet — mainnet; message = "Time is the only fair currency. 25/Feb/2026"
  - fn GenesisConfig::testnet — testnet
  - fn GenesisConfig::devnet — devnet; timestamp = 0 (dynamic)
  - fn GenesisConfig::for_network — dispatch to mainnet/testnet/devnet by variant
- enum GenesisError — 9 variants: InvalidSlot, InvalidPrevHash, InvalidProducer, InvalidTransactionCount, NotCoinbase, CoinbaseHasInputs, InvalidOutputCount, InvalidTimestamp, InvalidReward
- const GENESIS_PUBKEY: [u8; 32] — all-zeros unspendable public key used as genesis coinbase producer
- const NULL_HASH: [u8; 32] — all-zeros hash used as prev_hash for genesis block
- const MAINNET_GENESIS_PRODUCERS: &[(&str, u32)] — 5 entries; N1-N5 producer pubkeys and bond counts
- const TESTNET_GENESIS_PRODUCERS: &[(&str, u32)] — 5 entries; NT1-NT5 producer pubkeys and bond counts
- fn mainnet_genesis_producers — decode MAINNET_GENESIS_PRODUCERS hex strings into (PublicKey, bond_count) pairs
- fn testnet_genesis_producers — decode TESTNET_GENESIS_PRODUCERS hex strings into (PublicKey, bond_count) pairs
- fn generate_genesis_block — build deterministic genesis block
- fn genesis_hash — compute genesis block hash for a network; NOTE: Devnet is non-deterministic (dynamic timestamp)
- fn verify_genesis_block — validate slot==0, prev_hash==NULL_HASH, producer==GENESIS_PUBKEY, exactly 1 tx, tx is coinbase, no inputs, exactly 1 output, timestamp and reward match config

---

### Attestation (`crates/core/src/attestation.rs`)

#### struct Attestation
- field block_hash: Hash
- field slot: u32
- field height: u64
- field attester: PublicKey
- field attester_weight: u64
- field signature: Signature
- field bls_signature: Option<Vec<u8>> — 96-byte BLS signature
- fn Attestation::new — create and sign with Ed25519 only
- fn Attestation::new_with_bls — create and sign with Ed25519 + BLS
- fn Attestation::verify — verify the Ed25519 attestation signature
- fn Attestation::to_bytes — serialize for gossip transmission (bincode)
- fn Attestation::from_bytes — deserialize from gossip bytes

#### struct RegionAggregate
- field block_hash: Hash
- field slot: u32
- field region: u32
- field attester_count: usize
- field total_weight: u64
- fn RegionAggregate::from_attestations — build aggregate from individual attestations; all must be for the same block_hash
- fn RegionAggregate::verify — verify all Ed25519 signatures
- fn RegionAggregate::attestation_weight — total attestation weight

#### struct MinuteAttestationTracker — in-memory tracker for gossip attestations; NOT for epoch reward qualification
- fn MinuteAttestationTracker::new — create empty tracker
- fn MinuteAttestationTracker::record — record that a producer attested in a given minute
- fn MinuteAttestationTracker::record_with_bls — record attestation with a BLS signature
- fn MinuteAttestationTracker::fingerprint — deterministic fingerprint of the attested map for cross-node divergence detection
- fn MinuteAttestationTracker::total_entries — total count of (pubkey, minute) entries
- fn MinuteAttestationTracker::attested_in_minute — get all producers that attested in a specific minute
- fn MinuteAttestationTracker::bls_sigs_for_minute — get BLS signatures for all producers in a specific minute
- fn MinuteAttestationTracker::bls_sig_count — total BLS signatures stored
- fn MinuteAttestationTracker::reset — clear all attestation and BLS signature data

#### enum AttestationError — 3 variants: InvalidSignature, BlockMismatch, EmptyAttestations

#### Constants
- const SLOTS_PER_ATTESTATION_MINUTE: u32 = 6 — 6 slots × 10s = 60s
- const ATTESTATION_MINUTES_PER_EPOCH: u32 = 60 — attestation minutes per epoch (mainnet default)
- const ATTESTATION_QUALIFICATION_THRESHOLD: u32 = 54 — 90% of 60 minutes (mainnet default)

#### Free functions
- fn attestation_minutes_per_epoch — compute attestation minutes per epoch from blocks_per_epoch; testnet-aware
- fn attestation_qualification_threshold — compute 90% qualification threshold from blocks_per_epoch
- fn attestation_minute — compute the attestation minute from a slot number
- fn encode_attestation_bitfield — encode attestation bitfield into a Hash for presence_root; supports up to 256 producers
- fn encode_attestation_bitfield_vec — encode attestation bitfield into a Vec<u8> with no 256-producer cap; used for post-BITFIELD_BODY_ACTIVATION_HEIGHT blocks
- fn decode_attestation_bitfield_vec — decode attestation bitfield from a Vec<u8>
- fn validate_attestation_bitfield_vec — validate body attestation bitfield
- fn decode_attestation_bitfield — decode attestation bitfield from presence_root Hash
- fn validate_attestation_bitfield — validate presence_root bitfield has no bits set beyond producer_count

---

### Finality (`crates/core/src/finality.rs`)

#### struct FinalityCheckpoint
- field block_hash: Hash
- field height: u64
- field slot: u32
- field attestation_weight: u64
- field total_weight: u64
- fn FinalityCheckpoint::is_finalized — check if checkpoint has reached finality threshold (67%)

#### struct FinalityTracker
- fn FinalityTracker::new — create new finality tracker
- fn FinalityTracker::track_block — start tracking a new block for finality; applies buffered early attestations
- fn FinalityTracker::add_attestation_weight — add attestation weight to a pending block
- fn FinalityTracker::check_finality — check if any pending blocks have reached finality; returns highest finalized checkpoint
- fn FinalityTracker::is_at_or_below_finalized — check if a given height is at or below last finalized height
- fn FinalityTracker::prune_old_pending — prune pending blocks older than a given slot

#### Constants
- const FINALITY_THRESHOLD_PCT: u32 = 67 — percentage of total weight required for finality
- const FINALITY_TIMEOUT_SLOTS: u32 = 3 — slots to wait before timing out pending finality

---

### Conditions (Covenants) (`crates/core/src/conditions/`)

#### enum Condition (11 variants)
- Signature(Hash) — requires valid signature from key whose pubkey_hash matches
- Multisig { threshold: u8, keys: Vec<Hash> } — requires threshold-of-N valid signatures
- Hashlock(Hash) — requires revealing a 32-byte preimage whose BLAKE3(DOLI_HASHLOCK, preimage) matches
- Timelock(BlockHeight) — spendable only at or after min_height
- TimelockExpiry(BlockHeight) — spendable only after expiry height is reached (refund path for HTLC)
- And(Box<Condition>, Box<Condition>) — both sub-conditions must be satisfied
- Or(Box<Condition>, Box<Condition>) — at least one sub-condition must be satisfied
- Threshold { n: u8, conditions: Vec<Condition> } — at least n of the sub-conditions must be satisfied
- AmountGuard { min_amount: Amount, output_index: u8 } — spending tx output[output_index].amount >= min_amount
- OutputTypeGuard { expected_type: OutputType, output_index: u8 } — output type equality check
- RecipientGuard { expected_pubkey_hash: Hash, output_index: u8 } — output recipient check

#### Condition constants
- const CONDITION_VERSION: u8 = 1 — version byte prefix for all encoded conditions
- const MAX_CONDITION_OPS: usize = 128 — max cryptographic operations in a condition tree (DoS guard)
- const MAX_CONDITION_DEPTH: usize = 4 — max nesting depth for And/Or/Threshold
- const MAX_MULTISIG_KEYS: usize = 127 — max keys in a Multisig condition
- const MAX_THRESHOLD_CONDITIONS: usize = 5 — max sub-conditions in a Threshold
- const HASHLOCK_DOMAIN: &[u8] = b"DOLI_HASHLOCK" — domain separator for hashlock preimage hashing
- const ADDRESS_DOMAIN — re-exported from crypto::ADDRESS_DOMAIN

#### Condition constructors
- fn Condition::signature — create Signature condition
- fn Condition::multisig — create Multisig condition
- fn Condition::hashlock — create Hashlock from pre-hashed value
- fn Condition::hashlock_from_preimage — create Hashlock by hashing the preimage with HASHLOCK_DOMAIN
- fn Condition::timelock — create Timelock condition
- fn Condition::timelock_expiry — create TimelockExpiry condition
- fn Condition::htlc — composite HTLC: Or(And(Hashlock, Timelock), TimelockExpiry)
- fn Condition::vesting — composite vesting: And(Signature, Timelock)
- fn Condition::amount_guard — create AmountGuard condition
- fn Condition::output_type_guard — create OutputTypeGuard condition
- fn Condition::recipient_guard — create RecipientGuard condition

#### Condition encoding/decoding methods
- fn Condition::encode — encode condition to extra_data bytes with version prefix
- fn Condition::decode — decode condition from extra_data bytes with version prefix
- fn Condition::decode_prefix — decode condition from start of byte slice allowing trailing data; returns (condition, bytes_consumed); used for NFT/FungibleAsset/BridgeHTLC outputs
- fn Condition::contains_guard — true if condition tree contains any AmountGuard, OutputTypeGuard, or RecipientGuard
- fn Condition::ops_count — count cryptographic operations; Timelock/TimelockExpiry/Guards count as 0
- fn Condition::validate — validate condition tree for ops count, depth, and structural integrity

#### enum ConditionError (14 variants)
- BufferTooShort, UnsupportedVersion, UnknownTag, InvalidThreshold, TooManyKeys, TooManyConditions, ThresholdExceedsCount, TooManyOperations, TooDeep, EncodingTooLarge, TrailingBytes, ZeroThreshold, InvalidTimelockRange, InvalidPublicKey

#### struct EvalContext
- field current_height: BlockHeight
- field signing_hash: &Hash
- field transaction: Option<&Transaction> — None for legacy contexts; guard conditions return false when None

#### fn evaluate — evaluate a condition tree against witness data; or_branch_idx: &mut usize tracks consumption of witness.or_branches across nested Or conditions

#### struct Witness
- field signatures: Vec<WitnessSignature>
- field preimage: Option<[u8; 32]>
- field or_branches: Vec<bool>
- fn Witness::encode — encode witness data into bytes with WITNESS_VERSION prefix
- fn Witness::decode — decode witness data; empty bytes returns default Witness

#### struct WitnessSignature
- field pubkey: PublicKey
- field signature: Signature

#### const MAX_WITNESS_SIZE: usize = 1024 — maximum witness size in bytes

---

### Validation (`crates/core/src/validation/`)

#### enum ValidationError (50 variants)
Includes: GenesisHashMismatch, ForkIdMismatch, InvalidVersion, InvalidTimestamp, TimestampTooFuture, InvalidSlot, SlotNotAdvancing, SlotTooFuture, SlotTooPast, InvalidMerkleRoot, InvalidDataRoot, InvalidVdfProof, InvalidProducer, BlockTooLarge, MissingCoinbase, InvalidCoinbase, InvalidBlock, InvalidTransaction, DoubleSpend, InsufficientFunds, InvalidSignature, OutputLocked, OutputNotFound, OutputAlreadySpent, AmountOverflow, AmountExceedsSupply, InvalidRegistration, PubkeyHashMismatch, InvalidBond, InvalidClaim, InvalidBondClaim, InvalidSlash, InvalidAddBond, InvalidWithdrawalRequest, InvalidClaimWithdrawal, InvalidMintAsset, InvalidBurnAsset, InvalidEpochReward, UnexpectedEpochReward, MissingEpochReward, EpochRewardMismatch, InvalidMaintainerChange, InvalidDelegation, InvalidProtocolActivation, InsufficientFee, InvalidPool, InvalidSwap, InvalidLiquidity, InvalidFractionalization, InvalidRedemption, MissingPublicKey

#### enum ValidationMode
- Full — full validation including VDF proof verification; used for gossip blocks
- Light — skips VDF proof verification; used for gap blocks after snap sync

#### struct UtxoInfo — information about an unspent transaction output
- field output: Output
- field pubkey: Option<PublicKey>
- field spent: bool

#### struct RegistrationChainState — anti-Sybil chain state for chained VDF verification
- field last_registration_hash: Hash
- field registration_sequence: u64
- fn RegistrationChainState::new — create new registration chain state
- fn RegistrationChainState::expected_prev_hash — get expected prev_registration_hash for next registration
- fn RegistrationChainState::expected_sequence — get expected sequence number for next registration

#### struct ValidationContext (20+ fields for block/tx validation)
- fn ValidationContext::new — create with defaults
- fn ValidationContext::with_inc_i_026_scheduler_activation_height — builder
- fn ValidationContext::with_epoch_producer_list — builder: set epoch-frozen producer list
- fn ValidationContext::with_prev_block — builder: set previous block slot/timestamp/hash
- fn ValidationContext::with_producers — builder: set active producers (legacy)
- fn ValidationContext::with_producers_weighted — builder: set active producers with weights
- fn ValidationContext::with_bootstrap_producers — builder: set bootstrap producers sorted by pubkey
- fn ValidationContext::with_bootstrap_liveness — builder: set liveness split for bootstrap producers
- fn ValidationContext::with_registration_chain — builder: set registration chain state
- fn ValidationContext::with_pending_producer_keys — builder: set pending producer keys
- fn ValidationContext::with_sig_verification_height — builder: set sig_verification_height
- fn ValidationContext::with_fork_id — builder: set fork_id enforcement parameters

#### trait UtxoProvider
- fn get_utxo — look up an unspent output; returns None if doesn't exist or spent

#### Public validation functions
- fn validate_block — validate a complete block in Full mode
- fn validate_block_with_mode — validate a block with explicit mode; Full mode: parallel VDF pre-verification for Registration and SlashProducer; Light mode: skips VDF, MAX_FUTURE_SLOTS, MAX_PAST_SLOTS, timestamp-too-future, and producer eligibility checks
- fn validate_header — validate a block header
- fn validate_transaction — structural validation of a transaction without UTXO access
- fn validate_transaction_skip_registration_vdf — same as validate_transaction but skips VDF verification for Registration and SlashProducer
- fn validate_transaction_with_utxos — full UTXO-context validation
- fn validate_producer_eligibility — dispatches producer validation: bootstrap → bootstrap_fallback_order; epoch list → round-robin slot % n
- fn bootstrap_fallback_order — deterministic fallback rank order for bootstrap scheduling
- fn bootstrap_schedule_with_liveness — liveness-filtered bootstrap schedule

#### ZK Verification
- struct ZkVerifyContext — fields: budget_us_remaining, proof_system_id, height
- enum ZkVerifyError — 6 variants: NotYetActivated, UnsupportedProofSystem, VerifyingKeyMalformed, ProofTooLarge, InvalidProof, BudgetExceeded
- fn verify_zk_proof — verify a zero-knowledge proof; gated by ZK_SETTLE_ACTIVATION_HEIGHT (currently u64::MAX)
- fn proof_system — identify proof system from proof bytes
- mod proof_system — constants: UNASSIGNED=0, PLONKY2=1, HALO2=2, GROTH16=3, RISC0=4

#### Parallel validation
- struct IndependentGroup — field: tx_indices: Vec<usize>
- struct DependencyGraph — fields: groups: Vec<IndependentGroup>, total_txs: usize
  - fn DependencyGraph::parallelism_ratio — ratio of largest independent group to total_txs
- fn build_dependency_graph — builds dependency graph from block's transactions; greedy independent set extraction

---

### AMM Pools (`crates/core/src/pool.rs`)
- fn compute_swap — constant product AMM swap output; returns (dy, reserve_a_new, reserve_b_new); returns None if any input is zero or result would drain reserve
- fn compute_initial_lp_shares — LP shares for initial liquidity deposit (integer sqrt of a*b)
- fn compute_lp_shares — LP shares for subsequent deposit; returns None if reserves or total_shares are zero
- fn compute_remove_liquidity — assets returned when burning LP shares; returns None if total_shares zero or shares > total_shares
- fn update_twap — update TWAP cumulative price accumulator; u128 fixed-point; saturating arithmetic
- fn compute_twap_price — compute TWAP price over a window; returns None if window_slots zero
- fn verify_invariant — verify x*y=k constant product invariant holds (new_k >= old_k)

---

### Lending (`crates/core/src/lending.rs`)
- const SLOTS_PER_YEAR: u64 = 3_155_760 — number of 10-second slots per year (~365.25 days)
- fn compute_interest — principal * rate_bps * elapsed_slots / (10000 * SLOTS_PER_YEAR); u128 intermediates
- fn compute_total_debt — principal + accrued interest
- fn compute_ltv_bps — debt * 10000 / collateral_value; returns u16::MAX if collateral_value zero
- fn is_liquidatable — liquidatable when collateral_value * 10000 < debt * liquidation_ratio_bps (strict less-than)
- fn collateral_value_from_twap — (collateral_amount * twap_price_fixed) >> 64; uses u128 fixed-point TWAP price (scaled by << 64)
- fn verify_creation_ltv — returns Ok(ltv) if ltv <= max_ltv_bps, Err(ltv) if over
- fn compute_depositor_earnings — total_interest * depositor_amount / total_deposits; 0 if total_deposits zero

---

### NFT (`crates/core/src/nft.rs`)
- enum NftContentFormat — 26 variants for detected NFT/on-chain content formats: Png, Jpeg, Gif, WebP, Bmp, Ico, Tiff, Avif, Svg, DoliPixelArt { width: u8, height: u8, palette_colors: u8 }, Pdf, Html, Json, Markdown, Csv, Mp3, Ogg, Wav, Flac, Mp4, Zip, Gzip, Wasm, Text, Binary, HashReference
- fn detect_content_format — detect content format from raw bytes; returns HashReference for exactly 32 bytes; falls back to Text (valid UTF-8) or Binary
- fn format_name — return human-readable name for a content format
- fn format_mime — return MIME type string for a content format

---

### Presence (`crates/core/src/presence.rs`)

#### struct PresenceCommitment — compact presence commitment stored in each block
- field bitfield: Vec<u8>
- field merkle_root: Hash
- field weights: Vec<Amount>
- field total_weight: Amount
- fn PresenceCommitment::empty — create empty presence commitment
- fn PresenceCommitment::new — create with producer_count, present_indices, weights, merkle_root
- fn PresenceCommitment::is_present — check if producer at index was present
- fn PresenceCommitment::get_weight — get weight for producer if present
- fn PresenceCommitment::present_count — number of present producers
- fn PresenceCommitment::total_weight — total weight of all present producers
- fn PresenceCommitment::is_empty — check if no producers were present
- fn PresenceCommitment::max_producer_index — maximum producer index representable
- fn PresenceCommitment::size — estimate serialized size in bytes
- fn PresenceCommitment::commitment_hash — domain-separated with "DOLI_PRESENCE_V1"
- fn PresenceCommitment::serialize — serialize using bincode
- fn PresenceCommitment::deserialize — deserialize from bincode bytes
- fn PresenceCommitment::verify_total_weight — verify stored total_weight matches sum of weights
- fn PresenceCommitment::verify_bitfield_weight_count — verify set bits in bitfield equals weights.len()
- fn PresenceCommitment::ensure_present — ensure a producer is marked present; used so block producer is always marked even if gossip didn't echo their heartbeat back
- fn PresenceCommitment::iter_present — iterate over (producer_index, weight) pairs

#### struct PresenceCommitmentV2 — compact 40-byte presence commitment for V2 blocks
- field heartbeats_root: Hash
- field total_weight: u64
- const PresenceCommitmentV2::SIZE: usize = 40
- fn PresenceCommitmentV2::empty — create empty V2 presence commitment
- fn PresenceCommitmentV2::new — create with heartbeats_root and total_weight
- fn PresenceCommitmentV2::is_empty — total_weight == 0
- fn PresenceCommitmentV2::total_weight — getter
- fn PresenceCommitmentV2::commitment_hash — domain-separated with "DOLI_PRESENCE_V2"; goes into BlockHeader.presence_root for V2 blocks
- fn PresenceCommitmentV2::serialize — serialize using bincode
- fn PresenceCommitmentV2::deserialize — deserialize from bincode bytes
- fn PresenceCommitmentV2::size — always 40

---

### Heartbeat (`crates/core/src/heartbeat.rs`)

#### struct Heartbeat — heartbeat proof of presence for a slot
- field version: u8
- field producer: PublicKey
- field slot: Slot
- field prev_block_hash: Hash
- field vdf_output: [u8; 32]
- field signature: Signature
- field witnesses: Vec<WitnessSignature>
- const Heartbeat::HEARTBEAT_VERSION: u8 = 1 (also pub const HEARTBEAT_VERSION: u8 = 1)
- fn Heartbeat::new — create without computing VDF; sets version, empty witnesses
- fn Heartbeat::compute_vdf_input — H(producer || slot || prev_hash); makes pre-computation impossible
- fn Heartbeat::compute_vdf — compute the hash-chain VDF (~1 second on modern hardware)
- fn Heartbeat::compute_signing_message — signing message for the producer
- fn Heartbeat::signing_message — get the signing message for this heartbeat
- fn Heartbeat::verify_vdf — verify the VDF output by recomputing
- fn Heartbeat::verify_signature — verify the producer's Ed25519 signature
- fn Heartbeat::add_witness — add a witness signature
- fn Heartbeat::has_enough_witnesses — check if at least MIN_WITNESS_SIGNATURES witnesses
- fn Heartbeat::verify_witnesses — verify all witness signatures; checks count, active-producer membership, no self-witness, valid signatures
- fn Heartbeat::verify_full — full verification: version, prev_hash, VDF, producer signature, witness signatures
- fn Heartbeat::id — unique identifier H(producer || slot)
- fn Heartbeat::serialize — bincode
- fn Heartbeat::deserialize — bincode
- fn Heartbeat::size — approximate size in bytes

#### struct WitnessSignature — witness attestation that a heartbeat is valid
- field witness: PublicKey
- field signature: Signature
- fn WitnessSignature::new — constructor
- fn WitnessSignature::compute_message — witness message for signing (includes producer, slot, vdf_output)
- fn WitnessSignature::verify — verify this witness signature against heartbeat data

#### enum HeartbeatError (8 variants)
- InvalidVdf, InvalidSignature, InsufficientWitnesses { have, need }, InvalidWitness(PublicKey), SelfWitness, InvalidWitnessSignature(PublicKey), UnsupportedVersion(u8), PrevHashMismatch

#### Constants
- const HEARTBEAT_VDF_ITERATIONS: u64 = 10_000_000 — VDF iterations for heartbeat proof (~1 second on modern hardware; 10M hash iterations)
- const MIN_WITNESS_SIGNATURES: usize = 2 — minimum witness signatures required

#### Free functions
- fn hash_chain_vdf — compute hash-chain VDF by iterating BLAKE3; ~1 second with HEARTBEAT_VDF_ITERATIONS
- fn verify_hash_chain_vdf — verify hash-chain VDF by recomputing

---

### TPoP (Telemetry Only — does NOT affect consensus) (`crates/core/src/tpop/`)

All items in this module are TELEMETRY ONLY. They do not affect consensus.

#### Presence scoring (tpop/presence/)
- struct ProducerPresenceState — fields: pubkey, last_vdf_output, last_slot, last_sequence, consecutive_presence, total_slots_active, missed_slots, presence_score, registered_era, bond_count
  - fn ProducerPresenceState::new — initial state with 1 bond
  - fn ProducerPresenceState::with_bonds — initial state with specified bond count
  - fn ProducerPresenceState::set_bond_count — update bond count
  - fn ProducerPresenceState::apply_presence — update VDF state, recalculate score
  - fn ProducerPresenceState::apply_missed_slots — increment missed_slots, decay score
- struct PresenceCheckpoint — fields: height, slot, prev_checkpoint_hash, producer_states, states_merkle_root, total_presence_score
  - fn PresenceCheckpoint::new — construct checkpoint with Merkle root
  - fn PresenceCheckpoint::hash — canonical hash with "DOLI_PRESENCE_CHECKPOINT_V1" domain
  - fn PresenceCheckpoint::get_producer_state — linear search for producer state by pubkey
- struct VdfLink — single link in presence VDF chain; fields: sequence, slot, input_hash, output, proof
  - fn VdfLink::new — construct link
  - fn VdfLink::compute_input — H("DOLI_PRESENCE_V1" || prev_output || slot || producer)
  - fn VdfLink::verify — verify this link's VDF proof
  - fn VdfLink::verify_chain — check sequence increment, slot advance, correct input derivation, and VDF proof
- struct PresenceProof — proof of presence for a specific slot; fields: producer, slot, vdf_chain, checkpoint_height, checkpoint_hash
  - fn PresenceProof::verify — verify checkpoint reference, chain continuity, and VDF
  - fn PresenceProof::slots_present — count of VDF links
- struct EpochPresenceRecords — records of presence proofs within an epoch
  - fn EpochPresenceRecords::new — create empty records
  - fn EpochPresenceRecords::record_presence — append slot to producer's record
  - fn EpochPresenceRecords::slots_with_valid_proof — count valid proof slots for a producer
- fn calculate_presence_score — consecutive + history + age bonuses; capped at MAX_PRESENCE_SCORE (2,000,000)
- fn select_producer_by_presence — deterministic round-robin by bond count; ticket_index = slot % total_tickets
- fn select_producer_by_presence_filtered — same as above but pre-filters producers below min_presence_score
- fn rank_producers_by_presence — rank all producers by score descending; returns (pubkey, score, bond_count, rank)
- fn can_produce_at_time — check eligibility using PRESENCE_WINDOWS
- fn producer_eligibility_offset — earliest offset (in seconds) when producer at given rank becomes eligible
- fn distribute_epoch_rewards — rewards proportional to (slots_present * score_multiplier)
- fn compute_next_presence_vdf — compute next VDF link in presence chain
- fn create_genesis_vdf_link — compute first VDF link for a new producer
- const CHECKPOINT_INTERVAL: u64 = 60 — checkpoint interval in slots
- const MAX_PRESENCE_SCORE: u64 = 2_000_000 — maximum presence score cap
- const PRESENCE_WINDOWS: [(u64, usize); 4] = [(0,1), (15,3), (30,10), (45,100)]

#### Heartbeat collection (tpop/heartbeat.rs)
Note: struct PresenceHeartbeat (not Heartbeat) in the tpop module; this is different from crates/core/src/heartbeat.rs
- struct PresenceHeartbeat — fields: version, producer, slot, prev_block_hash, vdf_output, vdf_proof, signature (hash-chain VDF, no separate proof object)
  - fn PresenceHeartbeat::create — compute VDF input, run hash-chain VDF, sign, return heartbeat
  - fn PresenceHeartbeat::compute_vdf_input — H("DOLI_HEARTBEAT_V1" || producer || slot || prev_hash)
  - fn PresenceHeartbeat::verify — check version, prev_hash, hash-chain VDF, and signature
  - fn PresenceHeartbeat::id — H(producer || slot)
  - fn PresenceHeartbeat::serialize / deserialize / size
- enum HeartbeatError — 9 variants: VdfFailed, InvalidVdfOutput, InvalidVdf, InvalidSignature, PrevHashMismatch, UnsupportedVersion, TooLate, FutureSlot, TooOld
- struct HeartbeatCollector — collects and validates heartbeats; fields: heartbeats (HashMap), per_slot_count
  - fn HeartbeatCollector::new — create empty collector
  - fn HeartbeatCollector::add — validate timing, verify content, enforce rate limit (MAX_HEARTBEATS_PER_SLOT=1000), insert
  - fn HeartbeatCollector::heartbeats_for_slot — get all heartbeats for a slot
  - fn HeartbeatCollector::producers_for_slot — get all producer pubkeys for a slot
  - fn HeartbeatCollector::count_for_slot — count heartbeats for a slot
  - fn HeartbeatCollector::prune_before — remove all entries for slots before min_slot
  - fn HeartbeatCollector::has_heartbeat — check if a producer submitted a heartbeat for a slot
- fn validate_heartbeat_timing — validate slot bounds (future/old) and deadline+grace period
- fn calculate_heartbeat_score — simplified heartbeat scoring: ratio (0-100) + consecutive bonus (max 50) + consistency bonus (10/20/30) + age bonus (logarithmic, max 20)
- const HEARTBEAT_DEADLINE_SECS: u64 = 55
- const HEARTBEAT_DISCRIMINANT_BITS: usize = 1024 (legacy constant; not used with hash-chain VDF)
- fn hash_chain_vdf — compute hash-chain micro-VDF (iterated BLAKE3)
- fn verify_hash_chain_vdf — verify hash-chain VDF by recomputing

#### VDF calibration (tpop/calibration.rs)
- struct VdfCalibrator — dynamic VDF iteration adjuster
  - fn VdfCalibrator::new — create calibrator, clamps initial to [MIN, MAX]
  - fn VdfCalibrator::disabled — create calibrator with dynamic adjustment disabled
  - fn VdfCalibrator::for_network — estimate initial iterations from reference rate
  - fn VdfCalibrator::iterations / target_time_ms / is_enabled / set_enabled
  - fn VdfCalibrator::record_timing — record timing sample; triggers recalibration if enough samples
  - fn VdfCalibrator::calibrate_now — perform immediate calibration run
  - fn VdfCalibrator::stats — return CalibrationStats snapshot
  - fn VdfCalibrator::load_iterations — restore iteration count from persistent storage
- struct CalibrationStats — fields: current_iterations, target_time_ms, sample_count, avg_duration_ms, enabled
- const TARGET_VDF_TIME_MS: u64 = 700 — target VDF computation time in ms for heartbeat proofs
- const DEFAULT_VDF_ITERATIONS: u64 = 10_000_000 — DEPRECATED; use vdf_iterations_for_network()
- const MIN_VDF_ITERATIONS: u64 = 100_000
- const MAX_VDF_ITERATIONS: u64 = 100_000_000
- fn vdf_iterations_for_network — returns network-specific VDF iterations from NetworkParams

#### TPoP integration (tpop/mod.rs)
- struct SimplePresenceState — simplified in-memory presence tracking; fields: producers (HashMap), current_epoch, last_checkpoint
  - fn SimplePresenceState::new / record_heartbeat / record_missed / advance_slot / ranked_producers / producer_rank / can_produce / new_epoch
- struct SimpleProducerState — fields: pubkey, epoch_heartbeats, total_heartbeats, total_slots, consecutive_present, registered_era, score
- struct TpopConfig — fields: activation_height, parallel_mode, parallel_duration
  - fn TpopConfig::mainnet — activation_height=10_080, parallel_mode=true
  - fn TpopConfig::testnet — activation_height=100, parallel_mode=false
  - fn TpopConfig::devnet — activation_height=0, parallel_mode=false
  - fn TpopConfig::is_active — check if TPoP is active at given height
  - fn TpopConfig::is_parallel — check if in parallel mode at given height
- type TpopMigrationConfig — legacy alias for TpopConfig
- struct TpopMetrics — fields: heartbeats_received, active_producers, total_presence_score, avg_presence_score, score_gini, slot_fill_rate, avg_heartbeats_per_slot
  - fn TpopMetrics::calculate — compute all metric fields from current state and slot history
- struct SlotStats — fields: slot, heartbeats_received, block_produced, producer_rank
- trait TpopConsensus — required methods: presence_state, heartbeat_collector, prev_block_hash, current_slot, tpop_enabled
- trait PresenceConsensus — legacy trait; required methods: current_checkpoint, proofs_for_slot

---

### Rewards (`crates/core/src/rewards.rs`)

#### trait BlockSource
- fn get_block_by_height — returns Ok(None) for nonexistent heights; Err only for actual storage failures (NOTE: spec used wrong name get_block_at_height)

#### struct WeightedRewardCalculation — DEPRECATED (always returns reward_amount=0 in current deterministic scheduler model)
- field epoch: u64
- field producer: PublicKey
- field producer_index: usize
- field blocks_present: u64
- field total_blocks: u64
- field total_producer_weight: Amount
- field total_all_weights: Amount
- field block_reward: Amount
- field reward_amount: Amount — always 0 from deprecated method
- fn WeightedRewardCalculation::has_reward — true if reward_amount > 0
- fn WeightedRewardCalculation::average_weight — total_producer_weight / blocks_present
- fn WeightedRewardCalculation::presence_rate — (blocks_present * 100) / total_blocks capped at 100

#### struct ClaimableSummary
- field epoch: u64
- field blocks_present: u64
- field estimated_reward: Amount
- field is_claimed: bool
- field claim_tx_hash: Option<Hash>

#### struct WeightedRewardCalculator (DEPRECATED)
- fn WeightedRewardCalculator::new — constructor with default 360 blocks_per_epoch
- fn WeightedRewardCalculator::with_blocks_per_epoch — constructor with custom epoch size
- fn WeightedRewardCalculator::calculate_producer_reward — DEPRECATED; always returns reward_amount=0
- fn WeightedRewardCalculator::calculate_multiple_epochs — calls deprecated method for each epoch
- fn WeightedRewardCalculator::total_claimable_reward — always returns 0 in current model

#### enum RewardError — 4 variants
- BlockNotFound { height }
- StorageError(String)
- EpochNotComplete { epoch, current_height }
- ProducerNotFound { producer }

#### Free functions
- fn complete_epochs_at_height — number of complete epochs at height; mainnet default 360 blocks/epoch
- fn complete_epochs_at_height_with — network-aware variant
- fn epoch_boundaries — start and end height for an epoch; mainnet default
- fn epoch_boundaries_with — network-aware variant
- fn is_epoch_complete — true if epoch has ended by current_height
- fn is_epoch_complete_with — network-aware variant
- fn complete_epoch_range — range 0..current_epoch for all complete epochs
- fn complete_epoch_range_with — network-aware variant

---

### Maintainer Governance (`crates/core/src/maintainer.rs`)

#### Constants
- const INITIAL_MAINTAINER_COUNT: usize = 5 — number of initial maintainers bootstrapped from first N registrations
- const MAINTAINER_THRESHOLD: usize = 3 — informational; actual threshold is dynamically computed
- const MIN_MAINTAINERS: usize = 3 — minimum maintainers allowed
- const MAX_MAINTAINERS: usize = 5 — maximum maintainers allowed

#### struct MaintainerSet
- field members: Vec<PublicKey>
- field threshold: usize — dynamically recalculated on every add/remove
- field last_updated: u64
- fn MaintainerSet::new — creates empty set
- fn MaintainerSet::with_members — constructs set with provided members and computes threshold
- fn MaintainerSet::is_maintainer — returns true if pubkey is in members
- fn MaintainerSet::can_remove — true if member_count > MIN_MAINTAINERS
- fn MaintainerSet::can_add — true if member_count < MAX_MAINTAINERS
- fn MaintainerSet::member_count — current number of members
- fn MaintainerSet::calculate_threshold — static; 0→0, 1→1, 2→2, 3→2, 4→3, 5→3, n>5→(n/2)+1
- fn MaintainerSet::verify_multisig — counts valid signatures from current maintainers; returns true if count >= threshold
- fn MaintainerSet::verify_multisig_excluding — same but skips excluded pubkey (used when removing a maintainer)
- fn MaintainerSet::add_maintainer — adds pubkey; recalculates threshold
- fn MaintainerSet::remove_maintainer — removes pubkey; recalculates threshold
- fn MaintainerSet::force_remove_maintainer — removes bypassing MIN_MAINTAINERS check (used for slashing)
- fn MaintainerSet::is_fully_bootstrapped — true if member_count >= INITIAL_MAINTAINER_COUNT
- fn MaintainerSet::needs_bootstrap_member — true if member_count < INITIAL_MAINTAINER_COUNT

#### struct MaintainerSignature
- field pubkey: PublicKey
- field signature: Signature
- fn MaintainerSignature::new — direct constructor
- fn MaintainerSignature::verify — verifies signature against pubkey

#### struct MaintainerChangeData
- field target: PublicKey
- field signatures: Vec<MaintainerSignature>
- field reason: Option<String>
- fn MaintainerChangeData::new — constructor without reason
- fn MaintainerChangeData::with_reason — constructor with reason
- fn MaintainerChangeData::signing_message — canonical bytes for signing; "add:{pubkey_hex}" or "remove:{pubkey_hex}"
- fn MaintainerChangeData::to_bytes — bincode serialization
- fn MaintainerChangeData::from_bytes — bincode deserialization

#### struct ProtocolActivationData
- field protocol_version: u32
- field activation_epoch: u64
- field description: String
- field signatures: Vec<MaintainerSignature>
- fn ProtocolActivationData::new — direct constructor
- fn ProtocolActivationData::signing_message — canonical bytes; "activate:{version}:{epoch}"
- fn ProtocolActivationData::to_bytes — bincode serialization
- fn ProtocolActivationData::from_bytes — bincode deserialization

#### enum MaintainerError (7 variants)
- MaxMaintainersReached, MinMaintainersRequired, AlreadyMaintainer, NotMaintainer, InsufficientSignatures { found, required }, NotRegisteredProducer, MaintainerSlashed

#### enum MaintainerChange
- Add(MaintainerChangeData)
- Remove(MaintainerChangeData)

#### trait BlockchainReader — abstracts chain access for derive_maintainer_set
- fn get_registrations_in_order — all registration public keys in chronological order
- fn get_maintainer_changes — all AddMaintainer/RemoveMaintainer transactions in order
- fn get_slashed_producers — all producers that have been slashed

#### fn derive_maintainer_set — deterministically computes maintainer set from chain history

---

### Producer Discovery (`crates/core/src/discovery/`)

#### Constants
- const PRODUCER_ANNOUNCEMENT_DOMAIN: &[u8] = b"DOLI_PRODUCER_ANN_V1" — domain separation tag for Ed25519 signing
- const MAX_ANNOUNCEMENT_AGE_SECS: u64 = 3600 — maximum announcement age before rejection (1 hour)
- const MAX_FUTURE_TIMESTAMP_SECS: u64 = 300 — maximum future timestamp tolerance (5 minutes)

#### struct ProducerAnnouncement — cryptographically signed producer presence announcement
- field pubkey: PublicKey
- field network_id: u32 — cross-network replay prevention
- field genesis_hash: Hash — cross-genesis contamination prevention
- field sequence: u64 — monotonically increasing; higher supersedes lower
- field timestamp: u64
- field signature: Signature
- fn ProducerAnnouncement::new — create and sign using current system time
- fn ProducerAnnouncement::new_with_timestamp — create and sign with explicit timestamp
- fn ProducerAnnouncement::new_from_private_key — create and sign without a full KeyPair
- fn ProducerAnnouncement::verify — verify the Ed25519 signature
- fn ProducerAnnouncement::message_bytes — return the 84-byte canonical signed message

#### struct ProducerBloomFilter — probabilistic set for efficient delta synchronization
- fn ProducerBloomFilter::new — create filter sized for expected element count at ~1% false positive rate
- fn ProducerBloomFilter::with_params — create filter with explicit bit size and hash count
- fn ProducerBloomFilter::insert — insert a public key
- fn ProducerBloomFilter::probably_contains — test membership (~1% false positive)
- fn ProducerBloomFilter::to_bytes — serialize bit array for network transfer
- fn ProducerBloomFilter::from_bytes — reconstruct filter from serialized bytes plus metadata
- fn ProducerBloomFilter::hash_count / element_count / size_bits / size_bytes
- fn ProducerBloomFilter::false_positive_rate — theoretical FP rate

#### struct ProducerGSet — G-Set CRDT for producer announcements providing eventual consistency
- fn ProducerGSet::new — create empty in-memory GSet
- fn ProducerGSet::new_with_persistence — create GSet with disk persistence
- fn ProducerGSet::is_stable — true if no changes for at least duration
- fn ProducerGSet::len / is_empty / clear / contains / get
- fn ProducerGSet::network_id / genesis_hash / sequence_for / has_persistence
- fn ProducerGSet::merge_one — merge a single announcement; validates signature, network_id, genesis_hash, timestamp bounds, and sequence vector
- fn ProducerGSet::merge — batch merge with DoS protection (aborts after 50 consecutive rejections)
- fn ProducerGSet::sorted_producers — all known producer keys sorted deterministically
- fn ProducerGSet::active_producers — sorted producer keys filtered by max_age_secs
- fn ProducerGSet::export — non-stale announcements for gossip
- fn ProducerGSet::export_all — all announcements regardless of age
- fn ProducerGSet::purge_stale — permanently remove producers with stale announcements
- fn ProducerGSet::to_bloom_filter — build bloom filter containing all known producers
- fn ProducerGSet::delta_for_peer — announcements for producers not in the peer's bloom filter
- fn ProducerGSet::persist_to_disk — serialize and atomically write to storage_path
- fn ProducerGSet::load_from_disk — load and re-verify announcements from storage_path

#### enum MergeOneResult — 3 variants: NewProducer, SequenceUpdate, Duplicate

#### struct MergeResult — statistics from batch merge
- field added: usize
- field new_producers: usize
- field rejected: usize
- field duplicates: usize
- fn MergeResult::is_empty — true if no announcements processed
- fn MergeResult::total — total announcements processed

#### enum ProducerSetError — 6 variants: InvalidSignature, StaleAnnouncement, FutureTimestamp, NetworkMismatch { expected, got }, GenesisHashMismatch, SequenceRegression { current, received }

#### struct AdaptiveGossip — adaptive gossip interval controller
- fn AdaptiveGossip::new — create with defaults (5s initial, 1s min, 60s max)
- fn AdaptiveGossip::with_config — create with custom settings
- fn AdaptiveGossip::on_gossip_result — update interval after a gossip round
- fn AdaptiveGossip::interval — current recommended gossip interval
- fn AdaptiveGossip::use_delta_sync — true if estimated network size exceeds 20 nodes
- fn AdaptiveGossip::stability_period — recommended stability period before block production; scales with network size
- fn AdaptiveGossip::estimated_network_size — current high-water-mark network size estimate
- fn AdaptiveGossip::rounds_without_change — consecutive rounds without new producer discovery
- fn AdaptiveGossip::reset — reset interval to default

#### struct EpochSnapshot — compact snapshot of the producer set at an epoch boundary
- field epoch: u64
- field merkle_root: Hash
- field active_producers: Vec<PublicKey> — sorted by pubkey bytes
- field total_producers: u64
- field total_weight: u64
- fn EpochSnapshot::new — create snapshot; computes Merkle root internally
- fn EpochSnapshot::compute_merkle_root — static; deterministic Merkle root
- fn EpochSnapshot::epoch_from_height — static; compute epoch from block height
- fn EpochSnapshot::epoch_from_height_with — static; network-aware
- fn EpochSnapshot::is_epoch_boundary — static; true if height > 0 and multiple of SLOTS_PER_EPOCH
- fn EpochSnapshot::is_epoch_boundary_with — static; network-aware

#### Protobuf (pub mod proto)
- fn encode_announcement / decode_announcement — single announcement protobuf encoding
- fn encode_producer_set / decode_producer_set — ProducerSet protobuf encoding
- fn encode_digest / decode_digest — bloom filter protobuf encoding
- fn is_legacy_bincode_format — heuristic to detect old bincode Vec<PublicKey> format
- enum ProtoError — 4 variants: InvalidPublicKey, InvalidSignature, DecodeError, MissingField

---

### ZK Verification (`crates/core/src/validation/zk.rs`)
(See also Validation section above for ZkVerifyContext, ZkVerifyError, verify_zk_proof, proof_system)
- const ZK_SETTLE_ACTIVATION_HEIGHT: u64 = u64::MAX — activation height for L2 settlement (disabled by default; lowered only via ProtocolActivation tx)

---

### Crate-level constant (`crates/core/src/lib.rs`)
- const PROTOCOL_VERSION: u32 = 1 — protocol version included in block headers for backward compatibility (distinct from INITIAL_PROTOCOL_VERSION re-exported from consensus and CURRENT_PROTOCOL_VERSION in crates/network)
> Compiler: Section 4-6 compiler
> Source reports: A29, A30, A31, A32, A33, A34, A35, A36, A37, A38, A39, A40, A43
> Date: 2026-04-19
> Status: CORRECTED — all agent-reported errors fixed, critical gaps filled

---

## 4. STORAGE (`crates/storage`)

### Block Store
- `BlockStore` — persistent block storage (RocksDB, **9** column families; LZ4 compression, bloom filters)
- `BlockBody` — serializable block body (pub(super)): transactions, aggregate_bls_signature, attestation_bitfield
- Column families (9): `headers`, `bodies`, `height_index`, `slot_index`, `presence` (deprecated, cleaned on startup), `hash_to_height`, `tx_index`, `addr_tx_index`, `meta`
  - `headers` — hash → bincode(BlockHeader); all blocks including forks
  - `height_index` — height (u64 LE) → hash; canonical chain index
  - `slot_index` — slot (u32 LE) → hash; slot-based block lookup
  - `hash_to_height` — hash → height (u64 LE); reverse canonical index
  - `tx_index` — tx_hash → block height (u64 LE)
  - `addr_tx_index` — pubkey_hash(32) ‖ height(8 BE) → empty; address history index
  - `meta` — metadata keys (e.g., `snap_horizon`)
- `MIN_RETENTION: u64 = 2000` — minimum blocks retained during pruning (2× MAX_REORG_DEPTH)
- `BlockStore::open(path)` — opens or creates RocksDB store; runs four one-time migrations on first startup
- `BlockStore::get_block(hash)` — retrieves full block (header + body) by hash
- `BlockStore::get_header(hash)` — retrieves block header only by hash
- `BlockStore::get_block_by_height(height)` — retrieves full block by canonical height
- `BlockStore::get_block_by_slot(slot: u32)` — retrieves full block by slot
- `BlockStore::get_hash_by_height(height)` — canonical height → hash via height_index CF
- `BlockStore::get_height_by_hash(hash)` — O(1) reverse lookup: hash → canonical height via hash_to_height CF
- `BlockStore::get_hash_by_slot(slot: u32)` — slot → block hash via slot_index CF
- `BlockStore::has_block(hash)` — checks whether a block exists in headers CF
- `BlockStore::has_block_for_slot(slot: u64)` — fast slot existence check
- `BlockStore::has_any_block_in_slot_range(start, end)` — efficient emptiness check for [start,end) slot range
- `BlockStore::get_blocks_in_slot_range(start, end)` — returns all blocks in [start,end) slot range; primary method for epoch reward calculation
- `BlockStore::get_tx_block_height(tx_hash)` — looks up block height containing a given tx hash via tx_index CF
- `BlockStore::get_address_heights(pubkey_hash, before_height, limit)` — paginated address history (descending heights) via addr_tx_index CF
- `BlockStore::get_last_rewarded_epoch()` — scans chain in reverse to find most recent epoch with an EpochReward tx
- `BlockStore::get_snap_horizon()` — returns the snap sync floor height from meta CF
- `BlockStore::ensure_blocks_present(low, high)` — verifies height_index contains every height in [low,high]; FORK_GUARD backfill invariant check
- `BlockStore::count_slot_index_entries()` — diagnostic: counts entries in slot_index CF
- `BlockStore::put_block(block, height)` — stores header + body + slot/tx/addr indexes; does NOT update height_index or hash_to_height
- `BlockStore::put_block_canonical(block, height)` — put_block + direct height_index/hash_to_height update; for simple chains without fork handling
- `BlockStore::set_canonical_chain(tip_hash, tip_height)` — updates height_index + hash_to_height by walking backwards from tip; sole writer to canonical indexes; respects snap_horizon floor
- `BlockStore::seed_canonical_index(hash, height)` — called after snap sync; writes snap_horizon to meta CF; allows set_canonical_chain early exit
- `BlockStore::rebuild_canonical_index()` — rebuilds canonical index from scratch by scanning all headers; used when height_index is corrupt
- `BlockStore::create_checkpoint(path)` — RocksDB checkpoint (point-in-time snapshot via hard links); near-instant
- `BlockStore::cleanup_fork_blocks()` — removes non-canonical (fork) blocks; returns count removed
- `BlockStore::clear_indexes()` — clears only index CFs (height_index, slot_index, hash_to_height); preserves block data
- `BlockStore::clear()` — clears ALL column families; used only by CLI `recover --yes`
- `BlockStore::delete_blocks_above(keep_height)` — deletes all blocks above keep_height; returns count deleted
- `BlockStore::prune_blocks_below(keep_above_height, chain_tip)` — prunes history with MIN_RETENTION=2000 safety; returns (deleted_count, lowest_remaining_height)
- `BlockStore::storage_stats()` — counts entries in each of 7 CFs; returns Vec<(&'static str, u64)>
- `BlockStore::height_range()` — returns (min_height, max_height) from height_index, or None if empty
- `deserialize_body(bytes)` — backward-compatible body deserializer; tries current format (v3.7.1+), then v3.6.0, then v3.5.0, then plain Vec
- impl `BlockSource for BlockStore` — enables WeightedRewardCalculator to access blocks by height

### State DB
- `StateDb` — unified RocksDB state database; 6 column families: `cf_utxo`, `cf_utxo_by_pubkey`, `cf_producers`, `cf_exit_history`, `cf_meta`, `cf_undo`
- `BlockBatch<'a>` — atomic write batch for a single block application; fields: db ref, rocksdb::WriteBatch, utxo_delta (i64), pending_utxos (HashMap), spent_in_batch (Vec)
- `UndoData` — per-block undo data for rollback; fields: `spent_utxos: Vec<(Outpoint, UtxoEntry)>`, `created_utxos: Vec<Outpoint>`, `producer_snapshot: Vec<u8>`, `epoch_state_snapshot: Option<Vec<u8>>`
- `LastApplied` — consistency canary stored in same WriteBatch as state; fields: `height: u64`, `hash: Hash`, `slot: u32`; serialized as 44-byte LE array
- `StateDb::open(path)` — opens or creates unified state DB with 6 CFs; LZ4 compression; WAL PointInTime recovery
- `StateDb::begin_batch()` — creates a new empty BlockBatch for atomic writes
- `BlockBatch::add_utxo(outpoint, entry)` — stage UTXO add (writes cf_utxo + cf_utxo_by_pubkey secondary index)
- `BlockBatch::spend_utxo(outpoint)` — stage UTXO spend; checks pending_utxos first for same-block-spend
- `BlockBatch::spend_transaction_utxos(tx)` — spend all inputs of a transaction; returns total input amount
- `BlockBatch::add_transaction_utxos(tx, height, is_coinbase, slot)` — add all outputs; stamps Bond UTXOs with creation_slot
- `BlockBatch::put_producer(pubkey_hash, info)` / `BlockBatch::remove_producer(pubkey_hash)` — stage producer record mutations
- `BlockBatch::put_exit_history(pubkey_hash, exit_height)` — stage exit history entry
- `BlockBatch::put_chain_state(cs)` — stage ChainState (versioned 0x01 prefix + bincode)
- `BlockBatch::put_pending_updates(updates)` — stage pending producer updates
- `BlockBatch::put_epoch_producer_list(keys)` — stage frozen epoch producer list
- `BlockBatch::put_attestation_accumulators(...)` — stage attestation accumulators (local state)
- `BlockBatch::put_epoch_bond_snapshot(snapshot, epoch)` — stage epoch bond snapshot
- `BlockBatch::put_epoch_state(bytes)` / `BlockBatch::put_epoch_state_version(version)` — stage EpochState + protocol version marker
- `BlockBatch::put_chain_commitment(commitment)` — stage incremental chain commitment hash atomically with block
- `BlockBatch::put_active_production_list(keys)` — stage active production list
- `BlockBatch::set_last_applied(height, hash, slot)` — stage LastApplied consistency canary
- `BlockBatch::put_undo(height, undo)` — stage undo data in same WriteBatch as state
- `BlockBatch::commit(self)` — atomically commit entire batch to RocksDB; updates utxo_count atomic counter
- `BlockBatch::write_dirty_producers(ps, dirty_keys, removed_keys, dirty_exit_keys)` — write only changed producers (dirty-key optimization)
- `BlockBatch::write_full_producer_set(ps)` — clear existing cf_producers + cf_exit_history then write full ProducerSet; used for reorg/migration
- `StateDb::create_checkpoint(path)` — RocksDB hard-link checkpoint
- `StateDb::get_chain_state()` / `StateDb::get_pending_updates()` / `StateDb::get_last_applied()` — key query methods
- `StateDb::get_utxo(outpoint)` / `StateDb::contains_utxo(outpoint)` / `StateDb::get_utxos_by_pubkey(pubkey_hash)` — UTXO queries
- `StateDb::get_producer(pubkey_hash)` / `StateDb::iter_producers()` / `StateDb::load_producer_set()` — producer queries
- `StateDb::get_balance_with_maturity(pubkey_hash, height, maturity)` — spendable balance with explicit maturity
- `StateDb::serialize_canonical_utxo()` — canonical UTXO bytes for state root (lexicographic key order)
- `StateDb::get_epoch_state()` / `StateDb::get_epoch_bond_snapshot()` / `StateDb::get_attestation_accumulators()` — epoch state queries
- `StateDb::get_chain_commitment()` / `StateDb::put_chain_commitment(commitment)` / `StateDb::delete_chain_commitment()` — chain commitment management
- `StateDb::put_undo(height, undo)` / `StateDb::get_undo(height)` / `StateDb::delete_undo(height)` — undo log operations
- `StateDb::prune_undo_before(keep_height)` — O(1) targeted expiry; compacts every 100 blocks
- `StateDb::prune_undo_above(keep_height)` — delete all undo data above keep_height (for truncation)
- `StateDb::atomic_replace(cs, ps, utxo_iter)` — atomically replace all state; preserves cf_meta scheduler keys (Fix #10)
- `StateDb::clear_and_write_genesis(genesis_cs)` — atomically clear all non-undo CFs and write genesis state; crash-safe
- `StateDb::import_utxos(entries)` — bulk import UTXOs; batches in 50,000-entry chunks
- Meta keys (stored in cf_meta): `chain_state`, `pending_updates`, `last_applied`, `epoch_producer_list`, `active_production_list`, `epoch_attested_set`, `epoch_attestation_accum`, `epoch_blocks_produced`, `epoch_bond_snapshot`, `epoch_state`, `epoch_state_version`, `chain_commitment`

### Chain State
- `ChainState` — persistent consensus chain state; fields: `best_hash`, `best_height`, `best_slot`, `total_work` (= best_height), `genesis_hash`, `genesis_timestamp`, `last_registration_hash`, `registration_sequence`, `total_minted`, `snap_sync_height: Option<u64>`, `active_protocol_version: u32`, `pending_protocol_activation: Option<(u32, u64)>`
- `ChainState::new(genesis_hash)` — create initial chain state at genesis
- `ChainState::load(path)` / `ChainState::save(path)` — bincode persistence with backward-compat handling
- `ChainState::update(hash, height, slot)` — update best tip; sets total_work = height
- `ChainState::serialize_canonical()` — fixed 140-byte canonical encoding for state root (covers 9 fields; excludes snap_sync_height, active_protocol_version, pending_protocol_activation)
- `ChainState::apply_coinbase(reward)` — track minted coins; returns Err if would exceed TOTAL_SUPPLY
- `ChainState::can_mint(reward)` / `ChainState::remaining_supply()` / `ChainState::effective_reward(calculated)` — supply cap helpers
- `ChainState::record_registration(tx_hash)` — advance registration chain (last_hash + sequence++)
- `ChainState::verify_registration_chain(prev_hash, sequence)` — validate a registration's chain fields
- `ChainState::mark_snap_synced(height)` / `ChainState::clear_snap_sync()` / `ChainState::is_snap_synced()` — snap sync state management

### UTXO Set
- `UtxoSet` — **enum** backend dispatcher (NOT a trait); variants: `InMemory(InMemoryUtxoStore)`, `RocksDb(RocksDbUtxoStore)`
- `UtxoEntry` — a single UTXO record; fields: `output: Output`, `height: BlockHeight`, `is_coinbase: bool`, `is_epoch_reward: bool`
- `Outpoint` — UTXO identifier; fields: `tx_hash: Hash`, `index: u32`; serialized as 36-byte canonical form
- `InMemoryUtxoStore` — HashMap-based UTXO backend; serializable; used during migration and testing
- `RocksDbUtxoStore` — RocksDB-backed production UTXO store; 3 CFs: `utxo`, `utxo_by_pubkey`, `unique_id`
- `UtxoSet::new()` / `UtxoSet::open_rocksdb(path)` / `UtxoSet::load(path)` / `UtxoSet::save(path)` — construction/persistence
- `UtxoSet::get(outpoint)` / `UtxoSet::contains(outpoint)` / `UtxoSet::clear()` — basic UTXO access
- `UtxoSet::add_transaction(tx, height, is_coinbase, slot)` — add all outputs; stamps Bond/Pool UTXOs with creation_slot
- `UtxoSet::spend_transaction(tx)` — remove all inputs; returns total native amount consumed
- `UtxoSet::get_by_pubkey_hash(pubkey_hash)` — all UTXOs for an address
- `UtxoSet::get_balance_with_maturity(pubkey_hash, height, maturity)` — spendable balance with explicit maturity
- `UtxoSet::get_immature_balance_with_maturity(pubkey_hash, height, maturity)` — immature coinbase/epoch-reward balance
- `UtxoSet::get_bonded_balance(pubkey_hash)` / `UtxoSet::count_bonds(pubkey_hash, bond_unit)` / `UtxoSet::get_bond_entries(pubkey_hash)` — bond queries (FIFO-sorted by creation_slot)
- `UtxoSet::total_value()` / `UtxoSet::total_confirmed(height, maturity, pool_pkh)` / `UtxoSet::total_supply()` — supply queries
- `UtxoSet::len()` / `UtxoSet::is_empty()` / `UtxoSet::utxo_count()` / `UtxoSet::address_count()` — count queries
- `UtxoSet::get_all_pools()` / `UtxoSet::get_all_collateral()` / `UtxoSet::get_pool_utxo(pool_id)` — pool/collateral queries
- `UtxoSet::serialize_canonical()` / `UtxoSet::deserialize_canonical(bytes)` — canonical bytes for state root (both backends produce identical output)
- `UtxoSet::has_unique_id(prefix, id)` / `UtxoSet::find_nft_by_token_id(token_id)` — unique ID index
- `UtxoSet::insert(outpoint, entry)` / `UtxoSet::remove(outpoint)` — direct insert/remove (migration/reorgs)
- `UtxoEntry::serialize_canonical_bytes()` / `UtxoEntry::deserialize_canonical_bytes(bytes)` — fixed-field canonical encoding immune to struct evolution; used for state root
- `UtxoEntry::is_spendable_at_with_maturity(height, maturity)` — checks time lock and coinbase/epoch_reward maturity
- `uid_key(prefix, id)` — builds 33-byte unique index key; `UID_PREFIX_NFT=0x01`, `UID_PREFIX_ASSET=0x02`, `UID_PREFIX_POOL=0x03`, `UID_PREFIX_CHANNEL=0x04`
- `reward_maturity_for_network(network)` — network-aware reward maturity lookup
- impl `UtxoProvider for UtxoSet` — implements `get_utxo(tx_hash, output_index)` for core validation

### Producer Set
- `ProducerSet` — registered producers, bonds, delegations, pending updates; fields: producers (HashMap), exit_history (HashMap), active_cache (skip-serialized), unbonding_index (BTreeMap, skip-serialized), pending_updates (Vec)
- `ProducerInfo` — full producer state; key fields: `public_key`, `registered_at`, `bond_amount`, `bond_count`, `status`, `bls_pubkey`, `bond_entries`, `additional_bonds`, `delegated_to`, `delegated_bonds`, `received_delegations`, `has_prior_exit`, `last_activity`, `withdrawal_pending_count`
- `ProducerStatus` — **Active**, **Unbonding { started_at: u64 }**, **Exited**, **Slashed { slashed_at: u64 }** (no Pending or Exiting variants)
- `ActivityStatus` — **Active**, **RecentlyInactive**, **Dormant** — controls governance power and quorum
- `StoredBondEntry` — single bond entry: `creation_slot: u32`, `amount: u64`; sorted ascending for FIFO withdrawal
- `PendingProducerUpdate` — **7** variants: `Register { info, height }`, `Exit { pubkey, height }`, `Slash { pubkey, height }`, `AddBond { pubkey, outpoints, bond_unit, creation_slot }`, `DelegateBond { delegator, delegate, bond_count }`, `RevokeDelegation { delegator }`, `RequestWithdrawal { pubkey, bond_count, bond_unit }` (no ClaimWithdrawal variant)
- `ACTIVATION_DELAY` = 10 blocks — propagation buffer before scheduling eligibility
- `BOND_UNIT` = 1,000,000,000 base units (10 DOLI per bond, mainnet/testnet)
- `MAX_WEIGHT` = 4 / `MIN_WEIGHT` = 1 — seniority weight caps
- `VETO_THRESHOLD_PERCENT` = 40 — percent of effective weight required to veto
- `EXIT_HISTORY_RETENTION` = 4,204,800 blocks (~8 years)
- `REACTIVATION_THRESHOLD` = 8,640 blocks (~1 day to regain Active status)
- `ProducerSet::new()` / `ProducerSet::from_parts(producers, exit_history, pending_updates)` / `ProducerSet::as_parts()` — construction
- `ProducerSet::register(info, current_height)` — registers producer; checks exit history; errors if already registered and not Exited
- `ProducerSet::request_exit(pubkey, current_height)` — starts unbonding; updates unbonding_index
- `ProducerSet::cancel_exit(pubkey)` — reverts Unbonding to Active
- `ProducerSet::process_unbonding(current_height, unbonding_duration)` — O(k) via unbonding_index; marks completed unbonders as Exited; records in exit_history
- `ProducerSet::slash_producer(pubkey, current_height)` — marks producer Slashed; returns slashed bond amount
- `ProducerSet::apply_pending_updates()` — applies all queued mutations at epoch boundaries; clears queue; invalidates cache
- `ProducerSet::queue_update(update)` — enqueues a deferred mutation
- `ProducerSet::active_producers()` / `ProducerSet::active_producers_at_height(height)` / `ProducerSet::all_producers()` — producer queries
- `ProducerSet::active_count()` / `ProducerSet::active_count_at_height(height)` / `ProducerSet::total_count()` — counts
- `ProducerSet::get(pubkey_hash)` / `ProducerSet::get_by_pubkey(pubkey)` — lookups
- `ProducerSet::delegate_bonds(delegator, delegatee, bond_count)` / `ProducerSet::revoke_delegation(delegator)` — delegation management
- `ProducerSet::total_weight_for_network(height, network)` / `ProducerSet::weighted_veto_threshold_for_network(height, network)` — governance weight
- `ProducerSet::has_weighted_veto_for_network(veto_pubkeys, height, network)` — weighted veto check
- `ProducerSet::serialize_canonical()` — deterministic serialization (sorted by Hash key) for state root
- `ProducerSet::load(path)` / `ProducerSet::save(path)` — file persistence (JSON; atomic rename)
- `ProducerSet::register_genesis_producer(pubkey, bond_count, bond_unit)` — genesis registration at height 0
- `ProducerSet::prune_exit_history_for_network(height, network)` — removes expired exit records
- `ProducerInfo::is_active()` / `ProducerInfo::can_produce()` — true if Active or Unbonding
- `ProducerInfo::selection_weight()` — own bonds + received_delegations bonds (0 if not active)
- `ProducerInfo::add_bonds(outpoints, amount_per_bond, creation_slot)` — adds bonds up to MAX cap
- `ProducerInfo::calculate_withdrawal_with_quarter(count, current_slot, quarter_slots)` — FIFO net + penalty
- `ProducerInfo::apply_withdrawal(count, bond_unit)` — removes oldest bond_entries; auto-exits if all withdrawn
- `ProducerInfo::activity_status_for_network(height, network)` — returns ActivityStatus
- `producer_weight_for_network(registered_at, current_height, network)` — discrete yearly seniority steps 1/2/3/4
- `total_weight_for_network(producers, current_height, network)` — sum of weights of active producers
- `calculate_withdrawal_from_bonds(bonds, count, current_slot, quarter_slots)` — FIFO withdrawal from UTXO-derived bond entries

### Snapshot
- `StateSnapshot` — serialized state ready for snap sync transfer; fields: `block_hash`, `block_height`, `chain_state_bytes`, `utxo_set_bytes`, `producer_set_bytes`, `state_root`
- `StateSnapshot::create(chain_state, utxo_set, producer_set)` — serializes all three components and computes state_root; logs sizes and component hashes
- `StateSnapshot::total_bytes()` — sum of all serialized component sizes
- `compute_state_root(chain_state, utxo_set, producer_set)` — deterministic state root: `H(H(cs_canonical) ‖ H(utxo_canonical) ‖ H(ps_canonical))`
- `compute_state_root_with_epoch_state(cs, utxo, ps, epoch_state_hash)` — M-Choice1 Phase-1 primitive: None → identical to compute_state_root; Some(h) → 4-component hash (Phase-2 call sites deferred)
- `compute_scheduler_root(...)` — hash over all consensus-derived scheduler inputs (Fix #9: covers scheduler divergence not detected by state root)
- `compute_state_root_from_bytes(cs_bytes, utxo_bytes, ps_bytes)` — verify snapshot integrity from raw bytes

### Archiver
- `BlockArchiver` — filesystem block archiver; receives blocks via mpsc channel; stores `{height:010}.block` + `{height:010}.blake3` sidecar + manifest.json
- `ArchiveBlock` — block data for archiving; fields: `height: u64`, `hash: Hash`, `data: Vec<u8>`
- `BlockArchiver::new(rx, dir)` / `BlockArchiver::run(self)` — constructor and async archiver loop
- `BlockArchiver::catch_up(dir, block_store, tip)` — static; archives all missing blocks from 1..=tip
- `manifest_height(dir)` — read latest archived height from manifest.json
- `restore_from_archive(archive_dir, block_store, expected_genesis_hash)` — full restore with checksum verification
- `backfill_from_archive(archive_dir, block_store, expected_genesis_hash)` — backfill: skips blocks already in BlockStore; for post-snap-sync gap filling

### Content Store
- `ContentStore` — content-addressed blob storage with reference counting; deduplicates large NFT data
- `ContentStore::put(data)` — store content; increments ref count; returns content hash
- `ContentStore::get(content_hash)` — retrieve by hash
- `ContentStore::release(content_hash)` — decrement ref count; deletes content at 0; returns new ref count
- `ContentStore::get_refcount(content_hash)` / `ContentStore::contains(content_hash)` — ref count queries
- `ContentStore::len()` / `ContentStore::is_empty()` — size queries

### MMR (Merkle Mountain Range)
- `CompactMmr` — Merkle Mountain Range storing only peaks; O(log n) append and root
- `CompactMmr::append(leaf_hash)` — append a new leaf; O(log n); merges peaks via `H(left ‖ right)`
- `CompactMmr::root()` — compute root from peaks right-to-left; returns ZERO for empty MMR
- `CompactMmr::len()` / `CompactMmr::is_empty()` — size queries
- `IncrementalStateRoot` — incremental state root tracker using MMR for UTXO commitment; O(log n) per block vs O(n) for full UTXO hash
- `IncrementalStateRoot::add_utxo(outpoint_hash)` — record UTXO creation; appends to MMR
- `IncrementalStateRoot::spend_utxo(outpoint_hash)` / `IncrementalStateRoot::unspend_utxo(outpoint_hash)` — spend/rollback via XOR accumulator
- `IncrementalStateRoot::compute_root(cs_hash, ps_hash)` — compute `H(mmr_root ‖ spent_hash ‖ cs_hash ‖ ps_hash)`
- `IncrementalStateRoot::total_created()` — total UTXOs ever created (MMR leaf count)

### Maintainer State
- `MaintainerState` — cached MaintainerSet with derivation height; avoids re-deriving from genesis on restart
- `MaintainerState::load(data_dir)` / `MaintainerState::save(data_dir)` — bincode persistence
- `MaintainerState::update(set, height, data_dir)` — update cached set and height then persist

### Update State
- `UpdateState` — complete persisted auto-update state; fields: `pending_releases`, `votes`, `history`
- `PersistedVote` — persisted producer vote: `producer_id`, `vote`, `timestamp`, `weight`
- `PersistedRelease` — persisted release info: `version`, `binary_sha256`, `published_at`, `changelog`
- `UpdateHistoryEntry` — record of an applied/rolled-back/vetoed update: `version`, `applied_at`, `outcome`
- `UpdateState::load(data_dir)` / `UpdateState::save(data_dir)` — persistence
- `UpdateState::add_pending_release(release)` / `UpdateState::remove_pending_release(version)` — release management
- `UpdateState::record_vote(version, vote)` / `UpdateState::get_votes(version)` / `UpdateState::clear_votes(version)` — vote tracking
- `UpdateState::record_history(entry)` — append to chronological update history

### Storage Error
- `StorageError` — storage layer error type; variants: `Database(String)`, `Serialization(String)`, `NotFound(String)`, `AlreadyExists(String)`, `Io(std::io::Error)`

---

## 5. NETWORK (`crates/network`)

### Service
- `NetworkService` — main P2P network coordinator; owns event receiver, command sender, peers map, and local peer ID
- `NetworkConfig` — full network configuration (20 fields: listen_addr, bootstrap_nodes, max_peers, node_key_path, network_id, genesis_hash, no_dht, peer_cache_path, mesh_n, mesh_n_low, mesh_n_high, gossip_lazy, nat_config, external_address, seed_mode, tx_announce_enabled, bootstrap_slots, enable_discv5, discv5_port, bootnode_enrs)
- `NetworkEvent` — 20-variant event enum emitted from swarm loop to node event loop
  - `PeerConnected(PeerId)`, `PeerDisconnected(PeerId)`, `NewBlock(Block, PeerId)`, `NewHeader(BlockHeader)`, `NewTransaction(Transaction)`, `StatusRequest { peer_id, request, channel }`, `SyncRequest { peer_id, request, channel }`, `SyncResponse { peer_id, response }`, `PeerStatus { peer_id, status }`, `NetworkMismatch { peer_id, ... }`, `GenesisMismatch { peer_id }`, `VersionMismatch { peer_id, ... }`, `ProducersAnnounced(Vec<PublicKey>)`, `ProducerAnnouncementsReceived(Vec<ProducerAnnouncement>)`, `ProducerDigestReceived { peer_id, digest }`, `NewVote(Vec<u8>)`, `NewHeartbeat(Vec<u8>)`, `NewAttestation(Vec<u8>)`, `TxAnnouncement { peer_id, hashes }`, `TxFetchRequest { peer_id, hashes, channel }`, `TxFetchResponse { peer_id, transactions }`
- `NetworkCommand` — 19-variant command enum from node to swarm loop
  - `BroadcastBlock(Block)`, `BroadcastHeader(BlockHeader)`, `BroadcastTransaction(Transaction)`, `RequestStatus { peer_id, request }`, `RequestSync { peer_id, request }`, `SendStatusResponse { channel, response }`, `SendSyncResponse { channel, response }`, `Connect(Multiaddr)`, `Disconnect(PeerId)`, `Bootstrap`, `BroadcastProducers(Vec<u8>)`, `BroadcastProducerAnnouncements(Vec<ProducerAnnouncement>)`, `BroadcastProducerDigest(ProducerBloomFilter)`, `SendProducerDelta { peer_id, announcements }`, `BroadcastVote(Vec<u8>)`, `BroadcastHeartbeat(Vec<u8>)`, `BroadcastAttestation(Vec<u8>)`, `RequestTxFetch { peer_id, hashes }`, `SendTxFetchResponse { channel, response }`
- `NetworkError` — 5 variants: `BindError`, `ConnectionFailed(String)`, `ChannelClosed`, `PeerNotFound(String)`, `Other(String)`
- `NetworkService::new(config)` — async constructor; generates/loads Ed25519 keypair; builds transport + DoliBehaviour; starts Discv5 if enabled; spawns swarm loop; dials bootstrap nodes
- `NetworkService::next_event()` — blocking receive of next event from swarm loop
- `NetworkService::try_next_event()` — non-blocking try_recv for draining pending events before production
- `NetworkService::broadcast_block(block)` / `NetworkService::broadcast_header(header)` / `NetworkService::broadcast_attestation(data)` / `NetworkService::broadcast_transaction(tx)` — gossip publish methods
- `NetworkService::broadcast_producer_announcements(announcements)` / `NetworkService::broadcast_producer_digest(digest)` / `NetworkService::send_producer_delta(peer_id, announcements)` — producer announcement methods
- `NetworkService::request_status(peer_id, request)` / `NetworkService::request_sync(peer_id, request)` / `NetworkService::request_tx_fetch(peer_id, hashes)` — outbound request methods
- `NetworkService::send_status_response(channel, response)` / `NetworkService::send_sync_response(channel, response)` / `NetworkService::send_tx_fetch_response(channel, response)` — response sending
- `NetworkService::connect(address)` / `NetworkService::disconnect(peer_id)` / `NetworkService::bootstrap()` — connection management
- `NetworkService::peer_count()` / `NetworkService::get_peer(peer_id)` / `NetworkService::get_peers()` / `NetworkService::local_peer_id()` — peer queries
- `NetworkService::peers_arc()` — returns shared peer map Arc for external read access
- `NetworkService::command_sender()` — returns cloned command sender for external use
- `GENESIS_MISMATCH_COOLDOWN_SECS` = 86400 — 24-hour cooldown before genesis-mismatched peer reconnection
- `DEFAULT_PORT: u16` = 30300 — default P2P port
- `PROTOCOL_ID: &str` = `/doli/1.0.0` — libp2p protocol identifier
- `extract_peer_id_from_multiaddr(addr)` — extracts PeerId from a multiaddr string

### Transport
- `build_transport(keypair, relay_transport)` — builds DNS/TCP + Noise encryption + Yamux multiplexing transport stack; optionally composes relay client for NAT traversal
- `yamux_config()` (private) — Yamux config with 256KB receive window (ENV-tunable via `DOLI_YAMUX_WINDOW`)

### Behaviour
- `DoliBehaviour` — composite libp2p NetworkBehaviour with **11** sub-behaviours:
  - `connection_limits` — connection cap enforcement
  - `gossipsub` — GossipSub for block and transaction propagation
  - `kademlia` — Kademlia DHT for peer discovery
  - `identify` — Identify protocol for peer info exchange
  - `status` — request-response for status handshake (30s timeout)
  - `sync` — request-response for sync protocol (120s timeout)
  - `txfetch` — request-response for transaction fetching (5s timeout)
  - `relay_client` — relay client for NAT traversal
  - `relay_server` — relay server for other nodes
  - `dcutr` — direct connection upgrade through relay (hole punching)
  - `autonat` — automatic NAT status detection

### Gossip Topics (8)
- `BLOCKS_TOPIC` = `/doli/blocks/1`
- `TRANSACTIONS_TOPIC` = `/doli/txs/1`
- `PRODUCERS_TOPIC` = `/doli/producers/1`
- `VOTES_TOPIC` = `/doli/votes/1`
- `HEARTBEATS_TOPIC` = `/doli/heartbeats/1`
- `TIER1_BLOCKS_TOPIC` = `/doli/t1/blocks/1`
- `HEADERS_TOPIC` = `/doli/headers/1`
- `ATTESTATION_TOPIC` = `/doli/attestations/1`
- `region_topic(region: u32) -> String` — generates regional block topic string for Tier 2 sharding

### Gossip Config
- `MeshConfig` — mesh_n, mesh_n_low, mesh_n_high, gossip_lazy
- `compute_dynamic_mesh(total_peers)` — sqrt-scaled mesh parameters (MESH_N_CAP = 50 private constant)
- `new_gossipsub(keypair, mesh)` — BLAKE3 message IDs, per-topic scoring
- `subscribe_to_topics(gossipsub)` — subscribe to all standard topics
- `GossipError` — 4 variants: `Config(String)`, `Init(String)`, `Subscribe(String)`, `Publish(String)`

### Gossip Publishing
- `publish_block(gossipsub, block_data)` / `publish_header(gossipsub, header_data)` / `publish_attestation(gossipsub, data)` — publish to respective topics
- `publish_transaction(gossipsub, tx_data)` / `publish_vote(gossipsub, vote_data)` / `publish_heartbeat(gossipsub, data)` — publish to respective topics
- `publish_producer(gossipsub, producer_data)` / `publish_tier1_block(gossipsub, block_data)` / `publish_to_region(gossipsub, region, block_data)` — publish to respective topics
- `TxGossipMessage` — `FullBatch(Vec<Transaction>)` or `Announce(Vec<Hash>)` (prefix: 0xBA batch, 0xAA announce)
- `encode_tx_batch(transactions)` / `decode_tx_gossip(data)` / `decode_tx_message(data)` / `encode_tx_announce(hashes)` — tx gossip encoding/decoding

### Protocols — Status
- `StatusRequest` — fields: `version: u32`, `network_id: u32`, `genesis_hash: Hash`, `producer_pubkey: Option<PublicKey>`
- `StatusResponse` — fields: `version: u32`, `network_id: u32`, `genesis_hash: Hash`, `best_height: u64`, `best_hash: Hash`, `best_slot: u32`, `producer_pubkey: Option<PublicKey>`
- `StatusProtocol` — protocol identifier struct; `STATUS_PROTOCOL` = `/doli/status/1.0.0`
- `StatusCodec` — bincode codec with 4-byte LE length prefix framing; MAX_STATUS_SIZE = 64KB
- `CURRENT_PROTOCOL_VERSION: u32` = 5 (history: 1=original, 2=version enforcement, 3=INC-I-026, 4=M-Choice1 epoch state, 5=DirectAttestation)
- `MIN_PEER_PROTOCOL_VERSION: u32` = 1 (held at 1 during Phase-1 rollout)

### Protocols — Sync
- `SyncRequest` — **8** variants: `GetHeaders { start_hash, max_count }`, `GetBodies { hashes }`, `GetBlockByHeight { height }`, `GetBlockByHash { hash }`, `GetStateSnapshot { block_hash }`, `GetStateRoot { block_hash }`, `DirectAttestation { data }` (protocol v5+; bypasses gossip mesh), `GetHeadersByHeight { start_height, max_count }` (INC-I-012 F1; used after snap sync)
- `SyncResponse` — **6** variants: `Headers(Vec<BlockHeader>)`, `Bodies(Vec<Block>)`, `Block(Option<Block>)`, `StateSnapshot { block_hash, block_height, chain_state, utxo_set, producer_set, state_root, block_header_bytes, epoch_bond_snapshot_bytes, epoch_accumulators_bytes, epoch_state_bytes }`, `StateRoot { block_hash, block_height, state_root }`, `Error(String)`
- `SyncProtocol` — protocol identifier struct; `SYNC_PROTOCOL` = `/doli/sync/1.0.0`
- `SyncCodec` — bincode codec with 4-byte LE length prefix; `MAX_SYNC_SIZE` = 16 MB
- `SyncResponse::type_name()` — returns human-readable variant name for logging

### Protocols — TxFetch
- `TxFetchRequest` — `hashes: Vec<Hash>` (max 50)
- `TxFetchResponse` — `transactions: Vec<Transaction>`
- `TxFetchCodec` — bincode codec; `TXFETCH_PROTOCOL` = `/doli/txfetch/1.0.0`; `MAX_TXFETCH_HASHES` = 50

### Discovery
- `new_kademlia(local_peer_id)` — Kademlia DHT with replication factor 20, 60s query timeout, server mode; `KAD_PROTOCOL` = `/doli/kad/1.0.0`
- `Discv5Service` — Discv5 UDP peer discovery with DOLI network filtering (custom ENR key: network_id + genesis_hash prefix)
- `Discv5Config` — fields: `udp_port`, `tcp_port`, `external_ip: Option<Ipv4Addr>`, `network_id`, `genesis_hash`, `bootnode_enrs`
- `Discv5Service::new(keypair, config)` — async; creates and starts Discv5 service; builds ENR with DOLI network filter
- `Discv5Service::find_random_peers()` — trigger random-walk query; results arrive via event stream
- `Discv5Service::is_same_network(enr)` — network filter comparison; accepts ENRs without "doli" field for compatibility

### Peers
- `PeerInfo` — 11 fields: `id`, `address`, `version`, `best_height`, `best_hash`, `connected_at`, `last_seen`, `bytes_received`, `bytes_sent`, `latency`, `is_producer`
- `PeerState` — `Connecting`, `Handshaking`, `Active`, `Disconnecting`, `Disconnected`
- `IpPrefix` — /24 IPv4 or /48 IPv6 prefix for eclipse attack prevention
- `PeerDiversity` — tracks peer diversity; checks can_connect(), records connections, enforces prefix diversity
- `PeerDiversityConfig` — fields: `max_per_prefix`, `max_per_asn`, `enabled`
- `DiversityStats` — snapshot: total_peers, unique_prefixes, unique_asns, max_peers_per_prefix, max_peers_per_asn

### Peer Cache
- `PeerCache` — disk-backed peer address cache (bincode binary format, NOT JSON); max 100 peers
- `CachedPeer` — fields: `peer_id: String`, `address: String`, `last_seen: u64`
- `MAX_CACHED_PEERS` = 100
- `PeerCache::load(path)` / `PeerCache::save(path)` — bincode persistence; atomic rename
- `PeerCache::add(peer_id, address)` — upsert; silently ignores loopback; trims to MAX_CACHED_PEERS

### NAT
- `NatConfig` — fields: `relay_servers`, `enable_autonat`, `enable_dcutr`, `enable_relay_server`, `max_relay_reservations`, `max_circuits_per_peer`
- `NatStatus` — `Unknown`, `Public`, `Private`
- `NatInfo` — fields: `status`, `external_address: Option<Multiaddr>`, `active_relay_connections: usize`, `dcutr_available: bool`
- `NatConfig::client()` — NAT-traversal config with mainnet seed relay servers
- `NatConfig::relay_server()` — public relay server config (256 reservations, 8 circuits/peer)

### Rate Limiting
- `RateLimiter` — per-peer and global rate limiter; manages per-peer limits and global token buckets
- `TokenBucket` — token bucket with capacity, tokens (u64), last_refill, refill_rate (f64 tokens/sec)
- `RateLimitConfig` — fields: `max_blocks_per_minute`, `max_txs_per_second`, `max_requests_per_second`, `max_bytes_per_second`, `enabled`
- `PeerLimits` — per-peer buckets: `blocks`, `transactions`, `requests`, `bandwidth`, `last_activity`
- `RateLimiterStats` — snapshot: `tracked_peers`, `enabled`
- `RateLimiter::check_block(peer)` / `check_transaction(peer)` / `check_request(peer)` / `check_bandwidth(peer, bytes)` — rate limit checks
- `RateLimiter::cleanup(max_age)` — LRU eviction; caps at MAX_TRACKED_PEERS = 1000

### Scoring
- `PeerScorer` — peer reputation tracker maintaining scores and banned peer set
- `PeerScore` — per-peer reputation score: `value (i32, clamped -1000..1000)`, `infractions`, `valid_blocks`, `valid_transactions`
- `PeerScorerConfig` — `disconnect_threshold=-200`, `ban_threshold=-500`, `ban_duration=1h`, `decay_rate=1.0`, `max_infractions=100`
- `ScorerStats` — snapshot: peer_count, banned_count, average_score, min_score, max_score
- `Infraction` — **7** variants with penalties:
  - `InvalidBlock { slot }` — penalty: -100
  - `InvalidTransaction { hash }` — penalty: -20
  - `Timeout { count }` — penalty: -5 × min(count, 10)
  - `Spam { msg_type }` — penalty: -50
  - `Duplicate` — penalty: -5
  - `MalformedMessage` — penalty: -30
  - `IncompatibleVersion { their_version }` — penalty: -200 (immediately crosses disconnect threshold)
- `PeerScorer::record_valid_block(peer)` (+10) / `record_invalid_block(peer, slot)` / `record_valid_tx(peer)` (+1) / `record_invalid_tx(peer, hash)` — scoring methods
- `PeerScorer::should_disconnect(peer)` / `should_ban(peer)` / `ban(peer)` / `is_banned(peer)` — eviction methods
- `PeerScorer::tick()` — decay all scores; remove expired bans
- `PeerScorer::peers_to_disconnect()` / `PeerScorer::stats()` — diagnostics

### Sync Manager
- `SyncManager` — central sync state machine (34 fields); orchestrates header-first and snap sync, production gating, fork recovery, and peer management
- `SyncState` — 3 variants: `Idle`, `Syncing { phase: SyncPhase, started_at: Instant }`, `Synchronized`
- `SyncPhase` — diagnostic label (NOT a state machine): `DownloadingHeaders`, `DownloadingBodies`, `ProcessingBlocks`, `SnapCollecting`, `SnapDownloading`
- `SyncPipelineData` — 7 variants (operational data, separate from SyncState): `None`, `Headers { target_slot, peer, headers_count }`, `Bodies { pending, total }`, `Processing { height }`, `SnapCollecting { target_hash, target_height, votes, asked }`, `SnapDownloading { target_hash, target_height, quorum_root, peer, alternate_peers }`, `SnapReady { snapshot: VerifiedSnapshot }`
- `SyncConfig` — max_headers_per_request=500, max_bodies_per_request=128, max_concurrent_body_requests=8, request_timeout=30s, min_peers_for_sync=1, stale_timeout=300s
- `SyncPipeline` (pub(crate)) — pipeline state: pending_headers, headers_needing_bodies, pending_blocks, pending_requests, next_request_id, sync_epoch, header_downloader, body_downloader, body_stall_retries
- `NetworkState` (pub(crate)) — network tip tracking: network_tip_height, network_tip_slot, last_block_seen, last_block_applied, last_sync_activity, blocks_applied, last_progress_log, idle_behind_retries, tip_unsupported_since
- `SnapSyncState` (pub(crate)) — snap sync config + runtime: threshold=50, quorum=3, root_timeout=15s, download_timeout=60s, blacklisted_peers, attempts, fresh_node_wait_start, store_floor=1, last_snap_completed, discv5_peer_grace_deadline
- `ForkState` (pub(crate)) — fork detection state: consecutive_empty_headers, consecutive_sync_failures, consecutive_apply_failures, needs_genesis_resync, fork_recovery (ForkRecoveryTracker), header_blacklisted_peers, stable_gap_since, peak_height, stuck_fork_signal, use_height_based_headers, height_fallback_attempted, last_rollback_local_height
- `ProductionAuthorization` — **13** variants; first variant is **Authorized** (not Allowed):
  - `Authorized` — production is authorized
  - `BlockedSyncing` — blocked during active sync
  - `BlockedResync { grace_remaining_secs }` — blocked during resync grace period
  - `BlockedBehindPeers { local_height, peer_height, height_diff }` — too far behind peers
  - `BlockedAheadOfPeers { local_height, peer_height, height_ahead }` — suspiciously ahead of all peers
  - `BlockedChainMismatch { peer_id, local_hash, peer_hash, local_height }` — critical chain mismatch
  - `BlockedInsufficientPeers { peer_count, min_required }` — too few peers (echo chamber prevention)
  - `BlockedSyncFailures { failure_count }` — excessive sync failures
  - `BlockedNoGossipActivity { seconds_since_gossip, peer_count }` — no gossip activity
  - `BlockedExplicit { reason }` — explicitly blocked (invariant violation)
  - `BlockedBootstrap { reason }` — waiting for fresh peer status
  - `BlockedConflictsFinality { local_finalized_height }` — conflicts with finalized block
  - `BlockedAwaitingCanonicalBlock` — after snap sync: waiting for canonical gossip block
- `RecoveryPhase` — 4 variants:
  - `Normal` — no recovery in progress
  - `ResyncInProgress` — forced resync from genesis/snap in progress
  - `PostRecoveryGrace { started: Instant, blocks_applied: u32 }` — grace period; clears at 10 blocks applied
  - `AwaitingCanonicalBlock { started: Instant }` — snap sync complete; 60s timeout enforced by cleanup()
- `RecoveryReason` — **7** variants: `AllPeersBlacklistedDeepFork`, `StuckSyncLargeGap { gap }`, `HeightOffsetDetected { gap }`, `GenesisFallbackEmptyHeaders`, `BodyDownloadPeerError`, `ApplyFailuresSnapThreshold { gap }`, `RollbackDeathSpiral { peak, current }`
- `ForkAction` — 3 variants: `None`, **`RollbackOne`** (not ShallowRollback), **`NeedsGenesisResync`** (not DeepResync)
- `VerifiedSnapshot` — verified snap sync state for node application; carries all state bytes + epoch data
- `MAX_CONSECUTIVE_RESYNCS: u32` = 5 — max consecutive force-resyncs before requiring manual intervention
- `PeerSyncStatus` — per-peer sync status: best_height, best_hash, best_slot, last_status_response, last_block_received, pending_request, protocol_version, producer_pubkey
- `SyncManager::new(config, genesis_hash)` / `SyncManager::new_with_settings(config, genesis_hash, grace_secs, max_slots_behind)` — constructors
- `SyncManager::can_produce(current_slot)` — single source of truth for production authorization; returns ProductionAuthorization
- `SyncManager::is_production_safe(current_slot)` — boolean wrapper over can_produce
- `SyncManager::block_production(reason)` / `SyncManager::unblock_production()` — explicit production gate
- `SyncManager::start_resync()` / `SyncManager::complete_resync()` — resync lifecycle
- `SyncManager::request_genesis_resync(reason)` — gated genesis resync; 5 guards (floor, concurrent recovery, rate limit, snap availability, snap attempts); emergency reasons bypass guards 1 and 4
- `SyncManager::track_block_for_finality(hash, height, slot, weight)` / `SyncManager::add_attestation_weight(block_hash, weight)` — finality tracking
- `SyncManager::signal_stuck_fork()` / `SyncManager::take_stuck_fork_signal()` — stuck fork signal
- `SyncManager::block_applied_with_weight(hash, height, slot, weight, prev_hash)` — full block-applied update; maintains confirmed_height_floor
- `SyncManager::block_apply_failed()` — tracks apply failures; at ≥3 triggers fork signal or genesis resync
- `SyncManager::reset_local_state(genesis_hash)` — full genesis reset; respects confirmed_height_floor (monotonic progress floor)
- `SyncManager::confirmed_height_floor()` — INC-I-005 Fix C: monotonic floor preventing infinite snap-sync death spirals
- `SyncManager::cleanup()` — periodic maintenance: stale peer removal, stuck sync detection, snap collecting/downloading timeouts, header blacklist expiry, height offset detection, network tip decay
- `SyncManager::add_peer(peer, height, hash, slot)` / `SyncManager::update_peer(...)` / `SyncManager::remove_peer(peer)` — peer management
- `SyncManager::note_orphan_gossip_block(height, slot)` — tracks orphan gossip blocks; at ≥3 triggers batch sync or stuck_fork_signal
- `SyncManager::checkpoint_health()` — returns (total_peers, agreeing_peers, unique_chain_tips) for fork diagnosis
- `SyncManager::recommend_fork_action(gap, consecutive_rollbacks, max_rollback_depth)` — returns ForkAction via ForkState
- `SyncManager::take_needs_mass_status_refresh()` — consumed once when peer count rises from 0 to non-zero
- `SyncManager::next_request()` / `SyncManager::handle_response(peer, response)` / `SyncManager::get_blocks_to_apply()` — sync engine dispatch

### Sync — Headers
- `HeaderDownloader` — sequential header download state machine
- `HeaderDownloader::create_request(start_hash)` — produces GetHeaders request from expected_prev_hash
- `HeaderDownloader::process_headers(headers, local_tip)` — validates chain linkage and timestamps; returns count accepted
- `HeaderDownloader::take_headers(count)` — drains up to count headers from validated queue
- `HeaderDownloader::resume_from(hash)` — sets expected_prev_hash for continuation; clears stale buffers
- `HeaderDownloader::pending_count()` / `total_downloaded()` / `expected_prev_hash()` / `clear()` — state accessors

### Sync — Bodies
- `BodyDownloader` — parallel block body download with retry logic; tracks active requests, downloaded blocks, failed queues, per-hash failure counts
- `BodyDownloader::next_request(needed, peers)` — selects next batch; permanently fails after 3 attempts (60s expiry); returns (PeerId, SyncRequest::GetBodies) or None
- `BodyDownloader::process_response(peer, bodies)` — correlates response; marks received/failed hashes
- `BodyDownloader::handle_timeout(peer)` — cancels active request; moves hashes back to failed queue
- `BodyDownloader::take_block(hash)` / `has_block(hash)` / `downloaded_count()` / `in_flight_count()` — block access
- `BodyDownloader::cleanup_timeouts()` / `cancel_peer(peer)` / `expire_permanently_failed()` / `clear()` — maintenance
- `BodyDownloader::all_needed_permanently_failed(needed)` — deadlock detection: all needed hashes permanently failed

### Sync — Equivocation
- `EquivocationDetector` — tracks (producer, slot) → first header; LRU eviction; holds pending_proofs and sliding window state
- `EquivocationProof` — double-signing evidence: `producer (PublicKey)`, `block_header_1`, `block_header_2`, `slot (u32)`
- `EquivocationProof::to_slash_transaction(reporter_keypair)` — creates SlashProducer tx from proof
- `EquivocationDetector::check_block(block)` — main entry point; returns Option<EquivocationProof>
- `EquivocationDetector::take_pending_proofs()` / `has_pending_proofs()` — proof drain
- `EquivocationDetector::cleanup_before_slot(min_slot)` / `tracked_count()` / `clear()` — maintenance
- `MAX_TRACKED_ENTRIES` = 10,000 — LRU cap on seen (producer, slot) pairs
- `SLIDING_WINDOW_SLOTS` = 360 — slots retained in the sliding window (1 epoch)

### Sync — Fork Recovery
- `ForkRecoveryTracker` — parent chain walking for fork resolution; tracks active session, cooldown, exceeded_max_depth flag, genesis_hash
- `CompletedRecovery` — result of completed recovery: `blocks: Vec<Block>` (forward order), `connection_point: Hash` (where fork connects to stored chain)
- `ForkRecoveryTracker::start(orphan_block, peer)` — begins parent-walk recovery; returns bool
- `ForkRecoveryTracker::next_fetch()` — returns (peer, hash_to_fetch); handles session timeout and per-request peer failover
- `ForkRecoveryTracker::handle_block(peer, block)` — feeds block response; validates; checks depth limit
- `ForkRecoveryTracker::check_connection(parent_known)` — completes recovery when parent_known=true; returns CompletedRecovery
- `ForkRecoveryTracker::set_alternate_peers(peers)` / `cancel(reason)` / `take_exceeded_max_depth()` — control methods
- `ForkRecoveryTracker::is_active()` / `can_start()` / `current_parent()` — state queries
- `MAX_RECOVERY_DEPTH` = 1,000 — maximum parent chain walk depth
- `RECOVERY_COOLDOWN` = 30s — cooldown between recovery attempts
- `RECOVERY_TIMEOUT` = 120s — timeout for a single recovery session
- `REQUEST_TIMEOUT` = 10s — per-block request timeout before peer failover

### Sync — Reorg
- `ReorgHandler` — weight-based fork choice; maintains recent_blocks, block_parents, block_weights maps with LRU eviction; max_tracked = 10,000
- `BlockWeight` — per-block metadata: `prev_hash`, `producer_weight`, `accumulated_weight`, `height`
- `ReorgResult` — reorg plan output: `rollback: Vec<Hash>`, `common_ancestor: Hash`, `new_blocks: Vec<Hash>`, `weight_delta: i64`
- `ReorgHandler::record_block_with_weight(hash, prev_hash, weight)` — records a chain block; updates current_chain_weight
- `ReorgHandler::record_fork_block(hash, prev_hash, weight)` — records competing fork block WITHOUT updating current_chain_weight
- `ReorgHandler::should_reorg_by_weight_with_tiebreak(new_tip, current_tip)` — true if new_tip heavier, or equal weight with lower hash
- `ReorgHandler::check_reorg_weighted(block, current_tip, weight)` — full fork choice; enforces finality guard; returns Option<ReorgResult>
- `ReorgHandler::plan_reorg(current_tip, new_tip, get_parent)` — builds full reorg plan; enforces finality guard on common ancestor
- `ReorgHandler::set_last_finality_height(height)` — updates finality boundary; reorgs at or below this height are rejected
- `ReorgHandler::compare_chains(chain_a_tip, chain_b_tip)` — Ordering by accumulated weight
- `ReorgHandler::clear()` / `set_current_weight(weight)` / `get_block_weight(hash)` / `knows_block(hash)` / `get_parent(hash)` — accessors
- `MAX_REORG_DEPTH` = 1,000 — maximum depth for reorg ancestor search

### Sync — Recovery Coordinator
- `RecoveryCoordinator` — centralized recovery decision maker; holds rolling evidence window (VecDeque, max 256 entries, 120s TTL) and last-action cooldown (5s)
- `RecoveryEvidence` — **5** variants: `EmptyHeaders { peer, gap }`, `OrphanGossip { slot, gap }`, `ApplyFailure { height }`, `DeepForkSuspected { empty, gap }`, `StaleTip { last_applied_secs, gap }`
- `RecoveryAction` — **5** variants (ordered by severity): `None`, `ShallowRollback { depth }`, `HeaderFirstSync`, `SnapSync`, `GenesisResync`
- `RecoveryContext` — node-wide snapshot for classifier: local_height, network_tip_height, peer_count, last_applied_secs, shallow_rollback_count, snap_attempts, last_rollback_local_height, in_grace_period
- `RecoveryContext::gap()` / `applied_since_rollback()` / `recently_synced()` — classifier helpers
- `RecoveryCoordinator::report(evidence)` — appends evidence to rolling window; prunes stale entries
- `RecoveryCoordinator::classify(ctx)` — pure read; evaluates 4 ordered rules; respects grace period, applied_since_rollback, and cooldown gates; returns RecoveryAction
- `RecoveryCoordinator::record_action(action)` — starts cooldown timer after caller executes action
- Threshold constants (mod thresholds): `MIN_MINOR_FORK_EVIDENCE=3`, `MINOR_FORK_GAP_MAX=50`, `SNAP_SYNC_GAP_MIN=500`, `SHALLOW_ROLLBACK_MAX=10`, `SNAP_ATTEMPTS_MAX=3`, `SNAP_MIN_PEERS=3`, `STALE_TIP_SECS=300`
- Note: RecoveryCoordinator is fully implemented and tested; currently in shadow/logging mode (phase-2 wiring deferred)

### Sync — Production Gate
- `SyncManager::can_produce(current_slot)` — multi-layer production authorization; 3 active checks
- `SyncManager::block_production(reason)` / `SyncManager::unblock_production()` — explicit gate
- `SyncManager::start_resync()` / `SyncManager::complete_resync()` / `SyncManager::is_resync_in_progress()` — resync lifecycle
- `SyncManager::track_block_for_finality(hash, height, slot, weight)` / `SyncManager::add_attestation_weight(block_hash, weight)` / `SyncManager::prune_finality(current_slot)` / `SyncManager::last_finalized_height()` — finality tracking
- `SyncManager::signal_stuck_fork()` / `SyncManager::take_stuck_fork_signal()` — stuck fork signal (Normal phase only)
- `SyncManager::request_genesis_resync(reason)` — gated genesis resync entry point; returns bool (honored or refused)
- `SyncManager::configure_production_gate(grace_secs, max_slots_behind)` / `set_min_peers_for_production(min)` / `set_bootstrap_grace_period_secs(secs)` — configuration
- `MAX_CONSECUTIVE_RESYNCS: u32` = 5

---

## 6. MEMPOOL (`crates/mempool`)

- `Mempool` — transaction mempool; primary store (HashMap by hash), fee-rate index (BTreeSet), address index (HashMap), spent-outpoint map; holds MempoolPolicy, ConsensusParams, Network
- `MempoolEntry` — pending-transaction record; fields: `tx`, `tx_hash`, `fee`, `fee_rate`, `size`, `added_time`, `ancestors: HashSet<Hash>`, `descendants: HashSet<Hash>`, `ancestor_fee: u64`, `ancestor_size: usize`
- `MempoolEntry::effective_fee_rate()` — returns ancestor_fee/ancestor_size (CPFP package rate); falls back to fee_rate
- `MempoolEntry::add_ancestor(hash, fee, size)` / `remove_ancestor(hash, fee, size)` — ancestor tracking with cumulative fee/size bookkeeping
- `MempoolEntry::add_descendant(hash)` / `remove_descendant(hash)` — descendant tracking
- `MempoolEntry::age()` — elapsed seconds since added_time
- `MempoolPolicy` — acceptance and eviction rules; fields: `max_count` (5,000), `max_size` (10 MB), `min_fee_rate` (0), `max_tx_size` (600 KB), `max_ancestors` (25), `max_age` (14 days)
- `MempoolPolicy::mainnet()` / `MempoolPolicy::testnet()` / `MempoolPolicy::local()` — network-specific policy constructors
- `MempoolError` — **9** variants:
  - `AlreadyExists` — exact transaction hash already in pool
  - `Full` — pool at max_count or max_size and eviction could not make room
  - `InvalidTransaction(String)` — failed structural or signature/covenant validation; tagged error codes [MPTX001]–[MPTX009]
  - `FeeTooLow(u64, u64)` — actual fee below required minimum (both absolute fee and fee-rate)
  - `TooLarge(usize, usize)` — transaction size exceeds max_tx_size
  - `TooManyAncestors(usize, usize)` — ancestor count exceeds max_ancestors
  - `TooManyDescendants(usize, usize)` — descendant count exceeds policy limit (variant declared; enforcement deferred)
  - `MissingInput(Hash, u32)` — input outpoint does not exist in UTXO set or pool
  - `DoubleSpend { tx_hash: Hash, output_index: u32, spending_tx: Hash }` — outpoint already claimed by mempool tx
- `Mempool::new(policy, params, network)` / `Mempool::mainnet()` / `Mempool::testnet()` — constructors
- `Mempool::add_transaction(tx, utxo_set, current_height)` — full admission path: duplicate → size → structural → signature/covenant → fee → ancestor limit → double-spend → eviction → CPFP ancestor wiring; returns tx hash
- `Mempool::add_system_transaction(tx, current_height)` — bypass fee/UTXO for state-only txs (e.g. SlashProducer); still validates structure; inserts at fee_rate=0
- `Mempool::remove_transaction(tx_hash)` — removes entry; cleans all indexes and cross-links; returns removed entry
- `Mempool::remove_for_block(transactions)` — remove confirmed transactions
- `Mempool::select_for_block(max_size)` — CPFP-aware fee-rate ordering: sorts by effective_fee_rate() descending; greedily selects transactions whose ancestors are already selected
- `Mempool::remove_registration_txs()` — prevent infinite retry loops on stale registrations
- `Mempool::remove_by_error_pattern(err_msg)` — heuristic toxic-tx purge for NFT/Pool/Registration errors
- `Mempool::get(tx_hash)` / `contains(tx_hash)` — entry lookup
- `Mempool::len()` / `is_empty()` / `size()` / `max_size()` / `max_count()` — size queries
- `Mempool::is_outpoint_spent(outpoint)` — double-spend query from external callers
- `Mempool::min_fee_rate()` — dynamic floor: raises to current lowest pool rate when >90% full
- `Mempool::iter()` — iterator over all (hash, entry) pairs
- `Mempool::get_by_address(pubkey_hash)` — address-indexed entry lookup
- `Mempool::calculate_unconfirmed_balance(pubkey_hash, utxo_set)` — returns (incoming, outgoing) amounts across all mempool txs
- `Mempool::get_unconfirmed_balance(pubkey_hash, utxo_set)` — returns signed net unconfirmed balance (incoming - outgoing)
- `Mempool::expire_old()` — remove transactions older than policy.max_age (evict_lowest_fee helper; called externally)
- `Mempool::revalidate(utxo_set, current_height)` — post-reorg cleanup; re-validates all entries against new UTXO set

---
---

## 7. RPC (`crates/rpc`)

### Server
- RpcServer — HTTP/WebSocket server (axum-based); `run()` binds and blocks, `spawn()` runs in background task
- RpcServerConfig — listen_addr, enable_cors, allowed_origins, admin_token; Default impl binds 127.0.0.1:8500
- RpcContext — shared state for all handlers; 23 fields: block_store, utxo_set, chain_state, producer_set, mempool, params, network, blocks_per_reward_epoch, coinbase_maturity, bond_unit, peer_id, peer_count, peer_list, broadcast_tx, sync_status, broadcast_vote, update_status, maintainer_state, vesting_quarter_slots, backfill_state, sync_manager, state_db, data_dir
- RpcContext::new_for_network() — preferred constructor; RpcContext::new() — deprecated (mainnet defaults only)
- RpcContext::handle_request() — public async dispatch method; routes all 47 methods by name string
- BackfillState — public struct for live backfill progress tracking; fields: running (AtomicBool), imported (AtomicU64), total (AtomicU64), error (RwLock<Option<String>>)
- SyncStatus — public struct; fields: is_syncing (bool), progress (Option<f64>)
- RpcError — JSON-RPC error with code, message, data
- MAX_BODY_SIZE = 2 MB
- ADMIN_METHODS: pauseProduction, resumeProduction, createCheckpoint, pruneBlocks, backfillFromPeer (auth-gated in HTTP layer via bearer token; getGuardianStatus is NOT admin-gated)

### RPC Types
- JsonRpcRequest — JSON-RPC 2.0 request envelope (jsonrpc, method, params, id)
- JsonRpcResponse — JSON-RPC 2.0 response envelope; JsonRpcResponse::success() / ::error()
- BlockResponse — hash, prev_hash, height, slot, timestamp, producer, merkle_root, tx_count, transactions, size, presence, aggregate_bls_sig, attestation_count
- TransactionResponse — hash, version, tx_type, inputs, outputs, covenant_witnesses, size, fee, block_hash, block_height, confirmations
- BalanceResponse — confirmed (spendable: confirmed minus mempool-spent UTXOs), unconfirmed, immature, bonded, total
- UtxoResponse — tx_hash, output_index, amount, output_type, lock_until, height, spendable, pending, condition, nft, asset, bridge
- HistoryEntryResponse — hash, tx_type, block_hash, height, timestamp, amount_received, amount_sent, fee, confirmations, from, to
- ProducerResponse — public_key, address_hash, registration_height, bond_amount, bond_count, status, era, pending_withdrawals, pending_updates, bls_pubkey
- BondDetailsResponse — per-bond FIFO vesting details including creation_slot, age_slots, penalty_pct, vested, maturation_slot; summary by quarter (q1/q2/q3/vested)
- SlotScheduleResponse / ProducerScheduleResponse — slot schedule and per-producer schedule with weekly_earnings, doubling_weeks
- AttestationStatsResponse / ProducerAttestationStats — per-producer attested_minutes, qualified, has_bls
- ChainStatsResponse, ChainInfoResponse, NetworkInfoResponse, MempoolInfoResponse, EpochInfoResponse
- BackfillStatusResponse — running, imported, total, pct, error
- condition_to_json() — public free function; converts Condition enum to human-readable JSON value

### WebSocket
- WsEvent — NewBlock {hash, height, slot, timestamp, producer, tx_count}, NewTx {hash, tx_type, size, fee}
- broadcast_channel() — capacity 256; returns (Sender, Receiver)

### RPC Methods (47)
- getBlockByHash — fetch block by hash; note: height field in response is always 0 (no reverse index)
- getBlockByHeight — fetch block by height
- getBlockRaw — base64-encoded bincode-serialized block with BLAKE3 checksum; used by archiver backfill
- getBlockData — raw extra_data from a specific output identified by hash and global output_index (not per-tx index)
- getTransaction — mempool-first then confirmed; resolves input addresses, computes fee
- getNftByTokenId — scans UTXO set for NFT output with matching token_id; returns owner, outpoint, content hash, size, royalty
- sendTransaction — decode hex, add to mempool, broadcast; returns tx hash hex
- getBalance — confirmed/unconfirmed/immature/bonded/total; confirmed field = spendable (mempool-spent UTXOs subtracted)
- getUtxos — UTXOs for address with type, lock_until, condition, NFT metadata, fungible asset metadata, bridge HTLC metadata; appends pending mempool outputs
- getMempoolInfo — tx_count, total_size, min_fee_rate, max_size, max_count
- getNetworkInfo — peer_id, peer_count, syncing, sync_progress
- getPeerInfo — detailed peer list
- getChainInfo — network, version, height, hash, slot, genesis_hash, reward_pool_balance
- getNodeInfo — version, network, peerId, peerCount, platform, arch
- getEpochInfo — epoch boundaries, blocks_remaining, last_complete_epoch, block_reward
- getNetworkParams — bondUnit, slotDuration, slotsPerEpoch, blocksPerRewardEpoch, coinbaseMaturity, initialReward, genesisTime
- getProducer — status, bond_count/amount (UTXO-derived with ProducerInfo fallback), era, pending_updates, pending_withdrawals, bls_pubkey
- getProducers — all or active-only producers; appends pending registrations not yet in producer set
- getBondDetails — per-bond FIFO vesting details; includes summary by vesting quarter
- getHistory — paginated tx history for address using addr_tx_index for O(1) height lookup; maps all 30 tx types
- submitVote — validates producer, verifies Ed25519 signature over "version:vote:timestamp", broadcasts vote
- getUpdateStatus — delegates to update_status callback reading live UpdateService state
- getMaintainerSet — reads on-chain MaintainerState if available, else derives from first INITIAL_MAINTAINER_COUNT producers
- submitMaintainerChange — validates action (add/remove), parses target pubkey and 3+ maintainer signatures, creates and submits system transaction
- getSlotSchedule — upcoming slot assignments using select_producer_for_slot; returns SlotScheduleResponse
- getProducerSchedule — all slots in current epoch for a specific producer; fill_rate, weekly_earnings, doubling_weeks
- getAttestationStats — scans all blocks in current epoch decoding attestation bitfields using 3-era decoder strategy (legacy sort_all before REWARDS_EPOCH_LIST_FIX_HEIGHT / epoch_list / full bitfield decode after FULL_BITFIELD_DECODE_HEIGHT) gated on activation heights
- getChainStats — total_supply, address_count, utxo_count, active_producers, total_staked, height, reward_pool_balance, total_confirmed
- getStateRootDebug — computes ChainState/UtxoSet/ProducerSet canonical hashes and combined state_root; returns per-component hashes plus totalMinted, registrationSeq
- getUtxoDiff — sorted list of (outpoint, entry_hash, detail) for all UTXOs; accepts optional referenceHashes to return only differing entries; InMemory UTXO set only (returns internal error for RocksDb)
- getStateSnapshot — full StateSnapshot; includes epochBondSnapshot and epochAccumulators fields for correct post-snap-sync convergence; returns hex-encoded components with height, blockHash, stateRoot, totalBytes
- getPoolInfo — AMM pool details (reserves, fee_bps, price, TWAP, LP shares, status, creation_slot)
- getPoolList — all AMM pools; deduplicates by pool_id keeping UTXO with highest reserveA
- getPoolPrice — spot price; optionally computes TWAP over windowSlots using cumulative_price fixed-point
- getSwapQuote — simulate constant-product swap; returns amount_out, price_impact, fee; no state mutation
- getLoanInfo — looks up Collateral UTXO by outpoint; computes accrued interest, total_debt, LTV bps, liquidatable flag
- getLoanList — all active Collateral UTXOs; optional filter by borrower hash
- pruneBlocks — prune block_store below (tip - keep_last_n) with minimum 2,000-block retention floor; optionally verifies archive coverage before pruning (admin)
- getStorageInfo — chain_tip, height_range, column_family entry counts, prunable_blocks estimate, archive_height
- backfillFromPeer — SSRF-protected URL validation; tip-agreement preflight check (rejects diverged tip hashes); Phase-2 commitment-divergence reverse scan; background tokio task fetching blocks via getBlockRaw, verifying BLAKE3, writing to block_store (admin)
- backfillStatus — reads BackfillState atomics; returns running, imported, total, pct, error
- verifyChainIntegrity — fast path O(1) via persisted incremental chain commitment; fallback full BLAKE3 scan; returns missing height ranges, missingCount, chainCommitment hex
- pauseProduction — calls sync_manager.block_production(); node continues syncing and serving RPC (admin)
- resumeProduction — calls sync_manager.unblock_production() (admin)
- createCheckpoint — validates optional path (rejects absolute paths and ../ traversal); creates RocksDB hard-link checkpoints for state_db and block_store under {data_dir}/checkpoints/h{height}-{timestamp}/ (admin)
- getGuardianStatus — reads production_paused from SyncManager; scans checkpoints directory; returns chain_height, chain_slot, best_hash, last_checkpoint, last_healthy_checkpoint

### RPC Error Codes
- PARSE_ERROR = -32700
- INVALID_REQUEST = -32600
- METHOD_NOT_FOUND = -32601
- INVALID_PARAMS = -32602
- INTERNAL_ERROR = -32603
- BLOCK_NOT_FOUND = -32000
- TX_NOT_FOUND = -32001
- INVALID_TX = -32002
- TX_ALREADY_KNOWN = -32003
- MEMPOOL_FULL = -32004
- UTXO_NOT_FOUND = -32005
- PRODUCER_NOT_FOUND = -32006
- POOL_NOT_FOUND = -32007
- UNAUTHORIZED = -32008

---

## 8. AUTO-UPDATE (`crates/updater`)

NOTE: UpdateService, PendingUpdate, and spawn_update_service() are NOT in this crate — they live in bins/node/src/updater/. This crate provides the primitives consumed by the node binary.

### Hard Fork Schedule
- HardForkInfo — activation_height, min_version, consensus_changes; is_active(), version_is_compatible(), should_stop_producing(), blocks_until_activation()
- HardForkSchedule — sorted list of fork entries; new(), add(), should_stop_producing(), next_pending(), active_forks(), all(), is_empty(), fork_id(), default_schedule(), for_network()
- HardForkSchedule::fork_id(genesis_hash: &Hash, current_height: u64) -> Hash — BLAKE3(genesis || h1_le || h2_le ...) for all active fork heights; Hash::ZERO when no forks active. NOTE: there is no function called current_fork_id(); passing u64::MAX as current_height activates ALL entries
- HardForkSchedule::should_stop_producing() — true if ANY fork in schedule blocks production
- HardForkSchedule::for_network() — Mainnet: one entry at h=2750 (min_version="6.14.11", EpochState state root inclusion); Testnet: placeholder at h=10_000_080; Devnet: empty schedule

### Update Types
- ReleaseMetadata — metadata.json format: version, networks, optional min_protocol_version; used to filter releases by network
- Release — version, binary_sha256, binary_url_template, changelog, published_at, signatures, target_networks
- MaintainerSignature — public_key (hex), signature (hex)
- SignaturesFile — version, checksums_sha256, signatures
- UpdateConfig — enabled, notify_only, auto_rollback, check_interval_secs, veto_period_secs, grace_period_secs, custom_url; Default: enabled=true, notify_only=false, auto_rollback=true
- GithubReleaseInfo — version, tarball_url, expected_hash, changelog; fetched directly from GitHub API
- VoteResult — total_producers, veto_count, veto_percent, approved
- UpdateError — 11 variants: InsufficientSignatures, InvalidSignature, HashMismatch, DownloadFailed, InstallFailed, Network, Io, Json, VetoPeriodActive, RejectedByVeto, NotApproved

### Download & Verification
- fetch_latest_release() — fetch from custom URL → GitHub API → fallback mirror; filters by network target
- fetch_github_release() — fetch specific or latest release from GitHub API; downloads CHECKSUMS.txt + parses hash for current platform
- download_from_url() — raw HTTP GET download with 5-minute timeout
- download_signatures_json() — download SIGNATURES.json for a version; returns None if not found
- download_checksums_txt() — download CHECKSUMS.txt; returns (content, sha256_of_file)
- verify_hash() — SHA-256 verification of downloaded binary (case-insensitive hex compare)
- verify_release_signatures() — verify release using bootstrap keys only (convenience wrapper)
- verify_release_signatures_with_keys() — verify with on-chain keys if non-empty, else bootstrap keys; requires REQUIRED_SIGNATURES (3) valid signatures
- sign_release_hash() — sign "version:sha256" with maintainer key; returns MaintainerSignature

### Application
- apply_update() — apply update after security checks (veto period ended + approved); downloads, verifies, backs up, installs
- auto_apply_from_github() — bypass approval checks (already verified by UpdateService); fetches GitHub release, downloads tarball, extracts and installs doli-node; also updates doli CLI binary best-effort
- backup_current() — copy current binary to .backup path; removes old backup first
- rollback() — restore current binary from .backup
- restart_node() — exec()-restart (Unix) or spawn+exit (Windows); does not return
- extract_binary_from_tarball() — extract "doli-node" from .tar.gz archive
- extract_named_binary_from_tarball() — extract named binary from .tar.gz archive by filename match
- install_binary() — write binary to temp path then atomic rename; falls back to sudo cp if Permission denied
- current_binary_path() — path to current running binary; strips " (deleted)" suffix for Linux atomic replace

### Enforcement
- VersionEnforcement — min_version, enforcement_time, active, binary_ready (serde default=false); from_approved_release(), from_approved_release_with_params(), should_enforce(), seconds_until_enforcement(), hours_until_enforcement(), version_meets_requirement()
- ProductionBlocked — current_version, required_version; Display renders boxed warning banner
- ENFORCEMENT_TIMEOUT_SECS = 1,800 (30 min); enforcement auto-expires to prevent indefinite halt on download failure
- check_production_allowed() — blocks production if: enforcement_time passed AND version too old AND binary_ready AND not timed out
- veto_deadline() / veto_period_ended() — mainnet-default timing helpers
- in_grace_period() / in_grace_period_for_network() — true if now is between veto end and grace end
- grace_period_deadline() / grace_period_deadline_for_network() — timestamp when grace period ends

### Voting
- Vote — Approve, Veto
- VoteMessage — version, vote, producer_id, timestamp, signature; new(), message_bytes() ("version:vote_str:timestamp"), verify()
- VoteTracker — per-version vote tracking with weighted veto; fields: version, vetos (HashSet), approvals (HashSet), producer_weights (HashMap); new(), with_weights(), set_weights(), record_vote(), veto_count(), approval_count(), total_votes(), should_reject(), should_reject_weighted(), veto_weight(), approval_weight(), veto_percent(), veto_percent_weighted(), veto_producers()

### Update Parameters (UpdateParams — on UpdateParams, NOT VoteTracker)
- UpdateParams — per-network timing: veto_period_secs, grace_period_secs, min_voting_age_secs, min_voting_age_blocks, check_interval_secs, crash_window_secs, crash_threshold, seniority_maturity_blocks, seniority_step_blocks, network
- UpdateParams::for_network() — create from network configuration
- UpdateParams::calculate_vote_weight(bond_count, blocks_active) — vote weight = bond_count × seniority_multiplier (1.0 + min(years,4)×0.75)
- UpdateParams::seniority_multiplier(blocks_active) — 1.0–4.0 based on registration age
- UpdateParams::is_eligible_to_vote(), veto_deadline(), grace_period_deadline(), veto_period_ended(), in_grace_period()
- VETO_PERIOD = 300s (5 min)
- GRACE_PERIOD = 3,600s (1 hour)
- VETO_THRESHOLD_PERCENT = 40
- REQUIRED_SIGNATURES = 3 (of 5)
- CHECK_INTERVAL = 21,600s (6 hours)

### Watchdog
- UpdateWatchdog — crash detection and automatic rollback; new(data_dir, network), record_update(), record_clean_shutdown(), check_and_maybe_rollback(), clear()
- WatchdogState — last_update_version, last_update_time, crash_timestamps, clean_shutdown; load(), save()
- DEFAULT_CRASH_THRESHOLD — private constant (3); exposed via UpdateParams::crash_threshold field

### Bootstrap Keys
- BOOTSTRAP_MAINTAINER_KEYS_MAINNET — 5 Ed25519 hex public keys (mainnet N1–N5)
- BOOTSTRAP_MAINTAINER_KEYS_TESTNET — 5 Ed25519 hex public keys (testnet NT1–NT5)
- bootstrap_maintainer_keys(network) — returns network-specific array
- is_using_placeholder_keys(network) — true if any key starts with "00000000" (must be false before mainnet launch)
- assert_production_keys(network) — panics on placeholder keys; call during node init

### Utility (crates/updater/src/util.rs)
- current_timestamp() — current Unix timestamp in seconds
- current_version() — current binary version from CARGO_PKG_VERSION
- is_newer_version(new, current) — true if new semver > current; strips leading 'v'
- platform_identifier() — "linux-x64", "linux-arm64", "macos-x64", "macos-arm64", or "unknown"

### Test Keys (crates/updater/src/test_keys.rs — devnet only)
- TestMaintainerKey — public_key (hex), private_key (hex)
- TEST_MAINTAINER_KEYS — static LazyLock<[TestMaintainerKey; 5]>; 5 deterministic key pairs from seeds 1–5
- test_maintainer_pubkeys() — hex public keys from TEST_MAINTAINER_KEYS
- sign_with_test_key(index, message) — sign with test key at index; returns hex sig or None
- create_test_release_signatures(version, sha256) — sign with first 3 test maintainers
- should_use_test_keys() — true if DOLI_TEST_KEYS=1 env var set

---

## 9. NODE BINARY (`bins/node`)

### CLI
- Cli — top-level parser: network (-n), config (-c), data_dir (-d), log_level, command
- Data-dir resolution priority: --data-dir flag → DOLI_DATA_DIR env → platform default (/var/lib/doli/<net> Linux, ~/Library/Application Support/doli/<net> macOS) → legacy ~/.doli/<net>
- Commands — 15 variants: Run, Init, Status, Import, Export, Update, Maintainer, Truncate, Recover, Restore, Reindex, Devnet, Release, Upgrade, CheckpointInfo
- UpdateCommands — 7 variants: Check, Status, Vote, Votes, Apply, Rollback, Verify
- MaintainerCommands — List, Remove, Add, Sign, Verify
- DevnetCommands — 6 variants: Init, Start, Stop, Status, Clean, AddProducer
- ReleaseCommands — Sign
- expand_tilde_path() — expands leading ~ to home directory (needed for clap default values)
- Key env vars: DOLI_DATA_DIR (data dir override), DOLI_PRODUCER_KEY (producer key path fallback), DOLI_RPC_ADMIN_TOKEN (RPC admin bearer token), DOLI_MAX_PEERS (max peer override for start_network)

### Configuration
- NodeConfig — network, data_dir, listen_addr, bootstrap_nodes, max_peers, rpc, producer, no_dht, relay_server, genesis_time_override, chainspec (serde-skip), slot_duration_override, external_address (serde-skip), no_snap_sync, seed_mode, auto_checkpoint_interval, bootnode_enrs, no_discv5, discv5_port; NodeConfig::for_network(network)
- RpcConfig — enabled, listen_addr, allowed_methods, admin_token, allowed_origins; RpcConfig::for_network(network); Default is mainnet-specific (port 8500)
- ProducerConfig — key_file, reward_address

### Entry Points (bins/node/src/run.rs)
- run_node() — full node entry: loads/creates producer key and BLS key, runs producer safety checks, builds NodeConfig, starts metrics server, loads chainspec, loads ProducerSet, spawns UpdateService, creates Node::new(), wires vote_tx/pending_update/maintainer_state, optionally starts BlockArchiver, spawns node.run(), waits for SIGINT/SIGTERM, graceful shutdown (30s timeout)
- run_bootnode() — lightweight UDP-only Discv5 bootnode; no gossip/sync; loads or generates node key; prints local ENR; event loop logs peer events every 60s
- load_producer_key(path) — reads JSON wallet file; deserializes Wallet{addresses: Vec<{private_key}>}; returns KeyPair from first Ed25519 private key
- load_bls_key(path) — reads JSON wallet file; returns BlsKeyPair if bls_private_key field present; returns None gracefully for pre-BLS wallets

### Node Core (62 fields)
- Node — 62 fields: config, params, block_store, state_db, utxo_set, chain_state, producer_set, mempool, network, seed_peer_ids, seeds_released, sync_manager, shutdown, producer_key, bls_key, last_produced_slot, known_producers, first_peer_connected, equivocation_detector, vdf_calibrator, fork_block_cache, last_resync_time, last_producer_list_change, producer_gset, adaptive_gossip, our_announcement, announcement_sequence, last_broadcast_gset_len, signed_slots_db, consecutive_fork_blocks, shallow_rollback_count, cumulative_rollback_depth, seen_blocks_for_slot, epoch_state, is_active_producer, last_active_status_epoch, vote_tx, pending_update, last_peer_redial, bootstrap_backoff, producer_liveness, genesis_vdf_output, cached_state_root, cached_genesis_producers, port_check_done, maintainer_state, archive_tx, pending_archive, archive_dir, archive_caught_up, ws_sender, minute_tracker, rejected_fork_tips, snap_sync_height, sync_requests_this_interval, last_checkpoint_height, pending_tx_announcements, hardfork_schedule, peer_churn, last_integrity_check_tip, last_active_fork_correction_height
- PEER_CHURN_MAX: usize = 5 — max connect+disconnect events per peer within PEER_CHURN_WINDOW before rate-limit
- PEER_CHURN_WINDOW: Duration = 30s — rolling window for peer churn tracking
- Node::set_vote_tx() — wires gossip vote channel to UpdateService
- Node::set_pending_update() — wires shared pending update state to RPC
- Node::set_archive_tx() — wires archive channel and directory
- Node::set_maintainer_state() — sets on-chain maintainer state
- Node::block_store() — returns Arc<BlockStore> reference (for archiver catch-up)
- Node::current_fork_id() — computes fork_id via hardfork_schedule.fork_id(genesis_hash, u64::MAX); u64::MAX activates ALL scheduled forks for peer compatibility checks
- Node::bond_weights_for_scheduling() — computes bond weights from epoch snapshot (or UTXO fallback for epoch 0); single source of truth for scheduler bond weights
- Node::best_height() — async getter for chain tip height
- Node::shutdown() — sets shutdown flag

### Node Initialization
- Node::new() — ~890 lines: opens BlockStore and StateDb, cleans RocksDB diagnostic logs, migrates legacy state files (chain_state.bin, producers.bin, utxo_rocks, utxo.bin) into unified StateDb, validates genesis hash, checks chain state consistency (body gap repair, snap sync re-seed, missing tip recovery), applies slot_duration and genesis_time overrides, rebuilds producer liveness, loads EpochState (version check + UTXO reconstruction fallback), creates EquivocationDetector and VdfCalibrator, initializes ProducerGSet and AdaptiveGossip, builds Node struct
- Node::new_for_test() — ~175 lines: minimal Node for integration tests using real RocksDB; no networking, no archiver, no updater; VdfCalibrator set to 100 iterations; SyncManager with 0 grace period and 0 min peers

### Node Startup
- run() — checks placeholder maintainer keys on mainnet, calls start_network(), registers self as bootstrap producer (restores GSet sequence, creates ProducerAnnouncement, broadcasts initial announcements), computes genesis VDF proof in background (spawn_blocking), calls start_rpc() if enabled, calls run_event_loop(), calls shutdown() on exit
- start_network() — configures NetworkService: parses listen_addr, applies DOLI_MAX_PEERS env override, extracts seed PeerIds for post-bootstrap release, computes dynamic gossip mesh, enables relay server mode (max_peers=125), configures discv5/ENRs
- start_rpc() — creates RpcServerConfig (listen_addr, CORS, DOLI_RPC_ADMIN_TOKEN), builds RpcContext with all state references and method-specific callbacks, creates WebSocket broadcast channel, stores ws_sender, spawns RpcServer
- recompute_active_status(height) — pub(super); checks if epoch changed, looks up own pubkey in epoch_state.active_list, updates is_active_producer and last_active_status_epoch
- create_and_broadcast_attestation(block_hash, slot, height) — creates Ed25519+BLS Attestation (Ed25519-only if no BLS key), adds own weight to SyncManager finality tracker, broadcasts via gossip; performs direct attestation delivery to producer of slot+1 via SyncRequest::DirectAttestation if a v5+ peer is found

### Event Loop
- run_event_loop() — production timer 200ms devnet / 1s otherwise; gossip timer (adaptive); biased tokio::select! with 3 arms: (1) network event (highest priority), (2) production timer (drain pending events then try_produce_block + run_periodic_tasks; purges toxic mempool TXs), (3) gossip timer (merge own announcement into GSet, purge ghost producers >14400s, broadcast delta bloom or full fallback, log schedule divergence, update adaptive interval); production escape-hatch: if network arm runs but production hasn't fired within production_interval, drain up to max_peers*3 events then force production check
- handle_network_event() — dispatches 20 NetworkEvent variants: PeerConnected, PeerDisconnected, NewBlock, NewHeader (debug log only / no-op), NewTransaction, PeerStatus, StatusRequest, SyncRequest, SyncResponse, NetworkMismatch, GenesisMismatch, VersionMismatch, ProducersAnnounced, ProducerAnnouncementsReceived, ProducerDigestReceived, NewVote, NewHeartbeat (no-op), TxAnnouncement, TxFetchRequest, TxFetchResponse, NewAttestation
- handle_sync_request_bg() — free async function (#[allow(dead_code)]); background handler; 9 SyncRequest variants: GetHeaders, GetBodies, GetBlockByHeight, GetBlockByHash, GetHeadersByHeight (INC-I-012 F1, max 2000 headers), DirectAttestation (re-broadcasts), GetStateRoot (cached), GetStateSnapshot (includes anchor header, epoch_bond_snapshot, epoch_accumulators; epoch_state_bytes=None in this path)

### Network Events (15 public handlers)
- on_peer_connected() — record_peer_churn_and_check; sync_manager.set_peer_connected(); sends StatusRequest
- on_peer_disconnected() — record_peer_churn_and_check; sync_manager.remove_peer(); rate-limited bootstrap redial when peer count drops to zero
- on_new_block_event() — snap sync guard (drops during snap sync); stale gossip drop (slot < current_slot - 50 if current_slot > 50); rejected_fork_tips cache check (1000-entry cap); slot sanity checks; delegates to handle_new_block
- on_peer_status() — sync_manager.update_peer() or add_peer(); note_peer_status_received(); maybe_add_bootstrap_producer()
- on_status_request() — builds StatusResponse (version=CURRENT_PROTOCOL_VERSION, network_id, genesis_hash, best_height, best_hash, best_slot, producer_pubkey); sends via network.send_status_response(); maybe_add_bootstrap_producer()
- on_sync_request() — global rate limit 24/interval (MAX_SYNC_REQUESTS_PER_INTERVAL); DirectAttestation registered locally in minute_tracker first (gossipsub does not deliver published messages back to publisher); delegates to handle_sync_request
- on_sync_response() — passes to sync_manager.handle_response(); applies blocks via handle_new_block; applies completed snap snapshot via apply_snap_snapshot
- on_producers_announced() — legacy full-list merge; Testnet/Devnet/genesis only
- on_producer_announcements() — GSet CRDT merge; adaptive_gossip.on_gossip_result(); syncs new producers into known_producers
- on_producer_digest() — bloom filter delta sync; reads producer_gset.delta_for_peer(); sends delta to requesting peer
- on_new_vote() — deserializes VoteMessage from JSON bytes; forwards to UpdateService via vote_tx.try_send()
- on_new_attestation() — verifies Attestation signature; adds weight to sync manager; records in minute_tracker; flush_finalized_to_archive
- on_tx_announcement() — finds unknown hashes; adds to pending_tx_announcements (plain HashMap<PeerId, Vec<Hash>> on Node, NOT PendingTxAnnouncements struct); sends network.request_tx_fetch() per peer
- on_tx_fetch_request() — looks up requested hashes in mempool; sends TxFetchResponse
- on_tx_fetch_response() — removes hash from pending_tx_announcements; calls handle_new_transaction per tx

### Block Handling
- handle_new_block() — [APPLY_START] log; duplicate check (O(1)); equivocation check; if prev_hash != best_hash: genesis-hash drop guard, fork-id drop guard, height-occupied guard (fork choice: lower slot wins), orphan Case A (normal: request current_height+1 from peer) vs Case B (fork orphan: request current_height); fork_block_cache (cap 100); weight-based reorg via sync_manager.handle_new_block_weighted(); snap-sync ValidationMode selection (Light if snap_sync_height.is_some()); post-apply orphan drain; post-apply catch-up request; [APPLY_END] log
- execute_reorg() — chain linkage validation; ensure_blocks_present / [FORK_GUARD_BACKFILL_REQUIRED] guard; genesis-boundary cached_genesis_producers invalidation; undo-based rollback (preferred): reverts UTXOs + restores ProducerSet from snapshot; legacy fallback (no undo data): rebuild UTXO and ProducerSet from genesis; rebuild_producer_liveness(); atomic_replace() persistence; ValidationMode::Light for all new blocks; mempool.revalidate()

### Block Application
- apply_block() — snap sync guard (silently drops blocks at or below snap_sync_height); duplicate check with poisoned-block re-apply path; validation; tx processing loop; chain state update; finality tracking; mempool pruning; atomic StateDb commit with undo data (UNDO_KEEP_DEPTH = 2,000); emits [STATE_FP] diagnostics fingerprint (8 consensus-derived hashes)
- UNDO_KEEP_DEPTH = 2,000 — number of historical undo entries to retain (2× MAX_REORG_DEPTH)
- process_transaction_utxos() — captures undo log for spent UTXOs; pre-activation EpochReward pool consumption side-effect path (inputs empty, pool UTXOs removed directly, pre-EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT); validates UTXO spending via validate_transaction_with_utxos; enforces NFT/Pool unique-ID uniqueness; mirrors changes in atomic batch
- process_transaction_producer_effects() — queues epoch-deferred PendingProducerUpdates for: Registration (skipped during genesis phase — only process_unbonding runs; genesis producers come from maybe_complete_genesis), Exit, SlashProducer, AddBond, RequestWithdrawal, DelegateBond, RevokeDelegation
- process_unbonding(producers, height) — static method (no self); calls ProducerSet::process_unbonding(height, UNBONDING_PERIOD); logs completions
- update_known_producers(new_registrations, height) — appends newly registered producer pubkeys to known_producers; sorts for deterministic ordering; testnet/devnet or genesis only; sets last_producer_list_change timestamp
- process_transaction_governance() — MaintainerAdd: verifies multisig, add_maintainer, persists immediately; MaintainerRemove: same; ProtocolActivation: verifies multisig, returns Some((version, epoch)) for deferred activation in update_chain_state_for_block
- derive_ad_hoc_maintainer_set(producers, height) — associated function (no self); fallback when on-chain maintainer set is not yet bootstrapped; sorts all producers by registered_at, takes first INITIAL_MAINTAINER_COUNT
- update_chain_state_for_block() — acquires chain_state write lock, state.update(hash, height, slot), clears snap-sync marker, schedules pending protocol activation, activates pending version at epoch boundaries, caches state root via storage::compute_state_root
- track_finality_and_apply_deferred() — computes total active-producer weight; sync_manager.track_block_for_finality; applies ProducerSet::apply_pending_updates at epoch boundaries (every block during epoch 0); calls maybe_bootstrap_maintainer_set; returns true if full producer batch write needed
- post_commit_actions() — recompute_active_status; attestation bitfield decode (full-decode path [base | extra-sorted] after FULL_BITFIELD_DECODE_HEIGHT, legacy header path before BITFIELD_BODY_ACTIVATION_HEIGHT); [ATTEST_DECODE] / [ATTEST_MISS] diagnostics; epoch_state.accumulate_block; persists epoch_state every block; incremental chain commitment BLAKE3(prev || block_hash); at epoch boundaries: builds EpochSnapshot, calls EpochState::derive_at_boundary, persists new producer list/active list/attestation accumulators, rotates epoch_state; create_and_broadcast_attestation; archives buffer; WsEvent::NewBlock broadcast
- maybe_complete_genesis() — triggers at height == genesis_blocks + 1; clears entire ProducerSet (removes phantom bootstrap producers); consumes all reward-pool UTXOs sorted deterministically (height/tx_hash/index), tracked in undo log and batch; creates Bond UTXO per genesis producer with lock=u64::MAX and registered_at=0 (bypasses ACTIVATION_DELAY); returns surplus pool funds as normal UTXO

### Production
- try_produce_block() — version enforcement (LAST_WARNING rate-limited 1/min), hard fork check (LAST_FORK_WARNING rate-limited 1/min), slot dedup, production gate (handle_production_authorization), peer-aware behind-network check (3 blocks if height < 10, else 5), bootstrap vs epoch scheduling, rank guard, propagation delay (1s / 500ms bootstrap), slashing protection via signed_slots_db.check_and_mark(), block content build, drain events, VDF, post-VDF stale parent re-check, behind-tip broadcast suppression (skip if net_tip >= height + 2 && peer_count >= 3), apply, broadcast, attest; on apply_block failure: rollback + mempool purge (poison recovery)
- resolve_bootstrap_eligibility() — 8 guard stages: stability wait, bootstrap node connection check, joining node bootstrap guard (BOOTSTRAP_MIN_HEIGHT=3), bootstrap timeout (60s devnet / 180s testnet), chain tip freshness check, peer height check, discovery grace (3s devnet / 30s testnet), late joiner guard; then liveness-filtered round-robin from known/on-chain/GSet producer list with ACTIVATION_DELAY filter
- resolve_epoch_eligibility() — epoch-frozen deterministic round-robin (slot % active_list.len()); uses epoch_state.active_list if non-empty, else falls back to epoch_state.producer_list
- build_block_content() — fork_id selection (hardfork_schedule.fork_id(genesis_hash, height)), epoch reward TX at epoch boundaries (skip epoch 0; explicit pool UTXO inputs post-EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT), genesis VDF Registration TX inclusion, mempool TX inclusion with 60% slot deadline and MAX_BLOCK_USER_DATA=1MB budget, NFT/Pool unique-ID conflict pre-check, coinbase, slot boundary abort, attestation bitfield encoding (body vs presence_root based on BITFIELD_BODY_ACTIVATION_HEIGHT), missed_producers computation (MAX_MISSED_PER_BLOCK=3 cap, max_total=list_len/3, gap ≤ 3 only)
- drain_pending_events() — non-blocking drain of all pending NetworkEvents before VDF; returns true if chain advanced (abort production)
- compute_block_vdf() — construct_vdf_input(prev_hash, merkle_root, slot, producer); spawn_blocking(hash_chain_vdf(..., iterations)); records timing in vdf_calibrator; devnet: uses prev_hash as placeholder with empty proof
- aggregate_bls_signatures() — collects BLS sigs for current attestation minute from minute_tracker; calls crypto::bls_aggregate; returns aggregated sig bytes or empty Vec
- attest_own_block() — calls create_and_broadcast_attestation then records own attestation in minute_tracker (with BLS sig if bls_key present)
- handle_production_authorization() — calls sync_manager.can_produce(current_slot); handles 13 ProductionAuthorization variants: Authorized (resets fork/rollback counters), BlockedSyncing, BlockedResync, BlockedBehindPeers, BlockedAheadOfPeers (increments fork counter + try_trigger_fork_recovery + maybe_auto_resync), BlockedSyncFailures (if failure_count >= 50: fork counter + maybe_auto_resync), BlockedInsufficientPeers, BlockedChainMismatch (increments fork counter + try_trigger_fork_recovery + maybe_auto_resync), BlockedNoGossipActivity, BlockedExplicit, BlockedBootstrap, BlockedConflictsFinality, BlockedAwaitingCanonicalBlock

### Rewards
- calculate_epoch_rewards() — scans epoch blocks, decodes attestation bitfields (3-tier qualification: Tier 1 ≥54/60 min, Tier 2 ≥80% of median, Tier 3 accumulate), bond-weighted distribution among qualifiers, delegation reward splitting (own-share/delegate-fee/staker-pool); fail-fast returns empty vec when block store is incomplete for non-epoch-0 epochs (M-RC9, INC-I-034); gated on REWARDS_EPOCH_LIST_FIX_HEIGHT and BITFIELD_BODY_ACTIVATION_HEIGHT activation heights
- handle_equivocation() — receives EquivocationProof, creates SlashProducer tx, adds via add_system_transaction, broadcasts to network
- rebuild_producer_liveness() — clears producer_liveness and re-populates by scanning last LIVENESS_WINDOW_MIN blocks from block_store; called after any rollback
- rebuild_epoch_state_from_blocks() — reconstructs epoch_state from block history when undo data absent or pre-upgrade; 3-epoch attestation lookback (Fix #4A), accumulator rebuild from scan (Fix #4B), edge scan guard (Fix #4B-edge), mid-epoch accumulator replay (Fix #4C); DEPRECATED: only fires on pre-upgrade undo data; firing on post-upgrade blocks indicates persistence bug
- rebuild_producer_set_from_blocks(&self, producers: &mut ProducerSet, target_height: u64) -> Result<()> — clears and fully replays producer state by iterating blocks 1..=target_height; replicates genesis phase completion; applies deferred pending_updates at every epoch boundary and every block in epoch 0; calls process_unbonding after each block; does NOT apply remaining pending_updates if target_height is mid-epoch

### Rollback
- rollback_one_block() — O(1) when undo data present: reverts UTXOs (remove created, restore spent), restores ProducerSet from bincode snapshot (fallback to rebuild_producer_set_from_blocks on deserialization failure); O(chain) legacy fallback when no undo data: clears UTXO set and replays all blocks from genesis; refuses rollback to height 0 from established chain (Fix 3); refuses if cumulative_rollback_depth >= MAX_CUMULATIVE_ROLLBACK (Fix 4); invalidates cached_genesis_producers on genesis boundary crossing; calls rebuild_producer_liveness; updates chain_state and sync_manager (note_rollback_completed); atomic_replace() persistence; restores epoch_state from undo.epoch_state_snapshot if present, else rebuild_epoch_state_from_blocks; calls state_db.delete_chain_commitment()
- resolve_shallow_fork() — heuristic path (stuck_fork_signal NOT set): requires empty_headers >= 3 AND last_applied_secs >= 300; stuck path (signal set, Fix #2c): bypasses heuristic but applies anti-cascade backoff (suppresses if shallow_rollback_count >= 3 and last_applied_secs < 60); skips rollback if height advanced since last rollback (LAST_ROLLBACK_HEIGHT tracking); for gap <= 50 and shallow_rollback_count < 50: calls rollback_one_block; post_recovery_grace_active early-return guard prevents Sisyphean loops
- MAX_CUMULATIVE_ROLLBACK = 50 — local constant inside rollback_one_block method body (not a module-level public constant)
- LAST_ROLLBACK_HEIGHT — AtomicU64 file-scoped static; tracks local height at most recent rollback; detects whether height advanced (sync is working)

### Fork Recovery
- handle_completed_fork_recovery() — records per-block weights into reorg handler, moves blocks into fork_block_cache, attempts check_reorg_weighted (single-block forks), falls back to plan_reorg for deeper forks; deterministic hash tie-break (lower hash wins) when weight_delta == 0
- try_trigger_fork_recovery() — checks can_start_fork_recovery(), picks orphan seed from fork_block_cache, starts sync_manager.start_fork_recovery(orphan, peer)
- try_apply_cached_chain() — builds contiguous chain backwards from latest_block to our_tip (MAX_CHAIN_LENGTH = 50 hops) via fork_block_cache; validates producer eligibility; applies blocks via apply_block(ValidationMode::Full)
- maybe_auto_resync() — exponential backoff (60s * 2^min(resyncs, 4)); guards: fork threshold (devnet=5, other=10), height 0, resync in progress, MAX_CONSECUTIVE_RESYNCS hard cap, blocks_applied > 0 progress check; when all pass: calls rollback_one_block()
- apply_checkpoint_state() — #[allow(dead_code)]; verifies state root against received_state_root and hardcoded CHECKPOINT_STATE_ROOT; deserializes all three state components; only called for new nodes (height=0) during initial sync; NOT used in current production
- apply_snap_snapshot() — re-verifies state root; deserializes ChainState, UtxoSet, ProducerSet; envelope/state consistency check; atomic state replacement; mark_snap_synced(); seed canonical index; Option C: anchor header persistence; fast-path EpochState if epoch_state_bytes present; legacy reconstruction path (runs only if epoch_state_bytes absent or producer_list empty): bond snapshot (peer payload → persisted → UTXO fallback), epoch_accumulators_bytes, EpochState::derive_at_boundary(); sets snap_sync_height; set_store_floor() + record_block_applied_after_snap()

### Validation
- check_producer_eligibility() — lightweight gossip-block eligibility check; verifies producer in active set or GSet; builds weighted producer list from epoch-locked bond snapshot; validates via validate_producer_eligibility(); does NOT validate against local chain state (micro-fork safe)
- validate_block_for_apply() — full pre-apply validation; epoch-locked weighted producers; empty bootstrap producers in Light mode (avoids historical GSet mismatch); liveness split; validates missed_producers (MAX_MISSED_PER_BLOCK=3, membership against epoch producer list); validates attestation bitfield commitment (presence_root == BLAKE3(attestation_bitfield)) post-BITFIELD_BODY_ACTIVATION_HEIGHT; delegates to validate_block_with_mode()
- validate_block_economics() — Coinbase: presence, type, amount (accepts base_reward OR base_reward+extra_fees for version-transition tolerance), recipient must be reward_pool_pubkey_hash(); EpochReward structural checks (both modes): boundary enforcement, epoch-0 exclusion, exactly-one, extra_data >=16 bytes, conservation check; EpochReward full checks (Full mode only): exact match vs calculate_epoch_rewards(), explicit pool input verification post-EPOCH_REWARD_EXPLICIT_INPUTS_HEIGHT; Missing EpochReward at boundary (Full mode): bail if expected and absent
- handle_new_transaction() — checks mempool for duplicate; add via mempool.add_transaction(); broadcasts WsEvent::NewTx; broadcasts to network
- handle_sync_request() — #[allow(dead_code)]; legacy inline handler (production uses handle_sync_request_bg); handles 8 SyncRequest variants including GetStateSnapshot (M7: includes epoch_state_bytes)

### Periodic Tasks
- run_periodic_tasks() — 24 ordered sub-tasks: (1) startup block-store integrity scan, (2) stale seen_blocks_for_slot eviction (keep last 10 slots), (3) ordered sync-block application, (4) snap snapshot consumption, (5) sync manager cleanup + prune_finality, (6) archive catch-up, (7) mempool.expire_old(), (8) fork recovery polling, (9) fork recovery max-depth warning, (10) bootstrap redial with exponential backoff (capped 60s) when peer_count==0, (11) discv5 seed fallback (reconnect TCP seeds after 60s of 0 peers), (12) stale chain detection (3-slot threshold): redial+DHT or request status from up to 10 peers; infected-node recovery (height<10 resets backoff), (13) silence pull (30s no block → catch_up_request), (14) active fork detection: minority fork rollback in stuck mode (>120s + minority) or normal mode with epoch cooldown (max 1 correction per 360 blocks via last_active_fork_correction_height), (15) resolve_shallow_fork(), (16) deep fork detection warning, (17) snap batch requests, (18) next_request() dispatch, (19) periodic status refresh (2s bootstrap / 30s normal, capped at 5 or 20 peers), (20) port reachability warning (one-shot, mainnet producers), (21) auto-checkpoint with health tagging and rotation (keep last 5, numeric sort by height), (22) 30s health diagnostic ([HEALTH] + [SYNC_STATE] log lines, shadow_classify_recovery), (23) seed release (disconnect seeds after 5+ DHT peers + blocks), (24) maybe_run_integrity_check()
- flush_finalized_to_archive() — drains pending_archive up to last_finalized_height(); no-op if no finality checkpoint yet
- maybe_bootstrap_maintainer_set() — one-shot bootstrap of MaintainerSet from first INITIAL_MAINTAINER_COUNT (5) producers sorted by registered_at; persists to disk; no-op if already bootstrapped or fewer than 5 producers
- maybe_run_integrity_check() — pub(crate); checks should_run_integrity_check predicate; spawns blocking block_store.ensure_blocks_present(1, tip); logs [INTEGRITY_CHECK] INFO on success or CRITICAL on gap; updates last_integrity_check_tip
- should_run_integrity_check(current_tip, last_checked_tip, min_interval_blocks) -> bool — pub(crate); pure scheduling predicate; true iff tip > 0 AND (never scanned OR tip advanced >= min_interval_blocks since last scan); defensive against backward tip (rollback)
- parse_checkpoint_height() — pub(crate); parses numeric height from checkpoint dir name "h{N}-{timestamp}"; returns 0 on failure
- INTEGRITY_CHECK_INTERVAL_BLOCKS: u64 = 1,000 — pub(crate); minimum blocks between periodic block-store integrity scans (~3h at 10s slots)

### Genesis
- derive_genesis_producers_from_chain() — scans genesis blocks (heights 1..=genesis_blocks) for Registration txs; deduplicates; memoized via OnceLock (cached_genesis_producers); falls back to hardcoded chainspec producers for snap-synced nodes missing genesis blocks
- genesis_bls_pubkeys() — scans genesis blocks for Registration txs with non-empty bls_pubkey; returns HashMap<PublicKey, Vec<u8>>
- consume_genesis_bond_utxos(utxo: &mut UtxoSet) — static-style (takes explicit UtxoSet param, not &self); consumes all pool UTXOs at height == genesis_blocks + 1, creates Bond UTXOs sorted deterministically, returns remainder as normal UTXO; used during UTXO rebuild to match apply_block() genesis bond migration behavior

### Producer Safety
- ProducerGuard — exclusive OS-level flock on producer.lock; auto-releases on Drop; stale PID reclaim (checks if holder PID is dead)
- SignedSlotsDb — sled-based slot signing history; check_and_mark(): atomic compare_and_swap + flush (critical: flush before return to prevent double-sign on crash); open(), was_signed(), prune(), count()
- ProducerStartupError — AnotherLocalInstance, DuplicateKeyActive{last_block_slot, seconds_ago, wait_seconds}, SlotAlreadySigned{slot}, LockFileFailed(io::Error), SignedSlotsDbFailed(String)
- startup_checks() — async; returns Result<(ProducerGuard, SignedSlotsDb), ProducerStartupError>; acquires lock file, opens signed_slots DB, checks for duplicate active key (unless --force-start); callers MUST hold the returned ProducerGuard or the lock is released
- confirm_force_start() — interactive CLI prompt requiring user to type "I UNDERSTAND"; returns bool
- DUPLICATE_KEY_DETECTION_SECONDS = 300 — 5 min window for duplicate key detection
- DUPLICATE_KEY_DETECTION_BLOCKS = 50 — defined but unused at runtime (#[allow(dead_code)])
- PRODUCER_LOCK_FILE = "producer.lock" — lock file name within data directory
- SIGNED_SLOTS_DB_DIR = "signed_slots.db" — sled database directory name within data directory

### Transaction Announcements
- PendingTxAnnouncements — batch announcement tracker; fields: pending (HashMap<Hash, (Vec<PeerId>, Instant)>), requested (HashSet<Hash>); methods: new(), record(), take_batch(), complete(), expire_old(), len(); NOTE: #[allow(dead_code)] — NOT wired into the live event loop; network_events.rs uses a plain HashMap<PeerId, Vec<Hash>> on Node instead; reserved for future full txfetch integration
- MAX_PENDING = 10,000 — #[allow(dead_code)]
- MAX_FETCH_BATCH = 50 — #[allow(dead_code)]

### Commands (bins/node/src/commands/)
- handle_maintainer_command(action, data_dir, network) — dispatches all MaintainerCommands variants with formatted terminal UI output
- handle_devnet_command(action) — dispatches all DevnetCommands variants to crate::devnet module
- handle_release_command(action) — ReleaseCommands::Sign: loads maintainer key, optionally fetches CHECKSUMS.txt from GitHub, calls sign_release_hash, prints JSON signature
- handle_upgrade_command(version, yes) — checks for newer GitHub release, prompts confirmation, downloads tarball, verifies SHA-256, backs up, installs; leaves restart to user
- handle_update_command(action, data_dir, network) — dispatches all UpdateCommands variants: Check, Status, Vote, Votes, Apply, Rollback, Verify

### Devnet (bins/node/src/devnet/)
- DevnetConfig — node_count, base_p2p_port (50300), base_rpc_port (28500), base_metrics_port (29000); p2p_port(idx), rpc_port(idx), metrics_port(idx) computed port accessors
- init(node_count) — validate count (1-100), create directory structure (keys/, data/, logs/, pids/), generate N keypairs, build ChainSpec with fixed genesis timestamp, save chainspec.json and devnet.toml
- start() — DESTROYS all data directories before starting (unconditional wipe + recreate); spawns node 0, waits 2s, verifies running (prints last 10 log lines on failure), waits for RPC ready (30s), spawns nodes 1..N-1 with --bootstrap (500ms stagger)
- stop() — scan_and_kill_all_pids on pids/ directory
- status() — queries getChainInfo and getProducers from first running node; prints producers table and formatted status table for all nodes
- clean(keep_keys) — stop nodes; if keep_keys=false removes entire devnet root; if keep_keys=true removes data/logs/pids and config files but preserves keys/
- add_producer(count, bonds, fund_amount) — generate keypair, write wallet, fund from rotating genesis wallet via CLI, poll balance, register via CLI, create data directory, spawn node process, save PID

### Operations (bins/node/src/operations/)
- truncate_chain(network, data_dir, blocks_to_remove, skip_confirm) — undo-based rollback (2000-block window limit); iterates from current tip down applying undo records; restores ProducerSet from undo snapshot; prunes blocks and undo data above new_tip
- recover_chain_state(network, data_dir, skip_confirm) — full chain replay: scan heights 1..MAX to find tip, rebuild canonical index, replay all blocks applying UTXOs and tx types, process unbonding, merge hardcoded genesis producers for Mainnet (consensus-critical)
- reindex_canonical_chain(data_dir) — open BlockStore, rebuild_canonical_index(), prints resulting tip hash and height
- backfill_from_archive(network, data_dir, archive_dir, skip_confirm) — imports only missing blocks; does NOT rebuild state; detects genesis_hash from existing blocks in store (scans heights 1..10000; handles relaunched networks)
- restore_from_archive(network, data_dir, archive_dir, skip_confirm) — imports from archive then calls recover_chain_state(skip_confirm=true) automatically
- restore_from_rpc(network, data_dir, rpc_url, backfill, skip_genesis_check, skip_confirm) — fetches block 1 to validate genesis hash (unless skip_genesis_check); downloads all blocks via getBlockRaw (base64+BLAKE3 verified); in backfill=true mode skips existing blocks; full restore suggests running `recover` afterward
- show_status(data_dir) — STUB: prints "Status: Not implemented yet"
- import_blocks(data_dir, path) — STUB: prints "Import: Not implemented yet"
- export_blocks(data_dir, path, from, to) — STUB: prints "Export: Not implemented yet"

### Update Service (bins/node/src/updater/ — NOT crates/updater)
- UpdateService — background update checker; fields: config, network, pending (Arc<RwLock<Option<PendingUpdate>>>), vote_tx, vote_rx, data_dir, last_notification; new() (auto-clears stale enforcement if already on required version), vote_sender(), pending_state(), run(producer_count_fn, is_producer_fn, maintainer_keys_fn) — main service loop: periodic update check, incoming vote processing, veto transitions, periodic reminders
- PendingUpdate — fields: release, vote_tracker, first_notified_at (u64 Unix timestamp), approved (bool), enforcement (Option<VersionEnforcement>); load(), save(), remove(), days_remaining(), hours_remaining()
- spawn_update_service(config, data_dir, network, producer_count_fn, is_producer_fn, maintainer_keys_fn) -> (Sender<VoteMessage>, Arc<RwLock<Option<PendingUpdate>>>) — spawns background tokio task; returns vote channel sender and shared pending state handle
- get_pending_version(data_dir) / get_version_enforcement(data_dir) / is_production_allowed(data_dir) / get_pending_update(data_dir) — free functions reading disk state
- show_status_from_disk(data_dir) — print formatted update status: veto period banner, grace period banner, enforcement active banner, or "No pending updates"
- display_update_notification() / display_grace_period_notification() / display_enforcement_notification() — console notification banners (stderr + tracing::info/warn)
- NOTIFICATION_INTERVAL_SECS = 21,600 (6 hours) — reminder interval

### Metrics (29)
- BLOCKS_PROCESSED / BLOCKS_BY_STATUS (label: status)
- CHAIN_HEIGHT / CURRENT_SLOT
- BLOCK_PROCESSING_TIME (buckets: 0.001–5.0s)
- TRANSACTIONS_VALIDATED / TRANSACTIONS_BY_TYPE / TRANSACTIONS_BY_RESULT
- MEMPOOL_SIZE / MEMPOOL_BYTES
- PEERS_CONNECTED / PEERS_SEEN_TOTAL / PEERS_BY_STATUS
- MESSAGES_RECEIVED / MESSAGES_SENT
- BYTES_RECEIVED / BYTES_SENT
- SYNC_PROGRESS / IS_SYNCING / BLOCKS_BEHIND
- VDF_COMPUTE_SECONDS / VDF_VERIFY_SECONDS (vestigial; DOLI does not use VDF in production; metrics are never updated by active production paths)
- ACTIVE_PRODUCERS / BLOCKS_PRODUCED / SLOT_LATENCY (buckets: 1–60s)
- UTXO_SET_SIZE / STORAGE_BYTES
- UPTIME_SECONDS / BUILD_INFO (labels: version, commit)
- Metrics HTTP endpoint: GET /metrics (axum); spawn_metrics_server(addr) starts server in background task
- REGISTRY — global Prometheus registry; register_metrics() registers all 29 metrics

---

## 10. CLI BINARY (`bins/cli`)

### Commands (42 top-level)

- Init — initialize producer wallet: creates wallet + BLS key (skipped with `--non-producer`)
- New — create new wallet file with BIP-39 24-word seed phrase
- Restore — restore wallet from 24-word BIP-39 mnemonic phrase
- Address — generate new address in existing wallet; returns bech32m
- Addresses — list primary address (secondary addresses hidden)
- Balance — query confirmed, bonded, activating, immature, unconfirmed, and total balance
- Send — build and broadcast transfer transaction; supports optional covenant condition; flat 1-satoshi fee
- Spend — spend covenant-conditioned UTXO with parsed witness string
- History — display transaction history
- Export — export wallet to file
- Import — import wallet from file
- Info — display wallet name, address count, primary address, pubkey, BLS key
- AddBls — add BLS attestation key to existing wallet
- Sign — sign a message with Ed25519; supports specific address override
- Verify — verify an Ed25519 message signature
- Producer (9 subcommands) — Register, Status, Bonds, List, AddBond, RequestWithdrawal, SimulateWithdrawal, Exit, Slash
- Rewards (5 subcommands) — List, Claim, ClaimAll, History, Info (List/Claim/ClaimAll/History are informational; rewards are auto-distributed; only Info calls RPC)
- Chain — display chain info via RPC (network, height, slot, hash, genesis hash, reward pool)
- ChainVerify — verify chain integrity; display BLAKE3 chain commitment and missing ranges
- Update (6 subcommands) — Check, Status, Vote, Votes, Apply, Rollback
- Maintainer (1 subcommand) — List
- Upgrade — download, checksum-verify, and install latest binary release; restart service
- Release (1 subcommand) — Sign
- Protocol (2 subcommands) — Sign, Activate (requires 3/5 maintainer signatures)
- Nft — all NFT operations via mutually exclusive flags: `--list`, `--info`, `--mint`, `--transfer`, `--sell`, `--sell-sign`, `--buy`, `--from`, `--export`, `--batch-mint`, `--fractionalize`, `--redeem` (11 operations; single command variant with flags, NOT subcommands)
- IssueToken — issue fungible token (MintAsset transaction)
- TokenInfo — query fungible token metadata from a UTXO
- BridgeSwap — initiate complete cross-chain atomic swap (CSPRNG preimage, BLAKE3/SHA256/keccak lock, timelocks)
- BridgeStatus — check bridge swap status; optional auto-refund on expiry
- BridgeBuy — buyer-side atomic swap completion; resolves preimage from argument, disk, or counter-chain scan
- BridgeWatch — run bridge watcher daemon
- BridgeList — list active BridgeHTLC swaps by scanning recent blocks
- BridgeLock — lock DOLI in BridgeHTLC (manual/advanced); supports optional And(Multisig, Htlc) multi-party control
- BridgeClaim — claim BridgeHTLC by providing preimage
- BridgeRefund — refund expired BridgeHTLC
- Pool (6 subcommands) — Create, Swap, Add, Remove, List, Info
- Loan (7 subcommands) — Deposit, Withdraw, Create, Repay, Liquidate, List, Info
- Channel (5 subcommands) — Open, Pay, Close, List, Info
- Service (7 subcommands) — Install, Uninstall, Start, Stop, Restart, Status, Logs
- Guardian (5 subcommands) — Status, Halt, Resume, Checkpoint, Monitor
- Snap — fast-sync via state snapshot: verify 2/3-quorum state root, wipe chain data, download snapshot, verify integrity, apply atomically
- Wipe — stop service, delete chain data (preserves: keys/, .env, wallet.json, wallet.seed.txt, node_key, config.toml), dry-run supported

### Wallet (CLI)

- Wallet — name, version (1=legacy, 2=BIP-39), addresses; seed phrase NOT stored in JSON
- WalletAddress — address (hex), public_key (hex), private_key (hex, private), label (optional), bls_private_key (optional), bls_public_key (optional)
- new() — create wallet; returns (wallet, 24-word mnemonic)
- from_seed_phrase() — restore wallet; derives identical Ed25519 key; generates new (random) BLS key
- load() / save() / export() / import() — file I/O; save creates parent directories
- primary_address() — 20-byte truncated hex address
- primary_pubkey_hash() — 32-byte domain-separated BLAKE3 hash of pubkey for RPC queries
- primary_public_key() / primary_bech32_address() / primary_keypair() — key accessors
- has_bls_key() / primary_bls_public_key() / add_bls_key() — BLS key management
- generate_address() — generate new random address (NOT derived from seed; warns user)
- all_pubkey_hashes() / keypair_for_pubkey_hash() — multi-address UTXO support
- sign_message() / verify_message() — Ed25519 message signing

### RPC Client (CLI)

- RpcClient — JSON-RPC client with archiver fallback; tries local node first, falls back to seed archivers on "not found"
- Balance / Utxo / ChainInfo / ChainIntegrity / NetworkParams / TransactionInfo / HistoryEntry — typed response structs
- ProducerInfo / PendingWithdrawalInfo / PendingUpdateInfo / BondDetailsInfo / BondsSummaryInfo / BondEntryInfo — producer response structs
- BlockInfo / EpochInfoResponse — chain and epoch response structs
- units_to_coins() / coins_to_units() / format_balance() — integer arithmetic; no f64 precision loss
- get_balance() / get_utxos() / get_utxos_json() — UTXO queries
- get_chain_info() / verify_chain_integrity() / get_network_params() — chain queries
- get_transaction() / get_transaction_json() / get_history() — transaction queries with archiver fallback
- get_producer() / get_producers() / get_bond_details() — producer queries
- get_epoch_info() / get_update_status() / get_node_info() — state queries
- register_producer() / withdraw_producer() / send_transaction() — submission methods
- ping() / get_block() / submit_vote() / get_maintainer_set() — utility methods

### Parsers

- parse_condition() — string to Condition AST; supported conditions: multisig, hashlock, htlc, timelock, timelock_expiry, vesting, and, or
- parse_witness() — string to encoded witness bytes; supports: preimage, sign, branch, none/empty, compound with `+`
- resolve_to_hash() — bech32m or hex address to pubkey_hash
- condition_to_output_type() — map Condition to OutputType

### Internal Modules

- common.rs — ADDRESS_PREFIX / NETWORK OnceLock statics; address_prefix(), prefix_for_network(), default_rpc_for_network(), archiver_endpoints_for_network(), expand_tilde()
- paths.rs — resolve_base_dir() / resolve_wallet_path() — priority: flag > DOLI_DATA_DIR env > platform default > legacy ~/.doli
- cmd_chain.rs — WIPE_PRESERVE constant; collect_deletable(); wipe_data_dir() with multi-node layout support; 10 unit tests for wipe logic

---

## 11. WALLET LIBRARY (`crates/wallet`)

### Public API

- Wallet — BIP-39 key management; see CLI Wallet section for method list
- WalletAddress — address entry with Ed25519 and optional BLS keys
- RpcClient — simplified async JSON-RPC client; 14 public methods: get_balance(), get_utxos(), send_transaction(), get_chain_info(), get_history(), get_producers(), get_network_params(), get_epoch_info(), get_rewards_list(), get_bond_details(), simulate_withdrawal(), test_connection(), url(), new()
- TxBuilder — transaction construction and signing; methods: new(), add_input(), add_output(), set_extra_data(), build_for_signing(), sign_and_build(), input_count(), output_count(), tx_type()
- TxType — 15 wallet-level transaction types: Transfer, Registration, ProducerExit, Coinbase, NftMint, NftTransfer, RewardClaim, AddBond, RequestWithdrawal, ClaimWithdrawal (tombstone), SlashingEvidence, TokenIssuance, BridgeLock, DelegateBond, RevokeDelegation
- TxInput / TxOutput — input and output structs for builder
- build_transfer() / build_add_bond() / build_request_withdrawal() / build_reward_claim() — convenience static builders
- calculate_registration_cost() — (bond_cost, registration_fee, total); validates 1 ≤ bond_count ≤ MAX_BONDS_PER_PRODUCER
- vesting_penalty_pct() — 75/50/25/0 by vesting quarter (boundaries at VESTING_QUARTER_SLOTS multiples)
- calculate_withdrawal_net() — net return after vesting penalty
- default_endpoints() / network_prefix() — RPC endpoint and bech32m prefix by network
- verify_message() — Ed25519 message signature verification
- units_to_coins() / coins_to_units() / format_balance() — integer arithmetic, no f64

### Response Types (types.rs)

- Balance / Utxo / ChainInfo / HistoryEntry — basic RPC response types
- ProducerInfo / PendingWithdrawalInfo / PendingUpdateInfo — producer response types
- BondDetailsInfo / BondsSummaryInfo / BondEntryInfo — bond detail response types
- RewardEpoch / EpochInfo / NetworkParams — epoch and network response types
- WithdrawalSimulation / BondWithdrawalDetail — withdrawal simulation response types

### Constants (types.rs) — verified against doli-core by serialization_compat.rs

- UNITS_PER_DOLI = 100_000_000 — 1 DOLI = 100 million base units
- BOND_UNIT = 1_000_000_000 — 1 bond = 10 DOLI in base units
- MAX_BONDS_PER_PRODUCER = 3_000 — maximum bonds per producer
- BLOCKS_PER_REWARD_EPOCH = 360 — blocks per reward epoch
- COINBASE_MATURITY = 6 — blocks before coinbase/reward is spendable (NOTE: spec §3 listed 100; wallet code and serialization_compat test assert 6 against doli-core — code is SOT)
- UNBONDING_PERIOD = 60_480 — unbonding delay in blocks (~7 days)
- BASE_REGISTRATION_FEE = 100_000 — base registration fee (0.001 DOLI)
- MAX_REGISTRATION_FEE = 1_000_000 — maximum registration fee cap (0.01 DOLI)
- VESTING_QUARTER_SLOTS = 3_153_600 — one vesting quarter in mainnet slots (~1 year) (NOTE: serde default in BondDetailsInfo is 2_160 for RPC backward-compat; the module constant and serialization_compat test assert 3_153_600 against doli-core — code is SOT)

### Integration Test Files

- crates/wallet/tests/tx_builder.rs — 8 tests: end-to-end build, constant parity, protocol mapping, no-VDF dependency
- crates/wallet/tests/wallet_compat.rs — 8 tests: JSON structure, CLI compatibility, save/load roundtrip
- crates/wallet/tests/serialization_compat.rs — 10 tests: byte-for-byte wire parity with doli-core, constant parity, TxType mapping, fee multiplier parity

---

## 12. PAYMENT CHANNELS (`crates/channels`)

### Channel

- ChannelRecord — persistent channel state (20 fields): channel_id, state, local/remote pubkey_hash, funding_outpoint, capacity, balance, commitment_number, channel_seed, revocation_store, dispute_window, htlcs, funding_confirmations, created_at, updated_at, close_tx_hash, penalty_tx_hash
- ChannelId — 32-byte identifier derived from funding outpoint hash; methods: from_funding_outpoint(), to_hex(), short()
- ChannelBalance — local/remote distribution; methods: new(), total(), pay_local_to_remote(), pay_remote_to_local()
- ChannelState — 10-state lifecycle: Opening, FundingSigned, FundingBroadcast, Active, CooperativeClosing, ForceClosing, CounterpartyClosing, AwaitingClaim, Closed, PenaltyInFlight; methods: is_terminal(), is_active(), is_closing()
- ChannelConfig — dispute_window, min_channel_capacity, reserve_percent (default 1%), max_htlcs (default 30), htlc_minimum, fee_rate, max_htlc_expiry_delta, funding_confirmations (mainnet=3, testnet=1), rpc_url, poll_interval_secs, store_path; constructors: mainnet(), testnet()
- ChannelError — 19 variants including InvalidTransition, NotFound, InsufficientBalance, InvalidRevocation, HtlcExpired, CapacityMismatch, ReserveViolation, DisputeWindowActive, Rpc, Http, Json, Io, Protocol, Config
- CommitmentNumber — u64 type alias; monotonically increasing
- PaymentDirection — Outgoing, Incoming
- HtlcState — Pending, Fulfilled, Expired, Resolved
- FundingOutpoint — tx_hash, output_index; method: tx_hash_as_crypto()

### Commitment

- CommitmentPair — 6 fields: number, balance, revocation_preimage, revocation_hash, remote_revocation_hash, htlcs; methods: new(), set_remote_revocation_hash(), build_local_commitment(), verify_revocation()
- RevocationStore — ordered preimage store; methods: new(), add(), get(), find_by_hash(), len(), is_empty()
- generate_revocation_preimage() — deterministic: H(REVOCATION_DOMAIN || channel_seed || commitment_number)
- revocation_hash() — hash preimage using HASHLOCK_DOMAIN for L1 compatibility
- derive_channel_seed() — H("DOLI_CHANNEL_SEED" || pubkey || channel_id)
- build_delayed_claim_witness() — witness for to_local delayed claim (right/true branch)
- build_penalty_witness() — witness for penalty path (left/false branch with revocation preimage)
- build_htlc_claim_witness() — witness for HTLC claim with payment preimage
- build_htlc_timeout_witness() — witness for HTLC timeout refund

### Conditions

- funding_condition() — 2-of-2 multisig (keys sorted lexicographically)
- funding_output() — Output of type Multisig holding channel capacity
- to_local_condition() — LN-Penalty: Or(And(Sig(counterparty), Hashlock(revocation)), And(Sig(owner), Timelock(dispute_height)))
- to_local_output() — to_local Output of type Multisig
- to_remote_output() — plain Normal Output for counterparty (immediately spendable)
- htlc_offered_condition() / htlc_offered_output() — offered HTLC: Or(And(Sig(remote), Hashlock), And(Sig(local), TimelockExpiry))
- htlc_received_condition() / htlc_received_output() — received HTLC: Or(And(Sig(local), Hashlock), And(Sig(remote), TimelockExpiry))
- verify_encoding_size() — verify condition encodes within MAX_EXTRA_DATA_SIZE (4096 bytes)

### Funding

- build_funding_tx() — build funding transaction into 2-of-2 multisig; supports dual-funded via AnyoneCanPay
- build_funding_tx_with_change() — build funding tx with automatic change computation
- sign_funding_input() — sign a specific funding transaction input

### Close

- build_cooperative_close() — mutual close spending funding output to two Normal outputs
- sign_cooperative_close() — build 2-of-2 multisig witness (signatures sorted by pubkey hash)
- build_force_close() — unilateral close via CommitmentPair::build_local_commitment
- build_penalty_tx() — penalty sweep of revoked to_local output
- build_delayed_claim() — delayed claim of to_local output after dispute window

### HTLCs

- HtlcManager — lifecycle manager: fields next_id, htlcs; methods: new(), add_outgoing(), add_incoming(), fulfill(), expire(), resolve(), pending(), total_outgoing_pending(), total_incoming_pending(), all(), gc_resolved()
- InFlightHtlc — htlc_id, payment_hash, amount, expiry_height, direction, state, preimage

### Protocol Messages (12)

- OpenChannel / AcceptChannel — channel negotiation
- FundingCreated / FundingSigned — funding transaction handshake
- UpdateCommitment / RevokeAndAck — off-chain payment update
- AddHtlc / FulfillHtlc / FailHtlc — HTLC lifecycle
- CloseChannel / CloseAccepted / Error (ErrorMessage) — close and error

### Routing

- ChannelGraph — adjacency list for Dijkstra pathfinding; methods: new(), add_channel(), find_route()
- Route — hops, total_fee, total_amount
- RouteHop — channel_id, node_id, amount_to_forward, fee, expiry_delta
- ChannelEdge — source, target, capacity, fee_rate_ppm, base_fee; method: fee_for_amount()
- NodeId — [u8; 32] type alias

### Invoice

- Invoice — payment_hash, amount (0=any), description, payee_pubkey_hash, expiry_timestamp, created_at
- encode() / decode() — "doli:pay:<base64>" format
- is_expired() — check against current Unix time

### Payment

- Payment — payment_hash, preimage, total_amount, destination_amount, total_fees, status, hop_count, created_at
- PaymentStatus — Pending, InFlight, Succeeded, Failed(String)
- from_route() / succeed() / fail() / is_terminal() — lifecycle methods

### State Machine

- validate_transition() — validates ChannelState→ChannelState; returns ChannelError::InvalidTransition if not allowed

### Store

- ChannelStore — JSON-based file persistence with atomic write (temp file + rename); methods: open(), save(), active_channels(), all_channels(), find(), find_mut(), find_by_funding(), add(), active_count()

### Monitor

- ChainMonitor — polls DOLI chain; methods: new(), check_channel(), update_height(), last_height()
- MonitorEvent — 5 variants: FundingConfirmed, FundingSpent, RevokedCommitment (5 fields including revocation_preimage, to_local_amount, to_local_output_index), DisputeWindowExpired, HtlcExpired

### Manager

- ChannelManager — coordination loop; fields: config, rpc, store, monitor; methods: new(), run(), store(), store_mut()

### RPC (channels)

- RpcClient (channels::rpc) — distinct from wallet RpcClient; methods: new(), ping(), get_height(), get_utxos(), submit_transaction(), get_transaction_status(), broadcast_transaction(), get_block()
- RpcUtxo / TxStatus / BlockInfo — channel-specific RPC response types

### Watchtower

- PenaltyBlob — encrypted penalty transaction; fields: tx_hint (first 16 bytes), encrypted_data
- WatchtowerSession — channel_id, endpoint, session_token, blobs_uploaded; method: new()

---

## 13. CROSS-CHAIN BRIDGE (`crates/bridge`)

### Swap

- SwapRecord — full swap state (22 fields): id, state, role, target_chain, target_address, doli_tx_hash, doli_output_index, doli_amount, doli_hash, doli_lock_height, doli_expiry_height, doli_creator, counter_tx_hash, counter_amount, counter_hash, preimage, preimage_source, doli_claim_tx, counter_claim_tx, doli_refund_tx, created_at, updated_at
- SwapState — 7 states: DoliLocked, BothLocked, PreimageRevealed, Complete, Expired, Refunded, Failed(String) — terminal states: Complete, Refunded, Failed
- SwapRole — Initiator (we locked DOLI first), Responder (counterparty locked DOLI)
- SwapRecord methods: new(), transition(), is_terminal()
- BridgeError — 11 variants: DoliRpc, BitcoinRpc, EthereumRpc, Http, Json, Io, SwapNotFound, InvalidPreimage, InvalidConfig, SwapExpired, ChainMismatch { expected, got }

### DOLI Client

- DoliClient — DOLI node RPC for bridge operations; methods: new(), get_chain_info(), get_bridge_utxos(), get_transaction(), send_transaction(), scan_for_htlcs(), scan_for_preimage_reveals(), height(), ping()
- ChainInfo — best_height, best_hash, best_slot
- DoliUtxo — tx_hash, output_index, amount, output_type, lock_until, spendable, pubkey_hash, bridge metadata
- BridgeMetadata — target_chain, target_chain_id, target_address (optional fields)
- DetectedHtlc — detected BridgeHTLC on DOLI chain (9 fields including hash, lock_height, expiry_height, creator_pubkey_hash, target_chain, target_address)
- RevealedPreimage — preimage extracted from DOLI HTLC claim (4 fields: htlc_tx_hash, htlc_output_index, claim_tx_hash, preimage)

### Bitcoin Client

- BitcoinClient — Bitcoin Core JSON-RPC; methods: new(), get_block_count(), get_block_hash(), get_block(), get_raw_transaction(), sha256_hash() (Bitcoin uses SHA256; DOLI uses BLAKE3), scan_for_htlcs(), scan_for_preimage_reveals(), ping()
- DetectedBtcHtlc — txid, vout, amount_sat, confirmations, hash, locktime
- BtcPreimageReveal — htlc_txid, htlc_vout, claim_txid, preimage

### Ethereum Client

- EthereumClient — Ethereum JSON-RPC; methods: new(), ping(), get_block_number(), scan_for_htlc() (eth_getLogs for LogHTLCNew), scan_for_preimage() (eth_getTransactionReceipt for LogHTLCWithdraw), keccak256()
- DetectedEthHtlc — tx_hash, block_number, confirmations, amount (string), token_address
- EthPreimageReveal — tx_hash, preimage

### Watcher

- WatcherConfig — doli_rpc, our_pubkey_hash, btc_rpc (optional), eth_rpc (optional), data_dir, poll_interval_secs
- Watcher — bridge watcher daemon; methods: new(), run() (async main loop with Ctrl-C), swap_dir(), active_swaps(), get_swap()

---

## 14. GUI (`bins/gui`)

### Tauri Application

- AppState — wallet (RwLock), wallet_path (RwLock), rpc_client, config (RwLock), node_manager (RwLock); methods: new(), has_wallet(), network_prefix()
- AppConfig — network, custom_rpc_url, default_wallet_path, last_wallet_path, poll_interval, rpc_endpoints; methods: load_or_default(), save(), effective_rpc_url()
- NodeManager — manages embedded doli-node child process; fields: process, data_dir, network, rpc_port, log_path; methods: start(), stop(), restart(), is_running(), log_path(); impl Drop (graceful shutdown on drop)
- NodeManager constants: MAINNET_RPC_PORT=8500, TESTNET_RPC_PORT=18500, DEVNET_RPC_PORT=28500, NODE_BINARY="doli-node", SHUTDOWN_TIMEOUT=10s
- Binary location: checks adjacent-to-GUI executable first, then PATH

### Response Types (commands/mod.rs)

- CreateWalletResponse / WalletInfo / AddressInfo / BalanceResponse / SendResponse / TxResponse / HistoryEntryResponse
- ProducerStatusResponse / SimulateResponse / RewardEpochResponse
- NftInfoResponse / TokenInfoResponse / BridgeLockParams
- UpdateInfo / UpdateStatusResponse / ChainInfoResponse / ConnectionStatus / ConnectionTestResult / NodeStatus

### Tauri Commands (44 total)

Wallet (9):
- create_wallet — creates new wallet with mnemonic, saves to disk, loads into AppState
- restore_wallet — restores from BIP-39 mnemonic
- load_wallet — loads existing wallet file from path
- generate_address — derives next address
- list_addresses — returns all derived addresses
- export_wallet — exports wallet to file
- import_wallet — imports wallet file
- get_wallet_info — returns WalletInfo for loaded wallet
- add_bls_key — derives BLS keypair and attaches to wallet

Transaction (3):
- get_balance — queries RPC, returns BalanceResponse
- send_doli — builds, signs, broadcasts transfer; private key never crosses IPC boundary
- get_history — returns paginated transaction history

Producer (6):
- get_producer_status — queries registration status and bond details
- register_producer — **STUB**: returns Err directing user to CLI; registration not supported via GUI
- add_bonds — builds and broadcasts AddBond transaction
- request_withdrawal — builds and broadcasts RequestWithdrawal transaction
- simulate_withdrawal — calls RPC simulation; returns SimulateResponse
- exit_producer — builds and broadcasts Exit transaction

Rewards (3):
- list_rewards — queries unclaimed epoch rewards
- claim_reward — broadcasts ClaimReward for specific epoch
- claim_all_rewards — iterates and claims all unclaimed epoch rewards

Network (5):
- get_chain_info — queries chain state; returns ChainInfoResponse
- set_rpc_endpoint — updates custom RPC URL in AppConfig and persists
- set_network — switches active network (mainnet/testnet/devnet)
- test_connection — RPC ping with latency; returns ConnectionTestResult
- get_connection_status — returns ConnectionStatus

Node (5):
- start_node — spawns embedded doli-node
- stop_node — terminates embedded doli-node
- get_node_status — returns NodeStatus (running, network, rpc_url, log_path)
- restart_node — stop then start
- get_logs — reads last N lines from node log file

NFT/Token (5):
- mint_nft — builds and broadcasts MintNFT transaction
- transfer_nft — **STUB**: returns Err("not implemented in GUI — use CLI")
- nft_info — **STUB**: returns Err("not implemented in GUI — use CLI")
- issue_token — builds and broadcasts MintAsset transaction
- token_info — **STUB**: returns Err("not implemented in GUI — use CLI")

Bridge (3):
- bridge_lock — builds and broadcasts bridge lock transaction
- bridge_claim — **STUB**: returns Err("not implemented in GUI — use CLI")
- bridge_refund — **STUB**: returns Err("not implemented in GUI — use CLI")

Governance (5):
- check_updates — **STUB**: returns Err("not implemented in GUI — use CLI")
- get_update_status — queries RPC for pending updates and vote tallies
- vote_update — **STUB**: returns Err("not implemented in GUI — use CLI")
- sign_message — signs message with wallet private key (BLAKE3 domain-separated)
- verify_signature — verifies Ed25519 signature against public key and message

---

## 15. TESTING

### Integration Tests — bins/node/tests/

- bins/node/tests/fork_recovery.rs — 11 fork recovery tests: bond divergence handling, rollback cap reset, scheduler divergence, post-snap gossip validation mode, multiple independent node recovery
- bins/node/tests/checkpoint_rotation.rs — 3 tests: ASCII sort bug regression (h5 vs h30), gossip network rotation, 2→3 digit boundary bug
- bins/node/tests/epoch_reward_explicit_inputs.rs — 9 tests (7 active + 2 #[ignore]): pre/post-activation EpochReward construction, UTXO validation, conservation checks; 2 ignored tests for pre-activation paths that no longer exist
- bins/node/tests/epoch_state_regression.rs — 7 tests: accumulator tracking, epoch boundary derivation, undo data roundtrip, multi-epoch rotation, cross-node determinism, rollback across epoch boundary, persistence roundtrip
- bins/node/tests/m_rc9_silent_vec_regression.rs — 3 tests: complete store regression anchor, adversarial gap silent undercount, Santiago cascade mainnet-scale replay
- bins/node/tests/m_rc10_apply_after_reject_regression.rs — 4 tests (3 active + 1 #[ignore]): light mode apply after reject, duplicate reject without damage; 1 ignored (validation order changed)
- bins/node/tests/m_rc11_fork_guard_backfill_regression.rs — 3 tests: tip reorg completeness invariant, missing ancestor silent corruption regression, stale new block no chain advance
- bins/node/src/node/tests/fork_recovery_tests.rs — 3 unit tests: Node::new_for_test initialization, apply blocks, rollback

NOTE: The following files listed in a previous spec version do NOT exist on disk and must be removed:
- ~~bins/node/tests/inc_i_026_excluded_divergence.rs~~ — does not exist
- ~~bins/node/tests/inc_i_034_scheduler_divergence.rs~~ — does not exist

### Integration Tests — testing/integration/

- testing/integration/two_node_sync.rs — 9 tests: basic sync, incremental catch-up, large gap, duplicate block handling, multi-producer sync, UTXO sync, concurrent additions, chain tip tracking
- testing/integration/partition_heal.rs — 7 tests: divergent chain partition, heal to longer chain, three-way convergence, UTXO reconciliation, large length difference, mock peer simulation, gradual healing
- testing/integration/reorg_test.rs — 9 tests: single-block reorg, 10-block deep reorg, very deep reorg (15/30 blocks), UTXO consistency, multiple sequential reorgs, different-producer reorg, chain integrity, equal-length reorg, empty revert
- testing/integration/attack_reorg_test.rs — 15 tests: double-spend via reorg, confirmation depth, selfish mining (withholding + lead), long-range attack, checkpoint prevention, nothing-at-stake, timestamp attacks (future + past), finality after confirmations, shallow reorg preserves finality, sybil resistance, eclipse attack, rapid + concurrent reorg attempts
- testing/integration/mempool_stress.rs — 11 tests: 10K sequential, 10K concurrent, batched concurrent, full rejection, clear, varying sizes, throughput measurement, memory footprint, churn, concurrent read-write, burst traffic
- testing/integration/staggered_validator_rewards.rs — 4 tests: mid-epoch join no rewards, zero blocks no rewards, ten-producer fair distribution, proportional rewards
- testing/integration/two_producer_pop.rs — 7 tests: alternating production, missed slots, presence rate calculation, minimum presence threshold, producer activity transitions, genesis block validity, full PoP chain
- testing/integration/bond_stacking.rs — 30 tests: bond constants, vesting schedule, withdrawal penalties (FIFO, mixed-age, all-vested, all-Q1), bond entry serialization, max limit enforcement, lifecycle (register→add→withdraw→claim)
- testing/integration/epoch_rewards.rs — 28 tests: fair share calculation (even, remainder, single, many), epoch reward transaction structure, UTXO maturity, pool accumulation, distribution totals, producer sorting determinism, first-producer remainder, reward mode, epoch boundary detection, large epoch numbers, multi-epoch catch-up, proportional rounding
- testing/integration/equivocation_slashing.rs — 11 tests: detection (same block, different slots, different producers), proof-to-slash transaction, status update, re-registration rejection, multiple equivocations, E2E scenario, detector memory bound
- testing/integration/mempool_poison.rs — 10 tests: NFT purge by error pattern, normal TX preservation, 10 purge cycles, pattern specificity, mixed mempool selective purge, regossip repurge, idempotent purge, full lifecycle
- testing/integration/malicious_peer.rs — 15 tests: wrong prev_hash, bad merkle root, overflow amount, duplicate inputs, unknown producer, future timestamp, empty block, excessive coinbase, corrupted data, too many outputs, slot/timestamp mismatch, rapid invalid submissions
- testing/integration/presence_manipulation_test.rs — 8 tests: presence root block hash sensitivity, legacy vs V2 commitment, total_weight manipulation, weighted reward calculation, V2 determinism, all-component hash changes, DOLI_NETWORK_BUG attack scenario

Infrastructure (not test files):
- testing/integration/common/mod.rs — TestNode, TestNodeConfig, MockPeer, generate_test_chain, create_coinbase, create_test_block, create_transfer, init_test_logging, wait_for
- bins/node/tests/test_network.rs — TestNetwork struct (shared infrastructure for checkpoint_rotation tests; no #[test] functions)

### Unit Tests

- Consensus: constants, bonds, exit, vesting, epoch, rewards
- Validation: block, transaction, UTXO, registration, pool, lending, fractionalization, ZK, guards, NFT uniqueness, P0001 exploit regression
- Network: sync manager (130 tests), adversarial sync (26 tests), reorg handler (17 tests), gossip (8 tests), service/routing (16 tests)
- Storage: block store, state DB, UTXO, producer set, snapshot, archiver, MMR
- Crypto: hash, keys, signatures, BLS, adaptor, merkle, address
- Mempool: add/remove, fee enforcement, CPFP, revalidation, poison purge
- Channels: state machine, commitment, conditions, funding, HTLCs, routing, invoice
- Conditions: encoding/decoding roundtrip, evaluation, witness, guard conditions
- Discovery: GSet CRDT, bloom filter, announcement, gossip
- Attestation: bitfield encode/decode, minute tracker
- Presence: scoring, selection, VDF helpers
- Scheduler: deterministic round-robin, weighted tickets
- Wallet library: 70+ inline tests + 26 integration tests across 3 files (tx_builder.rs, wallet_compat.rs, serialization_compat.rs)
- CLI binary: path resolution, wallet roundtrip, wipe logic (10 tests), RPC parsing, producer subcommand parsing

### Benchmarks

- testing/benchmarks — 4 sub-commands: Compute (T_BLOCK and T_REGISTER_BASE iterations), Verify (proof verification), Full (complete suite), Report (system info + all timings + pass/fail thresholds); also benchmarks raw class group squaring at multiple iteration counts

---

## STATISTICS

| Area | Files | Public Items |
|------|-------|-------------|
| Crypto | 7 | ~80 |
| VDF | 4 | ~25 |
| Core | 40+ | ~500+ |
| Storage | 15+ | ~200+ |
| Network | 49 | ~400+ |
| Mempool | 3 | ~40 |
| RPC | 20+ | ~150+ |
| Updater | 13 | ~100+ |
| Node Binary | 57 | ~300+ |
| CLI Binary | 39 | ~200+ |
| Wallet | 7 | ~60 |
| Channels | 20 | ~150+ |
| Bridge | 6 | ~50+ |
| GUI | 12 | ~50+ |
| Tests | 155+ | 2,203+ test functions |
| **Total** | **~430+ files** | **~2,300+ public items** |

### Test Count Breakdown

| Test Location | Count |
|---------------|-------|
| bins/node/tests/ (7 active files) | 40 active + 3 #[ignore] |
| bins/node/src/node/tests/ | 3 |
| testing/integration/ (13 files) | 164 |
| crates/network/ (5 files) | 197 (196 active + 1 #[ignore]) |
| crates/core/ | ~350+ |
| crates/storage/ | ~150+ |
| crates/wallet/ (inline + integration) | ~130+ |
| All other crates (crypto, mempool, rpc, channels, etc.) | ~1,100+ |
| **Estimated total** | **2,203+** |

NOTE: The previous Statistics entry of "2,039 test functions" did not count the 164 tests in testing/integration/ (entirely absent from the prior spec) nor the 43 tests in bins/node/tests/ that were missing or misattributed. Corrected minimum: 2,203+.
