#!/usr/bin/env python3
"""gs033.py — orchestrator of scripts/gauntlet-gs033.sh (INC-I-281). See that file's header.

  gs033.py <gs004|gs007|gs022|gs029|gs033>   run (or reuse) the shape's injection, then judge
  gs033.py --plan [scenario]                 read-only: RPC plan + timeline, touches nothing
  gs033.py --restore RUN_DIR                 recover after an injection process died
"""
import hashlib
import itertools
import json
import os
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import inject  # noqa: E402
import judge  # noqa: E402
import plan  # noqa: E402
import restore  # noqa: E402
import tn  # noqa: E402

SCEN = {"gs004": "len1", "gs007": "len2", "gs022": "len2", "gs029": "len2", "gs033": "len2"}
USERS = {"len1": "gs004", "len2": "gs007 gs022 gs029 gs033"}


def shape_params(shape):
    dead_len = 1 if shape == "len1" else 2
    spec = os.environ.get("GS033_%s_MINORITY" % shape.upper(), "n4,n5")
    M = [tn.idx(x) for x in spec.split(",") if x.strip()]
    obs = int(os.environ.get("GS033_OBSERVE", "45"))
    if not 1 <= len(M) <= 2 or obs < 35:
        raise SystemExit("minority must hold 1-2 nodes and GS033_OBSERVE >= 35 slots")
    return M, dead_len, obs


def cache_key(shape, M, dead_len, obs, bins):
    blob = json.dumps({"shape": shape, "minority": [tn.name(m) for m in M], "dead_len": dead_len, "obs": obs,
                       "bins": {k: v["sha256"] for k, v in bins.items()}}, sort_keys=True)
    return hashlib.sha256(blob.encode()).hexdigest()[:16]


def pid_alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def read_pid(lockdir):
    try:
        return int(open(os.path.join(lockdir, "pid")).read().strip() or 0)
    except (OSError, ValueError):
        return 0


def take_lock(path):
    """mkdir lock with the owner's PID; a lock whose owner is gone is removed."""
    try:
        os.mkdir(path)
    except FileExistsError:
        pid = read_pid(path)
        if pid and pid_alive(pid):
            return False
        for f in os.listdir(path):
            os.remove(os.path.join(path, f))
        os.rmdir(path)
        os.mkdir(path)
    with open(os.path.join(path, "pid"), "w") as f:
        f.write(str(os.getpid()))
    return True


def drop_lock(path):
    try:
        os.remove(os.path.join(path, "pid"))
        os.rmdir(path)
    except OSError:
        pass


def injection(scen, session):
    """Return the run dir of this shape's injection for this binary map, running it if needed;
    or (None, reason) when it cannot run now."""
    shape = SCEN[scen]
    M, dead_len, obs = shape_params(shape)
    binmap = tn.parse_binmap()
    bins = tn.effective_binaries(binmap)
    key = cache_key(shape, M, dead_len, obs, bins)
    run = os.path.join(session, shape, key)
    done = os.path.join(run, "DONE")
    if os.path.exists(done) and os.environ.get("GS033_RETRY") == "1" and open(done).read().strip() != "ok":
        os.rename(run, "%s.invalid-%d" % (run, int(time.time())))
    if os.path.exists(done):
        tn.say("reusing the %s injection %s (%s)" % (shape, run, open(done).read().strip()[:120]))
        return run, None
    if os.path.isdir(run):
        pid = read_pid(os.path.join(run, "LOCK"))
        if pid and pid_alive(pid):
            return None, "another process runs this injection now (%s)" % run
        tn.say("previous injection in %s died; restoring the testnet from its state.json" % run)
        tn.manual_restore(run)
        restore.heal_fleet(run)
        os.rename(run, "%s.died-%d" % (run, int(time.time())))
        return None, "the previous injection of this shape died; testnet restored; re-run the scenario"
    down = [n["name"] for n in tn.NODES if tn.tip(n["i"]) is None]
    if down:
        return None, "node(s) down before the injection: %s" % " ".join(down)
    glock = os.path.join(tempfile.gettempdir(), "gauntlet-gs033.lock")
    if not take_lock(glock):
        return None, "another GS-033 injection is running on this testnet (%s)" % glock
    try:
        os.makedirs(os.path.join(run, "LOCK"))
        with open(os.path.join(run, "LOCK", "pid"), "w") as f:
            f.write(str(os.getpid()))
        with open(os.path.join(run, "shape.json"), "w") as f:
            json.dump({"scenario_first": scen, "shape": shape, "minority": [tn.name(m) for m in M],
                       "dead_len": dead_len, "observe": obs, "binaries": bins}, f, indent=1)
        R = {}
        try:
            R = inject.run(run, M, dead_len, obs, binmap, bins)
        finally:
            try:
                restore.heal_fleet(run, R.get("B"))
            finally:
                with open(done, "w") as f:
                    f.write((R.get("error") or "ok") + "\n")
                drop_lock(os.path.join(run, "LOCK"))
    finally:
        drop_lock(glock)
    return run, None


