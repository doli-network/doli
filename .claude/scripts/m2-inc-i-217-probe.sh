#!/usr/bin/env bash
# INC-I-217 M2 outcome probe (run 552).
#
# Metric: can the SHIPPED `doli` binary restore a producer wallet whose BLS
# attestation key does not match the key the operator actually holds?
#
# This is the exact situation of the 13cf5207 census transient case: the
# operator still HOLDS the old BLS secret, but wallet.json carries a different
# one, so the node signs attestations the chain rejects (INC-I-217 M1). Until
# M2 there is no command that puts a held key back into a wallet -- `add-bls`
# only GENERATES a new random key and refuses when one is already present.
#
# Externally observable: everything below is driven through the real `doli`
# binary an operator would run, against a throwaway wallet in a temp dir. The
# probe reads the resulting wallet.json from disk. It counts operator
# CAPABILITY, not tests, assertions, verdicts or coverage.
#
#   IMPORT_BLS_SUBCOMMAND_PRESENT -- 1 if `doli import-bls --help` succeeds.
#   MISMATCHED_WALLET_RESTORED    -- 1 if, after running the command, the
#         victim wallet's bls_public_key equals the public key of the secret
#         the operator supplied. This is the whole point of M2: the wallet
#         once again matches the key its owner holds.
#
# Before M2 both read 0 -- clap exits 2 on an unknown subcommand and no
# restore path exists. After M2 both read 1.
set -uo pipefail
WT=/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/bls-key-rotation
cd "$WT" || exit 1

cargo build -p doli-cli --bin doli >/dev/null 2>&1
DOLI="$WT/target/debug/doli"

PRESENT=0
"$DOLI" import-bls --help >/dev/null 2>&1 && PRESENT=1

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# VICTIM: the producer's live wallet. HELD: the wallet whose BLS secret the
# operator still holds and wants back in the victim. Two fresh wallets always
# carry different BLS keys, which is the mismatch being repaired.
"$DOLI" -w "$TMP/victim.json" new >/dev/null 2>&1
"$DOLI" -w "$TMP/held.json"   new >/dev/null 2>&1

read_field() { python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['addresses'][0].get(sys.argv[2]) or '')" "$1" "$2" 2>/dev/null; }

HELD_SECRET=$(read_field "$TMP/held.json" bls_private_key)
HELD_PUB=$(read_field "$TMP/held.json" bls_public_key)
VICTIM_PUB_BEFORE=$(read_field "$TMP/victim.json" bls_public_key)

RESTORED=0
if [ -n "$HELD_SECRET" ] && [ "$HELD_PUB" != "$VICTIM_PUB_BEFORE" ]; then
  "$DOLI" -w "$TMP/victim.json" import-bls "$HELD_SECRET" --force >/dev/null 2>&1
  VICTIM_PUB_AFTER=$(read_field "$TMP/victim.json" bls_public_key)
  [ -n "$HELD_PUB" ] && [ "$VICTIM_PUB_AFTER" = "$HELD_PUB" ] && RESTORED=1
fi

echo "=== INC-I-217 M2 outcome probe — operator-side BLS restore capability ==="
echo "IMPORT_BLS_SUBCOMMAND_PRESENT $PRESENT"
echo "MISMATCHED_WALLET_RESTORED $RESTORED"
echo "=== end ==="
