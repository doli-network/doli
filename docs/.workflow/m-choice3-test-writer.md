# M-Choice3 Test Writer Report — `doli chain-repair` pure helpers

**Date**: 2026-04-16
**Milestone**: `M-Choice3` (docs/.workflow/milestone-progress.md row 40)
**Incident**: INC-I-034
**Spec**: `specs/scheduler-state-architecture.md` → "What ADDS" → `bins/cli/src/repair_chain.rs (new command, ~100 lines)`
**Branch**: `synmgrefactor`
**File touched**: `bins/cli/src/cmd_chain.rs` (new `mod repair_chain_tests` appended as sibling of `mod wipe_tests`)
**Confidence**: `conf(0.65)` per milestone row (below 0.7 FAIL→PASS threshold — FAIL evidence shipped nonetheless to protect developer against test-tailored-to-fix drift)

---

## 1. Output Contract enumeration (per CLAUDE.md Global Rule #21)

All four helpers are **pure**. The language-agnostic observable-output cells per CLAUDE.md / `.claude/protocols/output-contract.md`:

| Helper | Mutable params | Receiver/self | Return value | Persistent store |
|---|---|---|---|---|
| `validate_peer_url(peer: &str, local: &str) -> Result<(), String>` | none (both `&str` immutable borrows) | none (free fn) | `Result<(), String>` — `Ok(())` OR `Err(message)` with specified substrings per path | none |
| `format_gap_summary(&ChainIntegrity) -> String` | none (immutable borrow) | none (free fn) | `String` containing required substrings per path | none |
| `BackfillPhase::from_status(&BackfillStatusResponse) -> BackfillPhase` | none (immutable borrow) | none (associated fn) | `BackfillPhase` enum variant (`Running{imported,total,pct}` \| `Complete{imported}` \| `Failed(String)`) with populated fields | none |
| `format_progress(&BackfillPhase) -> String` | none (immutable borrow) | none (free fn) | `String` containing required substrings per phase variant | none |

**No I/O. No global state. No ad-hoc randomness.** Every cell is fully deterministic from inputs.

---

## 2. Output × Path matrix with assertion coverage

### 2.1 `validate_peer_url`

| # | Path (input class) | Cell asserted | Assertion name |
|---|---|---|---|
| 1 | Well-formed remote RPC URL, different host | Return = `Ok(())` | `test_validate_peer_url_accepts_remote_host` |
| 2 | Empty string | Return = `Err(msg)` AND `msg` contains `"required"` or `"empty"` | `test_validate_peer_url_rejects_empty_string` |
| 3 | libp2p peer ID (`12D3KooW...`) — MEMORY.md rule #1 trap | Return = `Err(msg)` AND `msg` contains both `"peer id"` and `"rpc url"` | `test_validate_peer_url_rejects_libp2p_peer_id` |
| 4 | Peer exactly equals local endpoint | Return = `Err(msg)` AND `msg` contains `"self"` | `test_validate_peer_url_rejects_exact_self` |
| 5 | Peer same host:port as local modulo trailing `/` | Return = `Err(msg)` AND `msg` contains `"self"` | `test_validate_peer_url_rejects_self_after_trailing_slash_strip` |
| 6 | Missing `http://` or `https://` scheme | Return = `Err(msg)` AND `msg` contains `"http"` | `test_validate_peer_url_rejects_missing_scheme` |

### 2.2 `format_gap_summary`

| # | Path | Cell asserted | Assertion name |
|---|---|---|---|
| 1 | `missing_count=0` (complete chain) | Return contains `"complete"` (case-insensitive) | `test_format_gap_summary_complete_chain` |
| 2 | Small gap list (`missing_count=5`, 2 ranges) | Return contains `"5"`, `"missing"`, `"1-3"`, `"7-8"` | `test_format_gap_summary_small_gap_list` |
| 3 | >5 ranges (8 ranges total, total=15) | Return contains first 5 ranges AND truncation marker (`"more"` or `"..."`) AND count `3` (8-5=3 truncated) | `test_format_gap_summary_truncates_after_five_ranges` |

