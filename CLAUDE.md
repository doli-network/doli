# CLAUDE.md — DOLI

> **#0 RULE — NO GENESIS RESETS FOR STORAGE/FEATURE CHANGES.**
> Bitcoin activates features forward-only (BIP9/BIP8) at a future height — never retroactively from block 0.
> DOLI follows the same practice. If you can activate a feature at a future height without changing the state root
> of existing blocks, you DO NOT need a genesis reset. Only change activation height to 0 if you are INTENTIONALLY
> resetting the chain. Before touching activation heights, feature gates, or consensus params, ALWAYS ask:
> "Does this require a genesis reset or can it activate at a future height?" and REMIND the user of this rule.
> A genesis reset costs hours of downtime, requires redeploying all nodes, and loses all on-chain state.

> Code is the single source of truth. Everything else is a projection.
> When a doc drifts from code, the doc is wrong — register hotfix in MEMORY.md.

## Local Development

This is a local-only devnet environment. All nodes run on `127.0.0.1`.
No remote servers (ai1/ai2/ai3), no GitHub access, no explorer API.

- **Devnet data**: `~/.doli/devnet/` (keys, chainspec, data, logs, pids)
- **Testnet data**: `~/testnet/` (keys, seed, n1-n12, logs)
- **Binaries**: Built from source — `cargo build --release` → `target/release/doli-node`, `target/release/doli`
- **RPC ports**: Devnet uses 28500-28550 (seed=28500, producers=28501+)
- **P2P ports**: 50300+ (seed=50300)

## Mental Model

DOLI is a PoS blockchain. Understand this flow or you will break things:

```
Producer scheduled for slot → builds block (coinbase → reward pool) → VDF proof → broadcast
All nodes: receive block → validate → apply_block() → update 3 states → cache state root
Epoch boundary: pool drained → rewards distributed bond-weighted to qualified producers
```

**3 states** that must be identical across all nodes (snap sync depends on it):
- `ChainState` — height, best hash, slot, genesis timestamp
- `UtxoSet` — every unspent output (coins, bonds, rewards). UTXO model, not accounts.
- `ProducerSet` — registered producers, bonds, delegations, pending updates

**Data flow**: Block → `apply_block()` → writes to in-memory state AND disk batch atomically → state root cached. On restart, disk → in-memory. Both paths MUST produce identical state.

**Bond lifecycle**: Register (creates Bond UTXOs) → ACTIVATION_DELAY (10 blocks) → scheduled for production → earn epoch rewards → RequestWithdrawal (FIFO, vesting penalty, 7-day delay) → ClaimWithdrawal. Bonds are UTXOs with `output_type=Bond`, `lock_until=MAX`, `extra_data=creation_slot`.

## Stability Pillars (read `docs/postmortems/2026-04-17-attestation-stability-pillars.md`)

Two root-cause fixes stabilized the network. All other fixes were symptom mitigation:
1. **Orphan Chase** (v6.16.1): request parent block from sender when orphan arrives. 14 lines.
2. **Full Bitfield Decode** (v6.17.1, h=14000): decoder matches encoder order `[base | extra sorted]`. Broke the death spiral where filtered producers could never re-enter.

**CRITICAL**: Any encoder/decoder pair MUST be verified for index parity. Any consensus change MUST use constant gate (NOT HardForkSchedule) for rolling deploy — `current_fork_id(u64::MAX)` includes ALL entries immediately.

## If You Touch

