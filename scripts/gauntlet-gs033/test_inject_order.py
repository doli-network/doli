#!/usr/bin/env python3
"""Offline tests for the GS-033 injector: restart order, peers.cache set-aside and its restore.

Fakes replace every launchd, process, RPC, signal and clock call; peers.cache files live in a
temp dir. Never starts a node and never touches the testnet.
Run: python3 -B -m unittest scripts/gauntlet-gs033/test_inject_order.py -v

OUTPUT CONTRACT
  inject.restart_plan(M, X, mapped) -> (actions, set_aside): actions = [(kind, node)] in start
    order (mapped | minority | x-follower | x-producer); set_aside = each node of actions once.
  inject.run setup: node starts up to the setup isolation check; each node starts with no
    peers.cache; state.json moved_cache = the nodes set aside; after the abort handler's restore
    or tn.manual_restore(RUN_DIR) every peers.cache equals its original (absent if it was absent).
  probes.lsof_check: one `isolation[<label>]: ...` line per check, ending in `ok` or `FAIL ...`.
INPUT PARTITIONS: map = minority only | n1,n2,n3,n6 with X=n3 mapped | all 7 (seed allowed) ·
  original cache present | absent (n6) · restore by the abort handler | from state.json after the
  runner died · isolation clean | inbound leak | outbound leak | minority pair missing.
"""
import contextlib
import json
import os
import shutil
import signal
import sys
import tempfile
import time
import types
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import inject  # noqa: E402
import plan  # noqa: E402
import probes  # noqa: E402
import tn  # noqa: E402

NAMES = ["seed"] + ["n%d" % i for i in range(1, 7)]
SEED, N1, N2, N3, N4, N5, N6 = range(7)
M, X = [N4, N5], N3
DEFAULT = [N4, N5]
ROW425 = [N1, N2, N3, N6]
ALL7 = list(range(7))
LEAK = "/ip4/127.0.0.1/tcp/31305"


def read(path):
    try:
        with open(path) as f:
            return f.read()
    except OSError:
        return None


def write(path, text):
    with open(path, "w") as f:
        f.write(text)


class Plan(unittest.TestCase):
    def test_default_layout(self):
        acts, aside = inject.restart_plan(M, X, DEFAULT)
        self.assertEqual(acts, [("minority", N4), ("minority", N5), ("x-follower", N3), ("x-producer", N3)])
        self.assertEqual(aside, [N4, N5, N3])

    def test_row425_layout(self):
        acts, aside = inject.restart_plan(M, X, ROW425)
        self.assertEqual(acts, [("mapped", N1), ("mapped", N2), ("mapped", N6), ("minority", N4),
                                ("minority", N5), ("x-follower", N3), ("x-producer", N3)])
        self.assertEqual(aside, [N1, N2, N6, N4, N5, N3])

    def test_set_aside_is_every_restarted_node_once(self):
        for mapped in ([], DEFAULT, ROW425, ALL7, [N2]):
            with self.subTest(mapped=mapped):
                acts, aside = inject.restart_plan(M, X, mapped)
                self.assertEqual(set(aside), set(M) | {X} | set(mapped))
                self.assertEqual(len(aside), len(set(aside)))
                self.assertEqual(set(aside), {i for _, i in acts})

    def test_every_mapped_respawn_precedes_the_first_minority_fronting(self):
        for mapped in ([], DEFAULT, ROW425, ALL7, [N2]):
            with self.subTest(mapped=mapped):
                kinds = [k for k, _ in inject.restart_plan(M, X, mapped)[0]]
                first_m = kinds.index("minority")
                self.assertNotIn("mapped", kinds[first_m:], kinds)
                self.assertEqual(kinds[-2:], ["x-follower", "x-producer"], kinds)

    def test_mapped_seed_restarts_last_of_the_majority(self):
        acts, _ = inject.restart_plan(M, X, ALL7)
        self.assertEqual([i for k, i in acts if k == "mapped"], [N1, N2, N6, SEED])


