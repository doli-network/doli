#!/usr/bin/env python3
"""Offline tests for scripts/gauntlet-devnet.py, the JSON verdict helper of the GS-028/GS-029 runners.

Canned JSON fixtures plus an in-process loopback mock of the node RPC; never starts a node.
Run: python3 -I scripts/test_gauntlet_devnet_py.py
"""
# REQ-LDP-029 — Decision: a FAIL means a pure runner helper (supply arithmetic, producer-set normalisation, hash history, attribution) returns a wrong verdict on fixed input, so GS-028/GS-029 PASS or FAIL for an arithmetic reason.
# REQ-LDP-008 — Decision: a FAIL means removed(S) is not supply(S-1) - supply(S) + issued(S) minus the other transactions' net, so the locked-amount burn check compares the wrong number.
# REQ-LDP-009 — Decision: a FAIL means removed(W) does not isolate the withdrawal's own net, so removed(W) + removed(S) == B can hold or break for an unrelated transfer or fee.
# REQ-LDP-010 — Decision: a FAIL means an unresolvable input still yields a number, so a funds assertion can PASS on an undefined measurement.
# REQ-LDP-017 — Decision: a FAIL means history misses a differing or missing height, or forkcheck misses a sampled tip that left the final chain, so a healed fork reads as agreement.
# REQ-LDP-020 — Decision: a FAIL means forkcheck calls a stall a fork or a fork a stall, so the negative control exits 1 or 2 for the wrong reason.
# REQ-LDP-023 — Decision: a FAIL means malformed RPC JSON or a non-digit / non-hex value yields a verdict instead of exit 2, so crafted node output could produce a PASS.
# REQ-LDP-028 — Decision: a FAIL means attrib miscounts old-built blocks or the producer transactions in them, so the METRIC misreports what old producers packed.
#
# OUTPUT CONTRACT: python3 -I scripts/gauntlet-devnet.py <sub> <args>
#   every subcommand: stdout is exactly ONE line holding a JSON object; exit 0 defined positive result ·
#   1 defined negative result · 2 malformed input or usage (verdict field null/false)
#   scan <outdir> <port> <lo> <hi>          writes <outdir>/<h>.json = {"result": block} with transactions
#                                           replaced by getTransaction objects (a hash getTransaction cannot
#                                           serve stays the raw string) · {"written": n, "missing": [h]} ·
#                                           exit 0 iff missing == []
#   removed <blkdir> <h> <supply_prev> <supply_h> <subject_txid>
#                                           issued = coinbase outputs of h; other = every tx of h except coinbase
#                                           and the subject; an input with no amount resolves through its
#                                           outpoint in <blkdir>/*.json · {"height", "removed", "issued",
#                                           "unresolved": ["<txid>:<index>"]} · exit 1 with removed null when an
#                                           other-tx input or the subject cannot be resolved/found
#   producers-norm <file> <file> [...]      getProducers replies -> sorted [publicKey, status, bondCount] ·
#                                           {"equal", "nodes", "set", "diff": [publicKey]} · exit 2 if < 2 files
#   history <lo> <hi> <blkdir> <blkdir> [...]
#                                           {"equal", "first_diff": h|null, "missing": [[node, h]]} ·
#                                           exit 0 iff every height on every node, one hash each
#   attrib <blkdir> <old_pk> <lo> <hi>      {"old_built": [h], "old_built_producer_txs": n, "missing": [h]} ·
#                                           producer txs = registration, add_bond, request_withdrawal
#   forkcheck <final_blkdir> <samples> [...]
#                                           samples: `T <node> <height> <hash> <producer>` lines (S/M ignored) ·
#                                           fork = two sampled hashes at one height <= min over nodes of their
#                                           highest sample, or a sampled (h, hash) the final chain has not ·
#                                           {"fork", "first_height", "sides": [[hash, producer]], "absent":
#                                           [[node, h, hash]], "min_height"} · exit 0 no fork · 1 fork · 2 malformed
# INPUT PARTITIONS:
#   scan       served 1..2 · height 3 not served · one tx hash getTransaction cannot serve
#   removed    W block (coinbase, subject withdrawal, transfer with fee) · transfer input amount absent and
#              resolved via another block file · amount null and unresolvable · subject input unresolvable
#              (subject excluded: still defined) · S block (coinbase + slash) · subject absent from block ·
#              non-digit supply · non-hex txid · bare-string tx · missing block file
#   norm       same set reordered with a differing non-normalised field · one status differs · one producer
#              missing · malformed JSON · error envelope · single file
#   history    3 nodes equal · node 2 differs at 2 · node 1 missing 3 · malformed block file · lo > hi
#   attrib     old producer at 2 and 4 with 3 producer txs · range past the last block · bare-string tx ·
#              non-hex old_pk
#   forkcheck  stall (stuck tip, all hashes on the final chain) · live fork at 5 · healed fork at 3 ·
#              non-digit height · non-hex hash · final chain with a gap

