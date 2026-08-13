//! INC-I-176 **M1a** — THE WIRE FREEZE. `MaintainerChangeData` moves ZERO bytes.
//!
//! This is the file that exists because M1 attempt 1 was rejected. Everything
//! here is a regression lock on ONE property:
//!
//! > The bincode encoding of `MaintainerChangeData` is byte-identical to
//! > `3f8bf185` (HEAD) for every input, and every payload already on chain still
//! > decodes.
//!
//! ---------------------------------------------------------------------------
//! WHY — the measured failure this file prevents from recurring
//! ---------------------------------------------------------------------------
//! Attempt 1 replaced `reason: Option<String>` (a 1-byte bincode `None` tail)
//! with `valid_before: u64` (an 8-byte tail) as a straight, UNGATED field swap.
//! `MaintainerChangeData::from_bytes` is consumed FATALLY and WITHOUT a height
//! gate at `crates/core/src/validation/tx_types.rs:809`, so a payload that fails
//! to decode becomes a hard block reject on the normal sync path.
//!
//! MEASURED on the LOCAL testnet (read-only RPC `127.0.0.1:8500`, 2026-08-12,
//! tip 146_889, network `testnet`, node 6.24.1):
//!
//! | fact | value |
//! |---|---|
//! | block | 136690, hash `b0c4bb41…dbc6`, 2 transactions |
//! | tx | `62a3bfbd388a208d98d1b3ebb35757426358d1fb3730112297b12eb69bf8bc81` |
//! | txType | `add_maintainer`, wire size 417 B |
//! | `extra_data` | **385 B**, 3 signatures, `reason = None`, trailing byte `0x00` |
//!
//! The 385 bytes below are the **REAL ON-CHAIN BYTES**, not a reconstruction.
//! They were obtained read-only: `getBlockRaw(136690)` → base64 → the block was
//! deserialized with `Block::deserialize` inside a DETACHED `git worktree` at
//! HEAD, placed outside the repository and removed afterwards. The `to_bytes`
//! goldens further down came out of the SAME worktree run, i.e. from HEAD's real
//! encoder — never from hand-reasoned bincode rules.
//!
//! With those bytes, attempt 1's decoder needs 8 tail bytes where 1 exists,
//! `from_bytes` returns `None`, and the binary cannot sync past block 136690 in
//! EITHER deploy direction. A synchronized deploy does not repair it: the block
//! is re-validated on every full sync from genesis, forever.
//!
//! **Therefore M1a changes the payload by zero bytes.** The `valid_before`
//! payload field is deferred to milestone **M2.5**, where an activation height
//! and an EXPLICIT format discriminator make it safe. `valid_before` remains a
//! `signing_message*` PARAMETER in M1a — that is intended, not a defect.
//!
//! TDD RED. Against the working tree as handed over (M1 attempt 1, uncommitted)
//! this file does NOT compile: `MaintainerChangeData` has no `reason` field and
//! `with_reason` / `MAX_MAINTAINER_CHANGE_REASON_BYTES` do not exist. THAT is the
//! red evidence. The fix is to revert the source, never to relax this file.
//!
//! Contract + full matrix: `docs/.workflow/inc-i-176-M1a-output-contract.md`.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! W `MaintainerChangeData::to_bytes(&self) -> Vec<u8>`
//!   W-O3  return — the bincode encoding. MUST equal the frozen HEAD literal.
//!   W-O3b return LENGTH — split out because a length change alone is already a
//!         consensus-visible break (`extra_data` feeds the txid,
//!         `transaction/core.rs:504-506`) and is the cheapest failure to read.
//! X `MaintainerChangeData::from_bytes(&[u8]) -> Option<Self>`
//!   X-O3  return DISCRIMINANT — `Some` for every HEAD-shaped payload, including
//!         the real on-chain one; `None` for truncated / garbage input.
//!   X-O3b return CONTENT — `target`, `signatures` (count, order, bytes) and
//!         `reason` must survive the round trip unchanged.
//! Y `MaintainerChangeData::new` / `::with_reason`
//!   Y-O3  return — the constructed struct. `reason` defaults to `None`;
//!         `signatures` keep INPUT ORDER (HEAD does not canonicalize, and adding
//!         canonicalization would silently change every constructed txid).
//! Z  STRUCTURAL — the field list itself: exactly `(target, signatures, reason)`,
//!    in that order, proven by an independent mirror struct rather than by
//!    reading `data.rs`.
//!   mutable params   : NONE.
//!   receiver mutation: NONE (`to_bytes` takes `&self`; the constructors return).
//!   persistent store : NONE. No I/O on any path in this file.
//!   side channels    : NONE. DECLARED UNASSERTED — nothing is logged here.
//!
//! CODE PATHS
//!   P-ENC  `to_bytes` — one path, no branches (`bincode::serialize(..).unwrap_or_default()`).
//!   P-DEC+ `from_bytes` returns `Some` — the decode succeeds.
//!   P-DEC- `from_bytes` returns `None`  — the decode fails (`.ok()` swallows the error).
//!
//! INPUT PARTITIONS
//!   IP-S0 / IP-S1 / IP-S5  signature count 0 / 1 / 5 (5 = `MAX_MAINTAINER_CHANGE_SIGNATURES`).
//!         Distinct because the `Vec` length prefix and the per-entry framing are
//!         what a field reorder would disturb first.
//!   IP-R-NONE     `reason = None`             -> 1-byte tail. THE ON-CHAIN CASE.
//!   IP-R-EMPTY    `reason = Some("")`         -> 9-byte tail. Byte-length ambiguity
//!                 with a `u64` tail: this is the SILENT misread partition.
//!   IP-R-TEXT     `reason = Some("rotation")` -> 17-byte tail.
//!   IP-R-UNICODE  `reason = Some("rotación 🔑")` -> multi-byte UTF-8 + emoji
//!                 (worst scenario #4); proves the cap counts BYTES, not chars.
//!   IP-R-MAX      `reason` of exactly `MAX_MAINTAINER_CHANGE_REASON_BYTES`
//!                 -> the maximal legal payload; INC-I-173 F5 headroom.
//!   IP-ORDER      signature vector supplied in DESCENDING pubkey order
//!                 -> catches a constructor that starts canonicalizing.
//!   IP-CHAIN      the real 385-byte on-chain payload (3 sigs, `reason = None`).
//!   IP-U64TAIL    attempt 1's shape (8-byte `u64` tail) -> P-DEC-; the mixed-fleet
//!                 break, asserted rather than described.
//!   IP-MALFORMED  empty / truncated / all-`0xff` / oversized length prefix -> P-DEC-.
//!   IP-RAWKEY     `PublicKey::from_bytes` on a non-curve byte pattern. MEASURED
//!                 in this suite: SERIALIZATION accepts it, DESERIALIZATION
//!                 REJECTS it (`Custom("invalid key bytes: not a valid Ed25519
//!                 key")`). The frozen goldens are therefore ENCODE-ONLY
//!                 fixtures, and every partition that exercises `from_bytes`
//!                 uses a REAL keypair — otherwise a `None` would be
//!                 attributable to the key material rather than to the shape,
//!                 and the regression lock would pass for the wrong reason.
//!   MATRIX: (W-O3, W-O3b) × {IP-S0,S1,S5} × {IP-R-NONE,EMPTY,TEXT,UNICODE};
//!           (X-O3, X-O3b) × the same set + IP-R-MAX + IP-CHAIN;
//!           X-O3 × {IP-U64TAIL, IP-MALFORMED};
//!           Y-O3 × {IP-ORDER × IP-R-NONE, IP-ORDER × IP-R-TEXT};
//!           Z    × the mirror struct, both directions.
//!
//! ---------------------------------------------------------------------------
//! WHAT THIS FILE DOES NOT DO
//! ---------------------------------------------------------------------------
//! 1. It does NOT rewrite `crates/core/tests/inc_i_173_m3_payload_bounds.rs`. That
//!    file returns to HEAD; the payload does not change, so INC-I-173's F5 bound is
//!    untouched. The single INC-I-173 fact restated here is the maximal-payload
//!    SIZE, and only as a cheap cross-check that the shape did not move.
//! 2. It adds NO reject condition anywhere in `crates/core/src/validation/`.
//! 3. It asserts nothing about expiry ENFORCEMENT. There is no expiry in M1a.

