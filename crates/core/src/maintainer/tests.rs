//! Unit tests for the maintainer module.
//!
//! These moved verbatim out of the old single-file `crates/core/src/maintainer.rs`
//! when INC-I-172 M2 split it into a directory module. `test_threshold_calculation`
//! is the ONE assertion that changed: it encoded `calculate_threshold(0) == 0`,
//! which is exactly the defect (AUDIT-P1-010 / FM-02) M2 removes.
//!
//! ---------------------------------------------------------------------------
//! OUTPUT CONTRACT
//! ---------------------------------------------------------------------------
//! Functions under test:
//!   F1 `MaintainerSet::calculate_threshold(usize) -> usize`        (associated, pure)
//!   F2 `MaintainerSet::is_authorizable(&self) -> bool`             (&self, pure)
//!   F3 `MaintainerSet::verify_multisig[_excluding][_legacy|_at](..) -> bool`
//!                                                                  (&self, pure)
//!   F4 `MaintainerSet::{add,remove,force_remove}_maintainer(&mut self, ..)`
//!   F5 `MaintainerSet::{is_maintainer,can_add,can_remove,member_count,
//!       is_fully_bootstrapped,needs_bootstrap_member}`              (&self, pure)
//!   F6 `MaintainerChangeData::{signing_message,to_bytes,from_bytes}`
//!   F7 `ProtocolActivationData::{signing_message,to_bytes,from_bytes}`
//!   F8 `derive_canonical_maintainer_set(&[(PublicKey,u64)], u64) -> MaintainerSet`
//!
//! OUTPUTS
//!   O1 return value of F1/F2/F3/F5/F6/F7/F8
//!   O2 receiver mutation for F4 — `self.members`, `self.threshold`,
//!      `self.last_updated`
//!   O3 `Result`/`Option` error arm of F4 and F6/F7 decode
//!   O4 (mutable params)          — NONE; every function takes `&self`, `&mut self`
//!                                  or values
//!   O5 (persistent store writes) — NONE; `crates/core` has no storage edge
//!   O6 (channels / logs)         — NONE
//!
//! PATHS
//!   PT-zero      F1 with member_count == 0
//!   PT-nonzero   F1 with member_count >= 1
//!   PA-accept    F3 returns true
//!   PA-reject    F3 returns false
//!   PA-refuse    F3 short-circuits on `!is_authorizable()` before counting
//!   PM-ok        F4 applies the mutation
//!   PM-err       F4 refuses (max / min / already / not a maintainer)
//!   PC-truncate  F8 with more than INITIAL_MAINTAINER_COUNT registrations
//!   PC-partial   F8 with fewer
//!   PS-roundtrip F6/F7 encode then decode
//!   PS-invalid   F6/F7 decode of garbage
//!
//! INPUT PARTITIONS
//!   IP-1  member_count == 0                                   -> PT-zero
//!   IP-2  member_count in 1..=7                               -> PT-nonzero
//!   IP-3  empty set, zero signature entries                   -> PA-refuse
//!   IP-4  set forced to empty by slashing                     -> PA-refuse
//!   IP-5  3-member set, 2 distinct signers                    -> PA-accept
//!   IP-6  3-member set, 1 signer repeated twice               -> PA-reject (distinct)
//!                                                                PA-accept (legacy)
//!   IP-7  IP-6 through the gated dispatcher at height AH-1    -> PA-accept
//!   IP-8  IP-6 through the gated dispatcher at height AH      -> PA-reject
//!   IP-9  excluding: only the excluded key signed             -> PA-reject
//!   IP-10 excluding: one non-excluded key repeated            -> PA-reject (distinct)
//!   IP-11 add to a 5-member set / add an existing member      -> PM-err
//!   IP-12 remove from a 3-member set / remove a non-member    -> PM-err
//!   IP-13 add/remove within bounds                            -> PM-ok
//!   IP-14 force_remove below MIN_MAINTAINERS down to 0        -> PM-ok
//!   IP-15 8 registrations tied at registered_at == 0, reversed -> PC-truncate
//!   IP-16 3 registrations at distinct registered_at            -> PC-partial
//!   IP-17 well-formed payload                                  -> PS-roundtrip
//!   IP-18 empty / 4-zero-byte payload                          -> PS-invalid
//!
//! MATRIX
//!   O1 x {IP-1, IP-2(7 counts), IP-3, IP-4, IP-5..IP-10, IP-15, IP-16, IP-17, IP-18}
//!   O2 x {IP-13, IP-14}
//!   O3 x {IP-11, IP-12, IP-18}
//!   O4/O5/O6 — structurally absent.

