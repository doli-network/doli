#!/usr/bin/env python3
"""judge.py — GS-004/007/022/029/033 assertions over one recorded injection (INC-I-281).

Reads only the run dir (result.json, samples.jsonl, restore.json, the minority node logs and the
post-heal log slice of every node); it never queries a node. evaluate() returns the report lines
(`PASS|FAIL|SKIP <id> <k=v>`, METRIC last) and the exit code (0 all PASS, 1 a FAIL, 2 SKIP).

Measures (per minority node, t = seconds after the heal, one slot = result.slot):
  canonical standing  a 2-s sample whose chain equals the seed's at min(tip, seed tip), or a log
                      line that applies / rolls back to a block in the seed's chain at h >= B
  rejoin              first sample from which no later sample is off the seed's chain
  return              after the first canonical standing: a sample off the seed's chain or an
                      `[BLOCK] Applied` of a block that is not in the seed's chain (a dead block)
  rollback→dead       a `ROLLBACK_DONE` whose next `[BLOCK] Applied` (h >= B) is a dead block;
                      "stuck" when no canonical standing follows within 30 slots
  reorg               `Executing reorg` / `Reorg complete` between the heal and rejoin + 2 slots
  fork-walk path      `[FORK] RECOVERY_START`, `RECOVERY_DONE ... reorged` or a `[WEDGE_ESCAPE]`
                      line in the same window
  snap sync           `[SNAP_SYNC] Applying snapshot`, `Snapshot applied successfully` or
                      `[SNAP_SYNC] Consuming snapshot state` after the heal, or block 1 != seed's
  missing heights     verifyChainIntegrity(B-100 .. final tip) not complete or missingCount > 0
"""
import json
import os
import re

import probes

HEX = re.compile(r"\b[0-9a-f]{8,64}\b")
APPLIED = re.compile(r"\[BLOCK\] Applied h=(\d+) hash=([0-9a-f]+)")
ROLLBACK = re.compile(r"ROLLBACK_DONE h=(\d+) hash=([0-9a-f]+)")
BADPROD = re.compile(r"producer=([0-9a-f]+), slot=(\d+)")
FROZEN = re.compile(r"epoch \d+: (\d+) producers")
REORG = ("Executing reorg", "Reorg complete")
WALK = ("[FORK] RECOVERY_START", "[WEDGE_ESCAPE]")
SNAP = ("[SNAP_SYNC] Applying snapshot", "Snapshot applied successfully", "[SNAP_SYNC] Consuming snapshot state")
INELIG = ("is ineligible", "Dropping ineligible", "invalid producer for slot")
WEDGE = ("finality_conflict", "[WEDGED]")


class Run:
    def __init__(self, d):
        self.d = d
        with open(os.path.join(d, "result.json")) as f:
            self.R = json.load(f)
        self.rest = {}
        if os.path.exists(os.path.join(d, "restore.json")):
            with open(os.path.join(d, "restore.json")) as f:
                self.rest = json.load(f)
        R = self.R
        self.end = R.get("end_at") or float("inf")  # the restore restarts nodes after this
        self.samples = []
        if os.path.exists(os.path.join(d, "samples.jsonl")):
            with open(os.path.join(d, "samples.jsonl")) as f:
                self.samples = [r for r in (json.loads(x) for x in f if x.strip()) if r["t"] <= self.end]
        self.slot = R.get("slot", 10)
        self.B = R.get("B")
        self.heal = R.get("heal_at")
        self.M = R.get("minority", [])
        self.canon = {int(h): v for h, v in R.get("canonical", {}).items()}
        self.cmax = max(self.canon) if self.canon else 0

    def is_canon(self, h, prefix):
        """True / False against the seed's chain at h; None when h is beyond the recorded tip."""
        if h > self.cmax:
            return None
        if h < (min(self.canon) if self.canon else 0):
            return True
        return bool(self.canon.get(h)) and self.canon[h][0].startswith(prefix)

    def any_canon_hash(self, token):
        return any(v[0].startswith(token) for v in self.canon.values())

    def events(self, nm):
        """Timestamped lines of the minority node's own log between the heal and the end."""
        out, t = [], None
        for ln in probes.read_lines(os.path.join(self.d, "%s.node.log" % nm)):
            lt = probes.line_time(ln)
            t = lt if lt is not None else t
            if t is not None and self.heal <= t <= self.end:
                out.append((t, ln))
        return out