- `apply_block()` → verify both UTXO paths match, check rollback paths mirror it, test state root convergence. Producer mutations (Register, AddBond, Exit, Slash, Withdrawal, Delegation) are DEFERRED to epoch boundary — never mid-epoch except epoch 0. Maintainer changes are immediate.
- **bitfield encoder/decoder** → encoder order is `[epoch_state.producer_list | extra sorted by pubkey]`. ALL decoders (post_commit, rewards, RPC) MUST use the same order or indices misalign. See Full Bitfield Decode pillar.
- **HardForkSchedule** → NEVER add entries for rolling deploys. `current_fork_id()` uses `u64::MAX`, which makes ALL entries active in fork_id immediately. Use constant gates instead.
- **CURRENT_PROTOCOL_VERSION** → **DO NOT bump unless the EpochState serialization format actually changes.** A protocol version bump triggers `delete_epoch_state()` on every node restart (`init.rs:727`), forcing a non-deterministic rebuild from local blocks. Snap-synced nodes have incomplete block history → different rebuild results → **guaranteed fork at the next epoch boundary**. INC-I-054 proved this: an unnecessary bump (8→9) for a height-gated feature caused a permanent chain split. The version check uses `!=` (not `<`), so rollback (9→8) triggers a SECOND deletion. Use `EPOCH_STATE_FORMAT_VERSION` for epoch_state compatibility, keep `CURRENT_PROTOCOL_VERSION` for peer handshake only.
- **activation heights** → Once activated on mainnet, an activation height is **IMMUTABLE** — it is part of the chain's consensus history. NEVER move an activation height forward (higher) after it has been crossed by the chain. INC-I-054: `security_audit_activation_height` was moved from 27,547→71,290, deactivating live security features. New features get their OWN activation height — never reuse or bundle with existing ones. Existing gates: `inc_i_026_scheduler_activation_height`, `fork_id_activation_height`, `full_bitfield_decode_height`, `rewards_epoch_list_fix_height`, `encrypted_content_activation_height`, `encrypted_content_v2_activation_height`, `epoch_state_reorg_activation_height`, `security_audit_activation_height`, `ghost_exclusion_activation_height`, `inc_i_068_weight_filter_activation_height` (INC-I-075, mainnet=197_800), `defi_activation_height` (INC-I-088 Phase 0, mainnet=`u64::MAX` — disabled until DeFi subsystem is audited and un-gated; freezes the 11 DeFi tx types and pairs with `OutputType::Collateral` in `is_conditioned()` + `[ERRTX-DEFI001]` hard-reject in `verify_input_conditions`).
- **Three-question consensus-shape checklist (INC-I-075)** → Before merging ANY code change that touches `active_producers`, scheduler inputs, bond snapshot, attestation bitfield encoding, coinbase shape, or any other consensus-visible computation, answer in the commit message:
  1. Can any user-submittable transaction trigger this code path?
  2. Can any producer-action or attestation pattern trigger it?
  3. Is the new behavior bit-identical to the old behavior for ALL reachable inputs?

  If (1) or (2) is YES and (3) is NO → an activation height in `NetworkParams` is REQUIRED. "Feature is currently unused" and "no producer has delegations yet" are NEVER valid justifications for skipping the gate (INC-I-075 cascade was triggered exactly by this assumption when the first DelegateBond activated post-deploy).
- rewards → check `calculate_epoch_rewards()` in `rewards.rs` AND `calculate_expected_epoch_rewards()` in `validation.rs` (currently disconnected — see MEMORY.md Open Items).
- storage serialization → every node diverges if canonical encoding changes. Requires chain reset. See `→ snapshot.rs`.
- consensus params → programmatic in `NetworkParams::defaults()`, NOT `include_str!`. Mainnet overrides blocked. Change requires new binary on ALL nodes simultaneously.
- rollback → undo-based rollback is first option (`node.rs:~6531`). Rebuild-from-genesis is fallback for blocks without undo data.
- Bond `extra_data` → CLI sends `creation_slot=0`, node stamps real slot at apply. Never trust raw tx `extra_data`.
- **data directory wipe** → **CRITICAL**: Before wiping ANY node's `data/` directory (manually or via script), ALWAYS verify that `wallet.json` and `producer.seed.txt` are NOT inside the directory being deleted. Back them up first if present. Losing wallet/seed files means the producer cannot start and keys may be unrecoverable. The standard wipe command preserves these, but manual `rm -rf data/*` does not.

## After Every Modification

After completing any code change, ALWAYS propose the following checklist to the user:

1. **Build gate**: `cargo build --release && cargo clippy -- -D warnings && cargo fmt --check`
2. **Test**: `cargo test -p <affected-crate> --lib` (or full `cargo test` if cross-crate)
3. **Version protection** (if consensus/protocol/validation changed):
   - **DO NOT bump `CURRENT_PROTOCOL_VERSION`** unless the `EpochState` serialization format changed (see "If You Touch" → CURRENT_PROTOCOL_VERSION). A needless bump causes `delete_epoch_state()` on every node → non-deterministic rebuild → fork (INC-I-054). Bump `EPOCH_STATE_FORMAT_VERSION` only when the struct changes.
   - **NEVER move an activation height forward** (higher) after the chain has crossed it — this deactivates live consensus rules (INC-I-054). New features get their own activation height.
   - Consider adding a `HardForkSchedule` entry in `crates/updater/src/hardfork.rs` if the change is consensus-breaking at a future height
   - Consider bumping `MIN_PEER_PROTOCOL_VERSION` if old peers must be partitioned immediately
   - **Block content check (INC-I-062)**: If the change alters what a producer puts INTO a block (attestation bitfield, coinbase format, tx ordering, presence_root, header fields) — even without changing validation rules — it REQUIRES synchronized deploy (stop ALL nodes, then start ALL). A rolling deploy creates competing valid blocks during the mixed-version window → fork. "No activation height needed" does NOT mean "safe for rolling restart."
