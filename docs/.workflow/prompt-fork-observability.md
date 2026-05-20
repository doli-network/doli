# Mission: End the Fork-Diagnosis Suffering — Build Trivial-Mode Observability for DOLI

> This is a fresh session. Read this whole brief carefully. Codebase is at
> `/Users/isudoajl/ownCloud/Projects/doli-network/doli`. Testnet is LOCAL
> (`~/testnet/`, launchd, 127.0.0.1 — NEVER ssh ai1–ai5). Read `CLAUDE.md` and
> `MEMORY.md` before touching anything.

## The pain (the problem you are solving)

Every time a DOLI node falls "BEHIND" or forks, the operator spends hours doing the same forensic ritual: grep across 18 log files (some 1.9 GB), manually correlate timestamps across nodes, RPC-poll each node for state-root debug, infer divergence point by comparing block hashes by hand, re-read 360-line `recovery.rs` to figure out why `classify()` picked an action, and reconstruct causality from "chain break valid_so_far=0" lines that *don't tell you which producer caused the fork or when the local block was applied or from which peer*. Then write a multi-page report that takes 4 hours and is *still* wrong half the time because the trajectory was reconstructed from endpoints. This has happened at least 30 times across the incident log (INC-I-009 → INC-I-083). It is the single biggest waste of operator time on the project.

**Two recent failure modes that prove the gap:**
- INC-I-083 (2026-05-19): 5/18 testnet nodes frozen with sync_fails climbing into the 200–360 range. Diagnosis required spawning 4 parallel domain investigators + a synthesizer + ~2 hours of grep, log forensics, and code reading — and STILL initially concluded "the bundle is innocent" while the user's baseline ("mainnet stable for weeks on 77bb3dfa, only INC was an invalid epoch boundary block, now testnet shows BEHIND divergence regression") was the controlling evidence the whole time. The investigator could not directly answer "how many fork events per hour happened pre-deploy vs post-deploy?" — that question is unanswerable today because no counter exists.
- "Behind then recover, behind then recover, over and over" — the operator can SEE this on the dashboard but cannot prove whether it's normal PoS rhythm or a regression because there is no per-node fork-event counter, no fork-frequency baseline, no fork-event ledger.

**The depressing pattern:** every fork diagnosis ends with the same realization — "if there had just been a single log line per fork event with all the context, this would have taken 30 seconds." Yet we keep not building it. End that here.

## The bar you are setting

After your work lands, the next time someone says "this wasn't happening before," the answer takes **5 seconds**:

```bash
$ doli --rpc http://127.0.0.1:8500 forks --last 1h
fork_events_last_hour: 47
fork_events_baseline_24h_avg: 2.1
verdict: REGRESSION (22x baseline)
top_producers_implicated: [98951083 (12), b03fe629 (8), 2d27fdcc6a24 (5)]
slot_distribution: epoch_boundary=31, mid_epoch=16
```

Not 4 hours of grep. Not "let me spawn 4 investigators." Five seconds, structured answer, decisive.

## Starting-point ideas (the floor, not the ceiling)

The orchestrator (me, prior session) proposed three things. Use them as the **floor**. Improve, extend, replace, or invert them as your judgment dictates:

**(1) Single-line structured FORK_EVENT log entry**, emitted exactly once per divergence on the producing node, with ALL context: `kind=ENTERED|RECOVERED`, `node`, `div_height`, `div_slot`, local_block=(hash, producer, applied_at, from_peer), canonical_block=(hash, producer, first_seen, from_peer), classify_ctx full struct dump, decision (action + reason + gate), monotonic counter. One line per fork. Whole forensics in one grep.

**(2) Append-only per-node fork ledger** at `<data_dir>/fork_log.jsonl` — same record as the log line but structured JSON. Survives restarts. Bounded retention (last 10,000 events or 30 days).

**(3) New RPC method `getForkForensics(window_secs)`** returning the recent ledger window plus aggregations (count, by-producer, by-slot-type, by-decision, average time-to-recover). Plus a dashboard tile reading the aggregations from every node.

These are the MINIMUM. The user explicitly said *"push the agent to think about whether they can add even more ingenious things related to this too... the sky's the limit."*

## What to think bigger about (provocations — not a checklist, a creativity prompt)

Brainstorm and pick the highest-leverage subset. You do not have to do all of these; you DO have to consider them and explain in your design what you kept, what you rejected, and why.