use crypto::{KeyPair, PublicKey, Signature};
use doli_core::maintainer::{
    MaintainerChangeData, MaintainerSignature, MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
    MAX_MAINTAINER_CHANGE_REASON_BYTES, MAX_MAINTAINER_CHANGE_SIGNATURES,
};
use serde::{Deserialize, Serialize};

// ===========================================================================
// FROZEN GOLDENS — produced by HEAD's REAL bincode encoder (3f8bf185), inside a
// detached worktree outside this repository, then pasted here as literals.
//
// DO NOT REGENERATE THESE FROM THE CURRENT IMPLEMENTATION. Regenerating them
// converts the only instrument that can detect a field add / remove / reorder
// into a mirror of whatever the code happens to do — the exact failure mode the
// Full Bitfield Decode pillar (CLAUDE.md) exists to prevent.
//
// Layout of every literal, read left to right:
//   target       : u64 LE length `20000000 00000000` (32) || 32 raw bytes
//   signatures   : u64 LE count  || per entry { pubkey: len||32B, sig: len||64B }
//   reason       : `00`  (None)  |  `01` || u64 LE byte-length || UTF-8 bytes
// The FINAL byte of every `reason = None` literal is `00`. That single byte is
// the whole incident.
// ===========================================================================

/// IP-S0 × IP-R-NONE — 49 bytes. The smallest legal payload.
const GOLD_S0_NONE: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0000000000000000",
    "00",
);