4. **Documentation alignment** (MANDATORY — not optional):
   - Update specs and docs BEFORE committing. Every code change that affects behavior must have matching documentation.
   - `specs/protocol.md` — if wire format, messages, encoding, or peer behavior changed
   - `specs/security_model.md` — if attack surface, scoring, or threat model changed
   - `docs/architecture.md` — if crate structure or component interactions changed
   - `docs/troubleshooting.md` — if new error conditions or failure modes added
   - `docs/rpc_reference.md` — if RPC endpoints changed
   - `docs/cli.md` — if CLI commands or flags changed
   - `CLAUDE.md` code map — if new files/modules added
   - Run `/sync-docs` for thorough alignment verification on larger changes.
5. **Copy binary** (if deploying to testnet): `cp target/release/doli-node ~/testnet/bin/ && codesign --force --sign - ~/testnet/bin/doli-node`
6. **Commit and push** — ALWAYS ask the user: "Ready to commit and push?" Do not skip this step. Do not assume. Always ask explicitly after every completed modification.
7. **Deploy consideration**: testnet first, NEVER mainnet without explicit confirmation

## Law

1. Show plan → show diff → WAIT for approval → execute. Broken after deploy? STOP, report.
2. Code is SOT. For design intent: WHITEPAPER > specs/ > docs/. For reality: read the code.
3. Simplest solution that doesn't compromise safety. Design for 1000s of producers in 10s slots.
4. Commit: `--author "Antonio Lozada <antonio@omegacortex.ai>"`. Gate: `cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test`.
5. Learning protocol: before following any doc/skill, check MEMORY.md hotfixes. Doc drifts from code? Register hotfix, fix the doc. A mistake not fixed at the source repeats forever.

## Map — Code

| What | Where |
|------|-------|
| **Node struct + getters** | `bins/node/src/node/mod.rs` |
| **Node::new()** | `bins/node/src/node/init.rs` |
| **run(), start_network(), start_rpc()** | `bins/node/src/node/startup.rs` |
| **run_event_loop(), handle_network_event()** | `bins/node/src/node/event_loop.rs` |
| **handle_new_block(), execute_reorg()** | `bins/node/src/node/block_handling.rs` |
| **fork recovery (9 functions)** | `bins/node/src/node/fork_recovery.rs` |
| **apply_block()** | `bins/node/src/node/apply_block.rs` |
| **try_produce_block()** | `bins/node/src/node/production.rs` |
| **check_producer_eligibility(), validate_block_*()** | `bins/node/src/node/validation_checks.rs` |
| **calculate_epoch_rewards(), handle_equivocation()** | `bins/node/src/node/rewards.rs` |
| **rollback_one_block()** | `bins/node/src/node/rollback.rs` |
| **Fork recovery integration tests (11)** | `bins/node/tests/fork_recovery.rs` |
| **Node::new_for_test()** | `bins/node/src/node/init.rs` |
| **Node lib (test access)** | `bins/node/src/lib.rs` |
| **run_periodic_tasks()** | `bins/node/src/node/periodic.rs` |
| **genesis producer derivation** | `bins/node/src/node/genesis.rs` |
| Constants | `crates/core/src/consensus.rs` |
| Config/env | `crates/core/src/network_params.rs` |
| Scheduler | `crates/core/src/scheduler.rs` |
| Validation (5,698 lines) | `crates/core/src/validation.rs` |
| Transactions (27 types) | `crates/core/src/transaction.rs` |
| Block + BlockBuilder | `crates/core/src/block.rs` |
| Chainspec + genesis hash | `crates/core/src/chainspec.rs` |
| Network/gossip | `crates/network/src/service.rs` |
| Status protocol + version constants | `crates/network/src/protocols/status.rs` |
| Peer scoring (incl. IncompatibleVersion) | `crates/network/src/scoring.rs` |
| Sync state machine | `crates/network/src/sync/manager.rs` |
| Block storage | `crates/storage/src/block_store.rs` |
| State DB (RocksDB) | `crates/storage/src/state_db.rs` |
| UTXO set (in-memory) | `crates/storage/src/utxo.rs` |
| UTXO set (RocksDB) | `crates/storage/src/utxo_rocks.rs` |
| ProducerSet + bonds | `crates/storage/src/producer.rs` |
| State root + snapshots | `crates/storage/src/snapshot.rs` |
| RPC methods (45) | `crates/rpc/src/methods/` |
| Transaction mempool | `crates/mempool/src/` |
| Auto-update + hard fork schedule | `crates/updater/src/` |
| Block archiver | `crates/storage/src/archiver.rs` |
| CLI | `bins/cli/src/` |

