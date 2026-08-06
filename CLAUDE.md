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

**Devnet and testnet are local-only** — all nodes run on `127.0.0.1`. NEVER SSH to ai1–ai5 for
devnet/testnet work: those hosts run **mainnet**. Mainnet is remote and is covered by the
`mainnet` / `release` / `guardian` skills, not by this section.

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
- **CURRENT_PROTOCOL_VERSION** → **DO NOT bump unless the EpochState serialization format actually changes** (INV-4, in every session briefing). A bump triggers `delete_epoch_state()` on restart (`init.rs:727`) → non-deterministic rebuild → fork at the next epoch boundary (INC-I-054). The check is `!=`, so rollback deletes a second time. Use `EPOCH_STATE_FORMAT_VERSION` for epoch_state; `CURRENT_PROTOCOL_VERSION` is peer handshake only.
- **activation heights** → Once crossed on mainnet, an activation height is **IMMUTABLE** — it is consensus history. NEVER move one forward (higher) after the chain has passed it: INC-I-054 moved `security_audit_activation_height` 27,547→71,290 and deactivated live security features. New features get their OWN height — never reuse or bundle. **The pinned values are in `crates/core/src/network_params/` (code is SoT) — read them there, never from this file.** Oracle + DeFi gates are `u64::MAX` (frozen pre-activation); pinning any real height is a separate decision-session per HC-6 / INC-I-075.
- **Three-question consensus-shape checklist (INC-I-075, INV-12)** → touching `active_producers`, scheduler inputs, bond snapshot, bitfield encoding, coinbase shape, or any consensus-visible computation? Answer in the commit message: (1) can a user-submittable tx reach this path? (2) can a producer-action or attestation pattern reach it? (3) is the new behavior bit-identical for ALL reachable inputs? **(1|2) YES + (3) NO → activation height REQUIRED.** "Currently unused" is NEVER a valid skip — that assumption caused the INC-I-075 cascade.
- rewards → distribution `calculate_epoch_rewards()` (`node/rewards.rs`); validation `validate_block_economics()` (`node/validation_checks.rs`, weighted presence via `WeightedRewardCalculator`). The old `calculate_expected_epoch_rewards()` was dead code, removed 2026-03-16 (tombstone: `crates/core/src/validation/rewards_legacy.rs`).
- storage serialization → changing canonical encoding diverges every node and requires a chain reset. See `snapshot.rs`.
- consensus params → programmatic in `NetworkParams::defaults()`, NOT `include_str!`. Mainnet overrides blocked. Change requires a new binary on ALL nodes simultaneously.
- rollback → undo-based is first choice; rebuild-from-genesis is the fallback for blocks without undo data.
- Bond `extra_data` → CLI sends `creation_slot=0`; the node stamps the real slot at apply. Never trust raw tx `extra_data`.
- **data directory wipe** → **CRITICAL**: before wiping any `data/` dir, verify `wallet.json` and `producer.seed.txt` are not inside it (`find <dir> -name 'wallet*' -o -name '*.seed.txt'`). Manual `rm -rf data/*` does NOT preserve them; lost keys may be unrecoverable.
- **Phase 2.1 oracle** → shipped (M1-M11) but frozen at `oracle_activation_height = u64::MAX`. Touch points: `crates/core/src/oracle/`, `bins/node/src/node/apply_block/oracle.rs`, `crates/rpc/src/methods/oracle{,_status}.rs`. The §6 disclosure constant in `oracle_status.rs` is byte-equal-locked to the spec by `m11_centralization_disclosure_byte_equal_to_spec` — edit both or neither. Spec: `specs/oracle-structural-anchored-economics.md`.

## After Every Modification

After completing any code change, ALWAYS propose the following checklist to the user:

1. **Build gate**: `cargo build --release && cargo clippy -- -D warnings && cargo fmt --check`
2. **Test**: `cargo test -p <affected-crate> --lib` (or full `cargo test` if cross-crate)
3. **Version protection** (if consensus/protocol/validation changed) — ask BOTH deploy questions:
   - Does it change consensus RULES? → activation height required (see "If You Touch").
   - Does it change block CONTENT (bitfield, coinbase, tx ordering, presence_root, header fields)? → **synchronized deploy** (stop ALL, then start ALL), INC-I-062 / INV-8. NO to the first does NOT imply safe for a rolling restart.
   - Also consider: `HardForkSchedule` entry (`crates/updater/src/hardfork.rs`) for a future-height break; `MIN_PEER_PROTOCOL_VERSION` if old peers must partition immediately.
