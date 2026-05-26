<!-- OUTPUT CONTRACT: N/A — requirements specification, not a test file. Per-requirement output contract tables are included below. -->
<!-- INPUT PARTITIONS: documented inline per requirement in the Output Contract tables below. -->

# Covenant Guard CLI Parity + Template SDK — Requirements

> Workflow: omega-new-feature RUN_ID=356
> Source: docs/.workflow/{prompt-refinement,feature-evaluation,skeptic-analysis}.md
> Scope: Devnet/testnet-only ergonomics. NO consensus changes.

## Scope Summary

Expose 4 fully-implemented but CLI-unreachable `Condition` variants (Threshold, AmountGuard, OutputTypeGuard, RecipientGuard) through the CLI parser, upgrade the `spend` command to support multi-output transactions (required for guard satisfaction), add mainnet guard-disabled warning, and write parser tests covering all variants. Part B (template SDK) is gated behind a re-evaluation checkpoint after Part A lands. No consensus rules, activation heights, wire formats, or validation logic change. All work is additive to `bins/cli/`.

## Architecture Comprehension

### Data Flow

```
User CLI input
  ↓
parse_condition() → Condition AST        [bins/cli/src/parsers.rs:20-63]
  ↓
condition_to_output_type() → OutputType   [bins/cli/src/parsers.rs:183-205]
  ↓
Output::conditioned(type, amount, pkh, &cond) → Output  [crates/core/src/transaction/output.rs:224-238]
  ↓
Transaction::new_transfer(inputs, outputs) → Transaction  [cmd_wallet.rs:599]
  ↓
set_covenant_witnesses() → serialized     [cmd_wallet.rs:606]
  ↓
RPC send_transaction() → node mempool
  ↓
Node validation: validate_transaction()   [crates/core/src/validation/transaction.rs]
  ↓  (guard activation gate at line 357-365: checks guards_activation_height)
Condition evaluation: evaluate()          [crates/core/src/conditions/eval.rs]
  ↓  (guards inspect ctx.transaction.outputs[output_index])
Block inclusion → UTXO set
```

**Invariants:**
- `Condition` AST is immutable (defined in `crates/core/src/conditions/mod.rs:106-152`). CLI only constructs instances — never alters the enum.
- `condition_to_output_type()` is a display/indexing mapping, NOT a validation gate. Validation reads `extra_data`, not `output_type`. A mismatch is cosmetic, not consensus-breaking.
- Guards are disabled on mainnet (`guards_activation_height() = u64::MAX`, `consensus/params.rs:163`). Enabled on devnet/testnet (height 0, lines 161-162).
- Guard evaluation (`eval.rs:121-165`) inspects the **spending** transaction's outputs at specific indices. The locking side only encodes the condition. The spending side must produce matching outputs.
- Witness struct (`witness.rs:16-24`) has 3 fields: `signatures`, `preimage`, `or_branches`. Guards need NO witness data — they evaluate against `ctx.transaction` directly. Threshold reuses witnesses from its sub-conditions.

### Blast Radius

- **`bins/cli/src/parsers.rs`** — primary target. Adding 4 match arms to `parse_simple_condition()`, plus a `threshold` arm in `parse_condition()`. Zero downstream consumers outside CLI.
- **`bins/cli/src/cmd_wallet.rs`** — `cmd_spend()` (lines 548-647) changes from single-output to multi-output builder. Existing single-output behavior must remain the default path.
- **`bins/cli/src/commands.rs`** — Clap argument definition for `Spend` (lines 101-122) needs new `--output` flag.
- **`bins/cli/src/common.rs`** — `NETWORK` global (line 8) read for mainnet warning. Read-only access.
- **`docs/cli.md`** — documentation update (sections 2.2, 2.3).
- **Zero blast radius on consensus/validation/node** — all changes are CLI-only. No `crates/core/`, `crates/storage/`, `bins/node/` modifications.

## Requirements

### Part A — Guard CLI Parity + Spend Upgrade (Must)