import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HELPER = os.path.join(ROOT, "scripts", "gauntlet-devnet.py")
NP = 0
NF = 0


def ok(name, why=""):
    global NP
    NP += 1
    print("PASS %s%s" % (name, " (%s)" % why if why else ""))


def bad(name, why):
    global NF
    NF += 1
    print("FAIL %s: %s" % (name, why))


def hx(n):
    return "%064x" % n


PKA, PKB, OLD = "a" * 64, "b" * 64, "c" * 64


def run(*args):
    p = subprocess.run(
        [sys.executable, "-I", HELPER] + [str(a) for a in args],
        capture_output=True, text=True, timeout=60,
    )
    return p.returncode, p.stdout


def one_json(out):
    lines = out.splitlines()
    if len(lines) != 1:
        return None
    try:
        v = json.loads(lines[0])
    except ValueError:
        return None
    return v if isinstance(v, dict) else None


def tx(h, ttype, ins=(), outs=()):
    inputs = []
    for prev, idx, amount in ins:
        i = {"prevTxHash": prev, "outputIndex": idx, "signature": "00"}
        if amount != "absent":
            i["amount"] = amount
        inputs.append(i)
    outputs = [{"outputType": t, "amount": a, "pubkeyHash": "e" * 64, "lockUntil": lock, "extraDataSize": 0}
               for t, a, lock in outs]
    return {"hash": h, "version": 1, "txType": ttype, "inputs": inputs, "outputs": outputs,
            "covenantWitnesses": [], "size": 100, "fee": None}


def block(height, producer, txs, bhash=None):
    return {"hash": bhash or hx(0x1000 + height), "prevHash": hx(0x1000 + height - 1), "height": height,
            "slot": height, "timestamp": 1000 + height, "producer": producer, "merkleRoot": "0" * 64,
            "txCount": len(txs), "transactions": txs, "size": 900}


def write_dir(path, blocks, wrap=True):
    os.makedirs(path, exist_ok=True)
    for b in blocks:
        with open(os.path.join(path, "%d.json" % b["height"]), "w") as f:
            json.dump({"result": b} if wrap else b, f)
    return path


def expect(name, args, rc_want, check, why_ok=""):
    if not os.path.isfile(HELPER):
        bad(name, "scripts/gauntlet-devnet.py missing")
        return
    rc, out = run(*args)
    v = one_json(out)
    if v is None:
        bad(name, "stdout is not exactly one JSON object line: %r" % out[:200])
        return
    if rc != rc_want:
        bad(name, "exit %d want %d: %s" % (rc, rc_want, json.dumps(v)[:200]))
        return
    why = check(v)
    if why:
        bad(name, "%s: %s" % (why, json.dumps(v)[:240]))
        return
    ok(name, why_ok)


# --- removed ---------------------------------------------------------------------------
W, S_H = 20, 25
SUBJ, TRANS, COINB, SLASH, PREV = hx(0xA1), hx(0xA2), hx(0xA3), hx(0xA4), hx(0xA5)


