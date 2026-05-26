<!-- OUTPUT CONTRACT: N/A — architecture specification, not a test file -->
<!-- INPUT PARTITIONS: N/A — architecture specification, not a test file -->

# Covenant Guard CLI Parity + Template SDK -- Architecture

> Workflow: omega-new-feature RUN_ID=356
> Source: specs/sdk-templates-requirements.md, 3 parallel perspective analyses
> Scope: CLI-only. Zero consensus, validation, or node changes.

## Three-Question Consensus-Shape Answers

1. **Can any user-submittable transaction trigger this code path?** NO -- all changes are in `bins/cli/`. The node already validates and evaluates all 4 guard variants.
2. **Can any producer-action or attestation pattern trigger it?** NO -- no changes to block production, attestation, coinbase, or consensus-visible computation.
3. **Is the new behavior bit-identical to the old behavior for ALL reachable inputs?** N/A -- CLI-only change. On-chain semantics unchanged. CLI gains ability to construct transactions it previously could not, using existing `Condition` AST and `Output` types.

## Design Space Analysis

**Existing patterns identified:** 6 (parser flat match, cmd_producer subcommand dispatch, cmd_nft flags dispatch, channels/conditions.rs pure-function templates, conditions/tests.rs sibling test file, NETWORK global for CLI-side checks). **Tech stack constraints:** Clap 4.x derive API; Rust module system; 500-line module budget (OMEGA). **Performance requirements:** None on node side. Parser is O(depth x children), bounded by MAX_CONDITION_DEPTH=4, MAX_CONDITION_OPS=128. **Anti-overengineering gate:** PASS -- all changes extend existing modules or follow established patterns.

## Witness Verification (REQ-SDK-008)

Guard conditions (AmountGuard, OutputTypeGuard, RecipientGuard) evaluate against `ctx.transaction` outputs at `crates/core/src/conditions/eval.rs:121-165`. They consume NO fields from the `Witness` struct (`witness.rs:16-24`). `Threshold` evaluates sub-conditions recursively; existing Witness fields (`signatures`, `preimage`, `or_branches`) suffice for sub-conditions that need them. No changes to `parse_witness()` or `Witness` encoding are needed. Guards with `witness="none()"` will evaluate based solely on the spending transaction's output structure.

## Module Structure

### M1. Parser arms -- `bins/cli/src/parsers.rs`

**Decision: P1 (inline in existing match arms).** conf(0.85, observed)
- Rejected P2 (helper function): 3 small arms do not justify indirection.
- Rejected P3 (trait registry): massive overengineering for 10 total arms.

**CRITICAL IMPLEMENTATION NOTE (from Skeptic S1):** `threshold` MUST be added to the **top-level** `parse_condition()` match at line 38, alongside `and`/`or`, because it uses `split_top_level()` for its sub-conditions. If placed in `parse_simple_condition()` (the natural-looking but WRONG place), input like `threshold(2, and(sig(x), timelock(100)), hashlock(abc))` will be flat-split by `args_str.split(',')` at line 59, producing mangled fragments instead of proper sub-condition strings. The three guard arms (`amount_guard`, `output_type_guard`, `recipient_guard`) go in `parse_simple_condition()` because they use flat comma-separated args with no nesting.

**Placement summary:**

| Arm | Function | Why |
|-----|----------|-----|
| `threshold(n, cond1, cond2, ...)` | `parse_condition()` line 38 match | Uses `split_top_level()` for nested sub-conditions |
| `amount_guard(amount, index)` | `parse_simple_condition()` line 90 match | Flat args, no nesting |
| `output_type_guard(type, index)` | `parse_simple_condition()` line 90 match | Flat args, no nesting |
| `recipient_guard(addr, index)` | `parse_simple_condition()` line 90 match | Flat args, no nesting |

Post-expansion: parsers.rs grows from 281 to ~350 lines (under 500-line budget).

### M2. Mainnet warning -- `bins/cli/src/cmd_wallet.rs`

**Decision: W1 (inline in cmd_send after parse_condition).** conf(0.80, observed)
- Rejected W2 (inside parse_condition): mixes parsing with I/O, makes parse_condition impure.
- Rejected W3 (standalone helper): +1 function for a 3-line check; caller must remember to call it.
- Rejected W4 (return tuple): changes parse_condition signature for a single warning.