---

#### REQ-SDK-001 — Threshold parser arm

**Priority:** Must
**User story:** As a developer, I want to construct n-of-many threshold conditions over arbitrary sub-conditions so I can express complex spending policies like "any 2 of these 3 conditions."
**Likely files touched:** `bins/cli/src/parsers.rs`
**Test ID:** TEST-SDK-001

**Acceptance criteria:**

- [ ] `parse_condition("threshold(2, hashlock(aabb...cc), timelock(100), multisig(1, addr1))")` returns `Condition::Threshold { n: 2, conditions: [Hashlock(..), Timelock(100), Multisig { threshold: 1, keys: [addr1] }] }`
- [ ] `threshold(...)` uses `split_top_level` (not flat comma split) to correctly handle nested parentheses in sub-conditions
- [ ] `n` must parse as `u8`; values 0 or > number of sub-conditions produce a descriptive error
- [ ] Sub-condition count must be >= 2 and <= `MAX_THRESHOLD_CONDITIONS` (5). Out-of-range produces error referencing the limit.
- [ ] Recursion through `parse_condition()` for each sub-condition allows nesting
- [ ] `condition_to_output_type()` for Threshold already returns `OutputType::Multisig` (line 200) — no change needed, verified by test

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| Valid: n=2, 3 simple sub-conditions | happy path | `Condition::Threshold { n: 2, conditions: [..3] }` |
| Valid: n=1, 2 sub-conditions (minimum) | happy path | `Condition::Threshold { n: 1, conditions: [..2] }` |
| Valid: nested — threshold containing and/or | recursive parse | `Condition::Threshold { n: .., conditions: [And(..), Or(..)] }` |
| Invalid: n=0 | validation error | `Err("threshold n must be >= 1")` |
| Invalid: n > len(conditions) | validation error | `Err("threshold n (3) exceeds condition count (2)")` |
| Invalid: only 1 sub-condition | validation error | `Err("threshold requires at least 2 conditions")` |
| Invalid: 6+ sub-conditions | validation error | `Err("... exceeds MAX_THRESHOLD_CONDITIONS (5)")` |
| Invalid: n not a u8 | parse error | `Err("Invalid threshold: ...")` |
| Invalid: malformed sub-condition | recursive error propagation | `Err("Unknown condition: ...")` |

---

#### REQ-SDK-002 — AmountGuard parser arm

**Priority:** Must
**User story:** As a developer, I want to create a condition that enforces a minimum amount on a specific output of the spending transaction, so I can implement limit orders and MEV protection.
**Likely files touched:** `bins/cli/src/parsers.rs`
**Test ID:** TEST-SDK-002

**Acceptance criteria:**

- [ ] `parse_condition("amount_guard(500.0, 0)")` returns `Condition::AmountGuard { min_amount: 50000000000, output_index: 0 }`
- [ ] `min_amount` parses through `coins_to_units()` (DOLI decimal format) — NOT raw integer
- [ ] `output_index` parses as `u8`
- [ ] Exactly 2 arguments required; fewer or more produces descriptive error
- [ ] Amount of 0 is rejected (likely user error — trivially satisfied)
- [ ] `condition_to_output_type()` already returns `OutputType::Multisig` (line 201-203) — no change needed

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| Valid: "500.0, 0" | happy path | `AmountGuard { min_amount: 50_000_000_000, output_index: 0 }` |
| Valid: "0.00000001, 255" (1 unit, max index) | boundary | `AmountGuard { min_amount: 1, output_index: 255 }` |
| Invalid: "0, 0" (zero amount) | validation error | `Err("min_amount must be greater than zero")` |
| Invalid: "abc, 0" (non-numeric amount) | parse error | `Err("Invalid amount: ...")` |
| Invalid: "500.0" (missing output_index) | arity error | `Err("amount_guard requires 2 args: ...")` |
| Invalid: "500.0, 0, 1" (too many args) | arity error | `Err("amount_guard requires 2 args: ...")` |
| Invalid: "500.0, 256" (output_index overflow) | parse error | `Err("Invalid output_index: ...")` |

