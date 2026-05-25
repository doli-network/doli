// OUTPUT CONTRACT: fn Output::compute_pool_id(asset_a: &Hash, asset_b: &Hash, fee_bps: u16) -> Hash
//   Outputs:
//     O1: deterministic Hash for a given (asset_a, asset_b, fee_bps) triple.
//   PATHS:
//     P1: ordering — asset_a vs asset_b argument order MUST NOT change the result
//         (canonical sort by raw byte order).
//     P2: distinctness — distinct fee_bps tiers (5/30/100) for the SAME pair MUST
//         produce DIFFERENT pool_ids (multi-fee-tier pool identity, D2).
//     P3: domain separation — the V2 domain ("DOLI_POOL_V2") MUST produce a
//         different hash than the legacy V1 domain ("DOLI_POOL") for the same
//         underlying bytes, so any pre-existing V1 artifact is provably
//         non-collidable with a V2 pool_id.
//     P4: payload order — the documented layout is
//         `fee_bps.to_le_bytes() || lo_asset || hi_asset`. The test pins this
//         by recomputing BLAKE3 with that exact byte order and asserting equality.
//   INPUT PARTITIONS:
//     P1: 1 partition — same triple with arguments swapped.
//     P2: 3 partitions — (5 bps, 30 bps, 100 bps).
//     P3: 1 partition — V1 vs V2 domain.
//     P4: 1 partition — manual blake3 vs library output.
//   MATRIX:
//     O1 × P1 × {swap}        → 1 assertion (commutativity)
//     O1 × P2 × {3 tiers}     → C(3,2)=3 distinctness assertions
//     O1 × P3 × {V1 vs V2}    → 1 assertion (domain separation)
//     O1 × P4 × {byte order}  → 1 assertion (canonical payload)
//
// Pre-fix expectation (TDD red phase): without the (asset_a, asset_b, fee_bps)
// signature + V2 domain, this file will not compile.

use crypto::{hash::hash_with_domain, Hash};
use doli_core::consensus::{ConsensusParams, GENESIS_TIME};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, Transaction, TxType, POOL_ID_DOMAIN};
use doli_core::validation::{self, ValidationContext, ValidationError};