## Map — Scripts (local testnet)

| Task | Script |
|------|--------|
| Install launchd services | `scripts/install-local-services.sh` — creates plists for seed + n1-n12 |
| Start/stop/status | `scripts/testnet.sh start\|stop\|restart\|status [seed\|n1\|...\|all]` |
| Tail logs | `scripts/testnet.sh logs [seed\|n1\|...]` |

**Port layout**:
- Seed: P2P=30300, RPC=8500, Metrics=9000
- N{i}: P2P=30300+i, RPC=8500+i, Metrics=9000+i

**Directories**: `~/testnet/` — keys, seed, n1-n12, logs, bin

**Logs on remote servers (mainnet/testnet)**: `journalctl` only shows systemd lifecycle events (start/stop/restart), NOT application logs. App logs are written to files — check the node's data directory for `*.log` files or the `--log-file` flag in the systemd unit. Always look at log files, not journalctl, when debugging node behavior.

## Map — Scripts (local devnet)

| Task | Script |
|------|--------|
| Node status | `scripts/status.sh` — scans local RPC ports 28500-28550 |
| Wallet balances | `scripts/balances.sh` — queries all producer wallets |
| Bond details | `scripts/bonds.sh` — shows producer/bond info via RPC |
| Chain reset | `scripts/chain-reset.sh devnet` — kill processes, wipe data |
| Build from source | `scripts/update.sh` — `cargo build --release` |
| Launch 2-node testnet | `scripts/launch_testnet.sh` — creates local devnet |
| Deploy producers | `scripts/deploy_producers.sh` — interactive producer setup |

## Map — Scripts (Seed Guardian)

| Task | Script |
|------|--------|
| Fork detection | `scripts/fork-monitor.sh` — polls all nodes, detects chain tip divergence |
| Emergency halt | `scripts/emergency-halt.sh` — pauses production on all nodes via RPC |
| Emergency resume | `scripts/emergency-resume.sh` — resumes production on all nodes via RPC |
| Seed backup | `scripts/seed-backup.sh` — creates RocksDB checkpoint via RPC |
| Test guardian | `scripts/test_guardian.sh` — smoke test all guardian features |

**Seed auto-checkpoint**: Start seeds with `--auto-checkpoint 100` for automatic snapshots every 100 blocks (keeps last 5, rotates oldest). Essential for seed protection.

## Map — Docs & Skills

| What | Where |
|------|-------|
| Architecture | `docs/architecture.md` |
| Rewards system | `docs/rewards.md` |
| RPC reference (45 methods) | `docs/rpc_reference.md` |
| CLI reference | `docs/cli.md` |
| Troubleshooting | `docs/troubleshooting.md` |
| Protocol spec | `specs/protocol.md` |
| Security model | `specs/security_model.md` |
| RPC/debug skill | `.claude/skills/doli-network/SKILL.md` |
| Drift tracker | `MEMORY.md` (auto-memory) |
| Bug reports | `docs/legacy/bugs/` |
| CLI issues | `CLI.md` |

---


---

# OMEGA Ω

## Philosophy
OMEGA is a multi-agent workflow where each agent has a specific role and code passes through multiple validation layers.
Every agent reads from and writes to a shared institutional memory (SQLite) — no agent acts alone, without backpressure.

