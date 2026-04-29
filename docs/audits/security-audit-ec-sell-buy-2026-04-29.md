# Security Audit: EncryptedContent Sell/Buy Flow

**Date**: 2026-04-29
**Scope**: Commits bb0e227c, 3b404d00 — EncryptedContent sell/buy/transfer with royalty enforcement
**Auditors**: 5 parallel (injection, auth, crypto, logic, config)
**Mode**: Read-only (no fixes applied)

---

## Executive Summary

The EncryptedContent sell/buy implementation is **solid at the consensus layer** — no exploitable fund-theft vectors were found. The UTXO model, AnyoneCanPay with `committed_output_count`, and on-chain royalty enforcement (EC009) provide strong guarantees.

However, the **CLI layer** has several gaps where it trusts data that it should verify before presenting to the user or constructing transactions. The highest-severity finding is a CLI-side trust-without-verify pattern in the PSBT buy flow.

**Total findings**: 38 raw, **22 unique** after deduplication across auditors.

---

## Findings by Priority

### P0 — Critical: 0

None found.

### P1 — High: 3

| ID | Domain | Description | Location |
|----|--------|-------------|----------|
| **AUDIT-AUTH-001** | Auth | PSBT buyer blindly trusts seller's `partial_tx` outputs. No verification that content goes to buyer, payment goes to seller, or amounts match the offer. A malicious seller can craft outputs that redirect funds. | `buy.rs:422-454` |
| **AUDIT-CRYPTO-002** | Crypto | `PublicKey::from_bytes()` accepts any 32 bytes without curve validation. The downstream `ed25519_to_x25519_public` panics (`.expect()`) on invalid Edwards points. A user providing an invalid hex pubkey crashes the CLI. | `mod.rs:42`, `transfer.rs:68` |
| **AUDIT-LOGIC-001** | Logic | CLI fee calculation is computed inline before the change output is added, diverging from canonical `Transaction::minimum_fee()`. Currently safe because Normal change outputs have 0 `extra_data`, but a latent regression hazard. | `buy.rs:150-154`, `transfer.rs:208-212` |

### P2 — Medium: 10

| ID | Domain | Description | Location |
|----|--------|-------------|----------|
| **AUDIT-AUTH-003** | Auth | `creator_hash` in EncryptedContent v1 metadata is NOT enforced as immutable across transfers. A reseller can rewrite it to redirect all future royalties. Requires consensus fix at future activation height. | `validation/utxo.rs` |
| **AUDIT-INJ-001** | Injection | Royalty BPS silently truncates from `u64` to `u16` at 5 call sites. A crafted offer file with `bps: 65536` truncates to `0`, zeroing royalties at the CLI layer. Consensus (EC007) is the only backstop. | `buy.rs:100-102`, `sell.rs:291`, `transfer.rs:122`, `mod.rs:103` |
| **AUDIT-AUTH-002** | Auth | `cmd_nft_sell` and `cmd_nft_sell_sign` don't verify that the wallet's pubkey_hash matches the UTXO's pubkey_hash. Consensus rejects the eventual broadcast, but the buyer wastes time with an invalid offer. | `sell.rs:56,249` |
| **AUDIT-CRYPTO-001** | Crypto | `content_key` ([u8; 32]) is never zeroized after use. The PrivateKey type uses `ZeroizeOnDrop`, but the content key (the master decryption secret) lives as plain stack bytes. | `mod.rs:86-88`, `transfer.rs:105-106` |
| **AUDIT-CFG-003** | Config | RPC `OutputResponse` exposes `wrappedKey` as a parsed convenience field alongside `extraData`. While on-chain data, the parsed field reduces attacker effort for bulk collection. | `block.rs:391-393` |
| **AUDIT-LOGIC-002** | Logic | `price_units + fee_units` uses unchecked u64 addition. Practically bounded by TOTAL_SUPPLY, but `coins_to_units` can theoretically parse near-u64::MAX. | `buy.rs:155`, `buy.rs:393` |
| **AUDIT-CFG-001** | Config | Offer files written with `std::fs::write()` using default umask (potentially world-readable). Signed offers are bearer credentials on shared systems. | `sell.rs:118,405` |
| **AUDIT-LOGIC-006** | Logic | Signed offer files have no expiration or revocation mechanism. Seller can only invalidate by spending the UTXO. (Converges with AUDIT-CFG-004.) | `sell.rs:386-402` |
| **AUDIT-CRYPTO-010** | Crypto | `ct_len` parsed from RPC `extraData` cast to `usize` without pre-offset sanity check. Safe on 64-bit, but misleading error path on theoretical 32-bit. | `mod.rs:72-74` |
| **AUDIT-CFG-005** | Config | `resolve_pubkey_from_address()` queries recipient history before transfer, creating a correlatable intent signal in RPC logs. | `transfer.rs:361` |

