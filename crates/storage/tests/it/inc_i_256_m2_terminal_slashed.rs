// covers: slash_state slash_window slash_evidence slash_admission pool unbonding entry error assembly tx_processing validation_checks mod lib block transaction types set_registration set_lifecycle defaults network_params
//! INC-I-256 M2 / R-4 — terminal Slashed state (AUDIT-P2-546), the
//! re-registration height that feeds the tenure floor (AUDIT-P2-547), and the
//! ProducerSet canonical bytes the window feature must not change.
//!
// OUTPUT CONTRACT: fn ProducerSet::slash_producer(&mut self, &PublicKey, u64) -> Result<u64, StorageError>
//   O2 self: target status Slashed { slashed_at }, canonical bytes
//   O3 return: Ok(bond_amount) | Err(STOR041) for a Slashed key
// OUTPUT CONTRACT: fn ProducerSet::register(&mut self, ProducerInfo, u64) -> Result<(), StorageError>
//   O2 self: entry replaced (Exited key) | unchanged (Slashed key)
//   O3 return: Ok | Err(STOR011)
// OUTPUT CONTRACT: fn ProducerSet::serialize_canonical(&self) -> Vec<u8>, state_root_component(true)
//   O3 return: bytes and root equal to the golden captured at the base commit
// PATHS: P-SLASH from Active | Unbonding | Exited ; P-AGAIN second slash ; P-REG-SLASHED ; P-REG-EXITED ; P-GOLDEN
// INPUT PARTITIONS:
//   status before slash: Active | Unbonding { started_at } | Exited
//   re-registration: Slashed key (refused) | Exited key (new registered_at)
//   golden set: Active, Slashed (exit_history entry), Exited then re-registered, one queued Slash
// MATRIX:
//   O2,O3 x P-SLASH x status            -> inc_i_256_m2_slash_producer_reaches_slashed_from_every_status
//   O2,O3 x P-AGAIN                     -> inc_i_256_m2_second_slash_refused_and_changes_nothing
//   O2,O3 x {P-REG-SLASHED, P-REG-EXITED} -> inc_i_256_m2_register_refuses_slashed_and_restamps_exited
//   O3 x P-GOLDEN                       -> inc_i_256_m2_producer_set_canonical_bytes_golden

use crypto::{Hash, KeyPair};
use storage::{PendingProducerUpdate, ProducerInfo, ProducerSet, ProducerStatus};

const UNIT: u64 = 1_000_000_000;
const SLASH_H: u64 = 77;

fn info(kp: &KeyPair, registered_at: u64, outpoint: u8) -> ProducerInfo {
    ProducerInfo::new_with_bonds(
        *kp.public_key(),
        registered_at,
        UNIT,
        (Hash::from_bytes([outpoint; 32]), 0),
        0,
        1,
    )
}

fn set_with(kp: &KeyPair) -> ProducerSet {
    let mut set = ProducerSet::new();
    set.register(info(kp, 0, 0x01), 0)
        .expect("FIXTURE: register");
    set
}