def removed_fixtures(tmp):
    cb = tx(COINB, "coinbase", outs=[("normal", 200, 0)])
    wd = tx(SUBJ, "request_withdrawal", ins=[(hx(0xB0), 0, 1000)], outs=[("unbonding", 249, 100)])
    tr = tx(TRANS, "transfer", ins=[(PREV, 1, 500)], outs=[("normal", 499, 0)])
    src = tx(PREV, "transfer", ins=[(hx(0xB9), 0, 900)], outs=[("normal", 399, 0), ("normal", 500, 0)])
    sl = tx(SLASH, "slash_producer")
    d = {}
    d["w"] = write_dir(os.path.join(tmp, "w"), [block(W, PKA, [cb, wd, tr]), block(S_H, PKA, [cb, sl])])
    tr_absent = tx(TRANS, "transfer", ins=[(PREV, 1, "absent")], outs=[("normal", 499, 0)])
    d["resolved"] = write_dir(os.path.join(tmp, "resolved"), [block(19, PKB, [src]), block(W, PKA, [cb, wd, tr_absent])])
    tr_null = tx(TRANS, "transfer", ins=[(PREV, 1, None)], outs=[("normal", 499, 0)])
    d["unresolved"] = write_dir(os.path.join(tmp, "unresolved"), [block(W, PKA, [cb, wd, tr_null])])
    wd_null = tx(SUBJ, "request_withdrawal", ins=[(hx(0xB0), 0, None)], outs=[("unbonding", 249, 100)])
    d["subj_null"] = write_dir(os.path.join(tmp, "subj_null"), [block(W, PKA, [cb, wd_null, tr])])
    d["bare"] = write_dir(os.path.join(tmp, "bare"), [block(W, PKA, [cb, SUBJ, tr])])
    return d


def c_removed(tmp):
    d = removed_fixtures(tmp)
    want = 10000 - 9448 + 200 - 1

    def is_val(n):
        return lambda v: None if v.get("removed") == n and v.get("issued") == 200 else "removed %r issued %r want %d, 200" % (v.get("removed"), v.get("issued"), n)

    expect("removed-withdrawal-block", ["removed", d["w"], W, 10000, 9448, SUBJ], 0, is_val(want), "W: 751 = bond in - payout")
    expect("removed-slash-block", ["removed", d["w"], S_H, 9448, 9399, SLASH], 0, is_val(249), "S: the locked 249")
    expect("removed-resolves-outpoint", ["removed", d["resolved"], W, 10000, 9448, SUBJ], 0, is_val(want), "absent amount resolved via block 19")
    expect("removed-subject-inputs-not-needed", ["removed", d["subj_null"], W, 10000, 9448, SUBJ], 0, is_val(want))
    expect("removed-unresolvable-undefined", ["removed", d["unresolved"], W, 10000, 9448, SUBJ], 1,
           lambda v: None if v.get("removed") is None and any(PREV in u for u in v.get("unresolved", [])) else "want removed null and %s:1 unresolved" % PREV[:8])
    expect("removed-subject-absent", ["removed", d["w"], S_H, 9448, 9399, SUBJ], 1,
           lambda v: None if v.get("removed") is None else "subject not in block must leave removed null")
    for name, args in [
        ("removed-nondigit-supply", ["removed", d["w"], W, "10000", "94a8", SUBJ]),
        ("removed-negative-supply", ["removed", d["w"], W, "-5", 9448, SUBJ]),
        ("removed-nonhex-txid", ["removed", d["w"], W, 10000, 9448, "zz" * 32]),
        ("removed-bare-string-tx", ["removed", d["bare"], W, 10000, 9448, SUBJ]),
        ("removed-missing-block", ["removed", d["w"], 99, 10000, 9448, SUBJ]),
    ]:
        expect(name, args, 2, lambda v: None if v.get("removed") is None else "malformed input must not yield a number")


# --- producers-norm --------------------------------------------------------------------
def prod(pk, status, bonds, weight):
    return {"publicKey": pk, "addressHash": "d" * 64, "registrationHeight": 0, "bondAmount": bonds * 100,
            "bondCount": bonds, "producerSetBondCount": bonds, "status": status, "era": 0,
            "pendingWithdrawals": [], "delegatedBonds": 0, "selectionWeight": weight}