/// IP-S1 × IP-R-NONE — 161 bytes.
const GOLD_S1_NONE: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0100000000000000",
    "2000000000000000404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f4000000000000000",
    "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeef",
    "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff00",
);

/// IP-S5 × IP-R-NONE — 609 bytes. The full `MAX_MAINTAINER_CHANGE_SIGNATURES` set.
const GOLD_S5_NONE: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0500000000000000",
    "2000000000000000404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f4000000000000000",
    "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeef",
    "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff2000000000000000505152535455565758595a5b5c5d5e5f6061626364656667",
    "68696a6b6c6d6e6f4000000000000000f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff000102030405060708090a0b0c0d0e0f",
    "101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f20000000000000006061626364656667",
    "68696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f4000000000000000202122232425262728292a2b2c2d2e2f",
    "303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
    "2000000000000000707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f4000000000000000",
    "505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f",
    "808182838485868788898a8b8c8d8e8f2000000000000000808182838485868788898a8b8c8d8e8f9091929394959697",
    "98999a9b9c9d9e9f4000000000000000808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
    "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf00",
);

/// IP-S0 × IP-R-EMPTY — 57 bytes. `Some("")` = tag `01` + an 8-byte zero length.
///
/// THE AMBIGUITY PARTITION. Those 9 tail bytes begin `01 00 00 00 00 00 00 00`,
/// which a `u64`-tail decoder reads as `valid_before = 1` and ACCEPTS. Shape
/// confusion here is silent, not loud — see
/// `req_176_m1a_wire_the_u64_tail_shape_is_ambiguous_with_the_head_shape`.
const GOLD_S0_EMPTY: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0000000000000000",
    "010000000000000000",
);

/// IP-S1 × IP-R-TEXT — 177 bytes.
const GOLD_S1_TEXT: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0100000000000000",
    "2000000000000000404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f4000000000000000",
    "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeef",
    "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff010800000000000000726f746174696f6e",
);

/// IP-S5 × IP-R-TEXT — 625 bytes.
const GOLD_S5_TEXT: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0500000000000000",
    "2000000000000000404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f4000000000000000",
    "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeef",
    "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff2000000000000000505152535455565758595a5b5c5d5e5f6061626364656667",
    "68696a6b6c6d6e6f4000000000000000f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff000102030405060708090a0b0c0d0e0f",
    "101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f20000000000000006061626364656667",
    "68696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f4000000000000000202122232425262728292a2b2c2d2e2f",
    "303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
    "2000000000000000707172737475767778797a7b7c7d7e7f808182838485868788898a8b8c8d8e8f4000000000000000",
    "505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f",
    "808182838485868788898a8b8c8d8e8f2000000000000000808182838485868788898a8b8c8d8e8f9091929394959697",
    "98999a9b9c9d9e9f4000000000000000808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
    "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf010800000000000000726f746174696f",
    "6e",
);

/// IP-S0 × IP-R-UNICODE — 72 bytes. `"rotación 🔑"` = 15 UTF-8 bytes, 10 chars.
const GOLD_S0_UNICODE: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0000000000000000",
    "010f00000000000000726f74c3a16369c3b36e20f09f9491",
);

