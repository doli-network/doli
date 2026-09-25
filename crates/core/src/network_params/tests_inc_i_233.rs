//! INC-I-233 F4 — env override lock for `inc_i_233_activation_height` (REQ-233-F4-AH).
//!
// OUTPUT CONTRACT: fn env_loader::load_from_env(network: Network) -> NetworkParams
//   O1 return .inc_i_233_activation_height <- env value off mainnet, default on mainnet
//   O2 return .inc_i_233_activation_height with env unset <- defaults(network) value
//   O3 mutable params / receiver / store: NONE (reads process env only)
//   PATHS: mainnet arm (locked) | non-mainnet arm (env_parse)
//   INPUT PARTITIONS: IP-M mainnet | IP-T testnet | IP-D devnet; env set ("7") | env unset
//   MATRIX: {IP-M, IP-T, IP-D} x {env set, env unset} -> the two tests below cover all 6 cells

use std::sync::Mutex;

use crate::Network;

use super::NetworkParams;

const ENV: &str = "DOLI_INC_I_233_ACTIVATION_HEIGHT";

static ENV_MUTEX: Mutex<()> = Mutex::new(());

fn load_with(value: Option<&str>) -> [NetworkParams; 3] {
    let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let original = std::env::var(ENV);
    match value {
        Some(v) => std::env::set_var(ENV, v),
        None => std::env::remove_var(ENV),
    }
    let out = [
        super::env_loader::load_from_env(Network::Mainnet),
        super::env_loader::load_from_env(Network::Testnet),
        super::env_loader::load_from_env(Network::Devnet),
    ];
    match original {
        Ok(v) => std::env::set_var(ENV, v),
        Err(_) => std::env::remove_var(ENV),
    }
    out
}

// REQ-233-F4-AH — Decision: a failure means one operator's .env can arm/disarm a consensus gate on mainnet.
#[test]
fn inc_i_233_f4_env_override_locked_on_mainnet_honoured_elsewhere() {
    let [mainnet, testnet, devnet] = load_with(Some("7"));
    assert_eq!(
        mainnet.inc_i_233_activation_height,
        u64::MAX,
        "mainnet must ignore env"
    );
    assert_eq!(
        testnet.inc_i_233_activation_height, 7,
        "testnet must honour env"
    );
    assert_eq!(
        devnet.inc_i_233_activation_height, 7,
        "devnet must honour env"
    );
}

// REQ-233-F4-AH — Decision: a failure means the env path diverges from defaults() when unset.
#[test]
fn inc_i_233_f4_env_unset_falls_back_to_defaults() {
    let [mainnet, testnet, devnet] = load_with(None);
    assert_eq!(mainnet.inc_i_233_activation_height, u64::MAX);
    assert_eq!(testnet.inc_i_233_activation_height, u64::MAX);
    assert_eq!(devnet.inc_i_233_activation_height, 0);
}
