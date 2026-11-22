# INC-I-308 R-2 — population matrix for the backfill anchor rule

Run 734, milestone M11 (M3a). END = `dbdae50e` (anchor rule, R-1). BEFORE = `4cf8be85`.

Question: under the anchor rule, can each node population still fill its block-store gaps from an
HONEST peer? A peer block is stored at `h` only when its header hash equals what our chain commits to
at `h`: `best_hash` at the tip, else `prev_hash` of the stored block at `h+1`. The walk runs top-down.

Evidence for the END column is a test in `crates/rpc/tests/it/inc_i_308_r2_populations.rs` (all 10
pass: `cargo test -p rpc --test it inc_i_308_r2` → `10 passed; 0 failed`), or an R-1 test in
`inc_i_308_r1_anchor.rs`, or gauntlet row GS-063 (written, not run yet).

The BEFORE column is **code-read, not run**: the `4cf8be85` walk fetched every missing height and
stored any decodable block that passed signature/body checks, with no hash link to the chain
(`git show 4cf8be85:crates/rpc/src/methods/backfill.rs | grep -c prev_hash` → 0). When there were no
gaps and the peer commitment differed, Phase 2 (`repair_heights`) overwrote the divergent heights
with the peer's blocks.

| # | Population | Gaps | Upper anchor | Fills on 4cf8be85 | Fills at END | Evidence (END) | Verdict |
|---|---|---|---|---|---|---|---|
| 1 | Archive / full node | none | — | nothing to fill; Phase 2 could overwrite on a commitment mismatch | nothing to fill, store not changed; archive repair from an honest peer writes h1..h5 | `r2_p1_full_node_has_nothing_to_fill_and_keeps_store`, `r1_repair_archive_honest_peer_full_store` | OK |
| 2 | Snap-synced external producer | `[1, snap_h-1]` | header-only block at snap_h (`install_contract.rs:62-73`) | yes | yes, 4 of 4 under a header-only anchor | `r2_p2_snap_synced_node_fills_gap_under_header_only_anchor`, `r1_backfill_honest_peer_fills_snap_gap`; live: GS-063 `gs063-honest-fills` (not run) | OK |
| 3 | Middle holes | separate holes 3-4, 8-9, 11 | stored block above each hole | yes | yes, all 5 | `r2_p3_middle_holes_all_filled`, `r1_backfill_honest_peer_fills_every_position` | OK |
| 4 | Pruned node | `[1, cutoff-1]` after `prune_blocks_below(6, 2006)` | stored block at cutoff | yes | yes, h1..h5 | `r2_p4_pruned_node_refills_below_cutoff` | OK |
| 5 | `--no-snap-sync`, behind the peer | own holes below own tip; peer tip higher | stored block above the hole | yes | yes; nothing above our tip is stored | `r2_p5_node_behind_peer_fills_own_gaps_only` | OK |
| 6a | Losing branch, default call | gap below the fork | — | no: tip preflight refuses | no: tip preflight refuses; nothing written | `r2_p6_losing_branch_default_refused_by_preflight` | Same as before |
| 6b | Losing branch, `skip_divergence_check`, gap below fork only | gap below the fork | our own branch (f4 links to common h3) | yes | yes below the fork; our branch f4..f6 kept | `r2_p6_losing_branch_skip_fills_below_fork_keeps_branch` | OK |
| 6c | Losing branch, `skip_divergence_check`, also a hole ABOVE the fork | gap below + hole in our branch | our branch above the hole | yes, but it stored the PEER's block into our branch hole (mixed chain) | no: the walk breaks at the upper hole (`stopped at height 5`), so the gap below the fork stays open in that pass | `r2_p6_losing_branch_hole_above_fork_stops_walk` | PARTIAL — path lost; tool = reorg door (`forceReorgTo`) or a peer on our own branch |
| 6d | Losing branch, no gaps, commitment differs, `skip_divergence_check` | none | — | Phase 2 overwrote our branch with the peer's | report only (`divergence_check: diverged…`), no write | `audit_p1_rpc_005_backfill_reports_divergence_without_overwriting_own_branch` | DELIBERATE REMOVAL (R-1) |
| 7 | Old-binary peer in a mixed fleet; peer missing a block mid-gap | gap 2-6; peer has no h4 | stored h7 | yes (it skipped the missing height) | first pass stores h6, h5 and stops at h4 with nothing below; a second honest peer fills h2..h4 | `r2_p7_peer_missing_block_then_second_peer_completes`, `r1_backfill_stops_where_peer_has_no_block` | OK with a second peer; one peer with a hole no longer fills below the hole |
| 8 | Fresh restart (store reopened from disk) | holes 2-3, 5 | stored blocks read from the reopened RocksDB | yes | yes, 3 of 3 | `r2_p8_fresh_restart_reopened_store_fills` | OK |

## Notes

- **Population 7, wire shape.** The in-process peer stub (`audit_rpc_effects::peer_rpc`) answers
  `getBlockRaw {block, blake3, height}` and `getBlockByHeight {hash}`, which is the
  `4cf8be85`/`7b7cdd8e` reply shape. `getBlockRaw` and `getBlockByHeight` are public methods, so an
  old peer's admin gate does not affect the walk. Only the no-gap commitment report
  (`verifyChainIntegrity`) can be refused by an old gated peer; that changes the message, not a store.
  UNTESTED against a real old binary: GS-063 runs against one honest testnet node of whatever version
  the fleet runs (v6.46.1 at the time of writing).
- **Population 6c is a real loss** compared with `4cf8be85`, but the `4cf8be85` behaviour stored a
  block of the other branch into our branch, which is the INC-I-308 defect. The node needs the reorg
  door first, then a backfill.
- **Population 7, single peer with a hole.** `4cf8be85` stored the heights below the peer's hole
  without a link check. END stops; any second honest peer completes the fill (tested).
- **Live evidence.** GS-063 (`scripts/gauntlet-gs063.sh`) is written and syntax-checked only
  (`bash -n`, `python3 -m py_compile`); the run on the END binary is a later step. h=1 and the gap
  middle are not reached by a bounded top-down live walk; R-1 tests
  `r1_backfill_refuses_forged_h1_and_snap_gap` and `r1_backfill_refuses_forged_gap_middle` cover them.
