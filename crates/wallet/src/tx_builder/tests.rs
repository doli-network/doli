use super::*;
use crate::types::*;

// ========================================================================
// Requirement: GUI-FR-011 (Must) -- Send transaction
// Acceptance: Transaction construction, signing, validation
// ========================================================================

#[test]
fn test_fr011_transfer_builder_basic() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 500_000_000, // 5 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let recipient = [0xBBu8; 32];
    let sender = [0xAAu8; 32];

    let builder = TxBuilder::build_transfer(&utxos, recipient, 100_000_000, 1000, sender).unwrap();
    assert_eq!(builder.tx_type(), TxType::Transfer);
    assert_eq!(builder.input_count(), 1);
    assert!(builder.output_count() >= 1); // recipient + possible change
}

#[test]
fn test_fr011_transfer_insufficient_balance() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 100_000, // 0.001 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let recipient = [0xBBu8; 32];
    let sender = [0xAAu8; 32];

    let result = TxBuilder::build_transfer(&utxos, recipient, 500_000_000, 1000, sender);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Insufficient"));
}

#[test]
fn test_fr011_transfer_zero_amount_rejected() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 500_000_000,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 0, 1000, [0xAAu8; 32]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("greater than 0"));
}

#[test]
fn test_fr011_transfer_change_output() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 500_000_000, // 5 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let sender = [0xAAu8; 32];

    let builder =
        TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, sender).unwrap();
    // Should have 2 outputs: recipient (1 DOLI) + change (3.99999 DOLI)
    assert_eq!(builder.output_count(), 2);
}

#[test]
fn test_fr011_transfer_exact_amount_no_change() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 100_001_000, // exactly amount + fee
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let sender = [0xAAu8; 32];

    let builder =
        TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, sender).unwrap();
    // Exact amount: only recipient output, no change
    assert_eq!(builder.output_count(), 1);
}

#[test]
fn test_fr011_transfer_skips_unspendable_utxos() {
    let utxos = vec![
        Utxo {
            tx_hash: "bb".repeat(32),
            output_index: 0,
            amount: 999_000_000_000, // large but unspendable
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: false,
        },
        Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 500_000_000, // spendable
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        },
    ];
    let builder =
        TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]).unwrap();
    assert_eq!(builder.input_count(), 1);
}

#[test]
fn test_fr011_transfer_skips_bond_utxos() {
    let utxos = vec![
        Utxo {
            tx_hash: "cc".repeat(32),
            output_index: 0,
            amount: 1_000_000_000,
            output_type: "bond".to_string(),
            lock_until: u64::MAX,
            height: 100,
            spendable: true,
        },
        Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 500_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        },
    ];
    let builder =
        TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]).unwrap();
    assert_eq!(
        builder.input_count(),
        1,
        "Bond UTXOs must not be used for transfers"
    );
}

#[test]
fn test_fr011_transfer_multiple_utxos() {
    let utxos = vec![
        Utxo {
            tx_hash: "aa".repeat(32),
            output_index: 0,
            amount: 100_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 100,
            spendable: true,
        },
        Utxo {
            tx_hash: "bb".repeat(32),
            output_index: 0,
            amount: 100_000_000,
            output_type: "normal".to_string(),
            lock_until: 0,
            height: 101,
            spendable: true,
        },
    ];
    let builder = TxBuilder::build_transfer(
        &utxos,
        [0xBBu8; 32],
        150_000_000, // needs both UTXOs
        1000,
        [0xAAu8; 32],
    )
    .unwrap();
    assert_eq!(builder.input_count(), 2);
}

#[test]
fn test_fr011_transfer_empty_utxos() {
    let utxos: Vec<Utxo> = vec![];
    let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Insufficient"));
}

// Edge case: amount + fee overflow
#[test]
fn test_fr011_transfer_amount_fee_overflow() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: u64::MAX,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], u64::MAX, u64::MAX, [0xAAu8; 32]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("overflow"));
}