def run_scenario(scen):
    session = os.environ.get("GAUNTLET_TN_SESSION")
    note = None
    if not session:
        session = tempfile.mkdtemp(prefix="gauntlet-gs033-")
        note = "NOTE GAUNTLET_TN_SESSION unset: private session %s (no injection shared)" % session
    os.makedirs(session, exist_ok=True)
    tn.init(need_up=False)
    run, why = injection(scen, session)
    if run:
        lines, rc = judge.evaluate(scen, run)
    else:
        lines, rc = ["SKIP %s: %s" % (scen, why), "METRIC %s_REJOIN_SLOTS=none injection=not-run" % scen.upper()], 2
    if note:
        lines.insert(0, note)
    with open(os.path.join(session, "report-%s.txt" % scen), "w") as f:
        f.write("\n".join(lines) + "\nexit=%d\n" % rc)
    print("\n".join(lines), flush=True)
    return rc


def do_plan(scen):
    tn.init(need_up=True)
    order = plan.learn_order(allow_prev=True)
    donor = tn.idx(os.environ.get("GS033_DONOR", "n1"))
    forced = os.environ.get("GS033_X")
    h0, hh, s0 = tn.tip(0)
    binmap = tn.parse_binmap()
    bins = tn.effective_binaries(binmap)
    print("GS-033 plan (read-only, RPC only) at %s" % plan.utc(time.time()))
    print("testnet: slot=%ds epoch=%d blocks; seed tip h=%d slot %d %s" % (
        tn.NET["slot"], tn.NET["epoch"], h0, s0, hh[:16]))
    for n in tn.NODES:
        b = bins[n["name"]]
        print("  %-5s rpc=%d p2p=%d producer=%s %s [%s]" % (
            n["name"], n["rpc"], n["p2p"], (n["pk"] or "-")[:8], b["version"], "TN_BINMAP" if b["mapped"] else "installed"))
    print("order (leader = list[slot %% %d]): %s; donor=%s" % (len(order), " ".join(tn.name(v) for v in order), tn.name(donor)))
    session = os.environ.get("GAUNTLET_TN_SESSION")
    shapes = [SCEN[scen]] if scen else ["len1", "len2"]
    for shape in shapes:
        M, dead_len, obs = shape_params(shape)
        key = cache_key(shape, M, dead_len, obs, bins)
        cached = bool(session) and os.path.exists(os.path.join(session, shape, key, "DONE"))
        best = plan.choose_plan(order, M, dead_len, tn.idx(forced) if forced else None, donor, h0, s0, 25)
        print("\nshape %s (%s): minority=%s dead_len=%d observe=%d slots key=%s cached=%s" % (
            shape, USERS[shape], ",".join(tn.name(m) for m in M), dead_len, obs, key, cached))
        if not best:
            print("  NO PLAN within %s epochs for this minority (see the feasibility table)" % os.environ.get("GS033_PLAN_EPOCHS", "10"))
            continue
        _, B, X, sx, hx, p0, p1 = best
        print("  next boundary B=%d (epoch %d)  X=%s  X stops after its block at slot %d (h~%d)" % (
            B, B // tn.NET["epoch"], tn.name(X), sx, hx))
        print("  %s | robust to one missed slot: %s" % (plan.fmt_plan(p0), "yes" if p1["ok"] else "no"))
        print("  dead list at B (X absent): %s" % " ".join(tn.name(v) for v in p0["DL"]))
        for at, what in plan.timeline(best, dead_len, obs, int(os.environ.get("GS033_X_LEAD", "75")),
                                      int(os.environ.get("GS033_FOLLOW_AT", "14"))):
            print("    %s  %s" % (at, what))
    print("\nfeasibility: earliest B with a valid plan (X != donor, X leads S+1, canonical heavier at heal)")
    prods = [v for v in order if v != donor]
    cands = [[v] for v in prods] + [list(c) for c in itertools.combinations(prods, 2)]
    for M in cands:
        cells = []
        for dl in (1, 2):
            b = plan.choose_plan(order, M, dl, None, donor, h0, s0, 25)
            cells.append("len%d=%s" % (dl, ("B=%d X=%s%s" % (b[1], tn.name(b[2]), "" if b[0][0] else " (not robust)")) if b else "none"))
        print("  %-7s %s" % (",".join(tn.name(m) for m in M), "  ".join(cells)))
    return 0


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    if argv[0] == "--plan":
        tn.load_nodes()
        return do_plan(argv[1] if len(argv) > 1 else None)
    if argv[0] == "--restore":
        tn.init(need_up=False)
        run = os.path.abspath(argv[1])
        tn.manual_restore(run)
        rec = restore.heal_fleet(run)
        return 0 if rec.get("healthy") else 1
    if argv[0] in SCEN:
        return run_scenario(argv[0])
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