/// IP-ORDER × IP-R-NONE — 273 bytes. `new(target, [entry(0x80), entry(0x40)])`.
///
/// The high-seeded entry is FIRST. HEAD stores what the caller gave it; a
/// constructor that sorted would emit `0x40` first and miss this literal.
const GOLD_CTOR_NEW_DESC: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0200000000000000",
    "2000000000000000808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f4000000000000000",
    "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeaf",
    "b0b1b2b3b4b5b6b7b8b9babbbcbdbebf2000000000000000404142434445464748494a4b4c4d4e4f5051525354555657",
    "58595a5b5c5d5e5f4000000000000000c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
    "e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff00",
);

/// IP-ORDER × IP-R-TEXT — 289 bytes. `with_reason(target, [0x80, 0x40], "rotation")`.
const GOLD_CTOR_REASON_DESC: &str = concat!(
    "2000000000000000202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f0200000000000000",
    "2000000000000000808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f4000000000000000",
    "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeaf",
    "b0b1b2b3b4b5b6b7b8b9babbbcbdbebf2000000000000000404142434445464748494a4b4c4d4e4f5051525354555657",
    "58595a5b5c5d5e5f4000000000000000c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
    "e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff010800000000000000726f746174696f",
    "6e",
);

/// IP-CHAIN — **REAL ON-CHAIN BYTES**. Local testnet block 136690, transaction
/// `62a3bfbd388a208d98d1b3ebb35757426358d1fb3730112297b12eb69bf8bc81`,
/// `txType = add_maintainer`, `extra_data` = 385 bytes.
///
/// Read out on 2026-08-12 via `getBlockRaw(136690)` on `127.0.0.1:8500`
/// (read-only, local testnet — never mainnet, never a remote host) and decoded with
/// `Block::deserialize` at HEAD. Target `3047e96b…7602`, 3 signatures,
/// `reason = None`, final byte `0x00`.
const ONCHAIN_136690_ADD_MAINTAINER: &str = concat!(
    "20000000000000003047e96b13276dd92ef5eb2d6396e66c29909217f11f8c0544ea7d76a76c76020300000000000000",
    "2000000000000000202047256a8072a8b8f476691b9a5ae87710cc545e8707ca9fe0c803c3e6d3df4000000000000000",
    "da6a37df8333c1a51496f1b64368aab0268cce9003d93a146bdce5ab49bbf5c2b1945dc16deae2936df8cfbb4b5453e5",
    "823a9936a026023a9a59fc5c2b76f0012000000000000000effe88fefb6d992a1329277a1d49c7296d252bbc368319cb",
    "4bc061119926272b4000000000000000527000b0219e125e1941753527cf2d1c4dc54baeb66af06516c5071d0fd2d149",
    "effa12cdcccf9251a9a32b156ca0369f56655b6b591c59ee4c33bc72baa39f06200000000000000054323cefd0eabac8",
    "9b2a2198c95a8f261598c341a8e579a05e26322325c48c2b400000000000000071b00c5026920a9d2a5c5af668a2a92d",
    "732e891dd969ae0e226c1bfe2bfec822ce86596afe7580db702fd4ab3aaa844f65ecee512c66ba1b174fbb0979e55d07",
    "00",
);

/// The target public key inside `ONCHAIN_136690_ADD_MAINTAINER`, restated
/// independently so the decode assertion has something to compare against that
/// did not come out of the decoder.
const ONCHAIN_136690_TARGET_HEX: &str =
    "3047e96b13276dd92ef5eb2d6396e66c29909217f11f8c0544ea7d76a76c7602";

// ---------------------------------------------------------------------------
// Fixture builders. TWO families, and the difference is load-bearing.
//
// RAW (`raw_pk` / `raw_sig`): fixed byte patterns, matching the generator run at
// HEAD. Only raw bytes enter the ENCODING and `PublicKey::from_bytes` does not
// validate the curve point, so a hex-editor-reproducible pattern is the right
// fixture for a frozen golden.
//
// MEASURED CONSEQUENCE — these are ENCODE-ONLY. `PublicKey`'s Deserialize impl
// DOES validate: feeding a golden back through `from_bytes` fails with
// `Custom("invalid key bytes: not a valid Ed25519 key")`, i.e. for a reason that
// has nothing to do with the payload shape.
//
// REAL (`real_pk` / `real_entry`): deterministic `KeyPair::from_seed` keys. Every
// test that exercises `from_bytes` uses these, so a `None` is attributable to the
// SHAPE and the regression lock cannot pass for the wrong reason.
// ---------------------------------------------------------------------------

