# M-RC12-full — Test Writer Report (TDD step 1)

**Milestone**: `M-RC12-full` — Complete asymmetric blacklist (42fe7982 was partial).
**Target code**: `crates/network/src/sync/manager/sync_engine/response.rs:317-347`
(`consecutive_empty_headers < 3` arm inserts into `fork.header_blacklisted_peers`).
**Spec**: `specs/scheduler-state-architecture.md`, "Sync manager asymmetric
blacklist (B-5 / F-4 convergence)" + disposition rows **S-6** and **B-5**.
**Confidence gate**: M-RC12-full is `conf(0.7, partial fix exists)` per
`docs/.workflow/milestone-progress.md:37`. CLAUDE.md #21 requires FAIL->PASS
test evidence for any fix above 0.7 — this report provides the FAIL half.
**DB cross-reference**: pins DIAG-SYNC-002 (P1 open finding in memory.db):
*"Empty headers handler always blacklists the responding peer (blaming peer
for fork). After 3-4 cycles, all canonical peers are blacklisted, leaving
only stuck peers available for sync. Node becomes isolated from the
canonical chain."*

---

## 1. Output Contract enumeration

```
OUTPUT CONTRACT: fn handle_response(&mut self, peer: PeerId, SyncResponse::Headers(vec![]))
  (delegates to handle_headers_response for the empty case)

Outputs observed for the empty-headers codepath (O2 = &mut self field
mutations; O3 = return value):

  O2: self.fork.header_blacklisted_peers : HashMap<PeerId, Instant>
      INVARIANT (M-RC12-full): responding peer MUST NOT be inserted on
      empty-headers alone. Blacklist requires positive fault evidence
      (bad signature / bad economics), never inference from empty
      headers. Peer stays eligible.

  O2: self.fork.use_height_based_headers : bool
      Post-condition: set to true so the next sync cycle issues
      GetHeadersByHeight(local_height) and consults fork choice
      (replacement for the deleted blacklist heuristic).

  O2: self.fork.consecutive_empty_headers : u32
      Post-condition: incremented by exactly 1. Counter still feeds
      cascade detection at >=3.

  O2: self.state : SyncState
      Post-condition: transitions to Idle (unchanged by M-RC12-full;
      matches existing behavior across all empty-headers arms).

  O3: return value : Vec<Block>
      Post-condition: vec![] (Headers responses never return blocks).

Paths covered by the four tests:

  P1 first_empty       — counter 0 -> 1, not recently snapped,
                         gap > 3, snap.attempts=0, recovery=Normal.
                         Hits response.rs:317-347 (BUG).
  P2 second_empty      — counter 1 -> 2, same pre-conditions.
                         Hits same branch (BUG).
  P3 snap_available    — snap.attempts=1, snap.threshold=1000. Pre-M-RC12
                         the gate at 210-217 required snap_exhausted;
                         M-RC12-full removes that gate for the blacklist
                         invariant. Hits same branch (BUG).
  P4 multi_peer        — adversarial: peer1 and peer2 each give one empty.
                         Blacklist must stay empty after both. Catches
                         regressions where insert() creeps back under a
                         different code path.

Paths intentionally out of M-RC12-full scope:
  - counter >= 3 stuck-fork branch (response.rs:269-316): already CLEARS
    blacklist, correct by today.
  - post-snap AwaitingCanonicalBlock branch (response.rs:218-234): already
    asymmetric-correct, covered by
    test_post_snap_empty_headers_triggers_height_fallback (tests.rs:1342).
  - gap <= 3 small-gap gossip-timing branch (response.rs:256-267): no
    blacklist interaction; unchanged.
```

## 2. Outputs x Paths assertion matrix

