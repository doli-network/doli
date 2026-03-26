use super::*;

fn test_pubkey(seed: u8) -> PublicKey {
    let mut bytes = [0u8; 32];
    bytes[0] = seed;
    bytes[31] = seed;
    PublicKey::from_bytes(bytes)
}

#[test]
fn test_threshold_calculation() {
    assert_eq!(MaintainerSet::calculate_threshold(0), 0);
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