use super::*;
use crypto::PublicKey;

fn test_pubkey(seed: u8) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    bytes[31] = seed;
    PublicKey::from_bytes(bytes)
}

#[test]
fn test_threshold_calculation() {
    // INC-I-172 M2 / AUDIT-P1-010: the 0 arm used to be 0, which made
    // `valid_count >= threshold` vacuous on an empty set. This assertion
    // encoded the exact defect M2 removes, so it is updated, not preserved.
    assert_eq!(
        MaintainerSet::calculate_threshold(0),
        MAINTAINER_THRESHOLD,
        "an empty set must carry an UNSATISFIABLE threshold, never 0"
    );
    assert_eq!(MaintainerSet::calculate_threshold(1), 1);
    assert_eq!(MaintainerSet::calculate_threshold(2), 2);
    assert_eq!(MaintainerSet::calculate_threshold(3), 2);
    assert_eq!(MaintainerSet::calculate_threshold(4), 3);
    assert_eq!(MaintainerSet::calculate_threshold(5), 3);
    // Extra: majority for larger sets
    assert_eq!(MaintainerSet::calculate_threshold(6), 4);
    assert_eq!(MaintainerSet::calculate_threshold(7), 4);
}

#[test]
fn test_empty_set_is_not_authorizable() {
    // FM-02, UNGATED: an empty set authorizes nothing on any path, at any height.
    let empty = MaintainerSet::new();
    assert!(!empty.is_authorizable());
    assert!(!empty.verify_multisig(&[], b"add:attacker"));
    assert!(!empty.verify_multisig_legacy(&[], b"add:attacker"));
    assert!(!empty.verify_multisig_excluding(&[], b"remove:x", &test_pubkey(1)));
    assert!(!empty.verify_multisig_excluding_legacy(&[], b"remove:x", &test_pubkey(1)));

    let derived = MaintainerSet::with_members(Vec::new(), 0);
    assert!(!derived.is_authorizable());
    assert_ne!(derived.threshold, 0);
}

#[test]
fn test_add_maintainer() {
    let mut set = MaintainerSet::new();

    // Add 5 maintainers (bootstrap)
    for i in 1..=5 {
        assert!(set.add_maintainer(test_pubkey(i), i as u64).is_ok());
    }

    assert_eq!(set.member_count(), 5);
    assert!(!set.can_add()); // At max
    assert!(set.can_remove()); // Can remove

    // Cannot add 6th maintainer
    assert_eq!(
        set.add_maintainer(test_pubkey(6), 6),
        Err(MaintainerError::MaxMaintainersReached)
    );
}

#[test]
fn test_remove_maintainer() {
    let members: Vec<PublicKey> = (1..=5).map(test_pubkey).collect();
    let mut set = MaintainerSet::with_members(members, 0);

    // Remove 2 maintainers (down to 3)
    assert!(set.remove_maintainer(&test_pubkey(1), 1).is_ok());
    assert!(set.remove_maintainer(&test_pubkey(2), 2).is_ok());

    assert_eq!(set.member_count(), 3);
    assert!(!set.can_remove()); // At min
    assert!(set.can_add()); // Can add

    // Cannot remove below minimum
    assert_eq!(
        set.remove_maintainer(&test_pubkey(3), 3),
        Err(MaintainerError::MinMaintainersRequired)
    );
}

#[test]
fn test_is_maintainer() {
    let members: Vec<PublicKey> = (1..=3).map(test_pubkey).collect();
    let set = MaintainerSet::with_members(members, 0);

    assert!(set.is_maintainer(&test_pubkey(1)));
    assert!(set.is_maintainer(&test_pubkey(2)));
    assert!(set.is_maintainer(&test_pubkey(3)));
    assert!(!set.is_maintainer(&test_pubkey(4)));
}

