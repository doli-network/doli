# Encrypted Content UTXO — Privacy-First On-Chain Content

**Status**: Planned  
**Type**: Hard fork (new OutputType, new TxType)  
**Priority**: Next major feature after network stability confirmed  

## Problem

Current NFT/content model stores plaintext data on-chain. Every node can read it. Every explorer can display it. This creates two problems:

1. **No privacy**: creators cannot store private content (drafts, personal documents, private art) on-chain without it being visible to everyone
2. **Liability**: operators store and serve readable content — exposing them to legal risk for content they didn't create and can't control

## Solution: Encrypted by Default

All content stored on-chain is encrypted with a unique symmetric key. The key is wrapped with the owner's public key inside the UTXO. Without the key, the content is indistinguishable from random noise.

Three properties emerge from one mechanism:
- **Privacy**: content is unreadable without the key — nodes store ciphertext, not content
- **Transferable access**: transferring the UTXO re-wraps the key with the new owner's public key
- **Explicit publication**: RevealContent posts the symmetric key on-chain — irrevocable, signed, deliberate

## Technical Design

### New OutputType: EncryptedContent (type 32)

```rust
struct EncryptedContentOutput {
    // Encrypted payload (AES-256-GCM)
    ciphertext: Vec<u8>,           // up to 512 KB
    
    // Symmetric key wrapped with owner's public key (ECIES)
    wrapped_key: [u8; 80],         // ECIES(owner_pubkey, k_content)
    
    // AES-GCM nonce
    nonce: [u8; 12],
    
    // BLAKE3 hash of plaintext — for identification without decryption
    content_hash: [u8; 32],
    
    // Public metadata (NOT encrypted) — visible to everyone
    metadata: ContentMetadata,
}

struct ContentMetadata {
    title: String,                 // max 256 bytes
    content_type: String,          // MIME type: "image/png", "text/plain", etc.
    creator: PublicKey,            // original creator (immutable across transfers)
    royalty_bps: u16,              // basis points (500 = 5%)
    created_at: u64,               // slot of creation
}
```

### New TxType: RevealContent

```rust
struct RevealContentTx {
    // Reference to the EncryptedContent UTXO
    target_utxo: Outpoint,
    
    // The symmetric key — once on-chain, anyone can decrypt
    k_content: [u8; 32],
    
    // Explicit acknowledgment that this is irrevocable
    acknowledgment: bool,          // must be true
    
    // Signed by the owner — cryptographic proof of who published
    signature: Signature,
}
```

### Lifecycle

```
Create:
  1. Creator generates random k_content (32 bytes)
  2. Encrypts content: ciphertext = AES-256-GCM(k_content, plaintext)
  3. Wraps key: wrapped_key = ECIES(creator_pubkey, k_content)
  4. Computes content_hash = BLAKE3(plaintext)
  5. Submits TX with EncryptedContent output

Transfer:
  1. Sender decrypts wrapped_key with their private key → k_content
  2. Re-wraps: new_wrapped_key = ECIES(recipient_pubkey, k_content)
  3. New UTXO has same ciphertext, nonce, content_hash but new wrapped_key
  4. Sender can no longer decrypt (old wrapped_key spent)

View (owner only):
  1. Owner decrypts wrapped_key with private key → k_content
  2. Decrypts ciphertext with k_content → plaintext
  3. Verifies BLAKE3(plaintext) == content_hash

Reveal (publish permanently):
  1. Owner submits RevealContent TX with k_content in plaintext
  2. TX is signed — cryptographic proof of who published
  3. acknowledgment must be true — deliberate act
  4. k_content is now on-chain — anyone can decrypt the ciphertext
  5. Irrevocable — cannot be undone

Export (CLI):
  doli nft --export <UTXO> -o file.png  (owner decrypts and saves)
  doli nft --reveal <UTXO>              (publish key on-chain)
```

### What Nodes See

| State | Nodes see | Owner sees |
|-------|-----------|------------|
| Created | ciphertext + metadata (title, type, creator) | plaintext content |
| Transferred | same ciphertext, new wrapped_key | plaintext content |
| Revealed | ciphertext + k_content → can decrypt | plaintext content |
| Not revealed | random noise | plaintext content |

