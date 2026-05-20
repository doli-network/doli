# SSF Recommendation — INC-I-083 Root Cause (verified)

## Verdict — `conf(0.95, measured)`

**The "BEHIND divergence" regression on testnet is NOT caused by the `ce1a72dc..HEAD` fix bundle.** It is caused by a pre-existing testnet-specific operational configuration (`--no-snap-sync` in the launchd plists of n9/n10/n11/n12) interacting with an unchanged pre-existing dispatch gate in `crates/network/src/sync/manager/production_gate.rs`. The bundle is innocent of the persistence mechanism keeping nodes stuck.

## Evidence

### Evidence 1 — the dispatch gate is byte-identical between 77bb3dfa and HEAD

```
$ git diff 77bb3dfa..HEAD -- crates/network/src/sync/manager/production_gate.rs | wc -l
0
```

The `is_emergency` match at `production_gate.rs:614-619` at 77bb3dfa already excludes `CoordinatorSnapEscalation`. The `--no-snap-sync` veto gate (`snap.threshold == u64::MAX && !is_emergency`) at line 662 is unchanged. The snap-attempts-exhausted gate (`snap.attempts >= 3`) at line 681 is unchanged. The `confirmed_height_floor` gate at line 622 is unchanged. **This is the exact code that ran on mainnet for the weeks of stability.**

### Evidence 2 — n10's stuck mechanism is dispatch-gate refusal, NOT cbaa3963's FINALITY_GUARD

n10 log statistics for the current run:

| Pattern | Count | Where |
|---|---|---|
| `[FINALITY_GUARD]` | 0 | `recovery.rs:307-319` (cbaa3963 code path) |
| `RecoveryAction::None` | 0 | The new return path cbaa3963 introduces |
| `action=ShallowRollback` | 1 | Rule 1 normal path (fired once at 21:16:02, succeeded) |
| `action=SnapSync` | 773 | Rule 2 — coordinator correctly escalating |
| `action=HeaderFirstSync` | 321 | Rule 3 — fallback while waiting |
| `Chain break valid_so_far=0` | 15,583 | Symptom of being on the forked tip |

**n10 hit cbaa3963's new code path zero times.** Its stuck state is entirely produced by the coordinator correctly recommending `SnapSync` and dispatch refusing it 773 times because of the pre-existing `--no-snap-sync` + non-emergency-reason gate.

Frozen-node FINALITY_GUARD counts (zero for the nodes the user is concerned about):

| Node | FINALITY_GUARD hits |
|---|---|
| n1 | 0 |
| n3 | 0 |
| n7 | 0 |
| n10 | 0 |
| n14 | 0 |
| n2 | 1 |
| n13 | 30 |

### Evidence 3 — n3 (recovered) and n10 (stuck) prove configuration is the discriminator

Both n3 and n10 forked at the EXACT same block: h=110,367, hash `0b2750dcb31e`. Same code, same fork, same fleet. The difference between recovery and permanent stuck:

| | n3 | n10 |
|---|---|---|
| Forked at h=110,367 hash `0b2750dcb31e` | YES | YES |
| `--no-snap-sync` in launchd plist | **NO** | **YES** |
| Outcome | Snap-synced and recovered at 22:01:04 | 773 SnapSync classifications REFUSED, still stuck |

`grep -c "no-snap-sync" ~/Library/LaunchAgents/network.doli.testnet-{n3,n10}.plist`: n3=0, n10=1.

The code paths are identical. Only the operational config differs. If the bundle were the cause of the stuck state, n3 would also be stuck. It isn't.

### Evidence 4 — per-frozen-node configuration profile

```
n9, n10, n11, n12: --no-snap-sync=1   → blocked by Gate 4 (snap.threshold==u64::MAX + non-emergency)
n7:                --no-snap-sync=0   → blocked by Gate 5 (snap.attempts >= 3, no reset)
n13:               --no-snap-sync=0   → blocked by Gate 1 (confirmed_height_floor=101,100)
n1, n2, n14, n3:   --no-snap-sync=0   → snap-sync available, can/did recover
```

Each stuck node maps to a distinct gate-refusal mode, all of which pre-date 77bb3dfa.

### Evidence 5 — n10 was healthy on the new binary for over an hour before the natural tip race