#[test]
fn test_already_maintainer() {
    let members: Vec<PublicKey> = (1..=3).map(test_pubkey).collect();
    let mut set = MaintainerSet::with_members(members, 0);

    assert_eq!(
        set.add_maintainer(test_pubkey(1), 1),
        Err(MaintainerError::AlreadyMaintainer)
    );
}

#[test]
fn test_not_maintainer() {
    let members: Vec<PublicKey> = (1..=5).map(test_pubkey).collect();
    let mut set = MaintainerSet::with_members(members, 0);

    assert_eq!(
        set.remove_maintainer(&test_pubkey(6), 1),
        Err(MaintainerError::NotMaintainer)
    );
}

#[test]
fn test_force_remove_ignores_minimum() {
    let members: Vec<PublicKey> = (1..=3).map(test_pubkey).collect();
    let mut set = MaintainerSet::with_members(members, 0);

    // Normal remove should fail at minimum
    assert_eq!(
        set.remove_maintainer(&test_pubkey(1), 1),
        Err(MaintainerError::MinMaintainersRequired)
    );

    // Force remove (for slashing) should work
    assert!(set.force_remove_maintainer(&test_pubkey(1), 1));
    assert_eq!(set.member_count(), 2);

    // Can continue forcing down to 0
    assert!(set.force_remove_maintainer(&test_pubkey(2), 2));
    assert!(set.force_remove_maintainer(&test_pubkey(3), 3));
    assert_eq!(set.member_count(), 0);
    // ...and a set forced to empty authorizes nothing (FM-02).
    assert!(!set.is_authorizable());
}

#[test]
fn test_bootstrap_status() {
    let mut set = MaintainerSet::new();

    assert!(set.needs_bootstrap_member());
    assert!(!set.is_fully_bootstrapped());

    for i in 1..=4 {
        let _ = set.add_maintainer(test_pubkey(i), i as u64);
        assert!(set.needs_bootstrap_member());
        assert!(!set.is_fully_bootstrapped());
    }

    let _ = set.add_maintainer(test_pubkey(5), 5);
    assert!(!set.needs_bootstrap_member());
    assert!(set.is_fully_bootstrapped());
}

#[test]
fn test_maintainer_change_data_serialization() {
    let data = MaintainerChangeData::with_reason(test_pubkey(1), vec![], "Test reason".to_string());

    let bytes = data.to_bytes();
    let recovered = MaintainerChangeData::from_bytes(&bytes).unwrap();

    assert_eq!(data.target, recovered.target);
    assert_eq!(data.reason, recovered.reason);
}

#[test]
fn test_signing_message_format() {
    let data = MaintainerChangeData::new(test_pubkey(1), vec![]);

    let add_msg = data.signing_message(true);
    assert!(String::from_utf8_lossy(&add_msg).starts_with("add:"));

    let remove_msg = data.signing_message(false);
    assert!(String::from_utf8_lossy(&remove_msg).starts_with("remove:"));
}

// Integration test with real signatures
#[test]
fn test_verify_multisig_with_real_signatures() {
    // Generate 3 keypairs
    let kp1 = crypto::KeyPair::generate();
    let kp2 = crypto::KeyPair::generate();
    let kp3 = crypto::KeyPair::generate();

    let members = vec![*kp1.public_key(), *kp2.public_key(), *kp3.public_key()];
    let set = MaintainerSet::with_members(members, 0);

    // Message to sign
    let message = b"test message";

    // Sign with 2 of 3 (threshold is 2 for 3 members)
    let sig1 = MaintainerSignature::new(
        *kp1.public_key(),
        crypto::signature::sign(message, kp1.private_key()),
    );
    let sig2 = MaintainerSignature::new(
        *kp2.public_key(),
        crypto::signature::sign(message, kp2.private_key()),
    );

    let signatures = vec![sig1, sig2];
    assert!(set.verify_multisig(&signatures, message));

    // Only 1 signature should fail
    let signatures = vec![signatures[0].clone()];
    assert!(!set.verify_multisig(&signatures, message));

    // ...and repeating that ONE signature must not manufacture a quorum
    // (AUDIT-P0-010).
    let padded = vec![signatures[0].clone(), signatures[0].clone()];
    assert!(!set.verify_multisig(&padded, message));
    // The pre-activation counter is the one that accepts it.
    assert!(set.verify_multisig_legacy(&padded, message));
    // Gate dispatch: legacy below, distinct-signer at and above.
    assert!(set.verify_multisig_at(&padded, message, 99, 100));
    assert!(!set.verify_multisig_at(&padded, message, 100, 100));
    let _ = kp3;
}

