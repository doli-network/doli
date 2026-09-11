#!/usr/bin/env bash
# INC-I-217 M7 outcome probe — does a validated rotation change on-chain state?
#
# ROT_KEY_CHANGES_AT_BOUNDARY  producers whose ProducerInfo.bls_pubkey was
#                              observed to change at the epoch-boundary flush
#                              after a RotateBlsKey was queued (of 1 rotated
#                              producer).
# ROT_UNDO_CARRIES_ROTATION    blocks whose only producer-affecting tx is a
#                              rotation that were observed to take the
#                              non-sentinel undo-snapshot branch (of 1).
#
# ROT_UNDO_CARRIES_ROTATION is read at apply_block/helpers.rs, where
# `needs_producer_snapshot` is actually decided (apply_block/mod.rs:173-183),
# not end-to-end: validate_block_for_apply (mod.rs:110) rejects every
# rotation-carrying block below bls_key_rotation_activation_height (= u64::MAX
# on all three networks, and not env-overridable), so no undo entry for such a
# block can exist until the pinning session. M8/post-pin owes the end-to-end
# form of this criterion.
#
# Both numbers come from tokens the tests print on the assertion that proves
# the fact, so a harness that does not build, does not run, or does not reach
# the assertion reports 0 — never a stale 1.
cd "$(git rev-parse --show-toplevel)" || exit 1
sout=$(cargo nextest run -p storage --lib --no-capture rotation 2>&1)
nout=$(cargo nextest run -p doli-node --lib --no-capture rotation_tests 2>&1)
changed=$(printf '%s\n%s\n' "$sout" "$nout" | grep -c 'ROT-KEY-CHANGED-AT-BOUNDARY')
undo=$(printf '%s\n' "$nout" | grep -c 'ROT-UNDO-SNAPSHOT-NON-EMPTY')
[ "$changed" -gt 1 ] && changed=1
[ "$undo" -gt 1 ] && undo=1
echo "ROT_KEY_CHANGES_AT_BOUNDARY=${changed}/1 ROT_UNDO_CARRIES_ROTATION=${undo}/1"