4. **Documentation alignment** (MANDATORY) — update specs/docs BEFORE committing; run `/sync-docs` (it knows which of `specs/protocol.md`, `specs/security_model.md`, `docs/{architecture,troubleshooting,rpc_reference,cli}.md`, and the code map below apply).
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
| **FORK_GUARD wedge-escape (INC-I-143 F2)** | `bins/node/src/node/wedge_escape.rs` |
| **fork recovery (9 functions)** | `bins/node/src/node/fork_recovery.rs` |
| **apply_block()** | `bins/node/src/node/apply_block/` (dir) |
| **try_produce_block(), compute_block_vdf()** | `bins/node/src/node/production/mod.rs` |
| **check_producer_eligibility(), validate_block_*()** | `bins/node/src/node/validation_checks.rs` |
| **calculate_epoch_rewards(), handle_equivocation()** | `bins/node/src/node/rewards.rs` |
| **rollback_one_block()** | `bins/node/src/node/rollback.rs` |
| **Fork recovery integration tests (11)** | `bins/node/tests/fork_recovery.rs` |
| **Node::new_for_test()** | `bins/node/src/node/init.rs` |
| **Node lib (test access)** | `bins/node/src/lib.rs` |
| **run_periodic_tasks()** | `bins/node/src/node/periodic.rs` |
| **genesis producer derivation** | `bins/node/src/node/genesis.rs` |
| Constants | `crates/core/src/consensus/` (dir) |
| Config/env + activation heights | `crates/core/src/network_params/` (dir) |
| Scheduler | `crates/core/src/scheduler.rs` |
| Validation (~11,000 lines) | `crates/core/src/validation/` (dir; VDF check in `producer.rs`) |
| Transactions (`TxType`, 24 variants) | `crates/core/src/transaction/types.rs` |
| Block + BlockBuilder | `crates/core/src/block.rs` |
| Chainspec + genesis hash | `crates/core/src/chainspec.rs` |
| Network/gossip | `crates/network/src/service/` (dir) |
| Gossip staleness/dedup gate (INC-I-142) | `crates/network/src/gossip/staleness.rs` |
| Status protocol + version constants | `crates/network/src/protocols/status.rs` |
| Peer scoring (incl. IncompatibleVersion) | `crates/network/src/scoring.rs` |
| Sync state machine | `crates/network/src/sync/manager/` (dir) |
| Block storage | `crates/storage/src/block_store/` (dir) |
| State DB (RocksDB) | `crates/storage/src/state_db/` (dir) |
| UTXO set (in-memory) | `crates/storage/src/utxo/in_memory.rs` |
| UTXO set (RocksDB) | `crates/storage/src/utxo/set.rs` |
| ProducerSet + bonds | `crates/storage/src/producer/` (dir) |
| State root + snapshots | `crates/storage/src/snapshot.rs` |
| RPC methods (56) | `crates/rpc/src/methods/` (incl. `oracle.rs` + `oracle_status.rs` for Phase 2.1 M9-M11) |
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
| System-impact gauntlet | `scripts/gauntlet.sh` — replays paid-for failure modes over the live testnet (10 scenarios). Default is observational + one safe launchd restart; it NEVER wipes or pkills. Destructive scenarios are opt-in behind confirm-vars: `--chaos`, `--gs009` (fleet rolling restart), `--gs010` (**testnet only — the one scenario that writes to the CHAIN**). The scenario list and required confirm-vars are in the header comment of `scripts/gauntlet.sh` and in `scripts/README.md`. Gate armed by `.omega/gauntlet.conf`. |

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
| **Skill index (grep-first)** | `.claude/skills/SKILLS-INDEX.md` — keyword→skill:section:line map for all 30 skills (15 code + 15 ops). Grep it before reading any skill file. |
| Architecture | `docs/architecture.md` |
| Rewards system | `docs/rewards.md` |
| RPC reference (56 methods) | `docs/rpc_reference.md` |
| CLI reference | `docs/cli.md` |
| Troubleshooting | `docs/troubleshooting.md` |
| Protocol spec | `specs/protocol.md` |
| Security model | `specs/security_model.md` |
| RPC/debug skill | `.claude/skills/doli-network/SKILL.md` |
| Drift tracker | auto-memory `MEMORY.md` (in `~/.claude/projects/<project>/memory/`, NOT the repo) |
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
- **DB Detection**: `test -f .omega/memory.db` at start. If missing, skip memory ops.
- **Session briefing = behavioral learnings + open incidents + active invariants**. Decisions/bug details/outcomes/hotspots are on-demand.
- **Briefing before action**: Every agent queries memory.db for scope-specific context (hotspots, failed approaches, findings, decisions, patterns, invariants) before starting.
- **Log incrementally**: Write to memory.db after each significant action. Never batch — context compaction loses batched entries.
- **Self-score every action**: Rate significant actions (-1/0/+1) immediately after completing them.
- **Track bugs as incidents**: Every bug gets an INC-{PREFIX}-NNN ticket (prefix from `user_profile.contributor_prefix`). Use `--incident` on doctor to resume. Read @INDEX of `.claude/protocols/incident-protocol.md`.
- **Extract invariants from bugs**: Level 2+ incidents must produce an `invariants` record (INV-{DOMAIN}-NNN) + linked `regression_tests`. Level 3 also requires `monitoring_signals`. Query `v_regression_map` before modifying files with linked invariants. Read `.claude/protocols/invariant-protocol.md`.
- **Extract behavioral learnings**: When the user corrects you or an incident reveals a reasoning flaw, extract a behavioral rule (HOW to think, not domain patterns).
- **Close-out**: Verify completeness, distill lessons, track bugs as incidents.
- **Pipeline tracking**: Code-modifying `/omega-*` commands register `workflow_runs` (start + end); informal work uses type `'manual'`; read-only commands skip.
- **sqlite3 quoting**: Use heredoc (`<<'EOF' ... EOF`). Inline single-quote wrapping breaks `datetime('now')`.
- **Self-correction**: When the user corrects you, save a behavioral learning immediately and fix.
- **Error tolerance**: If sqlite3 fails, log and continue. Never block work for a DB failure.

