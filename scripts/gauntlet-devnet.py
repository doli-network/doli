#!/usr/bin/env python3
"""JSON verdict helper for the ephemeral-devnet gauntlet runners. Run: python3 -I gauntlet-devnet.py <sub> <args>

Every subcommand prints exactly one JSON object line. Exit 0 = defined positive result,
1 = defined negative result, 2 = malformed input or usage (verdict fields null).
  sample <scratch> <every_s> <port>...   append S/M/T lines to <scratch>/n<i>/samples.txt; every_s 0 = one poll
  scan <outdir> <port> <lo> <hi>         <outdir>/<h>.json = {"result": block}, txs expanded via getTransaction
  removed <blkdir> <h> <supply_prev> <supply_h> <subject_txid>
  producers-norm <file> <file>...        history <lo> <hi> <blkdir> <blkdir>...
  attrib <blkdir> <old_pk> <lo> <hi>     forkcheck <final_blkdir> <samples>...
  gset-merge <scratch> <n> <p>           chainspec <out> <gen_ts> <slot> <reward> <bond_unit> <name> <msg> <pk:bonds>...
"""
import json
import os
import re
import signal
import struct
import sys
import time
import urllib.request

HEX = re.compile(r"[0-9a-f]{64}\Z")
DIGITS = re.compile(r"[0-9]+\Z")
PRODUCER_TX = ("registration", "add_bond", "request_withdrawal")
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))
SHAPES = {
    "sample": {"polls": None},
    "scan": {"written": None, "missing": None, "slashes": []},
    "removed": {"height": None, "removed": None, "issued": None, "unresolved": []},
    "producers-norm": {"equal": None, "nodes": None, "set": None, "diff": []},
    "history": {"equal": None, "first_diff": None, "missing": []},
    "attrib": {"old_built": None, "old_built_producer_txs": None, "missing": []},
    "forkcheck": {"fork": None, "first_height": None, "sides": [], "absent": [], "min_height": None},
    "gset-merge": {"count": None},
    "chainspec": {"producers": None},
}


class Bad(Exception):
    pass


def emit(obj, rc):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
    return rc


def num(s):
    s = str(s)
    if not DIGITS.match(s):
        raise Bad("not digits: %r" % s[:40])
    return int(s)


def hexid(s):
    if not isinstance(s, str) or not HEX.match(s):
        raise Bad("not 64-hex: %r" % str(s)[:70])
    return s


def is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def amount(v):
    if not is_int(v) or v < 0:
        raise Bad("bad amount %r" % (v,))
    return v


def rpc(port, method, params=None):
    body = json.dumps({"jsonrpc": "2.0", "method": method, "params": params or {}, "id": 1}).encode()
    req = urllib.request.Request("http://127.0.0.1:%d" % port, data=body, headers={"Content-Type": "application/json"})
    try:
        with OPENER.open(req, timeout=5) as r:
            d = json.load(r)
    except Exception:
        return None
    return d.get("result") if isinstance(d, dict) else None


def load_block(path, height=None):
    if not os.path.isfile(path):
        return None
    name = os.path.basename(path)
    try:
        with open(path) as f:
            d = json.load(f)
    except (OSError, ValueError):
        raise Bad("unreadable block file %s" % name)
    if not isinstance(d, dict) or "error" in d:
        raise Bad("block file %s is not a block" % name)
    b = d.get("result", d)
    if not isinstance(b, dict) or not is_int(b.get("height")) or not isinstance(b.get("transactions"), list):
        raise Bad("block file %s has no height or transactions" % name)
    hexid(b.get("hash"))
    if height is not None and b["height"] != height:
        raise Bad("block file %s holds height %d" % (name, b["height"]))
    return b


def block_at(blkdir, h):
    return load_block(os.path.join(blkdir, "%d.json" % h), h)


def typed_txs(b):
    for t in b["transactions"]:
        if not isinstance(t, dict) or not isinstance(t.get("txType"), str):
            raise Bad("unclassifiable tx at height %d" % b["height"])
    return b["transactions"]


def out_sum(t):
    outs = t.get("outputs")
    if not isinstance(outs, list):
        raise Bad("tx %s has no outputs list" % str(t.get("hash"))[:16])
    total = 0
    for o in outs:
        if not isinstance(o, dict):
            raise Bad("malformed output")
        total += amount(o.get("amount"))
    return total