## Source of Truth Hierarchy
1. **Codebase** — the ultimate source of truth. Always trust code over documentation.
2. **`.omega/memory.db`** — institutional memory. Accumulated decisions, failed approaches, hotspots, findings across all sessions.
3. **specs/** — technical specifications per domain. `specs/SPECS.md` is the master index.
4. **docs/** — user-facing and developer documentation. `docs/DOCS.md` is the master index.

When specs or docs conflict with the codebase, the codebase wins. Agents must flag the discrepancy and update specs/docs accordingly.

## Institutional Memory

Every workflow reads from and writes to `.omega/memory.db`. **This protocol is not optional.**

**Cortex (team sharing):** Read @INDEX of `.claude/protocols/cortex-protocol.md` for shared knowledge rules.

**Full protocol reference:** Read the **@INDEX** (first 13 lines) of `.claude/protocols/memory-protocol.md` to find section line ranges, then Read ONLY needed sections with offset/limit. For cross-file lookup: `.claude/protocols/PROTOCOLS-INDEX.md`.

**Core rules (always in effect):**
- **DB Detection**: `test -f .omega/memory.db` at session/workflow start. If missing, skip memory ops gracefully.
- **Session briefing = behavioral learnings + open incidents + active invariants**. NOT decisions, bug details, outcomes, or hotspots — those are on-demand.
- **Briefing before action**: Every agent queries memory.db for scope-specific context (hotspots, failed approaches, findings, decisions, patterns, invariants) before starting work.
- **Log incrementally**: Write to memory.db immediately after each significant action. Never batch for the end — context compaction loses batched entries.
- **Self-score every action**: Rate significant actions (-1/0/+1) immediately after completing them.
- **Track bugs as incidents**: Every bug gets an INC-{PREFIX}-NNN ticket (prefix from `user_profile.contributor_prefix`). Use `--incident` on doctor to resume. Read @INDEX of `.claude/protocols/incident-protocol.md`.
- **Extract invariants from bugs**: Level 2+ incidents must produce an `invariants` record (INV-{DOMAIN}-NNN) + linked `regression_tests`. Level 3 also requires `monitoring_signals`. Query `v_regression_map` before modifying files with linked invariants. Read `.claude/protocols/invariant-protocol.md`.
- **Extract behavioral learnings**: When the user corrects you or an incident reveals a reasoning flaw, extract a behavioral rule (HOW to think, not domain patterns).
- **Close-out when done**: Verify completeness, distill lessons, extract learnings, track bugs as incidents.
- **Pipeline tracking**: Every code-modifying `/omega-*` command registers a `workflow_runs` entry at start, updates at end. Read-only commands skip tracking.
- **Non-pipeline work**: Informal work gets a `workflow_runs` entry with type `'manual'`.
- **sqlite3 quoting**: ALWAYS use heredoc syntax (`<<'EOF' ... EOF`). NEVER inline single-quote wrapping — breaks `datetime('now')` due to shell quote nesting.
- **Immediate self-correction**: When the user points out a mistake, immediately save a behavioral learning and fix. The correction target is Claude, not the user.
- **Error tolerance**: If sqlite3 fails, log the error and continue working. Never block work for a DB failure.

## Identity

The briefing hook may inject an identity block. **Full reference:** Read @INDEX of `.claude/protocols/identity.md` for section lookup.

**Core rules:**
- Protocol always overrides identity. Identity influences communication style, not functional behavior.
- **Auto-onboarding:** If the session briefing contains "No OMEGA profile found", you MUST run `/omega-onboard` before responding to the user's first message. This is a blocking prerequisite — do not skip it.

## Contextual Tips (Progressive Onboarding)

After completing a user's task, if they did something manually that an OMEGA command handles better, add a **one-line tip** at the end of your response. Format: `Tip: /omega-<command> can <benefit>.` Rules: max 1 tip per session, never repeat a tip the user has seen (query `tips_shown` in memory.db), and never tip about the command they just used.

## Global Rules

1. **NEVER write code without tests first** (strict TDD)
2. **NEVER assume** — if something is unclear, the analyst must ask
3. **Module by module** — do not implement everything at once
4. **Understand architecture before ANY modification** — comprehend module boundaries, data flows, dependencies, blast radius
5. **Every assumption must be explicit** — technical + human-readable summary
6. **Codebase is king** — when in doubt, read the actual code
7. **Keep specs/, docs/, and protocols in sync** — every code change must update relevant specs, docs, and protocol files
8. **Every requirement has acceptance criteria** — "it should work" is not acceptable
9. **Every requirement has a priority** — Must/Should/Could/Won't (MoSCoW)
10. **Every requirement is traceable** — from ID through tests to implementation
11. **60% context budget** — every agent must complete its work within 60% of the context window
12. **Briefing before action** — every agent queries memory.db before starting work
13. **Log incrementally during work** — every agent writes to memory.db immediately after each significant action
14. **Self-score every action** — every agent rates its own significant actions (-1/0/+1) immediately
15. **Distill lessons from patterns** — when 3+ outcomes share a theme, distill a permanent lesson that changes future agent behavior
16. **Read-only agents stay in their lane** — research agents (codebase-expert, functionality-analyst) NEVER offer to implement. They report findings and suggest appropriate commands
17. **Security chain enforcement** — when a feature involves external data (multi-user, network, file ingestion), 5 agents independently verify security: Analyst (REQ-*-SEC requirements), Architect (trust boundary analysis or STOP), Test Writer (adversarial injection tests), QA (independent probing), Reviewer (injection pattern scan — automatic blocker)
18. **Stupid Simple First (SSF)** — before ANY design work, state the stupidest one-sentence solution that works. Present it ALONE. Only add complexity if the user rejects it with a specific reason. Never present a menu of options. Read `.claude/protocols/anti-overengineering.md`
19. **Modular coding enforcement** — no source file exceeds 500 lines (800 for test files). When approaching the limit, split into focused modules. Read MODULE-SIZE-BUDGET in `.claude/protocols/anti-overengineering.md`
20. **Intellectual honesty** — if your analysis contradicts itself (claim X, find not-X), STOP and resolve before continuing. Show your work on math/logic claims. State what you don't understand before proposing solutions. Try to disprove your own hypotheses before acting on them. Max 2 inferences without verification. Read `.claude/protocols/intellectual-honesty.md`
21. **Output Contract (all tests, all languages)** — before writing ANY test assertion, produce the Output Contract Checklist: ALL outputs, ALL paths, ALL **input partitions** per path (distinct input classes with different math/logic). No test complete until every output × path × partition cell has an assertion. Read `.claude/protocols/output-contract.md`. Fix confidence above 0.7 requires FAIL→PASS test evidence — no exceptions. **Sequence: test BEFORE fix** — the reproduction test must exist and FAIL before any fix code is written OR fix plan presented. Describing the fix before the test biases the test toward confirming the fix rather than verifying correctness
22. **Prompt refinement at intake** — every omega command that accepts a user description MUST run prompt refinement BEFORE any agent work begins. Detect and neutralize investigation-anchoring language ("in the logs", "the bug is in X"), preserve domain context (symptoms, nodes, timestamps), reframe assumed root causes as hypotheses. Display the refinement to the user. Read `.claude/protocols/prompt-refinement.md`

## Fail-Safe Controls

**Full reference:** Read @INDEX of `.claude/protocols/fail-safes.md` for section lookup.

**Core rules:** Prerequisite gates (agents verify upstream output). Iteration limits (QA↔Dev: 3, Reviewer↔Dev: 2, Audit fix: 5). Error recovery to `docs/.workflow/chain-state.md`. Developer max 5 retries/module.

## Context Efficiency (ENFORCED)

**Full reference:** Read @INDEX of `.claude/protocols/context-budget.md` for section lookup.

**Rules:** CLAUDE.md MUST stay under 15,000 characters. Never inline templates/SQL/procedures — put them in `core/protocols/` and reference. Never duplicate content across files. Lazy-load protocol sections via @INDEX (first N lines) then Read with offset/limit. 60% context budget per agent. Never read the entire codebase.

## Traceability Chain
```
Discovery → Analyst (REQ-XXX-001) → Architect (module map) → Test Writer (TEST-XXX-001) → Developer → QA (acceptance criteria) → Reviewer (completeness)
```

## Project Layout
```
root-project/
├── backend/              ← Backend source code
├── frontend/             ← Frontend (if applicable)
├── specs/                ← Technical specifications
├── docs/                 ← Documentation
├── CLAUDE.md             ← Workflow rules
├── .omega/
│   └── memory.db         ← Institutional memory (SQLite)
└── .claude/
    ├── agents/           ← Agent definitions
    ├── commands/         ← Command definitions
    ├── protocols/        ← Protocol reference files (loaded on-demand)
    └── db-queries/       ← Query reference files
```

## Conventions
- Preferred language: Rust (or whatever the user defines)
- Tests: alongside code or in `backend/tests/` (or `frontend/tests/`)
- Commits: conventional (feat:, fix:, docs:, refactor:, test:)
- Branches: feature/, bugfix/, hotfix/
