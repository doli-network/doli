#!/usr/bin/env python3
"""gs010_txwire.py — minimal replica of DOLI's transaction wire format.

Imported by scripts/gs010_build_dup_register.py (GS-010 gauntlet scenario). It
exists ONLY so the gauntlet can build a duplicate `Registration` transaction
without the CLI, which since INC-I-148 refuses to build one
(bins/cli/src/cmd_producer/register.rs:37). Not a general-purpose library — it
implements exactly the subset a Registration needs.

bincode 1.3 defaults are replicated: fixint, little-endian, u64 sequence
lengths, u32 enum variant index, u8 Option tag. `Hash`, `PublicKey` and
`Signature` all serialise through serde `bytes` (crypto/src/hash.rs:194,
keys.rs:189, signature.rs:~185), i.e. u64 length + raw bytes.

`Output.hash_bytes()` mirrors `Output::serialize()`
(crates/core/src/transaction/output.rs:748) — a SEPARATE hand-rolled encoding
used inside the signing hash and the tx hash, NOT the bincode encoding.

VALIDATED AGAINST THE LIVE TESTNET, not assumed: 177 real transactions
(Transfer, Registration, EpochReward) re-serialised byte-identical to the
node's own bytes, computed tx hashes matched `getBlockByHeight` for 4 blocks
spanning genesis to tip, and 64/64 real on-chain Ed25519 signatures verified
against the recomputed `signing_message_for_input`.
"""

import struct

try:
    import blake3 as _blake3_mod
except ImportError as exc:  # pragma: no cover - environment guard
    raise SystemExit(
        "gs010-txwire: missing python dependency (%s). Needs `blake3`: "
        "python3 -m pip install blake3" % exc
    )

# bincode encodes an enum by VARIANT INDEX (declaration position), while
# `Output::serialize()` writes `self.output_type as u8` (the DISCRIMINANT).
# OutputType tombstones discriminants 11-12 (B.1 DeFi), so the two diverge from
# index 11 up. A Registration only ever carries Normal + Bond outputs, so this
# module refuses anything above LPShare (index 10) rather than guess.
MAX_SUPPORTED_OUTPUT_INDEX = 10

OUT_NORMAL = 0  # crates/core/src/transaction/types.rs:185
OUT_BOND = 1  # crates/core/src/transaction/types.rs:187
TXTYPE_REGISTRATION = 1  # crates/core/src/transaction/types.rs:11


def b3(data):
    return _blake3_mod.blake3(data).digest()


class WireError(Exception):
    """Raised when a buffer cannot be parsed as this wire format."""


# ── bincode reader ──────────────────────────────────────────────────────────


class Reader:
    __slots__ = ("buf", "pos")

    def __init__(self, buf, pos=0):
        self.buf = buf
        self.pos = pos

    def take(self, n):
        if n < 0 or self.pos + n > len(self.buf):
            raise ValueError("short read")
        out = self.buf[self.pos : self.pos + n]
        self.pos += n
        return out

    def u8(self):
        return self.take(1)[0]

    def u32(self):
        return struct.unpack("<I", self.take(4))[0]

    def u64(self):
        return struct.unpack("<Q", self.take(8))[0]

    def sized_bytes(self, expect):
        """serde `bytes` under bincode: u64 length + raw. Length is asserted so a
        wrong scan offset dies immediately instead of consuming garbage."""
        n = self.u64()
        if n != expect:
            raise ValueError("expected %d-byte field, got length %d" % (expect, n))
        return self.take(n)

    def byte_vec(self, limit=8 << 20):
        n = self.u64()
        if n > limit:
            raise ValueError("implausible Vec<u8> length %d" % n)
        return self.take(n)


def u32le(v):
    return struct.pack("<I", v)


def u64le(v):
    return struct.pack("<Q", v)


def bincode_bytes(raw):
    return u64le(len(raw)) + raw


# ── transaction model ───────────────────────────────────────────────────────


class Input:
    __slots__ = (
        "prev_tx_hash",
        "output_index",
        "signature",
        "sighash_type",
        "committed_output_count",
        "public_key",
    )

    def __init__(self, prev_tx_hash, output_index, signature, sighash, committed, pk):
        self.prev_tx_hash = prev_tx_hash
        self.output_index = output_index
        self.signature = signature
        self.sighash_type = sighash
        self.committed_output_count = committed
        self.public_key = pk

    @staticmethod
    def read(r):
        prev = r.sized_bytes(32)
        idx = r.u32()
        sig = r.sized_bytes(64)
        sighash = r.u32()
        if sighash > 1:
            raise ValueError("bad sighash_type %d" % sighash)
        committed = r.u32()
        tag = r.u8()
        if tag == 0:
            pk = None
        elif tag == 1:
            pk = r.sized_bytes(32)
        else:
            raise ValueError("bad Option tag %d" % tag)
        return Input(prev, idx, sig, sighash, committed, pk)

    def encode(self):
        out = bincode_bytes(self.prev_tx_hash)
        out += u32le(self.output_index)
        out += bincode_bytes(self.signature)
        out += u32le(self.sighash_type)
        out += u32le(self.committed_output_count)
        if self.public_key is None:
            out += b"\x00"
        else:
            out += b"\x01" + bincode_bytes(self.public_key)
        return out