### P3 — Low/Informational: 9

| ID | Domain | Description |
|----|--------|-------------|
| AUDIT-INJ-002 | Injection | `output_index` read as u64 then cast `as u32` — use `u32::try_from()` |
| AUDIT-AUTH-004 | Auth | `build_ec_output_for_buyer` doesn't explicitly verify seller keypair matches UTXO pubkey_hash (ECIES unwrap is implicit check) |
| AUDIT-AUTH-005 | Auth | PSBT `buyer_pubkey_hash` check compares plaintext JSON field, not cryptographic commitment |
| AUDIT-CRYPTO-005 | Crypto | Nonce reuse across transfers is correct (ciphertext unchanged) but fragile if ciphertext ever changes |
| AUDIT-INJ-003-007 | Injection | Path traversal (expected CLI behavior), extra JSON fields (ignored safely), ct_len theoretical 32-bit, UTF-8 MIME handled, hex::decode with unwrap_or_default |
| AUDIT-LOGIC-003 | Logic | Greedy UTXO selection is suboptimal (UX only) |
| AUDIT-LOGIC-004 | Logic | Seller==creator royalty skip is mathematically sound |
| AUDIT-LOGIC-008/009 | Logic | AnyoneCanPay protection and saturating_sub verified correct |
| AUDIT-CFG-007 | Config | Verbose error messages expose internal state (acceptable for CLI) |

---

## Convergence Matrix

Findings confirmed by multiple independent auditors (higher confidence):

| Finding | Auditors | Confidence |
|---------|----------|------------|
| PSBT output trust-without-verify | Auth, Logic | HIGH |
| `from_bytes` panic on invalid pubkey | Crypto, Injection | HIGH |
| RPC wrappedKey exposure | Crypto, Config | HIGH |
| No offer expiration/revocation | Logic, Config | HIGH |
| BPS truncation u64→u16 | Injection, Auth | MEDIUM |
| content_key not zeroized | Crypto | MEDIUM |
| Fee calculation divergence | Logic | MEDIUM |

---

## Systemic Patterns

**Pattern 1: CLI trusts external data without verification**
The CLI trusts RPC responses, offer file contents, and partial transactions without re-deriving or cross-checking values. The consensus layer catches most issues, but the CLI can mislead users (e.g., display wrong royalty, wrong recipient).

**Pattern 2: Defensive casting gaps**
Multiple `as u16` and `as u32` casts from JSON u64 values without bounds checking. While individually low-impact, this pattern across 5+ locations suggests a missing validation helper.

---

## Recommended Fix Priority

1. **AUDIT-AUTH-001** (P1) — Verify PSBT partial_tx outputs match offer claims before buyer signs
2. **AUDIT-CRYPTO-002** (P1) — Validate pubkey bytes before `from_bytes`, return error instead of panic
3. **AUDIT-LOGIC-001** (P1) — Use `tx.minimum_fee()` after full transaction construction
4. **AUDIT-AUTH-003** (P2) — Enforce creator_hash immutability at consensus (future activation height)
5. **AUDIT-INJ-001** (P2) — Add `bps <= MAX_ROYALTY_BPS` check before u16 cast
6. **AUDIT-CRYPTO-001** (P2) — Wrap content_key in `Zeroizing<[u8; 32]>`
7. **AUDIT-CFG-001** (P2) — Write offer files with mode 0o600

---

## Verified Secure

The following were explicitly verified as NOT vulnerable:
- Non-owners cannot sell/transfer EncryptedContent (ECIES unwrap + consensus signature check)
- PSBT offers cannot be replayed (UTXO double-spend protection)
- Content keys cannot be extracted from offer files (only wrapped keys present)
- AnyoneCanPay `committed_output_count` prevents buyer output injection
- Royalty enforcement (EC009) is robust at the consensus layer
- No SQL, command, XSS, or template injection possible (Rust CLI)
- No private key material in offer files or RPC responses
- Integer arithmetic uses u128 intermediate for royalty calculation (no overflow)
- No new dependencies introduced
