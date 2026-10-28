#!/usr/bin/env python3
"""plan.py — slot schedule and dead-branch plan for the GS-033 runner (INC-I-281).

Shape: producer X stops right after its own block at a height in [B-UPPER, B-LOWER], so its last
attestation is outside the attestation window of boundary B (3 epochs) but inside the window of
B-EPOCH. The minority M (1-2 producer nodes) builds the boundary block D_B at slot S (and D_{B+1}
for a 2-block branch); X builds the canonical B at slot S+1, so only the canonical branch carries
X: the dead branch freezes one producer fewer at B than the canonical branch (the list diverges).
The heal comes 4 s before the minority's next slot on its dead list, when the canonical branch is
heavier than the dead one.
"""
import os
import time

import tn


def bounds():
    e = tn.NET["epoch"]
    lo = int(os.environ.get("GS033_X_LOWER", str(3 * e + 4)))
    hi = int(os.environ.get("GS033_X_UPPER", str(4 * e - 4)))
    return lo, hi


class TooFewBlocks(RuntimeError):
    pass


def learn_order(allow_prev=False, min_blocks=12, n=30):
    """Active list in slot order (leader = list[slot % len]) from the seed's recent blocks of the
    current epoch only (a list can change at a boundary). Too few blocks yet: TooFewBlocks, or
    (allow_prev, plan mode) the previous epoch's blocks."""
    e = tn.NET["epoch"]
    h = tn.tip(0)[0]
    lo = (h // e) * e + 1
    if h - lo + 1 < min_blocks:
        if not allow_prev:
            raise TooFewBlocks("only %d blocks in the current epoch" % (h - lo + 1))
        lo, h = max(1, lo - e), lo - 1
    lo = max(lo, h - n + 1)
    blocks = [tn.block(0, hh) for hh in range(lo, h + 1)]
    seen = {}
    for b in blocks:
        if not b:
            raise RuntimeError("seed did not serve a recent block")
        seen.setdefault(tn.pnode(b), int(b["slot"]))
    if None in seen:
        raise RuntimeError("a recent block was built by a key no plist names")
    size = len(seen)
    by_mod = {}
    for b in blocks:
        s, node = int(b["slot"]), tn.pnode(b)
        if by_mod.get(s % size, node) != node:
            raise RuntimeError("recent blocks are not a %d-producer slot round-robin" % size)
        by_mod[s % size] = node
    if sorted(by_mod) != list(range(size)):
        raise RuntimeError("could not map every slot residue to a producer (%d blocks)" % len(blocks))
    return [by_mod[k] for k in range(size)]


def plan_from_S(order, X, M, dead_len, S, sT):
    lead = lambda s: order[s % len(order)]
    DL = [v for v in order if v != X]
    P1 = lead(S)
    dead, s = [(S, P1)], S
    while len(dead) < dead_len:
        s += 1
        if DL[s % len(DL)] in M:
            dead.append((s, DL[s % len(DL)]))
    s = dead[-1][0] + 1
    while DL[s % len(DL)] not in M:
        s += 1
    canon = [(t, lead(t)) for t in range(S + 1, s) if lead(t) not in M]
    xb = lead(S + 1) == X
    return {"sT": sT, "S": S, "P1": P1, "dead": dead, "heal_slot": s, "canon": canon, "DL": DL,
            "x_builds_B": xb, "ok": P1 in M and len(canon) > dead_len and xb}


def simulate(order, X, M, dead_len, h0, s0, B, sx, drift=0):
    """From tip h0 at slot s0: every slot produces except X's slots after sx and `drift` misses."""
    lead = lambda s: order[s % len(order)]
    s, h, d = s0, h0, drift
    while h < B - 1:
        s += 1
        if lead(s) == X and s > sx:
            continue
        if d > 0 and lead(s) != X:
            d -= 1
            continue
        h += 1
    S = s + 1
    while lead(S) == X:
        S += 1
    return plan_from_S(order, X, M, dead_len, S, s)


def choose_plan(order, M, dead_len, forced_x, donor, h0, s0, min_lead_slots, epochs=None):
    """Earliest boundary B with a valid plan; prefer one that also survives one missed slot."""
    lower, upper = bounds()
    e = tn.NET["epoch"]
    epochs = epochs or int(os.environ.get("GS033_PLAN_EPOCHS", "10"))
    cands = [forced_x] if forced_x is not None else [v for v in order if v not in M and v != donor]
    best = None
    b0 = ((h0 + min_lead_slots + lower) // e) * e
    for k in range(epochs):
        B = b0 + k * e
        for X in cands:
            # hx = h0 + (sx - s0) (every slot produces until X stops) must be in [B-upper, B-lower]
            for sx in range(max(s0 + min_lead_slots, s0 + B - upper - h0), s0 + B - lower - h0 + 1):
                if order[sx % len(order)] != X:
                    continue
                hx = h0 + (sx - s0)
                p0 = simulate(order, X, M, dead_len, h0, s0, B, sx, 0)
                if not p0["ok"]:
                    continue
                p1 = simulate(order, X, M, dead_len, h0, s0, B, sx, 1)
                score = (1 if p1["ok"] else 0, -B, -sx)
                if best is None or score > best[0]:
                    best = (score, B, X, sx, hx, p0, p1)
        if best and best[0][0] == 1:
            break
    return best


def fmt_plan(p):
    nm = tn.name
    return "S=%d P1=%s dead=%s heal_slot=%d canon=%s ok=%s" % (
        p["S"], nm(p["P1"]), [(a, nm(b)) for a, b in p["dead"]], p["heal_slot"],
        [(a, nm(b)) for a, b in p["canon"]], p["ok"])


def utc(t):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t))


def timeline(best, dead_len, obs, x_lead, follow_at):
    """Human timeline of one injection (estimates; the live run re-plans from its own tip)."""
    _, B, X, sx, hx, p0, p1 = best
    st, slot = tn.slot_start, tn.NET["slot"]
    rows = [("X=%s stops after its block at slot %d (h~%d)" % (tn.name(X), sx, hx), st(sx) + 3),
            ("X follower (non-producer) starts at tip >= B-%d" % follow_at,
             st(p0["sT"]) - follow_at * slot),
            ("X producer starts isolated (%d s before S+1)" % x_lead, st(p0["S"] + 1) - x_lead),
            ("CUT: minority held, X joins the majority (5 s before S=%d)" % p0["S"], st(p0["S"]) - 5),
            ("HEAL (4 s before minority dead-list slot %d)" % p0["heal_slot"], st(p0["heal_slot"]) - 4),
            ("observation ends (%d slots)" % obs, st(p0["heal_slot"]) - 4 + obs * slot)]
    return [(utc(t), what) for what, t in rows]