fn raw_pk(seed: u8) -> PublicKey {
    let mut b = [0u8; 32];
    for (i, x) in b.iter_mut().enumerate() {
        *x = seed.wrapping_add(i as u8);
    }
    PublicKey::from_bytes(b)
}

fn raw_sig(seed: u8) -> Signature {
    let mut b = [0u8; 64];
    for (i, x) in b.iter_mut().enumerate() {
        *x = seed.wrapping_mul(3).wrapping_add(i as u8);
    }
    Signature::from_bytes(b)
}

fn entry(seed: u8) -> MaintainerSignature {
    MaintainerSignature::new(raw_pk(seed), raw_sig(seed))
}

fn target() -> PublicKey {
    raw_pk(0x20)
}

fn five_entries() -> Vec<MaintainerSignature> {
    vec![
        entry(0x40),
        entry(0x50),
        entry(0x60),
        entry(0x70),
        entry(0x80),
    ]
}

/// A REAL, on-curve public key. Deterministic seed so payloads stay byte-stable.
fn real_pk(seed: u8) -> PublicKey {
    *KeyPair::from_seed([seed; 32]).public_key()
}

/// A signature entry whose PUBKEY is on-curve. The signature bytes themselves are
/// never verified by anything in this file — `to_bytes` / `from_bytes` only frame
/// them — so a default signature is exactly as representative and keeps sizes exact.
fn real_entry(seed: u8) -> MaintainerSignature {
    MaintainerSignature::new(real_pk(seed), Signature::default())
}

fn real_five() -> Vec<MaintainerSignature> {
    vec![
        real_entry(0x41),
        real_entry(0x52),
        real_entry(0x63),
        real_entry(0x74),
        real_entry(0x85),
    ]
}

/// Decode-capable payload: on-curve target, on-curve signer keys.
fn real_payload(
    signatures: Vec<MaintainerSignature>,
    reason: Option<&str>,
) -> MaintainerChangeData {
    MaintainerChangeData {
        target: real_pk(0x21),
        signatures,
        reason: reason.map(str::to_string),
    }
}

/// Struct-literal construction. Deliberately NOT via a constructor: this
/// separates "the ENCODER is frozen" from "the CONSTRUCTORS are frozen", so a
/// failure names which one moved.
fn payload(signatures: Vec<MaintainerSignature>, reason: Option<&str>) -> MaintainerChangeData {
    MaintainerChangeData {
        target: target(),
        signatures,
        reason: reason.map(str::to_string),
    }
}

fn unhex(s: &str) -> Vec<u8> {
    hex::decode(s).expect("golden literal must be valid hex")
}

/// An INDEPENDENT mirror of HEAD's field list. It never references
/// `MaintainerChangeData`, so if a field is added, removed or reordered there,
/// the two stop agreeing — which is the whole point.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct HeadShapeMirror {
    target: PublicKey,
    signatures: Vec<MaintainerSignature>,
    reason: Option<String>,
}

// ===========================================================================
// ACCEPTANCE CRITERION 1 — the encoding is byte-identical to HEAD
// ===========================================================================

