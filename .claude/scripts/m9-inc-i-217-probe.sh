#!/usr/bin/env bash
# INC-I-217 M9 outcome probe — can an OPERATOR and the EXPLORER see a BLS
# rotation at all? Every number here is a fact observable from OUTSIDE the node
# process: JSON bytes an explorer parses, Prometheus exposition an operator
# scrapes, a log line an operator greps.
#
# ROT_RPC_NEW_KEY        pending-update entries served by the getProducer /
#                        getProducers mapper that carry the queued 48-byte BLS
#                        key and the height the queue flushes at, so a rotation
#                        is visible BEFORE it takes effect (of 1).
# ROT_RPC_SHAPE_FROZEN   pre-existing pending-update kinds whose JSON bytes are
#                        unchanged by the two additive fields — the explorer on
#                        ai2 reads this object today (of 1).
# ROT_METRIC_MOVES       scrape-visible doli_producer_bls_rotation_total series
#                        that actually MOVE when a real node flushes a rotation
#                        at an epoch boundary (of 1). INC-I-187: 28/57 doli_*
#                        metrics were registered and never written; registration
#                        alone does not count here.
# ROT_LOG_APPLIED_OLD    [BLS_ROTATE] applied lines that carry BOTH the old and
#                        the new key prefix, so a grep shows which key was
#                        replaced (of 1).
#
# Every number comes from a token the test prints ON the assertion that proves
# the fact, so a harness that does not build, does not run, or does not reach
# the assertion reports 0 — never a stale 1.
cd "$(git rev-parse --show-toplevel)" || exit 1
rout=$(cargo nextest run -p rpc --lib --no-capture inc_i217_m9 2>&1)
nout=$(cargo nextest run -p doli-node --test bls_rotation_metrics --no-capture 2>&1)
sout=$(cargo nextest run -p storage --lib --no-capture rotation 2>&1)
rpckey=$(printf '%s\n' "$rout" | grep -c 'ROT-RPC-NEW-KEY')
frozen=$(printf '%s\n' "$rout" | grep -c 'ROT-RPC-SHAPE-FROZEN')
metric=$(printf '%s\n' "$nout" | grep -c 'ROT-METRIC-MOVES')
logold=$(printf '%s\n%s\n' "$nout" "$sout" | grep -c 'ROT-LOG-APPLIED-OLD')
[ "$rpckey" -gt 1 ] && rpckey=1
[ "$frozen" -gt 1 ] && frozen=1
[ "$metric" -gt 1 ] && metric=1
[ "$logold" -gt 1 ] && logold=1
echo "ROT_RPC_NEW_KEY=${rpckey}/1 ROT_RPC_SHAPE_FROZEN=${frozen}/1 ROT_METRIC_MOVES=${metric}/1 ROT_LOG_APPLIED_OLD=${logold}/1"
