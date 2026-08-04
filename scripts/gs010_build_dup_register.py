#!/usr/bin/env python3
"""gs010_build_dup_register.py — build the GS-010 duplicate Registration tx.

Used ONLY by scripts/gauntlet-gs010.sh. Emits a signed, hex-encoded
`TxType::Registration` transaction that duplicates an ALREADY-MINED
registration for the same producer pubkey, so the gauntlet can inject it with a
raw `sendTransaction` JSON-RPC call instead of a second `doli producer register`
invocation.

WHY NOT THE CLI: `bins/cli/src/cmd_producer/register.rs:37` (INC-I-148 RC-2)
consults the PLURAL `getProducers` and bails BEFORE building anything when the
key already has a `pending` registration. That fix is correct and MUST NOT be
weakened. But raw `sendTransaction` is publicly dispatchable
(`crates/rpc/src/methods/dispatch.rs:22`), so the duplicate-registration vector
is still real — it just moved from the CLI to the wire. GS-010 must therefore
test the NODE's mempool rejection, not the CLI's pre-check.

TEMPLATE-OFF-TX1, DO NOT SYNTHESISE. The one field that cannot be recomputed
outside the Rust binary is `RegistrationData` (BLS12-381 public key + proof of
possession + a 5M-iteration hash-chain VDF output). It is reused BYTE-FOR-BYTE
from registration #1, which is sound because:
  * `validate_registration_vdf` (crates/core/src/validation/registration.rs:222)
    recomputes the VDF over `registration_input(pubkey, reg_data.epoch)` —
    self-consistent, never cross-checked against the chain's current epoch.
  * `validate_registration_chain` (:187) compares against
    `RegistrationChainState::default()` (Hash::ZERO / seq 0) at every non-test
    construction site, which is exactly what the CLI always emits.
  * Bond `lock_until` is the ONE field that cannot be reused verbatim: it is
    validated as `>= current_height + blocks_per_era` (:104) and the CLI mints
    only `best_height + blocks_per_era + 1000`, so tx1's value expires 1000
    blocks after #1 was built. MEASURED against the live testnet: replaying a
    2045-block-old registration is rejected with `bond lock too short:
    12696292 < 12697378`. It is therefore shifted forward by the elapsed
    height plus the CLI's own 1000-block slack, which is derived entirely from
    on-chain data and needs no hardcoded `blocks_per_era`.

DISJOINT INPUTS ARE STRUCTURAL HERE, NOT INCIDENTAL. tx2's single input is the
CHANGE OUTPUT CREATED BY TX1 (`tx1_hash:change_vout`). A transaction cannot
spend its own outputs, so tx2's input set is disjoint from tx1's by
construction — no reliance on wallet coin-selection happening to pick the right
UTXO. This is the property the whole scenario rests on: with shared inputs,
`Mempool::revalidate` evicts #2 ~95us after #1 mines and nothing reproduces.

SELF-VERIFYING. Before emitting anything, the parser round-trips tx1:
re-serialise the parsed struct and require byte-equality with the slice it came
from, AND require the recomputed BLAKE3 tx hash to equal the caller-supplied
tx1 hash. If the wire format ever drifts, this exits non-zero with a clear
message and the gauntlet SKIPS cleanly — it can never emit a malformed tx that
silently fails to inject.

The bincode/BLAKE3/Ed25519 wire replica lives in scripts/gs010_txwire.py.
"""

import argparse
import base64
import json
import os
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gs010_txwire import (  # noqa: E402
    OUT_BOND,
    OUT_NORMAL,
    TXTYPE_REGISTRATION,
    Input,
    Output,
    Tx,
    WireError,
    b3,
    find_tx_in_block,
)

# crates/core/src/consensus/constants.rs:644,655,659
BASE_FEE = 1
FEE_PER_BYTE = 1
FEE_DIVISOR = 100


def die(msg):
    print("gs010-build: " + msg, file=sys.stderr)
    sys.exit(1)


try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
except ImportError as exc:  # pragma: no cover - environment guard
    die(
        "missing python dependency (%s). Needs `cryptography`: "
        "python3 -m pip install cryptography" % exc
    )

# ── JSON-RPC ────────────────────────────────────────────────────────────────