---

#### REQ-SDK-003 — OutputTypeGuard parser arm

**Priority:** Must
**User story:** As a developer, I want to lock a UTXO so the spending transaction must produce a specific output type at a given index, preventing fund redirection.
**Likely files touched:** `bins/cli/src/parsers.rs`
**Test ID:** TEST-SDK-003

**Acceptance criteria:**

- [ ] `parse_condition("output_type_guard(normal, 0)")` returns `Condition::OutputTypeGuard { expected_type: OutputType::Normal, output_index: 0 }`
- [ ] Type name parsing is case-insensitive
- [ ] All `OutputType` variants accepted by name: `normal`, `bond`, `multisig`, `hashlock`, `htlc`, `vesting`, `nft`, `fungibleasset`, `bridgehtlc`, `pool`, `lpshare`, `collateral`, `lendingdeposit`, `zkrollup`, `encryptedcontent`
- [ ] Unknown type name produces descriptive error listing valid names
- [ ] `output_index` parses as `u8`; exactly 2 arguments required

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| Valid: "normal, 0" | happy path | `OutputTypeGuard { expected_type: Normal, output_index: 0 }` |
| Valid: "HTLC, 1" (case insensitive) | happy path | `OutputTypeGuard { expected_type: HTLC, output_index: 1 }` |
| Valid: "vesting, 255" (max index) | boundary | `OutputTypeGuard { expected_type: Vesting, output_index: 255 }` |
| Invalid: "unknown_type, 0" | type parse error | `Err("Unknown output type 'unknown_type'. Valid: normal, bond, ...")` |
| Invalid: "normal" (missing index) | arity error | `Err("output_type_guard requires 2 args: ...")` |
| Invalid: "normal, abc" (non-numeric index) | parse error | `Err("Invalid output_index: ...")` |

---

#### REQ-SDK-004 — RecipientGuard parser arm

**Priority:** Must
**User story:** As a developer, I want to lock a UTXO so the spending transaction must pay a specific address at a given output index, enabling conditional payments and bounded delegation.
**Likely files touched:** `bins/cli/src/parsers.rs`
**Test ID:** TEST-SDK-004

**Acceptance criteria:**

- [ ] `parse_condition("recipient_guard(doli1abc..., 0)")` returns `Condition::RecipientGuard { expected_pubkey_hash: <resolved_hash>, output_index: 0 }`
- [ ] Address resolution uses existing `resolve_to_hash()` (line 171-180): bech32 or hex
- [ ] `output_index` parses as `u8`; exactly 2 arguments required
- [ ] Invalid address produces descriptive error from `resolve_to_hash()` propagation

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| Valid: bech32 address + index 0 | happy path | `RecipientGuard { expected_pubkey_hash: <hash>, output_index: 0 }` |
| Valid: hex hash + index 1 | happy path (hex) | `RecipientGuard { expected_pubkey_hash: <hash>, output_index: 1 }` |
| Invalid: "doli1abc..." (missing index) | arity error | `Err("recipient_guard requires 2 args: ...")` |
| Invalid: "not_an_address, 0" | address resolution error | `Err("Invalid address 'not_an_address': ...")` |
| Invalid: "doli1abc..., 0, extra" | arity error | `Err("recipient_guard requires 2 args: ...")` |

---

#### REQ-SDK-005 — Guard composition with and/or

**Priority:** Must
**User story:** As a developer, I want to compose guard conditions with `and()` and `or()` so I can build complex spending policies.
**Likely files touched:** `bins/cli/src/parsers.rs` (no changes — verify existing arms recursively invoke `parse_condition` which will resolve new guard arms)
**Test ID:** TEST-SDK-005

**Acceptance criteria:**

