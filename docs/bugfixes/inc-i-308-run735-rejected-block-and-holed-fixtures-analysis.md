# INC-I-308 run 735 — rejected peer block stops the honest fill; GS-045/GS-052 holed-store fixtures

Branch `fix/rpc-admin-access`, start END `e31a0f3c`. Run 735.

## Shared testnet at start (2026-11-21 23:02 WET)
| node | PID | tip |
|---|---|---|
| n1 | 30089 | 503645 bd07b4f31ccb091b |
| n2 | 30430 | 503645 bd07b4f31ccb091b |
| n3 | 30541 | 503645 bd07b4f31ccb091b |
| n4 | 35431 | 503645 bd07b4f31ccb091b |
| n5 | 37039 | 503645 bd07b4f31ccb091b |
| n6 | 37085 | 503645 bd07b4f31ccb091b |
| seed | 77975 | 503645 bd07b4f31ccb091b |

## Architecture context
`backfillFromPeer` (crates/rpc/src/methods/backfill.rs) computes the missing heights and hands them to
`Walk::run` (crates/rpc/src/methods/backfill_anchor.rs). The walk goes top-down. For each height h it
fetches the peer block and calls `admit(block, h, want)`: `Break` when the hash differs from the hash our
chain commits to, `SkipKeepChain` when the hash matches but the signature or body is rejected, `Store`
otherwise. After any non-Break verdict, `want = block.header.prev_hash` (the header is hash-committed,
so its prev_hash is a commitment of our chain even when the block is not stored).

`Walk::store_at` (added by d3d3e00f, M12) re-derives the anchor at write time from the CURRENT store:
`anchor_above(store, h, best_height, best_hash)` = best_hash at the tip, else prev_hash of the STORED block
at h+1. Not a consensus path: no block or tx validation calls this module.

## Probable cause (conf 0.85, code reading; red test to confirm)
When the block at h+1 was `SkipKeepChain` (rejected, not stored), `store_at(h)` finds no stored block at
h+1, gets `None` and stops the pass with "no stored block at h+1". So one rejected block stops every
honest block below it. d3d3e00f made this deliberate ("A block below a signature/body-rejected block is
no longer stored"). It breaks the must-keep tests `req_auth_019_p13_backfill_forged_not_stored`
(tip block forged/unsigned → h1..h3 must be stored) and `backfill_presence_junk` (tip body variant →
h1..h3 must be stored).

## Fix direction (R-1 stays as strong)
The write-time re-check must also accept a commitment that passes through blocks this walk verified by
hash but rejected: anchor(h) = prev_hash of the rejected block at h+1 **only if** anchor(h+1), re-derived
from the current store in the same way, still equals that rejected block's hash. Every stored block then
still has a hash chain to the CURRENT best_hash; nothing unanchored is written; the reorg protection of
M12 stays (if the chain moved, the re-derived chain no longer reaches the rejected block's hash).

## Requirements
| ID | Requirement | Priority | Acceptance |
|---|---|---|---|
| R-1 | A rejected block does not block an honest fill | Must | `req_auth_019_p13_backfill_forged_not_stored` red at e31a0f3c, green at END; new test: peer A serves a body/sig-rejected block at h (hash matches) → after A's pass every height except h is stored (red at e31a0f3c); then honest peer B fills [1..tip] completely; 11 tests in inc_i_308_r1_anchor.rs pass; M12 reorg test still passes |
| R-2 | INV-FORK-007 kill proof | Must | `backfill_presence_junk` green; mutation_proof.py output KILLED |
| R-3 | GS-045, GS-052 run again | Must | holed store built without unanchored write (no backfillFromPeer bottom-up partial fill); same hole heights/count as old fixture (both lists quoted); pass rule unchanged in diff; PASS on END binary |

## Hydra note
The incident holds 4 confirmed root_cause/discovery rows. They are separate audit findings of one
audit ticket (R11 has 10 P1 findings), not a cascade where each fix exposes a new subsystem bug. The
operator brief orders this short run; no redesign escalation.

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.85, code reading of backfill_anchor.rs:133-195 and the commit text of d3d3e00f)
Reasoning: one function (Walk::store_at re-check) in one file; deterministic; the fixture work is a script-only change.
━━━━━━━━━━━━━━━━━━━━━━
