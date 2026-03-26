use super::*;
use rug::ops::Pow;
use rug::ops::RemRounding;
use rug::Integer;

#[test]
fn test_identity() {
    let disc = Integer::from(-23);
    let id = ClassGroupElement::identity(&disc);

    assert_eq!(id.a, 1);
    assert_eq!(id.b, 1); // -23 ≡ 1 (mod 4)
}

#[test]
fn test_identity_composition() {
    let disc = generate_discriminant(256, b"test");
    let id = ClassGroupElement::identity(&disc);
    let elem = ClassGroupElement::from_hash(b"input", &disc);

    // id ∘ elem = elem
    let result = id.compose(&elem);
    assert_eq!(result.a, elem.a);
    assert_eq!(result.b, elem.b);

    // elem ∘ id = elem
    let result2 = elem.compose(&id);
    assert_eq!(result2.a, elem.a);
    assert_eq!(result2.b, elem.b);
}

#[test]
fn test_square_deterministic() {
    let disc = generate_discriminant(256, b"test");
    let elem = ClassGroupElement::from_hash(b"input", &disc);

    let sq1 = elem.square();
    let sq2 = elem.square();

    assert_eq!(sq1, sq2);
}

#[test]
fn test_repeated_squaring() {
    let disc = generate_discriminant(256, b"test");
    let elem = ClassGroupElement::from_hash(b"input", &disc);

    // Square 10 times
    let mut result = elem.clone();
    for _ in 0..10 {
        result = result.square();
    }

    // Should equal elem.pow(2^10)
    let exp = Integer::from(1024); // 2^10
    let via_pow = elem.pow(&exp);

    assert_eq!(result.a, via_pow.a);
    assert_eq!(result.b, via_pow.b);
}

#[test]
fn test_serialization_roundtrip() {
    let disc = generate_discriminant(256, b"test");
    let elem = ClassGroupElement::from_hash(b"input", &disc);

    let bytes = elem.to_bytes();
    let recovered = ClassGroupElement::from_bytes(&bytes, &disc).unwrap();

    assert_eq!(elem, recovered);
}

#[test]
fn test_discriminant_properties() {
    let disc = generate_discriminant(2048, b"DOLI_VDF_DISCRIMINANT_V1");

    // Must be negative
    assert!(disc < 0);

    // Must be ≡ 1 (mod 4)
    let rem = disc.clone().rem_floor(Integer::from(4));
    assert!(rem == 1 || rem == -3);
}

#[test]
fn test_inverse() {
    let disc = generate_discriminant(256, b"test");
    let elem = ClassGroupElement::from_hash(b"input", &disc);

    // Create inverse: (a, -b, c)
    let inv = ClassGroupElement::new_unchecked(
        elem.a.clone(),
        Integer::from(-&elem.b),
        elem.discriminant().clone(),
    );

    // elem ∘ inv should give identity (a=1)
    let result = elem.compose(&inv);
    let identity = ClassGroupElement::identity(&disc);

    assert_eq!(result.a, identity.a, "elem ∘ inverse should have a=1");
    assert_eq!(result.b, identity.b, "elem ∘ inverse should be identity");
}

#[test]
fn test_pow2_mod() {
    let l = Integer::from(17);

    // 2^10 = 1024 = 60*17 + 4
    let result = pow2_mod(10, &l);
    assert_eq!(result, 4);
}

#[test]
fn test_div_2pow_by_l() {
    let l = Integer::from(17);

    // 2^10 = 1024, 1024 / 17 = 60
    let result = div_2pow_by_l(10, &l);
    assert_eq!(result, 60);
}

// Property-based tests
use proptest::prelude::*;

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(10))]

    #[test]
    fn prop_squaring_is_composition_with_self(seed: [u8; 32]) {
        let disc = generate_discriminant(128, b"prop_test");
        let elem = ClassGroupElement::from_hash(&seed, &disc);

        let squared = elem.square();
        let composed = elem.compose(&elem);

        prop_assert_eq!(squared.a, composed.a);
        prop_assert_eq!(squared.b, composed.b);
    }

    #[test]
    fn prop_serialization_roundtrip(seed: [u8; 32]) {
        let disc = generate_discriminant(128, b"prop_test");
        let elem = ClassGroupElement::from_hash(&seed, &disc);

        let bytes = elem.to_bytes();
        let recovered = ClassGroupElement::from_bytes(&bytes, &disc);

        prop_assert!(recovered.is_ok());
        prop_assert_eq!(elem, recovered.unwrap());
    }
}

#[test]
fn test_div_2pow_by_l_small_t() {
    // For small t, verify against direct computation
    let l = Integer::from(17);
    // 2^3 = 8, floor(8/17) = 0
    assert_eq!(div_2pow_by_l(3, &l), Integer::from(0));
    // 2^5 = 32, floor(32/17) = 1
    assert_eq!(div_2pow_by_l(5, &l), Integer::from(1));
    // 2^10 = 1024, floor(1024/17) = 60
    assert_eq!(div_2pow_by_l(10, &l), Integer::from(60));
}

#[test]
fn test_div_2pow_by_l_large_t_matches_direct() {
    // Verify iterative path (t > 128) matches direct computation
    let l = Integer::from(17);

    // Direct: 2^200 / 17
    let two_200 = Integer::from(2).pow(200);
    let expected = Integer::from(&two_200 / &l);

    let result = div_2pow_by_l(200, &l);
    assert_eq!(result, expected, "t=200: iterative must match direct");
}

#[test]
fn test_div_2pow_by_l_boundary_128() {
    // Test at the boundary between direct and iterative paths
    let l = Integer::from(31);

    let two_128 = Integer::from(2).pow(128);
    let expected_128 = Integer::from(&two_128 / &l);
    assert_eq!(div_2pow_by_l(128, &l), expected_128);

    let two_129 = Integer::from(2).pow(129);
    let expected_129 = Integer::from(&two_129 / &l);
    assert_eq!(
        div_2pow_by_l(129, &l),
        expected_129,
        "t=129 (first iterative) must match direct"
    );
}

#[test]
fn test_div_2pow_by_l_large_prime() {
    // Use a large prime similar to what Wesolowski VDF uses
    let l = Integer::from_str_radix(
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF43",
        16,
    )
    .unwrap();

    // t=200: verify iterative matches direct
    let two_200 = Integer::from(2).pow(200);
    let expected = Integer::from(&two_200 / &l);
    let result = div_2pow_by_l(200, &l);
    assert_eq!(result, expected, "Large prime: iterative must match direct");
}
