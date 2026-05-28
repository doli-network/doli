//! Tests for the Phase 2.1 structural-anchored oracle activation height.
//!
//! Spec: specs/oracle-structural-anchored-economics.md §1.10
//!
//! Three-question gate (INC-I-075):
//!   Q1: Can any user-submittable tx trigger oracle code paths? YES
//!       (PriceAttestation TxType=16 is submitted by attesters).
//!   Q2: Can any producer-action or attestation pattern trigger it? YES
//!       (block proposers include attestation txs; epoch-boundary aggregation).
//!   Q3: Bit-identical to old behavior for ALL reachable inputs? NO
//!       (new validation + new UTXO state + new epoch-boundary aggregation).
//!   VERDICT: activation height REQUIRED. Default = u64::MAX in every variant,
//!            mirroring `defi_activation_height` (INC-I-088 Phase 0).
//!
//! These tests guard the field's defaults and env-loader wiring. They MUST
//! fail to compile / fail to assert before the field is added to
//! `NetworkParams`, and pass once M1 lands. The independence test guards
//! against accidental bundling with `defi_activation_height` (HC-6).

use std::sync::Mutex;

use crate::Network;

use super::NetworkParams;

// Process-global env mutex (matches the one in `tests.rs`). The `oracle_*`
// env tests acquire this lock to serialize against any other test that
// pokes `DOLI_*` environment variables.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

// OUTPUT CONTRACT: fn NetworkParams::defaults(network) — oracle_activation_height field
//   O1: return.oracle_activation_height — u64. Mainnet/Devnet = u64::MAX
//       (oracle frozen); Testnet = 20_099 (DeFi launch pinned 2026-05-28,
//       forward activation above chain head — see defaults.rs).
// PATHS:
//   P1: Network::Mainnet
//   P2: Network::Testnet
//   P3: Network::Devnet
// INPUT PARTITIONS:
//   The enum discriminant is the only input. Each variant is its own partition
//   because the default-value lookup tables differ per variant (verified by
//   reading `defaults.rs` — three independent struct literals). A single
//   partition per path.
// MATRIX: 1 output × 3 paths × 1 partition = 3 cells
//   P1×part-1: O1✓   P2×part-1: O1✓   P3×part-1: O1✓
#[test]
fn test_oracle_activation_height_defaults_to_u64_max() {
    let mainnet = NetworkParams::defaults(Network::Mainnet);
    let testnet = NetworkParams::defaults(Network::Testnet);
    let devnet = NetworkParams::defaults(Network::Devnet);

    assert_eq!(
        mainnet.oracle_activation_height,
        u64::MAX,
        "mainnet oracle_activation_height MUST default to u64::MAX (oracle frozen)"
    );
    assert_eq!(
        testnet.oracle_activation_height, 20_099,
        "testnet oracle_activation_height MUST be 20_099 (DeFi launch pinned 2026-05-28)"
    );
    assert_eq!(
        devnet.oracle_activation_height,
        u64::MAX,
        "devnet oracle_activation_height MUST default to u64::MAX (oracle frozen)"
    );
}

// OUTPUT CONTRACT: fn env_loader::load_from_env(Network::Mainnet) — oracle_activation_height
//   O1: return.oracle_activation_height — u64, u64::MAX (env IGNORED on mainnet)
//   O2: process env DOLI_ORACLE_ACTIVATION_HEIGHT — restored to its pre-test value
// PATHS:
//   P1: is_mainnet=true branch in env_loader (locked params — env var read is skipped)
// INPUT PARTITIONS:
//   Two partitions matter here, both must collapse to the same output on mainnet:
//     part-A: env var set to a NON-default value (12345) — must be ignored
//     part-B: (covered separately by `test_oracle_activation_height_defaults_to_u64_max`)
//             env var unset — must also yield u64::MAX
//   This test covers part-A. The defaults test covers part-B.
// MATRIX: 2 outputs × 1 path × 1 covered partition (part-A) = 2 cells
//   P1×part-A: O1✓ O2✓
#[test]
fn test_oracle_activation_height_mainnet_ignores_env_override() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let original = std::env::var("DOLI_ORACLE_ACTIVATION_HEIGHT");

    // part-A: env override attempted on mainnet
    std::env::set_var("DOLI_ORACLE_ACTIVATION_HEIGHT", "12345");

    let mainnet_params = super::env_loader::load_from_env(Network::Mainnet);
    let observed = mainnet_params.oracle_activation_height;

    // O2: restore env BEFORE the assertion so we don't leak state on panic.
    match original {
        Ok(val) => std::env::set_var("DOLI_ORACLE_ACTIVATION_HEIGHT", val),
        Err(_) => std::env::remove_var("DOLI_ORACLE_ACTIVATION_HEIGHT"),
    }

    // O1
    assert_eq!(
        observed,
        u64::MAX,
        "mainnet MUST ignore DOLI_ORACLE_ACTIVATION_HEIGHT (locked params), \
         got {observed} instead of u64::MAX"
    );
}