### Canonical SQL — copy verbatim, do not guess columns

Before composing ANY query against `.omega/memory.db`, use one of these or run `.schema TABLE` first. The full catalog (decisions, failed_approaches, bugs, findings, patterns, artifacts, hotspots, lessons, incidents, invariants) lives in `core/protocols/memory-protocol.md`.

```bash
# Start a run — INSERT and rowid capture MUST share one sqlite3 invocation (per-connection)
RUN_ID=$(sqlite3 .omega/memory.db "INSERT INTO workflow_runs (type, description, scope) VALUES ('TYPE', 'DESCRIPTION', 'SCOPE_OR_NULL'); SELECT last_insert_rowid();")
# Self-score an action (score ∈ {-1,0,1}; run_id may be NULL)
sqlite3 .omega/memory.db "INSERT INTO outcomes (run_id, agent, score, domain, action, lesson) VALUES (NULL, 'AGENT', -1, 'DOMAIN', 'What I did', 'What I learned');"
# Behavioral learning (NO score column — uses confidence + occurrences; UNIQUE on rule)
sqlite3 .omega/memory.db "INSERT INTO behavioral_learnings (rule, context) VALUES ('THE_RULE', 'What triggered it') ON CONFLICT(rule) DO UPDATE SET occurrences = occurrences + 1, confidence = MIN(1.0, confidence + 0.1), last_reinforced = datetime('now');"
# Close a run
sqlite3 .omega/memory.db "UPDATE workflow_runs SET status='completed', completed_at=datetime('now') WHERE id=$RUN_ID;"
```

