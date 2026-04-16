# INC-I-034 — M-RC12-full: Complete asymmetric blacklist in sync manager

**Date:** 2026-04-16
**Branch:** `synmgrefactor`
**Files touched:**
- `crates/network/src/sync/manager/sync_engine/response.rs` (else branch of empty-headers handler, lines 317-347 pre-fix)
- `docs/bugfixes/inc-i-034-m-rc12-full-asymmetric-blacklist-fix.md` (this file, new)
- `docs/.workflow/milestone-progress.md` (status update M-RC12-full PENDING → COMPLETE local)

**Tests:** `crates/network/src/sync/manager/tests.rs` module `m_rc12_full_asymmetric_blacklist_tests` — 4/4 PASS (FAIL → PASS). Pre-existing `test_post_snap_empty_headers_triggers_height_fallback` — still PASS. Full `cargo test -p network --lib` — 302 passed, 0 failed, 1 ignored.
**Incident:** INC-I-034 (live mainnet cascade 2026-04-16 05:11 UTC, santiago / ivan / seed3 on ai3).
**Prior milestones:** M-RC9 (`inc-i-034-m-rc9-silent-vec-fix.md`), M-RC10 (`inc-i-034-m-rc10-apply-after-reject-fix.md`), M-RC11 (`inc-i-034-m-rc11-fork-guard-backfill-fix.md`). Partial RC#12 landed earlier on `synmgrefactor` as `42fe7982` (discriminated "behind" vs "forked" in the gossip-orphan path); this fix closes the remaining sync-manager-side empty-headers blacklist heuristic.
**Requirement:** REQ-REDESIGN-012 — "blacklist requires positive evidence (signature / economics fail); never inference from an empty response."

## Symptom (replayed from the 2026-04-16 santiago cascade)

```
2026-04-16T05:11 UTC   santiago has local_height=39599 on minority fork hash H_local
2026-04-16T05:11 UTC   santiago dispatches GetHeaders(H_local) -> peer_A
                       peer_A returns empty headers (does not recognize H_local)
                       sync_engine/response.rs:328-332 inserts peer_A into
                       fork.header_blacklisted_peers
2026-04-16T05:11 UTC   santiago dispatches GetHeaders(H_local) -> peer_B
                       peer_B returns empty headers
                       sync_engine/response.rs:328-332 inserts peer_B
2026-04-16T05:11 UTC   santiago dispatches GetHeaders(H_local) -> peer_C
                       peer_C returns empty headers
                       consecutive_empty_headers now >= 3 -> stuck-fork branch
                       clears blacklist, signals fork recovery -- but too late:
                       peers A/B already dropped out of the request rotation
                       for long enough that santiago is stranded from the
                       canonical chain.
```

The canonical chain was at `H_canonical` where `peer_A`, `peer_B`, `peer_C` all had the majority history. The empty-headers responses were **correct** — the peers genuinely had no link from `H_local`. The bug was that the sync manager treated the peer's correct silence as fault evidence against the peer, removing good canonical peers from the pool the moment the node most needed them.

## Root cause

```rust
// crates/network/src/sync/manager/sync_engine/response.rs, pre-fix (317-347)
} else {
    // First 1-2 empties: could be a peer-specific issue.
    // INC-I-012 F7: Skip blacklisting if snap sync completed
    // within the last 5 minutes -- ALL canonical peers will return
    // empty for our unrecognizable hash, so blacklisting just
    // removes good peers from the pool.
    let recently_snapped = self
        .snap
        .last_snap_completed
        .map(|t| t.elapsed().as_secs() < 300)
        .unwrap_or(false);
    if !recently_snapped {
        self.fork
            .header_blacklisted_peers
            .insert(peer, Instant::now());          // <-- the bug
    }
    warn!(
        "Empty headers from {} (peer_h={}, local_h={}, gap={}) -- \
         fork evidence (consecutive={}){}.",
        ...
    );
}
```

The INC-I-012 F7 comment above the bug already stated the invariant ("all canonical peers will return empty for our unrecognizable hash"), but the mitigation was scoped only to the 5-minute post-snap window. Outside that window — including the santiago scenario where snap sync was NOT the originating action — the symmetric heuristic fired and blacklisted the correct peers.