/// W-O3, W-O3b — `to_bytes` reproduces the frozen HEAD literal for every
/// partition of (signature count × reason shape).
///
/// The golden literals came out of HEAD's real encoder. If ANY field is added,
/// removed, widened or reordered, every one of these misses — including the
/// `reason = None` cases, whose final `0x00` is the byte attempt 1 destroyed.
#[test]
fn req_176_m1a_wire_bincode_encoding_is_byte_identical_to_head() {
    let cases: [(&str, MaintainerChangeData, &str); 7] = [
        ("IP-S0 x IP-R-NONE", payload(vec![], None), GOLD_S0_NONE),
        (
            "IP-S1 x IP-R-NONE",
            payload(vec![entry(0x40)], None),
            GOLD_S1_NONE,
        ),
        (
            "IP-S5 x IP-R-NONE",
            payload(five_entries(), None),
            GOLD_S5_NONE,
        ),
        (
            "IP-S0 x IP-R-EMPTY",
            payload(vec![], Some("")),
            GOLD_S0_EMPTY,
        ),
        (
            "IP-S1 x IP-R-TEXT",
            payload(vec![entry(0x40)], Some("rotation")),
            GOLD_S1_TEXT,
        ),
        (
            "IP-S5 x IP-R-TEXT",
            payload(five_entries(), Some("rotation")),
            GOLD_S5_TEXT,
        ),
        (
            "IP-S0 x IP-R-UNICODE",
            payload(vec![], Some("rot\u{e1}ci\u{f3}n \u{1f511}")),
            GOLD_S0_UNICODE,
        ),
    ];

    for (name, data, golden_hex) in cases {
        let got = data.to_bytes();
        let want = unhex(golden_hex);

        assert_eq!(
            got.len(),
            want.len(),
            "W-O3b {name}: the ENCODED LENGTH moved (HEAD {} -> now {}). extra_data feeds the \
             txid, and `from_bytes` is consumed fatally and UNGATED at \
             validation/tx_types.rs:809 — a length change is a chain-history break, not a \
             refactor. M1a must move ZERO bytes.",
            want.len(),
            got.len()
        );
        assert_eq!(
            hex::encode(&got),
            hex::encode(&want),
            "W-O3 {name}: the bincode encoding of MaintainerChangeData is NOT byte-identical to \
             HEAD (3f8bf185). Every add_maintainer/remove_maintainer payload ever written to a \
             DOLI chain is decoded by this format with NO height gate. Any wire change needs its \
             own activation height and an explicit format discriminator — that is milestone \
             M2.5, not M1a."
        );
    }
}

/// Y-O3 — the CONSTRUCTORS are frozen too: `new` defaults `reason` to `None`,
/// `with_reason` stores the string, and NEITHER reorders `signatures`.
///
/// Split from the encoder test on purpose. Attempt 1 changed both the field list
/// AND added a canonical sort in the constructor. A sort emits the same FORMAT
/// but different BYTES for the same caller input, which changes the txid of every
/// authorization a node or CLI builds. Zero wire change means zero here as well.
#[test]
fn req_176_m1a_wire_constructors_produce_head_identical_bytes() {
    let descending = vec![entry(0x80), entry(0x40)];

    let built = MaintainerChangeData::new(target(), descending.clone());
    assert_eq!(
        built.reason, None,
        "Y-O3: `new` must leave `reason` at `None`, exactly as HEAD does — that `None` is the \
         1-byte tail every on-chain payload carries"
    );
    assert_eq!(
        hex::encode(built.to_bytes()),
        hex::encode(unhex(GOLD_CTOR_NEW_DESC)),
        "Y-O3 / IP-ORDER: `new` must store the signature vector in CALLER ORDER and encode \
         byte-identically to HEAD. If this fails with the right bytes in the wrong order, a \
         canonicalizing sort was added — it changes the txid of every constructed authorization \
         and belongs behind an activation height, not in M1a."
    );

    let with_reason =
        MaintainerChangeData::with_reason(target(), descending, "rotation".to_string());
    assert_eq!(
        with_reason.reason.as_deref(),
        Some("rotation"),
        "Y-O3: `with_reason` must still exist and still store the string"
    );
    assert_eq!(
        hex::encode(with_reason.to_bytes()),
        hex::encode(unhex(GOLD_CTOR_REASON_DESC)),
        "Y-O3 / IP-ORDER: `with_reason` must encode byte-identically to HEAD"
    );
}

/// X-O3, X-O3b — `from_bytes(to_bytes(x)) == x` for every partition, including
/// the byte-length extremes.
///
/// Round-tripping is what proves `reason` is genuinely carried rather than
/// accepted and dropped. IP-R-MAX exercises the largest legal payload.
#[test]
fn req_176_m1a_wire_round_trip_preserves_every_field() {
    let max_reason = "x".repeat(MAX_MAINTAINER_CHANGE_REASON_BYTES);
    // REAL keys: `PublicKey` deserialization validates the curve point, so a raw
    // fixture would fail here for a reason unrelated to the payload shape.
    let cases = vec![
        real_payload(vec![], None),
        real_payload(vec![], Some("")),
        real_payload(vec![real_entry(0x41)], Some("rotation")),
        real_payload(real_five(), None),
        real_payload(real_five(), Some("rot\u{e1}ci\u{f3}n \u{1f511}")),
        real_payload(real_five(), Some(max_reason.as_str())),
    ];

    for original in cases {
        let bytes = original.to_bytes();
        let decoded = MaintainerChangeData::from_bytes(&bytes)
            .expect("X-O3: a payload this encoder just produced must decode");

        assert_eq!(
            decoded,
            original,
            "X-O3b: the round trip must preserve target, signatures (count, order and bytes) and \
             reason. reason_len={:?}, sigs={}",
            original.reason.as_ref().map(String::len),
            original.signatures.len()
        );
        assert_eq!(
            decoded.reason, original.reason,
            "X-O3b: `reason` specifically — it is the field attempt 1 deleted, so it gets its own \
             assertion rather than hiding inside the struct comparison"
        );
    }
}

