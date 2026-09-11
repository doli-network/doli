#!/usr/bin/env bash
# INC-I-217 / run 552 / M4 outcome probe.
#
# Measures two properties of the SHIPPED crates, by executing them:
#
#   TXTYPE_WIRE_DECODABLE
#     How many u32 discriminants in 0..=255 the real `TxType::from_u32` decoder
#     accepts. This is the size of the protocol's transaction-type surface — what
#     a peer sending bytes at `sendTransaction` can get past the decoder. Not a
#     test count: the number comes from running the decoder over every input.
#
#   ROTATE_AUTH_PRIMITIVES_LIVE
#     How many of the three rotation-authorisation primitives a third party can
#     actually execute against the shipped crates:
#       1. the 240-byte RotateBlsData codec (exact-length encode/decode)
#       2. the published Ed25519 authorisation preimage/digest encoder
#       3. the BLS rotation proof-of-possession under its own DST, including
#          cross-DST rejection against the registration PoP
#     A primitive that does not exist does not compile, and the whole harness
#     reports 0 — which is the correct reading of "the capability is absent".
#
# The harness test files are temporary: written, run, removed. They are never
# committed and never counted.
set -uo pipefail

REPO="/Users/isudoajl/ownCloud/Projects/doli-network/doli/.claude/worktrees/bls-key-rotation"
cd "$REPO" || exit 1

A="crates/core/tests/zz_m4_probe_txtype_surface.rs"
B="crates/core/tests/zz_m4_probe_rotation_primitives.rs"
cleanup() { rm -f "$A" "$B"; }
trap cleanup EXIT
cleanup

cat > "$A" <<'PROBE_A'
// TEMPORARY outcome probe harness (INC-I-217 M4). Removed by the probe script.
use doli_core::transaction::TxType;

#[test]
fn probe_txtype_wire_decodable() {
    let live = (0u32..=255).filter(|d| TxType::from_u32(*d).is_some()).count();
    println!("PROBE_TXTYPE_WIRE_DECODABLE={live}");
}
PROBE_A

cat > "$B" <<'PROBE_B'
// TEMPORARY outcome probe harness (INC-I-217 M4). Removed by the probe script.
use crypto::{
    bls_sign_pop, bls_verify_pop, sign_rotation_pop, verify_rotation_pop, BlsKeyPair,
};
use doli_core::transaction::{
    rotation_auth_digest, rotation_auth_preimage, RotateBlsData, ROTATE_BLS_DATA_LEN,
};

fn sample() -> RotateBlsData {
    RotateBlsData {
        producer: [7u8; 32],
        new_bls_pubkey: [9u8; 48],
        bls_pop: [3u8; 96],
        signature: [1u8; 64],
    }
}

#[test]
fn probe_primitive_1_payload_codec() {
    let d = sample();
    let bytes = d.encode();
    assert_eq!(bytes.len(), ROTATE_BLS_DATA_LEN);
    assert_eq!(ROTATE_BLS_DATA_LEN, 240);
    assert_eq!(RotateBlsData::decode(&bytes), Some(d));
    assert_eq!(RotateBlsData::decode(&bytes[..239]), None);
    let mut long = bytes.to_vec();
    long.push(0);
    assert_eq!(RotateBlsData::decode(&long), None);
    println!("PROBE_PRIMITIVE_1_OK");
}

#[test]
fn probe_primitive_2_auth_preimage() {
    let genesis = [4u8; 32];
    let new_key = [9u8; 48];
    let prev = [5u8; 32];
    let pre = rotation_auth_preimage(&genesis, &new_key, &prev, 1u32);
    assert_eq!(pre.len(), b"DOLI-ROTATE-BLS-V1".len() + 32 + 48 + 32 + 4);
    assert!(pre.starts_with(b"DOLI-ROTATE-BLS-V1"));
    let dig = rotation_auth_digest(&genesis, &new_key, &prev, 1u32);
    assert_eq!(dig.len(), 32);
    assert_ne!(dig, rotation_auth_digest(&genesis, &new_key, &prev, 2u32));
    println!("PROBE_PRIMITIVE_2_OK");
}

#[test]
fn probe_primitive_3_rotation_pop() {
    let kp = BlsKeyPair::from_seed(&[42u8; 32]).expect("keypair");
    let genesis = [4u8; 32];
    let ed = [8u8; 32];
    let rot = sign_rotation_pop(kp.secret_key(), kp.public_key(), &genesis, &ed).expect("sign");
    assert!(verify_rotation_pop(kp.public_key(), &genesis, &ed, &rot).is_ok());
    assert!(bls_verify_pop(kp.public_key(), &rot).is_err());
    let reg = bls_sign_pop(kp.secret_key(), kp.public_key()).expect("reg pop");
    assert!(verify_rotation_pop(kp.public_key(), &genesis, &ed, &reg).is_err());
    println!("PROBE_PRIMITIVE_3_OK");
}
PROBE_B

OUT_A=$(cargo test -q -p doli-core --test zz_m4_probe_txtype_surface -- --nocapture 2>/dev/null)
TXTYPE=$(printf '%s' "$OUT_A" | sed -n 's/.*PROBE_TXTYPE_WIRE_DECODABLE=\([0-9]*\).*/\1/p' | head -1)
[ -z "$TXTYPE" ] && TXTYPE=0

OUT_B=$(cargo test -q -p doli-core --test zz_m4_probe_rotation_primitives -- --nocapture 2>/dev/null)
PRIMS=$(printf '%s' "$OUT_B" | grep -c 'PROBE_PRIMITIVE_._OK')

cleanup
echo "TXTYPE_WIRE_DECODABLE=${TXTYPE} ROTATE_AUTH_PRIMITIVES_LIVE=${PRIMS}"