- [ ] `parse_condition("and(amount_guard(500.0, 0), recipient_guard(doli1abc..., 0))")` returns `Condition::And(Box<AmountGuard { .. }>, Box<RecipientGuard { .. }>)`
- [ ] `parse_condition("or(amount_guard(100.0, 0), timelock(1000))")` returns `Condition::Or(Box<AmountGuard { .. }>, Box<Timelock(1000)>)`
- [ ] Triple nesting works: `and(threshold(2, hashlock(...), timelock(100)), recipient_guard(addr, 0))`
- [ ] `condition_to_output_type()` for `And(AmountGuard, RecipientGuard)` returns `OutputType::Vesting` (known limitation — display-level only)

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| And(guard, guard) | recursive parse → And | `And(Box<AmountGuard>, Box<RecipientGuard>)` |
| Or(guard, timelock) | recursive parse → Or | `Or(Box<AmountGuard>, Box<Timelock>)` |
| And(threshold(...), guard) | deep recursion | `And(Box<Threshold { .. }>, Box<RecipientGuard { .. }>)` |
| And with 3 args | and arity check | `Err("and requires exactly 2 args")` |
| Or with 1 arg | or arity check | `Err("or requires exactly 2 args")` |

---

#### REQ-SDK-006 — Mainnet guard warning

**Priority:** Must
**User story:** As a user on mainnet, I want to be warned when I construct a guard condition that will be rejected by the network.
**Likely files touched:** `bins/cli/src/cmd_wallet.rs` (in `cmd_send`, around lines 488-543), `bins/cli/src/common.rs` (read `NETWORK` global)
**Test ID:** TEST-SDK-006

**Acceptance criteria:**

- [ ] When `NETWORK` global equals `"mainnet"` (or is unset, defaulting to mainnet) and the parsed `Condition` contains any guard variant (including Threshold containing guards), emit warning to stderr: `WARNING: Guard conditions are not yet activated on mainnet (guards_activation_height = MAX). This transaction WILL be rejected by mainnet nodes. Use --network devnet or --network testnet.`
- [ ] Warning emitted BEFORE broadcast, not after rejection
- [ ] Warning does NOT block — user can still broadcast
- [ ] On devnet/testnet, no warning
- [ ] Reuses `Condition::contains_guard()` from `crates/core/` — do not reimplement

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| Mainnet + condition with AmountGuard | warning path | stderr warning printed, tx still broadcast |
| Mainnet + condition without guards (e.g., multisig) | no-warning path | no warning, normal flow |
| Devnet + condition with guard | no-warning path | no warning, normal flow |
| Testnet + condition with guard | no-warning path | no warning, normal flow |

---

#### REQ-SDK-007 — Multi-output spend command

**Priority:** Must
**User story:** As a developer, I want to construct spending transactions with multiple outputs at specific indices so guard conditions can be satisfied.
**Likely files touched:** `bins/cli/src/cmd_wallet.rs` (lines 548-647), `bins/cli/src/commands.rs` (lines 101-122)
**Test ID:** TEST-SDK-007

**Acceptance criteria:**

- [ ] New optional repeatable flag: `--output <spec>` where `<spec>` is `index:type:recipient:amount`
- [ ] When `--output` flags provided, they REPLACE the default single output. Positional `<TO>` and `<AMOUNT>` become optional.
- [ ] When NO `--output` flags, behavior identical to current single-output (backward compatible)
- [ ] Indices must be contiguous starting from 0 (no gaps)
- [ ] `type` parsed case-insensitively. Only user-constructible types: `normal`, `multisig`, `hashlock`, `htlc`, `vesting`. Protocol-internal types rejected.
- [ ] `recipient` resolved through `resolve_to_hash()`
- [ ] `amount` parsed through `coins_to_units()`
- [ ] Maximum 8 outputs per spend transaction
- [ ] `--witness` flag continues to apply to input 0
- [ ] Output summary printed before broadcast

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| No --output flags (backward compat) | legacy path | single `Output::normal(amount, recipient)` |
| Single `--output 0:normal:addr:500.0` | multi-output path | `vec![Output::normal(50_000_000_000, hash)]` |
| Two outputs: `--output 0:normal:addr1:300.0 --output 1:normal:addr2:200.0` | multi-output path | 2-output vec, correct indices |
| `--output 0:vesting:addr:100.0` | typed output | `vec![Output { output_type: Vesting, .. }]` |
| Gap: `--output 0:... --output 2:...` (no index 1) | validation error | `Err("Output indices must be contiguous starting from 0. Missing index 1")` |
| Duplicate index | validation error | `Err("Duplicate output index: 0")` |
| 9+ outputs | cap error | `Err("Maximum 8 outputs per spend transaction")` |
| Invalid type: `--output 0:bond:addr:100` | type restriction | `Err("Output type 'bond' cannot be used in spend transactions")` |
| Invalid amount | parse error | `Err("Invalid amount: ...")` |
| Mixed: `--output` with positional TO/AMOUNT | conflict error | `Err("Cannot use --output with positional <TO> and <AMOUNT> arguments")` |