// ===========================================================================
// ACCEPTANCE CRITERION 2 — THE REGRESSION ATTEMPT 1 FAILED
// ===========================================================================

/// **THE REGRESSION LOCK.** The real `add_maintainer` payload mined into local
/// testnet block 136690 must still decode.
///
/// These are the REAL ON-CHAIN BYTES (see the module header for provenance), not
/// a reconstruction. Attempt 1's binary returns `None` here, and
/// `crates/core/src/validation/tx_types.rs:809` turns that into a hard reject
/// with NO height gate, so the node cannot sync past this block. In
/// `ValidationMode::Replay` the reject is tolerated and
/// `apply_block/governance.rs` then SKIPS the maintainer add instead — a silent
/// trust-root divergence, which is worse.
///
/// If this test fails, do not touch it. Revert the payload change.
#[test]
fn req_176_wire_testnet_block_136690_add_maintainer_payload_still_decodes() {
    let bytes = unhex(ONCHAIN_136690_ADD_MAINTAINER);

    assert_eq!(
        bytes.len(),
        385,
        "fixture: the on-chain payload is 385 bytes (tx wire size 417). If this literal was \
         edited, the whole test is worthless."
    );
    assert_eq!(
        *bytes.last().expect("non-empty"),
        0x00u8,
        "fixture / THE INCIDENT IN ONE BYTE: the payload ends in the 1-byte bincode `Option::None` \
         tag. A `u64` field in that position needs EIGHT bytes, so the decoder EOFs."
    );

    let decoded = MaintainerChangeData::from_bytes(&bytes).expect(
        "X-O3: REGRESSION — the real on-chain add_maintainer payload from local testnet block \
         136690 (tx 62a3bfbd..bc81) NO LONGER DECODES. This is exactly the failure that got M1 \
         attempt 1 rejected: from_bytes is consumed fatally and UNGATED at \
         validation/tx_types.rs:809, so this binary cannot sync past block 136690 in either \
         deploy direction, and a full sync from genesis re-hits it forever. REVERT the payload \
         change; do not relax this test.",
    );

    assert_eq!(
        decoded.target.to_hex(),
        ONCHAIN_136690_TARGET_HEX,
        "X-O3b: the decoded target must be the key the chain actually authorized"
    );
    assert_eq!(
        decoded.signatures.len(),
        3,
        "X-O3b: the on-chain authorization carries 3 maintainer signatures"
    );
    assert!(
        decoded.signatures.len() <= MAX_MAINTAINER_CHANGE_SIGNATURES,
        "X-O3b: a real on-chain payload must satisfy INC-I-173's F5 signature cap"
    );
    assert_eq!(
        decoded.reason, None,
        "X-O3b: the on-chain payload carries `reason = None`. It was built by the 2-arity \
         `new_add_maintainer`, which is why the tail is one byte."
    );
    assert!(
        bytes.len() <= MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
        "X-O3b: a real on-chain payload must satisfy INC-I-173's F5 outer cap"
    );

    // Re-encoding must reproduce the chain's bytes exactly. Decode-only equality
    // would still permit an encoder drift that silently rewrites history on
    // rebuild.
    assert_eq!(
        hex::encode(decoded.to_bytes()),
        ONCHAIN_136690_ADD_MAINTAINER,
        "W-O3 / X-O3b: re-encoding the decoded on-chain payload must reproduce the chain's bytes \
         EXACTLY — the encoder and the decoder must agree on history, not merely tolerate it"
    );
}

