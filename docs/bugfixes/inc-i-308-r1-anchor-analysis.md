# INC-I-308 follow-up (run 734) — anchored peer writes + operator token tools

Base: `fix/rpc-admin-access` @ 7b7cdd8e (fix run 4cf8be85..7b7cdd8e). Trigger: the second fresh
verifier `docs/verifications/verify-4cf8be8531e6-7b7cdd8e5bcb.md` (main repo) gave FAIL.

Inputs (read these, do not re-derive):
- R-1 design (Fable): `docs/bugfixes/inc-i-308-r1-anchor-design.md` (this worktree)
- Code findings: `/Users/isudoajl/ownCloud/Projects/doli-network/doli/docs/.workflow/investigation-code.md`
- R-2 population baseline + test helpers: `/Users/isudoajl/ownCloud/Projects/doli-network/doli/docs/.workflow/investigation-constraints.md`
- Operator tools, P2-b, P2-c, P3: `/Users/isudoajl/ownCloud/Projects/doli-network/doli/docs/.workflow/investigation-state.md`

## Root cause (confirmed by code read, 3 reports agree)
`backfillFromPeer` and `repairArchiveFromPeer` accept a peer block when no STORED neighbor
contradicts it. A block at the bottom of a multi-block gap, or at h=1, is tied to our chain only by
its own `prev_hash` and a self-attested signature — both forgeable. The writes run ascending, so the
strong anchor (stored h+1 `prev_hash`) is never available below the first fetched block.

## Rule (R-1)
A peer block may be stored at height h only if its header hash equals the hash our own chain already
commits to at h: `chain_state.best_hash` at the tip, otherwise the `prev_hash` of the trusted block
at h+1 (stored, or accepted earlier in the same top-down walk). Each fill is one walk down from our
tip. A break stops the pass with an error; blocks written before the stop are anchored and stay.
Archive writes use the same anchor (index hash at h, or trusted h+1 `prev_hash`); signature and body
are always checked; the `prev`-alone branch goes. No consensus rule, no block content, no
activation height.

## Requirements
| ID | Requirement | Priority | Acceptance |
|---|---|---|---|
| R-1 | No unanchored peer block is ever stored | Must | Red on 7b7cdd8e, green at END: hostile peer (preflight answers our tip, self-signed blocks) refused at h=1, gap bottom, middle, top, for backfillFromPeer AND repairArchiveFromPeer; nothing written to store or archive; reply is an error, imported counts stored blocks only. Honest peer fills the same gaps completely. |
| R-2 | Impact per node population | Must | Matrix of 8 populations, each cell with test/scenario/log evidence. |
| R-3 | Operator token tools match the node rule | Must | Short env token (<32 after trim) + readable token file: CLI, shell helper, Python helper reach the admin method; `doli guardian halt` works; no usable token → hint names the exact cause and the file path. Red then green. |
| R-4 | Truthful backfill reply + real-producer parity | Must | No-gap reply never says "complete" when divergence_check is skipped (test). Real `verify_chain_integrity` output feeds the chainCommitment consumer (test). Script admin-method lists checked against server.rs (test). |
| R-5 | P3 clean-up | Should | rpc SKILL.md stale lines; `--data-dir=PATH`; audit test names/comments; "15 vs 13" answered. |

## Milestones
- M1 (R-1, R-4 backfill part): crates/rpc backfill/backfill_neighbor→anchor/guardian_repair_check + rpc tests.
- M2 (R-3, R-4 script-list parity, R-5): bins/cli rpc_admin_auth*, scripts/lib/rpc-admin-token.sh, rpc_admin_token.py, scripts/tests, .claude/skills/rpc/SKILL.md, audit test names.
- M3 (R-2 + gauntlet): R-2 matrix doc with evidence, hostile-backfill gauntlet row on run-started nodes, GS-056..061 on END binary.

## Removed behavior (decided, report to owner)
Phase 2 "repair" overwrite of stored blocks that differ from the peer (with `skip_divergence_check`)
becomes report-only. On 4cf8be85 it wrote the peer's branch into the store while ChainState/UTXO
stayed on ours — store and state on two branches. The supported way to change branch is the reorg
door (forceReorgTo, Run 4b). Gaps below the fork point still fill.

━━━ TRIAGE VERDICT ━━━
Path: FAST
Confidence: conf(0.85, measured — 3 code-read reports agree on the same write sites)
Reasoning: deterministic admission rule in 2 RPC handlers + operator helpers; localized; reproducible with an in-process fake peer.
━━━━━━━━━━━━━━━━━━━━━━
