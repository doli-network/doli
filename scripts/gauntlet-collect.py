#!/usr/bin/env python3
"""gauntlet-collect.py — metrics collector for the OMEGA gauntlet runner.

Reads a node-config JSON (produced by scripts/gauntlet.sh), queries each live
node's RPC, scans the NEW bytes each node's log grew by during the observation
window, and emits a single metrics JSON blob on stdout. The bash runner reads
that blob and evaluates each scenario's assertions against it.

Every metric is a real signal:
  * RPC:  getChainInfo, getStateRootDebug, getBlockByHeight(1), getGuardianStatus
  * Logs: windowed counts of panic / empty-headers / rejected-epoch / snap-trigger
          / rollback-event / dedup / orphan-request / eviction / busy markers, and
          the LATEST structured telemetry line ([SYNC_STATE], [HEALTH], [GOSSIP_MESH]).

Assertions never key off raw keywords that also appear in telemetry (e.g. the word
"rollback" appears every second as `rollback_depth=0`); they key off the structured
field value or off distinct *event* phrases.

Usage: gauntlet-collect.py <node_config.json>
  node_config.json = {"nodes":[{"name","port","pid","logfile","offset","baseline_height","rss_mb"}]}
"""
import json
import re
import sys
import urllib.request

ANSI = re.compile(r"\x1b\[[0-9;]*m")

# Event patterns — deliberately match ACTIONS/events, not telemetry field names.
PATTERNS = {
    "win_panic":            re.compile(r"panicked at|thread '[^']*' panicked|SIGSEGV|SIGABRT|\bFATAL\b"),
    "win_empty_headers":    re.compile(r"Empty headers|returned 0 headers|\b0 headers\b"),
    "win_rejected":         re.compile(r"rejected block|invalid block|invalid epoch|missing EpochReward|block rejected|InvalidEpoch"),
    "win_snap_trigger":     re.compile(r"Starting snap sync|initiat\w* snap|Snapshot download|snap sync (?:start|begin|trigger)", re.I),
    "win_rollback_events":  re.compile(r"Rolling back|Rolled back|execute_reorg|ShallowRollback"),
    "win_already_published": re.compile(r"already been published|already published"),
    "win_orphan_reqs":      re.compile(r"Requesting parent|request(?:ing)? parent block|ChaseParent|orphan chase", re.I),
    "win_evictions":        re.compile(r"Evicting peer|Disconnecting peer|peer evicted", re.I),
    "win_busy":             re.compile(r"rate limit|peer busy|marked busy|GOSSIP_SHED|rate.?limited|dropping canonical", re.I),
    "win_integrity_gap":    re.compile(r"integrity gap|integrity chain (?:loss|broken)|missing block \d|IntegrityGap", re.I),
    "win_gossip_total":     re.compile(r"\[GOSSIP_MESH\]"),
}
LAST_SYNC = re.compile(r'\[SYNC_STATE\][^\n]*?gap=(\d+)[^\n]*?phase="(\w+)"[^\n]*?rollback_depth=(\d+)')
LAST_HEALTH = re.compile(r'\[HEALTH\][^\n]*?peers=(\d+)[^\n]*?sync_fails=(\d+)[^\n]*?state="(\w+)"')
LAST_MESH = re.compile(r'\[GOSSIP_MESH\][^\n]*?gossip_peers=(\d+)')


def rpc(port, method, params=None):
    body = json.dumps({"jsonrpc": "2.0", "method": method,
                       "params": params or {}, "id": 1}).encode()
    req = urllib.request.Request("http://127.0.0.1:%s" % port, data=body,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=4) as r:
        return json.loads(r.read()).get("result")


def scan_window(logfile, offset):
    """Count event patterns and extract latest telemetry in the bytes added since `offset`."""
    out = {k: 0 for k in PATTERNS}
    out.update({"gap": None, "phase": None, "rollback_depth": None,
                "peers": None, "sync_fails": None, "health_state": None, "gossip_peers": None})
    try:
        with open(logfile, "rb") as f:
            f.seek(offset)
            chunk = f.read()
    except OSError:
        return out
    text = ANSI.sub("", chunk.decode("utf-8", "replace"))
    for k, pat in PATTERNS.items():
        out[k] = len(pat.findall(text))
    m = None
    for m in LAST_SYNC.finditer(text):
        pass
    if m:
        out["gap"], out["phase"], out["rollback_depth"] = int(m.group(1)), m.group(2), int(m.group(3))
    m = None
    for m in LAST_HEALTH.finditer(text):
        pass
    if m:
        out["peers"], out["sync_fails"], out["health_state"] = int(m.group(1)), int(m.group(2)), m.group(3)
    m = None
    for m in LAST_MESH.finditer(text):
        pass
    if m:
        out["gossip_peers"] = int(m.group(1))
    return out


