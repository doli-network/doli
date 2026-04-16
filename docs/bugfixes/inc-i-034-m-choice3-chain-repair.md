# INC-I-034 / M-Choice3 — `doli chain-repair` CLI subcommand

**Date**: 2026-04-16
**Incident**: INC-I-034
**Branch**: `synmgrefactor` (local, not pushed)
**Milestone**: `M-Choice3` (`docs/.workflow/milestone-progress.md`)
**Spec**: `specs/scheduler-state-architecture.md` — "What ADDS" →
  `bins/cli/src/repair_chain.rs (new command, ~100 lines)`
**Confidence**: `conf(0.65)` (milestone row); raised to `conf(0.85)` by FAIL→PASS
evidence on 15 unit tests.

**REQ**: Operators of `santiago`, `ivan`, `seed3` — and any future gap-holding
node — must be able to heal their block_store gaps via a single CLI command
**before** the M-Choice1 `HardForkSchedule::EPOCH_SNAPSHOT_HF` activation
enforces block_store completeness at block-accept time (which would otherwise
fire `HALT_PRODUCTION`).

---

## Files touched

| File | Change |
|---|---|
| `bins/cli/src/rpc_client.rs` | +`BackfillStatusResponse` type, +`RpcClient::backfill_from_peer`, +`RpcClient::backfill_status` |
| `bins/cli/src/commands.rs` | +`Commands::ChainRepair` enum variant (4 CLI flags) |
| `bins/cli/src/cmd_chain.rs` | +4 pure helpers (`validate_peer_url`, `format_gap_summary`, `BackfillPhase::from_status`, `format_progress`) and +`cmd_chain_repair` async orchestrator |
| `bins/cli/src/main.rs` | +dispatch arm for `Commands::ChainRepair` |
| `docs/bugfixes/inc-i-034-m-choice3-chain-repair.md` | (this file) |
| `docs/.workflow/milestone-progress.md` | M-Choice3 row: `PENDING` → `COMPLETE (local, pending commit)` |

**No tests added** — the Test Writer's `mod repair_chain_tests` block in
`bins/cli/src/cmd_chain.rs` (15 unit tests) was already in place. This report
documents the Developer's response to the FAIL baseline.

**No server-side code changed.** The subcommand re-uses existing RPC surface:
- `verifyChainIntegrity` — already shipped
- `backfillFromPeer` — already shipped (since 2026-04-15, see `crates/rpc/src/methods/backfill.rs`)
- `backfillStatus` — already shipped

---

## What was added

### 1. Pure helpers (unit-tested)

Four pure helpers live in `bins/cli/src/cmd_chain.rs` above the test module:

```rust
pub(crate) fn validate_peer_url(peer: &str, local_endpoint: &str) -> Result<(), String>
pub(crate) fn format_gap_summary(integrity: &ChainIntegrity) -> String
pub(crate) fn format_progress(phase: &BackfillPhase) -> String

pub(crate) enum BackfillPhase {
    Running { imported: u64, total: u64, pct: u64 },
    Complete { imported: u64 },
    Failed(String),
}
impl BackfillPhase {
    pub(crate) fn from_status(s: &BackfillStatusResponse) -> Self
}
```

All four are side-effect-free (no I/O, no globals, no randomness), making them
fully testable without a tokio runtime or live RPC. All 15 `repair_chain_tests`
assertions pin the behavior contract.

### 2. RPC client additions

`bins/cli/src/rpc_client.rs` now exposes:

- `pub struct BackfillStatusResponse { running, imported, total, pct, error }`
  — mirrors the server-side type at `crates/rpc/src/types/chain.rs`
- `RpcClient::backfill_from_peer(&self, rpc_url: &str) -> Result<Value>`
  — returns the raw server JSON so the orchestrator can handle both the
  `{started: true, gaps, total}` and `{started: false, message}` shapes
- `RpcClient::backfill_status(&self) -> Result<BackfillStatusResponse>`

### 3. `doli chain-repair` subcommand

```
doli chain-repair --peer http://127.0.0.1:8500 [--yes]
    [--poll-interval-secs 5] [--max-wait-secs 3600]
```

Orchestrator flow (`cmd_chain_repair` in `cmd_chain.rs`):