**Location:** `cmd_wallet.rs`, after line 479 (`let cond = parse_condition(cond_str)?;`), before `Output::conditioned()`. Check `NETWORK.get()` value equals `"mainnet"` (or default) AND `cond.contains_guard()`. Emit warning to stderr. Do not block.

**Precedent:** `cmd_producer/delegation.rs:117-120` -- CLI produces unconditionally, warns if on mainnet, lets node validate. Exact same pattern.

### M3. Multi-output spend -- `bins/cli/src/cmd_wallet.rs` + `bins/cli/src/commands.rs`

**CLI shape decision: M1 (colon-delimited `--output`).** conf(0.80, observed)
- Rejected M2 (repeated flag groups): 4 flags per output, fragile zip of parallel Vecs.
- Rejected M3 (no explicit index): guards reference output_index by number; implicit ordering is a footgun.
- Rejected M4 (JSON): terrible CLI UX, shell quoting hell.

**Backward compat decision: B3 (Option + runtime validation), combined with B1 branch.** conf(0.80, inferred)
- `to` and `amount` become `Option<String>` in the `Spend` clap struct.
- Rejected: clap `required_unless_present` on positionals (S2 attack: fragile with positional args).
- Runtime validation: if `--output` present AND `to`/`amount` present, error. If neither `--output` nor `to`/`amount`, error.
- When `--output` absent: existing single-output path executes unchanged (B1 branch).
- When `--output` present: parse each `index:type:recipient:amount` string, validate contiguity/caps, build output vec.
- Use `splitn(4, ':')` for defensive parsing (S7 mitigation).

**Output type whitelist:** `normal`, `multisig`, `hashlock`, `htlc`, `vesting`, `nft`. NFT included per S4 finding (`cmd_nft/sell.rs:348` proves users construct NFT outputs). Protocol-internal types (`bond`, `pool`, `lpshare`, `collateral`, etc.) rejected.

**Fee overpayment warning (S3 mitigation):** After building the output vec, compute `fee = total_input - total_output`. If `fee > max(total_input / 100, 10_000)` (1% of input or 10000 units, whichever larger), print WARNING to stderr: `"WARNING: Computed fee is {fee} units ({fee_doli} DOLI), which is unusually high. Verify output amounts cover the full input. Use --yes to proceed anyway."` Require `--yes` flag or interactive confirmation to continue. CLI-only safety net, not a consensus change.

### M4. Tests

**Parser unit tests (REQ-SDK-009):** `#[cfg(test)] mod tests` at bottom of `parsers.rs`. At ~350 lines of source + ~200 lines of tests = ~550 lines, slightly over budget. **Decision: tolerate the overrun** because extracting to a sibling file for ~50 lines over the soft limit adds more complexity than it saves. If the test module grows beyond 250 lines during implementation, extract to `parsers_tests.rs` using `#[cfg(test)] #[path = "parsers_tests.rs"] mod tests;`. conf(0.75, inferred)

**E2E tests (REQ-SDK-010):** Place in `bins/node/tests/sdk_guard_e2e.rs`. `Node::new_for_test()` is the only way to get a validation pipeline without a live node, and it lives in `bins/node/`. 23 existing integration tests already use this pattern. Cannot access from `bins/cli/tests/` without cross-crate hacks. conf(0.85, observed)

### M5. Part B template functions (REQ-SDK-012, gated)

**Decision: `crates/core/src/conditions/templates.rs`** conf(0.80, observed)
- Rejected T1 (new `crates/sdk/` crate): 5 functions do not justify a new crate (speculative generality anti-pattern). Zero external consumers exist.
- Rejected T2 (inline in CLI): templates not reusable outside CLI. The channels crate precedent shows library placement works.
- Rejected T3 (docs-only): no validation, no type safety, examples drift.

Templates mirror `crates/channels/src/conditions.rs` exactly: pure functions `(params) -> Condition`, ~10-15 lines each, doc comments explaining both locking and spending scenarios. Each template documents the required witness format.

### M6. Part B CLI subcommand (REQ-SDK-013, gated)