### 2.3 `BackfillPhase::from_status`

| # | Path | Cell asserted | Assertion name |
|---|---|---|---|
| 1 | `running=true, error=None` | Return = `Running { imported=50, total=100, pct=50 }` (exact field equality) | `test_backfill_phase_from_status_running` |
| 2 | `running=false, error=Some(msg)` | Return = `Failed(msg)` AND msg preserves the input string | `test_backfill_phase_from_status_failed` |
| 3 | `running=false, error=None` | Return = `Complete { imported=100 }` (exact field equality) | `test_backfill_phase_from_status_complete` |

### 2.4 `format_progress`

| # | Path | Cell asserted | Assertion name |
|---|---|---|---|
| 1 | `Running{50,100,50}` | Return contains `"50/100"` AND `"50%"` | `test_format_progress_running` |
| 2 | `Complete{100}` | Return contains `"imported 100"` or `"imported: 100"` (case-insensitive) | `test_format_progress_complete` |
| 3 | `Failed("connection refused")` | Return contains `"FAILED"` AND `"connection refused"` | `test_format_progress_failed` |

**Total: 15 assertions across 4 helpers × all paths.** Every cell in the Output × Path matrix has at least one assertion. No orphan return shapes, no untested paths.

---

## 3. FAIL evidence

Command:
```
cargo test -p doli-cli repair_chain_tests
```

Result: **compile error (22 errors) — tests cannot run because helpers do not exist**. This is the canonical FAIL state for skeleton-first TDD.

### Error classes (de-duplicated):

```
error[E0432]: unresolved import `crate::rpc_client::BackfillStatusResponse`
   --> bins/cli/src/cmd_chain.rs:637:9

error[E0425]: cannot find function `validate_peer_url` in this scope
   --> bins/cli/src/cmd_chain.rs:689:17  (and 5 other call sites)

error[E0425]: cannot find function `format_gap_summary` in this scope
   --> bins/cli/src/cmd_chain.rs:767:17  (and 2 other call sites)

error[E0433]: failed to resolve: use of undeclared type `BackfillPhase`
   --> bins/cli/src/cmd_chain.rs:839:21  (and 7 other references)

error[E0425]: cannot find function `format_progress` in this scope
   --> bins/cli/src/cmd_chain.rs:897:17  (and 2 other call sites)

error: could not compile `doli-cli` (bin "doli" test) due to 22 previous errors
```

Error inventory confirms all four helpers + one type are missing, matching the expected skeleton-first handoff.

---

## 4. Handoff to developer

The developer must add the following to `bins/cli/src/cmd_chain.rs` (or a new `bins/cli/src/repair_chain.rs` that is `mod`-declared from `main.rs` — either placement works as long as the tests in `mod repair_chain_tests` can resolve these names via `super::*`).

### 4.1 Type re-export / addition in `bins/cli/src/rpc_client.rs`

Add a local mirror of `crates/rpc/src/types/chain.rs::BackfillStatusResponse`:

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillStatusResponse {
    pub running: bool,
    pub imported: u64,
    pub total: u64,
    pub pct: u64,
    #[serde(default)]
    pub error: Option<String>,
}
```

(Plus an `RpcClient::backfill_status()` and `RpcClient::backfill_from_peer(rpc_url: &str)` when wiring the command proper — but the test module only needs the struct.)

### 4.2 Helper signatures the tests demand

```rust
/// Reject malformed or self-pointing peer RPC URLs.
/// Accepts: well-formed http://HOST:PORT to a host != local_endpoint host:port.
/// Rejects: empty, libp2p peer ID (looks like "12D3KooW..."), self URL (exact or
/// trailing-slash variant), URL missing http:// or https:// scheme.
pub(crate) fn validate_peer_url(peer: &str, local_endpoint: &str) -> Result<(), String>;