class Harness(unittest.TestCase):
    """inject.run up to the setup isolation check, with fakes and temp data dirs."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="gs033-test-")
        self.out = os.path.join(self.tmp, "len2", "k0")
        os.makedirs(self.out)
        self.nodes = []
        for i, nm in enumerate(NAMES):
            dd = os.path.join(self.tmp, nm, "data")
            os.makedirs(dd)
            if nm != "n6":
                write(os.path.join(dd, "peers.cache"), "orig-%s" % nm)
            argv = ["/bin/doli-node", "--data-dir", dd, "run", "--producer", "--producer-key", "/k%d" % i,
                    "--p2p-port", str(30300 + i), "--bootstrap", "/ip4/127.0.0.1/tcp/30300"]
            self.nodes.append({"i": i, "name": nm, "args": argv, "bin": argv[0], "datadir": dd, "p2p": 30300 + i,
                               "rpc": 8500 + i, "met": 9000 + i, "log": os.path.join(self.tmp, "%s.log" % nm),
                               "key": None, "pk": None, "producer": True})
        self.calls, self.at_spawn, self.lines = [], {}, []
        self.stack = contextlib.ExitStack()
        self.addCleanup(shutil.rmtree, self.tmp)
        self.addCleanup(self.stack.close)
        st = {"procs": {}, "moved_cache": {}, "launchd_stopped": [], "stop": False, "partitioned": False}
        sig = types.SimpleNamespace(signal=lambda *a: None, SIGTERM=signal.SIGTERM, SIGINT=signal.SIGINT,
                                    SIGUSR1=signal.SIGUSR1, SIGUSR2=signal.SIGUSR2)
        for obj, attr, val in [
                (tn, "NODES", self.nodes), (tn, "BY_NAME", {n["name"]: n for n in self.nodes}), (tn, "ST", st),
                (tn, "OUT", None), (tn, "LOGF", None), (tn, "say", self.lines.append),
                (tn, "tip", lambda i: (376992, "a" * 64, 1638634)), (tn, "stop_launchd", self.stop),
                (tn, "start_launchd", lambda i: self.calls.append(("launchd-start", i)) or True),
                (tn, "launchd_pid", lambda i: None), (tn, "wait_rpc", lambda i, secs=90: True),
                (tn, "kill", lambda tag, secs=40: self.calls.append(("kill", tag))),
                (inject, "BINCOPY", {}), (inject, "signal", sig),
                (inject, "time", types.SimpleNamespace(time=lambda: 1000.0, sleep=lambda s: None)),
                (inject, "fleet_healthy", lambda: []), (inject, "wait_own_slot", lambda i, order, secs=120: None),
                (inject, "setup_binmap", lambda bm: inject.BINCOPY.update({i: "/m/doli-node" for i in bm})),
                (inject, "spawn_fwd", lambda tag, *a, **k: self.calls.append(("fwd", tag))),
                (inject, "spawn_node", self.spawn),
                (plan, "learn_order", lambda: [N1, N4, N5, N3, N2]), (plan, "fmt_plan", lambda p: ""),
                (plan, "choose_plan", lambda *a: ((True,), 377172, X, 1638698, 377056, {}, {"ok": True})),
                (probes, "lsof_check", self.check)]:
            self.stack.enter_context(mock.patch.object(obj, attr, val))
        self.stack.enter_context(mock.patch.dict(tn.NET, {"slot": 10, "epoch": 36, "genesis": 0}))
        env = {k: v for k, v in os.environ.items() if not k.startswith("GS033_")}
        self.stack.enter_context(mock.patch.dict(os.environ, dict(env, GS033_ALLOW_SEED="1"), clear=True))

    def stop(self, i):
        self.calls.append(("stop", i))
        tn.ST["launchd_stopped"].append(i)
        tn.save_state()

    def spawn(self, tag, i, argv, logname):
        p = os.path.join(self.nodes[i]["datadir"], "peers.cache")
        self.at_spawn[tag] = read(p)
        write(p, "scratch-%s %s" % (tag, LEAK))  # what the node writes while it runs
        self.calls.append(("start", tag))

    def check(self, label, M, tolerate=()):
        self.calls.append(("check", label))
        raise RuntimeError("stop-at-%s" % label)

    def run_setup(self, mapped, died=False):
        with contextlib.ExitStack() as st:
            if died:
                st.enter_context(mock.patch.object(tn, "restore_all", lambda: None))
            r = inject.run(self.out, M, 2, 45, {i: "/src/doli-node" for i in mapped}, {})
            tn.LOGF.close()
        self.assertIn("stop-at-setup", r.get("error", ""), r)
        return [c[1] for c in self.calls if c[0] == "start"]

    def state_moved(self):
        with open(os.path.join(self.out, "state.json")) as f:
            return {int(k) for k in json.load(f)["moved_cache"]}

    def assert_caches_back(self):
        for n in self.nodes:
            files = os.listdir(n["datadir"])
            self.assertEqual([f for f in files if "gs033" in f], [], n["name"])
            want = None if n["name"] == "n6" else "orig-%s" % n["name"]
            self.assertEqual(read(os.path.join(n["datadir"], "peers.cache")), want, n["name"])


class Setup(Harness):
    def test_row425_mapped_majority_starts_before_the_minority(self):
        self.assertEqual(self.run_setup(ROW425), ["e1", "e2", "e6", "m4", "m5"])

    def test_default_layout_starts_only_the_minority(self):
        self.assertEqual(self.run_setup(DEFAULT), ["m4", "m5"])

    def test_mapped_seed_starts_last_of_the_majority(self):
        self.assertEqual(self.run_setup(ALL7), ["e1", "e2", "e6", "e0", "m4", "m5"])

    def test_every_started_node_starts_without_its_peers_cache(self):
        started = self.run_setup(ROW425)
        self.assertEqual({t: self.at_spawn[t] for t in started}, {t: None for t in started})

    def test_abort_handler_restore_puts_every_cache_back(self):
        self.run_setup(ROW425)
        self.assertEqual(self.state_moved(), set())
        self.assert_caches_back()

    def test_state_json_lists_every_set_aside_node_and_manual_restore_puts_them_back(self):
        self.run_setup(ROW425, died=True)
        self.assertEqual(self.state_moved(), {N1, N2, N6, N4, N5})
        _, aside = inject.restart_plan(M, X, ROW425)
        self.assertEqual(self.state_moved(), set(aside) - {X})
        fresh = {"procs": {}, "moved_cache": {}, "launchd_stopped": [], "stop": False, "partitioned": False}
        ps = types.SimpleNamespace(run=lambda *a, **k: types.SimpleNamespace(stdout=""))
        clock = types.SimpleNamespace(time=time.time, sleep=lambda s: None, strftime=time.strftime, gmtime=time.gmtime)
        with mock.patch.object(tn, "ST", fresh), mock.patch.object(tn, "subprocess", ps), \
                mock.patch.object(tn, "time", clock):
            tn.manual_restore(self.out)
        self.assert_caches_back()

    def test_x_starts_without_its_peers_cache_and_is_restored(self):
        tn.OUT = self.out
        inject.x_follower_start(X, M)
        inject.x_isolated_start(X, M)
        self.assertEqual(self.at_spawn, {"xf": None, "x": None})
        self.assertEqual(self.state_moved(), {X})
        tn.restore_all()
        self.assert_caches_back()


class Isolation(unittest.TestCase):
    """probes.lsof_check log line on canned `lsof -Fpn` output."""
    PIDS = {"m4": 4, "m5": 5, "fwd4": 14, "fwd5": 15, "e1": 21}
    BASE = [("fwd4", "127.0.0.1:40001", "127.0.0.1:31304"), ("m4", "127.0.0.1:31304", "127.0.0.1:40001"),
            ("m4", "127.0.0.1:40010", "127.0.0.1:31305"), ("m5", "127.0.0.1:31305", "127.0.0.1:40010")]

    def check(self, socks, label="pre-cut"):
        tmp = tempfile.mkdtemp(prefix="gs033-iso-")
        self.addCleanup(shutil.rmtree, tmp)
        by = {}
        for tag, loc, rem in socks:
            by.setdefault(self.PIDS[tag], []).append("n%s->%s" % (loc, rem))
        text = "".join("p%d\n%s\n" % (p, "\n".join(v)) for p, v in sorted(by.items()))
        procs = {t: types.SimpleNamespace(pid=p, poll=lambda: None) for t, p in self.PIDS.items()}
        nodes = [{"i": i, "name": nm, "rpc": 8500 + i, "met": 9000 + i} for i, nm in enumerate(NAMES)]
        lines = []
        with mock.patch.object(tn, "ST", {"procs": procs}), mock.patch.object(tn, "NODES", nodes), \
                mock.patch.object(tn, "OUT", tmp), mock.patch.object(tn, "say", lines.append), \
                mock.patch.object(tn, "datadir_pids", lambda i: []), \
                mock.patch.object(probes, "subprocess", types.SimpleNamespace(
                    run=lambda *a, **k: types.SimpleNamespace(stdout=text))):
            bad = probes.lsof_check(label, M)
        self.assertEqual(len(lines), 1, lines)
        return bad, lines[0]

    def test_clean_pre_cut_logs_ok(self):
        bad, line = self.check(self.BASE)
        self.assertEqual(bad, [])
        self.assertEqual(line, "isolation[pre-cut]: minority P2P sockets=3 direct-to-minority=0 "
                               "direct-to-majority=0 minority-pair sockets=2 ok")

    def test_majority_dial_into_an_internal_port_is_direct_to_minority(self):
        bad, line = self.check(self.BASE + [("e1", "127.0.0.1:50000", "127.0.0.1:31305"),
                                            ("m5", "127.0.0.1:31305", "127.0.0.1:50000")], "setup")
        self.assertEqual(bad, ["m5 127.0.0.1:31305->127.0.0.1:50000 owner=e1"])
        self.assertEqual(line, "isolation[setup]: minority P2P sockets=4 direct-to-minority=1 direct-to-majority=0 "
                               "minority-pair sockets=2 FAIL m5 127.0.0.1:31305->127.0.0.1:50000 owner=e1")

    def test_minority_dial_out_is_direct_to_majority(self):
        bad, line = self.check(self.BASE + [("m4", "127.0.0.1:40020", "127.0.0.1:30301"),
                                            ("e1", "127.0.0.1:30301", "127.0.0.1:40020")])
        self.assertEqual(bad, ["m4 127.0.0.1:40020->127.0.0.1:30301 owner=e1"])
        self.assertIn(" direct-to-minority=0 direct-to-majority=1 ", line)
        self.assertTrue(line.endswith(" FAIL m4 127.0.0.1:40020->127.0.0.1:30301 owner=e1"), line)

    def test_missing_minority_pair_fails_the_line(self):
        bad, line = self.check(self.BASE[:2])
        self.assertEqual(bad, ["minority nodes not linked to each other"])
        self.assertTrue(line.endswith("minority-pair sockets=0 FAIL minority nodes not linked to each other"), line)


if __name__ == "__main__":
    unittest.main()