---

#### REQ-SDK-008 — Witness verification for guards (documentation-only)

**Priority:** Must
**User story:** As an architect/developer, I need documented confirmation that guard conditions require NO witness changes.
**Likely files touched:** None (documentation artifact)
**Test ID:** TEST-SDK-008

**Acceptance criteria:**

- [ ] Architect's design document states: "Guard conditions (AmountGuard, OutputTypeGuard, RecipientGuard) evaluate against `ctx.transaction` outputs — they consume NO fields from the `Witness` struct. No changes to `parse_witness()` or `Witness` encoding are needed."
- [ ] Threshold evaluates sub-conditions recursively; existing Witness fields suffice
- [ ] Verified against `crates/core/src/conditions/eval.rs:121-165` and `crates/core/src/conditions/witness.rs:16-24`
- [ ] Test: guard-only condition satisfied with `witness="none()"` — unit test on evaluator

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| AmountGuard satisfaction | eval checks tx.outputs[idx].amount | true/false (no witness consumed) |
| OutputTypeGuard satisfaction | eval checks tx.outputs[idx].output_type | true/false (no witness consumed) |
| RecipientGuard satisfaction | eval checks tx.outputs[idx].pubkey_hash | true/false (no witness consumed) |
| Threshold(2, [guard1, guard2, sig]) | recursive eval | guards consume no witness; sig consumes signature |

---

#### REQ-SDK-009 — Parser unit tests

**Priority:** Must
**User story:** As a developer, I want comprehensive unit tests for all parser arms so regressions are caught at compile time.
**Likely files touched:** `bins/cli/src/parsers.rs` (add `#[cfg(test)] mod tests`)
**Test ID:** TEST-SDK-009

**Acceptance criteria:**

- [ ] `parsers.rs` currently has ZERO tests; add `#[cfg(test)] mod tests`
- [ ] Every existing parser arm: at least one positive and one negative test
- [ ] Every new parser arm: tests covering ALL input partitions from REQ-SDK-001 through REQ-SDK-004
- [ ] Composition tests: and/or with guards (REQ-SDK-005 partitions)
- [ ] `condition_to_output_type()` tested for all Condition variants
- [ ] `split_top_level()` tested for: no nesting, single nesting, double nesting, empty string, trailing comma
- [ ] All tests `#[test]` (sync)
- [ ] Minimum 40 test functions total

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| All positive cases per arm | parse_condition happy path | `Ok(expected_variant)` |
| All negative cases per arm | parse_condition error path | `Err(descriptive_message)` |
| Composition cases | recursive parse | `Ok(And/Or/Threshold(guard_children))` |
| condition_to_output_type all variants | type mapping | correct OutputType per variant |
| split_top_level edge cases | top-level splitter | correct Vec<&str> splits |

---

#### REQ-SDK-010 — E2E devnet round-trip test

