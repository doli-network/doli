# Mission: ChainBreakLoop classifier rule (Phase 1.5 hotfix)

> Fresh session. This brief is self-contained. Codebase at
> `/Users/isudoajl/ownCloud/Projects/doli-network/doli`. Testnet is LOCAL
> (`~/testnet/`, launchd, 127.0.0.1). Phase 1 + Phase 2a are on branch
> `feature/fork-observability-346` (HEAD: `87dee5e9` at brief-write time).
> This workflow adds ONE classifier rule + ONE enum variant. Small, focused,
> high-impact. Estimated scope: ~150 LoC + 2 fixture tests + docs update.

---

## The Problem

Workflow #347's M4 fixture tests revealed a gap in the classifier the dev team
acknowledged in their own commit message:

| Incident | Classifier verdict | Catch? |
|----------|-------------------|--------|
| INC-I-081 (broken producer, missing EpochReward) | `EpochBoundaryInvalid` (rule b) | ✅ |
| INC-I-083 (post-snap chain-break loop) | `Unknown` — no rule matched | ❌ |

**Today's testnet is the live proof.** n6 is stuck in the same INC-I-083 pattern
RIGHT NOW (height 115219 while fleet at 115269, frozen). When queried, n6's
RPC returns:

```
Classification: TipRaceNatural confidence 0.70
Recommended action: normal_operation
```

This is **catastrophically wrong**. The action routes the operator AWAY from
the fix. The underlying signals are unambiguous:

