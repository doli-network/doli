#!/usr/bin/env python3
"""tn.py — local-testnet model for the GS-033 gauntlet runner (INC-I-281).

Everything is derived at runtime: the nodes from their launchd plists (binary, data dir, ports,
log file, producer key), the producer public keys from the key files those plists name, the slot
length, epoch length and genesis time from RPC. Nothing names a host, a LAN address or a user path.

LOCAL testnet only: every RPC and P2P socket is 127.0.0.1, and init() refuses unless
getNetworkParams.network == "testnet".
SAFETY: a launchd node stops and starts only through scripts/testnet.sh with ONE target. Signals go
only to PIDs this package spawned, or (for a node launchd does not track) to the single process on
that node's data dir. Every peers.cache set aside is restored. state.json records what is to undo.
"""
import hashlib
import json
import os
import plistlib
import re
import signal
import subprocess
import sys
import time
import urllib.request

HOME = os.path.expanduser("~")
TN = os.environ.get("TESTNET_HOME") or os.path.join(HOME, "testnet")
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
TESTNET_SH = os.path.join(ROOT, "scripts", "testnet.sh")
AGENTS = os.path.join(HOME, "Library", "LaunchAgents")
LABEL = "network.doli.testnet-%s"
INTERNAL_BASE = int(os.environ.get("GS033_INTERNAL_BASE", "31300"))
FWD_BASE = int(os.environ.get("GS033_FWD_BASE", "32000"))
ANSI = re.compile(r"\x1b\[[0-9;]*m")

NODES = []      # index -> node dict (index 0 = the first name in GS033_NODES, the seed)
BY_NAME = {}
NET = {}        # slot, epoch, genesis
OUT = None
LOGF = None
ST = {"procs": {}, "moved_cache": {}, "launchd_stopped": [], "stop": False, "partitioned": False}


def ts():
    t = time.time()
    return time.strftime("%H:%M:%S", time.gmtime(t)) + ".%03dZ" % (int(t * 1000) % 1000)


def say(msg):
    """Progress goes to stderr and the run log; stdout is the runner's PASS/FAIL contract."""
    line = "[%s] %s" % (ts(), msg)
    print(line, file=sys.stderr, flush=True)
    if LOGF:
        LOGF.write(line + "\n")
        LOGF.flush()


def clean(s, n=300):
    return ANSI.sub("", s)[:n]


def _opt(args, name):
    return args[args.index(name) + 1] if name in args else None


def load_nodes():
    """Build the node table from the launchd plists named by GS033_NODES (read-only)."""
    names = os.environ.get("GS033_NODES", "seed n1 n2 n3 n4 n5 n6").split()
    if len(names) > 10:
        raise SystemExit("GS033_NODES: at most 10 nodes (forwarder port layout)")
    NODES.clear()
    BY_NAME.clear()
    for i, nm in enumerate(names):
        path = os.path.join(AGENTS, (LABEL % nm) + ".plist")
        with open(path, "rb") as f:
            pl = plistlib.load(f)
        args = list(pl["ProgramArguments"])
        key = _opt(args, "--producer-key")
        pk = None
        if key and os.path.exists(key):
            with open(key) as f:
                pk = json.load(f)["addresses"][0]["public_key"]
        n = {"i": i, "name": nm, "args": args, "bin": args[0], "datadir": _opt(args, "--data-dir"),
             "p2p": int(_opt(args, "--p2p-port")), "rpc": int(_opt(args, "--rpc-port")),
             "met": int(_opt(args, "--metrics-port") or 0), "log": pl.get("StandardOutPath"),
             "key": key, "pk": pk, "producer": "--producer" in args}
        NODES.append(n)
        BY_NAME[nm] = n
    return NODES


def idx(name):
    name = name.strip()
    if name not in BY_NAME:
        raise SystemExit("unknown node %r (GS033_NODES = %s)" % (name, " ".join(BY_NAME)))
    return BY_NAME[name]["i"]


def name(i):
    return NODES[i]["name"]


def internal(i):
    return INTERNAL_BASE + i


def fwd_port(m, j):
    return FWD_BASE + 10 * m + j


