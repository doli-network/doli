"""Admin RPC token lookup (INC-I-308): env DOLI_RPC_ADMIN_TOKEN, then <data_dir>/rpc-admin.token."""
import os
import shlex
import subprocess

ADMIN_METHODS = frozenset((
    "pauseProduction", "resumeProduction", "createCheckpoint", "pruneBlocks", "backfillFromPeer",
    "enterRecoveryMode", "exitRecoveryMode", "bridgeFromArchive", "getUtxoDiff", "getStateSnapshot",
    "getStateRootDebug", "verifyChainIntegrity", "repairArchiveFromPeer", "forceReorgTo"))


def admin_token(data_dir=None):
    """Return the admin token string, or None."""
    tok = os.environ.get("DOLI_RPC_ADMIN_TOKEN", "")
    if tok:
        return tok
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


def data_dir_for_port(port):
    """--data-dir of the local process listening on TCP `port`, or None."""
    try:
        pids = subprocess.run(["lsof", "-nP", "-iTCP:%d" % int(port), "-sTCP:LISTEN", "-t"],
                              capture_output=True, text=True, timeout=10).stdout.split()
        for pid in pids:
            cmd = subprocess.run(["ps", "-o", "command=", "-p", pid],
                                 capture_output=True, text=True, timeout=10).stdout
            a = shlex.split(cmd)
            if "--data-dir" in a and a.index("--data-dir") + 1 < len(a):
                return a[a.index("--data-dir") + 1]
    except (OSError, ValueError, subprocess.SubprocessError):
        pass
    return None


def headers_for(method, port=None, data_dir=None):
    """auth_headers for an admin `method` (data dir given or found from the local RPC port), else {}."""
    if method not in ADMIN_METHODS:
        return {}
    if not data_dir and port is not None and not os.environ.get("DOLI_RPC_ADMIN_TOKEN"):
        data_dir = data_dir_for_port(port)
    return auth_headers(data_dir)