### Operator Liability Protection

Nodes store **ciphertext** — encrypted bytes indistinguishable from random data. They cannot:
- Read the content
- Moderate the content  
- Be held responsible for content they cannot access

Like a bank with sealed safety deposit boxes. The bank stores the box. The bank cannot open it. The bank is not responsible for what's inside.

If a user publishes illegal content via RevealContent:
- Their **signature** is on the RevealContent TX — irrefutable proof of who published
- Their **public key** is linked to their on-chain identity
- The operator facilitated storage of encrypted data, not publication of illegal content
- The **user** made the deliberate, signed act of revealing

### Migration from Current NFT

Current NFTs (OutputType::NFT) store plaintext in `extra_data`. EncryptedContent replaces this:

- **New content**: EncryptedContent only — no plaintext option
- **Existing NFTs**: remain as-is in the UTXO set (legacy, read-only)
- **No new plaintext**: after activation, validation rejects new OutputType::NFT transactions
- **CLI**: `doli nft --mint` always creates EncryptedContent. No `--plaintext` flag.

Cifrado obligatorio. Si plaintext sigue permitido, la protección no existe.

### Activation

- Hard fork with activation height (constant gate, no HardForkSchedule)
- Before activation: only OutputType::NFT accepted
- After activation: only OutputType::EncryptedContent accepted for NEW transactions. Existing NFT UTXOs remain spendable/transferable but new NFT outputs are rejected.
- Rolling deploy — nodes with new binary enforce the gate

## Files That Change

| File | Change |
|------|--------|
| `core/src/transaction/output.rs` | New OutputType::EncryptedContent, serialization |
| `core/src/transaction/data.rs` | New TxType::RevealContent |
| `core/src/validation/` | Validate EncryptedContent outputs, RevealContent TXs |
| `storage/src/utxo/` | Store/retrieve encrypted content, index by content_hash |
| `rpc/src/methods/` | getNftByTokenId supports EncryptedContent, getRevealedContent |
| `cli/src/cmd_nft.rs` | Encrypt on mint, decrypt on export, reveal command |
| `consensus/constants.rs` | ENCRYPTED_CONTENT_ACTIVATION_HEIGHT |
| `crypto/` | ECIES key wrapping, AES-256-GCM encrypt/decrypt |

## Cryptographic Primitives

- **Content encryption**: AES-256-GCM (authenticated, 12-byte nonce)
- **Key wrapping**: ECIES with X25519 (derived from Ed25519 keys already in use)
- **Content identification**: BLAKE3 hash of plaintext
- **Publication signature**: Ed25519 (existing key infrastructure)

## Cost

- **Storage**: ciphertext ≈ plaintext + 28 bytes (GCM tag + nonce). Negligible overhead.
- **Computation**: AES-256-GCM is hardware-accelerated on all modern CPUs. ~1μs per KB.
- **Transfer overhead**: one ECIES unwrap + wrap per transfer (~0.1ms)
- **Validation**: nodes validate metadata and structure, never decrypt content

## Edge Cases

| Case | Behavior |
|------|----------|
| Transfer to self | Re-wrap with same key (refresh wrapped_key) |
| Reveal by non-owner | TX rejected — signature must match UTXO owner |
| Double reveal | TX rejected — content already revealed |
| Content > 512KB | TX rejected — size limit enforced at validation |
| Lost private key | Content permanently inaccessible (key is gone) |
| Reveal then transfer | New owner gets both: wrapped_key + on-chain k_content |
| content_hash collision | Different content, same hash — astronomically unlikely with BLAKE3 |

## Future Considerations

- **Selective reveal**: reveal to specific addresses without publishing globally (multi-party ECIES)
- **Time-locked reveal**: content auto-reveals after a block height (dead man's switch)
- **Streaming**: EncryptedContent with chunked ciphertext for large files
- **DRM-free marketplace**: buy = transfer UTXO = transfer access. No platform lock-in.
- **Private messaging**: EncryptedContent with recipient's pubkey. Transfer IS delivery.
