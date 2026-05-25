// OUTPUT CONTRACT: fn validate_transaction(tx: &Transaction, ctx: &ValidationContext)
//   Outputs:
//     O1: returned Result<(), ValidationError> for CreatePool transactions whose
//         Pool UTXO + first-deposit LPShare lock MINIMUM_LIQUIDITY (D1) into the
//         pool (Uniswap v2 semantics: the gap between Pool.total_lp_shares and
//         the creator's LPShare.amount equals MINIMUM_LIQUIDITY).
//   PATHS:
//     P1: total_lp_shares < MINIMUM_LIQUIDITY → reject with the new
//         AmmMinimumLiquidity variant ([ERRTX-AMM002], stable error_code
//         "AMM_MINIMUM_LIQUIDITY").
//     P2: total_lp_shares >= MINIMUM_LIQUIDITY AND
//         creator_lpshare.amount + MINIMUM_LIQUIDITY == total_lp_shares → accept
//         (locks exactly MINIMUM_LIQUIDITY).
//     P3: total_lp_shares >= MINIMUM_LIQUIDITY AND
//         creator_lpshare.amount + MINIMUM_LIQUIDITY != total_lp_shares → reject
//         with AmmMinimumLiquidity (creator-share mismatch).
//   INPUT PARTITIONS:
//     P1: 1 partition (LPShare must have amount >= 1 per [ERRTX003], so the
//         total=0 case is unreachable through this test surface — covered
//         indirectly by the boundary test below).
//       - total = MINIMUM_LIQUIDITY - 1 (999), creator = 1 → reject (boundary)
//     P2: 2 partitions
//       - total = MINIMUM_LIQUIDITY+1,   creator = 1      → accept (smallest valid)
//       - total = MINIMUM_LIQUIDITY+707, creator = 707    → accept (standard
//                                                            UniV2 sqrt-style)
//     P3: 1 partition
//       - total = 2000, creator = 1001 (off-by-one too much) → reject
//   ADVERSARIAL PARTITION (D1 justification — Adversarial Capital A4/A7):
//     Replays the first-deposit inflation attack threshold: at MINIMUM_LIQUIDITY=1
//     the attacker mints 2 LP shares (1 locked, 1 retained) for ~1 unit of
//     capital and controls ~50% of the pool. At MINIMUM_LIQUIDITY=1000 the
//     attacker MUST mint at least MIN_LIQ+1 LP shares (locking 1000) to retain
//     even 1, making the attack 1:1 cost/payoff = unprofitable. The structural
//     validator alone
//     enforces this threshold; the apply_block AMM consumer (separate session)
//     does proportional minting on top.
//
// Pre-fix expectation (TDD red phase): without the MINIMUM_LIQUIDITY constant +
// the AmmMinimumLiquidity variant + the validate_create_pool wire-up, this file
// will not compile.

use crypto::Hash;
use doli_core::consensus::{ConsensusParams, GENESIS_TIME, MINIMUM_LIQUIDITY};
use doli_core::network::Network;
use doli_core::transaction::{Input, Output, Transaction, TxType};
use doli_core::validation::{self, ValidationContext, ValidationError};

