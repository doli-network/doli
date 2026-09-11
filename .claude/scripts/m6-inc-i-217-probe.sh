#!/usr/bin/env bash
# INC-I-217 M6 outcome probe — rotation relay/consensus verdict agreement.
#
# ROT_CLASSES_AGREEING   rotation-tx classes (of a fixed 10-class corpus) where
#                        the relay verdict equals the consensus verdict.
# ROT_RELAY_SURFACES_WIRED  relay surfaces (mempool admission, block builder)
#                        observed calling the shared predicate by name.
# On a tree with no rotation predicate in `crates/mempool` the harness cannot
# build and both numbers are 0.
cd "$(git rev-parse --show-toplevel)" || exit 1
out=$(cargo nextest run -p mempool --test rotation_parity --no-capture 2>&1)
agree=$(printf '%s\n' "$out" | grep -c 'ROT-CLASS-AGREE ')
wired=$(printf '%s\n' "$out" | grep -cE 'PASS .*(req_rot_009_the_block_builder_calls_the_named_predicate|req_rot_009_the_relay_calls_the_named_predicate_before_the_generic_validator)')
echo "ROT_CLASSES_AGREEING=${agree}/10 ROT_RELAY_SURFACES_WIRED=${wired}/2"
