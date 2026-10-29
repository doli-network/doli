#!/usr/bin/env python3
"""inject.py — one dead-branch injection on the LOCAL testnet for the GS-033 runner (INC-I-281).

run(out, M, dead_len, obs, binmap, bins) writes OUT/result.json and returns it:
 1. Front each minority node right after its own block: stop its launchd job, set its peers.cache
    aside, restart it (plist args, mapped binary if any) with its P2P listener on an internal port,
    --no-dht --no-discv5, --external-address = its public port and <= 5 bootstraps (the libp2p
    pending-dial limit): every majority node except X through its own fwd.py, plus the other
    minority node's internal port. Isolation check (lsof) at setup, every 60 s, pre-cut, cut+10 s
    and pre-heal; a leak before the cut aborts the run.
 2. Any other TN_BINMAP node restarts on its mapped binary right after its own slot.
 3. Plan from the live tip; X stops right after its own block at slot sx.
 4. X follower (non-producer) from B-FOLLOW_AT; X producer starts isolated behind its own forwarder
    X_LEAD s before S+1 (its startup gate clears; it attests nothing); released at the cut.
 5. Cut 5 s before S (minority forwarders hold), heal 4 s before the minority's next dead-list
    slot, observe OBS slots; a sampler records every tip every 2 s.
 6. Record what the judge needs (probes.py); finally tn.restore_all() undoes every change.
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import threading
import time

import plan
import probes
import tn

PATTERN_FROZEN = "Frozen producer list for epoch %d:"
BINCOPY = {}


def strip_opt(args, opt, has_val=True):
    out, k = [], 0
    while k < len(args):
        if args[k] == opt:
            k += 2 if has_val else 1
            continue
        out.append(args[k])
        k += 1
    return out


def bin_for(i, argv):
    return ([BINCOPY[i]] + argv[1:]) if i in BINCOPY else argv


def setup_binmap(binmap):
    """Copy each mapped binary to OUT/bin/doli-node-<node> and ad-hoc codesign the copy. The
    installed binaries and the plists are never modified: the launchd restore runs the originals."""
    os.makedirs(os.path.join(tn.OUT, "bin"), exist_ok=True)
    for i, src in sorted(binmap.items()):
        if i == 0 and os.environ.get("GS033_ALLOW_SEED") != "1":
            raise RuntimeError("TN_BINMAP names the seed; set GS033_ALLOW_SEED=1 to restart it")
        dst = os.path.join(tn.OUT, "bin", "doli-node-%s" % tn.name(i))
        shutil.copy2(src, dst)
        subprocess.run(["codesign", "--force", "--sign", "-", dst], check=True, capture_output=True)
        BINCOPY[i] = dst
        tn.say("binmap %s -> %s (%s, codesigned copy of %s)" % (tn.name(i), dst, tn.version(dst), src))


def args(i):
    return list(tn.NODES[i]["args"])


def minority_argv(m, M, X):
    a = strip_opt(args(m), "--bootstrap")
    a[a.index("--p2p-port") + 1] = str(tn.internal(m))
    boots = []
    for n in tn.NODES:
        j = n["i"]
        if j in (m, X):
            continue
        boots.append(tn.internal(j) if j in M else tn.fwd_port(m, j))
    if len(boots) > 5:
        raise RuntimeError("too many bootstraps for %s: %s (pending-dial limit 5)" % (tn.name(m), boots))
    for port in boots:
        a += ["--bootstrap", "/ip4/127.0.0.1/tcp/%d" % port]
    a += ["--no-dht", "--no-discv5", "--external-address", "/ip4/127.0.0.1/tcp/%d" % tn.NODES[m]["p2p"]]
    return bin_for(m, a)


def x_follower_argv(x, M):
    a = strip_opt(args(x), "--bootstrap")
    a = strip_opt(strip_opt(strip_opt(a, "--producer", False), "--producer-key"), "--force-start", False)
    for n in tn.NODES:
        if n["i"] != x and n["i"] not in M:
            a += ["--bootstrap", "/ip4/127.0.0.1/tcp/%d" % n["p2p"]]
    return bin_for(x, a + ["--no-dht", "--no-discv5"])


def x_isolated_argv(x, M):
    a = strip_opt(args(x), "--bootstrap")
    a[a.index("--p2p-port") + 1] = str(tn.internal(x))
    for n in tn.NODES:
        if n["i"] != x and n["i"] not in M:
            a += ["--bootstrap", "/ip4/127.0.0.1/tcp/%d" % tn.fwd_port(x, n["i"])]
    a += ["--no-dht", "--no-discv5", "--external-address", "/ip4/127.0.0.1/tcp/%d" % tn.NODES[x]["p2p"]]
    return bin_for(x, a)


def spawn_node(tag, i, argv, logname):
    p = tn.spawn(tag, argv, logname)
    probes.ACTIVE_LOG[i] = os.path.join(tn.OUT, logname)
    return p


def spawn_fwd(tag, owner, M, ctl, held):
    fl = os.path.join(tn.OUT, "fwd-%s.log" % tn.name(owner))
    fargs = [sys.executable, "-I", os.path.join(tn.HERE, "fwd.py"), "--log", fl, "--ctl", ctl,
             "--mode", "hold", "--map", "%d:%d:%d-99" % (tn.NODES[owner]["p2p"], tn.internal(owner), owner)]
    for n in tn.NODES:
        j = n["i"]
        if j != owner and j not in M:
            fargs += ["--map", "%d:%d:%d-%d" % (tn.fwd_port(owner, j), n["p2p"], owner, j)]
    fp = tn.spawn(tag, fargs, "fwd-%s.out" % tn.name(owner))
    for _ in range(50):
        time.sleep(0.2)
        if os.path.exists(fl) and "LISTEN" in open(fl).read():
            break
    if held:
        time.sleep(0.5)
        os.kill(fp.pid, signal.SIGUSR1)
        time.sleep(0.3)
    return fp


def x_isolated_start(x, M):
    """Replace the X follower by the X producer with every X link held, so it attests nothing and
    its startup gate (60 s timer) clears before its slot S+1; released at the cut."""
    tn.kill("xf", 30)
    tn.move_cache(x)
    ctl = os.path.join(tn.OUT, "x.ctl")
    with open(ctl, "w") as f:
        f.write("%d\n" % x)
    spawn_fwd("fwdx", x, M, ctl, held=True)
    spawn_node("x", x, x_isolated_argv(x, M), "%s.node.log" % tn.name(x))


def wait_own_slot(i, order, secs=120):
    """Return right after node i built its block in the current round (2 s into its slot)."""
    for _ in range(secs):
        cs = tn.cur_slot()
        if i not in order or (order[cs % len(order)] == i and time.time() - tn.slot_start(cs) >= 2):
            return
        time.sleep(1)


def fleet_healthy():
    rt = tn.tip(0)
    bad = []
    for n in tn.NODES:
        t = tn.tip(n["i"])
        if not t or abs(t[0] - rt[0]) > 2 or probes.on_ref(n["i"], t, rt) is not True:
            bad.append(n["name"])
    return bad


def front_minority(M, X, order):
    for m in M:
        wait_own_slot(m, order)
        tn.stop_launchd(m)
        tn.move_cache(m)
        spawn_fwd("fwd%d" % m, m, M, os.path.join(tn.OUT, "minority.ctl"), held=False)
        time.sleep(1)
        spawn_node("m%d" % m, m, minority_argv(m, M, X), "%s.node.log" % tn.name(m))
        if not tn.wait_rpc(m, 60):
            raise RuntimeError("%s RPC not up behind the forwarder" % tn.name(m))


def restart_mapped_others(M, X, order):
    for e in sorted(BINCOPY):
        if e in M or e == X:
            continue
        wait_own_slot(e, order)
        tn.stop_launchd(e)
        spawn_node("e%d" % e, e, bin_for(e, args(e)), "%s.node.log" % tn.name(e))
        if not tn.wait_rpc(e, 90):
            raise RuntimeError("%s RPC not up on the mapped binary" % tn.name(e))


def stop_x_after_its_block(order, M, dead_len, X):
    attempts = int(os.environ.get("GS033_ATTEMPTS", "2"))
    while True:
        h0, _, s0 = tn.tip(0)
        best = plan.choose_plan(order, M, dead_len, X, None, h0, s0, 3)
        if not best:
            raise RuntimeError("no plan")
        _, B, X, sx, hx, p0, p1 = best
        tn.say("PLAN B=%d X=%s X-stop-slot=%d (h~%d, in %ds) | %s | drift+1 ok=%s" % (
            B, tn.name(X), sx, hx, tn.slot_start(sx) - int(time.time()), plan.fmt_plan(p0), p1["ok"]))
        while time.time() < tn.slot_start(sx) + 3:
            time.sleep(0.5)
        t = None
        for _ in range(10):
            t = tn.tip(0)
            if t and t[2] >= sx:
                break
            time.sleep(0.5)
        bx = tn.block(0, t[0]) if t else None
        if t and t[2] == sx and tn.pnode(bx) == X:
            break
        attempts -= 1
        tn.say("X block at slot %d not seen (tip slot %s by %s); re-plan (%d left)" % (
            sx, t[2] if t else None, tn.pname(bx), attempts))
        if attempts <= 0:
            raise RuntimeError("could not time the X stop")
    tn.stop_launchd(X)
    return B, t, sx, p0


def wait_for_boundary(order, M, dead_len, X, B):
    follow_at = B - int(os.environ.get("GS033_FOLLOW_AT", "14"))
    x_lead = int(os.environ.get("GS033_X_LEAD", "75"))
    follower, last, x_up = False, 0, False
    while True:
        t = tn.tip(0)
        if not t:
            time.sleep(1)
            continue
        if t[0] >= B - 1:
            return x_up
        if follower and not x_up:
            pS = plan.simulate(order, X, M, dead_len, t[0], t[2], B, -1, 0)["S"]
            if time.time() >= tn.slot_start(pS + 1) - x_lead:
                tn.say("X producer start (isolated) %ds before predicted slot S+1=%d" % (
                    tn.slot_start(pS + 1) - int(time.time()), pS + 1))
                x_isolated_start(X, M)
                x_up = True
        if time.time() - last > 60:
            last = time.time()
            p = plan.simulate(order, X, M, dead_len, t[0], t[2], B, -1, 0)
            tn.say("live check tip=%d: %s" % (t[0], plan.fmt_plan(p)))
            if not p["ok"]:
                raise RuntimeError("schedule drifted: no dead branch with this minority")
            if probes.lsof_check("wait-%d" % t[0], M):
                raise RuntimeError("direct link to the minority before the cut (isolation leak)")
        if not follower and t[0] >= follow_at:
            tn.move_cache(X)
            spawn_node("xf", X, x_follower_argv(X, M), "%s.follower.log" % tn.name(X))
            follower = True
        time.sleep(1)


def table_at(B):
    rows = {}
    for n in tn.NODES:
        t = tn.tip(n["i"])
        row = {"tip": [t[0], t[1][:16]] if t else None}
        for hh in range(B - 1, B + 4):
            b = tn.block(n["i"], hh)
            row["h%d" % hh] = "%s/s%s/%s" % (b["hash"][:16], b["slot"], tn.pname(b)) if b else None
        rows[n["name"]] = row
    return rows


def run(out, M, dead_len, obs, binmap, bins):
    tn.OUT = out
    os.makedirs(out, exist_ok=True)
    tn.LOGF = open(os.path.join(out, "harness.log"), "a")
    result = {"minority": [tn.name(m) for m in M], "dead_len": dead_len, "observe": obs, "ref": tn.name(0),
              "slot": tn.NET["slot"], "epoch": tn.NET["epoch"], "binaries": bins}
    forced = os.environ.get("GS033_X")

    def handler(sig, frm):
        raise KeyboardInterrupt("signal %d" % sig)
    signal.signal(signal.SIGTERM, handler)
    signal.signal(signal.SIGINT, handler)
    try:
        bad = fleet_healthy()
        if bad:
            raise RuntimeError("fleet not on one chain before the injection: %s" % bad)
        while True:
            try:
                order = plan.learn_order()
                break
            except plan.TooFewBlocks as e:
                tn.say("waiting for the order: %s" % e)
                time.sleep(tn.NET["slot"] * 4)
        donor = tn.idx(os.environ.get("GS033_DONOR", "n1"))
        result["order"] = [tn.name(v) for v in order]
        if any(m not in order or m in (0, donor) for m in M):
            raise RuntimeError("every minority node must hold an active producer and not be the seed or donor")
        h0, _, s0 = tn.tip(0)
        pre = plan.choose_plan(order, M, dead_len, tn.idx(forced) if forced else None, donor, h0, s0, 25)
        if not pre:
            raise RuntimeError("no plan within GS033_PLAN_EPOCHS epochs for minority %s" % result["minority"])
        X = pre[2]
        result["X"] = tn.name(X)
        tn.say("PRE-PLAN B=%d X=%s X-stop-slot=%d | %s" % (pre[1], tn.name(X), pre[3], plan.fmt_plan(pre[5])))
        result["log_offsets"] = {n["name"]: probes.size(n["log"]) for n in tn.NODES}
        setup_binmap(binmap)
        front_minority(M, X, order)
        restart_mapped_others(M, X, order)
        time.sleep(25)
        if probes.lsof_check("setup", M, tolerate=(tn.name(X),)):  # X's launchd links end when X stops
            raise RuntimeError("direct link to the minority before the cut")
        threading.Thread(target=probes.sampler, daemon=True).start()
        B, t, sx, p0 = stop_x_after_its_block(order, M, dead_len, X)
        result.update({"B": B, "x_last_block": [t[0], sx], "plan0": p0})
        tn.say("X=%s stopped; its last block h=%d slot %d; B=%d" % (tn.name(X), t[0], sx, B))
        if not wait_for_boundary(order, M, dead_len, X, B):
            raise RuntimeError("X producer was not started before T")
        T = tn.block(0, B - 1)
        sT = int(T["slot"])
        S = sT + 1
        while order[S % len(order)] == X:
            S += 1
        p = plan.plan_from_S(order, X, M, dead_len, S, sT)
        result.update({"plan": p, "T": [B - 1, T["hash"][:16], sT]})
        tn.say("T=h%d %s slot %d; %s" % (B - 1, T["hash"][:16], sT, plan.fmt_plan(p)))
        if not p["ok"]:
            raise RuntimeError("final schedule check failed; no cut")
        t_cut = tn.slot_start(S) - 5
        if time.time() > t_cut - 0.5:
            raise RuntimeError("too late for the cut")
        if probes.lsof_check("pre-cut", M):
            raise RuntimeError("direct link to the minority at the cut (isolation leak); no cut")
        while time.time() < t_cut:
            time.sleep(0.1)
        with open(os.path.join(out, "minority.ctl"), "w") as f:
            f.write(" ".join(str(m) for m in M) + "\n")
        for m in M:
            os.kill(tn.ST["procs"]["fwd%d" % m].pid, signal.SIGUSR1)
        tn.ST["partitioned"] = True
        result["cut_at"] = time.time()
        os.kill(tn.ST["procs"]["fwdx"].pid, signal.SIGUSR2)  # X joins the majority side
        tn.say("CUT (slot S=%d starts in 5 s); X released to the majority" % S)
        time.sleep(10)
        bad = {"cut+10": probes.lsof_check("cut+10", M)}
        t_heal = tn.slot_start(p["heal_slot"]) - 4
        while time.time() < t_heal - 8:
            time.sleep(0.2)
        bad["pre-heal"] = probes.lsof_check("pre-heal", M)
        while time.time() < t_heal:
            time.sleep(0.1)
        pre_heal = table_at(B)
        heal_logs = {n["name"]: [probes.active_log(n["i"]), probes.size(probes.active_log(n["i"]))] for n in tn.NODES}
        for m in M:
            os.kill(tn.ST["procs"]["fwd%d" % m].pid, signal.SIGUSR2)
        tn.ST["partitioned"] = False
        result.update({"pre_heal": pre_heal, "heal_at": time.time(), "isolation_bad": bad, "heal_logs": heal_logs})
        tn.say("HEAL")
        end = result["heal_at"] + obs * tn.NET["slot"]
        mark = 0
        while time.time() < end:
            if time.time() - mark > 60:
                mark = time.time()
                tn.say("obs: " + " ".join("%s=%s" % (n["name"], (tn.tip(n["i"]) or ["DOWN"])[0]) for n in tn.NODES))
            time.sleep(2)
        record(result, M, X, B)
    except BaseException as e:
        tn.say("ABORT/EXC: %r" % (e,))
        result["error"] = repr(e)
    finally:
        tn.restore_all()
        with open(os.path.join(out, "result.json"), "w") as f:
            json.dump(result, f, indent=1, default=str)
        tn.say("injection done; result.json written (%s)" % result.get("error", "ok"))
    return result


def record(result, M, X, B):
    """Everything the judge reads, taken before the restore while the run's binaries still run."""
    out = tn.OUT
    result["end_at"] = time.time()
    result["procs_alive_at_end"] = {tag: p.poll() is None for tag, p in tn.ST["procs"].items()}
    for nm, (path, off) in result["heal_logs"].items():
        probes.slice_to(path, off, probes.size(path), os.path.join(out, "post-heal-%s.log" % nm))
    result["end_table"] = probes.fleet_table()
    time.sleep(tn.NET["slot"])
    rt = tn.tip(0)
    result["canonical"] = probes.canonical_blocks(B - 2, rt[0])
    result["seed_block1"] = tn.bhash(0, 1)
    per = {}
    for m in M:
        t = tn.tip(m)
        per[tn.name(m)] = {"final": [t[0], t[1]] if t else None, "block1": tn.bhash(m, 1),
                           "integrity": probes.integrity(m, B - 100, t[0]) if t else {"error": "down"}}
    result["per_minority"] = per
    key = PATTERN_FROZEN % (B // tn.NET["epoch"])
    proof = {}
    for n in tn.NODES:
        i = n["i"]
        if i in M or i == X:
            path, off = os.path.join(out, "%s.node.log" % n["name"]), 0
        else:
            path, off = n["log"], result["log_offsets"][n["name"]]
        if os.path.exists(os.path.join(out, "%s.node.log" % n["name"])) and i not in M and i != X:
            path, off = os.path.join(out, "%s.node.log" % n["name"]), 0  # mapped majority node
        proof[n["name"]] = probes.grep_from(path, off, key, 3)
    result["list_proof"] = proof
    tn.say("recorded: end table one_tip=%s, canonical %d blocks" % (
        result["end_table"][-1]["one_tip"], len(result["canonical"])))