def shape(run):
    """The injection made the intended shape: completed, isolated, dead branch of the planned
    length at heal, canonical branch heavier, producer list diverged at B."""
    R = run.R
    if R.get("error"):
        return False, "injection_error=%s" % R["error"][:160]
    iso = {k: len(v) for k, v in (R.get("isolation_bad") or {}).items()}
    pre = R["pre_heal"]
    seed_nm = R.get("ref") or next(iter(pre))
    seed = pre[seed_nm]
    B, want = run.B, R["dead_len"]
    cb = (seed.get("h%d" % B) or "").split("/")[0]
    dead = {}
    for m in run.M:
        row = pre[m]
        mb = (row.get("h%d" % B) or "").split("/")[0]
        dead[m] = (row["tip"][0] - (B - 1)) if (row.get("tip") and mb and mb != cb) else 0
    canon_len = seed["tip"][0] - (B - 1) if seed.get("tip") else 0
    lists = {}
    for nm, lines in (R.get("list_proof") or {}).items():
        m = FROZEN.search(lines[0]) if lines else None
        lists[nm] = int(m.group(1)) if m else None
    nd = {lists.get(m) for m in run.M}
    nc = lists.get(seed_nm)
    diverged = len(nd) == 1 and None not in nd and nc is not None and nc != next(iter(nd))
    ok = (not any(iso.values()) and all(v == want for v in dead.values()) and canon_len > want and diverged)
    kv = "B=%s X=%s dead=%s canonical_at_heal=%d lists_dead/canonical=%s/%s isolation_leaks=%s" % (
        B, R.get("X"), ",".join("%s:%d" % kv_ for kv_ in dead.items()), canon_len,
        "|".join(str(x) for x in sorted(nd, key=str)), nc, sum(iso.values()))
    return ok, kv


def rejoin(run, nm):
    post = [s for s in run.samples if s["t"] >= run.heal and nm in s.get("canon", {})]
    first = next((s["t"] for s in post if s["canon"][nm] is True), None)
    rj = None
    for k, s in enumerate(post):
        if s["canon"][nm] is True and all(x["canon"].get(nm) is not False for x in post[k:]):
            rj = s["t"]
            break
    return first, rj


def node_facts(run, nm):
    ev = run.events(nm)
    first, rj = rejoin(run, nm)
    stand = [first] if first else []
    applied, rollbacks = [], []
    for t, ln in ev:
        a = APPLIED.search(ln)
        if a:
            h = int(a.group(1))
            applied.append((t, h, run.is_canon(h, a.group(2))))
            if h >= run.B and applied[-1][2] is True:
                stand.append(t)
        r = ROLLBACK.search(ln)
        if r:
            h = int(r.group(1))
            rollbacks.append((t, h, run.is_canon(h, r.group(2))))
            if rollbacks[-1][2] is True:
                stand.append(t)
    applied.sort()
    rollbacks.sort()
    m0 = min(stand) if stand else None
    back_samples = sum(1 for s in run.samples if m0 and s["t"] > m0 and s.get("canon", {}).get(nm) is False)
    back_applies = sum(1 for t, h, c in applied if m0 and t > m0 and h >= run.B and c is False)
    end = (rj or run.heal) + 2 * run.slot
    win = [ln for t, ln in ev if t <= end]
    rb = []
    for t_rb, h, c in rollbacks:
        nxt = next((x for x in applied if x[0] > t_rb), None)
        later = [x for x in stand if x >= t_rb]
        dead_next = bool(nxt and nxt[2] is False and nxt[1] >= run.B)
        rejoin_after = min(later) if later else None
        stuck = dead_next and (rejoin_after is None or rejoin_after - t_rb > 30 * run.slot)
        rb.append({"t": round(t_rb - run.heal, 1), "h": h, "dead_next": dead_next, "stuck": stuck})
    inelig = 0
    for t, ln in ev:
        if not any(p in ln for p in INELIG):
            continue
        if any(run.any_canon_hash(tok) for tok in HEX.findall(ln)):
            inelig += 1
            continue
        bp = BADPROD.search(ln)
        if bp and any(v[1] == int(bp.group(2)) and v[2].startswith(bp.group(1)) for v in run.canon.values()):
            inelig += 1
    pm = run.R.get("per_minority", {}).get(nm, {})
    integ = pm.get("integrity") or {}
    return {
        "rejoin_slots": round((rj - run.heal) / run.slot, 1) if rj else None,
        "first_canon_slots": round((m0 - run.heal) / run.slot, 1) if m0 else None,
        "returns": back_samples + back_applies,
        "reorg": sum(1 for ln in win if any(p in ln for p in REORG)),
        "walk": sum(1 for ln in win if any(p in ln for p in WALK) or ("RECOVERY_DONE" in ln and "reorged" in ln)),
        "snap": sum(1 for t, ln in ev if any(p in ln for p in SNAP)),
        "block1_ok": bool(pm.get("block1")) and pm.get("block1") == run.R.get("seed_block1"),
        "missing": integ.get("missingCount") if isinstance(integ, dict) else None,
        "complete": bool(integ.get("complete")) if isinstance(integ, dict) else False,
        "rollbacks": rb,
        "inelig_canonical": inelig,
    }


