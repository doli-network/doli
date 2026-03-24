# CLAUDE.md — DOLI

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

## If You Touch

- `apply_block()` → verify both UTXO paths match, check rollback paths mirror it, test state root convergence. Producer mutations (Register, AddBond, Exit, Slash, Withdrawal, Delegation) are DEFERRED to epoch boundary — never mid-epoch except epoch 0. Maintainer changes are immediate.
- rewards → check `calculate_epoch_rewards()` in `node.rs:~5993` AND `calculate_expected_epoch_rewards()` in `validation.rs` (currently disconnected — see MEMORY.md Open Items).
- storage serialization → every node diverges if canonical encoding changes. Requires chain reset. See `→ snapshot.rs`.
- consensus params → programmatic in `NetworkParams::defaults()`, NOT `include_str!`. Mainnet overrides blocked. Change requires new binary on ALL nodes simultaneously.
- rollback → undo-based rollback is first option (`node.rs:~6531`). Rebuild-from-genesis is fallback for blocks without undo data.
- Bond `extra_data` → CLI sends `creation_slot=0`, node stamps real slot at apply. Never trust raw tx `extra_data`.

## Law

1. Show plan → show diff → WAIT for approval → execute. Broken after deploy? STOP, report.
2. Code is SOT. For design intent: WHITEPAPER > specs/ > docs/. For reality: read the code.
3. Simplest solution that doesn't compromise safety. Design for 1000s of producers in 10s slots.
4. Commit: `--author "Ivan D. Lozada <ivan@doli.network>"`. Gate: `cargo build && cargo clippy -- -D warnings && cargo fmt --check && cargo test`.
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
| **rollback_one_block(), resolve_shallow_fork()** | `bins/node/src/node/rollback.rs` |
| **run_periodic_tasks()** | `bins/node/src/node/periodic.rs` |
| **genesis producer derivation** | `bins/node/src/node/genesis.rs` |
| Constants | `crates/core/src/consensus.rs` |
| Config/env | `crates/core/src/network_params.rs` |
| Scheduler | `crates/core/src/scheduler.rs` |
| Validation (5,698 lines) | `crates/core/src/validation.rs` |
| Transactions (16 types) | `crates/core/src/transaction.rs` |
| Block + BlockBuilder | `crates/core/src/block.rs` |
| Chainspec + genesis hash | `crates/core/src/chainspec.rs` |
| Network/gossip | `crates/network/src/service.rs` |
| Sync state machine | `crates/network/src/sync/manager.rs` |
| Block storage | `crates/storage/src/block_store.rs` |
| State DB (RocksDB) | `crates/storage/src/state_db.rs` |
| UTXO set (in-memory) | `crates/storage/src/utxo.rs` |
| UTXO set (RocksDB) | `crates/storage/src/utxo_rocks.rs` |
| ProducerSet + bonds | `crates/storage/src/producer.rs` |
| State root + snapshots | `crates/storage/src/snapshot.rs` |
| RPC methods (31) | `crates/rpc/src/methods.rs` |
| Transaction mempool | `crates/mempool/src/` |
| Auto-update system | `crates/updater/src/` |
| Block archiver | `crates/storage/src/archiver.rs` |
| CLI | `bins/cli/src/` |

## Local Testnet Operations — `ops/`

See `docs/ops.md` for full script reference, port layout, chain reset procedure, and directory structure.

## Map — Docs & Skills

| What | Where |
|------|-------|
| **Testnet ops** | `docs/ops.md` — scripts, ports, chain reset, deployment |
| Architecture | `docs/architecture.md` |
| Rewards system | `docs/rewards.md` |
| RPC reference (31 methods) | `docs/rpc_reference.md` |
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
2. **`.claude/memory.db`** — institutional memory. Accumulated decisions, failed approaches, hotspots, findings across all sessions.
3. **specs/** — technical specifications per domain. `specs/SPECS.md` is the master index.
4. **docs/** — user-facing and developer documentation. `docs/DOCS.md` is the master index.

When specs or docs conflict with the codebase, the codebase wins. Agents must flag the discrepancy and update specs/docs accordingly.

## Institutional Memory

Every workflow reads from and writes to `.claude/memory.db`. **This protocol is not optional.**

**Cortex (team sharing):** Read @INDEX of `.claude/protocols/cortex-protocol.md` for shared knowledge rules.

**Full protocol reference:** Read the **@INDEX** (first 13 lines) of `.claude/protocols/memory-protocol.md` to find section line ranges, then Read ONLY needed sections with offset/limit. For cross-file lookup: `.claude/protocols/PROTOCOLS-INDEX.md`.

**Core rules (always in effect):**
- **DB Detection**: `test -f .claude/memory.db` at session/workflow start. If missing, skip memory ops gracefully.
- **Session briefing = behavioral learnings + open incidents**. NOT decisions, bug details, outcomes, or hotspots — those are on-demand.
- **Briefing before action**: Every agent queries memory.db for scope-specific context (hotspots, failed approaches, findings, decisions, patterns) before starting work.
- **Log incrementally**: Write to memory.db immediately after each significant action. Never batch for the end — context compaction loses batched entries.
- **Self-score every action**: Rate significant actions (-1/0/+1) immediately after completing them.
- **Track bugs as incidents**: Every bug gets a contributor-prefixed ticket (INC-{PREFIX}-NNN, e.g., `INC-AJL-001`). The PREFIX comes from `user_profile.contributor_prefix`. Use `--incident=INC-{PREFIX}-NNN` on doctor to resume. Read the @INDEX of `.claude/protocols/incident-protocol.md` for section lookup.
- **Extract behavioral learnings**: When the user corrects you or an incident reveals a reasoning flaw, extract a behavioral rule (HOW to think, not domain patterns).
- **Close-out when done**: Verify completeness, distill lessons, extract behavioral learnings, track bugs as incidents.
- **Pipeline tracking**: Every `/omega-*` command that modifies code registers a `workflow_runs` entry at start, updates status at end. Read-only commands (e.g., `audit` without `--fix`) skip tracking — the artifact is sufficient.
- **Non-pipeline work**: Even informal work gets a `workflow_runs` entry with type `'manual'`.
- **sqlite3 quoting**: ALWAYS use heredoc syntax (`<<'EOF' ... EOF`) for sqlite3 commands. NEVER use inline single-quote wrapping — it breaks `datetime('now')` and string literals due to shell quote nesting.
- **Immediate self-correction**: When the user points out a mistake (even indirectly, e.g. "how can this be avoided?"), immediately save a behavioral learning and fix the behavior. Do not explain the fix back to the user as if they caused the problem — the correction target is Claude, not the user.
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
18. **Anti-overengineering gate** — verify necessity, scale, and simplicity before proposing any solution. Read `.claude/protocols/anti-overengineering.md`

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
└── .claude/
    ├── agents/           ← Agent definitions
    ├── commands/         ← Command definitions
    ├── protocols/        ← Protocol reference files (loaded on-demand)
    ├── memory.db         ← Institutional memory (SQLite)
    └── db-queries/       ← Query reference files
```

## Conventions
- Preferred language: Rust (or whatever the user defines)
- Tests: alongside code or in `backend/tests/` (or `frontend/tests/`)
- Commits: conventional (feat:, fix:, docs:, refactor:, test:)
- Branches: feature/, bugfix/, hotfix/