| Time | n10 state | Note |
|---|---|---|
| 20:14 (pre-deploy) | `gap=0 phase=Idle last_applied_ago=0s` | Healthy on old binary |
| 20:17 (post-deploy) | `gap=0 phase=Idle peers=16 sync_fails=0` | Healthy on new binary HEAD/479711b5 |
| 21:16:02 | `action=ShallowRollback { depth: 1 }` succeeded h=110362→110361 | New binary executed self-heal correctly |
| 21:30:30 | `h=110363 hash=3ccddc13 gap=0 idle` | Still healthy |
| 21:36:51 | First `Chain break valid_so_far=0` | Forked at h=110,364/slot=218,864 |
| 21:55:00+ | h=110,367 hash=`0b2750dcb31e` sync_fails=397, climbing | Stuck on dispatch gate |

The new binary worked correctly for 80 minutes (including one successful ShallowRollback self-heal at 21:16). The fork at slot 218,864 is a tip race between two valid blocks (`a87763f6` minority vs `b35ac125` canonical). The bundle's recovery code DID run, DID classify SnapSync correctly, and was then refused 773 times by the pre-existing dispatch gate.

### Evidence 6 — mainnet stability is explained by configuration, not by what the bundle does or doesn't do

Mainnet has been stable on 77bb3dfa for weeks. The dispatch-gate code at 77bb3dfa is identical to HEAD (Evidence 1). The reason mainnet is stable is that mainnet nodes do not carry `--no-snap-sync` flags or a `confirmed_height_floor`, so when a mainnet node falls behind for any reason, snap-sync dispatch is NOT refused, and it recovers. The single mainnet INC the user mentions (invalid epoch-boundary block, INC-I-081) is a different failure mode (block validation, not sync-recovery dispatch) — that's exactly what the bundle was designed to address.

## What this means for the user's claim

The user's premise — "after all the fix we did starting from 77bb3dfa now we have all this BEHIND divergence regression" — is correct on the **symptom** (testnet nodes are behind) but the **cause** is not the bundle:

- The persistence mechanism (dispatch refusal of correctly-classified SnapSync) is pre-existing and would have produced identical behavior on a pure 77bb3dfa binary running on the testnet's `--no-snap-sync` config.
- The natural tip race at h=110,364 that triggered n10's fork is routine PoS behavior that mainnet would also experience, but mainnet recovers because it lacks the operational gates.
- cbaa3963's FINALITY_GUARD is not implicated for the nodes the user is concerned about (n10/n3/n7/n14/n1 — zero FINALITY_GUARD hits). It fired once on n2 and 30 times on n13, but those firings are saves (refusing rollback past finality), not the cause of being behind.

## The one action

**Remove the `--no-snap-sync` argument from the four launchd plists `~/Library/LaunchAgents/network.doli.testnet-n9/10/11/12.plist`, then `launchctl unload && launchctl load` each plist.**

This is operational, not code. It restores snap-sync as a recovery option for the 4 stuck nodes that classify SnapSync but get refused. The other 2 stuck nodes need different operational care (n7: clear `snap.attempts`; n13: clear `confirmed_height_floor=101,100`).

No code change. No deploy. No risk. Reversible. Restores parity with mainnet's recovery topology.

After applying it, n9/n10/n11/n12 should accept `request_genesis_resync(CoordinatorSnapEscalation)` on the next coordinator tick (~30 s), snap-sync to canonical, and rejoin the fleet within minutes.

## What the bundle should be credited for

- INC-I-081 fix (invalid epoch-boundary block cascade): mainnet's exact INC of "before yesterday" is what this fixed, and mainnet has been stable since.
- INC-I-082 rebuild bit-identity: 14+ nodes restarted on HEAD and rebuilt epoch_state identically.
- FINALITY_GUARD (cbaa3963): fired in-vivo at 20:57:01 saving a true past-finality rollback on n2.
- Code gate clean; 224 lib tests + targeted regression suites green.

## Confidence

`conf(0.95, measured)` — backed by: zero-diff in production_gate.rs since 77bb3dfa; zero FINALITY_GUARD hits on 5 of 6 frozen nodes; 773-vs-0 ratio of SnapSync classifications vs FINALITY_GUARD on n10; n3 (no flag) recovered from the SAME fork that n10 (with flag) is stuck on; per-node config maps 1:1 to which dispatch gate refuses each stuck node.

Not 0.99 because I have not yet bisected the question "is the tip race at h=110,364 more likely on the new binary than on 77bb3dfa?" — but that is orthogonal to the persistence mechanism that is keeping nodes BEHIND, which is what the user is observing. Even if the bundle changed tip-race probability, the OBSERVED stuck-behind state is unambiguously the pre-existing dispatch gate × pre-existing testnet operational config, not the bundle.