- **1299 ForkBlockReceived** events in 1 hour (vs ~43× more than BlockApplied)
- **241 RecoveryClassifyCall** events (recovery state machine churning)
- **42 RollbackStarted** events (spread across hour, so rule (c) doesn't fire)
- **30 BlockApplied** total — the node is stuck

Rule (f) `TipRaceNatural` fired because: a `ForkBlockReceived` event matched,
its corresponding `BlockApplied` had `validation_duration_ms < 500`, and rule (f)'s
"no other signals in same correlation_key group" check found no other
fork-classified events in THAT specific correlation_key — even though there
are 1299 fork events across the window in DIFFERENT correlation_keys. The
rule's locality check is too narrow.

**A confidently-wrong named verdict is more harmful than `Unknown` with
evidence.** Skeptic Reframe 4 from the original Phase 1 review predicted this
exact failure mode. n6 is the existence proof.

---

## What's Already on Disk (do NOT re-build)

Branch HEAD: `87dee5e9`. The classifier already has 7 rules in
`crates/storage/src/diagnostic_ledger/classifier.rs`. Rule (g) is the catch-all
`Unknown`.

**Fixtures captured BEFORE n6 changed state (time-sensitive, will not reproduce
on demand):**

| Path | What | Size |
|------|------|------|
| `crates/storage/tests/fixtures/inc-n6-chain-break-loop.json` | Live n6 DiagnosticBundle (1h window, post-snap chain-break loop) | ~1.4 MB |
| `crates/storage/tests/fixtures/inc-n6-stuck.log` | Tail of n6's raw log (5000 lines, last hour of stuck state) | ~1.1 MB |
| `crates/storage/tests/fixtures/inc-i-083-n10.fixture` | The earlier n10 log capture from May 19 (already replays to `Unknown`) | ~42 KB |
| `crates/storage/tests/fixtures/inc-i-081-broken-producer.fixture` | INC-I-081 epoch-boundary log (already replays correctly to `EpochBoundaryInvalid`) | ~2 KB |

**Key signals already in the n6 bundle** (`jq` to verify):
```bash
jq '.fork_summary' crates/storage/tests/fixtures/inc-n6-chain-break-loop.json
# Confirms: 1582 fork_events_in_window, 1299 ForkBlockReceived, 241 RecoveryClassifyCall,
# 42 RollbackStarted/Completed, 30 BlockApplied
```

---

## Phase 1.5 Mission

Add ONE classifier rule that correctly diagnoses INC-I-083-class chain-break
loops, and verify it against BOTH the live-captured n6 fixture AND the
historical INC-I-083 fixture. Both must classify as the new variant.

### Deliverable 1 — `ForkType::ChainBreakLoop` variant

Add to `crates/storage/src/diagnostic_ledger/types.rs`:

```rust
pub enum ForkType {
    // ... existing variants ...

    /// Node is stuck in a chain-break or empty-header loop. Local tip is not
    /// advancing despite repeated sync attempts. The recommended fix is to
    /// reset the data directory and snap-sync from a majority peer.
    ChainBreakLoop {
        chain_break_count: u32,        // # of ChainBreakDetected events in window
        empty_header_count: u32,       // # of RecoveryClassifyCall with empty_count > 0
        seconds_stuck: u64,            // window over which local tip did not advance
        rollback_count: u32,           // # of RollbackStarted events in window
    },
}
```

This is an additive change — existing serialized events with old variants
continue to deserialize (the format-marker byte + schema_version mechanism
already in place).

### Deliverable 2 — Rule (h) `ChainBreakLoop` in the classifier

Add to `crates/storage/src/diagnostic_ledger/classifier.rs`. **Insert BEFORE
rules (e) and (f)** in the first-match-wins precedence order — this is the
critical ordering fix. Stuck-state signals must preempt timing-based verdicts
(see why above).

```rust
// Rule (h) — Chain-break loop. Fires when the node is repeatedly observing
// chain breaks or getting empty header responses without making forward
// progress. Must fire BEFORE rules (e)/(f) because timing-based rules will
// incorrectly match individual fork events in this scenario.
fn rule_h_chain_break_loop(events: &[DiagnosticEvent], now_ms: u64) -> Option<Classification> {
    // Look at the most recent <window_secs> of events. Default 1 hour.
    const WINDOW_SECS: u64 = 3600;
    let window_start = now_ms.saturating_sub(WINDOW_SECS * 1000);
    let recent: Vec<&DiagnosticEvent> = events.iter()
        .filter(|e| e.timestamp_ms >= window_start)
        .collect();

    let chain_break_count = recent.iter()
        .filter(|e| matches!(e.kind, EventKind::ChainBreakDetected))
        .count() as u32;

    let block_applied_count = recent.iter()
        .filter(|e| matches!(e.kind, EventKind::BlockApplied))
        .count() as u32;

    let fork_block_received_count = recent.iter()
        .filter(|e| matches!(e.kind, EventKind::ForkBlockReceived))
        .count() as u32;

    let rollback_count = recent.iter()
        .filter(|e| matches!(e.kind, EventKind::RollbackStarted))
        .count() as u32;

    // Recovery events with empty_count > 0 (peers returning empty header responses)
    let empty_header_count = recent.iter()
        .filter(|e| {
            if let EventPayload::RecoveryClassifyCall { empty_count, .. } = &e.payload {
                *empty_count > 0
            } else {
                false
            }
        })
        .count() as u32;

    // Triggers (any of these is sufficient — chain-break loop is multi-modal):
    //   (1) >3 ChainBreakDetected in window
    //   (2) fork_block_received >> block_applied (ratio >10x) AND >100 fork blocks total
    //   (3) >10 RollbackStarted in window (loose threshold — rule (c) catches the tight 60s case)
    //   (4) empty_header_count > 20 (peers can't return useful headers)
    let signal_a = chain_break_count > 3;
    let signal_b = fork_block_received_count > 100
        && block_applied_count > 0
        && fork_block_received_count / block_applied_count.max(1) > 10;
    let signal_c = rollback_count > 10;
    let signal_d = empty_header_count > 20;

    if !(signal_a || signal_b || signal_c || signal_d) {
        return None;
    }

    // Compute seconds_stuck: time since the most recent BlockApplied (if any) or
    // since the window start (if no BlockApplied at all).
    let last_applied_ms = recent.iter()
        .filter(|e| matches!(e.kind, EventKind::BlockApplied))
        .map(|e| e.timestamp_ms)
        .max()
        .unwrap_or(window_start);
    let seconds_stuck = (now_ms.saturating_sub(last_applied_ms)) / 1000;

    // Build evidence: include up to 10 representative events from each signal source.
    let evidence_event_ids: Vec<String> = recent.iter()
        .filter(|e| matches!(
            e.kind,
            EventKind::ChainBreakDetected
                | EventKind::RecoveryClassifyCall
                | EventKind::RollbackStarted
        ))
        .take(20)
        .map(|e| e.event_id.clone())
        .collect();

    Some(Classification {
        fork_type: ForkType::ChainBreakLoop {
            chain_break_count,
            empty_header_count,
            seconds_stuck,
            rollback_count,
        },
        confidence: 0.85,
        evidence_event_ids,
        recommended_action: Some("restart_with_resync".to_string()),
        recommended_action_args: Some(serde_json::json!({
            "approach": "stop_node + rm -rf <data_dir>/{blocks,state_db,utxo,diagnostics} + restart with --no-snap=false",
            "preserve": ["wallet.json", "producer.seed.txt"],
            "verify_after": "doli forks --explain --human after 10 minutes of sync",
        })),
    })
}

// In the main classify() function, insert the rule BEFORE the existing (e) and (f):
pub fn classify(events: &[DiagnosticEvent]) -> Classification {
    let now_ms = current_time_ms();  // or pull from latest event timestamp

    if let Some(c) = rule_a_producer_equivocation(events) { return c; }
    if let Some(c) = rule_b_epoch_boundary_invalid(events) { return c; }
    if let Some(c) = rule_c_rollback_loop(events) { return c; }
    if let Some(c) = rule_d_post_snap_dead_tip(events) { return c; }
    if let Some(c) = rule_h_chain_break_loop(events, now_ms) { return c; }  // <-- NEW
    if let Some(c) = rule_e_tip_race_high_latency(events) { return c; }
    if let Some(c) = rule_f_tip_race_natural(events) { return c; }
    // ... rule (g) Unknown is the catch-all
}
```

> **Verify the exact function signatures** in the current classifier.rs —
> the snippet above is illustrative. The actual rule functions may take
> additional context (e.g., now_ms might come from a `now: SystemTime`
> parameter, or from the latest event timestamp). Read the file first.

### Deliverable 3 — Fixture-replay tests

Add to `crates/storage/tests/diagnostic_classifier_test.rs`:

```rust
#[test]
fn test_rule_h_chain_break_loop_n6_live_fixture() {
    // Captured from live testnet n6 (workflow #349, 2026-05-20).
    // n6 was stuck in INC-I-083 pattern: post-snap chain-break loop with
    // 1299 ForkBlockReceived, 241 RecoveryClassifyCall, 42 rollbacks,
    // only 30 BlockApplied over 1 hour window.
    let bundle_json = include_str!("fixtures/inc-n6-chain-break-loop.json");
    let bundle: DiagnosticBundle = serde_json::from_str(bundle_json)
        .expect("fixture must parse as valid DiagnosticBundle");

    let classification = classify(&bundle.events);

    // Must classify as ChainBreakLoop, NOT TipRaceNatural (the bug we're fixing)
    assert!(
        matches!(classification.fork_type, ForkType::ChainBreakLoop { .. }),
        "n6 fixture must classify as ChainBreakLoop, got {:?}",
        classification.fork_type
    );

    // The recommended action must point at the right fix
    assert_eq!(
        classification.recommended_action.as_deref(),
        Some("restart_with_resync"),
        "ChainBreakLoop must recommend restart_with_resync"
    );

    // Evidence must include ChainBreakDetected and/or RecoveryClassifyCall events
    assert!(
        !classification.evidence_event_ids.is_empty(),
        "ChainBreakLoop must carry evidence event IDs"
    );
}

#[test]
fn test_rule_h_chain_break_loop_inc_i_083_historical_fixture() {
    // The n10 log captured from the May 19 incident.
    // Workflow #347's M4 fixture test classified this as Unknown.
    // Workflow #349 must flip the verdict to ChainBreakLoop.
    //
    // NOTE: this fixture is a RAW LOG, not a JSON bundle. It must first
    // be run through the replay tool (workflow #347's M3 parser) to
    // produce DiagnosticEvents, then through classify().
    let log = include_str!("fixtures/inc-i-083-n10.fixture");
    let events = doli_cli::cmd_forks_replay::parse_log_to_events(log);  // exact path TBD

    let classification = classify(&events);

    assert!(
        matches!(classification.fork_type, ForkType::ChainBreakLoop { .. }),
        "INC-I-083 historical fixture must classify as ChainBreakLoop, got {:?}",
        classification.fork_type
    );
}
```

> If the replay parser is in `bins/cli/src/cmd_forks_replay.rs` (per workflow
> #347's M3), expose `pub fn parse_log_to_events(log: &str) -> Vec<DiagnosticEvent>`
> for test access. If it's not yet exposed as `pub`, do that as part of #349.

### Deliverable 4 — Docs update

Update `docs/fork_observability.md`:

1. Add `ChainBreakLoop` to the ForkType variants table (after `RollbackLoop`,
   before `Unknown`) with the same column structure as existing variants
   (variant | meaning | recommended_action).

2. Update the classification rules section to document rule (h) and its
   placement BEFORE rules (e)/(f).

3. Note in "Retroactive Validation" section that INC-I-083 now classifies
   correctly as ChainBreakLoop (close the open item from workflow #347).

Update `.claude/skills/testnet-deploy/SKILL.md` and `.claude/skills/mainnet/SKILL.md`:
add `ChainBreakLoop` to the "Expected outcomes" table in the post-deploy
verification section. Tag as STOP-class (do not proceed to next deploy).

---

## Hard Constraints

Same as Phase 1 + Phase 2a:

1. **No consensus impact.** Pure classifier extension. Read-only over already-emitted events.
2. **Safe for rolling deploy.** Q1 NO / Q2 NO / Q3 YES. No activation height.
3. **Additive enum variant only.** Old serialized events (without ChainBreakLoop) deserialize as before. New events with ChainBreakLoop fail to deserialize on old binaries — but those events are produced by the classifier ad-hoc, not persisted, so this is local-only.
4. **DO NOT modify** existing rules (a)-(g) behavior. Rule (h) inserts in precedence order; existing rules keep their thresholds. The only "modification" is the precedence position.
5. **No PII.** Use only what's already in the events (PeerIds, hashes, heights, slot numbers).
6. **Test discipline.** Both fixture tests must pass. Full regression (`cargo test -p storage -p doli-node -p rpc -p doli-cli`) must remain green.
7. **Modular.** classifier.rs is currently 364 lines after M3's rules. Rule (h) adds ~80-100 lines. Should still fit under 500 — if not, split into a submodule.
8. **Docs in sync.** `/sync-docs` at end. Three-question checklist in commit (NO/NO/YES).

---

## Acceptance Criteria

A. `cargo test -p storage --test diagnostic_classifier_test test_rule_h_chain_break_loop_n6_live_fixture` passes.

B. `cargo test -p storage --test diagnostic_classifier_test test_rule_h_chain_break_loop_inc_i_083_historical_fixture` passes.

C. All existing classifier tests still pass (precedence change must not break rules (a)-(g) coverage).

D. Running `doli forks --rpc <stuck-node-port> --explain --human` on a stuck node produces:
   ```
   Classification: ChainBreakLoop (confidence 0.85)
   Recommended action: restart_with_resync
   ```
   (NOT `TipRaceNatural` / `normal_operation`.)

E. Workflow #347's existing INC-I-083 fixture test is UPDATED to expect `ChainBreakLoop` (previously expected `Unknown`). Workflow #347's INC-I-081 fixture test continues to expect `EpochBoundaryInvalid`.

F. The two new fixture files (`inc-n6-chain-break-loop.json` and `inc-n6-stuck.log`) are committed to the repo.

---

## Process

Single-milestone workflow. TDD:

1. **Read** the existing `classifier.rs` to confirm the exact function signatures + rule precedence + the helper functions (`find_validation_duration`, `has_other_signals`, etc.).
2. **Write the two fixture tests FIRST** (test-writer phase). Both must FAIL on `87dee5e9` because `ChainBreakLoop` variant doesn't exist.
3. **Add the enum variant** to types.rs.
4. **Add rule (h)** to classifier.rs in the correct precedence position.
5. **Run the tests** — both new tests should PASS, all existing tests should still PASS.
6. **Update docs** (`docs/fork_observability.md`, both SKILL.md files).
7. **Compile gates** — `cargo build`, `cargo clippy -- -D warnings`, `cargo fmt --check`, full `cargo test`.
8. **Commit** with conventional message + three-question checklist + `--author "Antonio Lozada <antonio@omegacortex.ai>"`.
9. **Update `docs/.workflow/milestone-progress.md`** (or create a new entry) noting workflow #349 closed.
10. **Register in memory.db** the new workflow_run + completion + linked git commit hash.

Single commit. ~150 LoC code + ~80 LoC tests + ~50 LoC docs. Total ~300 LoC.

---

## What This Workflow Will NOT Do

- Add more classifier rules beyond ChainBreakLoop. Other novel patterns (network partition, peer-scoring exclusion, etc.) are deferred to future incidents that produce their own evidence.
- Modify the existing fixture-test framework (the M4 replay test infrastructure stays as-is — we just add two new tests using it).
- Tune the thresholds of rules (a)-(g). Their thresholds are still correct for their domains.
- Touch any consensus code, network code, validation code, or apply_block path.

---

## The Branch

Build on `feature/fork-observability-346` (currently at `87dee5e9`). Name the
follow-up branch `feature/fork-observability-346-chain-break-loop` OR just
continue on the same branch — the change is small enough to land as a single
commit on top of Phase 1+2a.

If Phase 1+2a has already merged to main by the time this workflow runs,
branch off main: `feature/chain-break-loop-rule`.

---

## Final Reminder

n6 was the smoking gun: the classifier confidently misclassified a stuck node
as "normal operation." That false confidence is the kind of bug that makes
operators stop trusting the diagnostic. One more rule closes the most
important known gap. Land it, then watch the next incident — if a NEW pattern
emerges, capture its fixture (NOW, while it's live) and propose rule (i).

The classifier improves by accumulating evidence one fixture at a time. n6
is the second contribution to that corpus. INC-I-081 was the first.

Go.