**Decision: `cmd_producer/` Pattern 1 (Subcommand enum + dispatch module).** conf(0.80, observed)
- Rejected cmd_nft Pattern 2 (flags dispatch): anti-pattern for distinct sub-commands with different flag sets.

File structure:
```
bins/cli/src/cmd_template/
  mod.rs          -- module declarations + re-export cmd_template
  dispatch.rs     -- match TemplateCommands variant to handler
  vault.rs        -- handle_vault()
  escrow.rs       -- handle_escrow()
  htlc.rs         -- handle_htlc()
  subscription.rs -- handle_subscription()
  allowance.rs    -- handle_agent_allowance()
```

## Data Flow

### Guard-conditioned UTXO creation (send)
```
User: doli send --condition "and(amount_guard(500.0, 0), recipient_guard(addr, 0))" ...
  |
  v
parse_condition() -> Condition::And(AmountGuard{..}, RecipientGuard{..})
  |
  v  [if NETWORK=="mainnet" && cond.contains_guard() -> stderr warning]
  v
condition_to_output_type(&cond) -> OutputType::Vesting  (known lossy mapping, accepted)
  |
  v
Output::conditioned(Vesting, amount, pkh, &cond)  [calls cond.validate() internally]
  |
  v
Transaction::new_transfer(inputs, [conditioned_output, change_output])
  |
  v
RPC send_transaction() -> node mempool -> validation -> block
```

### Guard-conditioned UTXO spending (spend)
```
User: doli spend <utxo> --output 0:normal:alice:500.0 --output 1:normal:self:499.99999 --witness "sign(wallet.json)"
  |
  v
Parse --output specs: split each by splitn(4, ':'), validate contiguity/caps/types
  |
  v
Build Vec<Output> at specified indices with specified types/recipients/amounts
  |
  v  [fee = total_input - total_output; if fee > threshold -> stderr warning]
  v
Transaction::new_transfer([input], outputs)
  |
  v
parse_witness(witness_str, signing_hash) -> witness_bytes
  |
  v
tx.set_covenant_witnesses(&[witness_bytes])  [applied to input 0]
  |
  v
RPC send_transaction() -> node evaluates guard conditions against tx.outputs[]
```

### Invariants
- `Condition` AST is immutable; CLI only constructs instances.
- `condition_to_output_type()` is display-only, not consensus. Guard conditions map to `Multisig` (known limitation).
- `encode()` calls `validate()` before serializing; depth/ops errors surface at CLI time.
- Guards are disabled on mainnet (`guards_activation_height = u64::MAX`). Enabled on devnet/testnet (height 0).
- Guard evaluation inspects the **spending** transaction's outputs. Locking side only encodes the condition.

## Failure Modes and Mitigations

| ID | Attack | Severity | Mitigation |
|----|--------|----------|------------|
| S1 | Threshold placed in `parse_simple_condition` mangles nested sub-conditions | Critical | Mandated placement in top-level `parse_condition()` match alongside `and`/`or`. Documented in Module Structure M1 above. Parser tests verify nested threshold parsing (REQ-SDK-001 output contract). |
| S2 | Clap conditional positional args fragile | High | `to`/`amount` become `Option<String>` with runtime validation in `cmd_spend`. No clap-level conditional requirements on positionals. |
| S3 | Multi-output spend without change = fee overpayment | High | CLI-side fee reasonableness warning when fee > max(1% of input, 10000 units). Requires `--yes` or interactive confirmation to proceed. |
| S4 | Output type whitelist excludes NFT | Medium | NFT added to whitelist. `cmd_nft/sell.rs:348` proves NFT is user-constructible. |
| S5 | E2E test needs `Node::new_for_test()` from `bins/node/` | Medium | E2E tests placed in `bins/node/tests/sdk_guard_e2e.rs` where infrastructure exists. |
| S6 | `condition_to_output_type` maps guards to Multisig | Low | Accepted as known limitation. Code comment added explaining the lossy mapping. |
| S7 | `--output` parser could over-split on `:` | Low | Use `splitn(4, ':')` for bounded parsing. |
| S8 | MAX_CONDITION_DEPTH=4 tight for template composition | Low | CLI docs note depth budget. Templates individually fit within depth 2-3, leaving room for one layer of user composition. |

## Security