def per_node(run, facts, key, fmt=str):
    return ",".join("%s:%s" % (nm, fmt(f[key])) for nm, f in facts.items())


def evaluate(scen, d):
    """scen in gs004 gs007 gs022 gs029 gs033 -> (lines, exit code)."""
    run = Run(d)
    sid = scen
    lines = ["NOTE %s run_dir=%s" % (sid, d)]
    ok, kv = shape(run)
    restored = bool(run.rest.get("healthy"))
    rest_kv = "healthy=%s restored=%s errors=%d" % (restored, ",".join(run.rest.get("restored", [])) or "none",
                                                    len(run.rest.get("errors", [])))
    lines.append("%s %s-shape %s" % ("PASS" if ok else "FAIL", sid, kv))
    if not ok:
        lines.append("%s %s-testnet-restored %s" % ("PASS" if restored else "FAIL", sid, rest_kv))
        lines.append("SKIP %s: the injection did not make the intended shape; no product verdict" % sid)
        lines.append("METRIC %s shape=invalid %s" % (sid.upper(), kv))
        return lines, 2
    res = []
    facts = {nm: node_facts(run, nm) for nm in run.M}
    bound = 30

    def add(name, good, kv_):
        res.append(good)
        lines.append("%s %s-%s %s" % ("PASS" if good else "FAIL", sid, name, kv_))

    rj = [f["rejoin_slots"] for f in facts.values()]
    within = all(r is not None and r <= bound for r in rj)
    clean_hist = all(f["complete"] and f["missing"] == 0 for f in facts.values())
    no_snap = all(f["snap"] == 0 and f["block1_ok"] for f in facts.values())
    hist_kv = "missing=%s complete=%s" % (per_node(run, facts, "missing"), per_node(run, facts, "complete"))
    snap_kv = "snap_lines=%s block1_equal=%s" % (per_node(run, facts, "snap"), per_node(run, facts, "block1_ok"))
    if sid in ("gs004", "gs022", "gs033"):
        add("canonical-within-30-slots", within, "rejoin_slots=%s bound=%d" % (per_node(run, facts, "rejoin_slots"), bound))
    if sid in ("gs004", "gs022"):
        add("by-reorg", all(f["reorg"] > 0 for f in facts.values()), "reorg_lines=%s" % per_node(run, facts, "reorg"))
    if sid == "gs022":
        add("fork-walk-path", all(f["walk"] > 0 and f["reorg"] > 0 for f in facts.values()),
            "walk_or_wedge_lines=%s reorg_lines=%s" % (per_node(run, facts, "walk"), per_node(run, facts, "reorg")))
        add("no-ineligible-canonical", all(f["inelig_canonical"] == 0 for f in facts.values()),
            "ineligible_lines_naming_canonical=%s" % per_node(run, facts, "inelig_canonical"))
    if sid == "gs007":
        add("rejoined", all(r is not None for r in rj), "rejoin_slots=%s" % per_node(run, facts, "rejoin_slots"))
        add("no-return-after-rejoin", all(f["returns"] == 0 for f in facts.values()),
            "returns=%s first_canonical_slots=%s" % (per_node(run, facts, "returns"), per_node(run, facts, "first_canon_slots")))
        stuck = {nm: sum(1 for r in f["rollbacks"] if r["stuck"]) for nm, f in facts.items()}
        add("rollbacks-rejoin", all(v == 0 for v in stuck.values()) and all(r is not None for r in rj),
            "rollbacks=%s dead_reapply_stuck=%s" % (per_node(run, facts, "rollbacks", len),
                                                    ",".join("%s:%d" % x for x in stuck.items())))
    if sid == "gs033":
        dn = {nm: sum(1 for r in f["rollbacks"] if r["dead_next"]) for nm, f in facts.items()}
        add("no-return-after-rollback", all(f["returns"] == 0 and dn[nm] == 0 for nm, f in facts.items()),
            "returns_after_canonical=%s rollback_then_dead_apply=%s rollbacks=%s" % (
                per_node(run, facts, "returns"), ",".join("%s:%d" % x for x in dn.items()),
                per_node(run, facts, "rollbacks", len)))
    if sid in ("gs004", "gs022", "gs033"):
        add("no-snap-sync", no_snap, snap_kv)
    if sid in ("gs004", "gs007", "gs022", "gs033"):
        add("no-missing-heights", clean_hist, hist_kv)
    if sid == "gs029":
        gs029(run, add)
    add("testnet-restored", restored, rest_kv)
    worst = max((r for r in rj if r is not None), default=None)
    vers = sorted({b["version"].replace("doli-node ", "") for b in run.R.get("binaries", {}).values()})
    lines.append("METRIC %s_REJOIN_SLOTS=%s minority=%s X=%s B=%s dead_len=%s versions=%s" % (
        sid.upper(), worst if worst is not None else "none", ",".join(run.M), run.R.get("X"), run.B,
        run.R.get("dead_len"), "|".join(vers)))
    return lines, 0 if all(res) else 1