**The asymmetry**: peer-returned-empty vs. node-returned-empty are not symmetric events. The peer returning empty is evidence about OUR hash (we don't appear in their canonical history), not evidence about THEIR fault. Blacklisting on this signal is always wrong.

## Fix

Replace the entire else block (lines 317-347 pre-fix) with an asymmetric-invariant version:

```rust
} else {
    // M-RC12-full (INC-I-034) -- asymmetric blacklist invariant (B-5 / S-6 / F-4).
    //
    // Empty-headers is NOT positive evidence against the peer.
    // The peer correctly returned empty because it does not recognize
    // OUR hash -- either we are on a minority fork (our problem, not
    // theirs) or gossip has not yet delivered the missing parent.
    //
    // Per specs/scheduler-state-architecture.md "Sync manager asymmetric
    // blacklist": blacklist requires positive evidence (signature/economics
    // fail), never inference from an empty response. Instead, set
    // use_height_based_headers = true so the next sync attempt consults
    // fork choice via GetHeadersByHeight(local_height) -- the same
    // mechanism used by the INC-I-012 / INC-I-017 post-snap path above.
    //
    // The consecutive_empty_headers counter still advances (see +=1
    // above) so the >=3 cascade branch can still detect broad fork
    // conditions across multiple peers. That branch CLEARS the
    // blacklist; this branch must never ADD to it.
    self.fork.use_height_based_headers = true;
    warn!(
        "Empty headers from {} (peer_h={}, local_h={}, gap={}, consecutive={}) -- \
         local hash not recognized by peer. NOT blacklisting (asymmetric invariant). \
         Consulting fork choice via height-based headers on next sync attempt.",
        peer, peer_height, self.local_height, gap, self.fork.consecutive_empty_headers,
    );
}
```

LOC delta: -30 +25 inside the else branch. No other modification to the file.

## Three-layer coverage (per spec F-16 defense-in-depth)

**(a) Trigger removal.** The `self.fork.header_blacklisted_peers.insert(peer, Instant::now())` call in the `consecutive_empty_headers < 3` branch is deleted. This eliminates the structural path by which an empty-headers response can blacklist the responding peer.

**(b) Recovery imperfection fix.** Setting `self.fork.use_height_based_headers = true` means the next `start_sync()` dispatch (driven by `SyncManager::dispatch` at `sync_engine/dispatch.rs:72`) will issue `GetHeadersByHeight(local_height)` instead of `GetHeaders(local_hash)`. Height-based consultation bypasses the hash-not-recognized failure mode entirely — the peer answers from its canonical chain at the given height, regardless of whether it has seen our `local_hash`. This is the **replacement recovery mechanism** for the removed blacklist heuristic; it is identical in spirit (and in code) to the INC-I-012 / INC-I-017 post-snap fallback at `response.rs:218-234`, which was already proven in production.

**(c) Architectural invariant (enforced by tests).** The invariant "empty-headers never blacklists the responding peer" is now pinned by four regression tests in `crates/network/src/sync/manager/tests.rs` module `m_rc12_full_asymmetric_blacklist_tests`:

| Test | Path | Primary assertion |
|---|---|---|
| `test_m_rc12_first_empty_headers_does_not_blacklist` | counter 0 → 1 | `!fork.header_blacklisted_peers.contains(peer)` + `fork.use_height_based_headers == true` + `consecutive_empty_headers == 1` + `state == Idle` + returned `Vec<Block>` empty |
| `test_m_rc12_second_empty_headers_still_no_blacklist` | counter 1 → 2 | same set with counter `== 2` |
| `test_m_rc12_snap_not_exhausted_still_no_blacklist` | `snap.attempts=1`, threshold=1000 | invariant holds even when the old F7 "recently_snapped" gate would NOT have protected the peer |
| `test_m_rc12_blacklist_invariant_covers_multiple_peers` | adversarial multi-peer | blacklist stays empty across two peers; catches any regression where `insert()` creeps back under an alternate branch |

Together with the still-passing `test_post_snap_empty_headers_triggers_height_fallback` (covering the post-snap `AwaitingCanonicalBlock` path at response.rs:218-234), the invariant is pinned across both sync phases: header-first recovery AND post-snap recovery.

## REQ traceability

| REQ | Text | Asserted by |
|---|---|---|
| **REQ-REDESIGN-012** | "Blacklist requires positive evidence (signature / economics fail); never inference from an empty response." | All four `m_rc12_full_asymmetric_blacklist_tests` (primary blacklist assertions). |
| **REQ-REDESIGN-012-B** | "When local hash is unrecognized by the peer, the node must consult fork choice via `GetHeadersByHeight(local_height)` on the next sync attempt." | `use_height_based_headers == true` assertion in all four tests + `test_post_snap_empty_headers_triggers_height_fallback`. |

## Test evidence (FAIL → PASS)

**FAIL baseline (before fix, HEAD prior to this commit):**

```
$ cargo test -p network --lib sync::manager::tests::m_rc12_full_asymmetric_blacklist_tests
test test_m_rc12_second_empty_headers_still_no_blacklist ... FAILED
test test_m_rc12_first_empty_headers_does_not_blacklist ... FAILED
test test_m_rc12_blacklist_invariant_covers_multiple_peers ... FAILED
test test_m_rc12_snap_not_exhausted_still_no_blacklist ... FAILED
test result: FAILED. 0 passed; 4 failed; 0 ignored
```

All four panic on the primary blacklist assertion. Panic message from `test_m_rc12_first_empty_headers_does_not_blacklist`:

```
panicked at tests.rs:4777:
  M-RC12-full: first empty-headers must NOT blacklist responding peer
  (asymmetric invariant -- peer is not fault evidence).
  Blacklist contents: [PeerId("1AWzk8GpmPCXtCQwk3K3PwNDWhWk58nxBi3vzQwhuXJ6gi")]
```

**PASS (after fix):**

```
$ cargo test -p network --lib m_rc12_full_asymmetric_blacklist_tests
test sync::manager::tests::m_rc12_full_asymmetric_blacklist_tests::test_m_rc12_first_empty_headers_does_not_blacklist ... ok
test sync::manager::tests::m_rc12_full_asymmetric_blacklist_tests::test_m_rc12_snap_not_exhausted_still_no_blacklist ... ok
test sync::manager::tests::m_rc12_full_asymmetric_blacklist_tests::test_m_rc12_blacklist_invariant_covers_multiple_peers ... ok
test sync::manager::tests::m_rc12_full_asymmetric_blacklist_tests::test_m_rc12_second_empty_headers_still_no_blacklist ... ok
test result: ok. 4 passed; 0 failed

$ cargo test -p network --lib test_post_snap_empty_headers_triggers_height_fallback
test sync::manager::tests::test_post_snap_empty_headers_triggers_height_fallback ... ok
test result: ok. 1 passed; 0 failed

$ cargo test -p network --lib
test result: ok. 302 passed; 0 failed; 1 ignored; 0 measured
```

No regressions in the 302-test network lib suite.

## Why no protocol version bump

This change is purely local recovery logic inside the sync manager. The wire protocol (`GetHeaders`, `GetHeadersByHeight`, `SyncResponse::Headers`) is unchanged. Peers running the pre-fix code can interoperate with post-fix peers byte-identically — the difference is only in how the post-fix node **reacts** to an empty headers response locally. No `CURRENT_PROTOCOL_VERSION` bump in `crates/network/src/protocols/status.rs` is required.

## Why no HardForkSchedule entry

This change does not alter block validity, state transitions, consensus parameters, or any on-chain invariant. A pre-fix node and a post-fix node observing the same sequence of blocks will produce identical state roots. No `HardForkSchedule` entry in `crates/updater/src/hardfork.rs` is required.

## Deployment checklist (per CLAUDE.md "After Every Modification")

1. **Build gate:** `cargo build --release && cargo clippy -p network -- -D warnings && cargo fmt --check` — network crate clean. (Pre-existing fmt drift in `bins/node/tests/m_rc10_*` and `bins/node/tests/m_rc11_*` test files is unrelated to M-RC12-full and should be addressed by a separate cleanup commit.)
2. **Test:** `cargo test -p network --lib` — 302 passed, 0 failed.
3. **Version protection:** not applicable — no consensus / protocol / validation change.
4. **Documentation alignment:**
   - `specs/scheduler-state-architecture.md` — already describes "Sync manager asymmetric blacklist (B-5 / F-4 convergence)" with disposition rows S-6 and B-5. No spec change required; the fix matches the spec exactly.
   - `docs/architecture.md` — no crate-structure change.
   - `docs/troubleshooting.md` — optional future addition: "if a node appears stuck with empty-headers warnings, expect `use_height_based_headers` to be triggered on the next sync attempt; look for the log line starting with `local hash not recognized by peer. NOT blacklisting`".
5. **Copy binary:** for post-commit deployment, `cp target/release/doli-node ~/testnet/bin/ && codesign --force --sign - ~/testnet/bin/doli-node` (per `feedback_codesign_after_cp.md`).
6. **Commit and push:** deferred to orchestrator. Commit author when orchestrator commits: `Antonio Lozada <antonio@omegacortex.ai>`.
7. **Deploy consideration:** testnet first. Monitor for the new `NOT blacklisting (asymmetric invariant)` log line under normal fork conditions — it should appear on nodes that have a minority fork hash and is the expected, benign signal that the asymmetric path is active.

## Operator-visible surface

Pre-fix log line (was issued on every empty-headers response, 1-2 counter range):

```
WARN Empty headers from 12D3KooW... (peer_h=39628, local_h=39599, gap=29) --
     fork evidence (consecutive=1). Blacklisted peer
```

Post-fix log line (issued on every empty-headers response, 1-2 counter range):

```
WARN Empty headers from 12D3KooW... (peer_h=39628, local_h=39599, gap=29, consecutive=1) --
     local hash not recognized by peer. NOT blacklisting (asymmetric invariant).
     Consulting fork choice via height-based headers on next sync attempt.
```

The 3+ cascade branch and the small-gap branch are unchanged and continue to emit their existing log lines. Operators grep'ing for the old "Blacklisted peer" suffix in response to sync issues should adjust to the new "NOT blacklisting (asymmetric invariant)" semantics — this is now the expected path for the 1-2 empty range.

## Out-of-scope (deliberately)

- `consecutive_empty_headers += 1` at `response.rs:244` — kept. Cascade detection at >=3 still relies on it.
- Small-gap gossip-timing branch at `response.rs:256-267` — untouched. No blacklist interaction.
- Stuck-fork / anti-cascade branch at `response.rs:269-316` — untouched. Already clears the blacklist; legitimate fork-choice signaling.
- Post-snap height-fallback gate at `response.rs:218-234` — untouched. Already asymmetric-correct; covered by `test_post_snap_empty_headers_triggers_height_fallback`.
- `BlacklistDecision` enum (Phase-2 structural type) — out of scope. S-6 deletion (Phase-1) is sufficient to make the invariant structurally testable.
- Busy-error blacklist at `response.rs:115-118` — untouched. `SyncResponse::Error(err.contains("busy"))` is positive fault evidence (peer explicitly reports rate-limit), so the asymmetric invariant does not apply.

## Handoff

- **QA:** run `cargo test -p network --lib` and confirm 302/0/1 result. Run `cargo test -p network --lib m_rc12_full_asymmetric_blacklist_tests` and confirm 4/0/0. Replay the santiago scenario locally if testnet capacity allows: start a node on a minority fork hash, observe the new log line pattern, confirm `GetHeadersByHeight` is dispatched on the next sync tick and that the canonical peer (returning empty on `GetHeaders(local_hash)`) is NOT in `fork.header_blacklisted_peers` after the response is handled.
- **Reviewer:** verify the three-layer coverage is genuinely independent: (a) is the `insert` call deleted? (b) does the `use_height_based_headers = true` line drive the `dispatch.rs:72` branch on the next sync attempt? (c) do the four tests actually fail if you re-insert `.insert(peer, Instant::now())` in the else branch? (Answer to all three: yes.)