#[test]
fn test_verify_multisig_excluding() {
    // Generate 3 keypairs
    let kp1 = crypto::KeyPair::generate();
    let kp2 = crypto::KeyPair::generate();
    let kp3 = crypto::KeyPair::generate();

    let members = vec![*kp1.public_key(), *kp2.public_key(), *kp3.public_key()];
    let set = MaintainerSet::with_members(members, 0);

    let message = b"remove target";

    // Sign with all 3
    let sig1 = MaintainerSignature::new(
        *kp1.public_key(),
        crypto::signature::sign(message, kp1.private_key()),
    );
    let sig2 = MaintainerSignature::new(
        *kp2.public_key(),
        crypto::signature::sign(message, kp2.private_key()),
    );
    let sig3 = MaintainerSignature::new(
        *kp3.public_key(),
        crypto::signature::sign(message, kp3.private_key()),
    );

    // If we exclude kp1 (the target), we need 2 valid sigs from others
    let signatures = vec![sig1.clone(), sig2.clone(), sig3.clone()];

    // Should pass: sig2 and sig3 are valid and not excluded
    assert!(set.verify_multisig_excluding(&signatures, message, kp1.public_key()));

    // Should fail if we only have the target's signature
    let signatures = vec![sig1];
    assert!(!set.verify_multisig_excluding(&signatures, message, kp1.public_key()));

    // AUDIT-P0-010: ONE non-excluded key repeated must not clear a 2-of-3
    // threshold on the removal path.
    let padded = vec![sig2.clone(), sig2];
    assert!(!set.verify_multisig_excluding(&padded, message, kp1.public_key()));
    assert!(set.verify_multisig_excluding_at(&padded, message, kp1.public_key(), 99, 100));
    assert!(!set.verify_multisig_excluding_at(&padded, message, kp1.public_key(), 100, 100));
}

#[test]
fn test_canonical_derivation_total_order() {
    // Reversed input, all tied at registered_at == 0: the total order must put
    // the lowest pubkey bytes first and truncate at INITIAL_MAINTAINER_COUNT.
    let regs: Vec<(PublicKey, u64)> = (1..=8u8).rev().map(|i| (test_pubkey(i), 0u64)).collect();
    let set = derive_canonical_maintainer_set(&regs, 7);

    assert_eq!(
        set.members,
        (1..=5u8).map(test_pubkey).collect::<Vec<_>>(),
        "ties break on ASCENDING pubkey bytes"
    );
    assert_eq!(set.threshold, 3);
    assert_eq!(set.last_updated, 7);

    // registered_at is the PRIMARY key.
    let mixed = vec![
        (test_pubkey(99), 1u64),
        (test_pubkey(10), 5u64),
        (test_pubkey(11), 6u64),
    ];
    assert_eq!(
        derive_canonical_maintainer_set(&mixed, 0).members,
        vec![test_pubkey(99), test_pubkey(10), test_pubkey(11)]
    );
}

#[test]
fn test_protocol_activation_data_serialization() {
    let data = ProtocolActivationData::new(2, 500, "Enable new rules".to_string(), vec![]);

    let bytes = data.to_bytes();
    let recovered = ProtocolActivationData::from_bytes(&bytes).unwrap();

    assert_eq!(data.protocol_version, recovered.protocol_version);
    assert_eq!(data.activation_epoch, recovered.activation_epoch);
    assert_eq!(data.description, recovered.description);
    assert_eq!(data.signatures.len(), recovered.signatures.len());
}

#[test]
fn test_protocol_activation_signing_message() {
    let data = ProtocolActivationData::new(3, 1000, "Test".to_string(), vec![]);
    let msg = data.signing_message();
    assert_eq!(msg, b"activate:3:1000");
}

#[test]
fn test_protocol_activation_from_bytes_invalid() {
    assert!(ProtocolActivationData::from_bytes(&[]).is_none());
    assert!(ProtocolActivationData::from_bytes(&[0u8; 4]).is_none());
}
