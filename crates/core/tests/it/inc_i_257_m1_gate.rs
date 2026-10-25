//! INC-I-257 M1 — unbonding gate and unbonding period: per-network values, gate
//! order against slashing, devnet-only env overrides, the context fields.
//!
// covers: network_params defaults env_loader ordering validation types
//
// OUTPUT CONTRACT: fn NetworkParams::defaults(Network) / fn NetworkParams::load(Network)
//   O1 return .inc_i_257_unbonding_activation_height
//   O2 return .unbonding_period
// OUTPUT CONTRACT: fn NetworkParams::effective_slashing_activation_height(&self) -> u64
//   O3 return; the unbonding gate must never exceed it on the defaults
// OUTPUT CONTRACT: fn ValidationContext::new(..) / fn with_inc_i_257_unbonding(self, u64, u64) -> Self
//   O4 return .inc_i_257_unbonding_activation_height (default u64::MAX) and .unbonding_period;
//      neighbour gates and height untouched; pure, no store writes
// PATHS: P-DEF defaults | P-ENV load in a re-exec child (fresh OnceLock) | P-CTX context builder
// INPUT PARTITIONS:
//   P-DEF: mainnet -> gate 806_859; testnet -> 330_849; devnet -> 0; period 60_480 / 72 / 60
//   P-ENV: nothing set | gate env set (with a higher slashing env, so no order inversion) |
//          period env set | both set
//   P-CTX: heights 0, 1_000, u64::MAX - 1; builder with sentinels
// MATRIX:
//   O1 x P-DEF -> inc_i_257_m1_req_unbond_015_gate_defaults_per_network
//   O1,O3 x P-DEF -> inc_i_257_m1_req_unbond_016_gate_never_above_effective_slashing
//   O2 x P-DEF -> inc_i_257_m1_req_unbond_008_period_defaults_per_network
//   O4 x P-CTX -> inc_i_257_m1_req_unbond_008_context_default_fail_closed_and_builder
//   O1,O2 x P-ENV -> inc_i_257_m1_req_unbond_018_env_* / inc_i_257_m1_req_unbond_008_env_* (child each)

use doli_core::consensus::ConsensusParams;
use doli_core::validation::ValidationContext;
use doli_core::{Network, NetworkParams};

const GATE_ENV: &str = "DOLI_INC_I_257_UNBONDING_ACTIVATION_HEIGHT";
const PERIOD_ENV: &str = "DOLI_UNBONDING_PERIOD";
const SLASH_ENV: &str = "DOLI_INC_I_255_SLASHING_ACTIVATION_HEIGHT";
const SIG_ENV: &str = "DOLI_INC_I_236_BLOCK_SIGNATURE_ACTIVATION_HEIGHT";
const ROOT_ENV: &str = "DOLI_INC_I_181_PENDING_ROOT_ACTIVATION_HEIGHT";
const CHILD_ENV: &str = "INC_I_257_M1_GATE_ENV_CHILD";
const NETWORKS: [Network; 3] = [Network::Mainnet, Network::Testnet, Network::Devnet];
const PERIODS: [u64; 3] = [60_480, 72, 60];
/// Pins of the four defense gates (user decisions); see
/// `network_params::tests_defense_pin`.
const MAINNET_PIN: u64 = 806_859;
const TESTNET_PIN: u64 = 330_849;