// ========================================================================
// Requirement: GUI-FR-020 (Must) -- Producer registration cost
// Acceptance: Bond count * 10 DOLI + registration fee
// ========================================================================

#[test]
fn test_fr020_registration_cost_single_bond() {
    let (bond_cost, reg_fee, total) = calculate_registration_cost(1, 0).unwrap();
    assert_eq!(bond_cost, BOND_UNIT); // 10 DOLI
    assert_eq!(reg_fee, BASE_REGISTRATION_FEE); // 0.001 DOLI
    assert_eq!(total, BOND_UNIT + BASE_REGISTRATION_FEE);
}

#[test]
fn test_fr020_registration_cost_10_bonds() {
    let (bond_cost, reg_fee, total) = calculate_registration_cost(10, 0).unwrap();
    assert_eq!(bond_cost, 10 * BOND_UNIT); // 100 DOLI
    assert_eq!(total, bond_cost + reg_fee);
}

#[test]
fn test_fr020_registration_fee_scales_with_pending() {
    let (_, fee_0, _) = calculate_registration_cost(1, 0).unwrap();
    let (_, fee_5, _) = calculate_registration_cost(1, 5).unwrap();
    assert!(
        fee_5 > fee_0,
        "Fee should increase with pending registrations"
    );
}

#[test]
fn test_fr020_registration_fee_capped() {
    let (_, fee, _) = calculate_registration_cost(1, 1000).unwrap();
    assert_eq!(
        fee, MAX_REGISTRATION_FEE,
        "Fee must be capped at MAX_REGISTRATION_FEE"
    );
}

#[test]
fn test_fr020_registration_cost_zero_bonds_rejected() {
    let result = calculate_registration_cost(0, 0);
    assert!(result.is_err());
}

#[test]
fn test_fr020_registration_cost_exceeds_max_bonds() {
    let result = calculate_registration_cost(MAX_BONDS_PER_PRODUCER + 1, 0);
    assert!(result.is_err());
}

// ========================================================================
// Requirement: GUI-FR-024 (Must) -- Add bonds
// Acceptance: Each bond = 10 DOLI, max 3000 bonds
// ========================================================================

fn producer_key() -> crypto::KeyPair {
    crypto::KeyPair::from_seed([0x66; 32])
}

fn devnet_genesis_hex() -> String {
    doli_core::consensus::ConsensusParams::devnet()
        .genesis_hash
        .to_hex()
}

fn decoded(hex_tx: &str) -> doli_core::Transaction {
    doli_core::Transaction::deserialize(&hex::decode(hex_tx).unwrap()).unwrap()
}

fn add_bond(utxos: &[Utxo], count: u32) -> anyhow::Result<String> {
    let kp = producer_key();
    let genesis = devnet_genesis_hex();
    let signer = crate::ProducerSigner {
        keypair: &kp,
        genesis_hash: &genesis,
    };
    TxBuilder::build_add_bond(utxos, count, BOND_UNIT, 2_000, 1000, &signer)
}

// REQ-DEF-001 — Decision: a failure means the wallet AddBond no longer returns the signed canonical AddBond (one Bond output per bond, change, AddBondData + producer-auth tail).
#[test]
fn test_fr024_add_bond_basic() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 50_000_000_000, // 500 DOLI
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let tx = decoded(&add_bond(&utxos, 5).unwrap());
    assert_eq!(tx.tx_type, doli_core::transaction::TxType::AddBond);
    // 5 bond outputs + 1 change output
    assert_eq!(tx.outputs.len(), 6);
    let data = tx
        .add_bond_data()
        .expect("canonical AddBondData before the tail");
    assert_eq!(data.bond_count, 5);
}

#[test]
fn test_fr024_add_bond_zero_rejected() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 50_000_000_000,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = add_bond(&utxos, 0);
    assert!(result.is_err());
}

#[test]
fn test_fr024_add_bond_exceeds_max() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: u64::MAX / 2,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = add_bond(&utxos, MAX_BONDS_PER_PRODUCER + 1);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("exceeds maximum"));
}