/// Human summary of a `ChainIntegrity` report.
/// complete=true                  → contains "complete"
/// missing_count>0, ≤5 ranges     → contains total, "missing", and every range
/// missing_count>0, >5 ranges     → first 5 ranges + "(... N more ranges)"
pub(crate) fn format_gap_summary(integrity: &crate::rpc_client::ChainIntegrity) -> String;

#[derive(Debug)]
pub(crate) enum BackfillPhase {
    Running { imported: u64, total: u64, pct: u64 },
    Complete { imported: u64 },
    Failed(String),
}

impl BackfillPhase {
    /// Interpret a `BackfillStatusResponse` from the RPC server into a local phase.
    /// running=true          → Running
    /// running=false+err=Some → Failed(err)
    /// running=false+err=None → Complete
    pub(crate) fn from_status(s: &crate::rpc_client::BackfillStatusResponse) -> Self;
}

/// One-line progress string for a phase.
/// Running{i,t,p}    → "…i/t (p%)…"
/// Complete{i}        → "…imported i…"
/// Failed(m)          → "…FAILED: m…"
pub(crate) fn format_progress(phase: &BackfillPhase) -> String;
```

Substring contracts (enforced by tests — must match exactly or the test will fail):

- `validate_peer_url`:
  - empty → `"required"` or `"empty"`
  - peer ID → must contain BOTH `"peer id"` and `"rpc url"` (case-insensitive)
  - self (exact and `/`-variant) → `"self"`
  - missing scheme → `"http"`
- `format_gap_summary` complete branch → `"complete"` (case-insensitive)
- `format_gap_summary` with gaps → total count as digit(s), the token `"missing"`, and every range string present (first 5 when truncated)
- `format_gap_summary` truncation → `"more"` or `"..."` AND a digit matching `(total_ranges − 5)`
- `format_progress` Running → `"i/t"` and `"p%"` (with `%` suffix)
- `format_progress` Complete → `"imported N"` or `"imported: N"`
- `format_progress` Failed → token `"FAILED"` AND the failure message substring

### 4.3 Work the developer still owes AFTER helpers pass

Out of scope for this test-writer step (and explicitly disallowed by the task):
- `doli chain-repair` subcommand variant in `bins/cli/src/commands.rs`
- Dispatch in `bins/cli/src/main.rs`
- Async orchestrator `pub(crate) async fn cmd_chain_repair(rpc_endpoint, peer, yes, poll_interval_secs, max_wait_secs) -> Result<()>`
- Integration tests over live RPC (out of scope per instructions)

---

## 5. Specs Gaps Found

None. `specs/scheduler-state-architecture.md` mentions the new command under "What ADDS" with a ~100 LOC budget and a clear purpose statement ("for ops to heal gap-holding nodes before Phase 2 activation"). The CLI contract, helper set, and substring shapes defined by the task brief are consistent with the existing `verify_chain_integrity()` surface in `bins/cli/src/rpc_client.rs` and the server-side `backfill.rs` error strings ("Backfill already in progress", "Tip divergence detected", "HTTP error at height N: ...").

---

## 6. Summary

- **Tests written**: 15 assertions across 4 pure helpers, organized by priority as Must acceptance-criteria tests (all helpers are Must for M-Choice3 to compile at all).
- **FAIL evidence**: 22-compile-error log captured above. No helper exists → every test fails → canonical TDD red state.
- **Output Contract**: enumerated per CLAUDE.md #21; every cell in the output × path matrix has a corresponding assertion.
- **Adversarial input**: covered where relevant (empty string, wrong-type input that looks like RPC URL but is a peer ID, self-reference). No external data crosses a trust boundary in these pure helpers — SQL/shell/path injection not applicable.
- **Handoff**: developer has exact function signatures, exact substring contracts, and exact enum shape.