def outpoints(blkdir):
    idx = {}
    for name in os.listdir(blkdir):
        if not name.endswith(".json"):
            continue
        try:
            b = load_block(os.path.join(blkdir, name))
        except Bad:
            continue
        for t in b["transactions"]:
            if isinstance(t, dict) and isinstance(t.get("hash"), str) and isinstance(t.get("outputs"), list):
                for i, o in enumerate(t["outputs"]):
                    if isinstance(o, dict) and is_int(o.get("amount")):
                        idx[(t["hash"], i)] = o["amount"]
    return idx


def poll(scratch, ports, stop):
    for i, port in enumerate(ports):
        if stop:
            return
        a, b = rpc(port, "getChainStats"), rpc(port, "getChainStats")
        m = rpc(port, "getMempoolTransactions", {"limit": 500})
        lines = []
        h = a.get("height") if isinstance(a, dict) else None
        if is_int(h) and is_int(a.get("totalSupply")) and isinstance(b, dict) and \
                (h, a["totalSupply"]) == (b.get("height"), b.get("totalSupply")):
            lines.append("S %d %d" % (h, a["totalSupply"]))
        if isinstance(m, list):
            lines.append("M %d" % sum(1 for t in m if isinstance(t, dict) and t.get("txType") == "slashproducer"))
        blk = rpc(port, "getBlockByHeight", {"height": h}) if is_int(h) and h >= 1 else None
        if isinstance(blk, dict) and blk.get("height") == h and all(
                isinstance(blk.get(k), str) and HEX.match(blk[k]) for k in ("hash", "producer")):
            lines.append("T %d %d %s %s" % (i, h, blk["hash"], blk["producer"]))
        try:
            with open(os.path.join(scratch, "n%d" % i, "samples.txt"), "a") as f:
                f.write("".join(x + "\n" for x in lines))
        except OSError:
            pass


def cmd_sample(a, out):
    if len(a) < 3 or not os.path.isdir(a[0]):
        raise Bad("usage: sample <scratch> <every_s> <port>...")
    scratch, every, ports = a[0], num(a[1]), [num(p) for p in a[2:]]
    stop = []
    signal.signal(signal.SIGTERM, lambda *_: stop.append(1))
    ppid, polls = os.getppid(), 0
    while True:
        poll(scratch, ports, stop)
        polls += 1
        if every == 0:
            break
        end = time.time() + every
        while not stop and time.time() < end:
            time.sleep(0.2)
        if stop or not os.path.isdir(scratch) or os.getppid() != ppid:
            break
    out["polls"] = polls
    return emit(out, 0)


def cmd_scan(a, out):
    if len(a) != 4 or not os.path.isdir(a[0]):
        raise Bad("usage: scan <outdir> <port> <lo> <hi>")
    outdir, port, lo, hi = a[0], num(a[1]), num(a[2]), num(a[3])
    if lo > hi:
        raise Bad("lo > hi")
    out.update(written=0, missing=[])
    for h in range(lo, hi + 1):
        b = rpc(port, "getBlockByHeight", {"height": h})
        if not isinstance(b, dict) or not isinstance(b.get("transactions", []), list):
            out["missing"].append(h)
            continue
        txs = []
        for x in b.get("transactions", []):
            t = rpc(port, "getTransaction", {"hash": x}) if isinstance(x, str) else x
            txs.append(t if isinstance(t, dict) and t else x)
        b["transactions"] = txs
        with open(os.path.join(outdir, "%d.json" % h), "w") as f:
            json.dump({"result": b}, f)
        out["written"] += 1
        for t in txs:
            if isinstance(t, dict) and t.get("txType") == "slash_producer" and isinstance(t.get("hash"), str) \
                    and HEX.match(t["hash"]):
                out["slashes"].append([h, t["hash"]])
    return emit(out, 1 if out["missing"] else 0)


