#!/usr/bin/env python3
"""GS-063 hostile backfill peer (INC-I-308 R-2). Stdlib only; run with python3 -I.

Forwards JSON-RPC to an HONEST node, except:
  getBlockByHeight {"height": H}  -> the VICTIM's own hash at H (the tip preflight always agrees)
  getBlockRaw at the target height -> tampered, per the mode in the config file:
    swap         the honest block from height target-SWAP (a real block, wrong height)
    flip_header  the honest block with one prev_hash byte flipped (header hash changes)
    flip_tail    the honest block with its last byte flipped (body altered, header kept)
  getBlockRaw below `floor` -> "no block" error, so every hostile pass ends fast.
The config file (JSON: mode, target, floor) is re-read per request; the runner rewrites it per pass.
Usage: gauntlet-gs063-proxy.py PORT HONEST_URL VICTIM_URL CONFIG_JSON LOG
"""
import base64
import json
import sys
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SWAP = 5
PORT, HONEST, VICTIM, CONFIG, LOG = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]


def post(url, body):
    req = urllib.request.Request(url, data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.loads(r.read())


def log(line):
    with open(LOG, "a") as f:
        f.write(line + "\n")


def raw_at(h):
    return post(HONEST, {"jsonrpc": "2.0", "id": 1, "method": "getBlockRaw", "params": {"height": h}})


def tampered(mode, h):
    if mode == "swap":
        src = raw_at(h - SWAP)["result"]
        log(f"TAMPER swap h={h} src={h - SWAP}")
        return {"block": src["block"], "blake3": src.get("blake3", ""), "height": h}
    data = bytearray(base64.b64decode(raw_at(h)["result"]["block"]))
    pos = 10 if mode == "flip_header" else len(data) - 1
    data[pos] ^= 0x01
    log(f"TAMPER {mode} h={h} byte={pos}")
    return {"block": base64.b64encode(bytes(data)).decode(), "blake3": "", "height": h}


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, obj):
        out = json.dumps(obj).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        rid, m, p = req.get("id"), req.get("method"), req.get("params")
        with open(CONFIG) as f:
            cfg = json.load(f)
        try:
            if m == "getBlockByHeight" and isinstance(p, dict):
                v = post(VICTIM, req)
                log(f"PREFLIGHT h={p.get('height')} -> victim hash")
                return self.reply({"jsonrpc": "2.0", "id": rid, "result": v.get("result")})
            if m == "getBlockRaw" and isinstance(p, dict):
                h = int(p.get("height", 0))
                if h < cfg["floor"]:
                    log(f"FLOOR h={h}")
                    return self.reply({"jsonrpc": "2.0", "id": rid,
                                       "error": {"code": -32000, "message": "no block"}})
                if h == cfg["target"] and cfg["mode"] != "none":
                    return self.reply({"jsonrpc": "2.0", "id": rid, "result": tampered(cfg["mode"], h)})
            fwd = {k: v for k, v in req.items()}
            return self.reply(post(HONEST, fwd))
        except Exception as e:  # noqa: BLE001 — a proxy fault is an error reply, never a crash
            log(f"PROXY_ERROR {m}: {e}")
            return self.reply({"jsonrpc": "2.0", "id": rid, "error": {"code": -32603, "message": str(e)}})


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
