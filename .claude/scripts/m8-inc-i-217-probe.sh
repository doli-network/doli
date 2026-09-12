#!/usr/bin/env bash
# INC-I-217 M8 outcome probe — does a rotation SURVIVE the three state-recovery
# paths a node actually takes, and is the activation gate reachable at all?
#
# ROT_REBUILD_EQ_LIVE   nodes whose ProducerSet, rebuilt from the SAME blocks by
#                       `rebuild_producer_set_from_blocks` (rewards.rs), is
#                       byte-identical (serialize_canonical + pending_updates) to
#                       the node that applied those blocks live (of 1).
# ROT_REORG_OLD_KEY     nodes that, after reorging onto a sibling chain WITHOUT
#                       the rotation block, serve the OLD bls_pubkey and a state
#                       root equal to a node that never saw the rotation (of 1).
# ROT_ENV_AH_HONORED    networks on which a real node honours
#                       DOLI_BLS_KEY_ROTATION_ACTIVATION_HEIGHT so a rotation can
#                       be applied below u64::MAX at all (devnet; mainnet must
#                       still ignore it) (of 1).
#
# Every number comes from a token the test prints ON the assertion that proves
# the fact, so a harness that does not build, does not run, or does not reach the
# assertion reports 0 — never a stale 1.
cd "$(git rev-parse --show-toplevel)" || exit 1
# The tests are MODULES of the one `it` binary per crate (.claude/hooks/test-binary-gate.sh
# forbids a new top-level tests/*.rs target), so the target is `it` and the module name is
# the filter. `--test bls_rotation_convergence` resolves to no target and would report 0
# forever, including after the fix.
nout=$(cargo nextest run -p doli-node --test it --no-capture bls_rotation_convergence 2>&1)
cout=$(cargo nextest run -p doli-core --lib --no-capture network_params 2>&1)
rebuild=$(printf '%s\n' "$nout" | grep -c 'ROT-REBUILD-EQ-LIVE')
reorg=$(printf '%s\n' "$nout" | grep -c 'ROT-REORG-OLD-KEY')
envah=$(printf '%s\n%s\n' "$nout" "$cout" | grep -c 'ROT-ENV-AH-HONORED')
[ "$rebuild" -gt 1 ] && rebuild=1
[ "$reorg" -gt 1 ] && reorg=1
[ "$envah" -gt 1 ] && envah=1
echo "ROT_REBUILD_EQ_LIVE=${rebuild}/1 ROT_REORG_OLD_KEY=${reorg}/1 ROT_ENV_AH_HONORED=${envah}/1"