fn lo_hi<'a>(a: &'a Hash, b: &'a Hash) -> (&'a Hash, &'a Hash) {
    if a.as_bytes() < b.as_bytes() {
        (a, b)
    } else {
        (b, a)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// O1 × P4 — canonical payload order is `fee_bps_le || lo || hi`.
// Pins the exact byte layout (IRREVERSIBLE once amm_activation_height crosses).
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn pool_id_payload_is_fee_le_then_lo_then_hi() {
    let asset_a = Hash::ZERO;
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let fee_bps: u16 = 30;

    let (lo, hi) = lo_hi(&asset_a, &asset_b);
    let mut expected_payload = Vec::with_capacity(2 + 32 + 32);
    expected_payload.extend_from_slice(&fee_bps.to_le_bytes());
    expected_payload.extend_from_slice(lo.as_bytes());
    expected_payload.extend_from_slice(hi.as_bytes());
    let expected = hash_with_domain(POOL_ID_DOMAIN, &expected_payload);

    let actual = Output::compute_pool_id(&asset_a, &asset_b, fee_bps);
    assert_eq!(
        actual, expected,
        "compute_pool_id must hash `fee_bps.to_le_bytes() || lo_asset || hi_asset` under \
         POOL_ID_DOMAIN. This layout is IRREVERSIBLE post amm_activation_height."
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// O1 × P1 — ordering: compute_pool_id is symmetric in asset_a vs asset_b.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn pool_id_is_argument_order_independent() {
    let a = Hash::from_bytes([0x11; 32]);
    let b = Hash::from_bytes([0x99; 32]);
    let fee_bps: u16 = 30;

    let h_ab = Output::compute_pool_id(&a, &b, fee_bps);
    let h_ba = Output::compute_pool_id(&b, &a, fee_bps);
    assert_eq!(
        h_ab, h_ba,
        "compute_pool_id must sort asset arguments — order must not change the hash"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// O1 × P2 — distinctness: same pair with different fee_bps → different pool_id.
// This is the WHOLE POINT of D2 — enables multi-fee-tier pools per pair.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn pool_id_distinct_per_fee_tier() {
    let asset_a = Hash::ZERO;
    let asset_b = Hash::from_bytes([0xBB; 32]);

    let p5 = Output::compute_pool_id(&asset_a, &asset_b, 5);
    let p30 = Output::compute_pool_id(&asset_a, &asset_b, 30);
    let p100 = Output::compute_pool_id(&asset_a, &asset_b, 100);

    assert_ne!(p5, p30, "5 bps vs 30 bps pools MUST have distinct pool_ids");
    assert_ne!(
        p30, p100,
        "30 bps vs 100 bps pools MUST have distinct pool_ids"
    );
    assert_ne!(
        p5, p100,
        "5 bps vs 100 bps pools MUST have distinct pool_ids"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// O1 × P2 — also distinct for asset-A == DOLI vs same pair re-keyed with
// asset_b = Hash::ZERO. (sanity)
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn pool_id_distinct_per_pair() {
    let asset_b1 = Hash::from_bytes([0xBB; 32]);
    let asset_b2 = Hash::from_bytes([0xCC; 32]);
    let fee_bps: u16 = 30;

    let p1 = Output::compute_pool_id(&Hash::ZERO, &asset_b1, fee_bps);
    let p2 = Output::compute_pool_id(&Hash::ZERO, &asset_b2, fee_bps);
    assert_ne!(
        p1, p2,
        "distinct asset pairs MUST produce distinct pool_ids"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// O1 × P3 — domain separation: V2 domain MUST differ from legacy V1 domain.
// Even though no V1 Pool UTXO exists on any network at commit time (defi gate
// is u64::MAX everywhere), the V2 domain bump is a belt-and-braces guarantee
// that a future operator who reuses a salt cannot produce a V1↔V2 collision.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn pool_id_v2_domain_separated_from_v1() {
    assert_eq!(
        POOL_ID_DOMAIN, b"DOLI_POOL_V2",
        "POOL_ID_DOMAIN must be bumped to V2 to encode the fee_bps inclusion. \
         Any pre-existing experimental V1 artifact remains provably distinct."
    );

    let asset_a = Hash::ZERO;
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let fee_bps: u16 = 30;

    let (lo, hi) = lo_hi(&asset_a, &asset_b);
    let mut v2_payload = Vec::with_capacity(2 + 32 + 32);
    v2_payload.extend_from_slice(&fee_bps.to_le_bytes());
    v2_payload.extend_from_slice(lo.as_bytes());
    v2_payload.extend_from_slice(hi.as_bytes());
    let v2 = hash_with_domain(b"DOLI_POOL_V2", &v2_payload);
    let v1 = hash_with_domain(b"DOLI_POOL", &v2_payload);
    assert_ne!(
        v1, v2,
        "V1 and V2 domain hashes of the same payload MUST differ — proves \
         domain separation"
    );
    assert_eq!(
        Output::compute_pool_id(&asset_a, &asset_b, fee_bps),
        v2,
        "compute_pool_id must use the V2 domain"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// VALIDATOR INTEGRITY — validate_create_pool MUST recompute pool_id from
// (Hash::ZERO, asset_b_id, fee_bps) and reject a forged Pool UTXO whose
// asserted pool_id does NOT match. Without this check a producer could mint
// a Pool at an attacker-chosen pool_id, breaking the per-(pair, fee_bps)
// singleton invariant downstream (INV-DEFI-010 generalized by M2).
// ═══════════════════════════════════════════════════════════════════════════
fn devnet_ctx_post_amm_activation() -> ValidationContext {
    ValidationContext::new(
        ConsensusParams::devnet(),
        Network::Devnet,
        GENESIS_TIME + 120,
        1,
    )
    .with_prev_block(0, GENESIS_TIME, Hash::ZERO)
    .with_sig_verification_height(0)
    .with_amm_activation_height(0)
}

#[test]
fn validate_create_pool_accepts_correct_pool_id() {
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let fee_bps: u16 = 30;
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, fee_bps);
    let pool_output = Output::pool(pool_id, asset_b, 1000, 2000, 707, 0, 100, fee_bps, 100);
    let lp_output = Output::lp_share(707, pool_id, Hash::from_bytes([0x01; 32]));
    let tx = Transaction {
        version: 1,
        tx_type: TxType::CreatePool,
        inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
        outputs: vec![pool_output, lp_output],
        extra_data: vec![],
    };
    let ctx = devnet_ctx_post_amm_activation();
    let res = validation::validate_transaction(&tx, &ctx);
    assert!(
        res.is_ok(),
        "well-formed CreatePool with correct pool_id must pass validation, got: {:?}",
        res
    );
}

#[test]
fn validate_create_pool_rejects_forged_pool_id() {
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let fee_bps: u16 = 30;
    // Correct pool_id derived from (Hash::ZERO, asset_b, 30)
    let _correct = Output::compute_pool_id(&Hash::ZERO, &asset_b, fee_bps);
    // Attacker-chosen pool_id (uses WRONG fee_bps 100)
    let forged_pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, 100);
    // Pool UTXO asserts forged_pool_id but stamps fee_bps=30 → mismatch
    let pool_output = Output::pool(
        forged_pool_id,
        asset_b,
        1000,
        2000,
        707,
        0,
        100,
        fee_bps,
        100,
    );
    let lp_output = Output::lp_share(707, forged_pool_id, Hash::from_bytes([0x01; 32]));
    let tx = Transaction {
        version: 1,
        tx_type: TxType::CreatePool,
        inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
        outputs: vec![pool_output, lp_output],
        extra_data: vec![],
    };
    let ctx = devnet_ctx_post_amm_activation();
    let res = validation::validate_transaction(&tx, &ctx);
    match res {
        Err(ValidationError::InvalidPool(msg)) => {
            assert!(
                msg.contains("pool_id"),
                "rejection must reference pool_id mismatch, got: {}",
                msg
            );
        }
        other => panic!(
            "forged pool_id MUST be rejected by validate_create_pool. Got: {:?}",
            other
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// O1 × P4 — determinism: same inputs → same output across calls.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn pool_id_is_deterministic() {
    let asset_a = Hash::ZERO;
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let fee_bps: u16 = 30;

    let p1 = Output::compute_pool_id(&asset_a, &asset_b, fee_bps);
    let p2 = Output::compute_pool_id(&asset_a, &asset_b, fee_bps);
    let p3 = Output::compute_pool_id(&asset_a, &asset_b, fee_bps);
    assert_eq!(p1, p2);
    assert_eq!(p2, p3);
}