/// Z — the field list is exactly `(target, signatures, reason)`, in that order.
///
/// Proven with an independent mirror struct rather than by reading `data.rs`, so
/// it holds even if someone regenerates the goldens from the implementation. Both
/// directions are asserted: the mirror must encode to the real bytes AND the real
/// decoder must accept the mirror's bytes.
#[test]
fn req_176_m1a_wire_payload_field_list_is_frozen_at_three_fields() {
    // ENCODE direction — raw fixtures are fine, serialization does not validate.
    let raw_real = payload(vec![entry(0x40)], Some("rotation"));
    let raw_mirror = HeadShapeMirror {
        target: target(),
        signatures: vec![entry(0x40)],
        reason: Some("rotation".to_string()),
    };
    assert_eq!(
        hex::encode(bincode::serialize(&raw_mirror).expect("mirror encodes")),
        hex::encode(raw_real.to_bytes()),
        "Z: MaintainerChangeData must encode exactly like `(target, signatures, reason)`. A \
         mismatch means a field was added, removed, reordered or re-typed."
    );

    // DECODE direction — REAL keys, because `PublicKey` validates on deserialize.
    let real = real_payload(vec![real_entry(0x41)], Some("rotation"));
    let mirror = HeadShapeMirror {
        target: real_pk(0x21),
        signatures: vec![real_entry(0x41)],
        reason: Some("rotation".to_string()),
    };
    let via_mirror = bincode::serialize(&mirror).expect("mirror encodes");
    assert_eq!(
        hex::encode(&via_mirror),
        hex::encode(real.to_bytes()),
        "Z: encode parity must also hold for on-curve key material"
    );
    let decoded = MaintainerChangeData::from_bytes(&via_mirror)
        .expect("Z: the real decoder must accept the frozen three-field shape");
    assert_eq!(decoded, real, "Z: and must decode it to the same value");

    // Anchor the mirror to REALITY, not to another fixture: it must read the same
    // three fields out of the bytes the chain actually stored.
    let onchain = unhex(ONCHAIN_136690_ADD_MAINTAINER);
    let mirror_onchain: HeadShapeMirror =
        bincode::deserialize(&onchain).expect("Z: the mirror must decode the REAL on-chain bytes");
    assert_eq!(
        mirror_onchain.reason, None,
        "Z: and read the same `reason = None` the real decoder reads"
    );
    assert_eq!(
        mirror_onchain.target.to_hex(),
        ONCHAIN_136690_TARGET_HEX,
        "Z: and the same target — proving the field ORDER matches the chain, not just the count"
    );
    assert_eq!(
        mirror_onchain.signatures.len(),
        3,
        "Z: and the same signature count"
    );
}

/// INC-I-173 cross-check — the maximal legal payload still fits under the F5
/// outer cap, and the reason cap still exists.
///
/// This does NOT replace `crates/core/tests/inc_i_173_m3_payload_bounds.rs`,
/// which returns to HEAD untouched. It is the cheap tripwire that says "the
/// shape INC-I-173 measured has not moved", measured through the real encoder
/// rather than restated as prose.
#[test]
fn req_176_m1a_wire_maximal_legal_payload_still_fits_under_the_inc_i_173_cap() {
    assert_eq!(
        MAX_MAINTAINER_CHANGE_REASON_BYTES, 256,
        "the `reason` byte cap must survive M1a — attempt 1 deleted it along with the field"
    );
    assert_eq!(MAX_MAINTAINER_CHANGE_SIGNATURES, 5);
    assert_eq!(MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES, 1024);

    let maximal = payload(
        five_entries(),
        Some(&"x".repeat(MAX_MAINTAINER_CHANGE_REASON_BYTES)),
    );
    let encoded = maximal.to_bytes();

    assert_eq!(
        encoded.len(),
        873,
        "the maximal legal payload measured 873 bytes at HEAD (609 for 5 signatures with a \
         `None` tail, minus that 1 byte, plus 1 tag + 8 length + 256 content). A different \
         number means the payload SHAPE moved."
    );
    assert!(
        encoded.len() <= MAX_MAINTAINER_CHANGE_EXTRA_DATA_BYTES,
        "INC-I-173 F5: a 5-of-5 rotation with a maximal reason must stay mineable — a payload \
         that satisfies both inner caps and still fails the outer one is permanently unmineable, \
         which is the INC-I-173 bug class itself"
    );
}
