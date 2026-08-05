# Code Review: INC-I-149 bootstrap mint gate + `inc_i_147_activation_height` pin (staged)

**Reviewer:** OMEGA Reviewer · **Date:** 2026-08-05 · **Run:** 480
**Target:** `git diff --cached` at `main` (13 files, +1852 / -32)
**Prerequisite gate:** source code present (`bins/`, `crates/`) — PASS. No QA report in `docs/qa/` for this change — noted, proceeded with code review.

---

## Scope Reviewed

| File | Read | Verdict |
|---|---|---|
| `bins/node/src/node/production/mod.rs` | full guard region 90-250 | reviewed |
| `bins/node/src/node/production/scheduling.rs` | full (486 lines) | reviewed |
| `bins/node/src/node/mod.rs` | import hunk | reviewed |
| `crates/network/src/sync/manager/peers.rs` | 1-260, 490-560 | reviewed |
| `crates/network/src/{lib.rs, sync/mod.rs, sync/manager/mod.rs}` | re-export hunks | reviewed |
| `crates/core/src/network_params/{defaults.rs, mod.rs}` | AH hunks + all 3 network blocks | reviewed |
| `bins/node/tests/inc_i_149_bootstrap_mint_gate.rs` | full (799 lines) | reviewed |
| `docs/troubleshooting.md`, `docs/bugfixes/inc-i-149-structural-design.md` | full diff / design doc | reviewed |
| **Blast radius (not in diff, read to bound impact)** | `production_gate.rs`, `init.rs:700-740`, `startup.rs:540-560`, `reorg/mod.rs:120-190, 350-400, 490-572`, `block_handling.rs:125-175`, `updater/constants.rs`, `scripts/install-local-services.sh` | reviewed |

Not reviewed: `crates/core/src/validation*` (confirmed untouched by the diff and unreachable from the new code path — see Consensus Safety), `bins/node/src/node/production/assembly.rs` (untouched).

Build/test gate **not** executed by this review — reviewer tooling is read-only (no `cargo`). Test evidence below is cited from the commit draft and the task statement, not re-measured.

---

## Summary

**⚠️ APPROVE WITH NOTES.** The two shipped behavioral changes are correct, consensus-safe, and genesis-safe by construction. No finding proves the code produces a wrong outcome for any reachable input, so this is not a BLOCK. Three P1 items should be resolved before the commit lands — two are ~5-minute edits, one is a documentation completeness item on a residual the design doc claims to have enumerated but has not.

- Injection pattern scan: **CLEAN**. No SQL/shell/eval sinks anywhere in the diff. No f-string/`format!` interpolation into a query or command. No external input reaches any interpreter.
- Consensus safety: **CONFIRMED CLEAN** (evidence in §Consensus Safety).
- AH pin arithmetic and immutability: **CONFIRMED CORRECT** (evidence in §Activation-Height Pin).
- Test quality: **STRONG, non-vacuous** — one structural weakness (F-P3-013).

---

## Critical Findings (P1)

### F-P1-001 — New protection mechanism is not registered in `protection_mechanisms`

- **Location:** `bins/node/src/node/production/scheduling.rs:84-91` (new gate); `bins/node/src/node/production/mod.rs:183` (widened guard)
- **Severity:** Major
- **Evidence:**
  ```
  $ ls -la .omega/gauntlet.conf
  -rw-r--r--@ 1 isudoajl staff 1393 Jul 7 18:58 .omega/gauntlet.conf     <- system-impact protocol ARMED

  $ sqlite3 .omega/memory.db "SELECT COUNT(*) FROM protection_mechanisms;"
  22
  $ sqlite3 .omega/memory.db "SELECT mechanism_id,name FROM protection_mechanisms
      WHERE name LIKE '%produc%' OR name LIKE '%evidence%' OR name LIKE '%bootstrap%';"
  PM-008|GSet producer-announcement staleness filter
  PM-018|single-owner evidence counter (consecutive_empty_headers)
  ```
  Neither row is the INC-I-149 gate. The gate meets the protocol's own definition of a protection mechanism verbatim: "code that constrains system dynamics: rate limit, backoff, blacklist, watchdog, escalation ladder, **staleness filter**, queue bound, cap, circuit breaker."
- **Confidence:** `conf(0.97, measured)` — direct query output.
- **Interaction analysis (performed here so the registry row can be written correctly):**
  - **vs PM-012 `GOSSIP_WATCHDOG`** — shares the production-authorization surface. PM-012 is non-blocking (`production_gate.rs:159-164` only `warn!`s). The new gate is downstream of `can_produce` (it lives in eligibility resolution), so when both evaluate, the new gate's `return None` wins. Strictly more conservative. **No feedback loop**: the gate's action (do not build) cannot create PM-012's trigger in a way that unblocks the gate.
  - **vs `can_produce` genesis quorum / `genesis_bypass`** (`production_gate.rs:109-118, 124`) — same surface, both can fire on the same tick. The new gate is strictly narrower in effect (skip one slot) and cannot starve its own disarm input: its disarm input is peer status, produced by the network layer independently of whether this node builds.
  - **Scale sensitivity:** the gate carries **no numeric threshold**. It is a pure predicate over `(bootstrap_nodes.is_empty(), peer_count()==0)`. Rule-1 compliant by construction — nothing to calibrate, nothing to mis-scale between a 6-node LAN and a 27+-node mainnet. This is the change's strongest property and should be recorded as such.