| Path | O2 blacklist (!contains peer) | O2 use_height_based_headers (==true) | O2 consecutive_empty_headers | O2 state (==Idle) | O3 return (==vec![]) |
|------|-------------------------------|--------------------------------------|------------------------------|-------------------|----------------------|
| P1 first_empty | `test_m_rc12_first_empty_headers_does_not_blacklist` (primary assert + `.is_empty()`) | same test | `== 1` (same test) | same test (`matches!(state, Idle)`) | same test (`returned.is_empty()`) |
| P2 second_empty | `test_m_rc12_second_empty_headers_still_no_blacklist` | same test | `== 2` (same test) | invariant covered in P1 | invariant covered in P1 |
| P3 snap_available | `test_m_rc12_snap_not_exhausted_still_no_blacklist` | same test | `== 1` (same test) | invariant covered in P1 | invariant covered in P1 |
| P4 multi_peer | `test_m_rc12_blacklist_invariant_covers_multiple_peers` (both peers + overall `.is_empty()`) | same test | n/a (counter reset mid-test to re-enter first-empty branch for peer2) | invariant covered in P1 | invariant covered in P1 |

**Cells**: 4 outputs x 4 paths = 16 total. 4 primary cells (blacklist per
path) are asserted directly. 4 secondary cells (`use_height_based_headers`)
are asserted directly. 3 counter cells are asserted directly (P4 is
intentionally n/a because the test mutates the counter to re-enter the
buggy branch). State and return-value cells are invariant across the
empty-headers family and asserted once in P1 — per
`.claude/protocols/output-contract.md` ASSERTION-QUALITY-RULES (assert the
delta, not repeat invariants). The adversarial P4 compensates by asserting
the blacklist is empty across the whole loop, catching regressions that
would insert under alternate code paths.

## 3. FAIL evidence (RED phase — as required before ANY fix code)

Command:
```
cargo test -p network --lib sync::manager::tests::m_rc12_full_asymmetric_blacklist_tests
```

Result (4 failed / 0 passed on HEAD):
```
test test_m_rc12_second_empty_headers_still_no_blacklist ... FAILED
test test_m_rc12_first_empty_headers_does_not_blacklist ... FAILED
test test_m_rc12_blacklist_invariant_covers_multiple_peers ... FAILED
test test_m_rc12_snap_not_exhausted_still_no_blacklist ... FAILED
```

