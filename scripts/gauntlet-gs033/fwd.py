#!/usr/bin/env python3
"""Userspace TCP forwarder with a partition switch (GS-033 gauntlet runner, INC-I-281).

The local testnet has no packet filter we may touch, so a minority node is cut off in userspace:
its P2P listener moves to an internal port and this forwarder takes its public port and one port
per majority peer. Holding a link keeps every TCP connection open but moves no bytes.

Usage: python3 -I fwd.py --log FILE --ctl FILE --mode hold|drop --map LPORT:TPORT:A-B [--map ...]
Every link A-B is "node B dials LPORT, which forwards to node A's P2P port TPORT".
All sockets bind/connect on 127.0.0.1 only.

Control by signal (send only to this process's own PID):
  SIGUSR1 -> PARTITION: read --ctl (node indices separated by spaces = the minority);
             every link with exactly one endpoint in the minority is cut.
     mode=hold: keep the TCP connection open but move no bytes either way.
     mode=drop: abort the connection and refuse new ones on that link.
  SIGUSR2 -> HEAL every link.
  SIGTERM -> exit.
"""
import argparse
import asyncio
import signal
import time

ST = {"log": None, "mode": "hold", "links": {}}


def log(msg):
    t = time.time()
    stamp = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t)) + ".%03dZ" % (int(t * 1000) % 1000)
    ST["log"].write("%s %s\n" % (stamp, msg))
    ST["log"].flush()


class Link:
    def __init__(self, lport, tport, a, b):
        self.lport, self.tport, self.a, self.b = lport, tport, a, b
        self.cut = False
        self.ok = asyncio.Event()
        self.ok.set()
        self.conns = set()
        self.bytes = 0

    @property
    def tag(self):
        return "%d-%d" % (self.a, self.b)


async def pipe(link, reader, writer):
    try:
        while True:
            if link.cut:
                if ST["mode"] == "drop":
                    break
                await link.ok.wait()
            data = await reader.read(65536)
            if not data:
                break
            if link.cut:
                if ST["mode"] == "drop":
                    break
                await link.ok.wait()
            writer.write(data)
            link.bytes += len(data)
            await writer.drain()
    except Exception:
        pass
    finally:
        try:
            writer.close()
        except Exception:
            pass


def make_handler(link):
    async def handle(creader, cwriter):
        peer = cwriter.get_extra_info("peername")
        if link.cut and ST["mode"] == "drop":
            log("REFUSE link=%s from=%s" % (link.tag, peer))
            cwriter.close()
            return
        try:
            treader, twriter = await asyncio.open_connection("127.0.0.1", link.tport)
        except Exception as e:
            log("CONNECT_FAIL link=%s %s" % (link.tag, e))
            cwriter.close()
            return
        pair = (cwriter, twriter)
        link.conns.add(pair)
        log("OPEN link=%s from=%s live=%d" % (link.tag, peer, len(link.conns)))
        await asyncio.gather(pipe(link, creader, twriter), pipe(link, treader, cwriter))
        link.conns.discard(pair)
        log("CLOSE link=%s live=%d" % (link.tag, len(link.conns)))

    return handle


def partition(ctl):
    try:
        with open(ctl) as f:
            minority = {int(x) for x in f.read().split() if x.isdigit()}
    except OSError as e:
        log("PARTITION_CTL_ERROR %s" % e)
        return
    cut = []
    for link in ST["links"].values():
        if (link.a in minority) != (link.b in minority):
            link.cut = True
            link.ok.clear()
            cut.append("%s(live=%d)" % (link.tag, len(link.conns)))
            if ST["mode"] == "drop":
                for (x, y) in list(link.conns):
                    for w in (x, y):
                        try:
                            w.transport.abort()
                        except Exception:
                            pass
    log("PARTITION mode=%s minority=%s cut=%s" % (ST["mode"], sorted(minority), " ".join(cut)))


def heal():
    for link in ST["links"].values():
        link.cut = False
        link.ok.set()
    log("HEAL live=%s" % " ".join("%s:%d" % (l.tag, len(l.conns)) for l in ST["links"].values()))


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--log", required=True)
    ap.add_argument("--ctl", required=True)
    ap.add_argument("--mode", choices=["hold", "drop"], default="hold")
    ap.add_argument("--map", action="append", required=True)
    a = ap.parse_args()
    ST["log"] = open(a.log, "a")
    ST["mode"] = a.mode
    loop = asyncio.get_running_loop()
    loop.add_signal_handler(signal.SIGUSR1, lambda: partition(a.ctl))
    loop.add_signal_handler(signal.SIGUSR2, heal)
    stop = loop.create_future()
    loop.add_signal_handler(signal.SIGTERM, lambda: stop.done() or stop.set_result(1))
    for m in a.map:
        lp, tp, tag = m.split(":")
        x, y = (int(v) for v in tag.split("-"))
        link = Link(int(lp), int(tp), x, y)
        ST["links"][link.tag] = link
        await asyncio.start_server(make_handler(link), "127.0.0.1", link.lport)
        log("LISTEN 127.0.0.1:%d -> 127.0.0.1:%d link=%s mode=%s" % (link.lport, link.tport, link.tag, a.mode))
    await stop
    log("EXIT")


if __name__ == "__main__":
    asyncio.run(main())