**Priority:** Must
**User story:** As a developer, I want an end-to-end test that creates a guard-conditioned UTXO and spends it with multi-output, verifying the full pipeline.
**Likely files touched:** `bins/cli/tests/` (new integration test file), possibly `bins/node/tests/`
**Test ID:** TEST-SDK-010

**Acceptance criteria:**

- [ ] Test creates UTXO with condition: `and(amount_guard(100.0, 0), recipient_guard(<test_addr>, 0))`
- [ ] Test spends UTXO using `--output 0:normal:<test_addr>:100.0` with appropriate witness
- [ ] Spend tx accepted by node (validated, included in block)
- [ ] Negative test: wrong recipient at index 0 fails validation
- [ ] Negative test: insufficient amount at index 0 fails validation
- [ ] Runs on devnet config (guards_activation_height = 0)
- [ ] No live network needed — uses `Node::new_for_test()` or equivalent

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| Correct: matching recipient + amount | guard eval passes | Tx accepted, UTXO spent |
| Wrong recipient | RecipientGuard fails | Tx rejected with guard error |
| Insufficient amount | AmountGuard fails | Tx rejected with guard error |
| Wrong output index | guard checks idx 0, finds wrong data | Tx rejected |

---

### Part B — Template SDK (Should, Gated)

---

#### REQ-SDK-011 — Re-gate checkpoint before Part B

**Priority:** Should (gate)
**User story:** As a product owner, I want an explicit decision point after Part A lands, comparing `crates/sdk` (full crate) vs `docs/covenant-recipes.md` (copy-paste examples) before committing to Part B.
**Likely files touched:** None (process gate)
**Test ID:** N/A (process)

**Acceptance criteria:**

- [ ] Part A (REQ-SDK-001 through REQ-SDK-010) is merged and all tests pass
- [ ] Architect produces brief (max 1 page) comparing: (a) SDK crate with 5 pure functions returning `Condition` + CLI sub-command, vs. (b) `docs/covenant-recipes.md` with copy-paste CLI examples for the same 5 patterns
- [ ] User decides which path before Part B implementation begins
- [ ] Decision recorded in `docs/.workflow/` for traceability

---

#### REQ-SDK-012 — Template functions (5 patterns)

**Priority:** Should (gated by REQ-SDK-011)
**User story:** As a developer, I want pre-built condition templates for common transaction patterns.
**Likely files touched:** TBD by architect (either `crates/sdk/src/templates.rs` or `docs/covenant-recipes.md`)
**Test ID:** TEST-SDK-012

**Acceptance criteria:**

- [ ] 5 templates, each a pure function returning `Condition`:
  - `vault(owner_hash, cosigner_hash, delay_blocks)` → `Or(And(Signature(owner), Timelock(delay)), And(Signature(owner), Signature(cosigner)))`
  - `escrow(parties: Vec<Hash>, threshold: u8, timeout: u64, refund_hash: Hash)` → `Or(Multisig(threshold, parties), And(Signature(refund), TimelockExpiry(timeout)))`
  - `htlc_payment(hash, lock, expiry, refund_hash)` → delegates to `Condition::htlc_signed_refund()`
  - `subscription(recipient_hash, max_amount, output_index, interval_start, interval_end)` → nested And with guards + timelocks
  - `agent_allowance(agent_hash, recipient_hash, max_amount, output_index)` → `And(And(Signature(agent), RecipientGuard(recipient, idx)), AmountGuard(max_amount, idx))`
- [ ] Each function returns `Condition` (not `Result`) — composition is infallible
- [ ] Each function has doc comment explaining the spending scenario
- [ ] Each function has at least 2 unit tests: structure + round-trip encode/decode

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| vault: valid inputs | constructor | `Or(And(Sig, Timelock), And(Sig, Sig))` |
| escrow: 3-of-5 + timeout | constructor | `Or(Multisig(3, [5 keys]), And(Sig, TimelockExpiry))` |
| htlc_payment: standard | delegation | `htlc_signed_refund()` output |
| subscription: valid | constructor | nested And tree with guards + timelocks |
| agent_allowance: valid | constructor | `And(And(Sig, RecipientGuard), AmountGuard)` |