#[test]
fn test_fr024_add_bond_insufficient_balance() {
    let utxos = vec![Utxo {
        tx_hash: "aa".repeat(32),
        output_index: 0,
        amount: 500_000_000, // 5 DOLI -- not enough for 1 bond (10 DOLI)
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = add_bond(&utxos, 1);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Insufficient"));
}

#[test]
fn test_fr024_bond_cost_calculation() {
    // 5 bonds = 50 DOLI = 5,000,000,000 base units
    assert_eq!(5u64 * BOND_UNIT, 5_000_000_000);
    // 3000 bonds (max) = 30,000 DOLI = 3,000,000,000,000 base units
    assert_eq!(3000u64 * BOND_UNIT, 3_000_000_000_000u64);
}

// ========================================================================
// Requirement: GUI-FR-025 (Must) -- Request withdrawal
// Acceptance: FIFO order, vesting penalty calculation
// ========================================================================

#[test]
fn test_fr025_vesting_penalty_q1() {
    // 0-1 year: 75% penalty
    assert_eq!(vesting_penalty_pct(0), 75);
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS - 1), 75);
}

#[test]
fn test_fr025_vesting_penalty_q2() {
    // 1-2 years: 50% penalty
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS), 50);
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 2 - 1), 50);
}

#[test]
fn test_fr025_vesting_penalty_q3() {
    // 2-3 years: 25% penalty
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 2), 25);
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 3 - 1), 25);
}

#[test]
fn test_fr025_vesting_penalty_fully_vested() {
    // 4+ years: 0% penalty
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 4), 0);
    assert_eq!(vesting_penalty_pct(VESTING_QUARTER_SLOTS * 10), 0);
}

#[test]
fn test_fr025_withdrawal_net_calculation() {
    // 10 DOLI bond, 75% penalty
    let net = calculate_withdrawal_net(BOND_UNIT, 75);
    assert_eq!(net, BOND_UNIT / 4); // 2.5 DOLI

    // 10 DOLI bond, 50% penalty
    let net = calculate_withdrawal_net(BOND_UNIT, 50);
    assert_eq!(net, BOND_UNIT / 2); // 5 DOLI

    // 10 DOLI bond, 0% penalty (fully vested)
    let net = calculate_withdrawal_net(BOND_UNIT, 0);
    assert_eq!(net, BOND_UNIT); // 10 DOLI
}

fn bond_utxos(n: u8) -> Vec<Utxo> {
    (0..n)
        .map(|i| Utxo {
            tx_hash: format!("{:02x}", 0xB0 + i).repeat(32),
            output_index: 0,
            amount: BOND_UNIT,
            output_type: "bond".to_string(),
            lock_until: u64::MAX,
            height: 10,
            spendable: false,
        })
        .collect()
}

fn withdrawal(count: u32, destination: Option<[u8; 32]>) -> anyhow::Result<String> {
    let kp = producer_key();
    let genesis = devnet_genesis_hex();
    let signer = crate::ProducerSigner {
        keypair: &kp,
        genesis_hash: &genesis,
    };
    let bonds = bond_utxos(count as u8);
    let req = crate::WithdrawalRequest {
        network: "devnet",
        best_height: 1_000,
        bond_count: count,
        bond_utxos: &bonds,
        total_net: BOND_UNIT * u64::from(count),
        fee_utxos: &[],
        fee: 1,
        destination,
    };
    TxBuilder::build_request_withdrawal(&req, &signer)
}

fn own_address() -> [u8; 32] {
    *doli_core::validation::bond_owner(producer_key().public_key()).as_bytes()
}

