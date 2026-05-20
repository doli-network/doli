# INC-I-083 — Testnet Regression-Validation Report

**Scope:** fix batch `ce1a72dc..HEAD` (`641c00f6`) — INC-I-078/079/080/081/082
**Environment:** LOCAL testnet (`~/testnet/`, launchd, 127.0.0.1, 18 nodes: seed + n1–n17)
**Run:** workflow 342 · branch `main` · 2026-05-19

---

## Executive Verdict: ✅ PASS (with required deploy correction performed)

The fix batch behaves as designed. A premise error was caught and corrected mid-validation:
the testnet was **not** running HEAD — it was on `3faeccc0` (HEAD−1, missing INC-I-082).
After an approved synchronized HEAD deploy + n2 clean resync, the **full 18-node fleet
converged bit-identically on HEAD**, and the pre-existing n2 anomaly was **resolved**.

---

## Per-incident results

| Incident | Fix | Evidence | Verdict |
|---|---|---|---|
| **INC-I-079** | economic-sim INV-4 halving fix (test-only) | `economic_sim_s1`/`s2` + lib suite green | ✅ PASS |
| **INC-I-078** | per-producer `received_delegation_cap` + Ed25519 auth on DelegateBond/RevokeDelegation | `delegated_bond_attestation` + 224 lib tests green; testnet AH=109_559 crossed (active); 0/17 producers exceed cap=3000; chain liveness intact | ✅ PASS (over-cap path via regression tests — testnet has 0 live delegations) |
| **INC-I-080** | AddBond cap: reject post-AH / clip pre-AH | `addbond_cap_overflow` + lib tests green; activation active on testnet; normal bonds unaffected | ✅ PASS (over-cap path via regression tests) |
| **INC-I-081** | E613 sync-cascade hotfix bundle (epoch abort, ShallowRollback finality guard, plan_reorg fallback, direct-apply fallback, finality reset) | `fork_recovery` 10✓, `inc_i_081_direct_apply_fallback` 2✓, `inc_i_081_incomplete_store_aborts_slot` 3✓; on 3faeccc0 fleet survived epoch boundary h=109728 with perfect convergence; n2 self-healed a 1-block fork (`shallow_rb=0`, single-block rollback); on HEAD 18/18 converge incl. fresh-snap n2 | ✅ PASS |
| **INC-I-082** | `rebuild_epoch_state_from_blocks` bit-identity + explicit `target_height` | `inc_i_082_rebuild_safety` 8✓ (incl. caller-contract vs `post_commit`, defect1/2/3); **on HEAD 14+ independently-restarted nodes rebuilt epoch_state to bit-identical 3-state; freshly-wiped n2 snap-synced on HEAD and converged to canonical** | ✅ PASS |

## Code gate (M1)
`cargo build --release` ✓ · `cargo clippy --workspace --all-targets -- -D warnings` ✓ · `cargo fmt --check` ✓ · regression suites: libs **224 passed / 0 failed**, all incident-specific targets **0 failed**.

## Live-testnet regression test (M5 — pure operations, no code)
Recreated the historical INC-I-081 trigger on the running HEAD testnet by **operationally partitioning 3/14 producers (n6/n7/n8)** for ~75 s, then healing:
- **Partition:** majority **15/15 stayed bit-converged**, chain kept finalizing (`lastCompleteEpoch` stable, h advanced) — liveness held, no majority fork.
- **Heal/recovery:** fleet re-converged groups **3 → 2 → 1**, ending **18/18 bit-identical h=110047 `8c60aedbd0`** in ~1 min; n6/n7/n8 `Synchronized`; **no** FINALITY_GUARD/ShallowRollback/Chain-break/Empty-headers/SNAP thrash; **no permanent fork, no stuck node.**
- **Verdict:** the INC-I-081 *system* regression (producer subset disagrees → sync amplifies → permanent fleet fork + stuck nodes) **does NOT reproduce on HEAD**.
- **Attestation/epoch path (in-vivo, log-observed):** fleet has **REJECTED invalid epoch blocks** fleet-wide — `[ECON_EPOCH_MISSING] missing EpochReward TX` (the exact INC-I-081 root cause) and hundreds of `[ECON_EPOCH_NOT_BOUNDARY]` — **without cascading**. Defense observably active.
- **OLD-vs-HEAD differential (observed live this session):** freshly-wiped **n2 on `3faeccc0` = chronic post-snap fork-recovery deadlock**; same **n2 on HEAD = clean snap-sync + converge**. Regression manifested on old binary, resolved on HEAD.
- **Honest scope limit:** `cbaa3963` Bug-1's `[FINALITY_GUARD]` log was **not** forced in-vivo — it is a last-resort guard that only triggers on a competing-tip-past-finality state, which the upstream fixes prevent nodes from reaching; clean operational disruption takes the safe HeaderFirstSync path. Forcing that exact branch needs injected bad state (out of scope) or a pre-fix differential binary (build OOM-blocked by the running testnet). Its FAIL→PASS proof is the commit's unit test `classify_refuses_shallow_rollback_below_finality` — **re-verified PASS on HEAD this session** (8 passed/0 failed); **FAIL pre-fix documented in the commit**. Deployed binary contains the guard (`strings` → 2× `FINALITY_GUARD`).