- **Suggested fix:** one `INSERT` into `protection_mechanisms` (PM-023: trigger = `bootstrap_nodes` non-empty AND `SyncManager::peer_count()==0`; action = `resolve_bootstrap_eligibility` returns `None`, slot skipped; scale = no numeric threshold, config-derived; interacts-with = PM-012, `can_produce` genesis quorum), plus an update of the behind-network guard's record to note its domain widened from `height > 1` to all heights.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — one-time sqlite3 INSERT, not on any node code path)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  N-A (local memory.db only, no node process involved)
  Disk:     +~1KB one-time (observed — a single memory.db row)
  Latency:  0 (observed — no runtime path touched)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: the registry is the only place a future agent discovers this gate interacts with PM-012; omitting it is how composite failures ship.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

### F-P1-002 — `NetworkEvidence`'s documented contract is false: the classification is contaminated by LOCAL state

- **Location:** `crates/network/src/sync/manager/peers.rs:13-56` (new code, this diff)
- **Severity:** Major (latent — zero behavioral effect on the shipped gate)
- **Evidence:** The new docstrings assert a purely peer-derived contract:
  ```rust
  // peers.rs:34-38 (new)
  /// Peers are connected and none reports any blocks — a genuine fresh genesis.
  AtGenesis,
  /// At least one peer reports height > 0 — the network has history, so an
  /// empty local disk means WE are behind, not that the chain is new.
  HasHistory,
  ```
  and the file header claims it is derived "only from evidence that **SURVIVES A DATA-DIRECTORY WIPE**" (peers.rs:13-14). But the predicate is `best_peer_height() > 0`, and:
  ```rust
  // peers.rs:238-247 (pre-existing)
  pub fn best_peer_height(&self) -> u64 {
      let peer_max = self.peers.values().map(|p| p.best_height).max().unwrap_or(0);
      peer_max.max(self.network.network_tip_height)   // <-- NOT peer-derived
  }
  ```
  and `network.network_tip_height` has three LOCAL writers:
  - `peers.rs:169` — `remove_peer`: `= peer_max_height.max(self.local_height)`
  - `peers.rs:496-500` — `update_network_tip_height`, whose own doc says *"called after we successfully apply a block ... not from gossip"* — i.e. **our own** blocks
  - `peers.rs:544` — gossip block height

  Concrete falsifying trace: node applies its own block 1 → `network_tip_height = 1`; one peer connects reporting height 0 → `peer_count()==1`, `best_peer_height() == max(0, 1) == 1 > 0` → **`HasHistory`**, while *no peer has any history*. Symmetrically, `AtGenesis` is unreachable for any node that has ever applied a block or had a higher peer disconnect.
- **Confidence:** `conf(0.93, observed)` — read from the three writer sites; not executed.
- **Why it matters despite zero current impact:** the only consumer matches `== NetworkEvidence::Unknown` (`scheduling.rs:84`), which is exactly `peer_count()==0` and is unaffected. But the whole justification for adding this type is that it is *the concept that was missing* — a wipe-surviving, peer-derived predicate. Two of its three variants do not satisfy that claim. This is the same defect class as `INV-SYNC-012` ("any height compared against a chain-global quantity MUST itself be a chain-derived height, never a per-process counter"), which the fleet has already paid for twice.
- **Suggested fix (pick one):** (a) compute from the peer map directly — `self.peers.values().map(|p| p.best_height).max().unwrap_or(0) > 0` — making the docstring true; or (b) keep `best_peer_height()` and correct both docstrings to state that the tip includes our own applied-block height and any gossip tip. (a) is preferred: it makes the type mean what the design says it means.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      +O(peers) once per production tick (~1 Hz) (observed — replaces a max() over the same map with a max() over the same map; identical complexity, ~27 entries on mainnet)
  Memory:   0 (observed — no allocation, iterator max)
  IO:       0 (observed)
  Network:  0 (observed — reads cached peer status, sends nothing)
  Disk:     0 (observed)
  Latency:  0 (observed — sub-microsecond, inside a lock already held)
Inevitability: AVOIDABLE
Cheaper alternative: option (b) — leave the code, fix the two docstrings to say the tip includes our own applied-block height (zero runtime delta).
Why this proposal anyway: option (a) makes the type satisfy the property the design is built on, so the next consumer of AtGenesis/HasHistory cannot inherit a locally-contaminated predicate; (b) preserves a footgun in exchange for saving one iterator.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

### F-P1-003 — Unrecorded residual: a first peer status from a height-0 peer re-opens the fossil-mint window

