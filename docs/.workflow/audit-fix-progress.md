# Audit Fix Progress

## P1 Findings
| ID | Description | Status |
|----|-------------|--------|
| AUDIT-AUTH-001 | PSBT buyer trusts partial_tx outputs without verification | PENDING |
| AUDIT-CRYPTO-002 | PublicKey::from_bytes panics on invalid curve points | PENDING |
| AUDIT-LOGIC-001 | CLI fee calculation diverges from tx.minimum_fee() | PENDING |

## P2 Findings
| ID | Description | Status |
|----|-------------|--------|
| AUDIT-AUTH-003 | creator_hash not enforced immutable (consensus) | PENDING |
| AUDIT-INJ-001 | BPS truncation u64→u16 without bounds check | PENDING |
| AUDIT-AUTH-002 | Sell doesn't verify wallet owns UTXO | PENDING |
| AUDIT-CRYPTO-001 | content_key not zeroized after use | PENDING |
| AUDIT-CFG-003 | RPC exposes wrappedKey convenience field | PENDING |
| AUDIT-LOGIC-002 | Unchecked price + fee addition | PENDING |
| AUDIT-CFG-001 | Offer files written world-readable | PENDING |
| AUDIT-LOGIC-006 | No offer expiration/revocation | SKIPPED (design change) |
| AUDIT-CRYPTO-010 | ct_len bounds check improvement | PENDING |
| AUDIT-CFG-005 | resolve_pubkey creates intent signal | SKIPPED (inherent) |