- **Mainnet guard rejection:** CLI warns; does not block. Node enforces rejection at validation (`guards_activation_height = u64::MAX`). Trust boundary is the node, not the CLI.
- **Locked funds from parser bugs:** Mitigated by exhaustive parser tests (REQ-SDK-009: 40+ tests covering all input partitions). Round-trip parse-encode-decode tests verify field ordering.
- **Fee overpayment:** CLI warning when computed fee exceeds threshold. No change to consensus fee validation.
- **Parameter swap (amount/index):** Parser tests verify each field independently. Round-trip tests ensure encoded condition matches parsed input.

## Performance Budget

- **Parser:** O(depth x children), bounded by MAX_CONDITION_DEPTH=4, MAX_CONDITION_OPS=128. Worst case: threshold(5, [threshold(5, ...)]) = 25 recursive calls. Negligible.
- **Multi-output spend:** O(N) where N <= 8 outputs. Negligible.
- **No node-side changes:** Zero performance impact on validation, block processing, or consensus.

## Milestones

| ID | Name | Scope (Modules) | Scope (Requirements) | Est. Size | Dependencies |
|----|------|-----------------|---------------------|-----------|-------------|
| A1 | Parser arms + unit tests | parsers.rs | REQ-SDK-001 to REQ-SDK-005, REQ-SDK-009 | M | None |
| A2 | Mainnet warning | cmd_wallet.rs, common.rs | REQ-SDK-006 | S | None |
| A3 | Multi-output spend | cmd_wallet.rs, commands.rs | REQ-SDK-007 | M | None |
| A4 | E2E test + witness doc | bins/node/tests/ | REQ-SDK-008, REQ-SDK-010 | M | A1, A2, A3 |
| A5 | Docs + SPECS index | docs/cli.md | REQ-SDK-014 | S | A1, A2, A3 |
| -- | **Re-gate (REQ-SDK-011)** | process only | REQ-SDK-011 | -- | A1-A5 |
| B1 | Template functions in core | conditions/templates.rs | REQ-SDK-012 | S | Re-gate |
| B2 | CLI template subcommand | cmd_template/ | REQ-SDK-013 | M | B1 |

### Milestone A1: Parser Arms + Unit Tests

**Scope:** REQ-SDK-001 (threshold), REQ-SDK-002 (amount_guard), REQ-SDK-003 (output_type_guard), REQ-SDK-004 (recipient_guard), REQ-SDK-005 (composition verification), REQ-SDK-009 (parser unit tests)

**Files touched:**
- `bins/cli/src/parsers.rs` -- 4 new match arms + `#[cfg(test)] mod tests`

**Test files produced:**
- `bins/cli/src/parsers.rs` (inline `#[cfg(test)] mod tests`) -- 40+ test functions

**Acceptance criteria:**
- All 10 output contract rows from REQ-SDK-001 pass (threshold parsing)
- All 7 output contract rows from REQ-SDK-002 pass (amount_guard parsing)
- All 6 output contract rows from REQ-SDK-003 pass (output_type_guard parsing)
- All 5 output contract rows from REQ-SDK-004 pass (recipient_guard parsing)
- All 5 output contract rows from REQ-SDK-005 pass (guard composition)
- `condition_to_output_type()` tested for all Condition variants
- `split_top_level()` tested for 5 edge cases
- Minimum 40 test functions in `#[cfg(test)] mod tests`
- `cargo test -p doli-cli --lib` passes
- `cargo clippy -- -D warnings` passes

**Estimated LOC delta:** +250 lines (50 source + 200 tests)

### Milestone A2: Mainnet Warning

**Scope:** REQ-SDK-006

**Files touched:**
- `bins/cli/src/cmd_wallet.rs` -- 5 lines after `parse_condition()` call in `cmd_send`

**Test files produced:**
- None required (warning is stderr output; acceptance tested manually or in A4 E2E)

**Acceptance criteria:**
- Mainnet + guard condition -> stderr warning printed, tx still broadcast
- Mainnet + non-guard condition -> no warning
- Devnet/testnet + guard condition -> no warning
- Uses `Condition::contains_guard()` from `crates/core/` (no reimplementation)

**Estimated LOC delta:** +5 lines

### Milestone A3: Multi-Output Spend