- **Location:** `bins/node/src/node/production/scheduling.rs:84-91` + `bins/node/src/node/production/mod.rs:172-198`; design doc `docs/bugfixes/inc-i-149-structural-design.md:94-101`
- **Severity:** Major (residual, **not** a regression — pre-fix behavior was strictly worse)
- **Evidence:** compose the two shipped guards for a wiped producer whose only peer-status-so-far reports height 0:
  1. `peers.rs:47-55` → `peer_count()==1`, `best_peer_height()==0` → **`AtGenesis`**
  2. `scheduling.rs:84` → gate tests `evidence == Unknown` only → **passes**
  3. `production/mod.rs:172` → `network_tip_height==0`, `height.saturating_sub(1)==0` → `network_height_ahead = 0 > 0 = false` → guard **inert**
  4. → mints its own block 1 = the exact INC-I-149 fossil orphan.

  This row is `R3` in the shipped truth table (`inc_i_149_bootstrap_mint_gate.rs:582-609`), which **asserts it must mint** — correctly, for a genuine fresh-genesis fleet. The two situations are indistinguishable from the node's evidence.

  Reachability is not theoretical: the project's own documented recovery procedure is a **full-fleet wipe** (`MEMORY.md → feedback_deploy_integrity`: *"must full-wipe (blocks+state_db) all nodes, snap sync fresh, backfill from ONE canonical seed"*). During that window every wiped sibling reports height 0 to every other; whichever status lands first decides. If a sibling's status beats the canonical seed's, `AtGenesis` wins and the fossil is minted.

  The design doc's `## Known residual, stated not hidden` (lines 94-101) enumerates **only** the `has_bootstrap_nodes == false` wiped-seed row. This path — `has_bootstrap_nodes == true`, first status from a height-0 peer — is not recorded anywhere in the diff.
- **Confidence:** `conf(0.85, inferred)` — the three-step composition is read directly from the shipped code; the fleet-wipe reachability is inferred from the documented procedure, not observed in a log.
- **Suggested fix:** minimum — extend the design doc's Known-residual section with this row and add it to `docs/troubleshooting.md §2.6` so an operator running a fleet wipe knows to expect it. Optional hardening — require corroboration before `AtGenesis` unlocks production (e.g. `peer_count() >= 2` all at 0, or hold `Unknown` for one status-refresh round). **Any hardening must keep `r3_p2b_...` green**, which is precisely why documenting is the recommended action and hardening is not.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — recommended action is documentation only)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     +~2KB (observed — two markdown paragraphs)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: the optional hardening (require peer_count()>=2 before AtGenesis unlocks) — costs one extra comparison per tick but risks failing R3 on a 2-node genesis fleet.
Why this proposal anyway: the classification is genuinely undecidable from local evidence; recording the residual costs nothing and preserves genesis liveness, whereas hardening trades a real liveness property for a probabilistic safety gain.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## Minor Findings (P2)

### F-P2-004 — Design change-set item 3 was silently dropped, and it is live for the incident shape

- **Location:** `crates/network/src/sync/manager/production_gate.rs:124` (unchanged); mandated by `docs/bugfixes/inc-i-149-structural-design.md:108-109`
- **Severity:** Minor
- **Evidence:** The design's change set says:
  > 3. **FIX** `production_gate.rs:124` — `genesis_bypass = local_height == 0 && min_peers <= 1` waives the echo-chamber peer minimum on any empty disk. **Must key on evidence, not height.**

  The line is untouched by the diff (`git diff --cached --name-only` does not list `production_gate.rs`) and reads exactly as the design describes it:
  ```rust
  // production_gate.rs:124
  let genesis_bypass = self.local_height == 0 && self.min_peers_for_production <= 1;
  ```
  It is **live** for a wiped producer, because `min_peers_for_production` is set to 1 for exactly that node:
  ```rust
  // init.rs:726-735
  let in_genesis_at_start = config.network.is_in_genesis(state.best_height + 1);   // best_height==0 -> true
  let min_peers = match config.network {
      Network::Devnet => 1,
      _ if in_genesis_at_start => 1,        // <-- wiped mainnet producer lands here
      Network::Testnet | Network::Mainnet => 2,
  };
  ```
  So `local_height == 0 && min_peers <= 1` → `genesis_bypass == true` → echo-chamber minimum waived, on the same empty disk the fix is about.