- **Time-machine RPC**: `getApplySequence(from_slot, to_slot)` returns the ordered list of blocks this node applied, with: slot, height, hash, producer, received_from_peer, received_at_ms, applied_at_ms, validation_duration_ms, peer_count_at_apply, gossip_arrival_latency. Lets you literally replay what each node *saw* in chronological order without ever opening a log file.

- **Pairwise divergence query**: `compareWithPeer(peer_rpc)` returns the first height where this node's chain diverges from a peer's chain, the two competing blocks, both producers, and a recommendation. Diagnose "n14 vs canonical" in one RPC call.

- **Causality DAG / fork tree**: maintain a per-node tree of `(slot, hash, parent_hash, producer, applied|orphaned|rolled_back)` for the last N slots. `getForkTree()` returns it. Render in the explorer as an actual visual tree where you SEE the branches. A picture of the divergence beats a paragraph describing it.

- **Producer fork-attribution scoring**: every fork event credits the producers of both competing blocks. Aggregate per-producer: "producer X is implicated in N forks in last 24h." Surfaces broken-producer cases (e.g. INC-I-081's broken-producer-emitting-bad-epoch-block) automatically without any incident at all.

- **Auto-classifier of fork type**: each FORK_EVENT carries a `classification` field from a deterministic rule set: `tip_race_natural | tip_race_high_latency | producer_equivocation | epoch_boundary_invalid | post_snap_dead_tip | validation_disagreement`. The classifier is just a `match` on the recorded context. No ML. Pure structured logic. Once the classifier exists, the answer to "what kind of fork is this" is a `jq` filter, not a meeting.

- **Reorg play-by-play**: when a reorg happens, emit one structured `[REORG_EVENT]` line with the full causal chain: trigger_block (received from peer P at time T), old_chain_unwound (heights H1..H2, blocks B1..Bn, producers P1..Pn), new_chain_applied (heights H1..H2', blocks B1'..Bn', producers P1'..Pn'), state_root_changes_per_step. Today reorgs are detectable only by inferring from `[ROLLBACK]` + subsequent apply lines. Make it explicit.

- **Recovery decision trace**: every call to `RecoveryCoordinator::classify()` and every dispatch attempt to `request_genesis_resync()` should produce ONE line containing the full RecoveryContext (gap, last_applied_secs, recently_synced, shallow_rollback_count, snap_attempts, peer_count, in_grace_period, last_finality_height, empty_count, deep_fork, rollback_exhausted, large_gap, rule_matched) and the result (action + reason). One line. Then `recovery.rs:252-363` is no longer a 360-line mystery — every decision has its receipts.

- **Cross-fleet fork-correlation RPC**: dashboard query that for each fork height in the last hour, lists which nodes were on the minority branch and how long each took to recover. Surfaces "5 nodes always lose the tip race together" patterns instantly — a clue that there's a topology or peer-scoring bias.

- **Baseline auto-capture**: after a clean genesis reset, the node persists a `baseline_fork_rate.json` after 1 hour of steady operation. Future fork-rate queries return both `current` and `baseline_delta`. The system answers "is this happening more than before" by default, not by manual SQL.

- **Fork honeypot mode**: a debug flag that intentionally introduces 100 ms apply jitter on N% of received blocks to provoke tip races. Lets a test reliably reproduce the conditions a regression would cause, on demand, in a single-machine test harness. Then any commit can be A/B-tested for fork rate before merge.

- **Block provenance trail**: each applied block stores `from_peer_id`, `received_at_ms`, `gossip_hop_count_estimate`. Then per-block forensics ("who told n14 about this block first?") is a single query, not a multi-node log grep.

- **Witness Mode for the orchestrator (Claude)**: a single `doli forensics report --last-incident` command that produces a structured Markdown report from the ledger — divergence point, competing blocks, producer attribution, decision trace, recovery duration, peer topology at the moment, and the verdict. Eliminates the multi-agent investigation entirely for routine cases.

- **Inverse-thinking**: instead of detecting forks AFTER they happen, detect *fork preconditions* — gossip propagation latency exceeding a threshold, two competing blocks for the same slot seen within W ms, a peer whose tip diverges from the local tip but whose chain is heavier. Pre-fork warnings give operators a chance to act before the deadlock starts.

- **What is missing that I haven't thought of?** This is the most important question. The user explicitly said "sky's the limit." Spend real thinking budget here. The list above is the orchestrator's brainstorm — you should produce your own list and argue for it.

## Hard constraints (non-negotiable)

1. **No consensus impact.** Pure observability. No change to apply_block, validation, state-root computation, peer scoring, fork choice, attestation, or any code that decides what is canonical. Read-only instrumentation only. If any proposed feature requires a consensus change, mark it explicitly and put it in a follow-up section — do not implement.
2. **Safe for rolling deploy.** No block-content change (INC-I-062 rule). No activation height (INC-I-075). The TWO deploy-safety questions in `MEMORY.md` #0 both answer NO.
3. **No mainnet contamination.** Ship as `cargo build --release` artifact, deploy to testnet first, validate baseline, then mainnet ops can adopt at their own pace.
4. **Bounded storage.** The per-node ledger must self-cap. Rotate after 30 days or 10,000 events, configurable. Never unbounded growth.
5. **Privacy / no PII.** Peer IDs are public libp2p multihashes — fine to log. Do not log wallet keys, raw transactions, or peer IP addresses (libp2p PeerId is OK, raw IP is not).
6. **Test discipline.** Every new RPC method gets a unit test. The FORK_EVENT emitter gets a regression test that reproduces a tip-race in a unit-test harness and asserts the event payload. Run `cargo test -p doli-core -p storage -p network -p rpc` clean. Run `cargo clippy --workspace --all-targets -- -D warnings` clean. Run `cargo fmt --check` clean.
7. **Documentation in sync.** Update `docs/rpc_reference.md` for every new RPC. Update `docs/troubleshooting.md` with the new "how to diagnose a fork in 5 seconds" workflow. Add a new `docs/fork_observability.md` if substantial.
8. **`/sync-docs` at the end.** Run it. Commit message must include the three-question consensus-shape checklist from `CLAUDE.md` — even though every answer should be NO/NO/YES (no consensus impact), explicitly write them out so the next reviewer knows you checked.
9. **No genesis reset.** Per `CLAUDE.md` #0 RULE.
10. **DO NOT modify**: `crates/core/src/consensus.rs`, `crates/core/src/network_params/defaults.rs` activation heights, `bins/node/src/node/apply_block/*`, `crates/storage/src/snapshot.rs` (state root logic), `crates/core/src/validation/*` (consensus rules). You may READ them. You may add NEW observability calls FROM them (e.g., `apply_block` can call `fork_ledger.record_apply(...)`), but you may not change their decision logic.

## Codebase pointers (start here, not by guessing)

| Subsystem | Where |
|---|---|
| RecoveryCoordinator + classify() + thresholds | `crates/network/src/sync/manager/recovery.rs` (~720 lines) |
| Request dispatch gates (request_genesis_resync) | `crates/network/src/sync/manager/production_gate.rs:608-688` |
| Header sync response + chain break | `crates/network/src/sync/manager/sync_engine/response.rs` (Empty headers + Chain break logs originate here) |
| Block apply (record provenance here) | `bins/node/src/node/apply_block/` (multi-file module) |
| Block handling + reorg dispatch | `bins/node/src/node/block_handling.rs`, `bins/node/src/node/fork_recovery.rs` |
| Rollback path | `bins/node/src/node/rollback.rs` |
| Periodic task (HEALTH lines, coordinator dispatch) | `bins/node/src/node/periodic.rs` |
| RPC method registry | `crates/rpc/src/methods/` |
| Storage (where to persist the ledger) | `crates/storage/src/` — pick `~/testnet/<node>/data/fork_log.jsonl` or a dedicated RocksDB column family — argue your choice |
| Explorer dashboard | `~/ownCloud/Projects/doli-network/doli/testnetlinux/explorer/` (server.js + network.html — small JS, port 8080) |

## Evidence on disk (use it — do not start from scratch)

- Existing fork incident, all 5 frozen nodes plus 2 advancing clusters: `docs/.workflow/inc-i-083-session-handoff.md` and the deeper analysis at `docs/.workflow/domain-investigation-{fork,connectivity,parameters,code}.md` + the synthesis at `docs/.workflow/domain-diagnosis-report.md`.
- Live logs of the recent freeze + recovery: `~/testnet/logs/n10.log` (now ~1 GB; PID changed at 23:09, snap-synced to canonical in 90s), `~/testnet/logs/n14.log` (still stuck at h=110,596 due to wipe missing `nodes1/n14/data`), `~/testnet/logs/seed.log` (healthy advance).
- INC-I-081 postmortem and the broader fork history in `docs/postmortems/` and `MEMORY.md`.
- Use these to *calibrate* what your FORK_EVENT schema must capture so that, retroactively, INC-I-083 and INC-I-081 would have been diagnosed in 5 seconds. If your schema cannot retroactively explain a real past incident, the schema is wrong.

## Deliverables

1. **Design doc** at `docs/fork_observability.md` — what you decided to build, what you considered and rejected, the FORK_EVENT schema in full, the ledger format, the RPC API, the dashboard changes, the test plan, the rollout plan.
2. **Implementation** across the relevant crates, modular (no file over 500 lines per `CLAUDE.md`).
3. **Tests** — unit tests for the emitter, the ledger I/O, the RPC method, the auto-classifier (if you build it), the cross-fleet correlation (if you build it). Reproduction test for at least one historical incident (INC-I-083) using captured log fixtures.
4. **Updated docs** — `docs/rpc_reference.md`, `docs/troubleshooting.md`, plus the new `docs/fork_observability.md`.
5. **Skill update** to `.claude/commands/testnet.md` with the new `forks` subcommand and the 5-second diagnostic workflow.
6. **A one-pager** at `docs/fork_observability_quickstart.md` titled "How to diagnose a fork in 5 seconds" — shows the three commands an operator runs and the expected output. This is what makes the work feel real to users.

## Process expectations

- **Read first, code second.** Spend the first hour reading the three referenced INC docs, the recovery.rs/production_gate.rs code, and the existing RPC method registry. Then design. Then implement.
- **Argue your choices.** For every brainstorm provocation above, take a position: kept, rejected, deferred. Don't silently drop ideas — defend them.
- **Build for the operator, not for the developer.** The end user is someone who saw a node go BEHIND, who is panicking, who wants a verdict in 5 seconds. The API should answer THEIR question, not require them to learn a query language.
- **Show the receipts.** Every RPC must include an example invocation and expected output in the docs.
- **Validate retroactively.** Take the n10 log from INC-I-083 (`~/testnet/logs/n10.log`), feed the historical chain-break events into your emitter offline, and prove the resulting ledger would have produced the right diagnosis. If yes, ship. If no, the schema is incomplete.
- **No multi-agent investigation.** Build it yourself. The whole point of this work is to make the next investigation NOT need multi-agent forensics.

## Anti-patterns to avoid (the orchestrator has fallen into these)

- "Let me add more logging" — without structure, more logs is more grep, not less. Every new emitter MUST be structured records, not free-text WARN lines.
- "Let me build a Grafana dashboard" — Prometheus metrics are fine for counters, but they cannot answer "which producer caused which fork." You need the per-event record, not just the aggregate.
- "This is normal PoS behavior" — the only valid version of that sentence is "this is within N% of the pre-deploy baseline as measured by `fork_events_per_hour`." Without numeric baseline comparison, "normal" is hand-waving.
- "I'll spawn 4 investigators" — if the answer requires 4 agents and 2 hours, the observability is broken. Fix the observability instead of pipelining around it.
- Premature abstraction — keep the schema flat. Each FORK_EVENT is ONE JSON line. No nested objects, no polymorphic variants, no inheritance.
- "Configurable via env vars" — only if there's a real operator need. Default-ON for the observability, default-OFF for any debug honeypot mode.

## The success test

When you are done, the operator runs ONE command after seeing a node go BEHIND:

```
$ doli --rpc http://127.0.0.1:8514 forks --explain
```

And receives a complete, structured, actionable verdict in 5 seconds that includes the divergence height, the two competing blocks with their producers, the recovery decision trace, the classification (natural tip race / regression / equivocation / dead-fork), the baseline comparison, and the recommended action — operational fix vs code investigation. Then the operator runs the recommended action and the node recovers, all without ever opening a log file or running a grep.

That is the only success criterion. If the next fork still requires a 4-hour multi-agent investigation, the work failed.

## Final reminder

The user's words, verbatim: *"the sky's the limit, enough of this when you know it's always too difficult to figure out what the problem is, doing thousands of greps, thousands of logs, and the analyses are always horribly depressing."*

Build the thing that makes the next fork diagnosis a non-event. Do not be timid. Do not under-scope. The cost of overshooting is a few extra files. The cost of undershooting is another four hours of grep, another depressing report, and another operator burnout. Err toward the ambitious.

Go.
