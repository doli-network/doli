#!/usr/bin/env python3
"""restore.py — leave the testnet healthy after a GS-033 injection (INC-I-281).

Runs after tn.restore_all() put every node back under launchd on its installed binary:
 1. Wait GS033_RESTORE_WAIT s for every node to stand on the seed's chain (tip within 2 blocks).
 2. Each node that does not (a node still on its dead branch) gets the full-history restore of
    scripts/gauntlet.sh chaos_full_history_restore: the donor (GS033_DONOR, default n1) must be on
    the seed's chain with the seed's block 1; refuse if a target data dir holds a wallet or seed
    file; stop the targets, then the donor; wipe each target's data/* except its own
    signed_slots.db and maintainer_state.bin; rsync -a --delete the donor's blocks/ and state_db/;
    delete pending_update.json; start the donor, then each target (one testnet.sh call each).
 3. Final 7-node table (tip, h1, h300000, tip-10) and verifyChainIntegrity of restored nodes.
The seed is never a target and never stopped. Writes OUT/restore.json.
"""
import json
import os
import subprocess
import time

import probes
import tn

KEEP = {"signed_slots.db", "maintainer_state.bin"}


def rejoined(i):
    rt, t = tn.tip(0), tn.tip(i)
    return bool(rt and t and abs(rt[0] - t[0]) <= 2 and probes.on_ref(i, t, rt) is True)


def wait_rejoin(nodes, secs):
    end = time.time() + secs
    left = list(nodes)
    while left and time.time() < end:
        left = [i for i in left if not rejoined(i)]
        if left:
            time.sleep(5)
    return left


def secret_files(datadir):
    bad = []
    for root, dirs, files in os.walk(datadir):
        if root.count(os.sep) - datadir.count(os.sep) >= 1:
            dirs[:] = []
        bad += [os.path.join(root, f) for f in files if f.startswith("wallet") or f.endswith(".seed.txt")]
    return bad


def donor_restore(donor, targets, rec):
    s1, d1 = tn.bhash(0, 1), tn.bhash(donor, 1)
    if not (s1 and d1 == s1 and rejoined(donor)):
        raise RuntimeError("donor %s is not full-history on the seed chain (block1 %s vs seed %s)" % (
            tn.name(donor), d1, s1))
    for i in targets:
        bad = secret_files(tn.NODES[i]["datadir"])
        if bad:
            raise RuntimeError("%s data dir holds %s; refusing to wipe" % (tn.name(i), bad))
    for i in targets:
        tn.stop_launchd(i)
    tn.stop_launchd(donor)
    ddonor = tn.NODES[donor]["datadir"]
    try:
        for i in targets:
            td = tn.NODES[i]["datadir"]
            gone = []
            for f in os.listdir(td):
                if f not in KEEP:
                    subprocess.run(["rm", "-rf", os.path.join(td, f)], check=True)
                    gone.append(f)
            tn.say("%s: removed %s; kept %s" % (tn.name(i), gone, sorted(KEEP & set(os.listdir(td)))))
            for sub in ("blocks", "state_db"):
                r = subprocess.run(["rsync", "-a", "--delete", "%s/%s/" % (ddonor, sub), "%s/%s/" % (td, sub)],
                                   capture_output=True, text=True)
                tn.say("%s: rsync %s rc=%d %s" % (tn.name(i), sub, r.returncode, r.stderr[:200]))
                if r.returncode:
                    raise RuntimeError("rsync %s -> %s failed" % (sub, tn.name(i)))
            for pu in (os.path.join(td, "pending_update.json"),
                       os.path.join(os.path.dirname(td), "pending_update.json")):
                if os.path.exists(pu):
                    os.remove(pu)
                    tn.say("removed %s" % pu)
            rec["restored"].append(tn.name(i))
    finally:
        tn.start_launchd(donor)
        for i in targets:
            tn.start_launchd(i)


def heal_fleet(out, B=None):
    tn.OUT = out
    tn.LOGF = open(os.path.join(out, "restore.log"), "a")
    wait = int(os.environ.get("GS033_RESTORE_WAIT", "150"))
    donor = tn.idx(os.environ.get("GS033_DONOR", "n1"))
    rec = {"wait_s": wait, "donor": tn.name(donor), "restored": [], "errors": []}
    others = [n["i"] for n in tn.NODES if n["i"] != 0]
    try:
        down = [n["name"] for n in tn.NODES if not tn.wait_rpc(n["i"], 150)]
        if down:
            rec["errors"].append("RPC down after the launchd restore: %s" % down)
        stuck = wait_rejoin(others, wait)
        rec["stuck_after_launchd"] = [tn.name(i) for i in stuck]
        tn.say("after the launchd restore, not on the seed chain: %s" % rec["stuck_after_launchd"])
        if donor in stuck:
            raise RuntimeError("donor %s is itself off the seed chain; manual recovery needed" % tn.name(donor))
        if stuck:
            donor_restore(donor, stuck, rec)
            left = wait_rejoin(others, 300)
            rec["stuck_after_donor_restore"] = [tn.name(i) for i in left]
            if left:
                rec["errors"].append("still off the seed chain after the donor restore: %s" % rec["stuck_after_donor_restore"])
        lo = (B - 100) if B else max(1, tn.tip(0)[0] - 200)
        rec["integrity"] = {}
        for nm in rec["restored"]:
            i = tn.idx(nm)
            t = tn.tip(i)
            rec["integrity"][nm] = probes.integrity(i, lo, t[0]) if t else {"error": "down"}
            if tn.bhash(i, 1) != tn.bhash(0, 1):
                rec["errors"].append("%s block 1 differs from the seed's after the restore" % nm)
    except Exception as e:
        rec["errors"].append(repr(e))
        tn.say("RESTORE ERROR: %r" % (e,))
    time.sleep(tn.NET["slot"])
    rec["final_table"] = probes.fleet_table()
    last = rec["final_table"][-1]
    same = all(len({json.dumps(r["at"][str(h)]) for r in last["rows"].values()}) == 1 for h in last["heights"])
    rec["healthy"] = bool(last["one_tip"] and same and not rec["errors"])
    tn.say("restore: healthy=%s restored=%s errors=%s" % (rec["healthy"], rec["restored"], rec["errors"]))
    with open(os.path.join(out, "restore.json"), "w") as f:
        json.dump(rec, f, indent=1)
    return rec