def rpc(url, method, params, timeout=20):
    body = json.dumps(
        {"jsonrpc": "2.0", "method": method, "params": params, "id": 1}
    ).encode()
    req = urllib.request.Request(
        url, data=body, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        payload = json.load(resp)
    if "error" in payload and payload["error"] is not None:
        raise RuntimeError("%s: %s" % (method, payload["error"]))
    return payload["result"]



# ── main ────────────────────────────────────────────────────────────────────


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--rpc", required=True, help="node JSON-RPC endpoint")
    ap.add_argument("--wallet", required=True, help="wallet json of the target")
    ap.add_argument("--tx1", required=True, help="hex hash of registration #1")
    ap.add_argument("--height", required=True, type=int, help="height tx1 mined at")
    args = ap.parse_args()

    try:
        want = bytes.fromhex(args.tx1.strip())
    except ValueError:
        die("--tx1 is not hex")
    if len(want) != 32:
        die("--tx1 must be a 32-byte hash")

    try:
        with open(args.wallet) as fh:
            wallet = json.load(fh)
        addr = wallet["addresses"][0]
        priv = bytes.fromhex(addr["private_key"])
        pub = bytes.fromhex(addr["public_key"])
    except (OSError, ValueError, KeyError, IndexError) as exc:
        die("cannot read wallet %s: %s" % (args.wallet, exc))
    if len(priv) != 32 or len(pub) != 32:
        die("wallet key material is not 32 bytes")

    try:
        blk = rpc(args.rpc, "getBlockRaw", {"height": args.height})
        raw = base64.b64decode(blk["block"])
    except Exception as exc:  # noqa: BLE001 - report and skip
        die("getBlockRaw(%d) failed: %s" % (args.height, exc))
    if b3(raw).hex() != blk.get("blake3", ""):
        die("getBlockRaw checksum mismatch — refusing to parse")

    try:
        tx1 = find_tx_in_block(raw, want)
    except WireError as exc:
        die(str(exc))
    if tx1 is None:
        die("registration #1 %s not found in block %d" % (args.tx1, args.height))
    if tx1.tx_type != TXTYPE_REGISTRATION:
        die("tx1 is TxType %d, not Registration" % tx1.tx_type)
    if not tx1.inputs or tx1.inputs[0].public_key != pub:
        die("tx1 input pubkey does not match the wallet — wrong target")

    bonds = [o for o in tx1.outputs if o.output_type == OUT_BOND]
    changes = [o for o in tx1.outputs if o.output_type == OUT_NORMAL]
    if not bonds:
        die("tx1 has no Bond outputs")
    if len(changes) != 1:
        die(
            "tx1 has %d Normal outputs, expected exactly 1 change output — the "
            "DISJOINT-INPUT precondition (register #1 must bond less than half "
            "the wallet) was not met" % len(changes)
        )
    change_vout = tx1.outputs.index(changes[0])
    change_amt = changes[0].amount

    # The change output must still be unspent, otherwise there is nothing
    # disjoint to fund #2 from.
    try:
        utxos = rpc(
            args.rpc,
            "getUtxos",
            {"address": changes[0].pubkey_hash.hex(), "spendable_only": False},
        )
    except Exception:  # noqa: BLE001 - advisory only
        utxos = None
    if isinstance(utxos, list):
        live = any(
            u.get("txHash", "").lower() == args.tx1.strip().lower()
            and u.get("outputIndex") == change_vout
            for u in utxos
        )
        if not live:
            die(
                "tx1 change output %s:%d is not in the node's UTXO set — it was "
                "already spent, or tx1 has not settled" % (args.tx1, change_vout)
            )

    required = sum(o.amount for o in bonds)
    extra_bytes = sum(len(o.extra_data) for o in bonds)
    fee = BASE_FEE + extra_bytes * FEE_PER_BYTE // FEE_DIVISOR
    if change_amt < required + fee:
        die(
            "confirmed change %d cannot fund a duplicate of %d bonds (needs "
            "%d + %d fee) — raise GS010_FUND or lower GS010_BONDS"
            % (change_amt, len(bonds), required, fee)
        )

    # Bond lock: `validate_registration_data_inner` requires
    # `lock_until >= current_height + blocks_per_era` (validation/registration.rs:104).
    # tx1's value was minted against tx1's build height, so it decays as the
    # chain advances. Shift it by the elapsed height plus the CLI's own
    # 1000-block slack (cmd_producer/register.rs:122). Because tx1 was itself
    # accepted at `--height`, the shifted value is >= current + blocks_per_era
    # for any elapsed distance — without this helper needing to know
    # blocks_per_era, which no RPC exposes.
    try:
        cur_height = rpc(args.rpc, "getChainInfo", {})["bestHeight"]
    except Exception as exc:  # noqa: BLE001
        die("getChainInfo failed: %s" % exc)
    lock_shift = max(0, int(cur_height) - args.height) + 1000

    # tx2: same producer, same bonds, same RegistrationData — funded ONLY by
    # the change output tx1 itself created, so the input sets are disjoint by
    # construction.
    outputs = [
        Output(
            o.output_type,
            o.amount,
            o.pubkey_hash,
            o.lock_until + lock_shift,
            o.extra_data,
        )
        for o in bonds
    ]
    change2 = change_amt - required - fee
    if change2 > 0:
        outputs.append(Output(OUT_NORMAL, change2, changes[0].pubkey_hash, 0, b""))

    tx2 = Tx(
        tx1.version,
        tx1.tx_type,
        [Input(want, change_vout, b"\x00" * 64, 0, 0, pub)],
        outputs,
        tx1.extra_data,
    )
    msg = tx2.signing_message_for_input(0)
    tx2.inputs[0].signature = Ed25519PrivateKey.from_private_bytes(priv).sign(msg)

    encoded = tx2.encode()
    print(
        json.dumps(
            {
                "tx_hex": encoded.hex(),
                "tx_hash": tx2.hash().hex(),
                "input": "%s:%d" % (args.tx1, change_vout),
                "tx1_inputs": [
                    "%s:%d" % (i.prev_tx_hash.hex(), i.output_index) for i in tx1.inputs
                ],
                "bonds": len(bonds),
                "fee": fee,
                "change": change2,
                "lock_until": bonds[0].lock_until + lock_shift,
                "size": len(encoded),
            }
        )
    )


if __name__ == "__main__":
    main()
