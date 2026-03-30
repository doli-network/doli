# DOLI Blockchain Security Audit — 2026-03-30

**Scope:** Security-focused audit of the DOLI blockchain codebase
**Mode:** Read-only (no code changes)
**Auditor:** OMEGA Reviewer Agent

---

## Summary

| Priority | Count | Description |
|----------|-------|-------------|
| **P0 (Critical)** | 2 | Exploitable vulnerabilities — fund loss or network takeover |
| **P1 (Major)** | 5 | Security weaknesses exploitable under specific conditions |
| **P2 (Minor)** | 4 | Defense-in-depth improvements |
| **P3 (Suggestion)** | 3 | Best practice recommendations |
| **Total** | **14** | |

**Verdict: REQUIRES CHANGES — DO NOT DEPLOY TO MAINNET.**

---

## P0: Critical

### AUDIT-P0-001: Transaction Signature Verification Bypassed in Production

- **Priority:** P0 (Critical)
- **Category:** crypto
- **Location:** `crates/core/src/validation/utxo.rs:597-611`, `crates/storage/src/utxo/set.rs:345-354`
- **Description:** DOLI uses P2PKH where UTXOs store only the `pubkey_hash`. The `Input` struct (`crates/core/src/transaction/types.rs:234-252`) does NOT include a `public_key` field. The production `UtxoSet` implementation always returns `pubkey: None` in `UtxoInfo`. The function `verify_input_signature` at `utxo.rs:608-611` explicitly returns `Ok(())` when `pubkey` is `None`:
  ```rust
  let pubkey = match utxo.pubkey.as_ref() {
      Some(pk) => pk,
      None => return Ok(()), // SKIPS ALL VERIFICATION
  };
  ```
  This means for every Normal/Bond output in production, signature verification is completely skipped. The pubkey-hash match check and Ed25519 verify call are dead code.
- **Impact:** **Any party can spend any UTXO in the system.** An attacker can construct a transaction referencing any unspent output, provide garbage in the signature field, and the transaction will pass all validation. This enables unlimited theft of all DOLI, bonds, NFTs, and fungible assets.
- **Suggested Fix:** The `Input` struct must include a `public_key: PublicKey` field. The spender provides their public key alongside their signature. Verification must: (1) hash the provided pubkey and compare to the UTXO's `pubkey_hash`, (2) verify the signature against the provided pubkey. This is consensus-breaking (hard fork required).
- **Test Strategy:** Create a transaction spending a UTXO owned by key A, sign with key B (or all-zero signature). Submit via `sendTransaction`. If accepted and included in a block, vulnerability confirmed.

### AUDIT-P0-002: RPC Admin Methods Lack Authentication

- **Priority:** P0 (Critical)
- **Category:** access_control
- **Location:** `crates/rpc/src/methods/dispatch.rs:59-62`, `crates/rpc/src/methods/guardian.rs`, `crates/rpc/src/methods/pruning.rs`, `crates/rpc/src/methods/backfill.rs`
- **Description:** Administrative RPC methods have no authentication:
  - `pauseProduction` — halts block production
  - `resumeProduction` — resumes production
  - `createCheckpoint` — writes RocksDB checkpoints to user-specified filesystem paths
  - `pruneBlocks` — deletes block data
  - `backfillFromPeer` — initiates HTTP requests to arbitrary URLs

  The `allowed_methods` field in `RpcConfig` is dead code (never checked in dispatch). RPC defaults to `127.0.0.1` but can be exposed via `--rpc-bind 0.0.0.0`. CORS uses `allow_origin(Any)`.
- **Impact:** When RPC is network-accessible (required for explorer), any participant can: halt production on any node, write checkpoint data to arbitrary paths via path traversal, delete block history, trigger SSRF via `backfillFromPeer`. Coordinated attack could halt all reachable nodes.
- **Suggested Fix:** (1) API key auth for admin methods; (2) Separate admin RPC on localhost-only port; (3) IP allowlist; (4) Implement the existing `allowed_methods` filter. For `createCheckpoint`, sanitize paths.
- **Test Strategy:** Start node with `--rpc-bind 0.0.0.0`. Call `pauseProduction` from another machine without credentials. Call `createCheckpoint` with `["../../../tmp/test"]`.