---

#### REQ-SDK-013 — CLI template sub-command

**Priority:** Should (gated by REQ-SDK-011)
**User story:** As a developer, I want a `doli template <name> [args]` command that prints the corresponding condition string or constructs the transaction directly.
**Likely files touched:** `bins/cli/src/commands.rs`, `bins/cli/src/main.rs`, new `bins/cli/src/cmd_template.rs`
**Test ID:** TEST-SDK-013

**Acceptance criteria:**

- [ ] `doli template vault --owner <addr> --cosigner <addr> --delay <blocks>` prints condition string to stdout
- [ ] All 5 templates have sub-commands with named flags
- [ ] `--dry-run` (default): prints `--condition` string for use with `doli send`
- [ ] `--send --wallet <path> --to <addr> --amount <amt>`: constructs and broadcasts directly
- [ ] Mainnet warning (REQ-SDK-006) applies when guard-containing templates used on mainnet
- [ ] `doli template --help` lists templates with one-line descriptions

**Output Contract:**

| Input Partition | Code Path | Expected Output |
|-----------------|-----------|-----------------|
| `template vault --owner ... --cosigner ... --delay 100 --dry-run` | print path | stdout: equivalent condition string |
| `template vault --help` | help path | lists flags with descriptions |
| `template unknown_name` | error path | `Err("Unknown template 'unknown_name'. Available: ...")` |
| `template agent-allowance` (missing required args) | error path | clap error listing required flags |

---

### Cross-Cutting (Must)

---

#### REQ-SDK-014 — Documentation and specs index update

**Priority:** Must
**User story:** As a developer, I want the CLI documentation and specs index to reflect the new parser arms and multi-output spend.
**Likely files touched:** `docs/cli.md` (sections 2.2 and 2.3), `specs/SPECS.md`
**Test ID:** N/A (documentation)

**Acceptance criteria:**

- [ ] `docs/cli.md` section 2.2 updated with new condition examples (4 new + composition)
- [ ] `docs/cli.md` section 2.3 documents `--output` flag with format `index:type:recipient:amount`
- [ ] `docs/cli.md` documents mainnet guard warning behavior
- [ ] `specs/SPECS.md` updated with new entry pointing to this spec
- [ ] `docs/cli.md` includes note: "Guard conditions are active on devnet/testnet only. Mainnet activation pending."
- [ ] Three-question consensus-shape answers included in commit message

## Traceability Matrix

| Requirement ID | Files | Tests | Priority |
|---------------|-------|-------|----------|
| REQ-SDK-001 | `bins/cli/src/parsers.rs` | TEST-SDK-001 | Must |
| REQ-SDK-002 | `bins/cli/src/parsers.rs` | TEST-SDK-002 | Must |
| REQ-SDK-003 | `bins/cli/src/parsers.rs` | TEST-SDK-003 | Must |
| REQ-SDK-004 | `bins/cli/src/parsers.rs` | TEST-SDK-004 | Must |
| REQ-SDK-005 | `bins/cli/src/parsers.rs` (verify, no changes) | TEST-SDK-005 | Must |
| REQ-SDK-006 | `bins/cli/src/cmd_wallet.rs`, `bins/cli/src/common.rs` | TEST-SDK-006 | Must |
| REQ-SDK-007 | `bins/cli/src/cmd_wallet.rs`, `bins/cli/src/commands.rs` | TEST-SDK-007 | Must |
| REQ-SDK-008 | Documentation artifact | TEST-SDK-008 | Must |
| REQ-SDK-009 | `bins/cli/src/parsers.rs` | TEST-SDK-009 | Must |
| REQ-SDK-010 | `bins/cli/tests/` or `bins/node/tests/` | TEST-SDK-010 | Must |
| REQ-SDK-011 | Process gate (no code) | N/A | Should |
| REQ-SDK-012 | TBD by architect | TEST-SDK-012 | Should |
| REQ-SDK-013 | `bins/cli/src/cmd_template.rs`, `commands.rs`, `main.rs` | TEST-SDK-013 | Should |
| REQ-SDK-014 | `docs/cli.md`, `specs/SPECS.md` | N/A | Must |

