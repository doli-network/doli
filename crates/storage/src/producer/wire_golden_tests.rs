//! Golden vectors freezing the persisted producer surfaces.
//!
//! covers: REQ-ROT-001
//!
//! Two surfaces: the serde NAME TAG of every `PendingProducerUpdate` variant
//! (written into `producers.bin` JSON by `ProducerSet::save`/`load`) and the
//! byte layout of `serialize_canonical()`, which feeds the state root.

use crypto::{hash::hash, Hash, KeyPair, PublicKey};

use super::constants::BOND_UNIT;
use super::types::{PendingProducerUpdate, ProducerInfo, ProducerSet};

const PENDING_UPDATE_VARIANT_COUNT: usize = 7;

const CANONICAL_LEN: usize = 652;
const CANONICAL_HASH_HEX: &str = "9638dad997fc8ddb24c1e122b39cf71c5b7023db52e0efb025701a73bd08099c";

fn pk(byte: u8) -> PublicKey {
    *KeyPair::from_seed([byte; 32]).public_key()
}

fn h(byte: u8) -> Hash {
    Hash::from_bytes([byte; 32])
}

/// Exhaustive — NO `_` arm. An 8th variant fails the build here.
fn serde_tag(update: &PendingProducerUpdate) -> &'static str {
    match update {
        PendingProducerUpdate::Register { .. } => "Register",
        PendingProducerUpdate::Exit { .. } => "Exit",
        PendingProducerUpdate::Slash { .. } => "Slash",
        PendingProducerUpdate::AddBond { .. } => "AddBond",
        PendingProducerUpdate::DelegateBond { .. } => "DelegateBond",
        PendingProducerUpdate::RevokeDelegation { .. } => "RevokeDelegation",
        PendingProducerUpdate::RequestWithdrawal { .. } => "RequestWithdrawal",
    }
}

fn golden_producer_info() -> ProducerInfo {
    ProducerInfo::new(pk(0x01), 100, BOND_UNIT, (h(0x0A), 1), 0, BOND_UNIT)
}

fn all_pending_updates() -> Vec<PendingProducerUpdate> {
    vec![
        PendingProducerUpdate::Register {
            info: Box::new(golden_producer_info()),
            height: 100,
        },
        PendingProducerUpdate::Exit {
            pubkey: pk(0x02),
            height: 200,
        },
        PendingProducerUpdate::Slash {
            pubkey: pk(0x03),
            height: 300,
        },
        PendingProducerUpdate::AddBond {
            pubkey: pk(0x04),
            outpoints: vec![(h(0x0B), 2)],
            bond_unit: BOND_UNIT,
            creation_slot: 400,
        },
        PendingProducerUpdate::DelegateBond {
            delegator: pk(0x05),
            delegate: pk(0x06),
            bond_count: 5,
        },
        PendingProducerUpdate::RevokeDelegation {
            delegator: pk(0x07),
        },
        PendingProducerUpdate::RequestWithdrawal {
            pubkey: pk(0x08),
            bond_count: 3,
            bond_unit: BOND_UNIT,
        },
    ]
}

const GOLDEN_JSON: [&str; PENDING_UPDATE_VARIANT_COUNT] = [
    r#"{"Register":{"info":{"public_key":"8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c","registered_at":100,"bond_amount":1000000000,"bond_outpoint":["0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a",1],"status":"Active","blocks_produced":0,"slots_missed":0,"registration_era":0,"pending_rewards":0,"has_prior_exit":false,"last_activity":100,"activity_gaps":0,"bond_count":1,"additional_bonds":[],"delegated_to":null,"delegated_bonds":0,"received_delegations":[],"bond_entries":[{"creation_slot":100,"amount":1000000000}],"withdrawal_pending_count":0,"bls_pubkey":[]},"height":100}}"#,
    r#"{"Exit":{"pubkey":"8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394","height":200}}"#,
    r#"{"Slash":{"pubkey":"ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1","height":300}}"#,
    r#"{"AddBond":{"pubkey":"ca93ac1705187071d67b83c7ff0efe8108e8ec4530575d7726879333dbdabe7c","outpoints":[["0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",2]],"bond_unit":1000000000,"creation_slot":400}}"#,
    r#"{"DelegateBond":{"delegator":"6e7a1cdd29b0b78fd13af4c5598feff4ef2a97166e3ca6f2e4fbfccd80505bf1","delegate":"8a875fff1eb38451577acd5afee405456568dd7c89e090863a0557bc7af49f17","bond_count":5}}"#,
    r#"{"RevokeDelegation":{"delegator":"ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"}}"#,
    r#"{"RequestWithdrawal":{"pubkey":"1398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca","bond_count":3,"bond_unit":1000000000}}"#,
];

