#!/usr/bin/env bash
# INC-I-217 / M3 outcome probe — "can the wire numbering drift in silence?"
#
# M3 is byte-for-byte behaviour preserving by design, so the number it moves is
# not a behaviour: it is the DETECTABILITY of a silent numbering change. This
# probe measures that directly, by perturbation of the shipped source, from
# outside the test suite's own bookkeeping. It never counts tests.
#
# DOLI numbers a TxType on two independent surfaces, and they are NOT equal:
#   * the serde/bincode variant index  = DECLARATION POSITION (ZKSettle -> 23)
#   * the `#[repr(u32)]` discriminant  = `as u32`, used by `signing_message`
#     and `TxType::from_u32`           (ZKSettle -> 31)
# Each experiment moves exactly ONE surface and leaves the other identical, so
# a 0 means that surface specifically is unguarded.
#
# A  TXTYPE_BINCODE_ORDINAL_DRIFT_DETECTED
#      Swap the DECLARATION ORDER of `MintAsset = 17` and `BurnAsset = 18`.
#      Both keep their `= N`, so every `as u32`, every `from_u32` arm and every
#      match arm are bit-identical; only the bincode variant index swaps. A peer
#      decoding the wire now reads BurnAsset where MintAsset was sent.
#
# C  TXTYPE_DISCRIMINANT_DRIFT_DETECTED
#      Renumber `ZKSettle = 31` to `= 33` (and its `from_u32` arm to match).
#      Declaration position is untouched, so the bincode ordinal stays 23; what
#      changes is `tx_type as u32`, which is hashed into the signing message.
#
# B  PENDING_UPDATE_TAG_DRIFT_DETECTED
#      Rename the persisted serde tag of `PendingProducerUpdate::Exit`. The Rust
#      variant, every match arm and every caller stay identical; only the name
#      written into (and expected out of) the producers.bin JSON changes — a
#      silent deferred-mutation persistence break of exactly the kind M7 must
#      prove it did not cause.
#
# 1 = the tree rejects the change. 0 = it ships in silence.
#
# Perturbations are applied in place to backed-up copies and reverted by an EXIT
# trap. Before exiting the script asserts, with cmp against those backups, that
# every touched file is byte-identical to how it found it.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO" || exit 1

TXTYPE="crates/core/src/transaction/types.rs"
PENDING="crates/storage/src/producer/types.rs"
LOGDIR="${M3_PROBE_LOGDIR:-$(mktemp -d)}"
mkdir -p "$LOGDIR"

BAK_TX="$LOGDIR/types.rs.orig"
BAK_PD="$LOGDIR/producer_types.rs.orig"
cp "$TXTYPE" "$BAK_TX"
cp "$PENDING" "$BAK_PD"
trap 'cp "$BAK_TX" "$TXTYPE"; cp "$BAK_PD" "$PENDING"' EXIT

# classify(logfile, rc) -> BUILD_ERROR | 0 (silent) | 1 (detected)
classify() {
    if grep -qE 'error\[E[0-9]+\]|could not compile' "$1"; then
        echo BUILD_ERROR
    elif [ "$2" -eq 0 ]; then
        echo 0
    else
        echo 1
    fi
}

# ---------- A: bincode variant index ----------
python3 - "$TXTYPE" <<'PY' || { echo "PROBE_ABORT surgery_A" >&2; exit 3; }
import sys
p = sys.argv[1]; src = open(p).read()
mint = "    /// Mint new units of a fungible asset (issuer-only, requires matching asset_id).\n    MintAsset = 17,\n"
burn = "    /// Burn units of a fungible asset (holder burns own tokens, provably destroyed).\n    BurnAsset = 18,\n"
assert src.count(mint) == 1, "MintAsset anchor"
assert src.count(burn) == 1, "BurnAsset anchor"
assert src.index(mint) < src.index(burn)
src = src.replace(mint + burn, burn + mint)
open(p, "w").write(src)
PY
cargo nextest run -p doli-core -p wallet -p storage --status-level none --final-status-level fail \
    > "$LOGDIR/expA.log" 2>&1
A_RESULT=$(classify "$LOGDIR/expA.log" $?)
cp "$BAK_TX" "$TXTYPE"

# ---------- C: repr(u32) discriminant ----------
python3 - "$TXTYPE" <<'PY' || { echo "PROBE_ABORT surgery_C" >&2; exit 5; }
import sys
p = sys.argv[1]; src = open(p).read()
assert src.count("    ZKSettle = 31,\n") == 1, "ZKSettle decl anchor"
assert src.count("            31 => Some(Self::ZKSettle),\n") == 1, "from_u32 arm anchor"
src = src.replace("    ZKSettle = 31,\n", "    ZKSettle = 33,\n")
src = src.replace("            31 => Some(Self::ZKSettle),\n",
                  "            33 => Some(Self::ZKSettle),\n")
open(p, "w").write(src)
PY
cargo nextest run -p doli-core -p wallet -p storage --status-level none --final-status-level fail \
    > "$LOGDIR/expC.log" 2>&1
C_RESULT=$(classify "$LOGDIR/expC.log" $?)
cp "$BAK_TX" "$TXTYPE"

# ---------- B: persisted serde tag ----------
python3 - "$PENDING" <<'PY' || { echo "PROBE_ABORT surgery_B" >&2; exit 4; }
import sys
p = sys.argv[1]; src = open(p).read()
anchor = "pub enum PendingProducerUpdate {\n"
assert src.count(anchor) == 1
head, tail = src.split(anchor, 1)
target = "    Exit {\n"
assert target in tail
tail = tail.replace(target, '    #[serde(rename = "ExitRenamed")]\n' + target, 1)
open(p, "w").write(head + anchor + tail)
PY
cargo nextest run -p storage --status-level none --final-status-level fail \
    > "$LOGDIR/expB.log" 2>&1
B_RESULT=$(classify "$LOGDIR/expB.log" $?)
cp "$BAK_PD" "$PENDING"

# Byte-exact restore assertion. Compares against the backup taken above, not
# against HEAD, so the probe is runnable on an uncommitted tree and still
# proves it left the source exactly as it found it.
REVERT="clean"
cmp -s "$BAK_TX" "$TXTYPE" || REVERT="DIRTY:$TXTYPE"
cmp -s "$BAK_PD" "$PENDING" || REVERT="DIRTY:$PENDING"

echo "TXTYPE_BINCODE_ORDINAL_DRIFT_DETECTED $A_RESULT"
echo "TXTYPE_DISCRIMINANT_DRIFT_DETECTED $C_RESULT"
echo "PENDING_UPDATE_TAG_DRIFT_DETECTED $B_RESULT"
echo "revert $REVERT"
echo "logs $LOGDIR"