Column reminders that trip agents up:
- `behavioral_learnings` has **no** `score` column — it has `confidence` (REAL) and `occurrences` (INT). `score` lives on `outcomes`.
- `outcomes.score` is constrained to `(-1, 0, 1)` — any other value rejects.
- Use heredoc (`<<'EOF' ... EOF`) for multi-statement scripts; inline single-quoting breaks `datetime('now')`.

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
16. **Read-only agents stay in their lane** — research agents (codebase-expert) NEVER offer to implement. They report findings and suggest appropriate commands
17. **Security chain** — features touching external data: 5-agent independent verification (Analyst REQ-*-SEC, Architect trust-boundaries, Test Writer injection, QA probing, Reviewer pattern scan — blocker)
18. **Stupid Simple First (SSF)** — before ANY design work, state the simplest one-sentence solution that resolves the root cause (fewest moving parts, never a symptom patch). Present it ALONE. Only add complexity if the user rejects it with a specific reason. Never present a menu. Read `.claude/protocols/anti-overengineering.md`
19. **Modular coding enforcement** — no source file exceeds 500 lines (800 for test files). When approaching the limit, split into focused modules. Read MODULE-SIZE-BUDGET in `.claude/protocols/anti-overengineering.md`
20. **Intellectual honesty** — STOP on self-contradiction. Show your work on math/logic claims. State what you don't understand before proposing. Try to disprove your own hypotheses before acting. Max 2 inferences without verification. Read `.claude/protocols/intellectual-honesty.md`
21. **Output Contract** — before any test assertion, produce Output Contract Checklist (outputs × paths × input partitions). Fix confidence >0.7 requires FAIL→PASS test evidence. **Test BEFORE fix** — reproduction test exists and FAILS before any fix code or fix plan. Read `.claude/protocols/output-contract.md`
22. **Prompt refinement at intake** — every omega command with a user description refines BEFORE agent work (neutralize anchoring, reframe causes as hypotheses; REGRESSION CONTEXT triggers git archeology). Read `.claude/protocols/prompt-refinement.md`
23. **Evidence floor** — diagnostic synthesizers publish `VERDICT` or `PRELIMINARY`; reviewers/auditors give per-finding evidence pointers and open FINAL reports with a canonical `━━━ FINDINGS` summary block. Blocking-enforced by `evidence-floor-gate.sh` (the block message states the required shape). **Present findings by quoting the gate-passed block verbatim, never narrating it** — blocking-enforced by `verdict-quote-gate.sh` (Stop), armed by `verdict-arm.sh`. Read `.claude/protocols/evidence-floor.md`
24. **Path-Coverage attestation** — new early-return guards in non-test Rust need a per-branch `Path-Coverage:` commit block. Blocking-enforced by `path-coverage-gate.sh` (the block message states the format). Read `.claude/protocols/path-coverage.md`
25. **Communication style** — user-facing replies use BLUF + Progressive Disclosure + Cognitive Load (≤4 items/turn). Shape: 1 sentence bottom line + up to 3 sentences action + 1 question. Hold complexity in files. Read `.claude/protocols/communication-style.md`
26. **Resource cost** — every proposal (architect, design-evaluator, design-synthesizer, reviewer) carries a `━━━ RESOURCE COST` block. Blocking-enforced by `resource-cost-gate.sh` (the block message states the required dimensions). Read `.claude/protocols/resource-cost.md`
27. **Evidence pivot** — a failed fix buys evidence, not another guess: when a shipped fix does not move the symptom, capture runtime evidence from the FAILING environment before editing source again. Blocking-enforced by `pipeline-gate.sh` (armed by `evidence-pivot.sh`). Read `.claude/protocols/evidence-pivot.md`
28. **Code graph for structural questions** — answer ANY dependency/blast-radius/caller-callee/architecture question from the code graph BEFORE grep (`blast.py`, `graphify explain|path`); grep is the fallback only when graphify cannot be provisioned. Blocking-enforced by `graph-first-gate.sh`. Read `.claude/protocols/graph-briefing.md`
29. **System impact** — in projects with `.omega/gauntlet.conf`, "done" is a SYSTEM property, not a diff property: system-dynamics changes need a failure-mode matrix at briefing, a `Failure-Modes:` commit block, protection registration, and a passing gauntlet run before close. Blocking-enforced by `gauntlet-gate.sh`. Read `.claude/protocols/system-impact.md`
29. **Requirements before code** — a greenfield project gets no source file until `specs/*-requirements.md` exists: run `/omega-new` or `/omega-new-feature` first. Blocking-enforced by `pipeline-gate.sh` (specs-first gate); existing codebases self-exempt on first contact

## Fail-Safe Controls

**Full reference:** Read @INDEX of `.claude/protocols/fail-safes.md` for section lookup.

**Core rules:** Prerequisite gates (agents verify upstream output). Iteration limits (QA↔Dev: 3, Reviewer↔Dev: 2, Audit fix: 5). Error recovery to `docs/.workflow/chain-state.md`. Developer max 5 retries/module.

## Context Efficiency (ENFORCED)

**Full reference:** Read @INDEX of `.claude/protocols/context-budget.md` for section lookup.

**Rules:** CLAUDE.md MUST stay under 18,000 characters. Never inline templates/SQL/procedures — put them in `core/protocols/` and reference. Never duplicate content across files. Lazy-load protocol sections via @INDEX (first N lines) then Read with offset/limit. 60% context budget per agent. Never read the entire codebase.

## Traceability Chain
```
Discovery → Analyst (REQ-XXX-001) → Architect (module map) → Test Writer (TEST-XXX-001) → Developer → QA (acceptance criteria) → Reviewer (completeness)
```

## Project Layout
```
root-project/
├── backend/ frontend/    ← source code
├── specs/  docs/         ← specifications, documentation
├── CLAUDE.md             ← workflow rules
├── .omega/memory.db      ← institutional memory (SQLite)
└── .claude/              ← agents/, commands/, protocols/, db-queries/
```

## Conventions
- Preferred language: Rust (or whatever the user defines)
- Tests: alongside code or in `backend/tests/` (or `frontend/tests/`)
- Commits: conventional (feat:, fix:, docs:, refactor:, test:)
- Branches: feature/, bugfix/, hotfix/