def c_norm(tmp):
    def put(name, body):
        p = os.path.join(tmp, name)
        with open(p, "w") as f:
            f.write(body if isinstance(body, str) else json.dumps(body))
        return p

    n1 = put("n1.json", {"jsonrpc": "2.0", "result": [prod(PKA, "active", 2, 2), prod(PKB, "exited", 0, 0)], "id": 1})
    n2 = put("n2.json", {"jsonrpc": "2.0", "result": [prod(PKB, "exited", 0, 9), prod(PKA, "active", 2, 7)], "id": 1})
    n3 = put("n3.json", {"jsonrpc": "2.0", "result": [prod(PKA, "active", 2, 2), prod(PKB, "slashed", 0, 0)], "id": 1})
    n4 = put("n4.json", {"jsonrpc": "2.0", "result": [prod(PKA, "active", 2, 2)], "id": 1})
    mal = put("mal.json", '{"jsonrpc":"2.0","result":[{"publicKey":"' + PKA)
    env = put("err.json", {"jsonrpc": "2.0", "error": {"code": -32000, "message": "x"}, "id": 1})
    want_set = [[PKA, "active", 2], [PKB, "exited", 0]]
    expect("norm-equal-reordered", ["producers-norm", n1, n2], 0,
           lambda v: None if v.get("equal") is True and v.get("set") == want_set and v.get("nodes") == 2 else "want equal, the sorted triple set, nodes 2")
    expect("norm-status-differs", ["producers-norm", n1, n2, n3], 1,
           lambda v: None if v.get("equal") is False and v.get("diff") == [PKB] else "want equal false, diff [PKB]")
    expect("norm-producer-missing", ["producers-norm", n1, n4], 1,
           lambda v: None if v.get("equal") is False and PKB in v.get("diff", []) else "want PKB in diff")
    for name, args in [("norm-malformed", [n1, mal]), ("norm-error-envelope", [n1, env]), ("norm-single-file", [n1])]:
        expect(name, ["producers-norm"] + args, 2, lambda v: None if v.get("equal") is not True else "malformed or single input must not be equal")


# --- history ---------------------------------------------------------------------------
def c_history(tmp):
    base = [block(h, PKA, []) for h in (1, 2, 3)]
    a = write_dir(os.path.join(tmp, "ha"), base)
    b = write_dir(os.path.join(tmp, "hb"), base)
    c = write_dir(os.path.join(tmp, "hc"), [base[0], block(2, PKB, [], hx(0xF2)), base[2]])
    m = write_dir(os.path.join(tmp, "hm"), base[:2])
    x = write_dir(os.path.join(tmp, "hx"), base)
    with open(os.path.join(x, "2.json"), "w") as f:
        f.write('{"result": {"hash": "')
    expect("history-equal", ["history", 1, 3, a, b, a], 0,
           lambda v: None if v.get("equal") is True and v.get("first_diff") is None and v.get("missing") == [] else "want equal, no diff, none missing")
    expect("history-differs", ["history", 1, 3, a, b, c], 1,
           lambda v: None if v.get("equal") is False and v.get("first_diff") == 2 else "want first_diff 2")
    expect("history-missing-height", ["history", 1, 3, a, m, b], 1,
           lambda v: None if v.get("equal") is False and [1, 3] in v.get("missing", []) else "want missing [[1, 3]]")
    expect("history-malformed-block", ["history", 1, 3, a, x], 2, lambda v: None if v.get("equal") is not True else "malformed must not be equal")
    expect("history-bad-range", ["history", 3, 1, a, b], 2, lambda v: None if v.get("equal") is not True else "lo > hi must not be equal")