def collect_node(n):
    d = {"name": n["name"], "port": n["port"], "rss_mb": n.get("rss_mb"),
         "up": False}
    try:
        ci = rpc(n["port"], "getChainInfo") or {}
        d["up"] = True
        d["height"] = ci.get("bestHeight")
        d["bestHash"] = ci.get("bestHash")
        d["genesisHash"] = ci.get("genesisHash")
        d["rewardPool"] = ci.get("rewardPoolBalance")
    except Exception as e:
        d["error"] = str(e)
        return d
    try:
        sr = rpc(n["port"], "getStateRootDebug") or {}
        d["sr_height"] = sr.get("height")
        d["stateRoot"] = sr.get("stateRoot")
        d["csHash"] = sr.get("csHash")
        d["psHash"] = sr.get("psHash")
        d["utxoHash"] = sr.get("utxoHash")
        d["utxoCount"] = sr.get("utxoCount")
        d["producerCount"] = sr.get("producerCount")
    except Exception:
        pass
    try:
        b1 = rpc(n["port"], "getBlockByHeight", {"height": 1}) or {}
        d["block1Hash"] = b1.get("hash")
    except Exception:
        d["block1Hash"] = None
    try:
        g = rpc(n["port"], "getGuardianStatus") or {}
        d["recovery_mode"] = bool(g.get("recovery_mode"))
        d["production_paused"] = bool(g.get("production_paused"))
        d["checkpoint"] = g.get("last_healthy_checkpoint")
    except Exception:
        d["recovery_mode"] = None
        d["production_paused"] = None
    d.update(scan_window(n["logfile"], int(n["offset"])))
    d["baseline_height"] = n.get("baseline_height")
    d["liveness_delta"] = (d["height"] - n["baseline_height"]) if (d.get("height") is not None and n.get("baseline_height") is not None) else None
    return d


def block_hash_at(port, height):
    try:
        b = rpc(port, "getBlockByHeight", {"height": height}) or {}
        return b.get("hash")
    except Exception:
        return None


def main():
    cfg = json.load(open(sys.argv[1]))
    nodes = [collect_node(n) for n in cfg["nodes"]]
    up = [n for n in nodes if n.get("up")]

    heights = [n["height"] for n in up if n.get("height") is not None]
    min_h = min(heights) if heights else None
    max_h = max(heights) if heights else None

    # Fork check at the common (min) height: fetch block hash at min_h on every
    # up node — agreement here means one canonical chain regardless of tip lag.
    common_hashes = set()
    if min_h is not None:
        for n in up:
            h = block_hash_at(n["port"], min_h)
            common_hashes.add(h)

    # State-root agreement: compare only among nodes AT the modal height (state
    # root RPC is tip-only; comparing across different heights is meaningless).
    from collections import Counter
    hcount = Counter(n["height"] for n in up if n.get("height") is not None)
    modal_h = hcount.most_common(1)[0][0] if hcount else None
    modal_nodes = [n for n in up if n.get("height") == modal_h]

    def distinct(nodes_, key):
        return len(set(n.get(key) for n in nodes_ if n.get(key) is not None))

    net = {
        "up_count": len(up),
        "total_count": len(nodes),
        "min_height": min_h,
        "max_height": max_h,
        "liveness_delta": (max_h - min(n["baseline_height"] for n in up if n.get("baseline_height") is not None)) if up and any(n.get("baseline_height") is not None for n in up) else None,
        "distinct_common_hash": len(common_hashes),
        "distinct_genesis": distinct(up, "genesisHash"),
        "distinct_block1": distinct(up, "block1Hash"),
        "modal_height": modal_h,
        "modal_node_count": len(modal_nodes),
        "distinct_stateroot_modal": distinct(modal_nodes, "stateRoot"),
        "distinct_cshash_modal": distinct(modal_nodes, "csHash"),
        "distinct_pshash_modal": distinct(modal_nodes, "psHash"),
        "distinct_utxocount_modal": distinct(modal_nodes, "utxoCount"),
        "recovery_mode_any": any(n.get("recovery_mode") for n in up),
        "production_paused_any": any(n.get("production_paused") for n in up),
        # windowed aggregates
        "win_panic": sum(n.get("win_panic", 0) for n in nodes),
        "win_empty_headers": sum(n.get("win_empty_headers", 0) for n in nodes),
        "win_rejected": sum(n.get("win_rejected", 0) for n in nodes),
        "win_snap_trigger": sum(n.get("win_snap_trigger", 0) for n in nodes),
        "win_rollback_events": sum(n.get("win_rollback_events", 0) for n in nodes),
        "win_already_published": sum(n.get("win_already_published", 0) for n in nodes),
        "win_orphan_reqs": sum(n.get("win_orphan_reqs", 0) for n in nodes),
        "win_evictions": sum(n.get("win_evictions", 0) for n in nodes),
        "win_busy": sum(n.get("win_busy", 0) for n in nodes),
        "win_integrity_gap": sum(n.get("win_integrity_gap", 0) for n in nodes),
        "win_gossip_total": sum(n.get("win_gossip_total", 0) for n in nodes),
        "max_rss_mb": max((n.get("rss_mb") or 0) for n in nodes) if nodes else 0,
        "max_gap": max((n.get("gap") or 0) for n in up) if up else 0,
        "max_sync_fails": max((n.get("sync_fails") or 0) for n in up) if up else 0,
        "max_rollback_depth": max((n.get("rollback_depth") or 0) for n in up) if up else 0,
    }
    print(json.dumps({"nodes": nodes, "net": net}))


if __name__ == "__main__":
    main()