fn devnet_ctx() -> ValidationContext {
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

fn create_pool_tx_with_lp(total_lp: u64, creator_lp: u64, fee_bps: u16) -> Transaction {
    let asset_b = Hash::from_bytes([0xBB; 32]);
    let pool_id = Output::compute_pool_id(&Hash::ZERO, &asset_b, fee_bps);
    let pool_output = Output::pool(pool_id, asset_b, 1000, 2000, total_lp, 0, 100, fee_bps, 100);
    let lp_output = Output::lp_share(creator_lp, pool_id, Hash::from_bytes([0x01; 32]));
    Transaction {
        version: 1,
        tx_type: TxType::CreatePool,
        inputs: vec![Input::new(Hash::from_bytes([0xFF; 32]), 0)],
        outputs: vec![pool_output, lp_output],
        extra_data: vec![],
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANT PRESENCE — MINIMUM_LIQUIDITY is locked at 1000 (Uniswap v2 standard,
// D1 approved 2026-05-25). Must be set before amm_activation_height ever
// crosses on any network.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn minimum_liquidity_constant_is_1000() {
    assert_eq!(
        MINIMUM_LIQUIDITY, 1000,
        "MINIMUM_LIQUIDITY must be 1000 per spec D1. Lower values are unsafe \
         (first-deposit inflation attack, Adversarial Capital A4/A7)."
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// P2 — creator locks exactly MINIMUM_LIQUIDITY: smallest legitimate pool.
// total = MINIMUM_LIQUIDITY + 1 (1001), creator = 1. The 1000-share gap is
// the permanent lock; the creator holds 1 spendable LP share.
// (LPShare UTXO must have amount >= 1 per the generic output validator
//  [ERRTX003] — zero-amount non-native outputs are rejected before M3 runs.)
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn create_pool_smallest_locks_exactly_minimum_liquidity_accepted() {
    let tx = create_pool_tx_with_lp(MINIMUM_LIQUIDITY + 1, 1, 30);
    let ctx = devnet_ctx();
    let res = validation::validate_transaction(&tx, &ctx);
    assert!(
        res.is_ok(),
        "CreatePool with total=MINIMUM_LIQUIDITY+1, creator=1 must be accepted \
         (smallest valid pool, MINIMUM_LIQUIDITY locked, 1 share to creator). \
         Got: {:?}",
        res
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// P2 — standard Uniswap v2 first deposit: creator gets sqrt(a*b) - MIN_LIQ,
// MIN_LIQ is locked. Modelled here as total=1707, creator=707 (gap=1000).
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn create_pool_locks_minimum_liquidity_accepted() {
    let tx = create_pool_tx_with_lp(MINIMUM_LIQUIDITY + 707, 707, 30);
    let ctx = devnet_ctx();
    let res = validation::validate_transaction(&tx, &ctx);
    assert!(
        res.is_ok(),
        "CreatePool with creator=total-MINIMUM_LIQUIDITY must be accepted. \
         Got: {:?}",
        res
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// P1 — under-minted: total_lp_shares < MINIMUM_LIQUIDITY → reject.
// Boundary at MINIMUM_LIQUIDITY - 1 with creator_share = 1 (the smallest
// non-zero LPShare amount, since [ERRTX003] forbids zero-amount LPShare).
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn create_pool_under_minted_rejected_at_boundary() {
    let tx = create_pool_tx_with_lp(MINIMUM_LIQUIDITY - 1, 1, 30);
    let ctx = devnet_ctx();
    let res = validation::validate_transaction(&tx, &ctx);
    match res {
        Err(ValidationError::AmmMinimumLiquidity {
            declared_total,
            minimum_liquidity,
            ..
        }) => {
            assert_eq!(declared_total, MINIMUM_LIQUIDITY - 1);
            assert_eq!(minimum_liquidity, MINIMUM_LIQUIDITY);
        }
        other => panic!(
            "total=MINIMUM_LIQUIDITY-1 must be rejected with AmmMinimumLiquidity. \
             Got: {:?}",
            other
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// P3 — creator-share mismatch: total >= MIN_LIQ but creator + MIN_LIQ != total.
// This is the attack vector: a malicious CreatePool that claims to lock
// MINIMUM_LIQUIDITY but actually only burns less (giving the creator a larger
// share than they paid for).
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn create_pool_creator_share_mismatch_rejected() {
    // total=2000, creator=1001 → only 999 locked (less than MIN_LIQ)
    let tx = create_pool_tx_with_lp(2000, 1001, 30);
    let ctx = devnet_ctx();
    let res = validation::validate_transaction(&tx, &ctx);
    match res {
        Err(ValidationError::AmmMinimumLiquidity {
            declared_total,
            creator_share,
            minimum_liquidity,
        }) => {
            assert_eq!(declared_total, 2000);
            assert_eq!(creator_share, 1001);
            assert_eq!(minimum_liquidity, MINIMUM_LIQUIDITY);
        }
        other => panic!(
            "creator share above (total - MINIMUM_LIQUIDITY) must be rejected. \
             Got: {:?}",
            other
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ERROR_CODE — stable machine-readable code for agentic consumers.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn amm_minimum_liquidity_error_code_is_stable() {
    // total = MINIMUM_LIQUIDITY - 1, creator = 1 → under-minted → AmmMinimumLiquidity.
    let tx = create_pool_tx_with_lp(MINIMUM_LIQUIDITY - 1, 1, 30);
    let ctx = devnet_ctx();
    let err = validation::validate_transaction(&tx, &ctx).expect_err("must reject");
    assert_eq!(
        err.error_code(),
        "AMM_MINIMUM_LIQUIDITY",
        "error_code must be the stable string AMM_MINIMUM_LIQUIDITY"
    );
    assert!(
        err.to_string().contains("[ERRTX-AMM002]"),
        "error message must carry the [ERRTX-AMM002] log prefix, got: {}",
        err
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ADVERSARIAL — D1 justification: threshold transition between MIN_LIQ=1 and
// MIN_LIQ=1000 makes the first-deposit inflation attack unprofitable.
//
// Demonstration: at MIN_LIQ=1000 the attacker MUST lock at least 1000 LP
// shares (i.e., at least 1000 units of value in proportional terms) for the
// validator to accept their pool. At MIN_LIQ=1 the same attack succeeds with
// 1 unit of locked value (Adversarial Capital A4/A7).
//
// Since apply_block AMM math is out of scope this milestone, the test pins
// the STRUCTURAL threshold: a CreatePool that locks fewer than MINIMUM_LIQUIDITY
// must always fail validation, regardless of how cleverly the reserves /
// creator shares are arranged. The apply_block consumer (future session) is
// responsible for the proportional minting on subsequent deposits.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn adversarial_first_deposit_attack_blocked_at_min_liquidity_1000() {
    // The attacker's smallest possible pool that would let them retain 1
    // share of LP for ~1 unit of locked value at MIN_LIQ=1:
    //   total=2, creator=1 (legacy MIN_LIQ=1 semantics: 1 share locked,
    //   1 share to creator)
    // would have been accepted with MINIMUM_LIQUIDITY=1.
    // At MINIMUM_LIQUIDITY=1000, the same shape is rejected because total<MIN_LIQ.
    let tx = create_pool_tx_with_lp(2, 1, 30);
    let ctx = devnet_ctx();
    let res = validation::validate_transaction(&tx, &ctx);
    assert!(
        matches!(res, Err(ValidationError::AmmMinimumLiquidity { .. })),
        "first-deposit inflation attack vector (total=2, creator=1, lock=1) \
         must be REJECTED at MINIMUM_LIQUIDITY=1000. Got: {:?}",
        res
    );

    // The smallest legitimately-acceptable pool requires the attacker to
    // lock 1000 LP shares (1000x more value than under MIN_LIQ=1) to retain
    // even a single share, making the attack 1:1 cost/payoff:
    let tx_ok = create_pool_tx_with_lp(MINIMUM_LIQUIDITY + 1, 1, 30);
    assert!(
        validation::validate_transaction(&tx_ok, &ctx).is_ok(),
        "the smallest legitimate pool (total=MIN_LIQ+1, creator=1, lock=MIN_LIQ) \
         must be accepted"
    );
}