def cmd_removed(a, out):
    if len(a) != 5:
        raise Bad("usage: removed <blkdir> <h> <supply_prev> <supply_h> <subject_txid>")
    blkdir, h, sp, sh, subj = a[0], num(a[1]), num(a[2]), num(a[3]), hexid(a[4])
    b = block_at(blkdir, h)
    if b is None:
        raise Bad("no block file for height %d" % h)
    issued, other, found, idx = 0, 0, False, None
    unresolved = []
    for t in typed_txs(b):
        if t["txType"] == "coinbase":
            issued += out_sum(t)
            continue
        if t.get("hash") == subj:
            found = True
            continue
        ins = t.get("inputs")
        if not isinstance(ins, list):
            raise Bad("tx %s has no inputs list" % str(t.get("hash"))[:16])
        total_in = 0
        for i in ins:
            if not isinstance(i, dict):
                raise Bad("malformed input")
            v = i.get("amount")
            if not is_int(v):
                idx = outpoints(blkdir) if idx is None else idx
                v = idx.get((i.get("prevTxHash"), i.get("outputIndex")))
                if v is None:
                    unresolved.append("%s:%s" % (i.get("prevTxHash"), i.get("outputIndex")))
                    continue
            total_in += amount(v)
        other += total_in - out_sum(t)
    out.update(height=h, issued=issued, unresolved=unresolved)
    if not found or unresolved:
        out["error"] = "subject not in block" if not found else "unresolved inputs"
        return emit(out, 1)
    out["removed"] = sp - sh + issued - other
    return emit(out, 0)


def norm_file(path):
    name = os.path.basename(path)
    try:
        with open(path) as f:
            d = json.load(f)
    except (OSError, ValueError):
        raise Bad("unreadable %s" % name)
    if not isinstance(d, dict) or "error" in d or not isinstance(d.get("result"), list):
        raise Bad("%s is not a getProducers reply" % name)
    m = {}
    for p in d["result"]:
        if not isinstance(p, dict) or not isinstance(p.get("status"), str) or not is_int(p.get("bondCount")):
            raise Bad("malformed producer in %s" % name)
        pk = hexid(p.get("publicKey"))
        if pk in m:
            raise Bad("duplicate producer in %s" % name)
        m[pk] = (p["status"], p["bondCount"])
    return m


def cmd_norm(a, out):
    if len(a) < 2:
        raise Bad("usage: producers-norm <file> <file>...")
    maps = [norm_file(p) for p in a]
    keys = sorted(set().union(*maps))
    diff = [k for k in keys if len({m.get(k) for m in maps}) > 1]
    out.update(equal=not diff, nodes=len(maps), diff=diff,
               set=[[k, maps[0][k][0], maps[0][k][1]] for k in sorted(maps[0])])
    return emit(out, 0 if not diff else 1)


def cmd_history(a, out):
    if len(a) < 4:
        raise Bad("usage: history <lo> <hi> <blkdir> <blkdir>...")
    lo, hi, dirs = num(a[0]), num(a[1]), a[2:]
    if lo > hi:
        raise Bad("lo > hi")
    first, missing = None, []
    for h in range(lo, hi + 1):
        seen = set()
        for n, d in enumerate(dirs):
            b = block_at(d, h)
            if b is None:
                missing.append([n, h])
            else:
                seen.add(b["hash"])
        if len(seen) > 1 and first is None:
            first = h
    out.update(first_diff=first, missing=missing, equal=first is None and not missing)
    return emit(out, 0 if out["equal"] else 1)


def cmd_attrib(a, out):
    if len(a) != 4:
        raise Bad("usage: attrib <blkdir> <old_pk> <lo> <hi>")
    d, pk, lo, hi = a[0], hexid(a[1]), num(a[2]), num(a[3])
    if lo > hi:
        raise Bad("lo > hi")
    built, ptx, missing = [], 0, []
    for h in range(lo, hi + 1):
        b = block_at(d, h)
        if b is None:
            missing.append(h)
        elif b.get("producer") == pk:
            built.append(h)
            ptx += sum(1 for t in typed_txs(b) if t["txType"] in PRODUCER_TX)
    out.update(old_built=built, old_built_producer_txs=ptx, missing=missing)
    return emit(out, 1 if missing else 0)


def final_chain(d):
    chain = {}
    for name in os.listdir(d):
        m = re.match(r"([0-9]+)\.json\Z", name)
        if m:
            b = block_at(d, int(m.group(1)))
            chain[b["height"]] = (b["hash"], b.get("producer"))
    if not chain or sorted(chain) != list(range(min(chain), max(chain) + 1)):
        raise Bad("final chain is empty or has a gap")
    return chain


