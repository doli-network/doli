#!/usr/bin/env bash
# gauntlet-gs033.sh — dead-branch fork scenarios on the LOCAL testnet (INC-I-281, INC-I-235):
#   GS-004 one-block-fork-recovery, GS-007 rollback-rejoin, GS-022 losing-sibling-fork-walk,
#   GS-029 mixed-version-gate-crossing (testnet form), GS-033 dead-branch-minority-above-activation-height.
# A FAIL means a node on a short dead branch that starts at an epoch boundary with a diverging
# producer list stays on it, goes back onto it, recovers only by snap sync, loses heights, or a
# mixed-version fleet does not converge, so the fork-recovery fix must not ship.
#
# Usage: scripts/gauntlet-gs033.sh <gs004|gs007|gs022|gs029|gs033>
#        scripts/gauntlet-gs033.sh --plan [scenario]   read-only: RPC plan + timeline, no writes
#        scripts/gauntlet-gs033.sh --restore RUN_DIR   recover after an injection process died
#
# INJECTION (scripts/gauntlet-gs033/inject.py, live testnet above the activation height): producer
# X stops right after its own block 112-140 blocks before boundary B (its last attestation leaves
# the attestation window of B). The minority (default n4,n5) runs behind userspace forwarders
# (127.0.0.1 only; --no-dht --no-discv5, <= 5 bootstraps) and is cut 5 s before slot S of height
# B; it builds the dead boundary block D_B (len2: and D_{B+1}) without X, so it freezes one
# producer fewer at B; X restarts isolated, is released at the cut and builds the canonical B with
# X in its list. Heal 4 s before the minority's next dead-list slot, with the canonical branch
# heavier; observe GS033_OBSERVE slots. Shapes: len2 = 2-block dead branch (GS-007/022/029/033),
# len1 = 1-block dead branch (GS-004). Both are feasible with minority n4,n5 on the 7-node testnet
# (len1 needs S = 2 mod 20 so the two slots after S are not minority slots on the dead list;
# --plan prints a feasibility table for every minority).
# ONE injection per shape per session: the first scenario that needs a shape runs it; the others
# judge its recorded logs. Cache: $GAUNTLET_TN_SESSION/<shape>/<key>/, key = sha256 of the
# shape, the minority, the observe window and the sha256 of every node's effective binary.
# Every injection ends with its own restore (scripts/gauntlet-gs033/restore.py).
#
# OUTPUT CONTRACT: one `PASS|FAIL <id> <measured k=v>` line per assertion (every assertion over
# every minority node), then the METRIC line (last line), then the exit code. A copy goes to
# $GAUNTLET_TN_SESSION/report-<scenario>.txt (gauntlet-run.sh discards stdout).
#   <s>-shape                       injection completed; 0 isolation leaks (pre-cut, cut+10, pre-heal);
#                                   each minority node held exactly dead_len dead blocks at heal; the
#                                   canonical branch was heavier; dead and canonical lists differ at B
#   gs004|gs022|gs033-canonical-within-30-slots  on the seed's chain <= 30 slots after the heal and
#                                   never off it again in the window
#   gs004|gs022-by-reorg            `Executing reorg`/`Reorg complete` between heal and rejoin+2 slots
#   gs022-fork-walk-path            a fork-walk (`[FORK] RECOVERY_START`/`RECOVERY_DONE … reorged`) or
#                                   `[WEDGE_ESCAPE]` line in that window
#   gs022-no-ineligible-canonical   no `is ineligible`/`Dropping ineligible`/`invalid producer for slot`
#                                   line that names a block of the seed's chain
#   gs007-rejoined                  on the seed's chain by the end and stays there
#   gs007-no-return-after-rejoin    after the first canonical standing: no sample off the seed's chain
#                                   and no `[BLOCK] Applied` of a dead block
#   gs007-rollbacks-rejoin          no ROLLBACK_DONE followed by a dead apply that stays > 30 slots
#   gs033-no-return-after-rollback  as gs007-no-return-after-rejoin (the brief's "goes back onto it")
#   gs004|gs022|gs033-no-snap-sync  no client snap-sync line after the heal; block 1 = the seed's
#   gs004|gs007|gs022|gs033-no-missing-heights  verifyChainIntegrity(B-100..tip) complete, missing 0
#   gs029-mixed-binaries            >= 2 distinct `doli-node --version` among the 7 nodes and every
#                                   TN_BINMAP node still ran its mapped binary at the end (FAIL if not mixed)
#   gs029-one-tip                   all 7 nodes on one tip hash at the end of the observation
#   gs029-history-agrees            getBlockByHeight at 1, 300000 and tip-10 equal on all 7 nodes
#   gs029-no-wedge-after-heal       no `finality_conflict` or `[WEDGED]` in any node's log after the heal
#   <s>-testnet-restored            after the restore all 7 nodes share one tip and h1/h300000/tip-10
#   METRIC <S>_REJOIN_SLOTS=<worst> minority= X= B= dead_len= versions=
#   exit: 0 all PASS · 1 any FAIL · 2 SKIP (`SKIP <s>: <reason>`: shape not made, node down,
#   another injection running) — gauntlet-run.sh counts 1 and 2 as not passed.
# INPUT PARTITIONS: pre-fix binary on the minority (stuck: GS-004/007/022/033 FAIL) · fixed binary on
#   the minority (all PASS) · no TN_BINMAP (gs029-mixed-binaries FAIL) · shape not made (SKIP).
#
# Env:
#   GAUNTLET_TN_SESSION  session dir shared by the five runners (unset: a private mktemp session,
#                        so every scenario runs its own injection)
#   TN_BINMAP            "n4=/path/doli-node,n5=/path/doli-node,n2=/path/doli-node": binary each
#                        listed node runs during the injection (copied into the run dir and ad-hoc
#                        codesigned; unlisted nodes keep their installed binary; the installed
#                        binaries and the launchd plists are never modified)
#   GS033_LEN1_MINORITY / GS033_LEN2_MINORITY  minority per shape (default n4,n5)
#   GS033_X              force producer X (default: planner; never the donor, never the minority)
#   GS033_DONOR          full-history donor for the restore (default n1)
#   GS033_OBSERVE        slots observed after the heal (default 45, minimum 35)
#   GS033_RESTORE_WAIT   seconds a node gets to rejoin under launchd before the donor restore (150)
#   GS033_RETRY=1        re-run a cached injection that did not complete (default: reuse its verdict)
#   GS033_ALLOW_SEED=1   allow TN_BINMAP to restart the seed (default refused)
#   GS033_NODES          launchd node names (default "seed n1 n2 n3 n4 n5 n6"; the first is the reference)
#   GS033_X_LEAD (75 s), GS033_FOLLOW_AT (14 blocks), GS033_ATTEMPTS (2), GS033_PLAN_EPOCHS (10),
#   GS033_X_LOWER / GS033_X_UPPER (3*epoch+4 / 4*epoch-4), GS033_INTERNAL_BASE (31300),
#   GS033_FWD_BASE (32000), TESTNET_HOME (~/testnet)
# Wall time: one injection = wait for a reachable boundary (25-60 min) + observe (7.5 min) +
#   restore (2-8 min); a full session (len1 + len2) is about 1.5-2.5 h; cached scenarios take seconds.
#
# SAFETY: LOCAL testnet only (refuses unless getNetworkParams.network == testnet; 127.0.0.1 only).
#   Launchd nodes stop/start only via scripts/testnet.sh with ONE target; never pkill; signals only
#   to PIDs the runner spawned (or the single process on a data dir launchd does not track). The
#   seed is never stopped (unless GS033_ALLOW_SEED=1 and TN_BINMAP names it) and never wiped.
#   Failure modes and their restore: a minority node, X and any TN_BINMAP node run outside launchd
#   on moved P2P ports with their peers.cache set aside; the restore kills those PIDs, puts every
#   peers.cache back and starts each launchd job. A node still off the seed's chain after
#   GS033_RESTORE_WAIT s gets a full-history restore from the stopped donor: its data/* is wiped
#   except signed_slots.db and maintainer_state.bin (refused if a wallet or seed file is inside),
#   blocks/ and state_db/ are rsync'd from the donor, pending_update.json is removed. state.json in
#   the run dir records what to undo; `--restore RUN_DIR` replays it if the runner itself died.
#   Do not run it while another job loads the CPU: slot timing decides the shape.

set -u
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in
  gs004 | gs007 | gs022 | gs029 | gs033 | --plan) ;;
  --restore) [ -n "${2:-}" ] || { echo "usage: $0 --restore RUN_DIR" >&2; exit 2; } ;;
  *)
    sed -n '2,12p' "$0" >&2
    exit 2
    ;;
esac
for tool in python3 launchctl lsof rsync codesign; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "SKIP ${1#--}: $tool not found (macOS local testnet host required)"
    echo "METRIC GS033_PRECONDITION=missing-$tool"
    exit 2
  }
done
exec python3 -I "$ROOT/scripts/gauntlet-gs033/gs033.py" "$@"