Failure details (each panics at the primary blacklist assertion — the
responding peer's PeerId is visibly present in `header_blacklisted_peers`):

```
---- test_m_rc12_first_empty_headers_does_not_blacklist ----
panicked at tests.rs:4777:
  M-RC12-full: first empty-headers must NOT blacklist responding peer
  (asymmetric invariant — peer is not fault evidence).
  Blacklist contents: [PeerId("1AWzk8GpmPCXtCQwk3K3PwNDWhWk58nxBi3vzQwhuXJ6gi")]

---- test_m_rc12_second_empty_headers_still_no_blacklist ----
panicked at tests.rs:4833:
  M-RC12-full: second consecutive empty-headers must NOT blacklist
  responding peer

---- test_m_rc12_snap_not_exhausted_still_no_blacklist ----
panicked at tests.rs:4875:
  M-RC12-full: empty-headers must NOT blacklist peer even when snap is
  available and not exhausted.
  Blacklist contents: [PeerId("1AcsgyBLabdKbrejeMAYTLPFuoG9sBc2W27n5ZyEp4a2M8")]

---- test_m_rc12_blacklist_invariant_covers_multiple_peers ----
panicked at tests.rs:4955:
  M-RC12-full: asymmetric-blacklist invariant must hold across multiple
  peers. Blacklist contents after 2 peers:
  [PeerId("1AZBJuDcuLcQMSBJaJUYb5a5SjmuBy5BZsLyisYNHrgJf4"),
   PeerId("1AXBKYKcV9STGAFBX3rWovDMbV1e2ZX6vDiFz74353we8g")]

test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured;
299 filtered out
```

The failure messages match the RC#12 bug signature: peers are being
inserted into `fork.header_blacklisted_peers` purely because they returned
an empty headers response — exactly the symmetric-blacklist heuristic that
B-5 / S-6 must delete. The tests ARE the contract.

## 4. Handoff to the developer (TDD step 2)

**What to change** (one file, one hunk):

- Edit `crates/network/src/sync/manager/sync_engine/response.rs:317-347`.

**Three surgical edits inside the `else` branch at line 317**:

1. **Delete** the insert at lines 328-332:
   ```rust
   if !recently_snapped {
       self.fork
           .header_blacklisted_peers
           .insert(peer, Instant::now());
   }
   ```
   This is the S-6 deletion (spec row: "delete empty-headers blacklist
   heuristic", 5/5 DEFINITE).

2. **Add** the B-5 replacement behavior in the same branch — set
   `use_height_based_headers = true` so the next sync cycle consults fork
   choice via `GetHeadersByHeight(local_height)` (same mechanism already
   used by the post-snap path at response.rs:228):
   ```rust
   self.fork.use_height_based_headers = true;
   ```

3. **Update** the `warn!` message at lines 333-346 to reflect the new
   semantic. Suggested text:
   ```
   "Empty headers from {} (peer_h={}, local_h={}, gap={}, consecutive={}) \
    — peer returned no link from our local_hash; empty-headers is not fault \
    evidence against peer; consulting fork choice via height-based headers."
   ```
   Drop the `recently_snapped` conditional log suffix — blacklist no longer
   depends on it.

**Do NOT touch** the following (out of M-RC12-full scope):
- The `consecutive_empty_headers += 1` at line 244 — keep it (cascade
  detection at >=3 still relies on it).
- The post-snap branch at 218-234 — already asymmetric-correct.
- The small-gap branch at 256-267 — no blacklist interaction.
- The stuck-fork branch at 269-316 — already clears blacklist.
- The `BlacklistDecision` enum (B-5 structural type) — out of scope for
  this phase. Spec notes B-5 enum is Phase-2; S-6 deletion is Phase-1 and
  sufficient for the invariant.
- `crates/network/src/sync/manager/tests.rs:1342-1388`
  (`test_post_snap_empty_headers_triggers_height_fallback`) — unchanged;
  covers the already-correct post-snap path.

**Expected post-fix test outcome**: all 4 `m_rc12_full_asymmetric_blacklist_tests`
tests pass (GREEN) AND the pre-existing
`test_post_snap_empty_headers_triggers_height_fallback` continues to pass.
Together they pin the invariant across both sync phases.

**Spec consistency check** — one incidental fix I had to make (not an
M-RC12 test):

- `crates/network/src/sync/manager/tests.rs:2213` — the pre-existing test
  `test_regression_snap_ready_to_synchronized` was broken on HEAD: the
  `VerifiedSnapshot` struct literal was missing the `epoch_state_bytes:
  Option<Vec<u8>>` field (types.rs:174), which had been added to the
  struct but not propagated to this test. The crate would not compile
  otherwise. I added `epoch_state_bytes: None,` to the literal. This is
  unrelated to M-RC12-full; it is a spec-drift finding the developer
  should be aware of (the field matches the M-Choice1/snap-sync work at
  `specs/scheduler-state-architecture.md` line 244).

## 5. Specs Gaps Found

| Where | What | Severity |
|-------|------|----------|
| `crates/network/src/sync/manager/tests.rs:2213` | `VerifiedSnapshot` literal missed `epoch_state_bytes` field after struct was extended in `types.rs:174`. Blocked compilation of the entire `network` lib-test target. | P2 (blocking compile) — fixed in this commit as incidental compile-gate repair. |
| `specs/scheduler-state-architecture.md` row RC#12 | Correctly states "Empty-headers blacklist in `sync_engine/response.rs` deleted (S-6)" — but the partial fix at 42fe7982 only touched `block_lifecycle.rs`/`peers.rs`/`types.rs`/`rollback.rs` (orphan-gossip path), not `response.rs`. The milestone row `M-RC12-full` in `docs/.workflow/milestone-progress.md:37` captures this accurately. No spec change needed. | informational |

---

Handoff: Developer (TDD step 2) — implement the 3-edit S-6 fix above;
re-run `cargo test -p network --lib sync::manager::tests::m_rc12_full_asymmetric_blacklist_tests`;
expect all 4 tests to go FAIL -> PASS.
