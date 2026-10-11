#!/usr/bin/env bash
# inc-i-237-m0-latency-probe.sh — read the INC-I-237 M0 gossip metrics.
#
# Usage: inc-i-237-m0-latency-probe.sh [--series-count] [--file PATH | URL]
#   default URL: http://127.0.0.1:9000/metrics
# Prints per-topic verdict latency (histogram_quantile linear interpolation),
# block_recv, attestation_coverage, tx_batch, verdict_late (M2 late-verdict
# counter per topic) and a GATE line (p99 <= 2s).
# --series-count prints only the number of verdict-latency series.
# Exit 0 on success, 2 when the source is unreachable or unreadable.
set -u

url="http://127.0.0.1:9000/metrics"
file=""
series_count=0
while [ $# -gt 0 ]; do
    case "$1" in
        --series-count) series_count=1 ;;
        --file)
            [ $# -ge 2 ] || { echo "--file needs a PATH" >&2; exit 2; }
            file="$2"
            shift
            ;;
        -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
        *) url="$1" ;;
    esac
    shift
done

if [ -n "$file" ]; then
    [ -f "$file" ] && [ -r "$file" ] || { echo "cannot read $file" >&2; exit 2; }
    metrics=$(cat "$file") || exit 2
else
    metrics=$(curl -fsS --max-time 10 "$url" 2>/dev/null) || { echo "cannot reach $url" >&2; exit 2; }
fi

if [ "$series_count" -eq 1 ]; then
    printf '%s\n' "$metrics" | grep -c '^doli_gossip_verdict_latency_seconds_count{'
    exit 0
fi

printf '%s\n' "$metrics" | awk '
function labels(s,    body, parts, i, kv, n, k) {
    body = s; sub(/^[^{]*\{/, "", body); sub(/\}.*$/, "", body)
    n = split(body, parts, ",")
    for (k in L) delete L[k]
    for (i = 1; i <= n; i++) {
        split(parts[i], kv, "=")
        gsub(/"/, "", kv[2])
        L[kv[1]] = kv[2]
    }
}
function add_bucket(key, le, c,    i) {
    i = ++NB[key]
    INF[key, i] = (le == "+Inf")
    LE[key, i] = INF[key, i] ? 1e308 : le + 0
    CUM[key, i] = c + 0
}
function sort_buckets(key,    i, j, tl, tc, ti) {
    for (i = 2; i <= NB[key]; i++)
        for (j = i; j > 1 && LE[key, j - 1] > LE[key, j]; j--) {
            tl = LE[key, j]; tc = CUM[key, j]; ti = INF[key, j]
            LE[key, j] = LE[key, j - 1]; CUM[key, j] = CUM[key, j - 1]; INF[key, j] = INF[key, j - 1]
            LE[key, j - 1] = tl; CUM[key, j - 1] = tc; INF[key, j - 1] = ti
        }
}
function quantile(key, q, total,    i, rank, pl, pc) {
    if (total <= 0) return "NaN"
    rank = q * total; pl = 0; pc = 0
    for (i = 1; i <= NB[key]; i++) {
        if (CUM[key, i] >= rank) {
            if (INF[key, i]) return pl
            if (i == 1 && LE[key, i] <= 0) return LE[key, i]
            return pl + (LE[key, i] - pl) * (rank - pc) / (CUM[key, i] - pc)
        }
        pl = LE[key, i]; pc = CUM[key, i]
    }
    return pl
}
function cum_at(key, le,    i) {
    for (i = 1; i <= NB[key]; i++) if (!INF[key, i] && LE[key, i] == le) return CUM[key, i]
    return 0
}
function fmt(v) { return (v == "NaN") ? "NaN" : sprintf("%.6f", v) }
/^doli_gossip_verdict_latency_seconds_bucket\{/ {
    labels($1); add_bucket("v:" L["topic"], L["le"], $2); TOPIC[L["topic"]] = 1; next
}
/^doli_gossip_verdict_latency_seconds_count\{/ { labels($1); VCOUNT[L["topic"]] = $2 + 0; TOPIC[L["topic"]] = 1; next }
/^doli_gossip_block_recv_latency_seconds_bucket\{/ { labels($1); add_bucket("br", L["le"], $2); next }
/^doli_gossip_block_recv_latency_seconds_count / { BRC = $2 + 0; next }
/^doli_attestation_coverage_ratio_bucket\{/ { labels($1); add_bucket("ac", L["le"], $2); next }
/^doli_attestation_coverage_ratio_sum / { ACS = $2 + 0; next }
/^doli_attestation_coverage_ratio_count / { ACC = $2 + 0; next }
/^doli_gossip_tx_batch_messages_total\{/ { labels($1); TB[L["size"]] = $2 + 0; TBM += $2; next }
/^doli_gossip_tx_batch_txs_total / { TBT = $2 + 0; next }
/^doli_gossip_verdict_late_total\{/ { labels($1); LT[L["topic"]] = $2 + 0; LTS += $2; next }
END {
    nt = 0
    for (t in TOPIC) T[++nt] = t
    for (i = 2; i <= nt; i++) for (j = i; j > 1 && T[j - 1] > T[j]; j--) { x = T[j]; T[j] = T[j - 1]; T[j - 1] = x }
    max = -1
    for (i = 1; i <= nt; i++) {
        t = T[i]; key = "v:" t; sort_buckets(key); c = VCOUNT[t] + 0
        p50 = quantile(key, 0.5, c); p99 = quantile(key, 0.99, c)
        le2 = (c > 0) ? cum_at(key, 2) / c : "NaN"
        printf "topic=%s count=%.0f p50=%s p99=%s le2s=%s\n", t, c, fmt(p50), fmt(p99), fmt(le2)
        if (c > 0 && p99 > max) max = p99
    }
    sort_buckets("br")
    printf "block_recv count=%.0f p50=%s p99=%s\n", BRC, fmt(quantile("br", 0.5, BRC)), fmt(quantile("br", 0.99, BRC))
    sort_buckets("ac")
    printf "attestation_coverage count=%.0f mean=%s p50=%s\n", ACC, fmt(ACC > 0 ? ACS / ACC : "NaN"), fmt(quantile("ac", 0.5, ACC))
    line = sprintf("tx_batch msgs=%.0f txs=%.0f", TBM, TBT)
    ns = split("1 2_5 6_10 11_25 26_50 51_100 101_plus", S, " ")
    for (i = 1; i <= ns; i++) line = line sprintf(" %s=%.0f", S[i], TB[S[i]] + 0)
    print line
    nl = 0
    for (t in LT) K[++nl] = t
    for (i = 2; i <= nl; i++) for (j = i; j > 1 && K[j - 1] > K[j]; j--) { x = K[j]; K[j] = K[j - 1]; K[j - 1] = x }
    line = sprintf("verdict_late total=%.0f", LTS)
    for (i = 1; i <= nl; i++) line = line sprintf(" %s=%.0f", K[i], LT[K[i]])
    print line
    if (max < 0) print "GATE verdict_p99_max=NaN NO_DATA (<= 2s)"
    else printf "GATE verdict_p99_max=%s %s (<= 2s)\n", fmt(max), (max <= 2 + 1e-9) ? "PASS" : "FAIL"
}'