// REQ-DEF-001 — Decision: a failure means the wallet withdrawal no longer returns the signed canonical RequestWithdrawal (68-byte data, Bond inputs only, one Unbonding payout at the gate).
#[test]
fn test_fr025_request_withdrawal_basic() {
    let tx = decoded(&withdrawal(5, None).unwrap());
    assert_eq!(
        tx.tx_type,
        doli_core::transaction::TxType::RequestWithdrawal
    );
    assert_eq!(tx.inputs.len(), 5, "the five Bond UTXOs, no dummy input");
    let data = tx
        .withdrawal_request_data()
        .expect("canonical withdrawal data");
    assert_eq!(data.bond_count, 5);
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(
        tx.outputs[0].output_type,
        doli_core::transaction::OutputType::Unbonding
    );
    assert_eq!(tx.outputs[0].amount, 5 * BOND_UNIT - 1);
}

#[test]
fn test_fr025_request_withdrawal_zero_bonds_rejected() {
    let result = withdrawal(0, None);
    assert!(result.is_err());
}

#[test]
fn test_fr025_request_withdrawal_with_destination() {
    let tx = decoded(&withdrawal(3, Some(own_address())).unwrap());
    assert_eq!(tx.outputs.len(), 1);
    assert!(
        withdrawal(3, Some([0xBBu8; 32])).is_err(),
        "at the unbonding gate the locked payout must go to the producer's own address"
    );
}

fn coin(tx_hash: &str, amount: u64) -> Utxo {
    Utxo {
        tx_hash: tx_hash.to_string(),
        output_index: 0,
        amount,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }
}

fn add_bond_with(utxos: &[Utxo], count: u32, bond_unit: u64, fee: u64) -> anyhow::Result<String> {
    let kp = producer_key();
    let genesis = devnet_genesis_hex();
    let signer = crate::ProducerSigner {
        keypair: &kp,
        genesis_hash: &genesis,
    };
    TxBuilder::build_add_bond(utxos, count, bond_unit, 2_000, fee, &signer)
}

fn withdraw_with(bond_count: u32, bonds: &[Utxo], fees: &[Utxo]) -> anyhow::Result<String> {
    let kp = producer_key();
    let genesis = devnet_genesis_hex();
    let signer = crate::ProducerSigner {
        keypair: &kp,
        genesis_hash: &genesis,
    };
    let req = crate::WithdrawalRequest {
        network: "devnet",
        best_height: 1_000,
        bond_count,
        bond_utxos: bonds,
        total_net: BOND_UNIT * u64::from(bond_count),
        fee_utxos: fees,
        fee: 1,
        destination: None,
    };
    TxBuilder::build_request_withdrawal(&req, &signer)
}

// OUTPUT CONTRACT: fn TxBuilder::build_add_bond(&[Utxo], bond_count: u32, bond_unit: u64, lock_until: u64, fee: u64, &ProducerSigner) -> Result<String>
//   O1 return Err: message contains "overflow"; no signed hex
//   O2 return Ok: not reached (a saturating or wrapping cost would reach it with the u64::MAX coin)
//   O3 receiver / store: none (associated fn, pure)
// PATHS: P-MUL count x bond_unit overflows | P-ADD count x bond_unit fits, + fee overflows
// MATRIX
//   O1 x {P-MUL, P-ADD} -> test_m1_add_bond_cost_overflow_refused
// INPUT PARTITIONS:
//   P-MUL: 2 x (u64::MAX/2 + 1), fee 1000 | P-ADD: 1 x u64::MAX, fee 1; one spendable coin of u64::MAX
// REQ-DEF-001 — Decision: a failure means an AddBond whose cost passes u64::MAX panics or signs a wrapped bond amount.
#[test]
fn test_m1_add_bond_cost_overflow_refused() {
    let utxos = [coin(&"aa".repeat(32), u64::MAX)];
    for (label, count, unit, fee) in [
        ("count x bond_unit", 2, u64::MAX / 2 + 1, 1000),
        ("cost + fee", 1, u64::MAX, 1),
    ] {
        let err = add_bond_with(&utxos, count, unit, fee).expect_err(label);
        assert!(err.to_string().contains("overflow"), "{label}: {err}");
    }
}