---

## P1: Major

### AUDIT-P1-001: missed_producers Header Field Not Validated on Receiving Nodes

- **Priority:** P1 (Major)
- **Category:** consensus
- **Location:** `bins/node/src/node/apply_block/post_commit.rs:142-154`, `bins/node/src/node/validation_checks.rs` (absent)
- **Description:** Block producers include `missed_producers` in the header. The producing side caps at `MAX_MISSED_PER_BLOCK=3` and `max_total=epoch_producer_list.len()/3`. The receiving/validation side iterates without checking: (1) list length within MAX_MISSED_PER_BLOCK; (2) keys are in the epoch_producer_list; (3) total excluded doesn't exceed active/3.
- **Impact:** A malicious producer can include unbounded or arbitrary pubkeys in `missed_producers`, manipulating scheduling rotation to exclude competitors.
- **Suggested Fix:** Add validation checking length, membership, and total exclusion cap.
- **Test Strategy:** Create a block with 50 `missed_producers` entries. Verify rejection.

### AUDIT-P1-002: CORS Wildcard on RPC Server

- **Priority:** P1 (Major)
- **Category:** access_control
- **Location:** `crates/rpc/src/server.rs:88-91`
- **Description:** CORS enabled with `allow_origin(Any)`, `allow_methods(Any)`, `allow_headers(Any)`. Combined with lack of RPC auth.
- **Impact:** Any website visited by a node operator can make cross-origin requests to the operator's RPC endpoint, calling `pauseProduction`, `pruneBlocks`, or `sendTransaction` from the browser.
- **Suggested Fix:** Restrict CORS to configured origins. Never use `Any` for admin methods.
- **Test Strategy:** From a webpage on a different origin, POST to `http://localhost:8500/` calling `pauseProduction`.

### AUDIT-P1-003: Backfill RPC Enables Server-Side Request Forgery (SSRF)

- **Priority:** P1 (Major)
- **Category:** input_validation
- **Location:** `crates/rpc/src/methods/backfill.rs:95-128`
- **Description:** `backfillFromPeer` takes a `rpc_url` parameter and makes HTTP POST requests to it with no validation. Attacker can provide internal URLs like `http://169.254.169.254/latest/meta-data/`.
- **Impact:** SSRF to cloud metadata services, internal APIs, or internal network scanning.
- **Suggested Fix:** Validate URL scheme, reject private/link-local IP ranges, gate behind authentication.
- **Test Strategy:** Call with AWS metadata URL on cloud instance.

### AUDIT-P1-004: Path Traversal in createCheckpoint RPC

- **Priority:** P1 (Major)
- **Category:** input_validation
- **Location:** `crates/rpc/src/methods/guardian.rs:84-86`
- **Description:** `createCheckpoint` accepts a filesystem path from RPC params via `PathBuf::from(p)` with no sanitization.
- **Impact:** Write RocksDB checkpoint data to arbitrary filesystem paths.
- **Suggested Fix:** Reject `..` segments, enforce paths within data directory.
- **Test Strategy:** Call with `["/../../../tmp/evil"]`, verify rejection.

### AUDIT-P1-005: Mempool Does Not Verify Transaction Signatures

- **Priority:** P1 (Major)
- **Category:** input_validation
- **Location:** `crates/mempool/src/pool.rs:102-215`
- **Description:** Mempool's `add_transaction` calls structural validation and fee checks but never calls `validate_transaction_with_utxos` for signature verification. Even with AUDIT-P0-001 fixed, mempool would still propagate unsigned transactions.
- **Impact:** Mempool fillable with invalid transactions, bandwidth-based DoS via gossip spam.
- **Suggested Fix:** Call `validate_transaction_with_utxos` in mempool acceptance path (after fixing P0-001).
- **Test Strategy:** Submit transaction with garbage signature, verify rejection.

---

## P2: Minor

### AUDIT-P2-001: ip_colocation_factor_threshold Set to 500