fn exit(set: &mut ProducerSet, kp: &KeyPair) {
    set.get_by_pubkey_mut(kp.public_key())
        .expect("FIXTURE: registered")
        .status = ProducerStatus::Exited;
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// R-4 (AUDIT-P2-546) — Decision: a failure means a status exists from which a valid slash cannot make the producer terminally Slashed.
#[test]
fn inc_i_256_m2_slash_producer_reaches_slashed_from_every_status() {
    for start in ["Active", "Unbonding", "Exited"] {
        let kp = KeyPair::generate();
        let mut set = set_with(&kp);
        match start {
            "Unbonding" => set
                .request_exit(kp.public_key(), 10)
                .expect("FIXTURE: exit"),
            "Exited" => exit(&mut set, &kp),
            _ => {}
        }
        let burned = set.slash_producer(kp.public_key(), SLASH_H);
        assert_eq!(burned.ok(), Some(UNIT), "{start}: the whole bond is burned");
        let x = set.get_by_pubkey(kp.public_key()).expect("still listed");
        assert_eq!(
            x.status,
            ProducerStatus::Slashed {
                slashed_at: SLASH_H
            },
            "{start}: terminal Slashed"
        );
        assert!(x.is_slashed() && !x.is_active(), "{start}");
    }
}

// R-4 (AUDIT-P2-546) — Decision: a failure means one offence slashes twice or a second slash rewrites the first one's height.
#[test]
fn inc_i_256_m2_second_slash_refused_and_changes_nothing() {
    let kp = KeyPair::generate();
    let mut set = set_with(&kp);
    set.slash_producer(kp.public_key(), SLASH_H)
        .expect("PRECONDITION: first slash");
    let before = set.serialize_canonical();
    let err = set
        .slash_producer(kp.public_key(), SLASH_H + 5)
        .expect_err("a Slashed key is not slashable");
    assert!(err.to_string().contains("STOR041"), "got: {err}");
    assert_eq!(
        set.serialize_canonical(),
        before,
        "the refusal writes nothing"
    );
}

// R-4 (AUDIT-P2-546, AUDIT-P2-547) — Decision: a failure means a slashed key re-enters the set, or a re-registered key keeps its old registration height and old evidence passes the tenure floor.
#[test]
fn inc_i_256_m2_register_refuses_slashed_and_restamps_exited() {
    let kp = KeyPair::generate();
    let mut set = set_with(&kp);
    set.slash_producer(kp.public_key(), SLASH_H)
        .expect("PRECONDITION: slash");
    let before = set.serialize_canonical();
    let err = set
        .register(info(&kp, 500, 0x02), 500)
        .expect_err("a Slashed key cannot re-register");
    assert!(err.to_string().contains("STOR011"), "got: {err}");
    assert_eq!(
        set.serialize_canonical(),
        before,
        "the refusal writes nothing"
    );

    let kp = KeyPair::generate();
    let mut set = set_with(&kp);
    exit(&mut set, &kp);
    set.register(info(&kp, 500, 0x02), 500)
        .expect("an Exited key re-registers");
    let x = set.get_by_pubkey(kp.public_key()).expect("listed");
    assert_eq!(
        x.registered_at, 500,
        "the tenure floor input is the new height"
    );
    assert_eq!(x.status, ProducerStatus::Active);
}

// Captured at the base commit.
const GOLDEN_CANONICAL_BLAKE3: &str =
    "a105cab8f615e468bd57734c03905aa9a37a4aeda6bf533a4df958b6ff3c3d61";
const GOLDEN_ROOT_COMPONENT: &str =
    "3aa816c6f7bdca3c8856e14bb3f4dd1a151d77fae5015d1a61fd4c57fbc49725";
const GOLDEN_CANONICAL_LEN: usize = 790;

// R-4 — Decision: a failure means the window feature added a persisted field or changed the producer-set encoding, which moves the state root on every node.
#[test]
fn inc_i_256_m2_producer_set_canonical_bytes_golden() {
    let [a, b, c] = [0x11u8, 0x22, 0x33].map(|s| KeyPair::from_seed([s; 32]));
    let mut set = ProducerSet::new();
    set.register(info(&a, 0, 0xA0), 0).expect("FIXTURE: a");
    set.register(info(&b, 10, 0xB0), 10).expect("FIXTURE: b");
    set.register(info(&c, 20, 0xC0), 20).expect("FIXTURE: c");
    set.slash_producer(b.public_key(), 40)
        .expect("FIXTURE: slash b");
    set.request_exit(c.public_key(), 100)
        .expect("FIXTURE: exit c");
    exit(&mut set, &c);
    set.register(info(&c, 300, 0xC1), 300)
        .expect("FIXTURE: re-register c");
    set.queue_update(PendingProducerUpdate::Slash {
        pubkey: *a.public_key(),
        height: 90,
    });

    let bytes = set.serialize_canonical();
    let (root, len) = set.state_root_component(true);
    let actual = (
        hex(crypto::hash::hash(&bytes).as_bytes()),
        hex(root.as_bytes()),
        len,
    );
    assert_eq!(
        actual,
        (
            GOLDEN_CANONICAL_BLAKE3.to_string(),
            GOLDEN_ROOT_COMPONENT.to_string(),
            GOLDEN_CANONICAL_LEN
        ),
        "ProducerSet canonical bytes moved"
    );
}
