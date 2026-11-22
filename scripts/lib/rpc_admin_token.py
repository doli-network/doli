"""Admin RPC token lookup (INC-I-308): env DOLI_RPC_ADMIN_TOKEN, then <data_dir>/rpc-admin.token."""
import os
import re
import subprocess
import sys

MIN_ADMIN_TOKEN_LEN = 32

ADMIN_METHODS = frozenset((
    "pauseProduction", "resumeProduction", "createCheckpoint", "pruneBlocks", "backfillFromPeer",
    "enterRecoveryMode", "exitRecoveryMode", "bridgeFromArchive", "getUtxoDiff", "getStateSnapshot",
    "getStateRootDebug", "verifyChainIntegrity", "repairArchiveFromPeer", "forceReorgTo"))


def env_token():
    """Trimmed DOLI_RPC_ADMIN_TOKEN when it has >= MIN_ADMIN_TOKEN_LEN chars, else None."""
    tok = os.environ.get("DOLI_RPC_ADMIN_TOKEN", "").strip()
    return tok if len(tok) >= MIN_ADMIN_TOKEN_LEN else None


def admin_token(data_dir=None):
    """Return the admin token string, or None."""
    tok = env_token()
    if tok:
        return tok
    tok = ""
    if data_dir:
        try:
            with open(os.path.join(os.path.expanduser(str(data_dir)), "rpc-admin.token")) as f:
                tok = "".join(f.read().split())
        except OSError:
            return None
    return tok or None


def auth_headers(data_dir=None):
    """Return {'Authorization': 'Bearer <tok>'} or {}."""
    tok = admin_token(data_dir)
    return {"Authorization": "Bearer " + tok} if tok else {}


def data_dir_from_command(cmd):
    """Node data dir from a `ps` command line, by the rule of bins/node/src/main.rs (see the shell helper)."""
    cmd = " %s " % cmd.strip()
    m = re.search(r" --data-dir[ =](.*)", cmd)
    if m:
        rest = re.split(r" --| -[A-Za-z] ", m.group(1))[0].rstrip()
        if rest.startswith("~/"):
            rest = os.path.join(os.path.expanduser("~"), rest[2:])
        cand = rest
        while not os.path.isdir(cand):
            if " " not in cand:
                return rest.split(" ")[0]
            cand = cand.rsplit(" ", 1)[0]
        return cand
    if os.environ.get("DOLI_DATA_DIR"):
        return os.environ["DOLI_DATA_DIR"]
    n = re.search(r" (?:--network[ =]|-n )([A-Za-z]+) ", cmd)
    net = (n.group(1) if n else "mainnet").lower()
    home = os.path.expanduser("~")
    if sys.platform.startswith("linux"):
        plat = "/var/lib/doli/" + net
    elif sys.platform == "darwin":
        plat = os.path.join(home, "Library/Application Support/doli", net)
    else:
        plat = os.path.join(home, ".doli", net)
    legacy = os.path.join(home, ".doli", net)
    return legacy if not os.path.exists(plat) and os.path.exists(legacy) else plat


def data_dir_for_port(port):
    """Data dir of the local process listening on TCP `port` whose uid owns its token file, or None."""
    try:
        pids = subprocess.run(["lsof", "-nP", "-iTCP:%d" % int(port), "-sTCP:LISTEN", "-t"],
                              capture_output=True, text=True, timeout=10).stdout.split()
        for pid in pids:
            cmd = subprocess.run(["ps", "-o", "command=", "-p", pid],
                                 capture_output=True, text=True, timeout=10).stdout
            uid = subprocess.run(["ps", "-o", "uid=", "-p", pid],
                                 capture_output=True, text=True, timeout=10).stdout.strip()
            ddir = data_dir_from_command(cmd)
            try:  # accept only a listener whose uid owns the token file (no forged --data-dir)
                if uid and int(uid) == os.stat(os.path.join(ddir, "rpc-admin.token")).st_uid:
                    return ddir
            except (OSError, ValueError):
                pass
    except (OSError, ValueError, subprocess.SubprocessError):
        pass
    return None


def headers_for(method, port=None, data_dir=None):
    """auth_headers for an admin `method` (data dir given or found from the local RPC port), else {}."""
    if method not in ADMIN_METHODS:
        return {}
    if not data_dir and port is not None and not env_token():
        data_dir = data_dir_for_port(port)
    return auth_headers(data_dir)


def is_loopback(host):
    """Exact loopback host match: localhost, ::1, [::1], 127.a.b.c."""
    if host in ("localhost", "::1", "[::1]"):
        return True
    parts = str(host).split(".")
    return (len(parts) == 4 and parts[0] == "127"
            and all(p.isdigit() and len(p) <= 3 for p in parts))


def headers_for_host(method, host, port=None):
    """headers_for an RPC at host:port; the local token file is read only for a loopback host."""
    return headers_for(method, port if is_loopback(host) else None)