## Drift Findings

- **`docs/cli.md` section 2.2**: Lists conditions matching current parser. After this feature, the list must grow by 4 entries. Covered by REQ-SDK-014.
- **`docs/cli.md` section 2.3**: Does not document multi-output spend capability. Covered by REQ-SDK-014.
- No other drift detected. `specs/SPECS.md` does not reference guard primitives or SDK — this is new scope.

## Three-Question Consensus-Shape Commit Answers (Pre-Filled)

1. **Can any user-submittable transaction trigger this code path?** **NO** — all changes are in `bins/cli/`. The node's validation and evaluation code is untouched. The CLI constructs transactions using existing `Condition` constructors and `Transaction`/`Output` types. The node already validates and evaluates all 4 guard variants.

2. **Can any producer-action or attestation pattern trigger it?** **NO** — no changes to block production, attestation, coinbase, or any consensus-visible computation.

3. **Is the new behavior bit-identical to the old behavior for ALL reachable inputs?** **N/A** — CLI-only change. On-chain semantics (validation, evaluation, encoding) are unchanged. CLI gains ability to construct transactions it previously could not, but those transactions use the same Condition AST and Output types that already exist.

## Known Limitations (Document, Do Not Fix)

1. **`condition_to_output_type` flat matching**: `And(AmountGuard, RecipientGuard)` returns `OutputType::Vesting` (line 191-194). Display-level mismatch only — validation reads `extra_data`, not `output_type`. Fixing requires either new `OutputType::Guard` variant (consensus change) or smarter pattern matching (fragile). Documented as-is.

2. **Guards disabled on mainnet**: `guards_activation_height = u64::MAX`. All CLI guard tooling is devnet/testnet only until a separate consensus activation workflow. REQ-SDK-006 warns the user.

## Assumptions

| # | Assumption (technical) | Plain language | Confirmed |
|---|----------------------|----------------|-----------|
| 1 | `Condition::contains_guard()` exists and is public | Method used at `validation/transaction.rs:357` available for CLI warning | Yes |
| 2 | `coins_to_units()` available in CLI scope | Amount parser reusable for `amount_guard` parsing | Yes — used at `cmd_wallet.rs:584` |
| 3 | `Output::conditioned()` handles arbitrary output types | Constructor at `output.rs:224-238` accepts any `OutputType` + `Condition` | Yes |
| 4 | `split_top_level()` correctly handles nested parentheses | Existing helper at `parsers.rs:66-86` handles depth tracking | Yes (lacks tests — covered by REQ-SDK-009) |
| 5 | No `bins/cli/` test infrastructure exists | No test harness, no test utilities, no `#[cfg(test)]` blocks | Yes — grep confirmed |
| 6 | Part B scope determined after Part A | Architect evaluates options at REQ-SDK-011 gate | Yes — user confirmed |

## Identified Risks

- **Locked funds from parser bugs**: A bug swapping `min_amount`/`output_index` in `amount_guard` would create unsatisfiable conditions. **Mitigation**: REQ-SDK-009 partition-exhaustive tests.
- **Multi-output index confusion**: Users may confuse `output_index` (spending TX) with the UTXO being spent. **Mitigation**: CLI help text + REQ-SDK-014 docs.
- **Backward compat of `spend` command**: Adding `--output` must not break existing positional usage. **Mitigation**: REQ-SDK-007 specifies backward-compatible default + mutual exclusion.

## Out of Scope (Won't)

- Mainnet activation height for guards (separate consensus task)
- New `OutputType` variants
- Changes to `Condition` AST
- Changes to validation, evaluation, or witness encoding logic
- RPC display fixes (known limitation)
- Auto-change computation in multi-output spend (separate ergonomics task)