- **Priority:** P2 (Minor)
- **Category:** network
- **Location:** `crates/network/src/gossip/config.rs:149`
- **Description:** Gossipsub `ip_colocation_factor_threshold` is 500 (for "large local testnets"). Standard production recommendation is 1-3.
- **Impact:** Sybil nodes from a single IP face no gossipsub scoring penalty, weakening eclipse attack resistance.
- **Suggested Fix:** Use network-aware config: mainnet/testnet=3-5, devnet=500.
- **Test Strategy:** Launch 50 peers from same IP on mainnet config, verify scoring penalties.

### AUDIT-P2-002: bincode Deserialization of Untrusted Snapshot Data

- **Priority:** P2 (Minor)
- **Category:** serialization
- **Location:** `bins/node/src/node/fork_recovery.rs:572-581`
- **Description:** Snap sync deserializes `chain_state`, `utxo_set`, `producer_set` from peer bytes using `bincode::deserialize`. The 16MB transport cap helps but doesn't prevent internal amplification.
- **Impact:** Crafted snapshot with pathological bincode data could cause excessive memory allocation.
- **Suggested Fix:** Use `bincode::Options::with_limit()` for deserialization size cap.
- **Test Strategy:** Craft bincode payload within 16MB that decodes to >1GB structure.

### AUDIT-P2-003: Gossip TX Batch Count Unbounded Pre-Allocation

- **Priority:** P2 (Minor)
- **Category:** dos
- **Location:** `crates/network/src/gossip/publish.rs:188-192`
- **Description:** `Vec::with_capacity(count)` where `count` is a u32 from gossip message. A malicious count of `u32::MAX` would attempt multi-GB allocation.
- **Impact:** OOM crash on receiving nodes from crafted gossip message.
- **Suggested Fix:** Cap `count` to `count.min(10_000)` or derive from remaining message size.
- **Test Strategy:** Send gossip message with count=4294967295 but 100 bytes of data.

### AUDIT-P2-004: Deprecated `select_producer_for_slot` Still in Codebase

- **Priority:** P2 (Minor)
- **Category:** consensus
- **Location:** `crates/core/src/consensus/selection.rs:24-80`
- **Description:** Deprecated function with different algorithm (linear vs binary search) still exists. If any consensus path still calls it, scheduling disagreements could occur.
- **Impact:** Potential scheduling inconsistency between nodes using different functions.
- **Suggested Fix:** Remove or verify all call sites are non-consensus.
- **Test Strategy:** Grep all call sites, verify none in consensus paths.

---

## P3: Suggestions

### AUDIT-P3-001: RPC Request Body Limit Too Large

- **Location:** `crates/rpc/src/server.rs:24`
- **Description:** `MAX_BODY_SIZE` is 256KB. Most RPC requests are <1KB.
- **Suggested Fix:** Reduce to 64KB for most endpoints.

### AUDIT-P3-002: No Rate Limiting on RPC Endpoints

- **Location:** `crates/rpc/src/server.rs`
- **Description:** P2P has rate limiting but RPC has none.
- **Suggested Fix:** Add per-IP rate limiting middleware (e.g., `tower::limit::RateLimitLayer`).

### AUDIT-P3-003: Test Keys Should Be Compile-Time Gated

- **Location:** `crates/updater/src/test_keys.rs`
- **Description:** Test maintainer keys available via runtime check, not compile-time gating.
- **Suggested Fix:** Gate behind `#[cfg(test)]` or feature flag.

---

## Specs/Docs Drift

1. `specs/security_model.md:26` references Wesolowski VDF — should reference hash-chain VDF per actual implementation
2. `CLAUDE.md` says "27 types" for transactions — should verify against current `TxType` enum
3. `specs/security_model.md` Sybil resistance section references VDF construction inconsistently

## Modules Not Fully Audited

- `crates/bridge/src/` — Bitcoin bridge (early-stage)
- `bins/cli/src/` — CLI wallet (spot-checked only)
- `crates/updater/src/apply.rs` — Binary replacement logic (partial review)
- `crates/core/src/conditions/` — Covenant evaluation (spot-checked)
- `crates/core/src/lending.rs`, `crates/core/src/pool.rs` — DeFi modules (not audited)

---

*Generated by OMEGA Reviewer Agent — 2026-03-30*