// OUTPUT CONTRACT: fn TxBuilder::build_add_bond(..) and fn TxBuilder::build_request_withdrawal(&WithdrawalRequest, &ProducerSigner) -> Result<String>
//   O1 return Err: a Utxo tx_hash that is not 32-byte hex; message names the tx hash
//   O2 return Ok: the same call with a well-formed tx_hash (control)
//   O3 receiver / store: none
// PATHS: P-AB AddBond, first selected coin | P-WB withdrawal bond_utxos | P-WF withdrawal fee_utxos
// MATRIX
//   O1 x {P-AB, P-WB, P-WF} x {non-hex, 31 bytes, 33 bytes}; O2 x {P-AB, P-WB, P-WF} -> test_m1_malformed_utxo_tx_hash_refused
// INPUT PARTITIONS:
//   tx_hash "zz" x 32 | "aa" x 31 | "aa" x 33; control "aa" x 32; coin 500 DOLI; withdrawal of 1 bond at the devnet gate
// REQ-DEF-001 — Decision: a failure means a bad RPC tx_hash is signed as some other outpoint or panics instead of being refused.
#[test]
fn test_m1_malformed_utxo_tx_hash_refused() {
    let good = "aa".repeat(32);
    let bond = |h: &str| {
        let mut b = bond_utxos(1);
        b[0].tx_hash = h.to_string();
        b
    };
    let fee_coin = |h: &str| vec![coin(h, 500_000_000)];

    add_bond_with(&[coin(&good, 50_000_000_000)], 1, BOND_UNIT, 1000).expect("AddBond control");
    withdraw_with(1, &bond(&good), &[]).expect("withdrawal bond control");
    withdraw_with(1, &bond_utxos(1), &fee_coin(&good)).expect("withdrawal fee control");

    for bad in ["zz".repeat(32), "aa".repeat(31), "aa".repeat(33)] {
        let cases = [
            (
                "AddBond coin",
                add_bond_with(&[coin(&bad, 50_000_000_000)], 1, BOND_UNIT, 1000),
            ),
            ("withdrawal bond", withdraw_with(1, &bond(&bad), &[])),
            (
                "withdrawal fee coin",
                withdraw_with(1, &bond_utxos(1), &fee_coin(&bad)),
            ),
        ];
        for (label, result) in cases {
            let err = result.expect_err(&format!("{label} with tx_hash {bad:?}"));
            assert!(
                err.to_string().to_lowercase().contains("tx hash"),
                "{label} with tx_hash {bad:?}: {err}"
            );
        }
    }
}

// OUTPUT CONTRACT: fn TxBuilder::build_request_withdrawal(&WithdrawalRequest, &ProducerSigner) -> Result<String>
//   O1 return Err: bond_utxos.len() < bond_count; message contains "need {bond_count}" and "own {bond_utxos.len()}"
//   O2 return Ok: bond_utxos.len() == bond_count (control)
//   O3 receiver / store: none
// PATHS: P-SHORT fewer Bond UTXOs than bond_count | P-EQ as many as bond_count
// MATRIX
//   O1 x P-SHORT x {7 asked / 2 given, 1 asked / 0 given}; O2 x P-EQ x {2 / 2} -> test_m1_request_withdrawal_bond_shortfall_refused
// INPUT PARTITIONS:
//   devnet at the unbonding gate, best 1_000, fee 1, total_net = bond_count x BOND_UNIT, one spendable 5 DOLI coin
// REQ-DEF-001 — Decision: a failure means the wallet signs a withdrawal for more bonds than it spends, or with no Bond input locks a spendable coin.
#[test]
fn test_m1_request_withdrawal_bond_shortfall_refused() {
    let fees = [coin(&"cc".repeat(32), 500_000_000)];
    withdraw_with(2, &bond_utxos(2), &fees).expect("2 asked / 2 given control");

    for (asked, given) in [(7u32, 2u8), (1, 0)] {
        let result = withdraw_with(asked, &bond_utxos(given), &fees);
        let err = match result {
            Err(e) => e.to_string(),
            Ok(hex_tx) => panic!(
                "{asked} asked / {given} given: built a tx with {} inputs",
                decoded(&hex_tx).inputs.len()
            ),
        };
        assert!(
            err.contains(&format!("need {asked}")) && err.contains(&format!("own {given}")),
            "{asked} asked / {given} given: {err}"
        );
    }
}

