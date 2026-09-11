//! INC-I-217 M9 — REQ-ROT-012: a queued BLS rotation must be visible in the
//! `getProducer` / `getProducers` `pendingUpdates` objects, and the seven
//! pre-existing kinds must keep the exact JSON shape the explorer reads today.
//!
//! OUTPUT CONTRACT — `pending_update_to_info(&PendingProducerUpdate, best_height,
//!   blocks_per_epoch) -> PendingUpdateInfo`, observed through `serde_json::to_value`
//!   O1 `updateType`         the kind string
//!   O2 `bondCount`          present only for the kinds that carry a count
//!   O3 `newBlsPubkey`       lowercase hex of the queued 48-byte key
//!   O4 `effectiveAtHeight`  the height the queue actually flushes at
//!   O5 the KEY SET of the emitted object — absence IS an output here, because
//!      both new fields are `skip_serializing_if = "Option::is_none"` and an
//!      explorer that sees a new key on an old kind is a shape break
//!   O6 mutable params / store writes / process statics / channels — NONE: the
//!      mapper takes `&PendingProducerUpdate` and two scalars and returns a value
//! PATHS
//!   P1 the 7 pre-existing kinds
//!   P2 `RotateBlsKey`
//!   P3 the flush-height derivation INSIDE epoch 0 (the queue flushes every block)
//!   P4 the flush-height derivation ABOVE epoch 0 (only at a boundary)
//! INPUT PARTITIONS: `best_height` at 0 / mid epoch 0 / the last height of epoch 0 /
//!   one below a boundary / exactly at a boundary / one above; two DISTINCT 48-byte keys.
//! MATRIX
//!   P1: O1 ✓ O2 ✓ O5 ✓  (O3 and O4 asserted ABSENT, not null)
//!   P2: O1 ✓ O3 ✓ O4 ✓ O5 ✓
//!   P3: O4 ✓ at three heights inside epoch 0
//!   P4: O4 ✓ at one below / exactly at / one above a boundary
//!   O3 non-vacuity: two distinct keys through the same mapper must differ
//!
//! `flush_height_reference` is a forward scan over the LIVE predicate of
//! `bins/node/src/node/apply_block/state_update.rs` (`is_epoch_0 || is_boundary`),
//! not a closed form: a closed form would restate the derivation instead of checking it.

use super::*;
use doli_core::EpochSnapshot;
use serde_json::{json, Value};
use storage::{PendingProducerUpdate, ProducerInfo};

/// Mainnet/testnet `blocks_per_reward_epoch`. Every derivation partition below is
/// expressed relative to this value, never as a bare number.
const BPE: u64 = 360;

/// A distinctive 48-byte queued key: a hard-coded or empty `new_bls_pubkey` cannot
/// reproduce it, and no byte repeats in a way that hides a truncation.
const KEY_A_HEX: &str = concat!(
    "000102030405060708090a0b0c0d0e0f",
    "101112131415161718191a1b1c1d1e1f",
    "202122232425262728292a2b2c2d2e2f",
);

fn key_a() -> Vec<u8> {
    (0u8..48).collect()
}

fn key_b() -> Vec<u8> {
    vec![0xb7u8; 48]
}

fn a_pubkey() -> crypto::PublicKey {
    *crypto::KeyPair::generate().public_key()
}

fn a_producer_info(pubkey: crypto::PublicKey) -> ProducerInfo {
    ProducerInfo::new(pubkey, 10, 1_000_000_000, (crypto::Hash::ZERO, 0), 0, 0)
}

/// The height `state_update.rs` will actually flush the queue at, found by walking
/// candidate heights through the SAME two conditions the node evaluates per block.
fn flush_height_reference(best_height: u64, blocks_per_epoch: u64) -> u64 {
    let mut height = best_height + 1;
    loop {
        let is_epoch_0 = height < blocks_per_epoch;
        let is_boundary = EpochSnapshot::is_epoch_boundary_with(height, blocks_per_epoch);
        if is_epoch_0 || is_boundary {
            return height;
        }
        height += 1;
    }
}

fn mapped(update: &PendingProducerUpdate, best_height: u64) -> Value {
    serde_json::to_value(pending_update_to_info(update, best_height, BPE))
        .expect("PendingUpdateInfo must serialise")
}