1. **Pre-flight**: `validate_peer_url(peer, rpc_endpoint)` — rejects empty,
   peer IDs (MEMORY.md rule #1 trap), self-references, and missing-scheme URLs.
2. **Step 1 — verify**: call `verifyChainIntegrity` on the local node.
   Print gap summary via `format_gap_summary`. Early-return if `complete`.
3. **Confirm**: prompt y/N unless `--yes`.
4. **Step 2 — start**: `backfillFromPeer(peer)`. Parse `started` / `gaps` /
   `total`. Early-return on `started: false`.
5. **Step 3 — poll**: loop on `backfillStatus`, print one-line progress per
   cycle via `format_progress(BackfillPhase::from_status(status))`. Break on
   `Complete`, bail on `Failed` or `max_wait_secs` timeout.
6. **Step 4 — re-verify**: call `verifyChainIntegrity` again, print summary.
   If still incomplete, suggest trying another peer.

---

## Design notes

### Re-uses existing RPC, no new consensus surface

The server side already ships `backfillFromPeer` (with SSRF protection,
preflight tip-divergence check, and gap scanning), `backfillStatus` (progress
counter), and `verifyChainIntegrity` (complete-or-not + commitment). This
subcommand is a **thin orchestration layer** over those existing RPCs. No
protocol version bump required.

### Separation of pure and side-effecting code

The four pure helpers contain all the logic that could drift (URL validation,
phase classification, summary formatting). They have 15 tests covering every
cell in the Output × Path matrix per CLAUDE.md Global Rule #21. The async
orchestrator is thin glue: print, prompt, poll, break. This keeps the testable
surface maximal and the async surface minimal.

### Rejection categories (defense-in-depth for MEMORY.md rule #1)

`validate_peer_url` returns **distinct error classes** for the four common
user mistakes:

| Input | Error class |
|---|---|
| `""` | `"peer RPC URL is required (empty string not allowed)"` |
| `12D3KooW...` (libp2p peer ID) | explicit "peer id" / "rpc url" distinction — hard-blocks the classic MEMORY.md rule #1 trap |
| `http://127.0.0.1:8500` (== local) | "cannot backfill from self" |
| `http://127.0.0.1:8500/` (trailing slash) | same (slash-normalized) |
| `127.0.0.1:8500` (no scheme) | "missing a scheme — use http:// or https://" |

The peer-ID check runs BEFORE the scheme check so users pasting a peer ID get
the more helpful error instead of the generic "missing scheme".

### Self-detection is slash-normalized but host-loose

`peer.trim_end_matches('/') == local_endpoint.trim_end_matches('/')` covers
the two cases asserted by the tests. A fancier DNS-resolving check would
false-positive on operators running a local seed + N producers on the same
box, where each producer is a legitimate backfill target for its neighbors.

---

## Test evidence

### FAIL baseline (before implementation)

```
$ cargo test -p doli-cli repair_chain_tests --no-run
... 22 compile errors ...
error[E0432]: unresolved import `crate::rpc_client::BackfillStatusResponse`
error[E0425]: cannot find function `validate_peer_url` in this scope  (x6)
error[E0425]: cannot find function `format_gap_summary` in this scope (x3)
error[E0433]: failed to resolve: use of undeclared type `BackfillPhase` (x8)
error[E0425]: cannot find function `format_progress` in this scope    (x3)
error: could not compile `doli-cli` (bin "doli" test) due to 22 previous errors
```

### PASS state (after implementation)

```
$ cargo test -p doli-cli repair_chain_tests
running 15 tests
test cmd_chain::repair_chain_tests::test_backfill_phase_from_status_complete ... ok
test cmd_chain::repair_chain_tests::test_backfill_phase_from_status_running ... ok
test cmd_chain::repair_chain_tests::test_backfill_phase_from_status_failed ... ok
test cmd_chain::repair_chain_tests::test_format_gap_summary_complete_chain ... ok
test cmd_chain::repair_chain_tests::test_format_progress_complete ... ok
test cmd_chain::repair_chain_tests::test_format_progress_failed ... ok
test cmd_chain::repair_chain_tests::test_format_gap_summary_truncates_after_five_ranges ... ok
test cmd_chain::repair_chain_tests::test_format_progress_running ... ok
test cmd_chain::repair_chain_tests::test_format_gap_summary_small_gap_list ... ok
test cmd_chain::repair_chain_tests::test_validate_peer_url_accepts_remote_host ... ok
test cmd_chain::repair_chain_tests::test_validate_peer_url_rejects_empty_string ... ok
test cmd_chain::repair_chain_tests::test_validate_peer_url_rejects_exact_self ... ok
test cmd_chain::repair_chain_tests::test_validate_peer_url_rejects_libp2p_peer_id ... ok
test cmd_chain::repair_chain_tests::test_validate_peer_url_rejects_missing_scheme ... ok
test cmd_chain::repair_chain_tests::test_validate_peer_url_rejects_self_after_trailing_slash_strip ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured
```

### Regression check

```
$ cargo test -p doli-cli
test result: ok. 56 passed; 0 failed; 0 ignored; 0 measured
```

All pre-existing tests (including 10 `wipe_tests`) still pass.

### Build gates

```
$ cargo build --release -p doli-cli        # OK
$ cargo build --release --workspace        # OK (1m 44s)
$ cargo clippy -p doli-cli -- -D warnings  # OK (no warnings)
$ cargo fmt --check -p doli-cli            # OK
```

---

## Why this unblocks M-Choice1

When the M-Choice1 `HardForkSchedule::EPOCH_SNAPSHOT_HF` activates, the
runtime will enforce block_store completeness at block-accept time — nodes
missing historical blocks will hit `HALT_PRODUCTION`. Today, known-gap nodes
are `santiago`, `ivan`, and `seed3`. They need a pre-activation path to heal.

`doli chain-repair --peer http://<canonical-seed>:8500` gives operators exactly
that path:

1. Verify they have gaps.
2. Backfill from a canonical peer.
3. Re-verify they are now complete.

Without this command, the operator workflow today requires a manual
`curl`-based JSON-RPC call against `backfillFromPeer` plus a polling loop —
easy to get wrong, and a footgun in a high-pressure pre-activation window.

---

## Protocol / version notes

- **No `CURRENT_PROTOCOL_VERSION` bump**: this is a CLI-only additive change.
  The wire protocol, peer scoring, validation, and consensus surfaces are
  untouched.
- **No `HardForkSchedule` entry**: this ships before M-Choice1 specifically so
  operators can heal before M-Choice1 enforcement kicks in.
- **No mainnet deploy required for this milestone**: the CLI binary is
  operator tooling; deploy alongside the next scheduled `doli` release.

---

## Deployment checklist (per CLAUDE.md "After Every Modification")

1. **Build gate**: `cargo build --release && cargo clippy -- -D warnings && cargo fmt --check`
   — confirmed passing for `-p doli-cli` and `--workspace`.
2. **Test**: `cargo test -p doli-cli` — all 56 tests green.
3. **Version protection**: NOT required (CLI additive, no consensus change).
4. **Documentation alignment**:
   - `docs/cli.md` — SHOULD add a `chain-repair` entry (QA handoff item).
   - `docs/rpc_reference.md` — NOT required (no new RPC endpoints, re-uses
     `backfillFromPeer` / `backfillStatus` / `verifyChainIntegrity`).
   - `docs/troubleshooting.md` — SHOULD add "healing a gap-holding node"
     operator recipe using `chain-repair` (QA handoff item).
   - `specs/protocol.md` — NOT required.
   - `CLAUDE.md` code map — NOT required (no new files, all changes in
     existing `cmd_chain.rs` / `commands.rs` / `main.rs` / `rpc_client.rs`).
5. **Copy binary**: `cp target/release/doli ~/testnet/bin/` + codesign
   (on macOS). NOT mandatory unless testnet smoke test is planned.
6. **Commit and push**: DEFERRED per orchestrator instructions; changes left
   staged/unstaged for orchestrator review.
7. **Deploy consideration**: testnet first. NEVER mainnet without explicit
   confirmation.

---

## Handoff notes for QA

1. **Formatting applied to test module**: `rustfmt` reformatted whitespace
   inside `mod repair_chain_tests` (7 assertions wrapped across multiple
   lines). No assertion logic, substrings, or test names changed. Diff is
   whitespace-only and required to pass `cargo fmt --check`. QA can confirm
   by comparing pre- and post-fmt test assertions — they are byte-identical
   semantically.
2. **Docs not yet updated**: `docs/cli.md` and `docs/troubleshooting.md`
   should receive entries for `chain-repair`. Out of scope per the task's
   "DO NOT touch mainnet code paths outside the 3 specified files" rule.
3. **No live RPC smoke test was run**: the task specified that integration
   tests over live RPC are out of scope. A QA smoke test against a local
   devnet seed would be valuable before testnet deploy: start a node, wipe
   blocks mid-chain, run `doli chain-repair --peer http://127.0.0.1:8500`,
   confirm the chain heals.
4. **Interactive prompt**: the `--yes` flag bypasses the confirmation prompt;
   without it, the command blocks on stdin. QA should test both paths.
5. **Timeout semantics**: `--max-wait-secs` bails with a clear error message
   AND preserves whatever backfill state the server already made. A second
   invocation (possibly after extending the timeout) will report the
   remaining gaps.