# --- attrib ----------------------------------------------------------------------------
def c_attrib(tmp):
    cb = tx(COINB, "coinbase", outs=[("normal", 200, 0)])
    blocks = [
        block(1, PKA, [cb]),
        block(2, OLD, [cb, tx(hx(0xC1), "add_bond"), tx(hx(0xC2), "request_withdrawal")]),
        block(3, PKA, [cb, tx(hx(0xC3), "registration")]),
        block(4, OLD, [cb, tx(hx(0xC4), "registration"), tx(hx(0xC5), "transfer")]),
    ]
    d = write_dir(os.path.join(tmp, "at"), blocks)
    bare = write_dir(os.path.join(tmp, "atb"), blocks[:3] + [block(4, OLD, [cb, hx(0xC4)])])
    expect("attrib-old-built", ["attrib", d, OLD, 1, 4], 0,
           lambda v: None if v.get("old_built") == [2, 4] and v.get("old_built_producer_txs") == 3 else "want old_built [2, 4], 3 producer txs")
    expect("attrib-range-missing", ["attrib", d, OLD, 1, 5], 1,
           lambda v: None if 5 in v.get("missing", []) else "want missing [5]")
    expect("attrib-bare-string-tx", ["attrib", bare, OLD, 1, 4], 2, lambda v: None)
    expect("attrib-nonhex-pk", ["attrib", d, "xyz", 1, 4], 2, lambda v: None)


# --- forkcheck -------------------------------------------------------------------------
def c_forkcheck(tmp):
    final = write_dir(os.path.join(tmp, "final"), [block(h, PKA if h % 2 else PKB, []) for h in range(1, 7)])
    gap = write_dir(os.path.join(tmp, "gap"), [block(h, PKA, []) for h in (1, 2, 4)])

    def fh(h):
        return hx(0x1000 + h)

    def samples(name, lines):
        p = os.path.join(tmp, name)
        with open(p, "w") as f:
            f.write("\n".join(lines) + "\n")
        return p

    def t(node, h, hsh=None, p=PKA):
        return "T %d %d %s %s" % (node, h, hsh or fh(h), p)

    stall = [t(n, h) for n in range(3) for h in (1, 2, 3, 4)] + [t(n, 4) for n in range(3) for _ in range(40)]
    stall += ["S 4 10000", "M 0"]
    live = [t(n, h) for n in range(3) for h in (1, 2, 3, 4)] + [t(0, 5), t(1, 5), t(2, 5, hx(0xE5), PKB), t(0, 6), t(1, 6), t(2, 6)]
    healed = [t(n, h) for n in range(3) for h in (1, 2)] + [t(1, 3, hx(0xE3), PKB)] + [t(n, h) for n in range(3) for h in (3, 4, 5, 6)]
    s_stall, s_live, s_healed = samples("stall", stall), samples("live", live), samples("healed", healed)
    expect("forkcheck-stall-is-not-fork", ["forkcheck", final, s_stall], 0,
           lambda v: None if v.get("fork") is False and v.get("first_height") is None and v.get("min_height") == 4 else "a stuck tip with canonical hashes is no fork (min_height 4)")
    expect("forkcheck-live-fork", ["forkcheck", final, s_live], 1,
           lambda v: None if v.get("fork") is True and v.get("first_height") == 5 and [hx(0xE5), PKB] in v.get("sides", []) and [fh(5), PKA] in v.get("sides", []) else "want fork at 5 naming both sides")
    expect("forkcheck-healed-fork", ["forkcheck", final, s_healed], 1,
           lambda v: None if v.get("fork") is True and v.get("first_height") == 3 and [1, 3, hx(0xE3)] in v.get("absent", []) else "want fork at 3, [1, 3, hash] absent")
    split = [samples("n0", [t(0, h) for h in (1, 2, 3, 4)]), samples("n1", [t(1, h) for h in (1, 2, 3, 4)])]
    expect("forkcheck-per-node-files", ["forkcheck", final] + split, 0, lambda v: None if v.get("fork") is False else "two clean files: no fork")
    for name, lines, fdir in [
        ("forkcheck-nondigit-height", [t(0, 1), "T 0 12x %s %s" % (fh(2), PKA)], final),
        ("forkcheck-nonhex-hash", [t(0, 1), "T 0 2 %s %s" % ("g" * 64, PKA)], final),
        ("forkcheck-final-gap", [t(0, 1), t(0, 2)], gap),
    ]:
        expect(name, ["forkcheck", fdir, samples(name, lines)], 2,
               lambda v: None if v.get("fork") is None else "malformed input must leave fork null")


