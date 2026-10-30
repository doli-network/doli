//! INC-I-288 F-1 (AUDIT-P2-629-006) — lock of `inc_i_288_fork_choice_activation_height`.
//!
//! Threat: one edit to `defaults.rs` or to the mainnet arm of `env_loader.rs`
//! arms the fork-choice gate on mainnet (by default or by one operator's .env)
//! before its own pin decision. Commit 4edf0d20 cites this test by name.
//!
// OUTPUT CONTRACT: fn env_loader::load_from_env(network: Network) -> NetworkParams
//   O1 return .inc_i_288_fork_choice_activation_height, env set <- default on mainnet, env value elsewhere
//   O2 return .inc_i_288_fork_choice_activation_height, env unset <- defaults(network) value
//   O3 mutable params / receiver / store: NONE (reads process env only)
//   PATHS: P1 mainnet arm (locked) | P2 non-mainnet arm (env_parse)
//   INPUT PARTITIONS: IP-M mainnet | IP-T testnet | IP-D devnet; env set ("7") | env unset
//   MATRIX: {IP-M, IP-T, IP-D} x {env set, env unset} = 6 cells, plus defaults() x 3 = 9 asserts

use std::sync::Mutex;

use crate::Network;

use super::NetworkParams;

const ENV: &str = "DOLI_INC_I_288_FORK_CHOICE_ACTIVATION_HEIGHT";

static ENV_MUTEX: Mutex<()> = Mutex::new(());

fn load_with(value: Option<&str>) -> [u64; 3] {
    let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let original = std::env::var(ENV);
    match value {
        Some(v) => std::env::set_var(ENV, v),
        None => std::env::remove_var(ENV),
    }
    let out = [Network::Mainnet, Network::Testnet, Network::Devnet]
        .map(|n| super::env_loader::load_from_env(n).inc_i_288_fork_choice_activation_height);
    match original {
        Ok(v) => std::env::set_var(ENV, v),
        Err(_) => std::env::remove_var(ENV),
    }
    out
}

// AUDIT-P2-629-006 — Decision: a failure means mainnet fork choice can change by default or by one node's .env before its pin decision.
#[test]
fn network_params_mainnet_lock() {
    let defaults = [Network::Mainnet, Network::Testnet, Network::Devnet]
        .map(|n| NetworkParams::defaults(n).inc_i_288_fork_choice_activation_height);
    assert_eq!(
        defaults,
        [u64::MAX, 366_891, 0],
        "defaults(): mainnet MAX, testnet 366_891, devnet 0"
    );

    let [mainnet, testnet, devnet] = load_with(None);
    assert_eq!(mainnet, u64::MAX, "env unset: mainnet must stay u64::MAX");
    assert_eq!(testnet, 366_891, "env unset: testnet must be 366_891");
    assert_eq!(devnet, 0, "env unset: devnet must be 0");

    let [mainnet, testnet, devnet] = load_with(Some("7"));
    assert_eq!(
        mainnet,
        u64::MAX,
        "env set: mainnet must ignore the override"
    );
    assert_eq!(testnet, 7, "env set: testnet must honour the override");
    assert_eq!(devnet, 7, "env set: devnet must honour the override");
}