# ---------------------------------------------------------------------------------- RPC
def rpc(i, method, params=None, timeout=4):
    body = json.dumps({"jsonrpc": "2.0", "method": method, "params": params or {}, "id": 1}).encode()
    req = urllib.request.Request("http://127.0.0.1:%d" % NODES[i]["rpc"], body,
                                 {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        d = json.loads(r.read())
    if d.get("error"):
        raise RuntimeError(str(d["error"])[:200])
    return d["result"]


def tip(i):
    try:
        r = rpc(i, "getChainInfo", timeout=3)
        return int(r["bestHeight"]), r["bestHash"], int(r.get("bestSlot") or 0)
    except Exception:
        return None


def block(i, h):
    try:
        return rpc(i, "getBlockByHeight", {"height": int(h)})
    except Exception:
        return None


def bhash(i, h):
    b = block(i, h)
    return b["hash"] if b else None


def pname(b):
    """Node name whose producer key built block b ('?' if unknown)."""
    p = (b or {}).get("producer") or ""
    for n in NODES:
        if n["pk"] and p and (n["pk"].startswith(p) or p.startswith(n["pk"][:16])):
            return n["name"]
    return "?"


def pnode(b):
    nm = pname(b)
    return BY_NAME[nm]["i"] if nm in BY_NAME else None


def init(need_up=True):
    load_nodes()
    p = rpc(0, "getNetworkParams")
    if p.get("network") != "testnet":
        raise SystemExit("refusing: %s answers network=%r, not testnet" % (name(0), p.get("network")))
    NET.update({"slot": int(p["slotDuration"]), "epoch": int(p["blocksPerRewardEpoch"]),
                "genesis": int(p["genesisTime"])})
    if need_up:
        down = [n["name"] for n in NODES if tip(n["i"]) is None]
        if down:
            raise SystemExit("refusing: node(s) down: %s" % " ".join(down))
    return NET


def slot_start(s):
    return NET["genesis"] + s * NET["slot"]


def cur_slot():
    return (int(time.time()) - NET["genesis"]) // NET["slot"]


# ---------------------------------------------------------------------------------- processes
def save_state():
    if not OUT:
        return
    st = {"procs": {k: p.pid for k, p in ST["procs"].items() if p.poll() is None},
          "moved_cache": {str(k): v for k, v in ST["moved_cache"].items()},
          "launchd_stopped": ST["launchd_stopped"]}
    with open(os.path.join(OUT, "state.json"), "w") as f:
        json.dump(st, f)


def launchd_pid(i):
    out = subprocess.run(["launchctl", "list"], capture_output=True, text=True).stdout
    for ln in out.splitlines():
        p = ln.split()
        if len(p) == 3 and p[2] == LABEL % name(i):
            return None if p[0] == "-" else int(p[0])
    return None


def datadir_pids(i):
    """PIDs of any process whose cmdline uses this node's data dir (read-only scan)."""
    dd = "--data-dir %s " % NODES[i]["datadir"]
    out = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    return [int(ln.split(None, 1)[0]) for ln in out.splitlines() if dd in ln + " "]


def testnet_sh(action, i):
    r = subprocess.run(["bash", TESTNET_SH, action, name(i)], capture_output=True, text=True)
    say("testnet.sh %s %s -> rc=%d" % (action, name(i), r.returncode))
    return r.returncode


def wait_gone(i, secs=40):
    end = time.time() + secs
    while time.time() < end:
        if not datadir_pids(i):
            return True
        time.sleep(1)
    return False


def wait_rpc(i, secs=90):
    end = time.time() + secs
    while time.time() < end:
        if tip(i):
            return True
        time.sleep(2)
    return False


def stop_launchd(i):
    if i == 0 and os.environ.get("GS033_ALLOW_SEED") != "1":
        raise RuntimeError("refusing to stop the seed (slow RPC after restart); set GS033_ALLOW_SEED=1")
    if launchd_pid(i) or not datadir_pids(i):
        testnet_sh("stop", i)
    else:
        pids = datadir_pids(i)
        if len(pids) != 1:
            raise RuntimeError("%s: %d processes on its data dir" % (name(i), len(pids)))
        say("%s is not launchd-tracked; SIGTERM its single data-dir pid %d" % (name(i), pids[0]))
        os.kill(pids[0], signal.SIGTERM)
    if i not in ST["launchd_stopped"]:
        ST["launchd_stopped"].append(i)
    save_state()
    if not wait_gone(i, 60):
        raise RuntimeError("%s still running after stop: %s" % (name(i), datadir_pids(i)))
    say("%s stopped (no process on its data dir)" % name(i))


def start_launchd(i):
    if datadir_pids(i):
        say("WARN %s data dir still in use by %s; not starting its launchd job" % (name(i), datadir_pids(i)))
        return False
    testnet_sh("start", i)
    ok = wait_rpc(i, 150)
    say("%s launchd pid=%s rpc=%s" % (name(i), launchd_pid(i), "up" if ok else "DOWN"))
    if i in ST["launchd_stopped"]:
        ST["launchd_stopped"].remove(i)
    save_state()
    return ok


def _cache(i):
    return os.path.join(NODES[i]["datadir"], "peers.cache")


def move_cache(i):
    """First call: move the original peers.cache aside. Later calls: set aside the scratch cache
    the node wrote during this run (the original stays aside until restore_cache)."""
    p, tag = _cache(i), os.path.basename(OUT)
    if i in ST["moved_cache"]:
        if os.path.exists(p):
            os.rename(p, p + ".scratch-gs033-%s-%d" % (tag, int(time.time())))
        return
    bak = p + ".pre-gs033-%s" % tag
    if os.path.exists(p):
        os.rename(p, bak)
        ST["moved_cache"][i] = bak
    else:
        ST["moved_cache"][i] = None
    save_state()
    say("%s peers.cache moved aside -> %s" % (name(i), ST["moved_cache"][i]))


def restore_cache(i):
    p, tag = _cache(i), os.path.basename(OUT)
    bak = ST["moved_cache"].pop(i, None)
    save_state()
    d = os.path.dirname(p)
    for f in os.listdir(d):
        if f.startswith("peers.cache.scratch-gs033-%s-" % tag):
            os.remove(os.path.join(d, f))
    if bak is None:
        return
    if os.path.exists(p):
        os.remove(p)  # scratch cache written during this run
    os.rename(bak, p)
    say("%s peers.cache restored" % name(i))


def spawn(tag, argv, logname):
    env = {"HOME": HOME, "PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "USER": os.environ.get("USER", ""),
           "LOGNAME": os.environ.get("USER", ""), "TMPDIR": os.environ.get("TMPDIR", "/tmp")}
    lf = open(os.path.join(OUT, logname), "ab")
    p = subprocess.Popen(argv, stdout=lf, stderr=subprocess.STDOUT, cwd="/", env=env, start_new_session=True)
    ST["procs"][tag] = p
    save_state()
    say("spawned %s pid=%d log=%s args=%s" % (tag, p.pid, logname, " ".join(argv[1:])[:400]))
    return p


def kill(tag, secs=40):
    p = ST["procs"].pop(tag, None)
    save_state()
    if not p or p.poll() is not None:
        return
    p.send_signal(signal.SIGTERM)
    try:
        p.wait(secs)
    except subprocess.TimeoutExpired:
        say("WARN %s pid=%d did not exit in %ds; SIGKILL by its own PID" % (tag, p.pid, secs))
        p.kill()
        p.wait(10)
    say("stopped %s pid=%d rc=%s" % (tag, p.pid, p.returncode))


def restore_all():
    """Undo every change of an injection: harness nodes, forwarders, peers.cache, launchd jobs."""
    ST["stop"] = True
    for tag in sorted(ST["procs"]):
        if not tag.startswith("fwd"):
            kill(tag)
    for tag in sorted(ST["procs"]):
        kill(tag, 10)
    for i in list(ST["moved_cache"]):
        restore_cache(i)
    for i in list(ST["launchd_stopped"]):
        start_launchd(i)
    say("restore: launchd pids " + " ".join("%s=%s" % (n["name"], launchd_pid(n["i"])) for n in NODES))


def manual_restore(out):
    """Recovery when the injection process died without its finally block (reads state.json)."""
    global OUT
    OUT = out
    sp = os.path.join(out, "state.json")
    if not os.path.exists(sp):
        say("no state.json in %s: the injection changed nothing yet" % out)
        return
    with open(sp) as f:
        st = json.load(f)
    ps = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    live = {int(ln.split(None, 1)[0]): ln for ln in ps.splitlines()}
    for tag, pid in st["procs"].items():
        cmd = live.get(pid, "")
        if pid in live and ("fwd.py" in cmd or any(n["datadir"] in cmd for n in NODES)):
            os.kill(pid, signal.SIGTERM)
            say("SIGTERM %s pid=%d" % (tag, pid))
    time.sleep(15)
    for k, v in st["moved_cache"].items():
        ST["moved_cache"][int(k)] = v
    ST["launchd_stopped"] = list(st["launchd_stopped"])
    restore_all()


# ---------------------------------------------------------------------------------- binaries
def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def version(path):
    try:
        return subprocess.run([path, "--version"], capture_output=True, text=True, timeout=20).stdout.strip()
    except Exception as e:
        return "unknown (%s)" % e


def parse_binmap(spec=None):
    """TN_BINMAP="n4=/path/doli-node,n5=/path/..." -> {index: source path}."""
    spec = (os.environ.get("TN_BINMAP", "") if spec is None else spec).strip()
    out = {}
    for item in filter(None, (x.strip() for x in spec.split(","))):
        nm, path = item.split("=", 1)
        path = os.path.abspath(os.path.expanduser(path.strip()))
        if not os.path.isfile(path):
            raise SystemExit("TN_BINMAP: %s is not a file" % path)
        out[idx(nm)] = path
    return out


def effective_binaries(binmap):
    """Per node: the source binary it runs during an injection, its version and sha256."""
    rows = {}
    for n in NODES:
        src = binmap.get(n["i"], n["bin"])
        rows[n["name"]] = {"i": n["i"], "path": src, "mapped": n["i"] in binmap, "version": version(src),
                           "sha256": sha256(src)}
    return rows
