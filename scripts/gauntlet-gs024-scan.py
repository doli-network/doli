#!/usr/bin/env python3
"""GS-024 block-history scanner (INC-I-235): getBlockByHeight for every height LO..HI.

usage: gauntlet-gs024-scan.py URL LO HI [--compare URL2] [--threads N] [--timeout S] [--tries N]

Prints one line:
  scan LO..HI checked=N missing=M ranges=R mismatched=K mismatch_ranges=R2 errors=E error_ranges=R3 seconds=T
missing    URL answered "not found" for the height
mismatched URL has the block, URL2 has none or another hash
errors     URL or URL2 never answered within the tries (unknown, never counted as present)
Exit 0 iff missing = mismatched = errors = 0; 1 otherwise; 2 on a usage error.
"""
import argparse
import http.client
import json
import sys
import threading
import time
import urllib.parse

ERROR_RUN_LIMIT = 20


def ranges(heights):
    out = []
    for h in sorted(set(heights)):
        if out and out[-1][1] == h - 1:
            out[-1][1] = h
        else:
            out.append([h, h])
    return ",".join("%d..%d" % (a, b) for a, b in out) or "none"


class Node:
    def __init__(self, url, timeout, tries):
        u = urllib.parse.urlsplit(url)
        if u.scheme != "http" or not u.hostname:
            raise ValueError("URL must be http://HOST:PORT, got %r" % url)
        self.host, self.port, self.path = u.hostname, u.port or 80, u.path or "/"
        self.timeout, self.tries, self.local = timeout, tries, threading.local()

    def _drop(self):
        conn = getattr(self.local, "conn", None)
        if conn is not None:
            conn.close()
        self.local.conn = None

    def block_hash(self, h):
        """Hash at height h; "" when the node answers "not found"; None when it never answers."""
        body = json.dumps({"jsonrpc": "2.0", "id": h, "method": "getBlockByHeight", "params": {"height": h}})
        for attempt in range(self.tries):
            try:
                conn = getattr(self.local, "conn", None)
                if conn is None:
                    conn = self.local.conn = http.client.HTTPConnection(self.host, self.port, timeout=self.timeout)
                conn.request("POST", self.path, body, {"Content-Type": "application/json"})
                reply = json.loads(conn.getresponse().read())
                result = reply.get("result")
                if isinstance(result, dict) and result.get("hash"):
                    return result["hash"]
                if "not found" in str((reply.get("error") or {}).get("message", "")).lower():
                    return ""
            except (OSError, http.client.HTTPException, ValueError):
                self._drop()
            if attempt + 1 < self.tries:
                time.sleep(0.5)
        return None


def scan_slice(primary, verifier, heights, out, lock):
    missing, mismatched, errors, run = [], [], [], 0
    for i, h in enumerate(heights):
        if run >= ERROR_RUN_LIMIT:
            errors.extend(heights[i:])
            break
        a = primary.block_hash(h)
        if a is None:
            errors.append(h)
            run += 1
            continue
        run = 0
        if not a:
            missing.append(h)
            continue
        if verifier is not None:
            b = verifier.block_hash(h)
            if b is None:
                errors.append(h)
            elif b != a:
                mismatched.append(h)
    with lock:
        out["missing"] += missing
        out["mismatched"] += mismatched
        out["errors"] += errors


def main(argv):
    ap = argparse.ArgumentParser(prog="gauntlet-gs024-scan.py", add_help=True)
    ap.add_argument("url")
    ap.add_argument("lo", type=int)
    ap.add_argument("hi", type=int)
    ap.add_argument("--compare", default=None)
    ap.add_argument("--threads", type=int, default=16)
    ap.add_argument("--timeout", type=float, default=30.0)
    ap.add_argument("--tries", type=int, default=3)
    try:
        args = ap.parse_args(argv)
        if args.lo < 1 or args.hi < args.lo or args.threads < 1 or args.tries < 1 or args.timeout <= 0:
            raise ValueError("need 1 <= LO <= HI, threads >= 1, tries >= 1, timeout > 0")
        primary = Node(args.url, args.timeout, args.tries)
        verifier = Node(args.compare, args.timeout, args.tries) if args.compare else None
    except SystemExit:
        return 2
    except ValueError as e:
        print("usage error: %s" % e, file=sys.stderr)
        return 2
    t0 = time.time()
    heights = list(range(args.lo, args.hi + 1))
    n = min(args.threads, len(heights))
    out, lock = {"missing": [], "mismatched": [], "errors": []}, threading.Lock()
    workers = [threading.Thread(target=scan_slice, args=(primary, verifier, heights[i::n], out, lock)) for i in range(n)]
    for w in workers:
        w.start()
    for w in workers:
        w.join()
    print("scan %d..%d checked=%d missing=%d ranges=%s mismatched=%d mismatch_ranges=%s errors=%d error_ranges=%s seconds=%d" % (
        args.lo, args.hi, len(heights), len(out["missing"]), ranges(out["missing"]), len(out["mismatched"]),
        ranges(out["mismatched"]), len(out["errors"]), ranges(out["errors"]), int(time.time() - t0)))
    return 0 if not (out["missing"] or out["mismatched"] or out["errors"]) else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