**Scope:** REQ-SDK-007

**Files touched:**
- `bins/cli/src/commands.rs` -- modify Spend struct: `to`/`amount` -> `Option<String>`, add `--output Vec<String>`, add `--yes bool`
- `bins/cli/src/cmd_wallet.rs` -- `cmd_spend()` signature change, branch for multi-output path, fee warning
- `bins/cli/src/main.rs` -- update `cmd_spend()` call site for new signature

**Test files produced:**
- Inline unit tests for output-spec parsing in `cmd_wallet.rs` (5-10 tests) if space permits; otherwise tested via A4 E2E

**Acceptance criteria:**
- All 11 output contract rows from REQ-SDK-007 pass
- Backward compat: `doli spend <utxo> <to> <amount> --witness ...` works unchanged
- Multi-output: `doli spend <utxo> --output 0:normal:addr:500.0 --output 1:normal:addr2:200.0 --witness ...` works
- Conflict: `--output` with positional `to`/`amount` produces error
- Fee warning when computed fee > max(1% of input, 10000 units)
- Output summary printed before broadcast
- `splitn(4, ':')` used for parsing

**Estimated LOC delta:** +120 lines

### Milestone A4: E2E Test + Witness Verification

**Scope:** REQ-SDK-008 (witness doc -- satisfied by this architecture doc above), REQ-SDK-010 (E2E test)

**Files touched:**
- `bins/node/tests/sdk_guard_e2e.rs` -- new integration test file

**Test files produced:**
- `bins/node/tests/sdk_guard_e2e.rs` -- 4 test functions (positive + 3 negative)

**Acceptance criteria:**
- Test creates UTXO with `and(amount_guard(100.0, 0), recipient_guard(addr, 0))` condition
- Test spends UTXO with matching outputs -> accepted
- Negative: wrong recipient at index 0 -> rejected
- Negative: insufficient amount at index 0 -> rejected
- Negative: wrong output index -> rejected
- Uses `Node::new_for_test()` with devnet config (guards_activation_height = 0)
- `cargo test -p doli-node --test sdk_guard_e2e` passes

**Estimated LOC delta:** +150 lines

### Milestone A5: Documentation

**Scope:** REQ-SDK-014

**Files touched:**
- `docs/cli.md` -- sections 2.2 and 2.3 updated

**Test files produced:** None

**Acceptance criteria:**
- `docs/cli.md` section 2.2 lists all 4 new condition types with examples
- `docs/cli.md` section 2.3 documents `--output` flag format
- `docs/cli.md` documents mainnet guard warning
- `docs/cli.md` includes depth budget note
- Three-question consensus-shape answers in commit message

**Estimated LOC delta:** +60 lines in docs

### Re-gate Checkpoint (REQ-SDK-011)

After A1-A5 merge and all tests pass, architect produces a 1-page comparison:
- Option A: `crates/core/src/conditions/templates.rs` with 5 pure functions + `bins/cli/src/cmd_template/` subcommand
- Option B: `docs/covenant-recipes.md` with copy-paste CLI examples

User decides. Decision recorded in `docs/.workflow/`.

### Milestone B1: Template Functions in Core (gated)

**Scope:** REQ-SDK-012

**Files touched:**
- `crates/core/src/conditions/templates.rs` -- new file, 5 functions
- `crates/core/src/conditions/mod.rs` -- add `pub mod templates;`

**Test files produced:**
- `crates/core/src/conditions/templates.rs` (inline tests) -- 10+ tests

**Acceptance criteria:**
- 5 templates: `vault`, `escrow`, `htlc_payment`, `subscription`, `agent_allowance`
- Each is a pure function returning `Condition` (not `Result`)
- Each has doc comment explaining locking condition AND required spending witness
- Each has >= 2 unit tests (structure + round-trip encode/decode)
- All templates fit within MAX_CONDITION_DEPTH=4 (verified by tests)
- Mirrors `crates/channels/src/conditions.rs` pattern

**Estimated LOC delta:** +150 lines

### Milestone B2: CLI Template Subcommand (gated)

**Scope:** REQ-SDK-013