- **Confidence:** `conf(0.9, observed)`
- **Impact:** currently harmless — the waiver only matters at 0 peers, and at 0 peers the new gate already refuses for any bootstrap-configured node. The defect is one of **traceability**: the design doc still presents item 3 as part of the shipped change set, so a future reader will believe a fix landed that did not.
- **Suggested fix:** amend `inc-i-149-structural-design.md` to mark item 3 **DEFERRED** with the reason (subsumed by the gate for bootstrap-configured nodes; residual applies only to the row-4 wiped seed), or implement it.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — doc amendment)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     +~0.5KB (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: implement item 3 (swap local_height==0 for evidence != HasHistory in genesis_bypass) — ~2 lines, but adds a network-layer dependency to production_gate for no currently-reachable benefit.
Why this proposal anyway: the residual it would close is already closed by the shipped gate for every bootstrap-configured node; recording the deferral is honest and free, implementing is churn.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### F-P2-005 — Gate placement deviates from the design; the epoch path has no no-evidence gate

- **Location:** `bins/node/src/node/production/scheduling.rs:84` vs design item 2 (`production/mod.rs`, "placed before the bootstrap/epoch branch")
- **Severity:** Minor
- **Evidence:** the gate ships inside `resolve_bootstrap_eligibility`, which `production/mod.rs:254-264` reaches only when `use_bootstrap` is true:
  ```rust
  // production/mod.rs:254
  let use_bootstrap = in_genesis || active_with_weights.is_empty();
  ```
  On the EPOCH branch (`resolve_epoch_eligibility`, scheduling.rs:451) there is no no-evidence gate at all.
- **Confidence:** `conf(0.95, observed)`
- **Impact:** the incident shape is fully covered — a wiped disk has an empty producer set, so `use_bootstrap` is always true. And this is not a regression: the pre-diff `peer_count == 0` check lived in the same function. But the shipped scope is narrower than the design states, and nothing documents the narrowing.
- **Suggested fix:** amend the design doc to record the actual placement and why the bootstrap branch suffices (empty disk ⇒ empty producer set ⇒ `use_bootstrap`), or hoist the gate as designed.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — doc amendment; hoisting would move one existing predicate, net zero)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     +~0.4KB (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: hoist the gate to try_produce_block as designed — same predicate, one evaluation, also covers the epoch path.
Why this proposal anyway: hoisting changes behavior for the epoch path (a long-running node that loses all peers) which is out of scope for this incident and has its own protections (peer_loss_timeout, can_produce "Lost all peers"); documenting is the zero-risk option for a pre-commit fix.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### F-P2-006 — `troubleshooting.md §2.6` documents only half the fix; the operator-visible half is missing

- **Location:** `docs/troubleshooting.md:426-479` (new section, this diff)
- **Severity:** Minor
- **Evidence:** the **Resolution** paragraph and the closing `Code:` line cite only the behind-network guard:
  > "the behind-network guard now covers `height == 1`, so a node whose peers report a materially higher tip defers production"
  > `Code: bins/node/src/node/production/mod.rs.`

  The no-evidence gate (`scheduling.rs:84-91`) is absent. That gate is the operator-visible half: a node with `bootstrap_nodes` configured and **no peer status yet** now refuses to produce **with no timeout escape**. The existing `bootstrap_timeout_secs` rescue (`scheduling.rs:147-174`, 180 s testnet/mainnet) sits **inside** the `has_bootstrap_nodes && !in_genesis` block at `scheduling.rs:99` and therefore cannot rescue a node at height 1. An operator whose producer sits silent with 0 peers will find nothing in troubleshooting.
- **Confidence:** `conf(0.95, observed)` — the diff text and the guard nesting are both read directly.
- **Liveness assessment (review dimension 4, answered):** **Yes, a bootstrap-configured node can wait indefinitely at `Unknown`.** Outside genesis this was already the behavior (pre-diff: `has_bootstrap_nodes && !in_genesis && peer_count == 0 → return None`), so the only NEW wedge surface is *at genesis*. It is acceptable in every shipped deployment because the origin gets an empty bootstrap list — verified: `scripts/install-local-services.sh:144` passes `--bootstrap` to `n${n}` producers only, and the seed plist does not. It is **not** acceptable to leave undocumented.
- **Suggested fix:** add to §2.6 — "a node with `--bootstrap` configured and zero peer statuses will not produce, indefinitely and by design (there is no timeout); check connectivity to the bootstrap address. The origin/seed node must be started with **no** `--bootstrap` flag." Add `scheduling.rs` and `peers.rs` to the `Code:` line.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — docs only)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     +~1KB (observed)
  Latency:  0 (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: the change makes a producer silently refuse to produce forever under a reachable misconfiguration; that behavior must be discoverable from the troubleshooting doc or it becomes a support incident.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### F-P2-007 — `inc_i_147_activation_height` is missing from the CLAUDE.md activation-height registry

- **Location:** `CLAUDE.md` ("If You Touch → activation heights"); `docs/DOCS.md`
- **Severity:** Minor
- **Evidence:**
  ```
  $ grep -n "inc_i_147" CLAUDE.md
  159:| System-impact gauntlet | scripts/gauntlet.sh ... replays INC-I-147/INC-I-148 ...
  ```
  The only hit is the gauntlet table row. The registry enumerates every other gate (`inc_i_026`, `fork_id`, `full_bitfield_decode`, `rewards_epoch_list_fix`, `encrypted_content`(+v2), `epoch_state_reorg`, `security_audit`, `ghost_exclusion`, `inc_i_068`, `defi`, `amm`, `inc_i_092`, `inc_i_096`, `large_block`) but not `inc_i_147_activation_height`. This commit promotes it from `u64::MAX` to a live mainnet height; leaving it unregistered means the next agent auditing activation heights will not see it. Also: `docs/DOCS.md` does not index the two new `docs/bugfixes/inc-i-149-*.md` files (`grep -n "inc-i-149\|bugfixes" docs/DOCS.md` → no output), and `DOCS.md` is modified-but-**unstaged**, so the index will not ship with this commit.
- **Confidence:** `conf(0.98, measured)` — grep output.
- **Suggested fix:** add `inc_i_147_activation_height` to the CLAUDE.md registry with mainnet 129_500 / testnet 80_700 / devnet 0 and the immutability note; index the two bugfix docs in `docs/DOCS.md` and stage it.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      N-A (documentation index; no code path)
  Memory:   N-A (documentation index; no code path)
  IO:       N-A (documentation index; no code path)
  Network:  N-A (documentation index; no code path)
  Disk:     +~0.6KB (observed)
  Latency:  0 (observed — CLAUDE.md is read per-session; +0.6KB against an 18,000-char budget)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: the registry is the sole mechanism preventing INC-I-054-class activation-height accidents; a live mainnet height absent from it is exactly the failure the registry exists to prevent.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### F-P2-008 — The 24.2 h lead is measured from the PIN, not from RELEASE PUBLICATION

- **Location:** `crates/core/src/network_params/defaults.rs:249-252`
- **Severity:** Minor (operational precondition, not a code defect)
- **Evidence:**
  ```
  $ grep -n "^version" Cargo.toml        ->  11:version = "6.24.1"
  $ git tag --sort=-creatordate | head -1 ->  v6.24.0
  ```
  The binary carrying `129_500` is **not published yet**. External producers upgrade through the updater, whose worst-case path is `CHECK_INTERVAL` 6 h (`crates/updater/src/constants.rs:116`) + `VETO_PERIOD` 5 min (`:13`) + `GRACE_PERIOD` 1 h (`:16`) ≈ **7.1 h ≈ 2,560 blocks** at 10 s slots. The stated lead is 8,701 blocks; subtracting the updater path leaves ~6,140 blocks of publication slack.
- **Confidence:** `conf(0.9, observed)` — constants read directly; tag list measured.
- **Suggested fix:** state the deadline explicitly — **the v6.24.1 release must be published before mainnet height ≈ 126,900** to preserve the full auto-update window for external producers. Put it in the commit body or the ops runbook next to the pin.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      N-A (release-schedule note; no code path)
  Memory:   N-A (release-schedule note; no code path)
  IO:       N-A (release-schedule note; no code path)
  Network:  N-A (release-schedule note; no code path)
  Disk:     +~0.2KB (observed)
  Latency:  0 (observed)
Inevitability: INEVITABLE
Cheaper alternative: NONE-EXISTS
Why this proposal anyway: without the deadline the lead time silently degrades from 24.2 h to whatever remains after publication, and a late release partitions external producers at the boundary.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### F-P2-009 — `SyncConfig::default()` fails OPEN on the INC-I-147 activation height

- **Location:** `crates/network/src/sync/manager/types.rs:54` (pre-existing, made material by this pin)
- **Severity:** Minor (latent)
- **Evidence:**
  ```rust
  // types.rs:54
  inc_i_147_activation_height: 0,          // always-on unless overridden
  ```
  I searched for a bypass and found **none in production**: the only non-test `SyncManager` construction is `init.rs:703-708`, which sets the field from `params`. All other `SyncConfig::default()` sites are `#[cfg(test)]` or `new_for_test` (`init.rs:1150`, `init.rs:1359`, `bins/node/tests/inc_i_089_startup_lockout.rs`, `crates/network/src/sync/manager/tests_*`).
- **Confidence:** `conf(0.9, observed)` — exhaustive grep of `SyncConfig::default()` / `SyncConfig {` / `ReorgHandler::new` across `bins` and `crates`.
- **Impact:** none today. Before this pin, a defaulted config would merely enable the fix early against a `u64::MAX` mainnet — harmless. After the pin, a defaulted construction site would use REAL heights below 129,500 while the fleet uses synthetic ones — fork-choice divergence inside the very window the AH exists to coordinate.
- **Suggested fix:** flip the default to `u64::MAX` (fail-closed) and let `init.rs` remain the sole enabler; tests that need it on set it explicitly.

```
━━━ RESOURCE COST — NEGLIGIBLE ━━━
Dimensions:
  CPU:      0 (observed — a const initializer value change)
  Memory:   0 (observed)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  0 (observed)
Inevitability: AVOIDABLE
Cheaper alternative: leave the default and rely on init.rs being the only construction site (true today, verified by grep).
Why this proposal anyway: "only construction site today" is a property that silently expires; a fail-closed default converts a future fork into a future test failure.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## Suggestions (P3)

### F-P3-010 — Two of three `NetworkEvidence` variants are dead at ship time
- **Location:** `crates/network/src/sync/manager/peers.rs:28-56`
- **Evidence:** `grep -rn "NetworkEvidence" bins crates | grep -v bins/node/tests/` returns only the definition, three re-export lines, one import, and the single match site `scheduling.rs:84`, which tests `== NetworkEvidence::Unknown`. `Unknown` is by definition `peer_count() == 0`, so the gate is behaviorally **identical** to the pre-diff `peer_count == 0` check — the entire behavioral delta in `scheduling.rs` is moving that check outside the `!in_genesis` gate. `AtGenesis` and `HasHistory` are never matched in non-test code.
- **Confidence:** `conf(0.97, measured)` — grep output.
- **Note:** documentation-as-code is a legitimate reason to name the concept. But F-P1-002 shows the two unused variants are already wrong, which is the predictable cost of shipping unexercised abstraction. No change recommended beyond fixing F-P1-002.

### F-P3-011 — Redundant `sync_manager` read-lock acquisitions on the ~1 Hz production path
- **Location:** `bins/node/src/node/production/scheduling.rs:54-61` and `:222`
- **Evidence:** the read lock is now taken **unconditionally** on every bootstrap-path tick (pre-diff it was taken only inside `has_bootstrap_nodes && !in_genesis`), and `peer_count` is captured at `:57` then re-read from a second acquisition at `:222`:
  ```rust
  // :54-61  (new, unconditional)
  let (peer_count, best_peer_height, evidence) = { let sync_state = self.sync_manager.read().await; ... };
  // :222   (pre-existing, shadows the above)
  let peer_count = self.sync_manager.read().await.peer_count();
  ```
- **Confidence:** `conf(0.9, observed)` — read from the diff; not benchmarked.
- **Hot-path Resource-Cost audit (required dimension):** the production entry point runs from a 1 Hz timer (`event_loop.rs:14`, corroborated by the test's own `POLL` comment). One extra `tokio::RwLock` read acquisition per second against a lock whose writers are peer-status updates. This is genuinely negligible in absolute terms — but the upstream proposal carried no Resource Cost statement for it, which is itself the finding.
- **Suggested fix:** drop the `:222` re-read and use the value captured at `:57`.

```
━━━ RESOURCE COST — COST-DECLARED ━━━
Dimensions:
  CPU:      -1 RwLock read acquisition/sec/node (observed — removes one of two acquisitions in the same function)
  Memory:   0 (observed — no allocation either way)
  IO:       0 (observed)
  Network:  0 (observed)
  Disk:     0 (observed)
  Latency:  -sub-microsecond p50 on the production tick (inferred — one fewer chance to queue behind a pending writer)
Inevitability: AVOIDABLE
Cheaper alternative: leave both acquisitions — the cost is unmeasurable at 1 Hz.
Why this proposal anyway: the second read can observe a DIFFERENT peer_count than the first, so the function silently reasons about two inconsistent snapshots of the same state; removing it is a correctness tidy, not a perf win.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### F-P3-012 — Module size budget exceeded in both touched source files
- **Location:** `crates/network/src/sync/manager/peers.rs` (705 lines, +45), `bins/node/src/node/production/mod.rs` (696 lines, +11)
- **Evidence:** `wc -l` output. Budget is 500 for source, 800 for tests (`inc_i_149_bootstrap_mint_gate.rs` is 799 — inside budget by one line).
- **Confidence:** `conf(1.0, measured)`
- **Note:** pre-existing overage; the diff grows both. No action recommended in this commit — flagged so it is not lost. `NetworkEvidence` would be a natural seed for a `peers/evidence.rs` split.

### F-P3-013 — The tests re-implement `network_evidence()` instead of calling it
- **Location:** `bins/node/tests/inc_i_149_bootstrap_mint_gate.rs:316-325`
- **Evidence:** the harness defines its own `enum Evidence` and `evidence_of()` that duplicates the production predicate (deliberately, per the docstring: "computed from the seams that existed BEFORE the fix ... so these tests were runnable — and RED — before the implementation existed"). Sound justification for the RED phase; but now that `network_evidence()` exists, the duplication means the truth table cannot detect divergence between the two — and cannot catch F-P1-002.
- **Confidence:** `conf(0.9, observed)`
- **Suggested fix:** add one assertion per row that `sync_manager.network_evidence()` equals the harness's `evidence_of()` classification, pinning the two together.

### F-P3-014 — `Path-Coverage:` block enumerates only one of the two new guard paths
- **Location:** `.git/COMMIT_EDITMSG` Path-Coverage block; `bins/node/src/node/production/mod.rs:183-198`
- **Evidence:** the block covers `scheduling.rs:84` with a four-partition Q3 enumeration. `production/mod.rs:183`'s `if network_height_ahead { ... return Ok(()); }` is not enumerated. The commit argues it is a "widened existing guard, not a new branch" — defensible, but the `return Ok(())` at `:196` is reachable at `height == 1` for the first time, which is a new path through an existing guard. The body text does cite `r5` (defer) and `p3` (mint) as the covering partitions, so the coverage exists; only the structured block omits it.
- **Confidence:** `conf(0.85, observed)`
- **Suggested fix:** add a second `Path-Coverage` entry for `production/mod.rs:183` with partitions `(tip==0)`, `(0 < blocks_behind <= 3)`, `(blocks_behind > 3)` and tests `r3/r4`, `p3`, `r1/r5`.

---

## Consensus Safety (review dimension 2) — CONFIRMED CLEAN

| Check | Result | Evidence |
|---|---|---|
| Does `NetworkEvidence` reach any validation rule? | **NO** | `grep -rn "network_evidence()" bins crates` → exactly one call site, `scheduling.rs:59`. Its only downstream effect is `return None` from `resolve_bootstrap_eligibility` (`scheduling.rs:90`) = skip this slot. |
| Does the diff touch validation? | **NO** | `git diff --cached --name-only` contains no `crates/core/src/validation*`. |
| Does the diff touch block content? | **NO** | No `production/assembly.rs`, no coinbase/header/tx-ordering/presence-root code in the staged file list. |
| INC-I-075 Q1 — can a user-submittable tx trigger this path? | **NO** | The path is the local production timer; no transaction reaches it. |
| INC-I-075 Q2 — can a producer-action/attestation pattern trigger it? | **NO** | Peer *status* messages move the classification, but the output is only "do I build this slot". A skipped slot is an ordinary empty slot, not a consensus-visible divergence. |
| INC-I-075 Q3 — bit-identical for all reachable inputs? | **NO** (intentionally) | With Q1 and Q2 both NO, the checklist's rule (`(1) or (2) YES and (3) NO → AH required`) does not fire. **No activation height required — agreed with the commit's reasoning.** |
| Would an AH even work here? | **NO** | Confirmed: the path executes at local height 0-1 on an empty disk; any real activation value makes a height-keyed gate read inactive forever. The design's argument (`inc-i-149-structural-design.md:119-123`) is correct. |
| Mixed-fleet fork risk | **NONE** | An un-upgraded wiped producer still mints its fossil; an upgraded one does not. The fossil was always rejected by peers, so the fleets do not diverge on validity. |
| `HardForkSchedule` entry added? | **NO** | Correct — CLAUDE.md forbids it for rolling deploys (`current_fork_id(u64::MAX)`). |
| `CURRENT_PROTOCOL_VERSION` / `EPOCH_STATE_FORMAT_VERSION` bumped? | **NO** | Correct — no `EpochState` format change; a bump would trigger `delete_epoch_state()` (INC-I-054). |

**Invariant conformance:** the change is the direct implementation of `INV-PROD-004`, already recorded in `.omega/memory.db`:
> *"A node must never build a block while its connected peers report a chain height materially ahead of its own, at ANY local height including the very first block. ... any guard keyed on peer-reported height is genesis-safe by construction, because at a real fresh genesis every peer reports height 0."*

Verified against the code: `production/mod.rs:172` keys on `best_peer_height()`, and at fresh genesis every peer reports 0 → `network_height_ahead == false`. **Genesis-safe by construction — claim confirmed.** No recorded invariant or lesson is contradicted by this change.

---

## Activation-Height Pin (review dimension 3) — CONFIRMED CORRECT

| Check | Result | Evidence |
|---|---|---|
| `129_500` > current mainnet tip | **PASS** (unverifiable here) | Tip `120_799` is asserted by the commit body and the task statement, measured this session on seed1. **I cannot verify it from this repository** — no mainnet RPC or explorer is reachable per CLAUDE.md's local-only environment note. Recommend the commit cite the raw RPC output. |
| Lead time arithmetic | **PASS** | `129_500 − 120_799 = 8,701` blocks × 10 s = 87,010 s = **24.17 h**. This is a *floor*: missed slots only lengthen wall-clock. Consistent with the AMM precedent (`367_660` @ tip `359_042`, ~23.9 h). |
| INC-I-054 immutability | **PASS** | Mainnet moves `u64::MAX → 129_500` — a **decrease**, and `u64::MAX` was never crossed. `INV-PARAMS-001` forbids only *forward* moves after crossing. |
| Testnet / devnet untouched | **PASS** | `git diff --cached -U0 -- crates/core/src/network_params/defaults.rs` shows exactly one value line changed (`:252`). `defaults.rs:411` testnet `80_700` and `:548` devnet `0` are outside the hunk. |
| Doc-drift fix in `mod.rs` | **PASS** | The field doc previously claimed testnet `0`; it now reads `80_700`, matching `defaults.rs:411`. Verified. |
| Does the pin change block validity? | **NO** | The gate feeds `ReorgHandler` fork-choice height recording (`reorg/mod.rs:145`) and node-local rolled-back-block re-adoption (`block_handling.rs:140-160`). Neither decides whether a block is valid — only which chain this node selects and whether it re-evaluates a body it already holds. |
| Boundary behavior of the mixed `block_weights` map | **SAFE** | `plan_reorg` decides `post_activation` from the ancestor's **real** height via the caller's `get_height` (`reorg/mod.rs:515-526`), never from the possibly-synthetic map value, so the boundary cannot mix units. `check_reorg_weighted` (`:374-378`) does read `w.height` with `unwrap_or(0)` — but pre-activation that is exactly today's behavior, and post-activation the near-tip ancestors carry real heights, so the pin can only *improve* it. One-reorg-deep window at the boundary retains today's behavior; bounded and self-healing. |
| Production bypass of the AH | **NONE** | Exhaustive grep: the only non-test `SyncManager`/`ReorgHandler` construction is `init.rs:703-708` → `ReorgHandler::with_activation_height(config.inc_i_147_activation_height)` (`sync/manager/mod.rs:198`). All `ReorgHandler::new()` sites are in `adversarial_tests.rs` / `reorg/tests*.rs`. See F-P2-009 for the fail-open default. |

---

## Test Quality (review dimension 5) — STRONG, NOT VACUOUS

8 tests, all pinning real truth-table rows. The vacuity defenses are unusually good:

- **Non-vacuity precondition on every negative row** — `assert_gate_authorizes` (`:223-233`) proves `SyncManager::can_produce()` returns `Authorized` *before* driving production, so "no block appeared" cannot be caused by an unrelated upstream block.
- **Matched pairs with identical budgets** — R2↔R4 (`OBSERVE_MATCHED_PAIR`, byte-identical node state differing in exactly `has_bootstrap_nodes`), R1↔R3, R5↔R4, P4↔P4c. R4 minting inside the same budget is what makes R2's negative meaningful.
- **Input assertions** — `assert_row_inputs` (`:330-354`) pins both truth-table inputs *and* local height, so R2 and R4 cannot silently collapse into the same row.
- **Observation windows justified** — `OBSERVE_NO_MINT = 12 s` is deliberately longer than one 10 s testnet slot so a negative covers every slot offset and at least one boundary (`:126-128`).
- **Magnitude reconciliation flagged, not hidden** — `assert_materially_behind` (`:365-375`) explicitly reconciles the design's literal `HasHistory ⇒ defer` against P3's `HasHistory ⇒ must mint`, resolving it by magnitude and pinning the reading. This is the honest handling of a genuine spec conflict.
- **Behavior-level assertions** — `assert_path_a` / `assert_path_b` check `last_produced_slot`, `chain_state.best_height`, and `block_store.get_block_by_height()`, not a guard's internals.
- **P4's setup was migrated when the fix made the old scaffolding illegal** (`:676-683`) and the migration is documented rather than silently weakened.

Weaknesses: **F-P3-013** (the harness duplicates `network_evidence()` rather than calling it, so it cannot catch F-P1-002). FAIL→PASS evidence is cited from the commit draft and the task statement (`8/8`, FAIL→PASS measured across the fix), **not re-measured by this review** — reviewer tooling is read-only.

---

## Specs/Docs Drift

| File | Status |
|---|---|
| `crates/core/src/network_params/mod.rs` | **FIXED by this diff** — testnet doc value corrected `0` → `80_700`, now matches `defaults.rs:411`. Good catch. |
| `docs/troubleshooting.md` | **PARTIAL** — see F-P2-006. Documents the guard, omits the gate. |
| `docs/bugfixes/inc-i-149-structural-design.md` | **DRIFTED from shipped code** — see F-P2-004 (item 3 not implemented) and F-P2-005 (item 2 placed elsewhere). |
| `CLAUDE.md` | **MISSING ENTRY** — see F-P2-007. Also modified-but-unstaged (unrelated OMEGA-protocol edits). |
| `docs/DOCS.md` | **NOT INDEXED / NOT STAGED** — the two new `docs/bugfixes/inc-i-149-*.md` files are unindexed; `DOCS.md` is modified but not in the commit. |
| `specs/protocol.md`, `specs/security_model.md` | **No update needed** — no wire format, message, encoding, or threat-model change. |
| `docs/rpc_reference.md`, `docs/cli.md` | **No update needed** — no RPC or CLI surface change. |

---

## Contradiction Check (intellectual honesty)

- **Stated fix vs actual change:** consistent. The commit says "the behind-network guard now covers height 1" and the diff removes exactly `height > 1 &&` at `production/mod.rs:183`. It says a no-evidence gate was added and `scheduling.rs:84` adds exactly that. No patch-instead-of-root-cause pattern.
- **One upstream contradiction, already self-disclosed:** the design doc's truth table asserts `HasHistory ⇒ MUST NOT produce` (row 1) while P3 requires a `HasHistory` node 2 blocks behind to mint. The design resolves it in a call-out (`:75-79`) and the test harness re-states the resolution at `:358-364`. **Acknowledged, not hidden — does not trigger rejection.**
- **One undisclosed gap:** the design doc's `## Known residual, stated not hidden` claims to enumerate the residuals; F-P1-003 shows one is missing. That is the reason F-P1-003 is P1 rather than P3 — a section that claims completeness must be complete.

---

## Modules Not Reviewed

None within scope. Blast radius outside the diff was bounded by reading every consumer of the changed symbols (`network_evidence`, `NetworkEvidence`, `inc_i_147_activation_height`, `best_peer_height`, `peer_count`) rather than by sampling.

---

## Final Verdict

**APPROVE WITH NOTES — merge after F-P1-001 and F-P1-002 (both ~5 min) and the F-P1-003 documentation addition.**

The engineering is sound. `INV-PROD-004` is implemented correctly, genesis liveness is preserved by configuration rather than by a timeout (a genuinely better design than a grace period), the truth-table test is one of the stronger regression suites in this repository, and the AH pin is arithmetically and procedurally correct. Nothing in the diff can produce a wrong block, change a validation rule, or fork a mixed-version fleet.

What holds it back from a clean approve: a new public type ships with two of three variants carrying a **false docstring contract** (F-P1-002), a section claiming to enumerate residuals **does not** (F-P1-003), and a new protection mechanism is **unregistered** in a project where the system-impact protocol is armed (F-P1-001).

---

━━━ SECURITY AUDIT VERDICT ━━━
Verdict: AUDIT-REQUIRED
Signals: (1) External data / trust boundary — `NetworkEvidence` is derived entirely from peer-supplied `StatusResponse` heights (`peers.rs:64` `add_peer`, reached from `on_peer_status`), and that untrusted input now gates whether this node produces a block; a peer that reports height 0 moves the classification from `Unknown` to `AtGenesis` and unlocks production (F-P1-003). (2) State integrity / consensus-adjacent — `inc_i_147_activation_height` moves from `u64::MAX` to a live mainnet height, arming fork-choice height semantics and rolled-back-block re-adoption on the production chain. (3) Enforcement surface — the diff changes a production-gating guard, i.e. the mechanism that decides when this node is permitted to extend the chain.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