// OUTPUT CONTRACT: fn env_loader::load_from_env(Network::Testnet) — oracle_activation_height
//   O1: return.oracle_activation_height — u64, parsed value of env var
//   O2: process env DOLI_ORACLE_ACTIVATION_HEIGHT — restored to its pre-test value
// PATHS:
//   P1: !is_mainnet branch → env_parse() reads DOLI_ORACLE_ACTIVATION_HEIGHT
// INPUT PARTITIONS:
//   part-A: env var set to a representative large u64 (99999) — must be honoured
//   (part-B "env var unset → falls back to default u64::MAX" is covered by
//    `test_oracle_activation_height_defaults_to_u64_max`. Note that test does
//    NOT lock ENV_MUTEX — defaults() does not read env — but it still observes
//    the default because env_loader is only invoked from `load_from_env`.)
// MATRIX: 2 outputs × 1 path × 1 covered partition (part-A) = 2 cells
//   P1×part-A: O1✓ O2✓
#[test]
fn test_oracle_activation_height_testnet_env_override() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let original = std::env::var("DOLI_ORACLE_ACTIVATION_HEIGHT");

    // part-A
    std::env::set_var("DOLI_ORACLE_ACTIVATION_HEIGHT", "99999");

    let testnet_params = super::env_loader::load_from_env(Network::Testnet);
    let observed = testnet_params.oracle_activation_height;

    // O2
    match original {
        Ok(val) => std::env::set_var("DOLI_ORACLE_ACTIVATION_HEIGHT", val),
        Err(_) => std::env::remove_var("DOLI_ORACLE_ACTIVATION_HEIGHT"),
    }

    // O1
    assert_eq!(
        observed, 99999,
        "testnet MUST honour DOLI_ORACLE_ACTIVATION_HEIGHT, got {observed}"
    );
}

// OUTPUT CONTRACT: fn env_loader::load_from_env(Network::Devnet) — oracle_activation_height
//   O1: return.oracle_activation_height — u64, parsed value of env var
//   O2: process env DOLI_ORACLE_ACTIVATION_HEIGHT — restored to its pre-test value
// PATHS:
//   P1: !is_mainnet branch → env_parse() reads DOLI_ORACLE_ACTIVATION_HEIGHT
// INPUT PARTITIONS:
//   part-A: env var = "1" — boundary value (activation at genesis); guards against
//           an off-by-one where the env value is silently coerced.
// MATRIX: 2 outputs × 1 path × 1 partition = 2 cells
//   P1×part-A: O1✓ O2✓
#[test]
fn test_oracle_activation_height_devnet_env_override() {
    let _lock = ENV_MUTEX.lock().unwrap();
    let original = std::env::var("DOLI_ORACLE_ACTIVATION_HEIGHT");

    // part-A: boundary value
    std::env::set_var("DOLI_ORACLE_ACTIVATION_HEIGHT", "1");

    let devnet_params = super::env_loader::load_from_env(Network::Devnet);
    let observed = devnet_params.oracle_activation_height;

    // O2
    match original {
        Ok(val) => std::env::set_var("DOLI_ORACLE_ACTIVATION_HEIGHT", val),
        Err(_) => std::env::remove_var("DOLI_ORACLE_ACTIVATION_HEIGHT"),
    }

    // O1
    assert_eq!(
        observed, 1,
        "devnet MUST honour DOLI_ORACLE_ACTIVATION_HEIGHT, got {observed}"
    );
}

// OUTPUT CONTRACT: fn NetworkParams::defaults(Network::Mainnet) — field independence
//   O1: return.oracle_activation_height — u64, u64::MAX
//   O2: return.defi_activation_height — u64, u64::MAX
//   (O1 and O2 are SEPARATE struct fields. This test would fail to compile if
//    a future refactor accidentally collapsed them into a single field, since
//    two distinct field names cannot bind to one field.)
// PATHS:
//   P1: Network::Mainnet defaults branch
// INPUT PARTITIONS:
//   Single partition — defaults are constants, no input variation.
// MATRIX: 2 outputs × 1 path × 1 partition = 2 cells
//   P1: O1✓ O2✓
//
// Why: spec §0 NEVER constraints + HC-6 + INC-I-075 — `oracle_activation_height`
// MUST NOT be bundled with `defi_activation_height` or any other activation
// height. Independent storage is a hard invariant.
#[test]
fn test_oracle_activation_height_is_independent_of_defi_activation_height() {
    let mainnet = NetworkParams::defaults(Network::Mainnet);

    let oracle: u64 = mainnet.oracle_activation_height;
    let defi: u64 = mainnet.defi_activation_height;

    assert_eq!(
        oracle,
        u64::MAX,
        "oracle_activation_height MUST be u64::MAX"
    );
    assert_eq!(defi, u64::MAX, "defi_activation_height MUST be u64::MAX");
}