## Fleet convergence (M2) — on HEAD/INC-I-082
- 18/18 nodes **bit-identical** `(height, stateRoot, csHash, psHash, utxoHash)`, sustained:
  - h=109819 `1da1dbada1dc` (20:24:26) — incl. n2 fresh-snap (h=7→109819)
  - h=109822 `25cedbeaa160` (20:24:58) · h=109829 `1a0fe90ba711` (20:26:00) · h=109832 `effd02b45266` (20:26:31)
  - **Independent spot-check h=109834: single state group `(109834, 8dd212524f7c) → 18/18`, zero divergence**
- **Epoch processing on HEAD healthy:** `lastCompleteEpoch=3049` while fleet stayed 18/18 converged through epoch 3049 completion (epochStart 3050 = h=109800) into epoch 3050 — INC-I-082 rebuild + epoch-reward path exercised at the boundary with no divergence.
- **Original n2 anomaly RESOLVED:** on `3faeccc0` n2 was in a chronic post-snap fork-recovery deadlock (local fork tip not in any peer chain → `Chain break valid_so_far=0` retry loop, `sync_fails=134+`, stuck 8+ min). On HEAD, wiped n2 snap-syncs and converges in ~30s.

## Functional cap/auth (M3)
Testnet AH=109_559 < current height → INC-I-078/080 caps + Ed25519 auth **active**. No producer exceeds `received_delegation_cap` (max delegatedBonds=0, cap=3000). Chain advancing — happy path intact. Over-cap rejection / bad-signature paths validated by regression tests (not reproducible live: 0 delegations on testnet, cap=3000 bonds impractical to reach in-session).

---

## Findings & follow-ups (not fix-batch defects)

1. **[Premise correction — performed]** Testnet ran `3faeccc0`, not HEAD. Caught by md5/commit verification (not trusting `--version`). Synchronized HEAD deploy executed with approval.
2. **[Build tooling — deferred, non-consensus]** `bins/node/build.rs:5` embeds `git rev-parse --short HEAD` with **no `cargo:rerun-if-changed=.git/HEAD`** → the binary's `--version` commit string goes stale (showed `3faeccc0` even when built from `641c00f6`). This is the root cause of the "is testnet on HEAD?" ambiguity. Recommend a 1-line build.rs fix so deployed binaries self-identify correctly. **Deferred** (rebuild churn; non-consensus).
3. **[Operational — resolved in-session]** Synchronized restart of all 18 nodes on a single-seed local testnet causes thundering-herd dial contention → some nodes (n2/n3/n4/n5) land at 0 peers and stall. Resolved by **staggered restart**. Not an INC-I-078..082 defect. Future synchronized testnet deploys should stagger producer starts after the seed.
4. **[Material — what was actually validated]** The deployed testnet binary = **committed HEAD (`641c00f6`) + the user's UNCOMMITTED `crates/core/src/network_params/defaults.rs`** (built from working tree, not a clean HEAD checkout). That uncommitted change (authored by the user, present before this workflow; Claude made **zero** source edits) does two things:
   - **Mainnet**: re-pin INC-I-078/080 activation `231_830 → 240_138` (3 gates). User's in-code comment argues this is the routine pre-activation case (chain not crossed, no binary honors it → not INC-I-054). Mainnet-scoped — **not touched, not committed here**.
   - **Testnet**: pin the same 3 gates `0 → 109_559` (dress-rehearsal of the mainnet event). **This is the source of the "testnet AH=109_559 active" result in M3** — it comes from the uncommitted change, not committed code.
   Therefore this report validates *HEAD + the user's uncommitted AH pins* (the realistic pre-deploy state), **not** pure committed HEAD. The uncommitted `defaults.rs` remains uncommitted pending the user's decision.

## Confidence
Fix-batch correctness: **conf(0.9, measured)** — FAIL→PASS regression evidence + live 18/18 bit-identical fleet convergence on HEAD across the observation window incl. fresh-snap recovery. INC-I-078/080 over-cap rejection path: **conf(0.8, measured-by-regression-test)** (not exercised by live traffic).
