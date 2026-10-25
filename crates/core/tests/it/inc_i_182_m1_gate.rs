//! INC-I-182 M1 — producer-auth gate: per-network values, devnet-only env
//! override, and a context that is off unless a site sets the gate.
//!
// covers: producer_auth transaction types unbonding defaults env_loader network_params pool assembly tx_processing validation_checks withdrawal_holdings genesis register bonds withdrawal exit unbonding_tx core data block rpc_client
//
// OUTPUT CONTRACT: fn NetworkParams::defaults(Network) / fn NetworkParams::load(Network)
//   O1 return .inc_i_182_producer_auth_activation_height
// OUTPUT CONTRACT: fn ValidationContext::new(..) / fn with_inc_i_182_producer_auth(self, u64) -> Self
//   O2 verdict of check_producer_auth on an unsigned withdrawal (default off, builder on)
//   O3 neighbour fields untouched by the builder; pure, no store writes
// PATHS: P-DEF defaults | P-ENV load in a re-exec child (fresh OnceLock) | P-CTX context builder
// MATRIX
//   O1 x P-DEF -> inc_i_182_m1_req_pauth_003_gate_defaults_per_network
//   O2, O3 x P-CTX -> inc_i_182_m1_req_pauth_003_context_default_off_and_builder
//   O1 x P-ENV x unset -> inc_i_182_m1_req_pauth_003_env_unset_loads_defaults
//   O1 x P-ENV x {500, u64::MAX} -> inc_i_182_m1_req_pauth_003_env_gate_honoured_on_devnet_only
// INPUT PARTITIONS:
//   network: mainnet | testnet | devnet; env: unset | 500 | u64::MAX; ctx height: 0 | 1_000 | u64::MAX - 1

use doli_core::consensus::ConsensusParams;
use doli_core::transaction::TxType;
use doli_core::validation::producer_auth::check_producer_auth;
use doli_core::validation::ValidationContext;
use doli_core::{Network, NetworkParams};

use super::inc_i_182_m1_core_fx::{coded, unsigned};

const GATE_ENV: &str = "DOLI_INC_I_182_PRODUCER_AUTH_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_182_M1_GATE_ENV_CHILD";
const NETWORKS: [Network; 3] = [Network::Mainnet, Network::Testnet, Network::Devnet];

/// Parent: re-exec `test` alone with exactly `env` set and return true.
/// Child: return false so the caller runs its assertions.
fn spawned_child(test: &str, env: &[(&str, u64)]) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.arg(format!("inc_i_182_m1_gate::{test}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env_remove(GATE_ENV);
    for (key, value) in env {
        cmd.env(key, value.to_string());
    }
    let out = cmd.output().expect("spawn child test process");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{test}: child failed or did not run:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

/// Testnet pin of the four defense gates (user decision 2026-10-25); see
/// `network_params::tests_defense_pin`.
const TESTNET_PIN: u64 = 330_849;

fn gates(params: [&NetworkParams; 3]) -> [u64; 3] {
    params.map(|p| p.inc_i_182_producer_auth_activation_height)
}

fn loaded() -> [u64; 3] {
    gates(NETWORKS.map(NetworkParams::load))
}

// REQ-PAUTH-003 — Decision: a failure means the rule ships armed on mainnet or testnet before a height is pinned (a consensus split with old nodes), or devnet loses its armed default.
#[test]
fn inc_i_182_m1_req_pauth_003_gate_defaults_per_network() {
    let d = NETWORKS.map(NetworkParams::defaults);
    assert_eq!(gates([&d[0], &d[1], &d[2]]), [u64::MAX, TESTNET_PIN, 0]);
}

// REQ-PAUTH-003 — Decision: a failure means a context site that never sets the gate enforces the rule (old test and tool contexts break) or the builder clobbers a neighbour gate.
#[test]
fn inc_i_182_m1_req_pauth_003_context_default_off_and_builder() {
    let tx = unsigned(TxType::RequestWithdrawal, 0x50);
    for height in [0, 1_000, u64::MAX - 1] {
        let base = ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height)
            .with_inc_i_255_slashing_activation_height(255_255)
            .with_inc_i_257_unbonding(257_257, 61);
        let off = check_producer_auth(&tx, &base);
        assert!(
            off.is_ok(),
            "default context at {height} must be off: {off:?}"
        );
        let on = base.clone().with_inc_i_182_producer_auth(0);
        assert!(
            coded(&check_producer_auth(&tx, &on)),
            "builder at 0 must arm the rule at {height}"
        );
        assert_eq!(
            (
                on.current_height,
                on.inc_i_255_slashing_activation_height,
                on.inc_i_257_unbonding_activation_height,
                on.unbonding_period,
                on.inc_i_257_rules_at_next_height,
            ),
            (height, 255_255, 257_257, 61, false),
            "builder moved a neighbour field at {height}"
        );
    }
}

// REQ-PAUTH-003 — Decision: a failure means the runtime path diverges from defaults() when no override is set.
#[test]
fn inc_i_182_m1_req_pauth_003_env_unset_loads_defaults() {
    if spawned_child("inc_i_182_m1_req_pauth_003_env_unset_loads_defaults", &[]) {
        return;
    }
    assert_eq!(loaded(), [u64::MAX, TESTNET_PIN, 0]);
}

fn env_case(value: u64) {
    assert_eq!(
        std::env::var(GATE_ENV).ok(),
        Some(value.to_string()),
        "PRECONDITION"
    );
    assert_eq!(
        loaded(),
        [u64::MAX, TESTNET_PIN, value],
        "only devnet may honour {GATE_ENV}={value}"
    );
}

// REQ-PAUTH-003 — Decision: a failure means an operator env var arms or moves the producer-auth rule on a mainnet or testnet build (a local consensus split), or the devnet override does not work.
#[test]
fn inc_i_182_m1_req_pauth_003_env_gate_honoured_on_devnet_only() {
    for value in [500, u64::MAX] {
        let test = if value == 500 {
            "inc_i_182_m1_req_pauth_003_env_child_500"
        } else {
            "inc_i_182_m1_req_pauth_003_env_child_max"
        };
        assert!(
            spawned_child(test, &[(GATE_ENV, value)]),
            "PRECONDITION: parent"
        );
    }
}

// REQ-PAUTH-003 — Decision: child of the env test; a failure means devnet ignores the override at 500 or another network honours it.
#[test]
fn inc_i_182_m1_req_pauth_003_env_child_500() {
    if std::env::var_os(CHILD_ENV).is_some() {
        env_case(500);
    }
}

// REQ-PAUTH-003 — Decision: child of the env test; a failure means devnet cannot be moved above the chain, which the below-gate golden test relies on.
#[test]
fn inc_i_182_m1_req_pauth_003_env_child_max() {
    if std::env::var_os(CHILD_ENV).is_some() {
        env_case(u64::MAX);
    }
}