fn keys_of(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("a PendingUpdateInfo serialises to a JSON object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// The seven kinds that existed before M9, each with the object the explorer reads.
fn pre_existing_kinds() -> Vec<(&'static str, PendingProducerUpdate, Value)> {
    let pubkey = a_pubkey();
    let delegate = a_pubkey();
    vec![
        (
            "Register",
            PendingProducerUpdate::Register {
                info: Box::new(a_producer_info(pubkey)),
                height: 11,
            },
            json!({ "updateType": "register" }),
        ),
        (
            "Exit",
            PendingProducerUpdate::Exit { pubkey, height: 12 },
            json!({ "updateType": "exit" }),
        ),
        (
            "Slash",
            PendingProducerUpdate::Slash { pubkey, height: 13 },
            json!({ "updateType": "slash" }),
        ),
        (
            "AddBond",
            PendingProducerUpdate::AddBond {
                pubkey,
                outpoints: vec![
                    (crypto::Hash::ZERO, 0),
                    (crypto::Hash::ZERO, 1),
                    (crypto::Hash::ZERO, 2),
                ],
                bond_unit: 1_000_000_000,
                creation_slot: 77,
            },
            json!({ "updateType": "add_bond", "bondCount": 3 }),
        ),
        (
            "DelegateBond",
            PendingProducerUpdate::DelegateBond {
                delegator: pubkey,
                delegate,
                bond_count: 7,
            },
            json!({ "updateType": "delegate_bond", "bondCount": 7 }),
        ),
        (
            "RevokeDelegation",
            PendingProducerUpdate::RevokeDelegation { delegator: pubkey },
            json!({ "updateType": "revoke_delegation" }),
        ),
        (
            "RequestWithdrawal",
            PendingProducerUpdate::RequestWithdrawal {
                pubkey,
                bond_count: 2,
                bond_unit: 1_000_000_000,
            },
            json!({ "updateType": "withdrawal", "bondCount": 2 }),
        ),
    ]
}

// REQ-ROT-012 (Must) — Decision: a failure here means M9 changed the JSON the explorer
// already parses for the seven shipped pending-update kinds, so the running explorer
// starts reading an object it was never written against — the break is invisible in the
// node's own tests and only shows up as a broken producer page in production.
#[test]
fn req_rot_012_seven_pre_existing_kinds_serialise_unchanged() {
    for (name, update, expected) in pre_existing_kinds() {
        let got = mapped(&update, 719);
        assert_eq!(
            got, expected,
            "{name} must serialise to the pre-M9 object, byte for byte"
        );
        let keys = keys_of(&got);
        assert!(
            !keys.contains(&"newBlsPubkey".to_string()),
            "{name} must not emit newBlsPubkey at all (absent, never null): got {keys:?}"
        );
        assert!(
            !keys.contains(&"effectiveAtHeight".to_string()),
            "{name} must not emit effectiveAtHeight at all (absent, never null): got {keys:?}"
        );
    }
    println!("ROT-RPC-SHAPE-FROZEN 7/7 pre-existing pendingUpdates kinds unchanged");
}

// REQ-ROT-012 (Must) — Decision: a failure here means the update_type discriminator the
// explorer switches on drifted, so every consumer that matched on the old string silently
// falls through to its unknown-kind branch while the node believes it published the update.
#[test]
fn req_rot_012_update_type_strings_are_the_shipped_seven() {
    let expected = [
        "register",
        "exit",
        "slash",
        "add_bond",
        "delegate_bond",
        "revoke_delegation",
        "withdrawal",
    ];
    let got: Vec<String> = pre_existing_kinds()
        .iter()
        .map(|(_, update, _)| {
            mapped(update, 719)["updateType"]
                .as_str()
                .expect("updateType is a string")
                .to_string()
        })
        .collect();
    assert_eq!(got, expected, "the seven shipped update_type strings");
}

// REQ-ROT-012 (Must) — Decision: a failure here means a rotation is queued on chain but
// invisible to the explorer and to the CLI that reads getProducers, so an operator cannot
// tell a pending rotation from a lost transaction and re-sends one that double-rotates.
#[test]
fn req_rot_012_rotate_bls_key_carries_new_key_and_effective_height() {
    let best_height = 719;
    let update = PendingProducerUpdate::RotateBlsKey {
        pubkey: a_pubkey(),
        new_bls_pubkey: key_a(),
        height: 700,
    };
    let got = mapped(&update, best_height);
    let expected_height = flush_height_reference(best_height, BPE);
    assert_eq!(
        got,
        json!({
            "updateType": "rotate_bls_key",
            "newBlsPubkey": KEY_A_HEX,
            "effectiveAtHeight": expected_height,
        }),
        "RotateBlsKey must publish the queued key and the flush height, and nothing else"
    );
    println!(
        "ROT-RPC-NEW-KEY newBlsPubkey={KEY_A_HEX} effectiveAtHeight={expected_height} \
         bestHeight={best_height} blocksPerEpoch={BPE}"
    );
}

// REQ-ROT-012 (Must) — Decision: a failure here means the mapper hex-encodes something
// other than the queued key — a constant, a truncation, or the OLD key — so an operator
// who compares the published key against the one their wallet holds gets a false match.
#[test]
fn req_rot_012_new_bls_pubkey_is_the_queued_key_not_a_constant() {
    let pubkey = a_pubkey();
    let mapped_a = mapped(
        &PendingProducerUpdate::RotateBlsKey {
            pubkey,
            new_bls_pubkey: key_a(),
            height: 700,
        },
        719,
    );
    let mapped_b = mapped(
        &PendingProducerUpdate::RotateBlsKey {
            pubkey,
            new_bls_pubkey: key_b(),
            height: 700,
        },
        719,
    );
    let hex_a = mapped_a["newBlsPubkey"].as_str().expect("hex string");
    let hex_b = mapped_b["newBlsPubkey"].as_str().expect("hex string");
    assert_eq!(hex_a, KEY_A_HEX, "the ascending key, all 48 bytes");
    assert_eq!(hex_b, "b7".repeat(48), "the constant key, all 48 bytes");
    assert_ne!(
        hex_a, hex_b,
        "two distinct queued keys must map to two distinct hex strings"
    );
    assert_eq!(hex_a.len(), 96, "48 bytes is 96 lowercase hex characters");
    assert_eq!(
        hex_a,
        hex_a.to_lowercase(),
        "the hex must be lowercase: the explorer compares it against `doli` CLI output"
    );
}

// REQ-ROT-012 (Must) — Decision: a failure here means the RPC advertises a flush height
// inside epoch 0 that the node will not flush at; during epoch 0 the queue drains EVERY
// block, so an operator told to wait for a boundary waits through a rotation that already
// happened and concludes the feature is broken.
#[test]
fn req_rot_012_effective_height_inside_epoch_zero_matches_the_flush_predicate() {
    for best_height in [0u64, 1, BPE / 2, BPE - 2] {
        let update = PendingProducerUpdate::RotateBlsKey {
            pubkey: a_pubkey(),
            new_bls_pubkey: key_a(),
            height: best_height,
        };
        let got = mapped(&update, best_height)["effectiveAtHeight"]
            .as_u64()
            .expect("effectiveAtHeight is a number");
        let reference = flush_height_reference(best_height, BPE);
        assert_eq!(
            got, reference,
            "inside epoch 0 the queue flushes at the very next block \
             (best_height={best_height}, blocks_per_epoch={BPE})"
        );
        assert_eq!(
            got,
            best_height + 1,
            "the epoch-0 branch of the live predicate is `height < blocks_per_epoch`"
        );
    }
}

// REQ-ROT-012 (Must) — Decision: a failure here means the published height is off by one
// epoch or one block around a boundary; the explorer then shows a rotation as "effective"
// while the node still signs with the old BLS key, which is exactly the attestation-credit
// confusion the milestone exists to remove.
#[test]
fn req_rot_012_effective_height_above_epoch_zero_matches_the_flush_predicate() {
    let boundary = BPE * 2;
    for best_height in [boundary - 1, boundary, boundary + 1, BPE - 1] {
        let update = PendingProducerUpdate::RotateBlsKey {
            pubkey: a_pubkey(),
            new_bls_pubkey: key_a(),
            height: best_height,
        };
        let got = mapped(&update, best_height)["effectiveAtHeight"]
            .as_u64()
            .expect("effectiveAtHeight is a number");
        let reference = flush_height_reference(best_height, BPE);
        assert_eq!(
            got, reference,
            "above epoch 0 the queue flushes only at a boundary \
             (best_height={best_height}, blocks_per_epoch={BPE})"
        );
        assert!(
            got > best_height,
            "a flush height at or below the tip is a height the node has already passed"
        );
        assert!(
            EpochSnapshot::is_epoch_boundary_with(got, BPE),
            "above epoch 0 every published flush height must BE an epoch boundary"
        );
    }
}