# --- scan (loopback mock of getBlockByHeight / getTransaction) --------------------------
def c_scan(tmp):
    known = {COINB: tx(COINB, "coinbase", outs=[("normal", 200, 0)]), SUBJ: tx(SUBJ, "transfer")}
    blocks = {1: block(1, PKA, [COINB]), 2: block(2, PKB, [COINB, SUBJ, hx(0xDD)])}

    class H(BaseHTTPRequestHandler):
        def do_POST(self):
            req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
            prm = req.get("params") or {}
            if isinstance(prm, list):
                prm = {"height": prm[0], "hash": prm[0]} if prm else {}
            res = None
            if req.get("method") == "getBlockByHeight":
                res = blocks.get(int(prm.get("height", -1)))
            elif req.get("method") == "getTransaction":
                res = known.get(prm.get("hash"))
            body = {"jsonrpc": "2.0", "id": req.get("id", 1)}
            body.update({"result": res} if res is not None else {"error": {"code": -32000, "message": "not found"}})
            data = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *a):
            pass

    srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
    th = threading.Thread(target=srv.serve_forever, daemon=True)
    th.start()
    port = srv.server_address[1]
    try:
        out = os.path.join(tmp, "scan")
        os.makedirs(out)

        def files_ok(v):
            if v.get("missing") != [3] or v.get("written") != 2:
                return "want written 2, missing [3]"
            try:
                b2 = json.load(open(os.path.join(out, "2.json")))["result"]
            except (OSError, ValueError, KeyError):
                return "2.json not written as {\"result\": block}"
            txs = b2.get("transactions", [])
            if [t.get("txType") if isinstance(t, dict) else t for t in txs] != ["coinbase", "transfer", hx(0xDD)]:
                return "transactions not expanded (unknown hash kept raw): %r" % txs
            return None if not os.path.exists(os.path.join(out, "3.json")) else "3.json written for an unserved height"

        expect("scan-missing-height", ["scan", out, port, 1, 3], 1, files_ok)
        out2 = os.path.join(tmp, "scan2")
        os.makedirs(out2)
        expect("scan-complete", ["scan", out2, port, 1, 2], 0,
               lambda v: None if v.get("missing") == [] and sorted(os.listdir(out2)) == ["1.json", "2.json"] else "want both heights written, none missing")
    finally:
        srv.shutdown()
        srv.server_close()


def main():
    tmp = tempfile.mkdtemp(prefix="t-devnet-py.")
    try:
        for c in (c_removed, c_norm, c_history, c_attrib, c_forkcheck, c_scan):
            sub = os.path.join(tmp, c.__name__)
            os.makedirs(sub)
            c(sub)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("%d passed, %d failed" % (NP, NF))
    return 0 if NF == 0 else 1


# pytest entry points: `pytest scripts/test_gauntlet_devnet_py.py` runs this suite and the GS-027 offline
# suite (covers: scripts/gauntlet-devnet.py scripts/gauntlet-devnet-lib.sh scripts/gauntlet-gs027.sh).
def test_gauntlet_devnet_py_suite():
    assert main() == 0


def test_gauntlet_gs027_offline_suite():
    p = subprocess.run(["bash", os.path.join(ROOT, "scripts", "test_gauntlet_gs027.sh")],
                       capture_output=True, text=True, timeout=120)
    print(p.stdout)
    assert p.returncode == 0


# covers: scripts/gauntlet-gs028.sh (offline shape suite scripts/test_gauntlet_gs028.sh)
def test_gauntlet_gs028_offline_suite():
    p = subprocess.run(["bash", os.path.join(ROOT, "scripts", "test_gauntlet_gs028.sh")],
                       capture_output=True, text=True, timeout=120)
    print(p.stdout)
    assert p.returncode == 0, "test_gauntlet_gs028.sh failed"


# covers: scripts/gauntlet-gs029.sh scripts/gauntlet-gs029-probes.sh (offline shape suite
# scripts/test_gauntlet_gs029.sh)
def test_gauntlet_gs029_offline_suite():
    p = subprocess.run(["bash", os.path.join(ROOT, "scripts", "test_gauntlet_gs029.sh")],
                       capture_output=True, text=True, timeout=120)
    print(p.stdout)
    assert p.returncode == 0, "test_gauntlet_gs029.sh failed"


if __name__ == "__main__":
    sys.exit(main())