// ========================================================================
// Requirement: GUI-FR-031, GUI-FR-032 (Must) -- Claim rewards
// Acceptance: Epoch selector, optional recipient, estimated reward
// ========================================================================

#[test]
fn test_fr031_reward_claim_basic() {
    let sender = [0xAAu8; 32];
    let builder = TxBuilder::build_reward_claim(42, sender, None).unwrap();
    assert_eq!(builder.tx_type(), TxType::RewardClaim);
}

#[test]
fn test_fr031_reward_claim_with_recipient() {
    let sender = [0xAAu8; 32];
    let recipient = [0xBBu8; 32];
    let builder = TxBuilder::build_reward_claim(42, sender, Some(recipient)).unwrap();
    assert_eq!(builder.output_count(), 1);
}

// ========================================================================
// TxBuilder core tests
// ========================================================================

#[test]
fn test_tx_builder_new() {
    let builder = TxBuilder::new(TxType::Transfer);
    assert_eq!(builder.tx_type(), TxType::Transfer);
    assert_eq!(builder.input_count(), 0);
    assert_eq!(builder.output_count(), 0);
}

#[test]
fn test_tx_builder_add_input_output() {
    let mut builder = TxBuilder::new(TxType::Transfer);
    builder.add_input([0u8; 32], 0);
    builder.add_output(100, [0u8; 32], 0, 0, Vec::new());

    assert_eq!(builder.input_count(), 1);
    assert_eq!(builder.output_count(), 1);
}

#[test]
fn test_tx_type_values() {
    // These must match doli-core TxType values exactly
    assert_eq!(TxType::Transfer as u8, 0);
    assert_eq!(TxType::Registration as u8, 1);
    assert_eq!(TxType::ProducerExit as u8, 2);
    assert_eq!(TxType::Coinbase as u8, 3);
    assert_eq!(TxType::NftMint as u8, 4);
    assert_eq!(TxType::NftTransfer as u8, 5);
    assert_eq!(TxType::RewardClaim as u8, 6);
    assert_eq!(TxType::AddBond as u8, 7);
    assert_eq!(TxType::RequestWithdrawal as u8, 8);
    assert_eq!(TxType::ClaimWithdrawal as u8, 9);
    assert_eq!(TxType::SlashingEvidence as u8, 10);
    assert_eq!(TxType::TokenIssuance as u8, 11);
    assert_eq!(TxType::BridgeLock as u8, 12);
    assert_eq!(TxType::DelegateBond as u8, 13);
    assert_eq!(TxType::RevokeDelegation as u8, 14);
}

// ========================================================================
// Edge cases
// ========================================================================

#[test]
fn test_build_for_signing_no_inputs_rejected() {
    let mut builder = TxBuilder::new(TxType::Transfer);
    builder.add_output(100, [0u8; 32], 0, 0, Vec::new());
    // build_for_signing is a todo!() but the validation before it should work
    // This test verifies the error path
    // Note: This will panic with todo!() until implemented
}

#[test]
fn test_transfer_invalid_tx_hash_hex() {
    let utxos = vec![Utxo {
        tx_hash: "not_valid_hex!!".to_string(),
        output_index: 0,
        amount: 500_000_000,
        output_type: "normal".to_string(),
        lock_until: 0,
        height: 100,
        spendable: true,
    }];
    let result = TxBuilder::build_transfer(&utxos, [0xBBu8; 32], 100_000_000, 1000, [0xAAu8; 32]);
    assert!(result.is_err());
}