def cmd_forkcheck(a, out):
    if len(a) < 2:
        raise Bad("usage: forkcheck <final_blkdir> <samples>...")
    by_h, top, seen = {}, {}, set()
    for p in a[1:]:
        try:
            with open(p) as f:
                lines = f.read().splitlines()
        except OSError:
            raise Bad("unreadable samples %s" % os.path.basename(p))
        for line in lines:
            w = line.split()
            if not w or w[0] != "T":
                continue
            if len(w) != 5:
                raise Bad("malformed T line")
            node, h, hsh, prod = num(w[1]), num(w[2]), hexid(w[3]), hexid(w[4])
            by_h.setdefault(h, {}).setdefault(hsh, prod)
            top[node] = max(top.get(node, 0), h)
            seen.add((node, h, hsh))
    if not top:
        raise Bad("no T samples")
    chain = final_chain(a[0])
    mn = min(top.values())
    bad, absent = set(), []
    for node, h, hsh in sorted(seen):
        if h not in chain:
            raise Bad("sampled height %d is outside the final chain" % h)
        if chain[h][0] != hsh:
            absent.append([node, h, hsh])
            bad.add(h)
    bad.update(h for h, hs in by_h.items() if h <= mn and len(hs) > 1)
    out.update(min_height=mn, absent=absent, fork=bool(bad))
    if not bad:
        return emit(out, 0)
    fh = min(bad)
    sides = dict(by_h[fh])
    sides.setdefault(chain[fh][0], chain[fh][1])
    out.update(first_height=fh, sides=sorted([k, v] for k, v in sides.items()))
    return emit(out, 1)


def cmd_gset(a, out):
    if len(a) != 3:
        raise Bad("usage: gset-merge <scratch> <n> <p>")
    s, n, p = a[0], num(a[1]), num(a[2])
    count, body = 0, b""
    try:
        for i in range(p):
            with open(os.path.join(s, "n%d" % i, "data", "producer_gset.bin"), "rb") as f:
                d = f.read()
            if len(d) < 8:
                raise Bad("short producer_gset.bin for node %d" % i)
            count, body = count + struct.unpack("<Q", d[:8])[0], body + d[8:]
        for i in range(n):
            with open(os.path.join(s, "n%d" % i, "data", "producer_gset.bin"), "wb") as f:
                f.write(struct.pack("<Q", count) + body)
    except OSError as e:
        raise Bad("gset io: %s" % e.strerror)
    out["count"] = count
    return emit(out, 0)


def cmd_chainspec(a, out):
    if len(a) < 8:
        raise Bad("usage: chainspec <out> <gen_ts> <slot> <reward> <bond_unit> <name> <msg> <pk:bonds>...")
    prods = []
    for i, spec in enumerate(a[7:]):
        pk, _, cnt = spec.partition(":")
        if not pk:
            raise Bad("empty producer key")
        prods.append({"name": "p%d" % i, "public_key": pk, "bond_count": num(cnt)})
    spec = {"name": a[5], "id": "ephemeral-devnet", "network": "Devnet",
            "genesis": {"timestamp": num(a[1]), "message": a[6], "initial_reward": num(a[3])},
            "consensus": {"slot_duration": num(a[2]), "slots_per_epoch": 30, "bond_amount": num(a[4])},
            "genesis_producers": prods}
    try:
        with open(a[0], "w") as f:
            f.write(json.dumps(spec, indent=2))
    except OSError as e:
        raise Bad("chainspec io: %s" % e.strerror)
    out["producers"] = len(prods)
    return emit(out, 0)


CMDS = {"sample": cmd_sample, "scan": cmd_scan, "removed": cmd_removed, "producers-norm": cmd_norm,
        "history": cmd_history, "attrib": cmd_attrib, "forkcheck": cmd_forkcheck, "gset-merge": cmd_gset,
        "chainspec": cmd_chainspec}


def main(argv):
    if len(argv) < 2 or argv[1] not in CMDS:
        return emit({"error": "usage: %s" % " | ".join(sorted(CMDS))}, 2)
    sub = argv[1]
    try:
        return CMDS[sub](argv[2:], json.loads(json.dumps(SHAPES[sub])))
    except Exception as e:
        out = json.loads(json.dumps(SHAPES[sub]))
        out["error"] = str(e)[:200] if isinstance(e, Bad) else "%s: %s" % (type(e).__name__, str(e)[:160])
        return emit(out, 2)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
