#!/usr/bin/env python3
"""probes.py — read-only measurements for the GS-033 runner (INC-I-281): isolation check, tip
sampler, log windows, fleet tables, canonical block list. Nothing here signals or writes a node.
"""
import calendar
import json
import os
import re
import subprocess
import time

import tn

ACTIVE_LOG = {}  # node index -> log file the node writes now (launchd log or a run-dir log)
TS = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?Z")


def active_log(i):
    return ACTIVE_LOG.get(i, tn.NODES[i]["log"])


def size(path):
    try:
        return os.path.getsize(path)
    except OSError:
        return 0


def line_time(line):
    """Epoch seconds of a node log line ('2026-10-28T03:30:55.188047Z  INFO ...'), or None."""
    m = TS.match(tn.ANSI.sub("", line))
    if not m:
        return None
    return calendar.timegm(time.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S")) + float(m.group(2) or 0)


def read_lines(path, start=0, end=None):
    """Lines of path between byte offsets [start, end) (whole lines, ANSI stripped)."""
    out = []
    try:
        with open(path, "rb") as f:
            f.seek(start)
            data = f.read((end - start) if end is not None else -1)
    except OSError:
        return out
    for raw in data.decode("utf-8", "replace").splitlines():
        out.append(tn.ANSI.sub("", raw))
    return out


def grep_from(path, offset, pattern, maxl=5):
    """First maxl lines containing pattern after byte offset (tail + grep; logs can be GBs)."""
    q = "'" + pattern.replace("'", "'\\''") + "'"
    pq = "'" + path.replace("'", "'\\''") + "'"
    r = subprocess.run("tail -c +%d %s | grep -a -F -- %s | head -n %d" % (offset + 1, pq, q, maxl),
                       shell=True, capture_output=True, text=True)
    return [tn.clean(x) for x in r.stdout.splitlines()]


def slice_to(path, start, end, dst):
    with open(path, "rb") as f, open(dst, "wb") as g:
        f.seek(start)
        left = max(0, end - start)
        while left > 0:
            chunk = f.read(min(left, 1 << 20))
            if not chunk:
                break
            g.write(chunk)
            left -= len(chunk)


# ---------------------------------------------------------------------------------- isolation
def lsof_check(label, M, tolerate=()):
    """Every ESTABLISHED P2P socket of a minority node must lead to its forwarder or to another
    minority node; the owner of a remote endpoint = the tracked process holding it locally."""
    pids = {p.pid: tag for tag, p in tn.ST["procs"].items() if p.poll() is None}
    for n in tn.NODES:
        for pid in tn.datadir_pids(n["i"]):
            pids.setdefault(pid, n["name"])
    out = subprocess.run(["lsof", "-nP", "-iTCP", "-sTCP:ESTABLISHED", "-a", "-p", ",".join(map(str, pids)), "-Fpn"],
                         capture_output=True, text=True).stdout
    socks, cur = [], None
    for ln in out.splitlines():
        if ln.startswith("p"):
            cur = int(ln[1:])
        elif ln.startswith("n") and "->" in ln:
            loc, rem = ln[1:].split("->")
            socks.append((cur, loc, rem))
    owner = {loc: pids.get(pid, str(pid)) for pid, loc, _ in socks}
    mtags = {"m%d" % m for m in M}
    svc = {n["rpc"] for n in tn.NODES} | {n["met"] for n in tn.NODES}
    bad, cnt = [], 0
    for pid, loc, rem in socks:
        tag = pids.get(pid)
        if tag not in mtags:
            continue
        if int(loc.rsplit(":", 1)[1]) in svc or int(rem.rsplit(":", 1)[1]) in svc:
            continue
        cnt += 1
        o = owner.get(rem, "untracked")
        if not (o in {"fwd%d" % mm for mm in M} or o in mtags or o in tolerate):
            bad.append("%s %s->%s owner=%s" % (tag, loc, rem, o))
    with open(os.path.join(tn.OUT, "lsof-%s.txt" % label), "w") as f:
        f.write("\n".join("%s %s->%s" % (pids.get(p), lo, r) for p, lo, r in socks) + "\n")
    pair = sum(1 for pid, loc, rem in socks if pids.get(pid) in mtags and owner.get(rem) in mtags)
    tn.say("isolation[%s]: minority P2P sockets=%d direct-to-majority=%d minority-pair sockets=%d %s" % (
        label, cnt, len(bad), pair, "; ".join(bad)[:300]))
    if len(M) > 1 and pair == 0:
        bad.append("minority nodes not linked to each other")
    return bad


# ---------------------------------------------------------------------------------- chain
def on_ref(i, ti, ref_tip, ref=0):
    """True iff node i's chain equals the reference node's at min(tip_i, tip_ref)."""
    hc = min(ti[0], ref_tip[0])
    a = ti[1] if hc == ti[0] else tn.bhash(i, hc)
    b = ref_tip[1] if hc == ref_tip[0] else tn.bhash(ref, hc)
    if a is None or b is None:
        return None
    return a == b


def sampler():
    with open(os.path.join(tn.OUT, "samples.jsonl"), "a") as f:
        while not tn.ST["stop"]:
            row = {"t": round(time.time(), 2), "part": tn.ST["partitioned"], "tips": {}, "canon": {}}
            ref = 0
            rt = tn.tip(0)
            if rt is None:
                ref, rt = 1, tn.tip(1)
            row["ref"] = tn.name(ref)
            for n in tn.NODES:
                t = rt if n["i"] == ref else tn.tip(n["i"])
                if t:
                    row["tips"][n["name"]] = [t[0], t[1][:16]]
                    row["canon"][n["name"]] = on_ref(n["i"], t, rt, ref) if rt else None
            f.write(json.dumps(row) + "\n")
            f.flush()
            time.sleep(2)


def fleet_table(heights=(1, 300000), tries=5):
    """7-node table: tip + hashes at fixed heights and the reference tip-10; retried until every
    tip hash is equal (tips move every slot) or `tries` is exhausted. Every attempt is kept."""
    attempts = []
    for _ in range(tries):
        rt = tn.tip(0)
        hs = list(heights) + ([rt[0] - 10] if rt else [])
        rows = {}
        for n in tn.NODES:
            t = tn.tip(n["i"])
            rows[n["name"]] = {"tip": [t[0], t[1]] if t else None,
                               "at": {str(h): tn.bhash(n["i"], h) for h in hs}}
        tips = {json.dumps(r["tip"]) for r in rows.values()}
        attempts.append({"t": round(time.time(), 2), "heights": hs, "rows": rows, "one_tip": len(tips) == 1})
        if len(tips) == 1:
            break
        time.sleep(3)
    return attempts


def canonical_blocks(lo, hi, ref=0):
    out = {}
    for h in range(max(1, lo), hi + 1):
        b = tn.block(ref, h)
        if b:
            out[str(h)] = [b["hash"], int(b["slot"]), b.get("producer") or ""]
    return out


def integrity(i, lo, hi):
    try:
        return tn.rpc(i, "verifyChainIntegrity", {"from_height": lo, "up_to_height": hi}, timeout=90)
    except Exception as e:
        return {"error": repr(e)}