class Output:
    __slots__ = ("output_type", "amount", "pubkey_hash", "lock_until", "extra_data")

    def __init__(self, output_type, amount, pubkey_hash, lock_until, extra_data):
        self.output_type = output_type
        self.amount = amount
        self.pubkey_hash = pubkey_hash
        self.lock_until = lock_until
        self.extra_data = extra_data

    @staticmethod
    def read(r):
        ot = r.u32()
        if ot > MAX_SUPPORTED_OUTPUT_INDEX:
            raise ValueError("unsupported OutputType variant index %d" % ot)
        amount = r.u64()
        pkh = r.sized_bytes(32)
        lock = r.u64()
        extra = r.byte_vec(limit=1 << 20)
        return Output(ot, amount, pkh, lock, extra)

    def encode(self):
        return (
            u32le(self.output_type)
            + u64le(self.amount)
            + bincode_bytes(self.pubkey_hash)
            + u64le(self.lock_until)
            + bincode_bytes(self.extra_data)
        )

    def hash_bytes(self):
        """Mirror of `Output::serialize()` — the hand-rolled encoding fed to the
        signing hash and the tx hash. NOT the bincode encoding."""
        out = bytes([self.output_type])  # discriminant == index for 0..=10
        out += u64le(self.amount)
        out += self.pubkey_hash
        out += u64le(self.lock_until)
        if len(self.extra_data) > 65535:
            out += struct.pack("<H", 0xFFFF) + u32le(len(self.extra_data))
        else:
            out += struct.pack("<H", len(self.extra_data))
        out += self.extra_data
        return out


class Tx:
    __slots__ = ("version", "tx_type", "inputs", "outputs", "extra_data")

    def __init__(self, version, tx_type, inputs, outputs, extra_data):
        self.version = version
        self.tx_type = tx_type
        self.inputs = inputs
        self.outputs = outputs
        self.extra_data = extra_data

    @staticmethod
    def read(r):
        version = r.u32()
        tx_type = r.u32()
        if tx_type > 40:
            raise ValueError("implausible TxType %d" % tx_type)
        n = r.u64()
        if n > 4096:
            raise ValueError("implausible input count %d" % n)
        inputs = [Input.read(r) for _ in range(n)]
        n = r.u64()
        if n > 65536:
            raise ValueError("implausible output count %d" % n)
        outputs = [Output.read(r) for _ in range(n)]
        extra = r.byte_vec()
        return Tx(version, tx_type, inputs, outputs, extra)

    def encode(self):
        out = u32le(self.version) + u32le(self.tx_type)
        out += u64le(len(self.inputs))
        for i in self.inputs:
            out += i.encode()
        out += u64le(len(self.outputs))
        for o in self.outputs:
            out += o.encode()
        out += bincode_bytes(self.extra_data)
        return out

    def _common_prefix(self):
        h = u32le(self.version) + u32le(self.tx_type)
        h += u32le(len(self.inputs))
        for i in self.inputs:
            h += i.prev_tx_hash + u32le(i.output_index)
        h += u32le(len(self.outputs))
        for o in self.outputs:
            h += o.hash_bytes()
        return h

    def hash(self):
        """Mirror of `Transaction::hash()` (transaction/core.rs:479)."""
        return b3(
            self._common_prefix() + u32le(len(self.extra_data)) + self.extra_data
        )

    def signing_message_for_input(self, i):
        """Mirror of `signing_message_for_input` for SighashType::All
        (transaction/core.rs:546-569). extra_data is EXCLUDED."""
        inp = self.inputs[i]
        if inp.sighash_type != 0:
            raise ValueError("only SighashType::All is supported")
        return b3(
            self._common_prefix() + inp.prev_tx_hash + u32le(inp.output_index)
        )


# ── block scan ──────────────────────────────────────────────────────────────


def find_tx_in_block(raw, want_hash):
    """Locate a transaction inside bincode(Block) WITHOUT parsing BlockHeader.

    The header carries VdfOutput/VdfProof/Vec<PublicKey> fields whose layout is
    free to evolve; anchoring on them would make this helper rot silently. So
    instead: scan the first bytes for an offset where `Vec<Transaction>` parses
    AND the two trailing `Vec<u8>` body fields consume the buffer EXACTLY. The
    u64 length prefixes on every Hash/Signature/PublicKey make a false positive
    effectively impossible, and the exact-consumption requirement makes it a
    whole-structure check rather than a guess.
    """
    n = len(raw)
    for start in range(0, min(n, 8192)):
        r = Reader(raw, start)
        try:
            count = r.u64()
            if count < 1 or count > 4096:
                continue
            spans = []
            for _ in range(count):
                begin = r.pos
                tx = Tx.read(r)
                spans.append((tx, raw[begin : r.pos]))
            r.byte_vec(limit=1 << 16)  # aggregate_bls_signature
            r.byte_vec(limit=1 << 16)  # attestation_bitfield
            if r.pos != n:
                continue
        except (ValueError, struct.error, IndexError):
            continue
        for tx, span in spans:
            if tx.hash() == want_hash:
                # Round-trip gate: our encoder must reproduce the node's bytes.
                # A mismatch means the wire format drifted away from this
                # module — the caller must ABORT, never emit a guessed tx.
                if tx.encode() != span:
                    raise WireError(
                        "tx re-serialisation mismatch — the bincode wire format "
                        "drifted from this module. Refusing to emit."
                    )
                return tx
        return None
    return None

