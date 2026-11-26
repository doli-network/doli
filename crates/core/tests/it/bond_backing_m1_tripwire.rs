//! Bond-backing fix — source tripwire + activation-height pins (mutant guard).
//!
//! OUTPUT CONTRACT: N/A — source-content tripwire + param-value asserts.
//!
//! Purpose: a mutant that deletes or weakens the bond-backing enforcement must
//! turn a test RED. These assertions pin (a) that both consensus call sites and
//! the state clamp still name the shared check, and (b) the mainnet gate value,
//! which is IMMUTABLE once crossed (INC-I-054).

use doli_core::network::Network;

const REGISTRATION_RS: &str = include_str!("../../src/validation/registration.rs");
const TRANSACTION_RS: &str = include_str!("../../src/validation/transaction.rs");
const BOND_BACKING_RS: &str = include_str!("../../src/validation/bond_backing.rs");

fn repo_file(rel: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

#[test]
fn registration_enforces_bond_backing() {
    assert!(
        REGISTRATION_RS.contains("check_bond_backing"),
        "registration validation must call check_bond_backing — removing it \
         reopens the 3000-bond-for-10-DOLI exploit"
    );
}

#[test]
fn addbond_enforces_bond_backing() {
    assert!(
        TRANSACTION_RS.contains("check_bond_backing"),
        "the AddBond arm must call check_bond_backing — the same amplification \
         hole exists for stake increases"
    );
}

#[test]
fn check_uses_checked_multiply() {
    assert!(
        BOND_BACKING_RS.contains("checked_mul"),
        "check_bond_backing must use checked arithmetic, not a wrapping or \
         unchecked multiply"
    );
}

#[test]
fn both_flush_paths_run_the_clamp() {
    // Live apply path and the rebuild/reorg path must BOTH clamp, or a rebuilt
    // ProducerSet diverges from a live one (INC-I-034 class of bug).
    let live = repo_file("bins/node/src/node/apply_block/state_update.rs");
    let rebuild = repo_file("bins/node/src/node/rewards.rs");
    assert!(
        live.contains("clamp_bond_counts_to_locked"),
        "live epoch-flush path must clamp bond_count to locked"
    );
    assert!(
        rebuild.contains("clamp_bond_counts_to_locked"),
        "rebuild/reorg epoch-flush path must clamp bond_count to locked"
    );
}

#[test]
fn mainnet_gate_is_pinned_and_immutable() {
    // IMMUTABLE once crossed (INC-I-054): this value must never move once the
    // chain passes it. Chosen as an epoch boundary (2921 * 360) ~48 h ahead of
    // the tip at fix time.
    let p = Network::Mainnet.params();
    assert_eq!(
        p.bond_backing_activation_height, 1_051_560,
        "mainnet bond_backing_activation_height pin changed — if the chain has \
         not yet crossed it this is a deliberate re-pin; if it has, this is a \
         forbidden move of consensus history"
    );
    assert_eq!(1_051_560 % 360, 0, "gate must land on an epoch boundary");
}

#[test]
fn devnet_and_testnet_frozen_until_pinned() {
    // Both ship FROZEN (u64::MAX): a dev/test run that wants the rule live sets
    // DOLI_BOND_BACKING_ACTIVATION_HEIGHT. Mainnet is the only pinned network.
    assert_eq!(
        Network::Devnet.params().bond_backing_activation_height,
        u64::MAX
    );
    assert_eq!(
        Network::Testnet.params().bond_backing_activation_height,
        u64::MAX
    );
}