/// Fixed set: insertion order is the REVERSE of sorted order, so the canonical
/// sort is exercised rather than accidentally satisfied.
fn golden_producer_set() -> ProducerSet {
    let mut set = ProducerSet::new();

    let mut first = golden_producer_info();
    first.bls_pubkey = vec![0x5Au8; 48];
    first.blocks_produced = 42;
    set.producers.insert(h(0xF0), first);

    let mut second = ProducerInfo::new(pk(0x02), 250, BOND_UNIT * 3, (h(0x0C), 0), 1, BOND_UNIT);
    second.blocks_produced = 7;
    second.last_activity = 900;
    set.producers.insert(h(0x10), second);

    set.exit_history.insert(h(0xE1), 987_654);
    set.exit_history.insert(h(0x21), 1);

    set
}

// REQ-ROT-001 — Decision: whether a persisted variant tag was renamed, which silently drops every queued deferred mutation on the next node restart.
#[test]
fn pending_producer_update_json_golden_vectors() {
    let updates = all_pending_updates();
    assert_eq!(updates.len(), PENDING_UPDATE_VARIANT_COUNT);

    for (update, expected) in updates.iter().zip(GOLDEN_JSON.iter()) {
        let json = serde_json::to_string(update).expect("update serializes");
        assert_eq!(
            &json,
            expected,
            "persisted JSON drifted for PendingProducerUpdate::{}",
            serde_tag(update)
        );

        let tag_prefix = format!("{{\"{}\":", serde_tag(update));
        assert!(
            json.starts_with(&tag_prefix),
            "variant name tag drifted: expected outer key {tag_prefix}, got {json}"
        );

        let decoded: PendingProducerUpdate =
            serde_json::from_str(&json).expect("update round-trips");
        assert_eq!(serde_tag(&decoded), serde_tag(update));
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            *expected,
            "round-trip changed the persisted bytes"
        );
    }
}

// REQ-ROT-001 — Decision: whether an 8th deferred mutation appeared without the persisted-tag freeze being revisited.
#[test]
fn exactly_seven_pending_producer_update_variants() {
    let tags: Vec<&str> = all_pending_updates().iter().map(serde_tag).collect();
    assert_eq!(tags.len(), PENDING_UPDATE_VARIANT_COUNT);
    assert_eq!(
        tags,
        vec![
            "Register",
            "Exit",
            "Slash",
            "AddBond",
            "DelegateBond",
            "RevokeDelegation",
            "RequestWithdrawal",
        ],
        "variant order or naming changed"
    );
}

// REQ-ROT-001 — Decision: whether the producer contribution to the state root changed shape, which forks every node that did not upgrade in lockstep.
#[test]
fn serialize_canonical_golden_hash() {
    let bytes = golden_producer_set().serialize_canonical();

    assert_eq!(bytes.len(), CANONICAL_LEN, "canonical byte length drifted");
    assert_eq!(
        hash(&bytes).to_hex(),
        CANONICAL_HASH_HEX,
        "canonical producer-set encoding drifted"
    );
}

// REQ-ROT-001 — Decision: whether the golden hash actually binds the payload or would survive a real state change.
#[test]
fn serialize_canonical_golden_hash_is_not_vacuous() {
    let baseline = hash(&golden_producer_set().serialize_canonical()).to_hex();

    let mut moved_exit = golden_producer_set();
    moved_exit.exit_history.insert(h(0xE1), 987_655);
    assert_ne!(
        hash(&moved_exit.serialize_canonical()).to_hex(),
        baseline,
        "a changed exit height must change the canonical bytes"
    );

    let mut moved_bls = golden_producer_set();
    moved_bls
        .producers
        .get_mut(&h(0xF0))
        .unwrap()
        .bls_pubkey
        .fill(0x5B);
    assert_ne!(
        hash(&moved_bls.serialize_canonical()).to_hex(),
        baseline,
        "a changed BLS pubkey must change the canonical bytes"
    );
}

// REQ-ROT-001 — Decision: whether HashMap iteration order leaked into the state root, which would fork nodes that hold identical logical state.
#[test]
fn serialize_canonical_is_insertion_order_independent() {
    let forward = golden_producer_set().serialize_canonical();

    let mut reversed = ProducerSet::new();
    let mut second = ProducerInfo::new(pk(0x02), 250, BOND_UNIT * 3, (h(0x0C), 0), 1, BOND_UNIT);
    second.blocks_produced = 7;
    second.last_activity = 900;
    reversed.producers.insert(h(0x10), second);
    let mut first = golden_producer_info();
    first.bls_pubkey = vec![0x5Au8; 48];
    first.blocks_produced = 42;
    reversed.producers.insert(h(0xF0), first);
    reversed.exit_history.insert(h(0x21), 1);
    reversed.exit_history.insert(h(0xE1), 987_654);

    assert_eq!(reversed.serialize_canonical(), forward);
}
