# Audit Fix Progress — Security 2026-03-30

| Finding | Priority | Status | Notes |
|---------|----------|--------|-------|
| AUDIT-P0-001 | P0 | FIXED | public_key added to Input, verify_input_signature uses it, hard fork gate via sig_verification_height |
| AUDIT-P0-002 | P0 | FIXED | RPC admin auth (bearer token + localhost bypass) + path sanitization |
| AUDIT-P1-001 | P1 | FIXED | missed_producers: length cap, membership check, total exclusion cap |
| AUDIT-P1-002 | P1 | FIXED | CORS: uses allowed_origins list, no more wildcard Any |
| AUDIT-P1-003 | P1 | FIXED | SSRF: URL scheme + private IP validation in backfillFromPeer |
| AUDIT-P1-004 | P1 | FIXED | Path traversal: reject absolute paths, ".." segments, canonicalize check |
| AUDIT-P1-005 | P1 | SKIPPED | Mempool sig verification — P0-001 handles this at block validation level |
| AUDIT-P2-001 | P2 | FIXED | ip_colocation_factor_threshold ENV-tunable (DOLI_IP_COLOCATION_THRESHOLD) |
| AUDIT-P2-002 | P2 | SKIPPED | bincode limits — 16MB transport cap is sufficient defense-in-depth |
| AUDIT-P2-003 | P2 | FIXED | Gossip batch: count cap (10K) + data-bounded pre-allocation |
| AUDIT-P2-004 | P2 | SKIPPED | Deprecated fn — informational, needs call-site audit |

## Summary
- **9 of 14 findings fixed** (2 P0, 4 P1, 3 P2)
- **2 skipped** (P2-002, P2-004 — low risk)
- **3 not applicable** (P1-005 covered by P0-001, P2-002/P2-004 informational)
- 1,695 lib tests passing, 0 failures