def gs029(run, add):
    R = run.R
    bins = R.get("binaries", {})
    vers = {b["version"] for b in bins.values()}
    alive = R.get("procs_alive_at_end", {})
    mapped = [nm for nm, b in bins.items() if b.get("mapped")]

    def tag(nm):
        return "x" if nm == R.get("X") else ("m%d" if nm in run.M else "e%d") % bins[nm]["i"]
    ran = bool(mapped) and all(alive.get(tag(nm)) is True for nm in mapped)
    add("mixed-binaries", len(vers) >= 2 and ran,
        "versions=%d mapped=%s mapped_running_at_end=%s" % (len(vers), ",".join(mapped) or "none", ran))
    last = (R.get("end_table") or [{}])[-1]
    rows = last.get("rows", {})
    tips = {json.dumps(r.get("tip")) for r in rows.values()}
    add("one-tip", bool(rows) and len(tips) == 1 and "null" not in tips,
        "nodes=%d distinct_tips=%d attempts=%d" % (len(rows), len(tips), len(R.get("end_table") or [])))
    agree = []
    for h in last.get("heights", []):
        hs = {r["at"].get(str(h)) for r in rows.values()}
        agree.append("h%s:%s" % (h, "same" if len(hs) == 1 and None not in hs else "DIFF"))
    add("history-agrees", bool(agree) and all(a.endswith(":same") for a in agree), " ".join(agree))
    bad = {}
    for nm in bins:
        n = sum(1 for ln in probes.read_lines(os.path.join(run.d, "post-heal-%s.log" % nm)) if any(p in ln for p in WEDGE))
        if n:
            bad[nm] = n
    add("no-wedge-after-heal", not bad, "finality_conflict_or_wedged=%s" % (json.dumps(bad) if bad else "0"))