/// Parent: re-exec this test alone with exactly `env` set and return true.
/// Child: return false so the caller runs its assertions.
fn spawned_child(test: &str, env: &[(&str, u64)]) -> bool {
    if std::env::var_os(CHILD_ENV).is_some() {
        return false;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().expect("current exe"));
    cmd.arg(format!("inc_i_257_m1_gate::{test}"))
        .args(["--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1");
    for key in [GATE_ENV, PERIOD_ENV, SLASH_ENV, SIG_ENV, ROOT_ENV] {
        cmd.env_remove(key);
    }
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

fn loaded() -> [&'static NetworkParams; 3] {
    NETWORKS.map(NetworkParams::load)
}

// REQ-UNBOND-015 — Decision: a failure means the unbonding lock engages on mainnet or testnet at a height other than the pinned one, or devnet loses its armed default.
#[test]
fn inc_i_257_m1_req_unbond_015_gate_defaults_per_network() {
    for (network, expected) in [
        (Network::Mainnet, MAINNET_PIN),
        (Network::Testnet, TESTNET_PIN),
        (Network::Devnet, 0),
    ] {
        assert_eq!(
            NetworkParams::defaults(network).inc_i_257_unbonding_activation_height,
            expected,
            "{network:?}"
        );
    }
}

// REQ-UNBOND-016 — Decision: a failure means some network has a height range where slashing works but a withdrawal still pays out instantly (AUDIT-P2-540 reopened).
#[test]
fn inc_i_257_m1_req_unbond_016_gate_never_above_effective_slashing() {
    for network in NETWORKS {
        let p = NetworkParams::defaults(network);
        assert!(
            p.inc_i_257_unbonding_activation_height <= p.effective_slashing_activation_height(),
            "{network:?}: unbonding gate {} above effective slashing gate {}",
            p.inc_i_257_unbonding_activation_height,
            p.effective_slashing_activation_height()
        );
    }
}

// REQ-UNBOND-008 — Decision: a failure means the consensus lock length differs from the agreed per-network value, so nodes and the CLI disagree on when withdrawn funds unlock.
#[test]
fn inc_i_257_m1_req_unbond_008_period_defaults_per_network() {
    for (network, expected) in NETWORKS.into_iter().zip(PERIODS) {
        assert_eq!(
            NetworkParams::defaults(network).unbonding_period,
            expected,
            "{network:?}"
        );
    }
    assert_eq!(
        NetworkParams::defaults(Network::Mainnet).unbonding_period,
        doli_core::consensus::UNBONDING_PERIOD,
        "mainnet period equals the global constant"
    );
}

// REQ-UNBOND-008 — Decision: a failure means a context site that forgets the builder arms the lock (fail-open) or the builder clobbers a neighbour gate.
#[test]
fn inc_i_257_m1_req_unbond_008_context_default_fail_closed_and_builder() {
    for height in [0, 1_000, u64::MAX - 1] {
        let ctx = ValidationContext::new(ConsensusParams::devnet(), Network::Devnet, 0, height);
        assert_eq!(
            ctx.inc_i_257_unbonding_activation_height,
            u64::MAX,
            "default at height {height}"
        );
        let set = ctx
            .clone()
            .with_inc_i_255_slashing_activation_height(255_255)
            .with_inc_i_257_unbonding(257_257, 61);
        assert_eq!(set.inc_i_257_unbonding_activation_height, 257_257);
        assert_eq!(set.unbonding_period, 61);
        assert_eq!(set.inc_i_255_slashing_activation_height, 255_255);
        assert_eq!(set.current_height, height);
        assert_eq!(
            set.inc_i_234_activation_height,
            ctx.inc_i_234_activation_height
        );
        assert_eq!(
            set.inc_i_173_activation_height,
            ctx.inc_i_173_activation_height
        );
    }
}

// REQ-UNBOND-018 — Decision: a failure means the runtime path diverges from defaults() when no override is set.
#[test]
fn inc_i_257_m1_req_unbond_018_env_unset_loads_defaults() {
    if spawned_child("inc_i_257_m1_req_unbond_018_env_unset_loads_defaults", &[]) {
        return;
    }
    for (network, p) in NETWORKS.iter().zip(loaded()) {
        let d = NetworkParams::defaults(*network);
        assert_eq!(
            p.inc_i_257_unbonding_activation_height, d.inc_i_257_unbonding_activation_height,
            "{network:?} gate"
        );
        assert_eq!(p.unbonding_period, d.unbonding_period, "{network:?} period");
        assert!(
            p.inc_i_257_unbonding_activation_height <= p.effective_slashing_activation_height(),
            "{network:?}: loaded order"
        );
    }
}

// REQ-UNBOND-018 / REQ-UNBOND-SEC-003 — Decision: a failure means an operator env var arms or moves the unbonding lock on a mainnet or testnet build (a local consensus split).
#[test]
fn inc_i_257_m1_req_unbond_018_env_gate_honoured_on_devnet_only() {
    if spawned_child(
        "inc_i_257_m1_req_unbond_018_env_gate_honoured_on_devnet_only",
        &[(GATE_ENV, 500), (SLASH_ENV, 1_000)],
    ) {
        return;
    }
    assert_eq!(
        std::env::var(GATE_ENV).as_deref(),
        Ok("500"),
        "PRECONDITION"
    );
    let [mainnet, testnet, devnet] = loaded();
    assert_eq!(
        mainnet.inc_i_257_unbonding_activation_height, MAINNET_PIN,
        "mainnet must ignore the override and keep its pin"
    );
    assert_eq!(
        testnet.inc_i_257_unbonding_activation_height, TESTNET_PIN,
        "testnet must ignore the override"
    );
    assert_eq!(
        devnet.inc_i_257_unbonding_activation_height, 500,
        "devnet must honour the override"
    );
    assert_eq!(devnet.effective_slashing_activation_height(), 1_000);
}

// REQ-UNBOND-008 / REQ-UNBOND-SEC-003 — Decision: a failure means a testnet operator env var changes the consensus lock length, so testnet nodes disagree on when an Unbonding output unlocks.
#[test]
fn inc_i_257_m1_req_unbond_008_env_period_honoured_on_devnet_only() {
    if spawned_child(
        "inc_i_257_m1_req_unbond_008_env_period_honoured_on_devnet_only",
        &[(PERIOD_ENV, 123)],
    ) {
        return;
    }
    assert_eq!(
        std::env::var(PERIOD_ENV).as_deref(),
        Ok("123"),
        "PRECONDITION"
    );
    let [mainnet, testnet, devnet] = loaded();
    assert_eq!(
        mainnet.unbonding_period, 60_480,
        "mainnet must ignore the override"
    );
    assert_eq!(
        testnet.unbonding_period, 72,
        "testnet must ignore the override"
    );
    assert_eq!(
        devnet.unbonding_period, 123,
        "devnet must honour the override"
    );
}

// REQ-UNBOND-018 — Decision: a failure means the two devnet overrides interfere (one masks the other) when both are set.
#[test]
fn inc_i_257_m1_req_unbond_018_env_both_set_devnet_reads_both() {
    if spawned_child(
        "inc_i_257_m1_req_unbond_018_env_both_set_devnet_reads_both",
        &[(GATE_ENV, 40), (PERIOD_ENV, 9), (SLASH_ENV, 40)],
    ) {
        return;
    }
    let [mainnet, testnet, devnet] = loaded();
    assert_eq!(
        (
            devnet.inc_i_257_unbonding_activation_height,
            devnet.unbonding_period
        ),
        (40, 9),
        "devnet reads both"
    );
    for (p, gate, period) in [(mainnet, MAINNET_PIN, 60_480), (testnet, TESTNET_PIN, 72)] {
        assert_eq!(
            (p.inc_i_257_unbonding_activation_height, p.unbonding_period),
            (gate, period)
        );
    }
}