**Files touched:**
- `bins/cli/src/cmd_template/mod.rs` -- module declarations
- `bins/cli/src/cmd_template/dispatch.rs` -- match routing
- `bins/cli/src/cmd_template/vault.rs` -- vault handler
- `bins/cli/src/cmd_template/escrow.rs` -- escrow handler
- `bins/cli/src/cmd_template/htlc.rs` -- htlc handler
- `bins/cli/src/cmd_template/subscription.rs` -- subscription handler
- `bins/cli/src/cmd_template/allowance.rs` -- allowance handler
- `bins/cli/src/commands.rs` -- add `TemplateCommands` enum + `Commands::Template` variant
- `bins/cli/src/main.rs` -- add `Commands::Template` dispatch arm

**Test files produced:**
- Inline tests in each handler file (clap parse + output string verification)

**Acceptance criteria:**
- `doli template vault --owner <addr> --cosigner <addr> --delay <blocks>` prints condition string
- All 5 templates have sub-commands with named flags
- `--dry-run` (default) prints condition string for use with `doli send`
- `--send --wallet <path> --to <addr> --amount <amt>` constructs and broadcasts
- Mainnet warning applies when guard-containing templates used on mainnet
- `doli template --help` lists templates with one-line descriptions

**Estimated LOC delta:** +350 lines

## Traceability Matrix

| Requirement ID | Architecture Section | Files | Tests | Priority |
|---------------|---------------------|-------|-------|----------|
| REQ-SDK-001 | M1 (threshold in parse_condition) | parsers.rs | TEST-SDK-001 (A1 inline) | Must |
| REQ-SDK-002 | M1 (amount_guard in parse_simple_condition) | parsers.rs | TEST-SDK-002 (A1 inline) | Must |
| REQ-SDK-003 | M1 (output_type_guard in parse_simple_condition) | parsers.rs | TEST-SDK-003 (A1 inline) | Must |
| REQ-SDK-004 | M1 (recipient_guard in parse_simple_condition) | parsers.rs | TEST-SDK-004 (A1 inline) | Must |
| REQ-SDK-005 | M1 (verify recursive parse) | parsers.rs | TEST-SDK-005 (A1 inline) | Must |
| REQ-SDK-006 | M2 (inline in cmd_send) | cmd_wallet.rs | TEST-SDK-006 (A4 E2E) | Must |
| REQ-SDK-007 | M3 (colon-delimited --output, B3 compat) | cmd_wallet.rs, commands.rs | TEST-SDK-007 (A3 inline + A4 E2E) | Must |
| REQ-SDK-008 | Witness Verification section | (documentation artifact) | TEST-SDK-008 (A4 E2E) | Must |
| REQ-SDK-009 | M4 (inline cfg test) | parsers.rs | TEST-SDK-009 (A1 inline) | Must |
| REQ-SDK-010 | M4 (bins/node/tests/) | sdk_guard_e2e.rs | TEST-SDK-010 (A4) | Must |
| REQ-SDK-011 | Re-gate checkpoint | (process gate) | N/A | Should |
| REQ-SDK-012 | M5 (core/conditions/templates.rs) | templates.rs | TEST-SDK-012 (B1 inline) | Should |
| REQ-SDK-013 | M6 (cmd_template/ subdir) | cmd_template/*.rs | TEST-SDK-013 (B2 inline) | Should |
| REQ-SDK-014 | Milestone A5 | docs/cli.md | N/A | Must |

## Known Limitations (accepted, do not fix)

1. **`condition_to_output_type` lossy mapping:** Guards map to `OutputType::Multisig`, `And(guard, guard)` maps to `Vesting`, `Or(guard, x)` maps to `HTLC`. Display-only; validation reads `extra_data`. Fixing requires `OutputType::Guard` (consensus change) or fragile pattern matching.
2. **Guards disabled on mainnet:** `guards_activation_height = u64::MAX`. All guard tooling is devnet/testnet until a separate consensus activation workflow.
3. **Single-input spend:** `cmd_spend` takes one UTXO reference. Multi-input spend (two guard UTXOs) is unsupported.
4. **No auto-change:** Multi-output spend requires user to manually account for full input amount. Fee overpayment warning mitigates but does not prevent.
5. **MAX_CONDITION_DEPTH=4:** Templates fit individually (depth 2-3). One layer of user composition around a template is possible. Two layers hit the ceiling.
